//! Boundary to the Herdr host (CLI today; socket / more ops later).
//!
//! All process-spawn and host RPC for Corral go through this module tree.
//! [`cli`] is the first surface (`HERDR_BIN_PATH`); pane labels are just one
//! function among future open/resize/preview helpers.

pub mod cli;
pub mod launch;

pub use cli::{report_sidebar_heartbeat, set_pane_label, SIDEBAR_LABEL, SIDEBAR_TOKEN};
