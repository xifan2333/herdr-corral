//! Sidebar layout helpers.
//!
//! Corral is a **left-docked sidebar pane** (herdr-sidebar shape), not a full
//! workbench that owns left+right regions. The host only draws the sidebar;
//! previews open as a separate Herdr pane later.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Height of the top activity strip (icon row + breathing room).
pub const ACTIVITY_HEIGHT: u16 = 3;

/// Split the sidebar body into: activity strip | feature content | optional footer.
pub fn split_sidebar(area: Rect, footer: bool) -> (Rect, Rect, Rect) {
    let footer_h = if footer { 1 } else { 0 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(ACTIVITY_HEIGHT),
            Constraint::Min(1),
            Constraint::Length(footer_h),
        ])
        .split(area);
    (rows[0], rows[1], rows[2])
}
