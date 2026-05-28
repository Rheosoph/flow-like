use crate::data::{
    atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider},
    path::FlowPath,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Confluence attachment
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfluenceAttachment {
    pub id: String,
    pub title: String,
    pub media_type: String,
    pub file_size: i64,
    pub download_url: String,
    pub created_at: Option<String>,
}

fn filename_from_flow_path(path: &FlowPath, fallback: &str) -> String {
    Path::new(&path.path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn parse_attachment(value: &Value, provider: &AtlassianProvider) -> Option<ConfluenceAttachment> {
    let obj = value.as_object()?;
    let links = obj.get("_links");
    let raw_download = links
        .and_then(|l| l.get("download"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let download_url =
        if raw_download.starts_with("http://") || raw_download.starts_with("https://") {
            raw_download.to_string()
        } else if raw_download.is_empty() {
            String::new()
        } else {
            provider.confluence_wiki_url(raw_download)
        };

    Some(ConfluenceAttachment {
        id: obj
            .get("id")
            .and_then(|v| v.as_str().map(String::from))
            .or_else(|| {
                obj.get("id")
                    .and_then(|v| v.as_i64())
                    .map(|i| i.to_string())
            })
            .unwrap_or_default(),
        title: obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        media_type: obj
            .get("metadata")
            .and_then(|m| m.get("mediaType"))
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream")
            .to_string(),
        file_size: obj
            .get("extensions")
            .and_then(|e| e.get("fileSize"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        download_url,
        created_at: obj
            .get("version")
            .and_then(|v| v.get("when"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                obj.get("history")
                    .and_then(|h| h.get("createdDate"))
                    .and_then(|v| v.as_str())
            })
            .map(String::from),
    })
}

/// List attachments for a page
#[crate::register_node]
#[derive(Default)]
pub struct ListConfluenceAttachmentsNode {}

impl ListConfluenceAttachmentsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListConfluenceAttachmentsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_list_attachments",
            "List Attachments",
            "List attachments on a Confluence page",
            "Data/Atlassian/Confluence",
        );
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "page_id",
            "Page ID",
            "The page ID to list attachments for",
            VariableType::String,
        );

        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of attachments to return",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50)));

        node.add_output_pin(
            "attachments",
            "Attachments",
            "List of attachments",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<ConfluenceAttachment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of attachments",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:confluence-content.all"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(50);

        if page_id.is_empty() {
            return Err(flow_like_types::anyhow!("Page ID is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.confluence_rest_api_url(&format!(
            "/content/{}/child/attachment?limit={}&expand=version",
            page_id,
            limit.clamp(1, 100)
        ));

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to list attachments: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let attachments: Vec<ConfluenceAttachment> = data["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|a| parse_attachment(a, &provider))
            .collect();
        let count = attachments.len() as i64;

        context
            .set_pin_value("attachments", json!(attachments))
            .await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Upload an attachment to a page
#[crate::register_node]
#[derive(Default)]
pub struct UploadConfluenceAttachmentNode {}

impl UploadConfluenceAttachmentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UploadConfluenceAttachmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_upload_attachment",
            "Upload Attachment",
            "Upload a file attachment to a Confluence page",
            "Data/Atlassian/Confluence",
        );
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "page_id",
            "Page ID",
            "The page ID to upload the attachment to",
            VariableType::String,
        );

        node.add_input_pin("file", "File", "File to upload", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "filename",
            "Filename",
            "Override file name for the uploaded attachment (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "comment",
            "Comment",
            "Attachment version comment (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "attachments",
            "Attachments",
            "Created or updated attachments",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<ConfluenceAttachment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of attachments",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:confluence-content"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(5)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(7)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let filename: String = context.evaluate_pin("filename").await.unwrap_or_default();
        let comment: String = context.evaluate_pin("comment").await.unwrap_or_default();

        if page_id.is_empty() {
            return Err(flow_like_types::anyhow!("Page ID is required"));
        }

        let bytes = file.get(context, false).await?;
        let filename = if filename.is_empty() {
            filename_from_flow_path(&file, "attachment.bin")
        } else {
            filename
        };

        let part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
        let mut form = reqwest::multipart::Form::new().part("file", part);
        if !comment.is_empty() {
            form = form.text("comment", comment);
        }

        let client = reqwest::Client::new();
        let url =
            provider.confluence_rest_api_url(&format!("/content/{}/child/attachment", page_id));

        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("X-Atlassian-Token", "no-check")
            .header("Accept", "application/json")
            .multipart(form)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to upload attachment: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let attachments: Vec<ConfluenceAttachment> = data["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|a| parse_attachment(a, &provider))
            .collect();
        let count = attachments.len() as i64;

        context
            .set_pin_value("attachments", json!(attachments))
            .await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Download an attachment to a FlowPath
#[crate::register_node]
#[derive(Default)]
pub struct DownloadConfluenceAttachmentNode {}

impl DownloadConfluenceAttachmentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DownloadConfluenceAttachmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_download_attachment",
            "Download Attachment",
            "Download a Confluence attachment to a FlowPath",
            "Data/Atlassian/Confluence",
        );
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "attachment_id",
            "Attachment ID",
            "The attachment content ID to download",
            VariableType::String,
        );

        node.add_input_pin(
            "output_path",
            "Output Path",
            "FlowPath to write the downloaded attachment into",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("path", "Path", "Written file path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "attachment",
            "Attachment",
            "Downloaded attachment metadata",
            VariableType::Struct,
        )
        .set_schema::<ConfluenceAttachment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("size", "Size", "Size in bytes", VariableType::Integer);

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:confluence-content.all"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(5)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(7)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let attachment_id: String = context.evaluate_pin("attachment_id").await?;
        let output_path: FlowPath = context.evaluate_pin("output_path").await?;

        if attachment_id.is_empty() {
            return Err(flow_like_types::anyhow!("Attachment ID is required"));
        }

        let client = reqwest::Client::new();
        let metadata_url =
            provider.confluence_rest_api_url(&format!("/content/{}?expand=version", attachment_id));

        let metadata_response = client
            .get(&metadata_url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/json")
            .send()
            .await?;

        if !metadata_response.status().is_success() {
            let status = metadata_response.status();
            let error_text = metadata_response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get attachment metadata: {} - {}",
                status,
                error_text
            ));
        }

        let metadata: Value = metadata_response.json().await?;
        let attachment = parse_attachment(&metadata, &provider)
            .ok_or_else(|| flow_like_types::anyhow!("Failed to parse attachment metadata"))?;

        if attachment.download_url.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Attachment metadata did not include a download URL"
            ));
        }

        let response = client
            .get(&attachment.download_url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to download attachment: {} - {}",
                status,
                error_text
            ));
        }

        let bytes = response.bytes().await?.to_vec();
        let size = bytes.len() as i64;
        output_path.put(context, bytes, false).await?;

        context.set_pin_value("path", json!(output_path)).await?;
        context
            .set_pin_value("attachment", json!(attachment))
            .await?;
        context.set_pin_value("size", json!(size)).await?;

        Ok(())
    }
}

/// Delete an attachment
#[crate::register_node]
#[derive(Default)]
pub struct DeleteConfluenceAttachmentNode {}

impl DeleteConfluenceAttachmentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteConfluenceAttachmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_delete_attachment",
            "Delete Attachment",
            "Delete a Confluence attachment",
            "Data/Atlassian/Confluence",
        );
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "attachment_id",
            "Attachment ID",
            "The attachment content ID to delete",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the attachment was deleted",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:confluence-content"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let attachment_id: String = context.evaluate_pin("attachment_id").await?;

        if attachment_id.is_empty() {
            return Err(flow_like_types::anyhow!("Attachment ID is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.confluence_rest_api_url(&format!("/content/{}", attachment_id));

        let response = client
            .delete(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to delete attachment: {} - {}",
                status,
                error_text
            ));
        }

        context.set_pin_value("success", json!(true)).await?;

        Ok(())
    }
}
