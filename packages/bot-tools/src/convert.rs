use async_trait::async_trait;
use flow_like_types::{Bytes, Result};

pub struct ConvertInput<'a> {
    pub name: &'a str,
    pub mime: &'a str,
    pub bytes: &'a Bytes,
}

/// A document as text. `page_starts` are byte offsets into `text` where pages 1, 2, … begin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Converted {
    pub text: String,
    pub page_starts: Vec<usize>,
}

impl Converted {
    pub fn plain(text: String) -> Self {
        Self {
            text,
            page_starts: Vec::new(),
        }
    }

    pub fn page_at(&self, offset: usize) -> Option<i64> {
        if self.page_starts.is_empty() {
            return None;
        }
        let page = self.page_starts.partition_point(|start| *start <= offset);
        Some(page.max(1) as i64)
    }
}

/// Turns a file into text. Hosts back it with the markitdown service; text files never reach it.
#[async_trait]
pub trait DocumentConverter: Send + Sync {
    async fn convert(&self, input: ConvertInput<'_>) -> Result<Converted>;
}

const TEXT_MIMES: [&str; 9] = [
    "application/json",
    "application/xml",
    "application/yaml",
    "application/x-yaml",
    "application/toml",
    "application/javascript",
    "application/x-ndjson",
    "application/sql",
    "application/x-sh",
];

pub fn looks_like_text(bytes: &[u8], mime: &str) -> bool {
    let mime = mime.split(';').next().unwrap_or("").trim();
    let known_text = mime.starts_with("text/")
        || TEXT_MIMES.contains(&mime)
        || mime.ends_with("+json")
        || mime.ends_with("+xml");
    if known_text {
        return std::str::from_utf8(bytes).is_ok() || bytes.iter().all(|b| *b != 0);
    }
    if !(mime.is_empty() || mime == "application/octet-stream") {
        return false;
    }
    let head = &bytes[..bytes.len().min(8192)];
    !head.contains(&0)
        && match std::str::from_utf8(head) {
            Ok(_) => true,
            Err(error) => error.error_len().is_none(),
        }
}

pub fn decode_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
