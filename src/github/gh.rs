use super::{
    GitHubAdapter, GitHubDetailAdapter, GitHubMutation, Issue, IssueDetail, PullRequest,
    PullRequestDetail, Repository, Workflow, WorkflowRun, WorkflowRunDetail,
};
use serde::Deserialize;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

const ISSUE_FIELDS: &str = "number,title,state,author,labels,updatedAt,url";
const PR_FIELDS: &str = "number,title,state,isDraft,author,baseRefName,headRefName,headRefOid,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup,updatedAt,url";
const RUN_FIELDS: &str = "databaseId,workflowName,displayTitle,status,conclusion,headBranch,event,attempt,createdAt,updatedAt,url";
const ISSUE_DETAIL_FIELDS: &str =
    "number,title,state,author,labels,body,comments,createdAt,updatedAt,url";
const PR_DETAIL_FIELDS: &str = "number,title,state,isDraft,author,body,comments,reviews,files,baseRefName,headRefName,headRefOid,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup,additions,deletions,changedFiles,createdAt,updatedAt,url";
const RUN_DETAIL_FIELDS: &str = "databaseId,workflowName,displayTitle,status,conclusion,headBranch,headSha,event,attempt,jobs,createdAt,startedAt,updatedAt,url";

#[derive(Clone, Debug)]
pub struct GhCli {
    cwd: PathBuf,
}

impl GhCli {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }

    fn output(&self, args: &[String]) -> Result<Vec<u8>, String> {
        self.output_with_input(args, None)
    }

    fn output_with_input(&self, args: &[String], input: Option<&str>) -> Result<Vec<u8>, String> {
        let mut child = Command::new("gh")
            .args(args)
            .current_dir(&self.cwd)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_PAGER", "cat")
            .env("NO_COLOR", "1")
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => {
                    "GitHub CLI not found; install `gh` and run `gh auth login`".to_string()
                }
                _ => format!("could not run gh: {error}"),
            })?;

        if let Some(input) = input {
            let Some(mut stdin) = child.stdin.take() else {
                let _ = child.kill();
                let _ = child.wait();
                return Err("could not open gh stdin".into());
            };
            if let Err(error) = stdin.write_all(input.as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("could not write gh stdin: {error}"));
            }
        }

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
        // Some list endpoints return an empty body for an empty result set.
        // Treat that as `[]` so callers can deserialize Vec types cleanly.
        let bytes = if bytes.iter().all(u8::is_ascii_whitespace) {
            b"[]".to_vec()
        } else {
            bytes
        };
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid gh JSON: {error}"))
    }

    fn text(&self, args: &[String]) -> Result<String, String> {
        let bytes = self.output(args)?;
        String::from_utf8(bytes).map_err(|error| format!("invalid gh text: {error}"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoJson {
    name_with_owner: String,
    url: String,
    #[serde(default)]
    default_branch_ref: Option<BranchRefJson>,
}

#[derive(Deserialize)]
struct BranchRefJson {
    #[serde(default)]
    name: String,
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
        default_branch: raw
            .default_branch_ref
            .map(|branch| branch.name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "main".into()),
    })
}

impl GitHubDetailAdapter for GhCli {
    fn issue_detail(&self, repo: &str, number: u64) -> Result<IssueDetail, String> {
        self.json(&[
            "issue".into(),
            "view".into(),
            number.to_string(),
            "--repo".into(),
            repo.into(),
            "--json".into(),
            ISSUE_DETAIL_FIELDS.into(),
        ])
    }

    fn pull_detail(&self, repo: &str, number: u64) -> Result<PullRequestDetail, String> {
        self.json(&[
            "pr".into(),
            "view".into(),
            number.to_string(),
            "--repo".into(),
            repo.into(),
            "--json".into(),
            PR_DETAIL_FIELDS.into(),
        ])
    }

    fn run_detail(&self, repo: &str, run_id: u64) -> Result<WorkflowRunDetail, String> {
        self.json(&[
            "run".into(),
            "view".into(),
            run_id.to_string(),
            "--repo".into(),
            repo.into(),
            "--json".into(),
            RUN_DETAIL_FIELDS.into(),
        ])
    }

    fn pull_patch(&self, repo: &str, number: u64) -> Result<String, String> {
        self.text(&[
            "pr".into(),
            "diff".into(),
            number.to_string(),
            "--repo".into(),
            repo.into(),
        ])
    }

    fn run_log(&self, repo: &str, run_id: u64, failed_only: bool) -> Result<String, String> {
        let mut args = vec![
            "run".into(),
            "view".into(),
            run_id.to_string(),
            "--repo".into(),
            repo.into(),
        ];
        args.push(if failed_only { "--log-failed" } else { "--log" }.into());
        self.text(&args)
    }

    fn mutate(&self, repo: &str, mutation: &GitHubMutation) -> Result<String, String> {
        let (args, input): (Vec<String>, Option<&str>) = match mutation {
            GitHubMutation::IssueComment { number, body } => (
                vec![
                    "issue".into(),
                    "comment".into(),
                    number.to_string(),
                    "--repo".into(),
                    repo.into(),
                    "--body-file".into(),
                    "-".into(),
                ],
                Some(body),
            ),
            GitHubMutation::IssueState { number, open } => (
                vec![
                    "issue".into(),
                    if *open { "reopen" } else { "close" }.into(),
                    number.to_string(),
                    "--repo".into(),
                    repo.into(),
                ],
                None,
            ),
            GitHubMutation::PullComment { number, body } => (
                vec![
                    "pr".into(),
                    "comment".into(),
                    number.to_string(),
                    "--repo".into(),
                    repo.into(),
                    "--body-file".into(),
                    "-".into(),
                ],
                Some(body),
            ),
            GitHubMutation::PullApprove { number } => (
                vec![
                    "pr".into(),
                    "review".into(),
                    number.to_string(),
                    "--repo".into(),
                    repo.into(),
                    "--approve".into(),
                ],
                None,
            ),
            GitHubMutation::PullRequestChanges { number, body } => (
                vec![
                    "pr".into(),
                    "review".into(),
                    number.to_string(),
                    "--repo".into(),
                    repo.into(),
                    "--request-changes".into(),
                    "--body-file".into(),
                    "-".into(),
                ],
                Some(body),
            ),
            GitHubMutation::PullMerge {
                number,
                head_sha,
                method,
            } => (
                vec![
                    "pr".into(),
                    "merge".into(),
                    number.to_string(),
                    "--repo".into(),
                    repo.into(),
                    method.flag().into(),
                    "--match-head-commit".into(),
                    head_sha.clone(),
                ],
                None,
            ),
            GitHubMutation::PullState { number, open } => (
                vec![
                    "pr".into(),
                    if *open { "reopen" } else { "close" }.into(),
                    number.to_string(),
                    "--repo".into(),
                    repo.into(),
                ],
                None,
            ),
            GitHubMutation::RunCancel { run_id } => (
                vec![
                    "run".into(),
                    "cancel".into(),
                    run_id.to_string(),
                    "--repo".into(),
                    repo.into(),
                ],
                None,
            ),
            GitHubMutation::RunRerun {
                run_id,
                failed_only,
            } => {
                let mut args = vec![
                    "run".into(),
                    "rerun".into(),
                    run_id.to_string(),
                    "--repo".into(),
                    repo.into(),
                ];
                if *failed_only {
                    args.push("--failed".into());
                }
                (args, None)
            }
            GitHubMutation::WorkflowDispatch {
                workflow,
                r#ref,
                inputs,
            } => {
                let mut args = vec![
                    "workflow".into(),
                    "run".into(),
                    workflow.clone(),
                    "--repo".into(),
                    repo.into(),
                    "--ref".into(),
                    r#ref.clone(),
                ];
                for (key, value) in inputs {
                    args.push("-f".into());
                    args.push(format!("{key}={value}"));
                }
                (args, None)
            }
        };
        let output = self.output_with_input(&args, input)?;
        Ok(String::from_utf8_lossy(&output).trim().to_string())
    }
}

impl GitHubAdapter for GhCli {
    fn discover(&self) -> Result<Repository, String> {
        let args = [
            "repo".into(),
            "view".into(),
            "--json".into(),
            "nameWithOwner,url,defaultBranchRef".into(),
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

    fn workflows(&self, repo: &Repository) -> Result<Vec<Workflow>, String> {
        self.json(&[
            "workflow".into(),
            "list".into(),
            "--repo".into(),
            repo.selector.clone(),
            "--json".into(),
            "id,name,path,state".into(),
        ])
    }

    fn workflow_yaml(
        &self,
        repo: &Repository,
        workflow: &str,
        r#ref: &str,
    ) -> Result<String, String> {
        self.text(&[
            "workflow".into(),
            "view".into(),
            workflow.into(),
            "--repo".into(),
            repo.selector.clone(),
            "--ref".into(),
            r#ref.into(),
            "--yaml".into(),
        ])
    }

    fn dispatch_workflow(
        &self,
        repo: &Repository,
        workflow: &str,
        r#ref: &str,
        inputs: &[(String, String)],
    ) -> Result<String, String> {
        let mut args = vec![
            "workflow".into(),
            "run".into(),
            workflow.into(),
            "--repo".into(),
            repo.selector.clone(),
            "--ref".into(),
            r#ref.into(),
        ];
        for (key, value) in inputs {
            args.push("-f".into());
            args.push(format!("{key}={value}"));
        }
        let output = self.output(&args)?;
        Ok(String::from_utf8_lossy(&output).trim().to_string())
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
            default_branch_ref: Some(BranchRefJson {
                name: "main".into(),
            }),
        })
        .unwrap();
        assert_eq!(github.selector, "owner/repo");
        assert_eq!(github.host, "github.com");
        assert_eq!(github.default_branch, "main");

        let enterprise = repository_from(RepoJson {
            name_with_owner: "team/repo".into(),
            url: "https://github.example.test/team/repo".into(),
            default_branch_ref: None,
        })
        .unwrap();
        assert_eq!(enterprise.selector, "github.example.test/team/repo");
        assert_eq!(enterprise.default_branch, "main");
    }
}
