//! GitHub Issues, Pull Requests, and Actions backed by the `gh` CLI.
//!
//! The body follows SCM's state-tree shape: three real section headers with
//! count badges, collapsible children, and one shared selection/scroll axis.
//! Long content is handed to the owner-scoped nvim pane.

use super::view::{FeatureView, KeyOutcome};
use crate::config::{self, Config};
use crate::github::{
    parse_workflow_dispatch, GhCli, GitHubAdapter, Issue, PullRequest, Repository, Workflow,
    WorkflowInput, WorkflowRun,
};
use crate::ui::Palette;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use serde_json::Value;
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ISSUE_PAGE: usize = 30;
const PR_PAGE: usize = 30;
const RUN_PAGE: usize = 20;
const RUN_REFRESH: Duration = Duration::from_secs(5);
const NOTICE_SUCCESS_TTL: Duration = Duration::from_secs(2);
const NOTICE_ERROR_TTL: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Issues,
    Pulls,
    Workflows,
    Actions,
}

impl Section {
    const ALL: [Section; 4] = [
        Section::Issues,
        Section::Pulls,
        Section::Workflows,
        Section::Actions,
    ];

    fn index(self) -> usize {
        match self {
            Section::Issues => 0,
            Section::Pulls => 1,
            Section::Workflows => 2,
            Section::Actions => 3,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Section::Issues => "ISSUES",
            Section::Pulls => "PULL REQUESTS",
            Section::Workflows => "WORKFLOWS",
            Section::Actions => "ACTIONS",
        }
    }
}

struct Collection<T> {
    items: Vec<T>,
    limit: usize,
    loading: bool,
    error: Option<String>,
}

impl<T> Collection<T> {
    fn new(limit: usize) -> Self {
        Self {
            items: Vec::new(),
            limit,
            loading: false,
            error: None,
        }
    }
}

#[derive(Clone, Debug)]
enum Row {
    Header {
        section: Section,
        count: usize,
        collapsed: bool,
    },
    Issue(usize),
    Pull(usize),
    Run(usize),
    Workflow(usize),
    Status {
        section: Section,
        message: String,
        error: bool,
    },
}

impl Row {
    fn section(&self) -> Section {
        match self {
            Row::Header { section, .. } | Row::Status { section, .. } => *section,
            Row::Issue(_) => Section::Issues,
            Row::Pull(_) => Section::Pulls,
            Row::Workflow(_) => Section::Workflows,
            Row::Run(_) => Section::Actions,
        }
    }

    fn selectable(&self) -> bool {
        !matches!(self, Row::Status { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKey {
    Header(Section),
    Issue(u64),
    Pull(u64),
    Run(u64),
    Workflow(u64),
}

enum Payload {
    Repository(Repository),
    Issues(Vec<Issue>),
    Pulls(Vec<PullRequest>),
    Runs(Vec<WorkflowRun>),
    Workflows(Vec<Workflow>),
    WorkflowYaml {
        workflow: String,
        display: String,
        r#ref: String,
        yaml: String,
    },
    Dispatch(String),
}

#[derive(Clone, Copy)]
enum LoadTarget {
    Repository,
    Issues,
    Pulls,
    Runs,
    Workflows,
    WorkflowYaml,
    Dispatch,
}

struct Completion {
    generation: u64,
    target: LoadTarget,
    result: Result<Payload, String>,
}

struct PendingDispatch {
    workflow: String,
    r#ref: String,
    inputs: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
struct DispatchField {
    input: WorkflowInput,
    value: String,
}

enum DispatchMode {
    Confirm(PendingDispatch),
    Form {
        workflow: String,
        display: String,
        r#ref: String,
        fields: Vec<DispatchField>,
        selected: usize,
        editing: bool,
        edit: Vec<char>,
    },
}

struct FilterEdit {
    section: Section,
    chars: Vec<char>,
}

pub struct GitHubView {
    cwd: PathBuf,
    adapter: Arc<dyn GitHubAdapter>,
    config: Arc<Config>,
    repo: Option<Repository>,
    repo_loading: bool,
    repo_error: Option<String>,
    issues: Collection<Issue>,
    pulls: Collection<PullRequest>,
    runs: Collection<WorkflowRun>,
    workflows: Collection<Workflow>,
    collapsed: [bool; 4],
    issue_state: usize,
    pr_state: usize,
    filters: [String; 4],
    filter_edit: Option<FilterEdit>,
    rows: Vec<Row>,
    selectable: Vec<usize>,
    selected: usize,
    scroll: usize,
    generation: u64,
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
    body_top: Cell<u16>,
    body_height: Cell<u16>,
    notice: Option<(String, bool)>,
    notice_at: Option<Instant>,
    last_runs_refresh: Instant,
    dispatch_mode: Option<DispatchMode>,
    dispatching: bool,
}

impl GitHubView {
    pub fn new(cwd: PathBuf, config: Arc<Config>) -> Self {
        let adapter = Arc::new(GhCli::new(cwd.clone()));
        Self::with_adapter(cwd, config, adapter)
    }

    fn with_adapter(cwd: PathBuf, config: Arc<Config>, adapter: Arc<dyn GitHubAdapter>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut view = Self {
            cwd,
            adapter,
            config,
            repo: None,
            repo_loading: false,
            repo_error: None,
            issues: Collection::new(ISSUE_PAGE),
            pulls: Collection::new(PR_PAGE),
            runs: Collection::new(RUN_PAGE),
            workflows: Collection::new(50),
            collapsed: [false; 4],
            issue_state: 0,
            pr_state: 0,
            filters: std::array::from_fn(|_| String::new()),
            filter_edit: None,
            rows: Vec::new(),
            selectable: Vec::new(),
            selected: 0,
            scroll: 0,
            generation: 0,
            sender,
            receiver,
            body_top: Cell::new(0),
            body_height: Cell::new(0),
            notice: None,
            notice_at: None,
            last_runs_refresh: Instant::now(),
            dispatch_mode: None,
            dispatching: false,
        };
        view.rebuild_rows(None);
        view
    }

    fn issue_state(&self) -> &'static str {
        ["open", "closed", "all"][self.issue_state]
    }

    fn pr_state(&self) -> &'static str {
        ["open", "closed", "merged", "all"][self.pr_state]
    }

    fn section_state(&self, section: Section) -> &'static str {
        match section {
            Section::Issues => self.issue_state(),
            Section::Pulls => self.pr_state(),
            Section::Workflows => "active",
            Section::Actions => "recent",
        }
    }

    fn begin_discover(&mut self) {
        if self.repo_loading {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.repo_loading = true;
        self.repo_error = None;
        self.rebuild_rows(self.selected_key());
        let adapter = Arc::clone(&self.adapter);
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = adapter.discover().map(Payload::Repository);
            let _ = sender.send(Completion {
                generation,
                target: LoadTarget::Repository,
                result,
            });
        });
    }

    fn start_load(&mut self, section: Section) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let generation = self.generation;
        let adapter = Arc::clone(&self.adapter);
        let sender = self.sender.clone();
        match section {
            Section::Issues if !self.issues.loading => {
                self.issues.loading = true;
                self.issues.error = None;
                let limit = self.issues.limit;
                let state = self.issue_state().to_string();
                std::thread::spawn(move || {
                    let result = adapter.issues(&repo, limit, &state).map(Payload::Issues);
                    let _ = sender.send(Completion {
                        generation,
                        target: LoadTarget::Issues,
                        result,
                    });
                });
            }
            Section::Pulls if !self.pulls.loading => {
                self.pulls.loading = true;
                self.pulls.error = None;
                let limit = self.pulls.limit;
                let state = self.pr_state().to_string();
                std::thread::spawn(move || {
                    let result = adapter.pulls(&repo, limit, &state).map(Payload::Pulls);
                    let _ = sender.send(Completion {
                        generation,
                        target: LoadTarget::Pulls,
                        result,
                    });
                });
            }
            Section::Workflows if !self.workflows.loading => {
                self.workflows.loading = true;
                self.workflows.error = None;
                let adapter = Arc::clone(&adapter);
                let repo = repo.clone();
                let sender = sender.clone();
                std::thread::spawn(move || {
                    let result = adapter.workflows(&repo).map(Payload::Workflows);
                    let _ = sender.send(Completion {
                        generation,
                        target: LoadTarget::Workflows,
                        result,
                    });
                });
            }
            Section::Actions if !self.runs.loading => {
                self.runs.loading = true;
                self.runs.error = None;
                let limit = self.runs.limit;
                self.last_runs_refresh = Instant::now();
                let adapter = Arc::clone(&adapter);
                let repo = repo.clone();
                let sender = sender.clone();
                std::thread::spawn(move || {
                    let result = adapter.runs(&repo, limit).map(Payload::Runs);
                    let _ = sender.send(Completion {
                        generation,
                        target: LoadTarget::Runs,
                        result,
                    });
                });
            }
            _ => return,
        }
        self.rebuild_rows(self.selected_key());
    }

    fn load_expanded(&mut self) {
        for section in Section::ALL {
            if !self.collapsed[section.index()] {
                self.start_load(section);
            }
        }
    }

    fn apply_completion(&mut self, completion: Completion) {
        if completion.generation != self.generation {
            return;
        }
        let selected = self.selected_key();
        match completion.result {
            Ok(Payload::Repository(repo)) => {
                self.repo_loading = false;
                let changed = self
                    .repo
                    .as_ref()
                    .is_some_and(|current| current.selector != repo.selector);
                if changed {
                    self.issues.items.clear();
                    self.pulls.items.clear();
                    self.runs.items.clear();
                    self.workflows.items.clear();
                }
                self.repo = Some(repo);
                self.rebuild_rows(selected);
                self.load_expanded();
                return;
            }
            Ok(Payload::Issues(items)) => {
                self.issues.items = items;
                self.issues.loading = false;
            }
            Ok(Payload::Pulls(items)) => {
                self.pulls.items = items;
                self.pulls.loading = false;
            }
            Ok(Payload::Runs(items)) => {
                self.runs.items = items;
                self.runs.loading = false;
            }
            Ok(Payload::Workflows(items)) => {
                self.workflows.items = items
                    .into_iter()
                    .filter(|workflow| workflow.state.eq_ignore_ascii_case("active"))
                    .collect();
                self.workflows.loading = false;
            }
            Ok(Payload::WorkflowYaml {
                workflow,
                display,
                r#ref,
                yaml,
            }) => {
                self.dispatching = false;
                let (has_dispatch, inputs) = parse_workflow_dispatch(&yaml);
                if !has_dispatch {
                    self.set_notice("workflow has no workflow_dispatch trigger", true);
                } else if inputs.is_empty() {
                    self.dispatch_mode = Some(DispatchMode::Confirm(PendingDispatch {
                        workflow,
                        r#ref,
                        inputs: Vec::new(),
                    }));
                } else {
                    let fields = inputs
                        .into_iter()
                        .map(|input| DispatchField {
                            value: input.default.clone(),
                            input,
                        })
                        .collect();
                    self.dispatch_mode = Some(DispatchMode::Form {
                        workflow,
                        display,
                        r#ref,
                        fields,
                        selected: 0,
                        editing: false,
                        edit: Vec::new(),
                    });
                }
            }
            Ok(Payload::Dispatch(message)) => {
                self.dispatching = false;
                self.dispatch_mode = None;
                self.set_notice(
                    if message.is_empty() {
                        "workflow dispatched".into()
                    } else {
                        message
                    },
                    false,
                );
                // Fresh runs usually take a moment to appear.
                self.start_load(Section::Actions);
            }
            Err(error) => match completion.target {
                LoadTarget::Repository => {
                    self.repo_loading = false;
                    self.repo_error = Some(error);
                }
                LoadTarget::Issues => {
                    self.issues.loading = false;
                    self.issues.error = Some(error);
                }
                LoadTarget::Pulls => {
                    self.pulls.loading = false;
                    self.pulls.error = Some(error);
                }
                LoadTarget::Runs => {
                    self.runs.loading = false;
                    self.runs.error = Some(error);
                }
                LoadTarget::Workflows => {
                    self.workflows.loading = false;
                    self.workflows.error = Some(error);
                }
                LoadTarget::WorkflowYaml | LoadTarget::Dispatch => {
                    self.dispatching = false;
                    self.set_notice(error, true);
                }
            },
        }
        self.rebuild_rows(selected);
    }

    fn issue_indices(&self) -> Vec<usize> {
        let needle = self.filters[Section::Issues.index()].to_ascii_lowercase();
        self.issues
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                needle.is_empty()
                    || item.title.to_ascii_lowercase().contains(&needle)
                    || item.number.to_string().contains(&needle)
                    || item
                        .author
                        .as_ref()
                        .is_some_and(|author| author.login.to_ascii_lowercase().contains(&needle))
                    || item
                        .labels
                        .iter()
                        .any(|label| label.name.to_ascii_lowercase().contains(&needle))
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn pull_indices(&self) -> Vec<usize> {
        let needle = self.filters[Section::Pulls.index()].to_ascii_lowercase();
        self.pulls
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                needle.is_empty()
                    || item.title.to_ascii_lowercase().contains(&needle)
                    || item.number.to_string().contains(&needle)
                    || item.head_ref_name.to_ascii_lowercase().contains(&needle)
                    || item
                        .author
                        .as_ref()
                        .is_some_and(|author| author.login.to_ascii_lowercase().contains(&needle))
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn run_indices(&self) -> Vec<usize> {
        let needle = self.filters[Section::Actions.index()].to_ascii_lowercase();
        self.runs
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                needle.is_empty()
                    || item.workflow_name.to_ascii_lowercase().contains(&needle)
                    || item.display_title.to_ascii_lowercase().contains(&needle)
                    || item.head_branch.to_ascii_lowercase().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn workflow_indices(&self) -> Vec<usize> {
        let needle = self.filters[Section::Workflows.index()].to_ascii_lowercase();
        self.workflows
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                needle.is_empty()
                    || item.name.to_ascii_lowercase().contains(&needle)
                    || item.path.to_ascii_lowercase().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn section_count(&self, section: Section) -> usize {
        match section {
            Section::Issues => self.issue_indices().len(),
            Section::Pulls => self.pull_indices().len(),
            Section::Workflows => self.workflow_indices().len(),
            Section::Actions => self.run_indices().len(),
        }
    }

    fn section_loading(&self, section: Section) -> bool {
        match section {
            Section::Issues => self.issues.loading,
            Section::Pulls => self.pulls.loading,
            Section::Workflows => self.workflows.loading,
            Section::Actions => self.runs.loading,
        }
    }

    fn section_error(&self, section: Section) -> Option<&str> {
        match section {
            Section::Issues => self.issues.error.as_deref(),
            Section::Pulls => self.pulls.error.as_deref(),
            Section::Workflows => self.workflows.error.as_deref(),
            Section::Actions => self.runs.error.as_deref(),
        }
    }

    fn section_empty(&self, section: Section) -> bool {
        match section {
            Section::Issues => self.issues.items.is_empty(),
            Section::Pulls => self.pulls.items.is_empty(),
            Section::Workflows => self.workflows.items.is_empty(),
            Section::Actions => self.runs.items.is_empty(),
        }
    }

    fn section_filter(&self, section: Section) -> &str {
        &self.filters[section.index()]
    }

    fn selected_key(&self) -> Option<RowKey> {
        let row = self.selected_row()?;
        Some(match row {
            Row::Header { section, .. } => RowKey::Header(*section),
            Row::Issue(index) => RowKey::Issue(self.issues.items.get(*index)?.number),
            Row::Pull(index) => RowKey::Pull(self.pulls.items.get(*index)?.number),
            Row::Run(index) => RowKey::Run(self.runs.items.get(*index)?.database_id),
            Row::Workflow(index) => RowKey::Workflow(self.workflows.items.get(*index)?.id),
            Row::Status { .. } => return None,
        })
    }

    fn row_key(&self, row: &Row) -> Option<RowKey> {
        Some(match row {
            Row::Header { section, .. } => RowKey::Header(*section),
            Row::Issue(index) => RowKey::Issue(self.issues.items.get(*index)?.number),
            Row::Pull(index) => RowKey::Pull(self.pulls.items.get(*index)?.number),
            Row::Run(index) => RowKey::Run(self.runs.items.get(*index)?.database_id),
            Row::Workflow(index) => RowKey::Workflow(self.workflows.items.get(*index)?.id),
            Row::Status { .. } => return None,
        })
    }

    fn rebuild_rows(&mut self, preserve: Option<RowKey>) {
        let old_selected = self.selected;
        self.rows.clear();
        self.selectable.clear();
        for section in Section::ALL {
            let collapsed = self.collapsed[section.index()];
            self.rows.push(Row::Header {
                section,
                count: self.section_count(section),
                collapsed,
            });
            if !collapsed {
                if let Some(error) = self.section_error(section) {
                    self.rows.push(Row::Status {
                        section,
                        message: error.to_string(),
                        error: true,
                    });
                } else {
                    let indices = match section {
                        Section::Issues => self.issue_indices(),
                        Section::Pulls => self.pull_indices(),
                        Section::Workflows => self.workflow_indices(),
                        Section::Actions => self.run_indices(),
                    };
                    if indices.is_empty() {
                        let message = if self.repo_loading || self.section_loading(section) {
                            "loading…"
                        } else if self.section_empty(section) {
                            "(empty)"
                        } else {
                            "no matches"
                        };
                        self.rows.push(Row::Status {
                            section,
                            message: message.into(),
                            error: false,
                        });
                    } else {
                        self.rows
                            .extend(indices.into_iter().map(|index| match section {
                                Section::Issues => Row::Issue(index),
                                Section::Pulls => Row::Pull(index),
                                Section::Workflows => Row::Workflow(index),
                                Section::Actions => Row::Run(index),
                            }));
                    }
                }
            }
        }
        self.selectable = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.selectable().then_some(index))
            .collect();
        self.selected = preserve
            .and_then(|key| {
                self.selectable.iter().position(|row| {
                    self.rows
                        .get(*row)
                        .and_then(|row| self.row_key(row))
                        .is_some_and(|candidate| candidate == key)
                })
            })
            .unwrap_or_else(|| old_selected.min(self.selectable.len().saturating_sub(1)));
        self.ensure_visible();
    }

    fn selected_row(&self) -> Option<&Row> {
        self.selectable
            .get(self.selected)
            .and_then(|index| self.rows.get(*index))
    }

    fn selected_section(&self) -> Option<Section> {
        self.selected_row().map(Row::section)
    }

    fn ensure_visible(&mut self) {
        let height = usize::from(self.body_height.get().max(1));
        let line = self.selectable.get(self.selected).copied().unwrap_or(0);
        if line < self.scroll {
            self.scroll = line;
        } else if line >= self.scroll.saturating_add(height) {
            self.scroll = line.saturating_add(1).saturating_sub(height);
        }
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(height));
    }

    fn move_selection(&mut self, delta: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.selectable.len().saturating_sub(1));
        self.ensure_visible();
    }

    fn focus_section(&mut self, section: Section) {
        if let Some(position) = self.selectable.iter().position(|row| {
            matches!(
                self.rows.get(*row),
                Some(Row::Header { section: candidate, .. }) if *candidate == section
            )
        }) {
            self.selected = position;
            self.ensure_visible();
        }
    }

    fn toggle_selected(&mut self) -> KeyOutcome {
        match self.selected_row() {
            Some(Row::Header { section, .. }) => {
                let section = *section;
                self.collapsed[section.index()] = !self.collapsed[section.index()];
                let expanded = !self.collapsed[section.index()];
                self.rebuild_rows(Some(RowKey::Header(section)));
                if expanded && self.section_empty(section) {
                    self.start_load(section);
                }
                KeyOutcome::Handled
            }
            Some(Row::Issue(_) | Row::Pull(_) | Row::Run(_)) => self.view_selected(),
            Some(Row::Workflow(_)) => self.request_dispatch(),
            _ => KeyOutcome::Handled,
        }
    }

    fn collapse_selected(&mut self) -> KeyOutcome {
        let Some(section) = self.selected_section() else {
            return KeyOutcome::Handled;
        };
        self.collapsed[section.index()] = true;
        self.rebuild_rows(Some(RowKey::Header(section)));
        KeyOutcome::Handled
    }

    fn expand_selected(&mut self) -> KeyOutcome {
        let Some(section) = self.selected_section() else {
            return KeyOutcome::Handled;
        };
        self.collapsed[section.index()] = false;
        self.rebuild_rows(Some(RowKey::Header(section)));
        if self.section_empty(section) {
            self.start_load(section);
        }
        KeyOutcome::Handled
    }

    fn collapse_all(&mut self) -> KeyOutcome {
        self.collapsed = [true; 4];
        let section = self.selected_section().unwrap_or(Section::Issues);
        self.rebuild_rows(Some(RowKey::Header(section)));
        KeyOutcome::Handled
    }

    fn refresh_selected(&mut self) {
        if self.repo.is_none() {
            self.begin_discover();
        } else if let Some(section) = self.selected_section() {
            self.start_load(section);
        }
    }

    fn cycle_state(&mut self) {
        match self.selected_section() {
            Some(Section::Issues) if !self.issues.loading => {
                self.issue_state = (self.issue_state + 1) % 3;
                self.issues.items.clear();
                self.rebuild_rows(Some(RowKey::Header(Section::Issues)));
                self.start_load(Section::Issues);
            }
            Some(Section::Pulls) if !self.pulls.loading => {
                self.pr_state = (self.pr_state + 1) % 4;
                self.pulls.items.clear();
                self.rebuild_rows(Some(RowKey::Header(Section::Pulls)));
                self.start_load(Section::Pulls);
            }
            _ => {}
        }
    }

    fn load_more(&mut self) {
        let Some(section) = self.selected_section() else {
            return;
        };
        if self.section_loading(section) {
            return;
        }
        match section {
            Section::Issues => self.issues.limit += ISSUE_PAGE,
            Section::Pulls => self.pulls.limit += PR_PAGE,
            Section::Workflows => {}
            Section::Actions => self.runs.limit += RUN_PAGE,
        }
        self.start_load(section);
    }

    fn selected_workflow(&self) -> Option<&Workflow> {
        match self.selected_row() {
            Some(Row::Workflow(index)) => self.workflows.items.get(*index),
            _ => None,
        }
    }

    fn request_dispatch(&mut self) -> KeyOutcome {
        if self.dispatching || self.dispatch_mode.is_some() {
            return KeyOutcome::Handled;
        }
        let Some(repo) = self.repo.clone() else {
            return KeyOutcome::Handled;
        };
        let Some(workflow) = self.selected_workflow().cloned() else {
            self.set_notice("select a workflow to dispatch", true);
            return KeyOutcome::Handled;
        };
        if !workflow.state.eq_ignore_ascii_case("active") {
            self.set_notice("only active workflows can be dispatched", true);
            return KeyOutcome::Handled;
        }
        let workflow_id = if workflow.path.is_empty() {
            workflow.id.to_string()
        } else {
            workflow.path.clone()
        };
        let r#ref = if repo.default_branch.is_empty() {
            "main".into()
        } else {
            repo.default_branch.clone()
        };
        self.dispatching = true;
        self.set_progress(format!("loading {} inputs…", workflow.name));
        let generation = self.generation;
        let adapter = Arc::clone(&self.adapter);
        let sender = self.sender.clone();
        let display = workflow.name.clone();
        std::thread::spawn(move || {
            let result = adapter
                .workflow_yaml(&repo, &workflow_id, &r#ref)
                .map(|yaml| Payload::WorkflowYaml {
                    workflow: workflow_id,
                    display,
                    r#ref,
                    yaml,
                });
            let _ = sender.send(Completion {
                generation,
                target: LoadTarget::WorkflowYaml,
                result,
            });
        });
        KeyOutcome::Handled
    }

    fn confirm_dispatch(&mut self) -> KeyOutcome {
        if self.dispatching {
            return KeyOutcome::Handled;
        }
        let Some(DispatchMode::Confirm(pending)) = self.dispatch_mode.take() else {
            return KeyOutcome::Handled;
        };
        let Some(repo) = self.repo.clone() else {
            return KeyOutcome::Handled;
        };
        self.dispatching = true;
        self.set_progress(format!(
            "dispatching {} @ {}…",
            pending.workflow, pending.r#ref
        ));
        let generation = self.generation;
        let adapter = Arc::clone(&self.adapter);
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = adapter
                .dispatch_workflow(&repo, &pending.workflow, &pending.r#ref, &pending.inputs)
                .map(Payload::Dispatch);
            let _ = sender.send(Completion {
                generation,
                target: LoadTarget::Dispatch,
                result,
            });
        });
        KeyOutcome::Handled
    }

    fn submit_dispatch_form(&mut self) -> KeyOutcome {
        let Some(DispatchMode::Form {
            workflow,
            r#ref,
            fields,
            ..
        }) = &self.dispatch_mode
        else {
            return KeyOutcome::Handled;
        };
        for field in fields {
            if field.input.required && field.value.trim().is_empty() {
                self.set_notice(format!("{} is required", field.input.name), true);
                return KeyOutcome::Handled;
            }
        }
        let pending = PendingDispatch {
            workflow: workflow.clone(),
            r#ref: r#ref.clone(),
            inputs: fields
                .iter()
                .map(|field| (field.input.name.clone(), field.value.clone()))
                .collect(),
        };
        self.dispatch_mode = Some(DispatchMode::Confirm(pending));
        KeyOutcome::Handled
    }

    fn cancel_pending(&mut self) -> KeyOutcome {
        self.dispatch_mode = None;
        KeyOutcome::Handled
    }

    fn on_dispatch_key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
        let action = config::key_token(code, mods)
            .and_then(|token| self.config.action_for_feature_key("github", &token))
            .map(str::to_string);
        match self.dispatch_mode.as_mut() {
            Some(DispatchMode::Confirm(_)) => match action.as_deref() {
                Some(
                    config::internal::GITHUB_CONFIRM
                    | config::internal::TOGGLE
                    | config::internal::OPEN
                    | config::internal::GITHUB_VIEW,
                ) => self.confirm_dispatch(),
                Some(config::internal::GITHUB_FILTER_CANCEL) => self.cancel_pending(),
                _ => KeyOutcome::Handled,
            },
            Some(DispatchMode::Form {
                fields,
                selected,
                editing,
                edit,
                ..
            }) => {
                if *editing {
                    match action.as_deref() {
                        Some(config::internal::GITHUB_FILTER_CANCEL) => {
                            *editing = false;
                            edit.clear();
                        }
                        Some(
                            config::internal::TOGGLE
                            | config::internal::OPEN
                            | config::internal::GITHUB_VIEW
                            | config::internal::GITHUB_CONFIRM,
                        ) => {
                            if let Some(field) = fields.get_mut(*selected) {
                                field.value = edit.iter().collect();
                            }
                            *editing = false;
                            edit.clear();
                        }
                        Some(config::internal::EDIT_BACKSPACE) => {
                            edit.pop();
                        }
                        _ => {
                            if let KeyCode::Char(ch) = code {
                                if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                                    edit.push(ch);
                                }
                            }
                        }
                    }
                    return KeyOutcome::Handled;
                }
                match action.as_deref() {
                    Some(config::internal::UP) => {
                        *selected = selected.saturating_sub(1);
                    }
                    Some(config::internal::DOWN) => {
                        *selected = (*selected + 1).min(fields.len().saturating_sub(1));
                    }
                    Some(
                        config::internal::TOGGLE
                        | config::internal::OPEN
                        | config::internal::GITHUB_VIEW,
                    ) => {
                        if let Some(field) = fields.get(*selected) {
                            *edit = field.value.chars().collect();
                            *editing = true;
                        }
                    }
                    Some(config::internal::GITHUB_CONFIRM) => return self.submit_dispatch_form(),
                    Some(config::internal::GITHUB_FILTER_CANCEL) => return self.cancel_pending(),
                    _ => {}
                }
                KeyOutcome::Handled
            }
            None => KeyOutcome::Handled,
        }
    }

    fn set_progress(&mut self, message: impl Into<String>) {
        // Progress notices do not auto-expire; success/error notices still do.
        self.notice = Some((message.into(), false));
        self.notice_at = None;
    }

    fn start_filter(&mut self) {
        let Some(section) = self.selected_section() else {
            return;
        };
        self.filter_edit = Some(FilterEdit {
            section,
            chars: self.section_filter(section).chars().collect(),
        });
    }

    fn on_filter_key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
        let action = config::key_token(code, mods)
            .and_then(|token| self.config.action_for_feature_key("github", &token));
        match action {
            Some(
                config::internal::GITHUB_FILTER_APPLY
                | config::internal::GITHUB_VIEW
                | config::internal::TOGGLE,
            ) => {
                if let Some(edit) = self.filter_edit.take() {
                    self.filters[edit.section.index()] = edit.chars.into_iter().collect();
                    self.rebuild_rows(Some(RowKey::Header(edit.section)));
                }
            }
            Some(config::internal::GITHUB_FILTER_CANCEL) => self.filter_edit = None,
            Some(config::internal::EDIT_BACKSPACE) => {
                if let Some(edit) = self.filter_edit.as_mut() {
                    edit.chars.pop();
                }
            }
            _ => {
                if let KeyCode::Char(ch) = code {
                    if !mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                        if let Some(edit) = self.filter_edit.as_mut() {
                            edit.chars.push(ch);
                        }
                    }
                }
            }
        }
        KeyOutcome::Handled
    }

    fn preview(&self, kind: &str) -> KeyOutcome {
        let Some(repo) = &self.repo else {
            return KeyOutcome::Handled;
        };
        let mut env = vec![
            ("CORRAL_GITHUB_KIND".into(), kind.into()),
            ("CORRAL_GITHUB_REPO".into(), repo.selector.clone()),
        ];
        match self.selected_row() {
            Some(Row::Issue(index)) => {
                let Some(issue) = self.issues.items.get(*index) else {
                    return KeyOutcome::Handled;
                };
                env.push(("CORRAL_GITHUB_NUMBER".into(), issue.number.to_string()));
            }
            Some(Row::Pull(index)) => {
                let Some(pull) = self.pulls.items.get(*index) else {
                    return KeyOutcome::Handled;
                };
                env.push(("CORRAL_GITHUB_NUMBER".into(), pull.number.to_string()));
            }
            Some(Row::Run(index)) => {
                let Some(run) = self.runs.items.get(*index) else {
                    return KeyOutcome::Handled;
                };
                env.push(("CORRAL_GITHUB_RUN_ID".into(), run.database_id.to_string()));
            }
            _ => return KeyOutcome::Handled,
        }
        KeyOutcome::Shell {
            action: "github_detail".into(),
            file: None,
            env,
        }
    }

    fn view_selected(&self) -> KeyOutcome {
        match self.selected_row() {
            Some(Row::Issue(_)) => self.preview("issue"),
            Some(Row::Pull(_)) => self.preview("pr"),
            Some(Row::Run(_)) => self.preview("run"),
            _ => KeyOutcome::Handled,
        }
    }

    fn dispatch_action(&mut self, action: &str) -> KeyOutcome {
        if self.dispatch_mode.is_some() {
            return KeyOutcome::Handled;
        }
        match action {
            config::internal::UP => {
                self.move_selection(-1);
                KeyOutcome::Handled
            }
            config::internal::DOWN => {
                self.move_selection(1);
                KeyOutcome::Handled
            }
            config::internal::TOP => {
                self.selected = 0;
                self.ensure_visible();
                KeyOutcome::Handled
            }
            config::internal::BOTTOM => {
                self.selected = self.selectable.len().saturating_sub(1);
                self.ensure_visible();
                KeyOutcome::Handled
            }
            config::internal::PAGE_UP => {
                let page = isize::try_from(self.body_height.get().max(2) - 1).unwrap_or(1);
                self.move_selection(-page);
                KeyOutcome::Handled
            }
            config::internal::PAGE_DOWN => {
                let page = isize::try_from(self.body_height.get().max(2) - 1).unwrap_or(1);
                self.move_selection(page);
                KeyOutcome::Handled
            }
            config::internal::TOGGLE | config::internal::OPEN | config::internal::GITHUB_VIEW => {
                self.toggle_selected()
            }
            config::internal::EXPAND => self.expand_selected(),
            config::internal::COLLAPSE => self.collapse_selected(),
            config::internal::COLLAPSE_ALL => self.collapse_all(),
            config::internal::REFRESH => {
                self.refresh_selected();
                KeyOutcome::Handled
            }
            config::internal::GITHUB_ISSUES => {
                self.focus_section(Section::Issues);
                KeyOutcome::Handled
            }
            config::internal::GITHUB_PULLS => {
                self.focus_section(Section::Pulls);
                KeyOutcome::Handled
            }
            config::internal::GITHUB_ACTIONS => {
                self.focus_section(Section::Actions);
                KeyOutcome::Handled
            }
            config::internal::GITHUB_WORKFLOWS => {
                self.focus_section(Section::Workflows);
                KeyOutcome::Handled
            }
            config::internal::GITHUB_NEXT_SECTION => {
                let next = self.selected_section().map_or(Section::Issues, |section| {
                    Section::ALL[(section.index() + 1) % Section::ALL.len()]
                });
                self.focus_section(next);
                KeyOutcome::Handled
            }
            config::internal::GITHUB_PREV_SECTION => {
                let previous = self.selected_section().map_or(Section::Actions, |section| {
                    Section::ALL[(section.index() + Section::ALL.len() - 1) % Section::ALL.len()]
                });
                self.focus_section(previous);
                KeyOutcome::Handled
            }
            config::internal::GITHUB_DIFF if matches!(self.selected_row(), Some(Row::Pull(_))) => {
                self.preview("diff")
            }
            config::internal::GITHUB_CHECKS
                if matches!(self.selected_row(), Some(Row::Pull(_))) =>
            {
                self.preview("checks")
            }
            config::internal::GITHUB_LOG if matches!(self.selected_row(), Some(Row::Run(_))) => {
                self.preview("log")
            }
            config::internal::GITHUB_LOG_FAILED
                if matches!(self.selected_row(), Some(Row::Run(_))) =>
            {
                self.preview("log-failed")
            }
            config::internal::GITHUB_FILTER => {
                self.start_filter();
                KeyOutcome::Handled
            }
            config::internal::GITHUB_LOAD_MORE => {
                self.load_more();
                KeyOutcome::Handled
            }
            config::internal::GITHUB_CYCLE_STATE => {
                self.cycle_state();
                KeyOutcome::Handled
            }
            config::internal::GITHUB_WORKFLOW_DISPATCH => self.request_dispatch(),
            config::internal::GITHUB_CONFIRM => KeyOutcome::Handled,
            config::internal::GITHUB_FILTER_CANCEL => {
                self.cancel_pending();
                KeyOutcome::Handled
            }
            other if Config::is_internal(other) => KeyOutcome::Handled,
            other => {
                let env = self.repo.as_ref().map_or_else(Vec::new, |repo| {
                    vec![("CORRAL_GITHUB_REPO".into(), repo.selector.clone())]
                });
                KeyOutcome::Shell {
                    action: other.into(),
                    file: Some(self.cwd.clone()),
                    env,
                }
            }
        }
    }

    fn set_notice(&mut self, message: impl Into<String>, error: bool) {
        self.notice = Some((message.into(), error));
        self.notice_at = Some(Instant::now());
    }

    fn expire_notice(&mut self) {
        let Some((_, error)) = self.notice.as_ref() else {
            self.notice_at = None;
            return;
        };
        let ttl = if *error {
            NOTICE_ERROR_TTL
        } else {
            NOTICE_SUCCESS_TTL
        };
        if self.notice_at.is_some_and(|shown| shown.elapsed() >= ttl) {
            self.notice = None;
            self.notice_at = None;
        }
    }

    fn row_at_mouse(&self, row: u16) -> Option<usize> {
        let top = self.body_top.get();
        let height = self.body_height.get();
        if height == 0 || row < top || row >= top.saturating_add(height) {
            return None;
        }
        let line = self.scroll + usize::from(row - top);
        self.selectable
            .iter()
            .position(|candidate| *candidate == line)
    }
}

impl FeatureView for GitHubView {
    fn draw(&self, frame: &mut Frame, area: Rect, palette: &Palette) {
        if area.height == 0 {
            return;
        }
        let repo_name = self
            .repo
            .as_ref()
            .map(|repo| repo.name_with_owner.as_str())
            .unwrap_or(if self.repo_loading {
                "connecting…"
            } else {
                "GitHub"
            });
        frame.render_widget(
            Paragraph::new(format!(" {repo_name}")).style(
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect { height: 1, ..area },
        );
        if area.height < 2 {
            return;
        }

        let footer_height = u16::from(area.height >= 3);
        let body = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1 + footer_height),
        };
        self.body_top.set(body.y);
        self.body_height.set(body.height);

        if let Some(error) = &self.repo_error {
            frame.render_widget(
                Paragraph::new(format!(" ! {error}"))
                    .style(
                        Style::default()
                            .fg(palette.red)
                            .add_modifier(Modifier::BOLD),
                    )
                    .wrap(ratatui::widgets::Wrap { trim: true }),
                body,
            );
        } else {
            for (offset, row) in self
                .rows
                .iter()
                .skip(self.scroll)
                .take(usize::from(body.height))
                .enumerate()
            {
                let absolute = self.scroll + offset;
                let selected = self
                    .selectable
                    .get(self.selected)
                    .is_some_and(|row| *row == absolute);
                let rect = Rect {
                    x: body.x,
                    y: body.y + u16::try_from(offset).unwrap_or(0),
                    width: body.width,
                    height: 1,
                };
                let background = if selected {
                    palette.surface0
                } else {
                    Color::Reset
                };
                match row {
                    Row::Header {
                        section,
                        count,
                        collapsed,
                    } => draw_header(
                        frame, rect, *section, *count, *collapsed, background, palette,
                    ),
                    Row::Issue(index) => {
                        if let Some(issue) = self.issues.items.get(*index) {
                            frame.render_widget(
                                Paragraph::new(issue_line(issue, palette))
                                    .style(Style::default().bg(background)),
                                rect,
                            );
                        }
                    }
                    Row::Pull(index) => {
                        if let Some(pull) = self.pulls.items.get(*index) {
                            frame.render_widget(
                                Paragraph::new(pull_line(pull, palette))
                                    .style(Style::default().bg(background)),
                                rect,
                            );
                        }
                    }
                    Row::Run(index) => {
                        if let Some(run) = self.runs.items.get(*index) {
                            frame.render_widget(
                                Paragraph::new(run_line(run, palette))
                                    .style(Style::default().bg(background)),
                                rect,
                            );
                        }
                    }
                    Row::Workflow(index) => {
                        if let Some(workflow) = self.workflows.items.get(*index) {
                            frame.render_widget(
                                Paragraph::new(workflow_line(workflow, palette))
                                    .style(Style::default().bg(background)),
                                rect,
                            );
                        }
                    }
                    Row::Status { message, error, .. } => {
                        frame.render_widget(
                            Paragraph::new(format!("   {message}")).style(Style::default().fg(
                                if *error {
                                    palette.red
                                } else {
                                    palette.overlay1
                                },
                            )),
                            rect,
                        );
                    }
                }
            }
        }

        if footer_height == 1 {
            let footer = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let (message, style) = if let Some(edit) = &self.filter_edit {
                (
                    format!(
                        " /{}: {}│",
                        edit.section.title(),
                        edit.chars.iter().collect::<String>()
                    ),
                    Style::default().fg(palette.text).bg(palette.surface0),
                )
            } else if let Some(mode) = &self.dispatch_mode {
                match mode {
                    DispatchMode::Confirm(pending) => {
                        let extra = if pending.inputs.is_empty() {
                            String::new()
                        } else {
                            format!(" · {} inputs", pending.inputs.len())
                        };
                        (
                            format!(
                                " dispatch {} @ {}{}? y/N",
                                pending.workflow, pending.r#ref, extra
                            ),
                            Style::default().fg(palette.yellow).bg(palette.surface0),
                        )
                    }
                    DispatchMode::Form {
                        display,
                        fields,
                        selected,
                        editing,
                        edit,
                        ..
                    } => {
                        let field = fields.get(*selected);
                        let name = field.map(|f| f.input.name.as_str()).unwrap_or("input");
                        let value = if *editing {
                            format!("{}│", edit.iter().collect::<String>())
                        } else {
                            field.map(|f| f.value.as_str()).unwrap_or("").to_string()
                        };
                        (
                            format!(
                                " {display}: {name}={value}  Enter edit  y dispatch  Esc cancel"
                            ),
                            Style::default().fg(palette.text).bg(palette.surface0),
                        )
                    }
                }
            } else if self.dispatching {
                (" dispatching…".into(), Style::default().fg(palette.accent))
            } else if let Some((message, error)) = &self.notice {
                (
                    format!(" {message}"),
                    Style::default().fg(if *error { palette.red } else { palette.green }),
                )
            } else if self.repo_loading {
                (" connecting…".into(), Style::default().fg(palette.accent))
            } else if let Some(section) = Section::ALL
                .into_iter()
                .find(|section| self.section_loading(*section))
            {
                (
                    format!(" loading {}…", section.title().to_ascii_lowercase()),
                    Style::default().fg(palette.accent),
                )
            } else if let Some(section) = self.selected_section() {
                let filter = self.section_filter(section);
                (
                    format!(
                        " {}{}",
                        self.section_state(section),
                        if filter.is_empty() {
                            String::new()
                        } else {
                            format!(" /{filter}")
                        }
                    ),
                    Style::default().fg(palette.overlay1),
                )
            } else {
                (String::new(), Style::default())
            };
            frame.render_widget(Paragraph::new(message).style(style), footer);
        }
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
        if self.filter_edit.is_some() {
            return self.on_filter_key(code, mods);
        }
        if self.dispatch_mode.is_some() {
            return self.on_dispatch_key(code, mods);
        }
        let Some(token) = config::key_token(code, mods) else {
            return KeyOutcome::Ignored;
        };
        let Some(action) = self
            .config
            .action_for_feature_key("github", &token)
            .map(str::to_string)
        else {
            return KeyOutcome::Ignored;
        };
        self.dispatch_action(&action)
    }

    fn captures_text_input(&self) -> bool {
        self.filter_edit.is_some()
            || matches!(
                self.dispatch_mode,
                Some(DispatchMode::Form { editing: true, .. })
            )
    }

    fn on_shell_result(&mut self, action: &str, ok: bool) {
        if action == "github_detail" && !ok {
            self.set_notice("preview failed", true);
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> KeyOutcome {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(selected) = self.row_at_mouse(mouse.row) else {
                    return KeyOutcome::Ignored;
                };
                let already_selected = self.selected == selected;
                self.selected = selected;
                self.ensure_visible();
                if matches!(
                    self.selected_row(),
                    Some(Row::Header { .. } | Row::Workflow(_))
                ) {
                    self.toggle_selected()
                } else if already_selected {
                    self.view_selected()
                } else {
                    KeyOutcome::Handled
                }
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(3);
                KeyOutcome::Handled
            }
            MouseEventKind::ScrollUp => {
                self.move_selection(-3);
                KeyOutcome::Handled
            }
            _ => KeyOutcome::Ignored,
        }
    }

    fn on_activate(&mut self) {
        if self.repo.is_some() {
            self.load_expanded();
        } else {
            self.begin_discover();
        }
    }

    fn on_tick(&mut self) {
        while let Ok(completion) = self.receiver.try_recv() {
            self.apply_completion(completion);
        }
        if !self.collapsed[Section::Actions.index()]
            && !self.runs.loading
            && self.repo.is_some()
            && self.runs.items.iter().any(|run| {
                matches!(
                    run.status.as_str(),
                    "queued" | "in_progress" | "waiting" | "pending"
                )
            })
            && self.last_runs_refresh.elapsed() >= RUN_REFRESH
        {
            self.start_load(Section::Actions);
        }
        self.expire_notice();
        self.ensure_visible();
    }
}

fn header_columns(rect: Rect, count: usize) -> (Rect, Rect) {
    let badge_width = ((count.max(1).ilog10() + 1) as u16 + 2).min(rect.width);
    let title = Rect {
        width: rect.width.saturating_sub(badge_width),
        ..rect
    };
    let badge = Rect {
        x: rect
            .x
            .saturating_add(rect.width.saturating_sub(badge_width)),
        width: badge_width,
        ..rect
    };
    (title, badge)
}

fn draw_header(
    frame: &mut Frame,
    rect: Rect,
    section: Section,
    count: usize,
    collapsed: bool,
    background: Color,
    palette: &Palette,
) {
    let (title, badge) = header_columns(rect, count);
    let title_style = if background == Color::Reset {
        Style::default().fg(palette.text)
    } else {
        Style::default().fg(palette.text).bg(background)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                if collapsed { "▸ " } else { "▾ " },
                title_style.add_modifier(Modifier::BOLD),
            ),
            Span::styled(section.title(), title_style.add_modifier(Modifier::BOLD)),
        ]))
        .style(title_style),
        title,
    );
    // Match SCM: independent right-side count badge with accent fill.
    frame.render_widget(
        Paragraph::new(count.to_string())
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(palette.panel_bg)
                    .bg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        badge,
    );
}

fn issue_line<'a>(issue: &'a Issue, palette: &Palette) -> Line<'a> {
    let open = issue.state.eq_ignore_ascii_case("open");
    Line::from(vec![
        Span::raw("   "),
        Span::styled(
            if open { "● " } else { "○ " },
            Style::default().fg(if open { palette.green } else { palette.mauve }),
        ),
        Span::styled(
            format!("#{} ", issue.number),
            Style::default().fg(palette.accent),
        ),
        Span::styled(issue.title.as_str(), Style::default().fg(palette.text)),
    ])
}

fn checks_bucket(value: &Value) -> &'static str {
    let Some(checks) = value.as_array() else {
        return "none";
    };
    if checks.is_empty() {
        return "none";
    }
    let mut pending = false;
    for check in checks {
        let state = check
            .get("conclusion")
            .or_else(|| check.get("state"))
            .or_else(|| check.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(
            state.as_str(),
            "failure" | "failed" | "error" | "timed_out" | "cancelled" | "action_required"
        ) {
            return "fail";
        }
        if matches!(
            state.as_str(),
            "" | "pending" | "queued" | "in_progress" | "expected"
        ) {
            pending = true;
        }
    }
    if pending {
        "pending"
    } else {
        "pass"
    }
}

fn pull_line<'a>(pull: &'a PullRequest, palette: &Palette) -> Line<'a> {
    let (glyph, color) = if pull.is_draft {
        ("◌", palette.overlay1)
    } else if pull.state.eq_ignore_ascii_case("merged") {
        ("◆", palette.mauve)
    } else {
        match checks_bucket(&pull.status_check_rollup) {
            "pass" => ("✓", palette.green),
            "fail" => ("×", palette.red),
            "pending" => ("…", palette.yellow),
            _ if pull.state.eq_ignore_ascii_case("open") => ("●", palette.green),
            _ => ("○", palette.red),
        }
    };
    Line::from(vec![
        Span::raw("   "),
        Span::styled(format!("{glyph} "), Style::default().fg(color)),
        Span::styled(
            format!("#{} ", pull.number),
            Style::default().fg(palette.accent),
        ),
        Span::styled(pull.title.as_str(), Style::default().fg(palette.text)),
    ])
}

fn workflow_line<'a>(workflow: &'a Workflow, palette: &Palette) -> Line<'a> {
    let path = PathBuf::from(&workflow.path);
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(workflow.path.as_str());
    Line::from(vec![
        Span::raw("   "),
        Span::styled(
            workflow.name.as_str(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(palette.overlay1)),
        Span::styled(file.to_string(), Style::default().fg(palette.subtext0)),
    ])
}

fn run_line<'a>(run: &'a WorkflowRun, palette: &Palette) -> Line<'a> {
    let (glyph, color) = match (run.status.as_str(), run.conclusion.as_str()) {
        ("completed", "success") => ("✓", palette.green),
        ("completed", "failure" | "timed_out" | "startup_failure") => ("×", palette.red),
        ("completed", "cancelled") => ("■", palette.overlay1),
        ("completed", _) => ("○", palette.yellow),
        _ => ("…", palette.yellow),
    };
    Line::from(vec![
        Span::raw("   "),
        Span::styled(format!("{glyph} "), Style::default().fg(color)),
        Span::styled(
            run.workflow_name.as_str(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(palette.overlay1)),
        Span::styled(
            run.display_title.as_str(),
            Style::default().fg(palette.subtext0),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_count_is_a_separate_right_badge() {
        let (title, badge) = header_columns(Rect::new(4, 7, 30, 1), 19);
        assert_eq!(title, Rect::new(4, 7, 26, 1));
        assert_eq!(badge, Rect::new(30, 7, 4, 1));
    }

    struct UnusedAdapter;

    impl GitHubAdapter for UnusedAdapter {
        fn discover(&self) -> Result<Repository, String> {
            Err("unused".into())
        }

        fn issues(
            &self,
            _repo: &Repository,
            _limit: usize,
            _state: &str,
        ) -> Result<Vec<Issue>, String> {
            Err("unused".into())
        }

        fn pulls(
            &self,
            _repo: &Repository,
            _limit: usize,
            _state: &str,
        ) -> Result<Vec<PullRequest>, String> {
            Err("unused".into())
        }

        fn runs(&self, _repo: &Repository, _limit: usize) -> Result<Vec<WorkflowRun>, String> {
            Err("unused".into())
        }

        fn workflows(&self, _repo: &Repository) -> Result<Vec<Workflow>, String> {
            Err("unused".into())
        }

        fn workflow_yaml(
            &self,
            _repo: &Repository,
            _workflow: &str,
            _ref: &str,
        ) -> Result<String, String> {
            Ok("on:\n  workflow_dispatch:\n".into())
        }

        fn dispatch_workflow(
            &self,
            _repo: &Repository,
            _workflow: &str,
            _ref: &str,
            _inputs: &[(String, String)],
        ) -> Result<String, String> {
            Ok(String::new())
        }
    }

    fn view() -> GitHubView {
        GitHubView::with_adapter(
            PathBuf::from("/repo"),
            Arc::new(Config::for_test()),
            Arc::new(UnusedAdapter),
        )
    }

    fn repo() -> Repository {
        Repository {
            selector: "owner/repo".into(),
            name_with_owner: "owner/repo".into(),
            host: "github.com".into(),
            url: "https://github.com/owner/repo".into(),
            default_branch: "main".into(),
        }
    }

    fn issue(number: u64, title: &str) -> Issue {
        Issue {
            number,
            title: title.into(),
            state: "OPEN".into(),
            author: None,
            labels: Vec::new(),
            updated_at: String::new(),
            url: String::new(),
        }
    }

    #[test]
    fn workflow_dispatch_opens_confirm_or_form_after_yaml_load() {
        let mut view = view();
        view.repo = Some(repo());
        view.workflows.items = vec![Workflow {
            id: 9,
            name: "CI".into(),
            path: ".github/workflows/ci.yml".into(),
            state: "active".into(),
        }];
        view.rebuild_rows(Some(RowKey::Workflow(9)));
        assert!(matches!(view.selected_row(), Some(Row::Workflow(_))));
        view.request_dispatch();
        // YAML load is async; simulate the worker completion path.
        view.apply_completion(Completion {
            generation: view.generation,
            target: LoadTarget::WorkflowYaml,
            result: Ok(Payload::WorkflowYaml {
                workflow: ".github/workflows/ci.yml".into(),
                display: "CI".into(),
                r#ref: "main".into(),
                yaml: "on:\n  workflow_dispatch:\n".into(),
            }),
        });
        assert!(matches!(
            view.dispatch_mode,
            Some(DispatchMode::Confirm(PendingDispatch {
                ref workflow,
                ref r#ref,
                ref inputs
            })) if workflow == ".github/workflows/ci.yml" && r#ref == "main" && inputs.is_empty()
        ));
        view.cancel_pending();
        assert!(view.dispatch_mode.is_none());

        view.apply_completion(Completion {
            generation: view.generation,
            target: LoadTarget::WorkflowYaml,
            result: Ok(Payload::WorkflowYaml {
                workflow: ".github/workflows/deploy.yml".into(),
                display: "Deploy".into(),
                r#ref: "main".into(),
                yaml: "on:\n  workflow_dispatch:\n    inputs:\n      tag_name:\n        required: true\n        type: string\n".into(),
            }),
        });
        assert!(matches!(
            view.dispatch_mode,
            Some(DispatchMode::Form { ref fields, .. }) if fields.len() == 1 && fields[0].input.name == "tag_name"
        ));
    }

    #[test]
    fn sections_are_real_selectable_collapsible_headers() {
        let mut view = view();
        assert_eq!(view.selectable.len(), 4);
        assert!(matches!(
            view.selected_row(),
            Some(Row::Header {
                section: Section::Issues,
                ..
            })
        ));
        view.toggle_selected();
        assert!(view.collapsed[Section::Issues.index()]);
        assert!(!view.rows.iter().any(|row| matches!(
            row,
            Row::Status {
                section: Section::Issues,
                ..
            }
        )));
        view.focus_section(Section::Pulls);
        assert!(matches!(
            view.selected_row(),
            Some(Row::Header {
                section: Section::Pulls,
                ..
            })
        ));
    }

    #[test]
    fn preview_uses_structured_repo_and_number_context() {
        let mut view = view();
        view.repo = Some(repo());
        view.issues.items = vec![issue(42, "Add GitHub view")];
        view.rebuild_rows(Some(RowKey::Issue(42)));
        let outcome = view.on_key(KeyCode::Enter, KeyModifiers::NONE);
        let KeyOutcome::Shell { action, env, .. } = outcome else {
            panic!("expected preview action");
        };
        assert_eq!(action, "github_detail");
        assert!(env.contains(&("CORRAL_GITHUB_KIND".into(), "issue".into())));
        assert!(env.contains(&("CORRAL_GITHUB_REPO".into(), "owner/repo".into())));
        assert!(env.contains(&("CORRAL_GITHUB_NUMBER".into(), "42".into())));
    }

    #[test]
    fn refresh_restores_selection_by_stable_number() {
        let mut view = view();
        view.generation = 7;
        view.issues.items = vec![issue(1, "one"), issue(2, "two")];
        view.rebuild_rows(Some(RowKey::Issue(2)));
        view.apply_completion(Completion {
            generation: 7,
            target: LoadTarget::Issues,
            result: Ok(Payload::Issues(vec![
                issue(2, "two updated"),
                issue(3, "three"),
            ])),
        });
        assert_eq!(view.selected_key(), Some(RowKey::Issue(2)));
    }

    #[test]
    fn stale_repository_results_are_ignored() {
        let mut view = view();
        view.generation = 9;
        view.apply_completion(Completion {
            generation: 8,
            target: LoadTarget::Repository,
            result: Ok(Payload::Repository(repo())),
        });
        assert!(view.repo.is_none());
    }

    #[test]
    fn check_rollup_distinguishes_failure_pending_and_success() {
        assert_eq!(
            checks_bucket(&serde_json::json!([{"conclusion":"SUCCESS"}])),
            "pass"
        );
        assert_eq!(
            checks_bucket(&serde_json::json!([{"status":"IN_PROGRESS"}])),
            "pending"
        );
        assert_eq!(
            checks_bucket(&serde_json::json!([{"conclusion":"FAILURE"}])),
            "fail"
        );
    }
}
