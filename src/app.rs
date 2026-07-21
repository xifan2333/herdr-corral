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
    // herdr-sidebar pattern:
    //   activity strip is 3 rows tall
    //   icons live on the MIDDLE row only
    //   selected chip grows with half-block caps (▄ top / ▀ bottom)
    // so the glyph sits visually centered in a tall button.
    if area.height < 3 || area.width == 0 {
        return;
    }

    let outer_top = area.y;
    let mid_y = area.y + 1;
    let outer_bottom = area.y + 2;
    let mid = Rect::new(area.x, mid_y, area.width, 1);

    // Build one middle-row line of spans: " {icon}{slack} " chips with gaps.
    // Material FA glyphs are often 2 cells wide — reserve a slack cell so chips
    // stay equal and icons center (same as herdr-sidebar).
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" "));
    let mut chip_bounds: Vec<(Feature, u16, u16)> = Vec::new(); // feature, start_x, end_x relative to mid.x
    let mut col: u16 = 1; // after leading space

    for feature in Feature::ALL {
        let icon = feature.icon(use_nf);
        let slack = if feature.icon_double_width(use_nf) {
            " "
        } else {
            ""
        };
        let selected = feature == state.feature;
        let label = format!(" {icon}{slack} ");
        let style = if selected {
            Style::default()
                .fg(palette.text)
                .bg(palette.surface1)
                .add_modifier(Modifier::BOLD)
        } else {
            // Idle: theme main text ("white"), no chip background.
            Style::default().fg(palette.text)
        };
        let span = Span::styled(label, style);
        let w = span.width() as u16;
        let start = col;
        let end = col.saturating_add(w);
        chip_bounds.push((feature, start, end));
        spans.push(span);
        spans.push(Span::raw(" "));
        col = end.saturating_add(1);
    }

    // Half-block caps only on the selected chip — tall button, icon on middle.
    if let Some((_, start, end)) = chip_bounds
        .iter()
        .find(|(f, _, _)| *f == state.feature)
    {
        let chip_x = mid.x.saturating_add(*start);
        let chip_w = end.saturating_sub(*start);
        if chip_w > 0 {
            let mut paint_cap = |glyph: &str, y: u16| {
                frame.render_widget(
                    Paragraph::new(glyph.repeat(usize::from(chip_w))).style(
                        Style::default()
                            .fg(palette.surface1)
                            .bg(palette.panel_bg),
                    ),
                    Rect::new(chip_x, y, chip_w, 1),
                );
            };
            paint_cap("▄", outer_top);
            paint_cap("▀", outer_bottom);
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), mid);

    // Hit targets cover the full 3-row tall button for easier clicking.
    for (feature, start, end) in chip_bounds {
        let w = end.saturating_sub(start).max(1);
        state.nav_hits.push((
            feature,
            Rect {
                x: mid.x.saturating_add(start),
                y: outer_top,
                width: w,
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
