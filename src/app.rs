//! Single-pane host TUI: left panel (nav + feature) | right panel.
//!
//! Feature switcher is a **horizontal icon row inside the left panel**
//! (as in the reference screenshot), not a separate leftmost activity rail.

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

/// Left container share of width (percent).
const LEFT_PCT: u16 = 32;

/// Columns per nav icon button (padding + glyph + padding) — reads larger.
const NAV_BTN_WIDTH: u16 = 5;

struct State {
    feature: Feature,
    focus: Focus,
    /// Hit targets for left-panel nav icons, filled each draw.
    nav_hits: Vec<(Feature, Rect)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            feature: Feature::Explorer,
            focus: Focus::Left,
            nav_hits: Vec::new(),
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
            state.nav_hits.clear();
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

        KeyCode::Char(c @ '1'..='3') => {
            if let Some(f) = Feature::from_digit(c) {
                state.feature = f;
                state.focus = Focus::Left;
            }
        }

        KeyCode::Tab | KeyCode::BackTab => state.focus = state.focus.toggle(),

        // When left is focused, h/l still move focus; j/k cycle features.
        KeyCode::Char('j') | KeyCode::Down if state.focus == Focus::Left => {
            state.feature = state.feature.next();
        }
        KeyCode::Char('k') | KeyCode::Up if state.focus == Focus::Left => {
            state.feature = state.feature.prev();
        }
        KeyCode::Char('h') | KeyCode::Left => state.focus = Focus::Left,
        KeyCode::Char('l') | KeyCode::Right => state.focus = Focus::Right,

        _ => {}
    }
    false
}

fn handle_click(state: &mut State, col: u16, row: u16) {
    for (feature, rect) in &state.nav_hits {
        if contains(*rect, col, row) {
            state.feature = *feature;
            state.focus = Focus::Left;
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

fn left_view(feature: Feature) -> PanelView {
    PanelView {
        // Title comes from the feature; nav icons sit under the border.
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

    draw_left_panel(
        frame,
        regions.left,
        left,
        state,
        use_nf,
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
        " corral  {}  {}  {}  {}  1/2/3  Tab  q ",
        ctx.mode.label(),
        palette.name,
        state.feature.id(),
        nf
    );
    let bar = Paragraph::new(hint).style(Style::default().fg(palette.subtext0));
    frame.render_widget(
        bar,
        Rect {
            x: area.x,
            y: area.y.saturating_add(area.height.saturating_sub(1)),
            width: area.width,
            height: 1,
        },
    );
}

/// Left panel: border + title, then horizontal feature icons, then body.
fn draw_left_panel(
    frame: &mut Frame,
    area: Rect,
    view: &PanelView,
    state: &mut State,
    use_nf: bool,
    focused: bool,
    palette: &Palette,
) {
    let border = if focused {
        Style::default().fg(palette.overlay1)
    } else {
        Style::default().fg(palette.overlay0)
    };

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .style(Style::default().bg(palette.panel_bg).fg(palette.text));

    if let Some(title) = view.title.as_deref().filter(|t| !t.is_empty()) {
        let title_style = Style::default().fg(palette.subtext0).add_modifier(
            if focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            },
        );
        block = block.title(Span::styled(format!(" {title} "), title_style));
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let (nav, body) = layout::split_left_nav(inner);
    draw_feature_nav(frame, nav, state, use_nf, palette);

    if !view.body.is_empty() {
        frame.render_widget(
            Paragraph::new(view.body.as_str()).style(Style::default().fg(palette.subtext0)),
            body,
        );
    }
}

/// Horizontal Nerd Font buttons inside the left panel.
/// Selected item: filled background chip; unselected: muted glyph only.
fn draw_feature_nav(
    frame: &mut Frame,
    area: Rect,
    state: &mut State,
    use_nf: bool,
    palette: &Palette,
) {
    // Vertical center of the nav strip (NAV_HEIGHT is typically 3).
    let icon_y = area.y.saturating_add(area.height.saturating_sub(1) / 2);
    let mut x = area.x.saturating_add(1);

    for feature in Feature::ALL {
        let avail = area
            .x
            .saturating_add(area.width)
            .saturating_sub(x);
        if avail < 3 {
            break;
        }
        let w = NAV_BTN_WIDTH.min(avail);
        let selected = feature == state.feature;
        let icon = feature.icon(use_nf);

        let (fg, bg) = if selected {
            // Accent text on a solid chip so the active feature reads clearly.
            (palette.text, palette.surface1)
        } else {
            (palette.overlay1, palette.panel_bg)
        };
        let style = Style::default().fg(fg).bg(bg).add_modifier(if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        });

        // Full chip background across button width (and full nav strip height for hit feel).
        let chip = Rect {
            x,
            y: area.y,
            width: w,
            height: area.height.max(1),
        };
        frame.render_widget(Block::default().style(Style::default().bg(bg)), chip);

        // Center the glyph inside the chip.
        let glyph_x = x.saturating_add(w.saturating_sub(1) / 2);
        let glyph = Rect {
            x: glyph_x,
            y: icon_y,
            width: 1,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(icon, style))),
            glyph,
        );

        state.nav_hits.push((feature, chip));
        x = x.saturating_add(w.saturating_add(1)); // 1-col gap between chips
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
        Style::default().fg(palette.overlay1)
    } else {
        Style::default().fg(palette.overlay0)
    };

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .style(Style::default().bg(palette.panel_bg).fg(palette.text));

    if let Some(title) = view.title.as_deref().filter(|t| !t.is_empty()) {
        let title_style = Style::default().fg(palette.subtext0).add_modifier(
            if focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            },
        );
        block = block.title(Span::styled(format!(" {title} "), title_style));
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if !view.body.is_empty() {
        frame.render_widget(
            Paragraph::new(view.body.as_str()).style(Style::default().fg(palette.subtext0)),
            inner,
        );
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
