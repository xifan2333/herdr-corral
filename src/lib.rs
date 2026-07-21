//! corral — VS Code-style terminal workbench for Herdr.
//!
//! Shape (same as herdr-file-viewer):
//! - one Herdr plugin pane hosts the whole workbench
//! - left/right containers are **in-process** regions, not separate Herdr panes
//! - [`theme`] resolves Herdr's UI palette so every future view shares one color source
//! - [`layout`] owns the two-container geometry
//! - [`app`] is the host event loop / shell

pub mod app;
pub mod layout;
pub mod theme;

pub use layout::{Containers, Focus};
pub use theme::Palette;

/// Entry point invoked by the binary.
pub fn run() -> std::io::Result<()> {
    app::run()
}
