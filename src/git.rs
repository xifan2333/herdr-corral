//! Git plumbing for the SCM feature: repo discovery, `status --porcelain -z`
//! parsing, and the stage / unstage operations, all via the `git` CLI (no
//! libgit2). Parsing is pure and unit-tested; commands run with the repo
//! toplevel as cwd so the repo-relative paths porcelain reports resolve even
//! when the pane's cwd is a subdirectory.
//!
//! Scope split (see design notes): reading and mutating the index live in
//! Rust for instant refresh; diff rendering and commit go through `config.sh`
//! shell functions (they need a reused pane / `$EDITOR`).

use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// One file in the staged or unstaged list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Repo-relative path (the new path, for renames), `/`-separated as git reports it.
    pub path: String,
    /// Rename/copy source, when there is one — unstaging a rename must reset both.
    pub orig: Option<String>,
    /// The VS Code-style status letter to display: M, A, D, R, C, U (untracked),
    /// or `!` for merge conflicts.
    pub letter: char,
}

/// Parsed working-tree status.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: String,
    pub staged: Vec<FileEntry>,
    pub unstaged: Vec<FileEntry>,
    /// Commits ahead of / behind the upstream, from the porcelain `##` header.
    pub ahead: usize,
    pub behind: usize,
    /// The branch has an upstream at all (the header carries `...remote`).
    pub has_upstream: bool,
}

/// A discovered git repository (its toplevel directory).
#[derive(Clone, Debug)]
pub struct Git {
    root: PathBuf,
}

impl Git {
    /// Locate the repository containing `dir`; Err with git's message when there
    /// is none (or git itself is missing).
    pub fn discover(dir: &Path) -> Result<Git, String> {
        let out = run_in(dir, &["rev-parse", "--show-toplevel"])?;
        let root = out.trim();
        if root.is_empty() {
            return Err("not inside a git repository".to_string());
        }
        Ok(Git {
            root: PathBuf::from(root),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Absolute path of a repo-relative entry (for the diff shell action).
    pub fn abs_path(&self, entry: &FileEntry) -> PathBuf {
        self.root.join(&entry.path)
    }

    /// Display name for the repo header: the root directory's name.
    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    pub fn status(&self) -> Result<Status, String> {
        let out = run_in(
            &self.root,
            &[
                "status",
                "--porcelain",
                "-z",
                "--branch",
                "--untracked-files=all",
            ],
        )?;
        Ok(parse_status(&out))
    }

    /// Stage one entry: `add -A` records modifications, additions, and deletions alike.
    pub fn stage(&self, entry: &FileEntry) -> Result<(), String> {
        run_in(&self.root, &["add", "-A", "--", &entry.path]).map(drop)
    }

    pub fn stage_all(&self) -> Result<(), String> {
        run_in(&self.root, &["add", "-A"]).map(drop)
    }

    /// Unstage one entry. `reset` needs a HEAD to reset against; on an unborn
    /// branch (no commits yet) fall back to dropping the path from the index.
    pub fn unstage(&self, entry: &FileEntry) -> Result<(), String> {
        let mut args = vec!["reset", "-q", "--"];
        args.extend(unstage_pathspecs(entry));
        if run_in(&self.root, &args).is_ok() {
            return Ok(());
        }
        run_in(
            &self.root,
            &["rm", "--cached", "-r", "-q", "--", &entry.path],
        )
        .map(drop)
    }

    pub fn unstage_all(&self) -> Result<(), String> {
        if run_in(&self.root, &["reset", "-q"]).is_ok() {
            return Ok(());
        }
        run_in(&self.root, &["rm", "--cached", "-r", "-q", "--", "."]).map(drop)
    }

    /// Permanently discard one unstaged entry. Untracked files are removed;
    /// tracked paths are restored from the index. The UI must confirm first.
    pub fn discard(&self, entry: &FileEntry) -> Result<(), String> {
        if entry.letter == 'U' {
            let path = safe_repo_path(&self.root, &entry.path)?;
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|e| format!("remove {}: {e}", entry.path))?;
            if meta.is_dir() && !meta.file_type().is_symlink() {
                std::fs::remove_dir_all(&path).map_err(|e| format!("remove {}: {e}", entry.path))
            } else {
                std::fs::remove_file(&path).map_err(|e| format!("remove {}: {e}", entry.path))
            }
        } else {
            let mut args = vec!["restore", "--worktree", "--", entry.path.as_str()];
            if entry.letter == 'R' {
                if let Some(orig) = &entry.orig {
                    args.push(orig);
                }
            }
            run_in(&self.root, &args).map(drop)
        }
    }

    pub fn graph(&self, limit: usize) -> Result<Vec<String>, String> {
        git_lines(
            &self.root,
            &[
                "log",
                "--graph",
                "--oneline",
                "--decorate=short",
                &format!("-{limit}"),
            ],
        )
    }

    pub fn commits(&self, limit: usize) -> Result<Vec<String>, String> {
        git_lines(
            &self.root,
            &["log", "--format=%h%x09%d %s", &format!("-{limit}")],
        )
    }

    pub fn file_history(&self, path: &str, limit: usize) -> Result<Vec<String>, String> {
        git_lines(
            &self.root,
            &[
                "log",
                "--format=%h%x09%s",
                "--follow",
                &format!("-{limit}"),
                "--",
                path,
            ],
        )
    }

    pub fn branches(&self) -> Result<Vec<String>, String> {
        git_lines(
            &self.root,
            &[
                "branch",
                "-a",
                "--sort=-committerdate",
                "--format=%(HEAD)%09%(refname:short)",
            ],
        )
    }

    pub fn worktrees(&self) -> Result<Vec<String>, String> {
        let raw = run_in(&self.root, &["worktree", "list", "--porcelain"])?;
        Ok(raw
            .split("\n\n")
            .filter_map(|block| {
                let mut path = None;
                let mut label = None;
                for line in block.lines() {
                    if let Some(value) = line.strip_prefix("worktree ") {
                        path = Some(value.to_string());
                    } else if let Some(value) = line.strip_prefix("branch refs/heads/") {
                        label = Some(value.to_string());
                    } else if line == "detached" {
                        label = Some("(detached)".into());
                    } else if line == "bare" {
                        label = Some("(bare)".into());
                    }
                }
                path.map(|path| format!("{path}\t{}", label.unwrap_or_default()))
            })
            .collect())
    }

    pub fn remotes(&self) -> Result<Vec<String>, String> {
        let lines = git_lines(&self.root, &["remote", "-v"])?;
        let mut seen = std::collections::HashSet::new();
        Ok(lines
            .into_iter()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let name = fields.next()?;
                let url = fields.next()?;
                let kind = fields.next()?;
                if kind != "(fetch)" || !seen.insert(name.to_string()) {
                    return None;
                }
                Some(format!("{name}\t{url}"))
            })
            .collect())
    }

    pub fn stashes(&self) -> Result<Vec<String>, String> {
        git_lines(&self.root, &["stash", "list", "--format=%gd%x09%s"])
    }

    pub fn tags(&self) -> Result<Vec<String>, String> {
        git_lines(
            &self.root,
            &[
                "tag",
                "--sort=-creatordate",
                "--format=%(refname:short)%09%(subject)",
            ],
        )
    }

    /// Synchronize the current branch without blocking the TUI caller.
    /// The view runs this method on a worker thread.
    pub fn sync(&self, status: &Status) -> Result<String, String> {
        if !status.has_upstream {
            return Err("branch has no upstream".into());
        }
        // Refresh divergence first; the status snapshot captured by the TUI can
        // predate a remote update and must not decide whether to pull.
        run_in(&self.root, &["fetch"])?;
        let fresh = self.status()?;
        if fresh.behind > 0 {
            run_in(&self.root, &["pull", "--rebase", "--autostash"])?;
        }
        if fresh.ahead > 0 || fresh.behind > 0 {
            run_in(&self.root, &["push"])?;
            Ok("sync complete".into())
        } else {
            Ok("already up to date".into())
        }
    }
}

fn git_lines(dir: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    Ok(run_in(dir, args)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn safe_repo_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("unsafe repository path".into());
    }
    Ok(root.join(path))
}

/// Pathspecs to reset for one entry. Renames need both old and new paths;
/// copies reset only the destination so a staged source modification survives.
fn unstage_pathspecs(entry: &FileEntry) -> Vec<&str> {
    let mut paths = vec![entry.path.as_str()];
    if entry.letter == 'R' {
        if let Some(orig) = &entry.orig {
            paths.push(orig);
        }
    }
    paths
}

fn run_in(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        // Porcelain paths are literal filenames. Without this global option a
        // valid name such as `:(glob)*` can match and mutate unrelated files.
        .arg("--literal-pathspecs")
        .arg("-c")
        .arg("color.ui=false")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("git failed")
        .trim()
        .to_string())
}

/// Parse `git status --porcelain -z --branch` output. Entries are NUL-separated
/// `XY path`; a rename/copy is followed by a second NUL-separated field holding
/// the source path. X is the index (staged) state, Y the worktree state.
pub fn parse_status(raw: &str) -> Status {
    let mut status = Status::default();
    let mut parts = raw.split('\0');
    while let Some(entry) = parts.next() {
        if entry.is_empty() {
            continue;
        }
        if let Some(header) = entry.strip_prefix("## ") {
            status.branch = parse_branch(header);
            (status.ahead, status.behind) = parse_ahead_behind(header);
            status.has_upstream = header.contains("...");
            continue;
        }
        let Some((xy, path)) = split_entry(entry) else {
            continue;
        };
        let (x, y) = xy;
        let orig = if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            parts.next().filter(|s| !s.is_empty()).map(str::to_string)
        } else {
            None
        };
        let path = path.to_string();
        if x == '?' && y == '?' {
            status.unstaged.push(FileEntry {
                path,
                orig: None,
                letter: 'U',
            });
            continue;
        }
        if x == '!' {
            continue; // ignored file
        }
        if is_conflict(x, y) {
            status.unstaged.push(FileEntry {
                path,
                orig,
                letter: '!',
            });
            continue;
        }
        if x != ' ' {
            status.staged.push(FileEntry {
                path: path.clone(),
                orig: orig.clone(),
                letter: display_letter(x),
            });
        }
        if y != ' ' {
            status.unstaged.push(FileEntry {
                path,
                orig,
                letter: display_letter(y),
            });
        }
    }
    status
}

/// `("XY", path)` from one porcelain entry; the XY columns are always ASCII.
fn split_entry(entry: &str) -> Option<((char, char), &str)> {
    let bytes = entry.as_bytes();
    if bytes.len() < 4 || bytes[2] != b' ' {
        return None;
    }
    Some(((bytes[0] as char, bytes[1] as char), &entry[3..]))
}

fn is_conflict(x: char, y: char) -> bool {
    matches!(
        (x, y),
        ('D', 'D') | ('A', 'U') | ('U', 'D') | ('U', 'A') | ('D', 'U') | ('A', 'A') | ('U', 'U')
    )
}

/// Type changes (T) read as plain modifications, matching VS Code.
fn display_letter(c: char) -> char {
    if c == 'T' {
        'M'
    } else {
        c
    }
}

/// Branch from the `## …` header: `main...origin/main [ahead 1]`, bare `main`,
/// `No commits yet on main`, or `HEAD (no branch)` when detached.
fn parse_branch(header: &str) -> String {
    let head = header.split("...").next().unwrap_or(header);
    head.strip_prefix("No commits yet on ")
        .unwrap_or(head)
        .to_string()
}

/// `(ahead, behind)` from the header's `[ahead 1, behind 2]` suffix (either
/// half may be absent; `[gone]` and no-bracket headers give zeros).
fn parse_ahead_behind(header: &str) -> (usize, usize) {
    let Some(bracket) = header
        .rsplit_once('[')
        .map(|(_, b)| b.trim_end_matches(']'))
    else {
        return (0, 0);
    };
    let count_after = |tag: &str| {
        bracket
            .split(',')
            .map(str::trim)
            .find_map(|part| part.strip_prefix(tag))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0)
    };
    (count_after("ahead "), count_after("behind "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, letter: char, orig: Option<&str>) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            orig: orig.map(str::to_string),
            letter,
        }
    }

    #[test]
    fn parses_branch_and_ahead_behind() {
        let raw = "## main...origin/main [ahead 2, behind 1]\0";
        let s = parse_status(raw);
        assert_eq!(s.branch, "main");
        assert_eq!((s.ahead, s.behind), (2, 1));
        assert!(s.has_upstream);
    }

    #[test]
    fn splits_staged_and_unstaged() {
        // M in index, M in worktree → both lists; ?? → untracked (unstaged).
        let raw = "## main\0M  staged.rs\0 M dirty.rs\0MM both.rs\0?? new.rs\0";
        let s = parse_status(raw);
        assert_eq!(
            s.staged,
            vec![entry("staged.rs", 'M', None), entry("both.rs", 'M', None)]
        );
        assert_eq!(
            s.unstaged,
            vec![
                entry("dirty.rs", 'M', None),
                entry("both.rs", 'M', None),
                entry("new.rs", 'U', None),
            ]
        );
    }

    #[test]
    fn rename_carries_source_path() {
        let raw = "## main\0R  new.rs\0old.rs\0";
        let s = parse_status(raw);
        assert_eq!(s.staged, vec![entry("new.rs", 'R', Some("old.rs"))]);
    }

    #[test]
    fn conflicts_land_in_unstaged_with_bang() {
        let raw = "## main\0UU conflict.rs\0";
        let s = parse_status(raw);
        assert_eq!(s.unstaged, vec![entry("conflict.rs", '!', None)]);
        assert!(s.staged.is_empty());
    }

    #[test]
    fn unborn_branch_header() {
        let raw = "## No commits yet on main\0A  first.rs\0";
        let s = parse_status(raw);
        assert_eq!(s.branch, "main");
        assert_eq!(s.staged, vec![entry("first.rs", 'A', None)]);
    }

    #[test]
    fn unstage_copy_does_not_reset_its_source() {
        let rename = entry("new.rs", 'R', Some("old.rs"));
        assert_eq!(unstage_pathspecs(&rename), vec!["new.rs", "old.rs"]);
        let copy = entry("copy.rs", 'C', Some("source.rs"));
        assert_eq!(unstage_pathspecs(&copy), vec!["copy.rs"]);
    }

    #[test]
    fn discard_path_cannot_escape_repository() {
        let root = Path::new("/repo");
        assert_eq!(
            safe_repo_path(root, "src/main.rs").unwrap(),
            PathBuf::from("/repo/src/main.rs")
        );
        assert!(safe_repo_path(root, "../outside").is_err());
        assert!(safe_repo_path(root, "/outside").is_err());
    }

    #[test]
    fn discard_restores_tracked_and_removes_untracked() {
        let root = std::env::temp_dir().join(format!(
            "corral-discard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let git_cmd = |args: &[&str]| {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&root)
                .status()
                .unwrap()
                .success());
        };
        git_cmd(&["init", "-q"]);
        git_cmd(&["config", "user.email", "test@example.com"]);
        git_cmd(&["config", "user.name", "Test"]);
        std::fs::write(root.join("tracked.txt"), "base\n").unwrap();
        std::fs::write(root.join(":(glob)*"), "magic base\n").unwrap();
        std::fs::write(root.join("normal"), "normal base\n").unwrap();
        git_cmd(&["add", "."]);
        git_cmd(&["commit", "-qm", "base"]);

        let git = Git { root: root.clone() };
        std::fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        git.discard(&entry("tracked.txt", 'M', None)).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "base\n"
        );

        // A porcelain filename that looks like pathspec magic must remain
        // literal: discarding it cannot touch the unrelated dirty file.
        std::fs::write(root.join(":(glob)*"), "magic changed\n").unwrap();
        std::fs::write(root.join("normal"), "normal changed\n").unwrap();
        git.discard(&entry(":(glob)*", 'M', None)).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(":(glob)*")).unwrap(),
            "magic base\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("normal")).unwrap(),
            "normal changed\n"
        );

        std::fs::write(root.join("new.txt"), "new\n").unwrap();
        git.discard(&entry("new.txt", 'U', None)).unwrap();
        assert!(!root.join("new.txt").exists());

        git_cmd(&["tag", "v1.0"]);
        assert!(git
            .tags()
            .unwrap()
            .iter()
            .any(|line| line.starts_with("v1.0\t")));
        git_cmd(&["stash", "push", "-qm", "drawer test"]);
        assert!(git
            .stashes()
            .unwrap()
            .iter()
            .any(|line| line.starts_with("stash@{0}\t")));

        std::fs::remove_dir_all(root).unwrap();
    }
}
