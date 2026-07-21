//! corral — shared internals for the Herdr workbench plugin.
//!
//! The [`theme`] module is the single color source: it resolves Herdr's current
//! UI theme palette deterministically (ported from Herdr's own theme tables,
//! read from its `config.toml`) and hands every later component (explorer / scm
//! / github panes) one consistent set of colors to style against.

pub mod theme;

pub use theme::Palette;
