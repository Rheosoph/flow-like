use super::flow_path::FlowPath;
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_storage::{
    Path, display_file_name,
    object_store::{ObjectMeta, ObjectStore, ObjectStoreExt},
};
use flow_like_types::anyhow;
use std::sync::Arc;

/// The first ranged read: enough for every signature below and for nearly every image header.
pub const PROBE_PREFIX_BYTES: u64 = 64 * 1024;

/// Magic numbers for the formats a flow actually receives from uploads, HTTP
/// bodies and storage reads. Order matters: longer signatures come first.
const SIGNATURES: [(&[u8], &str, &str); 17] = [
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
    (b"fLaC", "audio/flac", "flac"),
    (b"ID3", "audio/mpeg", "mp3"),
    (b"\x7fELF", "application/x-executable", "elf"),
];

const EBML_MAGIC: &[u8] = b"\x1a\x45\xdf\xa3";

/// Media type and usual extension of `bytes`, read from its leading magic number.
pub fn sniff_file_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
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
        return Some(match &bytes[8..12] {
            b"avif" | b"avis" => ("image/avif", "avif"),
            b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1" => {
                ("image/heic", "heic")
            }
            b"M4A " | b"M4B " | b"M4P " => ("audio/mp4", "m4a"),
            b"qt  " => ("video/quicktime", "mov"),
            _ => ("video/mp4", "mp4"),
        });
    }

    if bytes.starts_with(EBML_MAGIC) {
        let header = &bytes[..bytes.len().min(64)];
        return Some(if contains(header, b"webm") {
            ("video/webm", "webm")
        } else {
            ("video/x-matroska", "mkv")
        });
    }

    None
}

/// True when the first kilobyte reads as UTF-8 text without null bytes.
pub fn looks_like_text(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(1024)];
    if sample.is_empty() || sample.contains(&0) {
        return false;
    }
    match std::str::from_utf8(sample) {
        Ok(_) => true,
        Err(error) => error.error_len().is_none() && bytes.len() > sample.len(),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// The front of a stored file, fetched with ranged reads only as far as a parser needs it.
pub struct FilePrefix {
    store: Arc<dyn ObjectStore>,
    location: Path,
    meta: ObjectMeta,
    bytes: Vec<u8>,
}

impl FilePrefix {
    pub async fn open(
        path: &FlowPath,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<Self> {
        let runtime = path.to_runtime(context).await?;
        Self::open_in(runtime.store.as_generic(), runtime.path).await
    }

    pub async fn open_in(
        store: Arc<dyn ObjectStore>,
        location: Path,
    ) -> flow_like_types::Result<Self> {
        let meta = store
            .head(&location)
            .await
            .map_err(|error| anyhow!("Reading the metadata of '{location}' failed: {error}"))?;
        Ok(Self {
            store,
            location,
            meta,
            bytes: Vec::new(),
        })
    }

    pub fn meta(&self) -> &ObjectMeta {
        &self.meta
    }

    pub fn location(&self) -> &Path {
        &self.location
    }

    /// The decoded name of the file, e.g. `Übersicht (2).pdf`.
    pub fn file_name(&self) -> Option<String> {
        display_file_name(&self.location)
    }

    /// Everything fetched so far, always starting at byte 0.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn is_complete(&self) -> bool {
        self.bytes.len() as u64 >= self.meta.size
    }

    /// Extends the prefix to `len` bytes (or the whole file when it is shorter), fetching
    /// only the bytes not read yet.
    pub async fn fill_to(&mut self, len: u64) -> flow_like_types::Result<&[u8]> {
        let start = self.bytes.len() as u64;
        let end = len.min(self.meta.size);
        if end > start {
            let chunk = self
                .store
                .get_range(&self.location, start..end)
                .await
                .map_err(|error| {
                    anyhow!(
                        "Reading bytes {start}..{end} of '{}' failed: {error}",
                        self.location
                    )
                })?;
            if chunk.len() as u64 != end - start {
                return Err(anyhow!(
                    "Reading bytes {start}..{end} of '{}' returned {} bytes; the file changed while it was read",
                    self.location,
                    chunk.len()
                ));
            }
            self.bytes.extend_from_slice(&chunk);
        }
        Ok(&self.bytes)
    }

    pub async fn fill_all(&mut self) -> flow_like_types::Result<&[u8]> {
        self.fill_to(self.meta.size).await
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flow_like_storage::object_store::{PutPayload, memory::InMemory};

    pub(crate) async fn stored(bytes: Vec<u8>) -> FilePrefix {
        let store = Arc::new(InMemory::new());
        let location = Path::from("upload/file.bin");
        store.put(&location, PutPayload::from(bytes)).await.unwrap();
        FilePrefix::open_in(store, location).await.unwrap()
    }

    #[test]
    fn sniffs_container_brands_and_leading_magic() {
        assert_eq!(
            sniff_file_type(b"\x89PNG\r\n\x1a\n...."),
            Some(("image/png", "png"))
        );
        assert_eq!(
            sniff_file_type(b"\x00\x00\x00\x18ftypheic\x00\x00"),
            Some(("image/heic", "heic"))
        );
        assert_eq!(
            sniff_file_type(b"\x00\x00\x00\x18ftypisom\x00\x00"),
            Some(("video/mp4", "mp4"))
        );
        assert_eq!(
            sniff_file_type(b"\x1a\x45\xdf\xa3\x9f\x42\x82\x84webm"),
            Some(("video/webm", "webm"))
        );
        assert_eq!(sniff_file_type(b"hello"), None);
    }

    #[test]
    fn text_detection_tolerates_a_character_cut_at_the_sample_edge() {
        let mut text = "a".repeat(1023).into_bytes();
        text.extend_from_slice("ü and more".as_bytes());
        assert!(looks_like_text(&text));
        assert!(!looks_like_text(b"\x00\x01binary"));
        assert!(!looks_like_text(b""));
    }

    #[tokio::test]
    async fn fill_to_fetches_only_missing_bytes_and_stops_at_the_end() {
        let mut file = stored((0..=255).collect()).await;
        assert_eq!(
            file.fill_to(16).await.unwrap(),
            &(0..16).collect::<Vec<u8>>()[..]
        );
        assert!(!file.is_complete());
        file.fill_to(8).await.unwrap();
        assert_eq!(file.bytes().len(), 16);
        file.fill_to(10_000).await.unwrap();
        assert_eq!(file.bytes(), &(0..=255).collect::<Vec<u8>>()[..]);
        assert!(file.is_complete());
        assert_eq!(file.file_name().as_deref(), Some("file.bin"));
    }

    #[tokio::test]
    async fn an_empty_file_is_complete_without_a_read() {
        let mut file = stored(Vec::new()).await;
        assert!(file.fill_to(PROBE_PREFIX_BYTES).await.unwrap().is_empty());
        assert!(file.is_complete());
    }
}
