//! `corral-diff` — a VS Code-style diff pager filter, themed with corral's
//! palette. Reads a unified `git diff` on stdin, writes ANSI to stdout:
//!
//! ```sh
//! git diff | corral-diff | less -R
//! GIT_EXTERNAL_DIFF= ... # (future) per-file mode
//! ```
//!
//! Width comes from the terminal (or `$COLUMNS`); theme from the same
//! resolution corral's sidebar uses, so the diff matches the sidebar.

use std::io::{Read, Write};

fn main() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        std::process::exit(1);
    }
    if input.trim().is_empty() {
        return;
    }

    let palette = corral::ui::Palette::resolve();
    let width = terminal_width();
    let rendered = corral::diffview::render(&input, &palette, width);

    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(rendered.as_bytes());
}

/// Terminal columns: real size, else `$COLUMNS`, else 80.
fn terminal_width() -> u16 {
    if let Ok((cols, _)) = crossterm::terminal::size() {
        if cols > 0 {
            return cols;
        }
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&c| c > 0)
        .unwrap_or(80)
}
