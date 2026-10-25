use flow_like::flow::execution::context::ExecutionContext;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_types::anyhow;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::flow_path::FlowPath;

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct Attachment {
    pub filename: Option<String>,
    pub content_type: String,
    pub data: Vec<u8>,
}

const MIME_TYPES: &[(&str, &str)] = &[
    ("pdf", "application/pdf"),
    ("doc", "application/msword"),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("xls", "application/vnd.ms-excel"),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("ppt", "application/vnd.ms-powerpoint"),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
    ("rtf", "application/rtf"),
    ("txt", "text/plain"),
    ("md", "text/markdown"),
    ("html", "text/html"),
    ("htm", "text/html"),
    ("csv", "text/csv"),
    ("ics", "text/calendar"),
    ("json", "application/json"),
    ("xml", "application/xml"),
    ("eml", "message/rfc822"),
    ("zip", "application/zip"),
    ("tar", "application/x-tar"),
    ("gz", "application/gzip"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("gif", "image/gif"),
    ("bmp", "image/bmp"),
    ("svg", "image/svg+xml"),
    ("webp", "image/webp"),
    ("heic", "image/heic"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("mp3", "audio/mpeg"),
    ("wav", "audio/wav"),
    ("m4a", "audio/mp4"),
    ("mp4", "video/mp4"),
    ("mov", "video/quicktime"),
    ("avi", "video/x-msvideo"),
    ("webm", "video/webm"),
];

/// The MIME type recipients see for `file_name`, chosen by its extension.
pub fn mime_type_for(file_name: &str) -> &'static str {
    let Some((_, extension)) = file_name.rsplit_once('.') else {
        return "application/octet-stream";
    };
    MIME_TYPES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(extension))
        .map_or("application/octet-stream", |&(_, mime)| mime)
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

fn ensure_total_within(sizes: &[u64], max_total_bytes: u64) -> flow_like_types::Result<()> {
    let total = sizes.iter().sum::<u64>();
    if total > max_total_bytes {
        return Err(anyhow!(
            "Attachments total {}, more than the {} allowed",
            mib(total),
            mib(max_total_bytes)
        ));
    }
    Ok(())
}

/// Reads `files` as attachments named after their files. Every size is checked against
/// `max_total_bytes` before the first download, so an oversized file is never loaded.
pub async fn read_file_attachments(
    context: &mut ExecutionContext,
    files: &[FlowPath],
    max_total_bytes: u64,
) -> flow_like_types::Result<Vec<Attachment>> {
    let mut sizes = Vec::with_capacity(files.len());
    for file in files {
        let runtime = file.to_runtime(context).await?;
        let meta = runtime
            .store
            .as_generic()
            .head(&runtime.path)
            .await
            .map_err(|error| anyhow!("Cannot read attachment '{}': {error}", file.path))?;
        sizes.push(meta.size);
    }
    ensure_total_within(&sizes, max_total_bytes)?;
    let mut attachments = Vec::with_capacity(files.len());
    for file in files {
        let filename = flow_like_storage::display_file_name(&file.object_path())
            .ok_or_else(|| anyhow!("Attachment FlowPath '{}' names no file", file.path))?;
        attachments.push(Attachment {
            content_type: mime_type_for(&filename).to_owned(),
            data: file.get(context, false).await?,
            filename: Some(filename),
        });
    }
    Ok(attachments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types_follow_the_extension_case_insensitively() {
        assert_eq!(mime_type_for("Report.PDF"), "application/pdf");
        assert_eq!(
            mime_type_for("q3.final.xlsx"),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        assert_eq!(mime_type_for("clip.mov"), "video/quicktime");
        for name in ["README", "archive.unknown", ".env"] {
            assert_eq!(mime_type_for(name), "application/octet-stream", "{name}");
        }
    }

    #[test]
    fn the_total_size_is_refused_before_any_download() {
        let limit = 3 * 1024 * 1024;
        assert!(ensure_total_within(&[], limit).is_ok());
        assert!(ensure_total_within(&[limit - 1, 1], limit).is_ok());
        assert!(ensure_total_within(&[limit, 1], limit).is_err());
        let error = ensure_total_within(&[limit, 512 * 1024], limit).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Attachments total 3.5 MiB, more than the 3.0 MiB allowed"
        );
    }
}
