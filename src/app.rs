//! Sidebar TUI: activity strip + active feature body.
//!
//! Shape matches herdr-sidebar:
//! - one left-docked Herdr pane
//! - Explorer / SCM / GitHub switch in-process via the top icon row
//! - previews are NOT drawn here (separate pane later via control file)

use crate::feature::Feature;
use crate::herdr_cli;
use crate::host::LaunchContext;
use crate::icons::{self, NerdFontSupport};
use crate::layout;
use crate::theme::Palette;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{Frame, Terminal};
use std::io::{self, Stdout};
use std::time::Duration;

struct State {
    feature: Feature,
    /// Last label pushed to Herdr (avoid spamming rename every frame).
    labeled_as: Option<&'static str>,
    nav_hits: Vec<(Feature, Rect)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            feature: Feature::Explorer,
            labeled_as: None,
            nav_hits: Vec::new(),
        }
    }
}

/// Run the sidebar TUI until the user quits.
pub fn run(ctx: LaunchContext) -> io::Result<()> {
    let _ = std::env::set_current_dir(&ctx.cwd);

    let palette = Palette::resolve();
    let nerd_font = icons::detect();
    let mut terminal = setup()?;
    let result = event_loop(&mut terminal, &palette, &nerd_font, &ctx);
    restore(&mut terminal)?;
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    palette: &Palette,
    nerd_font: &NerdFontSupport,
    ctx: &LaunchContext,
) -> io::Result<()> {
    let mut state = State::default();
    let use_nf = nerd_font.should_use_icons();

    // Initial Herdr border title = active feature.
    sync_pane_label(&mut state, ctx);

    loop {
        terminal.draw(|frame| {
            state.nav_hits.clear();
            draw(frame, palette, nerd_font, use_nf, &mut state, ctx);
        })?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let before = state.feature;
                if handle_key(&mut state, key.code, key.modifiers) {
                    break;
                }
                if state.feature != before {
                    sync_pane_label(&mut state, ctx);
                }
            }
            Event::Mouse(m) => {
                if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                    let before = state.feature;
                    handle_click(&mut state, m.column, m.row);
                    if state.feature != before {
                        sync_pane_label(&mut state, ctx);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn sync_pane_label(state: &mut State, ctx: &LaunchContext) {
    let title = state.feature.title();
    if state.labeled_as == Some(title) {
        return;
    }
    // Prefer HERDR_PANE_ID (this pane) over context focused_pane_id.
    let pane_id = std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| ctx.focused_pane_id.clone());
    herdr_cli::set_pane_label(
        ctx.herdr_bin().map(|p| p.as_path()),
        pane_id.as_deref(),
        title,
    );
    state.labeled_as = Some(title);
}

/// Returns true if the event loop should quit.
fn handle_key(state: &mut State, code: KeyCode, mods: KeyModifiers) -> bool {
    match code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => return true,
        // Esc does not quit the sidebar (herdr-sidebar closes preview instead).
        KeyCode::Esc => {}

        KeyCode::Char(c @ '1'..='3') => {
            if let Some(f) = Feature::from_digit(c) {
                state.feature = f;
            }
        }
        KeyCode::Char('j') | KeyCode::Down => state.feature = state.feature.next(),
        KeyCode::Char('k') | KeyCode::Up => state.feature = state.feature.prev(),
        KeyCode::Tab => state.feature = state.feature.next(),
        KeyCode::BackTab => state.feature = state.feature.prev(),

        _ => {}
    }
    false
}

fn handle_click(state: &mut State, col: u16, row: u16) {
    for (feature, rect) in &state.nav_hits {
        if contains(*rect, col, row) {
            state.feature = *feature;
            return;
        }
    }
}

fn contains(r: Rect, col: u16, row: u16) -> bool {
    col >= r.x
        && col < r.x.saturating_add(r.width)
        && row >= r.y
        && row < r.y.saturating_add(r.height)
}

fn draw(
    frame: &mut Frame,
    palette: &Palette,
    nerd_font: &NerdFontSupport,
    use_nf: bool,
    state: &mut State,
    ctx: &LaunchContext,
) {
    let area = frame.area();

    // No own outer border/title: herdr already frames the pane and labels it.
    // Fill the pane background once.
    frame.render_widget(
        Block::default().style(Style::default().bg(palette.panel_bg).fg(palette.text)),
        area,
    );

    let (activity, body, footer) = layout::split_sidebar(area, true);

    draw_activity(frame, activity, state, use_nf, palette);
    draw_body(frame, body, state.feature, palette, ctx);
    draw_footer(frame, footer, palette, nerd_font, state.feature, ctx);
}

fn draw_activity(
    frame: &mut Frame,
    area: Rect,
    state: &mut State,
    use_nf: bool,
    palette: &Palette,
) {
    // Port of herdr-sidebar `draw_activity_bar`:
    // - strip is 3 rows, plain pane background (no container)
    // - icons ONLY on the middle row
    // - active chip: middle-row bg + half-block caps on outer rows
    //   so the glyph sits in the vertical center of a tall button
    // - FA/Nerd glyphs are often 2 cells wide → reserve a slack cell
    if area.height < 3 || area.width == 0 {
        return;
    }

    let outer_top = area.y;
    let outer_bottom = area.y + 2;
    // Middle row only — same as `Rect::new(area.x, area.y + 1, area.width, 1)`.
    let mid = Rect::new(area.x, area.y + 1, area.width, 1);

    let chip_bg = palette.surface1;
    // Span list mirrors herdr-sidebar: leading pad, then alternating chip + gap.
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" "));
    for feature in Feature::ALL {
        let icon = feature.icon(use_nf);
        // FA/Nerd glyphs are often 2 cells wide — reserve a slack cell.
        let slack = if use_nf { " " } else { "" };
        let selected = feature == state.feature;
        let style = if selected {
            Style::default()
                .fg(palette.text)
                .bg(chip_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.text)
        };
        spans.push(Span::styled(format!(" {icon}{slack} "), style));
        spans.push(Span::raw(" "));
    }

    // Absolute chip bounds from real span widths (same scan as herdr-sidebar).
    // Layout: [pad, chip0, gap, chip1, gap, chip2, gap] → chips at indices 1,3,5.
    let mut x = mid.x;
    let mut bounds: Vec<(Feature, u16, u16)> = Vec::new();
    for (i, span) in spans.iter().enumerate() {
        let w = span.width() as u16;
        if i % 2 == 1 {
            // chip span
            let fi = i / 2;
            if let Some(feature) = Feature::from_index(fi) {
                bounds.push((feature, x, x + w));
            }
        }
        x = x.saturating_add(w);
    }

    // Symmetric half-block caps on the ACTIVE chip only (reference).
    if let Some((_, start, end)) = bounds.iter().find(|(f, _, _)| *f == state.feature) {
        let chip_w = end.saturating_sub(*start);
        if chip_w > 0 {
            // fg = chip color, default bg → half blocks read as extensions of the mid chip.
            let cap = Paragraph::new("▄".repeat(usize::from(chip_w)))
                .style(Style::default().fg(chip_bg));
            frame.render_widget(cap, Rect::new(*start, outer_top, chip_w, 1));
            let cap = Paragraph::new("▀".repeat(usize::from(chip_w)))
                .style(Style::default().fg(chip_bg));
            frame.render_widget(cap, Rect::new(*start, outer_bottom, chip_w, 1));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), mid);

    for (feature, start, end) in bounds {
        state.nav_hits.push((
            feature,
            Rect {
                x: start,
                y: outer_top,
                width: end.saturating_sub(start).max(1),
                height: 3,
            },
        ));
    }
}

fn draw_body(
    frame: &mut Frame,
    area: Rect,
    feature: Feature,
    palette: &Palette,
    ctx: &LaunchContext,
) {
    // Header line with feature title.
    if area.height == 0 {
        return;
    }
    let title = Paragraph::new(Line::from(Span::styled(
        format!(" {}", feature.title()),
        Style::default()
            .fg(palette.subtext0)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(
        title,
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );

    if area.height < 3 {
        return;
    }
    let body = Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(2),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let placeholder = match feature {
        Feature::Explorer => format!("file tree goes here\n{}", ctx.cwd.display()),
        Feature::Scm => "git changes go here".into(),
        Feature::GitHub => "issues / PRs go here".into(),
    };
    frame.render_widget(
        Paragraph::new(placeholder).style(Style::default().fg(palette.overlay1)),
        body,
    );
}

fn draw_footer(
    frame: &mut Frame,
    area: Rect,
    palette: &Palette,
    nerd_font: &NerdFontSupport,
    feature: Feature,
    ctx: &LaunchContext,
) {
    if area.height == 0 {
        return;
    }
    let nf = match nerd_font.available {
        Some(true) => "nf",
        Some(false) => "no-nf",
        None => "nf?",
    };
    let text = format!(
        " {}  {}  {}  {}  1/2/3  q ",
        ctx.mode.label(),
        palette.name,
        feature.id(),
        nf
    );
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(palette.subtext0).bg(palette.panel_bg)),
        area,
    );
}

fn setup() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Colors are UI, not pipeable output — ignore NO_COLOR from agent shells
    // (same rationale as herdr-sidebar).
    crossterm::style::force_color_output(true);
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}
