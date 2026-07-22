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
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    depth: usize,
}

fn visible_name(name: &str, show_hidden: bool) -> bool {
    name != ".git" && (show_hidden || !name.starts_with('.'))
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
    config: Arc<Config>,
}

impl ExplorerView {
    pub fn new(root: PathBuf, nerd_font: bool, config: Arc<Config>) -> Self {
        let root = root.canonicalize().unwrap_or(root);
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
            config,
        };
        view.rebuild();
        view
    }

    fn rebuild(&mut self) {
        let old_index = self.selected;
        let selected_path = self.rows.get(self.selected).map(|row| row.path.clone());
        self.rows.clear();
        self.error = None;

        let name = self
            .root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.root.display().to_string());

        self.rows.push(Entry {
            path: self.root.clone(),
            name,
            is_dir: true,
            depth: 0,
        });

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
                depth: depth + 1,
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
        self.body_top.set(area.y);
        self.body_height.set(area.height);

        if area.height == 0 {
            return;
        }

        if let Some(err) = &self.error {
            frame.render_widget(
                Paragraph::new(err.as_str()).style(Style::default().fg(palette.red)),
                area,
            );
            return;
        }
        if self.rows.is_empty() {
            frame.render_widget(
                Paragraph::new("  (empty)").style(Style::default().fg(palette.overlay1)),
                area,
            );
            return;
        }

        let h = area.height as usize;
        for (i, entry) in self.rows.iter().skip(self.scroll).take(h).enumerate() {
            let y = area.y.saturating_add(i as u16);
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
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
        let Some(token) = config::key_token(code, mods) else {
            return KeyOutcome::Ignored;
        };
        let Some(action) = self.config.action_for_key(&token).map(str::to_string) else {
            return KeyOutcome::Ignored;
        };
        self.dispatch_action(&action)
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> KeyOutcome {
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
