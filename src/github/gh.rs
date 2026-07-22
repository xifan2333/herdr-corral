use super::{GitHubAdapter, Issue, PullRequest, Repository, WorkflowRun};
use serde::Deserialize;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

const ISSUE_FIELDS: &str = "number,title,state,author,labels,updatedAt,url";
const PR_FIELDS: &str = "number,title,state,isDraft,author,baseRefName,headRefName,headRefOid,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup,updatedAt,url";
const RUN_FIELDS: &str = "databaseId,workflowName,displayTitle,status,conclusion,headBranch,event,attempt,createdAt,updatedAt,url";

#[derive(Clone, Debug)]
pub struct GhCli {
    cwd: PathBuf,
}

impl GhCli {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }

    fn output(&self, args: &[String]) -> Result<Vec<u8>, String> {
        let mut child = Command::new("gh")
            .args(args)
            .current_dir(&self.cwd)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_PAGER", "cat")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => {
                    "GitHub CLI not found; install `gh` and run `gh auth login`".to_string()
                }
                _ => format!("could not run gh: {error}"),
            })?;

        // Drain both pipes while waiting; otherwise a large JSON response can
        // fill an OS pipe and prevent `gh` from ever reaching exit.
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let stdout_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            bytes
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        });

        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if started.elapsed() < COMMAND_TIMEOUT => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err("gh timed out after 30 seconds".to_string());
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!("could not wait for gh: {error}"));
                }
            }
        };
        let stdout = stdout_reader.join().unwrap_or_default();
        let stderr = stderr_reader.join().unwrap_or_default();
        let status = status?;
        if status.success() {
            return Ok(stdout);
        }
        let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("gh exited with {status}")
        } else {
            stderr
        })
    }

    fn json<T: serde::de::DeserializeOwned>(&self, args: &[String]) -> Result<T, String> {
        let bytes = self.output(args)?;
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid gh JSON: {error}"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoJson {
    name_with_owner: String,
    url: String,
}

fn repository_from(raw: RepoJson) -> Result<Repository, String> {
    let without_scheme = raw
        .url
        .strip_prefix("https://")
        .or_else(|| raw.url.strip_prefix("http://"))
        .unwrap_or(&raw.url);
    let host = without_scheme
        .split('/')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("gh returned an invalid repository URL: {}", raw.url))?
        .to_string();
    let selector = if host.eq_ignore_ascii_case("github.com") {
        raw.name_with_owner.clone()
    } else {
        format!("{host}/{}", raw.name_with_owner)
    };
    Ok(Repository {
        selector,
        name_with_owner: raw.name_with_owner,
        host,
        url: raw.url,
    })
}

impl GitHubAdapter for GhCli {
    fn discover(&self) -> Result<Repository, String> {
        let args = [
            "repo".into(),
            "view".into(),
            "--json".into(),
            "nameWithOwner,url".into(),
        ];
        repository_from(self.json(&args)?)
    }

    fn issues(&self, repo: &Repository, limit: usize, state: &str) -> Result<Vec<Issue>, String> {
        self.json(&[
            "issue".into(),
            "list".into(),
            "--repo".into(),
            repo.selector.clone(),
            "--limit".into(),
            limit.to_string(),
            "--state".into(),
            state.into(),
            "--json".into(),
            ISSUE_FIELDS.into(),
        ])
    }

    fn pulls(
        &self,
        repo: &Repository,
        limit: usize,
        state: &str,
    ) -> Result<Vec<PullRequest>, String> {
        self.json(&[
            "pr".into(),
            "list".into(),
            "--repo".into(),
            repo.selector.clone(),
            "--limit".into(),
            limit.to_string(),
            "--state".into(),
            state.into(),
            "--json".into(),
            PR_FIELDS.into(),
        ])
    }

    fn runs(&self, repo: &Repository, limit: usize) -> Result<Vec<WorkflowRun>, String> {
        self.json(&[
            "run".into(),
            "list".into(),
            "--repo".into(),
            repo.selector.clone(),
            "--limit".into(),
            limit.to_string(),
            "--json".into(),
            RUN_FIELDS.into(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_github_and_enterprise_selectors() {
        let github = repository_from(RepoJson {
            name_with_owner: "owner/repo".into(),
            url: "https://github.com/owner/repo".into(),
        })
        .unwrap();
        assert_eq!(github.selector, "owner/repo");
        assert_eq!(github.host, "github.com");

        let enterprise = repository_from(RepoJson {
            name_with_owner: "team/repo".into(),
            url: "https://github.example.test/team/repo".into(),
        })
        .unwrap();
        assert_eq!(enterprise.selector, "github.example.test/team/repo");
    }
}
