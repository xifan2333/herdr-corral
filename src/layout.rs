//! In-process two-column layout: left container + right container.
//!
//! Like herdr-file-viewer, Corral is a **single Herdr pane**. The left/right
//! containers are regions drawn inside that one ratatui process — not separate
//! Herdr panes. Future features (Explorer / SCM / GitHub) plug into these
//! regions; they do not open their own host panes.

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

    pub fn label(self) -> &'static str {
        match self {
            Focus::Left => "left",
            Focus::Right => "right",
        }
    }
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
