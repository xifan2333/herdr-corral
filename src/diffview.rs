//! Standalone VS Code-style diff renderer: plain `git diff` text in, ANSI-
//! colored, dual-gutter, word-highlighted diff out. Used by the `corral-diff`
//! binary as a pipe filter (`git diff | corral-diff | less -R`), themed with
//! corral's [`Palette`] so it matches the sidebar instead of an external
//! tool's fixed look.
//!
//! No syntax highlighting yet — structure, red/green row tints, and a darker
//! word-level tint on the changed segment of paired lines (the herdr-sidebar
//! signature). Parsing is ours, so the look is ours.

use crate::ui::Palette;
use ratatui::style::Color;

// VS Code dark diff editor tints (theme-neutral; they read well on any bg).
const DEL_BG: (u8, u8, u8) = (0x42, 0x22, 0x26);
const DEL_WORD_BG: (u8, u8, u8) = (0x6f, 0x30, 0x36);
const ADD_BG: (u8, u8, u8) = (0x20, 0x39, 0x28);
const ADD_WORD_BG: (u8, u8, u8) = (0x35, 0x59, 0x3d);
const DEL_MARK: (u8, u8, u8) = (0xd1, 0x6d, 0x76);
const ADD_MARK: (u8, u8, u8) = (0x8c, 0xc9, 0x8f);

const RESET: &str = "\x1b[0m";

/// One parsed diff event.
enum Ev {
    File(String),
    Hunk(String),
    Ctx(usize, usize, String),
    Del(usize, String),
    Add(usize, String),
}

/// Render a unified `git diff` into an ANSI string ready for a pager.
pub fn render(diff: &str, palette: &Palette, width: u16) -> String {
    let events = parse(diff);
    let gw = gutter_width(&events);
    let w = width.max(24) as usize;
    let mut out = String::new();

    let mut i = 0;
    while i < events.len() {
        match &events[i] {
            Ev::File(name) => {
                render_file(&mut out, name, palette, w);
                i += 1;
            }
            Ev::Hunk(text) => {
                render_hunk(&mut out, text, palette, w, gw);
                i += 1;
            }
            Ev::Ctx(o, n, t) => {
                render_ctx(&mut out, *o, *n, t, palette, w, gw);
                i += 1;
            }
            Ev::Del(..) => {
                let start = i;
                while matches!(events.get(i), Some(Ev::Del(..))) {
                    i += 1;
                }
                let dels = &events[start..i];
                let astart = i;
                while matches!(events.get(i), Some(Ev::Add(..))) {
                    i += 1;
                }
                let adds = &events[astart..i];
                render_pair(&mut out, dels, adds, palette, w, gw);
            }
            Ev::Add(..) => {
                let start = i;
                while matches!(events.get(i), Some(Ev::Add(..))) {
                    i += 1;
                }
                render_pair(&mut out, &[], &events[start..i], palette, w, gw);
            }
        }
    }
    out
}

// --- parsing ----------------------------------------------------------------

fn parse(diff: &str) -> Vec<Ev> {
    let mut evs = Vec::new();
    let mut o = 0usize;
    let mut n = 0usize;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            evs.push(Ev::File(file_name(rest)));
            continue;
        }
        if line.starts_with("index ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
            || line.starts_with("similarity")
            || line.starts_with("dissimilarity")
            || line.starts_with("rename ")
            || line.starts_with("copy ")
            || line.starts_with("new file mode")
            || line.starts_with("deleted file mode")
            || line.starts_with('\\')
        {
            continue;
        }
        if line.starts_with("@@") {
            let (no, nn, section) = parse_hunk(line);
            o = no;
            n = nn;
            evs.push(Ev::Hunk(section));
            continue;
        }
        match line.as_bytes().first() {
            Some(b' ') => {
                evs.push(Ev::Ctx(o, n, line[1..].to_string()));
                o += 1;
                n += 1;
            }
            Some(b'-') => {
                evs.push(Ev::Del(o, line[1..].to_string()));
                o += 1;
            }
            Some(b'+') => {
                evs.push(Ev::Add(n, line[1..].to_string()));
                n += 1;
            }
            // Blank line in the body = an empty context line.
            None => {
                evs.push(Ev::Ctx(o, n, String::new()));
                o += 1;
                n += 1;
            }
            _ => {}
        }
    }
    evs
}

/// New path from a `diff --git a/x b/x` remainder (prefer the `b/` side).
fn file_name(rest: &str) -> String {
    if let Some((_, b)) = rest.split_once(" b/") {
        return b.to_string();
    }
    rest.trim_start_matches("a/").to_string()
}

/// `(old_start, new_start, section)` from `@@ -o,c +n,c @@ section`.
fn parse_hunk(line: &str) -> (usize, usize, String) {
    let after = line.trim_start_matches('@');
    let mut old = 0;
    let mut new = 0;
    for tok in after.split_whitespace() {
        if let Some(num) = tok.strip_prefix('-') {
            old = num
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if let Some(num) = tok.strip_prefix('+') {
            new = num
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            break;
        }
    }
    // Section text follows the second `@@`.
    let section = line
        .splitn(3, "@@")
        .nth(2)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    (old, new, section)
}

fn gutter_width(events: &[Ev]) -> usize {
    let max = events
        .iter()
        .map(|e| match e {
            Ev::Ctx(o, n, _) => (*o).max(*n),
            Ev::Del(o, _) => *o,
            Ev::Add(n, _) => *n,
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    format!("{max}").len().max(2)
}

// --- rendering --------------------------------------------------------------

fn render_file(out: &mut String, name: &str, palette: &Palette, w: usize) {
    out.push('\n');
    let text = format!(" {name} ");
    out.push_str(&fg(palette.accent));
    out.push_str("\x1b[1m");
    out.push_str(&text);
    out.push_str(RESET);
    out.push('\n');
    // Thin underline rule in a muted surface color.
    out.push_str(&fg(palette.surface1));
    out.push_str(&"─".repeat(w.min(120)));
    out.push_str(RESET);
    out.push('\n');
}

fn render_hunk(out: &mut String, section: &str, palette: &Palette, w: usize, gw: usize) {
    let label = if section.is_empty() {
        "  ⋯".to_string()
    } else {
        format!("  ⋯ {section}")
    };
    let pad = w.saturating_sub(disp(&label) + 2 * gw + 3);
    out.push_str(&fg(palette.overlay0));
    out.push_str(&" ".repeat(2 * gw + 3));
    out.push_str(&fg(palette.mauve));
    out.push_str(&label);
    out.push_str(&" ".repeat(pad));
    out.push_str(RESET);
    out.push('\n');
}

fn render_ctx(
    out: &mut String,
    o: usize,
    n: usize,
    text: &str,
    palette: &Palette,
    w: usize,
    gw: usize,
) {
    gutter(out, Some(o), Some(n), gw, palette);
    out.push_str("  ");
    out.push_str(&fg(palette.text));
    let body = clip(text, w.saturating_sub(2 * gw + 5));
    out.push_str(&body);
    out.push_str(RESET);
    out.push('\n');
}

/// Render a removed run then an added run, with word-level tint on paired lines.
fn render_pair(out: &mut String, dels: &[Ev], adds: &[Ev], palette: &Palette, w: usize, gw: usize) {
    for (k, ev) in dels.iter().enumerate() {
        let Ev::Del(o, text) = ev else { continue };
        let range = adds.get(k).and_then(|a| match a {
            Ev::Add(_, at) => Some(intra(text, at).0),
            _ => None,
        });
        code_line(
            out,
            Some(*o),
            None,
            gw,
            '-',
            DEL_BG,
            DEL_WORD_BG,
            DEL_MARK,
            text,
            range,
            palette,
            w,
        );
    }
    for (k, ev) in adds.iter().enumerate() {
        let Ev::Add(n, text) = ev else { continue };
        let range = dels.get(k).and_then(|d| match d {
            Ev::Del(_, dt) => Some(intra(dt, text).1),
            _ => None,
        });
        code_line(
            out,
            None,
            Some(*n),
            gw,
            '+',
            ADD_BG,
            ADD_WORD_BG,
            ADD_MARK,
            text,
            range,
            palette,
            w,
        );
    }
}

/// Common-prefix / common-suffix intra-line diff. Returns the changed char
/// range in each of `a` and `b`.
fn intra(a: &str, b: &str) -> ((usize, usize), (usize, usize)) {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let mut p = 0;
    while p < ac.len() && p < bc.len() && ac[p] == bc[p] {
        p += 1;
    }
    let mut s = 0;
    while s < ac.len().saturating_sub(p)
        && s < bc.len().saturating_sub(p)
        && ac[ac.len() - 1 - s] == bc[bc.len() - 1 - s]
    {
        s += 1;
    }
    ((p, ac.len() - s), (p, bc.len() - s))
}

/// Emit the dual line-number gutter (no background).
fn gutter(
    out: &mut String,
    left: Option<usize>,
    right: Option<usize>,
    gw: usize,
    palette: &Palette,
) {
    let l = left
        .map(|n| format!("{n:>gw$}"))
        .unwrap_or_else(|| " ".repeat(gw));
    let r = right
        .map(|n| format!("{n:>gw$}"))
        .unwrap_or_else(|| " ".repeat(gw));
    out.push_str(&fg(palette.overlay0));
    out.push_str(&l);
    out.push(' ');
    out.push_str(&r);
    out.push_str(RESET);
}

/// One added/removed code line: gutter + sign + tinted body (word range darker).
#[allow(clippy::too_many_arguments)]
fn code_line(
    out: &mut String,
    left: Option<usize>,
    right: Option<usize>,
    gw: usize,
    sign: char,
    base_bg: (u8, u8, u8),
    word_bg: (u8, u8, u8),
    mark: (u8, u8, u8),
    text: &str,
    word: Option<(usize, usize)>,
    palette: &Palette,
    w: usize,
) {
    gutter(out, left, right, gw, palette);
    // Code cell: sign + text, filled with base tint to the row's end.
    let cell_w = w.saturating_sub(2 * gw + 1);
    out.push_str(&bg_rgb(base_bg));
    out.push_str(&fg_rgb(mark));
    out.push(sign);
    out.push_str(&fg(palette.text));

    let mut cols = 1usize; // sign already took one column
    for (idx, ch) in text.chars().enumerate() {
        if cols >= cell_w {
            break;
        }
        let in_word = word.is_some_and(|(a, b)| idx >= a && idx < b && a < b);
        if in_word {
            out.push_str(&bg_rgb(word_bg));
        }
        out.push(ch);
        if in_word {
            out.push_str(&bg_rgb(base_bg));
        }
        cols += 1;
    }
    // Pad the rest of the row with the base tint.
    if cols < cell_w {
        out.push_str(&" ".repeat(cell_w - cols));
    }
    out.push_str(RESET);
    out.push('\n');
}

// --- helpers ----------------------------------------------------------------

/// Clip a string to at most `max` display columns (approx: char count).
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

/// Approximate display width (char count; good enough for source lines).
fn disp(s: &str) -> usize {
    s.chars().count()
}

/// SGR foreground for a palette [`Color`].
fn fg(c: Color) -> String {
    color_sgr(c, false)
}

fn fg_rgb((r, g, b): (u8, u8, u8)) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

fn bg_rgb((r, g, b): (u8, u8, u8)) -> String {
    format!("\x1b[48;2;{r};{g};{b}m")
}

fn color_sgr(c: Color, bg: bool) -> String {
    let base = if bg { 40 } else { 30 };
    let bright = if bg { 100 } else { 90 };
    match c {
        Color::Rgb(r, g, b) => format!("\x1b[{};2;{r};{g};{b}m", if bg { 48 } else { 38 }),
        Color::Black => format!("\x1b[{}m", base),
        Color::Red => format!("\x1b[{}m", base + 1),
        Color::Green => format!("\x1b[{}m", base + 2),
        Color::Yellow => format!("\x1b[{}m", base + 3),
        Color::Blue => format!("\x1b[{}m", base + 4),
        Color::Magenta => format!("\x1b[{}m", base + 5),
        Color::Cyan => format!("\x1b[{}m", base + 6),
        Color::Gray => format!("\x1b[{}m", base + 7),
        Color::DarkGray => format!("\x1b[{}m", bright),
        Color::LightRed => format!("\x1b[{}m", bright + 1),
        Color::LightGreen => format!("\x1b[{}m", bright + 2),
        Color::LightYellow => format!("\x1b[{}m", bright + 3),
        Color::LightBlue => format!("\x1b[{}m", bright + 4),
        Color::LightMagenta => format!("\x1b[{}m", bright + 5),
        Color::LightCyan => format!("\x1b[{}m", bright + 6),
        Color::White => format!("\x1b[{}m", bright + 7),
        Color::Indexed(i) => format!("\x1b[{};5;{i}m", if bg { 48 } else { 38 }),
        Color::Reset => "\x1b[39m".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hunk_header() {
        let (o, n, s) = parse_hunk("@@ -12,7 +14,9 @@ fn main()");
        assert_eq!((o, n), (12, 14));
        assert_eq!(s, "fn main()");
    }

    #[test]
    fn intra_finds_changed_middle() {
        let ((a0, a1), (b0, b1)) = intra("let x = 1;", "let x = 2;");
        assert_eq!(&"let x = 1;"[a0..a1], "1");
        assert_eq!(&"let x = 2;"[b0..b1], "2");
    }

    #[test]
    fn render_smoke_has_ansi_and_name() {
        let diff =
            "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,2 @@\n ctx\n-old\n+new\n";
        let out = render(diff, &Palette::named("catppuccin").unwrap(), 80);
        assert!(out.contains("f.rs"));
        assert!(out.contains("\x1b["));
    }
}
