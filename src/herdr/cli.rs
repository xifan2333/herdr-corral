//! Calls into the Herdr host via `HERDR_BIN_PATH` (and later the socket API).
//!
//! This is the only process-spawn / host-RPC boundary for Corral. Pane labels
//! are just one operation; open preview, resize, metadata, etc. land here too.
//!
//! Standalone mode has no herdr — callers treat `false` / `None` as "skip".
//! Never panics.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SIDEBAR_TOKEN: &str = "corral-sidebar";
pub const SIDEBAR_LABEL: &str = "Corral";

/// Read the current directory of the layout's designated shell.
///
/// Polls the host API socket when one is available and falls back to spawning
/// `herdr pane list`. See [`DirectoryPoll`] for the stateful version that the
/// event loop uses.
///
/// # Errors
/// Returns an error when the host command or response fails.
pub fn shell_cwd(herdr_bin: &Path, sidebar_id: &str) -> Result<Option<PathBuf>, String> {
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

/// Reusable shell-directory poll.
///
/// The poll runs every 500 ms for the whole session, so going through the CLI
/// would fork and exec a ~30 MB binary twice a second while the user is idle.
/// The socket round trip is about seven times cheaper and spawns no process at
/// all; the CLI stays as the fallback for hosts without a socket path, so the
/// two paths are never both paid for in steady state.
pub struct DirectoryPoll {
    bin: PathBuf,
    sidebar_id: String,
    /// `None` when the host exposed no socket path; CLI-only mode.
    rpc: Option<super::rpc::HerdrRpc>,
}

impl DirectoryPoll {
    /// Build a poll for `sidebar_id`, preferring `socket` when it is known.
    #[must_use]
    pub fn new(bin: PathBuf, sidebar_id: String, socket: Option<PathBuf>) -> Self {
        Self {
            bin,
            sidebar_id,
            rpc: socket.map(super::rpc::HerdrRpc::new),
        }
    }

    /// Current shell directory, or `None` when no shell pane is designated.
    ///
    /// # Errors
    /// Returns a message when both the socket and the CLI fallback fail, or
    /// when the host response is malformed.
    pub fn read(&self) -> Result<Option<PathBuf>, String> {
        if let Some(rpc) = self.rpc.as_ref() {
            // Transport failure only: fall through to the CLI so a stale
            // socket path still degrades to working behaviour. A reachable
            // socket that answers with garbage is a real error and is
            // returned as such instead of being masked by a CLI retry.
            if let Ok(response) = rpc.call("pane.list") {
                return super::launch::shell_cwd(&response, &self.sidebar_id);
            }
        }
        shell_cwd(&self.bin, &self.sidebar_id)
    }
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
