//! In-process workbench layout: left container + right container.
//!
//! Feature navigation lives *inside* the left panel (horizontal icon row),
//! not as a separate leftmost rail.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Which region currently owns keyboard focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    /// Left feature pane (nav + content).
    #[default]
    Left,
    /// Right detail pane.
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
#[derive(Clone, Debug, Default)]
pub struct PanelView {
    /// Optional title drawn on the border. Empty / None = no title.
    pub title: Option<String>,
    /// Body text until real views land.
    pub body: String,
}

/// Geometry for the two containers.
#[derive(Clone, Copy, Debug)]
pub struct Regions {
    pub left: Rect,
    pub right: Rect,
}

/// Split the host area into left | right.
///
/// `left_pct` is the left container's share of the width (clamped 15..=50).
/// Bottom status row is reserved by the caller (pass area already shortened).
pub fn split(area: Rect, left_pct: u16) -> Regions {
    let left_pct = left_pct.clamp(15, 50);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Percentage(100 - left_pct),
        ])
        .split(area);
    Regions {
        left: chunks[0],
        right: chunks[1],
    }
}

/// Height of the left-panel feature nav strip (icon row + vertical padding).
pub const NAV_HEIGHT: u16 = 3;

/// Split a left panel's inner area into: horizontal nav strip | feature body.
pub fn split_left_nav(inner: Rect) -> (Rect, Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(NAV_HEIGHT), Constraint::Min(1)])
        .split(inner);
    (rows[0], rows[1])
}
