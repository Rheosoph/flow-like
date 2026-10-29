use std::{
    any::Any,
    fmt,
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use super::{LLMCallback, ModelLogic, extract_headers};
use crate::provider::random_provider;
use crate::{
    authorization::AuthorizedHttpClient,
    egress::{EgressRefused, ProviderEgress},
    history::{History, HistoryThinking},
    llm::{
        CompletionModelHandle, ModelConstructor, emit_response_to_callback, invoke_with_stream,
        invoke_without_stream,
    },
    provider::{ModelProvider, ModelProviderConfiguration},
    response::Response,
};
use anyhow::Result;
use anyhow::anyhow;
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use flow_like_types_contracts::Cacheable;
use futures::{StreamExt, TryStreamExt, stream};
use reqwest::{StatusCode, Url, header};
use rig::completion::{CompletionModel, CompletionRequestBuilder, Message as RigMessage};
use rig::message::{DocumentSourceKind, MimeType, UserContent as RigUserContent};
use rig::providers::gemini::completion::gemini_api_types::{
    AdditionalParameters, GenerationConfig, ThinkingConfig, ThinkingLevel,
};
use serde_json::to_value;

/// The Gemini API's own host. It fetches eligible https links itself; for any other endpoint
/// every link is downloaded and sent inline.
const GEMINI_API_HOST: &str = "generativelanguage.googleapis.com";
/// The largest file the Gemini API fetches from a link.
const LINK_MAX_BYTES: u64 = 15_000_000;
const LINK_MAX_CHARS: usize = 8_192;
/// Gemini takes requests up to 100 MB. Inline media travel in them as base64; the rest of the
/// limit stays free for text, tools and JSON framing.
const REQUEST_MEDIA_BASE64_BYTES: u64 = 90_000_000;
/// The largest document Gemini reads inline.
const INLINE_DOCUMENT_BYTES: u64 = 50_000_000;
const CONCURRENT_DOWNLOADS: usize = 4;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Bounds the decoded first frame of a GIF that is turned into a PNG.
const GIF_FRAME_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// How base64 of a GIF (`GIF87a` or `GIF89a`) starts.
const GIF_BASE64_PREFIX: &str = "R0lGOD";

fn default_thinking_config() -> ThinkingConfig {
    ThinkingConfig {
        include_thoughts: Some(true),
        thinking_budget: Some(2048),
        thinking_level: None,
    }
}

fn is_gemini_3_model(model_name: Option<&str>) -> bool {
    model_name
        .map(|name| name.to_ascii_lowercase().contains("gemini-3"))
        .unwrap_or(false)
}

/// Gemini 2.5 Pro cannot turn thinking off; its smallest budget is 128.
const GEMINI_2_5_PRO_MIN_BUDGET: u32 = 128;

/// `MINIMAL` exists only on Gemini 3 Flash, 3.5/3.6 Flash and the Flash-Lite models; the
/// others reject it (3 Pro and 3.1 Pro cannot turn thinking off at all).
fn gemini_3_accepts_minimal(model: &str) -> bool {
    model.contains("flash-lite")
        || ["gemini-3-flash", "gemini-3.5-flash", "gemini-3.6-flash"]
            .iter()
            .any(|family| model.contains(family))
}

fn parse_base64_data_url(url: &str) -> Option<(&str, &str)> {
    let body = url.strip_prefix("data:")?;
    let comma_pos = body.find(',')?;
    let metadata = &body[..comma_pos];
    if !metadata
        .split(';')
        .any(|item| item.eq_ignore_ascii_case("base64"))
    {
        return None;
    }

    let mime_type = metadata.split(';').next()?.trim();
    if mime_type.is_empty() {
        return None;
    }

    Some((mime_type, &body[(comma_pos + 1)..]))
}

fn normalize_mime_type(mime_type: &str) -> String {
    let mime_type = mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_ascii_lowercase();

    match mime_type.as_str() {
        "image/jpg" => "image/jpeg",
        "audio/wave" | "audio/x-wav" => "audio/wav",
        "audio/mpeg" => "audio/mp3",
        "audio/x-aiff" => "audio/aiff",
        "audio/mp4" => "audio/m4a",
        "video/x-msvideo" => "video/avi",
        "video/quicktime" => "video/mov",
        "application/javascript" | "text/javascript" | "text/x-javascript" => {
            "application/x-javascript"
        }
        "text/python" | "text/x-python" => "application/x-python",
        "application/xml" => "text/xml",
        _ => return mime_type,
    }
    .to_string()
}

fn mime_type_from_query(url: &str) -> Option<String> {
    let query = url.split_once('?')?.1.split('#').next().unwrap_or_default();
    for part in query.split('&') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.eq_ignore_ascii_case("response-content-type")
            || key.eq_ignore_ascii_case("content-type")
        {
            return Some(
                value
                    .replace('+', " ")
                    .replace("%2F", "/")
                    .replace("%2f", "/")
                    .replace("%2B", "+")
                    .replace("%2b", "+")
                    .replace("%3B", ";")
                    .replace("%3b", ";"),
            );
        }
    }

    None
}

/// File extensions and the MIME types Rig knows them by.
const EXTENSION_MIME_TYPES: &[(&str, &str)] = &[
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("svg", "image/svg+xml"),
    ("svgz", "image/svg+xml"),
    ("pdf", "application/pdf"),
    ("txt", "text/plain"),
    ("rtf", "text/rtf"),
    ("html", "text/html"),
    ("htm", "text/html"),
    ("css", "text/css"),
    ("md", "text/markdown"),
    ("markdown", "text/markdown"),
    ("csv", "text/csv"),
    ("xml", "text/xml"),
    ("js", "application/x-javascript"),
    ("mjs", "application/x-javascript"),
    ("cjs", "application/x-javascript"),
    ("py", "application/x-python"),
    ("wav", "audio/wav"),
    ("mp3", "audio/mp3"),
    ("aiff", "audio/aiff"),
    ("aif", "audio/aiff"),
    ("aac", "audio/aac"),
    ("ogg", "audio/ogg"),
    ("oga", "audio/ogg"),
    ("flac", "audio/flac"),
    ("m4a", "audio/m4a"),
    ("avi", "video/avi"),
    ("mp4", "video/mp4"),
    ("m4v", "video/mp4"),
    ("mpeg", "video/mpeg"),
    ("mpg", "video/mpeg"),
    ("mov", "video/mov"),
    ("webm", "video/webm"),
];

fn mime_type_from_extension(url: &str) -> Option<&'static str> {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim()
        .to_ascii_lowercase();
    let (_, extension) = path.rsplit_once('.')?;
    EXTENSION_MIME_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|(_, mime_type)| *mime_type)
}

fn mime_type_from_url(url: &str) -> Option<String> {
    if let Some((mime_type, _)) = parse_base64_data_url(url) {
        return Some(normalize_mime_type(mime_type));
    }

    if let Some(mime_type) = mime_type_from_query(url) {
        return Some(normalize_mime_type(&mime_type));
    }

    mime_type_from_extension(url).map(str::to_string)
}

/// The image type its leading bytes give away, for a link typed by neither name nor server.
fn sniffed_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaKind {
    Image,
    Audio,
    Video,
    Document,
}

impl MediaKind {
    fn label(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio file",
            Self::Video => "video",
            Self::Document => "document",
        }
    }

    /// Whether the Gemini API fetches a link of this kind and type itself. It does not for
    /// audio, and HEIC/HEIF links are unconfirmed.
    fn fetched_by_gemini(self, mime: &str) -> bool {
        match self {
            Self::Image => matches!(mime, "image/jpeg" | "image/png" | "image/webp"),
            Self::Document => mime == "application/pdf",
            Self::Video => true,
            Self::Audio => false,
        }
    }

    /// The most one part of this kind may download, before the request's own budget applies.
    fn inline_cap(self) -> u64 {
        match self {
            Self::Document => INLINE_DOCUMENT_BYTES,
            Self::Image | Self::Audio | Self::Video => u64::MAX,
        }
    }
}

/// The declared media type of a part, whatever its kind.
trait MediaTypeSlot: Send {
    fn mime(&self) -> Option<&'static str>;
    /// Takes `mime` when it names a type of this kind, and keeps the current type otherwise.
    fn assign(&mut self, mime: &str);
}

impl<T: MimeType + Send> MediaTypeSlot for Option<T> {
    fn mime(&self) -> Option<&'static str> {
        self.as_ref().map(MimeType::to_mime_type)
    }

    fn assign(&mut self, mime: &str) {
        if let Some(media_type) = T::from_mime_type(&normalize_mime_type(mime)) {
            *self = Some(media_type);
        }
    }
}

fn media_fields(
    part: &mut RigUserContent,
) -> Option<(MediaKind, &mut DocumentSourceKind, &mut dyn MediaTypeSlot)> {
    match part {
        RigUserContent::Image(image) => {
            Some((MediaKind::Image, &mut image.data, &mut image.media_type))
        }
        RigUserContent::Audio(audio) => {
            Some((MediaKind::Audio, &mut audio.data, &mut audio.media_type))
        }
        RigUserContent::Video(video) => {
            Some((MediaKind::Video, &mut video.data, &mut video.media_type))
        }
        RigUserContent::Document(document) => Some((
            MediaKind::Document,
            &mut document.data,
            &mut document.media_type,
        )),
        RigUserContent::Text(_) | RigUserContent::ToolResult(_) => None,
    }
}

/// Puts an inline payload in the form Rig sends and types a link from its URL. Images take the
/// URL's type over the declared one, which is often a default. Returns the link of a part that
/// still has to be resolved.
fn normalize_part(part: &mut RigUserContent) -> Option<String> {
    let (kind, data, media_type) = media_fields(part)?;
    match data {
        DocumentSourceKind::Url(link) => {
            if let Some((mime_type, payload)) = parse_base64_data_url(link) {
                media_type.assign(mime_type);
                *data = DocumentSourceKind::Base64(payload.to_string());
                return None;
            }
            if let Some(mime_type) = mime_type_from_url(link)
                && (kind == MediaKind::Image || media_type.mime().is_none())
            {
                media_type.assign(&mime_type);
            }
            Some(link.clone())
        }
        DocumentSourceKind::Raw(bytes) => {
            *data = DocumentSourceKind::Base64(BASE64.encode(bytes));
            None
        }
        _ => None,
    }
}

/// Whether a client built for `endpoint` calls the Gemini API itself (Rig's default when none
/// is set). Custom endpoints may not fetch links, so they get every link inline.
fn calls_gemini_api(endpoint: Option<&str>) -> bool {
    endpoint.is_none_or(|endpoint| {
        Url::parse(endpoint.trim())
            .is_ok_and(|url| url.scheme() == "https" && url.host_str() == Some(GEMINI_API_HOST))
    })
}

/// Gemini 2.0 and older models do not fetch links.
fn model_fetches_links(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    !(model.contains("gemini-1.") || model.contains("gemini-2.0"))
}

/// Links Gemini resolves on its own side, which cannot be downloaded here: Cloud Storage
/// objects, files uploaded through the Files API, and YouTube videos.
fn is_gemini_reference(kind: MediaKind, url: &Url) -> bool {
    match (url.scheme(), url.host_str()) {
        ("gs", _) => true,
        ("https", Some(GEMINI_API_HOST)) => url.path_segments().is_some_and(|segments| {
            matches!(
                segments.collect::<Vec<_>>().as_slice(),
                [version, "files", name] if version.starts_with('v') && !name.is_empty()
            )
        }),
        ("https", Some("www.youtube.com" | "youtube.com" | "m.youtube.com" | "youtu.be")) => {
            kind == MediaKind::Video
        }
        _ => false,
    }
}

/// Whether `link` is one Gemini can fetch exactly as given: in canonical form, from a plain
/// public origin, and safe from re-encoding.
fn link_passable(link: &str, url: &Url) -> bool {
    url.as_str() == link && plain_public_origin(url) && fetch_safe(link, url)
}

/// https on port 443 with a public DNS name, without credentials or a fragment.
fn plain_public_origin(url: &Url) -> bool {
    ProviderEgress::Public.check_url(url).is_ok()
        && url.port().is_none()
        && url.domain().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

/// Within the length limit, and free of the encoded separators and raw characters that some
/// fetchers re-encode, which breaks signatures.
fn fetch_safe(link: &str, url: &Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    link.len() <= LINK_MAX_CHARS
        && !link.contains(['@', ' ', '+'])
        && !path.contains("%2f")
        && !path.contains("%5c")
}

/// Whether a part is worth downloading: Rig refuses SVG images, and GIFs this build cannot
/// convert, whatever their source, so those keep failing the call as before.
fn worth_downloading(kind: MediaKind, mime: Option<&str>, converts_gif: bool) -> bool {
    match (kind, mime) {
        (MediaKind::Image, Some("image/gif")) => converts_gif,
        (MediaKind::Image, Some("image/svg+xml")) => false,
        _ => true,
    }
}

/// The complete length in a `Content-Range: bytes 0-0/<total>` header, when the server states it.
fn content_range_total(value: &str) -> Option<u64> {
    let (unit, range) = value.trim().split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    range.rsplit_once('/')?.1.trim().parse().ok()
}

fn header_text(headers: &header::HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name)?.to_str().ok()
}

/// What a ranged GET reports about a link.
#[derive(Debug)]
struct LinkProbe {
    /// The complete size, when the server states it.
    size: Option<u64>,
    content_type: Option<String>,
}

impl LinkProbe {
    /// Whether Gemini may fetch the link: a known size within its limit, served as the type the
    /// part declares.
    fn fits(&self, mime: &str) -> bool {
        self.size.is_some_and(|size| size <= LINK_MAX_BYTES)
            && self.content_type.as_deref() == Some(mime)
    }
}

/// Bytes one call may still download for inline media, shared by its concurrent downloads.
struct RequestBudget {
    cap: u64,
    used: AtomicU64,
}

impl RequestBudget {
    fn new(cap: u64) -> Self {
        Self {
            cap,
            used: AtomicU64::new(0),
        }
    }

    /// What Gemini's request limit leaves for downloads once the media already inline (as
    /// base64) are counted.
    fn after_inline(inline_base64_bytes: u64) -> Self {
        Self::new(REQUEST_MEDIA_BASE64_BYTES.saturating_sub(inline_base64_bytes) / 4 * 3)
    }

    fn admits(&self, bytes: u64) -> bool {
        self.used.load(Ordering::Relaxed).saturating_add(bytes) <= self.cap
    }

    fn take(&self, bytes: u64) -> Result<(), CapExceeded> {
        let used = self
            .used
            .fetch_add(bytes, Ordering::Relaxed)
            .saturating_add(bytes);
        if used > self.cap {
            return Err(CapExceeded::Request);
        }
        Ok(())
    }
}

/// One part's download, counted against its own cap and the call's budget as it streams in.
struct PartBudget<'a> {
    request: &'a RequestBudget,
    cap: u64,
    used: u64,
}

impl<'a> PartBudget<'a> {
    fn new(request: &'a RequestBudget, cap: u64) -> Self {
        Self {
            request,
            cap,
            used: 0,
        }
    }

    /// Refuses a body whose announced length cannot fit, before any of it is read.
    fn admits(&self, length: u64) -> Result<(), CapExceeded> {
        if self.used.saturating_add(length) > self.cap {
            return Err(CapExceeded::Part(self.cap));
        }
        if !self.request.admits(length) {
            return Err(CapExceeded::Request);
        }
        Ok(())
    }

    fn take(&mut self, bytes: u64) -> Result<(), CapExceeded> {
        self.used = self.used.saturating_add(bytes);
        if self.used > self.cap {
            return Err(CapExceeded::Part(self.cap));
        }
        self.request.take(bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapExceeded {
    /// More than Gemini reads inline for one part of its kind.
    Part(u64),
    /// More than the rest of the request can carry.
    Request,
}

impl fmt::Display for CapExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Part(cap) => write!(
                formatter,
                "it is larger than the {} MB Gemini reads inline",
                *cap as f64 / 1e6
            ),
            Self::Request => formatter.write_str(
                "together with the call's other media it is larger than one Gemini request carries",
            ),
        }
    }
}

impl std::error::Error for CapExceeded {}

/// Why a media download failed, worded without the link: signed links are credentials.
#[derive(Debug)]
enum FetchFailure {
    Refused,
    Status(StatusCode),
    TooLarge(CapExceeded),
    TimedOut,
    Failed(String),
}

impl From<CapExceeded> for FetchFailure {
    fn from(cap: CapExceeded) -> Self {
        Self::TooLarge(cap)
    }
}

impl From<reqwest::Error> for FetchFailure {
    fn from(error: reqwest::Error) -> Self {
        let root: &(dyn std::error::Error + 'static) = &error;
        let causes = || std::iter::successors(Some(root), |cause| cause.source());
        if causes().any(|cause| cause.is::<EgressRefused>()) {
            return Self::Refused;
        }
        if error.is_timeout() {
            return Self::TimedOut;
        }
        let cause = causes()
            .skip(1)
            .last()
            .map(|cause| format!(": {cause}"))
            .unwrap_or_default();
        Self::Failed(format!("{}{cause}", error.without_url()))
    }
}

impl fmt::Display for FetchFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused => formatter.write_str("only public https links can be downloaded"),
            Self::Status(status) => write!(formatter, "the download answered HTTP {status}"),
            Self::TooLarge(cap) => cap.fmt(formatter),
            Self::TimedOut => formatter.write_str("the download timed out"),
            Self::Failed(detail) => write!(formatter, "the download failed: {detail}"),
        }
    }
}

/// The call error for a link that cannot be sent. It names the kind and host, never the link.
fn link_error(kind: MediaKind, host: &str, reason: impl fmt::Display) -> anyhow::Error {
    if host.is_empty() {
        anyhow!(
            "The {} link could not be sent to Gemini: {reason}",
            kind.label()
        )
    } else {
        anyhow!(
            "The {} at host '{host}' could not be sent to Gemini: {reason}",
            kind.label()
        )
    }
}

#[derive(Debug)]
struct Downloaded {
    bytes: Vec<u8>,
    content_type: Option<String>,
}

/// Probes and downloads media links under an egress policy, without following redirects.
struct MediaFetcher {
    egress: ProviderEgress,
    client: reqwest::Client,
}

impl MediaFetcher {
    fn new(egress: ProviderEgress, builder: reqwest::ClientBuilder) -> Result<Self> {
        let client = egress
            .guard(builder.connect_timeout(CONNECT_TIMEOUT))
            .build()
            .map_err(|error| anyhow!("Media downloads for Gemini could not start: {error}"))?;
        Ok(Self { egress, client })
    }

    /// The size and type a GET of the first byte reports (a HEAD is not covered by a signed GET
    /// link), or `None` when it does not answer 200 or 206 in time.
    async fn probe(&self, url: Url) -> Option<LinkProbe> {
        let response = self
            .client
            .get(url)
            .header(header::RANGE, "bytes=0-0")
            .header(header::ACCEPT_ENCODING, "identity")
            .timeout(PROBE_TIMEOUT)
            .send()
            .await
            .ok()?;
        let headers = response.headers();
        let encoded = header_text(headers, header::CONTENT_ENCODING)
            .is_some_and(|coding| !coding.trim().eq_ignore_ascii_case("identity"));
        let size = match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                header_text(headers, header::CONTENT_RANGE).and_then(content_range_total)
            }
            StatusCode::OK => header_text(headers, header::CONTENT_LENGTH)
                .and_then(|length| length.trim().parse().ok()),
            _ => return None,
        };
        Some(LinkProbe {
            size: size.filter(|_| !encoded),
            content_type: header_text(headers, header::CONTENT_TYPE).map(normalize_mime_type),
        })
    }

    async fn download(
        &self,
        url: Url,
        budget: &mut PartBudget<'_>,
    ) -> Result<Downloaded, FetchFailure> {
        self.egress
            .check_url(&url)
            .map_err(|_| FetchFailure::Refused)?;
        let response = self
            .client
            .get(url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchFailure::Status(status));
        }
        let content_type =
            header_text(response.headers(), header::CONTENT_TYPE).map(normalize_mime_type);
        Ok(Downloaded {
            bytes: read_capped(response, budget).await?,
            content_type,
        })
    }
}

/// Streams a body into memory and stops at the first byte over `budget`. A body whose announced
/// length cannot fit is refused before any of it is read.
async fn read_capped(
    mut response: reqwest::Response,
    budget: &mut PartBudget<'_>,
) -> Result<Vec<u8>, FetchFailure> {
    if let Some(length) = response.content_length() {
        budget.admits(length)?;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        budget.take(chunk.len() as u64)?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn converts_gif() -> bool {
    image::ImageFormat::Gif.reading_enabled() && image::ImageFormat::Png.writing_enabled()
}

fn first_gif_frame_as_png(gif: &[u8]) -> image::ImageResult<Vec<u8>> {
    let mut reader = image::ImageReader::with_format(Cursor::new(gif), image::ImageFormat::Gif);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(GIF_FRAME_MAX_BYTES);
    reader.limits(limits);
    let frame = reader.decode()?;
    let mut png = Vec::new();
    frame.write_to(Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(png)
}

/// Gemini cannot read GIFs, so it gets the first frame as a PNG, made on a blocking thread.
/// Growth over the GIF counts against the request's budget.
async fn png_for_gif(gif: Vec<u8>, budget: &RequestBudget) -> Result<Vec<u8>> {
    let gif_len = gif.len() as u64;
    let png = tokio::task::spawn_blocking(move || first_gif_frame_as_png(&gif))
        .await?
        .map_err(|error| {
            anyhow!(
                "it is a GIF, which Gemini cannot read, and converting it to PNG failed: {error}"
            )
        })?;
    budget.take((png.len() as u64).saturating_sub(gif_len))?;
    Ok(png)
}

/// What the parts of one call share while their media are resolved.
struct MediaContext {
    /// Whether the call goes to the Gemini API with a model that fetches links itself.
    passes_links: bool,
    converts_gif: bool,
    budget: RequestBudget,
    fetcher: Option<MediaFetcher>,
}

/// Leaves a link Gemini fetches itself, or downloads the part and sends it inline.
async fn resolve_link(part: &mut RigUserContent, link: &str, context: &MediaContext) -> Result<()> {
    let Some((kind, _, media_type)) = media_fields(part) else {
        return Ok(());
    };
    let mime = media_type.mime();
    let url = Url::parse(link).map_err(|_| link_error(kind, "", "it is not a valid URL"))?;
    if is_gemini_reference(kind, &url) || !worth_downloading(kind, mime, context.converts_gif) {
        return Ok(());
    }
    let host = url.host_str().unwrap_or_default();
    let Some(fetcher) = &context.fetcher else {
        return Err(link_error(kind, host, "media downloads are not set up"));
    };
    if context.passes_links && gemini_fetches(fetcher, link, &url, kind, mime).await {
        tracing::debug!(
            host,
            kind = kind.label(),
            "Leaving a media link for Gemini to fetch"
        );
        return Ok(());
    }
    let mut budget = PartBudget::new(&context.budget, kind.inline_cap());
    let downloaded = fetcher
        .download(url.clone(), &mut budget)
        .await
        .map_err(|failure| link_error(kind, host, failure))?;
    inline_download(part, downloaded, context)
        .await
        .map_err(|error| link_error(kind, host, error))
}

/// Whether Gemini can fetch the link itself: a listed kind and type, a passable link whose
/// every address is public, and a probe that finds it small enough and served as declared.
async fn gemini_fetches(
    fetcher: &MediaFetcher,
    link: &str,
    url: &Url,
    kind: MediaKind,
    mime: Option<&str>,
) -> bool {
    let Some(mime) = mime.filter(|mime| kind.fetched_by_gemini(mime)) else {
        return false;
    };
    link_passable(link, url)
        && ProviderEgress::Public
            .ensure_endpoint_allowed(link)
            .await
            .is_ok()
        && fetcher
            .probe(url.clone())
            .await
            .is_some_and(|probe| probe.fits(mime))
}

/// Types an untyped download by the server's answer, or an image by its leading bytes.
fn type_download(
    kind: MediaKind,
    media_type: &mut dyn MediaTypeSlot,
    content_type: Option<&str>,
    bytes: &[u8],
) {
    if let Some(content_type) = content_type.filter(|_| media_type.mime().is_none()) {
        media_type.assign(content_type);
    }
    if kind == MediaKind::Image
        && media_type.mime().is_none()
        && let Some(mime) = sniffed_image_mime(bytes)
    {
        media_type.assign(mime);
    }
}

/// Sends a download inline, typed if it was not, and a GIF image as a PNG.
async fn inline_download(
    part: &mut RigUserContent,
    downloaded: Downloaded,
    context: &MediaContext,
) -> Result<()> {
    let Some((kind, data, media_type)) = media_fields(part) else {
        return Ok(());
    };
    let Downloaded {
        mut bytes,
        content_type,
    } = downloaded;
    type_download(kind, media_type, content_type.as_deref(), &bytes);
    let gif = media_type.mime() == Some("image/gif") || bytes.starts_with(b"GIF8");
    if kind == MediaKind::Image && context.converts_gif && gif {
        bytes = png_for_gif(bytes, &context.budget).await?;
        media_type.assign("image/png");
    }
    *data = DocumentSourceKind::Base64(BASE64.encode(&bytes));
    Ok(())
}

/// Turns an inline GIF image, which Gemini cannot read, into a PNG.
async fn convert_inline_gif(part: &mut RigUserContent, context: &MediaContext) -> Result<()> {
    let Some((MediaKind::Image, DocumentSourceKind::Base64(data), media_type)) = media_fields(part)
    else {
        return Ok(());
    };
    let gif = media_type.mime() == Some("image/gif") || data.starts_with(GIF_BASE64_PREFIX);
    if !(context.converts_gif && gif) {
        return Ok(());
    }
    let bytes = BASE64.decode(data.as_bytes()).map_err(|error| {
        anyhow!("An inline image could not be sent to Gemini: it is not valid base64: {error}")
    })?;
    let png = png_for_gif(bytes, &context.budget)
        .await
        .map_err(|error| anyhow!("An inline image could not be sent to Gemini: {error}"))?;
    *data = BASE64.encode(&png);
    media_type.assign("image/png");
    Ok(())
}

async fn resolve_part(
    part: &mut RigUserContent,
    link: Option<String>,
    context: &MediaContext,
) -> Result<()> {
    match link {
        Some(link) => resolve_link(part, &link, context).await,
        None => convert_inline_gif(part, context).await,
    }
}

fn user_parts<'a>(
    prompt: &'a mut RigMessage,
    history: &'a mut [RigMessage],
) -> impl Iterator<Item = &'a mut RigUserContent> {
    std::iter::once(prompt)
        .chain(history.iter_mut())
        .filter_map(|message| match message {
            RigMessage::User { content } => Some(content.iter_mut()),
            _ => None,
        })
        .flatten()
}

/// Readies a call's media for Gemini. Inline payloads take the form Rig sends; links Gemini
/// fetches itself stay links; every other link is downloaded (several at once, within the
/// request's limits) and sent inline; GIFs become PNGs. A part that can be neither linked nor
/// inlined fails the call with an error naming its kind and host.
async fn prepare_media(
    prompt: &mut RigMessage,
    history: &mut [RigMessage],
    passes_links: bool,
    egress: ProviderEgress,
    builder: reqwest::ClientBuilder,
) -> Result<()> {
    let mut parts = Vec::new();
    let mut inline_base64_bytes = 0u64;
    for part in user_parts(prompt, history) {
        let link = normalize_part(part);
        if let Some((_, DocumentSourceKind::Base64(data), _)) = media_fields(part) {
            inline_base64_bytes = inline_base64_bytes.saturating_add(data.len() as u64);
        }
        parts.push((part, link));
    }
    let fetcher = if parts.iter().any(|(_, link)| link.is_some()) {
        Some(MediaFetcher::new(egress, builder)?)
    } else {
        None
    };
    let context = MediaContext {
        passes_links,
        converts_gif: converts_gif(),
        budget: RequestBudget::after_inline(inline_base64_bytes),
        fetcher,
    };
    let pending: Vec<_> = parts
        .into_iter()
        .map(|(part, link)| resolve_part(part, link, &context))
        .collect();
    stream::iter(pending)
        .buffer_unordered(CONCURRENT_DOWNLOADS)
        .try_collect()
        .await
}

fn thinking_config_for_history(
    history: Option<&History>,
    model_name: Option<&str>,
) -> ThinkingConfig {
    let Some(history) = history else {
        return default_thinking_config();
    };

    let Some(thinking) = history.thinking else {
        return default_thinking_config();
    };

    let include_thoughts = Some(thinking != HistoryThinking::Off);

    let model = model_name.unwrap_or_default().to_ascii_lowercase();
    if is_gemini_3_model(model_name) {
        let thinking_level = match thinking {
            _ if model.contains("pro-image") => ThinkingLevel::High,
            HistoryThinking::Off if gemini_3_accepts_minimal(&model) => ThinkingLevel::Minimal,
            HistoryThinking::Off | HistoryThinking::Low => ThinkingLevel::Low,
            HistoryThinking::Mid => ThinkingLevel::Medium,
            HistoryThinking::High => ThinkingLevel::High,
        };

        ThinkingConfig {
            include_thoughts,
            thinking_budget: None,
            thinking_level: Some(thinking_level),
        }
    } else {
        let thinking_budget = match thinking {
            HistoryThinking::Off if model.contains("2.5-pro") => GEMINI_2_5_PRO_MIN_BUDGET,
            HistoryThinking::Off => 0,
            HistoryThinking::Low => 1024,
            HistoryThinking::Mid => 2048,
            HistoryThinking::High => 4096,
        };

        ThinkingConfig {
            include_thoughts,
            thinking_budget: Some(thinking_budget),
            thinking_level: None,
        }
    }
}

pub struct GeminiModel {
    client: rig::providers::gemini::Client<AuthorizedHttpClient>,
    _provider: ModelProvider,
    default_model: Option<String>,
    /// Whether the client calls the Gemini API itself rather than a custom endpoint.
    calls_gemini_api: bool,
}

impl GeminiModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let gemini_config = random_provider(&config.gemini_config)?;
        let api_key = gemini_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();
        let endpoint = gemini_config.endpoint.as_deref();

        let mut builder = rig::providers::gemini::Client::builder()
            .api_key(&api_key)
            .http_client(AuthorizedHttpClient::default());
        if let Some(endpoint) = endpoint {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(GeminiModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
            calls_gemini_api: calls_gemini_api(endpoint),
        })
    }

    pub async fn from_provider(
        provider: &ModelProvider,
        egress: ProviderEgress,
    ) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));
        let custom_headers = extract_headers(&params);
        let endpoint = params.get("endpoint").and_then(|v| v.as_str());

        let mut builder = rig::providers::gemini::Client::builder()
            .api_key(api_key)
            .http_client(AuthorizedHttpClient::for_egress(egress)?);
        if let Some(endpoint) = endpoint {
            builder = builder.base_url(endpoint);
        }
        if !custom_headers.is_empty() {
            builder = builder.http_headers(custom_headers);
        }
        let client = builder.build()?;

        Ok(GeminiModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
            calls_gemini_api: calls_gemini_api(endpoint),
        })
    }

    /// The call's prompt and history as Rig messages, their media ready for Gemini. Since
    /// 2026-01-08 the Gemini API fetches public and presigned https links itself (up to 15 MB
    /// per file), so eligible links stay links; other media is downloaded and inlined.
    async fn rig_messages(
        &self,
        history: &History,
        model: &str,
    ) -> Result<(RigMessage, Vec<RigMessage>)> {
        let (mut prompt, mut chat_history) = history
            .extract_prompt_and_history()
            .map_err(|e| anyhow!("Failed to convert history into rig messages: {e}"))?;
        let passes_links = self.calls_gemini_api && model_fetches_links(model);
        let builder = reqwest::Client::builder();
        prepare_media(
            &mut prompt,
            &mut chat_history,
            passes_links,
            ProviderEgress::Public,
            builder,
        )
        .await?;
        Ok((prompt, chat_history))
    }
}

/// Applies the history's sampling settings, tools and tool choice to a fresh request, and its
/// extra parameters unless the model sends its own.
fn configure_request<M: CompletionModel>(
    builder: CompletionRequestBuilder<M>,
    history: &History,
    model_params: Option<&serde_json::Value>,
) -> Result<CompletionRequestBuilder<M>> {
    let tools = if history.tools.is_some() {
        history.tools_to_rig()?
    } else {
        Vec::new()
    };
    let history_params = if model_params.is_none() {
        history.build_additional_params()?
    } else {
        None
    };
    let builder = builder
        .temperature_opt(history.temperature.map(f64::from))
        .max_tokens_opt(history.max_completion_tokens.map(u64::from))
        .tools(tools)
        .additional_params_opt(history_params);
    Ok(match history.tool_choice_to_rig() {
        Some(choice) => builder.tool_choice(choice),
        None => builder,
    })
}

/// Sends the request, streaming it to `lambda` when the call streams and otherwise handing it
/// the whole response.
async fn send<'a>(
    builder: CompletionRequestBuilder<CompletionModelHandle<'a>>,
    lambda: Option<LLMCallback>,
    should_stream: bool,
    model_name: &str,
    params: Option<serde_json::Value>,
) -> Result<Response> {
    match lambda {
        Some(callback) if should_stream => {
            invoke_with_stream(builder, callback, model_name, params).await
        }
        lambda => {
            let response = invoke_without_stream(builder, model_name, params).await?;
            if let Some(callback) = lambda {
                emit_response_to_callback(&response, callback, model_name).await?;
            }
            Ok(response)
        }
    }
}

impl Cacheable for GeminiModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for GeminiModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone()),
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn transform_history(&self, _history: &mut History) {
        // Not used - we override invoke() to transform RigMessages instead
    }

    #[allow(deprecated)]
    async fn invoke(&self, history: &History, lambda: Option<LLMCallback>) -> Result<Response> {
        let mut history = history.clone();
        let should_stream = lambda.is_some() && history.stream.unwrap_or(true);
        history.stream = Some(should_stream);

        let model_name = self
            .default_model()
            .await
            .unwrap_or_else(|| history.model.clone());

        let constructor = self.provider().await?;
        let completion_model = constructor.inner.completion_model(&model_name);
        let completion_handle = CompletionModelHandle::new(Arc::from(completion_model));

        let (prompt, chat_history) = self.rig_messages(&history, &model_name).await?;
        let model_additional_params = self.additional_params(&Some(history.clone()));
        let builder = configure_request(
            completion_handle
                .completion_request(prompt)
                .messages(chat_history),
            &history,
            model_additional_params.as_ref(),
        )?;
        send(
            builder,
            lambda,
            should_stream,
            &model_name,
            model_additional_params,
        )
        .await
    }

    fn additional_params(&self, history: &Option<History>) -> Option<serde_json::Value> {
        // Gemini's AdditionalParameters MUST include generation_config field
        // We need to handle the 'stream' field specially: it comes from History.build_additional_params()
        // but Gemini doesn't accept 'stream' in the request body - it uses different endpoints instead

        // Get history's additional params (includes stream field)
        let history_params = history
            .as_ref()
            .and_then(|h| h.build_additional_params().ok())
            .flatten();

        let model_name = self
            .default_model
            .as_deref()
            .or_else(|| history.as_ref().map(|item| item.model.as_str()));

        let gen_cfg = GenerationConfig {
            thinking_config: Some(thinking_config_for_history(history.as_ref(), model_name)),
            ..Default::default()
        };
        let additional_params = AdditionalParameters::default().with_config(gen_cfg);
        let mut result = to_value(additional_params).ok()?;

        // Merge history params but exclude 'stream' field
        if let (Some(result_obj), Some(history_params)) = (result.as_object_mut(), history_params)
            && let Some(history_obj) = history_params.as_object()
        {
            for (key, value) in history_obj {
                // Skip 'stream' field - Gemini doesn't support it
                if key != "stream" {
                    result_obj.insert(key.clone(), value.clone());
                }
            }
        }

        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::tests::answer_every_connection;
    use rig::OneOrMany;
    use rig::message::{
        Audio as RigAudio, AudioMediaType, Document as RigDocument, DocumentMediaType,
        Image as RigImage, ImageMediaType, Video as RigVideo, VideoMediaType,
    };
    use std::net::SocketAddr;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const SIGNED_JPEG: &str = "https://example-bucket.s3.amazonaws.com/path/photo.jpg?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Signature=abc";
    const GIF_1X1: &str = "R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";
    const FORBIDDEN: &str =
        "HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

    fn link(url: &str) -> DocumentSourceKind {
        DocumentSourceKind::Url(url.to_string())
    }

    fn image(data: DocumentSourceKind, media_type: Option<ImageMediaType>) -> RigUserContent {
        RigUserContent::Image(RigImage {
            data,
            media_type,
            detail: None,
            additional_params: None,
        })
    }

    fn document(data: DocumentSourceKind, media_type: Option<DocumentMediaType>) -> RigUserContent {
        RigUserContent::Document(RigDocument {
            data,
            media_type,
            additional_params: None,
        })
    }

    fn audio(data: DocumentSourceKind, media_type: Option<AudioMediaType>) -> RigUserContent {
        RigUserContent::Audio(RigAudio {
            data,
            media_type,
            additional_params: None,
        })
    }

    fn video(url: &str, media_type: Option<VideoMediaType>) -> RigUserContent {
        RigUserContent::Video(RigVideo {
            data: link(url),
            media_type,
            additional_params: None,
        })
    }

    fn prompt(parts: Vec<RigUserContent>) -> RigMessage {
        RigMessage::User {
            content: OneOrMany::many(parts).unwrap(),
        }
    }

    fn parts(message: &RigMessage) -> Vec<RigUserContent> {
        let RigMessage::User { content } = message else {
            panic!("expected a user message");
        };
        content.iter().cloned().collect()
    }

    fn local_fetcher(addr: SocketAddr) -> MediaFetcher {
        let builder = reqwest::Client::builder().resolve("media.test", addr);
        MediaFetcher::new(ProviderEgress::Private, builder).unwrap()
    }

    fn local_url(addr: SocketAddr, path: &str) -> Url {
        Url::parse(&format!("http://media.test:{}{path}", addr.port())).unwrap()
    }

    /// Prepares `parts` as one prompt whose `media.test` links reach `addr`.
    async fn prepare_local(parts: Vec<RigUserContent>, addr: SocketAddr) -> Result<RigMessage> {
        let mut message = prompt(parts);
        let builder = reqwest::Client::builder().resolve("media.test", addr);
        prepare_media(
            &mut message,
            &mut [],
            false,
            ProviderEgress::Private,
            builder,
        )
        .await?;
        Ok(message)
    }

    /// Answers every connection after a pause and reports the most requests it held at once.
    async fn answer_slowly(response: &'static str) -> (SocketAddr, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let held = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let reported = peak.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (held, peak) = (held.clone(), peak.clone());
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    let _ = socket.read(&mut buffer).await;
                    peak.fetch_max(held.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    held.fetch_sub(1, Ordering::SeqCst);
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (addr, reported)
    }

    #[test]
    fn detects_image_mime_from_signed_url_path() {
        let url = "https://example-bucket.s3.amazonaws.com/path/photo.JPEG?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Signature=abc";

        assert_eq!(mime_type_from_url(url).as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn detects_image_mime_from_response_content_type_query() {
        let url = "https://example-bucket.s3.amazonaws.com/object?response-content-type=image%2Fwebp&X-Amz-Signature=abc";

        assert_eq!(mime_type_from_url(url).as_deref(), Some("image/webp"));
    }

    #[test]
    fn extension_types_come_from_the_path_alone() {
        for (url, expected) in [
            (
                "https://cdn.example.com/clip.MOV?sig=a.pdf",
                Some("video/mov"),
            ),
            (
                "https://cdn.example.com/notes.markdown#top",
                Some("text/markdown"),
            ),
            ("https://cdn.example.com/archive.tar.gz", None),
            ("https://cdn.example.com/v1.2/report", None),
        ] {
            assert_eq!(mime_type_from_extension(url), expected, "{url}");
        }
    }

    #[test]
    fn links_are_typed_by_their_url_and_inline_payloads_become_base64() {
        let mut part = image(link(SIGNED_JPEG), Some(ImageMediaType::PNG));
        assert_eq!(normalize_part(&mut part).as_deref(), Some(SIGNED_JPEG));
        assert_eq!(
            part,
            image(link(SIGNED_JPEG), Some(ImageMediaType::JPEG)),
            "the URL's type overrides the history default"
        );

        let mut part = image(link("data:image/jpg;base64,/9j/"), None);
        assert_eq!(normalize_part(&mut part), None);
        assert_eq!(
            part,
            image(
                DocumentSourceKind::Base64("/9j/".to_string()),
                Some(ImageMediaType::JPEG)
            )
        );

        let pdf = Some(DocumentMediaType::PDF);
        let mut part = document(DocumentSourceKind::Raw(b"%PDF-1.7".to_vec()), pdf.clone());
        assert_eq!(normalize_part(&mut part), None);
        assert_eq!(
            part,
            document(DocumentSourceKind::Base64(BASE64.encode(b"%PDF-1.7")), pdf)
        );
    }

    #[test]
    fn untyped_images_are_typed_by_their_leading_bytes() {
        assert_eq!(
            sniffed_image_mime(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg")
        );
        assert_eq!(sniffed_image_mime(b"\x89PNG\r\n\x1a\n"), Some("image/png"));
        assert_eq!(sniffed_image_mime(b"GIF87a"), Some("image/gif"));
        assert_eq!(
            sniffed_image_mime(b"RIFF\0\0\0\0WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(sniffed_image_mime(b"RIFF\0\0\0\0WAVEfmt "), None);
        assert_eq!(sniffed_image_mime(b"<svg"), None);
        for gif in [&b"GIF87a"[..], b"GIF89a"] {
            assert!(BASE64.encode(gif).starts_with(GIF_BASE64_PREFIX));
        }
    }

    #[test]
    fn only_plain_public_https_links_are_left_for_gemini() {
        let passable = |link: &str| Url::parse(link).is_ok_and(|url| link_passable(link, &url));
        assert!(passable(SIGNED_JPEG));
        assert!(passable("https://cdn.example.com/clips/launch.mp4"));
        for link in [
            "http://cdn.example.com/photo.jpg",
            "https://cdn.example.com:8443/photo.jpg",
            "https://93.184.216.34/photo.jpg",
            "https://[2606:4700:4700::1111]/photo.jpg",
            "https://user:secret@cdn.example.com/photo.jpg",
            "https://cdn.example.com/photo.jpg#preview",
            "https://cdn.example.com/a%2Fb/photo.jpg",
            "https://cdn.example.com/a%5cb/photo.jpg",
            "https://cdn.example.com/photo.jpg?sig=a+b",
            "https://cdn.example.com/photo@2x.jpg",
            "https://cdn.example.com/my photo.jpg",
            "https://CDN.example.com/photo.jpg",
            "https://localhost/photo.jpg",
            "https://metadata.google.internal/photo.jpg",
        ] {
            assert!(!passable(link), "{link}");
        }
        let long = format!(
            "https://cdn.example.com/photo.jpg?sig={}",
            "a".repeat(LINK_MAX_CHARS)
        );
        assert!(!passable(&long));
    }

    #[test]
    fn only_the_gemini_api_fetches_links_and_not_for_older_models() {
        assert!(calls_gemini_api(None));
        for endpoint in [
            "https://generativelanguage.googleapis.com",
            " https://generativelanguage.googleapis.com/v1beta/ ",
        ] {
            assert!(calls_gemini_api(Some(endpoint)), "{endpoint}");
        }
        for endpoint in [
            "https://gemini-gateway.example.com/v1beta",
            "http://generativelanguage.googleapis.com",
            "",
        ] {
            assert!(!calls_gemini_api(Some(endpoint)), "{endpoint}");
        }
        assert!(model_fetches_links("gemini-2.5-flash"));
        assert!(model_fetches_links("gemini-3-pro-preview"));
        assert!(!model_fetches_links("gemini-2.0-flash-001"));
        assert!(!model_fetches_links("models/Gemini-1.5-pro"));
    }

    #[test]
    fn gemini_fetches_links_only_of_listed_kinds() {
        for (kind, mime) in [
            (MediaKind::Image, "image/jpeg"),
            (MediaKind::Image, "image/png"),
            (MediaKind::Image, "image/webp"),
            (MediaKind::Document, "application/pdf"),
            (MediaKind::Video, "video/mp4"),
            (MediaKind::Video, "video/mov"),
        ] {
            assert!(kind.fetched_by_gemini(mime), "{mime}");
        }
        for (kind, mime) in [
            (MediaKind::Image, "image/heic"),
            (MediaKind::Image, "image/gif"),
            (MediaKind::Document, "text/plain"),
            (MediaKind::Audio, "audio/mp3"),
        ] {
            assert!(!kind.fetched_by_gemini(mime), "{mime}");
        }
        assert!(!worth_downloading(
            MediaKind::Image,
            Some("image/svg+xml"),
            true
        ));
        assert!(!worth_downloading(
            MediaKind::Image,
            Some("image/gif"),
            false
        ));
        assert!(worth_downloading(MediaKind::Image, Some("image/gif"), true));
        assert!(worth_downloading(MediaKind::Image, None, false));
    }

    #[test]
    fn a_probed_link_must_be_small_and_served_as_its_declared_type() {
        assert_eq!(content_range_total("bytes 0-0/1234"), Some(1234));
        assert_eq!(content_range_total("Bytes 0-0/15000000"), Some(15_000_000));
        assert_eq!(content_range_total("bytes 0-0/*"), None);
        assert_eq!(content_range_total("items 0-0/12"), None);

        let probe = |size, content_type: &str| LinkProbe {
            size,
            content_type: Some(normalize_mime_type(content_type)),
        };
        assert!(probe(Some(LINK_MAX_BYTES), "image/jpeg; charset=binary").fits("image/jpeg"));
        assert!(probe(Some(1), "video/quicktime").fits("video/mov"));
        assert!(!probe(Some(LINK_MAX_BYTES + 1), "image/jpeg").fits("image/jpeg"));
        assert!(!probe(None, "image/jpeg").fits("image/jpeg"));
        assert!(!probe(Some(1), "application/octet-stream").fits("image/jpeg"));
        let untyped = LinkProbe {
            size: Some(1),
            content_type: None,
        };
        assert!(!untyped.fits("application/pdf"));
    }

    #[tokio::test]
    async fn probes_read_size_and_type_from_a_ranged_get() {
        for (response, size) in [
            (
                "HTTP/1.1 206 Partial Content\r\ncontent-type: image/jpeg\r\ncontent-range: bytes 0-0/2048\r\ncontent-length: 1\r\nconnection: close\r\n\r\nx",
                Some(2048),
            ),
            (
                "HTTP/1.1 200 OK\r\ncontent-type: image/jpeg\r\ncontent-length: 4\r\nconnection: close\r\n\r\nxxxx",
                Some(4),
            ),
            (
                "HTTP/1.1 206 Partial Content\r\ncontent-type: image/jpeg\r\ncontent-range: bytes 0-0/*\r\ncontent-length: 1\r\nconnection: close\r\n\r\nx",
                None,
            ),
            (
                "HTTP/1.1 200 OK\r\ncontent-type: image/jpeg\r\ncontent-encoding: zstd\r\ncontent-length: 4\r\nconnection: close\r\n\r\nxxxx",
                None,
            ),
        ] {
            let (addr, _) = answer_every_connection(response).await;
            let probe = local_fetcher(addr)
                .probe(local_url(addr, "/photo.jpg"))
                .await
                .expect(response);
            assert_eq!(probe.size, size, "{response}");
            assert_eq!(probe.content_type.as_deref(), Some("image/jpeg"));
        }
        let (addr, _) = answer_every_connection(FORBIDDEN).await;
        let fetcher = local_fetcher(addr);
        assert!(fetcher.probe(local_url(addr, "/photo.jpg")).await.is_none());
    }

    #[test]
    fn budgets_stop_at_the_part_cap_and_at_the_shared_request_cap() {
        let request = RequestBudget::new(10);
        let mut first = PartBudget::new(&request, 6);
        assert_eq!(first.admits(7), Err(CapExceeded::Part(6)));
        assert_eq!(first.take(6), Ok(()));
        assert_eq!(first.take(1), Err(CapExceeded::Part(6)));
        let mut second = PartBudget::new(&request, u64::MAX);
        assert_eq!(second.admits(5), Err(CapExceeded::Request));
        assert_eq!(second.take(4), Ok(()));
        assert_eq!(second.take(1), Err(CapExceeded::Request));

        assert_eq!(
            RequestBudget::after_inline(0).cap,
            REQUEST_MEDIA_BASE64_BYTES / 4 * 3
        );
        assert_eq!(
            RequestBudget::after_inline(REQUEST_MEDIA_BASE64_BYTES - 4_000).cap,
            3_000
        );
        assert_eq!(RequestBudget::after_inline(u64::MAX).cap, 0);
    }

    #[tokio::test]
    async fn downloads_stop_reading_at_the_cap() {
        let (addr, _) = answer_every_connection(concat!(
            "HTTP/1.1 200 OK\r\ncontent-type: video/mp4\r\nconnection: close\r\n\r\n",
            "0123456789abcdef0123456789abcdef"
        ))
        .await;
        let fetcher = local_fetcher(addr);
        let url = local_url(addr, "/clip.mp4");

        let unlimited = RequestBudget::new(u64::MAX);
        let failure = fetcher
            .download(url.clone(), &mut PartBudget::new(&unlimited, 16))
            .await
            .unwrap_err();
        assert!(
            matches!(failure, FetchFailure::TooLarge(CapExceeded::Part(16))),
            "{failure}"
        );

        let small = RequestBudget::new(16);
        let failure = fetcher
            .download(url.clone(), &mut PartBudget::new(&small, u64::MAX))
            .await
            .unwrap_err();
        assert!(
            matches!(failure, FetchFailure::TooLarge(CapExceeded::Request)),
            "{failure}"
        );

        let exact = RequestBudget::new(32);
        let downloaded = fetcher
            .download(url, &mut PartBudget::new(&exact, 32))
            .await
            .unwrap();
        assert_eq!(downloaded.bytes.len(), 32);
        assert_eq!(downloaded.content_type.as_deref(), Some("video/mp4"));
    }

    #[tokio::test]
    async fn media_downloads_refuse_plain_http_private_targets_and_redirects() {
        let fetcher =
            MediaFetcher::new(ProviderEgress::Public, reqwest::Client::builder()).unwrap();
        let unlimited = RequestBudget::new(u64::MAX);
        for url in [
            "http://example.com/photo.jpg",
            "https://127.0.0.1/photo.jpg",
            "https://10.0.0.1/photo.jpg",
            "https://[::1]/photo.jpg",
            "https://169.254.169.254/latest/meta-data/",
            "https://localhost/photo.jpg",
            "file:///etc/passwd",
        ] {
            let failure = fetcher
                .download(
                    Url::parse(url).unwrap(),
                    &mut PartBudget::new(&unlimited, u64::MAX),
                )
                .await
                .unwrap_err();
            assert!(matches!(failure, FetchFailure::Refused), "{url}: {failure}");
        }

        let (addr, served) = answer_every_connection(
            "HTTP/1.1 302 Found\r\nlocation: http://media.test/next.jpg\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await;
        let failure = local_fetcher(addr)
            .download(
                local_url(addr, "/photo.jpg"),
                &mut PartBudget::new(&unlimited, u64::MAX),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(failure, FetchFailure::Status(StatusCode::FOUND)),
            "{failure}"
        );
        assert_eq!(served.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn links_outside_the_public_web_fail_the_call_without_connecting() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/path/photo.jpg?X-Amz-Signature=abc",
            listener.local_addr().unwrap()
        );
        let mut message = prompt(vec![image(link(&url), Some(ImageMediaType::PNG))]);

        let error = prepare_media(
            &mut message,
            &mut [],
            true,
            ProviderEgress::Public,
            reqwest::Client::builder(),
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("image at host '127.0.0.1'") && error.contains("public https"),
            "{error}"
        );
        assert!(!error.contains("X-Amz-Signature"), "{error}");
        assert!(
            tokio::time::timeout(Duration::from_millis(200), listener.accept())
                .await
                .is_err(),
            "a refused download never connects"
        );
    }

    #[tokio::test]
    async fn failed_links_name_their_kind_and_host_but_never_the_link() {
        let (addr, served) = answer_every_connection(FORBIDDEN).await;
        let url = local_url(addr, "/private/clip.mp4?X-Amz-Signature=SECRET");
        let error = prepare_local(vec![video(url.as_str(), Some(VideoMediaType::MP4))], addr)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("video at host 'media.test'") && error.contains("403"),
            "{error}"
        );
        assert!(
            !error.contains("SECRET") && !error.contains("/private/"),
            "{error}"
        );
        assert_eq!(served.load(Ordering::SeqCst), 1);

        let closed = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let url = local_url(closed, "/private/scan.pdf?X-Amz-Signature=SECRET");
        let pdf = document(link(url.as_str()), Some(DocumentMediaType::PDF));
        let error = prepare_local(vec![pdf], closed)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("document at host 'media.test'"), "{error}");
        assert!(
            !error.contains("SECRET") && !error.contains("/private/"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn links_download_concurrently_up_to_the_limit_and_take_the_served_type() {
        let (addr, peak) = answer_slowly(
            "HTTP/1.1 200 OK\r\ncontent-type: audio/wav\r\ncontent-length: 4\r\nconnection: close\r\n\r\nRIFF",
        )
        .await;
        let recordings = (0..6)
            .map(|index| {
                audio(
                    link(local_url(addr, &format!("/recording-{index}")).as_str()),
                    None,
                )
            })
            .collect();

        let message = prepare_local(recordings, addr).await.unwrap();

        let inlined = audio(
            DocumentSourceKind::Base64(BASE64.encode(b"RIFF")),
            Some(AudioMediaType::WAV),
        );
        assert!(parts(&message).iter().all(|part| *part == inlined));
        let peak = peak.load(Ordering::SeqCst);
        assert!(
            (2..=CONCURRENT_DOWNLOADS).contains(&peak),
            "{peak} downloads at once"
        );
    }

    #[tokio::test]
    async fn references_gemini_resolves_itself_stay_links() {
        let references = vec![
            video("https://www.youtube.com/watch?v=9hE5-98ZeCg", None),
            document(
                link("gs://reports-bucket/q3.pdf"),
                Some(DocumentMediaType::PDF),
            ),
            image(
                link("https://generativelanguage.googleapis.com/v1beta/files/abc-123"),
                Some(ImageMediaType::PNG),
            ),
        ];
        let mut message = prompt(references.clone());

        prepare_media(
            &mut message,
            &mut [],
            true,
            ProviderEgress::Public,
            reqwest::Client::builder(),
        )
        .await
        .unwrap();

        assert_eq!(parts(&message), references);
    }

    #[tokio::test]
    async fn inline_gifs_become_pngs_when_this_build_can_convert_them() {
        let gif = image(
            DocumentSourceKind::Base64(GIF_1X1.to_string()),
            Some(ImageMediaType::GIF),
        );
        let mut message = prompt(vec![gif.clone()]);

        prepare_media(
            &mut message,
            &mut [],
            true,
            ProviderEgress::Public,
            reqwest::Client::builder(),
        )
        .await
        .unwrap();

        let [part] = parts(&message).try_into().unwrap();
        if !converts_gif() {
            assert_eq!(part, gif, "without codecs Rig refuses the GIF, as before");
            return;
        }
        let RigUserContent::Image(image) = part else {
            panic!("expected an image");
        };
        assert_eq!(image.media_type, Some(ImageMediaType::PNG));
        let DocumentSourceKind::Base64(png) = image.data else {
            panic!("expected base64 data");
        };
        assert!(
            BASE64
                .decode(png)
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
    }

    #[test]
    fn thinking_off_uses_the_lowest_setting_each_model_accepts() {
        let mut history = History::new("gemini".to_string(), Vec::new());
        history.thinking = Some(HistoryThinking::Off);
        let config = |model| thinking_config_for_history(Some(&history), Some(model));

        assert!(matches!(
            config("gemini-3-flash-preview").thinking_level,
            Some(ThinkingLevel::Minimal)
        ));
        assert!(matches!(
            config("gemini-3.1-pro-preview").thinking_level,
            Some(ThinkingLevel::Low)
        ));
        assert!(matches!(
            config("gemini-3.8-flash").thinking_level,
            Some(ThinkingLevel::Low)
        ));
        assert!(matches!(
            config("gemini-3-pro-image-preview").thinking_level,
            Some(ThinkingLevel::High)
        ));
        assert_eq!(
            config("gemini-2.5-pro").thinking_budget,
            Some(GEMINI_2_5_PRO_MIN_BUDGET)
        );
        assert_eq!(config("gemini-2.5-flash").thinking_budget, Some(0));
    }
}
