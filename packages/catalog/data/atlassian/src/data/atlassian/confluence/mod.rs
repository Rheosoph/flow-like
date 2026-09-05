pub mod add_comment;
pub mod attachments;
pub mod children;
pub mod create_page;
pub mod delete_page;
pub mod get_comments;
pub mod get_page;
pub mod labels;
pub mod list_spaces;
pub mod search_content;
pub mod update_page;
pub mod users;

use flow_like_types::{JsonSchema, Value};
use serde::{Deserialize, Serialize};

// =============================================================================
// Confluence Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfluenceUser {
    /// User account ID
    pub account_id: String,
    /// User display name
    pub display_name: String,
    /// User email (if available)
    pub email: Option<String>,
    /// Profile picture URL
    pub profile_picture_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfluenceSpace {
    /// Space ID
    pub id: String,
    /// Space key (e.g., "TEAM")
    pub key: String,
    /// Space name
    pub name: String,
    /// Space type (global, personal)
    pub space_type: String,
    /// Space description
    pub description: Option<String>,
    /// Space homepage ID
    pub homepage_id: Option<String>,
    /// Space status (current, archived)
    pub status: String,
    /// Web URL to the space
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfluenceContentBody {
    /// Body in storage format (HTML-like)
    pub storage: Option<String>,
    /// Body in view format (rendered HTML)
    pub view: Option<String>,
    /// Body in plain text (for Confluence Cloud API v2)
    pub atlas_doc_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfluencePage {
    /// Page ID
    pub id: String,
    /// Page title
    pub title: String,
    /// Space key
    pub space_key: String,
    /// Page status (current, trashed, draft)
    pub status: String,
    /// Page body content
    pub body: Option<ConfluenceContentBody>,
    /// Parent page ID (if any)
    pub parent_id: Option<String>,
    /// Page version number
    pub version: i64,
    /// Page author
    pub author: Option<ConfluenceUser>,
    /// Creation timestamp
    pub created_at: String,
    /// Last update timestamp
    pub updated_at: String,
    /// Web URL to the page
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfluenceContent {
    /// Content ID
    pub id: String,
    /// Content type (page, blogpost, comment, attachment)
    pub content_type: String,
    /// Content title
    pub title: String,
    /// Space key
    pub space_key: Option<String>,
    /// Content status
    pub status: String,
    /// Web URL to the content
    pub url: String,
    /// Excerpt/snippet (for search results)
    pub excerpt: Option<String>,
    /// Last update timestamp
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfluenceSearchResult {
    /// Search results
    pub results: Vec<ConfluenceContent>,
    /// Total number of matching results
    pub total: i64,
    /// Starting index of results
    pub start: i64,
    /// Limit of results returned
    pub limit: i64,
}

// =============================================================================
// Helper functions for parsing Confluence API responses
// =============================================================================

pub fn parse_confluence_user(value: &Value) -> Option<ConfluenceUser> {
    let obj = value.as_object()?;
    Some(ConfluenceUser {
        account_id: obj
            .get("accountId")
            .or_else(|| obj.get("publicName"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        display_name: obj
            .get("displayName")
            .or_else(|| obj.get("publicName"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        email: obj.get("email").and_then(|v| v.as_str()).map(String::from),
        profile_picture_url: obj
            .get("profilePicture")
            .and_then(|v| v.get("path"))
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

pub fn parse_confluence_space(value: &Value, base_url: &str) -> Option<ConfluenceSpace> {
    let obj = value.as_object()?;

    // Handle both v1 and v2 API responses
    let id = obj
        .get("id")
        .and_then(|v| v.as_str().or_else(|| v.as_i64().map(|_| "")))
        .map(String::from)
        .or_else(|| {
            obj.get("id")
                .and_then(|v| v.as_i64())
                .map(|i| i.to_string())
        })
        .unwrap_or_default();

    let key = obj
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let url = obj
        .get("_links")
        .and_then(|l| l.get("webui"))
        .and_then(|v| v.as_str())
        .map(|path| format!("{}/wiki{}", base_url.trim_end_matches('/'), path));

    Some(ConfluenceSpace {
        id,
        key: key.clone(),
        name: obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        space_type: obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("global")
            .to_string(),
        description: obj
            .get("description")
            .and_then(|d| d.get("plain"))
            .and_then(|p| p.get("value"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                obj.get("description")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            }),
        homepage_id: obj
            .get("homepage")
            .and_then(|h| h.get("id"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                obj.get("homepageId")
                    .and_then(|v| v.as_i64())
                    .map(|i| i.to_string())
            }),
        status: obj
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("current")
            .to_string(),
        url,
    })
}

pub fn parse_confluence_page(value: &Value, base_url: &str) -> Option<ConfluencePage> {
    let obj = value.as_object()?;

    let id = obj
        .get("id")
        .and_then(|v| v.as_str().map(String::from))
        .or_else(|| {
            obj.get("id")
                .and_then(|v| v.as_i64())
                .map(|i| i.to_string())
        })
        .unwrap_or_default();

    let space_key = obj
        .get("spaceId")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            obj.get("space")
                .and_then(|s| s.get("key"))
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .unwrap_or_default();

    // Parse body content
    let body = parse_content_body(obj.get("body"));

    // Build URL
    let url = obj
        .get("_links")
        .and_then(|l| l.get("webui"))
        .and_then(|v| v.as_str())
        .map(|path| format!("{}/wiki{}", base_url.trim_end_matches('/'), path))
        .unwrap_or_else(|| {
            format!(
                "{}/wiki/spaces/{}/pages/{}",
                base_url.trim_end_matches('/'),
                space_key,
                id
            )
        });

    // Parse version (v1 vs v2 API)
    let version = obj
        .get("version")
        .and_then(|v| {
            v.get("number")
                .and_then(|n| n.as_i64())
                .or_else(|| v.as_i64())
        })
        .unwrap_or(1);

    // Parse dates
    let created_at = obj
        .get("createdAt")
        .or_else(|| obj.get("history").and_then(|h| h.get("createdDate")))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let updated_at = obj
        .get("version")
        .and_then(|v| v.get("when").or_else(|| v.get("createdAt")))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    Some(ConfluencePage {
        id,
        title: obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        space_key,
        status: obj
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("current")
            .to_string(),
        body,
        parent_id: obj
            .get("parentId")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                obj.get("ancestors")
                    .and_then(|a| a.as_array())
                    .and_then(|arr| arr.last())
                    .and_then(|p| p.get("id"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            }),
        version,
        author: obj
            .get("version")
            .and_then(|v| v.get("by"))
            .and_then(parse_confluence_user)
            .or_else(|| obj.get("ownerId").and_then(parse_confluence_user)),
        created_at,
        updated_at,
        url,
    })
}

fn parse_content_body(body_value: Option<&Value>) -> Option<ConfluenceContentBody> {
    let body = body_value?;
    let obj = body.as_object();

    Some(ConfluenceContentBody {
        storage: obj
            .and_then(|o| o.get("storage"))
            .and_then(|s| s.get("value").or(Some(s)))
            .and_then(|v| v.as_str())
            .map(String::from),
        view: obj
            .and_then(|o| o.get("view"))
            .and_then(|s| s.get("value").or(Some(s)))
            .and_then(|v| v.as_str())
            .map(String::from),
        atlas_doc_format: obj
            .and_then(|o| o.get("atlas_doc_format"))
            .and_then(|s| s.get("value").or(Some(s)))
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

pub fn parse_confluence_content(value: &Value, base_url: &str) -> Option<ConfluenceContent> {
    let obj = value.as_object()?;

    let id = obj
        .get("id")
        .and_then(|v| v.as_str().map(String::from))
        .or_else(|| {
            obj.get("id")
                .and_then(|v| v.as_i64())
                .map(|i| i.to_string())
        })
        .unwrap_or_default();

    let content_type = obj
        .get("type")
        .or_else(|| obj.get("contentType"))
        .and_then(|v| v.as_str())
        .unwrap_or("page")
        .to_string();

    let space_key = obj
        .get("space")
        .and_then(|s| s.get("key"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            obj.get("spaceId")
                .and_then(|v| v.as_str())
                .map(String::from)
        });

    let url = obj
        .get("_links")
        .and_then(|l| l.get("webui"))
        .and_then(|v| v.as_str())
        .map(|path| format!("{}/wiki{}", base_url.trim_end_matches('/'), path))
        .unwrap_or_else(|| {
            format!(
                "{}/wiki/pages/viewpage.action?pageId={}",
                base_url.trim_end_matches('/'),
                id
            )
        });

    Some(ConfluenceContent {
        id,
        content_type,
        title: obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        space_key,
        status: obj
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("current")
            .to_string(),
        url,
        excerpt: obj
            .get("excerpt")
            .and_then(|v| v.as_str())
            .map(String::from),
        updated_at: obj
            .get("version")
            .and_then(|v| v.get("when"))
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

// Re-export node implementations
pub use add_comment::AddConfluenceCommentNode;
pub use attachments::{
    DeleteConfluenceAttachmentNode, DownloadConfluenceAttachmentNode,
    ListConfluenceAttachmentsNode, UploadConfluenceAttachmentNode,
};
pub use create_page::CreateConfluencePageNode;
pub use delete_page::DeleteConfluencePageNode;
pub use get_comments::GetConfluenceCommentsNode;
pub use get_page::GetConfluencePageNode;
pub use list_spaces::ListConfluenceSpacesNode;
pub use search_content::SearchConfluenceContentNode;
pub use update_page::UpdateConfluencePageNode;
