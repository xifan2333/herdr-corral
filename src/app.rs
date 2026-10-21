//! Sidebar TUI: activity strip + active feature body.
//!
//! Shape matches herdr-sidebar:
//! - one left-docked Herdr pane
//! - Explorer / SCM / GitHub switch in-process via the top icon row
//! - previews are NOT drawn here (separate pane later via control file)
//!
//! Key ownership is configured entirely through `config.sh`. The shell handles
//! global action names (quit / feature switching); active views handle the
//! remaining configured actions. Activity icon clicks remain direct mouse UI.

use crate::config::{self, Config};
use crate::feature::{Feature, KeyOutcome, Views};
use crate::herdr;
use crate::host::LaunchContext;
use crate::ui::{self, ActivityItem, Palette};
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Frame, Terminal};
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const DIRECTORY_POLL_INTERVAL: Duration = Duration::from_millis(500);

struct State {
    feature: Feature,
    views: Views,
    config: Arc<Config>,
    /// Last label pushed to Herdr (avoid spamming rename every frame).
    labeled_as: Option<&'static str>,
    last_heartbeat: Option<Instant>,
    nav_hits: Vec<(Feature, Rect)>,
    cwd: PathBuf,
    last_directory_poll: Option<Instant>,
    pending_directory: Option<(PathBuf, mpsc::Receiver<Views>)>,
    directory_error: Option<String>,
}

impl State {
    fn new(ctx: &LaunchContext, nerd_font: bool, config: Arc<Config>) -> Self {
        Self {
            feature: Feature::Explorer,
            views: Views::new(&ctx.cwd, nerd_font, Arc::clone(&config)),
            config,
            labeled_as: None,
            last_heartbeat: None,
            nav_hits: Vec::new(),
            cwd: ctx.cwd.clone(),
            last_directory_poll: None,
            pending_directory: None,
            directory_error: None,
        }
    }
}

/// Run the sidebar TUI until the user quits.
///
/// # Errors
///
/// Returns an [`io::Error`] if terminal setup, event reading, or shell action execution fails.
pub fn run(ctx: &LaunchContext) -> io::Result<()> {
    std::env::set_current_dir(&ctx.cwd)?;

    let palette = Palette::resolve();
    let use_nf = ui::detect_nerd_font().should_use_icons();
    let config = Arc::new(Config::load());
    // TermGuard restores the terminal on Drop (normal return *and* panic).
    let mut term = TermGuard::enter()?;
    event_loop(term.terminal(), &palette, use_nf, ctx, config)
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    palette: &Palette,
    use_nf: bool,
    ctx: &LaunchContext,
    config: Arc<Config>,
) -> io::Result<()> {
    let mut state = State::new(ctx, use_nf, config);

    // Stable label + metadata identify this process independently of its
    // active feature. Activity identity remains inside the top icon strip.
    sync_pane_label(&mut state, ctx);
    sync_sidebar_heartbeat(&mut state, ctx);
    state.views.get_mut(state.feature).on_activate();

    loop {
        // Tick before polling so continuous mouse/key input cannot starve live
        // refresh; expensive views rate-limit themselves.
        state.views.get_mut(state.feature).on_tick();
        sync_sidebar_heartbeat(&mut state, ctx);
        sync_shell_directory(&mut state, ctx, use_nf)?;
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
                match handle_key(&mut state, key.code, key.modifiers) {
                    KeyHandle::Quit => break,
                    KeyHandle::Continue => {}
                    KeyHandle::Shell { action, file, env } => {
                        let ok =
                            run_shell_action(terminal, &state, &action, file.as_deref(), &env)?;
                        state
                            .views
                            .get_mut(state.feature)
                            .on_shell_result(&action, ok);
                    }
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
                        if let KeyOutcome::Shell { action, file, env } =
                            state.views.get_mut(state.feature).on_mouse(m)
                        {
                            let ok =
                                run_shell_action(terminal, &state, &action, file.as_deref(), &env)?;
                            state
                                .views
                                .get_mut(state.feature)
                                .on_shell_result(&action, ok);
                        }
                    }
                    if state.feature != before {
                        state.views.get_mut(state.feature).on_activate();
                        sync_pane_label(&mut state, ctx);
                    }
                } else if let KeyOutcome::Shell { action, file, env } =
                    state.views.get_mut(state.feature).on_mouse(m)
                {
                    let ok = run_shell_action(terminal, &state, &action, file.as_deref(), &env)?;
                    state
                        .views
                        .get_mut(state.feature)
                        .on_shell_result(&action, ok);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn sync_shell_directory(state: &mut State, ctx: &LaunchContext, use_nf: bool) -> io::Result<()> {
    if let Some((path, receiver)) = &state.pending_directory {
        match receiver.try_recv() {
            Ok(views) => {
                match std::env::set_current_dir(path) {
                    Ok(()) => {
                        state.cwd = path.clone();
                        state.views = views;
                        state.views.get_mut(state.feature).on_activate();
                        state.directory_error = None;
                    }
                    Err(error) => state.directory_error = Some(error.to_string()),
                }
                state.pending_directory = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(io::Error::other(
                    "directory view loader stopped unexpectedly",
                ));
            }
        }
    }
    let (Some(bin), Some(sidebar)) = (ctx.herdr_bin(), ctx.self_pane_id()) else {
        return Ok(());
    };
    if state
        .last_directory_poll
        .is_some_and(|last| last.elapsed() < DIRECTORY_POLL_INTERVAL)
    {
        return Ok(());
    }
    state.last_directory_poll = Some(Instant::now());
    match herdr::cli::shell_cwd(bin, sidebar) {
        Ok(Some(path)) => {
            if path == state.cwd {
                state.pending_directory = None;
            } else if state
                .pending_directory
                .as_ref()
                .is_none_or(|(pending, _)| *pending != path)
            {
                let (sender, receiver) = mpsc::channel();
                let config = Arc::clone(&state.config);
                let target = path.clone();
                std::thread::spawn(move || {
                    let views = Views::new(&target, use_nf, config);
                    // A newer shell directory can supersede this load.
                    let _ = sender.send(views);
                });
                state.pending_directory = Some((path, receiver));
            }
            state.directory_error = None;
        }
        Ok(None) => {}
        Err(error) => state.directory_error = Some(error),
    }
    Ok(())
}

enum KeyHandle {
    Quit,
    Continue,
    Shell {
        action: String,
        file: Option<PathBuf>,
        env: Vec<(String, String)>,
    },
}

fn run_shell_action(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &State,
    action: &str,
    file: Option<&std::path::Path>,
    env: &[(String, String)],
) -> io::Result<bool> {
    // Hosted (Herdr / WezTerm) actions use side-pane reuse — never leave the
    // alt-screen. Standalone $EDITOR still gets a real TTY after probing.
    let hosted =
        std::env::var_os("HERDR_ENV").is_some() || std::env::var_os("WEZTERM_PANE").is_some();

    if hosted {
        // Keep TUI up; capture action stdout/stderr.
        let result = state.config.run_shell(action, file, env, false);
        return Ok(result.ok);
    }

    // Standalone: suspend TUI for $EDITOR.
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen
    );
    let _ = terminal.show_cursor();

    let result = state.config.run_shell(action, file, env, true);

    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let _ = terminal.hide_cursor();
    terminal.clear()?;
    Ok(result.ok)
}

fn sync_pane_label(state: &mut State, ctx: &LaunchContext) {
    let title = herdr::SIDEBAR_LABEL;
    if state.labeled_as == Some(title) {
        return;
    }
    // Only rename *this* pane. Never fall back to invocation-focused neighbor.
    let ok = herdr::set_pane_label(ctx.herdr_bin(), ctx.self_pane_id(), title);
    if ok {
        state.labeled_as = Some(title);
    }
}

fn sync_sidebar_heartbeat(state: &mut State, ctx: &LaunchContext) {
    if state
        .last_heartbeat
        .is_some_and(|last| last.elapsed() < HEARTBEAT_INTERVAL)
    {
        return;
    }
    if herdr::report_sidebar_heartbeat(ctx.herdr_bin(), ctx.self_pane_id()) {
        state.last_heartbeat = Some(Instant::now());
    }
}

/// Resolve configured global actions first; views dispatch everything else.
fn handle_key(state: &mut State, code: KeyCode, mods: KeyModifiers) -> KeyHandle {
    if state.views.get(state.feature).captures_text_input() {
        return key_outcome(state.views.get_mut(state.feature).on_key(code, mods));
    }

    if let Some(action) =
        config::key_token(code, mods).and_then(|token| state.config.action_for_key(&token))
    {
        match action {
            config::internal::QUIT => return KeyHandle::Quit,
            config::internal::FEATURE_EXPLORER => {
                state.feature = Feature::Explorer;
                return KeyHandle::Continue;
            }
            config::internal::FEATURE_SCM => {
                state.feature = Feature::Scm;
                return KeyHandle::Continue;
            }
            config::internal::FEATURE_GITHUB => {
                state.feature = Feature::GitHub;
                return KeyHandle::Continue;
            }
            _ => {}
        }
    }

    key_outcome(state.views.get_mut(state.feature).on_key(code, mods))
}

fn key_outcome(outcome: KeyOutcome) -> KeyHandle {
    match outcome {
        KeyOutcome::Handled | KeyOutcome::Ignored => KeyHandle::Continue,
        KeyOutcome::Shell { action, file, env } => KeyHandle::Shell { action, file, env },
    }
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
    let mut area = frame.area();
    let status = state.directory_error.as_ref().map_or_else(
        || {
            state
                .pending_directory
                .as_ref()
                .map(|(path, _)| (format!("Opening {}", path.display()), palette.subtext0))
        },
        |error| Some((format!("Shell directory: {error}"), palette.red)),
    );
    if let Some((text, color)) = status.filter(|_| area.height > 0) {
        area.height -= 1;
        let footer = Rect::new(area.x, area.y + area.height, area.width, 1);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(text)
                .style(ratatui::style::Style::default().fg(color)),
            footer,
        );
    }

    // Transparent pane: no fill — host terminal bg shows through.
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

    const fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
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
