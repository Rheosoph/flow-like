use flow_like::flow::execution::context::ExecutionContext;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
                "Processing attachment URL",
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
            let msg = "URL is not a Tauri asset URL, returning unchanged";
            if let Some(ctx) = context.as_deref_mut() {
                ctx.log_message(msg, flow_like::flow::execution::LogLevel::Debug);
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
                if let Some(ctx) = context.as_deref_mut() {
                    ctx.log_message(
                        "Successfully extracted local attachment path",
                        flow_like::flow::execution::LogLevel::Debug,
                    );
                }
                path
            }
            Err(_) => {
                let msg = "Failed to validate local attachment URL; skipping this attachment";
                if let Some(ctx) = context.as_deref_mut() {
                    ctx.log_message(msg, flow_like::flow::execution::LogLevel::Warn);
                } else {
                    tracing::warn!("{}", msg);
                }
                return String::new();
            }
        };

        if let Some(ctx) = context.as_deref_mut() {
            ctx.log_message(
                "Attempting to read local attachment and convert it to a data URL",
                flow_like::flow::execution::LogLevel::Debug,
            );
        }

        // Try to read the file and convert to data URL
        match pathbuf_to_data_url(&file_path).await {
            Ok(data_url) => {
                let msg = format!(
                    "Successfully converted local attachment to a data URL (length: {} bytes)",
                    data_url.len()
                );
                if let Some(ctx) = context.as_deref_mut() {
                    ctx.log_message(&msg, flow_like::flow::execution::LogLevel::Debug);
                }
                data_url
            }
            Err(_) => {
                let msg = "Failed to read local attachment; skipping it";
                if let Some(ctx) = context {
                    ctx.log_message(msg, flow_like::flow::execution::LogLevel::Warn);
                } else {
                    tracing::warn!("{}", msg);
                }
                // Return empty string instead of the Tauri URL to prevent "Unsupported scheme" errors
                String::new()
            }
        }
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
