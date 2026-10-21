//! Calls into the Herdr host via `HERDR_BIN_PATH` (and later the socket API).
//!
//! This is the only process-spawn / host-RPC boundary for Corral. Pane labels
//! are just one operation; open preview, resize, metadata, etc. land here too.
//!
//! Standalone mode has no herdr — callers treat `false` / `None` as "skip".
//! Never panics.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SIDEBAR_TOKEN: &str = "corral-sidebar";
pub const SIDEBAR_LABEL: &str = "Corral";

/// Read the current directory of the layout's designated shell.
///
/// # Errors
/// Returns an error when the host command or response fails.
pub fn shell_cwd(herdr_bin: &Path, sidebar_id: &str) -> Result<Option<std::path::PathBuf>, String> {
    let output = Command::new(herdr_bin)
        .args(["pane", "list"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("read shell directory: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "read shell directory: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let response = std::str::from_utf8(&output.stdout)
        .map_err(|error| format!("invalid pane response: {error}"))?;
    super::launch::shell_cwd(response, sidebar_id)
}

/// Rename a pane's border label (what Herdr shows as the pane title).
///
/// Returns `true` only when the CLI exited successfully. Missing bin/id or a
/// failed spawn/non-zero status → `false` (caller must not assume the label stuck).
#[must_use]
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
        .is_ok_and(|s| s.success())
}

/// Stamp stable sidebar identity plus a timestamp heartbeat. The token is
/// independent of the cosmetic/activity state and expires host-side after 20s.
#[must_use]
pub fn report_sidebar_heartbeat(herdr_bin: Option<&Path>, pane_id: Option<&str>) -> bool {
    let (Some(bin), Some(id)) = (herdr_bin, pane_id) else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let token = format!("{SIDEBAR_TOKEN}={now}");
    Command::new(bin)
        .args([
            "pane",
            "report-metadata",
            id,
            "--source",
            "corral",
            "--token",
            &token,
            "--ttl-ms",
            "20000",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
