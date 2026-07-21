//! Sidebar TUI: activity strip + active feature body.
//!
//! Shape matches herdr-sidebar:
//! - one left-docked Herdr pane
//! - Explorer / SCM / GitHub switch in-process via the top icon row
//! - previews are NOT drawn here (separate pane later via control file)
//!
//! Key ownership:
//! - **shell**: `q`, `Ctrl-C`, `1`/`2`/`3`, activity icon clicks
//! - **active feature body**: everything else (`j`/`k`, …) via [`FeatureView`]

use crate::feature::{Feature, KeyOutcome, Views};
use crate::herdr;
use crate::host::LaunchContext;
use crate::ui::{self, ActivityItem, Palette};
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::{Frame, Terminal};
use std::io::{self, Stdout};
use std::time::Duration;

struct State {
    feature: Feature,
    views: Views,
    /// Last label pushed to Herdr (avoid spamming rename every frame).
    labeled_as: Option<&'static str>,
    nav_hits: Vec<(Feature, Rect)>,
}

impl State {
    fn new(ctx: &LaunchContext, nerd_font: bool) -> Self {
        Self {
            feature: Feature::Explorer,
            views: Views::new(&ctx.cwd, nerd_font),
            labeled_as: None,
            nav_hits: Vec::new(),
        }
    }
}

/// Run the sidebar TUI until the user quits.
pub fn run(ctx: LaunchContext) -> io::Result<()> {
    let _ = std::env::set_current_dir(&ctx.cwd);

    let palette = Palette::resolve();
    let use_nf = ui::detect_nerd_font().should_use_icons();
    // TermGuard restores the terminal on Drop (normal return *and* panic).
    let mut term = TermGuard::enter()?;
    event_loop(term.terminal(), &palette, use_nf, &ctx)
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    palette: &Palette,
    use_nf: bool,
    ctx: &LaunchContext,
) -> io::Result<()> {
    let mut state = State::new(ctx, use_nf);

    // Initial Herdr border title = active feature.
    sync_pane_label(&mut state, ctx);
    state.views.get_mut(state.feature).on_activate();

    loop {
        terminal.draw(|frame| {
            state.nav_hits.clear();
            draw(frame, palette, use_nf, &mut state);
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
                    state.views.get_mut(state.feature).on_activate();
                    sync_pane_label(&mut state, ctx);
                }
            }
            Event::Mouse(m) => {
                if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                    let before = state.feature;
                    if !handle_activity_click(&mut state, m.column, m.row) {
                        // Not an activity hit — body may handle later.
                        let _ = state.views.get_mut(state.feature).on_mouse(m);
                    }
                    if state.feature != before {
                        state.views.get_mut(state.feature).on_activate();
                        sync_pane_label(&mut state, ctx);
                    }
                } else {
                    let _ = state.views.get_mut(state.feature).on_mouse(m);
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
    let ok = herdr::set_pane_label(ctx.herdr_bin(), ctx.self_pane_id(), title);
    if ok {
        state.labeled_as = Some(title);
    }
}

/// Shell keys first; everything else goes to the active feature body.
/// Returns true if the event loop should quit.
fn handle_key(state: &mut State, code: KeyCode, mods: KeyModifiers) -> bool {
    // --- shell chords (never stolen by features) ---
    match code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => return true,
        // Esc reserved for closing preview later; not quit.
        KeyCode::Esc => return false,

        // Feature switch: digits only (Explorer needs j/k for lists).
        KeyCode::Char(c @ '1'..='3') => {
            if let Some(f) = Feature::from_digit(c) {
                state.feature = f;
            }
            return false;
        }
        _ => {}
    }

    // --- body ---
    match state.views.get_mut(state.feature).on_key(code, mods) {
        KeyOutcome::Handled | KeyOutcome::Ignored => {}
    }
    false
}

/// Returns true if the click hit the activity strip (feature switch).
fn handle_activity_click(state: &mut State, col: u16, row: u16) -> bool {
    for (feature, rect) in &state.nav_hits {
        if ui::hit(*rect, col, row) {
            state.feature = *feature;
            return true;
        }
    }
    false
}

fn draw(frame: &mut Frame, palette: &Palette, use_nf: bool, state: &mut State) {
    let area = frame.area();

    // No own outer border/title: herdr already frames the pane and labels it.
    frame.render_widget(
        Block::default().style(Style::default().bg(palette.panel_bg).fg(palette.text)),
        area,
    );

    let (activity, body) = ui::layout::split_sidebar(area);

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

    state.views.get(state.feature).draw(frame, body, palette);
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
