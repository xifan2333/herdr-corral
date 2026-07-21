//! Host boundary: plugin mode vs standalone mode.
//!
//! Corral is first a terminal TUI. Herdr is an optional host:
//! - **plugin mode**: launched by Herdr with `HERDR_ENV=1` / plugin env vars
//! - **standalone mode**: run as a normal binary (`corral` / `cargo run`)
//!
//! Malformed or missing host input degrades to a minimal context (process cwd).
//! Never panics.

use serde::Deserialize;
use std::path::PathBuf;

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
    pub fn herdr_bin(&self) -> Option<&PathBuf> {
        self.herdr_bin.as_ref()
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

    ctx.plugin_id = std::env::var("HERDR_PLUGIN_ID").ok().filter(|s| !s.is_empty());
    ctx.entrypoint_id = std::env::var("HERDR_PLUGIN_ENTRYPOINT_ID")
        .ok()
        .filter(|s| !s.is_empty());
    ctx.herdr_bin = std::env::var("HERDR_BIN_PATH")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);

    // Env vars can fill ids when JSON is partial/absent.
    if ctx.workspace_id.is_none() {
        ctx.workspace_id = std::env::var("HERDR_WORKSPACE_ID")
            .ok()
            .filter(|s| !s.is_empty());
    }
    if ctx.tab_id.is_none() {
        ctx.tab_id = std::env::var("HERDR_TAB_ID").ok().filter(|s| !s.is_empty());
    }
    if ctx.focused_pane_id.is_none() {
        ctx.focused_pane_id = std::env::var("HERDR_PANE_ID")
            .ok()
            .filter(|s| !s.is_empty());
    }

    ctx
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
        focused_pane_id: raw.focused_pane_id.filter(|s| !s.is_empty()),
        plugin_id: None,
        entrypoint_id: None,
        herdr_bin: None,
    }
}
