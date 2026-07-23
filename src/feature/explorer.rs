//! Explorer: expandable file tree rooted at the launch cwd.
//!
//! Navigation keys and “open file” come from [`crate::config::Config`]:
//! internal actions stay in-process; other actions run as shell functions
//! from the user’s `config.sh`.

use super::view::{FeatureView, KeyOutcome};
use crate::config::{self, Config};
use crate::ui::{self, Palette};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::cell::Cell;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

const NOTICE_SUCCESS_TTL: Duration = Duration::from_secs(2);
const NOTICE_ERROR_TTL: Duration = Duration::from_secs(4);

#[derive(Clone, Debug)]
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    depth: usize,
}

#[derive(Clone, Debug)]
enum PendingEdit {
    Create { parent: PathBuf },
    Rename { from: PathBuf },
    Delete { path: PathBuf },
}

fn visible_name(name: &str, show_hidden: bool) -> bool {
    name != ".git" && (show_hidden || !name.starts_with('.'))
}

/// Move a path to the desktop trash via FreeDesktop `gio trash`.
/// Never permanently unlink from Explorer.
fn trash_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err("path does not exist".into());
    }
    let output = Command::new("gio")
        .args(["trash", "--"])
        .arg(path)
        .output()
        .map_err(|error| format!("gio trash: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!("gio trash failed ({})", output.status))
    } else {
        Err(stderr)
    }
}

fn watch_event_affects_tree(root: &Path, event: &notify::Event) -> bool {
    let git_dir = root.join(".git");
    event.paths.is_empty() || event.paths.iter().any(|path| !path.starts_with(&git_dir))
}

/// Restore a path selection after rebuilding. If the exact path vanished
/// (hidden toggle, collapse, deletion), choose its nearest visible ancestor.
fn restored_index(rows: &[Entry], selected_path: Option<&Path>, old_index: usize) -> usize {
    if let Some(path) = selected_path {
        for ancestor in path.ancestors() {
            if let Some(i) = rows.iter().position(|row| row.path == ancestor) {
                return i;
            }
        }
    }
    old_index.min(rows.len().saturating_sub(1))
}

fn create_target(parent: &Path, input: &str) -> Result<(PathBuf, bool), String> {
    let input = input.trim();
    let is_dir = input.ends_with('/');
    let relative = input.trim_end_matches('/');
    if relative.is_empty() {
        return Err("name cannot be empty".into());
    }
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("use a relative path without . or ..".into());
    }
    Ok((parent.join(path), is_dir))
}

fn rename_target(from: &Path, input: &str) -> Result<PathBuf, String> {
    let name = input.trim();
    if name.is_empty() {
        return Err("name cannot be empty".into());
    }
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err("rename accepts one file name".into());
    }
    let parent = from
        .parent()
        .ok_or_else(|| "cannot rename root".to_string())?;
    Ok(parent.join(name))
}

pub struct ExplorerView {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    rows: Vec<Entry>,
    selected: usize,
    scroll: usize,
    body_top: Cell<u16>,
    body_height: Cell<u16>,
    nerd_font: bool,
    show_hidden: bool,
    error: Option<String>,
    notice: Option<(String, bool)>,
    notice_at: Option<Instant>,
    pending: Option<PendingEdit>,
    input: Vec<char>,
    cursor: usize,
    _watcher: Option<notify::RecommendedWatcher>,
    watch_rx: Receiver<notify::Result<notify::Event>>,
    watch_dirty: Option<Instant>,
    config: Arc<Config>,
}

impl ExplorerView {
    pub fn new(root: PathBuf, nerd_font: bool, config: Arc<Config>) -> Self {
        let root = root.canonicalize().unwrap_or(root);
        let (watch_tx, watch_rx) = mpsc::channel();
        let mut watch_notice = None;
        let watcher = match notify::recommended_watcher(move |event| {
            let _ = watch_tx.send(event);
        }) {
            Ok(mut watcher) => match watcher.watch(&root, RecursiveMode::Recursive) {
                Ok(()) => Some(watcher),
                Err(error) => {
                    watch_notice = Some((format!("watch: {error}"), true));
                    None
                }
            },
            Err(error) => {
                watch_notice = Some((format!("watch: {error}"), true));
                None
            }
        };
        let notice_at = watch_notice.as_ref().map(|_| Instant::now());
        let mut view = Self {
            root: root.clone(),
            expanded: HashSet::from([root]),
            rows: Vec::new(),
            selected: 0,
            scroll: 0,
            body_top: Cell::new(0),
            body_height: Cell::new(0),
            nerd_font,
            show_hidden: false,
            error: None,
            notice: watch_notice,
            notice_at,
            pending: None,
            input: Vec::new(),
            cursor: 0,
            _watcher: watcher,
            watch_rx,
            watch_dirty: None,
            config,
        };
        view.rebuild();
        view
    }

    fn set_notice(&mut self, message: impl Into<String>, error: bool) {
        self.notice = Some((message.into(), error));
        self.notice_at = Some(Instant::now());
    }

    fn clear_notice(&mut self) {
        self.notice = None;
        self.notice_at = None;
    }

    fn expire_notice(&mut self) {
        let Some((_, error)) = self.notice.as_ref() else {
            self.notice_at = None;
            return;
        };
        let ttl = if *error {
            NOTICE_ERROR_TTL
        } else {
            NOTICE_SUCCESS_TTL
        };
        if self.notice_at.is_some_and(|shown| shown.elapsed() >= ttl) {
            self.clear_notice();
        }
    }

    fn rebuild(&mut self) {
        let old_index = self.selected;
        let selected_path = self.rows.get(self.selected).map(|row| row.path.clone());
        self.rows.clear();
        self.error = None;

        match self.collect_children(&self.root, 0) {
            Ok(children) => self.rows.extend(children),
            Err(e) => self.error = Some(e),
        }

        self.selected = restored_index(&self.rows, selected_path.as_deref(), old_index);
        self.ensure_visible();
    }

    fn collect_children(&self, dir: &Path, depth: usize) -> Result<Vec<Entry>, String> {
        if !self.expanded.contains(dir) {
            return Ok(Vec::new());
        }

        let mut entries: Vec<(PathBuf, String, bool)> = fs::read_dir(dir)
            .map_err(|e| format!("read {}: {e}", dir.display()))?
            .filter_map(|res| res.ok())
            .filter(|e| visible_name(&e.file_name().to_string_lossy(), self.show_hidden))
            .map(|e| {
                let path = e.path();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let name = e.file_name().to_string_lossy().into_owned();
                (path, name, is_dir)
            })
            .collect();

        entries.sort_by(|a, b| match (a.2, b.2) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.1.to_lowercase().cmp(&b.1.to_lowercase()),
        });

        let mut out = Vec::new();
        for (path, name, is_dir) in entries {
            out.push(Entry {
                path: path.clone(),
                name,
                is_dir,
                depth,
            });
            if is_dir && self.expanded.contains(&path) {
                if let Ok(mut nested) = self.collect_children(&path, depth + 1) {
                    out.append(&mut nested);
                }
            }
        }
        Ok(out)
    }

    fn ensure_visible(&mut self) {
        let h = self.body_height.get().max(1) as usize;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + h {
            self.scroll = self.selected + 1 - h;
        }
        let max_scroll = self.rows.len().saturating_sub(h);
        self.scroll = self.scroll.min(max_scroll);
    }

    fn move_sel(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        self.selected = (self.selected as isize + delta).clamp(0, n - 1) as usize;
        self.ensure_visible();
    }

    fn move_page(&mut self, pages: isize) {
        let page = self.body_height.get().max(2).saturating_sub(1) as isize;
        self.move_sel(page.saturating_mul(pages));
    }

    /// Right/l: expand a closed directory; on an open directory enter its
    /// first child; on a file open it.
    fn expand_or_child(&mut self) -> KeyOutcome {
        let Some(entry) = self.rows.get(self.selected).cloned() else {
            return KeyOutcome::Handled;
        };
        if !entry.is_dir {
            return self.run_open(&entry.path);
        }
        if !self.expanded.contains(&entry.path) {
            self.expanded.insert(entry.path);
            self.rebuild();
            return KeyOutcome::Handled;
        }
        if self
            .rows
            .get(self.selected + 1)
            .is_some_and(|next| next.depth == entry.depth + 1)
        {
            self.selected += 1;
            self.ensure_visible();
        }
        KeyOutcome::Handled
    }

    fn collapse_all(&mut self) {
        self.expanded.clear();
        // The root row stays useful as the tree's permanent disclosure anchor.
        self.expanded.insert(self.root.clone());
        self.rebuild();
    }

    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.rebuild();
    }

    fn select_path(&mut self, path: &Path) {
        if let Some(index) = self.rows.iter().position(|entry| entry.path == path) {
            self.selected = index;
            self.ensure_visible();
        }
    }

    fn start_create(&mut self) -> KeyOutcome {
        let parent = self.rows.get(self.selected).map_or_else(
            || self.root.clone(),
            |entry| {
                if entry.is_dir {
                    entry.path.clone()
                } else {
                    entry
                        .path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| self.root.clone())
                }
            },
        );
        self.expanded.insert(parent.clone());
        self.pending = Some(PendingEdit::Create { parent });
        self.input.clear();
        self.cursor = 0;
        self.clear_notice();
        KeyOutcome::Handled
    }

    fn start_rename(&mut self) -> KeyOutcome {
        let Some(entry) = self.rows.get(self.selected) else {
            return KeyOutcome::Handled;
        };
        if entry.path == self.root {
            self.set_notice("cannot rename workspace root", true);
            return KeyOutcome::Handled;
        }
        self.input = entry.name.chars().collect();
        self.cursor = self.input.len();
        self.pending = Some(PendingEdit::Rename {
            from: entry.path.clone(),
        });
        self.clear_notice();
        KeyOutcome::Handled
    }

    fn start_delete(&mut self) -> KeyOutcome {
        let Some(entry) = self.rows.get(self.selected) else {
            return KeyOutcome::Handled;
        };
        if entry.path == self.root {
            self.set_notice("cannot delete workspace root", true);
            return KeyOutcome::Handled;
        }
        self.pending = Some(PendingEdit::Delete {
            path: entry.path.clone(),
        });
        self.input.clear();
        self.cursor = 0;
        self.clear_notice();
        KeyOutcome::Handled
    }

    fn cancel_pending(&mut self) -> KeyOutcome {
        self.pending = None;
        self.input.clear();
        self.cursor = 0;
        KeyOutcome::Handled
    }

    fn submit_edit(&mut self) -> KeyOutcome {
        let Some(pending) = self.pending.clone() else {
            return KeyOutcome::Handled;
        };
        let input: String = self.input.iter().collect();
        let result = match pending {
            PendingEdit::Create { parent } => {
                create_target(&parent, &input).and_then(|(target, is_dir)| {
                    if target.exists() {
                        return Err(format!("{} already exists", target.display()));
                    }
                    if is_dir {
                        fs::create_dir_all(&target)
                            .map_err(|error| format!("create {}: {error}", target.display()))?;
                    } else {
                        if let Some(parent) = target.parent() {
                            fs::create_dir_all(parent)
                                .map_err(|error| format!("create {}: {error}", parent.display()))?;
                        }
                        OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&target)
                            .map_err(|error| format!("create {}: {error}", target.display()))?;
                    }
                    Ok((target, format!("created {}", input.trim())))
                })
            }
            PendingEdit::Rename { from } => rename_target(&from, &input).and_then(|target| {
                if target != from && target.exists() {
                    return Err(format!("{} already exists", target.display()));
                }
                fs::rename(&from, &target)
                    .map_err(|error| format!("rename {}: {error}", from.display()))?;
                self.expanded = self
                    .expanded
                    .drain()
                    .map(|path| match path.strip_prefix(&from) {
                        Ok(suffix) => target.join(suffix),
                        Err(_) => path,
                    })
                    .collect();
                Ok((target, format!("renamed to {}", input.trim())))
            }),
            PendingEdit::Delete { .. } => return KeyOutcome::Handled,
        };
        match result {
            Ok((target, message)) => {
                self.pending = None;
                self.input.clear();
                self.cursor = 0;
                self.set_notice(message, false);
                self.rebuild();
                self.select_path(&target);
            }
            Err(error) => self.set_notice(error, true),
        }
        KeyOutcome::Handled
    }

    fn confirm_delete(&mut self) -> KeyOutcome {
        let Some(PendingEdit::Delete { path }) = self.pending.clone() else {
            return KeyOutcome::Handled;
        };
        match trash_path(&path) {
            Ok(()) => {
                self.expanded
                    .retain(|expanded| !expanded.starts_with(&path));
                self.pending = None;
                self.set_notice(format!("trashed {}", path.display()), false);
                self.rebuild();
            }
            Err(error) => self.set_notice(format!("trash {}: {error}", path.display()), true),
        }
        KeyOutcome::Handled
    }

    fn on_pending_key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
        let action = config::key_token(code, mods)
            .and_then(|token| self.config.action_for_feature_key("explorer", &token))
            .map(str::to_string);
        if matches!(self.pending, Some(PendingEdit::Delete { .. })) {
            // Enter (toggle/open) confirms; Esc (scm-cancel) aborts. No y/n.
            return match action.as_deref() {
                Some(
                    config::internal::TOGGLE
                    | config::internal::OPEN
                    | config::internal::SCM_OPEN_DIFF,
                ) => self.confirm_delete(),
                Some(config::internal::SCM_CANCEL) => self.cancel_pending(),
                _ => KeyOutcome::Handled,
            };
        }
        match action.as_deref() {
            Some(config::internal::TOGGLE) => return self.submit_edit(),
            Some(config::internal::SCM_CANCEL) => return self.cancel_pending(),
            Some(config::internal::COLLAPSE) if !matches!(code, KeyCode::Char(_)) => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            Some(config::internal::EXPAND) if !matches!(code, KeyCode::Char(_)) => {
                self.cursor = (self.cursor + 1).min(self.input.len());
            }
            Some(config::internal::EDIT_BACKSPACE) if self.cursor > 0 => {
                self.cursor -= 1;
                self.input.remove(self.cursor);
            }
            Some(config::internal::EDIT_DELETE) if self.cursor < self.input.len() => {
                self.input.remove(self.cursor);
            }
            Some(config::internal::EDIT_HOME) => self.cursor = 0,
            Some(config::internal::EDIT_END) => self.cursor = self.input.len(),
            _ => {
                if let KeyCode::Char(ch) = code {
                    if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                        self.input.insert(self.cursor, ch);
                        self.cursor += 1;
                    }
                }
            }
        }
        KeyOutcome::Handled
    }

    fn toggle_or_open(&mut self) -> KeyOutcome {
        let Some(entry) = self.rows.get(self.selected).cloned() else {
            return KeyOutcome::Handled;
        };
        if entry.is_dir {
            if self.expanded.contains(&entry.path) {
                self.expanded.remove(&entry.path);
            } else {
                self.expanded.insert(entry.path);
            }
            self.rebuild();
            KeyOutcome::Handled
        } else {
            self.run_open(&entry.path)
        }
    }

    fn run_open(&mut self, path: &Path) -> KeyOutcome {
        // Shell actions may take the TTY; shell suspends the terminal first.
        KeyOutcome::Shell {
            action: config::internal::OPEN.into(),
            file: Some(path.to_path_buf()),
            env: Vec::new(),
        }
    }

    /// Left/h: collapse an open directory; otherwise select its parent without
    /// also collapsing that parent (the familiar editor-tree behavior).
    fn collapse_or_parent(&mut self) {
        let Some(entry) = self.rows.get(self.selected).cloned() else {
            return;
        };
        if entry.is_dir && self.expanded.contains(&entry.path) && entry.path != self.root {
            self.expanded.remove(&entry.path);
            self.rebuild();
            return;
        }
        if entry.depth == 0 {
            return;
        }
        let Some(parent) = entry.path.parent() else {
            return;
        };
        if let Some(idx) = self.rows.iter().position(|row| row.path == parent) {
            self.selected = idx;
            self.ensure_visible();
        }
    }

    fn glyph_for(&self, entry: &Entry) -> ui::icons::FileGlyph {
        if entry.is_dir {
            ui::icons::dir_glyph(self.expanded.contains(&entry.path), self.nerd_font)
        } else {
            ui::icons::file_glyph(&entry.path, self.nerd_font)
        }
    }

    fn row_at_mouse(&self, row: u16) -> Option<usize> {
        let top = self.body_top.get();
        let height = self.body_height.get();
        if height == 0 || row < top || row >= top.saturating_add(height) {
            return None;
        }
        let idx = self.scroll + usize::from(row - top);
        (idx < self.rows.len()).then_some(idx)
    }

    fn dispatch_action(&mut self, action: &str) -> KeyOutcome {
        match action {
            a if a == config::internal::UP => {
                self.move_sel(-1);
                KeyOutcome::Handled
            }
            a if a == config::internal::DOWN => {
                self.move_sel(1);
                KeyOutcome::Handled
            }
            a if a == config::internal::TOP => {
                self.selected = 0;
                self.ensure_visible();
                KeyOutcome::Handled
            }
            a if a == config::internal::BOTTOM => {
                self.selected = self.rows.len().saturating_sub(1);
                self.ensure_visible();
                KeyOutcome::Handled
            }
            a if a == config::internal::PAGE_UP => {
                self.move_page(-1);
                KeyOutcome::Handled
            }
            a if a == config::internal::PAGE_DOWN => {
                self.move_page(1);
                KeyOutcome::Handled
            }
            a if a == config::internal::TOGGLE => self.toggle_or_open(),
            a if a == config::internal::EXPAND => self.expand_or_child(),
            a if a == config::internal::COLLAPSE => {
                self.collapse_or_parent();
                KeyOutcome::Handled
            }
            a if a == config::internal::COLLAPSE_ALL => {
                self.collapse_all();
                KeyOutcome::Handled
            }
            a if a == config::internal::TOGGLE_HIDDEN => {
                self.toggle_hidden();
                KeyOutcome::Handled
            }
            a if a == config::internal::REFRESH => {
                self.rebuild();
                KeyOutcome::Handled
            }
            a if a == config::internal::EXPLORER_CREATE => self.start_create(),
            a if a == config::internal::EXPLORER_DELETE => self.start_delete(),
            a if a == config::internal::EXPLORER_RENAME => self.start_rename(),
            a if a == config::internal::SCM_TOGGLE_STAGE
                || a == config::internal::SCM_STAGE_ALL
                || a == config::internal::SCM_UNSTAGE_ALL
                || a == config::internal::SCM_OPEN_DIFF
                || a == config::internal::SCM_FOCUS_MESSAGE
                || a == config::internal::SCM_COMMIT
                || a == config::internal::SCM_DISCARD
                || a == config::internal::SCM_CONFIRM
                || a == config::internal::SCM_CANCEL
                || a == config::internal::SCM_SYNC
                || a == config::internal::SCM_SUGGEST_MESSAGE
                || a == config::internal::EDIT_BACKSPACE
                || a == config::internal::EDIT_DELETE
                || a == config::internal::EDIT_HOME
                || a == config::internal::EDIT_END =>
            {
                // View-specific internal actions are inert outside SCM.
                KeyOutcome::Handled
            }
            a if a == config::internal::OPEN => {
                let Some(entry) = self.rows.get(self.selected) else {
                    return KeyOutcome::Handled;
                };
                if entry.is_dir {
                    self.toggle_or_open()
                } else {
                    self.run_open(&entry.path.clone())
                }
            }
            // Custom shell function from config.sh
            other => {
                let file = self.rows.get(self.selected).map(|e| e.path.clone());
                KeyOutcome::Shell {
                    action: other.to_string(),
                    file,
                    env: Vec::new(),
                }
            }
        }
    }
}

impl FeatureView for ExplorerView {
    fn draw(&self, frame: &mut Frame, area: Rect, palette: &Palette) {
        if area.height == 0 {
            return;
        }
        let has_footer = self.pending.is_some() || self.notice.is_some();
        let tree_top = area.y.saturating_add(1);
        let tree_height = area
            .height
            .saturating_sub(1)
            .saturating_sub(u16::from(has_footer));
        self.body_top.set(tree_top);
        self.body_height.set(tree_height);

        let workspace = self
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.root.display().to_string());
        frame.render_widget(
            Paragraph::new(format!(" ▾ {}", workspace.to_uppercase())).style(
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(area.x, area.y, area.width, 1),
        );

        let tree_area = Rect::new(area.x, tree_top, area.width, tree_height);
        if let Some(err) = &self.error {
            frame.render_widget(
                Paragraph::new(err.as_str()).style(Style::default().fg(palette.red)),
                tree_area,
            );
            return;
        }
        if self.rows.is_empty() {
            frame.render_widget(
                Paragraph::new("  (empty)").style(Style::default().fg(palette.overlay1)),
                tree_area,
            );
        }

        let h = tree_height as usize;
        for (i, entry) in self.rows.iter().skip(self.scroll).take(h).enumerate() {
            let y = tree_top.saturating_add(i as u16);
            let abs = self.scroll + i;
            let selected = abs == self.selected;
            let indent = "  ".repeat(entry.depth);
            let file_icon = self.glyph_for(entry);
            let chevron = if entry.is_dir {
                if self.expanded.contains(&entry.path) {
                    "▾ "
                } else {
                    "▸ "
                }
            } else {
                "  "
            };

            // Directories use Herdr's actual theme accent, not the semantic
            // blue slot. For Everforest this is #a7c080; changing the active
            // Arch theme changes the accent automatically.
            let name_fg = if entry.is_dir {
                palette.accent
            } else {
                palette.text
            };
            let icon_fg = if entry.is_dir {
                palette.accent
            } else {
                file_icon.color.unwrap_or(palette.text)
            };
            let name_style = if selected {
                Style::default()
                    .fg(name_fg)
                    .bg(palette.surface1)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(name_fg)
            };
            let icon_style = if selected {
                Style::default()
                    .fg(icon_fg)
                    .bg(palette.surface1)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(icon_fg)
            };
            let dim = if selected {
                Style::default().fg(palette.overlay1).bg(palette.surface1)
            } else {
                Style::default().fg(palette.overlay1)
            };
            let chevron_style = if selected {
                Style::default().fg(name_fg).bg(palette.surface1)
            } else {
                Style::default().fg(name_fg)
            };

            let line = Line::from(vec![
                Span::styled(indent, dim),
                Span::styled(chevron, chevron_style),
                Span::styled(format!("{} ", file_icon.glyph), icon_style),
                Span::styled(entry.name.as_str(), name_style),
            ]);
            let row_style = if selected {
                Style::default().bg(palette.surface1)
            } else {
                Style::default()
            };
            frame.render_widget(
                Paragraph::new(line).style(row_style),
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
            );
        }

        if has_footer {
            let footer = Rect::new(
                area.x,
                area.y.saturating_add(area.height.saturating_sub(1)),
                area.width,
                1,
            );
            let (text, error) = match &self.pending {
                Some(PendingEdit::Create { .. }) | Some(PendingEdit::Rename { .. }) => {
                    let mut input = self.input.clone();
                    input.insert(self.cursor.min(input.len()), '│');
                    let input: String = input.into_iter().collect();
                    let label = if matches!(self.pending, Some(PendingEdit::Create { .. })) {
                        " New: "
                    } else {
                        " Rename: "
                    };
                    (format!("{label}{input}"), false)
                }
                Some(PendingEdit::Delete { path }) => (
                    format!(
                        " Delete {}?",
                        path.file_name()
                            .map(|name| name.to_string_lossy())
                            .unwrap_or_else(|| path.display().to_string().into())
                    ),
                    true,
                ),
                None => self
                    .notice
                    .clone()
                    .unwrap_or_else(|| (String::new(), false)),
            };
            frame.render_widget(
                Paragraph::new(text).style(
                    Style::default()
                        .fg(if error { palette.red } else { palette.text })
                        .bg(palette.surface0),
                ),
                footer,
            );
        }
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
        if self.pending.is_some() {
            return self.on_pending_key(code, mods);
        }
        let Some(token) = config::key_token(code, mods) else {
            return KeyOutcome::Ignored;
        };
        let Some(action) = self
            .config
            .action_for_feature_key("explorer", &token)
            .map(str::to_string)
        else {
            return KeyOutcome::Ignored;
        };
        self.dispatch_action(&action)
    }

    fn captures_text_input(&self) -> bool {
        self.pending.is_some()
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> KeyOutcome {
        if self.pending.is_some() {
            return KeyOutcome::Handled;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(idx) = self.row_at_mouse(mouse.row) else {
                    return KeyOutcome::Ignored;
                };
                if self.selected == idx {
                    self.toggle_or_open()
                } else {
                    self.selected = idx;
                    self.ensure_visible();
                    KeyOutcome::Handled
                }
            }
            MouseEventKind::ScrollDown => {
                self.move_sel(3);
                KeyOutcome::Handled
            }
            MouseEventKind::ScrollUp => {
                self.move_sel(-3);
                KeyOutcome::Handled
            }
            _ => KeyOutcome::Ignored,
        }
    }

    fn on_activate(&mut self) {
        self.rebuild();
    }

    fn on_tick(&mut self) {
        loop {
            match self.watch_rx.try_recv() {
                Ok(Ok(event)) if watch_event_affects_tree(&self.root, &event) => {
                    self.watch_dirty = Some(Instant::now());
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => self.set_notice(format!("watch: {error}"), true),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        if self
            .watch_dirty
            .is_some_and(|changed| changed.elapsed() >= Duration::from_millis(75))
        {
            self.watch_dirty = None;
            self.rebuild();
        }
        self.expire_notice();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, depth: usize) -> Entry {
        Entry {
            path: PathBuf::from(path),
            name: path.to_string(),
            is_dir: true,
            depth,
        }
    }

    #[test]
    fn hidden_filter_never_shows_git() {
        assert!(!visible_name(".env", false));
        assert!(visible_name(".env", true));
        assert!(!visible_name(".git", false));
        assert!(!visible_name(".git", true));
        assert!(visible_name("src", false));
    }

    #[test]
    fn create_paths_are_relative_and_trailing_slash_means_directory() {
        assert_eq!(
            create_target(Path::new("/repo"), "src/new.rs").unwrap(),
            (PathBuf::from("/repo/src/new.rs"), false)
        );
        assert_eq!(
            create_target(Path::new("/repo"), "assets/").unwrap(),
            (PathBuf::from("/repo/assets"), true)
        );
        assert!(create_target(Path::new("/repo"), "../outside").is_err());
        assert!(create_target(Path::new("/repo"), "/outside").is_err());
    }

    #[test]
    fn create_rename_delete_and_external_watch_update_the_tree() {
        let root = std::env::temp_dir().join(format!(
            "corral-explorer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut view = ExplorerView::new(root.clone(), false, Arc::new(Config::for_test()));
        assert!(view.rows.is_empty()); // workspace root is a header, never a tree row
        view.set_notice("created", false);
        view.notice_at = Some(Instant::now() - Duration::from_secs(3));
        view.expire_notice();
        assert!(view.notice.is_none());
        view.set_notice("failed", true);
        view.notice_at = Some(Instant::now() - Duration::from_secs(3));
        view.expire_notice();
        assert!(view.notice.is_some());
        view.notice_at = Some(Instant::now() - Duration::from_secs(5));
        view.expire_notice();
        assert!(view.notice.is_none());

        view.start_create();
        view.on_pending_key(KeyCode::Char('h'), KeyModifiers::NONE);
        view.on_pending_key(KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(view.input.iter().collect::<String>(), "hl");
        view.cancel_pending();

        view.start_create();
        view.input = "created.txt".chars().collect();
        view.cursor = view.input.len();
        view.submit_edit();
        let created = root.join("created.txt");
        assert!(created.is_file());
        assert!(view
            .rows
            .iter()
            .any(|entry| entry.path == created && entry.depth == 0));
        assert!(!view.rows.iter().any(|entry| entry.path == root));

        view.select_path(&created);
        view.start_rename();
        view.input = "renamed.txt".chars().collect();
        view.cursor = view.input.len();
        view.submit_edit();
        let renamed = root.join("renamed.txt");
        assert!(!created.exists());
        assert!(renamed.is_file());

        view.select_path(&renamed);
        view.start_delete();
        view.confirm_delete();
        // Prefer trash (gio); if the host has no trash backend in CI, the
        // path may still exist and the notice carries the error — either way
        // Explorer must not hard-rm.
        if renamed.exists() {
            assert!(
                view.notice
                    .as_ref()
                    .is_some_and(|(msg, error)| *error && msg.contains("trash")),
                "expected trash notice when path remains, got {:?}",
                view.notice
            );
            fs::remove_file(&renamed).unwrap();
        }

        let external = root.join("external.txt");
        fs::write(&external, "outside\n").unwrap();
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(25));
            view.on_tick();
            if view.rows.iter().any(|entry| entry.path == external) {
                break;
            }
        }
        assert!(view.rows.iter().any(|entry| entry.path == external));

        drop(view);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restore_prefers_exact_path_then_visible_ancestor() {
        let rows = vec![
            row("/repo", 0),
            row("/repo/src", 1),
            row("/repo/src/a.rs", 2),
        ];
        assert_eq!(
            restored_index(&rows, Some(Path::new("/repo/src/a.rs")), 0),
            2
        );
        assert_eq!(
            restored_index(&rows[..2], Some(Path::new("/repo/src/a.rs")), 2),
            1
        );
        assert_eq!(
            restored_index(&rows[..1], Some(Path::new("/repo/.env")), 3),
            0
        );
    }
}
