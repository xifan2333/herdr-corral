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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubMutation {
    IssueComment { number: u64, body: String },
    IssueState { number: u64, open: bool },
    PullComment { number: u64, body: String },
    PullApprove { number: u64 },
    PullRequestChanges { number: u64, body: String },
    PullMergeSquash { number: u64, head_sha: String },
    PullState { number: u64, open: bool },
    RunCancel { run_id: u64 },
    RunRerun { run_id: u64, failed_only: bool },
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
