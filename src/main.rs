//! Thin binary over the `corral` library.
//!
//! - `corral`               → run the sidebar TUI (plugin pane or standalone)
//! - `corral bind KEY ACT`  → register a keybinding (used by config.sh, River-style)

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bind") => corral::config::cli_bind(&args[1..]),
        _ => corral::run(),
    }
}
