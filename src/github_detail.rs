//! Full-width interactive GitHub detail client used by `corral-github`.
//!
//! The 32-column sidebar remains a navigator. This app runs in the shared
//! owner-scoped nvim terminal and owns resource detail presentation.

use crate::config::{self, Config};
use crate::github::{
    Comment, GhCli, GitHubDetailAdapter, GitHubMutation, IssueDetail, PullRequestDetail, Review,
    WorkflowRunDetail,
};
use crate::ui::Palette;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};
use serde_json::Value;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const NOTICE_SUCCESS_TTL: Duration = Duration::from_secs(2);
const NOTICE_ERROR_TTL: Duration = Duration::from_secs(4);
const MAX_MESSAGE_CHARS: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailResource {
    Issue(u64),
    Pull(u64),
    Run(u64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InitialView {
    #[default]
    Overview,
    Conversation,
    Files,
    Diff,
    Checks,
    Jobs,
    Log,
    FailedLog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Overview,
    Conversation,
    Files,
    Diff,
    Checks,
    Jobs,
    Log,
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Overview => "OVERVIEW",
            Tab::Conversation => "CONVERSATION",
            Tab::Files => "FILES",
            Tab::Diff => "DIFF",
            Tab::Checks => "CHECKS",
            Tab::Jobs => "JOBS",
            Tab::Log => "LOG",
        }
    }
}

enum Detail {
    Issue(IssueDetail),
    Pull(PullRequestDetail),
    Run(WorkflowRunDetail),
}

enum Payload {
    Detail(Box<Detail>),
    Patch(String),
    Log { text: String, failed_only: bool },
    Mutation(String),
}

#[derive(Clone, Copy)]
enum Request {
    Detail,
    Patch,
    Log,
    Mutation,
}

#[derive(Clone, Copy)]
enum ComposeKind {
    Comment,
    RequestChanges,
}

enum Mode {
    Browse,
    Compose {
        kind: ComposeKind,
        text: Vec<char>,
    },
    Confirm {
        message: String,
        mutation: GitHubMutation,
    },
}

struct Completion {
    generation: u64,
    request: Request,
    result: Result<Payload, String>,
}

struct DetailApp {
    repo: String,
    resource: DetailResource,
    adapter: Arc<dyn GitHubDetailAdapter>,
    config: Arc<Config>,
    detail: Option<Detail>,
    patch: Option<String>,
    patch_error: Option<String>,
    log: Option<(String, bool)>,
    log_error: Option<String>,
    active_tab: usize,
    scroll: usize,
    body_height: u16,
    loading_detail: bool,
    loading_patch: bool,
    loading_log: bool,
    mutation_loading: bool,
    mode: Mode,
    notice: Option<(String, bool, Instant)>,
    error: Option<String>,
    content_revision: u64,
    rendered_key: Option<(Tab, u16, u64)>,
    rendered_lines: Vec<Line<'static>>,
    generation: u64,
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
}

impl DetailApp {
    fn new(
        repo: String,
        resource: DetailResource,
        initial: InitialView,
        config: Arc<Config>,
    ) -> Self {
        let adapter = Arc::new(GhCli::new(PathBuf::from(".")));
        Self::with_adapter(repo, resource, initial, config, adapter)
    }

    fn with_adapter(
        repo: String,
        resource: DetailResource,
        initial: InitialView,
        config: Arc<Config>,
        adapter: Arc<dyn GitHubDetailAdapter>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut app = Self {
            repo,
            resource,
            adapter,
            config,
            detail: None,
            patch: None,
            patch_error: None,
            log: None,
            log_error: None,
            active_tab: 0,
            scroll: 0,
            body_height: 0,
            loading_detail: false,
            loading_patch: false,
            loading_log: false,
            mutation_loading: false,
            mode: Mode::Browse,
            notice: None,
            error: None,
            content_revision: 0,
            rendered_key: None,
            rendered_lines: Vec::new(),
            generation: 0,
            sender,
            receiver,
        };
        let target = app.tab_for_initial(initial);
        app.active_tab = app
            .tabs()
            .iter()
            .position(|tab| *tab == target)
            .unwrap_or(0);
        app.start_detail();
        if target == Tab::Diff {
            app.start_patch();
        } else if target == Tab::Log {
            app.start_log(matches!(initial, InitialView::FailedLog));
        }
        app
    }

    fn tabs(&self) -> &'static [Tab] {
        tabs_for(self.resource)
    }

    fn tab_for_initial(&self, initial: InitialView) -> Tab {
        match (self.resource, initial) {
            (DetailResource::Issue(_), InitialView::Conversation) => Tab::Conversation,
            (DetailResource::Pull(_), InitialView::Conversation) => Tab::Conversation,
            (DetailResource::Pull(_), InitialView::Files) => Tab::Files,
            (DetailResource::Pull(_), InitialView::Diff) => Tab::Diff,
            (DetailResource::Pull(_), InitialView::Checks) => Tab::Checks,
            (DetailResource::Run(_), InitialView::Jobs) => Tab::Jobs,
            (DetailResource::Run(_), InitialView::Log | InitialView::FailedLog) => Tab::Log,
            _ => Tab::Overview,
        }
    }

    fn active_tab(&self) -> Tab {
        self.tabs()[self.active_tab.min(self.tabs().len().saturating_sub(1))]
    }

    fn start_detail(&mut self) {
        if self.loading_detail {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.loading_detail = true;
        self.error = None;
        let repo = self.repo.clone();
        let resource = self.resource;
        let adapter = Arc::clone(&self.adapter);
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = match resource {
                DetailResource::Issue(number) => adapter
                    .issue_detail(&repo, number)
                    .map(Detail::Issue)
                    .map(Box::new)
                    .map(Payload::Detail),
                DetailResource::Pull(number) => adapter
                    .pull_detail(&repo, number)
                    .map(Detail::Pull)
                    .map(Box::new)
                    .map(Payload::Detail),
                DetailResource::Run(run_id) => adapter
                    .run_detail(&repo, run_id)
                    .map(Detail::Run)
                    .map(Box::new)
                    .map(Payload::Detail),
            };
            let _ = sender.send(Completion {
                generation,
                request: Request::Detail,
                result,
            });
        });
    }

    fn start_patch(&mut self) {
        let DetailResource::Pull(number) = self.resource else {
            return;
        };
        if self.loading_patch || self.patch.is_some() {
            return;
        }
        self.loading_patch = true;
        self.patch_error = None;
        let generation = self.generation;
        let repo = self.repo.clone();
        let adapter = Arc::clone(&self.adapter);
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = adapter.pull_patch(&repo, number).map(Payload::Patch);
            let _ = sender.send(Completion {
                generation,
                request: Request::Patch,
                result,
            });
        });
    }

    fn start_log(&mut self, failed_only: bool) {
        let DetailResource::Run(run_id) = self.resource else {
            return;
        };
        if self.loading_log
            || self
                .log
                .as_ref()
                .is_some_and(|(_, current_failed)| *current_failed == failed_only)
        {
            return;
        }
        self.loading_log = true;
        self.log_error = None;
        let generation = self.generation;
        let repo = self.repo.clone();
        let adapter = Arc::clone(&self.adapter);
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = adapter
                .run_log(&repo, run_id, failed_only)
                .map(|text| Payload::Log { text, failed_only });
            let _ = sender.send(Completion {
                generation,
                request: Request::Log,
                result,
            });
        });
    }

    fn set_notice(&mut self, message: impl Into<String>, error: bool) {
        self.notice = Some((message.into(), error, Instant::now()));
    }

    fn start_mutation(&mut self, mutation: GitHubMutation) {
        if self.mutation_loading {
            return;
        }
        self.mode = Mode::Browse;
        self.mutation_loading = true;
        self.notice = None;
        let generation = self.generation;
        let repo = self.repo.clone();
        let adapter = Arc::clone(&self.adapter);
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = adapter.mutate(&repo, &mutation).map(Payload::Mutation);
            let _ = sender.send(Completion {
                generation,
                request: Request::Mutation,
                result,
            });
        });
    }

    fn start_compose(&mut self, kind: ComposeKind) {
        if !self.mutation_loading {
            self.mode = Mode::Compose {
                kind,
                text: Vec::new(),
            };
        }
    }

    fn confirm(&mut self, message: impl Into<String>, mutation: GitHubMutation) {
        if !self.mutation_loading {
            self.mode = Mode::Confirm {
                message: message.into(),
                mutation,
            };
        }
    }

    fn comment(&mut self) {
        if matches!(
            self.resource,
            DetailResource::Issue(_) | DetailResource::Pull(_)
        ) {
            self.start_compose(ComposeKind::Comment);
        }
    }

    fn context_action(&mut self) {
        match (&self.resource, &self.detail) {
            (DetailResource::Issue(number), Some(Detail::Issue(issue))) => {
                let open = !issue.state.eq_ignore_ascii_case("open");
                self.confirm(
                    if open {
                        "Reopen this issue?"
                    } else {
                        "Close this issue?"
                    },
                    GitHubMutation::IssueState {
                        number: *number,
                        open,
                    },
                );
            }
            (DetailResource::Pull(_), Some(Detail::Pull(pull)))
                if pull.state.eq_ignore_ascii_case("open") =>
            {
                self.start_compose(ComposeKind::RequestChanges);
            }
            (DetailResource::Run(run_id), Some(Detail::Run(run))) if run.status != "completed" => {
                self.confirm(
                    "Cancel this workflow run?",
                    GitHubMutation::RunCancel { run_id: *run_id },
                );
            }
            _ => self.set_notice("action unavailable for current state", true),
        }
    }

    fn close_reopen_pull(&mut self) {
        let (DetailResource::Pull(number), Some(Detail::Pull(pull))) =
            (&self.resource, &self.detail)
        else {
            return;
        };
        if pull.state.eq_ignore_ascii_case("merged") {
            self.set_notice("merged pull requests cannot be reopened", true);
            return;
        }
        let open = !pull.state.eq_ignore_ascii_case("open");
        self.confirm(
            if open {
                "Reopen this pull request?"
            } else {
                "Close this pull request?"
            },
            GitHubMutation::PullState {
                number: *number,
                open,
            },
        );
    }

    fn approve_pull(&mut self) {
        let (DetailResource::Pull(number), Some(Detail::Pull(pull))) =
            (self.resource, &self.detail)
        else {
            return;
        };
        if !pull.state.eq_ignore_ascii_case("open") {
            self.set_notice("only open pull requests can be approved", true);
            return;
        }
        self.confirm(
            "Approve this pull request?",
            GitHubMutation::PullApprove { number },
        );
    }

    fn merge_pull(&mut self) {
        let (DetailResource::Pull(number), Some(Detail::Pull(pull))) =
            (self.resource, &self.detail)
        else {
            return;
        };
        if pull.is_draft || !pull.state.eq_ignore_ascii_case("open") || pull.head_ref_oid.is_empty()
        {
            self.set_notice("pull request is not mergeable", true);
            return;
        }
        self.confirm(
            format!(
                "Squash merge #{number} at {}?",
                short_sha(&pull.head_ref_oid)
            ),
            GitHubMutation::PullMergeSquash {
                number,
                head_sha: pull.head_ref_oid.clone(),
            },
        );
    }

    fn rerun(&mut self, failed_only: bool) {
        let (DetailResource::Run(run_id), Some(Detail::Run(run))) = (self.resource, &self.detail)
        else {
            return;
        };
        if run.status != "completed" {
            self.set_notice("wait for the run to complete before rerunning", true);
            return;
        }
        self.confirm(
            if failed_only {
                "Rerun failed jobs?"
            } else {
                "Rerun all jobs?"
            },
            GitHubMutation::RunRerun {
                run_id,
                failed_only,
            },
        );
    }

    fn submit_compose(&mut self) {
        let Mode::Compose { kind, text } = &self.mode else {
            return;
        };
        let body = text.iter().collect::<String>().trim().to_string();
        if body.is_empty() {
            self.set_notice("message cannot be empty", true);
            return;
        }
        let mutation = match (kind, self.resource) {
            (ComposeKind::Comment, DetailResource::Issue(number)) => {
                GitHubMutation::IssueComment { number, body }
            }
            (ComposeKind::Comment, DetailResource::Pull(number)) => {
                GitHubMutation::PullComment { number, body }
            }
            (ComposeKind::RequestChanges, DetailResource::Pull(number)) => {
                GitHubMutation::PullRequestChanges { number, body }
            }
            _ => return,
        };
        self.start_mutation(mutation);
    }

    fn refresh(&mut self) {
        self.detail = None;
        self.patch = None;
        self.patch_error = None;
        self.log = None;
        self.log_error = None;
        self.scroll = 0;
        self.content_revision = self.content_revision.wrapping_add(1);
        self.start_detail();
        match self.active_tab() {
            Tab::Diff => self.start_patch(),
            Tab::Log => self.start_log(false),
            _ => {}
        }
    }

    fn select_tab(&mut self, tab: Tab) {
        if let Some(index) = self.tabs().iter().position(|candidate| *candidate == tab) {
            self.active_tab = index;
            self.scroll = 0;
            if tab == Tab::Diff {
                self.start_patch();
            } else if tab == Tab::Log {
                self.start_log(false);
            }
        }
    }

    fn move_tab(&mut self, delta: isize) {
        let len = self.tabs().len();
        self.active_tab = self
            .active_tab
            .saturating_add_signed(delta)
            .min(len.saturating_sub(1));
        self.scroll = 0;
        match self.active_tab() {
            Tab::Diff => self.start_patch(),
            Tab::Log => self.start_log(false),
            _ => {}
        }
    }

    fn apply_completion(&mut self, completion: Completion) {
        if completion.generation != self.generation {
            return;
        }
        match completion.request {
            Request::Detail => self.loading_detail = false,
            Request::Patch => self.loading_patch = false,
            Request::Log => self.loading_log = false,
            Request::Mutation => self.mutation_loading = false,
        }
        self.content_revision = self.content_revision.wrapping_add(1);
        match completion.result {
            Ok(Payload::Detail(detail)) => self.detail = Some(*detail),
            Ok(Payload::Patch(patch)) => {
                self.patch_error = None;
                self.patch = Some(patch);
            }
            Ok(Payload::Log { text, failed_only }) => {
                self.log_error = None;
                self.log = Some((text, failed_only));
            }
            Ok(Payload::Mutation(message)) => {
                self.set_notice(
                    if message.is_empty() {
                        "operation completed"
                    } else {
                        &message
                    },
                    false,
                );
                self.detail = None;
                self.patch = None;
                self.patch_error = None;
                self.log = None;
                self.log_error = None;
                self.start_detail();
            }
            Err(error) => match completion.request {
                Request::Mutation => self.set_notice(error, true),
                Request::Patch => self.patch_error = Some(error),
                Request::Log => self.log_error = Some(error),
                Request::Detail => self.error = Some(error),
            },
        }
    }

    fn tick(&mut self) {
        while let Ok(completion) = self.receiver.try_recv() {
            self.apply_completion(completion);
        }
        if self.notice.as_ref().is_some_and(|(_, error, shown)| {
            shown.elapsed()
                >= if *error {
                    NOTICE_ERROR_TTL
                } else {
                    NOTICE_SUCCESS_TTL
                }
        }) {
            self.notice = None;
        }
    }

    fn scroll_by(&mut self, delta: isize, line_count: usize) {
        let max = line_count.saturating_sub(usize::from(self.body_height.max(1)));
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
    }

    fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers, line_count: usize) -> bool {
        let Some(token) = config::key_token(code, mods) else {
            return false;
        };
        let action = self
            .config
            .action_for_feature_key("github-detail", &token)
            .map(str::to_string);

        if self.mutation_loading {
            return false;
        }
        if matches!(self.mode, Mode::Compose { .. }) {
            match action.as_deref() {
                Some(config::internal::GITHUB_SUBMIT) => self.submit_compose(),
                Some(config::internal::GITHUB_CANCEL) => self.mode = Mode::Browse,
                Some(config::internal::EDIT_BACKSPACE) => {
                    if let Mode::Compose { text, .. } = &mut self.mode {
                        text.pop();
                    }
                }
                _ if code == KeyCode::Enter => {
                    if let Mode::Compose { text, .. } = &mut self.mode {
                        if text.len() < MAX_MESSAGE_CHARS {
                            text.push('\n');
                        }
                    }
                }
                _ => {
                    if let KeyCode::Char(ch) = code {
                        if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                            if let Mode::Compose { text, .. } = &mut self.mode {
                                if text.len() < MAX_MESSAGE_CHARS {
                                    text.push(ch);
                                }
                            }
                        }
                    }
                }
            }
            return false;
        }
        if matches!(self.mode, Mode::Confirm { .. }) {
            match action.as_deref() {
                Some(config::internal::GITHUB_CONFIRM) => {
                    let mode = std::mem::replace(&mut self.mode, Mode::Browse);
                    if let Mode::Confirm { mutation, .. } = mode {
                        self.start_mutation(mutation);
                    }
                }
                Some(config::internal::GITHUB_CANCEL) => self.mode = Mode::Browse,
                _ => {}
            }
            return false;
        }

        let Some(action) = action.as_deref() else {
            return false;
        };
        match action {
            config::internal::QUIT => return true,
            config::internal::UP => self.scroll_by(-1, line_count),
            config::internal::DOWN => self.scroll_by(1, line_count),
            config::internal::TOP => self.scroll = 0,
            config::internal::BOTTOM => self.scroll_by(isize::MAX, line_count),
            config::internal::PAGE_UP => self.scroll_by(
                -isize::try_from(self.body_height.max(2) - 1).unwrap_or(1),
                line_count,
            ),
            config::internal::PAGE_DOWN => self.scroll_by(
                isize::try_from(self.body_height.max(2) - 1).unwrap_or(1),
                line_count,
            ),
            config::internal::COLLAPSE | config::internal::GITHUB_PREV_SECTION => self.move_tab(-1),
            config::internal::EXPAND | config::internal::GITHUB_NEXT_SECTION => self.move_tab(1),
            config::internal::REFRESH => self.refresh(),
            config::internal::GITHUB_DIFF if matches!(self.resource, DetailResource::Pull(_)) => {
                self.select_tab(Tab::Diff)
            }
            config::internal::GITHUB_CHECKS if matches!(self.resource, DetailResource::Pull(_)) => {
                self.select_tab(Tab::Checks)
            }
            config::internal::GITHUB_LOG if matches!(self.resource, DetailResource::Run(_)) => {
                self.select_tab(Tab::Log);
                self.start_log(false);
            }
            config::internal::GITHUB_LOG_FAILED
                if matches!(self.resource, DetailResource::Run(_)) =>
            {
                self.select_tab(Tab::Log);
                self.log = None;
                self.start_log(true);
            }
            config::internal::GITHUB_COMMENT => self.comment(),
            config::internal::GITHUB_APPROVE => self.approve_pull(),
            config::internal::GITHUB_CONTEXT_ACTION => self.context_action(),
            config::internal::GITHUB_CLOSE_REOPEN => self.close_reopen_pull(),
            config::internal::GITHUB_MERGE => self.merge_pull(),
            config::internal::GITHUB_RERUN_FAILED => self.rerun(true),
            config::internal::GITHUB_RERUN_ALL => self.rerun(false),
            _ => {}
        }
        false
    }

    fn title(&self) -> String {
        match &self.detail {
            Some(Detail::Issue(issue)) => format!("#{}  {}", issue.number, issue.title),
            Some(Detail::Pull(pull)) => format!("#{}  {}", pull.number, pull.title),
            Some(Detail::Run(run)) => format!("{}  #{}", run.workflow_name, run.database_id),
            None => match self.resource {
                DetailResource::Issue(number) => format!("Issue #{number}"),
                DetailResource::Pull(number) => format!("Pull Request #{number}"),
                DetailResource::Run(id) => format!("Actions Run #{id}"),
            },
        }
    }

    fn state(&self) -> &str {
        match &self.detail {
            Some(Detail::Issue(issue)) => &issue.state,
            Some(Detail::Pull(pull)) if pull.is_draft => "DRAFT",
            Some(Detail::Pull(pull)) => &pull.state,
            Some(Detail::Run(run)) if run.status != "completed" => &run.status,
            Some(Detail::Run(run)) => &run.conclusion,
            None => "LOADING",
        }
    }

    fn build_lines(&self, width: u16, palette: &Palette) -> Vec<Line<'static>> {
        let width = usize::from(width.max(8));
        if let Some(error) = &self.error {
            return styled_text(error, width, Style::default().fg(palette.red));
        }
        let Some(detail) = &self.detail else {
            return vec![Line::styled(
                "Loading…",
                Style::default().fg(palette.accent),
            )];
        };
        match (detail, self.active_tab()) {
            (Detail::Issue(issue), Tab::Overview) => issue_overview(issue, width, palette),
            (Detail::Issue(issue), Tab::Conversation) => {
                comments_lines(&issue.comments, width, palette)
            }
            (Detail::Pull(pull), Tab::Overview) => pull_overview(pull, width, palette),
            (Detail::Pull(pull), Tab::Conversation) => pull_conversation(pull, width, palette),
            (Detail::Pull(pull), Tab::Files) => pull_files(pull, palette),
            (Detail::Pull(_), Tab::Diff) => {
                if let Some(error) = &self.patch_error {
                    styled_text(error, width, Style::default().fg(palette.red))
                } else {
                    self.patch.as_ref().map_or_else(
                        || {
                            vec![Line::styled(
                                "Loading diff…",
                                Style::default().fg(palette.accent),
                            )]
                        },
                        |patch| patch_lines(patch, width, palette),
                    )
                }
            }
            (Detail::Pull(pull), Tab::Checks) => check_lines(&pull.status_check_rollup, palette),
            (Detail::Run(run), Tab::Overview) => run_overview(run, width, palette),
            (Detail::Run(run), Tab::Jobs) => run_jobs(run, palette),
            (Detail::Run(_), Tab::Log) => {
                if let Some(error) = &self.log_error {
                    styled_text(error, width, Style::default().fg(palette.red))
                } else {
                    self.log.as_ref().map_or_else(
                        || {
                            vec![Line::styled(
                                "Loading log…",
                                Style::default().fg(palette.accent),
                            )]
                        },
                        |(log, _)| log_lines(log, width, palette),
                    )
                }
            }
            _ => vec![Line::styled(
                "Not available",
                Style::default().fg(palette.overlay1),
            )],
        }
    }

    fn draw(&mut self, frame: &mut Frame, palette: &Palette) -> usize {
        let area = frame.area();
        if area.height == 0 {
            return 0;
        }
        let title_width = area.width.saturating_sub(14);
        frame.render_widget(
            Paragraph::new(self.title()).style(
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect {
                width: title_width,
                height: 1,
                ..area
            },
        );
        frame.render_widget(
            Paragraph::new(format!(" {} ", self.state().to_ascii_uppercase()))
                .alignment(ratatui::layout::Alignment::Right)
                .style(Style::default().fg(state_color(self.state(), palette))),
            Rect {
                x: area.x + title_width,
                width: area.width.saturating_sub(title_width),
                height: 1,
                ..area
            },
        );

        if area.height < 2 {
            return 0;
        }
        let mut spans = Vec::new();
        for (index, tab) in self.tabs().iter().enumerate() {
            if index > 0 {
                spans.push(Span::styled("  ", Style::default()));
            }
            let style = if index == self.active_tab {
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(palette.overlay1)
            };
            spans.push(Span::styled(tab.title(), style));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                y: area.y + 1,
                height: 1,
                ..area
            },
        );

        let footer_height = u16::from(area.height >= 4);
        let body = Rect {
            x: area.x,
            y: area.y + 2,
            width: area.width,
            height: area.height.saturating_sub(2 + footer_height),
        };
        self.body_height = body.height;
        let render_key = (self.active_tab(), body.width, self.content_revision);
        if self.rendered_key != Some(render_key) {
            self.rendered_lines = self.build_lines(body.width, palette);
            self.rendered_key = Some(render_key);
        }
        let max_scroll = self
            .rendered_lines
            .len()
            .saturating_sub(usize::from(body.height.max(1)));
        self.scroll = self.scroll.min(max_scroll);
        frame.render_widget(
            Paragraph::new(
                self.rendered_lines
                    .iter()
                    .skip(self.scroll)
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            body,
        );

        if footer_height == 1 {
            let hints = if self.mutation_loading {
                "working…".to_string()
            } else if let Some((message, _, _)) = &self.notice {
                message.clone()
            } else {
                match self.resource {
                    DetailResource::Issue(_) => {
                        "c reply  x close/reopen  h/l tabs  r refresh  q back".into()
                    }
                    DetailResource::Pull(_) => {
                        "c reply  a approve  x changes  m merge  D close  d diff".into()
                    }
                    DetailResource::Run(_) => {
                        "x cancel  R failed  A rerun all  f/L logs  h/l tabs".into()
                    }
                }
            };
            let color = self
                .notice
                .as_ref()
                .map_or(palette.overlay1, |(_, error, _)| {
                    if *error {
                        palette.red
                    } else {
                        palette.green
                    }
                });
            frame.render_widget(
                Paragraph::new(hints).style(Style::default().fg(color)),
                Rect {
                    y: area.y + area.height - 1,
                    height: 1,
                    ..area
                },
            );
        }

        match &self.mode {
            Mode::Browse => {}
            Mode::Compose { kind, text } => {
                let height = area.height.saturating_sub(1).clamp(1, 10);
                let rect = Rect {
                    x: area.x.saturating_add(2),
                    y: area.y + area.height.saturating_sub(height + 1),
                    width: area.width.saturating_sub(4),
                    height,
                };
                let title = match kind {
                    ComposeKind::Comment => " Reply — Ctrl+Enter/Ctrl+S submit ",
                    ComposeKind::RequestChanges => " Request changes — Ctrl+Enter/Ctrl+S submit ",
                };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .style(Style::default().fg(palette.accent).bg(palette.panel_bg));
                let inner = block.inner(rect);
                frame.render_widget(Clear, rect);
                frame.render_widget(block, rect);
                let mut value = text.iter().collect::<String>();
                value.push('│');
                frame.render_widget(
                    Paragraph::new(value)
                        .style(Style::default().fg(palette.text).bg(palette.panel_bg))
                        .wrap(ratatui::widgets::Wrap { trim: false }),
                    inner,
                );
            }
            Mode::Confirm { message, .. } => {
                let width = area.width.saturating_sub(2).clamp(1, 64);
                let rect = Rect {
                    x: area.x + area.width.saturating_sub(width) / 2,
                    y: area.y + area.height.saturating_sub(5) / 2,
                    width,
                    height: 5.min(area.height),
                };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Confirm ")
                    .style(Style::default().fg(palette.yellow).bg(palette.panel_bg));
                let inner = block.inner(rect);
                frame.render_widget(Clear, rect);
                frame.render_widget(block, rect);
                frame.render_widget(
                    Paragraph::new(format!("{message}\n\ny / N"))
                        .style(Style::default().fg(palette.text).bg(palette.panel_bg)),
                    inner,
                );
            }
        }
        self.rendered_lines.len()
    }
}

pub fn run(repo: String, resource: DetailResource, initial: InitialView) -> io::Result<()> {
    let palette = Palette::resolve();
    let config = Arc::new(Config::load());
    let mut app = DetailApp::new(repo, resource, initial, config);
    let mut terminal = DetailTerminal::enter()?;
    let result = (|| -> io::Result<()> {
        loop {
            app.tick();
            let mut line_count = 0;
            terminal.terminal.draw(|frame| {
                line_count = app.draw(frame, &palette);
            })?;
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.handle_key(key.code, key.modifiers, line_count) {
                        break;
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => app.scroll_by(3, line_count),
                    MouseEventKind::ScrollUp => app.scroll_by(-3, line_count),
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(())
    })();
    terminal.restore();
    result
}

struct DetailTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    restored: bool,
}

impl DetailTerminal {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            crossterm::event::EnableMouseCapture
        ) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self {
                terminal,
                restored: false,
            }),
            Err(error) => {
                let mut out = io::stdout();
                let _ = execute!(
                    out,
                    crossterm::event::DisableMouseCapture,
                    LeaveAlternateScreen
                );
                let _ = disable_raw_mode();
                Err(error)
            }
        }
    }

    fn restore(&mut self) {
        if self.restored {
            return;
        }
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

impl Drop for DetailTerminal {
    fn drop(&mut self) {
        self.restore();
    }
}

fn tabs_for(resource: DetailResource) -> &'static [Tab] {
    match resource {
        DetailResource::Issue(_) => &[Tab::Overview, Tab::Conversation],
        DetailResource::Pull(_) => &[
            Tab::Overview,
            Tab::Conversation,
            Tab::Files,
            Tab::Diff,
            Tab::Checks,
        ],
        DetailResource::Run(_) => &[Tab::Overview, Tab::Jobs, Tab::Log],
    }
}

fn state_color(state: &str, palette: &Palette) -> Color {
    match state.to_ascii_lowercase().as_str() {
        "open" | "success" | "completed" => palette.green,
        "merged" => palette.mauve,
        "failure" | "failed" | "closed" | "cancelled" | "timed_out" => palette.red,
        "in_progress" | "queued" | "pending" | "loading" => palette.yellow,
        _ => palette.overlay1,
    }
}

fn actor(actor: Option<&crate::github::Actor>) -> &str {
    actor.map(|actor| actor.login.as_str()).unwrap_or("unknown")
}

fn styled_text(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    wrap_text(text, width)
        .into_iter()
        .map(|line| Line::styled(line, style))
        .collect()
}

fn body_lines(text: &str, width: usize, palette: &Palette) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut code = false;
    for raw in text.lines() {
        if raw.trim_start().starts_with("```") {
            code = !code;
            lines.push(Line::styled(
                raw.to_string(),
                Style::default().fg(palette.yellow),
            ));
            continue;
        }
        let style = if code {
            Style::default().fg(palette.yellow).bg(palette.surface0)
        } else if raw.starts_with('#') {
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else if raw.starts_with('>') {
            Style::default()
                .fg(palette.subtext0)
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(palette.text)
        };
        let wrapped = wrap_text(raw, width);
        if wrapped.is_empty() {
            lines.push(Line::raw(String::new()));
        } else {
            lines.extend(wrapped.into_iter().map(|line| Line::styled(line, style)));
        }
    }
    if text.is_empty() {
        lines.push(Line::styled(
            "(no description)",
            Style::default().fg(palette.overlay1),
        ));
    }
    lines
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut output = Vec::new();
    for raw in text.lines() {
        if raw.is_empty() {
            output.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in raw.split_whitespace() {
            let extra = usize::from(!line.is_empty()) + word.chars().count();
            if !line.is_empty() && line.chars().count() + extra > width {
                output.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            output.push(line);
        }
    }
    output
}

fn metadata_line(parts: &[String], palette: &Palette) -> Line<'static> {
    Line::styled(parts.join("  ·  "), Style::default().fg(palette.subtext0))
}

fn issue_overview(issue: &IssueDetail, width: usize, palette: &Palette) -> Vec<Line<'static>> {
    let labels = issue
        .labels
        .iter()
        .map(|label| label.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![metadata_line(
        &[
            actor(issue.author.as_ref()).to_string(),
            if labels.is_empty() {
                "no labels".into()
            } else {
                labels
            },
            issue.updated_at.clone(),
        ],
        palette,
    )];
    lines.push(Line::raw(String::new()));
    lines.extend(body_lines(&issue.body, width, palette));
    lines
}

fn comments_lines(comments: &[Comment], width: usize, palette: &Palette) -> Vec<Line<'static>> {
    if comments.is_empty() {
        return vec![Line::styled(
            "No comments",
            Style::default().fg(palette.overlay1),
        )];
    }
    let mut lines = Vec::new();
    for (index, comment) in comments.iter().enumerate() {
        if index > 0 {
            lines.push(Line::styled(
                "─".repeat(width),
                Style::default().fg(palette.surface1),
            ));
        }
        lines.push(metadata_line(
            &[
                actor(comment.author.as_ref()).to_string(),
                comment.created_at.clone(),
            ],
            palette,
        ));
        lines.push(Line::raw(String::new()));
        lines.extend(body_lines(&comment.body, width, palette));
        lines.push(Line::raw(String::new()));
    }
    lines
}

fn pull_overview(pull: &PullRequestDetail, width: usize, palette: &Palette) -> Vec<Line<'static>> {
    let mut lines = vec![metadata_line(
        &[
            actor(pull.author.as_ref()).to_string(),
            format!("{} → {}", pull.head_ref_name, pull.base_ref_name),
            format!("+{} -{}", pull.additions, pull.deletions),
            format!("{} files", pull.changed_files),
        ],
        palette,
    )];
    lines.push(metadata_line(
        &[
            format!("review {}", display_or(&pull.review_decision, "pending")),
            format!("merge {}", display_or(&pull.mergeable, "unknown")),
            display_or(&pull.merge_state_status, "unknown").to_string(),
        ],
        palette,
    ));
    lines.push(Line::raw(String::new()));
    lines.extend(body_lines(&pull.body, width, palette));
    lines
}

fn pull_conversation(
    pull: &PullRequestDetail,
    width: usize,
    palette: &Palette,
) -> Vec<Line<'static>> {
    let mut lines = comments_lines(&pull.comments, width, palette);
    for review in &pull.reviews {
        if !lines.is_empty() {
            lines.push(Line::styled(
                "─".repeat(width),
                Style::default().fg(palette.surface1),
            ));
        }
        lines.push(review_header(review, palette));
        if !review.body.is_empty() {
            lines.push(Line::raw(String::new()));
            lines.extend(body_lines(&review.body, width, palette));
        }
        lines.push(Line::raw(String::new()));
    }
    lines
}

fn review_header(review: &Review, palette: &Palette) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            actor(review.author.as_ref()).to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(palette.overlay1)),
        Span::styled(
            review.state.clone(),
            Style::default().fg(state_color(&review.state, palette)),
        ),
        Span::styled(
            format!("  ·  {}", review.submitted_at),
            Style::default().fg(palette.subtext0),
        ),
    ])
}

fn pull_files(pull: &PullRequestDetail, palette: &Palette) -> Vec<Line<'static>> {
    if pull.files.is_empty() {
        return vec![Line::styled(
            "No changed files",
            Style::default().fg(palette.overlay1),
        )];
    }
    pull.files
        .iter()
        .map(|file| {
            Line::from(vec![
                Span::styled(
                    format!("+{} ", file.additions),
                    Style::default().fg(palette.green),
                ),
                Span::styled(
                    format!("-{} ", file.deletions),
                    Style::default().fg(palette.red),
                ),
                Span::styled(file.path.clone(), Style::default().fg(palette.text)),
            ])
        })
        .collect()
}

fn patch_lines(patch: &str, width: usize, palette: &Palette) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for raw in patch.lines() {
        let style =
            if raw.starts_with("diff --git") || raw.starts_with("+++") || raw.starts_with("---") {
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD)
            } else if raw.starts_with("@@") {
                Style::default().fg(palette.blue)
            } else if raw.starts_with('+') {
                Style::default().fg(palette.green).bg(palette.surface0)
            } else if raw.starts_with('-') {
                Style::default().fg(palette.red).bg(palette.surface0)
            } else {
                Style::default().fg(palette.subtext0)
            };
        for line in wrap_text(raw, width) {
            lines.push(Line::styled(line, style));
        }
    }
    lines
}

fn check_lines(checks: &Value, palette: &Palette) -> Vec<Line<'static>> {
    let Some(checks) = checks.as_array() else {
        return vec![Line::styled(
            "No checks",
            Style::default().fg(palette.overlay1),
        )];
    };
    if checks.is_empty() {
        return vec![Line::styled(
            "No checks",
            Style::default().fg(palette.overlay1),
        )];
    }
    checks
        .iter()
        .map(|check| {
            let state = check
                .get("conclusion")
                .or_else(|| check.get("state"))
                .or_else(|| check.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("pending");
            let name = check
                .get("name")
                .or_else(|| check.get("context"))
                .and_then(Value::as_str)
                .unwrap_or("check");
            let glyph = match state.to_ascii_lowercase().as_str() {
                "success" | "completed" => "✓",
                "failure" | "failed" | "error" | "timed_out" => "×",
                _ => "…",
            };
            Line::from(vec![
                Span::styled(
                    format!("{glyph} "),
                    Style::default().fg(state_color(state, palette)),
                ),
                Span::styled(name.to_string(), Style::default().fg(palette.text)),
                Span::styled(format!("  {state}"), Style::default().fg(palette.subtext0)),
            ])
        })
        .collect()
}

fn run_overview(run: &WorkflowRunDetail, width: usize, palette: &Palette) -> Vec<Line<'static>> {
    let mut lines = vec![
        metadata_line(
            &[
                run.event.clone(),
                run.head_branch.clone(),
                short_sha(&run.head_sha),
                format!("attempt {}", run.attempt),
            ],
            palette,
        ),
        Line::raw(String::new()),
    ];
    lines.extend(
        wrap_text(&run.display_title, width)
            .into_iter()
            .map(|line| {
                Line::styled(
                    line,
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                )
            }),
    );
    lines.push(Line::raw(String::new()));
    lines.push(metadata_line(
        &[run.created_at.clone(), run.updated_at.clone()],
        palette,
    ));
    lines
}

fn run_jobs(run: &WorkflowRunDetail, palette: &Palette) -> Vec<Line<'static>> {
    if run.jobs.is_empty() {
        return vec![Line::styled(
            "No jobs",
            Style::default().fg(palette.overlay1),
        )];
    }
    let mut lines = Vec::new();
    for job in &run.jobs {
        let state = if job.status == "completed" {
            &job.conclusion
        } else {
            &job.status
        };
        let glyph = match state.as_str() {
            "success" => "✓",
            "failure" | "timed_out" => "×",
            "cancelled" => "■",
            _ => "…",
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{glyph} "),
                Style::default().fg(state_color(state, palette)),
            ),
            Span::styled(
                job.name.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        for step in &job.steps {
            let state = if step.status == "completed" {
                &step.conclusion
            } else {
                &step.status
            };
            let glyph = if state == "success" {
                "✓"
            } else if state == "failure" {
                "×"
            } else {
                "·"
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("   {glyph} "),
                    Style::default().fg(state_color(state, palette)),
                ),
                Span::styled(step.name.clone(), Style::default().fg(palette.subtext0)),
            ]));
        }
        lines.push(Line::raw(String::new()));
    }
    lines
}

fn log_lines(log: &str, width: usize, palette: &Palette) -> Vec<Line<'static>> {
    log.lines()
        .flat_map(|raw| {
            let lower = raw.to_ascii_lowercase();
            let style = if lower.contains("error") || lower.contains("failed") {
                Style::default().fg(palette.red)
            } else if lower.contains("warning") {
                Style::default().fg(palette.yellow)
            } else {
                Style::default().fg(palette.subtext0)
            };
            wrap_text(raw, width)
                .into_iter()
                .map(move |line| Line::styled(line, style))
        })
        .collect()
}

fn display_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() {
        fallback
    } else {
        value
    }
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeAdapter;

    impl GitHubDetailAdapter for FakeAdapter {
        fn issue_detail(&self, _repo: &str, _number: u64) -> Result<IssueDetail, String> {
            Err("unused".into())
        }

        fn pull_detail(&self, _repo: &str, _number: u64) -> Result<PullRequestDetail, String> {
            Err("unused".into())
        }

        fn run_detail(&self, _repo: &str, _run_id: u64) -> Result<WorkflowRunDetail, String> {
            Err("unused".into())
        }

        fn pull_patch(&self, _repo: &str, _number: u64) -> Result<String, String> {
            Err("unused".into())
        }

        fn run_log(&self, _repo: &str, _run_id: u64, _failed_only: bool) -> Result<String, String> {
            Err("unused".into())
        }

        fn mutate(&self, _repo: &str, _mutation: &GitHubMutation) -> Result<String, String> {
            Ok(String::new())
        }
    }

    fn app(resource: DetailResource) -> DetailApp {
        DetailApp::with_adapter(
            "owner/repo".into(),
            resource,
            InitialView::Overview,
            Arc::new(Config::for_test()),
            Arc::new(FakeAdapter),
        )
    }

    struct RecordingAdapter {
        mutations: Arc<std::sync::Mutex<Vec<GitHubMutation>>>,
    }

    impl GitHubDetailAdapter for RecordingAdapter {
        fn issue_detail(&self, _repo: &str, _number: u64) -> Result<IssueDetail, String> {
            Err("unused".into())
        }

        fn pull_detail(&self, _repo: &str, _number: u64) -> Result<PullRequestDetail, String> {
            Err("unused".into())
        }

        fn run_detail(&self, _repo: &str, _run_id: u64) -> Result<WorkflowRunDetail, String> {
            Err("unused".into())
        }

        fn pull_patch(&self, _repo: &str, _number: u64) -> Result<String, String> {
            Err("unused".into())
        }

        fn run_log(&self, _repo: &str, _run_id: u64, _failed_only: bool) -> Result<String, String> {
            Err("unused".into())
        }

        fn mutate(&self, _repo: &str, mutation: &GitHubMutation) -> Result<String, String> {
            self.mutations.lock().unwrap().push(mutation.clone());
            Ok(String::new())
        }
    }

    #[test]
    fn wraps_text_without_losing_words() {
        assert_eq!(wrap_text("one two three", 7), vec!["one two", "three"]);
    }

    #[test]
    fn resource_tabs_are_context_specific() {
        assert_eq!(
            tabs_for(DetailResource::Issue(1)),
            &[Tab::Overview, Tab::Conversation]
        );
        assert_eq!(
            tabs_for(DetailResource::Pull(2)),
            &[
                Tab::Overview,
                Tab::Conversation,
                Tab::Files,
                Tab::Diff,
                Tab::Checks,
            ]
        );
        assert_eq!(
            tabs_for(DetailResource::Run(3)),
            &[Tab::Overview, Tab::Jobs, Tab::Log]
        );
    }

    #[test]
    fn compose_submission_becomes_a_typed_comment_mutation() {
        let mutations = Arc::new(std::sync::Mutex::new(Vec::new()));
        let adapter = Arc::new(RecordingAdapter {
            mutations: Arc::clone(&mutations),
        });
        let mut app = DetailApp::with_adapter(
            "owner/repo".into(),
            DetailResource::Issue(7),
            InitialView::Overview,
            Arc::new(Config::for_test()),
            adapter,
        );
        app.mode = Mode::Compose {
            kind: ComposeKind::Comment,
            text: "hello from Corral".chars().collect(),
        };
        app.submit_compose();
        for _ in 0..20 {
            if !mutations.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            mutations.lock().unwrap().as_slice(),
            &[GitHubMutation::IssueComment {
                number: 7,
                body: "hello from Corral".into(),
            }]
        );
    }

    #[test]
    fn context_actions_are_typed_and_destructive_actions_confirm() {
        let mut issue_app = app(DetailResource::Issue(7));
        issue_app.detail = Some(Detail::Issue(
            serde_json::from_str(r#"{"number":7,"title":"bug","state":"OPEN"}"#).unwrap(),
        ));
        issue_app.context_action();
        assert!(matches!(
            issue_app.mode,
            Mode::Confirm {
                mutation: GitHubMutation::IssueState {
                    number: 7,
                    open: false
                },
                ..
            }
        ));

        let mut pull_app = app(DetailResource::Pull(8));
        pull_app.detail = Some(Detail::Pull(
            serde_json::from_str(
                r#"{"number":8,"title":"feature","state":"OPEN","headRefOid":"abcdef123456","statusCheckRollup":[]}"#,
            )
            .unwrap(),
        ));
        pull_app.merge_pull();
        assert!(matches!(
            pull_app.mode,
            Mode::Confirm {
                mutation: GitHubMutation::PullMergeSquash { number: 8, .. },
                ..
            }
        ));
    }
}
