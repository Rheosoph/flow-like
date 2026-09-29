use super::{Connection, auth, context::clean_label};
use crate::{
    credentials::temporary_prefixes, entity::event_sink, error::ApiError, state::AppState,
};
use flow_like::credentials::StoreType;
use flow_like_storage::{
    Path as StoragePath, files::store::FlowLikeStore, object_store::ObjectStoreExt,
};
use flow_like_types::{
    dispatch::REQUEST_FILES_STORE_REF,
    mime_guess,
    tokio::{self, time::Instant},
};
use reqwest::{StatusCode, Url};
use serde_json::{Map, Value, json};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const MAX_FILES: usize = 10;
const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
/// Also bounded by the shared deadline, which leaves room within Microsoft's ~15 s window.
const FILE_TIMEOUT: Duration = Duration::from_secs(8);
const SIGNED_URL_TTL: Duration = Duration::from_secs(48 * 3600);
const MAX_LABEL_CHARS: usize = 128;
const MAX_STORED_NAME: usize = 100;
const MAX_EXTENSION: usize = 16;
const OCTET_STREAM: &str = "application/octet-stream";
const DOWNLOAD_INFO: &str = "application/vnd.microsoft.teams.file.download.info";
const SHAREPOINT_SUFFIXES: [&str; 4] = [
    ".sharepoint.com",
    ".sharepoint.us",
    ".sharepoint.cn",
    ".sharepoint.de",
];
const SHARED_FILE: &str =
    "Files shared in channels and group chats cannot be downloaded by the bot";
const FOREIGN_IMAGE: &str =
    "The image is not hosted by Microsoft Teams, so the bot did not download it";
const FOREIGN_DOWNLOAD: &str = "Teams did not provide a SharePoint download link for this file, so the bot did not download it";
const TOO_LARGE: &str = "The file is larger than 25 MB";
const TOTAL_TOO_LARGE: &str = "The message's files exceed 50 MB in total";
const TIMED_OUT: &str = "The download did not finish in time";
const UNREACHABLE: &str = "Microsoft could not be reached to download this file";
const STORE_FAILED: &str = "Flow-Like could not store this file";

const IMAGE_TYPES: [&str; 8] = [
    "image/jpeg",
    "image/jpg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/heic",
    "image/heif",
    "image/svg+xml",
];
const IMAGE_EXTENSIONS: [&str; 8] = ["jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "svg"];
const AUDIO_TYPES: [&str; 16] = [
    "audio/wav",
    "audio/x-wav",
    "audio/wave",
    "audio/mp3",
    "audio/mpeg",
    "audio/mpeg3",
    "audio/aiff",
    "audio/x-aiff",
    "audio/aac",
    "audio/ogg",
    "audio/flac",
    "audio/m4a",
    "audio/x-m4a",
    "audio/mp4",
    "audio/pcm16",
    "audio/pcm24",
];
const AUDIO_EXTENSIONS: [&str; 10] = [
    "wav", "mp3", "aif", "aiff", "aac", "ogg", "flac", "m4a", "pcm16", "pcm24",
];
const VIDEO_TYPES: [&str; 7] = [
    "video/avi",
    "video/x-msvideo",
    "video/mp4",
    "video/mpeg",
    "video/mov",
    "video/quicktime",
    "video/webm",
];
const VIDEO_EXTENSIONS: [&str; 6] = ["avi", "mp4", "mpg", "mpeg", "mov", "webm"];
const DOCUMENT_TYPES: [&str; 18] = [
    "application/pdf",
    "text/plain",
    "text/rtf",
    "application/rtf",
    "text/html",
    "text/css",
    "text/markdown",
    "text/md",
    "text/x-markdown",
    "text/csv",
    "text/xml",
    "application/xml",
    "application/x-javascript",
    "application/javascript",
    "text/javascript",
    "text/x-javascript",
    "application/x-python",
    "text/x-python",
];
const DOCUMENT_EXTENSIONS: [&str; 14] = [
    "pdf", "txt", "rtf", "htm", "html", "css", "md", "markdown", "csv", "xml", "js", "mjs", "cjs",
    "py",
];

/// Where a Teams attachment's bytes can come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Source {
    /// Teams-hosted content; the only place the bot token is sent.
    Teams(Url),
    /// A pre-authorized SharePoint download URL, fetched without credentials.
    SharePoint(Url),
    Unavailable {
        link: Option<String>,
        error: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Candidate {
    pub name: Option<String>,
    /// As Teams declared it; may be a wildcard such as `image/*`.
    pub mime: String,
    pub source: Source,
}

/// One entry of `local_session.teams.message.files`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct FileEntry {
    pub name: String,
    pub mime: String,
    pub size: Option<u64>,
    pub url: Option<String>,
    pub path: Option<String>,
    pub downloadable: bool,
    pub link: Option<String>,
    pub error: Option<String>,
}

impl FileEntry {
    /// The entry before any download: what Teams reported and why it cannot be fetched.
    pub(super) fn pending(index: usize, candidate: &Candidate) -> Self {
        let (downloadable, link, error) = match &candidate.source {
            Source::Unavailable { link, error } => (false, link.clone(), Some(error.clone())),
            _ => (true, None, None),
        };
        Self {
            name: candidate
                .name
                .clone()
                .unwrap_or_else(|| fallback_name(index, &candidate.mime)),
            mime: candidate.mime.clone(),
            downloadable,
            link,
            error,
            ..Self::default()
        }
    }

    pub(super) fn context(&self) -> Value {
        let mut entry = Map::new();
        entry.insert("name".into(), json!(self.name));
        entry.insert("type".into(), json!(self.mime));
        if let Some(size) = self.size {
            entry.insert("size".into(), json!(size));
        }
        if let Some(url) = &self.url {
            entry.insert("url".into(), json!(url));
        }
        if let Some(path) = &self.path {
            entry.insert(
                "path".into(),
                json!({"path": path, "store_ref": REQUEST_FILES_STORE_REF, "cache_store_ref": null}),
            );
        }
        entry.insert("downloadable".into(), json!(self.downloadable));
        if let Some(link) = &self.link {
            entry.insert("link".into(), json!(link));
        }
        if let Some(error) = &self.error {
            entry.insert("error".into(), json!(error));
        }
        Value::Object(entry)
    }

    /// The Chat Event `attachments` item, for downloaded files only.
    pub(super) fn attachment(&self) -> Option<Value> {
        let url = self.url.as_ref()?;
        Some(json!({"url": url, "name": self.name, "type": self.mime, "size": self.size}))
    }

    fn placeholder(&self) -> String {
        let kind = if self.mime.starts_with("image/") {
            "image"
        } else {
            "file"
        };
        format!("[{kind}: {}]", self.name)
    }
}

/// History keeps files as `[image: …]` / `[file: …]` lines after the text.
pub(super) fn with_placeholders(text: &str, files: &[FileEntry]) -> String {
    files.iter().fold(text.to_owned(), |mut body, file| {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&file.placeholder());
        body
    })
}

/// The files of a message that Teams exposes to the bot, in attachment order.
pub(super) fn classify(activity: &Value) -> Vec<Candidate> {
    activity["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(candidate)
        .collect()
}

fn candidate(attachment: &Value) -> Option<Candidate> {
    let kind = attachment["contentType"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let name = attachment["name"].as_str().and_then(file_label);
    if kind == "text/html" || kind.starts_with("application/vnd.microsoft.card.") {
        None
    } else if kind == DOWNLOAD_INFO {
        Some(personal_file(attachment, name))
    } else if kind.starts_with("image/") {
        let url = attachment["contentUrl"].as_str()?;
        Some(image(kind, url, name))
    } else {
        shared_file(&kind, attachment["contentUrl"].as_str(), name)
    }
}

fn unavailable(link: Option<String>, error: &str) -> Source {
    Source::Unavailable {
        link,
        error: error.into(),
    }
}

fn mime_by_name(name: Option<&str>) -> Option<&'static str> {
    name.and_then(extension).and_then(mime_for_extension)
}

/// A file sent in a 1:1 chat, with a pre-authorized SharePoint download link.
fn personal_file(attachment: &Value, name: Option<String>) -> Candidate {
    let content = &attachment["content"];
    let mime = content["fileType"]
        .as_str()
        .and_then(mime_for_extension)
        .or_else(|| mime_by_name(name.as_deref()))
        .unwrap_or(OCTET_STREAM)
        .to_owned();
    let source = match content["downloadUrl"].as_str().and_then(sharepoint_url) {
        Some(url) => Source::SharePoint(url),
        None => unavailable(None, FOREIGN_DOWNLOAD),
    };
    Candidate { name, mime, source }
}

fn image(mime: String, content_url: &str, name: Option<String>) -> Candidate {
    let source = match auth::service_url(content_url) {
        Ok(url) => Source::Teams(url),
        Err(_) => unavailable(None, FOREIGN_IMAGE),
    };
    Candidate { name, mime, source }
}

/// A SharePoint or OneDrive file, which only the user can open.
fn shared_file(kind: &str, content_url: Option<&str>, name: Option<String>) -> Option<Candidate> {
    let link = content_url
        .and_then(|raw| Url::parse(raw).ok())
        .filter(|url| url.scheme() == "https");
    let shared = link
        .as_ref()
        .and_then(Url::host_str)
        .is_some_and(|host| sharepoint_host(host) || onedrive_host(host));
    if kind != "reference" && !shared {
        return None;
    }
    Some(Candidate {
        mime: mime_by_name(name.as_deref())
            .unwrap_or(OCTET_STREAM)
            .to_owned(),
        name,
        source: unavailable(link.map(String::from), SHARED_FILE),
    })
}

fn sharepoint_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    SHAREPOINT_SUFFIXES
        .iter()
        .any(|suffix| host.ends_with(suffix))
}

fn onedrive_host(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "onedrive.live.com" | "1drv.ms"
    )
}

/// A SharePoint or OneDrive for Business download URL that is safe to GET without credentials.
fn sharepoint_url(raw: &str) -> Option<Url> {
    let url = Url::parse(raw).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some_and(sharepoint_host)
        && url.port().is_none_or(|port| port == 443)
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}

/// The file's display name: the last path component on one line.
fn file_label(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    Some(clean_label(base, MAX_LABEL_CHARS)).filter(|name| !name.is_empty())
}

fn extension(name: &str) -> Option<&str> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty()
        && (1..=MAX_EXTENSION).contains(&ext.len())
        && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
    .then_some(ext)
}

fn mime_for_extension(ext: &str) -> Option<&'static str> {
    let ext = ext.trim().trim_start_matches('.');
    if ext.is_empty() {
        return None;
    }
    mime_guess::from_ext(&ext.to_ascii_lowercase()).first_raw()
}

fn extension_for_mime(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/heic" => "heic",
        "image/heif" => "heif",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        _ => return None,
    })
}

fn fallback_name(index: usize, mime: &str) -> String {
    let number = index + 1;
    if !mime.starts_with("image/") {
        return format!("file-{number}");
    }
    match extension_for_mime(mime) {
        Some(ext) => format!("image-{number}.{ext}"),
        None => format!("image-{number}"),
    }
}

/// An object-store-safe file name: ASCII letters, digits, `.`, `-` and `_`, at most 100 bytes,
/// keeping the extension.
fn stored_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let safe: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim_matches(['.', '_']);
    let (stem, ext) = match extension(safe) {
        Some(ext) => (&safe[..safe.len() - ext.len() - 1], Some(ext)),
        None => (safe, None),
    };
    let stem = stem.trim_end_matches('.');
    let stem = if stem.is_empty() { "file" } else { stem };
    let budget = MAX_STORED_NAME - ext.map_or(0, |ext| ext.len() + 1);
    let stem = &stem[..stem.len().min(budget)];
    match ext {
        Some(ext) => format!("{stem}.{ext}"),
        None => stem.to_owned(),
    }
}

fn concrete(mime: &str) -> bool {
    mime.contains('/') && !mime.ends_with("/*") && mime != OCTET_STREAM
}

fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Teams often declares pasted images as `image/*`, so a downloaded file's type comes from
/// the declaration, then its bytes, the response header and finally its name.
fn resolve_mime(declared: &str, name: Option<&str>, header: Option<&str>, bytes: &[u8]) -> String {
    let header = header.map(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
    });
    [
        Some(declared.to_ascii_lowercase()),
        sniff(bytes).map(str::to_owned),
        header,
        name.and_then(extension)
            .and_then(mime_for_extension)
            .map(str::to_owned),
    ]
    .into_iter()
    .flatten()
    .find(|mime| concrete(mime))
    .unwrap_or_else(|| OCTET_STREAM.to_owned())
}

fn has_extension(clean_url: &str, extensions: &[&str]) -> bool {
    clean_url
        .rsplit_once('.')
        .is_some_and(|(_, ext)| extensions.contains(&ext))
}

/// The part key for each media kind, checked in this order.
const MEDIA: [(&str, &[&str], &[&str]); 4] = [
    ("image_url", &IMAGE_TYPES, &IMAGE_EXTENSIONS),
    ("audio_url", &AUDIO_TYPES, &AUDIO_EXTENSIONS),
    ("video_url", &VIDEO_TYPES, &VIDEO_EXTENSIONS),
    ("document_url", &DOCUMENT_TYPES, &DOCUMENT_EXTENSIONS),
];

/// Known types first, the URL's extension when the type is unknown, then the type's top
/// level, so an attachment always reaches the model.
fn media_key(mime: &str, url: &str) -> &'static str {
    let clean_url = url
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let listed = MEDIA.iter().find(|(_, types, extensions)| {
        types.contains(&mime) || (mime.is_empty() && has_extension(&clean_url, extensions))
    });
    if let Some(&(key, ..)) = listed {
        return key;
    }
    match mime.split('/').next() {
        Some("image") => "image_url",
        Some("audio") => "audio_url",
        Some("video") => "video_url",
        _ => "document_url",
    }
}

/// One media part, chosen like the web chat's `createHistoryMessage`.
fn part(url: &str, mime: &str) -> Value {
    let mime = mime.to_ascii_lowercase();
    let key = media_key(&mime, url);
    let mut part = if key == "image_url" {
        json!({"type": key, key: {"url": url}})
    } else {
        json!({"type": key, key: url})
    };
    if !mime.is_empty() {
        let target = if key == "image_url" {
            &mut part[key]
        } else {
            &mut part
        };
        target["media_type"] = json!(mime);
    }
    part
}

/// The last history message's content when files were downloaded: the text, then one media
/// part per file.
pub(super) fn content_parts(text: &str, files: &[FileEntry]) -> Option<Value> {
    let mut parts = vec![json!({"type": "text", "text": text})];
    parts.extend(
        files
            .iter()
            .filter_map(|file| Some(part(file.url.as_deref()?, &file.mime))),
    );
    (parts.len() > 1).then_some(Value::Array(parts))
}

/// Bytes counted against the per-message total; released unless the download is kept.
struct Reservation<'a> {
    total: &'a AtomicU64,
    bytes: u64,
}

impl<'a> Reservation<'a> {
    fn new(total: &'a AtomicU64) -> Self {
        Self { total, bytes: 0 }
    }

    fn take(&mut self, bytes: u64) -> Result<(), String> {
        let before = self.total.fetch_add(bytes, Ordering::SeqCst);
        self.bytes += bytes;
        if before + bytes > MAX_TOTAL_BYTES {
            return Err(TOTAL_TOO_LARGE.into());
        }
        Ok(())
    }

    fn keep(mut self) {
        self.bytes = 0;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.total.fetch_sub(self.bytes, Ordering::SeqCst);
    }
}

struct Download {
    bytes: Vec<u8>,
    content_type: Option<String>,
}

/// A file that is not stored and signed by `deadline` reports that it ran out of time.
async fn within(
    deadline: Instant,
    work: impl Future<Output = Result<FileEntry, String>>,
) -> Result<FileEntry, String> {
    tokio::time::timeout_at(deadline, work)
        .await
        .map_err(|_| TIMED_OUT.to_owned())?
}

struct Fetcher<'a> {
    connection: &'a Connection,
    client: reqwest::Client,
    token: Option<String>,
    store: FlowLikeStore,
    prefix: String,
    run: &'a str,
    ttl: Duration,
    deadline: Instant,
    total: AtomicU64,
}

impl Fetcher<'_> {
    async fn download(&self, url: Url, token: Option<&str>) -> Result<Download, String> {
        let mut request = self.client.get(url);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let transport = |_| UNREACHABLE.to_owned();
        let mut response = request.send().await.map_err(transport)?;
        let status = response.status();
        if !status.is_success() {
            if token.is_some() && matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
            {
                auth::forget_bot_token(self.connection);
            }
            return Err(format!(
                "Microsoft returned HTTP {} for this file",
                status.as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_FILE_BYTES)
        {
            return Err(TOO_LARGE.into());
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut reservation = Reservation::new(&self.total);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport)? {
            if (bytes.len() + chunk.len()) as u64 > MAX_FILE_BYTES {
                return Err(TOO_LARGE.into());
            }
            reservation.take(chunk.len() as u64)?;
            bytes.extend_from_slice(&chunk);
        }
        reservation.keep();
        Ok(Download {
            bytes,
            content_type,
        })
    }

    /// Downloads, stores and signs one file, all within the shared deadline.
    async fn fetch(&self, index: usize, candidate: Candidate) -> Result<FileEntry, String> {
        let Candidate { name, mime, source } = candidate;
        let (url, token) = match source {
            Source::Teams(url) => {
                let token = self.token.as_deref().ok_or_else(|| {
                    "The bot could not sign in to Teams to download this file".to_owned()
                })?;
                (url, Some(token))
            }
            Source::SharePoint(url) => (url, None),
            Source::Unavailable { error, .. } => return Err(error),
        };
        within(self.deadline, self.keep(index, name, &mime, url, token)).await
    }

    async fn keep(
        &self,
        index: usize,
        name: Option<String>,
        declared: &str,
        url: Url,
        token: Option<&str>,
    ) -> Result<FileEntry, String> {
        let download = tokio::time::timeout(FILE_TIMEOUT, self.download(url, token))
            .await
            .map_err(|_| TIMED_OUT.to_owned())??;
        let mime = resolve_mime(
            declared,
            name.as_deref(),
            download.content_type.as_deref(),
            &download.bytes,
        );
        let name = name.unwrap_or_else(|| fallback_name(index, &mime));
        let path = format!(
            "{}/runs/{}/request/teams/{index:04}-{}",
            self.prefix,
            self.run,
            stored_name(&name)
        );
        let size = download.bytes.len() as u64;
        let object = StoragePath::from(path.as_str());
        let stored = |error: &dyn std::fmt::Display| {
            tracing::warn!(connection_id = %self.connection.id, path = %path, %error, "Could not store a Teams file");
            STORE_FAILED.to_owned()
        };
        self.store
            .as_generic()
            .put(&object, download.bytes.into())
            .await
            .map_err(|error| stored(&error))?;
        let url = self
            .store
            .sign("GET", &object, self.ttl)
            .await
            .map_err(|error| stored(&error))?;
        Ok(FileEntry {
            name,
            mime,
            size: Some(size),
            url: Some(url.to_string()),
            path: Some(path),
            downloadable: true,
            link: None,
            error: None,
        })
    }
}

struct Storage {
    store: FlowLikeStore,
    prefix: String,
    ttl: Duration,
}

/// The run's request-files area in the Tmp store, owned like the sink's other runs.
async fn storage(
    state: &AppState,
    sink: &event_sink::Model,
    app_id: &str,
) -> Result<Storage, ApiError> {
    let master = state.master_credentials().await?;
    let store = master.to_store_type(StoreType::Tmp).await?;
    let subject = crate::mail_ingress::sink_owner(state, sink)
        .await?
        .unwrap_or_else(|| format!("sink:{}", sink.id));
    Ok(Storage {
        store,
        prefix: temporary_prefixes(&subject, app_id).0,
        ttl: master.signing_ttl(SIGNED_URL_TTL),
    })
}

impl<'a> Fetcher<'a> {
    /// Resolves storage, the HTTP client and, for Teams-hosted files, the bot token. A
    /// missing token only fails the Teams-hosted files.
    async fn prepare(
        state: &AppState,
        c: &'a Connection,
        sink: &event_sink::Model,
        run: &'a str,
        deadline: Instant,
        needs_token: bool,
    ) -> Result<Self, &'static str> {
        let storage = storage(state, sink, &c.app_id).await.map_err(|error| {
            tracing::warn!(connection_id = %c.id, %error, "Could not prepare storage for Teams files");
            STORE_FAILED
        })?;
        let client = super::client().map_err(|error| {
            tracing::warn!(connection_id = %c.id, %error, "Could not create the Teams download client");
            UNREACHABLE
        })?;
        let token = if needs_token {
            auth::bot_token(state, c)
                .await
                .inspect_err(|error| {
                    tracing::warn!(connection_id = %c.id, %error, "Could not get a bot token to download Teams files")
                })
                .ok()
        } else {
            None
        };
        Ok(Self {
            connection: c,
            client,
            token,
            store: storage.store,
            prefix: storage.prefix,
            run,
            ttl: storage.ttl,
            deadline,
            total: AtomicU64::new(0),
        })
    }
}

/// Splits off the files to download, marking any beyond the first ten.
fn jobs(candidates: Vec<Candidate>, entries: &mut [FileEntry]) -> Vec<(usize, Candidate)> {
    let mut jobs = Vec::new();
    for (index, candidate) in candidates.into_iter().enumerate() {
        if matches!(candidate.source, Source::Unavailable { .. }) {
            continue;
        }
        if jobs.len() == MAX_FILES {
            entries[index].error = Some(format!(
                "Only the first {MAX_FILES} files of a message are downloaded"
            ));
            continue;
        }
        jobs.push((index, candidate));
    }
    jobs
}

/// Downloads up to 10 files, stores them for the run and signs a download URL for each.
/// Never fails: every problem becomes the affected entry's `error`.
pub(super) async fn collect(
    state: &AppState,
    c: &Connection,
    sink: &event_sink::Model,
    run: &str,
    candidates: Vec<Candidate>,
    deadline: Instant,
) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| FileEntry::pending(index, candidate))
        .collect();
    let jobs = jobs(candidates, &mut entries);
    if jobs.is_empty() {
        return entries;
    }
    let needs_token = jobs
        .iter()
        .any(|(_, candidate)| matches!(candidate.source, Source::Teams(_)));
    let prepared = tokio::time::timeout_at(
        deadline,
        Fetcher::prepare(state, c, sink, run, deadline, needs_token),
    )
    .await
    .unwrap_or(Err(TIMED_OUT));
    let fetcher = match prepared {
        Ok(fetcher) => fetcher,
        Err(error) => {
            for (index, _) in jobs {
                entries[index].error = Some(error.to_owned());
            }
            return entries;
        }
    };
    let results = futures::future::join_all(jobs.into_iter().map(|(index, candidate)| {
        let fetcher = &fetcher;
        async move { (index, fetcher.fetch(index, candidate).await) }
    }))
    .await;
    for (index, result) in results {
        match result {
            Ok(entry) => entries[index] = entry,
            Err(error) => entries[index].error = Some(error),
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn teams(url: &str) -> Source {
        Source::Teams(Url::parse(url).unwrap())
    }

    #[test]
    fn attachments_are_classified_by_where_their_bytes_live() {
        let candidates = classify(&json!({"attachments": [
            {"contentType": "text/html", "content": "<p>hi</p>"},
            {"contentType": "application/vnd.microsoft.card.adaptive", "content": {}},
            {"contentType": "image/*", "contentUrl": "https://smba.trafficmanager.net/emea/v3/attachments/a/views/original"},
            {"contentType": "image/png", "contentUrl": "https://attacker.invalid/x.png", "name": "x.png"},
            {"contentType": "image/png"},
            {"contentType": DOWNLOAD_INFO, "name": "Report.PDF", "content": {
                "downloadUrl": "https://contoso.sharepoint.com/personal/x/_layouts/15/download.aspx?UniqueId=1&tempauth=t",
                "fileType": "pdf"
            }},
            {"contentType": DOWNLOAD_INFO, "name": "notes.md", "content": {"downloadUrl": "https://sharepoint.com.attacker.invalid/x"}},
            {"contentType": "reference", "name": "Plan.docx", "contentUrl": "https://contoso.sharepoint.com/sites/team/Shared%20Documents/Plan.docx"},
            {"contentType": "application/octet-stream", "name": "sheet.xlsx", "contentUrl": "https://contoso-my.sharepoint.com/x/sheet.xlsx"},
            {"contentType": "reference", "name": "odd", "contentUrl": "http://contoso.sharepoint.com/odd"},
            {"contentType": "application/json", "contentUrl": "https://example.com/x.json"}
        ]}));
        assert_eq!(
            candidates,
            vec![
                Candidate {
                    name: None,
                    mime: "image/*".into(),
                    source: teams(
                        "https://smba.trafficmanager.net/emea/v3/attachments/a/views/original"
                    ),
                },
                Candidate {
                    name: Some("x.png".into()),
                    mime: "image/png".into(),
                    source: Source::Unavailable {
                        link: None,
                        error: FOREIGN_IMAGE.into()
                    },
                },
                Candidate {
                    name: Some("Report.PDF".into()),
                    mime: "application/pdf".into(),
                    source: Source::SharePoint(
                        Url::parse("https://contoso.sharepoint.com/personal/x/_layouts/15/download.aspx?UniqueId=1&tempauth=t").unwrap()
                    ),
                },
                Candidate {
                    name: Some("notes.md".into()),
                    mime: "text/markdown".into(),
                    source: Source::Unavailable {
                        link: None,
                        error: FOREIGN_DOWNLOAD.into()
                    },
                },
                Candidate {
                    name: Some("Plan.docx".into()),
                    mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                        .into(),
                    source: Source::Unavailable {
                        link: Some(
                            "https://contoso.sharepoint.com/sites/team/Shared%20Documents/Plan.docx"
                                .into()
                        ),
                        error: SHARED_FILE.into()
                    },
                },
                Candidate {
                    name: Some("sheet.xlsx".into()),
                    mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                        .into(),
                    source: Source::Unavailable {
                        link: Some("https://contoso-my.sharepoint.com/x/sheet.xlsx".into()),
                        error: SHARED_FILE.into()
                    },
                },
                Candidate {
                    name: Some("odd".into()),
                    mime: OCTET_STREAM.into(),
                    source: Source::Unavailable {
                        link: None,
                        error: SHARED_FILE.into()
                    },
                },
            ]
        );
        assert!(classify(&json!({"text":"no attachments"})).is_empty());
    }

    #[test]
    fn the_bot_token_only_goes_to_teams_hosts() {
        for url in [
            "https://smba.trafficmanager.net/amer/v3/attachments/x/views/original",
            "https://eu.botapi.skype.com/x",
            "https://x.teams.microsoft.com/v3/attachments/y",
        ] {
            let found =
                classify(&json!({"attachments":[{"contentType":"image/png","contentUrl":url}]}));
            assert!(matches!(found[0].source, Source::Teams(_)), "{url}");
        }
        for url in [
            "http://smba.trafficmanager.net/x",
            "https://smba.trafficmanager.net.attacker.invalid/x",
            "https://smba.trafficmanager.net:8443/x",
            "https://user@smba.trafficmanager.net/x",
            "https://graph.microsoft.com/v1.0/chats/x/messages/y/hostedContents/z/$value",
            "data:image/png;base64,AAAA",
        ] {
            let found =
                classify(&json!({"attachments":[{"contentType":"image/png","contentUrl":url}]}));
            assert!(
                matches!(found[0].source, Source::Unavailable { .. }),
                "{url}"
            );
        }
    }

    #[test]
    fn download_links_must_be_sharepoint_over_https() {
        for url in [
            "https://contoso.sharepoint.com/x?tempauth=1",
            "https://contoso-my.SharePoint.com/x",
            "https://agency.sharepoint.us/x",
            "https://contoso.sharepoint.cn/x",
            "https://contoso.sharepoint.de:443/x",
        ] {
            assert!(sharepoint_url(url).is_some(), "{url}");
        }
        for url in [
            "http://contoso.sharepoint.com/x",
            "https://sharepoint.com/x",
            "https://contoso.sharepoint.com.attacker.invalid/x",
            "https://evilsharepoint.com/x",
            "https://contoso.sharepoint.com:8443/x",
            "https://a:b@contoso.sharepoint.com/x",
            "not a url",
        ] {
            assert!(sharepoint_url(url).is_none(), "{url}");
        }
        assert!(onedrive_host("OneDrive.live.com"));
        assert!(!onedrive_host("onedrive.live.com.attacker.invalid"));
    }

    #[test]
    fn stored_names_are_safe_and_keep_their_extension() {
        for (name, expected) in [
            (
                "Quarterly Report (final).pdf",
                "Quarterly_Report__final_.pdf",
            ),
            ("../../etc/passwd", "passwd"),
            ("C:\\Users\\me\\scan.PNG", "scan.PNG"),
            ("Übersicht.xlsx", "bersicht.xlsx"),
            (".htaccess", "htaccess"),
            ("..", "file"),
            ("", "file"),
            ("évidence", "vidence"),
            ("a.tar.gz", "a.tar.gz"),
        ] {
            assert_eq!(stored_name(name), expected, "{name}");
        }
        let long = stored_name(&format!("{}.pdf", "a".repeat(300)));
        assert_eq!(long.len(), MAX_STORED_NAME);
        assert!(long.ends_with("a.pdf"));
        assert_eq!(
            file_label(" folder/My\nfile.txt "),
            Some("My file.txt".into())
        );
        assert_eq!(file_label("dir/"), None);
    }

    #[test]
    fn mime_types_come_from_extensions_and_image_bytes() {
        assert_eq!(mime_for_extension("pdf"), Some("application/pdf"));
        assert_eq!(mime_for_extension(".PNG"), Some("image/png"));
        assert_eq!(mime_for_extension("md"), Some("text/markdown"));
        assert_eq!(mime_for_extension(""), None);
        assert_eq!(mime_for_extension("definitely-unknown"), None);
        assert_eq!(extension("archive.tar.gz"), Some("gz"));
        assert_eq!(extension(".bashrc"), None);
        assert_eq!(extension("a.b c"), None);
        let png = b"\x89PNG\r\n\x1a\n rest";
        assert_eq!(resolve_mime("image/*", None, None, png), "image/png");
        assert_eq!(
            resolve_mime("image/*", None, Some("image/jpeg; charset=binary"), b"??"),
            "image/jpeg"
        );
        assert_eq!(
            resolve_mime("image/*", Some("a.webp"), Some(OCTET_STREAM), b"??"),
            "image/webp"
        );
        assert_eq!(
            resolve_mime("application/pdf", None, Some("image/png"), png),
            "application/pdf"
        );
        assert_eq!(resolve_mime("image/*", None, None, b"??"), OCTET_STREAM);
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a"), Some("image/gif"));
        assert_eq!(fallback_name(0, "image/jpeg"), "image-1.jpg");
        assert_eq!(fallback_name(2, "image/*"), "image-3");
        assert_eq!(fallback_name(1, "application/pdf"), "file-2");
    }

    #[test]
    fn parts_follow_the_web_chat_rules() {
        let url = "https://store.example/x/0000-a?sig=1";
        assert_eq!(
            part(url, "image/png"),
            json!({"type":"image_url","image_url":{"url":url,"media_type":"image/png"}})
        );
        assert_eq!(
            part(url, "audio/x-m4a"),
            json!({"type":"audio_url","audio_url":url,"media_type":"audio/x-m4a"})
        );
        assert_eq!(
            part(url, "video/quicktime"),
            json!({"type":"video_url","video_url":url,"media_type":"video/quicktime"})
        );
        assert_eq!(
            part(url, "text/csv"),
            json!({"type":"document_url","document_url":url,"media_type":"text/csv"})
        );
        assert_eq!(
            part(url, "image/bmp"),
            json!({"type":"image_url","image_url":{"url":url,"media_type":"image/bmp"}})
        );
        assert_eq!(part(url, "audio/opus")["type"], "audio_url");
        assert_eq!(part(url, "video/x-matroska")["type"], "video_url");
        assert_eq!(
            part(
                url,
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            )["type"],
            "document_url"
        );
        assert_eq!(
            part("https://s/x/photo.JPG?sig=1#f", ""),
            json!({"type":"image_url","image_url":{"url":"https://s/x/photo.JPG?sig=1#f"}})
        );
        assert_eq!(part("https://s/x/a.mp3", "")["type"], "audio_url");
        assert_eq!(part("https://s/x/a.mpeg", "")["type"], "video_url");
        assert_eq!(part("https://s/x/a.py", "")["type"], "document_url");
        assert_eq!(
            part("https://s/x/a.mp4.bin", ""),
            json!({"type":"document_url","document_url":"https://s/x/a.mp4.bin"})
        );
    }

    fn downloaded(name: &str, mime: &str) -> FileEntry {
        FileEntry {
            name: name.into(),
            mime: mime.into(),
            size: Some(4),
            url: Some(format!("https://store.example/{name}?sig=1")),
            path: Some(format!(
                "tmp/user/u/apps/a/runs/r/request/teams/0000-{name}"
            )),
            downloadable: true,
            ..FileEntry::default()
        }
    }

    #[test]
    fn entries_become_context_attachments_parts_and_placeholders() {
        let image = downloaded("photo.png", "image/png");
        let shared = FileEntry::pending(
            1,
            &Candidate {
                name: Some("Plan.docx".into()),
                mime: OCTET_STREAM.into(),
                source: Source::Unavailable {
                    link: Some("https://contoso.sharepoint.com/Plan.docx".into()),
                    error: SHARED_FILE.into(),
                },
            },
        );
        assert_eq!(
            image.context(),
            json!({
                "name": "photo.png", "type": "image/png", "size": 4,
                "url": "https://store.example/photo.png?sig=1",
                "path": {"path": "tmp/user/u/apps/a/runs/r/request/teams/0000-photo.png", "store_ref": REQUEST_FILES_STORE_REF, "cache_store_ref": null},
                "downloadable": true
            })
        );
        assert_eq!(
            shared.context(),
            json!({"name":"Plan.docx","type":OCTET_STREAM,"downloadable":false,"link":"https://contoso.sharepoint.com/Plan.docx","error":SHARED_FILE})
        );
        assert_eq!(
            image.attachment(),
            Some(
                json!({"url":"https://store.example/photo.png?sig=1","name":"photo.png","type":"image/png","size":4})
            )
        );
        assert_eq!(shared.attachment(), None);
        let files = [image.clone(), shared.clone()];
        assert_eq!(
            with_placeholders("look", &files),
            "look\n[image: photo.png]\n[file: Plan.docx]"
        );
        assert_eq!(with_placeholders("", &files[..1]), "[image: photo.png]");
        assert_eq!(with_placeholders("only text", &[]), "only text");
        assert_eq!(
            content_parts("look", &files),
            Some(json!([
                {"type":"text","text":"look"},
                {"type":"image_url","image_url":{"url":"https://store.example/photo.png?sig=1","media_type":"image/png"}}
            ]))
        );
        assert_eq!(content_parts("look", &[shared]), None);
        let pending = FileEntry::pending(
            0,
            &Candidate {
                name: None,
                mime: "image/*".into(),
                source: teams("https://smba.trafficmanager.net/x"),
            },
        );
        assert_eq!(
            pending,
            FileEntry {
                name: "image-1".into(),
                mime: "image/*".into(),
                downloadable: true,
                ..FileEntry::default()
            }
        );
    }

    #[test]
    fn at_most_ten_files_are_downloaded_and_unavailable_files_do_not_count() {
        let shared = Candidate {
            name: Some("Plan.docx".into()),
            mime: OCTET_STREAM.into(),
            source: unavailable(None, SHARED_FILE),
        };
        let image = Candidate {
            name: None,
            mime: "image/png".into(),
            source: teams("https://smba.trafficmanager.net/x"),
        };
        let candidates = std::iter::once(shared)
            .chain(std::iter::repeat_n(image, 11))
            .collect::<Vec<_>>();
        let mut entries = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| FileEntry::pending(index, candidate))
            .collect::<Vec<_>>();
        let jobs = jobs(candidates, &mut entries);
        assert_eq!(
            jobs.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            (1..=10).collect::<Vec<_>>()
        );
        assert_eq!(
            entries[11].error.as_deref(),
            Some("Only the first 10 files of a message are downloaded")
        );
        assert_eq!(entries[11].name, "image-12.png");
        assert_eq!(entries[0].error.as_deref(), Some(SHARED_FILE));
        assert!(entries[1..11].iter().all(|entry| entry.error.is_none()));
    }

    #[tokio::test(start_paused = true)]
    async fn storing_and_signing_count_against_the_shared_deadline() {
        let deadline = Instant::now() + Duration::from_secs(10);
        let stored = downloaded("photo.png", "image/png");
        let slow_store = async {
            tokio::time::sleep(Duration::from_secs(9)).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(downloaded("late.png", "image/png"))
        };
        assert_eq!(within(deadline, slow_store).await, Err(TIMED_OUT.into()));
        assert_eq!(
            within(Instant::now() + Duration::from_secs(1), async {
                Ok(stored.clone())
            })
            .await,
            Ok(stored)
        );
        assert_eq!(
            within(deadline, async { Err(STORE_FAILED.to_owned()) }).await,
            Err(STORE_FAILED.into())
        );
    }

    #[test]
    fn the_total_size_guard_releases_failed_downloads() {
        let total = AtomicU64::new(0);
        let mut first = Reservation::new(&total);
        first.take(MAX_TOTAL_BYTES - 10).unwrap();
        first.keep();
        {
            let mut second = Reservation::new(&total);
            second.take(5).unwrap();
            assert_eq!(second.take(6), Err(TOTAL_TOO_LARGE.into()));
        }
        assert_eq!(total.load(Ordering::SeqCst), MAX_TOTAL_BYTES - 10);
        let mut third = Reservation::new(&total);
        third.take(10).unwrap();
        third.keep();
        assert_eq!(total.load(Ordering::SeqCst), MAX_TOTAL_BYTES);
    }
}
