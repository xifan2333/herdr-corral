use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    /// Selector accepted by `gh -R`: `[HOST/]OWNER/REPO`.
    pub selector: String,
    pub name_with_owner: String,
    pub host: String,
    pub url: String,
    pub default_branch: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Workflow {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowInput {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub input_type: String,
    pub default: String,
    pub options: Vec<String>,
}

/// Minimal parser for `on.workflow_dispatch` and its `inputs` block.
pub fn parse_workflow_dispatch(yaml: &str) -> (bool, Vec<WorkflowInput>) {
    let mut has_dispatch = false;
    let mut inputs = Vec::new();
    let lines: Vec<&str> = yaml.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        if trimmed.contains("workflow_dispatch")
            && (trimmed.starts_with("on:")
                || trimmed.contains('[')
                || trimmed == "workflow_dispatch:"
                || trimmed.starts_with("workflow_dispatch:"))
        {
            has_dispatch = true;
        }
        if trimmed == "workflow_dispatch:" || trimmed.starts_with("workflow_dispatch:") {
            has_dispatch = true;
            let base_indent = indent_of(raw);
            i += 1;
            while i < lines.len() {
                let line = lines[i];
                let t = line.trim();
                if t.is_empty() || t.starts_with('#') {
                    i += 1;
                    continue;
                }
                let ind = indent_of(line);
                if ind <= base_indent {
                    break;
                }
                if t == "inputs:" || t.starts_with("inputs:") {
                    let inputs_indent = ind;
                    i += 1;
                    while i < lines.len() {
                        let line = lines[i];
                        let t = line.trim();
                        if t.is_empty() || t.starts_with('#') {
                            i += 1;
                            continue;
                        }
                        let ind = indent_of(line);
                        if ind <= inputs_indent {
                            break;
                        }
                        if let Some(name) = t.strip_suffix(':').filter(|name| {
                            !name.is_empty()
                                && name
                                    .chars()
                                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
                        }) {
                            let field_indent = ind;
                            let mut input = WorkflowInput {
                                name: name.to_string(),
                                description: String::new(),
                                required: false,
                                input_type: "string".into(),
                                default: String::new(),
                                options: Vec::new(),
                            };
                            i += 1;
                            while i < lines.len() {
                                let line = lines[i];
                                let t = line.trim();
                                if t.is_empty() || t.starts_with('#') {
                                    i += 1;
                                    continue;
                                }
                                let ind = indent_of(line);
                                if ind <= field_indent {
                                    break;
                                }
                                if let Some(value) = t.strip_prefix("description:") {
                                    input.description = unquote(value.trim());
                                } else if let Some(value) = t.strip_prefix("type:") {
                                    input.input_type = unquote(value.trim());
                                } else if let Some(value) = t.strip_prefix("required:") {
                                    input.required = matches!(
                                        value.trim().to_ascii_lowercase().as_str(),
                                        "true" | "yes"
                                    );
                                } else if let Some(value) = t.strip_prefix("default:") {
                                    input.default = unquote(value.trim());
                                } else if t == "options:" || t.starts_with("options:") {
                                    let options_indent = ind;
                                    i += 1;
                                    while i < lines.len() {
                                        let line = lines[i];
                                        let t = line.trim();
                                        if t.is_empty() || t.starts_with('#') {
                                            i += 1;
                                            continue;
                                        }
                                        let ind = indent_of(line);
                                        if ind <= options_indent {
                                            break;
                                        }
                                        if let Some(item) = t.strip_prefix('-') {
                                            input.options.push(unquote(item.trim()));
                                        }
                                        i += 1;
                                    }
                                    continue;
                                }
                                i += 1;
                            }
                            inputs.push(input);
                            continue;
                        }
                        i += 1;
                    }
                    continue;
                }
                i += 1;
            }
            break;
        }
        i += 1;
    }
    (has_dispatch, inputs)
}

fn indent_of(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count()
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
        || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Actor {
    #[serde(default)]
    pub login: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Label {
    #[serde(default)]
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub author: Option<Actor>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub author: Option<Actor>,
    #[serde(default)]
    pub base_ref_name: String,
    #[serde(default)]
    pub head_ref_name: String,
    #[serde(default)]
    pub head_ref_oid: String,
    #[serde(default)]
    pub review_decision: String,
    #[serde(default)]
    pub mergeable: String,
    #[serde(default)]
    pub merge_state_status: String,
    #[serde(default)]
    pub status_check_rollup: Value,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
}

impl Eq for PullRequest {}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[serde(default)]
    pub author: Option<Actor>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub author: Option<Actor>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    #[serde(default)]
    pub author: Option<Actor>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub submitted_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PullFile {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetail {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub author: Option<Actor>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub reviews: Vec<Review>,
    #[serde(default)]
    pub files: Vec<PullFile>,
    #[serde(default)]
    pub base_ref_name: String,
    #[serde(default)]
    pub head_ref_name: String,
    #[serde(default)]
    pub head_ref_oid: String,
    #[serde(default)]
    pub review_decision: String,
    #[serde(default)]
    pub mergeable: String,
    #[serde(default)]
    pub merge_state_status: String,
    #[serde(default)]
    pub status_check_rollup: Value,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(default)]
    pub changed_files: u64,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
}

impl Eq for PullRequestDetail {}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub number: u64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowJob {
    pub database_id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub completed_at: String,
    #[serde(default)]
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub database_id: u64,
    #[serde(default)]
    pub workflow_name: String,
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub head_branch: String,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub attempt: u64,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunDetail {
    pub database_id: u64,
    #[serde(default)]
    pub workflow_name: String,
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub head_branch: String,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub attempt: u64,
    #[serde(default)]
    pub jobs: Vec<WorkflowJob>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_workflow_dispatch_inputs() {
        let yaml = r#"
name: Deployment
on:
  workflow_dispatch:
    inputs:
      tag_name:
        required: true
        type: string
        description: "The tag name"
      dry_run:
        type: boolean
        default: true
jobs: {}
"#;
        let (has, inputs) = parse_workflow_dispatch(yaml);
        assert!(has);
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].name, "tag_name");
        assert!(inputs[0].required);
        assert_eq!(inputs[1].name, "dry_run");
        assert_eq!(inputs[1].default, "true");
    }

    #[test]
    fn parses_minimal_gh_models() {
        let issue: Issue = serde_json::from_str(
            r#"{"number":7,"title":"Bug","state":"OPEN","author":{"login":"octo"},"labels":[],"updatedAt":"2026-01-01T00:00:00Z","url":"https://github.com/o/r/issues/7"}"#,
        )
        .unwrap();
        assert_eq!(issue.number, 7);
        assert_eq!(issue.author.unwrap().login, "octo");

        let run: WorkflowRun = serde_json::from_str(
            r#"{"databaseId":99,"workflowName":"CI","displayTitle":"test","status":"completed","conclusion":"success","headBranch":"main","event":"push","attempt":1,"createdAt":"","updatedAt":"","url":""}"#,
        )
        .unwrap();
        assert_eq!(run.database_id, 99);
        assert_eq!(run.conclusion, "success");
    }

    #[test]
    fn parses_detail_comments_files_and_jobs() {
        let issue: IssueDetail = serde_json::from_str(
            r#"{"number":7,"title":"Bug","state":"OPEN","body":"body","comments":[{"author":{"login":"octo"},"body":"reply","createdAt":"now"}]}"#,
        )
        .unwrap();
        assert_eq!(issue.comments[0].body, "reply");

        let pull: PullRequestDetail = serde_json::from_str(
            r#"{"number":8,"title":"PR","state":"OPEN","files":[{"path":"src/lib.rs","additions":2,"deletions":1}],"statusCheckRollup":[]}"#,
        )
        .unwrap();
        assert_eq!(pull.files[0].path, "src/lib.rs");

        let run: WorkflowRunDetail = serde_json::from_str(
            r#"{"databaseId":9,"workflowName":"CI","jobs":[{"databaseId":10,"name":"test","steps":[{"name":"cargo test","number":1,"status":"completed","conclusion":"success"}]}]}"#,
        )
        .unwrap();
        assert_eq!(run.jobs[0].steps[0].name, "cargo test");
    }
}
