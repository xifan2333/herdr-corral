//! Thin binary over the `corral` library.
//!
//! - `corral`               → run the sidebar TUI (plugin pane or standalone)
//! - `corral bind KEY ACT`  → register a keybinding (used by config.sh, River-style)

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bind") => corral::config::cli_bind(&args[1..]),
        Some("--launch-decision") => stdin_mode(corral::herdr::launch::launch_decision),
        Some("--focused-pane") => stdin_mode(corral::herdr::launch::focused_pane),
        Some("--open-plan") => stdin_mode(corral::herdr::launch::open_plan),
        Some("--resize-plan") => stdin_mode_arg(corral::herdr::launch::resize_plan, args.get(1)),
        Some("--prepare-split-plan") => {
            stdin_mode_arg(corral::herdr::launch::prepare_split_plan, args.get(1))
        }
        Some("--pane-live") => stdin_mode_arg(corral::herdr::launch::pane_live, args.get(1)),
        Some("--split-pane-id") => stdin_mode(corral::herdr::launch::split_pane_id),
        _ => corral::run(),
    }
}

fn stdin_mode(f: fn(&str) -> String) -> std::io::Result<()> {
    stdin_mode_inner(f)
}

fn stdin_mode_arg(f: fn(&str, &str) -> String, arg: Option<&String>) -> std::io::Result<()> {
    let arg = arg.map(String::as_str).unwrap_or_default();
    stdin_mode_inner(|input| f(input, arg))
}

fn stdin_mode_inner(f: impl FnOnce(&str) -> String) -> std::io::Result<()> {
    use std::io::Read as _;
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let output = f(&input);
    if !output.is_empty() {
        println!("{output}");
    }
    Ok(())
}
