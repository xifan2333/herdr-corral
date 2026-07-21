//! In-process two-column layout: left container + right container.
//!
//! Corral is a **single host process**. The left/right containers are regions
//! drawn inside that process — not separate panes. Features plug into these
//! regions and supply their own titles/content; the shell does not hardcode
//! feature names.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Which container currently owns keyboard focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Left,
    Right,
}

impl Focus {
    pub fn toggle(self) -> Self {
        match self {
            Focus::Left => Focus::Right,
            Focus::Right => Focus::Left,
        }
    }
}

/// What a feature mounts into a container for this frame.
///
/// Titles/body come from the feature; the shell draws the frame only.
#[derive(Clone, Debug, Default)]
pub struct PanelView {
    /// Optional title drawn on the border. Empty = no title.
    pub title: Option<String>,
    /// Placeholder body until real views land.
    pub body: String,
}

/// Geometry for the two containers inside the host pane.
#[derive(Clone, Copy, Debug)]
pub struct Containers {
    pub left: Rect,
    pub right: Rect,
}

/// Split the host pane into left + right containers.
///
/// `left_pct` is the left container's share of the width (clamped to 15..=50).
pub fn split(area: Rect, left_pct: u16) -> Containers {
    let left_pct = left_pct.clamp(15, 50);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Percentage(100 - left_pct),
        ])
        .split(area);
    Containers {
        left: chunks[0],
        right: chunks[1],
    }
}
