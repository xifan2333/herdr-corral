//! Icon capability detection (Nerd Font).
//!
//! File-type glyphs (via `devicons`) come later. This module only answers:
//! can this terminal render Nerd Font icons?
//!
//! Detection is best-effort and never panics. Call [`has_nerd_font`] once at
//! startup and pass the result into views that want icons.

use has_nerd_font::{Confidence, DetectionResult, DetectionSource};

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
