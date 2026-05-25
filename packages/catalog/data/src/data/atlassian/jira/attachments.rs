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

use super::JiraUser;

/// Jira attachment
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraAttachment {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub content_url: String,
    pub author: Option<JiraUser>,
    pub created: String,
}

fn parse_attachment(value: &Value) -> Option<JiraAttachment> {
    let obj = value.as_object()?;

    Some(JiraAttachment {
        id: obj.get("id")?.as_str()?.to_string(),
        filename: obj.get("filename")?.as_str()?.to_string(),
        mime_type: obj
            .get("mimeType")
            .and_then(|m| m.as_str())
            .unwrap_or("application/octet-stream")
            .to_string(),
        size: obj.get("size").and_then(|s| s.as_i64()).unwrap_or(0),
        content_url: obj
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string(),
        author: obj.get("author").and_then(super::parse_jira_user),
        created: obj
            .get("created")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

fn filename_from_flow_path(path: &FlowPath, fallback: &str) -> String {
    Path::new(&path.path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// Get attachments for an issue
#[crate::register_node]
#[derive(Default)]
pub struct GetAttachmentsNode {}

impl GetAttachmentsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetAttachmentsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_attachments",
            "Get Attachments",
            "Get all attachments for a Jira issue",
            "Data/Atlassian/Jira",
        );
        node.add_icon("/flow/icons/jira.svg");
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
            "issue_key",
            "Issue Key",
            "The issue key (e.g., PROJ-123)",
            VariableType::String,
        );

        node.add_output_pin(
            "attachments",
            "Attachments",
            "List of attachments",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraAttachment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of attachments",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:jira-work"]);
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
        let issue_key: String = context.evaluate_pin("issue_key").await?;

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/issue/{}?fields=attachment", issue_key));

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get attachments: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let attachments: Vec<JiraAttachment> = data["fields"]["attachment"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(parse_attachment)
            .collect();

        let count = attachments.len() as i64;

        context
            .set_pin_value("attachments", json!(attachments))
            .await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Upload an attachment to an issue
#[crate::register_node]
#[derive(Default)]
pub struct UploadAttachmentNode {}

impl UploadAttachmentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UploadAttachmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_upload_attachment",
            "Upload Attachment",
            "Upload a file attachment to a Jira issue",
            "Data/Atlassian/Jira",
        );
        node.add_icon("/flow/icons/jira.svg");
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
            "issue_key",
            "Issue Key",
            "The issue key (e.g., PROJ-123)",
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

        node.add_output_pin(
            "attachments",
            "Attachments",
            "Created Jira attachments",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraAttachment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of attachments created",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:jira-work"]);
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
        let issue_key: String = context.evaluate_pin("issue_key").await?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let filename: String = context.evaluate_pin("filename").await.unwrap_or_default();

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        let bytes = file.get(context, false).await?;
        let filename = if filename.is_empty() {
            filename_from_flow_path(&file, "attachment.bin")
        } else {
            filename
        };

        let part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
        let form = reqwest::multipart::Form::new().part("file", part);

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/issue/{}/attachments", issue_key));

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
        let attachments: Vec<JiraAttachment> = data
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(parse_attachment)
            .collect();
        let count = attachments.len() as i64;

        context
            .set_pin_value("attachments", json!(attachments))
            .await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Download an attachment's content
#[crate::register_node]
#[derive(Default)]
pub struct DownloadAttachmentNode {}

impl DownloadAttachmentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DownloadAttachmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_download_attachment",
            "Download Attachment",
            "Download the content of an attachment",
            "Data/Atlassian/Jira",
        );
        node.add_icon("/flow/icons/jira.svg");
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
            "The ID of the attachment to download",
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
            "size",
            "Size",
            "Size of the downloaded content in bytes",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
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
        let url = provider.jira_api_url(&format!("/attachment/content/{}", attachment_id));

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .query(&[("redirect", "false")])
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
        context.set_pin_value("size", json!(size)).await?;

        Ok(())
    }
}

/// Delete an attachment
#[crate::register_node]
#[derive(Default)]
pub struct DeleteAttachmentNode {}

impl DeleteAttachmentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteAttachmentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_delete_attachment",
            "Delete Attachment",
            "Delete an attachment from an issue",
            "Data/Atlassian/Jira",
        );
        node.add_icon("/flow/icons/jira.svg");
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
            "The ID of the attachment to delete",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the deletion was successful",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:jira-work"]);
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
        let url = provider.jira_api_url(&format!("/attachment/{}", attachment_id));

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
