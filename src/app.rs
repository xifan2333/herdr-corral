//! Single-pane host TUI: activity bar + left/right containers.
//!
//! Works as a Herdr plugin pane *or* a standalone terminal binary.
//! First interactive piece: switch Explorer / SCM / GitHub via Nerd Font
//! activity buttons (or `1`/`2`/`3`, `j`/`k` when activity is focused).

use crate::feature::Feature;
use crate::host::LaunchContext;
use crate::icons::{self, NerdFontSupport};
use crate::layout::{self, Focus, PanelView};
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
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use std::io::{self, Stdout};
use std::time::Duration;

/// Left container share of the body width (percent).
const LEFT_PCT: u16 = 32;

/// One cell per activity button (icon), stacked vertically with a gap row.
const ACTIVITY_BTN_HEIGHT: u16 = 2;

struct State {
    feature: Feature,
    focus: Focus,
    /// Hit targets for activity buttons, filled each draw.
    activity_hits: Vec<(Feature, Rect)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            feature: Feature::Explorer,
            focus: Focus::Activity,
            activity_hits: Vec::new(),
        }
    }
}

/// Run the host TUI until the user quits.
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
        let left = left_view(state.feature);
        let right = right_placeholder(state.feature);

        terminal.draw(|frame| {
            state.activity_hits.clear();
            draw(
                frame,
                palette,
                nerd_font,
                use_nf,
                &mut state,
                &left,
                &right,
                ctx,
            );
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
        KeyCode::Esc => return true,

        // Global feature shortcuts.
        KeyCode::Char(c @ '1'..='3') => {
            if let Some(f) = Feature::from_digit(c) {
                state.feature = f;
                state.focus = Focus::Left;
            }
        }

        KeyCode::Tab => state.focus = state.focus.cycle(),
        KeyCode::BackTab => {
            state.focus = match state.focus {
                Focus::Activity => Focus::Right,
                Focus::Left => Focus::Activity,
                Focus::Right => Focus::Left,
            };
        }

        // Activity bar navigation when focused.
        KeyCode::Char('j') | KeyCode::Down if state.focus == Focus::Activity => {
            state.feature = state.feature.next();
        }
        KeyCode::Char('k') | KeyCode::Up if state.focus == Focus::Activity => {
            state.feature = state.feature.prev();
        }
        KeyCode::Enter if state.focus == Focus::Activity => {
            state.focus = Focus::Left;
        }

        // Focus region shortcuts.
        KeyCode::Char('h') | KeyCode::Left => {
            state.focus = match state.focus {
                Focus::Right => Focus::Left,
                Focus::Left => Focus::Activity,
                Focus::Activity => Focus::Activity,
            };
        }
        KeyCode::Char('l') | KeyCode::Right => {
            state.focus = match state.focus {
                Focus::Activity => Focus::Left,
                Focus::Left => Focus::Right,
                Focus::Right => Focus::Right,
            };
        }

        _ => {}
    }
    false
}

fn handle_click(state: &mut State, col: u16, row: u16) {
    for (feature, rect) in &state.activity_hits {
        if contains(*rect, col, row) {
            state.feature = *feature;
            state.focus = Focus::Left;
            return;
        }
    }
}

fn contains(r: Rect, col: u16, row: u16) -> bool {
    col >= r.x && col < r.x.saturating_add(r.width) && row >= r.y && row < r.y.saturating_add(r.height)
}

fn left_view(feature: Feature) -> PanelView {
    PanelView {
        title: Some(feature.title().into()),
        body: String::new(),
    }
}

fn right_placeholder(feature: Feature) -> PanelView {
    PanelView {
        title: None,
        body: format!("({} selection will show here)", feature.id()),
    }
}

fn draw(
    frame: &mut Frame,
    palette: &Palette,
    nerd_font: &NerdFontSupport,
    use_nf: bool,
    state: &mut State,
    left: &PanelView,
    right: &PanelView,
    ctx: &LaunchContext,
) {
    let area = frame.area();
    let work = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    let regions = layout::split(work, LEFT_PCT);

    draw_activity(frame, regions.activity, state, use_nf, palette);
    draw_panel(
        frame,
        regions.left,
        left,
        state.focus == Focus::Left,
        palette,
    );
    draw_panel(
        frame,
        regions.right,
        right,
        state.focus == Focus::Right,
        palette,
    );

    let nf = match nerd_font.available {
        Some(true) => "nf",
        Some(false) => "no-nf",
        None => "nf?",
    };
    let hint = format!(
        " corral  {}  {}  {}  {}  1/2/3 features  Tab focus  q quit ",
        ctx.mode.label(),
        palette.name,
        state.feature.id(),
        nf
    );
    let bar = Paragraph::new(hint).style(Style::default().fg(palette.subtext0));
    let bar_area = Rect {
        x: area.x,
        y: area.y.saturating_add(area.height.saturating_sub(1)),
        width: area.width,
        height: 1,
    };
    frame.render_widget(bar, bar_area);
}

fn draw_activity(
    frame: &mut Frame,
    area: Rect,
    state: &mut State,
    use_nf: bool,
    palette: &Palette,
) {
    let active_bar = state.focus == Focus::Activity;
    let border = if active_bar {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.overlay0)
    };

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(border)
        .style(Style::default().bg(palette.surface_dim));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut y = inner.y.saturating_add(1);
    for feature in Feature::ALL {
        if y >= inner.y.saturating_add(inner.height) {
            break;
        }
        let selected = feature == state.feature;
        let icon = feature.icon(use_nf);

        let style = if selected {
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else if active_bar {
            Style::default().fg(palette.text)
        } else {
            Style::default().fg(palette.subtext0)
        };

        // Center icon in the 1-cell content column when possible.
        let btn = Rect {
            x: inner.x,
            y,
            width: inner.width.max(1),
            height: 1,
        };
        let label = if inner.width >= 2 {
            format!(" {icon}")
        } else {
            icon.to_string()
        };
        frame.render_widget(Paragraph::new(Line::from(Span::styled(label, style))), btn);
        state.activity_hits.push((
            feature,
            Rect {
                x: area.x,
                y,
                width: area.width.max(1),
                height: ACTIVITY_BTN_HEIGHT,
            },
        ));
        y = y.saturating_add(ACTIVITY_BTN_HEIGHT);
    }
}

fn draw_panel(
    frame: &mut Frame,
    area: Rect,
    view: &PanelView,
    focused: bool,
    palette: &Palette,
) {
    let border = if focused {
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.overlay0)
    };

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .style(Style::default().bg(palette.panel_bg).fg(palette.text));

    if let Some(title) = view.title.as_deref().filter(|t| !t.is_empty()) {
        let title_style = if focused {
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.subtext0)
        };
        block = block.title(Span::styled(format!(" {title} "), title_style));
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if !view.body.is_empty() {
        let body = Paragraph::new(view.body.as_str()).style(Style::default().fg(palette.subtext0));
        frame.render_widget(body, inner);
    }
}

fn setup() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
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
