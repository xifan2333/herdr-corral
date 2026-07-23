//! Small shared helpers for the GitHub detail client.

use super::markdown::wrap_text;
use crate::ui::Palette;
use ratatui::style::{Color, Style};
use ratatui::text::Line;

pub(crate) fn state_color(state: &str, palette: &Palette) -> Color {
    match state.to_ascii_lowercase().as_str() {
        "open" | "success" | "completed" => palette.green,
        "merged" => palette.mauve,
        "failure" | "failed" | "closed" | "cancelled" | "timed_out" => palette.red,
        "in_progress" | "queued" | "pending" | "loading" => palette.yellow,
        _ => palette.overlay1,
    }
}

pub(crate) fn actor(actor: Option<&crate::github::Actor>) -> &str {
    actor.map(|actor| actor.login.as_str()).unwrap_or("unknown")
}

pub(crate) fn styled_text(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    wrap_text(text, width)
        .into_iter()
        .map(|line| Line::styled(line, style))
        .collect()
}


pub(crate) fn display_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() {
        fallback
    } else {
        value
    }
}

pub(crate) fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}
