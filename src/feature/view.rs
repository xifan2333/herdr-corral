//! Mount contract for a sidebar feature body.
//!
//! The shell owns activity switching (`1`/`2`/`3`, icon clicks) and terminal
//! setup. Each feature owns its body keys and drawing. External behaviour
//! goes through shell functions in `config.sh` via [`KeyOutcome::Shell`].

use crate::ui::Palette;
use crossterm::event::{KeyCode, KeyModifiers, MouseEvent};
use ratatui::layout::Rect;
use ratatui::Frame;
use std::path::PathBuf;

/// Result of handing a key/mouse event to the active feature body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Feature handled the key in-process.
    Handled,
    /// Feature does not care.
    Ignored,
    /// Run a shell function from `config.sh` (TUI should suspend first).
    Shell {
        action: String,
        file: Option<PathBuf>,
        /// Structured context for the shell action (repo root, relative path,
        /// diff kind details, …); avoids lossy command-string encoding.
        env: Vec<(String, String)>,
    },
}

/// One feature's body UI (Explorer tree, SCM list, …).
pub trait FeatureView {
    /// Draw into the body region under the activity strip.
    fn draw(&self, frame: &mut Frame, area: Rect, palette: &Palette);

    /// Handle a key while this feature is active.
    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyOutcome;

    /// Text-entry modes receive keys before global configured actions, so a
    /// commit message can contain characters such as `q` or `1`.
    fn captures_text_input(&self) -> bool {
        false
    }

    /// Result callback for a shell action emitted by this view.
    fn on_shell_result(&mut self, _action: &str, _ok: bool) {}

    /// Optional mouse handling inside the body (not activity hits).
    fn on_mouse(&mut self, _mouse: MouseEvent) -> KeyOutcome {
        KeyOutcome::Ignored
    }

    /// Called when the shell switches *to* this feature.
    fn on_activate(&mut self) {}

    /// Periodic idle callback for lightweight live refresh. Implementations
    /// must rate-limit expensive work themselves; the shell ticks at 100ms.
    fn on_tick(&mut self) {}
}
