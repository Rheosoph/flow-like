//! Attachment resolution and temporary image files.

use copilot_sdk::{AttachmentType, UserMessageAttachment};
use flow_like::copilot::ChatImage;
use flow_like::flow::copilot::AttachmentManifestEntry;
use flow_like_bot_tools::{FileData, FileLink};
use flow_like_types::bot_message::WorkspaceHandle;
use flow_like_types::images::{
    PORTABLE_INLINE_IMAGE_BYTES, PORTABLE_INLINE_IMAGES_BYTES, fit_image_within,
};
use std::path::PathBuf;

fn copilot_attachment_extension(media_type: &str) -> &'static str {
    match media_type.to_lowercase().as_str() {
        "image/jpeg" | "jpeg" | "jpg" => "jpg",
        "image/png" | "png" => "png",
        "image/gif" | "gif" => "gif",
        "image/webp" | "webp" => "webp",
        _ => "bin",
    }
}

const MAX_PROMPT_IMAGE_BYTES: usize = 64 * 1024 * 1024;

/// Decode base64 prompt images and persist them as hash-deduped temp files.
/// Shared by every provider that attaches images by path (GitHub Copilot
/// SDK attachments, Codex `--image` flags).
pub(super) fn write_chat_image_temp_files(
    images: &[ChatImage],
) -> Result<Vec<std::path::PathBuf>, String> {
    use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};

    let attachment_dir = std::env::temp_dir().join("flow-like-copilot-attachments");
    std::fs::create_dir_all(&attachment_dir)
        .map_err(|e| format!("Failed to create attachment directory: {}", e))?;

    images
        .iter()
        .enumerate()
        .map(|(index, image)| {
            // Bound the decoded size before allocating: base64 inflates by 4/3.
            let estimated_bytes = image.data.len() / 4 * 3;
            if estimated_bytes > MAX_PROMPT_IMAGE_BYTES {
                return Err(format!(
                    "Prompt image {} is too large ({} MB, max {} MB)",
                    index + 1,
                    estimated_bytes / (1024 * 1024),
                    MAX_PROMPT_IMAGE_BYTES / (1024 * 1024)
                ));
            }
            let bytes = STANDARD
                .decode(&image.data)
                .map_err(|e| format!("Failed to decode prompt image {}: {}", index + 1, e))?;
            let extension = copilot_attachment_extension(&image.media_type);
            let file_name = format!("{}.{}", blake3::hash(&bytes).to_hex(), extension);
            let file_path = attachment_dir.join(file_name);

            if !file_path.exists() {
                std::fs::write(&file_path, &bytes).map_err(|e| {
                    format!("Failed to write attachment {}: {}", file_path.display(), e)
                })?;
            }

            Ok(file_path)
        })
        .collect()
}

pub(super) fn build_copilot_attachments(
    images: &[ChatImage],
) -> Result<Vec<UserMessageAttachment>, String> {
    let paths = write_chat_image_temp_files(images)?;
    Ok(paths
        .into_iter()
        .enumerate()
        .map(|(index, file_path)| {
            let extension = file_path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_else(|| "bin".to_string());
            UserMessageAttachment {
                attachment_type: AttachmentType::File,
                path: file_path.to_string_lossy().into_owned(),
                display_name: format!("prompt-image-{}.{}", index + 1, extension),
            }
        })
        .collect())
}

fn attachment_media_type(url: &str) -> String {
    let name_hint = url
        .split_once("filename=")
        .map(|(_, rest)| rest.split('&').next().unwrap_or(rest))
        .map(|encoded| urlencoding::decode(encoded).unwrap_or_default().to_string())
        .unwrap_or_else(|| url.split('?').next().unwrap_or(url).to_string());

    match name_hint.rsplit('.').next().map(str::to_ascii_lowercase) {
        Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg".to_string(),
        Some(ext) if ext == "gif" => "image/gif".to_string(),
        Some(ext) if ext == "webp" => "image/webp".to_string(),
        _ => "image/png".to_string(),
    }
}

/// Convert a Tauri asset-protocol URL (produced by `convertFileSrc`) back to the local file path.
fn local_asset_path(url: &str) -> Option<PathBuf> {
    let without_query = url.split('?').next().unwrap_or(url);
    let encoded_path = without_query
        .strip_prefix("asset://localhost/")
        .or_else(|| {
            without_query
                .split_once("asset.localhost/")
                .map(|(_, rest)| rest)
        })?;
    let decoded = urlencoding::decode(encoded_path).ok()?.to_string();
    // On unix the leading slash is consumed by the host split; restore it when missing.
    let path = if decoded.starts_with('/') || decoded.contains(":\\") || decoded.contains(":/") {
        PathBuf::from(decoded)
    } else {
        PathBuf::from(format!("/{decoded}"))
    };
    path.is_file().then_some(path)
}

/// A chat attachment's bytes and full size, from a local asset URL or an http(s) link. The bytes
/// stay empty when the file is larger than `max_bytes`, so the caller can name its size.
pub(crate) async fn read_chat_attachment(
    url: &str,
    max_bytes: u64,
) -> Result<(Vec<u8>, u64), String> {
    if let Some(path) = local_asset_path(url) {
        let size = tokio::fs::metadata(&path)
            .await
            .map_err(|error| format!("Reading the attachment failed: {error}"))?
            .len();
        if size > max_bytes {
            return Ok((Vec::new(), size));
        }
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|error| format!("Reading the attachment failed: {error}"))?;
        return Ok((bytes, size));
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("The attachment is neither a local file nor an HTTP(S) link".into());
    }
    let mut response = flow_like_types::reqwest::get(url)
        .await
        .map_err(|error| format!("Downloading the attachment failed: {}", error.without_url()))?;
    if !response.status().is_success() {
        return Err(format!(
            "Downloading the attachment failed (HTTP {})",
            response.status().as_u16()
        ));
    }
    if let Some(size) = response.content_length().filter(|size| *size > max_bytes) {
        return Ok((Vec::new(), size));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Downloading the attachment failed: {}", error.without_url()))?
    {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > max_bytes {
            return Ok((Vec::new(), bytes.len() as u64));
        }
    }
    let size = bytes.len() as u64;
    Ok((bytes, size))
}

/// The conversation id in the handle of a file attached to the message being answered. It names
/// no stored conversation: the run serves these files from the chat's upload.
pub(crate) const CHAT_FILES: &str = flow_like::flow::copilot::TURN_FILES;

/// The handle a file tool reads `entry` with.
pub(crate) fn chat_file_handle(entry: &AttachmentManifestEntry) -> Option<WorkspaceHandle> {
    let name = entry.name.as_deref().filter(|name| !name.is_empty())?;
    Some(WorkspaceHandle::conversation_file(
        CHAT_FILES,
        name,
        entry.size.unwrap_or_default(),
        "",
        entry.mime_type.as_deref().unwrap_or_default(),
    ))
}

/// The attachments, each with its handle: the stored file's when the message carries one, else a
/// handle for this turn when it has a link to read and a name no other attachment shares.
pub(super) fn with_chat_file_handles(
    attachments: &[AttachmentManifestEntry],
) -> Vec<AttachmentManifestEntry> {
    attachments
        .iter()
        .map(|entry| {
            let unique = attachments
                .iter()
                .filter(|other| other.name == entry.name)
                .count()
                == 1;
            AttachmentManifestEntry {
                handle: entry.handle.clone().or_else(|| {
                    (unique && entry.url.is_some())
                        .then(|| chat_file_handle(entry))
                        .flatten()
                }),
                ..entry.clone()
            }
        })
        .collect()
}

/// A download link to an attachment of this turn: its upload's web address, when it has one.
pub(crate) fn chat_file_link(
    attachments: &[AttachmentManifestEntry],
    path: &str,
) -> Option<FileLink> {
    attachments
        .iter()
        .find(|entry| entry.name.as_deref() == Some(path))
        .and_then(|entry| entry.url.as_deref())
        .and_then(FileLink::web)
}

/// The file a [`chat_file_handle`] names, read from the chat's upload.
pub(crate) async fn read_chat_file(
    attachments: &[AttachmentManifestEntry],
    path: &str,
    max_bytes: u64,
) -> flow_like_types::Result<FileData> {
    let entry = attachments
        .iter()
        .find(|entry| entry.name.as_deref() == Some(path))
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "'{path}' is not attached to the message being answered. A file from an earlier message can no longer be read: ask for it again, or use its copy in the workspace if one was saved"
            )
        })?;
    let url = entry.url.as_deref().ok_or_else(|| {
        flow_like_types::anyhow!("The attachment '{path}' has no link to read it from")
    })?;
    let (bytes, size) = read_chat_attachment(url, max_bytes)
        .await
        .map_err(|error| flow_like_types::anyhow!("{path}: {error}"))?;
    Ok(FileData {
        bytes: bytes.into(),
        size,
        mime: entry.mime_type.clone().filter(|mime| !mime.is_empty()),
        classification: None,
    })
}

/// Maximum size of a single fetched attachment; larger ones are skipped to bound memory use.
const MAX_ATTACHMENT_BYTES: u64 = 512 * 1024 * 1024;

/// Resolve chat attachment URLs into the images the model receives. A presigned http(s) upload
/// stays a link: the model's media handling hands it to a provider that fetches links and inlines
/// it, fitted to the provider's limits, for any other. A local file (asset protocol) is inlined
/// here, fitted into its share of [`PORTABLE_INLINE_IMAGES_BYTES`] so a call through the API's
/// model proxy stays within its request limit.
pub(super) async fn resolve_attachment_images(urls: &[String]) -> Vec<ChatImage> {
    let local = urls
        .iter()
        .filter(|url| local_asset_path(url).is_some())
        .count();
    let share = inline_share(local);
    let mut images = Vec::with_capacity(urls.len());
    for url in urls {
        let media_type = attachment_media_type(url);
        if let Some(path) = local_asset_path(url) {
            match tokio::fs::read(&path).await {
                Ok(bytes) => images.extend(inline_image(bytes, media_type, share).await),
                Err(error) => {
                    eprintln!("[global_chat] failed to read local attachment: {error}");
                }
            }
        } else if url.starts_with("http://") || url.starts_with("https://") {
            images.push(ChatImage::link(url.clone(), media_type));
        }
    }
    images
}

/// The images with every link downloaded and inlined, for agents that read image files rather
/// than links (Codex, Claude Code, GitHub Copilot).
pub(super) async fn inline_image_links(images: Option<Vec<ChatImage>>) -> Option<Vec<ChatImage>> {
    let images = images?;
    let links = images.iter().filter(|image| image.url.is_some()).count();
    if links == 0 {
        return Some(images);
    }
    let share = inline_share(links);
    let mut inlined = Vec::with_capacity(images.len());
    for image in images {
        let Some(url) = image.url.as_deref() else {
            inlined.push(image);
            continue;
        };
        if let Some(bytes) = download_attachment(url).await {
            inlined.extend(inline_image(bytes, image.media_type, share).await);
        }
    }
    (!inlined.is_empty()).then_some(inlined)
}

/// The bytes each of `count` images inlined together may take.
fn inline_share(count: usize) -> usize {
    PORTABLE_INLINE_IMAGE_BYTES.min(PORTABLE_INLINE_IMAGES_BYTES / count.max(1))
}

/// The image inlined as base64, fitted into `max_bytes`; `None` when it fits at no size.
async fn inline_image(bytes: Vec<u8>, media_type: String, max_bytes: usize) -> Option<ChatImage> {
    use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};

    let fitted = tokio::task::spawn_blocking(move || {
        let fitted = fit_image_within(&bytes, max_bytes);
        (bytes, fitted)
    })
    .await;
    match fitted {
        Ok((_, Ok(Some(fitted)))) => Some(ChatImage::inline(
            STANDARD.encode(&fitted.bytes),
            fitted.media_type.to_owned(),
        )),
        Ok((bytes, Ok(None))) => Some(ChatImage::inline(STANDARD.encode(&bytes), media_type)),
        Ok((_, Err(error))) => {
            eprintln!(
                "[global_chat] attachment image skipped, it does not fit {max_bytes} bytes: {error}"
            );
            None
        }
        Err(error) => {
            eprintln!("[global_chat] attachment image skipped, fitting it failed: {error}");
            None
        }
    }
}

async fn download_attachment(url: &str) -> Option<Vec<u8>> {
    let response = match flow_like_types::reqwest::get(url).await {
        Ok(response) => response,
        Err(error) => {
            eprintln!(
                "[global_chat] failed to fetch attachment: {}",
                error.without_url()
            );
            return None;
        }
    };
    // Reject oversized attachments by Content-Length before buffering the body into memory (a
    // malicious URL could otherwise OOM the process).
    if response
        .content_length()
        .is_some_and(|len| len > MAX_ATTACHMENT_BYTES)
    {
        eprintln!("[global_chat] attachment exceeds size limit, skipped");
        return None;
    }
    match response.bytes().await {
        Ok(bytes) if bytes.len() as u64 <= MAX_ATTACHMENT_BYTES => Some(bytes.to_vec()),
        Ok(_) => {
            eprintln!("[global_chat] attachment exceeds size limit, skipped");
            None
        }
        Err(error) => {
            eprintln!(
                "[global_chat] failed to read attachment: {}",
                error.without_url()
            );
            None
        }
    }
}
