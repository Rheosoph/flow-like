use super::provider::{GOOGLE_PROVIDER_ID, GoogleProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct DriveFile {
    id: String,
    name: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
    #[serde(default)]
    size: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DriveFileList {
    files: Vec<DriveFile>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

/// Represents a Google Drive file returned by the Drive API.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleDriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<u64>,
    pub path: String,
}

#[crate::register_node]
#[derive(Default)]
pub struct ListGoogleDriveFilesNode {}

impl ListGoogleDriveFilesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGoogleDriveFilesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_drive_list_files",
            "List Google Drive Files",
            "Lists files from a Google Drive folder",
            "Data/Google/Drive",
        );
        node.set_flowscript_name("google.drive", "listFiles");
        node.add_icon("/flow/icons/google.svg");

        // Execution pins
        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the file listing",
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
            "folder_id",
            "Folder ID",
            "The ID of the folder to list files from. Use 'root' for the root folder.",
            VariableType::String,
        )
        .set_default_value(Some(json!("root")));

        node.add_input_pin(
            "query",
            "Search Query",
            "Optional search query to filter files (e.g., 'name contains \"report\"')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "page_size",
            "Page Size",
            "Maximum number of files to return (1-1000)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_input_pin(
            "include_folders",
            "Include Folders",
            "Whether to include folders in the results",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        // Output pins
        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when files are successfully listed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "files",
            "Files",
            "Array of Google Drive files",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GoogleDriveFile>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "file_count",
            "File Count",
            "Number of files returned",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.metadata.readonly"],
        );
        // Set scores
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(8)
                .set_performance(7)
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
        let folder_id: String = context.evaluate_pin("folder_id").await?;
        let query: String = context.evaluate_pin("query").await?;
        let page_size: i64 = context.evaluate_pin("page_size").await?;
        let include_folders: bool = context.evaluate_pin("include_folders").await?;

        // Build query string
        let mut q_parts = vec![format!("'{}' in parents", folder_id)];
        if !include_folders {
            q_parts.push("mimeType != 'application/vnd.google-apps.folder'".to_string());
        }
        if !query.is_empty() {
            q_parts.push(format!("({})", query));
        }
        let q = q_parts.join(" and ");

        // Make API request
        let client = reqwest::Client::new();
        let url = format!(
            "https://www.googleapis.com/drive/v3/files?q={}&pageSize={}&fields=files(id,name,mimeType,size),nextPageToken",
            urlencoding::encode(&q),
            page_size.clamp(1, 1000)
        );

        context.log_message(
            &format!("Fetching Google Drive files: {}", url),
            LogLevel::Debug,
        );

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("Google Drive API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let file_list: DriveFileList = resp.json().await.map_err(|e| {
                    context
                        .log_message(&format!("Failed to parse response: {}", e), LogLevel::Error);
                    flow_like_types::anyhow!("Failed to parse Google Drive response")
                })?;

                // Convert to GoogleDriveFile format
                let files: Vec<GoogleDriveFile> = file_list
                    .files
                    .into_iter()
                    .map(|f| GoogleDriveFile {
                        id: f.id.clone(),
                        name: f.name.clone(),
                        mime_type: f.mime_type,
                        size: f.size.and_then(|s| s.parse().ok()),
                        path: format!("gdrive://{}", f.id),
                    })
                    .collect();

                let file_count = files.len() as i64;

                context.log_message(
                    &format!("Found {} files in Google Drive", file_count),
                    LogLevel::Info,
                );

                context.set_pin_value("files", json!(files)).await?;
                context
                    .set_pin_value("file_count", json!(file_count))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
