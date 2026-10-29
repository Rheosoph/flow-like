//! User media in provider requests. Every request fixup words its notes and link errors here.
//! The providers whose Rig 0.38.2 converter drops media or fails the call on it build their
//! parts here too: Ollama, Mistral, DeepSeek, Cohere, Perplexity, Galadriel and Mira, plus the
//! link errors of the llama.cpp client.
//!
//! A provider's request fixup maps every user part to a [`Part`] with [`deliver`]: left to Rig,
//! replaced for Rig, an OpenAI-shaped part, a document's text, or a [`note`] in place of media
//! the provider cannot read at all. A link to media the provider reads, though not by that link,
//! fails the call with the media's kind, type and host: the run inlines such media before the
//! call, so a link left over means that failed. The link itself is never logged or echoed: a
//! signed link is a credential.
//!
//! Rig sends Mistral, DeepSeek, Cohere and Perplexity user content as text, so a message with
//! OpenAI-shaped parts passes their converters as one text carrier, which [`MediaSplice`]
//! replaces with the parts in the request body.

use std::borrow::Cow;
use std::net::IpAddr;
use std::sync::{Arc, LazyLock};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use rig::OneOrMany;
use rig::agent::AgentBuilder;
use rig::completion::{
    CompletionError, CompletionRequest, CompletionRequestBuilder, CompletionResponse, Message,
};
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
};
use rig::message::{
    Document, DocumentMediaType, DocumentSourceKind, Image, ImageDetail, ImageMediaType, MimeType,
    UserContent,
};
use rig::streaming::StreamingCompletionResponse;
use rig::wasm_compat::{WasmBoxedFuture, WasmCompatSend};
use serde_json::{Value, json};

use crate::authorization::AuthorizedHttpClient;
use crate::egress::ProviderEgress;
use crate::llm::{
    CompletionClientDyn, CompletionModelDyn, CompletionModelHandle, DynamicResponse,
    DynamicStreamingResponse, ModelConstructor,
};

/// Rewrites a request for the named model before the provider client sees it, or refuses it.
pub(crate) type MediaFixup =
    fn(&str, CompletionRequest) -> Result<CompletionRequest, CompletionError>;

/// A constructor whose requests pass `fixup` first, for `invoke` and agents alike.
pub(crate) fn constructor(
    client: impl CompletionClientDyn + Send + Sync + 'static,
    fixup: MediaFixup,
) -> ModelConstructor {
    ModelConstructor {
        inner: Box::new(MediaClient {
            client: Box::new(client),
            fixup,
        }),
    }
}

struct MediaClient {
    client: Box<dyn CompletionClientDyn + Send + Sync>,
    fixup: MediaFixup,
}

impl MediaClient {
    fn model<'a>(&self, model: &str) -> MediaModel<'a> {
        MediaModel {
            inner: Arc::from(self.client.completion_model(model)),
            model: model.to_string(),
            fixup: self.fixup,
        }
    }
}

impl CompletionClientDyn for MediaClient {
    fn completion_model<'a>(&self, model: &str) -> Box<dyn CompletionModelDyn + 'a> {
        Box::new(self.model(model))
    }

    fn agent<'a>(&self, model: &str) -> AgentBuilder<CompletionModelHandle<'a>> {
        AgentBuilder::new(CompletionModelHandle::new(Arc::new(self.model(model))))
    }
}

#[derive(Clone)]
struct MediaModel<'a> {
    inner: Arc<dyn CompletionModelDyn + 'a>,
    model: String,
    fixup: MediaFixup,
}

impl MediaModel<'_> {
    fn fix(&self, request: CompletionRequest) -> Result<CompletionRequest, CompletionError> {
        let model = request.model.clone().unwrap_or_else(|| self.model.clone());
        (self.fixup)(&model, request)
    }
}

impl CompletionModelDyn for MediaModel<'_> {
    fn completion(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<'_, Result<CompletionResponse<DynamicResponse>, CompletionError>> {
        match self.fix(request) {
            Ok(request) => self.inner.completion(request),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }

    fn stream(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<
        '_,
        Result<StreamingCompletionResponse<DynamicStreamingResponse>, CompletionError>,
    > {
        match self.fix(request) {
            Ok(request) => self.inner.stream(request),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }

    fn completion_request(
        &self,
        prompt: Message,
    ) -> CompletionRequestBuilder<CompletionModelHandle<'_>> {
        CompletionRequestBuilder::new(CompletionModelHandle::new(Arc::new(self.clone())), prompt)
    }
}

/// What a user part becomes for the provider.
pub(crate) enum Part {
    /// Left to Rig's converter.
    Keep,
    /// Replaces the part for Rig's converter.
    Rig(UserContent),
    /// An OpenAI-shaped content part, which Rig's converter cannot express.
    Wire(Value),
    /// A document's text.
    Text(String),
    /// A note in place of media the provider cannot read.
    Omitted { kind: &'static str, note: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Image,
    Document,
    Audio,
    Video,
}

impl Kind {
    fn singular(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Document => "document",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }

    pub(crate) fn plural(self) -> &'static str {
        match self {
            Self::Image => "images",
            Self::Document => "documents",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

/// Where the bytes of a media part are.
pub(crate) enum Source<'a> {
    /// An https link to a host that is neither this machine nor a private network.
    Link(&'a str),
    /// A link no remote provider is sent: http, a local or private host, or another scheme, such
    /// as the desktop's `asset://`.
    LocalLink(&'a str),
    /// Base64 bytes, with the MIME type of the `data:` URL that held them.
    Inline {
        base64: Cow<'a, str>,
        mime: Option<&'a str>,
    },
    /// A text document's text.
    Text(&'a str),
    /// A provider's file id.
    FileId,
    /// A `data:` URL that is not base64, or nothing.
    Other,
}

impl<'a> Source<'a> {
    pub(crate) fn of(data: &'a DocumentSourceKind) -> Self {
        match data {
            DocumentSourceKind::Url(url) => match url.strip_prefix("data:") {
                Some(data_url) => data_source(data_url),
                None if public_link(url) => Self::Link(url),
                None => Self::LocalLink(url),
            },
            DocumentSourceKind::Base64(base64) => Self::Inline {
                base64: Cow::Borrowed(base64),
                mime: None,
            },
            DocumentSourceKind::Raw(bytes) => Self::Inline {
                base64: Cow::Owned(STANDARD.encode(bytes)),
                mime: None,
            },
            DocumentSourceKind::String(text) => Self::Text(text),
            DocumentSourceKind::FileId(_) => Self::FileId,
            _ => Self::Other,
        }
    }
}

fn data_source(data_url: &str) -> Source<'_> {
    match data_url.split_once(',') {
        Some((metadata, base64)) if metadata.ends_with(";base64") => Source::Inline {
            base64: Cow::Borrowed(base64),
            mime: metadata.split(';').next().filter(|mime| !mime.is_empty()),
        },
        _ => Source::Other,
    }
}

/// As far as the link itself tells: a host name may still resolve to a private address.
fn public_link(link: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(link) else {
        return false;
    };
    let Some(host) = url.host_str().filter(|host| !host.is_empty()) else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match host.parse::<IpAddr>() {
        Ok(ip) => ProviderEgress::Public.allows_ip(ip),
        Err(_) => {
            let host = host.to_ascii_lowercase();
            host != "localhost" && !host.ends_with(".localhost")
        }
    }
}

/// The host a link points to, for errors that must not echo the link.
fn origin(link: &str) -> String {
    match reqwest::Url::parse(link) {
        Ok(url) => match url.host_str().filter(|host| !host.is_empty()) {
            Some(host) => format!("host '{host}'"),
            None => format!("a {}: link", url.scheme()),
        },
        Err(_) => "a link that is not a valid URL".to_string(),
    }
}

fn described(kind: Kind, mime: Option<&str>) -> String {
    match mime {
        Some(mime) => format!("{} ({mime})", kind.singular()),
        None => kind.singular().to_string(),
    }
}

fn request_error(message: String) -> CompletionError {
    CompletionError::RequestError(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message,
    )))
}

/// A link to media that `provider` reads only inline.
pub(crate) fn inline_only(
    provider: &str,
    kind: Kind,
    mime: Option<&str>,
    link: &str,
) -> CompletionError {
    request_error(format!(
        "The {} from {} cannot be sent to {provider} as a link; send the {} inline as base64 data",
        described(kind, mime),
        origin(link),
        kind.singular()
    ))
}

/// A link to media that `provider` would fetch, were it a public https link.
pub(crate) fn unreachable(
    provider: &str,
    kind: Kind,
    mime: Option<&str>,
    link: &str,
) -> CompletionError {
    request_error(format!(
        "The {} from {} cannot be sent to {provider} as a link, since {provider} fetches only \
         public https links; send the {} inline as base64 data",
        described(kind, mime),
        origin(link),
        kind.singular()
    ))
}

/// The note in place of media the provider cannot read at all. It names the MIME type the media
/// was sent with: declared, held by its `data:` URL, or, for an image, told by its bytes or its
/// link's file extension. Media without one is named by its kind.
pub(crate) fn note(kind: Kind, mime: Option<&str>, source: &Source) -> String {
    let named = mime.or_else(|| match source {
        Source::Inline { base64, mime } => mime.or_else(|| match kind {
            Kind::Image => sniff_image(base64),
            _ => None,
        }),
        Source::Link(link) | Source::LocalLink(link) if kind == Kind::Image => {
            image_type_of_link(link)
        }
        _ => None,
    });
    let kind_name = match source {
        Source::FileId => "provider file id",
        _ => kind.singular(),
    };
    format!(
        "[attachment omitted: unsupported file type {}]",
        named.unwrap_or(kind_name)
    )
}

pub(crate) fn omitted(kind: Kind, mime: Option<&str>, source: &Source) -> Part {
    Part::Omitted {
        kind: kind.plural(),
        note: note(kind, mime, source),
    }
}

pub(crate) fn mime_of<T: MimeType>(media_type: Option<&T>) -> Option<&'static str> {
    media_type.map(MimeType::to_mime_type)
}

/// The part for a provider that reads only text: a document as its text, other media as a note.
pub(crate) fn text_only(provider: &str, part: &UserContent) -> Result<Part, CompletionError> {
    Ok(match part {
        UserContent::Text(_) | UserContent::ToolResult(_) => Part::Keep,
        UserContent::Document(document) => {
            return Doc::of(document).read_as_text(provider, document);
        }
        UserContent::Image(image) => omitted(
            Kind::Image,
            mime_of(image.media_type.as_ref()),
            &Source::of(&image.data),
        ),
        UserContent::Audio(audio) => omitted(
            Kind::Audio,
            mime_of(audio.media_type.as_ref()),
            &Source::of(&audio.data),
        ),
        UserContent::Video(video) => omitted(
            Kind::Video,
            mime_of(video.media_type.as_ref()),
            &Source::of(&video.data),
        ),
    })
}

fn is_text_type(document: &Document) -> bool {
    document
        .media_type
        .as_ref()
        .is_some_and(|media_type| *media_type != DocumentMediaType::PDF)
}

/// A document as the providers here read it.
pub(crate) enum Doc<'a> {
    /// The text of a text document, or of base64 bytes that are text. Decoded text carries a
    /// `[document: …]` header, as `media::retain_supported` writes it.
    Text(String),
    /// A document file, with the type declared or recognized from its bytes.
    File {
        mime: Option<&'a str>,
        source: Source<'a>,
    },
}

impl<'a> Doc<'a> {
    pub(crate) fn of(document: &'a Document) -> Self {
        let declared = mime_of(document.media_type.as_ref());
        let source = Source::of(&document.data);
        let (decoded, mime) = match &source {
            Source::Text(text) => return Doc::Text((*text).to_string()),
            Source::Inline { base64, mime } if declared != Some(PDF) => {
                (STANDARD.decode(base64.as_bytes()).ok(), declared.or(*mime))
            }
            _ => (None, declared),
        };
        let Some(bytes) = decoded else {
            return Doc::File { mime, source };
        };
        if declared.is_some() {
            return Doc::Text(decoded_text(mime, &String::from_utf8_lossy(&bytes)));
        }
        if let Some(sniffed) = sniff_document(&bytes) {
            return Doc::File {
                mime: Some(sniffed),
                source,
            };
        }
        match std::str::from_utf8(&bytes) {
            Ok(text) => Doc::Text(decoded_text(mime, text)),
            Err(_) => Doc::File { mime, source },
        }
    }

    /// The part for a provider that reads `document` only as text. A link to a text document
    /// fails, since its text would be readable inline; any other document file becomes a note.
    pub(crate) fn read_as_text(
        self,
        provider: &str,
        document: &Document,
    ) -> Result<Part, CompletionError> {
        match self {
            Doc::Text(text) => Ok(Part::Text(text)),
            Doc::File {
                mime,
                source: Source::Link(link) | Source::LocalLink(link),
            } if is_text_type(document) => Err(inline_only(provider, Kind::Document, mime, link)),
            Doc::File { mime, source } => Ok(omitted(Kind::Document, mime, &source)),
        }
    }
}

fn decoded_text(mime: Option<&str>, text: &str) -> String {
    format!("[document: {}]\n{text}", mime.unwrap_or("unknown type"))
}

pub(crate) const PDF: &str = "application/pdf";
pub(crate) const DOCX: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
pub(crate) const DOC: &str = "application/msword";

/// PDF, or Word's DOCX and DOC, by their bytes.
pub(crate) fn sniff_document(bytes: &[u8]) -> Option<&'static str> {
    const WORD_DOCUMENT: &[u8] = b"W\0o\0r\0d\0D\0o\0c\0u\0m\0e\0n\0t\0";
    if bytes.starts_with(b"%PDF-") {
        Some(PDF)
    } else if bytes.starts_with(b"PK\x03\x04") && contains(bytes, b"word/") {
        Some(DOCX)
    } else if bytes.starts_with(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1")
        && contains(bytes, WORD_DOCUMENT)
    {
        Some(DOC)
    } else {
        None
    }
}

/// The bytes at the start of a base64 payload, enough to recognise a file format.
pub(crate) fn leading_bytes(base64: &str, count: usize) -> Vec<u8> {
    let chars = base64.len().min(count.div_ceil(3) * 4) / 4 * 4;
    STANDARD
        .decode(&base64.as_bytes()[..chars])
        .unwrap_or_default()
}

/// The image type that the first bytes of `base64` name.
pub(crate) fn sniff_image(base64: &str) -> Option<&'static str> {
    const MAGIC: [(&[u8], &str); 7] = [
        (b"\x89PNG\r\n\x1a\n", "image/png"),
        (b"\xff\xd8\xff", "image/jpeg"),
        (b"GIF87a", "image/gif"),
        (b"GIF89a", "image/gif"),
        (b"II*\0", "image/tiff"),
        (b"MM\0*", "image/tiff"),
        (b"BM", "image/bmp"),
    ];
    let head = leading_bytes(base64, 12);
    if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    MAGIC
        .iter()
        .find(|(magic, _)| head.starts_with(magic))
        .map(|(_, mime)| *mime)
}

const WEB_IMAGES: [&str; 5] = [
    "image/jpeg",
    "image/jpg",
    "image/png",
    "image/gif",
    "image/webp",
];

/// Image types by link file extension, for links without a media type.
const IMAGE_EXTENSIONS: [(&str, &str); 13] = [
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("bmp", "image/bmp"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("ico", "image/vnd.microsoft.icon"),
    ("svg", "image/svg+xml"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
];

/// The last path segment of a link, without its query and fragment.
pub(crate) fn link_file_name(link: &str) -> &str {
    let path = link.split(['?', '#']).next().unwrap_or_default();
    path.rsplit('/').next().unwrap_or_default()
}

/// The image type a link's file extension names.
pub(crate) fn image_type_of_link(link: &str) -> Option<&'static str> {
    let (_, extension) = link_file_name(link).rsplit_once('.')?;
    IMAGE_EXTENSIONS
        .iter()
        .find(|(known, _)| extension.eq_ignore_ascii_case(known))
        .map(|(_, mime)| *mime)
}

/// Image bytes travel as base64, typed by their content where Rig has a name for the type.
pub(crate) fn type_image_bytes(image: &mut Image) {
    if let DocumentSourceKind::Raw(bytes) = &image.data {
        image.data = DocumentSourceKind::Base64(STANDARD.encode(bytes));
    }
    if image.media_type.is_none()
        && let DocumentSourceKind::Base64(data) = &image.data
    {
        image.media_type = sniff_image(data).and_then(ImageMediaType::from_mime_type);
    }
}

/// An image's MIME type: declared, held by its `data:` URL, or told by its link's file extension.
pub(crate) fn image_mime<'a>(image: &'a Image, source: &Source<'a>) -> Option<&'a str> {
    mime_of(image.media_type.as_ref()).or_else(|| match source {
        Source::Link(link) | Source::LocalLink(link) => image_type_of_link(link),
        Source::Inline { mime, .. } => *mime,
        _ => None,
    })
}

/// An `image_url` part for a provider that fetches public https links and reads JPEG, PNG, GIF
/// and WebP as links or `data:` URLs. `detail` is sent when the provider takes it.
pub(crate) fn web_image(
    provider: &str,
    image: &Image,
    detail: bool,
) -> Result<Part, CompletionError> {
    let source = Source::of(&image.data);
    let mime = mime_of(image.media_type.as_ref()).or_else(|| match &source {
        Source::Link(link) | Source::LocalLink(link) => image_type_of_link(link),
        Source::Inline { base64, mime } => mime.or_else(|| sniff_image(base64)),
        _ => None,
    });
    if mime.is_some_and(|mime| !WEB_IMAGES.contains(&mime)) {
        return Ok(omitted(Kind::Image, mime, &source));
    }
    let url = match (&source, mime) {
        (Source::Link(link), _) => (*link).to_string(),
        (Source::LocalLink(link), _) => {
            return Err(unreachable(provider, Kind::Image, mime, link));
        }
        (Source::Inline { base64, .. }, Some(mime)) => format!("data:{mime};base64,{base64}"),
        _ => return Ok(omitted(Kind::Image, mime, &source)),
    };
    let mut image_url = json!({ "url": url });
    if detail && let Some(detail) = &image.detail {
        image_url["detail"] = json!(match detail {
            ImageDetail::Low => "low",
            ImageDetail::High => "high",
            ImageDetail::Auto => "auto",
        });
    }
    Ok(Part::Wire(
        json!({ "type": "image_url", "image_url": image_url }),
    ))
}

/// Maps every user part with `classify`. A message with an OpenAI-shaped part becomes one carrier
/// of its parts, except for tool results, which stay for Rig's converter. `classify` leaves only
/// text and tool results to Rig in such a message.
pub(crate) fn deliver(
    provider: &'static str,
    model: &str,
    mut request: CompletionRequest,
    classify: impl Fn(&UserContent) -> Result<Part, CompletionError>,
) -> Result<CompletionRequest, CompletionError> {
    let mut omitted = Vec::new();
    for message in request.chat_history.iter_mut() {
        let Message::User { content } = message else {
            continue;
        };
        let parts = content
            .iter()
            .map(&classify)
            .collect::<Result<Vec<_>, _>>()?;
        omitted.extend(parts.iter().filter_map(|part| match part {
            Part::Omitted { kind, .. } => Some(*kind),
            _ => None,
        }));
        rewrite(content, parts);
    }
    if !omitted.is_empty() {
        let count = omitted.len();
        omitted.sort_unstable();
        omitted.dedup();
        tracing::warn!(
            provider,
            model,
            omitted = count,
            kinds = ?omitted,
            "Replaced user attachments the provider cannot read with notes"
        );
    }
    Ok(request)
}

fn rewrite(content: &mut OneOrMany<UserContent>, parts: Vec<Part>) {
    if parts.iter().any(|part| matches!(part, Part::Wire(_))) {
        *content = carried(content, parts);
        return;
    }
    for (slot, part) in content.iter_mut().zip(parts) {
        match part {
            Part::Keep | Part::Wire(_) => {}
            Part::Rig(part) => *slot = part,
            Part::Text(text) | Part::Omitted { note: text, .. } => *slot = UserContent::text(text),
        }
    }
}

/// The carrier sits where the first carried part was.
fn carried(content: &OneOrMany<UserContent>, parts: Vec<Part>) -> OneOrMany<UserContent> {
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut wire = Vec::new();
    for (original, part) in content.iter().zip(parts) {
        let text = match (part, original) {
            (Part::Wire(value), _) => {
                wire.push(value);
                continue;
            }
            (Part::Text(text) | Part::Omitted { note: text, .. }, _) => text,
            (Part::Rig(UserContent::Text(text)), _) => text.text,
            (Part::Keep, UserContent::Text(text)) => text.text.clone(),
            (part, original) => {
                let kept = match part {
                    Part::Rig(kept) => kept,
                    _ => original.clone(),
                };
                if wire.is_empty() {
                    before.push(kept);
                } else {
                    after.push(kept);
                }
                continue;
            }
        };
        wire.push(json!({ "type": "text", "text": text }));
    }
    let mut message = OneOrMany::one(UserContent::text(format!(
        "{}{}",
        *CARRIER,
        Value::Array(wire)
    )));
    for part in before.into_iter().rev() {
        message.insert(0, part);
    }
    for part in after {
        message.push(part);
    }
    message
}

/// Opens a carrier. The nonce is drawn once per process, so text a user writes cannot pose as
/// one.
static CARRIER: LazyLock<String> =
    LazyLock::new(|| format!("flow-like-media-parts:{:032x}:", rand::random::<u128>()));

/// Rig's HTTP client for Mistral, DeepSeek, Cohere and Perplexity. It puts each carrier's parts
/// in place of its text and refuses to send a request with a carrier it could not place.
#[derive(Clone, Debug, Default)]
pub(crate) struct MediaSplice(AuthorizedHttpClient);

impl MediaSplice {
    pub(crate) fn for_egress(egress: ProviderEgress) -> anyhow::Result<Self> {
        AuthorizedHttpClient::for_egress(egress).map(Self)
    }
}

impl HttpClientExt for MediaSplice {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let request = spliced(request);
        let client = self.0.clone();
        async move { client.send(request?).await }
    }

    fn send_multipart<U>(
        &self,
        request: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.0.send_multipart(request)
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        let request = spliced(request);
        let client = self.0.clone();
        async move { client.send_streaming(request?).await }
    }
}

fn spliced<T: Into<Bytes>>(request: Request<T>) -> http_client::Result<Request<Bytes>> {
    let (parts, body) = request.into_parts();
    let body: Bytes = body.into();
    if !contains(&body, CARRIER.as_bytes()) {
        return Ok(Request::from_parts(parts, body));
    }
    let body = splice(&body)
        .map_err(|error| http_client::Error::Instance(Box::new(std::io::Error::other(error))))?;
    Ok(Request::from_parts(parts, body))
}

fn splice(body: &[u8]) -> Result<Bytes, String> {
    let mut request: Value = serde_json::from_slice(body)
        .map_err(|error| format!("The request that carries media parts is not JSON: {error}"))?;
    let user_contents = request
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .filter_map(|message| message.get_mut("content"));
    for content in user_contents {
        place(content)?;
    }
    let body = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    if contains(&body, CARRIER.as_bytes()) {
        return Err(
            "The media parts of a user message have no place in the provider request, so it \
             was not sent"
                .to_string(),
        );
    }
    Ok(Bytes::from(body))
}

/// Puts the parts of a carrier in place of a user message's string content, or of the text
/// part that holds it.
fn place(content: &mut Value) -> Result<(), String> {
    let placed = match content {
        Value::String(text) => match carried_parts(text)? {
            Some(parts) => parts,
            None => return Ok(()),
        },
        Value::Array(items) => {
            let mut placed = Vec::with_capacity(items.len());
            for item in std::mem::take(items) {
                let text = item
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|_| item.get("type").and_then(Value::as_str) == Some("text"));
                match text.map(carried_parts).transpose()?.flatten() {
                    Some(parts) => placed.extend(parts),
                    None => placed.push(item),
                }
            }
            placed
        }
        _ => return Ok(()),
    };
    *content = Value::Array(placed);
    Ok(())
}

fn carried_parts(text: &str) -> Result<Option<Vec<Value>>, String> {
    let Some(parts) = text.strip_prefix(CARRIER.as_str()) else {
        return Ok(None);
    };
    serde_json::from_str(parts)
        .map(Some)
        .map_err(|error| format!("A media carrier is not a JSON array of parts: {error}"))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rig::message::{Audio, AudioMediaType, ToolResult, ToolResultContent};

    pub(crate) const PRESIGNED_PNG: &str = "https://flow-like-content.s3.eu-central-1.amazonaws.com/tmp/runs/r/scan.png?X-Amz-Credential=ASIAEXAMPLE&X-Amz-Signature=deadbeef";
    pub(crate) const PRESIGNED_PDF: &str = "https://flow-like-content.s3.eu-central-1.amazonaws.com/tmp/runs/r/report.pdf?X-Amz-Credential=ASIAEXAMPLE&X-Amz-Signature=deadbeef";
    /// The first bytes of a PNG, base64.
    pub(crate) const PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUg==";
    pub(crate) const PDF_BASE64: &str = "JVBERi0xLjQK";

    pub(crate) fn request(content: Vec<UserContent>) -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: Some("Answer from the attachments.".to_string()),
            chat_history: OneOrMany::one(Message::User {
                content: OneOrMany::many(content).unwrap(),
            }),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        }
    }

    pub(crate) fn image(
        data: DocumentSourceKind,
        media_type: Option<ImageMediaType>,
    ) -> UserContent {
        UserContent::Image(Image {
            data,
            media_type,
            detail: None,
            additional_params: None,
        })
    }

    pub(crate) fn document(
        data: DocumentSourceKind,
        media_type: Option<DocumentMediaType>,
    ) -> UserContent {
        UserContent::Document(Document {
            data,
            media_type,
            additional_params: None,
        })
    }

    pub(crate) fn audio(data: DocumentSourceKind) -> UserContent {
        UserContent::Audio(Audio {
            data,
            media_type: Some(AudioMediaType::WAV),
            additional_params: None,
        })
    }

    /// The error a request fails with, which must name the host but never the link.
    pub(crate) fn assert_names_host_only(error: &str, host: &str) {
        assert!(error.contains(&format!("host '{host}'")), "{error}");
        for secret in ["X-Amz", "scan.png", "report.pdf", "/tmp/"] {
            assert!(
                !error.contains(secret),
                "the error echoes the link: {error}"
            );
        }
    }

    fn body_with(content: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "model": "m",
            "messages": [
                {"role": "system", "content": "Be brief."},
                {"role": "user", "content": content},
            ],
        }))
        .unwrap()
    }

    fn carrier_of(parts: Value) -> String {
        format!("{}{parts}", *CARRIER)
    }

    #[test]
    fn a_carrier_becomes_its_parts_in_string_and_array_content() {
        let parts = json!([
            {"type": "text", "text": "What is on it?"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}},
        ]);
        for content in [
            json!(carrier_of(parts.clone())),
            json!([{"type": "text", "text": carrier_of(parts.clone())}]),
        ] {
            let body: Value =
                serde_json::from_slice(&splice(&body_with(content)).unwrap()).unwrap();
            assert_eq!(body["messages"][1]["content"], parts);
            assert_eq!(body["messages"][0]["content"], "Be brief.");
        }
    }

    #[test]
    fn text_that_only_looks_like_a_carrier_is_sent_as_written() {
        let forged = json!("flow-like-media-parts:0:[{\"type\":\"image_url\"}]");
        let body = Bytes::from(body_with(forged.clone()));
        let sent = spliced(
            Request::post("https://api.example.test")
                .body(body.clone())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(sent.body(), &body);
    }

    #[test]
    fn a_carrier_outside_user_content_stops_the_request() {
        let body = serde_json::to_vec(&json!({
            "messages": [{"role": "assistant", "content": carrier_of(json!([]))}],
        }))
        .unwrap();
        let error = splice(&body).unwrap_err();
        assert!(error.contains("was not sent"), "{error}");
    }

    #[test]
    fn a_message_with_wire_parts_travels_as_one_carrier_beside_its_tool_results() {
        let tool_result = UserContent::ToolResult(ToolResult {
            id: "call-1".to_string(),
            call_id: None,
            content: OneOrMany::one(ToolResultContent::text("42")),
        });
        let fixed = deliver(
            "Test",
            "model",
            request(vec![
                tool_result.clone(),
                UserContent::text("Compare."),
                image(DocumentSourceKind::Url(PRESIGNED_PNG.to_string()), None),
                document(
                    DocumentSourceKind::String("a,b".to_string()),
                    Some(DocumentMediaType::CSV),
                ),
                audio(DocumentSourceKind::Base64("AAAA".to_string())),
            ]),
            |part| match part {
                UserContent::Image(image) => web_image("Test", image, false),
                part => text_only("Test", part),
            },
        )
        .unwrap();

        let Message::User { content } = fixed.chat_history.first() else {
            panic!("the message is a user message");
        };
        assert_eq!(content.len(), 2);
        assert_eq!(content.first(), tool_result);
        let UserContent::Text(carrier) = content.last() else {
            panic!("the carrier is text");
        };
        let parts = carried_parts(&carrier.text).unwrap().unwrap();
        assert_eq!(
            Value::Array(parts),
            json!([
                {"type": "text", "text": "Compare."},
                {"type": "image_url", "image_url": {"url": PRESIGNED_PNG}},
                {"type": "text", "text": "a,b"},
                {"type": "text", "text": "[attachment omitted: unsupported file type audio/wav]"},
            ])
        );
    }

    #[test]
    fn links_name_their_host_and_never_the_link() {
        for (link, host, public) in [
            (
                PRESIGNED_PNG,
                "flow-like-content.s3.eu-central-1.amazonaws.com",
                true,
            ),
            (
                "http://localhost:9000/bucket/scan.png?X-Amz-Signature=1",
                "localhost",
                false,
            ),
            (
                "http://flow-like-content.s3.eu-central-1.amazonaws.com/tmp/scan.png?X-Amz-Signature=1",
                "flow-like-content.s3.eu-central-1.amazonaws.com",
                false,
            ),
            ("https://10.0.0.7/tmp/scan.png", "10.0.0.7", false),
            ("https://[::1]/tmp/scan.png", "[::1]", false),
            (
                "asset://localhost/%2FUsers%2Fme%2Fscan.png",
                "localhost",
                false,
            ),
        ] {
            let data = DocumentSourceKind::Url(link.to_string());
            assert_eq!(
                matches!(Source::of(&data), Source::Link(_)),
                public,
                "{host}"
            );
            let error = inline_only("Ollama", Kind::Image, Some("image/png"), link).to_string();
            assert_names_host_only(&error, host);
            assert!(error.contains("image (image/png)"), "{error}");
        }
        let error = unreachable("Mistral", Kind::Image, None, "file:///tmp/scan.png").to_string();
        assert!(error.contains("from a file: link"), "{error}");
        assert!(error.contains("public https links"), "{error}");
    }

    #[test]
    fn an_http_image_link_fails_naming_kind_type_and_host() {
        let link =
            "http://flow-like-content.s3.eu-central-1.amazonaws.com/tmp/scan.png?X-Amz-Signature=1";
        let UserContent::Image(png) = image(DocumentSourceKind::Url(link.to_string()), None) else {
            unreachable!()
        };

        let Err(error) = web_image("DeepSeek", &png, true) else {
            panic!("an http link must not reach DeepSeek");
        };

        let error = error.to_string();
        assert_names_host_only(&error, "flow-like-content.s3.eu-central-1.amazonaws.com");
        assert!(error.contains("The image (image/png) from"), "{error}");
        assert!(error.contains("cannot be sent to DeepSeek"), "{error}");
    }

    #[test]
    fn notes_name_the_type_the_media_was_sent_with_or_else_its_kind() {
        let note_of = |kind, data: DocumentSourceKind, mime| note(kind, mime, &Source::of(&data));
        for (kind, data, mime, expected) in [
            (
                Kind::Audio,
                DocumentSourceKind::Base64("AAAA".to_string()),
                Some("audio/wav"),
                "audio/wav",
            ),
            (
                Kind::Image,
                DocumentSourceKind::Url("data:image/heic;base64,AAAA".to_string()),
                None,
                "image/heic",
            ),
            (
                Kind::Image,
                DocumentSourceKind::Url(PRESIGNED_PNG.replace("scan.png", "scan.TIFF")),
                None,
                "image/tiff",
            ),
            (
                Kind::Image,
                DocumentSourceKind::Base64("R0lGODlhAQABAAAAACw=".to_string()),
                None,
                "image/gif",
            ),
            (
                Kind::Image,
                DocumentSourceKind::Base64("AAAAAAAA".to_string()),
                None,
                "image",
            ),
            (
                Kind::Document,
                DocumentSourceKind::FileId("file-abc".to_string()),
                None,
                "provider file id",
            ),
            (
                Kind::Document,
                DocumentSourceKind::FileId("file-abc".to_string()),
                Some(PDF),
                PDF,
            ),
            (
                Kind::Video,
                DocumentSourceKind::Url(PRESIGNED_PDF.replace("report.pdf", "clip")),
                None,
                "video",
            ),
        ] {
            assert_eq!(
                note_of(kind, data, mime),
                format!("[attachment omitted: unsupported file type {expected}]")
            );
        }
    }

    #[test]
    fn media_types_are_read_from_bytes() {
        assert_eq!(sniff_image(PNG_BASE64), Some("image/png"));
        assert_eq!(sniff_image("/9j/4AAQSkZJRg=="), Some("image/jpeg"));
        assert_eq!(sniff_image("UklGRiQAAABXRUJQVlA4"), Some("image/webp"));
        assert_eq!(sniff_image("R0lGODlhAQABAA=="), Some("image/gif"));
        assert_eq!(sniff_image("SGVsbG8="), None);
        assert_eq!(sniff_image("ü"), None);

        let mut docx = b"PK\x03\x04".to_vec();
        docx.extend_from_slice(b"[Content_Types].xml word/document.xml");
        let mut xlsx = b"PK\x03\x04".to_vec();
        xlsx.extend_from_slice(b"[Content_Types].xml xl/workbook.xml");
        assert_eq!(sniff_document(b"%PDF-1.7"), Some(PDF));
        assert_eq!(sniff_document(&docx), Some(DOCX));
        assert_eq!(sniff_document(&xlsx), None);
    }

    #[test]
    fn image_links_are_typed_by_their_file_extension() {
        for (link, expected) in [
            (PRESIGNED_PNG, Some("image/png")),
            (
                "https://cdn.example.com/a/photo.JPG#top",
                Some("image/jpeg"),
            ),
            (
                "asset://localhost/%2FUsers%2Fme%2Fscan.bmp",
                Some("image/bmp"),
            ),
            ("https://cdn.example.com/a/photo", None),
            ("https://cdn.example.com/a.png/photo", None),
        ] {
            assert_eq!(image_type_of_link(link), expected, "{link}");
        }
    }

    #[test]
    fn untyped_inline_documents_are_text_or_recognized_files() {
        let pdf = Document {
            data: DocumentSourceKind::Base64(PDF_BASE64.to_string()),
            media_type: None,
            additional_params: None,
        };
        assert!(matches!(
            Doc::of(&pdf),
            Doc::File {
                mime: Some(PDF),
                ..
            }
        ));
        let text = Document {
            data: DocumentSourceKind::Raw(b"plain words".to_vec()),
            ..pdf.clone()
        };
        assert!(
            matches!(Doc::of(&text), Doc::Text(text) if text == "[document: unknown type]\nplain words")
        );
        let binary = Document {
            data: DocumentSourceKind::Base64(STANDARD.encode([0xff, 0xfe, 0x00, 0x9f])),
            ..pdf
        };
        assert!(matches!(Doc::of(&binary), Doc::File { mime: None, .. }));
    }
}
