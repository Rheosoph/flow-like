pub mod confluence;
pub mod jira;
pub mod me;
pub mod provider;

// Re-export types for external use
pub use confluence::{
    ConfluenceContent, ConfluenceContentBody, ConfluencePage, ConfluenceSearchResult,
    ConfluenceSpace, ConfluenceUser,
};
pub use jira::{
    JiraComment, JiraIssue, JiraIssueType, JiraPriority, JiraProject, JiraSearchResult, JiraStatus,
    JiraTransition, JiraUser,
};
pub use me::AtlassianMe;
pub use provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
