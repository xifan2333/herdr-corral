//! Single-pane host TUI: draw left + right containers and own the event loop.
//!
//! Works as a Herdr plugin pane *or* a standalone terminal binary. Host-specific
//! details come from [`crate::host::LaunchContext`]; the shell itself does not
//! require Herdr.
//!
//! Panel titles/content are supplied by features via [`PanelView`]. The shell
//! does not hardcode "left"/"right" labels.

use crate::host::LaunchContext;
use crate::layout::{self, Focus, PanelView};
use crate::theme::Palette;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use std::io::{self, Stdout};
use std::time::Duration;

/// Left container share of the host width (percent).
const LEFT_PCT: u16 = 30;

/// Run the host TUI until the user quits.
pub fn run(ctx: LaunchContext) -> io::Result<()> {
    let _ = std::env::set_current_dir(&ctx.cwd);

    let palette = Palette::resolve();
    let mut terminal = setup()?;
    let result = event_loop(&mut terminal, &palette, &ctx);
    restore(&mut terminal)?;
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    palette: &Palette,
    ctx: &LaunchContext,
) -> io::Result<()> {
    let mut focus = Focus::Left;

    loop {
        // Features will supply these later; empty title for now.
        let left = PanelView {
            title: None,
            body: String::new(),
        };
        let right = PanelView {
            title: None,
            body: String::new(),
        };

        terminal.draw(|frame| draw(frame, palette, focus, &left, &right, ctx))?;

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

fn draw(
    frame: &mut Frame,
    palette: &Palette,
    focus: Focus,
    left: &PanelView,
    right: &PanelView,
    ctx: &LaunchContext,
) {
    let area = frame.area();
    let containers = layout::split(area, LEFT_PCT);

    draw_panel(
        frame,
        containers.left,
        left,
        focus == Focus::Left,
        palette,
    );
    draw_panel(
        frame,
        containers.right,
        right,
        focus == Focus::Right,
        palette,
    );

    // Status line: plain text only, no background block.
    let hint = format!(
        " corral  {}  {}  q quit ",
        ctx.mode.label(),
        palette.name
    );
    let bar = Paragraph::new(hint).style(Style::default().fg(palette.subtext0));
    let bar_area = ratatui::layout::Rect {
        x: area.x,
        y: area.y.saturating_add(area.height.saturating_sub(1)),
        width: area.width,
        height: 1,
    };
    frame.render_widget(bar, bar_area);
}

fn draw_panel(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    view: &PanelView,
    focused: bool,
    palette: &Palette,
) {
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
