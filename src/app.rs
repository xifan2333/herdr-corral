//! Single-pane host TUI: draw left + right containers and own the event loop.
//!
//! This is the corral shell. Future views mount into the containers; for now
//! each container is just a themed empty shell so the layout skeleton is real.

use crate::layout::{self, Focus};
use crate::theme::Palette;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use std::io::{self, Stdout};
use std::time::Duration;

/// Left container share of the host width (percent).
const LEFT_PCT: u16 = 30;

/// Run the host TUI until the user quits.
pub fn run() -> io::Result<()> {
    let palette = Palette::resolve();
    let mut terminal = setup()?;
    let result = event_loop(&mut terminal, &palette);
    restore(&mut terminal)?;
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    palette: &Palette,
) -> io::Result<()> {
    let mut focus = Focus::Left;

    loop {
        terminal.draw(|frame| draw(frame, palette, focus))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Char('q') => break,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
            KeyCode::Esc => break,
            KeyCode::Tab | KeyCode::BackTab => focus = focus.toggle(),
            KeyCode::Char('h') | KeyCode::Left => focus = Focus::Left,
            KeyCode::Char('l') | KeyCode::Right => focus = Focus::Right,
            _ => {}
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame, palette: &Palette, focus: Focus) {
    let area = frame.area();
    let containers = layout::split(area, LEFT_PCT);

    draw_container(
        frame,
        containers.left,
        " left ",
        "sidebar container\n(future: Explorer / SCM / GitHub)",
        focus == Focus::Left,
        palette,
    );
    draw_container(
        frame,
        containers.right,
        " right ",
        "main container\n(future views mount here)",
        focus == Focus::Right,
        palette,
    );

    // Bottom hint bar.
    let hint = format!(
        " corral  focus={}  Tab focus  h/l left/right  q quit ",
        focus.label()
    );
    let bar = Paragraph::new(Line::from(Span::styled(
        hint,
        Style::default().fg(palette.subtext0).bg(palette.surface_dim),
    )));
    let bar_area = ratatui::layout::Rect {
        x: area.x,
        y: area.y.saturating_add(area.height.saturating_sub(1)),
        width: area.width,
        height: 1,
    };
    // Overwrite the bottom row of both containers with the status bar.
    frame.render_widget(bar, bar_area);
}

fn draw_container(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    title: &str,
    body: &str,
    focused: bool,
    palette: &Palette,
) {
    // Leave the bottom status row alone if this container reaches it.
    let area = if area.height > 0 {
        ratatui::layout::Rect {
            height: area.height.saturating_sub(1),
            ..area
        }
    } else {
        area
    };

    let border = if focused {
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.overlay0)
    };
    let title_style = if focused {
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.subtext0)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(Span::styled(title, title_style))
        .style(Style::default().bg(palette.panel_bg).fg(palette.text));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let body = Paragraph::new(body).style(Style::default().fg(palette.subtext0));
    frame.render_widget(body, inner);
}

fn setup() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
