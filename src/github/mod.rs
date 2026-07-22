//! GitHub boundary for Corral.
//!
//! Authentication, host selection, and API compatibility belong to `gh`; the
//! TUI consumes typed models through [`GitHubAdapter`] and never handles tokens.

mod gh;
mod model;

pub use gh::GhCli;
pub use model::{Actor, Issue, PullRequest, Repository, WorkflowRun};

/// Read-only GitHub operations used by the first GitHub feature slice.
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
