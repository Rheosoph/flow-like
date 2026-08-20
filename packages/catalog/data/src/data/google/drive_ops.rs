use super::provider::{GOOGLE_PROVIDER_ID, GoogleProvider};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_storage::Path;
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

const DRIVE_FILE_FIELDS: &str =
    "id,name,mimeType,size,createdTime,modifiedTime,webViewLink,parents";
const MULTIPART_UPLOAD_LIMIT_BYTES: u64 = 5 * 1024 * 1024;
const RESUMABLE_UPLOAD_CHUNK_SIZE_BYTES: u64 = 8 * 1024 * 1024;

// =============================================================================
// Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleDriveItem {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<i64>,
    pub created_time: Option<String>,
    pub modified_time: Option<String>,
    pub web_view_link: Option<String>,
    pub parents: Vec<String>,
    pub is_folder: bool,
}

fn parse_drive_item(item: &Value) -> Option<GoogleDriveItem> {
    let mime_type = item["mimeType"].as_str()?.to_string();
    let is_folder = mime_type == "application/vnd.google-apps.folder";
    Some(GoogleDriveItem {
        id: item["id"].as_str()?.to_string(),
        name: item["name"].as_str()?.to_string(),
        mime_type,
        size: item["size"].as_str().and_then(|s| s.parse().ok()),
        created_time: item["createdTime"].as_str().map(String::from),
        modified_time: item["modifiedTime"].as_str().map(String::from),
        web_view_link: item["webViewLink"].as_str().map(String::from),
        parents: item["parents"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        is_folder,
    })
}

async fn drive_error_message(resp: reqwest::Response) -> String {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if body.is_empty() {
        format!("Google Drive API error {}", status)
    } else {
        format!("Google Drive API error {}: {}", status, body)
    }
}

fn filename_from_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(String::from)
}

fn flow_path_filename(flow_path: &FlowPath) -> flow_like_types::Result<String> {
    filename_from_path(&flow_path.path).ok_or_else(|| {
        flow_like_types::anyhow!(
            "Destination name is empty and FlowPath has no filename: {}",
            flow_path.path
        )
    })
}

async fn flow_path_size(
    context: &mut ExecutionContext,
    flow_path: &FlowPath,
) -> flow_like_types::Result<u64> {
    let runtime = flow_path.to_runtime(context).await?;
    let meta = runtime
        .store
        .as_generic()
        .head(&Path::from(runtime.path.as_ref()))
        .await?;
    Ok(meta.size)
}

fn default_export_mime_type(mime_type: &str) -> &'static str {
    match mime_type {
        "application/vnd.google-apps.document" => "application/pdf",
        "application/vnd.google-apps.spreadsheet" => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        }
        "application/vnd.google-apps.presentation" => "application/pdf",
        _ => "application/pdf",
    }
}

fn upload_metadata(file_name: &str, parent_id: &str, mime_type: &str) -> Value {
    let mut metadata = json!({
        "name": file_name,
        "mimeType": mime_type
    });

    if !parent_id.trim().is_empty() {
        metadata["parents"] = json!([parent_id]);
    }

    metadata
}

async fn upload_with_multipart(
    context: &mut ExecutionContext,
    client: &reqwest::Client,
    provider: &GoogleProvider,
    source_file: &FlowPath,
    file_name: &str,
    parent_id: &str,
    mime_type: &str,
) -> flow_like_types::Result<(Value, u64)> {
    let bytes = source_file.get(context, false).await?;
    let size = bytes.len() as u64;
    let metadata =
        flow_like_types::json::to_vec(&upload_metadata(file_name, parent_id, mime_type))?;
    let boundary = format!("flow_like_{}", flow_like_types::create_id());

    let mut body = Vec::new();
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(&metadata);
    body.extend_from_slice(
        format!("\r\n--{boundary}\r\nContent-Type: {mime_type}\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(&bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let resp = client
        .post("https://www.googleapis.com/upload/drive/v3/files")
        .header("Authorization", format!("Bearer {}", provider.access_token))
        .header(
            "Content-Type",
            format!("multipart/related; boundary={boundary}"),
        )
        .query(&[("uploadType", "multipart"), ("fields", DRIVE_FILE_FIELDS)])
        .body(body)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(flow_like_types::anyhow!(drive_error_message(resp).await));
    }

    Ok((resp.json().await?, size))
}

#[allow(clippy::too_many_arguments)]
async fn upload_with_resumable(
    context: &mut ExecutionContext,
    client: &reqwest::Client,
    provider: &GoogleProvider,
    source_file: &FlowPath,
    file_name: &str,
    parent_id: &str,
    mime_type: &str,
    size: u64,
) -> flow_like_types::Result<Value> {
    let metadata = upload_metadata(file_name, parent_id, mime_type);
    let session_resp = client
        .post("https://www.googleapis.com/upload/drive/v3/files")
        .header("Authorization", format!("Bearer {}", provider.access_token))
        .header("Content-Type", "application/json; charset=UTF-8")
        .header("X-Upload-Content-Type", mime_type)
        .header("X-Upload-Content-Length", size.to_string())
        .query(&[("uploadType", "resumable"), ("fields", DRIVE_FILE_FIELDS)])
        .json(&metadata)
        .send()
        .await?;

    if !session_resp.status().is_success() {
        return Err(flow_like_types::anyhow!(
            drive_error_message(session_resp).await
        ));
    }

    let upload_url = session_resp
        .headers()
        .get("Location")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| flow_like_types::anyhow!("Google Drive upload session had no Location"))?
        .to_string();

    let runtime = source_file.to_runtime(context).await?;
    let store = runtime.store.as_generic();
    let source_path = Path::from(runtime.path.as_ref());
    let mut start = 0_u64;

    while start < size {
        let end_exclusive = std::cmp::min(start + RESUMABLE_UPLOAD_CHUNK_SIZE_BYTES, size);
        let end_inclusive = end_exclusive.saturating_sub(1);
        let bytes = store.get_range(&source_path, start..end_exclusive).await?;

        let resp = client
            .put(&upload_url)
            .header("Content-Type", mime_type)
            .header("Content-Length", bytes.len().to_string())
            .header(
                "Content-Range",
                format!("bytes {start}-{end_inclusive}/{size}"),
            )
            .body(bytes)
            .send()
            .await?;

        if resp.status().as_u16() == 308 {
            start = end_exclusive;
            continue;
        }

        if !resp.status().is_success() {
            return Err(flow_like_types::anyhow!(drive_error_message(resp).await));
        }

        return Ok(resp.json().await?);
    }

    Err(flow_like_types::anyhow!(
        "Google Drive resumable upload finished without a completion response"
    ))
}

async fn upload_flow_path_to_google_drive(
    context: &mut ExecutionContext,
    client: &reqwest::Client,
    provider: &GoogleProvider,
    source_file: &FlowPath,
    file_name: &str,
    parent_id: &str,
    mime_type: &str,
) -> flow_like_types::Result<(Value, bool, u64)> {
    let size = flow_path_size(context, source_file).await?;
    if size <= MULTIPART_UPLOAD_LIMIT_BYTES {
        let (item, size) = upload_with_multipart(
            context,
            client,
            provider,
            source_file,
            file_name,
            parent_id,
            mime_type,
        )
        .await?;
        return Ok((item, false, size));
    }

    let item = upload_with_resumable(
        context,
        client,
        provider,
        source_file,
        file_name,
        parent_id,
        mime_type,
        size,
    )
    .await?;

    Ok((item, true, size))
}

// =============================================================================
// Create Folder Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGoogleDriveFolderNode {}

impl CreateGoogleDriveFolderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGoogleDriveFolderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_drive_create_folder",
            "Create Folder",
            "Create a new folder in Google Drive",
            "Data/Google/Drive",
        );
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("name", "Name", "Folder name", VariableType::String);
        node.add_input_pin(
            "parent_id",
            "Parent ID",
            "Parent folder ID (empty for root)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "folder_id",
            "Folder ID",
            "Created folder ID",
            VariableType::String,
        );
        node.add_output_pin(
            "folder",
            "Folder",
            "Created folder details",
            VariableType::Struct,
        )
        .set_schema::<GoogleDriveItem>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.file"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let name: String = context.evaluate_pin("name").await?;
        let parent_id: String = context.evaluate_pin("parent_id").await.unwrap_or_default();

        let mut metadata = json!({
            "name": name,
            "mimeType": "application/vnd.google-apps.folder"
        });

        if !parent_id.is_empty() {
            metadata["parents"] = json!([parent_id]);
        }

        let client = reqwest::Client::new();
        let response = client
            .post("https://www.googleapis.com/drive/v3/files")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .query(&[(
                "fields",
                "id,name,mimeType,createdTime,modifiedTime,webViewLink,parents",
            )])
            .json(&metadata)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(folder) = parse_drive_item(&body) {
                    let folder_id = folder.id.clone();
                    context.set_pin_value("folder_id", json!(folder_id)).await?;
                    context.set_pin_value("folder", json!(folder)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Delete File Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteGoogleDriveFileNode {}

impl DeleteGoogleDriveFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteGoogleDriveFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_drive_delete_file",
            "Delete File",
            "Delete a file or folder from Google Drive",
            "Data/Google/Drive",
        );
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_id",
            "File ID",
            "ID of file/folder to delete",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.file"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let file_id: String = context.evaluate_pin("file_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .delete(format!(
                "https://www.googleapis.com/drive/v3/files/{}",
                file_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status() == 204 => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Copy File Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CopyGoogleDriveFileNode {}

impl CopyGoogleDriveFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CopyGoogleDriveFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_drive_copy_file",
            "Copy File",
            "Copy a file in Google Drive",
            "Data/Google/Drive",
        );
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_id",
            "File ID",
            "ID of file to copy",
            VariableType::String,
        );
        node.add_input_pin(
            "new_name",
            "New Name",
            "Name for the copy (empty to keep original)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "parent_id",
            "Parent ID",
            "Destination folder ID (empty for same location)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "new_file_id",
            "New File ID",
            "ID of the copied file",
            VariableType::String,
        );
        node.add_output_pin("file", "File", "Copied file details", VariableType::Struct)
            .set_schema::<GoogleDriveItem>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.file"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let file_id: String = context.evaluate_pin("file_id").await?;
        let new_name: String = context.evaluate_pin("new_name").await.unwrap_or_default();
        let parent_id: String = context.evaluate_pin("parent_id").await.unwrap_or_default();

        let mut metadata = json!({});
        if !new_name.is_empty() {
            metadata["name"] = json!(new_name);
        }
        if !parent_id.is_empty() {
            metadata["parents"] = json!([parent_id]);
        }

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://www.googleapis.com/drive/v3/files/{}/copy",
                file_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .query(&[(
                "fields",
                "id,name,mimeType,size,createdTime,modifiedTime,webViewLink,parents",
            )])
            .json(&metadata)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(file) = parse_drive_item(&body) {
                    let new_id = file.id.clone();
                    context.set_pin_value("new_file_id", json!(new_id)).await?;
                    context.set_pin_value("file", json!(file)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Move File Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct MoveGoogleDriveFileNode {}

impl MoveGoogleDriveFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MoveGoogleDriveFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_drive_move_file",
            "Move File",
            "Move a file to a different folder in Google Drive",
            "Data/Google/Drive",
        );
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_id",
            "File ID",
            "ID of file to move",
            VariableType::String,
        );
        node.add_input_pin(
            "new_parent_id",
            "New Parent ID",
            "Destination folder ID",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("file", "File", "Updated file details", VariableType::Struct)
            .set_schema::<GoogleDriveItem>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.file"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let file_id: String = context.evaluate_pin("file_id").await?;
        let new_parent_id: String = context.evaluate_pin("new_parent_id").await?;

        // First get current parents
        let client = reqwest::Client::new();
        let meta_resp = client
            .get(format!(
                "https://www.googleapis.com/drive/v3/files/{}",
                file_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("fields", "parents")])
            .send()
            .await;

        let current_parents = match meta_resp {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                body["parents"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|p| p.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default()
            }
            _ => String::new(),
        };

        let response = client
            .patch(format!(
                "https://www.googleapis.com/drive/v3/files/{}",
                file_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[
                ("addParents", new_parent_id.as_str()),
                ("removeParents", current_parents.as_str()),
                (
                    "fields",
                    "id,name,mimeType,size,createdTime,modifiedTime,webViewLink,parents",
                ),
            ])
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(file) = parse_drive_item(&body) {
                    context.set_pin_value("file", json!(file)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Search Files Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct SearchGoogleDriveNode {}

impl SearchGoogleDriveNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchGoogleDriveNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_drive_search",
            "Search Drive",
            "Search for files in Google Drive",
            "Data/Google/Drive",
        );
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "query",
            "Query",
            "Search query (supports Drive query syntax)",
            VariableType::String,
        );
        node.add_input_pin(
            "page_size",
            "Page Size",
            "Max results (1-1000)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("files", "Files", "Search results", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<GoogleDriveItem>();
        node.add_output_pin("count", "Count", "Number of results", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.metadata.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let query: String = context.evaluate_pin("query").await?;
        let page_size: i64 = context.evaluate_pin("page_size").await.unwrap_or(100);

        let client = reqwest::Client::new();
        let response = client
            .get("https://www.googleapis.com/drive/v3/files")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[
                ("q", query.as_str()),
                ("pageSize", &page_size.to_string()),
                (
                    "fields",
                    "files(id,name,mimeType,size,createdTime,modifiedTime,webViewLink,parents)",
                ),
            ])
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let files: Vec<GoogleDriveItem> = body["files"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_drive_item).collect())
                    .unwrap_or_default();
                let count = files.len() as i64;
                context.set_pin_value("files", json!(files)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get File Metadata Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGoogleDriveFileMetadataNode {}

impl GetGoogleDriveFileMetadataNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGoogleDriveFileMetadataNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_drive_get_metadata",
            "Get File Metadata",
            "Get detailed metadata for a Google Drive file",
            "Data/Google/Drive",
        );
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("file_id", "File ID", "File ID", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("file", "File", "File metadata", VariableType::Struct)
            .set_schema::<GoogleDriveItem>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.file"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let file_id: String = context.evaluate_pin("file_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://www.googleapis.com/drive/v3/files/{}",
                file_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[(
                "fields",
                "id,name,mimeType,size,createdTime,modifiedTime,webViewLink,parents",
            )])
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(file) = parse_drive_item(&body) {
                    context.set_pin_value("file", json!(file)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Upload File Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct UploadGoogleDriveFileNode {}

impl UploadGoogleDriveFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UploadGoogleDriveFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_drive_upload",
            "Upload File",
            "Upload a FlowPath file to Google Drive",
            "Data/Google/Drive",
        );
        node.add_icon("/flow/icons/google.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "source_file",
            "Source File",
            "FlowPath file to upload",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_name",
            "File Name",
            "Destination filename. Leave empty to use the FlowPath filename.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "parent_id",
            "Parent ID",
            "Destination folder ID. Leave empty for My Drive root.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "mime_type",
            "MIME Type",
            "Uploaded file MIME type",
            VariableType::String,
        )
        .set_default_value(Some(json!("application/octet-stream")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "file_id",
            "File ID",
            "Uploaded file ID",
            VariableType::String,
        );
        node.add_output_pin(
            "file",
            "File",
            "Uploaded file metadata",
            VariableType::Struct,
        )
        .set_schema::<GoogleDriveItem>();
        node.add_output_pin(
            "used_resumable_upload",
            "Used Resumable Upload",
            "True when Google Drive resumable upload was used",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "size",
            "Size",
            "Uploaded size in bytes",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.file"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let source_file: FlowPath = context.evaluate_pin("source_file").await?;
        let file_name: String = context.evaluate_pin("file_name").await.unwrap_or_default();
        let parent_id: String = context.evaluate_pin("parent_id").await.unwrap_or_default();
        let mime_type: String = context
            .evaluate_pin("mime_type")
            .await
            .unwrap_or_else(|_| "application/octet-stream".to_string());
        let file_name = if file_name.trim().is_empty() {
            flow_path_filename(&source_file)?
        } else {
            file_name
        };

        let client = reqwest::Client::new();
        match upload_flow_path_to_google_drive(
            context,
            &client,
            &provider,
            &source_file,
            &file_name,
            &parent_id,
            &mime_type,
        )
        .await
        {
            Ok((body, used_resumable_upload, size)) => {
                if let Some(file) = parse_drive_item(&body) {
                    let file_id = file.id.clone();
                    context.set_pin_value("file_id", json!(file_id)).await?;
                    context.set_pin_value("file", json!(file)).await?;
                    context
                        .set_pin_value("used_resumable_upload", json!(used_resumable_upload))
                        .await?;
                    context.set_pin_value("size", json!(size as i64)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Download File Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DownloadGoogleDriveFileNode {}

impl DownloadGoogleDriveFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DownloadGoogleDriveFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_drive_download",
            "Download File",
            "Download a Google Drive file into a FlowPath",
            "Data/Google/Drive",
        );
        node.add_icon("/flow/icons/google.svg");
        node.set_version(2);

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_id",
            "File ID",
            "File ID to download",
            VariableType::String,
        );
        node.add_input_pin(
            "export_mime_type",
            "Export MIME Type",
            "For Google Docs, export format (e.g., 'application/pdf')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "output_path",
            "Output Path",
            "FlowPath to write the downloaded file into",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("path", "Path", "Written file path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "file_name",
            "File Name",
            "Drive file name",
            VariableType::String,
        );
        node.add_output_pin(
            "mime_type",
            "MIME Type",
            "Downloaded or exported MIME type",
            VariableType::String,
        );
        node.add_output_pin(
            "size",
            "Size",
            "Downloaded size in bytes",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.file"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let file_id: String = context.evaluate_pin("file_id").await?;
        let export_mime_type: String = context
            .evaluate_pin("export_mime_type")
            .await
            .unwrap_or_default();
        let output_path: FlowPath = context.evaluate_pin("output_path").await?;

        let client = reqwest::Client::new();

        let meta_resp = client
            .get(format!(
                "https://www.googleapis.com/drive/v3/files/{}",
                file_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("fields", "name,mimeType")])
            .send()
            .await;

        let (file_name, mime_type) = match meta_resp {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                (
                    body["name"].as_str().unwrap_or("").to_string(),
                    body["mimeType"].as_str().unwrap_or("").to_string(),
                )
            }
            Ok(resp) => {
                context
                    .set_pin_value("error_message", json!(drive_error_message(resp).await))
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        let is_google_doc = mime_type.starts_with("application/vnd.google-apps.");

        let url = if is_google_doc {
            let export_type = if export_mime_type.is_empty() {
                default_export_mime_type(&mime_type)
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

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp.bytes().await?;
                let size = bytes.len() as i64;
                output_path.put(context, bytes.to_vec(), false).await?;
                let output_mime_type = if is_google_doc && !export_mime_type.is_empty() {
                    export_mime_type
                } else if is_google_doc {
                    default_export_mime_type(&mime_type).to_string()
                } else {
                    mime_type
                };
                context.set_pin_value("path", json!(output_path)).await?;
                context.set_pin_value("file_name", json!(file_name)).await?;
                context
                    .set_pin_value("mime_type", json!(output_mime_type))
                    .await?;
                context.set_pin_value("size", json!(size)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                context
                    .set_pin_value("error_message", json!(drive_error_message(resp).await))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
