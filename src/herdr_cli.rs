//! Best-effort calls to the Herdr CLI via `HERDR_BIN_PATH`.
//!
//! Failures are silent: standalone mode has no herdr, and a missing pane id
//! is fine. Never panics.

use std::path::Path;
use std::process::{Command, Stdio};

/// Rename the current plugin pane's border label (what Herdr shows as the
/// pane title). Uses `HERDR_PANE_ID` + `HERDR_BIN_PATH` when present.
pub fn set_pane_label(herdr_bin: Option<&Path>, pane_id: Option<&str>, label: &str) {
    let (Some(bin), Some(id)) = (herdr_bin, pane_id) else {
        return;
    };
    if id.is_empty() || label.is_empty() {
        return;
    }
    let _ = Command::new(bin)
        .args(["pane", "rename", id, label])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
