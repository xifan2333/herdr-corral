//! User config: one shell file (`config.sh`).
//!
//! ```text
//! # ~/.config/corral/config.sh  — or $HERDR_PLUGIN_CONFIG_DIR/config.sh
//! bind enter open
//! bind j down
//!
//! open() {
//!   ${EDITOR:-vi} "$CORRAL_FILE"
//! }
//!
//! # optional: source ./git.sh
//! ```
//!
//! - `bind <key> <action>` maps keys to action names
//! - **internal** actions (`up`/`down`/`toggle`/…) stay in Rust
//! - any other action name is a **shell function** of the same name in config.sh
//! - if config is missing, an embedded default is used (still pure shell)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Internal actions handled by the TUI (not shell).
pub mod internal {
    pub const UP: &str = "up";
    pub const DOWN: &str = "down";
    pub const TOP: &str = "top";
    pub const BOTTOM: &str = "bottom";
    pub const TOGGLE: &str = "toggle";
    pub const COLLAPSE: &str = "collapse";
    pub const REFRESH: &str = "refresh";
    pub const OPEN: &str = "open";
}

/// Resolved config for this process.
#[derive(Clone, Debug)]
pub struct Config {
    pub dir: PathBuf,
    /// Absolute path to the shell file that was loaded (or would be written).
    pub path: PathBuf,
    /// Full shell text (file or embedded default).
    pub source: String,
    /// key token → action name
    binds: HashMap<String, String>,
}

impl Config {
    pub fn load() -> Self {
        let dir = config_dir();
        let path = dir.join("config.sh");

        // Seed the user config from the shipped default on first run, so it can
        // be edited on disk without recompiling Corral.
        if !path.exists() {
            if let Some(def) = shipped_default() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::copy(&def, &path);
            }
        }

        // Load user config; fall back to the shipped default file if the user
        // copy is still missing (e.g. seeding failed / read-only dir).
        let source = std::fs::read_to_string(&path)
            .ok()
            .or_else(|| shipped_default().and_then(|d| std::fs::read_to_string(d).ok()))
            .unwrap_or_default();

        let mut binds = parse_binds(&source);
        if binds.is_empty() {
            binds = fallback_binds();
        }

        Self {
            dir,
            path,
            source,
            binds,
        }
    }

    /// Action name bound to this key token, if any.
    pub fn action_for_key(&self, key_token: &str) -> Option<&str> {
        self.binds.get(&key_token.to_ascii_lowercase()).map(String::as_str)
    }

    pub fn is_internal(action: &str) -> bool {
        matches!(
            action,
            internal::UP
                | internal::DOWN
                | internal::TOP
                | internal::BOTTOM
                | internal::TOGGLE
                | internal::COLLAPSE
                | internal::REFRESH
        )
    }

    /// Run a shell function `action` defined in config.sh.
    ///
    /// Env always includes `CORRAL_*` and passes through `HERDR_*` from the
    /// process environment. `$1` is the file path when present.
    ///
    /// The action may print `CORRAL_SUSPEND=0|1` on stdout to tell the TUI
    /// whether to leave the alternate screen (standalone editors need 1).
    pub fn run_shell(
        &self,
        action: &str,
        file: Option<&Path>,
        extra_env: &[(&str, String)],
        inherit_tty: bool,
    ) -> ShellResult {
        if !is_safe_fn_name(action) {
            return ShellResult {
                ok: false,
                suspend: inherit_tty,
            };
        }

        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(format!(
            "set -euo pipefail\n{source}\n{action} \"$@\"\n",
            source = self.source,
            action = action,
        ));
        cmd.arg("--");
        if let Some(f) = file {
            cmd.arg(f);
        }

        cmd.env("CORRAL_CONFIG_DIR", &self.dir);
        cmd.env("CORRAL_CONFIG", &self.path);
        cmd.env("CORRAL_ACTION", action);
        if let Some(f) = file {
            cmd.env("CORRAL_FILE", f);
            if let Some(parent) = f.parent() {
                cmd.env("CORRAL_DIR", parent);
            }
        }
        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        if inherit_tty {
            // Standalone $EDITOR needs the real TTY.
            let ok = cmd
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            return ShellResult {
                ok,
                suspend: true,
            };
        }

        // Hosted: capture stdout for CORRAL_SUSPEND=*; keep TUI up by default.
        let output = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let suspend = stdout.lines().any(|l| l.trim() == "CORRAL_SUSPEND=1");
                // Explicit 0 wins as default when mixed; only 1 requests suspend.
                ShellResult {
                    ok: out.status.success(),
                    suspend,
                }
            }
            Err(_) => ShellResult {
                ok: false,
                suspend: false,
            },
        }
    }
}

/// Outcome of running a config.sh action.
#[derive(Clone, Copy, Debug)]
pub struct ShellResult {
    pub ok: bool,
    /// Caller should leave alt-screen while the action runs (standalone editor).
    pub suspend: bool,
}

fn config_dir() -> PathBuf {
    if let Ok(p) = std::env::var("HERDR_PLUGIN_CONFIG_DIR") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("corral");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config/corral");
    }
    PathBuf::from(".corral")
}

fn is_safe_fn_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse `bind <key> <action>` lines. Other lines are left for bash to run.
fn parse_binds(source: &str) -> HashMap<String, String> {
    let mut binds = HashMap::new();
    for raw in source.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("bind ") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let Some(key) = parts.next() else { continue };
        let Some(action) = parts.next() else { continue };
        binds.insert(key.to_ascii_lowercase(), action.to_string());
    }
    binds
}

/// Minimal navigation binds when no config file is found at all (no `open`).
fn fallback_binds() -> HashMap<String, String> {
    let pairs = [
        ("j", internal::DOWN),
        ("down", internal::DOWN),
        ("k", internal::UP),
        ("up", internal::UP),
        ("g", internal::TOP),
        ("G", internal::BOTTOM),
        ("h", internal::COLLAPSE),
        ("left", internal::COLLAPSE),
        ("l", internal::TOGGLE),
        ("right", internal::TOGGLE),
        ("enter", internal::TOGGLE),
        ("r", internal::REFRESH),
    ];
    pairs
        .iter()
        .map(|(k, a)| (k.to_string(), (*a).to_string()))
        .collect()
}

/// Locate the shipped `config.default.sh` (never embedded in the binary).
fn shipped_default() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CORRAL_DEFAULT_CONFIG") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(root) = std::env::var("HERDR_PLUGIN_ROOT") {
        if !root.is_empty() {
            let p = PathBuf::from(root).join("config.default.sh");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        // e.g. target/release/corral -> repo/config.default.sh (dev),
        // or <prefix>/bin/corral -> <prefix>/share/corral/config.default.sh.
        let candidates = [
            exe.parent().map(|d| d.join("config.default.sh")),
            exe.parent()
                .and_then(|d| d.parent())
                .map(|d| d.join("config.default.sh")),
            exe.parent()
                .and_then(|d| d.parent())
                .and_then(|d| d.parent())
                .map(|d| d.join("config.default.sh")),
        ];
        for cand in candidates.into_iter().flatten() {
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Map a crossterm key to a lowercase token used in `bind` lines.
pub fn key_token(code: crossterm::event::KeyCode) -> Option<String> {
    use crossterm::event::KeyCode::*;
    Some(match code {
        Char(c) => c.to_string(),
        Enter => "enter".into(),
        Left => "left".into(),
        Right => "right".into(),
        Up => "up".into(),
        Down => "down".into(),
        PageUp => "pageup".into(),
        PageDown => "pagedown".into(),
        Home => "home".into(),
        End => "end".into(),
        Esc => "esc".into(),
        Tab => "tab".into(),
        BackTab => "backtab".into(),
        _ => return None,
    })
}
