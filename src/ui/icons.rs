//! Icons: Nerd Font capability + per-file-type glyphs.
//!
//! - [`detect`] / [`has_nerd_font`]: can this terminal render Nerd Fonts?
//! - [`file_glyph`]: path → glyph for the Explorer tree (via `devicons`)

use has_nerd_font::{Confidence, DetectionResult, DetectionSource};
use ratatui::style::Color;
use std::path::Path;

/// Snapshot of Nerd Font detection for the current process environment.
#[derive(Clone, Debug)]
pub struct NerdFontSupport {
    /// `true` / `false` when detection decided; `None` when unknown.
    pub available: Option<bool>,
    pub confidence: Confidence,
    pub source: DetectionSource,
    pub terminal: Option<String>,
    pub font: Option<String>,
    pub error_reason: Option<String>,
}

impl NerdFontSupport {
    /// Convenience: treat only a confident yes as "use icons".
    pub fn should_use_icons(&self) -> bool {
        self.available == Some(true)
    }
}

/// Detect Nerd Font support from the current process environment.
pub fn detect() -> NerdFontSupport {
    let vars: Vec<(String, String)> = std::env::vars().collect();
    from_result(has_nerd_font::detect(&vars))
}

/// True when detection says Nerd Font glyphs are available.
pub fn has_nerd_font() -> bool {
    detect().should_use_icons()
}

fn from_result(r: DetectionResult) -> NerdFontSupport {
    NerdFontSupport {
        available: r.detected,
        confidence: r.confidence,
        source: r.source,
        terminal: r.terminal.map(|t| format!("{t:?}")),
        font: r.font,
        error_reason: r.error_reason,
    }
}

/// Glyph + optional color for a path in the file tree.
#[derive(Clone, Copy, Debug)]
pub struct FileGlyph {
    /// Display string (1–2 terminal cells typically).
    pub glyph: &'static str,
    /// Optional truecolor from devicons hex.
    pub color: Option<Color>,
}

/// Directory open/closed glyphs (Nerd Font or ASCII fallback).
pub fn dir_glyph(open: bool, nerd_font: bool) -> FileGlyph {
    if nerd_font {
        FileGlyph {
            glyph: if open {
                "\u{f07c}" // folder_open
            } else {
                "\u{f07b}" // folder
            },
            color: None, // caller may use theme blue
        }
    } else {
        FileGlyph {
            glyph: if open { "v" } else { ">" },
            color: None,
        }
    }
}

/// File-type glyph for `path` when Nerd Fonts are available; plain fallback otherwise.
pub fn file_glyph(path: &Path, nerd_font: bool) -> FileGlyph {
    if !nerd_font {
        return FileGlyph {
            glyph: "·",
            color: None,
        };
    }

    // devicons returns a char + "#rrggbb" color string.
    let icon = devicons::FileIcon::from(path);
    let color = parse_hex_color(icon.color);
    // Leak is intentional: icon set is static/finite; Explorer rebuilds often
    // and we need a 'static str for Span without per-frame allocation churn.
    // Prefer a tiny thread-local cache if this becomes hot.
    let glyph = icon_char_str(icon.icon);
    FileGlyph { glyph, color }
}

fn icon_char_str(c: char) -> &'static str {
    // Cache a few common paths: store each unique char once.
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<char, &'static str>>> = Mutex::new(None);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(s) = map.get(&c) {
        return s;
    }
    let s: &'static str = Box::leak(c.to_string().into_boxed_str());
    map.insert(c, s);
    s
}

fn parse_hex_color(s: &str) -> Option<Color> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}
