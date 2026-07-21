//! Temporary body for features that are not implemented yet.

use super::Feature;
use super::view::{FeatureView, KeyOutcome};
use crate::theme::Palette;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

pub struct PlaceholderView {
    feature: Feature,
    body: String,
}

impl PlaceholderView {
    pub fn new(feature: Feature, body: String) -> Self {
        Self { feature, body }
    }
}

impl FeatureView for PlaceholderView {
    fn draw(&self, frame: &mut Frame, area: Rect, palette: &Palette) {
        if area.height == 0 {
            return;
        }
        // Title line + blank + body (pane border title is owned by Herdr rename).
        let title = format!(" {}", self.feature.title());
        frame.render_widget(
            Paragraph::new(title).style(
                Style::default()
                    .fg(palette.subtext0)
                    .bg(palette.panel_bg)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
        );
        if area.height < 3 {
            return;
        }
        let body = Rect {
            x: area.x.saturating_add(1),
            y: area.y.saturating_add(2),
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        frame.render_widget(
            Paragraph::new(self.body.as_str()).style(Style::default().fg(palette.overlay1)),
            body,
        );
    }

    fn on_key(&mut self, _code: KeyCode, _mods: KeyModifiers) -> KeyOutcome {
        // No body navigation yet — j/k must not cycle features.
        KeyOutcome::Ignored
    }
}
