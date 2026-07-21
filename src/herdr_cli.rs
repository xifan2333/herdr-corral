//! Best-effort calls to the Herdr CLI via `HERDR_BIN_PATH`.
//!
//! Standalone mode has no herdr — callers should treat `false` as "skip".
//! Never panics.

use std::path::Path;
use std::process::{Command, Stdio};

/// Rename a pane's border label (what Herdr shows as the pane title).
///
/// Returns `true` only when the CLI exited successfully. Missing bin/id or a
/// failed spawn/non-zero status → `false` (caller must not assume the label stuck).
pub fn set_pane_label(herdr_bin: Option<&Path>, pane_id: Option<&str>, label: &str) -> bool {
    let (Some(bin), Some(id)) = (herdr_bin, pane_id) else {
        return false;
    };
    if id.is_empty() || label.is_empty() {
        return false;
    }
    Command::new(bin)
        .args(["pane", "rename", id, label])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
