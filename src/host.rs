//! Host boundary: plugin mode vs standalone mode.
//!
//! Corral is first a terminal TUI. Herdr is an optional host:
//! - **plugin mode**: launched by Herdr with `HERDR_ENV=1` / plugin env vars
//! - **standalone mode**: run as a normal binary (`corral` / `cargo run`)
//!
//! Pane identity (do not conflate these):
//! - [`LaunchContext::self_pane_id`] — **this** process's pane (`HERDR_PANE_ID`)
//! - [`LaunchContext::focused_pane_id`] — pane focused at *invocation* time
//!   (from `HERDR_PLUGIN_CONTEXT_JSON`); often a neighbor, not us
//!
//! Malformed or missing host input degrades to a minimal context (process cwd).
//! Never panics.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// How the process was launched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Running inside a Herdr plugin pane / action.
    Plugin,
    /// Running as a normal terminal binary.
    Standalone,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Plugin => "plugin",
            Mode::Standalone => "standalone",
        }
    }
}

/// Normalized launch context for the rest of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchContext {
    pub mode: Mode,
    /// Working directory the UI should root at.
    pub cwd: PathBuf,
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    /// This plugin process's own pane id (`HERDR_PANE_ID`).
    ///
    /// Use for rename / tokens / preview ownership. Never filled from the
    /// launch JSON "focused" pane — that is often a different pane.
    pub self_pane_id: Option<String>,
    /// Pane that was focused when the action/pane was invoked
    /// (`HERDR_PLUGIN_CONTEXT_JSON.focused_pane_id`).
    ///
    /// Useful as a neighbor/dock target; **not** necessarily this process.
    pub focused_pane_id: Option<String>,
    pub plugin_id: Option<String>,
    pub entrypoint_id: Option<String>,
    /// Absolute path to the Herdr binary when available (`HERDR_BIN_PATH`).
    pub herdr_bin: Option<PathBuf>,
}

impl LaunchContext {
    /// True when launched by Herdr as a plugin.
    pub fn is_plugin(&self) -> bool {
        self.mode == Mode::Plugin
    }

    /// Path to `herdr` for CLI callbacks, if known.
    pub fn herdr_bin(&self) -> Option<&Path> {
        self.herdr_bin.as_deref()
    }

    /// This process's pane id, if Herdr injected one.
    pub fn self_pane_id(&self) -> Option<&str> {
        self.self_pane_id.as_deref()
    }

    /// Invocation-time focused pane (neighbor), if known.
    pub fn focused_pane_id(&self) -> Option<&str> {
        self.focused_pane_id.as_deref()
    }
}

/// Shape of `HERDR_PLUGIN_CONTEXT_JSON`. Every field optional; unknown fields ignored.
#[derive(Deserialize, Default)]
struct RawContext {
    focused_pane_cwd: Option<String>,
    workspace_cwd: Option<String>,
    cwd: Option<String>,
    workspace_id: Option<String>,
    tab_id: Option<String>,
    focused_pane_id: Option<String>,
}

/// Detect host mode and build a [`LaunchContext`] from the process environment.
///
/// Never panics. Missing/malformed JSON → process cwd.
pub fn from_env() -> LaunchContext {
    let fallback_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mode = detect_mode();
    let json = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok();
    let mut ctx = parse_context(mode, json.as_deref(), fallback_cwd);

    ctx.plugin_id = env_nonempty("HERDR_PLUGIN_ID");
    ctx.entrypoint_id = env_nonempty("HERDR_PLUGIN_ENTRYPOINT_ID");
    ctx.herdr_bin = env_nonempty("HERDR_BIN_PATH").map(PathBuf::from);

    // Own pane: only HERDR_PANE_ID (never the JSON focused pane).
    ctx.self_pane_id = env_nonempty("HERDR_PANE_ID");

    // Fill workspace/tab from env when JSON omitted them.
    if ctx.workspace_id.is_none() {
        ctx.workspace_id = env_nonempty("HERDR_WORKSPACE_ID");
    }
    if ctx.tab_id.is_none() {
        ctx.tab_id = env_nonempty("HERDR_TAB_ID");
    }
    // focused_pane_id stays JSON-only (see field docs). Do not backfill from
    // HERDR_PANE_ID — that would re-alias self vs focused.

    ctx
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

fn detect_mode() -> Mode {
    // Herdr injects HERDR_ENV=1 for plugin runtime commands. Also treat presence
    // of HERDR_PLUGIN_ID as plugin mode (defensive).
    if std::env::var_os("HERDR_ENV").is_some() || std::env::var_os("HERDR_PLUGIN_ID").is_some() {
        Mode::Plugin
    } else {
        Mode::Standalone
    }
}

/// Pure parser (testable without process env).
fn parse_context(mode: Mode, json: Option<&str>, fallback_cwd: PathBuf) -> LaunchContext {
    let raw: RawContext = json
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let cwd = raw
        .focused_pane_cwd
        .filter(|s| !s.is_empty())
        .or_else(|| raw.workspace_cwd.filter(|s| !s.is_empty()))
        .or_else(|| raw.cwd.filter(|s| !s.is_empty()))
        .map(PathBuf::from)
        .unwrap_or(fallback_cwd);

    LaunchContext {
        mode,
        cwd,
        workspace_id: raw.workspace_id.filter(|s| !s.is_empty()),
        tab_id: raw.tab_id.filter(|s| !s.is_empty()),
        self_pane_id: None,
        focused_pane_id: raw.focused_pane_id.filter(|s| !s.is_empty()),
        plugin_id: None,
        entrypoint_id: None,
        herdr_bin: None,
    }
}
