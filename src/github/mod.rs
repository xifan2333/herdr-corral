//! GitHub boundary for Corral.
//!
//! Authentication, host selection, and API compatibility belong to `gh`; the
//! TUI consumes typed models through [`GitHubAdapter`] and never handles tokens.

mod gh;
mod model;

pub use gh::GhCli;
pub use model::{
    Actor, Comment, Issue, IssueDetail, PullFile, PullRequest, PullRequestDetail, Repository,
    Review, WorkflowJob, WorkflowRun, WorkflowRunDetail, WorkflowStep,
};

/// Read-only GitHub operations used by the first GitHub feature slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub const ALL: [MergeMethod; 3] =
        [MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase];

    pub fn label(self) -> &'static str {
        match self {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            MergeMethod::Merge => "Merge",
            MergeMethod::Squash => "Squash",
            MergeMethod::Rebase => "Rebase",
        }
    }

    pub fn flag(self) -> &'static str {
        match self {
            MergeMethod::Merge => "--merge",
            MergeMethod::Squash => "--squash",
            MergeMethod::Rebase => "--rebase",
        }
    }

    pub fn index(self) -> usize {
        match self {
            MergeMethod::Merge => 0,
            MergeMethod::Squash => 1,
            MergeMethod::Rebase => 2,
        }
    }

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
}

pub trait GitHubDetailAdapter: Send + Sync {
    fn issue_detail(&self, repo: &str, number: u64) -> Result<IssueDetail, String>;
    fn pull_detail(&self, repo: &str, number: u64) -> Result<PullRequestDetail, String>;
    fn run_detail(&self, repo: &str, run_id: u64) -> Result<WorkflowRunDetail, String>;
    fn pull_patch(&self, repo: &str, number: u64) -> Result<String, String>;
    fn run_log(&self, repo: &str, run_id: u64, failed_only: bool) -> Result<String, String>;
    fn mutate(&self, repo: &str, mutation: &GitHubMutation) -> Result<String, String>;
}

pub trait GitHubAdapter: Send + Sync {
    fn discover(&self) -> Result<Repository, String>;
    fn issues(&self, repo: &Repository, limit: usize, state: &str) -> Result<Vec<Issue>, String>;
    fn pulls(
        &self,
        repo: &Repository,
        limit: usize,
        state: &str,
    ) -> Result<Vec<PullRequest>, String>;
    fn runs(&self, repo: &Repository, limit: usize) -> Result<Vec<WorkflowRun>, String>;
}
