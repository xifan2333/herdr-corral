//! Explorer: expandable file tree rooted at the launch cwd.

use super::view::{FeatureView, KeyOutcome};
use crate::ui::Palette;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::cell::Cell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    depth: usize,
}

pub struct ExplorerView {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    rows: Vec<Entry>,
    selected: usize,
    scroll: usize,
    /// Last-drawn body top row (pane-local / absolute screen row from frame).
    body_top: Cell<u16>,
    body_height: Cell<u16>,
    nerd_font: bool,
    error: Option<String>,
}

impl ExplorerView {
    pub fn new(root: PathBuf, nerd_font: bool) -> Self {
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
            error: None,
        };
        view.rebuild();
        view
    }

    fn rebuild(&mut self) {
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

        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
        self.ensure_visible();
    }

    fn collect_children(&self, dir: &Path, depth: usize) -> Result<Vec<Entry>, String> {
        if !self.expanded.contains(dir) {
            return Ok(Vec::new());
        }

        let mut entries: Vec<(PathBuf, String, bool)> = fs::read_dir(dir)
            .map_err(|e| format!("read {}: {e}", dir.display()))?
            .filter_map(|res| res.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
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

    fn toggle_or_open(&mut self) {
        let Some(entry) = self.rows.get(self.selected).cloned() else {
            return;
        };
        if !entry.is_dir {
            // File preview lands later.
            return;
        }
        if self.expanded.contains(&entry.path) {
            self.expanded.remove(&entry.path);
        } else {
            self.expanded.insert(entry.path);
        }
        self.rebuild();
    }

    fn collapse_or_parent(&mut self) {
        let Some(entry) = self.rows.get(self.selected).cloned() else {
            return;
        };
        if entry.is_dir && self.expanded.contains(&entry.path) {
            self.expanded.remove(&entry.path);
            self.rebuild();
            return;
        }
        if entry.depth == 0 {
            return;
        }
        let Some(parent) = entry.path.parent().map(Path::to_path_buf) else {
            return;
        };
        if let Some(idx) = self.rows.iter().position(|r| r.path == parent) {
            self.selected = idx;
            if self.expanded.contains(&parent) {
                self.expanded.remove(&parent);
                self.rebuild();
            }
            self.ensure_visible();
        }
    }

    fn glyph_for(&self, entry: &Entry) -> &'static str {
        if entry.is_dir {
            let open = self.expanded.contains(&entry.path);
            if self.nerd_font {
                if open {
                    "\u{f07c}" // folder_open
                } else {
                    "\u{f07b}" // folder
                }
            } else if open {
                "v"
            } else {
                ">"
            }
        } else if self.nerd_font {
            "\u{f15b}" // file
        } else {
            "·"
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
            let glyph = self.glyph_for(entry);
            let chevron = if entry.is_dir {
                if self.expanded.contains(&entry.path) {
                    "▾ "
                } else {
                    "▸ "
                }
            } else {
                "  "
            };

            // No panel fill — only the selected row gets a chip bg.
            let fg = if entry.is_dir {
                palette.blue
            } else {
                palette.text
            };
            let style = if selected {
                Style::default()
                    .fg(fg)
                    .bg(palette.surface1)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg)
            };
            let dim = if selected {
                Style::default().fg(palette.overlay1).bg(palette.surface1)
            } else {
                Style::default().fg(palette.overlay1)
            };

            let line = Line::from(vec![
                Span::styled(format!("{indent}{chevron}"), dim),
                Span::styled(format!("{glyph} "), style),
                Span::styled(entry.name.as_str(), style),
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

    fn on_key(&mut self, code: KeyCode, _mods: KeyModifiers) -> KeyOutcome {
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_sel(1);
                KeyOutcome::Handled
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_sel(-1);
                KeyOutcome::Handled
            }
            KeyCode::Char('g') => {
                self.selected = 0;
                self.ensure_visible();
                KeyOutcome::Handled
            }
            KeyCode::Char('G') => {
                self.selected = self.rows.len().saturating_sub(1);
                self.ensure_visible();
                KeyOutcome::Handled
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                self.toggle_or_open();
                KeyOutcome::Handled
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.collapse_or_parent();
                KeyOutcome::Handled
            }
            KeyCode::Char('r') => {
                self.rebuild();
                KeyOutcome::Handled
            }
            KeyCode::PageDown => {
                self.move_sel(self.body_height.get().max(1) as isize);
                KeyOutcome::Handled
            }
            KeyCode::PageUp => {
                self.move_sel(-(self.body_height.get().max(1) as isize));
                KeyOutcome::Handled
            }
            _ => KeyOutcome::Ignored,
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> KeyOutcome {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(idx) = self.row_at_mouse(mouse.row) else {
                    return KeyOutcome::Ignored;
                };
                if self.selected == idx {
                    self.toggle_or_open();
                } else {
                    self.selected = idx;
                    self.ensure_visible();
                }
                KeyOutcome::Handled
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
