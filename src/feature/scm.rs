//! Source Control: branch header + staged / unstaged change lists.
//!
//! Reads and mutates the index in-process (git.rs) for instant refresh;
//! `enter` on a file opens a diff and `c` commits — both via `config.sh`
//! shell functions (they need the reused pane / `$EDITOR`).
//!
//! Every key comes from [`crate::config`], including SCM-specific stage,
//! unstage, commit, and diff actions. The view only dispatches action names;
//! it owns no default keyboard policy.

use super::view::{FeatureView, KeyOutcome};
use crate::config::{self, Config};
use crate::git::{FileEntry, Git, Status};
use crate::ui::{self, Palette};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const AUTO_REFRESH: Duration = Duration::from_millis(1500);
const NOTICE_SUCCESS_TTL: Duration = Duration::from_secs(2);
const NOTICE_ERROR_TTL: Duration = Duration::from_secs(4);

struct SuggestionCompletion {
    root: PathBuf,
    result: Result<String, String>,
}

fn clean_suggestion(output: &str) -> Result<String, String> {
    let candidate = output
        .lines()
        .map(str::trim)
        .rfind(|line| {
            !line.is_empty()
                && !line.starts_with("```")
                && !line.starts_with("CORRAL_")
                && !line.to_ascii_lowercase().starts_with("warning:")
        })
        .ok_or_else(|| "suggestion command returned no message".to_string())?;
    let message = candidate
        .trim_matches(|ch| matches!(ch, '\'' | '"' | '`'))
        .trim_end_matches('.')
        .trim();
    if message.is_empty() {
        return Err("suggestion command returned no message".into());
    }
    if message.chars().count() > 200 {
        return Err("suggested message is longer than 200 characters".into());
    }
    Ok(message.to_string())
}

fn diff_action(staged: bool, letter: char) -> &'static str {
    if staged {
        "diff_staged"
    } else if letter == 'U' {
        "diff_untracked"
    } else {
        "diff"
    }
}

fn status_columns(rect: Rect) -> (Rect, Rect) {
    let status_width = rect.width.min(3);
    let name = Rect {
        width: rect.width.saturating_sub(status_width),
        ..rect
    };
    let status = Rect {
        x: rect
            .x
            .saturating_add(rect.width.saturating_sub(status_width)),
        width: status_width,
        ..rect
    };
    (name, status)
}

fn header_columns(rect: Rect, count: usize) -> (Rect, Rect) {
    let badge_width = ((count.max(1).ilog10() + 1) as u16 + 2).min(rect.width);
    let title = Rect {
        width: rect.width.saturating_sub(badge_width),
        ..rect
    };
    let badge = Rect {
        x: rect
            .x
            .saturating_add(rect.width.saturating_sub(badge_width)),
        width: badge_width,
        ..rect
    };
    (title, badge)
}

fn display_path_parts(path: &str) -> (&str, Option<&str>) {
    match path.rsplit_once('/') {
        Some((parent, name)) if !parent.is_empty() => (name, Some(parent)),
        _ => (path, None),
    }
}

fn log_drawer_item(line: String) -> DrawerItem {
    match line.split_once('\t') {
        Some((reference, subject)) => DrawerItem {
            display: format!("{reference} {subject}"),
            reference: Some(reference.to_string()),
        },
        None => DrawerItem {
            display: line,
            reference: None,
        },
    }
}

fn graph_drawer_item(line: String) -> DrawerItem {
    let reference = line
        .split_whitespace()
        .find(|word| word.len() >= 7 && word.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_string);
    DrawerItem {
        display: line,
        reference,
    }
}

fn branch_drawer_item(line: String) -> DrawerItem {
    match line.split_once('\t') {
        Some((head, name)) if !name.is_empty() => DrawerItem {
            display: format!("{}{}", if head == "*" { "* " } else { "  " }, name),
            reference: Some(name.to_string()),
        },
        _ => DrawerItem {
            display: line,
            reference: None,
        },
    }
}

fn worktree_drawer_item(line: String) -> DrawerItem {
    match line.split_once('\t') {
        Some((path, label)) => {
            let name = Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .unwrap_or(path);
            DrawerItem {
                display: format!("{name}  {label}"),
                reference: Some(path.to_string()),
            }
        }
        _ => DrawerItem {
            display: line,
            reference: None,
        },
    }
}

fn short_remote_url(url: &str) -> &str {
    let url = url.strip_suffix(".git").unwrap_or(url);
    if let Some(rest) = url.strip_prefix("file://") {
        return Path::new(rest)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(rest);
    }
    if let Some((_, rest)) = url.split_once("://") {
        return rest.split_once('/').map_or(rest, |(_, path)| path);
    }
    if url
        .split_once(':')
        .is_some_and(|(host, _)| host.contains('@'))
    {
        return url.split_once(':').map_or(url, |(_, path)| path);
    }
    Path::new(url)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(url)
}

fn remote_drawer_item(line: String) -> DrawerItem {
    match line.split_once('\t') {
        Some((name, url)) => DrawerItem {
            display: format!("{name}  {}", short_remote_url(url)),
            reference: None,
        },
        _ => DrawerItem {
            display: line,
            reference: None,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Staged,
    Changes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drawer {
    Graph,
    Commits,
    FileHistory,
    Branches,
    Worktrees,
    Remotes,
    Stashes,
    Tags,
}

impl Drawer {
    const ALL: [Drawer; 8] = [
        Drawer::Graph,
        Drawer::Commits,
        Drawer::FileHistory,
        Drawer::Branches,
        Drawer::Worktrees,
        Drawer::Remotes,
        Drawer::Stashes,
        Drawer::Tags,
    ];

    fn title(self) -> &'static str {
        match self {
            Drawer::Graph => "Graph",
            Drawer::Commits => "Commits",
            Drawer::FileHistory => "File History",
            Drawer::Branches => "Branches",
            Drawer::Worktrees => "Worktrees",
            Drawer::Remotes => "Remotes",
            Drawer::Stashes => "Stashes",
            Drawer::Tags => "Tags",
        }
    }

    fn index(self) -> usize {
        match self {
            Drawer::Graph => 0,
            Drawer::Commits => 1,
            Drawer::FileHistory => 2,
            Drawer::Branches => 3,
            Drawer::Worktrees => 4,
            Drawer::Remotes => 5,
            Drawer::Stashes => 6,
            Drawer::Tags => 7,
        }
    }
}

#[derive(Clone, Debug)]
struct DrawerItem {
    display: String,
    reference: Option<String>,
}

/// A rendered row. Headers and files are both real selectable state nodes.
enum Row {
    Header {
        section: Section,
        title: String,
        count: usize,
        collapsed: bool,
    },
    File {
        staged: bool,
        index: usize,
    },
    DrawerHeader {
        drawer: Drawer,
        collapsed: bool,
    },
    DrawerLine {
        drawer: Drawer,
        index: usize,
    },
}

pub struct ScmView {
    cwd: PathBuf,
    git: Option<Git>,
    status: Status,
    rows: Vec<Row>,
    /// Indices into `rows` that can receive selection (headers and files).
    selectable: Vec<usize>,
    /// Index into `selectable`.
    selected: usize,
    scroll: usize,
    body_top: Cell<u16>,
    body_height: Cell<u16>,
    nerd_font: bool,
    error: Option<String>,
    flash: Option<(String, bool)>,
    flash_at: Option<Instant>,
    staged_collapsed: bool,
    changes_collapsed: bool,
    drawer_expanded: [bool; 8],
    drawer_lines: [Vec<DrawerItem>; 8],
    last_file_path: Option<String>,
    message: Vec<char>,
    cursor: usize,
    message_focused: bool,
    message_rect: Cell<Rect>,
    suggest_rect: Cell<Rect>,
    commit_rect: Cell<Rect>,
    pending_discard: Option<FileEntry>,
    syncing: Option<Receiver<Result<String, String>>>,
    suggesting: Option<Receiver<SuggestionCompletion>>,
    last_refresh: Instant,
    config: Arc<Config>,
}

impl ScmView {
    pub fn new(cwd: PathBuf, nerd_font: bool, config: Arc<Config>) -> Self {
        let mut view = Self {
            cwd,
            git: None,
            status: Status::default(),
            rows: Vec::new(),
            selectable: Vec::new(),
            selected: 0,
            scroll: 0,
            body_top: Cell::new(0),
            body_height: Cell::new(0),
            nerd_font,
            error: None,
            flash: None,
            flash_at: None,
            staged_collapsed: false,
            changes_collapsed: false,
            drawer_expanded: [false; 8],
            drawer_lines: std::array::from_fn(|_| Vec::new()),
            last_file_path: None,
            message: Vec::new(),
            cursor: 0,
            message_focused: false,
            message_rect: Cell::new(Rect::default()),
            suggest_rect: Cell::new(Rect::default()),
            commit_rect: Cell::new(Rect::default()),
            pending_discard: None,
            syncing: None,
            suggesting: None,
            last_refresh: Instant::now(),
            config,
        };
        view.refresh();
        view
    }

    fn set_flash(&mut self, message: impl Into<String>, error: bool) {
        self.flash = Some((message.into(), error));
        self.flash_at = Some(Instant::now());
    }

    fn set_progress(&mut self, message: impl Into<String>) {
        self.flash = Some((message.into(), false));
        self.flash_at = None;
    }

    fn expire_flash(&mut self) {
        let Some((_, error)) = self.flash.as_ref() else {
            self.flash_at = None;
            return;
        };
        let ttl = if *error {
            NOTICE_ERROR_TTL
        } else {
            NOTICE_SUCCESS_TTL
        };
        if self.flash_at.is_some_and(|shown| shown.elapsed() >= ttl) {
            self.flash = None;
            self.flash_at = None;
        }
    }

    fn refresh(&mut self) {
        let selected = self
            .selected_entry()
            .map(|(staged, entry)| (staged, entry.path.clone()));
        self.error = None;
        // Re-discover each refresh: the cwd's repo can change under us.
        self.git = match Git::discover(&self.cwd) {
            Ok(git) => Some(git),
            Err(e) => {
                self.error = Some(e);
                None
            }
        };
        if let Some(git) = &self.git {
            match git.status() {
                Ok(status) => self.status = status,
                Err(e) => {
                    self.error = Some(e);
                    self.status = Status::default();
                }
            }
        } else {
            self.status = Status::default();
        }
        for drawer in Drawer::ALL {
            if self.drawer_expanded[drawer.index()] {
                self.load_drawer(drawer);
            }
        }
        self.rebuild_rows();
        if let Some((staged, path)) = selected {
            let exact = self.selectable.iter().position(|&row_idx| {
                matches!(
                    self.rows.get(row_idx),
                    Some(Row::File { staged: s, index })
                        if *s == staged
                            && if *s {
                                self.status.staged.get(*index)
                            } else {
                                self.status.unstaged.get(*index)
                            }
                            .is_some_and(|entry| entry.path == path)
                )
            });
            let same_path = exact.or_else(|| {
                self.selectable
                    .iter()
                    .position(|&row_idx| match self.rows.get(row_idx) {
                        Some(Row::File { staged, index }) => {
                            let list = if *staged {
                                &self.status.staged
                            } else {
                                &self.status.unstaged
                            };
                            list.get(*index).is_some_and(|entry| entry.path == path)
                        }
                        _ => false,
                    })
            });
            if let Some(i) = same_path {
                self.selected = i;
            }
        }
        self.ensure_visible();
        self.last_refresh = Instant::now();
    }

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        self.selectable.clear();

        if !self.status.staged.is_empty() {
            self.selectable.push(self.rows.len());
            self.rows.push(Row::Header {
                section: Section::Staged,
                title: "Staged Changes".into(),
                count: self.status.staged.len(),
                collapsed: self.staged_collapsed,
            });
            if !self.staged_collapsed {
                for i in 0..self.status.staged.len() {
                    self.selectable.push(self.rows.len());
                    self.rows.push(Row::File {
                        staged: true,
                        index: i,
                    });
                }
            }
        }
        if !self.status.unstaged.is_empty() {
            self.selectable.push(self.rows.len());
            self.rows.push(Row::Header {
                section: Section::Changes,
                title: "Changes".into(),
                count: self.status.unstaged.len(),
                collapsed: self.changes_collapsed,
            });
            if !self.changes_collapsed {
                for i in 0..self.status.unstaged.len() {
                    self.selectable.push(self.rows.len());
                    self.rows.push(Row::File {
                        staged: false,
                        index: i,
                    });
                }
            }
        }

        for drawer in Drawer::ALL {
            self.selectable.push(self.rows.len());
            let expanded = self.drawer_expanded[drawer.index()];
            self.rows.push(Row::DrawerHeader {
                drawer,
                collapsed: !expanded,
            });
            if expanded {
                for index in 0..self.drawer_lines[drawer.index()].len() {
                    self.selectable.push(self.rows.len());
                    self.rows.push(Row::DrawerLine { drawer, index });
                }
            }
        }

        if self.selected >= self.selectable.len() {
            self.selected = self.selectable.len().saturating_sub(1);
        }
        self.ensure_visible();
    }

    fn selected_row(&self) -> Option<&Row> {
        let row_idx = *self.selectable.get(self.selected)?;
        self.rows.get(row_idx)
    }

    /// The file entry currently selected, plus whether it is staged.
    fn selected_entry(&self) -> Option<(bool, &FileEntry)> {
        match self.selected_row()? {
            Row::File { staged, index } => {
                let list = if *staged {
                    &self.status.staged
                } else {
                    &self.status.unstaged
                };
                list.get(*index).map(|e| (*staged, e))
            }
            Row::Header { .. } | Row::DrawerHeader { .. } | Row::DrawerLine { .. } => None,
        }
    }

    fn selected_section(&self) -> Option<Section> {
        match self.selected_row()? {
            Row::Header { section, .. } => Some(*section),
            Row::File { staged: true, .. } => Some(Section::Staged),
            Row::File { staged: false, .. } => Some(Section::Changes),
            Row::DrawerHeader { .. } | Row::DrawerLine { .. } => None,
        }
    }

    fn selected_drawer(&self) -> Option<Drawer> {
        match self.selected_row()? {
            Row::DrawerHeader { drawer, .. } | Row::DrawerLine { drawer, .. } => Some(*drawer),
            Row::Header { .. } | Row::File { .. } => None,
        }
    }

    fn selected_drawer_item(&self) -> Option<(Drawer, &DrawerItem)> {
        let Row::DrawerLine { drawer, index } = self.selected_row()? else {
            return None;
        };
        self.drawer_lines[drawer.index()]
            .get(*index)
            .map(|item| (*drawer, item))
    }

    fn select_header(&mut self, section: Section) {
        if let Some(index) = self.selectable.iter().position(|&row_idx| {
            matches!(
                self.rows.get(row_idx),
                Some(Row::Header { section: candidate, .. }) if *candidate == section
            )
        }) {
            self.selected = index;
            self.ensure_visible();
        }
    }

    fn select_drawer_header(&mut self, drawer: Drawer) {
        if let Some(index) = self.selectable.iter().position(|&row_idx| {
            matches!(
                self.rows.get(row_idx),
                Some(Row::DrawerHeader { drawer: candidate, .. }) if *candidate == drawer
            )
        }) {
            self.selected = index;
            self.ensure_visible();
        }
    }

    /// The screen line (in body space) of the selected file, for scroll math.
    fn selected_line(&self) -> usize {
        self.selectable.get(self.selected).copied().unwrap_or(0)
    }

    fn ensure_visible(&mut self) {
        let h = self.body_height.get().max(1) as usize;
        let line = self.selected_line();
        if line < self.scroll {
            self.scroll = line;
        } else if line >= self.scroll + h {
            self.scroll = line + 1 - h;
        }
        let max_scroll = self.rows.len().saturating_sub(h);
        self.scroll = self.scroll.min(max_scroll);
    }

    fn remember_selected_file(&mut self) {
        let path = self.selected_entry().map(|(_, entry)| entry.path.clone());
        let Some(path) = path else {
            return;
        };
        if self.last_file_path.as_deref() == Some(path.as_str()) {
            return;
        }
        self.last_file_path = Some(path);
        if self.drawer_expanded[Drawer::FileHistory.index()] {
            self.load_drawer(Drawer::FileHistory);
            self.rebuild_rows();
        }
    }

    fn move_sel(&mut self, delta: isize) {
        if self.selectable.is_empty() {
            return;
        }
        let n = self.selectable.len() as isize;
        self.selected = (self.selected as isize + delta).clamp(0, n - 1) as usize;
        self.remember_selected_file();
        self.ensure_visible();
    }

    /// Stage an unstaged file, or unstage a staged one, then refresh.
    fn toggle_stage(&mut self) -> KeyOutcome {
        let Some(git) = self.git.clone() else {
            return KeyOutcome::Handled;
        };
        let action = self.selected_entry().map(|(staged, e)| (staged, e.clone()));
        if let Some((staged, entry)) = action {
            let res = if staged {
                git.unstage(&entry)
            } else {
                git.stage(&entry)
            };
            if let Err(e) = res {
                self.error = Some(e);
            } else {
                self.refresh();
            }
        }
        KeyOutcome::Handled
    }

    fn stage_all(&mut self) -> KeyOutcome {
        if let Some(git) = self.git.clone() {
            if let Err(e) = git.stage_all() {
                self.error = Some(e);
            } else {
                self.refresh();
            }
        }
        KeyOutcome::Handled
    }

    fn unstage_all(&mut self) -> KeyOutcome {
        if let Some(git) = self.git.clone() {
            if let Err(e) = git.unstage_all() {
                self.error = Some(e);
            } else {
                self.refresh();
            }
        }
        KeyOutcome::Handled
    }

    fn set_section_collapsed(&mut self, section: Section, collapsed: bool) {
        match section {
            Section::Staged => self.staged_collapsed = collapsed,
            Section::Changes => self.changes_collapsed = collapsed,
        }
        self.rebuild_rows();
        self.select_header(section);
    }

    fn load_drawer(&mut self, drawer: Drawer) {
        let Some(git) = self.git.clone() else {
            return;
        };
        let result = match drawer {
            Drawer::Graph => git.graph(20),
            Drawer::Commits => git.commits(20),
            Drawer::FileHistory => self
                .last_file_path
                .as_deref()
                .ok_or_else(|| "select a file for history".to_string())
                .and_then(|path| git.file_history(path, 20)),
            Drawer::Branches => git.branches(),
            Drawer::Worktrees => git.worktrees(),
            Drawer::Remotes => git.remotes(),
            Drawer::Stashes => git.stashes(),
            Drawer::Tags => git.tags(),
        };
        match result {
            Ok(lines) => {
                self.drawer_lines[drawer.index()] = lines
                    .into_iter()
                    .map(|line| match drawer {
                        Drawer::Graph => graph_drawer_item(line),
                        Drawer::Commits | Drawer::FileHistory => log_drawer_item(line),
                        Drawer::Branches => branch_drawer_item(line),
                        Drawer::Worktrees => worktree_drawer_item(line),
                        Drawer::Remotes => remote_drawer_item(line),
                        Drawer::Stashes | Drawer::Tags => log_drawer_item(line),
                    })
                    .collect();
            }
            Err(error) => {
                self.drawer_lines[drawer.index()].clear();
                self.set_flash(error, true);
            }
        }
    }

    fn set_drawer_expanded(&mut self, drawer: Drawer, expanded: bool) {
        if expanded {
            self.load_drawer(drawer);
        }
        self.drawer_expanded[drawer.index()] = expanded;
        self.rebuild_rows();
        self.select_drawer_header(drawer);
    }

    fn toggle_selected(&mut self) -> KeyOutcome {
        let section = self.selected_row().and_then(|row| match row {
            Row::Header {
                section, collapsed, ..
            } => Some((*section, !*collapsed)),
            _ => None,
        });
        if let Some((section, collapsed)) = section {
            self.set_section_collapsed(section, collapsed);
            return KeyOutcome::Handled;
        }
        let drawer = self.selected_row().and_then(|row| match row {
            Row::DrawerHeader { drawer, collapsed } => Some((*drawer, *collapsed)),
            _ => None,
        });
        if let Some((drawer, expanded)) = drawer {
            self.set_drawer_expanded(drawer, expanded);
            return KeyOutcome::Handled;
        }
        match self.selected_row() {
            Some(Row::File { .. }) => self.toggle_stage(),
            Some(Row::DrawerLine { .. }) => self.open_drawer_ref(),
            _ => KeyOutcome::Handled,
        }
    }

    fn collapse_selected(&mut self) -> KeyOutcome {
        if let Some(section) = self.selected_section() {
            self.set_section_collapsed(section, true);
        } else if let Some(drawer) = self.selected_drawer() {
            self.set_drawer_expanded(drawer, false);
        }
        KeyOutcome::Handled
    }

    fn expand_selected(&mut self) -> KeyOutcome {
        if let Some(section) = self.selected_section() {
            self.set_section_collapsed(section, false);
        } else if let Some(drawer) = self.selected_drawer() {
            self.set_drawer_expanded(drawer, true);
        }
        KeyOutcome::Handled
    }

    fn collapse_all(&mut self) -> KeyOutcome {
        self.staged_collapsed = true;
        self.changes_collapsed = true;
        self.drawer_expanded.fill(false);
        self.rebuild_rows();
        KeyOutcome::Handled
    }

    fn focus_message(&mut self) -> KeyOutcome {
        self.message_focused = true;
        self.cursor = self.cursor.min(self.message.len());
        KeyOutcome::Handled
    }

    fn commit_message(&self) -> KeyOutcome {
        let message: String = self.message.iter().collect::<String>().trim().to_string();
        if message.is_empty() || self.status.staged.is_empty() {
            return KeyOutcome::Handled;
        }
        KeyOutcome::Shell {
            action: "commit_message".into(),
            file: self.git.as_ref().map(|git| git.root().to_path_buf()),
            env: vec![("CORRAL_COMMIT_MESSAGE".into(), message)],
        }
    }

    fn request_discard(&mut self) -> KeyOutcome {
        let Some((staged, entry)) = self.selected_entry() else {
            return KeyOutcome::Handled;
        };
        if staged {
            self.set_flash("unstage before discarding", true);
        } else {
            self.pending_discard = Some(entry.clone());
        }
        KeyOutcome::Handled
    }

    fn confirm_discard(&mut self) -> KeyOutcome {
        let Some(entry) = self.pending_discard.take() else {
            return KeyOutcome::Handled;
        };
        let Some(git) = self.git.clone() else {
            return KeyOutcome::Handled;
        };
        match git.discard(&entry) {
            Ok(()) => {
                self.set_flash(format!("discarded {}", entry.path), false);
                self.refresh();
            }
            Err(error) => self.set_flash(error, true),
        }
        KeyOutcome::Handled
    }

    fn cancel_transient(&mut self) -> KeyOutcome {
        self.pending_discard = None;
        self.message_focused = false;
        KeyOutcome::Handled
    }

    fn suggest_message(&mut self) -> KeyOutcome {
        if self.suggesting.is_some() {
            return KeyOutcome::Handled;
        }
        let Some(git) = self.git.as_ref() else {
            return KeyOutcome::Handled;
        };
        if self.status.staged.is_empty() && self.status.unstaged.is_empty() {
            self.set_flash("no changes to describe", true);
            return KeyOutcome::Handled;
        }
        let root = git.root().to_path_buf();
        let worker_root = root.clone();
        let config = Arc::clone(&self.config);
        let env = vec![("CORRAL_GIT_ROOT".to_string(), root.display().to_string())];
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = config
                .run_shell_capture("suggest_commit_message", Some(&worker_root), &env)
                .and_then(|output| clean_suggestion(&output));
            let _ = tx.send(SuggestionCompletion {
                root: worker_root,
                result,
            });
        });
        self.suggesting = Some(rx);
        self.set_progress("✧ generating commit message…");
        KeyOutcome::Handled
    }

    fn sync_changes(&mut self) -> KeyOutcome {
        if self.syncing.is_some() {
            return KeyOutcome::Handled;
        }
        let Some(git) = self.git.clone() else {
            return KeyOutcome::Handled;
        };
        let status = self.status.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(git.sync(&status));
        });
        self.syncing = Some(rx);
        self.set_progress("syncing…");
        KeyOutcome::Handled
    }

    fn on_message_key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
        let action = config::key_token(code, mods)
            .and_then(|token| self.config.action_for_feature_key("scm", &token))
            .map(str::to_string);
        match action.as_deref() {
            Some(config::internal::TOGGLE | config::internal::SCM_COMMIT) => {
                return self.commit_message();
            }
            Some(config::internal::SCM_CANCEL) => return self.cancel_transient(),
            Some(config::internal::COLLAPSE) if !matches!(code, KeyCode::Char(_)) => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            Some(config::internal::EXPAND) if !matches!(code, KeyCode::Char(_)) => {
                self.cursor = (self.cursor + 1).min(self.message.len());
            }
            Some(config::internal::EDIT_BACKSPACE) if self.cursor > 0 => {
                self.cursor -= 1;
                self.message.remove(self.cursor);
            }
            Some(config::internal::EDIT_DELETE) if self.cursor < self.message.len() => {
                self.message.remove(self.cursor);
            }
            Some(config::internal::EDIT_HOME) => self.cursor = 0,
            Some(config::internal::EDIT_END) => self.cursor = self.message.len(),
            _ => {
                // Printable text is data, not a shortcut. All editing commands
                // around it still arrive as configured actions above.
                if let KeyCode::Char(c) = code {
                    if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                        self.message.insert(self.cursor, c);
                        self.cursor += 1;
                    }
                }
            }
        }
        KeyOutcome::Handled
    }

    /// Open the selected file's staged, working-tree, or untracked diff in the
    /// reused pane. The kind must cross the shell boundary explicitly: one path
    /// can appear in both sections (`MM`) with different patches.
    fn open_diff(&self) -> KeyOutcome {
        let Some(git) = &self.git else {
            return KeyOutcome::Handled;
        };
        let Some((staged, entry)) = self.selected_entry() else {
            return KeyOutcome::Handled;
        };
        let mut env = vec![
            ("CORRAL_GIT_ROOT".into(), git.root().display().to_string()),
            ("CORRAL_GIT_PATH".into(), entry.path.clone()),
        ];
        if let Some(orig) = &entry.orig {
            env.push(("CORRAL_GIT_ORIG".into(), orig.clone()));
        }
        KeyOutcome::Shell {
            action: diff_action(staged, entry.letter).into(),
            file: Some(git.abs_path(entry)),
            env,
        }
    }

    fn open_drawer_ref(&self) -> KeyOutcome {
        let Some(git) = &self.git else {
            return KeyOutcome::Handled;
        };
        let Some((drawer, item)) = self.selected_drawer_item() else {
            return KeyOutcome::Handled;
        };
        let Some(reference) = &item.reference else {
            return KeyOutcome::Handled;
        };
        let mut env = vec![("CORRAL_GIT_ROOT".into(), git.root().display().to_string())];
        let (action, file) = match drawer {
            Drawer::Graph | Drawer::Commits | Drawer::Branches | Drawer::Stashes | Drawer::Tags => {
                env.push(("CORRAL_GIT_REF".into(), reference.clone()));
                ("show_ref", git.root().to_path_buf())
            }
            Drawer::FileHistory => {
                env.push(("CORRAL_GIT_REF".into(), reference.clone()));
                if let Some(path) = &self.last_file_path {
                    env.push(("CORRAL_GIT_PATH".into(), path.clone()));
                }
                ("show_ref", git.root().to_path_buf())
            }
            Drawer::Worktrees => {
                env.push(("CORRAL_WORKTREE_PATH".into(), reference.clone()));
                ("open_worktree", PathBuf::from(reference))
            }
            Drawer::Remotes => return KeyOutcome::Handled,
        };
        KeyOutcome::Shell {
            action: action.into(),
            file: Some(file),
            env,
        }
    }

    fn open_selected(&self) -> KeyOutcome {
        if matches!(self.selected_row(), Some(Row::DrawerLine { .. })) {
            self.open_drawer_ref()
        } else {
            self.open_diff()
        }
    }

    fn row_at_mouse(&self, row: u16) -> Option<usize> {
        let top = self.body_top.get();
        let height = self.body_height.get();
        if height == 0 || row < top || row >= top.saturating_add(height) {
            return None;
        }
        let line = self.scroll + usize::from(row - top);
        // Map the absolute row line back to a selectable index, if it is a file.
        self.selectable.iter().position(|&r| r == line)
    }

    fn dispatch_action(&mut self, action: &str) -> KeyOutcome {
        if self.pending_discard.is_some() {
            return match action {
                config::internal::SCM_CONFIRM => self.confirm_discard(),
                config::internal::SCM_CANCEL => self.cancel_transient(),
                _ => KeyOutcome::Handled,
            };
        }
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
                self.selected = self.selectable.len().saturating_sub(1);
                self.ensure_visible();
                KeyOutcome::Handled
            }
            a if a == config::internal::PAGE_UP => {
                let page = self.body_height.get().max(2).saturating_sub(1) as isize;
                self.move_sel(-page);
                KeyOutcome::Handled
            }
            a if a == config::internal::PAGE_DOWN => {
                let page = self.body_height.get().max(2).saturating_sub(1) as isize;
                self.move_sel(page);
                KeyOutcome::Handled
            }
            a if a == config::internal::TOGGLE => self.toggle_selected(),
            a if a == config::internal::OPEN || a == config::internal::SCM_OPEN_DIFF => {
                self.open_selected()
            }
            a if a == config::internal::EXPAND => self.expand_selected(),
            a if a == config::internal::COLLAPSE => self.collapse_selected(),
            a if a == config::internal::COLLAPSE_ALL => self.collapse_all(),
            a if a == config::internal::REFRESH => {
                self.refresh();
                KeyOutcome::Handled
            }
            a if a == config::internal::SCM_TOGGLE_STAGE => self.toggle_stage(),
            a if a == config::internal::SCM_STAGE_ALL => self.stage_all(),
            a if a == config::internal::SCM_UNSTAGE_ALL => self.unstage_all(),
            a if a == config::internal::SCM_FOCUS_MESSAGE || a == config::internal::SCM_COMMIT => {
                self.focus_message()
            }
            a if a == config::internal::SCM_DISCARD => self.request_discard(),
            a if a == config::internal::SCM_CONFIRM => KeyOutcome::Handled,
            a if a == config::internal::SCM_CANCEL => self.cancel_transient(),
            a if a == config::internal::SCM_SYNC => self.sync_changes(),
            a if a == config::internal::SCM_SUGGEST_MESSAGE => self.suggest_message(),
            a if a == config::internal::TOGGLE_HIDDEN
                || a == config::internal::EXPLORER_CREATE
                || a == config::internal::EXPLORER_DELETE
                || a == config::internal::EXPLORER_RENAME
                || a == config::internal::EDIT_BACKSPACE
                || a == config::internal::EDIT_DELETE
                || a == config::internal::EDIT_HOME
                || a == config::internal::EDIT_END =>
            {
                KeyOutcome::Handled
            }
            // Custom shell function from config.sh (file = selected path).
            other => {
                let file = self.selected_entry().map(|(_, e)| {
                    self.git
                        .as_ref()
                        .map(|g| g.abs_path(e))
                        .unwrap_or_else(|| PathBuf::from(&e.path))
                });
                KeyOutcome::Shell {
                    action: other.to_string(),
                    file,
                    env: Vec::new(),
                }
            }
        }
    }

    fn glyph_for(&self, entry: &FileEntry) -> ui::icons::FileGlyph {
        ui::icons::file_glyph(std::path::Path::new(&entry.path), self.nerd_font)
    }

    fn letter_color(&self, letter: char, palette: &Palette) -> ratatui::style::Color {
        match letter {
            'M' => palette.yellow,
            'A' | 'U' => palette.green,
            'D' => palette.red,
            'R' | 'C' => palette.blue,
            '!' => palette.red,
            _ => palette.subtext0,
        }
    }
}

impl FeatureView for ScmView {
    fn draw(&self, frame: &mut Frame, area: Rect, palette: &Palette) {
        self.body_top.set(area.y);
        self.body_height.set(area.height);
        if area.height == 0 {
            return;
        }

        // Repository identity is its own hierarchy row: root on the left,
        // branch and sync state aligned independently on the right.
        let repo = self
            .git
            .as_ref()
            .map(Git::name)
            .unwrap_or_else(|| "repository".into());
        let mut branch = if self.status.branch.is_empty() {
            "—".to_string()
        } else {
            self.status.branch.clone()
        };
        if self.status.ahead > 0 {
            branch.push_str(&format!(" ↑{}", self.status.ahead));
        }
        if self.status.behind > 0 {
            branch.push_str(&format!(" ↓{}", self.status.behind));
        }
        if self.syncing.is_some() {
            branch.push_str(" ⇅");
        }
        let branch_width = (branch.chars().count() as u16 + 1).min(area.width / 2);
        let repo_rect = Rect::new(area.x, area.y, area.width.saturating_sub(branch_width), 1);
        let branch_rect = Rect::new(
            area.x
                .saturating_add(area.width.saturating_sub(branch_width)),
            area.y,
            branch_width,
            1,
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ▾ ", Style::default().fg(palette.accent)),
                Span::styled(
                    repo,
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            repo_rect,
        );
        frame.render_widget(
            Paragraph::new(branch)
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette.yellow)),
            branch_rect,
        );

        if area.height < 5 {
            return;
        }

        let message_rect = Rect::new(area.x, area.y.saturating_add(1), area.width, 3);
        self.message_rect.set(message_rect);
        let mut shown = self.message.clone();
        if self.message_focused {
            shown.insert(self.cursor.min(shown.len()), '│');
        }
        let message: String = shown.into_iter().collect();
        let message_style = if self.message_focused {
            Style::default().fg(palette.text)
        } else {
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::DIM)
        };
        let border = if self.message_focused {
            palette.accent
        } else {
            palette.surface1
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(message_style)
                .block(Block::bordered().border_style(Style::default().fg(border))),
            message_rect,
        );
        let suggest_rect = if message_rect.width >= 6 {
            Rect::new(
                message_rect
                    .x
                    .saturating_add(message_rect.width.saturating_sub(4)),
                message_rect.y.saturating_add(1),
                3,
                1,
            )
        } else {
            Rect::default()
        };
        self.suggest_rect.set(suggest_rect);
        if suggest_rect.width > 0 {
            frame.render_widget(
                Paragraph::new(if self.suggesting.is_some() {
                    " … "
                } else {
                    " ✧ "
                })
                .style(
                    Style::default()
                        .fg(palette.accent)
                        .bg(palette.panel_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                suggest_rect,
            );
        }

        let commit_rect = Rect::new(area.x, area.y.saturating_add(4), area.width, 1);
        self.commit_rect.set(commit_rect);
        let can_commit = !self.status.staged.is_empty() && !self.message.is_empty();
        let commit_style = if can_commit {
            Style::default()
                .fg(palette.panel_bg)
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(palette.text)
                .bg(palette.surface0)
                .add_modifier(Modifier::DIM)
        };
        frame.render_widget(
            Paragraph::new("Commit")
                .alignment(Alignment::Center)
                .style(commit_style),
            commit_rect,
        );

        // One breathing row between the primary action and the SCM hierarchy.
        let body = Rect {
            x: area.x,
            y: area.y.saturating_add(6),
            width: area.width,
            height: area.height.saturating_sub(6),
        };
        self.body_top.set(body.y);
        self.body_height.set(body.height);

        if let Some(err) = &self.error {
            frame.render_widget(
                Paragraph::new(format!("  {err}")).style(Style::default().fg(palette.red)),
                body,
            );
        } else {
            let h = body.height as usize;
            for (i, row) in self.rows.iter().skip(self.scroll).take(h).enumerate() {
                let y = body.y.saturating_add(i as u16);
                let abs_line = self.scroll + i;
                let rect = Rect {
                    x: body.x,
                    y,
                    width: body.width,
                    height: 1,
                };
                match row {
                    Row::Header {
                        section: _,
                        title,
                        count,
                        collapsed,
                    } => {
                        let selected = self
                            .selectable
                            .get(self.selected)
                            .is_some_and(|&row| row == abs_line);
                        let background = selected.then_some(palette.surface1);
                        let title_style = background.map_or_else(
                            || Style::default().fg(palette.text),
                            |color| Style::default().fg(palette.text).bg(color),
                        );
                        let (title_rect, badge_rect) = header_columns(rect, *count);
                        frame.render_widget(
                            Paragraph::new(Line::from(vec![
                                Span::raw(" "),
                                Span::styled(
                                    if *collapsed { "▸ " } else { "▾ " },
                                    title_style.add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(
                                    title.as_str(),
                                    title_style.add_modifier(Modifier::BOLD),
                                ),
                            ]))
                            .style(title_style),
                            title_rect,
                        );
                        frame.render_widget(
                            Paragraph::new(count.to_string())
                                .alignment(Alignment::Center)
                                .style(
                                    Style::default()
                                        .fg(palette.panel_bg)
                                        .bg(palette.accent)
                                        .add_modifier(Modifier::BOLD),
                                ),
                            badge_rect,
                        );
                    }
                    Row::File { staged, index } => {
                        let list = if *staged {
                            &self.status.staged
                        } else {
                            &self.status.unstaged
                        };
                        let Some(entry) = list.get(*index) else {
                            continue;
                        };
                        let is_sel = self
                            .selectable
                            .get(self.selected)
                            .is_some_and(|&r| r == abs_line);
                        let file_icon = self.glyph_for(entry);
                        let icon_fg = file_icon.color.unwrap_or(palette.text);
                        let bg = if is_sel { Some(palette.surface1) } else { None };
                        let with_bg = |s: Style| match bg {
                            Some(b) => s.bg(b),
                            None => s,
                        };
                        let letter_fg = self.letter_color(entry.letter, palette);
                        let name_style = if is_sel {
                            with_bg(
                                Style::default()
                                    .fg(palette.text)
                                    .add_modifier(Modifier::BOLD),
                            )
                        } else {
                            with_bg(Style::default().fg(palette.text))
                        };
                        let (name, parent) = display_path_parts(&entry.path);
                        let mut spans = vec![
                            Span::styled("  ", with_bg(Style::default())),
                            Span::styled(
                                format!("{} ", file_icon.glyph),
                                with_bg(Style::default().fg(icon_fg)),
                            ),
                            Span::styled(name.to_string(), name_style),
                        ];
                        if let Some(parent) = parent {
                            spans.push(Span::styled(
                                format!("  {parent}"),
                                with_bg(
                                    Style::default()
                                        .fg(palette.text)
                                        .add_modifier(Modifier::DIM),
                                ),
                            ));
                        }
                        let line = Line::from(spans);
                        let row_style = match bg {
                            Some(b) => Style::default().bg(b),
                            None => Style::default(),
                        };
                        let (name_rect, status_rect) = status_columns(rect);
                        // Paint the row once, then render two independent
                        // columns. Status letters no longer move with path length.
                        frame.render_widget(Paragraph::new("").style(row_style), rect);
                        frame.render_widget(Paragraph::new(line).style(row_style), name_rect);
                        frame.render_widget(
                            Paragraph::new(entry.letter.to_string())
                                .alignment(Alignment::Center)
                                .style(with_bg(
                                    Style::default().fg(letter_fg).add_modifier(Modifier::BOLD),
                                )),
                            status_rect,
                        );
                    }
                    Row::DrawerHeader { drawer, collapsed } => {
                        let selected = self
                            .selectable
                            .get(self.selected)
                            .is_some_and(|&row| row == abs_line);
                        let style = if selected {
                            Style::default().fg(palette.text).bg(palette.surface1)
                        } else {
                            Style::default().fg(palette.text)
                        };
                        frame.render_widget(
                            Paragraph::new(Line::from(vec![
                                Span::styled(
                                    if *collapsed { " ▸ " } else { " ▾ " },
                                    style.fg(palette.accent),
                                ),
                                Span::styled(drawer.title(), style.add_modifier(Modifier::BOLD)),
                            ]))
                            .style(style),
                            rect,
                        );
                    }
                    Row::DrawerLine { drawer, index } => {
                        let selected = self
                            .selectable
                            .get(self.selected)
                            .is_some_and(|&row| row == abs_line);
                        let style = if selected {
                            Style::default().fg(palette.text).bg(palette.surface1)
                        } else {
                            Style::default()
                                .fg(palette.text)
                                .add_modifier(Modifier::DIM)
                        };
                        if let Some(item) = self.drawer_lines[drawer.index()].get(*index) {
                            frame.render_widget(
                                Paragraph::new(format!("    {}", item.display)).style(style),
                                rect,
                            );
                        }
                    }
                }
            }
        }

        let notice_rect = Rect::new(
            area.x,
            area.y.saturating_add(area.height.saturating_sub(1)),
            area.width,
            1,
        );
        if let Some(entry) = &self.pending_discard {
            frame.render_widget(
                Paragraph::new(format!(" Discard {}? y/N", entry.path))
                    .style(Style::default().fg(palette.red).bg(palette.surface0)),
                notice_rect,
            );
        } else if let Some((message, error)) = &self.flash {
            frame.render_widget(
                Paragraph::new(format!(" {message}")).style(
                    Style::default()
                        .fg(if *error { palette.red } else { palette.green })
                        .bg(palette.surface0),
                ),
                notice_rect,
            );
        }
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
        if self.message_focused {
            return self.on_message_key(code, mods);
        }
        let Some(token) = config::key_token(code, mods) else {
            return KeyOutcome::Ignored;
        };
        let Some(action) = self
            .config
            .action_for_feature_key("scm", &token)
            .map(str::to_string)
        else {
            return KeyOutcome::Ignored;
        };
        self.dispatch_action(&action)
    }

    fn captures_text_input(&self) -> bool {
        self.message_focused
    }

    fn on_shell_result(&mut self, action: &str, ok: bool) {
        if action != "commit_message" {
            return;
        }
        if ok {
            self.message.clear();
            self.cursor = 0;
            self.message_focused = false;
            self.set_flash("commit created", false);
            self.refresh();
        } else {
            self.set_flash("commit failed", true);
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> KeyOutcome {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if ui::hit(self.suggest_rect.get(), mouse.column, mouse.row) {
                    return self.suggest_message();
                }
                if ui::hit(self.message_rect.get(), mouse.column, mouse.row) {
                    return self.focus_message();
                }
                if ui::hit(self.commit_rect.get(), mouse.column, mouse.row) {
                    return self.commit_message();
                }
                self.message_focused = false;
                let Some(idx) = self.row_at_mouse(mouse.row) else {
                    return KeyOutcome::Ignored;
                };
                let header = self
                    .selectable
                    .get(idx)
                    .and_then(|&row| self.rows.get(row))
                    .is_some_and(|row| {
                        matches!(row, Row::Header { .. } | Row::DrawerHeader { .. })
                    });
                if header {
                    self.selected = idx;
                    self.toggle_selected()
                } else if self.selected == idx {
                    self.open_selected()
                } else {
                    self.selected = idx;
                    self.remember_selected_file();
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
        self.refresh();
    }

    fn on_tick(&mut self) {
        let suggestion = self.suggesting.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(SuggestionCompletion {
                root: self
                    .git
                    .as_ref()
                    .map(|git| git.root().to_path_buf())
                    .unwrap_or_default(),
                result: Err("suggestion worker stopped".into()),
            }),
        });
        if let Some(completion) = suggestion {
            self.suggesting = None;
            let same_repo = self
                .git
                .as_ref()
                .is_some_and(|git| git.root() == completion.root);
            if same_repo {
                match completion.result {
                    Ok(message) => {
                        self.message = message.chars().collect();
                        self.cursor = self.message.len();
                        self.message_focused = true;
                        self.set_flash("✧ suggestion ready — edit or commit", false);
                    }
                    Err(error) => self.set_flash(error, true),
                }
            } else {
                self.set_flash("suggestion ignored: repository changed", true);
            }
        }

        let completed = self.syncing.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("sync worker stopped".into())),
        });
        if let Some(result) = completed {
            self.syncing = None;
            match result {
                Ok(message) => self.set_flash(message, false),
                Err(error) => self.set_flash(error, true),
            }
            self.refresh();
        } else if self.last_refresh.elapsed() >= AUTO_REFRESH {
            self.refresh();
        }
        self.expire_flash();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_expire_but_progress_remains() {
        let mut view = ScmView::new(
            PathBuf::from("/corral-test-no-repo"),
            false,
            Arc::new(Config::for_test()),
        );
        view.set_flash("done", false);
        view.flash_at = Some(Instant::now() - Duration::from_secs(3));
        view.expire_flash();
        assert!(view.flash.is_none());

        view.set_flash("failed", true);
        view.flash_at = Some(Instant::now() - Duration::from_secs(3));
        view.expire_flash();
        assert!(view.flash.is_some());
        view.flash_at = Some(Instant::now() - Duration::from_secs(5));
        view.expire_flash();
        assert!(view.flash.is_none());

        view.set_progress("working…");
        view.expire_flash();
        assert!(view.flash.is_some());
    }

    #[test]
    fn diff_kind_respects_section_and_untracked_state() {
        assert_eq!(diff_action(true, 'M'), "diff_staged");
        assert_eq!(diff_action(false, 'M'), "diff");
        assert_eq!(diff_action(false, 'U'), "diff_untracked");
        // An MM path appears twice; section identity must win over its letter.
        assert_ne!(diff_action(true, 'M'), diff_action(false, 'M'));
    }

    #[test]
    fn status_letter_has_a_fixed_right_column() {
        let (name, status) = status_columns(Rect::new(4, 7, 30, 1));
        assert_eq!(name, Rect::new(4, 7, 27, 1));
        assert_eq!(status, Rect::new(31, 7, 3, 1));
    }

    #[test]
    fn section_count_is_a_separate_right_badge() {
        let (title, badge) = header_columns(Rect::new(4, 7, 30, 1), 19);
        assert_eq!(title, Rect::new(4, 7, 26, 1));
        assert_eq!(badge, Rect::new(30, 7, 4, 1));
    }

    #[test]
    fn paths_render_as_basename_then_parent() {
        assert_eq!(
            display_path_parts("src/herdr/cli.rs"),
            ("cli.rs", Some("src/herdr"))
        );
        assert_eq!(display_path_parts("Cargo.toml"), ("Cargo.toml", None));
    }

    #[test]
    fn suggestion_output_is_reduced_to_one_editable_subject() {
        assert_eq!(
            clean_suggestion("warning: noisy provider\n```\n'Fix generated message.'\n").unwrap(),
            "Fix generated message"
        );
        assert!(clean_suggestion("\n```\n").is_err());
        assert!(clean_suggestion(&"x".repeat(201)).is_err());
    }

    #[test]
    fn drawer_lines_keep_actionable_references() {
        let history = log_drawer_item("abc1234\tfix history".into());
        assert_eq!(history.display, "abc1234 fix history");
        assert_eq!(history.reference.as_deref(), Some("abc1234"));

        let graph = graph_drawer_item("* abc1234 (HEAD -> main) subject".into());
        assert_eq!(graph.reference.as_deref(), Some("abc1234"));

        let branch = branch_drawer_item("*\tmain".into());
        assert_eq!(branch.display, "* main");
        assert_eq!(branch.reference.as_deref(), Some("main"));

        let worktree = worktree_drawer_item("/tmp/my tree\tfeature".into());
        assert_eq!(worktree.reference.as_deref(), Some("/tmp/my tree"));

        let remote = remote_drawer_item("origin\tgit@github.com:example/repo.git".into());
        assert_eq!(remote.display, "origin  example/repo");
        assert_eq!(remote.reference, None);
        assert_eq!(
            short_remote_url("https://github.com/example/repo.git"),
            "example/repo"
        );
        assert_eq!(short_remote_url("/srv/git/repo.git"), "repo");
    }

    #[test]
    fn async_suggestion_replaces_message_and_focuses_input() {
        let root = std::env::temp_dir().join(format!(
            "corral-suggest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&root)
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "Test"]);
        std::fs::write(root.join("file.txt"), "change\n").unwrap();
        git(&["add", "file.txt"]);

        let mut config = Config::for_test();
        config.source = "suggest_commit_message() { printf '\"Generated subject.\"\\n'; }".into();
        let mut view = ScmView::new(root.clone(), false, Arc::new(config));
        view.message = "keep on failure".chars().collect();
        assert!(matches!(view.suggest_message(), KeyOutcome::Handled));
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(20));
            view.on_tick();
            if view.suggesting.is_none() {
                break;
            }
        }
        assert_eq!(view.message.iter().collect::<String>(), "Generated subject");
        assert!(view.message_focused);
        assert_eq!(view.cursor, view.message.len());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn section_header_is_selectable_and_really_collapses() {
        let config = Arc::new(Config::for_test());
        let mut view = ScmView::new(PathBuf::from("/corral-test-no-repo"), false, config);
        view.error = None;
        view.status = Status {
            staged: vec![FileEntry {
                path: "one.rs".into(),
                orig: None,
                letter: 'M',
            }],
            ..Status::default()
        };
        view.rebuild_rows();
        assert_eq!(
            view.rows
                .iter()
                .filter(|row| matches!(row, Row::File { .. }))
                .count(),
            1
        );
        assert!(matches!(view.selected_row(), Some(Row::Header { .. })));

        assert_eq!(view.toggle_selected(), KeyOutcome::Handled);
        assert!(view.staged_collapsed);
        assert_eq!(
            view.rows
                .iter()
                .filter(|row| matches!(row, Row::File { .. }))
                .count(),
            0
        ); // the file row is actually gone
        assert!(matches!(
            view.selected_row(),
            Some(Row::Header {
                collapsed: true,
                ..
            })
        ));

        view.toggle_selected();
        assert!(!view.staged_collapsed);
        assert_eq!(
            view.rows
                .iter()
                .filter(|row| matches!(row, Row::File { .. }))
                .count(),
            1
        );
    }
}
