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
use crate::ui::{self, ActivityItem};
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
    // TermGuard restores the terminal on Drop (normal return *and* panic).
    let mut term = TermGuard::enter()?;
    event_loop(term.terminal(), &palette, &nerd_font, &ctx)
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
    // Only rename *this* pane. Never fall back to invocation-focused neighbor.
    let ok = herdr_cli::set_pane_label(ctx.herdr_bin(), ctx.self_pane_id(), title);
    if ok {
        state.labeled_as = Some(title);
    }
    // If rename failed (standalone / herdr not ready), leave labeled_as unset
    // so the next feature switch or loop can retry.
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
        if ui::hit(*rect, col, row) {
            state.feature = *feature;
            return;
        }
    }
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
    frame.render_widget(
        Block::default().style(Style::default().bg(palette.panel_bg).fg(palette.text)),
        area,
    );

    let (activity, body, footer) = layout::split_sidebar(area, true);

    let items: Vec<ActivityItem> = Feature::ALL
        .iter()
        .map(|&feature| ActivityItem {
            feature,
            icon: feature.icon(use_nf),
            double_width: feature.icon_double_width(use_nf),
        })
        .collect();
    let bar = ui::draw_activity(frame, activity, &items, state.feature, palette);
    state.nav_hits = bar.hits;

    draw_body(frame, body, state.feature, palette, ctx);
    draw_footer(frame, footer, palette, nerd_font, state.feature, ctx);
}

fn draw_body(
    frame: &mut Frame,
    area: Rect,
    feature: Feature,
    palette: &Palette,
    ctx: &LaunchContext,
) {
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

/// Owns terminal raw mode / alt screen / mouse capture and always tears them
/// down in [`Drop`], including on panic paths through `event_loop`.
struct TermGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    /// Set after a successful explicit restore so Drop is a no-op.
    restored: bool,
}

impl TermGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        // Colors are UI, not pipeable output — ignore NO_COLOR from agent shells
        // (same rationale as herdr-sidebar).
        crossterm::style::force_color_output(true);
        // If alt-screen / mouse setup fails after raw mode is on, leave raw mode.
        if let Err(e) = execute!(
            stdout,
            EnterAlternateScreen,
            crossterm::event::EnableMouseCapture
        ) {
            let _ = disable_raw_mode();
            return Err(e);
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self {
                terminal,
                restored: false,
            }),
            Err(e) => {
                let mut out = io::stdout();
                let _ = execute!(
                    out,
                    crossterm::event::DisableMouseCapture,
                    LeaveAlternateScreen
                );
                let _ = disable_raw_mode();
                Err(e)
            }
        }
    }

    fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    fn restore_now(&mut self) {
        if self.restored {
            return;
        }
        // Best-effort: each step independent so one failure does not skip the rest.
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            crossterm::event::DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
        self.restored = true;
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        self.restore_now();
    }
}
