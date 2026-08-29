//! GitHub boundary for Corral.
//!
//! Authentication, host selection, and API compatibility belong to `gh`; the
//! sidebar and full-width detail client both consume typed models through
//! [`GitHubAdapter`] / [`GitHubDetailAdapter`] and never handle tokens.
//!
//! - [`gh`] / [`model`] — CLI adapter + DTOs (data layer)
//! - [`detail`] — full-width `corral-github` interactive client

pub mod detail;
mod gh;
mod model;

pub use detail::{run as run_detail, DetailResource, InitialView};
pub use gh::GhCli;
pub use model::{
    parse_workflow_dispatch, Actor, Comment, Issue, IssueDetail, PullFile, PullRequest,
    PullRequestDetail, Repository, Review, Workflow, WorkflowInput, WorkflowJob, WorkflowRun,
    WorkflowRunDetail, WorkflowStep,
};

/// Read-only GitHub operations used by the first GitHub feature slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub const ALL: [Self; 3] = [Self::Merge, Self::Squash, Self::Rebase];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Squash => "Squash",
            Self::Rebase => "Rebase",
        }
    }

    #[must_use]
    pub const fn flag(self) -> &'static str {
        match self {
            Self::Merge => "--merge",
            Self::Squash => "--squash",
            Self::Rebase => "--rebase",
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Merge => 0,
            Self::Squash => 1,
            Self::Rebase => 2,
        }
    }

    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubMutation {
    IssueComment {
        number: u64,
        body: String,
    },
    IssueState {
        number: u64,
        open: bool,
    },
    PullComment {
        number: u64,
        body: String,
    },
    PullApprove {
        number: u64,
    },
    PullRequestChanges {
        number: u64,
        body: String,
    },
    PullMerge {
        number: u64,
        head_sha: String,
        method: MergeMethod,
    },
    PullState {
        number: u64,
        open: bool,
    },
    RunCancel {
        run_id: u64,
    },
    RunRerun {
        run_id: u64,
        failed_only: bool,
    },
    WorkflowDispatch {
        workflow: String,
        r#ref: String,
        inputs: Vec<(String, String)>,
    },
}

pub trait GitHubDetailAdapter: Send + Sync {
    /// Fetch full issue details.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the GitHub CLI or API call fails.
    fn issue_detail(&self, repo: &str, number: u64) -> Result<IssueDetail, String>;

    /// Fetch full pull request details.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the GitHub CLI or API call fails.
    fn pull_detail(&self, repo: &str, number: u64) -> Result<PullRequestDetail, String>;

    /// Fetch full workflow run details.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the GitHub CLI or API call fails.
    fn run_detail(&self, repo: &str, run_id: u64) -> Result<WorkflowRunDetail, String>;

    /// Fetch the git patch/diff for a pull request.
    ///
    /// # Errors
    ///
    /// Returns `Err` if fetching the patch fails.
    fn pull_patch(&self, repo: &str, number: u64) -> Result<String, String>;

    /// Fetch execution logs for a workflow run.
    ///
    /// # Errors
    ///
    /// Returns `Err` if fetching run logs fails.
    fn run_log(&self, repo: &str, run_id: u64, failed_only: bool) -> Result<String, String>;

    /// Perform a mutation (comment, merge, state change, rerun).
    ///
    /// # Errors
    ///
    /// Returns `Err` if executing the mutation fails.
    fn mutate(&self, repo: &str, mutation: &GitHubMutation) -> Result<String, String>;
}

pub trait GitHubAdapter: Send + Sync {
    /// Discover the current GitHub repository from the current working directory.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the current directory is not a valid GitHub repository.
    fn discover(&self) -> Result<Repository, String>;

    /// List issues for the repository.
    ///
    /// # Errors
    ///
    /// Returns `Err` if fetching issues fails.
    fn issues(&self, repo: &Repository, limit: usize, state: &str) -> Result<Vec<Issue>, String>;

    /// List pull requests for the repository.
    ///
    /// # Errors
    ///
    /// Returns `Err` if fetching pull requests fails.
    fn pulls(
        &self,
        repo: &Repository,
        limit: usize,
        state: &str,
    ) -> Result<Vec<PullRequest>, String>;

    /// List workflow runs for the repository.
    ///
    /// # Errors
    ///
    /// Returns `Err` if fetching workflow runs fails.
    fn runs(&self, repo: &Repository, limit: usize) -> Result<Vec<WorkflowRun>, String>;

    /// List available workflows for the repository.
    ///
    /// # Errors
    ///
    /// Returns `Err` if fetching workflows fails.
    fn workflows(&self, repo: &Repository) -> Result<Vec<Workflow>, String>;

    /// Fetch workflow YAML content from a specific git ref.
    ///
    /// # Errors
    ///
    /// Returns `Err` if reading the workflow file fails.
    fn workflow_yaml(
        &self,
        repo: &Repository,
        workflow: &str,
        r#ref: &str,
    ) -> Result<String, String>;

    /// Dispatch a workflow run with the given inputs.
    ///
    /// # Errors
    ///
    /// Returns `Err` if triggering the workflow fails.
    fn dispatch_workflow(
        &self,
        repo: &Repository,
        workflow: &str,
        r#ref: &str,
        inputs: &[(String, String)],
    ) -> Result<String, String>;
}
