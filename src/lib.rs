//! corral — VS Code-style sidebar for the terminal.
//!
//! Shape (aligned with herdr-sidebar):
//! - **one left-docked pane** hosts Explorer / SCM / GitHub
//! - feature switch is in-process (top activity icons)
//! - file/diff/PR detail opens later as a **separate preview pane**
//! - also runs standalone (`corral`) without Herdr
//!
//! Modules:
//! - [`host`] — plugin vs standalone launch context
//! - [`theme`] — Herdr palette (or terminal fallback)
//! - [`icons`] — Nerd Font detection
//! - [`feature`] — Explorer / SCM / GitHub identity
//! - [`layout`] — sidebar strip geometry
//! - [`ui`] — terminal UI primitives (activity chips, hit tests)
//! - [`app`] — sidebar event loop

pub mod app;
pub mod feature;
pub mod herdr_cli;
pub mod host;
pub mod icons;
pub mod layout;
pub mod theme;
pub mod ui;

pub use feature::Feature;
pub use host::{LaunchContext, Mode};
pub use icons::{NerdFontSupport, has_nerd_font};
pub use theme::Palette;

/// Entry point invoked by the binary (plugin pane or standalone).
pub fn run() -> std::io::Result<()> {
    let ctx = host::from_env();
    app::run(ctx)
}
