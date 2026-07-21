//! Sidebar TUI: activity strip + active feature body.
//!
//! Shape matches herdr-sidebar:
//! - one left-docked Herdr pane
//! - Explorer / SCM / GitHub switch in-process via the top icon row
//! - previews are NOT drawn here (separate pane later via control file)

use crate::feature::Feature;
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

/// Columns per activity icon chip (padding + glyph + padding).
const NAV_BTN_WIDTH: u16 = 5;

struct State {
    feature: Feature,
    nav_hits: Vec<(Feature, Rect)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            feature: Feature::Explorer,
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
                if handle_key(&mut state, key.code, key.modifiers) {
                    break;
                }
            }
            Event::Mouse(m) => {
                if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                    handle_click(&mut state, m.column, m.row);
                }
            }
            _ => {}
        }
    }
    Ok(())
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
    // Soft underline under the activity strip.
    if area.height > 0 {
        let line_y = area.y.saturating_add(area.height.saturating_sub(1));
        frame.render_widget(
            Paragraph::new("─".repeat(area.width as usize))
                .style(Style::default().fg(palette.overlay0).bg(palette.panel_bg)),
            Rect {
                x: area.x,
                y: line_y,
                width: area.width,
                height: 1,
            },
        );
    }

    let icon_y = area.y.saturating_add(area.height.saturating_sub(1) / 2);
    let mut x = area.x.saturating_add(1);

    for feature in Feature::ALL {
        let avail = area.x.saturating_add(area.width).saturating_sub(x);
        if avail < 3 {
            break;
        }
        let w = NAV_BTN_WIDTH.min(avail);
        let selected = feature == state.feature;
        let icon = feature.icon(use_nf);

        let (fg, bg) = if selected {
            (palette.text, palette.surface1)
        } else {
            (palette.overlay1, palette.panel_bg)
        };
        let style = Style::default().fg(fg).bg(bg).add_modifier(if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        });

        let chip = Rect {
            x,
            y: area.y,
            width: w,
            height: area.height.saturating_sub(1).max(1),
        };
        frame.render_widget(Block::default().style(Style::default().bg(bg)), chip);

        let glyph = Rect {
            x: x.saturating_add(w.saturating_sub(1) / 2),
            y: icon_y.min(chip.y.saturating_add(chip.height.saturating_sub(1))),
            width: 1,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(icon, style))),
            glyph,
        );

        state.nav_hits.push((feature, chip));
        x = x.saturating_add(w.saturating_add(1));
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
