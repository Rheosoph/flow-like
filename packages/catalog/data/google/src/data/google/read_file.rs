use super::provider::{GOOGLE_PROVIDER_ID, GoogleProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

#[crate::register_node]
#[derive(Default)]
pub struct ReadGoogleDriveFileNode {}

impl ReadGoogleDriveFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ReadGoogleDriveFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_drive_read_file",
            "Read Google Drive File",
            "Reads the content of a file from Google Drive as text",
            "Data/Google/Drive",
        );
        node.set_flowscript_name("google.drive", "readFile");
        node.add_icon("/flow/icons/google.svg");

        // Execution pins
        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger file read",
            VariableType::Execution,
        );

        // Provider input - must connect to Google Drive provider node
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider (from Google Drive node)",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        // Input pins
        node.add_input_pin(
            "file_id",
            "File ID",
            "The ID of the file to read (from Google Drive)",
            VariableType::String,
        );

        node.add_input_pin(
            "export_mime_type",
            "Export MIME Type",
            "For Google Docs files, specify the export format (e.g., 'text/plain', 'application/pdf'). Leave empty for regular files.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        // Output pins
        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when file is successfully read",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "content",
            "Content",
            "The text content of the file",
            VariableType::String,
        );

        node.add_output_pin(
            "file_name",
            "File Name",
            "The name of the file",
            VariableType::String,
        );

        node.add_output_pin(
            "mime_type",
            "MIME Type",
            "The MIME type of the file",
            VariableType::String,
        );

        node.add_output_pin(
            "size",
            "Size (bytes)",
            "The size of the file in bytes",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.file"],
        );
        // Set scores
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(8)
                .set_performance(6) // Download can be slow for large files
                .set_governance(6)
                .set_reliability(9)
                .set_cost(5)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        // Get provider from input
        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let access_token = provider.access_token;

        // Get input values
        let file_id: String = context.evaluate_pin("file_id").await?;
        let export_mime_type: String = context.evaluate_pin("export_mime_type").await?;

        if file_id.is_empty() {
            context.log_message("File ID cannot be empty", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();

        // First, get file metadata
        let metadata_url = format!(
            "https://www.googleapis.com/drive/v3/files/{}?fields=id,name,mimeType,size",
            file_id
        );

        let metadata_response = client
            .get(&metadata_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await;

        let (file_name, mime_type, _size) = match metadata_response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("Failed to get file metadata {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let metadata: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse metadata: {}", e))?;

                (
                    metadata["name"].as_str().unwrap_or("unknown").to_string(),
                    metadata["mimeType"].as_str().unwrap_or("").to_string(),
                    metadata["size"]
                        .as_str()
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(0),
                )
            }
            Err(e) => {
                context.log_message(
                    &format!("Network error getting metadata: {}", e),
                    LogLevel::Error,
                );
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        // Determine if this is a Google Docs file that needs export
        let is_google_doc = mime_type.starts_with("application/vnd.google-apps.");

        // Download the file content
        let content_url = if is_google_doc {
            let export_type = if export_mime_type.is_empty() {
                // Default export types for common Google Docs formats
                match mime_type.as_str() {
                    "application/vnd.google-apps.document" => "text/plain",
                    "application/vnd.google-apps.spreadsheet" => "text/csv",
                    "application/vnd.google-apps.presentation" => "text/plain",
                    _ => "text/plain",
                }
            } else {
                &export_mime_type
            };

            format!(
                "https://www.googleapis.com/drive/v3/files/{}/export?mimeType={}",
                file_id,
                urlencoding::encode(export_type)
            )
        } else {
            format!(
                "https://www.googleapis.com/drive/v3/files/{}?alt=media",
                file_id
            )
        };

        context.log_message(
            &format!("Downloading file: {} ({})", file_name, mime_type),
            LogLevel::Debug,
        );

        let content_response = client
            .get(&content_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await;

        match content_response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("Failed to download file {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let content = resp
                    .text()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to read file content: {}", e))?;

                let content_size = content.len() as i64;

                context.log_message(
                    &format!(
                        "Successfully read {} bytes from {}",
                        content_size, file_name
                    ),
                    LogLevel::Info,
                );

                context.set_pin_value("content", json!(content)).await?;
                context.set_pin_value("file_name", json!(file_name)).await?;
                context.set_pin_value("mime_type", json!(mime_type)).await?;
                context.set_pin_value("size", json!(content_size)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(
                    &format!("Network error downloading file: {}", e),
                    LogLevel::Error,
                );
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
