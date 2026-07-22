//! Full-width interactive GitHub detail client used by `corral-github`.
//!
//! The 32-column sidebar remains a navigator. This app runs in the shared
//! owner-scoped nvim terminal and owns resource detail presentation.

use crate::config::{self, Config};
use crate::github::{
    GhCli, GitHubDetailAdapter, GitHubMutation, IssueDetail, MergeMethod, PullRequestDetail,
    Review, WorkflowRunDetail,
};
use crate::ui::Palette;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use pulldown_cmark::{Event as MdEvent, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Stdout};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const NOTICE_SUCCESS_TTL: Duration = Duration::from_secs(2);
const NOTICE_ERROR_TTL: Duration = Duration::from_secs(4);
const MAX_MESSAGE_CHARS: usize = 65_536;
const DEFAULT_IMAGE_ROWS: u16 = 12;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// One markdown image discovered while rendering, keyed by its final line index.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ImagePlacement {
    line: usize,
    url: String,
    rows: u16,
}

/// Runtime image toggle. Off unless the host opts in via config/env, because
/// inline graphics only work on a kitty-protocol-capable terminal.
fn images_enabled() -> bool {
    matches!(
        std::env::var("CORRAL_GITHUB_IMAGES").ok().as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

fn image_rows() -> u16 {
    std::env::var("CORRAL_GITHUB_IMAGE_ROWS")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|rows| *rows > 0)
        .unwrap_or(DEFAULT_IMAGE_ROWS)
}

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
    images: Vec<ImagePlacement>,
    images_enabled: bool,
    picker: Option<Picker>,
    protocols: HashMap<String, StatefulProtocol>,
    image_errors: HashSet<String>,
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
            images: Vec::new(),
            images_enabled: images_enabled(),
            picker: None,
            protocols: HashMap::new(),
            image_errors: HashSet::new(),
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
        // Default to squash, which matches the previous behavior and is usually
        // the least noisy history-preserving option for feature branches.
        self.mode = Mode::MergeMethod {
            number,
            head_sha: pull.head_ref_oid.clone(),
            selected: MergeMethod::Squash.index(),
        };
    }

    fn confirm_selected_merge(&mut self) {
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
        self.load_pending_images();
    }

    fn ensure_picker(&mut self) {
        if self.picker.is_none() {
            let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
            // The host explicitly opted into kitty graphics.
            picker.set_protocol_type(ProtocolType::Kitty);
            self.picker = Some(picker);
        }
    }

    /// Best-effort, bounded image fetch. Runs off the draw path so a slow image
    /// never blocks rendering; at most two new images are decoded per tick.
    fn load_pending_images(&mut self) {
        if !self.images_enabled || self.images.is_empty() {
            return;
        }
        self.ensure_picker();
        let picker = self.picker.take();
        if let Some(picker) = &picker {
            let mut loaded = 0;
            for placement in self.images.clone() {
                if loaded >= 2 {
                    break;
                }
                if self.protocols.contains_key(&placement.url)
                    || self.image_errors.contains(&placement.url)
                {
                    continue;
                }
                match fetch_image(&placement.url) {
                    Ok(image) => {
                        self.protocols
                            .insert(placement.url.clone(), picker.new_resize_protocol(image));
                    }
                    Err(_) => {
                        self.image_errors.insert(placement.url.clone());
                    }
                }
                loaded += 1;
            }
        }
        self.picker = picker;
    }

    fn render_images(&mut self, frame: &mut Frame, body: Rect) {
        if !self.images_enabled || self.protocols.is_empty() {
            return;
        }
        let scroll = self.scroll;
        let height = usize::from(body.height);
        // Collect visible placements first to avoid borrow conflicts.
        let visible: Vec<(u16, u16, String)> = self
            .images
            .iter()
            .filter_map(|placement| {
                if placement.line < scroll || placement.line - scroll >= height {
                    return None;
                }
                let offset = u16::try_from(placement.line - scroll).ok()?;
                let avail = body.height.saturating_sub(offset);
                let rows = placement.rows.min(avail);
                (rows >= 2).then(|| (offset, rows, placement.url.clone()))
            })
            .collect();
        for (offset, rows, url) in visible {
            if let Some(protocol) = self.protocols.get_mut(&url) {
                let rect = Rect {
                    x: body.x.saturating_add(3),
                    y: body.y.saturating_add(offset),
                    width: body.width.saturating_sub(3),
                    height: rows,
                };
                frame.render_stateful_widget(
                    StatefulImage::<StatefulProtocol>::new().resize(Resize::Fit(None)),
                    rect,
                    protocol,
                );
            }
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
            (Detail::Issue(issue), _) => issue_page(issue, width, palette, self.images_enabled),
            (Detail::Pull(pull), Tab::Conversation) => {
                pull_page(pull, width, palette, self.images_enabled)
            }
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
        self.render_images(frame, body);

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
        // Issue/PR overview and comments now live on one scrollable page.
        DetailResource::Issue(_) => &[Tab::Conversation],
        DetailResource::Pull(_) => &[Tab::Conversation, Tab::Files, Tab::Diff, Tab::Checks],
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

/// Fetch and decode a remote image. Best-effort; bounded size and default
/// ureq timeouts keep a bad URL from stalling the client.
fn fetch_image(url: &str) -> Result<image::DynamicImage, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("unsupported image url".into());
    }
    let mut response = ureq::get(url).call().map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_IMAGE_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    image::load_from_memory(&bytes).map_err(|error| error.to_string())
}

/// A rendered page: styled lines plus any image placements (by line index).
struct Page {
    lines: Vec<Line<'static>>,
    images: Vec<ImagePlacement>,
}

impl Page {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            images: Vec::new(),
        }
    }

    fn blank(&mut self) {
        self.lines.push(Line::raw(String::new()));
    }

    fn push(&mut self, line: Line<'static>) {
        self.lines.push(line);
    }

    fn rule(&mut self, width: usize, palette: &Palette) {
        self.lines.push(Line::styled(
            "─".repeat(width),
            Style::default().fg(palette.surface1),
        ));
    }

    fn markdown(&mut self, text: &str, width: usize, palette: &Palette, images_enabled: bool) {
        let (lines, images) = render_markdown(text, width, palette, images_enabled);
        let base = self.lines.len();
        let rows = image_rows();
        for (offset, url) in images {
            self.images.push(ImagePlacement {
                line: base + offset,
                url,
                rows,
            });
        }
        self.lines.extend(lines);
    }

    fn into_parts(self) -> (Vec<Line<'static>>, Vec<ImagePlacement>) {
        (self.lines, self.images)
    }
}

/// Markdown → styled ratatui lines using pulldown-cmark for parsing. Returns
/// the lines and `(line_index, url)` for any images (as reserved blank rows).
fn render_markdown(
    text: &str,
    width: usize,
    palette: &Palette,
    images_enabled: bool,
) -> (Vec<Line<'static>>, Vec<(usize, String)>) {
    if text.trim().is_empty() {
        return (
            vec![Line::styled(
                "(no description)",
                Style::default().fg(palette.overlay1),
            )],
            Vec::new(),
        );
    }
    let rows = usize::from(image_rows());
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut images: Vec<(usize, String)> = Vec::new();
    let mut segs: Vec<(String, Style)> = Vec::new();
    let (mut bold, mut italic, mut strike, mut link) = (false, false, false, false);
    let mut heading: Option<HeadingLevel> = None;
    let mut quote_depth: usize = 0;
    let mut in_code = false;
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut item_pending_marker: Option<String> = None;
    let mut image_url: Option<String> = None;

    let seg_style = |bold: bool,
                     italic: bool,
                     strike: bool,
                     link: bool,
                     heading: Option<HeadingLevel>,
                     quote: usize| {
        if let Some(_level) = heading {
            return Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD);
        }
        let mut style = if quote > 0 {
            Style::default()
                .fg(palette.subtext0)
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(palette.text)
        };
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if strike {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if link {
            style = style.fg(palette.blue).add_modifier(Modifier::UNDERLINED);
        }
        style
    };

    let quote_prefix = |depth: usize| "▏ ".repeat(depth);

    let parser = Parser::new_ext(
        text,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS,
    );
    for event in parser {
        match event {
            MdEvent::Start(Tag::Paragraph | Tag::Heading { .. }) => {
                if let Tag::Heading { level, .. } = event_heading(&event) {
                    heading = Some(level);
                }
                segs.clear();
            }
            MdEvent::End(TagEnd::Paragraph | TagEnd::Heading(_)) => {
                let prefix = quote_prefix(quote_depth);
                let first = item_pending_marker.take();
                flush_block(
                    &mut out,
                    &mut segs,
                    width,
                    &prefix,
                    first.as_deref(),
                    palette,
                );
                heading = None;
                out.push(Line::raw(String::new()));
            }
            MdEvent::Start(Tag::Strong) => bold = true,
            MdEvent::End(TagEnd::Strong) => bold = false,
            MdEvent::Start(Tag::Emphasis) => italic = true,
            MdEvent::End(TagEnd::Emphasis) => italic = false,
            MdEvent::Start(Tag::Strikethrough) => strike = true,
            MdEvent::End(TagEnd::Strikethrough) => strike = false,
            MdEvent::Start(Tag::Link { .. }) => link = true,
            MdEvent::End(TagEnd::Link) => link = false,
            MdEvent::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            MdEvent::End(TagEnd::BlockQuote(_)) => quote_depth = quote_depth.saturating_sub(1),
            MdEvent::Start(Tag::List(start)) => list_stack.push(start),
            MdEvent::End(TagEnd::List(_)) => {
                list_stack.pop();
            }
            MdEvent::Start(Tag::Item) => {
                let marker = match list_stack.last_mut() {
                    Some(Some(n)) => {
                        let marker = format!("{n}. ");
                        *n += 1;
                        marker
                    }
                    _ => "• ".to_string(),
                };
                item_pending_marker = Some(marker);
                segs.clear();
            }
            MdEvent::End(TagEnd::Item) => {
                let prefix = quote_prefix(quote_depth);
                let first = item_pending_marker.take();
                flush_block(
                    &mut out,
                    &mut segs,
                    width,
                    &prefix,
                    first.as_deref(),
                    palette,
                );
            }
            MdEvent::Start(Tag::CodeBlock(_)) => {
                flush_block(&mut out, &mut segs, width, "", None, palette);
                in_code = true;
            }
            MdEvent::End(TagEnd::CodeBlock) => {
                in_code = false;
                out.push(Line::raw(String::new()));
            }
            MdEvent::Start(Tag::Image { dest_url, .. }) => {
                image_url = Some(dest_url.to_string());
                segs.clear();
            }
            MdEvent::End(TagEnd::Image) => {
                let alt: String = std::mem::take(&mut segs)
                    .into_iter()
                    .map(|(text, _)| text)
                    .collect();
                if let Some(url) = image_url.take() {
                    flush_block(&mut out, &mut segs, width, "", None, palette);
                    if images_enabled {
                        images.push((out.len(), url));
                        for _ in 0..rows {
                            out.push(Line::raw(String::new()));
                        }
                    } else {
                        let label = if alt.trim().is_empty() {
                            "image"
                        } else {
                            alt.trim()
                        };
                        out.push(Line::styled(
                            format!("⬜ {label}  ({url})"),
                            Style::default().fg(palette.overlay1),
                        ));
                    }
                }
            }
            MdEvent::Text(text) => {
                if in_code {
                    for line in text.lines() {
                        out.push(Line::styled(
                            format!("  {line}"),
                            Style::default().fg(palette.yellow).bg(palette.surface0),
                        ));
                    }
                } else {
                    segs.push((
                        text.to_string(),
                        seg_style(bold, italic, strike, link, heading, quote_depth),
                    ));
                }
            }
            MdEvent::Code(code) => segs.push((
                code.to_string(),
                Style::default().fg(palette.yellow).bg(palette.surface0),
            )),
            MdEvent::SoftBreak | MdEvent::HardBreak => {
                segs.push((" ".to_string(), Style::default()))
            }
            MdEvent::Rule => {
                flush_block(&mut out, &mut segs, width, "", None, palette);
                out.push(Line::styled(
                    "─".repeat(width),
                    Style::default().fg(palette.surface1),
                ));
            }
            MdEvent::TaskListMarker(done) => segs.push((
                if done { "[x] ".into() } else { "[ ] ".into() },
                Style::default().fg(palette.accent),
            )),
            _ => {}
        }
    }
    flush_block(&mut out, &mut segs, width, "", None, palette);
    while out.last().is_some_and(|line| line.width() == 0) {
        out.pop();
    }
    (out, images)
}

fn event_heading(event: &MdEvent) -> Tag<'static> {
    if let MdEvent::Start(Tag::Heading { level, .. }) = event {
        Tag::Heading {
            level: *level,
            id: None,
            classes: Vec::new(),
            attrs: Vec::new(),
        }
    } else {
        Tag::Paragraph
    }
}

/// Wrap styled segments into lines, applying a first-line marker + continuation
/// indent so list bullets and blockquote bars align.
fn flush_block(
    out: &mut Vec<Line<'static>>,
    segs: &mut Vec<(String, Style)>,
    width: usize,
    prefix: &str,
    first_marker: Option<&str>,
    palette: &Palette,
) {
    if segs.is_empty() {
        return;
    }
    let marker = first_marker.unwrap_or("");
    let cont_indent = " ".repeat(marker.chars().count());
    let prefix_style = Style::default().fg(palette.overlay1);
    let avail = width
        .saturating_sub(prefix.chars().count() + marker.chars().count())
        .max(1);

    // Split segments into styled words.
    let mut words: Vec<(String, Style)> = Vec::new();
    for (text, style) in segs.drain(..) {
        let mut first = true;
        for word in text.split(' ') {
            if word.is_empty() {
                continue;
            }
            let _ = first;
            first = false;
            words.push((word.to_string(), style));
        }
    }
    if words.is_empty() {
        return;
    }

    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for (word, style) in words {
        let word_len = word.chars().count();
        let extra = usize::from(!current.is_empty()) + word_len;
        if !current.is_empty() && used + extra > avail {
            rows.push(std::mem::take(&mut current));
            used = 0;
        }
        if !current.is_empty() {
            current.push(Span::raw(" "));
            used += 1;
        }
        current.push(Span::styled(word, style));
        used += word_len;
    }
    if !current.is_empty() {
        rows.push(current);
    }

    for (index, mut spans) in rows.into_iter().enumerate() {
        let mut line_spans = Vec::new();
        if !prefix.is_empty() {
            line_spans.push(Span::styled(prefix.to_string(), prefix_style));
        }
        if index == 0 && !marker.is_empty() {
            line_spans.push(Span::styled(marker.to_string(), prefix_style));
        } else if !cont_indent.is_empty() {
            line_spans.push(Span::raw(cont_indent.clone()));
        }
        line_spans.append(&mut spans);
        out.push(Line::from(line_spans));
    }
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

fn comment_header(login: &str, timestamp: &str, palette: &Palette) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            login.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  {timestamp}"),
            Style::default().fg(palette.subtext0),
        ),
    ])
}

/// Issue overview + all comments on one scrollable page.
fn issue_page(
    issue: &IssueDetail,
    width: usize,
    palette: &Palette,
    images_enabled: bool,
) -> (Vec<Line<'static>>, Vec<ImagePlacement>) {
    let labels = issue
        .labels
        .iter()
        .map(|label| label.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut page = Page::new();
    page.push(metadata_line(
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
    ));
    page.blank();
    page.markdown(&issue.body, width, palette, images_enabled);
    for comment in &issue.comments {
        page.blank();
        page.rule(width, palette);
        page.push(comment_header(
            actor(comment.author.as_ref()),
            &comment.created_at,
            palette,
        ));
        page.blank();
        page.markdown(&comment.body, width, palette, images_enabled);
    }
    page.into_parts()
}

/// PR overview + conversation + reviews on one scrollable page.
fn pull_page(
    pull: &PullRequestDetail,
    width: usize,
    palette: &Palette,
    images_enabled: bool,
) -> (Vec<Line<'static>>, Vec<ImagePlacement>) {
    let mut page = Page::new();
    page.push(metadata_line(
        &[
            actor(pull.author.as_ref()).to_string(),
            format!("{} → {}", pull.head_ref_name, pull.base_ref_name),
            format!("+{} -{}", pull.additions, pull.deletions),
            format!("{} files", pull.changed_files),
        ],
        palette,
    ));
    page.push(metadata_line(
        &[
            format!("review {}", display_or(&pull.review_decision, "pending")),
            format!("merge {}", display_or(&pull.mergeable, "unknown")),
            display_or(&pull.merge_state_status, "unknown").to_string(),
        ],
        palette,
    ));
    page.blank();
    page.markdown(&pull.body, width, palette, images_enabled);
    for comment in &pull.comments {
        page.blank();
        page.rule(width, palette);
        page.push(comment_header(
            actor(comment.author.as_ref()),
            &comment.created_at,
            palette,
        ));
        page.blank();
        page.markdown(&comment.body, width, palette, images_enabled);
    }
    for review in &pull.reviews {
        page.blank();
        page.rule(width, palette);
        page.push(review_header(review, palette));
        if !review.body.is_empty() {
            page.blank();
            page.markdown(&review.body, width, palette, images_enabled);
        }
    }
    page.into_parts()
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
        // Issue/PR overview and comments now share one Conversation page.
        assert_eq!(tabs_for(DetailResource::Issue(1)), &[Tab::Conversation]);
        assert_eq!(
            tabs_for(DetailResource::Pull(2)),
            &[Tab::Conversation, Tab::Files, Tab::Diff, Tab::Checks]
        );
        assert_eq!(
            tabs_for(DetailResource::Run(3)),
            &[Tab::Overview, Tab::Jobs, Tab::Log]
        );
    }

    #[test]
    fn markdown_renders_headings_lists_and_extracts_images() {
        let palette = Palette::resolve();
        let (lines, images) = render_markdown(
            "# Title\n\nsome **bold** text\n\n- one\n- two\n\n![alt](https://example.test/a.png)",
            60,
            &palette,
            true,
        );
        assert!(lines
            .iter()
            .any(|line| line.spans.iter().any(|span| span.content.contains("Title"))));
        assert!(lines
            .iter()
            .any(|line| line.spans.iter().any(|span| span.content.contains('•'))));
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].1, "https://example.test/a.png");
    }

    #[test]
    fn images_off_keeps_a_textual_placeholder_and_no_placements() {
        let palette = Palette::resolve();
        let (lines, images) = render_markdown(
            "![diagram](https://example.test/x.png)",
            60,
            &palette,
            false,
        );
        assert!(images.is_empty());
        assert!(lines.iter().any(|line| line
            .spans
            .iter()
            .any(|span| span.content.contains("diagram"))));
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
            Mode::MergeMethod {
                number: 8,
                selected: 1,
                ..
            }
        ));
        pull_app.confirm_selected_merge();
        assert!(matches!(
            pull_app.mode,
            Mode::Confirm {
                mutation: GitHubMutation::PullMerge {
                    number: 8,
                    method: MergeMethod::Squash,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn merge_method_picker_cycles_and_confirms_selected_strategy() {
        let mut pull_app = app(DetailResource::Pull(9));
        pull_app.detail = Some(Detail::Pull(
            serde_json::from_str(
                r#"{"number":9,"title":"feature","state":"OPEN","isDraft":false,"headRefOid":"abcdef123456","statusCheckRollup":[]}"#,
            )
            .unwrap(),
        ));
        pull_app.merge_pull();
        pull_app.handle_key(KeyCode::Char('j'), KeyModifiers::NONE, 10);
        pull_app.confirm_selected_merge();
        assert!(matches!(
            pull_app.mode,
            Mode::Confirm {
                mutation: GitHubMutation::PullMerge {
                    number: 9,
                    method: MergeMethod::Rebase,
                    ..
                },
                ..
            }
        ));
    }
}
