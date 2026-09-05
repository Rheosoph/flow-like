pub mod add_comment;
pub mod attachments;
pub mod boards;
pub mod create_issue;
pub mod delete_issue;
pub mod epic;
pub mod fields;
pub mod get_issue;
pub mod get_project_issues;
pub mod get_transitions;
pub mod links;
pub mod list_projects;
pub mod search_issues;
pub mod sprints;
pub mod transition_issue;
pub mod update_issue;
pub mod users;
pub mod versions;
pub mod worklog;

use crate::data::atlassian::provider::AtlassianProvider;
use flow_like_types::{JsonSchema, Value, json::json};
use serde::{Deserialize, Serialize};

// =============================================================================
// Jira Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraUser {
    /// User account ID
    pub account_id: String,
    /// User display name
    pub display_name: String,
    /// User email (if available)
    pub email_address: Option<String>,
    /// Whether the user is active
    pub active: bool,
    /// User avatar URLs
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraProject {
    /// Project ID
    pub id: String,
    /// Project key (e.g., "PROJ")
    pub key: String,
    /// Project name
    pub name: String,
    /// Project description
    pub description: Option<String>,
    /// Project lead
    pub lead: Option<JiraUser>,
    /// Project URL
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraIssueType {
    /// Issue type ID
    pub id: String,
    /// Issue type name (e.g., "Bug", "Story", "Task")
    pub name: String,
    /// Issue type description
    pub description: Option<String>,
    /// Whether this is a subtask type
    pub subtask: bool,
    /// Icon URL
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraStatus {
    /// Status ID
    pub id: String,
    /// Status name (e.g., "To Do", "In Progress", "Done")
    pub name: String,
    /// Status description
    pub description: Option<String>,
    /// Status category (e.g., "new", "indeterminate", "done")
    pub status_category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraPriority {
    /// Priority ID
    pub id: String,
    /// Priority name (e.g., "Highest", "High", "Medium", "Low", "Lowest")
    pub name: String,
    /// Priority description
    pub description: Option<String>,
    /// Icon URL
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraIssue {
    /// Issue ID
    pub id: String,
    /// Issue key (e.g., "PROJ-123")
    pub key: String,
    /// Issue summary/title
    pub summary: String,
    /// Issue description (may contain ADF or plain text)
    pub description: Option<String>,
    /// Issue type
    pub issue_type: JiraIssueType,
    /// Issue status
    pub status: JiraStatus,
    /// Issue priority
    pub priority: Option<JiraPriority>,
    /// Project the issue belongs to
    pub project: JiraProject,
    /// Issue assignee
    pub assignee: Option<JiraUser>,
    /// Issue reporter
    pub reporter: Option<JiraUser>,
    /// Issue creation timestamp
    pub created: String,
    /// Issue last update timestamp
    pub updated: String,
    /// Labels assigned to the issue
    pub labels: Vec<String>,
    /// Web URL to view the issue
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraComment {
    /// Comment ID
    pub id: String,
    /// Comment author
    pub author: JiraUser,
    /// Comment body (may contain ADF or plain text)
    pub body: String,
    /// Comment creation timestamp
    pub created: String,
    /// Comment last update timestamp
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraTransition {
    /// Transition ID
    pub id: String,
    /// Transition name
    pub name: String,
    /// Target status after transition
    pub to_status: JiraStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraSearchResult {
    /// Issues matching the search
    pub issues: Vec<JiraIssue>,
    /// Total number of matching issues
    pub total: i64,
    /// Starting index of results
    pub start_at: i64,
    /// Maximum results returned
    pub max_results: i64,
}

pub fn build_jira_search_body(
    provider: &AtlassianProvider,
    jql: String,
    max_results: i64,
    start_at: i64,
    fields: Vec<String>,
    next_page_token: String,
) -> Value {
    let mut body = json!({
        "jql": jql,
        "maxResults": max_results.clamp(1, 100),
    });

    if provider.is_cloud {
        if !next_page_token.is_empty() {
            body["nextPageToken"] = json!(next_page_token);
        }
    } else {
        body["startAt"] = json!(start_at.max(0));
    }

    if !fields.is_empty() {
        body["fields"] = json!(fields);
    }

    body
}

pub fn jira_search_total(data: &Value, returned_count: i64) -> i64 {
    data.get("total")
        .and_then(|v| v.as_i64())
        .unwrap_or(returned_count)
}

pub fn jira_search_next_page_token(data: &Value) -> String {
    data.get("nextPageToken")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

pub fn jira_search_has_more(data: &Value, start_at: i64, returned_count: i64, total: i64) -> bool {
    if let Some(is_last) = data.get("isLast").and_then(|v| v.as_bool()) {
        return !is_last;
    }

    if let Some(next_page_token) = data.get("nextPageToken").and_then(|v| v.as_str()) {
        return !next_page_token.is_empty();
    }

    start_at + returned_count < total
}

// =============================================================================
// Helper functions for parsing Jira API responses
// =============================================================================

pub fn parse_jira_user(value: &Value) -> Option<JiraUser> {
    let obj = value.as_object()?;
    Some(JiraUser {
        account_id: obj
            .get("accountId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        display_name: obj
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        email_address: obj
            .get("emailAddress")
            .and_then(|v| v.as_str())
            .map(String::from),
        active: obj.get("active").and_then(|v| v.as_bool()).unwrap_or(true),
        avatar_url: obj
            .get("avatarUrls")
            .and_then(|v| v.get("48x48"))
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

pub fn parse_jira_project(value: &Value) -> Option<JiraProject> {
    let obj = value.as_object()?;
    Some(JiraProject {
        id: obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        key: obj
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        name: obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        description: obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        lead: obj.get("lead").and_then(parse_jira_user),
        url: obj.get("self").and_then(|v| v.as_str()).map(String::from),
    })
}

pub fn parse_jira_issue_type(value: &Value) -> Option<JiraIssueType> {
    let obj = value.as_object()?;
    Some(JiraIssueType {
        id: obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        name: obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        description: obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        subtask: obj
            .get("subtask")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        icon_url: obj
            .get("iconUrl")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

pub fn parse_jira_status(value: &Value) -> Option<JiraStatus> {
    let obj = value.as_object()?;
    Some(JiraStatus {
        id: obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        name: obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        description: obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        status_category: obj
            .get("statusCategory")
            .and_then(|v| v.get("key"))
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

pub fn parse_jira_priority(value: &Value) -> Option<JiraPriority> {
    let obj = value.as_object()?;
    Some(JiraPriority {
        id: obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        name: obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        description: obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        icon_url: obj
            .get("iconUrl")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

pub fn parse_jira_issue(value: &Value, base_url: &str) -> Option<JiraIssue> {
    let obj = value.as_object()?;
    let fields = obj.get("fields")?.as_object()?;
    let key = obj
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    // Parse description - handle ADF format for cloud
    let description = fields.get("description").and_then(|d| {
        if d.is_string() {
            d.as_str().map(String::from)
        } else if d.is_object() {
            // ADF format - try to extract text content
            extract_adf_text(d)
        } else {
            None
        }
    });

    Some(JiraIssue {
        id: obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        key: key.clone(),
        summary: fields
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        description,
        issue_type: fields.get("issuetype").and_then(parse_jira_issue_type)?,
        status: fields.get("status").and_then(parse_jira_status)?,
        priority: fields.get("priority").and_then(parse_jira_priority),
        project: fields.get("project").and_then(parse_jira_project)?,
        assignee: fields.get("assignee").and_then(parse_jira_user),
        reporter: fields.get("reporter").and_then(parse_jira_user),
        created: fields
            .get("created")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        updated: fields
            .get("updated")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        labels: fields
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        url: format!("{}/browse/{}", base_url.trim_end_matches('/'), key),
    })
}

pub fn parse_jira_comment(value: &Value) -> Option<JiraComment> {
    let obj = value.as_object()?;

    // Parse body - handle ADF format for cloud
    let body = obj.get("body").and_then(|b| {
        if b.is_string() {
            b.as_str().map(String::from)
        } else if b.is_object() {
            extract_adf_text(b)
        } else {
            None
        }
    })?;

    Some(JiraComment {
        id: obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        author: obj.get("author").and_then(parse_jira_user)?,
        body,
        created: obj
            .get("created")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        updated: obj
            .get("updated")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

pub fn parse_jira_transition(value: &Value) -> Option<JiraTransition> {
    let obj = value.as_object()?;
    Some(JiraTransition {
        id: obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        name: obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        to_status: obj.get("to").and_then(parse_jira_status)?,
    })
}

pub fn parse_jira_transitions(value: &Value) -> Vec<JiraTransition> {
    value["transitions"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(parse_jira_transition)
        .collect()
}

fn extract_adf_text(adf: &Value) -> Option<String> {
    let mut texts = Vec::new();
    extract_adf_text_recursive(adf, &mut texts);
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

fn extract_adf_text_recursive(node: &Value, texts: &mut Vec<String>) {
    if let Some(obj) = node.as_object() {
        // Check for text nodes
        if obj.get("type").and_then(|v| v.as_str()) == Some("text")
            && let Some(text) = obj.get("text").and_then(|v| v.as_str())
        {
            texts.push(text.to_string());
        }

        // Recursively process content array
        if let Some(content) = obj.get("content").and_then(|v| v.as_array()) {
            for child in content {
                extract_adf_text_recursive(child, texts);
            }
        }
    }
}

// Re-export node implementations
pub use add_comment::AddJiraCommentNode;
pub use attachments::{
    DeleteAttachmentNode, DownloadAttachmentNode, GetAttachmentsNode, UploadAttachmentNode,
};
pub use create_issue::CreateJiraIssueNode;
pub use delete_issue::DeleteJiraIssueNode;
pub use get_issue::GetJiraIssueNode;
pub use get_transitions::GetJiraTransitionsNode;
pub use list_projects::ListJiraProjectsNode;
pub use search_issues::SearchJiraIssuesNode;
pub use transition_issue::TransitionJiraIssueNode;
pub use update_issue::UpdateJiraIssueNode;
