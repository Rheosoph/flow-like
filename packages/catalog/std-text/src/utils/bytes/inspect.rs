use super::ops::{bytes_input, bytes_node};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Magic numbers for the formats a flow actually receives from uploads, HTTP
/// bodies and storage reads. Order matters: longer signatures come first.
const SIGNATURES: [(&[u8], &str, &str); 16] = [
    (b"\x89PNG\r\n\x1a\n", "image/png", "png"),
    (b"GIF89a", "image/gif", "gif"),
    (b"GIF87a", "image/gif", "gif"),
    (b"%PDF-", "application/pdf", "pdf"),
    (b"\xff\xd8\xff", "image/jpeg", "jpg"),
    (b"II*\x00", "image/tiff", "tiff"),
    (b"MM\x00*", "image/tiff", "tiff"),
    (b"BM", "image/bmp", "bmp"),
    (b"\x1f\x8b", "application/gzip", "gz"),
    (b"PK\x03\x04", "application/zip", "zip"),
    (b"Rar!\x1a\x07", "application/vnd.rar", "rar"),
    (b"7z\xbc\xaf\x27\x1c", "application/x-7z-compressed", "7z"),
    (b"SQLite format 3\x00", "application/vnd.sqlite3", "sqlite"),
    (b"OggS", "audio/ogg", "ogg"),
    (b"ID3", "audio/mpeg", "mp3"),
    (b"\x7fELF", "application/x-executable", "elf"),
];

fn sniff(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if let Some((_, mime, extension)) = SIGNATURES
        .iter()
        .find(|(signature, _, _)| bytes.starts_with(signature))
    {
        return Some((mime, extension));
    }

    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" {
        return match &bytes[8..12] {
            b"WEBP" => Some(("image/webp", "webp")),
            b"WAVE" => Some(("audio/wav", "wav")),
            b"AVI " => Some(("video/x-msvideo", "avi")),
            _ => None,
        };
    }

    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return Some(("video/mp4", "mp4"));
    }

    None
}

fn looks_like_text(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(1024)];
    !sample.is_empty() && std::str::from_utf8(sample).is_ok() && !sample.contains(&0)
}

#[crate::register_node]
#[derive(Default)]
pub struct BytesDetectTypeNode {}

impl BytesDetectTypeNode {
    pub fn new() -> Self {
        BytesDetectTypeNode {}
    }
}

#[async_trait]
impl NodeLogic for BytesDetectTypeNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "bytes_detect_type",
            "Detect Type",
            "Reads the leading bytes to work out what kind of file a buffer holds",
        );
        node.set_flowscript_name("bytes", "detectType");
        node.set_receiver("bytes");
        bytes_input(&mut node, "bytes", "Bytes", "Input Bytes");

        node.add_output_pin(
            "mime_type",
            "MIME Type",
            "Detected media type, empty when nothing matched",
            VariableType::String,
        );
        node.add_output_pin(
            "extension",
            "Extension",
            "Usual file extension for the detected type",
            VariableType::String,
        );
        node.add_output_pin(
            "detected",
            "Detected",
            "True when a signature matched",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "is_text",
            "Is Text",
            "True when the first kilobyte reads as UTF-8 text without null bytes",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let bytes: Vec<u8> = context.evaluate_pin("bytes").await?;

        let detected = sniff(&bytes);
        let is_text = detected.is_none() && looks_like_text(&bytes);
        let (mime, extension) = match detected {
            Some((mime, extension)) => (mime.to_string(), extension.to_string()),
            None if is_text => ("text/plain".to_string(), "txt".to_string()),
            None => (String::new(), String::new()),
        };

        context.set_pin_value("mime_type", json!(mime)).await?;
        context.set_pin_value("extension", json!(extension)).await?;
        context
            .set_pin_value("detected", json!(detected.is_some()))
            .await?;
        context.set_pin_value("is_text", json!(is_text)).await?;
        Ok(())
    }
}
