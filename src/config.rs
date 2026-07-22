//! User config: one shell file (`config.sh`), River-style.
//!
//! ```sh
//! # ~/.config/corral/config.sh  — or $HERDR_PLUGIN_CONFIG_DIR/config.sh
//! corral bind enter open
//! corral bind j down
//!
//! open() { ${EDITOR:-vi} "$CORRAL_FILE"; }
//! # optional: source "${CORRAL_CONFIG_DIR}/git.sh"
//! ```
//!
//! Like River's `riverctl map …`: the config is a script that calls
//! `corral bind <key> <action>`. Corral runs it once in *register mode* at
//! startup to collect the binds; the same `corral bind` calls are no-ops when
//! the file is later sourced to run an action function.
//!
//! - **internal** actions (`up`/`down`/`toggle`/…) stay in Rust
//! - any other action name is a **shell function** of that name in config.sh

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Internal actions handled by the TUI (not shell).
pub mod internal {
    pub const QUIT: &str = "quit";
    pub const FEATURE_EXPLORER: &str = "feature-explorer";
    pub const FEATURE_SCM: &str = "feature-scm";
    pub const FEATURE_GITHUB: &str = "feature-github";
    pub const SCM_TOGGLE_STAGE: &str = "scm-toggle-stage";
    pub const SCM_STAGE_ALL: &str = "scm-stage-all";
    pub const SCM_UNSTAGE_ALL: &str = "scm-unstage-all";
    pub const SCM_OPEN_DIFF: &str = "scm-open-diff";
    pub const SCM_FOCUS_MESSAGE: &str = "scm-focus-message";
    pub const SCM_COMMIT: &str = "scm-commit";
    pub const SCM_DISCARD: &str = "scm-discard";
    pub const SCM_CONFIRM: &str = "scm-confirm";
    pub const SCM_CANCEL: &str = "scm-cancel";
    pub const SCM_SYNC: &str = "scm-sync";
    pub const EDIT_BACKSPACE: &str = "edit-backspace";
    pub const EDIT_DELETE: &str = "edit-delete";
    pub const EDIT_HOME: &str = "edit-home";
    pub const EDIT_END: &str = "edit-end";
    pub const UP: &str = "up";
    pub const DOWN: &str = "down";
    pub const TOP: &str = "top";
    pub const BOTTOM: &str = "bottom";
    pub const PAGE_UP: &str = "page-up";
    pub const PAGE_DOWN: &str = "page-down";
    pub const TOGGLE: &str = "toggle";
    pub const EXPAND: &str = "expand";
    pub const COLLAPSE: &str = "collapse";
    pub const COLLAPSE_ALL: &str = "collapse-all";
    pub const TOGGLE_HIDDEN: &str = "toggle-hidden";
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

        // River-style: run the config once in register mode; `corral bind …`
        // calls record key→action. Fall back to built-in nav binds on failure.
        let mut binds = collect_binds(&source);
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

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            dir: PathBuf::new(),
            path: PathBuf::new(),
            source: String::new(),
            binds: fallback_binds(),
        }
    }

    /// Action name bound to this key token, if any.
    pub fn action_for_key(&self, key_token: &str) -> Option<&str> {
        self.binds
            .get(&normalize_key(key_token))
            .map(String::as_str)
    }

    pub fn is_internal(action: &str) -> bool {
        matches!(
            action,
            internal::QUIT
                | internal::FEATURE_EXPLORER
                | internal::FEATURE_SCM
                | internal::FEATURE_GITHUB
                | internal::SCM_TOGGLE_STAGE
                | internal::SCM_STAGE_ALL
                | internal::SCM_UNSTAGE_ALL
                | internal::SCM_OPEN_DIFF
                | internal::SCM_FOCUS_MESSAGE
                | internal::SCM_COMMIT
                | internal::SCM_DISCARD
                | internal::SCM_CONFIRM
                | internal::SCM_CANCEL
                | internal::SCM_SYNC
                | internal::EDIT_BACKSPACE
                | internal::EDIT_DELETE
                | internal::EDIT_HOME
                | internal::EDIT_END
                | internal::UP
                | internal::DOWN
                | internal::TOP
                | internal::BOTTOM
                | internal::PAGE_UP
                | internal::PAGE_DOWN
                | internal::TOGGLE
                | internal::EXPAND
                | internal::COLLAPSE
                | internal::COLLAPSE_ALL
                | internal::TOGGLE_HIDDEN
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
        extra_env: &[(String, String)],
        inherit_tty: bool,
    ) -> ShellResult {
        if !is_safe_fn_name(action) {
            return ShellResult {
                ok: false,
                suspend: inherit_tty,
            };
        }

        let mut cmd = Command::new("bash");
        // Guard with `declare -F`: an action whose function is missing from
        // config.sh (e.g. a stale user copy) must fail cleanly, never fall
        // through to a same-named external command like `diff`.
        cmd.arg("-c").arg(format!(
            "set -euo pipefail\n{source}\n\
             if declare -F {action} >/dev/null 2>&1; then {action} \"$@\"; \
             else printf 'corral: no shell action: %s\\n' {action} >&2; exit 127; fi\n",
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
        // Make `corral` resolvable so `corral bind …` (no-op here) never errors.
        cmd.env("PATH", path_with_exe_dir());
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
            return ShellResult { ok, suspend: true };
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

/// Run the config in *register mode* and collect `corral bind` calls.
///
/// Like River: the script calls `corral bind <key> <action>`; in register mode
/// that subcommand appends `key\taction` to `$CORRAL_BINDS_FILE`, which we read
/// back. On any failure returns empty (caller uses [`fallback_binds`]).
fn collect_binds(source: &str) -> HashMap<String, String> {
    let mut binds = HashMap::new();
    if source.trim().is_empty() {
        return binds;
    }
    let Ok(tmp) = tempfile_path("corral-binds") else {
        return binds;
    };

    let status = Command::new("bash")
        .arg("-c")
        .arg(source)
        .env("CORRAL_REGISTER", "1")
        .env("CORRAL_BINDS_FILE", &tmp)
        .env("PATH", path_with_exe_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if status.map(|s| s.success()).unwrap_or(false) || tmp.exists() {
        if let Ok(text) = std::fs::read_to_string(&tmp) {
            for line in text.lines() {
                let mut it = line.splitn(2, '\t');
                if let (Some(k), Some(a)) = (it.next(), it.next()) {
                    let k = k.trim();
                    let a = a.trim();
                    if !k.is_empty() && !a.is_empty() {
                        binds.insert(normalize_key(k), a.to_string());
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    binds
}

/// Subcommand: `corral bind <key> <action>`.
///
/// Records the binding only in register mode (`CORRAL_REGISTER=1` +
/// `CORRAL_BINDS_FILE`). Otherwise it is a harmless no-op, so the same call in
/// a sourced action run does nothing.
pub fn cli_bind(args: &[String]) -> std::io::Result<()> {
    if std::env::var("CORRAL_REGISTER").ok().as_deref() != Some("1") {
        return Ok(());
    }
    let (Some(key), Some(action)) = (args.first(), args.get(1)) else {
        return Ok(());
    };
    if let Ok(path) = std::env::var("CORRAL_BINDS_FILE") {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{}\t{}", normalize_key(key), action);
        }
    }
    Ok(())
}

/// Normalize a key token: single characters keep case (so `g` ≠ `G`); named
/// keys (`enter`, `left`, …) are case-insensitive.
fn normalize_key(s: &str) -> String {
    if s.chars().count() == 1 {
        s.to_string()
    } else {
        s.to_ascii_lowercase()
    }
}

/// PATH with the corral executable's directory prepended, so `corral` resolves.
fn path_with_exe_dir() -> String {
    let existing = std::env::var("PATH").unwrap_or_default();
    match std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        Some(dir) => format!("{}:{existing}", dir.display()),
        None => existing,
    }
}

/// A unique temp file path under the system temp dir.
fn tempfile_path(prefix: &str) -> std::io::Result<PathBuf> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    Ok(std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}")))
}

/// Minimal navigation binds when no config file is found at all (no `open`).
fn fallback_binds() -> HashMap<String, String> {
    let pairs = [
        ("q", internal::QUIT),
        ("ctrl+c", internal::QUIT),
        ("1", internal::FEATURE_EXPLORER),
        ("2", internal::FEATURE_SCM),
        ("3", internal::FEATURE_GITHUB),
        ("s", internal::SCM_TOGGLE_STAGE),
        ("space", internal::SCM_TOGGLE_STAGE),
        ("a", internal::SCM_STAGE_ALL),
        ("u", internal::SCM_UNSTAGE_ALL),
        ("o", internal::SCM_OPEN_DIFF),
        ("c", internal::SCM_FOCUS_MESSAGE),
        ("D", internal::SCM_DISCARD),
        ("y", internal::SCM_CONFIRM),
        ("n", internal::SCM_CANCEL),
        ("esc", internal::SCM_CANCEL),
        ("S", internal::SCM_SYNC),
        ("backspace", internal::EDIT_BACKSPACE),
        ("delete", internal::EDIT_DELETE),
        ("home", internal::EDIT_HOME),
        ("end", internal::EDIT_END),
        ("j", internal::DOWN),
        ("down", internal::DOWN),
        ("k", internal::UP),
        ("up", internal::UP),
        ("g", internal::TOP),
        ("G", internal::BOTTOM),
        ("pageup", internal::PAGE_UP),
        ("pagedown", internal::PAGE_DOWN),
        ("h", internal::COLLAPSE),
        ("left", internal::COLLAPSE),
        ("l", internal::EXPAND),
        ("right", internal::EXPAND),
        ("enter", internal::TOGGLE),
        (".", internal::TOGGLE_HIDDEN),
        ("z", internal::COLLAPSE_ALL),
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

/// Map a crossterm key chord to the token used in `bind` lines.
/// Character case is preserved (`g` ≠ `G`); named keys are lowercase.
pub fn key_token(
    code: crossterm::event::KeyCode,
    mods: crossterm::event::KeyModifiers,
) -> Option<String> {
    use crossterm::event::KeyCode::*;
    use crossterm::event::KeyModifiers;

    let base = match code {
        Char(' ') => "space".into(),
        Char(c) => c.to_string(),
        Enter => "enter".into(),
        Backspace => "backspace".into(),
        Delete => "delete".into(),
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
    };
    let mut prefixes = Vec::new();
    if mods.contains(KeyModifiers::CONTROL) {
        prefixes.push("ctrl");
    }
    if mods.contains(KeyModifiers::ALT) {
        prefixes.push("alt");
    }
    // Terminals normally encode shifted characters in the character itself;
    // retain Shift only for named keys such as shift+tab.
    if mods.contains(KeyModifiers::SHIFT) && !matches!(code, Char(_)) {
        prefixes.push("shift");
    }
    if prefixes.is_empty() {
        Some(base)
    } else {
        Some(format!("{}+{base}", prefixes.join("+")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn key_tokens_preserve_configurable_chords() {
        assert_eq!(
            key_token(KeyCode::Char('c'), KeyModifiers::CONTROL).as_deref(),
            Some("ctrl+c")
        );
        assert_eq!(
            key_token(KeyCode::Char('G'), KeyModifiers::SHIFT).as_deref(),
            Some("G")
        );
        assert_eq!(
            key_token(KeyCode::Char(' '), KeyModifiers::NONE).as_deref(),
            Some("space")
        );
        assert_eq!(
            key_token(KeyCode::PageDown, KeyModifiers::ALT).as_deref(),
            Some("alt+pagedown")
        );
        assert_eq!(
            key_token(KeyCode::Backspace, KeyModifiers::NONE).as_deref(),
            Some("backspace")
        );
    }
}
