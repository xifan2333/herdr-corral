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
        let source = std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_CONFIG.to_string());
        let binds = parse_binds(&source);
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
    // If config had no binds at all, use defaults from embedded config.
    if binds.is_empty() {
        binds = parse_binds(DEFAULT_CONFIG);
    }
    binds
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

/// Default config when the user has not created one yet.
pub const DEFAULT_CONFIG: &str = r#"# Corral config (shell). Not executed at startup — sourced when an action runs.
# bind <key> <action>
#   internal: up down top bottom toggle collapse refresh open
#   shell:    any other name = function of that name below
# Split modules yourself:  source "${CORRAL_CONFIG_DIR}/git.sh"
#
# Env when an action runs:
#   CORRAL_FILE CORRAL_DIR CORRAL_CONFIG_DIR CORRAL_ACTION
#   HERDR_BIN_PATH HERDR_ENV HERDR_PANE_ID (when hosted)
#
# Return convention for shell actions:
#   echo CORRAL_SUSPEND=0  — keep the TUI up (default for Herdr pane ops)
#   echo CORRAL_SUSPEND=1  — TUI leaves alt-screen (standalone $EDITOR)

bind j down
bind down down
bind k up
bind up up
bind g top
bind G bottom
bind h collapse
bind left collapse
bind l toggle
bind right toggle
bind enter toggle
bind r refresh

# Open selected file in $EDITOR — reuse one editor pane when possible.
# Herdr / WezTerm: dock right once, then swap files in that same pane.
# Standalone TTY fallback: suspend and run $EDITOR here.

CORRAL_EDITOR_LABEL="Corral Editor"

open() {
  local file="${1:-${CORRAL_FILE:-}}"
  [[ -n "$file" && -e "$file" ]] || return 1
  local editor="${EDITOR:-${VISUAL:-vi}}"
  local qfile
  qfile=$(printf '%q' "$file")

  # --- Herdr: one labeled editor pane, reuse on later opens ---
  if [[ -n "${HERDR_BIN_PATH:-}" && -n "${HERDR_ENV:-}" ]]; then
    echo CORRAL_SUSPEND=0
    local herdr="$HERDR_BIN_PATH" pid="" out
    # Find existing "Corral Editor" pane (any workspace tab).
    pid="$("$herdr" pane list 2>/dev/null | sed -n 's/.*"label":"'"$CORRAL_EDITOR_LABEL"'".*"pane_id":"\([^"]*\)".*/\1/p' | head -1)"
    # sed above is fragile if key order differs — use python when available.
    if command -v python3 >/dev/null 2>&1; then
      pid="$("$herdr" pane list 2>/dev/null | python3 -c '
import sys, json
label = sys.argv[1]
try:
    d = json.load(sys.stdin)
except Exception:
    raise SystemExit
for p in d.get("result", d).get("panes", []):
    if p.get("label") == label:
        print(p.get("pane_id", "")); break
' "$CORRAL_EDITOR_LABEL" 2>/dev/null || true)"
    fi

    if [[ -z "$pid" ]]; then
      out="$("$herdr" pane split --current --direction right --focus --ratio 0.75 2>&1)" || return 1
      pid="$(printf '%s' "$out" | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d["result"]["pane"]["pane_id"])' 2>/dev/null || true)"
      [[ -n "$pid" ]] || return 1
      "$herdr" pane rename "$pid" "$CORRAL_EDITOR_LABEL" >/dev/null 2>&1 || true
      "$herdr" pane send-text "$pid" "$editor $qfile" >/dev/null
      "$herdr" pane send-keys "$pid" enter >/dev/null
    else
      # Reuse: focus, then :edit for vi-family, else restart editor command.
      "$herdr" pane zoom "$pid" --on >/dev/null 2>&1 || true
      "$herdr" pane zoom "$pid" --off >/dev/null 2>&1 || true
      case "$editor" in
        *nvim*|*vim*|*vi)
          "$herdr" pane send-keys "$pid" esc >/dev/null
          "$herdr" pane send-text "$pid" ":edit $qfile" >/dev/null
          "$herdr" pane send-keys "$pid" enter >/dev/null
          ;;
        *)
          # Interrupt whatever is running, then launch editor on the file.
          "$herdr" pane send-keys "$pid" ctrl+c >/dev/null
          "$herdr" pane send-text "$pid" "$editor $qfile" >/dev/null
          "$herdr" pane send-keys "$pid" enter >/dev/null
          ;;
      esac
    fi
    return 0
  fi

  # --- WezTerm: reuse pane id stored under config dir ---
  if [[ -n "${WEZTERM_PANE:-}" ]] && command -v wezterm >/dev/null 2>&1; then
    echo CORRAL_SUSPEND=0
    local state="${CORRAL_CONFIG_DIR:-/tmp}/wezterm-editor.pane"
    local pid=""
    if [[ -f "$state" ]]; then
      pid="$(cat "$state" 2>/dev/null || true)"
      # Drop stale ids that are no longer in the mux.
      if [[ -n "$pid" ]] && ! wezterm cli list 2>/dev/null | grep -q "^$pid\b\|pane_id=$pid\|$pid "; then
        # list format is a table; match pane id column loosely
        if ! wezterm cli list 2>/dev/null | awk '{print $1}' | grep -qx "$pid"; then
          pid=""
        fi
      fi
    fi
    if [[ -z "$pid" ]]; then
      # shellcheck disable=SC2086
      pid="$(wezterm cli split-pane --right --percent 75 -- $editor "$file" 2>/dev/null | tr -d '[:space:]')" || return 1
      [[ -n "$pid" ]] || return 1
      mkdir -p "${CORRAL_CONFIG_DIR:-/tmp}"
      printf '%s' "$pid" >"$state"
    else
      wezterm cli activate-pane --pane-id "$pid" >/dev/null 2>&1 || true
      case "$editor" in
        *nvim*|*vim*|*vi)
          wezterm cli send-text --pane-id "$pid" --no-paste $'\x1b' 2>/dev/null || true
          wezterm cli send-text --pane-id "$pid" --no-paste ":edit $qfile\r" >/dev/null
          ;;
        *)
          wezterm cli send-text --pane-id "$pid" --no-paste $'\x03' 2>/dev/null || true
          # shellcheck disable=SC2086
          wezterm cli send-text --pane-id "$pid" --no-paste "$editor $qfile\r" >/dev/null
          ;;
      esac
    fi
    return 0
  fi

  # --- Fallback: editor in this TTY ---
  echo CORRAL_SUSPEND=1
  # shellcheck disable=SC2086
  exec $editor "$file"
}
"#;
