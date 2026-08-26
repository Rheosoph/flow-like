use super::provider::{NOTION_PROVIDER_ID, NotionProvider};
use super::utils::{
    NOTION_API_VERSION, auth_header, content_type_from_filename, file_object_from_upload_id,
    filename_from_path, log_and_error, notion_error,
};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

const SINGLE_PART_LIMIT: usize = 20 * 1024 * 1024;
const MAX_MULTIPART_PARTS: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotionFileUpload {
    pub id: String,
    pub status: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub content_length: Option<i64>,
    pub expiry_time: Option<String>,
    pub raw: Value,
}

fn parse_file_upload(value: &Value) -> Option<NotionFileUpload> {
    Some(NotionFileUpload {
        id: value["id"].as_str()?.to_string(),
        status: value["status"].as_str().unwrap_or("").to_string(),
        filename: value["filename"].as_str().map(String::from),
        content_type: value["content_type"].as_str().map(String::from),
        content_length: value["content_length"].as_i64(),
        expiry_time: value["expiry_time"].as_str().map(String::from),
        raw: value.clone(),
    })
}

async fn parse_response_json(
    context: &mut ExecutionContext,
    response: Result<reqwest::Response, reqwest::Error>,
) -> flow_like_types::Result<Option<Value>> {
    match response {
        Ok(resp) => {
            if !resp.status().is_success() {
                context.log_message(&notion_error(resp).await, LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(None);
            }

            Ok(Some(resp.json::<Value>().await.map_err(|err| {
                flow_like_types::anyhow!("Failed to parse response: {}", err)
            })?))
        }
        Err(err) => {
            log_and_error(context, format!("Network error: {}", err)).await?;
            Ok(None)
        }
    }
}

fn upload_file_name(file: &FlowPath, filename: String) -> String {
    if filename.is_empty() {
        filename_from_path(&file.path).unwrap_or_else(|| "notion-upload.bin".to_string())
    } else {
        filename
    }
}

fn upload_content_type(filename: &str, content_type: String) -> String {
    if content_type.is_empty() {
        content_type_from_filename(filename)
    } else {
        content_type
    }
}

fn file_part(
    bytes: Vec<u8>,
    filename: &str,
    content_type: &str,
) -> flow_like_types::Result<reqwest::multipart::Part> {
    reqwest::multipart::Part::bytes(bytes)
        .file_name(filename.to_string())
        .mime_str(content_type)
        .map_err(|err| flow_like_types::anyhow!("Invalid content type {}: {}", content_type, err))
}

fn extract_download_url(file_url: &str, file_object: &Value) -> Option<String> {
    let file_url = file_url.trim();
    if !file_url.is_empty() {
        return Some(file_url.to_string());
    }

    file_object["file"]["url"]
        .as_str()
        .or_else(|| file_object["external"]["url"].as_str())
        .or_else(|| file_object["url"].as_str())
        .map(String::from)
}

fn add_standard_scores(node: &mut Node) {
    node.add_required_oauth_scopes(NOTION_PROVIDER_ID, vec![]);
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
}

#[crate::register_node]
#[derive(Default)]
pub struct UploadNotionFileNode {}

impl UploadNotionFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UploadNotionFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_upload_file",
            "Upload Notion File",
            "Uploads a FlowPath file to Notion and returns a file_upload object",
            "Data/Notion",
        );
        node.set_flowscript_name("notion", "uploadFile");
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Notion provider (from Notion node)",
            VariableType::Struct,
        )
        .set_schema::<NotionProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file",
            "File",
            "FlowPath file to upload",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "filename",
            "Filename",
            "Notion filename. Uses the FlowPath filename when empty.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "content_type",
            "Content Type",
            "MIME type. Inferred from the filename when empty.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );
        node.add_output_pin(
            "file_upload",
            "File Upload",
            "Notion file_upload object",
            VariableType::Struct,
        )
        .set_schema::<NotionFileUpload>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "file_upload_id",
            "File Upload ID",
            "Notion file upload ID",
            VariableType::String,
        );
        node.add_output_pin(
            "file_object",
            "File Object",
            "Notion property/block file object referencing this upload",
            VariableType::Struct,
        )
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());
        node.add_output_pin(
            "size",
            "Size",
            "Uploaded file size in bytes",
            VariableType::Integer,
        );

        add_standard_scores(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: NotionProvider = context.evaluate_pin("provider").await?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let filename: String = context.evaluate_pin("filename").await.unwrap_or_default();
        let content_type: String = context
            .evaluate_pin("content_type")
            .await
            .unwrap_or_default();

        let bytes = file.get(context, false).await?;
        if bytes.is_empty() {
            log_and_error(context, "File cannot be empty").await?;
            return Ok(());
        }

        let filename = upload_file_name(&file, filename);
        let content_type = upload_content_type(&filename, content_type);
        let size = bytes.len() as i64;
        let part_count = bytes.len().div_ceil(SINGLE_PART_LIMIT);
        if part_count > MAX_MULTIPART_PARTS {
            log_and_error(
                context,
                format!(
                    "File is too large for Notion multipart upload: {} parts required",
                    part_count
                ),
            )
            .await?;
            return Ok(());
        }

        let multi_part = bytes.len() > SINGLE_PART_LIMIT;
        let mode = if multi_part {
            "multi_part"
        } else {
            "single_part"
        };
        let mut create_body = json!({
            "mode": mode,
            "filename": filename.clone(),
            "content_type": content_type.clone()
        });
        if multi_part {
            create_body["number_of_parts"] = json!(part_count);
        }

        let client = reqwest::Client::new();
        let upload = parse_response_json(
            context,
            client
                .post("https://api.notion.com/v1/file_uploads")
                .header("Authorization", auth_header(&provider.access_token))
                .header("Notion-Version", NOTION_API_VERSION)
                .header("Content-Type", "application/json")
                .json(&create_body)
                .send()
                .await,
        )
        .await?;
        let Some(upload) = upload else {
            return Ok(());
        };
        let upload_id = upload["id"].as_str().unwrap_or("").to_string();
        if upload_id.is_empty() {
            log_and_error(context, "Notion did not return a file upload ID").await?;
            return Ok(());
        }

        let mut final_upload = upload.clone();
        if multi_part {
            for (index, chunk) in bytes.chunks(SINGLE_PART_LIMIT).enumerate() {
                let part = match file_part(chunk.to_vec(), &filename, &content_type) {
                    Ok(part) => part,
                    Err(err) => {
                        log_and_error(context, err.to_string()).await?;
                        return Ok(());
                    }
                };
                let form = reqwest::multipart::Form::new()
                    .text("part_number", (index + 1).to_string())
                    .part("file", part);
                let send_url = format!("https://api.notion.com/v1/file_uploads/{}/send", upload_id);
                if parse_response_json(
                    context,
                    client
                        .post(&send_url)
                        .header("Authorization", auth_header(&provider.access_token))
                        .header("Notion-Version", NOTION_API_VERSION)
                        .multipart(form)
                        .send()
                        .await,
                )
                .await?
                .is_none()
                {
                    return Ok(());
                }
            }

            let complete_url = format!(
                "https://api.notion.com/v1/file_uploads/{}/complete",
                upload_id
            );
            if let Some(completed) = parse_response_json(
                context,
                client
                    .post(&complete_url)
                    .header("Authorization", auth_header(&provider.access_token))
                    .header("Notion-Version", NOTION_API_VERSION)
                    .send()
                    .await,
            )
            .await?
            {
                final_upload = completed;
            }
        } else {
            let part = match file_part(bytes, &filename, &content_type) {
                Ok(part) => part,
                Err(err) => {
                    log_and_error(context, err.to_string()).await?;
                    return Ok(());
                }
            };
            let form = reqwest::multipart::Form::new().part("file", part);
            let send_url = format!("https://api.notion.com/v1/file_uploads/{}/send", upload_id);
            if let Some(sent) = parse_response_json(
                context,
                client
                    .post(&send_url)
                    .header("Authorization", auth_header(&provider.access_token))
                    .header("Notion-Version", NOTION_API_VERSION)
                    .multipart(form)
                    .send()
                    .await,
            )
            .await?
            {
                final_upload = sent;
            }
        }

        let file_upload = parse_file_upload(&final_upload)
            .or_else(|| parse_file_upload(&upload))
            .ok_or_else(|| flow_like_types::anyhow!("Failed to parse Notion file upload"))?;
        let file_object = file_object_from_upload_id(&file_upload.id);

        context
            .set_pin_value("file_upload", json!(file_upload.clone()))
            .await?;
        context
            .set_pin_value("file_upload_id", json!(file_upload.id))
            .await?;
        context.set_pin_value("file_object", file_object).await?;
        context.set_pin_value("size", json!(size)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DownloadNotionFileNode {}

impl DownloadNotionFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DownloadNotionFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_download_file",
            "Download Notion File",
            "Downloads a Notion file URL into a FlowPath",
            "Data/Notion",
        );
        node.set_flowscript_name("notion", "downloadFile");
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "file_url",
            "File URL",
            "Signed Notion file URL. If empty, File Object is used.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "file_object",
            "File Object",
            "Notion file object containing file.url, external.url, or url",
            VariableType::Struct,
        )
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!(null)));
        node.add_input_pin(
            "output_path",
            "Output Path",
            "FlowPath to write the downloaded file into",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );
        node.add_output_pin("path", "Path", "Written FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "size",
            "Size",
            "Downloaded file size in bytes",
            VariableType::Integer,
        );
        node.add_output_pin(
            "content_type",
            "Content Type",
            "Response content type",
            VariableType::String,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(7)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let file_url: String = context.evaluate_pin("file_url").await.unwrap_or_default();
        let file_object: Value = context
            .evaluate_pin("file_object")
            .await
            .unwrap_or(json!(null));
        let output_path: FlowPath = context.evaluate_pin("output_path").await?;

        let Some(url) = extract_download_url(&file_url, &file_object) else {
            log_and_error(
                context,
                "File URL is required. file_upload objects cannot be downloaded directly without a signed Notion file URL.",
            )
            .await?;
            return Ok(());
        };

        let client = reqwest::Client::new();
        let response = client.get(&url).send().await;
        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    log_and_error(
                        context,
                        format!("Failed to download Notion file {}: {}", status, error_text),
                    )
                    .await?;
                    return Ok(());
                }

                let content_type = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let bytes = resp.bytes().await.map_err(|err| {
                    flow_like_types::anyhow!("Failed to read Notion file response: {}", err)
                })?;
                let size = bytes.len() as i64;
                output_path.put(context, bytes.to_vec(), false).await?;

                context.set_pin_value("path", json!(output_path)).await?;
                context.set_pin_value("size", json!(size)).await?;
                context
                    .set_pin_value("content_type", json!(content_type))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(err) => {
                log_and_error(context, format!("Network error: {}", err)).await?;
            }
        }

        Ok(())
    }
}
