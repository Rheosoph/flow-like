use super::{Connection, auth, context::clean_label};
use crate::{
    credentials::temporary_prefixes, entity::event_sink, error::ApiError, state::AppState,
};
use flow_like::credentials::StoreType;
use flow_like_storage::{
    Path as StoragePath,
    files::store::FlowLikeStore,
    object_store::{Attribute, ObjectStore, ObjectStoreExt, PutOptions},
};
use flow_like_types::{
    dispatch::REQUEST_FILES_STORE_REF,
    mime_guess,
    tokio::{self, time::Instant},
};
use reqwest::{StatusCode, Url};
use serde_json::{Map, Value, json};
use std::{
    collections::HashSet,
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
const MAX_HTML_BYTES: usize = 64 * 1024;
const MAX_AMS_ID: usize = 128;
const OCTET_STREAM: &str = "application/octet-stream";
const DOWNLOAD_INFO: &str = "application/vnd.microsoft.teams.file.download.info";
const SHAREPOINT_SUFFIXES: [&str; 4] = [
    ".sharepoint.com",
    ".sharepoint.us",
    ".sharepoint.cn",
    ".sharepoint.de",
];
/// Azure Media Services object hosts, which reject every app token.
const AMS_HOST_SUFFIXES: [&str; 2] = [".asm.skype.com", ".asyncgw.teams.microsoft.com"];
const SHARED_FILE: &str =
    "Files shared in channels and group chats cannot be downloaded by the bot";
const FOREIGN_IMAGE: &str = "Teams did not provide a download link the bot can use for this image";
const FOREIGN_DOWNLOAD: &str = "Teams did not provide a SharePoint download link for this file, so the bot did not download it";
const NO_DOWNLOAD_LINK: &str = "Teams did not include a download link for this file (Teams mobile does this for files picked from OneDrive or SharePoint); send it from Teams desktop or web, or attach a local copy";
const INLINE_MEDIA: &str = "Teams does not let bots download inline videos or stickers";
const TOO_LARGE: &str = "The file is larger than 25 MB";
const TOTAL_TOO_LARGE: &str = "The message's files exceed 50 MB in total";
const TIMED_OUT: &str = "The download did not finish in time";
const UNREACHABLE: &str = "Microsoft could not be reached to download this file";
const STORE_FAILED: &str = "Flow-Like could not store this file";
const SIGN_FAILED: &str = "Flow-Like stored this file but could not create a download link";

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
    /// A Teams-hosted view Teams declared; with `TeamsDerived`, the only places the bot token
    /// is sent.
    Teams(Url),
    /// The Bot Connector view of an image only the message's HTML names. It gets the bot token
    /// too, but Microsoft may refuse bot tokens for the object itself.
    TeamsDerived(Url),
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
    /// Where the user can open the file in SharePoint or OneDrive; the bot never fetches it.
    pub link: Option<String>,
    /// The host of the URL the bot declined to fetch. Logs may name it, never the URL, whose
    /// query can carry credentials.
    rejected_host: Option<String>,
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
            Source::Unavailable { link, error } => (false, link.as_ref(), Some(error.clone())),
            _ => (true, None, None),
        };
        Self {
            name: candidate
                .name
                .clone()
                .unwrap_or_else(|| fallback_name(index, &candidate.mime)),
            mime: candidate.mime.clone(),
            downloadable,
            link: link.or(candidate.link.as_ref()).cloned(),
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

/// The stored files of a message as its history entry keeps them: object paths, never links.
pub(super) fn stored_refs(files: &[FileEntry]) -> Vec<Value> {
    files
        .iter()
        .filter_map(|file| {
            let path = file.path.as_ref()?;
            Some(json!({"path": path, "name": file.name, "type": file.mime, "size": file.size}))
        })
        .collect()
}

/// Whether a stored file becomes a media part once it is signed again.
pub(super) fn linkable(reference: &Value) -> bool {
    let Some(path) = reference["path"].as_str() else {
        return false;
    };
    let mime = reference["type"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    media_key(&mime, path).is_some()
}

/// A file an earlier message stored under this app's runs, back from its history entry.
fn stored_file(reference: &Value, runs: &str) -> Option<FileEntry> {
    let path = reference["path"]
        .as_str()
        .filter(|path| path.starts_with(runs))?;
    Some(FileEntry {
        name: reference["name"].as_str()?.to_owned(),
        mime: reference["type"].as_str().unwrap_or_default().to_owned(),
        size: reference["size"].as_u64(),
        path: Some(path.to_owned()),
        downloadable: true,
        ..FileEntry::default()
    })
}

/// The files of a message that Teams exposes to the bot: its attachments in order, then the
/// media that only the message's HTML carries. `collect` limits how many are downloaded.
pub(super) fn classify(activity: &Value) -> Vec<Candidate> {
    let attachments = activity["attachments"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut candidates: Vec<Candidate> = attachments.iter().filter_map(candidate).collect();
    let image_urls: Vec<&str> = attachments
        .iter()
        .filter(|attachment| content_type(attachment).starts_with("image/"))
        .filter_map(|attachment| attachment["contentUrl"].as_str())
        .collect();
    let mut seen: HashSet<String> = image_urls
        .iter()
        .filter_map(|url| segment_after(url, "/v3/attachments/"))
        .map(str::to_ascii_lowercase)
        .collect();
    let service_url = activity["serviceUrl"].as_str();
    let html = attachments
        .iter()
        .filter(|attachment| content_type(attachment) == "text/html")
        .filter_map(|attachment| attachment["content"].as_str());
    for (kind, id) in html.flat_map(inline_media) {
        let id = id.filter(|id| ams_id(id));
        if id.is_some_and(|id| !seen.insert(id.to_ascii_lowercase())) {
            continue;
        }
        let number = candidates.len() + 1;
        let candidate = match kind {
            Inline::Image if id.is_none() && !image_urls.is_empty() => continue,
            Inline::Image => {
                let source = inline_image(service_url, id);
                Candidate {
                    name: None,
                    mime: "image/*".into(),
                    rejected_host: rejected_host(&source, id.and(service_url)),
                    source,
                    link: None,
                }
            }
            Inline::Video => not_downloadable(format!("video-{number}"), "video/*"),
            Inline::Sticker => not_downloadable(format!("sticker-{number}"), "image/*"),
        };
        candidates.push(candidate);
    }
    candidates
}

fn content_type(attachment: &Value) -> String {
    attachment["contentType"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn candidate(attachment: &Value) -> Option<Candidate> {
    let kind = content_type(attachment);
    if kind == "text/html" || kind.starts_with("application/vnd.microsoft.card.") {
        return None;
    }
    let name = attachment["name"].as_str().and_then(file_label);
    let content_url = attachment["contentUrl"].as_str();
    let (mime, source, fetched) = if kind == DOWNLOAD_INFO {
        let content = &attachment["content"];
        let (mime, source) = personal_file(content, name.as_deref());
        (mime, source, content["downloadUrl"].as_str())
    } else if kind.starts_with("image/") {
        (kind, image(content_url?), content_url)
    } else {
        let (mime, source) = shared_file(&kind, content_url, name.as_deref())?;
        (mime, source, None)
    };
    Some(Candidate {
        name,
        mime,
        rejected_host: rejected_host(&source, fetched),
        source,
        link: browsable_link(content_url),
    })
}

fn unavailable(link: Option<String>, error: &str) -> Source {
    Source::Unavailable {
        link,
        error: error.into(),
    }
}

/// The host of `fetched` when the bot declined to download from it.
fn rejected_host(source: &Source, fetched: Option<&str>) -> Option<String> {
    match source {
        Source::Unavailable { .. } => fetched.and_then(url_host),
        _ => None,
    }
}

fn url_host(raw: &str) -> Option<String> {
    Url::parse(raw).ok()?.host_str().map(str::to_owned)
}

fn mime_by_name(name: Option<&str>) -> Option<&'static str> {
    name.and_then(extension).and_then(mime_for_extension)
}

/// A file sent in a 1:1 chat, with a pre-authorized SharePoint download link.
fn personal_file(content: &Value, name: Option<&str>) -> (String, Source) {
    let mime = content["fileType"]
        .as_str()
        .and_then(mime_for_extension)
        .or_else(|| mime_by_name(name))
        .unwrap_or(OCTET_STREAM)
        .to_owned();
    let download_url = content["downloadUrl"]
        .as_str()
        .map(str::trim)
        .filter(|raw| !raw.is_empty());
    let source = match download_url {
        None => unavailable(None, NO_DOWNLOAD_LINK),
        Some(raw) => sharepoint_url(raw)
            .map_or_else(|| unavailable(None, FOREIGN_DOWNLOAD), Source::SharePoint),
    };
    (mime, source)
}

fn image(content_url: &str) -> Source {
    media_url(content_url).map_or_else(|| unavailable(None, FOREIGN_IMAGE), Source::Teams)
}

fn ams_host(host: &str) -> bool {
    AMS_HOST_SUFFIXES
        .iter()
        .any(|suffix| host.ends_with(suffix))
}

/// A Teams attachment view that may receive the bot token: an allowlisted Bot Connector host
/// serving `/v3/attachments/`, never an AMS object host.
fn media_url(raw: &str) -> Option<Url> {
    let url = auth::service_url(raw).ok()?;
    let host = url.host_str()?;
    (url.path().contains("/v3/attachments/") && !ams_host(host)).then_some(url)
}

/// A SharePoint or OneDrive file, which only the user can open.
fn shared_file(
    kind: &str,
    content_url: Option<&str>,
    name: Option<&str>,
) -> Option<(String, Source)> {
    let link = https_url(content_url);
    if kind != "reference" && !link.as_ref().is_some_and(shared_host) {
        return None;
    }
    Some((
        mime_by_name(name).unwrap_or(OCTET_STREAM).to_owned(),
        unavailable(link.map(String::from), SHARED_FILE),
    ))
}

fn https_url(raw: Option<&str>) -> Option<Url> {
    raw.and_then(|raw| Url::parse(raw).ok())
        .filter(|url| url.scheme() == "https")
}

fn shared_host(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|host| sharepoint_host(host) || onedrive_host(host))
}

/// The attachment's SharePoint or OneDrive page, shown to the user and never fetched.
fn browsable_link(content_url: Option<&str>) -> Option<String> {
    https_url(content_url).filter(shared_host).map(String::from)
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

#[derive(Clone, Copy)]
enum Inline {
    Image,
    Video,
    Sticker,
}

/// The AMS images, videos and stickers in a message's HTML, each with the AMS object id its
/// markup names. Scans a bounded prefix.
fn inline_media(html: &str) -> Vec<(Inline, Option<&str>)> {
    let html = &html[..html.floor_char_boundary(MAX_HTML_BYTES)];
    let lower = html.to_ascii_lowercase();
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(open) = lower[at..].find('<') {
        let start = at + open + 1;
        let end = lower[start..]
            .find('>')
            .map_or(lower.len(), |close| start + close);
        at = end;
        found.extend(inline_tag(&html[start..end]));
    }
    found
}

/// The AMS media kind and object id of one tag's source, without its angle brackets.
fn inline_tag(tag: &str) -> Option<(Inline, Option<&str>)> {
    let element = tag.split_ascii_whitespace().next()?.to_ascii_lowercase();
    let itemtype = attribute(tag, "itemtype")?.to_ascii_lowercase();
    let src = attribute(tag, "src");
    let kind = match element.as_str() {
        "img" if itemtype.ends_with("/amsimage") => Inline::Image,
        "video" if itemtype.ends_with("/amsvideo") => Inline::Video,
        "img" if ams_sticker(&itemtype, src) => Inline::Sticker,
        _ => return None,
    };
    let id =
        attribute(tag, "itemid").or_else(|| src.and_then(|src| segment_after(src, "/v1/objects/")));
    Some((kind, id))
}

/// Stickers on public hosts, such as Giphy GIFs, are not files the user sent.
fn ams_sticker(itemtype: &str, src: Option<&str>) -> bool {
    itemtype.ends_with("sticker") && src.and_then(url_host).is_some_and(|host| ams_host(&host))
}

/// A quoted attribute value from a tag's source, matched case-insensitively by name.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    lower.match_indices(name).find_map(|(start, _)| {
        let named = lower[..start].ends_with(|c: char| c.is_ascii_whitespace());
        let rest = tag[start + name.len()..].strip_prefix('=')?;
        let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        named.then(|| rest[1..].split(quote).next()).flatten()
    })
}

/// The path segment after `marker`, such as the id in `/v3/attachments/{id}/views/original`.
fn segment_after<'a>(url: &'a str, marker: &str) -> Option<&'a str> {
    let (_, rest) = url.split_once(marker)?;
    rest.split(['/', '?', '#'])
        .next()
        .filter(|segment| !segment.is_empty())
}

fn ams_id(id: &str) -> bool {
    (1..=MAX_AMS_ID).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// An image that only the HTML carries, fetched from the Bot Connector view of its AMS object.
fn inline_image(service_url: Option<&str>, id: Option<&str>) -> Source {
    let Some(id) = id else {
        return unavailable(None, FOREIGN_IMAGE);
    };
    attachment_view(service_url.unwrap_or_default(), id)
        .map_or_else(|| unavailable(None, FOREIGN_IMAGE), Source::TeamsDerived)
}

fn attachment_view(service_url: &str, id: &str) -> Option<Url> {
    let mut url = auth::service_url(service_url).ok()?;
    url.path_segments_mut().ok()?.pop_if_empty().extend([
        "v3",
        "attachments",
        id,
        "views",
        "original",
    ]);
    media_url(url.as_str())
}

/// AMS serves inline videos and stickers only to signed-in Teams users.
fn not_downloadable(name: String, mime: &str) -> Candidate {
    Candidate {
        name: Some(name),
        mime: mime.into(),
        source: unavailable(None, INLINE_MEDIA),
        link: None,
        rejected_host: None,
    }
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
        "image/tiff" => "tiff",
        "image/avif" => "avif",
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

/// Stored as the object's Content-Type, so it must be a valid header value.
fn header_safe(mime: &str) -> bool {
    !mime.is_empty() && mime.bytes().all(|b| b.is_ascii_graphic())
}

fn concrete(mime: &str) -> bool {
    header_safe(mime) && mime.contains('/') && !mime.ends_with("/*") && mime != OCTET_STREAM
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
    } else if bytes.starts_with(b"BM")
        && bytes
            .get(14..18)
            .is_some_and(|size| matches!(size, [12 | 40 | 52 | 56 | 64 | 108 | 124, 0, 0, 0]))
    {
        Some("image/bmp")
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some("image/tiff")
    } else if bytes.get(4..8) == Some(b"ftyp".as_slice()) {
        bytes.get(8..12).and_then(image_brand)
    } else {
        None
    }
}

/// The image type of an ISO base media file by its major brand; video brands are not images.
fn image_brand(brand: &[u8]) -> Option<&'static str> {
    match brand {
        b"heic" | b"heix" | b"hevc" => Some("image/heic"),
        b"mif1" | b"msf1" => Some("image/heif"),
        b"avif" => Some("image/avif"),
        _ => None,
    }
}

/// Teams often declares pasted images as `image/*`, so a downloaded file's type comes from
/// the declaration, then its bytes, the response header and finally its name. An image none
/// of them names keeps its declared image type.
fn resolve_mime(declared: &str, name: Option<&str>, header: Option<&str>, bytes: &[u8]) -> String {
    let declared = declared.to_ascii_lowercase();
    let header = header.map(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
    });
    [
        Some(declared.clone()),
        sniff(bytes).map(str::to_owned),
        header,
        name.and_then(extension)
            .and_then(mime_for_extension)
            .map(str::to_owned),
    ]
    .into_iter()
    .flatten()
    .find(|mime| concrete(mime))
    .unwrap_or_else(|| {
        if declared.starts_with("image/") && header_safe(&declared) {
            declared
        } else {
            OCTET_STREAM.to_owned()
        }
    })
}

/// Browsers show only media and plain text inline, so an attacker-declared HTML, SVG or script
/// file downloads instead of rendering from the bucket.
fn disposition(mime: &str, name: &str) -> String {
    let media = ["image/", "audio/", "video/"]
        .iter()
        .any(|family| mime.starts_with(family))
        && !mime.contains("svg")
        && !mime.contains("xml");
    let kind = if media || matches!(mime, "application/pdf" | "text/plain") {
        "inline"
    } else {
        "attachment"
    };
    format!("{kind}; filename*=UTF-8''{}", urlencoding::encode(name))
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

/// Known types first, the URL's extension when the type is unknown, then the image, audio or
/// video family. Any other type gets no part, because providers reject document URLs whose
/// type they cannot read and fail the whole run.
fn media_key(mime: &str, url: &str) -> Option<&'static str> {
    let clean_url = url
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let listed = MEDIA.iter().find(|(_, types, extensions)| {
        types.contains(&mime) || (mime.is_empty() && has_extension(&clean_url, extensions))
    });
    if let Some(&(key, ..)) = listed {
        return Some(key);
    }
    match mime.split('/').next() {
        Some("image") => Some("image_url"),
        Some("audio") => Some("audio_url"),
        Some("video") => Some("video_url"),
        _ => None,
    }
}

/// One media part, chosen like the web chat's `createHistoryMessage`.
fn part(url: &str, mime: &str) -> Option<Value> {
    let mime = mime.to_ascii_lowercase();
    let key = media_key(&mime, url)?;
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
    Some(part)
}

/// The last history message's content when files were downloaded: the text, then one media
/// part per file of a type the model can take. Other files stay in `attachments`,
/// `message.files` and the history placeholders.
pub(super) fn content_parts(text: &str, files: &[FileEntry]) -> Option<Value> {
    let mut parts = vec![json!({"type": "text", "text": text})];
    parts.extend(
        files
            .iter()
            .filter_map(|file| part(file.url.as_deref()?, &file.mime)),
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

/// An error and its causes on one line.
fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    std::iter::successors(Some(error), |cause| cause.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

/// The bot token a Teams-hosted download sends.
#[derive(Clone, Copy)]
struct Bearer<'a> {
    token: &'a str,
    /// Only views Teams declared are known to take the bot token, so only their refusals mean
    /// the cached token went stale.
    declared: bool,
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
    /// Logs hosts and statuses only: download URLs carry tempauth credentials in their query.
    async fn download(&self, url: Url, bearer: Option<Bearer<'_>>) -> Result<Download, String> {
        let connection_id = self.connection.id.as_str();
        let host = url.host_str().unwrap_or_default().to_owned();
        let mut request = self.client.get(url.clone());
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer.token);
        }
        let transport = |error: reqwest::Error| {
            let error = error.without_url();
            tracing::warn!(
                connection_id,
                %host,
                timeout = error.is_timeout(),
                connect = error.is_connect(),
                error = %error_chain(&error),
                "Could not reach Microsoft to download a Teams file"
            );
            UNREACHABLE.to_owned()
        };
        let mut response = request.send().await.map_err(transport)?;
        let status = response.status();
        if status.is_redirection() {
            let target = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|location| location.to_str().ok())
                .and_then(|location| url.join(location).ok())
                .and_then(|target| target.host_str().map(str::to_owned))
                .unwrap_or_else(|| "an unknown host".to_owned());
            tracing::warn!(connection_id, %host, status = status.as_u16(), location_host = %target, "Microsoft redirected a Teams file download");
            return Err(format!(
                "Microsoft redirected this download to {target}, which the bot does not follow"
            ));
        }
        if !status.is_success() {
            tracing::warn!(connection_id, %host, status = status.as_u16(), "Microsoft answered a Teams file download with an error");
            if bearer.is_some_and(|bearer| bearer.declared)
                && matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
            {
                tracing::warn!(connection_id, %host, "Forgetting the cached bot token because Microsoft rejected it");
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
        let Candidate {
            name, mime, source, ..
        } = candidate;
        let (url, bearer) = match source {
            Source::Teams(url) => (url, Some(self.bearer(true)?)),
            Source::TeamsDerived(url) => (url, Some(self.bearer(false)?)),
            Source::SharePoint(url) => (url, None),
            Source::Unavailable { error, .. } => return Err(error),
        };
        within(self.deadline, self.keep(index, name, &mime, url, bearer)).await
    }

    fn bearer(&self, declared: bool) -> Result<Bearer<'_>, String> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| "The bot could not sign in to Teams to download this file".to_owned())?;
        Ok(Bearer { token, declared })
    }

    /// A stored file whose download link cannot be signed keeps its `path`, so flows can
    /// still read it.
    async fn keep(
        &self,
        index: usize,
        name: Option<String>,
        declared: &str,
        url: Url,
        bearer: Option<Bearer<'_>>,
    ) -> Result<FileEntry, String> {
        let download = tokio::time::timeout(FILE_TIMEOUT, self.download(url, bearer))
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
        let options = PutOptions {
            attributes: [
                (Attribute::ContentType, mime.clone()),
                (Attribute::ContentDisposition, disposition(&mime, &name)),
            ]
            .into_iter()
            .collect(),
            ..PutOptions::default()
        };
        if let Err(error) = self
            .store
            .as_generic()
            .put_opts(&object, download.bytes.into(), options)
            .await
        {
            tracing::warn!(connection_id = %self.connection.id, path = %path, %error, "Could not store a Teams file");
            return Err(STORE_FAILED.into());
        }
        let (url, error) = match self.store.sign("GET", &object, self.ttl).await {
            Ok(url) => (Some(url.to_string()), None),
            Err(error) => {
                tracing::warn!(connection_id = %self.connection.id, path = %path, %error, "Could not sign a download link for a stored Teams file");
                (None, Some(SIGN_FAILED.to_owned()))
            }
        };
        Ok(FileEntry {
            name,
            mime,
            size: Some(size),
            url,
            path: Some(path),
            downloadable: true,
            link: None,
            error,
        })
    }
}

pub(super) struct Storage {
    pub(super) store: FlowLikeStore,
    pub(super) prefix: String,
    pub(super) ttl: Duration,
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

/// Splits off the files to download, marking and logging any beyond the first ten.
fn jobs(
    connection_id: &str,
    candidates: Vec<Candidate>,
    entries: &mut [FileEntry],
) -> Vec<(usize, Candidate)> {
    let mut jobs = Vec::new();
    for (index, candidate) in candidates.into_iter().enumerate() {
        if matches!(candidate.source, Source::Unavailable { .. }) {
            continue;
        }
        if jobs.len() == MAX_FILES {
            let error = format!("Only the first {MAX_FILES} files of a message are downloaded");
            not_downloaded(connection_id, index, &origin(&candidate.source), &error);
            entries[index].error = Some(error);
            continue;
        }
        jobs.push((index, candidate));
    }
    jobs
}

/// One warning per message for the files Teams exposes but the bot cannot fetch.
fn report_unavailable(connection_id: &str, candidates: &[Candidate]) {
    let reasons: Vec<String> = candidates
        .iter()
        .filter_map(|candidate| {
            let Source::Unavailable { link, error } = &candidate.source else {
                return None;
            };
            let host = candidate.rejected_host.clone().or_else(|| {
                link.as_deref()
                    .or(candidate.link.as_deref())
                    .and_then(url_host)
            });
            Some(format!(
                "{error} ({})",
                host.as_deref().unwrap_or("no host")
            ))
        })
        .collect();
    if !reasons.is_empty() {
        tracing::warn!(connection_id, files = ?reasons, "Teams sent files the bot cannot download");
    }
}

/// Where a job's bytes come from, as logs may name it: the source and host, never the URL.
fn origin(source: &Source) -> (&'static str, String) {
    let (kind, url) = match source {
        Source::Teams(url) => ("teams", Some(url)),
        Source::TeamsDerived(url) => ("teams_derived", Some(url)),
        Source::SharePoint(url) => ("sharepoint", Some(url)),
        Source::Unavailable { .. } => ("unavailable", None),
    };
    (
        kind,
        url.and_then(Url::host_str).unwrap_or_default().to_owned(),
    )
}

/// Entry errors are fixed texts or name only a status or host, so they are safe to log.
fn not_downloaded(connection_id: &str, index: usize, (kind, host): &(&str, String), error: &str) {
    tracing::warn!(connection_id, index, kind, %host, error, "Teams file was not downloaded");
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
    report_unavailable(&c.id, &candidates);
    let mut entries: Vec<FileEntry> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| FileEntry::pending(index, candidate))
        .collect();
    let jobs = jobs(&c.id, candidates, &mut entries);
    if jobs.is_empty() {
        return entries;
    }
    let origins: Vec<(&str, String)> = jobs
        .iter()
        .map(|(_, candidate)| origin(&candidate.source))
        .collect();
    let needs_token = jobs.iter().any(|(_, candidate)| {
        matches!(candidate.source, Source::Teams(_) | Source::TeamsDerived(_))
    });
    let prepared = tokio::time::timeout_at(
        deadline,
        Fetcher::prepare(state, c, sink, run, deadline, needs_token),
    )
    .await
    .unwrap_or(Err(TIMED_OUT));
    let fetcher = match prepared {
        Ok(fetcher) => fetcher,
        Err(error) => {
            for ((index, _), origin) in jobs.iter().zip(&origins) {
                not_downloaded(&c.id, *index, origin, error);
                entries[*index].error = Some(error.to_owned());
            }
            return entries;
        }
    };
    let results = futures::future::join_all(jobs.into_iter().map(|(index, candidate)| {
        let fetcher = &fetcher;
        async move { (index, fetcher.fetch(index, candidate).await) }
    }))
    .await;
    for ((index, result), origin) in results.into_iter().zip(&origins) {
        match result {
            Ok(entry) => entries[index] = entry,
            Err(error) => {
                not_downloaded(&c.id, index, origin, &error);
                entries[index].error = Some(error);
            }
        }
    }
    entries
}

/// Fresh download links for files that earlier messages stored, signed by `deadline`.
/// Never fails: files that cannot be signed in time are left out.
pub(super) async fn resign(
    state: &AppState,
    c: &Connection,
    sink: &event_sink::Model,
    refs: &[Value],
    current: &[FileEntry],
    deadline: Instant,
) -> Vec<FileEntry> {
    if refs.is_empty() {
        return Vec::new();
    }
    match tokio::time::timeout_at(deadline, storage(state, sink, &c.app_id)).await {
        Ok(Ok(storage)) => sign_stored(&c.id, &storage, refs, current, deadline).await,
        Ok(Err(error)) => {
            tracing::warn!(connection_id = %c.id, %error, "Could not prepare storage to re-sign earlier Teams files");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!(connection_id = %c.id, "Storage for earlier Teams files was not ready in time");
            Vec::new()
        }
    }
}

/// Signs the stored files that still exist and fit beside the message's own linked files
/// within the per-message limits, first come first. Files outside this app's runs are never
/// signed. A missing object is left out, because a link to it fails the model request.
pub(super) async fn sign_stored(
    connection_id: &str,
    storage: &Storage,
    refs: &[Value],
    current: &[FileEntry],
    deadline: Instant,
) -> Vec<FileEntry> {
    let runs = format!("{}/runs/", storage.prefix);
    let mut skipped = Vec::new();
    let mut files = Vec::new();
    for reference in refs {
        match stored_file(reference, &runs) {
            Some(file) => files.push(file),
            None => skipped.push(format!(
                "not stored under this app's runs ({})",
                reference["path"].as_str().unwrap_or("no path")
            )),
        }
    }
    let (kept, over) = within_limits(files, current);
    skipped.extend(
        over.into_iter()
            .filter_map(|file| Some(format!("over the per-message file limits ({})", file.path?))),
    );
    if !skipped.is_empty() {
        tracing::warn!(connection_id, %runs, ?skipped, "Earlier Teams files were not linked again");
    }
    let signing = kept.into_iter().map(|file| async move {
        let path = file.path.clone()?;
        let object = StoragePath::from(path.as_str());
        let signed = tokio::time::timeout_at(deadline, async {
            storage.store.as_generic().head(&object).await?;
            storage.store.sign("GET", &object, storage.ttl).await
        })
        .await;
        match signed {
            Ok(Ok(url)) => Some(FileEntry {
                url: Some(url.to_string()),
                ..file
            }),
            Ok(Err(error)) => {
                tracing::warn!(connection_id, %path, %error, "Could not link an earlier Teams file again");
                None
            }
            Err(_) => {
                tracing::warn!(connection_id, %path, "An earlier Teams file was not re-signed in time");
                None
            }
        }
    });
    futures::future::join_all(signing)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// Splits `files` into those that fit beside the message's own linked files within the
/// per-message limits, first come first, and those that do not.
fn within_limits(files: Vec<FileEntry>, current: &[FileEntry]) -> (Vec<FileEntry>, Vec<FileEntry>) {
    let linked = current.iter().filter(|file| file.url.is_some());
    let mut count = linked.clone().count();
    let mut total: u64 = linked.filter_map(|file| file.size).sum();
    files.into_iter().partition(|file| {
        let size = file.size.unwrap_or_default();
        let fits = count < MAX_FILES && total.saturating_add(size) <= MAX_TOTAL_BYTES;
        if fits {
            count += 1;
            total += size;
        }
        fits
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, Bytes},
        http::header,
        response::Response,
        routing::get,
    };
    use flow_like_storage::object_store::{Attributes, ObjectStoreExt, memory::InMemory};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tracing::instrument::WithSubscriber;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n rest";
    const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    const BOT: Option<Bearer<'static>> = Some(Bearer {
        token: "bot-token",
        declared: true,
    });

    fn teams(url: &str) -> Source {
        Source::Teams(Url::parse(url).unwrap())
    }

    fn fixture(raw: &str) -> Value {
        serde_json::from_str(raw).unwrap()
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
                    link: None,
                    rejected_host: None,
                },
                Candidate {
                    name: Some("x.png".into()),
                    mime: "image/png".into(),
                    source: Source::Unavailable {
                        link: None,
                        error: FOREIGN_IMAGE.into()
                    },
                    link: None,
                    rejected_host: Some("attacker.invalid".into()),
                },
                Candidate {
                    name: Some("Report.PDF".into()),
                    mime: "application/pdf".into(),
                    source: Source::SharePoint(
                        Url::parse("https://contoso.sharepoint.com/personal/x/_layouts/15/download.aspx?UniqueId=1&tempauth=t").unwrap()
                    ),
                    link: None,
                    rejected_host: None,
                },
                Candidate {
                    name: Some("notes.md".into()),
                    mime: "text/markdown".into(),
                    source: Source::Unavailable {
                        link: None,
                        error: FOREIGN_DOWNLOAD.into()
                    },
                    link: None,
                    rejected_host: Some("sharepoint.com.attacker.invalid".into()),
                },
                Candidate {
                    name: Some("Plan.docx".into()),
                    mime: DOCX.into(),
                    source: Source::Unavailable {
                        link: Some(
                            "https://contoso.sharepoint.com/sites/team/Shared%20Documents/Plan.docx"
                                .into()
                        ),
                        error: SHARED_FILE.into()
                    },
                    link: Some(
                        "https://contoso.sharepoint.com/sites/team/Shared%20Documents/Plan.docx"
                            .into()
                    ),
                    rejected_host: None,
                },
                Candidate {
                    name: Some("sheet.xlsx".into()),
                    mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                        .into(),
                    source: Source::Unavailable {
                        link: Some("https://contoso-my.sharepoint.com/x/sheet.xlsx".into()),
                        error: SHARED_FILE.into()
                    },
                    link: Some("https://contoso-my.sharepoint.com/x/sheet.xlsx".into()),
                    rejected_host: None,
                },
                Candidate {
                    name: Some("odd".into()),
                    mime: OCTET_STREAM.into(),
                    source: Source::Unavailable {
                        link: None,
                        error: SHARED_FILE.into()
                    },
                    link: None,
                    rejected_host: None,
                },
            ]
        );
        assert!(classify(&json!({"text":"no attachments"})).is_empty());
    }

    #[test]
    fn microsoft_payloads_yield_one_candidate_per_file() {
        let personal = classify(&fixture(include_str!(
            "testdata/attachments/personal-file.json"
        )));
        assert_eq!(
            personal,
            vec![Candidate {
                name: Some("outline.png".into()),
                mime: "image/png".into(),
                source: Source::SharePoint(
                    Url::parse("https://m365x9462XXXX-my.sharepoint.com/personal/admin_m365x9462XXXX_onmicrosoft_com/_layouts/15/download.aspx?UniqueId=2392290d-ff20-4520-9862-f5bf588688e0&Translate=false&tempauth=eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9lNA&ApiVersion=2.0").unwrap()
                ),
                link: Some("https://m365x9462xxxx-my.sharepoint.com/personal/admin_m365x9462XXXX_onmicrosoft_com/Documents/Microsoft%20Teams%20Chat%20Files/outline.png".into()),
                rejected_host: None,
            }]
        );
        let failed = FileEntry::pending(0, &personal[0]);
        assert_eq!(failed.link, personal[0].link);
        assert!(failed.downloadable);

        let html_only = "https://smba.trafficmanager.net/amer/v3/attachments/0-cus-d17-c53459922d66463fb94af32a61057191/views/original";
        for (raw, source, mime) in [
            (
                include_str!("testdata/attachments/web-paste.json"),
                teams(
                    "https://smba.trafficmanager.net/amer/v3/attachments/0-cus-d17-c53459922d66463fb94af32a61057191/views/original",
                ),
                "image/*",
            ),
            (
                include_str!("testdata/attachments/ios-photo.json"),
                teams(
                    "https://smba.trafficmanager.net/amer/v3/attachments/0-eus-d20-fd7c8146fbc34613ddb246bed1988c0e/views/original",
                ),
                "image/*",
            ),
            (
                include_str!("testdata/attachments/concrete-png.json"),
                teams(
                    "https://smba.trafficmanager.net/emea/v3/attachments/0-weu-d3-1a2b3c4d5e6f/views/original",
                ),
                "image/png",
            ),
            (
                include_str!("testdata/attachments/html-only-image.json"),
                Source::TeamsDerived(Url::parse(html_only).unwrap()),
                "image/*",
            ),
        ] {
            assert_eq!(
                classify(&fixture(raw)),
                vec![Candidate {
                    name: None,
                    mime: mime.into(),
                    source: source.clone(),
                    link: None,
                    rejected_host: None,
                }],
                "{source:?}"
            );
        }

        assert!(
            classify(&fixture(include_str!(
                "testdata/attachments/group-mention.json"
            )))
            .is_empty()
        );
        assert_eq!(
            classify(&fixture(include_str!(
                "testdata/attachments/inline-video.json"
            ))),
            vec![Candidate {
                name: Some("video-1".into()),
                mime: "video/*".into(),
                source: unavailable(None, INLINE_MEDIA),
                link: None,
                rejected_host: None,
            }]
        );
        let cloud_pick = classify(&fixture(include_str!(
            "testdata/attachments/mobile-cloud-pick.json"
        )));
        assert_eq!(
            cloud_pick,
            vec![Candidate {
                name: Some("Budget.xlsx".into()),
                mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
                source: unavailable(None, NO_DOWNLOAD_LINK),
                link: None,
                rejected_host: None,
            }]
        );
    }

    #[test]
    fn inline_html_media_is_deduplicated_bounded_and_never_fetched_from_ams() {
        let html = |content: &str| {
            json!({"serviceUrl": "https://smba.trafficmanager.net/amer/", "attachments": [
                {"contentType": "text/html", "content": content}
            ]})
        };
        let twice = html(
            "<img itemtype=\"http://schema.skype.com/AMSImage\" itemid=\"0-a\"><IMG ITEMTYPE='http://schema.skype.com/AMSImage' SRC='https://us-api.asm.skype.com/v1/objects/0-A/views/imgo'>",
        );
        assert_eq!(classify(&twice).len(), 1);
        let mut untrusted = twice.clone();
        untrusted["serviceUrl"] = json!("https://attacker.invalid/");
        let rejected = &classify(&untrusted)[0];
        assert_eq!(rejected.source, unavailable(None, FOREIGN_IMAGE));
        assert_eq!(rejected.rejected_host.as_deref(), Some("attacker.invalid"));
        let odd_id = html(
            "<img itemtype=\"http://schema.skype.com/AMSImage\" itemid=\"../../x\" src=\"https://us-api.asm.skype.com/v1/objects/../views/imgo\">",
        );
        let rejected = &classify(&odd_id)[0];
        assert_eq!(rejected.source, unavailable(None, FOREIGN_IMAGE));
        assert_eq!(rejected.rejected_host, None);
        let mut paired = odd_id.clone();
        paired["attachments"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({"contentType": "image/*", "contentUrl": "https://smba.trafficmanager.net/amer/v3/attachments/0-b/views/original"}));
        assert_eq!(classify(&paired).len(), 1);
        let sticker = html(
            "<p>hi <img itemtype=\"http://schema.skype.com/Emoji\" itemid=\"smile\" src=\"https://statics.teams.cdn.office.net/smile.png\"><img itemtype=\"http://schema.skype.com/Sticker\" src=\"https://us-prod.asyncgw.teams.microsoft.com/v1/objects/0-s/views/imgo\"></p>",
        );
        assert_eq!(
            classify(&sticker),
            vec![not_downloadable("sticker-1".into(), "image/*")]
        );
        let late = html(&format!(
            "{}<img itemtype=\"http://schema.skype.com/AMSImage\" itemid=\"0-late\">",
            "é".repeat(MAX_HTML_BYTES)
        ));
        assert!(classify(&late).is_empty());
    }

    #[test]
    fn the_download_limit_counts_files_not_duplicate_html_tags() {
        let pasted = |id: usize| {
            format!("<img itemtype=\"http://schema.skype.com/AMSImage\" itemid=\"0-i{id}\">")
        };
        let html = |content: String| {
            json!({"serviceUrl": "https://smba.trafficmanager.net/amer/", "attachments": [
                {"contentType": "text/html", "content": content}
            ]})
        };
        let mut paste_and_video = html(format!(
            "{}<video itemtype=\"http://schema.skype.com/AMSVideo\">",
            (0..MAX_FILES).map(pasted).collect::<String>()
        ));
        let attachments = paste_and_video["attachments"].as_array_mut().unwrap();
        for id in 0..MAX_FILES {
            attachments.insert(
                id,
                json!({"contentType": "image/*", "contentUrl": format!("https://smba.trafficmanager.net/amer/v3/attachments/0-i{id}/views/original")}),
            );
        }
        let found = classify(&paste_and_video);
        assert_eq!(found.len(), MAX_FILES + 1);
        assert!(
            found[..MAX_FILES]
                .iter()
                .all(|candidate| matches!(candidate.source, Source::Teams(_)))
        );
        assert_eq!(
            found[MAX_FILES],
            not_downloadable(format!("video-{}", MAX_FILES + 1), "video/*")
        );

        let html_only = classify(&html((0..=MAX_FILES).map(pasted).collect()));
        assert_eq!(html_only.len(), MAX_FILES + 1);
        assert!(
            html_only
                .iter()
                .all(|candidate| matches!(candidate.source, Source::TeamsDerived(_)))
        );
        let mut entries = html_only
            .iter()
            .enumerate()
            .map(|(index, candidate)| FileEntry::pending(index, candidate))
            .collect::<Vec<_>>();
        assert_eq!(jobs("c", html_only, &mut entries).len(), MAX_FILES);
        assert_eq!(
            entries[MAX_FILES].error.as_deref(),
            Some("Only the first 10 files of a message are downloaded")
        );
    }

    #[test]
    fn unavailable_files_are_reported_once_by_the_host_the_bot_declined() {
        let activity = json!({"attachments": [
            {"contentType": "image/png", "contentUrl": "https://attacker.invalid/x.png?tempauth=secret-marker"},
            {"contentType": DOWNLOAD_INFO, "name": "a.pdf", "contentUrl": "https://contoso.sharepoint.com/a.pdf", "content": {
                "downloadUrl": "https://files.example.invalid/a.pdf?tempauth=secret-marker"
            }},
            {"contentType": "reference", "name": "Plan.docx", "contentUrl": "https://contoso.sharepoint.com/Plan.docx"}
        ]});
        let logs = Logs::default();
        let candidates =
            tracing::subscriber::with_default(logs.subscriber(), || classify(&activity));
        assert_eq!(logs.text(), "");
        tracing::subscriber::with_default(logs.subscriber(), || {
            report_unavailable("c", &candidates)
        });
        let text = logs.text();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains("connection_id=\"c\""), "{text}");
        assert!(
            text.contains(&format!("{FOREIGN_IMAGE} (attacker.invalid)")),
            "{text}"
        );
        assert!(
            text.contains(&format!("{FOREIGN_DOWNLOAD} (files.example.invalid)")),
            "{text}"
        );
        assert!(
            text.contains(&format!("{SHARED_FILE} (contoso.sharepoint.com)")),
            "{text}"
        );
        assert!(!text.contains("secret-marker"), "{text}");
    }

    #[test]
    fn the_bot_token_only_goes_to_teams_hosts() {
        for url in [
            "https://smba.trafficmanager.net/amer/v3/attachments/x/views/original",
            "https://eu.botapi.skype.com/amer/v3/attachments/x/views/original",
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
            "https://smba.trafficmanager.net/amer/v1/objects/x/views/imgo",
            "https://us-api.asm.skype.com/v1/objects/x/views/imgo",
            "https://us-prod.asyncgw.teams.microsoft.com/v1/objects/x/views/imgo",
            "https://us-prod.asyncgw.teams.microsoft.com/v3/attachments/x/views/original",
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
        assert_eq!(resolve_mime("image/*", None, None, PNG), "image/png");
        assert_eq!(
            resolve_mime("image/*", None, Some("image/jpeg; charset=binary"), b"??"),
            "image/jpeg"
        );
        assert_eq!(
            resolve_mime("image/*", Some("a.webp"), Some(OCTET_STREAM), b"??"),
            "image/webp"
        );
        assert_eq!(
            resolve_mime("application/pdf", None, Some("image/png"), PNG),
            "application/pdf"
        );
        assert_eq!(resolve_mime("image/*", None, None, b"??"), "image/*");
        assert_eq!(
            resolve_mime("image/x-icon\r\n", None, None, b"??"),
            OCTET_STREAM
        );
        assert_eq!(resolve_mime(OCTET_STREAM, None, None, b"??"), OCTET_STREAM);
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a"), Some("image/gif"));
        assert_eq!(
            sniff(b"BM\x36\0\0\0\0\0\0\0\x36\0\0\0\x28\0\0\0"),
            Some("image/bmp")
        );
        assert_eq!(sniff(b"BMW service notes, not a bitmap"), None);
        assert_eq!(sniff(b"II*\0\x08\0\0\0"), Some("image/tiff"));
        assert_eq!(sniff(b"MM\0*\0\0\0\x08"), Some("image/tiff"));
        for (brand, mime) in [
            (b"heic", Some("image/heic")),
            (b"heix", Some("image/heic")),
            (b"hevc", Some("image/heic")),
            (b"mif1", Some("image/heif")),
            (b"msf1", Some("image/heif")),
            (b"avif", Some("image/avif")),
            (b"isom", None),
            (b"mp42", None),
        ] {
            let mut bytes = b"\0\0\0\x18ftyp".to_vec();
            bytes.extend_from_slice(brand);
            assert_eq!(sniff(&bytes), mime, "{}", String::from_utf8_lossy(brand));
        }
        assert_eq!(sniff(b"\0\0\0\x18ftyp"), None);
        assert_eq!(fallback_name(0, "image/jpeg"), "image-1.jpg");
        assert_eq!(fallback_name(2, "image/*"), "image-3");
        assert_eq!(fallback_name(0, "image/avif"), "image-1.avif");
        assert_eq!(fallback_name(0, "image/tiff"), "image-1.tiff");
        assert_eq!(fallback_name(1, "application/pdf"), "file-2");
    }

    #[test]
    fn only_media_plain_text_and_pdf_display_inline() {
        assert_eq!(
            disposition("image/png", "Über plan.png"),
            "inline; filename*=UTF-8''%C3%9Cber%20plan.png"
        );
        for mime in [
            "image/*",
            "image/jpeg",
            "audio/mpeg",
            "video/mp4",
            "application/pdf",
            "text/plain",
        ] {
            assert!(disposition(mime, "x").starts_with("inline; "), "{mime}");
        }
        for mime in [
            "text/html",
            "image/svg+xml",
            "application/xhtml+xml",
            "text/javascript",
            "application/javascript",
            OCTET_STREAM,
            DOCX,
        ] {
            assert!(disposition(mime, "x").starts_with("attachment; "), "{mime}");
        }
    }

    #[test]
    fn parts_follow_the_web_chat_rules() {
        let url = "https://store.example/x/0000-a?sig=1";
        assert_eq!(
            part(url, "image/png"),
            Some(json!({"type":"image_url","image_url":{"url":url,"media_type":"image/png"}}))
        );
        assert_eq!(
            part(url, "audio/x-m4a"),
            Some(json!({"type":"audio_url","audio_url":url,"media_type":"audio/x-m4a"}))
        );
        assert_eq!(
            part(url, "video/quicktime"),
            Some(json!({"type":"video_url","video_url":url,"media_type":"video/quicktime"}))
        );
        assert_eq!(
            part(url, "text/csv"),
            Some(json!({"type":"document_url","document_url":url,"media_type":"text/csv"}))
        );
        assert_eq!(
            part(url, "image/bmp"),
            Some(json!({"type":"image_url","image_url":{"url":url,"media_type":"image/bmp"}}))
        );
        assert_eq!(
            part(url, "image/*"),
            Some(json!({"type":"image_url","image_url":{"url":url,"media_type":"image/*"}}))
        );
        assert_eq!(part(url, "audio/opus").unwrap()["type"], "audio_url");
        assert_eq!(part(url, "video/x-matroska").unwrap()["type"], "video_url");
        for unknown in [DOCX, OCTET_STREAM, "application/zip", "text/x-unknown"] {
            assert_eq!(part(url, unknown), None, "{unknown}");
        }
        assert_eq!(
            part("https://s/x/photo.JPG?sig=1#f", ""),
            Some(json!({"type":"image_url","image_url":{"url":"https://s/x/photo.JPG?sig=1#f"}}))
        );
        assert_eq!(part("https://s/x/a.mp3", "").unwrap()["type"], "audio_url");
        assert_eq!(part("https://s/x/a.mpeg", "").unwrap()["type"], "video_url");
        assert_eq!(
            part("https://s/x/a.py", "").unwrap()["type"],
            "document_url"
        );
        assert_eq!(part("https://s/x/a.mp4.bin", ""), None);
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
                link: None,
                rejected_host: None,
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
                link: None,
                rejected_host: None,
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
    fn files_a_model_cannot_read_stay_out_of_the_parts_but_not_the_message() {
        let files = [
            downloaded("Plan.docx", DOCX),
            downloaded("blob", OCTET_STREAM),
            downloaded("a.pdf", "application/pdf"),
            downloaded("a.txt", "text/plain"),
        ];
        assert_eq!(
            content_parts("look", &files),
            Some(json!([
                {"type":"text","text":"look"},
                {"type":"document_url","document_url":"https://store.example/a.pdf?sig=1","media_type":"application/pdf"},
                {"type":"document_url","document_url":"https://store.example/a.txt?sig=1","media_type":"text/plain"}
            ]))
        );
        assert_eq!(content_parts("look", &files[..2]), None);
        assert!(files[..2].iter().all(|file| file.attachment().is_some()));
        assert_eq!(
            with_placeholders("", &files[..2]),
            "[file: Plan.docx]\n[file: blob]"
        );
    }

    #[test]
    fn at_most_ten_files_are_downloaded_and_unavailable_files_do_not_count() {
        let shared = Candidate {
            name: Some("Plan.docx".into()),
            mime: OCTET_STREAM.into(),
            source: unavailable(None, SHARED_FILE),
            link: None,
            rejected_host: None,
        };
        let image = Candidate {
            name: None,
            mime: "image/png".into(),
            source: teams("https://smba.trafficmanager.net/x"),
            link: None,
            rejected_host: None,
        };
        let candidates = std::iter::once(shared)
            .chain(std::iter::repeat_n(image, 11))
            .collect::<Vec<_>>();
        let mut entries = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| FileEntry::pending(index, candidate))
            .collect::<Vec<_>>();
        let logs = Logs::default();
        let jobs = tracing::subscriber::with_default(logs.subscriber(), || {
            jobs("c", candidates, &mut entries)
        });
        assert_eq!(
            jobs.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            (1..=10).collect::<Vec<_>>()
        );
        assert_eq!(
            entries[11].error.as_deref(),
            Some("Only the first 10 files of a message are downloaded")
        );
        let text = logs.text();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(
            text.contains("index=11 kind=\"teams\" host=smba.trafficmanager.net"),
            "{text}"
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

    #[derive(Clone, Default)]
    struct Logs(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Logs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Logs {
        fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
            let writer = self.clone();
            tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .finish()
        }

        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    fn connection() -> Connection {
        serde_json::from_value(json!({
            "id":"c", "app_id":"a", "event_id":"e", "mode":"customer_teams",
            "name":"Bot", "description":"", "customer_tenant_id":"t", "home_tenant_id":"h",
            "client_id":"bot-id", "secret":"test-only", "status":"ready", "allowed_responders":[]
        }))
        .unwrap()
    }

    fn fetcher(connection: &Connection, store: FlowLikeStore) -> Fetcher<'_> {
        Fetcher {
            connection,
            client: super::super::client().unwrap(),
            token: Some("bot-token".into()),
            store,
            prefix: "tmp/user/u/apps/a".into(),
            run: "r",
            ttl: Duration::from_secs(3600),
            deadline: Instant::now() + Duration::from_secs(60),
            total: AtomicU64::new(0),
        }
    }

    fn memory() -> FlowLikeStore {
        FlowLikeStore::Memory(Arc::new(InMemory::new()))
    }

    /// A stand-in for Microsoft's download endpoints on a local port.
    async fn microsoft() -> Url {
        let chunk = || Ok::<_, std::io::Error>(Bytes::from(vec![0_u8; 1 << 20]));
        let router = Router::new()
            .route(
                "/png",
                get(|| async { ([(header::CONTENT_TYPE, OCTET_STREAM)], PNG) }),
            )
            .route(
                "/bare",
                get(|| async { Response::new(Body::from("not an image")) }),
            )
            .route(
                "/redirect",
                get(|| async {
                    (
                        StatusCode::FOUND,
                        [(
                            header::LOCATION,
                            "https://b0mpua-by3301.files.1drv.com/y23vmagahszhxzlcvhasdhasghasodfi?tempauth=secret-marker",
                        )],
                    )
                }),
            )
            .route("/denied", get(|| async { StatusCode::UNAUTHORIZED }))
            .route(
                "/huge",
                get(|| async { vec![0_u8; MAX_FILE_BYTES as usize + 1] }),
            )
            .route(
                "/stream",
                get(move || async move {
                    Body::from_stream(futures::stream::iter((0..26).map(move |_| chunk())))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Url::parse(&format!("http://{address}/")).unwrap()
    }

    async fn stored(store: &FlowLikeStore, path: &str) -> (Attributes, Bytes) {
        let object = store
            .as_generic()
            .get(&StoragePath::from(path))
            .await
            .unwrap();
        let attributes = object.attributes.clone();
        (attributes, object.bytes().await.unwrap())
    }

    fn attribute_value(attributes: &Attributes, key: &Attribute) -> Option<String> {
        attributes.get(key).map(|value| value.to_string())
    }

    #[tokio::test]
    async fn downloads_are_stored_with_their_resolved_type_and_disposition() {
        let base = microsoft().await;
        let connection = connection();
        let store = memory();
        let fetcher = fetcher(&connection, store.clone());

        let entry = fetcher
            .keep(0, None, "image/*", base.join("png").unwrap(), BOT)
            .await
            .unwrap();
        let path = "tmp/user/u/apps/a/runs/r/request/teams/0000-image-1.png";
        assert_eq!(
            (
                entry.name.as_str(),
                entry.mime.as_str(),
                entry.path.as_deref()
            ),
            ("image-1.png", "image/png", Some(path))
        );
        assert_eq!(entry.size, Some(PNG.len() as u64));
        assert!(entry.url.is_some() && entry.error.is_none());
        let (attributes, bytes) = stored(&store, path).await;
        assert_eq!(bytes.as_ref(), PNG);
        assert_eq!(
            attribute_value(&attributes, &Attribute::ContentType).as_deref(),
            Some("image/png")
        );
        assert_eq!(
            attribute_value(&attributes, &Attribute::ContentDisposition).as_deref(),
            Some("inline; filename*=UTF-8''image-1.png")
        );

        let entry = fetcher
            .keep(1, None, "image/*", base.join("bare").unwrap(), BOT)
            .await
            .unwrap();
        let path = "tmp/user/u/apps/a/runs/r/request/teams/0001-image-2";
        assert_eq!(
            (
                entry.name.as_str(),
                entry.mime.as_str(),
                entry.path.as_deref()
            ),
            ("image-2", "image/*", Some(path))
        );
        let (attributes, bytes) = stored(&store, path).await;
        assert_eq!(bytes.as_ref(), b"not an image");
        assert_eq!(
            attribute_value(&attributes, &Attribute::ContentType).as_deref(),
            Some("image/*")
        );
        assert_eq!(
            fetcher.total.load(Ordering::SeqCst),
            (PNG.len() + 12) as u64
        );
    }

    #[tokio::test]
    async fn a_file_that_cannot_be_signed_keeps_its_stored_path() {
        let base = microsoft().await;
        let connection = connection();
        let store = FlowLikeStore::Other(Arc::new(InMemory::new()));
        let entry = fetcher(&connection, store.clone())
            .keep(
                0,
                Some("Plan.docx".into()),
                DOCX,
                base.join("bare").unwrap(),
                None,
            )
            .await
            .unwrap();
        let path = "tmp/user/u/apps/a/runs/r/request/teams/0000-Plan.docx";
        assert_eq!(
            entry,
            FileEntry {
                name: "Plan.docx".into(),
                mime: DOCX.into(),
                size: Some(12),
                url: None,
                path: Some(path.into()),
                downloadable: true,
                link: None,
                error: Some(SIGN_FAILED.into()),
            }
        );
        let (attributes, _) = stored(&store, path).await;
        assert_eq!(
            attribute_value(&attributes, &Attribute::ContentDisposition).as_deref(),
            Some("attachment; filename*=UTF-8''Plan.docx")
        );
    }

    #[tokio::test]
    async fn earlier_files_are_signed_again_unless_they_cannot_be() {
        let objects = Arc::new(InMemory::new());
        let storage = Storage {
            store: FlowLikeStore::Memory(objects.clone()),
            prefix: "tmp/user/u/apps/a".into(),
            ttl: Duration::from_secs(60),
        };
        let kept = downloaded("photo.png", "image/png");
        let foreign = "tmp/user/other/apps/a/runs/r/request/teams/0000-x.png";
        for path in [kept.path.as_deref().unwrap(), foreign] {
            objects
                .put(&StoragePath::from(path), PNG.into())
                .await
                .unwrap();
        }
        let mut refs = stored_refs(&[
            kept.clone(),
            downloaded("missing.png", "image/png"),
            FileEntry::default(),
        ]);
        assert_eq!(
            refs[0],
            json!({"path": kept.path, "name": "photo.png", "type": "image/png", "size": 4})
        );
        assert_eq!(refs.len(), 2);
        refs.extend([
            json!({"path": foreign, "name": "x.png", "type": "image/png", "size": 4}),
            json!({"name": "no-path.png", "type": "image/png"}),
        ]);
        let deadline = Instant::now() + Duration::from_secs(60);

        let logs = Logs::default();
        let signed = sign_stored("c", &storage, &refs, &[], deadline)
            .with_subscriber(logs.subscriber())
            .await;
        assert_eq!(signed.len(), 1, "{signed:?}");
        let url = signed[0].url.clone().unwrap();
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
        assert_eq!(
            signed[0],
            FileEntry {
                url: Some(url),
                ..kept.clone()
            }
        );
        let logged = logs.text();
        assert!(logged.contains("Earlier Teams files were not linked again"));
        assert!(logged.contains(&format!("not stored under this app's runs ({foreign})")));
        assert!(logged.contains("not stored under this app's runs (no path)"));
        assert!(logged.contains("Could not link an earlier Teams file again"));
        assert!(logged.contains("0000-missing.png"));
        assert!(!logged.contains("data:"), "{logged}");

        let refusing = Storage {
            store: FlowLikeStore::Other(objects),
            prefix: storage.prefix.clone(),
            ttl: storage.ttl,
        };
        assert!(
            sign_stored("c", &refusing, &refs, &[], deadline)
                .await
                .is_empty()
        );
        let full = vec![downloaded("other.png", "image/png"); MAX_FILES];
        let logs = Logs::default();
        assert!(
            sign_stored("c", &storage, &refs, &full, deadline)
                .with_subscriber(logs.subscriber())
                .await
                .is_empty()
        );
        assert!(logs.text().contains(&format!(
            "over the per-message file limits ({})",
            kept.path.as_deref().unwrap()
        )));
        let heavy = FileEntry {
            size: Some(MAX_TOTAL_BYTES - 3),
            ..downloaded("big.png", "image/png")
        };
        assert!(
            sign_stored("c", &storage, &refs, &[heavy], deadline)
                .await
                .is_empty()
        );
    }

    #[test]
    fn only_stored_files_a_model_can_take_are_linked_again() {
        let path = "tmp/user/u/apps/a/runs/r/request/teams/0000-";
        let reference = |name: &str, mime: &str| json!({"path": format!("{path}{name}"), "name": name, "type": mime});
        assert!(linkable(&reference("a.png", "image/png")));
        assert!(linkable(&reference("a.pdf", "application/pdf")));
        assert!(linkable(&reference("a.JPG", "")));
        assert!(linkable(&json!({"path": format!("{path}a.png")})));
        assert!(!linkable(&reference("a.docx", DOCX)));
        assert!(!linkable(&reference("a.bin", OCTET_STREAM)));
        assert!(!linkable(&json!({"name": "a.png", "type": "image/png"})));
    }

    #[tokio::test]
    async fn redirects_and_refusals_are_reported_without_urls() {
        let base = microsoft().await;
        let connection = connection();
        let fetcher = fetcher(&connection, memory());
        let logs = Logs::default();

        let redirected = fetcher
            .download(base.join("redirect").unwrap(), BOT)
            .with_subscriber(logs.subscriber())
            .await
            .err();
        assert_eq!(
            redirected.as_deref(),
            Some(
                "Microsoft redirected this download to b0mpua-by3301.files.1drv.com, which the bot does not follow"
            )
        );
        assert!(
            logs.text()
                .contains("location_host=b0mpua-by3301.files.1drv.com")
        );

        let denied = fetcher
            .download(base.join("denied").unwrap(), None)
            .with_subscriber(logs.subscriber())
            .await
            .err();
        assert_eq!(
            denied.as_deref(),
            Some("Microsoft returned HTTP 401 for this file")
        );
        assert!(logs.text().contains("status=401"));
        assert!(!logs.text().contains("Forgetting the cached bot token"));

        let derived = Candidate {
            name: None,
            mime: "image/*".into(),
            source: Source::TeamsDerived(base.join("denied").unwrap()),
            link: None,
            rejected_host: None,
        };
        let denied = fetcher
            .fetch(0, derived)
            .with_subscriber(logs.subscriber())
            .await
            .err();
        assert_eq!(
            denied.as_deref(),
            Some("Microsoft returned HTTP 401 for this file")
        );
        assert!(!logs.text().contains("Forgetting the cached bot token"));

        let denied = fetcher
            .download(base.join("denied").unwrap(), BOT)
            .with_subscriber(logs.subscriber())
            .await
            .err();
        assert_eq!(
            denied.as_deref(),
            Some("Microsoft returned HTTP 401 for this file")
        );
        assert!(logs.text().contains("Forgetting the cached bot token"));

        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let unreachable = fetcher
            .download(
                Url::parse(&format!("http://127.0.0.1:{port}/x?tempauth=secret-marker")).unwrap(),
                None,
            )
            .with_subscriber(logs.subscriber())
            .await
            .err();
        assert_eq!(unreachable.as_deref(), Some(UNREACHABLE));
        let text = logs.text();
        assert!(text.contains("connect=true"), "{text}");
        assert!(!text.contains("secret-marker"), "{text}");
        assert!(!text.contains("bot-token"), "{text}");
    }

    #[tokio::test]
    async fn oversized_files_are_refused_and_release_their_reservation() {
        let base = microsoft().await;
        let connection = connection();
        let fetcher = fetcher(&connection, memory());
        for route in ["huge", "stream"] {
            let error = fetcher
                .download(base.join(route).unwrap(), None)
                .await
                .err();
            assert_eq!(error.as_deref(), Some(TOO_LARGE), "{route}");
            assert_eq!(fetcher.total.load(Ordering::SeqCst), 0, "{route}");
        }
    }
}
