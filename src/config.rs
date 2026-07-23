//! User config: one shell file (`config.sh`), River-style.
//!
//! ```sh
//! # Herdr plugin: $HERDR_PLUGIN_CONFIG_DIR/config.sh
//! #   (~/.config/herdr/plugins/config/corral/config.sh)
//! # standalone fallback: ~/.config/corral/config.sh
//! corral bind enter open
//! corral bind j down
//!
//! open() { ${EDITOR:-vi} "$CORRAL_FILE"; }
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
use std::time::{Duration, Instant};

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
    pub const SCM_SUGGEST_MESSAGE: &str = "scm-suggest-message";
    pub const EXPLORER_CREATE: &str = "explorer-create";
    pub const EXPLORER_DELETE: &str = "explorer-delete";
    pub const EXPLORER_RENAME: &str = "explorer-rename";
    pub const GITHUB_ISSUES: &str = "github-issues";
    pub const GITHUB_PULLS: &str = "github-pulls";
    pub const GITHUB_ACTIONS: &str = "github-actions";
    pub const GITHUB_WORKFLOWS: &str = "github-workflows";
    pub const GITHUB_VIEW: &str = "github-view";
    pub const GITHUB_DIFF: &str = "github-diff";
    pub const GITHUB_CHECKS: &str = "github-checks";
    pub const GITHUB_LOG: &str = "github-log";
    pub const GITHUB_LOG_FAILED: &str = "github-log-failed";
    pub const GITHUB_FILTER: &str = "github-filter";
    pub const GITHUB_FILTER_APPLY: &str = "github-filter-apply";
    pub const GITHUB_FILTER_CANCEL: &str = "github-filter-cancel";
    pub const GITHUB_LOAD_MORE: &str = "github-load-more";
    pub const GITHUB_CYCLE_STATE: &str = "github-cycle-state";
    pub const GITHUB_NEXT_SECTION: &str = "github-next-section";
    pub const GITHUB_PREV_SECTION: &str = "github-prev-section";
    pub const GITHUB_COMMENT: &str = "github-comment";
    pub const GITHUB_APPROVE: &str = "github-approve";
    pub const GITHUB_CONTEXT_ACTION: &str = "github-context-action";
    pub const GITHUB_CLOSE_REOPEN: &str = "github-close-reopen";
    pub const GITHUB_MERGE: &str = "github-merge";
    pub const GITHUB_RERUN_FAILED: &str = "github-rerun-failed";
    pub const GITHUB_RERUN_ALL: &str = "github-rerun-all";
    pub const GITHUB_WORKFLOW_DISPATCH: &str = "github-workflow-dispatch";
    pub const GITHUB_SUBMIT: &str = "github-submit";
    pub const GITHUB_CONFIRM: &str = "github-confirm";
    pub const GITHUB_CANCEL: &str = "github-cancel";
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
        let default_path = shipped_default();

        // Seed the user config from the shipped default on first run, so it can
        // be edited on disk without recompiling Corral.
        if !path.exists() {
            if let Some(def) = default_path.as_ref() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::copy(def, &path);
            }
        }

        // Load user config; fall back to the shipped default file if the user
        // copy is still missing (e.g. seeding failed / read-only dir).
        let default_source = default_path
            .as_ref()
            .and_then(|default| std::fs::read_to_string(default).ok());
        let mut source = std::fs::read_to_string(&path)
            .ok()
            .or_else(|| default_source.clone())
            .unwrap_or_default();
        if path.is_file() {
            if let Some(default) = default_source.as_deref() {
                source = migrate_config(&path, &source, default);
            }
        }

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

    /// Feature-local binding wins over a global binding for the same key.
    /// Scoped keys retain the two-argument config syntax, for example:
    /// `corral bind explorer:a explorer-create`.
    pub fn action_for_feature_key(&self, feature: &str, key_token: &str) -> Option<&str> {
        let scoped = format!(
            "{}:{}",
            feature.to_ascii_lowercase(),
            normalize_key(key_token)
        );
        self.binds
            .get(&scoped)
            .or_else(|| self.binds.get(&normalize_key(key_token)))
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
                | internal::SCM_SUGGEST_MESSAGE
                | internal::EXPLORER_CREATE
                | internal::EXPLORER_DELETE
                | internal::EXPLORER_RENAME
                | internal::GITHUB_ISSUES
                | internal::GITHUB_PULLS
                | internal::GITHUB_ACTIONS
                | internal::GITHUB_WORKFLOWS
                | internal::GITHUB_VIEW
                | internal::GITHUB_DIFF
                | internal::GITHUB_CHECKS
                | internal::GITHUB_LOG
                | internal::GITHUB_LOG_FAILED
                | internal::GITHUB_FILTER
                | internal::GITHUB_FILTER_APPLY
                | internal::GITHUB_FILTER_CANCEL
                | internal::GITHUB_LOAD_MORE
                | internal::GITHUB_CYCLE_STATE
                | internal::GITHUB_NEXT_SECTION
                | internal::GITHUB_PREV_SECTION
                | internal::GITHUB_COMMENT
                | internal::GITHUB_APPROVE
                | internal::GITHUB_CONTEXT_ACTION
                | internal::GITHUB_CLOSE_REOPEN
                | internal::GITHUB_MERGE
                | internal::GITHUB_RERUN_FAILED
                | internal::GITHUB_RERUN_ALL
                | internal::GITHUB_WORKFLOW_DISPATCH
                | internal::GITHUB_SUBMIT
                | internal::GITHUB_CONFIRM
                | internal::GITHUB_CANCEL
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

    /// Run a provider-style shell action off the TUI thread and return stdout.
    /// The action receives the same Corral environment as `run_shell`, but no
    /// TTY. Diagnostics come from stderr and the caller decides how to display
    /// or parse the captured text.
    pub fn run_shell_capture(
        &self,
        action: &str,
        file: Option<&Path>,
        extra_env: &[(String, String)],
    ) -> Result<String, String> {
        if !is_safe_fn_name(action) {
            return Err("unsafe shell action name".into());
        }
        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(format!(
            "set -euo pipefail\n{source}\n\
             if declare -F {action} >/dev/null 2>&1; then {action} \"$@\"; \
             else printf 'corral: no shell action: %s\\n' {action} >&2; exit 127; fi\n",
            source = self.source,
            action = action,
        ));
        cmd.arg("--");
        if let Some(path) = file {
            cmd.arg(path);
        }
        cmd.env("CORRAL_CONFIG_DIR", &self.dir);
        cmd.env("CORRAL_CONFIG", &self.path);
        cmd.env("CORRAL_ACTION", action);
        cmd.env("PATH", path_with_exe_dir());
        if let Some(path) = file {
            cmd.env("CORRAL_FILE", path);
            if let Some(parent) = path.parent() {
                cmd.env("CORRAL_DIR", parent);
            }
        }
        for (key, value) in extra_env {
            cmd.env(key, value);
        }
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("suggestion command: {error}"))?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() < Duration::from_secs(60) => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("suggestion command timed out after 60 seconds".into());
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("suggestion command: {error}"));
                }
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("suggestion command: {error}"))?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if error.is_empty() {
                format!("suggestion command exited with {}", output.status)
            } else {
                error
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            Err("suggestion command returned no message".into())
        } else {
            Ok(stdout)
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

/// XDG user config: `~/.config/corral/config.sh`.
fn config_dir() -> PathBuf {
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

const GITHUB_CONFIG_VERSION: u32 = 14;
const GITHUB_FUNCTION_BEGIN: &str = "# CORRAL_MIGRATION_V6_FUNCTION_BEGIN";
const GITHUB_FUNCTION_END: &str = "# CORRAL_MIGRATION_V6_FUNCTION_END";
const GITHUB_DETAIL_BEGIN: &str = "# CORRAL_MIGRATION_V8_FUNCTION_BEGIN";
const GITHUB_DETAIL_END: &str = "# CORRAL_MIGRATION_V8_FUNCTION_END";
const GITHUB_DEFAULT_BINDS: [(&str, &str); 38] = [
    ("github:i", internal::GITHUB_ISSUES),
    ("github:p", internal::GITHUB_PULLS),
    ("github:a", internal::GITHUB_ACTIONS),
    ("github:w", internal::GITHUB_WORKFLOWS),
    ("github:enter", internal::GITHUB_VIEW),
    ("github:o", internal::GITHUB_VIEW),
    ("github:d", internal::GITHUB_DIFF),
    ("github:c", internal::GITHUB_CHECKS),
    ("github:x", internal::GITHUB_LOG_FAILED),
    ("github:L", internal::GITHUB_LOG),
    ("github:f", internal::GITHUB_FILTER),
    ("github:]", internal::GITHUB_LOAD_MORE),
    ("github:s", internal::GITHUB_CYCLE_STATE),
    ("github:t", internal::GITHUB_WORKFLOW_DISPATCH),
    ("github:tab", internal::GITHUB_NEXT_SECTION),
    ("github:backtab", internal::GITHUB_PREV_SECTION),
    ("github:y", internal::GITHUB_CONFIRM),
    ("github:n", internal::GITHUB_FILTER_CANCEL),
    ("github:esc", internal::GITHUB_FILTER_CANCEL),
    ("github-detail:d", internal::GITHUB_DIFF),
    ("github-detail:C", internal::GITHUB_CHECKS),
    ("github-detail:f", internal::GITHUB_LOG_FAILED),
    ("github-detail:L", internal::GITHUB_LOG),
    ("github-detail:tab", internal::GITHUB_NEXT_SECTION),
    ("github-detail:backtab", internal::GITHUB_PREV_SECTION),
    ("github-detail:c", internal::GITHUB_COMMENT),
    ("github-detail:o", internal::OPEN),
    ("github-detail:a", internal::GITHUB_APPROVE),
    ("github-detail:x", internal::GITHUB_CONTEXT_ACTION),
    ("github-detail:D", internal::GITHUB_CLOSE_REOPEN),
    ("github-detail:m", internal::GITHUB_MERGE),
    ("github-detail:R", internal::GITHUB_RERUN_FAILED),
    ("github-detail:A", internal::GITHUB_RERUN_ALL),
    ("github-detail:ctrl+enter", internal::GITHUB_SUBMIT),
    ("github-detail:ctrl+s", internal::GITHUB_SUBMIT),
    ("github-detail:y", internal::GITHUB_CONFIRM),
    ("github-detail:n", internal::GITHUB_CANCEL),
    ("github-detail:esc", internal::GITHUB_CANCEL),
];

fn config_version(source: &str) -> u32 {
    source
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("CORRAL_CONFIG_VERSION=")
                .and_then(|value| value.trim_matches(['\'', '"']).parse().ok())
        })
        .unwrap_or(0)
}

fn marked_block<'a>(source: &'a str, begin: &str, end: &str) -> Option<&'a str> {
    let start = source.find(begin)?;
    let tail = &source[start..];
    let finish = tail.find(end)?.saturating_add(end.len());
    Some(&tail[..finish])
}

fn textual_binds(source: &str) -> HashMap<String, String> {
    source
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            if words.next()? != "corral" || words.next()? != "bind" {
                return None;
            }
            let key = words.next()?;
            let action = words.next()?;
            Some((normalize_binding_key(key), action.to_string()))
        })
        .collect()
}

fn remove_stock_binding(source: &str, key: &str, action: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let mut words = line.split_whitespace();
            !(words.next() == Some("corral")
                && words.next() == Some("bind")
                && words.next() == Some(key)
                && words.next() == Some(action))
        })
        .fold(String::new(), |mut output, line| {
            output.push_str(line);
            output.push('\n');
            output
        })
}

fn declares_function(source: &str, name: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with(&format!("{name}()")) || line.starts_with(&format!("function {name}"))
    })
}

fn set_config_version(source: &str, version: u32) -> String {
    let mut replaced = false;
    let mut output = String::new();
    for line in source.lines() {
        if line.trim_start().starts_with("CORRAL_CONFIG_VERSION=") {
            output.push_str(&format!("CORRAL_CONFIG_VERSION={version}\n"));
            replaced = true;
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }
    if !replaced {
        output.insert_str(0, &format!("CORRAL_CONFIG_VERSION={version}\n"));
    }
    output
}

/// Apply one known migration without replacing user bindings or functions.
/// A backup is retained beside the config; failure to persist still uses the
/// migrated source for this process so an old read-only config degrades safely.
fn migrate_config(path: &Path, source: &str, default: &str) -> String {
    if config_version(source) >= GITHUB_CONFIG_VERSION
        || config_version(default) < GITHUB_CONFIG_VERSION
    {
        return source.to_string();
    }

    // v6 used `github:l` for failed logs, shadowing the tree's standard expand
    // key. Remove only that exact stock binding; custom `github:l` actions stay.
    let declared = textual_binds(source);
    let source =
        if declared.get("github:l").map(String::as_str) == Some(internal::GITHUB_LOG_FAILED) {
            remove_stock_binding(source, "github:l", internal::GITHUB_LOG_FAILED)
        } else {
            source.to_string()
        };
    // v8 temporarily used detail `x` for failed logs. `x` is now the
    // resource-context action; failed logs use `f`.
    let source = if textual_binds(&source)
        .get("github-detail:x")
        .map(String::as_str)
        == Some(internal::GITHUB_LOG_FAILED)
    {
        remove_stock_binding(&source, "github-detail:x", internal::GITHUB_LOG_FAILED)
    } else {
        source
    };
    let mut existing = collect_binds(&source);
    // Preserve declarative binds even when migration runs in an install/test
    // context where the `corral bind` helper is temporarily unavailable.
    existing.extend(textual_binds(&source));
    let mut migrated = source.trim_end().to_string();
    migrated.push_str("\n\n# Corral automatic migration v14: GitHub image text links + imv.\n");
    for (key, action) in GITHUB_DEFAULT_BINDS {
        if !existing.contains_key(key) {
            migrated.push_str(&format!("corral bind {key} {action}\n"));
        }
    }
    if !declares_function(&source, "github_preview") {
        if let Some(block) = marked_block(default, GITHUB_FUNCTION_BEGIN, GITHUB_FUNCTION_END) {
            migrated.push('\n');
            migrated.push_str(block);
            migrated.push('\n');
        }
    }
    // Always refresh the marked github_detail block so image viewer env
    // injection (CORRAL_GITHUB_IMAGE_VIEWER) lands without clobbering user binds.
    if let Some(block) = marked_block(default, GITHUB_DETAIL_BEGIN, GITHUB_DETAIL_END) {
        if let (Some(start), Some(end)) = (
            migrated.find(GITHUB_DETAIL_BEGIN),
            migrated.find(GITHUB_DETAIL_END),
        ) {
            if start < end {
                let end = end + GITHUB_DETAIL_END.len();
                migrated.replace_range(start..end, block.trim_end());
            }
        } else if !declares_function(&source, "github_detail") {
            migrated.push('\n');
            migrated.push_str(block);
            migrated.push('\n');
        }
    }
    migrated = set_config_version(&migrated, GITHUB_CONFIG_VERSION);

    let backup = path.with_extension(format!("sh.v{}.bak", config_version(&source)));
    if !backup.exists() {
        let _ = std::fs::copy(path, &backup);
    }
    let temp = path.with_extension(format!("sh.migrate-{}", std::process::id()));
    let persisted = (|| -> std::io::Result<()> {
        std::fs::write(&temp, &migrated)?;
        if let Ok(metadata) = std::fs::metadata(path) {
            std::fs::set_permissions(&temp, metadata.permissions())?;
        }
        std::fs::rename(&temp, path)
    })();
    if persisted.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    migrated
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
                        binds.insert(normalize_binding_key(k), a.to_string());
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
            let _ = writeln!(f, "{}\t{}", normalize_binding_key(key), action);
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

fn normalize_binding_key(s: &str) -> String {
    match s.split_once(':') {
        Some((feature, key)) if !feature.is_empty() && !key.is_empty() => {
            format!("{}:{}", feature.to_ascii_lowercase(), normalize_key(key))
        }
        _ => normalize_key(s),
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
        ("explorer:a", internal::EXPLORER_CREATE),
        ("explorer:d", internal::EXPLORER_DELETE),
        ("explorer:r", internal::EXPLORER_RENAME),
        ("github:i", internal::GITHUB_ISSUES),
        ("github:p", internal::GITHUB_PULLS),
        ("github:a", internal::GITHUB_ACTIONS),
        ("github:w", internal::GITHUB_WORKFLOWS),
        ("github:enter", internal::GITHUB_VIEW),
        ("github:o", internal::GITHUB_VIEW),
        ("github:d", internal::GITHUB_DIFF),
        ("github:c", internal::GITHUB_CHECKS),
        ("github:x", internal::GITHUB_LOG_FAILED),
        ("github:L", internal::GITHUB_LOG),
        ("github:f", internal::GITHUB_FILTER),
        ("github:]", internal::GITHUB_LOAD_MORE),
        ("github:s", internal::GITHUB_CYCLE_STATE),
        ("github:t", internal::GITHUB_WORKFLOW_DISPATCH),
        ("github:tab", internal::GITHUB_NEXT_SECTION),
        ("github:backtab", internal::GITHUB_PREV_SECTION),
        ("github:y", internal::GITHUB_CONFIRM),
        ("github:n", internal::GITHUB_FILTER_CANCEL),
        ("github:esc", internal::GITHUB_FILTER_CANCEL),
        ("github-detail:d", internal::GITHUB_DIFF),
        ("github-detail:C", internal::GITHUB_CHECKS),
        ("github-detail:x", internal::GITHUB_LOG_FAILED),
        ("github-detail:L", internal::GITHUB_LOG),
        ("github-detail:tab", internal::GITHUB_NEXT_SECTION),
        ("github-detail:backtab", internal::GITHUB_PREV_SECTION),
        ("github-detail:c", internal::GITHUB_COMMENT),
        ("github-detail:o", internal::OPEN),
        ("github-detail:a", internal::GITHUB_APPROVE),
        ("github-detail:x", internal::GITHUB_CONTEXT_ACTION),
        ("github-detail:D", internal::GITHUB_CLOSE_REOPEN),
        ("github-detail:m", internal::GITHUB_MERGE),
        ("github-detail:R", internal::GITHUB_RERUN_FAILED),
        ("github-detail:A", internal::GITHUB_RERUN_ALL),
        ("github-detail:f", internal::GITHUB_LOG_FAILED),
        ("github-detail:ctrl+enter", internal::GITHUB_SUBMIT),
        ("github-detail:ctrl+s", internal::GITHUB_SUBMIT),
        ("github-detail:y", internal::GITHUB_CONFIRM),
        ("github-detail:n", internal::GITHUB_CANCEL),
        ("github-detail:esc", internal::GITHUB_CANCEL),
        ("u", internal::SCM_UNSTAGE_ALL),
        ("o", internal::SCM_OPEN_DIFF),
        ("c", internal::SCM_FOCUS_MESSAGE),
        ("D", internal::SCM_DISCARD),
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
        // or /usr/bin/corral -> /usr/share/herdr-corral/config.default.sh.
        let candidates = [
            exe.parent().map(|d| d.join("config.default.sh")),
            exe.parent()
                .and_then(|d| d.parent())
                .map(|d| d.join("config.default.sh")),
            exe.parent()
                .and_then(|d| d.parent())
                .map(|d| d.join("share/herdr-corral/config.default.sh")),
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

    #[test]
    fn scoped_bindings_override_globals_without_losing_key_case() {
        let mut config = Config::for_test();
        config
            .binds
            .insert("explorer:a".into(), internal::EXPLORER_CREATE.into());
        config
            .binds
            .insert("explorer:G".into(), internal::BOTTOM.into());
        assert_eq!(
            config.action_for_feature_key("explorer", "a"),
            Some(internal::EXPLORER_CREATE)
        );
        assert_eq!(
            config.action_for_feature_key("scm", "a"),
            Some(internal::SCM_STAGE_ALL)
        );
        assert_eq!(normalize_binding_key("Explorer:G"), "explorer:G");
        assert_eq!(
            config.action_for_feature_key("explorer", "G"),
            Some(internal::BOTTOM)
        );
    }

    fn migration_default() -> &'static str {
        "CORRAL_CONFIG_VERSION=14\n# CORRAL_MIGRATION_V6_FUNCTION_BEGIN\ngithub_preview() { printf default; }\n# CORRAL_MIGRATION_V6_FUNCTION_END\n# CORRAL_MIGRATION_V8_FUNCTION_BEGIN\nCORRAL_GITHUB_IMAGE_VIEWER=\"${CORRAL_GITHUB_IMAGE_VIEWER:-imv}\"\ngithub_detail() { printf detail; }\n# CORRAL_MIGRATION_V8_FUNCTION_END\n"
    }

    #[test]
    fn migration_preserves_custom_github_bind_and_function() {
        let path = tempfile_path("corral-config-migrate").unwrap();
        let source = "CORRAL_CONFIG_VERSION=5\ncorral bind github:i my-issues\ngithub_preview() { printf custom; }\n";
        std::fs::write(&path, source).unwrap();
        let migrated = migrate_config(&path, source, migration_default());
        let binds = textual_binds(&migrated);
        assert_eq!(binds.get("github:i").map(String::as_str), Some("my-issues"));
        assert!(migrated.contains("github_preview() { printf custom; }"));
        assert!(!migrated.contains("github_preview() { printf default; }"));
        assert!(migrated.contains("github_detail() { printf detail; }"));
        assert_eq!(config_version(&migrated), 14);
        let _ = std::fs::remove_file(path.with_extension("sh.v5.bak"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn migration_adds_missing_bindings_and_detail_function() {
        let path = tempfile_path("corral-config-migrate").unwrap();
        let source = "CORRAL_CONFIG_VERSION=5\ncorral bind q quit\n";
        std::fs::write(&path, source).unwrap();
        let migrated = migrate_config(&path, source, migration_default());
        let binds = textual_binds(&migrated);
        assert_eq!(
            binds.get("github-detail:d").map(String::as_str),
            Some(internal::GITHUB_DIFF)
        );
        assert_eq!(
            binds.get("github:t").map(String::as_str),
            Some(internal::GITHUB_WORKFLOW_DISPATCH)
        );
        assert!(migrated.contains("github_preview() { printf default; }"));
        assert!(migrated.contains("github_detail() { printf detail; }"));
        assert!(path.with_extension("sh.v5.bak").is_file());
        let _ = std::fs::remove_file(path.with_extension("sh.v5.bak"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn migration_releases_stock_l_binding_but_preserves_custom_l() {
        let path = tempfile_path("corral-config-migrate").unwrap();
        let stock = "CORRAL_CONFIG_VERSION=7\ncorral bind github:l github-log-failed\ngithub_preview() { :; }\n";
        std::fs::write(&path, stock).unwrap();
        let migrated = migrate_config(&path, stock, migration_default());
        let binds = textual_binds(&migrated);
        assert!(!binds.contains_key("github:l"));
        assert_eq!(
            binds.get("github:x").map(String::as_str),
            Some(internal::GITHUB_LOG_FAILED)
        );

        let custom =
            "CORRAL_CONFIG_VERSION=7\ncorral bind github:l my-expand\ngithub_preview() { :; }\n";
        std::fs::write(&path, custom).unwrap();
        let migrated = migrate_config(&path, custom, migration_default());
        assert_eq!(
            textual_binds(&migrated).get("github:l").map(String::as_str),
            Some("my-expand")
        );
        let _ = std::fs::remove_file(path.with_extension("sh.v7.bak"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn migration_refreshes_marked_github_detail_block() {
        let path = tempfile_path("corral-config-migrate").unwrap();
        let source = "CORRAL_CONFIG_VERSION=12\n# CORRAL_MIGRATION_V8_FUNCTION_BEGIN\ngithub_detail() { printf stale; }\n# CORRAL_MIGRATION_V8_FUNCTION_END\n";
        std::fs::write(&path, source).unwrap();
        let migrated = migrate_config(&path, source, migration_default());
        assert!(migrated.contains("github_detail() { printf detail; }"));
        assert!(migrated.contains("CORRAL_GITHUB_IMAGE_VIEWER"));
        assert!(!migrated.contains("printf stale"));
        assert_eq!(config_version(&migrated), 14);
        let _ = std::fs::remove_file(path.with_extension("sh.v12.bak"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn migration_moves_detail_failed_log_off_context_key() {
        let path = tempfile_path("corral-config-migrate").unwrap();
        let source = "CORRAL_CONFIG_VERSION=8\ncorral bind github-detail:x github-log-failed\ngithub_preview() { :; }\ngithub_detail() { :; }\n";
        std::fs::write(&path, source).unwrap();
        let migrated = migrate_config(&path, source, migration_default());
        let binds = textual_binds(&migrated);
        assert_eq!(
            binds.get("github-detail:x").map(String::as_str),
            Some(internal::GITHUB_CONTEXT_ACTION)
        );
        assert_eq!(
            binds.get("github-detail:f").map(String::as_str),
            Some(internal::GITHUB_LOG_FAILED)
        );
        let _ = std::fs::remove_file(path.with_extension("sh.v8.bak"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shell_capture_returns_provider_output_and_errors() {
        let mut config = Config::for_test();
        config.source = "suggest_commit_message() { printf 'Fix generated message\\n'; }".into();
        assert_eq!(
            config
                .run_shell_capture("suggest_commit_message", None, &[])
                .unwrap(),
            "Fix generated message"
        );

        config.source =
            "suggest_commit_message() { printf 'provider failed\\n' >&2; return 9; }".into();
        assert_eq!(
            config
                .run_shell_capture("suggest_commit_message", None, &[])
                .unwrap_err(),
            "provider failed"
        );
    }
}
