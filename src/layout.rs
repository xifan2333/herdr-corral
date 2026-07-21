//! In-process workbench layout: activity bar + left container + right container.
//!
//! One process owns the whole UI. Features plug into the left/right regions;
//! the activity bar only switches which feature is active.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Which region currently owns keyboard focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    /// Activity bar (feature switcher).
    Activity,
    /// Left feature pane.
    #[default]
    Left,
    /// Right detail pane.
    Right,
}

impl Focus {
    pub fn cycle(self) -> Self {
        match self {
            Focus::Activity => Focus::Left,
            Focus::Left => Focus::Right,
            Focus::Right => Focus::Activity,
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

/// Geometry for activity bar + two containers.
#[derive(Clone, Copy, Debug)]
pub struct Regions {
    pub activity: Rect,
    pub left: Rect,
    pub right: Rect,
}

/// Width of the VS Code-style activity rail (indicator + icon + padding).
pub const ACTIVITY_WIDTH: u16 = 3;

/// Split the host area into activity bar | left | right.
///
/// - activity bar: fixed narrow icon rail
/// - left: `left_pct` of the body width (clamped 15..=50)
/// - right: the rest
/// - bottom status row is reserved by the caller (pass area already shortened)
pub fn split(area: Rect, left_pct: u16) -> Regions {
    // activity | body
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(ACTIVITY_WIDTH),
            Constraint::Min(10),
        ])
        .split(area);

    let body = cols[1];
    let left_pct = left_pct.clamp(15, 50);
    // left_pct is of the full width; approximate as percent of body.
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Percentage(100 - left_pct),
        ])
        .split(body);

    Regions {
        activity: cols[0],
        left: chunks[0],
        right: chunks[1],
    }
}
