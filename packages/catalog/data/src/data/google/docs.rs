use super::provider::{GOOGLE_PROVIDER_ID, GoogleProvider};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

// =============================================================================
// Google Docs Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleDocument {
    pub document_id: String,
    pub title: String,
    pub revision_id: Option<String>,
}

// =============================================================================
// Create Document Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGoogleDocNode {}

impl CreateGoogleDocNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGoogleDocNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_docs_create",
            "Create Document",
            "Create a new Google Document",
            "Data/Google/Docs",
        );
        node.set_flowscript_name("google.docs", "create");
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
        node.add_input_pin("title", "Title", "Document title", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("document_id", "Document ID", "", VariableType::String);
        node.add_output_pin("document", "Document", "", VariableType::Struct)
            .set_schema::<GoogleDocument>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/documents"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let title: String = context.evaluate_pin("title").await?;

        let body = json!({ "title": title });

        let client = reqwest::Client::new();
        let response = client
            .post("https://docs.googleapis.com/v1/documents")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let document = GoogleDocument {
                    document_id: body["documentId"].as_str().unwrap_or("").to_string(),
                    title: body["title"].as_str().unwrap_or("").to_string(),
                    revision_id: body["revisionId"].as_str().map(String::from),
                };
                let id = document.document_id.clone();
                context.set_pin_value("document_id", json!(id)).await?;
                context.set_pin_value("document", json!(document)).await?;
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
// Get Document Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGoogleDocNode {}

impl GetGoogleDocNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGoogleDocNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_docs_get",
            "Get Document",
            "Get a Google Document's metadata and content",
            "Data/Google/Docs",
        );
        node.set_flowscript_name("google.docs", "get");
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
        node.add_input_pin("document_id", "Document ID", "", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("document", "Document", "", VariableType::Struct)
            .set_schema::<GoogleDocument>();
        node.add_output_pin(
            "content",
            "Content",
            "Raw document content JSON",
            VariableType::Generic,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/documents.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let document_id: String = context.evaluate_pin("document_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://docs.googleapis.com/v1/documents/{}",
                document_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let document = GoogleDocument {
                    document_id: body["documentId"].as_str().unwrap_or("").to_string(),
                    title: body["title"].as_str().unwrap_or("").to_string(),
                    revision_id: body["revisionId"].as_str().map(String::from),
                };
                context.set_pin_value("document", json!(document)).await?;
                context.set_pin_value("content", body).await?;
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
// Get Document Text Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGoogleDocTextNode {}

impl GetGoogleDocTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

fn extract_text_from_doc(body: &Value) -> String {
    let mut text = String::new();
    if let Some(content) = body["body"]["content"].as_array() {
        for element in content {
            if let Some(paragraph) = element.get("paragraph")
                && let Some(elements) = paragraph["elements"].as_array()
            {
                for elem in elements {
                    if let Some(text_run) = elem.get("textRun")
                        && let Some(content) = text_run["content"].as_str()
                    {
                        text.push_str(content);
                    }
                }
            }
        }
    }
    text
}

#[async_trait]
impl NodeLogic for GetGoogleDocTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_docs_get_text",
            "Get Document Text",
            "Extract plain text from a Google Document",
            "Data/Google/Docs",
        );
        node.set_flowscript_name("google.docs", "getText");
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
        node.add_input_pin("document_id", "Document ID", "", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("text", "Text", "Plain text content", VariableType::String);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/documents.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let document_id: String = context.evaluate_pin("document_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://docs.googleapis.com/v1/documents/{}",
                document_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let text = extract_text_from_doc(&body);
                context.set_pin_value("text", json!(text)).await?;
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
// Insert Text Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct InsertGoogleDocTextNode {}

impl InsertGoogleDocTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for InsertGoogleDocTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_docs_insert_text",
            "Insert Text",
            "Insert text at a specific location in a Google Document",
            "Data/Google/Docs",
        );
        node.set_flowscript_name("google.docs", "insertText");
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
        node.add_input_pin("document_id", "Document ID", "", VariableType::String);
        node.add_input_pin("text", "Text", "Text to insert", VariableType::String);
        node.add_input_pin(
            "index",
            "Index",
            "Character index to insert at (1 = start)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/documents"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let document_id: String = context.evaluate_pin("document_id").await?;
        let text: String = context.evaluate_pin("text").await?;
        let index: i64 = context.evaluate_pin("index").await.unwrap_or(1);

        let body = json!({
            "requests": [{
                "insertText": {
                    "location": { "index": index },
                    "text": text
                }
            }]
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://docs.googleapis.com/v1/documents/{}:batchUpdate",
                document_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
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
// Delete Text Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteGoogleDocTextNode {}

impl DeleteGoogleDocTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteGoogleDocTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_docs_delete_text",
            "Delete Text",
            "Delete text from a range in a Google Document",
            "Data/Google/Docs",
        );
        node.set_flowscript_name("google.docs", "deleteText");
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
        node.add_input_pin("document_id", "Document ID", "", VariableType::String);
        node.add_input_pin(
            "start_index",
            "Start Index",
            "Start index (inclusive)",
            VariableType::Integer,
        );
        node.add_input_pin(
            "end_index",
            "End Index",
            "End index (exclusive)",
            VariableType::Integer,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/documents"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let document_id: String = context.evaluate_pin("document_id").await?;
        let start_index: i64 = context.evaluate_pin("start_index").await?;
        let end_index: i64 = context.evaluate_pin("end_index").await?;

        let body = json!({
            "requests": [{
                "deleteContentRange": {
                    "range": {
                        "startIndex": start_index,
                        "endIndex": end_index
                    }
                }
            }]
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://docs.googleapis.com/v1/documents/{}:batchUpdate",
                document_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
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
// Replace Text Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ReplaceGoogleDocTextNode {}

impl ReplaceGoogleDocTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ReplaceGoogleDocTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_docs_replace_text",
            "Replace All Text",
            "Replace all occurrences of text in a Google Document",
            "Data/Google/Docs",
        );
        node.set_flowscript_name("google.docs", "replaceText");
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
        node.add_input_pin("document_id", "Document ID", "", VariableType::String);
        node.add_input_pin(
            "search_text",
            "Search Text",
            "Text to find",
            VariableType::String,
        );
        node.add_input_pin(
            "replace_text",
            "Replace Text",
            "Replacement text",
            VariableType::String,
        );
        node.add_input_pin(
            "match_case",
            "Match Case",
            "Case-sensitive search",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "occurrences",
            "Occurrences",
            "Number of replacements",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/documents"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let document_id: String = context.evaluate_pin("document_id").await?;
        let search_text: String = context.evaluate_pin("search_text").await?;
        let replace_text: String = context.evaluate_pin("replace_text").await?;
        let match_case: bool = context.evaluate_pin("match_case").await.unwrap_or(true);

        let body = json!({
            "requests": [{
                "replaceAllText": {
                    "containsText": {
                        "text": search_text,
                        "matchCase": match_case
                    },
                    "replaceText": replace_text
                }
            }]
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://docs.googleapis.com/v1/documents/{}:batchUpdate",
                document_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let occurrences = body["replies"][0]["replaceAllText"]["occurrencesChanged"]
                    .as_i64()
                    .unwrap_or(0);
                context
                    .set_pin_value("occurrences", json!(occurrences))
                    .await?;
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
// Export Document Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ExportGoogleDocNode {}

impl ExportGoogleDocNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ExportGoogleDocNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_docs_export",
            "Export Document",
            "Export a Google Document into a FlowPath",
            "Data/Google/Docs",
        );
        node.set_flowscript_name("google.docs", "export");
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
        node.add_input_pin("document_id", "Document ID", "", VariableType::String);
        node.add_input_pin("format", "Format", "Export format", VariableType::String)
            .set_default_value(Some(json!("text/plain")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "text/plain".to_string(),
                        "text/html".to_string(),
                        "application/pdf".to_string(),
                        "application/rtf".to_string(),
                        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                            .to_string(),
                    ])
                    .build(),
            );
        node.add_input_pin(
            "output_path",
            "Output Path",
            "FlowPath to write the exported document into",
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
            "size",
            "Size",
            "Exported size in bytes",
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
        let document_id: String = context.evaluate_pin("document_id").await?;
        let format: String = context
            .evaluate_pin("format")
            .await
            .unwrap_or_else(|_| "text/plain".to_string());
        let output_path: FlowPath = context.evaluate_pin("output_path").await?;

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://www.googleapis.com/drive/v3/files/{}/export",
                document_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("mimeType", format.as_str())])
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp.bytes().await?;
                let size = bytes.len() as i64;
                output_path.put(context, bytes.to_vec(), false).await?;
                context.set_pin_value("path", json!(output_path)).await?;
                context.set_pin_value("size", json!(size)).await?;
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
