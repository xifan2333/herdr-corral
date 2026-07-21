//! corral — VS Code-style terminal workbench.
//!
//! First a standalone TUI (like gitui). Optionally also a Herdr plugin.
//!
//! Shape:
//! - one process owns the whole UI
//! - left/right containers are **in-process** regions, not separate host panes
//! - [`host`] is the only Herdr boundary (plugin vs standalone)
//! - [`theme`] resolves colors (Herdr config if present, else `terminal`)
//! - [`layout`] owns the two-container geometry
//! - [`app`] is the host event loop / shell

pub mod app;
pub mod host;
pub mod layout;
pub mod theme;

pub use host::{LaunchContext, Mode};
pub use layout::{Containers, Focus};
pub use theme::Palette;

/// Entry point invoked by the binary (plugin pane or standalone).
pub fn run() -> std::io::Result<()> {
    let ctx = host::from_env();
    app::run(ctx)
}
