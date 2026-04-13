use std::sync::Arc;

use flow_like::flow::{
    execution::{EventTrigger, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::{
    history::{Content, History, HistoryMessage, ImageUrl, MessageContent},
    response::Response,
    response_chunk::ResponseChunk,
};
use flow_like_types::{
    Cacheable, Value, anyhow, async_trait,
    intercom::InterComEvent,
    json::{from_str, json},
    sync::Mutex,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod attachment_from_path;
pub mod attachment_from_url;
pub mod attachment_to_url;
pub mod push_attachment;
pub mod push_attachments;
pub mod push_chunk;
pub mod push_global_session;
pub mod push_local_session;
pub mod push_reasoning;
pub mod push_response;
pub mod push_stat;
pub mod push_stats;
pub mod push_step;
pub mod push_text_to_step;
pub mod remove_step;

/// URL processing utilities for converting Tauri local file URLs to base64 data URLs
pub mod url_processing {
    use flow_like::flow::execution::context::ExecutionContext;
    use flow_like_types::utils::data_url::pathbuf_to_data_url;
    use std::path::PathBuf;

    pub fn is_remote_url(url: &str) -> bool {
        url.starts_with("https://")
            || (url.starts_with("http://") && !url.starts_with("http://asset.localhost/"))
    }

    pub fn is_tauri_asset_url(url: &str) -> bool {
        url.starts_with("asset://") || url.starts_with("http://asset.localhost/")
    }

    pub fn is_blake3_hash(filename: &str) -> bool {
        filename.len() == 64 && filename.chars().all(|c| c.is_ascii_hexdigit())
    }

    pub fn has_safe_path_components(path: &std::path::Path) -> flow_like_types::Result<()> {
        let path_str = path.to_string_lossy();

        // Check for .. traversal patterns
        if path_str.contains("/..")
            || path_str.contains("\\..")
            || path_str.contains("/../")
            || path_str.contains("\\..\\")
            || path_str.starts_with("..")
        {
            return Err(flow_like_types::anyhow!(
                "Security: Path traversal (..) detected in '{}'",
                path.display()
            ));
        }

        // Check for . current directory patterns
        if path_str.contains("/./")
            || path_str.contains("\\.\\")
            || path_str.starts_with("./")
            || path_str.starts_with(".\\")
        {
            return Err(flow_like_types::anyhow!(
                "Security: Current directory (.) patterns not allowed in '{}'",
                path.display()
            ));
        }

        for component in path.components() {
            match component {
                std::path::Component::Normal(part) => {
                    let part_str = part.to_string_lossy();
                    // Check for hidden files/directories (starting with .)
                    if part_str.starts_with('.') {
                        return Err(flow_like_types::anyhow!(
                            "Security: Hidden files/directories not allowed: '{}'",
                            part_str
                        ));
                    }
                }
                std::path::Component::ParentDir => {
                    return Err(flow_like_types::anyhow!(
                        "Security: Parent directory (..) not allowed"
                    ));
                }
                std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                    // Tauri URLs ALWAYS contain absolute paths - this is expected
                }
                std::path::Component::CurDir => {
                    return Err(flow_like_types::anyhow!(
                        "Security: Current directory (.) components not allowed"
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn extract_tauri_path(url: &str) -> flow_like_types::Result<PathBuf> {
        let without_scheme = url
            .replace("http://asset.localhost/", "")
            .replace("asset://localhost/", "");

        let path_str = without_scheme.split('?').next().unwrap_or(&without_scheme);

        let decoded = urlencoding::decode(path_str)?;
        let path = PathBuf::from(decoded.to_string());

        // Security check 1: Validate path components (no traversal, no hidden files, etc.)
        has_safe_path_components(&path)?;

        // Security check 2: Only allow files with Blake3 hash names
        if let Some(file_name) = path.file_stem() {
            let name = file_name.to_string_lossy();
            if !is_blake3_hash(&name) {
                return Err(flow_like_types::anyhow!(
                    "Security: Refusing to load file '{}' - filename is not a Blake3 hash",
                    name
                ));
            }
        } else {
            return Err(flow_like_types::anyhow!(
                "Invalid file path: no filename found"
            ));
        }

        Ok(path)
    }

    /// Processes a URL and converts Tauri local file URLs to base64 data URLs.
    /// Returns the URL unchanged if it's an HTTP(S) URL or already a data URL.
    /// Returns empty string if Tauri URL processing fails (invalid path or file not readable).
    pub async fn process_url(url: &str, mut context: Option<&mut ExecutionContext>) -> String {
        if let Some(ctx) = context.as_deref_mut() {
            ctx.log_message(
                &format!("Processing URL: {}", url),
                flow_like::flow::execution::LogLevel::Debug,
            );
        }

        // If it's already an HTTP(S) URL (S3 or other remote storage), return as-is
        if is_remote_url(url) {
            if let Some(ctx) = context.as_deref_mut() {
                ctx.log_message(
                    "URL is remote HTTPS, returning unchanged",
                    flow_like::flow::execution::LogLevel::Debug,
                );
            }
            return url.to_string();
        }

        if url.starts_with("data:") {
            if let Some(ctx) = context.as_deref_mut() {
                ctx.log_message(
                    "URL is already data URL, returning unchanged",
                    flow_like::flow::execution::LogLevel::Debug,
                );
            }
            return url.to_string();
        }

        if !is_tauri_asset_url(url) {
            let msg = format!(
                "URL is not a Tauri asset URL (doesn't start with asset:// or http://asset.localhost/), returning unchanged: {}",
                url
            );
            if let Some(ctx) = context.as_deref_mut() {
                ctx.log_message(&msg, flow_like::flow::execution::LogLevel::Debug);
            }
            return url.to_string();
        }

        if let Some(ctx) = context.as_deref_mut() {
            ctx.log_message(
                "URL is a Tauri asset URL, extracting path...",
                flow_like::flow::execution::LogLevel::Debug,
            );
        }

        let file_path = match extract_tauri_path(url) {
            Ok(path) => {
                let msg = format!("Successfully extracted path: {}", path.display());
                if let Some(ctx) = context.as_deref_mut() {
                    ctx.log_message(&msg, flow_like::flow::execution::LogLevel::Debug);
                }
                path
            }
            Err(e) => {
                let msg = format!(
                    "Failed to validate Tauri URL '{}': {}. Skipping this attachment.",
                    url, e
                );
                if let Some(ctx) = context.as_deref_mut() {
                    ctx.log_message(&msg, flow_like::flow::execution::LogLevel::Error);
                }
                return String::new();
            }
        };

        let msg = format!(
            "Attempting to read file and convert to data URL: {}",
            file_path.display()
        );
        if let Some(ctx) = context.as_deref_mut() {
            ctx.log_message(&msg, flow_like::flow::execution::LogLevel::Debug);
        }

        // Try to read the file and convert to data URL
        match pathbuf_to_data_url(&file_path).await {
            Ok(data_url) => {
                let preview = if data_url.len() > 100 {
                    format!("{}...", &data_url[0..100])
                } else {
                    data_url.clone()
                };
                let msg = format!(
                    "Successfully converted local file '{}' to data URL (length: {} bytes, preview: {})",
                    file_path.display(),
                    data_url.len(),
                    preview
                );
                if let Some(ctx) = context.as_deref_mut() {
                    ctx.log_message(&msg, flow_like::flow::execution::LogLevel::Debug);
                }
                data_url
            }
            Err(e) => {
                let msg = format!(
                    "Failed to read local file '{}': {}. File may be deleted or inaccessible. Skipping this attachment.",
                    file_path.display(),
                    e
                );
                if let Some(ctx) = context {
                    ctx.log_message(&msg, flow_like::flow::execution::LogLevel::Error);
                }
                // Return empty string instead of the Tauri URL to prevent "Unsupported scheme" errors
                String::new()
            }
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ChatEventNode {}

impl ChatEventNode {
    pub fn new() -> Self {
        ChatEventNode {}
    }

    async fn process_history_messages(
        messages: Vec<HistoryMessage>,
        mut context: Option<&mut ExecutionContext>,
    ) -> Vec<HistoryMessage> {
        let mut processed = Vec::with_capacity(messages.len());

        for mut message in messages {
            if let MessageContent::Contents(contents) = &message.content {
                let mut processed_contents = Vec::new();

                for content in contents {
                    match content {
                        Content::Image {
                            content_type,
                            image_url,
                        } => {
                            let processed_url =
                                url_processing::process_url(&image_url.url, context.as_deref_mut())
                                    .await;
                            // Only include the image if URL processing succeeded (not empty)
                            if !processed_url.is_empty() {
                                processed_contents.push(Content::Image {
                                    content_type: content_type.clone(),
                                    image_url: ImageUrl {
                                        url: processed_url,
                                        detail: image_url.detail.clone(),
                                    },
                                });
                            }
                        }
                        other => processed_contents.push(other.clone()),
                    }
                }

                message.content = MessageContent::Contents(processed_contents);
            }

            processed.push(message);
        }

        processed
    }
}

#[async_trait]
impl NodeLogic for ChatEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new("events_chat", "Chat Event", "A simple Chat event", "Events");
        node.add_icon("/flow/icons/event.svg");
        node.set_version(1);
        node.set_start(true);
        node.set_can_be_referenced_by_fns(true);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Starting an event",
            VariableType::Execution,
        );

        node.add_output_pin("history", "History", "Chat History", VariableType::Struct)
            .set_schema::<History>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "local_session",
            "Local Session",
            "Local to the Chat",
            VariableType::Struct,
        );

        node.add_output_pin(
            "global_session",
            "Global Session",
            "Global to the User",
            VariableType::Struct,
        );

        node.add_output_pin(
            "tools",
            "Tools",
            "Tools requested by the user",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin("actions", "Actions", "User Actions", VariableType::Struct)
            .set_schema::<ChatAction>()
            .set_value_type(flow_like::flow::pin::ValueType::Array)
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "attachments",
            "Attachments",
            "User Attachments or References",
            VariableType::Struct,
        )
        .set_schema::<Attachment>()
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("user", "User", "User Information", VariableType::Struct)
            .set_schema::<User>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let exec_out_pin = context.get_pin_by_name("exec_out").await?;

        if context.delegated {
            context.activate_exec_pin_ref(&exec_out_pin).await?;
            return Ok(());
        }

        let payload = context.get_payload().await?;
        let chat = payload
            .payload
            .clone()
            .ok_or(anyhow!("Failed to get payload"))?;
        let chat: Chat = flow_like_types::json::from_value(chat)
            .map_err(|e| anyhow!("Failed to deserialize payload: {}", e))?;

        // Process attachments to convert Tauri URLs to data URLs
        let processed_attachments = if let Some(attachments) = chat.attachments {
            Attachment::process_vec(attachments, Some(context)).await
        } else {
            vec![]
        };

        // Process history messages to convert Tauri URLs in image_url fields to data URLs
        let processed_messages = Self::process_history_messages(chat.messages, Some(context)).await;

        context
            .set_pin_value(
                "history",
                json!(History::new("".to_string(), processed_messages)),
            )
            .await?;
        context
            .set_pin_value(
                "local_session",
                chat.local_session.unwrap_or(from_str("{}")?),
            )
            .await?;
        context
            .set_pin_value(
                "global_session",
                chat.global_session.unwrap_or(from_str("{}")?),
            )
            .await?;
        context
            .set_pin_value("tools", json!(chat.tools.unwrap_or_default()))
            .await?;
        context
            .set_pin_value("actions", json!(chat.actions.unwrap_or_default()))
            .await?;
        context
            .set_pin_value("attachments", json!(processed_attachments))
            .await?;
        context
            .set_pin_value("user", json!(chat.user.unwrap_or_default()))
            .await?;
        context.activate_exec_pin_ref(&exec_out_pin).await?;

        let completion_event: EventTrigger = Arc::new(|run| {
            Box::pin(async move {
                if let Some(cached_response) = run.cache.read().await.get("chat_response") {
                    let cached_response = cached_response.clone();
                    let response = cached_response
                        .as_any()
                        .downcast_ref::<CachedChatResponse>()
                        .ok_or(anyhow!("Failed to downcast cached response"))?;

                    let event = {
                        let response = response.response.lock().await;
                        InterComEvent::with_type("chat_out", response.clone())
                    };
                    event.call(&run.callback).await?;
                }
                Ok(())
            })
        });

        context.hook_completion_event(completion_event).await;

        return Ok(());
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ComplexAttachment {
    pub url: String,
    pub preview_text: Option<String>,
    pub thumbnail_url: Option<String>,
    pub name: Option<String>,
    pub size: Option<u64>,
    pub r#type: Option<String>,
    pub anchor: Option<String>,
    pub page: Option<u32>,
}

impl ComplexAttachment {
    /// Processes the attachment's URLs and converts Tauri local file URLs to base64 data URLs.
    /// Returns None if the main URL processing fails (empty result).
    pub async fn process(&self, mut context: Option<&mut ExecutionContext>) -> Option<Self> {
        let mut processed = self.clone();
        processed.url = url_processing::process_url(&self.url, context.as_deref_mut()).await;

        // If main URL processing failed (empty string), skip this attachment
        if processed.url.is_empty() {
            return None;
        }

        if let Some(ref thumbnail) = self.thumbnail_url {
            let processed_thumbnail = url_processing::process_url(thumbnail, context).await;
            // Only set thumbnail if processing succeeded (not empty)
            processed.thumbnail_url = if processed_thumbnail.is_empty() {
                None
            } else {
                Some(processed_thumbnail)
            };
        }

        Some(processed)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(untagged)]
pub enum Attachment {
    Url(String),
    Complex(ComplexAttachment),
}

impl Attachment {
    /// Processes the attachment and converts Tauri local file URLs to base64 data URLs
    pub async fn process(&self, context: Option<&mut ExecutionContext>) -> Option<Self> {
        match self {
            Attachment::Url(url) => {
                let processed_url = url_processing::process_url(url, context).await;
                // Filter out empty URLs (failed Tauri URL processing)
                if processed_url.is_empty() {
                    None
                } else {
                    Some(Attachment::Url(processed_url))
                }
            }
            Attachment::Complex(complex) => complex.process(context).await.map(Attachment::Complex),
        }
    }

    /// Processes a vector of attachments and converts Tauri local file URLs to base64 data URLs.
    /// Filters out attachments that failed to process (empty URLs or invalid paths).
    pub async fn process_vec(
        attachments: Vec<Attachment>,
        mut context: Option<&mut ExecutionContext>,
    ) -> Vec<Attachment> {
        let mut processed = Vec::with_capacity(attachments.len());
        for attachment in attachments {
            if let Some(processed_attachment) = attachment.process(context.as_deref_mut()).await {
                processed.push(processed_attachment);
            }
        }
        processed
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub enum ButtonType {
    Outline,
    Primary,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub enum ChatAction {
    Button(String, ButtonType),
    Form(String, Value),
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct User {
    pub sub: String,
    pub name: String,
    pub bot: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Chat {
    pub chat_id: Option<String>,
    pub messages: Vec<HistoryMessage>,
    pub local_session: Option<Value>,
    pub global_session: Option<Value>,
    pub actions: Option<Vec<ChatAction>>,
    pub tools: Option<Vec<String>>,
    pub user: Option<User>,
    pub attachments: Option<Vec<Attachment>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ChatResponse {
    pub response: Response,
    pub local_session: Option<Value>,
    pub global_session: Option<Value>,
    pub actions: Vec<ChatAction>,
    pub attachments: Vec<Attachment>,
    pub model_id: Option<String>,
}

#[derive(Clone)]
pub struct CachedChatResponse {
    response: Arc<Mutex<ChatResponse>>,
    reasoning: Arc<Mutex<Reasoning>>,
}

impl CachedChatResponse {
    pub async fn load(context: &mut ExecutionContext) -> flow_like_types::Result<Self> {
        if let Some(cached_response) = context.get_cache("chat_response").await {
            let response = cached_response
                .as_any()
                .downcast_ref::<CachedChatResponse>()
                .ok_or(anyhow!("Failed to downcast cached response"))?;
            return Ok(response.clone());
        }

        let response = ChatResponse {
            response: Response::new(),
            actions: vec![],
            attachments: vec![],
            global_session: flow_like_types::json::from_str("{}")?,
            local_session: flow_like_types::json::from_str("{}")?,
            model_id: None,
        };

        let reasoning = Reasoning {
            current_message: "".to_string(),
            current_step: 0,
            plan: vec![],
        };

        let cached_response = CachedChatResponse {
            response: Arc::new(Mutex::new(response)),
            reasoning: Arc::new(Mutex::new(reasoning)),
        };

        let cacheable = Arc::new(cached_response.clone()) as Arc<dyn Cacheable>;
        context.set_cache("chat_response", cacheable).await;
        Ok(cached_response)
    }
}

impl Cacheable for CachedChatResponse {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Reasoning {
    pub plan: Vec<(u32, String)>,
    pub current_step: u32,
    pub current_message: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ChatStreamingResponse {
    pub chunk: Option<ResponseChunk>,
    pub actions: Vec<ChatAction>,
    pub attachments: Vec<Attachment>,
    pub plan: Option<Reasoning>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ChatUsageStat {
    pub step_name: String,
    pub stats: flow_like_model_provider::response::LLMUsageStats,
}

#[cfg(test)]
mod tests {
    use super::url_processing::*;
    use super::*;

    #[test]
    fn test_is_remote_url() {
        // Valid remote URLs
        assert!(is_remote_url("https://example.com/file.png"));
        assert!(is_remote_url("https://s3.amazonaws.com/bucket/file.pdf"));
        assert!(is_remote_url("http://example.com/image.jpg"));

        // Tauri asset URLs should not be considered remote
        assert!(!is_remote_url("http://asset.localhost/path/to/file.png"));
        assert!(!is_remote_url("asset://localhost/file.png"));

        // Data URLs should not be considered remote
        assert!(!is_remote_url("data:image/png;base64,iVBORw0KG..."));
    }

    #[test]
    fn test_is_tauri_asset_url() {
        // Valid Tauri asset URLs
        assert!(is_tauri_asset_url("asset://localhost/chat/file.png"));
        assert!(is_tauri_asset_url("http://asset.localhost/storage/doc.pdf"));

        // Non-Tauri URLs
        assert!(!is_tauri_asset_url("https://example.com/file.png"));
        assert!(!is_tauri_asset_url("http://example.com/file.png"));
        assert!(!is_tauri_asset_url("data:image/png;base64,iVBORw0KG..."));
    }

    #[test]
    fn test_is_blake3_hash() {
        // Valid Blake3 hash (64 hex characters)
        assert!(is_blake3_hash(
            "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce"
        ));
        assert!(is_blake3_hash(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(is_blake3_hash(
            "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789"
        ));

        // Invalid: too short
        assert!(!is_blake3_hash("3d65ddd83e92b1e3fffee47d8e209802"));
        assert!(!is_blake3_hash("abc123"));

        // Invalid: too long
        assert!(!is_blake3_hash(
            "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce00"
        ));

        // Invalid: non-hex characters
        assert!(!is_blake3_hash(
            "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadcg"
        ));
        assert!(!is_blake3_hash(
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
        ));

        // Invalid: path traversal attempts
        assert!(!is_blake3_hash("../etc/passwd"));
        assert!(!is_blake3_hash("../../sensitive_file"));

        // Invalid: special characters
        assert!(!is_blake3_hash("file@name"));
        assert!(!is_blake3_hash("my-file-name"));

        // Empty string
        assert!(!is_blake3_hash(""));
    }

    #[test]
    fn test_extract_tauri_path_valid_blake3() {
        let valid_hash = "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce";

        // Test asset:// URL
        let url = format!("asset://localhost/chat/{}.png", valid_hash);
        let result = extract_tauri_path(&url);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert_eq!(path.file_stem().unwrap().to_string_lossy(), valid_hash);

        // Test http://asset.localhost/ URL
        let url = format!("http://asset.localhost/storage/{}.pdf", valid_hash);
        let result = extract_tauri_path(&url);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert_eq!(path.file_stem().unwrap().to_string_lossy(), valid_hash);
    }

    #[test]
    fn test_extract_tauri_path_invalid_hash() {
        // Non-Blake3 hash filenames should fail
        let invalid_urls = vec![
            "asset://localhost/chat/myfile.png",
            "http://asset.localhost/storage/document.pdf",
            "asset://localhost/../etc/passwd",
            "http://asset.localhost/../../sensitive.txt",
        ];

        for url in invalid_urls {
            let result = extract_tauri_path(url);
            assert!(result.is_err(), "Expected error for URL: {}", url);
            let err = result.unwrap_err();
            assert!(
                err.to_string().contains("Security") || err.to_string().contains("Blake3"),
                "Error should mention security or Blake3 validation, got: {}",
                err
            );
        }
    }

    #[test]
    fn test_extract_tauri_path_url_encoded() {
        let valid_hash = "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce";

        // Test URL-encoded path
        let url = format!(
            "http://asset.localhost/path%20with%20spaces/{}.png",
            valid_hash
        );
        let result = extract_tauri_path(&url);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains("path with spaces"));
        assert_eq!(path.file_stem().unwrap().to_string_lossy(), valid_hash);
    }

    #[test]
    fn test_has_safe_path_components() {
        use std::path::Path;

        // Safe paths - normal directory structures
        assert!(has_safe_path_components(Path::new("chat/file.png")).is_ok());
        assert!(has_safe_path_components(Path::new("storage/documents/file.pdf")).is_ok());
        assert!(has_safe_path_components(Path::new("media/videos/file.mp4")).is_ok());
        assert!(has_safe_path_components(Path::new("file.txt")).is_ok());

        // Hidden files should be rejected
        assert!(has_safe_path_components(Path::new(".hidden/file.txt")).is_err());
        assert!(has_safe_path_components(Path::new("chat/.config")).is_err());
        assert!(has_safe_path_components(Path::new(".ssh/id_rsa")).is_err());

        // Absolute paths are allowed (Tauri always uses absolute paths)
        #[cfg(unix)]
        {
            assert!(
                has_safe_path_components(Path::new("/Users/felix/Library/Caches/file.txt")).is_ok()
            );
            assert!(has_safe_path_components(Path::new("/tmp/file.log")).is_ok());
        }

        #[cfg(windows)]
        {
            assert!(
                has_safe_path_components(Path::new("C:\\Users\\felix\\AppData\\file.txt")).is_ok()
            );
        }
    }

    #[test]
    fn test_has_safe_path_components_traversal() {
        use std::path::Path;

        // Path traversal attempts should fail
        assert!(has_safe_path_components(Path::new("chat/../sensitive.txt")).is_err());
        assert!(has_safe_path_components(Path::new("../etc/passwd")).is_err());
        assert!(has_safe_path_components(Path::new("../../root/.ssh")).is_err());
        assert!(has_safe_path_components(Path::new("dir1/../dir2/../../../etc")).is_err());

        // Current directory references
        assert!(has_safe_path_components(Path::new("./file.txt")).is_err());
        assert!(has_safe_path_components(Path::new("chat/./file.txt")).is_err());
    }

    #[test]
    fn test_extract_tauri_path_with_safe_paths() {
        let valid_hash = "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce";

        // Test various safe path structures with any extension
        let safe_urls = vec![
            format!("asset://localhost/chat/{}.png", valid_hash),
            format!("http://asset.localhost/storage/{}.pdf", valid_hash),
            format!("asset://localhost/media/videos/{}.mp4", valid_hash),
            format!("http://asset.localhost/documents/{}.docx", valid_hash),
            format!("asset://localhost/archives/{}.zip", valid_hash),
            format!("http://asset.localhost/{}.xyz", valid_hash), // Any extension works
        ];

        for url in safe_urls {
            let result = extract_tauri_path(&url);
            assert!(result.is_ok(), "Expected success for safe URL: {}", url);
            let path = result.unwrap();
            assert_eq!(path.file_stem().unwrap().to_string_lossy(), valid_hash);
        }
    }

    #[test]
    fn test_extract_tauri_path_path_traversal() {
        let valid_hash = "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce";

        // Test path traversal attempts - even with valid hash, should fail on path components
        let traversal_urls = vec![
            format!("asset://localhost/../etc/{}.txt", valid_hash),
            format!(
                "http://asset.localhost/chat/../../sensitive/{}.pdf",
                valid_hash
            ),
            format!("asset://localhost/.ssh/{}.key", valid_hash),
            format!("http://asset.localhost/.config/{}.conf", valid_hash),
        ];

        for url in traversal_urls {
            let result = extract_tauri_path(&url);
            assert!(result.is_err(), "Expected error for traversal URL: {}", url);
            let err = result.unwrap_err();
            assert!(
                err.to_string().contains("Security"),
                "Error should mention security, got: {}",
                err
            );
        }
    }

    #[tokio::test]
    async fn test_process_url_remote_https() {
        let url = "https://example.com/file.png";
        let processed = process_url(url, None).await;
        // Remote HTTPS URLs should be returned unchanged
        assert_eq!(processed, url);
    }

    #[tokio::test]
    async fn test_process_url_data_url() {
        let url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let processed = process_url(url, None).await;
        // Data URLs should be returned unchanged
        assert_eq!(processed, url);
    }

    #[tokio::test]
    async fn test_process_url_invalid_tauri_hash() {
        // File with non-Blake3 hash should be rejected and return empty string
        let url = "asset://localhost/chat/myfile.png";
        let processed = process_url(url, None).await;
        // Should return empty string due to security validation failure
        assert_eq!(processed, "");
    }

    #[tokio::test]
    async fn test_attachment_process_url() {
        let url = "https://example.com/file.png";
        let attachment = Attachment::Url(url.to_string());
        let processed = attachment.process(None).await;

        assert!(processed.is_some());
        match processed.unwrap() {
            Attachment::Url(processed_url) => {
                assert_eq!(processed_url, url);
            }
            _ => panic!("Expected Url variant"),
        }
    }

    #[tokio::test]
    async fn test_attachment_process_invalid_url() {
        // Invalid Tauri URL should be filtered out
        let url = "asset://localhost/chat/invalid.png";
        let attachment = Attachment::Url(url.to_string());
        let processed = attachment.process(None).await;

        assert!(
            processed.is_none(),
            "Invalid Tauri URL should be filtered out"
        );
    }

    #[tokio::test]
    async fn test_attachment_process_complex() {
        let complex = ComplexAttachment {
            url: "https://example.com/file.pdf".to_string(),
            preview_text: Some("Preview".to_string()),
            thumbnail_url: Some("https://example.com/thumb.jpg".to_string()),
            name: Some("document.pdf".to_string()),
            size: Some(1024),
            r#type: Some("application/pdf".to_string()),
            anchor: None,
            page: None,
        };

        let attachment = Attachment::Complex(complex.clone());
        let processed = attachment.process(None).await;

        assert!(processed.is_some());
        match processed.unwrap() {
            Attachment::Complex(processed_complex) => {
                assert_eq!(processed_complex.url, complex.url);
                assert_eq!(processed_complex.thumbnail_url, complex.thumbnail_url);
                assert_eq!(processed_complex.name, complex.name);
            }
            _ => panic!("Expected Complex variant"),
        }
    }

    #[tokio::test]
    async fn test_attachment_process_vec() {
        let attachments = vec![
            Attachment::Url("https://example.com/file1.png".to_string()),
            Attachment::Url("https://example.com/file2.jpg".to_string()),
            Attachment::Complex(ComplexAttachment {
                url: "https://example.com/doc.pdf".to_string(),
                preview_text: None,
                thumbnail_url: Some("https://example.com/thumb.jpg".to_string()),
                name: Some("document.pdf".to_string()),
                size: Some(2048),
                r#type: Some("application/pdf".to_string()),
                anchor: None,
                page: None,
            }),
        ];

        let processed = Attachment::process_vec(attachments.clone(), None).await;

        assert_eq!(processed.len(), 3);

        // Verify URLs are processed (in this case, remote URLs stay unchanged)
        match &processed[0] {
            Attachment::Url(url) => assert!(url.starts_with("https://")),
            _ => panic!("Expected Url variant"),
        }

        match &processed[2] {
            Attachment::Complex(complex) => {
                assert!(complex.url.starts_with("https://"));
                assert!(
                    complex
                        .thumbnail_url
                        .as_ref()
                        .unwrap()
                        .starts_with("https://")
                );
            }
            _ => panic!("Expected Complex variant"),
        }
    }

    #[tokio::test]
    async fn test_attachment_process_vec_filters_invalid() {
        // Test that invalid Tauri URLs are filtered out from the vec
        let attachments = vec![
            Attachment::Url("https://example.com/valid.png".to_string()),
            Attachment::Url("asset://localhost/chat/invalid.png".to_string()), // Should be filtered
            Attachment::Url("https://example.com/another-valid.jpg".to_string()),
        ];

        let processed = Attachment::process_vec(attachments, None).await;

        // Should only have 2 attachments (the 2 valid HTTPS URLs)
        assert_eq!(processed.len(), 2);

        // All remaining should be valid HTTPS URLs
        for attachment in processed {
            match attachment {
                Attachment::Url(url) => assert!(url.starts_with("https://")),
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn test_complex_attachment_process() {
        let complex = ComplexAttachment {
            url: "https://s3.amazonaws.com/bucket/file.pdf".to_string(),
            preview_text: Some("A preview".to_string()),
            thumbnail_url: Some("https://s3.amazonaws.com/bucket/thumb.jpg".to_string()),
            name: Some("report.pdf".to_string()),
            size: Some(4096),
            r#type: Some("application/pdf".to_string()),
            anchor: Some("#section-2".to_string()),
            page: Some(3),
        };

        let processed = complex.process(None).await;

        assert!(processed.is_some());
        let processed = processed.unwrap();

        // Remote URLs should remain unchanged
        assert_eq!(processed.url, complex.url);
        assert_eq!(processed.thumbnail_url, complex.thumbnail_url);

        // Other fields should be preserved
        assert_eq!(processed.preview_text, complex.preview_text);
        assert_eq!(processed.name, complex.name);
        assert_eq!(processed.size, complex.size);
        assert_eq!(processed.r#type, complex.r#type);
        assert_eq!(processed.anchor, complex.anchor);
        assert_eq!(processed.page, complex.page);
    }

    #[test]
    fn test_blake3_hash_edge_cases() {
        // Test case sensitivity (both should be valid)
        let lowercase = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let uppercase = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";
        let mixed = "aBcDeF0123456789aBcDeF0123456789aBcDeF0123456789aBcDeF0123456789";

        assert!(is_blake3_hash(lowercase));
        assert!(is_blake3_hash(uppercase));
        assert!(is_blake3_hash(mixed));

        // All zeros and all f's should be valid
        let all_zeros = "0000000000000000000000000000000000000000000000000000000000000000";
        let all_fs = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

        assert!(is_blake3_hash(all_zeros));
        assert!(is_blake3_hash(all_fs));
    }
}
