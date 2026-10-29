//! The media parts of a Vertex AI request. Inline data and Cloud Storage URIs go as they are.
//! Vertex AI fetches an https link itself only when the link is verifiably public; any other
//! link is downloaded here, through the egress guard and within Vertex AI's inline limits, and
//! sent inline. A file type Vertex AI cannot read becomes a note naming it, never a silent drop.
//! A signed link is a credential, so messages and logs name its host at most.

use std::{
    net::IpAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::future::BoxFuture;
use reqwest::{
    Url,
    header::{ACCEPT_ENCODING, CONTENT_RANGE, CONTENT_TYPE, HeaderMap, RANGE},
};
use rig::completion::CompletionError;
use rig::message::DocumentSourceKind;

use super::wire::{Blob, FileData, GenerateContentRequest, Part};
use crate::egress::ProviderEgress;
use crate::history::{Content, ContentType, History, MessageContent, Role};

const MB: u64 = 1_000_000;
/// Vertex AI's documented inline limits.
const INLINE_IMAGE_BYTES: u64 = 7 * MB;
const INLINE_PDF_BYTES: u64 = 50 * MB;
const INLINE_TEXT_BYTES: u64 = 7 * MB;
/// Vertex AI documents no inline limit for audio and video; this bounds what is downloaded.
const INLINE_MEDIA_BYTES: u64 = 20 * MB;
const UNTYPED_DOWNLOAD_BYTES: u64 = INLINE_PDF_BYTES;
/// The largest file Vertex AI fetches from an HTTP link.
const LINK_BYTES: u64 = 15 * MB;
const LINK_MAX_CHARS: usize = 8192;
const LINK_IMAGES: usize = 10;
const LINK_AUDIO: usize = 1;
const LINK_VIDEO: usize = 1;
/// A signed link Vertex AI fetches must outlive the call.
const LINK_VALIDITY: Duration = Duration::from_secs(15 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const REDIRECTS: usize = 5;
const OCTET_STREAM: &str = "application/octet-stream";

/// The content kind a part arrives as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Audio,
    Video,
    Document,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Document => "document",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Image => "an image",
            Self::Audio => "audio",
            Self::Video => "a video",
            Self::Document => "a document",
        }
    }
}

/// What Vertex AI does with a MIME type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Image,
    Pdf,
    Audio,
    Video,
    Text,
    Unsupported,
}

impl Class {
    fn of(mime: &str) -> Self {
        match mime {
            "image/jpeg" | "image/png" | "image/webp" | "image/heic" | "image/heif" => Self::Image,
            "application/pdf" => Self::Pdf,
            "audio/aac" | "audio/x-aac" | "audio/aiff" | "audio/flac" | "audio/mp3"
            | "audio/mpeg" | "audio/mpga" | "audio/m4a" | "audio/mp4" | "audio/ogg"
            | "audio/opus" | "audio/pcm" | "audio/wav" | "audio/webm" => Self::Audio,
            "video/mp4" | "video/mpeg" | "video/mpg" | "video/mpegs" | "video/quicktime"
            | "video/avi" | "video/x-flv" | "video/webm" | "video/wmv" | "video/3gpp" => {
                Self::Video
            }
            _ if is_text(mime) => Self::Text,
            _ => Self::Unsupported,
        }
    }

    fn inline_limit(self) -> Option<u64> {
        match self {
            Self::Image => Some(INLINE_IMAGE_BYTES),
            Self::Pdf => Some(INLINE_PDF_BYTES),
            Self::Text => Some(INLINE_TEXT_BYTES),
            Self::Audio | Self::Video | Self::Unsupported => None,
        }
    }

    fn download_limit(self) -> u64 {
        self.inline_limit().unwrap_or(INLINE_MEDIA_BYTES)
    }

    fn binary(self) -> bool {
        matches!(self, Self::Image | Self::Pdf | Self::Audio | Self::Video)
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Image => "images",
            Self::Pdf => "PDFs",
            Self::Audio => "audio files",
            Self::Video => "videos",
            Self::Text => "text documents",
            Self::Unsupported => "files",
        }
    }

    fn kind(self) -> Kind {
        match self {
            Self::Image => Kind::Image,
            Self::Audio => Kind::Audio,
            Self::Video => Kind::Video,
            Self::Pdf | Self::Text | Self::Unsupported => Kind::Document,
        }
    }
}

fn is_text(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-javascript"
                | "application/x-python"
                | "application/rtf"
                | "application/yaml"
                | "application/x-yaml"
                | "application/toml"
                | "application/sql"
                | "application/x-sh"
                | "application/x-ndjson"
        )
        || mime
            .strip_prefix("application/")
            .is_some_and(|subtype| subtype.ends_with("+json") || subtype.ends_with("+xml"))
}

/// The essence of `mime` as Vertex AI spells it, or `None` when it names no type.
fn known_mime(mime: &str) -> Option<String> {
    let essence = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let canonical = match essence.as_str() {
        "" | OCTET_STREAM | "binary/octet-stream" => return None,
        "image/jpg" | "image/pjpeg" => "image/jpeg",
        "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => "audio/wav",
        "audio/x-mp3" | "audio/mpeg3" => "audio/mp3",
        "audio/x-m4a" => "audio/m4a",
        "audio/x-flac" => "audio/flac",
        "audio/x-aiff" => "audio/aiff",
        "video/mov" => "video/quicktime",
        "video/x-msvideo" | "video/msvideo" => "video/avi",
        "video/x-ms-wmv" => "video/wmv",
        "application/x-pdf" => "application/pdf",
        _ => return Some(essence),
    };
    Some(canonical.to_string())
}

fn note_text(mime: &str) -> String {
    format!("[attachment omitted: unsupported file type {mime}]")
}

fn note(mime: &str) -> Part {
    tracing::warn!(
        mime,
        "Vertex AI cannot read this file type; sending a note in its place"
    );
    Part::text(note_text(mime))
}

fn text_part(mime: &str, text: &str) -> Part {
    Part::text(format!("[document: {mime}]\n{text}"))
}

fn decode_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).into_owned()
}

fn is_utf8_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// The type the leading bytes of a file announce.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    let at = |offset: usize, magic: &[u8]| bytes.get(offset..offset + magic.len()) == Some(magic);
    let mime = if at(0, b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if at(0, b"\xFF\xD8\xFF") {
        "image/jpeg"
    } else if at(0, b"GIF87a") || at(0, b"GIF89a") {
        "image/gif"
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        "image/webp"
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        "audio/wav"
    } else if at(0, b"RIFF") && at(8, b"AVI ") {
        "video/avi"
    } else if at(0, b"BM") && at(6, b"\0\0\0\0") {
        "image/bmp"
    } else if at(0, b"II*\0") || at(0, b"MM\0*") {
        "image/tiff"
    } else if at(0, b"%PDF-") {
        "application/pdf"
    } else if at(4, b"ftyp") {
        match bytes.get(8..12) {
            Some(b"heic" | b"heix" | b"hevc" | b"hevx") => "image/heic",
            Some(b"mif1" | b"msf1") => "image/heif",
            Some(b"avif" | b"avis") => "image/avif",
            Some(b"qt  ") => "video/quicktime",
            Some(b"M4A " | b"M4B ") => "audio/m4a",
            Some(b"3gp4" | b"3gp5" | b"3gp6" | b"3g2a") => "video/3gpp",
            _ => "video/mp4",
        }
    } else if at(0, b"ID3") {
        "audio/mp3"
    } else if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xF6 == 0xF0 {
        "audio/aac"
    } else if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0 {
        "audio/mp3"
    } else if at(0, b"fLaC") {
        "audio/flac"
    } else if at(0, b"OggS") {
        "audio/ogg"
    } else if at(0, b"\x1A\x45\xDF\xA3") {
        "video/webm"
    } else if at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) {
        "audio/aiff"
    } else if at(0, b"PK\x03\x04") {
        office_type(bytes)
    } else if at(0, b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1") {
        "application/vnd.ms-office"
    } else {
        return None;
    };
    Some(mime)
}

/// Office Open XML files are ZIP archives that name their part folders in plain text.
fn office_type(zip: &[u8]) -> &'static str {
    let contains = |needle: &[u8]| zip.windows(needle.len()).any(|window| window == needle);
    if contains(b"word/") {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    } else if contains(b"xl/") {
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    } else if contains(b"ppt/") {
        "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    } else {
        "application/zip"
    }
}

/// The type a file name's extension implies.
fn mime_from_path(path: &str, kind: Kind) -> Option<&'static str> {
    let name = path.rsplit('/').next()?;
    let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
    let mime = match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "avif" => "image/avif",
        "pdf" => "application/pdf",
        "wav" => "audio/wav",
        "mp3" => "audio/mp3",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "m4a" => "audio/m4a",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "aif" | "aiff" => "audio/aiff",
        "webm" if kind == Kind::Audio => "audio/webm",
        "webm" => "video/webm",
        "mp4" | "m4v" => "video/mp4",
        "mpeg" | "mpg" => "video/mpeg",
        "mov" => "video/quicktime",
        "avi" => "video/avi",
        "flv" => "video/x-flv",
        "wmv" => "video/wmv",
        "3gp" => "video/3gpp",
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "xml" => "text/xml",
        "json" => "application/json",
        "js" | "mjs" | "cjs" => "text/javascript",
        "py" => "text/x-python",
        "rtf" => "text/rtf",
        "yaml" | "yml" => "application/yaml",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "doc" => "application/msword",
        "xls" => "application/vnd.ms-excel",
        "ppt" => "application/vnd.ms-powerpoint",
        "zip" => "application/zip",
        _ => return None,
    };
    Some(mime)
}

/// The metadata and payload of a `data:` URL.
fn split_data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url
        .get(..5)
        .filter(|scheme| scheme.eq_ignore_ascii_case("data:"))
        .map(|_| &url[5..])?;
    rest.split_once(',')
}

fn data_url_mime(metadata: &str) -> Option<String> {
    metadata.split(';').next().and_then(known_mime)
}

fn data_url_is_base64(metadata: &str) -> bool {
    metadata
        .split(';')
        .skip(1)
        .any(|parameter| parameter.trim().eq_ignore_ascii_case("base64"))
}

fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    decoded
}

fn decoded_len(base64: &str) -> u64 {
    let symbols = base64
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace() && *byte != b'=')
        .count() as u64;
    symbols * 3 / 4
}

fn decode_base64(data: &str) -> Result<Vec<u8>, String> {
    let compact: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    STANDARD
        .decode(compact)
        .map_err(|_| "its data is not valid base64".to_string())
}

fn megabytes(bytes: u64) -> String {
    if bytes.is_multiple_of(MB) {
        format!("{} MB", bytes / MB)
    } else {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    }
}

fn check_inline_limit(class: Class, size: u64) -> Result<(), String> {
    match class.inline_limit() {
        Some(limit) if size > limit => Err(format!(
            "it is {}, and Vertex AI takes {} of at most {} inline",
            megabytes(size),
            class.plural(),
            megabytes(limit)
        )),
        _ => Ok(()),
    }
}

fn request_error(message: String) -> CompletionError {
    CompletionError::RequestError(message.into())
}

fn inline_error(kind: Kind, reason: String) -> CompletionError {
    request_error(format!(
        "Vertex AI cannot read the inline {}: {reason}",
        kind.label()
    ))
}

fn link_error(kind: Kind, host: &str, reason: String) -> CompletionError {
    request_error(format!(
        "Vertex AI cannot read the {} at '{host}': {reason}",
        kind.label()
    ))
}

/// Bytes sent inline as `mime`, or as the type the bytes announce.
fn inline_bytes(kind: Kind, mime: Option<String>, bytes: &[u8]) -> Result<Part, String> {
    let mime = mime
        .or_else(|| sniff(bytes).map(str::to_owned))
        .or_else(|| (kind == Kind::Document && is_utf8_text(bytes)).then(|| "text/plain".into()))
        .unwrap_or_else(|| OCTET_STREAM.to_string());
    let class = Class::of(&mime);
    check_inline_limit(class, bytes.len() as u64)?;
    Ok(match class {
        Class::Unsupported => note(&mime),
        Class::Text => text_part(&mime, &decode_text(bytes)),
        _ => Part::inline(mime, STANDARD.encode(bytes)),
    })
}

fn inline_base64(kind: Kind, declared: Option<String>, data: &str) -> Result<Part, String> {
    match declared {
        Some(mime) if Class::of(&mime) == Class::Unsupported => Ok(note(&mime)),
        Some(mime) if Class::of(&mime).binary() => {
            check_inline_limit(Class::of(&mime), decoded_len(data))?;
            let data = data.trim();
            Ok(if data.bytes().any(|byte| byte.is_ascii_whitespace()) {
                Part::inline(mime, data.split_ascii_whitespace().collect::<String>())
            } else {
                Part::inline(mime, data)
            })
        }
        declared => inline_bytes(kind, declared, &decode_base64(data)?),
    }
}

/// A downloaded body as the server labels it, else as its name or bytes suggest. Text served
/// for an image, audio or video link is an error page, not the media.
fn downloaded_part(kind: Kind, hint: Option<String>, download: Download) -> Result<Part, String> {
    let served = download.content_type.as_deref().and_then(known_mime);
    let mime = served
        .or(hint)
        .or_else(|| sniff(&download.bytes).map(str::to_owned));
    if let Some(mime) = mime.as_deref()
        && kind != Kind::Document
        && Class::of(mime) == Class::Text
    {
        return Err(format!("the link serves {mime}, not {}", kind.noun()));
    }
    inline_bytes(kind, mime, &download.bytes)
}

fn cloud_storage_part(
    kind: Kind,
    bucket: &str,
    hint: Option<String>,
    link: &str,
) -> Result<Part, CompletionError> {
    let mime = hint.ok_or_else(|| {
        link_error(
            kind,
            bucket,
            "Vertex AI needs its MIME type, and neither the part nor the file name gives one"
                .to_string(),
        )
    })?;
    Ok(match Class::of(&mime) {
        Class::Unsupported => note(&mime),
        Class::Text => Part::file("text/plain", link),
        _ => Part::file(mime, link),
    })
}

fn is_youtube(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtu.be")
    )
}

/// Links in a request that Vertex AI fetches over HTTP.
fn fetched_by_vertex(file: &FileData) -> bool {
    Url::parse(&file.file_uri)
        .is_ok_and(|url| matches!(url.scheme(), "https" | "http") && !is_youtube(&url))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The research table's origin, length and encoding checks for a link Vertex AI may fetch.
fn link_shape_allows_fetch(url: &Url, link: &str) -> bool {
    let host = url.host_str().unwrap_or_default();
    let path = url.path().to_ascii_lowercase();
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && !host.is_empty()
        && !host.starts_with('[')
        && host.parse::<IpAddr>().is_err()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && link.len() <= LINK_MAX_CHARS
        && !path.contains("%2f")
        && !path.contains("%5c")
        && !link.contains(['@', ' ', '+'])
        && ProviderEgress::Public.check_url(url).is_ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expiry {
    Unsigned,
    At(u64),
    Unknown,
}

const SIGNATURE_PARAMETERS: [&str; 11] = [
    "x-amz-signature",
    "x-amz-credential",
    "x-amz-security-token",
    "x-goog-signature",
    "x-goog-credential",
    "googleaccessid",
    "awsaccesskeyid",
    "signature",
    "sig",
    "key-pair-id",
    "policy",
];

/// When a signed link stops working: S3 and GCS V4 (`X-*-Date` + `X-*-Expires`), Azure SAS
/// (`se`), CloudFront and legacy signatures (`Expires`).
fn link_expiry(url: &Url) -> Expiry {
    let parameters: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| (key.to_ascii_lowercase(), value.into_owned()))
        .collect();
    let get = |name: &str| {
        parameters
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    if !SIGNATURE_PARAMETERS.iter().any(|name| get(name).is_some()) {
        return Expiry::Unsigned;
    }
    let window = |date: &str, expires: &str| {
        Some(compact_timestamp(get(date)?)? + get(expires)?.parse::<u64>().ok()?)
    };
    window("x-amz-date", "x-amz-expires")
        .or_else(|| window("x-goog-date", "x-goog-expires"))
        .or_else(|| get("se").and_then(iso_timestamp))
        .or_else(|| get("expires").and_then(|value| value.parse::<u64>().ok()))
        .map_or(Expiry::Unknown, Expiry::At)
}

fn link_valid_long_enough(url: &Url, now: u64) -> bool {
    match link_expiry(url) {
        Expiry::Unsigned => true,
        Expiry::At(expiry) => expiry >= now + LINK_VALIDITY.as_secs(),
        Expiry::Unknown => false,
    }
}

fn number(text: &str, range: std::ops::Range<usize>) -> Option<u32> {
    let digits = text.get(range)?;
    digits
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| digits.parse().ok())
        .flatten()
}

/// `20261028T120000Z`
fn compact_timestamp(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    (bytes.len() == 16 && bytes[8] == b'T' && bytes[15] == b'Z').then_some(())?;
    epoch_seconds(
        number(text, 0..4)?,
        number(text, 4..6)?,
        number(text, 6..8)?,
        number(text, 9..11)?,
        number(text, 11..13)?,
        number(text, 13..15)?,
    )
}

/// `2026-10-28`, `2026-10-28T12:00Z` or `2026-10-28T12:00:00[.fraction]Z`
fn iso_timestamp(text: &str) -> Option<u64> {
    let text = text.trim();
    let (date, time) = text.split_once('T').unwrap_or((text, "00:00:00Z"));
    let date_bytes = date.as_bytes();
    (date_bytes.len() == 10 && date_bytes[4] == b'-' && date_bytes[7] == b'-').then_some(())?;
    let time = time.strip_suffix('Z')?.split('.').next()?;
    let time_bytes = time.as_bytes();
    let second = match time_bytes.len() {
        5 => 0,
        8 if time_bytes[5] == b':' => number(time, 6..8)?,
        _ => return None,
    };
    (time_bytes[2] == b':').then_some(())?;
    epoch_seconds(
        number(date, 0..4)?,
        number(date, 5..7)?,
        number(date, 8..10)?,
        number(time, 0..2)?,
        number(time, 3..5)?,
        second,
    )
}

/// Seconds since the Unix epoch of a UTC civil time.
fn epoch_seconds(
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<u64> {
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    u64::try_from(
        days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second),
    )
    .ok()
}

/// The answer to a ranged GET of a link's first byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub status: u16,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}

/// The MIME type of a link Vertex AI may fetch itself: it answered directly, its size is known
/// and within Vertex AI's link limit, and its type is the one the part declares and one Vertex AI
/// reads.
fn probe_verdict(probe: &Probe, hint: Option<&str>) -> Option<String> {
    if !matches!(probe.status, 200 | 206) {
        return None;
    }
    probe.size.filter(|size| (1..=LINK_BYTES).contains(size))?;
    let served = probe.content_type.as_deref().and_then(known_mime)?;
    if hint.is_some_and(|hint| known_mime(hint).as_deref() != Some(served.as_str())) {
        return None;
    }
    Class::of(&served).binary().then_some(served)
}

fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.trim().parse().ok()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Download {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

/// The network side of media resolution.
pub trait MediaNetwork: Send + Sync {
    /// A ranged GET of the first byte of `url`, as a public client sees it, when it answers in
    /// time.
    fn probe<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Option<Probe>>;

    /// The body of `url`, at most `limit` bytes. The error names no URL.
    fn download<'a>(&'a self, url: &'a Url, limit: u64) -> BoxFuture<'a, Result<Download, String>>;
}

/// Probes through the public egress policy and downloads through the configured one, following
/// at most [`REDIRECTS`] redirects that the policy allows.
pub struct GuardedNetwork {
    egress: ProviderEgress,
    probe: reqwest::Client,
    download: reqwest::Client,
}

impl GuardedNetwork {
    pub fn new(egress: ProviderEgress) -> anyhow::Result<Self> {
        Self::with_builders(
            egress,
            reqwest::Client::builder(),
            reqwest::Client::builder(),
        )
    }

    fn with_builders(
        egress: ProviderEgress,
        probe: reqwest::ClientBuilder,
        download: reqwest::ClientBuilder,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            egress,
            probe: ProviderEgress::Public
                .guard(probe)
                .timeout(PROBE_TIMEOUT)
                .no_gzip()
                .no_brotli()
                .build()?,
            download: egress
                .guard(download)
                .redirect(checked_redirects(egress))
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(DOWNLOAD_TIMEOUT)
                .build()?,
        })
    }
}

fn checked_redirects(egress: ProviderEgress) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() > REDIRECTS {
            attempt.error("the link redirects too often")
        } else if let Err(refused) = egress.check_url(attempt.url()) {
            attempt.error(refused)
        } else {
            attempt.follow()
        }
    })
}

fn header(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// A transport error without its URL, with its causes.
pub fn describe_http_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        return "the request timed out".to_string();
    }
    let error = error.without_url();
    let mut message = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    scrub_links(&message)
}

impl MediaNetwork for GuardedNetwork {
    fn probe<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Option<Probe>> {
        Box::pin(self.probe_link(url))
    }

    fn download<'a>(&'a self, url: &'a Url, limit: u64) -> BoxFuture<'a, Result<Download, String>> {
        Box::pin(self.download_link(url, limit))
    }
}

impl GuardedNetwork {
    async fn probe_link(&self, url: &Url) -> Option<Probe> {
        let response = self
            .probe
            .get(url.clone())
            .header(RANGE, "bytes=0-0")
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .await
            .ok()?;
        let status = response.status().as_u16();
        let size = match status {
            206 => header(response.headers(), CONTENT_RANGE)
                .as_deref()
                .and_then(content_range_total),
            _ => response.content_length(),
        };
        Some(Probe {
            status,
            content_type: header(response.headers(), CONTENT_TYPE),
            size,
        })
    }

    async fn download_link(&self, url: &Url, limit: u64) -> Result<Download, String> {
        let too_large = || {
            format!(
                "it is larger than {}, the most this client downloads to send inline",
                megabytes(limit)
            )
        };
        self.egress
            .check_url(url)
            .map_err(|refused| refused.to_string())?;
        let mut response = self
            .download
            .get(url.clone())
            .send()
            .await
            .map_err(describe_http_error)?;
        if !response.status().is_success() {
            return Err(format!(
                "the download answered HTTP {}",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit)
        {
            return Err(too_large());
        }
        let content_type = header(response.headers(), CONTENT_TYPE);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(describe_http_error)? {
            if (bytes.len() + chunk.len()) as u64 > limit {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Download {
            bytes,
            content_type,
        })
    }
}

/// Replaces each link in `text` with its scheme and host.
pub fn scrub_links(text: &str) -> String {
    const SCHEMES: [&str; 3] = ["https://", "http://", "gs://"];
    let lowercase = text.to_ascii_lowercase();
    let mut scrubbed = String::with_capacity(text.len());
    let mut copied = 0;
    while let Some((start, scheme)) = SCHEMES
        .iter()
        .filter_map(|scheme| {
            lowercase[copied..]
                .find(scheme)
                .map(|offset| (copied + offset, *scheme))
        })
        .min_by_key(|(start, _)| *start)
    {
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
            .map_or(text.len(), |offset| start + offset);
        let link = text[start..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']']);
        let end = start + link.len();
        scrubbed.push_str(&text[copied..start]);
        match Url::parse(link)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
        {
            Some(host) => {
                scrubbed.push_str(scheme);
                scrubbed.push_str(&host);
                scrubbed.push_str("/…");
            }
            None => scrubbed.push_str("[link]"),
        }
        copied = end;
    }
    scrubbed.push_str(&text[copied..]);
    scrubbed
}

/// A media part of a request as Rig carries it.
#[derive(Debug, Clone)]
pub struct MediaItem {
    pub kind: Kind,
    /// The MIME type Rig carries for the part.
    pub declared: Option<String>,
    pub source: DocumentSourceKind,
    /// Vertex AI may fetch this part's link itself.
    pub may_link: bool,
}

/// How many links of each kind Vertex AI fetches for one request.
#[derive(Debug, Default)]
pub struct LinkQuota {
    images: usize,
    audio: usize,
    video: usize,
}

impl LinkQuota {
    /// Whether Vertex AI may fetch the link of a part of `kind` that `source` holds.
    pub fn take(&mut self, kind: Kind, source: &DocumentSourceKind) -> bool {
        let DocumentSourceKind::Url(link) = source else {
            return false;
        };
        if !Url::parse(link.trim()).is_ok_and(|url| url.scheme() == "https") {
            return false;
        }
        let (used, limit) = match kind {
            Kind::Image => (&mut self.images, LINK_IMAGES),
            Kind::Audio => (&mut self.audio, LINK_AUDIO),
            Kind::Video => (&mut self.video, LINK_VIDEO),
            Kind::Document => return true,
        };
        *used += 1;
        *used <= limit
    }
}

/// Turns media parts into Vertex AI parts.
#[derive(Clone)]
pub struct MediaResolver {
    network: Arc<dyn MediaNetwork>,
}

impl MediaResolver {
    pub fn new(egress: ProviderEgress) -> anyhow::Result<Self> {
        Ok(Self::with_network(Arc::new(GuardedNetwork::new(egress)?)))
    }

    pub fn with_network(network: Arc<dyn MediaNetwork>) -> Self {
        Self { network }
    }

    pub async fn resolve(&self, item: &MediaItem) -> Result<Part, CompletionError> {
        let declared = item.declared.as_deref().and_then(known_mime);
        match &item.source {
            DocumentSourceKind::String(text) if item.kind == Kind::Document => {
                check_inline_limit(Class::Text, text.len() as u64)
                    .map_err(|reason| inline_error(item.kind, reason))?;
                Ok(text_part(declared.as_deref().unwrap_or("text/plain"), text))
            }
            DocumentSourceKind::String(data) | DocumentSourceKind::Base64(data) => {
                inline_base64(item.kind, declared, data)
                    .map_err(|reason| inline_error(item.kind, reason))
            }
            DocumentSourceKind::Raw(bytes) => inline_bytes(item.kind, declared, bytes)
                .map_err(|reason| inline_error(item.kind, reason)),
            DocumentSourceKind::Url(link) => self.resolve_link(item, declared, link.trim()).await,
            DocumentSourceKind::FileId(_) => Err(request_error(format!(
                "Vertex AI cannot read the {} sent as a provider file id",
                item.kind.label()
            ))),
            _ => Err(request_error(format!(
                "The {} has no content to send to Vertex AI",
                item.kind.label()
            ))),
        }
    }

    async fn resolve_link(
        &self,
        item: &MediaItem,
        declared: Option<String>,
        link: &str,
    ) -> Result<Part, CompletionError> {
        if let Some((metadata, payload)) = split_data_url(link) {
            let mime = declared.or_else(|| data_url_mime(metadata));
            let part = if data_url_is_base64(metadata) {
                inline_base64(item.kind, mime, payload)
            } else {
                inline_bytes(item.kind, mime, &percent_decode(payload))
            };
            return part.map_err(|reason| inline_error(item.kind, reason));
        }
        let url = Url::parse(link).map_err(|_| {
            request_error(format!("The {} link is not a valid URL", item.kind.label()))
        })?;
        let host = url.host_str().unwrap_or_default().to_string();
        let hint = declared.or_else(|| mime_from_path(url.path(), item.kind).map(str::to_owned));
        let class = hint.as_deref().map(Class::of);
        if let Some(mime) = hint.as_deref()
            && class == Some(Class::Unsupported)
        {
            return Ok(note(mime));
        }
        match url.scheme() {
            "gs" => return cloud_storage_part(item.kind, &host, hint, link),
            "https" | "http" => {}
            scheme => {
                return Err(link_error(
                    item.kind,
                    &host,
                    format!("Vertex AI reads https and gs:// links, not {scheme}:"),
                ));
            }
        }
        if item.kind == Kind::Video && is_youtube(&url) {
            return if item.may_link {
                Ok(Part::file(hint.unwrap_or_else(|| "video/mp4".into()), link))
            } else {
                Err(link_error(
                    item.kind,
                    &host,
                    "Vertex AI takes one video link per request".to_string(),
                ))
            };
        }
        if item.may_link
            && class.is_none_or(Class::binary)
            && let Some(mime) = self
                .public_link(&url, link, hint.as_deref(), now_secs())
                .await
        {
            tracing::debug!(
                host = %host,
                kind = item.kind.label(),
                "Vertex AI fetches the media link itself"
            );
            return Ok(Part::file(mime, link));
        }
        let limit = class.map_or(UNTYPED_DOWNLOAD_BYTES, Class::download_limit);
        let download = self
            .network
            .download(&url, limit)
            .await
            .map_err(|reason| link_error(item.kind, &host, reason))?;
        tracing::debug!(
            host = %host,
            kind = item.kind.label(),
            bytes = download.bytes.len(),
            "Downloaded a media link to send it inline"
        );
        downloaded_part(item.kind, hint, download)
            .map_err(|reason| link_error(item.kind, &host, reason))
    }

    async fn public_link(
        &self,
        url: &Url,
        link: &str,
        hint: Option<&str>,
        now: u64,
    ) -> Option<String> {
        if !link_shape_allows_fetch(url, link) || !link_valid_long_enough(url, now) {
            return None;
        }
        let probe = self.network.probe(url).await?;
        probe_verdict(&probe, hint)
    }

    /// Sends every link Vertex AI was to fetch inline instead, after it failed to fetch one.
    pub async fn inline_links(
        &self,
        body: &mut GenerateContentRequest,
    ) -> Result<(), CompletionError> {
        for part in body.parts_mut() {
            if let Some(file) = part.file_data.take_if(|file| fetched_by_vertex(file)) {
                part.inline_data = Some(self.download_blob(&file).await?);
            }
            for response_part in part
                .function_response
                .iter_mut()
                .flat_map(|response| response.parts.iter_mut())
            {
                if let Some(file) = response_part
                    .file_data
                    .take_if(|file| fetched_by_vertex(file))
                {
                    response_part.inline_data = Some(self.download_blob(&file).await?);
                }
            }
        }
        Ok(())
    }

    async fn download_blob(&self, file: &FileData) -> Result<Blob, CompletionError> {
        let class = Class::of(&file.mime_type);
        let kind = class.kind();
        let url = Url::parse(&file.file_uri)
            .map_err(|_| request_error(format!("The {} link is not a valid URL", kind.label())))?;
        let host = url.host_str().unwrap_or_default().to_string();
        let download = self
            .network
            .download(&url, class.download_limit())
            .await
            .map_err(|reason| link_error(kind, &host, reason))?;
        check_inline_limit(class, download.bytes.len() as u64)
            .map_err(|reason| link_error(kind, &host, reason))?;
        Ok(Blob {
            mime_type: file.mime_type.clone(),
            data: STANDARD.encode(&download.bytes),
        })
    }
}

/// Whether `body` lets Vertex AI fetch any link itself.
pub fn has_fetched_links(body: &GenerateContentRequest) -> bool {
    body.contents
        .iter()
        .flat_map(|content| &content.parts)
        .any(|part| {
            part.file_data.as_ref().is_some_and(fetched_by_vertex)
                || part.function_response.as_ref().is_some_and(|response| {
                    response
                        .parts
                        .iter()
                        .any(|part| part.file_data.as_ref().is_some_and(fetched_by_vertex))
                })
        })
}

/// Replaces each user attachment whose type Vertex AI cannot read with a note naming the type
/// the user sent, which Rig's media types cannot always represent.
pub fn note_unreadable_attachments(history: &mut History) {
    for message in history
        .messages
        .iter_mut()
        .filter(|message| message.role == Role::User)
    {
        let MessageContent::Contents(contents) = &mut message.content else {
            continue;
        };
        for content in contents.iter_mut() {
            if let Some(mime) = unreadable_mime(content) {
                tracing::warn!(
                    mime = %mime,
                    "Vertex AI cannot read this attachment type; sending a note in its place"
                );
                *content = Content::Text {
                    content_type: ContentType::Text,
                    text: note_text(&mime),
                };
            }
        }
    }
}

fn unreadable_mime(content: &Content) -> Option<String> {
    let (link, media_type, kind) = match content {
        Content::Text { .. } => return None,
        Content::Image { image_url, .. } => (
            image_url.url.as_str(),
            image_url.media_type.as_deref(),
            Kind::Image,
        ),
        Content::Audio {
            audio_url,
            media_type,
            ..
        } => (audio_url.as_str(), media_type.as_deref(), Kind::Audio),
        Content::Video {
            video_url,
            media_type,
            ..
        } => (video_url.as_str(), media_type.as_deref(), Kind::Video),
        Content::Document {
            document_url,
            media_type,
            ..
        } => (document_url.as_str(), media_type.as_deref(), Kind::Document),
    };
    let mime = match split_data_url(link) {
        Some((metadata, _)) => data_url_mime(metadata).or_else(|| media_type.and_then(known_mime)),
        None => media_type.and_then(known_mime).or_else(|| {
            Url::parse(link.trim())
                .ok()
                .and_then(|url| mime_from_path(url.path(), kind))
                .map(str::to_owned)
        }),
    }?;
    (Class::of(&mime) == Class::Unsupported).then_some(mime)
}

#[cfg(test)]
pub(super) mod fake {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Answers every probe with `probe` and serves `downloads` by URL.
    #[derive(Default)]
    pub struct FakeNetwork {
        pub probe: Option<Probe>,
        pub downloads: HashMap<String, Download>,
        pub probed: Mutex<Vec<String>>,
        pub downloaded: Mutex<Vec<String>>,
    }

    impl FakeNetwork {
        fn serve(&self, url: &Url, limit: u64) -> Result<Download, String> {
            self.downloaded.lock().unwrap().push(url.to_string());
            let download = self
                .downloads
                .get(url.as_str())
                .cloned()
                .ok_or_else(|| "the download answered HTTP 404".to_string())?;
            if download.bytes.len() as u64 > limit {
                return Err(format!("it is larger than {}", megabytes(limit)));
            }
            Ok(download)
        }
    }

    impl MediaNetwork for FakeNetwork {
        fn probe<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Option<Probe>> {
            self.probed.lock().unwrap().push(url.to_string());
            Box::pin(std::future::ready(self.probe.clone()))
        }

        fn download<'a>(
            &'a self,
            url: &'a Url,
            limit: u64,
        ) -> BoxFuture<'a, Result<Download, String>> {
            Box::pin(std::future::ready(self.serve(url, limit)))
        }
    }

    pub fn public_probe(content_type: &str, size: u64) -> Probe {
        Probe {
            status: 206,
            content_type: Some(content_type.to_string()),
            size: Some(size),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeNetwork, public_probe};
    use super::*;
    use crate::history::{HistoryMessage, ImageUrl};
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    const PUBLIC_PHOTO: &str = "https://cdn.example.com/photos/cat.png";

    fn item(kind: Kind, declared: Option<&str>, source: DocumentSourceKind) -> MediaItem {
        MediaItem {
            kind,
            declared: declared.map(str::to_owned),
            source,
            may_link: true,
        }
    }

    fn resolver(network: FakeNetwork) -> (MediaResolver, Arc<FakeNetwork>) {
        let network = Arc::new(network);
        (MediaResolver::with_network(network.clone()), network)
    }

    async fn resolve(network: FakeNetwork, item: MediaItem) -> Result<Part, CompletionError> {
        resolver(network).0.resolve(&item).await
    }

    #[tokio::test]
    async fn inline_media_keeps_its_type_and_data() {
        let base64 = STANDARD.encode(PNG);
        for (kind, declared, mime) in [
            (Kind::Image, Some("image/png"), "image/png"),
            (Kind::Image, Some("image/jpg"), "image/jpeg"),
            (Kind::Document, Some("application/pdf"), "application/pdf"),
            (Kind::Audio, Some("audio/mpeg"), "audio/mpeg"),
            (Kind::Audio, Some("audio/x-wav"), "audio/wav"),
            (Kind::Video, Some("video/mov"), "video/quicktime"),
            (Kind::Video, Some("video/mp4"), "video/mp4"),
            (Kind::Image, None, "image/png"),
        ] {
            let part = resolve(
                FakeNetwork::default(),
                item(kind, declared, DocumentSourceKind::Base64(base64.clone())),
            )
            .await
            .unwrap();
            assert_eq!(part, Part::inline(mime, base64.as_str()), "{declared:?}");
        }
        let raw = resolve(
            FakeNetwork::default(),
            item(Kind::Image, None, DocumentSourceKind::Raw(PNG.to_vec())),
        )
        .await
        .unwrap();
        assert_eq!(raw, Part::inline("image/png", STANDARD.encode(PNG)));
        let (head, tail) = base64.split_at(8);
        let data_url = format!("data:image/png;base64,{head}\r\n{tail}");
        let from_data_url = resolve(
            FakeNetwork::default(),
            item(Kind::Image, None, DocumentSourceKind::Url(data_url)),
        )
        .await
        .unwrap();
        assert_eq!(from_data_url, Part::inline("image/png", base64.as_str()));
    }

    #[tokio::test]
    async fn text_documents_become_text_parts() {
        let csv = resolve(
            FakeNetwork::default(),
            item(
                Kind::Document,
                Some("text/csv"),
                DocumentSourceKind::String("a,b\n1,2".into()),
            ),
        )
        .await
        .unwrap();
        assert_eq!(csv, Part::text("[document: text/csv]\na,b\n1,2"));
        let json = resolve(
            FakeNetwork::default(),
            item(
                Kind::Document,
                None,
                DocumentSourceKind::Url("data:application/json,%7B%22a%22%3A1%7D".into()),
            ),
        )
        .await
        .unwrap();
        assert_eq!(json, Part::text("[document: application/json]\n{\"a\":1}"));
        let untyped = resolve(
            FakeNetwork::default(),
            item(
                Kind::Document,
                None,
                DocumentSourceKind::Base64(STANDARD.encode("plain words")),
            ),
        )
        .await
        .unwrap();
        assert_eq!(untyped, Part::text("[document: text/plain]\nplain words"));
    }

    #[tokio::test]
    async fn unsupported_types_become_a_note_naming_the_type() {
        let docx = b"PK\x03\x04\x14\0\x06\0[Content_Types].xml...word/document.xml";
        for (kind, declared, source, mime) in [
            (
                Kind::Image,
                Some("image/gif"),
                DocumentSourceKind::Base64(STANDARD.encode(b"GIF89a")),
                "image/gif",
            ),
            (
                Kind::Image,
                None,
                DocumentSourceKind::Base64(STANDARD.encode(b"BM\x10\0\0\0\0\0\0\0")),
                "image/bmp",
            ),
            (
                Kind::Document,
                None,
                DocumentSourceKind::Base64(STANDARD.encode(docx)),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ),
            (
                Kind::Document,
                None,
                DocumentSourceKind::Url("https://files.example.com/q3/report.xlsx?sig=abc".into()),
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
            (
                Kind::Audio,
                Some("audio/pcm16"),
                DocumentSourceKind::Base64("AAAA".into()),
                "audio/pcm16",
            ),
        ] {
            let (resolver, network) = resolver(FakeNetwork::default());
            let part = resolver
                .resolve(&item(kind, declared, source))
                .await
                .unwrap();
            assert_eq!(
                part.text.as_deref(),
                Some(format!("[attachment omitted: unsupported file type {mime}]").as_str())
            );
            assert!(network.downloaded.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn cloud_storage_uris_go_as_file_data() {
        for (kind, declared, uri, mime) in [
            (
                Kind::Video,
                None,
                "gs://bucket/clips/intro.mp4",
                "video/mp4",
            ),
            (
                Kind::Image,
                Some("image/webp"),
                "gs://bucket/a",
                "image/webp",
            ),
            (
                Kind::Document,
                None,
                "gs://bucket/q3.pdf",
                "application/pdf",
            ),
            (Kind::Audio, None, "gs://bucket/call.flac", "audio/flac"),
            (Kind::Document, None, "gs://bucket/notes.md", "text/plain"),
        ] {
            let (resolver, network) = resolver(FakeNetwork::default());
            let part = resolver
                .resolve(&item(kind, declared, DocumentSourceKind::Url(uri.into())))
                .await
                .unwrap();
            assert_eq!(part, Part::file(mime, uri));
            assert!(network.probed.lock().unwrap().is_empty());
        }
        let untyped = resolve(
            FakeNetwork::default(),
            item(
                Kind::Video,
                None,
                DocumentSourceKind::Url("gs://bucket/clip".into()),
            ),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(untyped.contains("MIME type"), "{untyped}");
    }

    #[tokio::test]
    async fn verified_public_links_are_fetched_by_vertex() {
        for (kind, link, mime) in [
            (Kind::Image, PUBLIC_PHOTO, "image/png"),
            (
                Kind::Document,
                "https://cdn.example.com/q3.pdf",
                "application/pdf",
            ),
            (Kind::Audio, "https://cdn.example.com/call", "audio/mpeg"),
            (Kind::Video, "https://cdn.example.com/clip.mp4", "video/mp4"),
        ] {
            let (resolver, network) = resolver(FakeNetwork {
                probe: Some(public_probe(mime, 2 * MB)),
                ..FakeNetwork::default()
            });
            let part = resolver
                .resolve(&item(kind, None, DocumentSourceKind::Url(link.into())))
                .await
                .unwrap();
            assert_eq!(part, Part::file(mime, link));
            assert_eq!(network.probed.lock().unwrap().len(), 1);
            assert!(network.downloaded.lock().unwrap().is_empty());
        }

        let (resolver, _) = resolver(FakeNetwork::default());
        let youtube = "https://www.youtube.com/watch?v=abc";
        let video = resolver
            .resolve(&item(
                Kind::Video,
                None,
                DocumentSourceKind::Url(youtube.into()),
            ))
            .await
            .unwrap();
        assert_eq!(video, Part::file("video/mp4", youtube));
    }

    #[tokio::test]
    async fn links_that_fail_verification_are_downloaded_and_inlined() {
        let signed_soon = format!(
            "https://bucket.s3.amazonaws.com/cat.png?X-Amz-Date={}&X-Amz-Expires=60&X-Amz-Signature=abc",
            "20260101T000000Z"
        );
        let cases = [
            (
                PUBLIC_PHOTO.to_string(),
                Some(public_probe("image/png", 20 * MB)),
                true,
            ),
            (
                PUBLIC_PHOTO.to_string(),
                Some(public_probe("text/html", MB)),
                true,
            ),
            (PUBLIC_PHOTO.to_string(), None, true),
            (
                "https://10.0.0.5/cat.png".to_string(),
                Some(public_probe("image/png", MB)),
                false,
            ),
            (signed_soon, Some(public_probe("image/png", MB)), false),
        ];
        for (link, probe, probes) in cases {
            let (resolver, network) = resolver(FakeNetwork {
                probe,
                downloads: HashMap::from([(
                    Url::parse(&link).unwrap().to_string(),
                    Download {
                        bytes: PNG.to_vec(),
                        content_type: Some("image/png".into()),
                    },
                )]),
                ..FakeNetwork::default()
            });
            let part = resolver
                .resolve(&item(
                    Kind::Image,
                    None,
                    DocumentSourceKind::Url(link.clone()),
                ))
                .await
                .unwrap();
            assert_eq!(
                part,
                Part::inline("image/png", STANDARD.encode(PNG)),
                "{link}"
            );
            assert_eq!(
                network.probed.lock().unwrap().len(),
                usize::from(probes),
                "{link}"
            );
            assert_eq!(network.downloaded.lock().unwrap().len(), 1, "{link}");
        }

        for (kind, link, bytes, served, mime) in [
            (
                Kind::Document,
                "https://files.internal.example/q3.pdf",
                &b"%PDF-1.7"[..],
                "application/pdf",
                "application/pdf",
            ),
            (
                Kind::Audio,
                "http://10.0.0.7/call.mp3",
                &b"ID3\x04"[..],
                "audio/mpeg",
                "audio/mpeg",
            ),
            (
                Kind::Video,
                "https://10.0.0.7/clip.webm",
                &b"\x1A\x45\xDF\xA3"[..],
                OCTET_STREAM,
                "video/webm",
            ),
        ] {
            let (resolver, network) = resolver(FakeNetwork {
                downloads: HashMap::from([(
                    link.to_string(),
                    Download {
                        bytes: bytes.to_vec(),
                        content_type: Some(served.into()),
                    },
                )]),
                ..FakeNetwork::default()
            });
            let part = resolver
                .resolve(&item(kind, None, DocumentSourceKind::Url(link.into())))
                .await
                .unwrap();
            assert_eq!(part, Part::inline(mime, STANDARD.encode(bytes)), "{link}");
            assert_eq!(network.downloaded.lock().unwrap().len(), 1, "{link}");
        }

        let (resolver, network) = resolver(FakeNetwork {
            probe: Some(public_probe("image/png", MB)),
            ..FakeNetwork::default()
        });
        let mut over_quota = item(
            Kind::Image,
            None,
            DocumentSourceKind::Url(PUBLIC_PHOTO.into()),
        );
        over_quota.may_link = false;
        assert!(resolver.resolve(&over_quota).await.is_err());
        assert!(network.probed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_downloads_name_the_kind_and_host_but_never_the_link() {
        let link = "https://storage.example.com/private/scan.pdf?X-Amz-Signature=secret-signature";
        let error = resolve(
            FakeNetwork::default(),
            item(Kind::Document, None, DocumentSourceKind::Url(link.into())),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("document at 'storage.example.com'"),
            "{error}"
        );
        assert!(error.contains("HTTP 404"), "{error}");
        assert!(!error.contains("secret-signature"), "{error}");

        let large = STANDARD.encode(vec![0xFFu8; 8_000_000]);
        let error = resolve(
            FakeNetwork::default(),
            item(
                Kind::Image,
                Some("image/jpeg"),
                DocumentSourceKind::Base64(large),
            ),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("8 MB") && error.contains("7 MB"), "{error}");

        let html = resolve(
            FakeNetwork {
                downloads: HashMap::from([(
                    "http://media.example.com/cat".to_string(),
                    Download {
                        bytes: b"<html>login</html>".to_vec(),
                        content_type: Some("text/html; charset=utf-8".into()),
                    },
                )]),
                ..FakeNetwork::default()
            },
            item(
                Kind::Image,
                None,
                DocumentSourceKind::Url("http://media.example.com/cat".into()),
            ),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(html.contains("serves text/html"), "{html}");
    }

    #[tokio::test]
    async fn private_links_are_downloaded_through_the_egress_guard() {
        let (addr, served) = crate::egress::tests::answer_every_connection(
            "HTTP/1.1 200 OK\r\ncontent-type: image/webp\r\ncontent-length: 16\r\nconnection: close\r\n\r\nRIFF\0\0\0\0WEBPVP8 ",
        )
        .await;
        let network = GuardedNetwork::with_builders(
            ProviderEgress::Private,
            reqwest::Client::builder(),
            reqwest::Client::builder().resolve("media.internal.test", addr),
        )
        .unwrap();
        let resolver = MediaResolver::with_network(Arc::new(network));
        let link = format!(
            "http://media.internal.test:{}/scans/cat?token=1",
            addr.port()
        );
        let part = resolver
            .resolve(&item(Kind::Image, None, DocumentSourceKind::Url(link)))
            .await
            .unwrap();
        assert_eq!(
            part,
            Part::inline("image/webp", STANDARD.encode(b"RIFF\0\0\0\0WEBPVP8 "))
        );
        assert_eq!(served.load(Ordering::SeqCst), 1);

        let public_only = GuardedNetwork::new(ProviderEgress::Public).unwrap();
        let refused = public_only
            .download(
                &Url::parse(&format!("http://127.0.0.1:{}/cat.png", addr.port())).unwrap(),
                MB,
            )
            .await
            .unwrap_err();
        assert!(refused.contains("127.0.0.1"), "{refused}");
        assert_eq!(
            served.load(Ordering::SeqCst),
            1,
            "a refused link is never fetched"
        );
    }

    #[tokio::test]
    async fn downloads_stop_at_the_limit() {
        let (addr, _) = crate::egress::tests::answer_every_connection(
            "HTTP/1.1 200 OK\r\ncontent-type: video/mp4\r\nconnection: close\r\n\r\n0123456789abcdef0123456789abcdef",
        )
        .await;
        let network = GuardedNetwork::with_builders(
            ProviderEgress::Private,
            reqwest::Client::builder(),
            reqwest::Client::builder().resolve("media.internal.test", addr),
        )
        .unwrap();
        let url = Url::parse(&format!(
            "http://media.internal.test:{}/clip.mp4",
            addr.port()
        ))
        .unwrap();
        let error = network.download(&url, 16).await.unwrap_err();
        assert!(error.contains("larger than"), "{error}");
        assert_eq!(network.download(&url, 64).await.unwrap().bytes.len(), 32);
    }

    #[test]
    fn link_shape_follows_the_research_checks() {
        for link in [
            "https://cdn.example.com/cat.png",
            "https://cdn.example.com:443/cat.png?v=2",
        ] {
            assert!(
                link_shape_allows_fetch(&Url::parse(link).unwrap(), link),
                "{link}"
            );
        }
        for link in [
            "http://cdn.example.com/cat.png",
            "https://cdn.example.com:8443/cat.png",
            "https://93.184.216.34/cat.png",
            "https://[2606:4700::1111]/cat.png",
            "https://user:pass@cdn.example.com/cat.png",
            "https://cdn.example.com/cat.png#frag",
            "https://cdn.example.com/a%2Fb/cat.png",
            "https://cdn.example.com/a%5cb/cat.png",
            "https://cdn.example.com/cat.png?a=b+c",
            "https://localhost/cat.png",
            "https://metadata.google.internal/cat.png",
        ] {
            assert!(
                !link_shape_allows_fetch(&Url::parse(link).unwrap(), link),
                "{link}"
            );
        }
        let long = format!("https://cdn.example.com/{}.png", "a".repeat(LINK_MAX_CHARS));
        assert!(!link_shape_allows_fetch(&Url::parse(&long).unwrap(), &long));
    }

    #[test]
    fn signed_links_must_outlive_the_call() {
        let now = epoch_seconds(2026, 10, 28, 12, 0, 0).unwrap();
        let check = |link: &str| link_valid_long_enough(&Url::parse(link).unwrap(), now);
        assert!(check("https://cdn.example.com/cat.png"));
        assert!(check(
            "https://b.s3.amazonaws.com/c.png?X-Amz-Date=20261028T115000Z&X-Amz-Expires=3600&X-Amz-Signature=s"
        ));
        assert!(!check(
            "https://b.s3.amazonaws.com/c.png?X-Amz-Date=20261028T115000Z&X-Amz-Expires=900&X-Amz-Signature=s"
        ));
        assert!(check(
            "https://storage.googleapis.com/b/c.png?X-Goog-Date=20261028T120000Z&X-Goog-Expires=7200&X-Goog-Signature=s"
        ));
        assert!(check(
            "https://a.blob.core.windows.net/c/d.png?sv=2024&se=2026-10-28T14:00:00Z&sig=s"
        ));
        assert!(!check(
            "https://a.blob.core.windows.net/c/d.png?se=2026-10-28T12:05:00.0000000Z&sig=s"
        ));
        assert!(check(
            "https://a.blob.core.windows.net/c/d.png?se=2026-10-29&sig=s"
        ));
        assert!(check(&format!(
            "https://d1.cloudfront.net/c.png?Expires={}&Signature=s&Key-Pair-Id=k",
            now + 3600
        )));
        assert!(!check(
            "https://d1.cloudfront.net/c.png?Policy=eyJ9&Signature=s&Key-Pair-Id=k"
        ));
        assert!(!check("https://b.s3.amazonaws.com/c.png?X-Amz-Signature=s"));
        assert_eq!(epoch_seconds(1970, 1, 1, 0, 0, 0), Some(0));
        assert_eq!(compact_timestamp("20000301T000000Z"), Some(951_868_800));
        assert_eq!(iso_timestamp("2000-03-01T00:00Z"), Some(951_868_800));
    }

    #[test]
    fn probes_pass_only_live_typed_links_within_the_limit() {
        let ok = public_probe("image/png", MB);
        assert_eq!(probe_verdict(&ok, None).as_deref(), Some("image/png"));
        assert_eq!(
            probe_verdict(&ok, Some("image/png")).as_deref(),
            Some("image/png")
        );
        assert_eq!(probe_verdict(&ok, Some("image/jpeg")), None);
        let full = Probe {
            status: 200,
            content_type: Some("application/pdf".into()),
            size: Some(LINK_BYTES),
        };
        assert_eq!(
            probe_verdict(&full, None).as_deref(),
            Some("application/pdf")
        );
        for probe in [
            Probe {
                status: 302,
                ..ok.clone()
            },
            Probe {
                status: 403,
                ..ok.clone()
            },
            Probe {
                size: None,
                ..ok.clone()
            },
            Probe {
                size: Some(LINK_BYTES + 1),
                ..ok.clone()
            },
            Probe {
                content_type: None,
                ..ok.clone()
            },
            Probe {
                content_type: Some(OCTET_STREAM.into()),
                ..ok.clone()
            },
            Probe {
                content_type: Some("text/plain".into()),
                ..ok.clone()
            },
            Probe {
                content_type: Some("image/gif".into()),
                ..ok.clone()
            },
        ] {
            assert_eq!(probe_verdict(&probe, None), None, "{probe:?}");
        }
        assert_eq!(content_range_total("bytes 0-0/12345"), Some(12_345));
        assert_eq!(content_range_total("bytes 0-0/*"), None);
    }

    #[test]
    fn link_quota_counts_https_links_per_kind() {
        let link = DocumentSourceKind::Url(PUBLIC_PHOTO.into());
        let mut quota = LinkQuota::default();
        assert!((0..LINK_IMAGES).all(|_| quota.take(Kind::Image, &link)));
        assert!(!quota.take(Kind::Image, &link));
        assert!(quota.take(Kind::Video, &link));
        assert!(!quota.take(Kind::Video, &link));
        assert!(quota.take(Kind::Audio, &link));
        assert!(!quota.take(Kind::Audio, &link));
        assert!(quota.take(Kind::Document, &link));
        assert!(!quota.take(Kind::Video, &DocumentSourceKind::Url("gs://b/c.mp4".into())));
        assert!(!quota.take(Kind::Image, &DocumentSourceKind::Base64("AAAA".into())));
    }

    #[test]
    fn sniffing_names_common_formats() {
        let cases: [(&[u8], &str); 8] = [
            (b"\xFF\xD8\xFF\xE0", "image/jpeg"),
            (b"RIFF\0\0\0\0WEBPVP8 ", "image/webp"),
            (b"\0\0\0\x18ftypheic", "image/heic"),
            (b"%PDF-1.7", "application/pdf"),
            (b"ID3\x04", "audio/mp3"),
            (b"\xFF\xF1\x50\x80", "audio/aac"),
            (b"\0\0\0\x20ftypisom", "video/mp4"),
            (
                b"PK\x03\x04xl/workbook.xml",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
        ];
        for (bytes, mime) in cases {
            assert_eq!(sniff(bytes), Some(mime));
        }
        assert_eq!(sniff(b"BMP is a word"), None);
        assert_eq!(sniff(b"plain text"), None);
    }

    #[test]
    fn provider_messages_keep_the_host_of_each_link() {
        let scrubbed = scrub_links(
            "Cannot fetch content from https://bucket.s3.amazonaws.com/a.png?X-Amz-Signature=secret, or \"gs://private-bucket/x.pdf\".",
        );
        assert_eq!(
            scrubbed,
            "Cannot fetch content from https://bucket.s3.amazonaws.com/…, or \"gs://private-bucket/…\"."
        );
        assert_eq!(scrub_links("no links here"), "no links here");
        assert_eq!(scrub_links("broken https:// link"), "broken [link] link");
    }

    #[test]
    fn history_attachments_vertex_cannot_read_become_notes_with_the_type_sent() {
        let docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        let mut history = History::new(
            "gemini-2.5-flash".into(),
            vec![HistoryMessage {
                role: Role::User,
                content: MessageContent::Contents(vec![
                    Content::Text {
                        content_type: ContentType::Text,
                        text: "Summarise these".into(),
                    },
                    Content::Document {
                        content_type: ContentType::DocumentUrl,
                        document_url: format!("data:{docx};base64,UEsDBA=="),
                        media_type: Some(docx.into()),
                        additional_params: None,
                    },
                    Content::Image {
                        content_type: ContentType::ImageUrl,
                        image_url: ImageUrl {
                            url: "https://cdn.example.com/scan.tiff".into(),
                            detail: None,
                            media_type: None,
                            additional_params: None,
                        },
                    },
                    Content::Image {
                        content_type: ContentType::ImageUrl,
                        image_url: ImageUrl {
                            url: "data:application/octet-stream;base64,iVBORw0KGgo=".into(),
                            detail: None,
                            media_type: None,
                            additional_params: None,
                        },
                    },
                    Content::Document {
                        content_type: ContentType::DocumentUrl,
                        document_url: "https://cdn.example.com/brief.pdf".into(),
                        media_type: Some("application/pdf".into()),
                        additional_params: None,
                    },
                ]),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                annotations: None,
            }],
        );
        note_unreadable_attachments(&mut history);
        let MessageContent::Contents(contents) = &history.messages[0].content else {
            panic!("contents");
        };
        let texts: Vec<Option<&str>> = contents
            .iter()
            .map(|content| match content {
                Content::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts,
            vec![
                Some("Summarise these"),
                Some(format!("[attachment omitted: unsupported file type {docx}]").as_str()),
                Some("[attachment omitted: unsupported file type image/tiff]"),
                None,
                None,
            ]
        );
    }
}
