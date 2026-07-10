use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkSource {
    GitHub,
    Jira,
}

impl fmt::Display for WorkSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitHub => write!(f, "github"),
            Self::Jira => write!(f, "jira"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkItem {
    pub source: WorkSource,
    pub id: String,
    pub external_id: String,
    pub repo: Option<String>,
    pub title: String,
    pub url: String,
    pub status: String,
    #[serde(default)]
    pub assignees: Vec<String>,
    pub labels: Vec<String>,
    pub priority: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    // Planning dates. Empty string when the source doesn't provide them
    // (GitHub issues, or Jira issues with no start/due date set).
    #[serde(default)]
    pub start_date: String,
    #[serde(default)]
    pub target_date: String,
    pub author: Option<String>,
    pub container: String,
    pub source_metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub author: Option<String>,
    pub created_at: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssigneeOption {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkItemDetail {
    pub item: WorkItem,
    pub body: Option<String>,
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub project_status: Option<String>,
    #[serde(default)]
    pub status_options: Vec<String>,
    #[serde(default)]
    pub assignee_options: Vec<AssigneeOption>,
    #[serde(default)]
    pub assignee_selected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceWarning {
    pub source: WorkSource,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkItemsResponse {
    pub data: Vec<WorkItem>,
    pub warnings: Vec<SourceWarning>,
    pub fetched_at: String,
    pub cache_status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryResponse {
    pub totals_by_source: BTreeMap<String, usize>,
    pub totals_by_status: BTreeMap<String, usize>,
    pub warnings: Vec<SourceWarning>,
    pub fetched_at: String,
    pub cache_status: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{SourceWarning, WorkItem, WorkItemsResponse, WorkSource};

    #[test]
    fn work_item_serializes_with_normalized_fields() {
        let item = WorkItem {
            source: WorkSource::GitHub,
            id: "github:openai/quasar#123".to_string(),
            external_id: "123".to_string(),
            repo: Some("openai/quasar".to_string()),
            title: "Investigate sync gap".to_string(),
            url: "https://example.com/issues/123".to_string(),
            status: "open".to_string(),
            assignees: vec!["kai".to_string()],
            labels: vec!["bug".to_string(), "infra".to_string()],
            priority: Some("medium".to_string()),
            created_at: "2026-07-06T10:00:00Z".to_string(),
            updated_at: "2026-07-06T11:00:00Z".to_string(),
            start_date: "2026-07-01".to_string(),
            target_date: "2026-07-15".to_string(),
            author: Some("octocat".to_string()),
            container: "openai/quasar".to_string(),
            source_metadata: Some(json!({ "repo": "openai/quasar" })),
        };

        let serialized = serde_json::to_value(item).expect("work item should serialize");

        assert_eq!(serialized["source"], "github");
        assert_eq!(serialized["id"], "github:openai/quasar#123");
        assert_eq!(serialized["external_id"], "123");
        assert_eq!(serialized["repo"], "openai/quasar");
        assert_eq!(serialized["container"], "openai/quasar");
        assert_eq!(serialized["title"], "Investigate sync gap");
    }

    #[test]
    fn response_serializes_with_warnings_and_fetch_metadata() {
        let response = WorkItemsResponse {
            data: vec![],
            warnings: vec![SourceWarning {
                source: WorkSource::Jira,
                message: "acli unavailable".to_string(),
            }],
            fetched_at: "2026-07-06T11:00:00Z".to_string(),
            cache_status: "miss".to_string(),
        };

        let serialized = serde_json::to_value(response).expect("response wrapper should serialize");

        assert_eq!(serialized["warnings"][0]["source"], "jira");
        assert_eq!(serialized["warnings"][0]["message"], "acli unavailable");
        assert_eq!(serialized["fetched_at"], "2026-07-06T11:00:00Z");
        assert_eq!(serialized["cache_status"], "miss");
    }

    #[test]
    fn work_item_detail_serializes_with_body_and_comments() {
        use super::{Comment, WorkItemDetail};

        let item = WorkItem {
            source: WorkSource::GitHub,
            id: "github:openai/quasar#123".to_string(),
            external_id: "123".to_string(),
            repo: Some("openai/quasar".to_string()),
            title: "Investigate sync gap".to_string(),
            url: "https://example.com/issues/123".to_string(),
            status: "open".to_string(),
            assignees: Vec::new(),
            labels: vec![],
            priority: None,
            created_at: "2026-07-06T10:00:00Z".to_string(),
            updated_at: "2026-07-06T11:00:00Z".to_string(),
            start_date: String::new(),
            target_date: String::new(),
            author: Some("octocat".to_string()),
            container: "openai/quasar".to_string(),
            source_metadata: None,
        };
        let detail = WorkItemDetail {
            item,
            body: Some("## Details\nsome text".to_string()),
            comments: vec![Comment {
                author: Some("octocat".to_string()),
                created_at: "2026-07-06T12:00:00Z".to_string(),
                body: "first comment".to_string(),
            }],
            project_status: None,
            status_options: Vec::new(),
            assignee_options: Vec::new(),
            assignee_selected: Vec::new(),
        };

        let serialized = serde_json::to_value(detail).expect("detail should serialize");
        assert_eq!(serialized["item"]["id"], "github:openai/quasar#123");
        assert_eq!(serialized["body"], "## Details\nsome text");
        assert_eq!(serialized["comments"][0]["author"], "octocat");
        assert_eq!(serialized["comments"][0]["body"], "first comment");
    }

    #[test]
    fn work_item_detail_carries_project_status_and_options() {
        use super::WorkItemDetail;

        let item = WorkItem {
            source: WorkSource::GitHub,
            id: "github:openai/quasar#123".to_string(),
            external_id: "123".to_string(),
            repo: Some("openai/quasar".to_string()),
            title: "Investigate sync gap".to_string(),
            url: "https://example.com/issues/123".to_string(),
            status: "open".to_string(),
            assignees: Vec::new(),
            labels: vec![],
            priority: None,
            created_at: "2026-07-06T10:00:00Z".to_string(),
            updated_at: "2026-07-06T11:00:00Z".to_string(),
            start_date: String::new(),
            target_date: String::new(),
            author: Some("octocat".to_string()),
            container: "openai/quasar".to_string(),
            source_metadata: None,
        };
        let detail = WorkItemDetail {
            item,
            body: None,
            comments: vec![],
            project_status: Some("In Progress".to_string()),
            status_options: vec![
                "Todo".to_string(),
                "In Progress".to_string(),
                "Done".to_string(),
            ],
            assignee_options: Vec::new(),
            assignee_selected: Vec::new(),
        };
        let serialized = serde_json::to_value(detail).expect("serialize");
        assert_eq!(serialized["project_status"], "In Progress");
        assert_eq!(serialized["status_options"][1], "In Progress");
    }
}
