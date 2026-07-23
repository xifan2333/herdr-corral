//! Interactive state machine and terminal loop for GitHub detail.

use super::images::{open_image_externally, ImagePlacement};
use super::pages::{
    check_lines, issue_page, log_lines, patch_lines, pull_files, pull_page, run_jobs, run_overview,
};
use super::util::{short_sha, state_color, styled_text};
use crate::config::{self, Config};
use crate::github::{
    GhCli, GitHubDetailAdapter, GitHubMutation, IssueDetail, MergeMethod, PullRequestDetail,
    WorkflowRunDetail,
};
use crate::ui::Palette;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
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
pub(super) enum Tab {
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

pub(super) enum Detail {
    Issue(IssueDetail),
    Pull(PullRequestDetail),
    Run(WorkflowRunDetail),
}

pub(super) enum Payload {
    Detail(Box<Detail>),
    Patch(String),
    Log { text: String, failed_only: bool },
    Mutation(String),
    ImageOpen(String),
}

#[derive(Clone, Copy)]
pub(super) enum Request {
    Detail,
    Patch,
    Log,
    Mutation,
    ImageOpen,
}

#[derive(Clone, Copy)]
pub(super) enum ComposeKind {
    Comment,
    RequestChanges,
}

pub(super) enum Mode {
    Browse,
    Compose {
        kind: ComposeKind,
        text: Vec<char>,
    },
    MergeMethod {
        number: u64,
        head_sha: String,
        selected: usize,
    },
    Confirm {
        message: String,
        mutation: GitHubMutation,
    },
}

pub(super) struct Completion {
    generation: u64,
    request: Request,
    result: Result<Payload, String>,
}

pub(super) struct DetailApp {
    repo: String,
    resource: DetailResource,
    adapter: Arc<dyn GitHubDetailAdapter>,
    config: Arc<Config>,
    pub(super) detail: Option<Detail>,
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
    pub(super) mode: Mode,
    notice: Option<(String, bool, Instant)>,
    error: Option<String>,
    content_revision: u64,
    rendered_key: Option<(Tab, u16, u64)>,
    rendered_lines: Vec<Line<'static>>,
    images: Vec<ImagePlacement>,
    body_area: Rect,
    image_loading: bool,
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

    pub(super) fn with_adapter(
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
            images: Vec::new(),
            body_area: Rect::default(),
            image_loading: false,
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
            (DetailResource::Issue(_), _) => Tab::Conversation,
            (DetailResource::Pull(_), InitialView::Files) => Tab::Files,
            (DetailResource::Pull(_), InitialView::Diff) => Tab::Diff,
            (DetailResource::Pull(_), InitialView::Checks) => Tab::Checks,
            (DetailResource::Pull(_), _) => Tab::Conversation,
            (DetailResource::Run(_), InitialView::Jobs) => Tab::Jobs,
            (DetailResource::Run(_), InitialView::Log | InitialView::FailedLog) => Tab::Log,
            (DetailResource::Run(_), _) => Tab::Overview,
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

    pub(super) fn context_action(&mut self) {
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

    pub(super) fn merge_pull(&mut self) {
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
        // Default to squash, which matches the previous behavior and is usually
        // the least noisy history-preserving option for feature branches.
        self.mode = Mode::MergeMethod {
            number,
            head_sha: pull.head_ref_oid.clone(),
            selected: MergeMethod::Squash.index(),
        };
    }

    pub(super) fn confirm_selected_merge(&mut self) {
        let Mode::MergeMethod {
            number,
            head_sha,
            selected,
        } = &self.mode
        else {
            return;
        };
        let Some(method) = MergeMethod::from_index(*selected) else {
            return;
        };
        let number = *number;
        let head_sha = head_sha.clone();
        self.confirm(
            format!(
                "{} merge #{number} at {}?",
                method.title(),
                short_sha(&head_sha)
            ),
            GitHubMutation::PullMerge {
                number,
                head_sha,
                method,
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

    pub(super) fn submit_compose(&mut self) {
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
            Request::ImageOpen => self.image_loading = false,
        }
        // Image open only updates the footer notice; avoid rebuilding the page.
        if !matches!(completion.request, Request::ImageOpen) {
            self.content_revision = self.content_revision.wrapping_add(1);
        }
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
            Ok(Payload::ImageOpen(message)) => self.set_notice(message, false),
            Err(error) => match completion.request {
                Request::Mutation | Request::ImageOpen => self.set_notice(error, true),
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

    fn open_image_at_line(&mut self, line: usize) {
        let Some(placement) = self.images.iter().find(|image| image.line == line).cloned() else {
            if !self.images.is_empty() {
                self.set_notice("no image on this line — click a [image] row or press o", true);
            }
            return;
        };
        self.open_image(&placement.url, &placement.alt);
    }

    fn open_first_visible_image(&mut self) {
        let height = usize::from(self.body_height.max(1));
        let scroll = self.scroll;
        let Some(placement) = self
            .images
            .iter()
            .find(|image| image.line >= scroll && image.line - scroll < height)
            .cloned()
        else {
            self.set_notice("no image on this page", true);
            return;
        };
        self.open_image(&placement.url, &placement.alt);
    }

    fn open_image(&mut self, url: &str, alt: &str) {
        if self.image_loading {
            self.set_notice("image already opening…", true);
            return;
        }
        self.image_loading = true;
        self.set_notice("downloading image…", false);
        let generation = self.generation;
        let url = url.to_string();
        let alt = alt.to_string();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = open_image_externally(&url, &alt).map(Payload::ImageOpen);
            let _ = sender.send(Completion {
                generation,
                request: Request::ImageOpen,
                result,
            });
        });
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                let count = self.rendered_lines.len();
                self.scroll_by(3, count);
            }
            MouseEventKind::ScrollUp => {
                let count = self.rendered_lines.len();
                self.scroll_by(-3, count);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if !matches!(self.mode, Mode::Browse) {
                    return;
                }
                let body = self.body_area;
                if mouse.column < body.x
                    || mouse.row < body.y
                    || mouse.column >= body.x.saturating_add(body.width)
                    || mouse.row >= body.y.saturating_add(body.height)
                {
                    return;
                }
                let line = self.scroll + usize::from(mouse.row.saturating_sub(body.y));
                self.open_image_at_line(line);
            }
            _ => {}
        }
    }

    fn scroll_by(&mut self, delta: isize, line_count: usize) {
        let max = line_count.saturating_sub(usize::from(self.body_height.max(1)));
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
    }

    pub(super) fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers, line_count: usize) -> bool {
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
        if matches!(self.mode, Mode::MergeMethod { .. }) {
            match action.as_deref() {
                Some(config::internal::UP) => {
                    if let Mode::MergeMethod { selected, .. } = &mut self.mode {
                        *selected = selected.saturating_sub(1);
                    }
                }
                Some(config::internal::DOWN) => {
                    if let Mode::MergeMethod { selected, .. } = &mut self.mode {
                        *selected = (*selected + 1).min(MergeMethod::ALL.len() - 1);
                    }
                }
                Some(config::internal::TOP) => {
                    if let Mode::MergeMethod { selected, .. } = &mut self.mode {
                        *selected = 0;
                    }
                }
                Some(config::internal::BOTTOM) => {
                    if let Mode::MergeMethod { selected, .. } = &mut self.mode {
                        *selected = MergeMethod::ALL.len() - 1;
                    }
                }
                Some(
                    config::internal::TOGGLE
                    | config::internal::OPEN
                    | config::internal::GITHUB_VIEW
                    | config::internal::GITHUB_MERGE
                    | config::internal::GITHUB_CONFIRM,
                ) => self.confirm_selected_merge(),
                Some(config::internal::GITHUB_CANCEL) => self.mode = Mode::Browse,
                _ => {
                    if let KeyCode::Char(ch @ '1'..='3') = code {
                        if let Mode::MergeMethod { selected, .. } = &mut self.mode {
                            *selected = (ch as u8 - b'1') as usize;
                        }
                        self.confirm_selected_merge();
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
            config::internal::OPEN => self.open_first_visible_image(),
            _ => {}
        }
        false
    }

    fn title_parts(&self) -> (String, String) {
        match &self.detail {
            Some(Detail::Issue(issue)) => (format!("#{}", issue.number), issue.title.clone()),
            Some(Detail::Pull(pull)) => (format!("#{}", pull.number), pull.title.clone()),
            Some(Detail::Run(run)) => (format!("#{}", run.database_id), run.workflow_name.clone()),
            None => match self.resource {
                DetailResource::Issue(number) => (format!("#{number}"), "Issue".into()),
                DetailResource::Pull(number) => (format!("#{number}"), "Pull Request".into()),
                DetailResource::Run(id) => (format!("#{id}"), "Actions Run".into()),
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

    fn build_lines(
        &self,
        width: u16,
        palette: &Palette,
    ) -> (Vec<Line<'static>>, Vec<ImagePlacement>) {
        let width = usize::from(width.max(8));
        if let Some(error) = &self.error {
            return (
                styled_text(error, width, Style::default().fg(palette.red)),
                Vec::new(),
            );
        }
        let Some(detail) = &self.detail else {
            return (
                vec![Line::styled(
                    "Loading…",
                    Style::default().fg(palette.accent),
                )],
                Vec::new(),
            );
        };
        let plain = |lines: Vec<Line<'static>>| (lines, Vec::new());
        match (detail, self.active_tab()) {
            (Detail::Issue(issue), _) => issue_page(issue, width, palette),
            (Detail::Pull(pull), Tab::Conversation) => pull_page(pull, width, palette),
            (Detail::Pull(pull), Tab::Files) => plain(pull_files(pull, palette)),
            (Detail::Pull(_), Tab::Diff) => plain({
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
            }),
            (Detail::Pull(pull), Tab::Checks) => {
                plain(check_lines(&pull.status_check_rollup, palette))
            }
            (Detail::Run(run), Tab::Overview) => plain(run_overview(run, width, palette)),
            (Detail::Run(run), Tab::Jobs) => plain(run_jobs(run, palette)),
            (Detail::Run(_), Tab::Log) => plain({
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
            }),
            _ => plain(vec![Line::styled(
                "Not available",
                Style::default().fg(palette.overlay1),
            )]),
        }
    }

    fn draw(&mut self, frame: &mut Frame, palette: &Palette) -> usize {
        let area = frame.area();
        if area.height == 0 {
            return 0;
        }
        // Reset the whole area's background every frame. Fenced code-block cells
        // still set bg = surface0; blank/short rows never overwrite those cells,
        // so without a full-area reset scrolling can smear leftover blocks.
        // Color::Reset (not a themed fill) keeps the body transparent as before.
        frame.render_widget(
            Block::default().style(Style::default().bg(Color::Reset)),
            area,
        );
        // Badge-style header: number pill + title, state pill on the right.
        let (number, title) = self.title_parts();
        let number_badge = format!(" {number} ");
        let state = self.state().to_ascii_uppercase();
        let state_badge = format!(" {state} ");
        let number_width = (number_badge.chars().count() as u16).min(area.width);
        let state_width =
            (state_badge.chars().count() as u16).min(area.width.saturating_sub(number_width));
        let title_width = area
            .width
            .saturating_sub(number_width.saturating_add(state_width).saturating_add(1));
        frame.render_widget(
            Paragraph::new(number_badge).style(
                Style::default()
                    .fg(palette.panel_bg)
                    .bg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect {
                width: number_width,
                height: 1,
                ..area
            },
        );
        if title_width > 0 {
            frame.render_widget(
                Paragraph::new(format!(" {title}")).style(
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Rect {
                    x: area.x + number_width,
                    width: title_width,
                    height: 1,
                    ..area
                },
            );
        }
        if state_width > 0 {
            frame.render_widget(
                Paragraph::new(state_badge).style(
                    Style::default()
                        .fg(palette.panel_bg)
                        .bg(state_color(self.state(), palette))
                        .add_modifier(Modifier::BOLD),
                ),
                Rect {
                    x: area.x + area.width.saturating_sub(state_width),
                    width: state_width,
                    height: 1,
                    ..area
                },
            );
        }

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
        self.body_area = body;
        let render_key = (self.active_tab(), body.width, self.content_revision);
        if self.rendered_key != Some(render_key) {
            let (lines, images) = self.build_lines(body.width, palette);
            self.rendered_lines = lines;
            self.images = images;
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
            } else if self.image_loading {
                "downloading image…".to_string()
            } else if let Some((message, _, _)) = &self.notice {
                message.clone()
            } else {
                match self.resource {
                    DetailResource::Issue(_) => {
                        "c reply  o image  x close/reopen  h/l tabs  r refresh  q back".into()
                    }
                    DetailResource::Pull(_) => {
                        "c reply  o image  a approve  x changes  m merge  D close  d diff".into()
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
            Mode::MergeMethod {
                number,
                head_sha,
                selected,
            } => {
                let width = area.width.saturating_sub(2).clamp(1, 56);
                let height = 8.min(area.height.max(1));
                let rect = Rect {
                    x: area.x + area.width.saturating_sub(width) / 2,
                    y: area.y + area.height.saturating_sub(height) / 2,
                    width,
                    height,
                };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Merge #{number} @ {} ", short_sha(head_sha)))
                    .style(Style::default().fg(palette.accent).bg(palette.panel_bg));
                let inner = block.inner(rect);
                frame.render_widget(Clear, rect);
                frame.render_widget(block, rect);
                let mut lines = Vec::new();
                for (index, method) in MergeMethod::ALL.into_iter().enumerate() {
                    let marker = if index == *selected { "› " } else { "  " };
                    let style = if index == *selected {
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD)
                            .bg(palette.surface0)
                    } else {
                        Style::default().fg(palette.text)
                    };
                    lines.push(Line::styled(
                        format!("{marker}{}  {}", index + 1, method.label()),
                        style,
                    ));
                }
                lines.push(Line::raw(String::new()));
                lines.push(Line::styled(
                    "j/k select  Enter confirm  Esc cancel",
                    Style::default().fg(palette.overlay1),
                ));
                frame.render_widget(
                    Paragraph::new(lines).style(Style::default().bg(palette.panel_bg)),
                    inner,
                );
            }
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
                Event::Mouse(mouse) => app.handle_mouse(mouse),
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

pub(super) fn tabs_for(resource: DetailResource) -> &'static [Tab] {
    match resource {
        // Issue/PR overview and comments now live on one scrollable page.
        DetailResource::Issue(_) => &[Tab::Conversation],
        DetailResource::Pull(_) => &[Tab::Conversation, Tab::Files, Tab::Diff, Tab::Checks],
        DetailResource::Run(_) => &[Tab::Overview, Tab::Jobs, Tab::Log],
    }
}

