use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    /// Selector accepted by `gh -R`: `[HOST/]OWNER/REPO`.
    pub selector: String,
    pub name_with_owner: String,
    pub host: String,
    pub url: String,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
