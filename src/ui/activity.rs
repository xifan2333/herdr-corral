//! VS Code-style activity strip for the sidebar.
//!
//! Terminal cells can't vertically center a glyph inside a multi-row button, so
//! the selected chip is painted as:
//!
//! ```text
//!   ▄▄▄▄   ← top half-block (fg = chip color)
//!        ← icon on the middle row only (bg = chip color)
//!   ▀▀▀▀   ← bottom half-block
//! ```
//!
//! That is the same convention as herdr-sidebar. Callers only pass the active
//! feature + palette; they never touch half-block characters.
//!
//! Nerd Font FA glyphs are often **two cells** wide in non-Mono fonts. We keep a
//! trailing slack cell inside each chip so widths stay even and icons look
//! centered (again matching herdr-sidebar).

use crate::feature::Feature;
use crate::theme::Palette;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// One activity-bar item to render.
#[derive(Clone, Copy, Debug)]
pub struct ActivityItem {
    pub feature: Feature,
    pub icon: &'static str,
    /// Reserve a trailing cell after the glyph (double-width Nerd icons).
    pub double_width: bool,
}

/// Result of painting the activity strip: hit targets for mouse clicks.
#[derive(Clone, Debug, Default)]
pub struct ActivityBar {
    pub hits: Vec<(Feature, Rect)>,
}

/// True when `(col, row)` lands inside `rect`.
pub fn hit(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

/// Draw the 3-row activity strip into `area` and return click hit targets.
///
/// Requires `area.height >= 3`. Icons are drawn only on the middle row; the
/// selected item gets half-block caps on the outer rows.
pub fn draw_activity(
    frame: &mut Frame,
    area: Rect,
    items: &[ActivityItem],
    active: Feature,
    palette: &Palette,
) -> ActivityBar {
    let mut bar = ActivityBar::default();
    if area.height < 3 || area.width == 0 || items.is_empty() {
        return bar;
    }

    let outer_top = area.y;
    let outer_bottom = area.y + 2;
    let mid = Rect::new(area.x, area.y + 1, area.width, 1);
    let chip_bg = palette.surface1;

    // Span layout: [pad, chip0, gap, chip1, gap, …]
    // Chip text: " {icon}{slack} " — spaces are part of the chip (herdr-sidebar).
    let mut spans: Vec<Span> = Vec::with_capacity(1 + items.len() * 2);
    spans.push(Span::raw(" "));
    for item in items {
        let slack = if item.double_width { " " } else { "" };
        let selected = item.feature == active;
        let style = if selected {
            Style::default()
                .fg(palette.text)
                .bg(chip_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            // Idle: theme main text ("white"), no chip background.
            Style::default().fg(palette.text)
        };
        spans.push(Span::styled(format!(" {}{slack} ", item.icon), style));
        spans.push(Span::raw(" "));
    }

    // Absolute chip bounds from real span widths (emoji vs nerd differ).
    let mut x = mid.x;
    let mut bounds: Vec<(Feature, u16, u16)> = Vec::with_capacity(items.len());
    for (i, span) in spans.iter().enumerate() {
        let w = span.width() as u16;
        if i % 2 == 1 {
            // chip span at indices 1, 3, 5…
            let fi = i / 2;
            if let Some(item) = items.get(fi) {
                bounds.push((item.feature, x, x + w));
            }
        }
        x = x.saturating_add(w);
    }

    // Half-block caps on the active chip only — tall button, glyph on middle.
    if let Some((_, start, end)) = bounds.iter().find(|(f, _, _)| *f == active) {
        let chip_w = end.saturating_sub(*start);
        if chip_w > 0 {
            // fg = chip color, terminal default bg → caps read as chip extensions.
            frame.render_widget(
                Paragraph::new("▄".repeat(usize::from(chip_w)))
                    .style(Style::default().fg(chip_bg)),
                Rect::new(*start, outer_top, chip_w, 1),
            );
            frame.render_widget(
                Paragraph::new("▀".repeat(usize::from(chip_w)))
                    .style(Style::default().fg(chip_bg)),
                Rect::new(*start, outer_bottom, chip_w, 1),
            );
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), mid);

    for (feature, start, end) in bounds {
        bar.hits.push((
            feature,
            Rect {
                x: start,
                y: outer_top,
                width: end.saturating_sub(start).max(1),
                height: 3,
            },
        ));
    }
    bar
}
