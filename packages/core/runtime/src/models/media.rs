//! Inlines remote documents and audio before a model call, since most providers refuse media
//! URLs. Only bodies served over https are inlined, after any redirects, and server-side the
//! egress guard refuses host-plane destinations on every hop. Private (RFC 1918 / ULA) hosts
//! stay reachable as for every guarded client: that space is the deployment's own network and
//! is governed by its network policy.

use crate::flow::execution::{ExecutionEnvironment, egress::GuardedHttpClient};
use flow_like_model_provider::{
    history::{Content, History, MessageContent},
    llm::{
        AgentSettings, CompletionModelHandle, DynamicCompletionModel, LLMCallback,
        ModelConstructor, ModelLogic, UsageReportingMode,
    },
    provider::is_hosted_provider_name,
    response::Response,
};
use flow_like_types::{
    Result, Value, anyhow, bail,
    base64::{Engine as _, engine::general_purpose::STANDARD},
    tokio::time::{Instant, timeout_at},
};
use futures::{StreamExt, stream};
use rig::message::{AudioMediaType, DocumentMediaType, MimeType, UserContent};
use std::{
    borrow::Cow,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use url::Url;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CACHE_BYTES: u64 = 64 * MIB;
const FAILURE_TTL: Duration = Duration::from_secs(60);
const FAILURE_ENTRIES: u64 = 1024;
const CONCURRENT_FETCHES: usize = 4;
/// Anthropic's Messages API limit, the smallest request size among the providers that read
/// inline PDFs.
const SMALLEST_REQUEST_BYTES: u64 = 32_000_000;

#[derive(Clone, Copy, Debug)]
struct InlinePolicy {
    /// Raw PDF and audio bytes per call. They travel as base64, which must fit the smallest
    /// provider request.
    binary_bytes: u64,
    /// Text document bytes per call. The model reads them as tokens, so this is sized by the
    /// context window.
    text_bytes: u64,
    /// Media parts considered per call, newest first.
    max_parts: usize,
    fetch_timeout: Duration,
    /// For all downloads of one call together.
    deadline: Duration,
    allow_http: bool,
}

const DEFAULT_POLICY: InlinePolicy = InlinePolicy {
    binary_bytes: 20 * MIB,
    text_bytes: 256 * KIB,
    max_parts: 10,
    fetch_timeout: Duration::from_secs(20),
    deadline: Duration::from_secs(30),
    allow_http: false,
};

const _: () = assert!(DEFAULT_POLICY.binary_bytes.div_ceil(3) * 4 < SMALLEST_REQUEST_BYTES);

impl InlinePolicy {
    fn fetches(&self, url: &str) -> bool {
        let scheme_allowed =
            has_scheme(url, "https://") || (self.allow_http && has_scheme(url, "http://"));
        scheme_allowed && Url::parse(url).is_ok()
    }

    fn cap(&self, class: MediaClass) -> u64 {
        match class {
            MediaClass::Text => self.text_bytes,
            MediaClass::Binary => self.binary_bytes,
        }
    }
}

fn has_scheme(url: &str, scheme: &str) -> bool {
    url.get(..scheme.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
}

fn url_host(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaClass {
    Text,
    Binary,
}

impl MediaClass {
    fn label(self) -> &'static str {
        match self {
            Self::Text => "text documents",
            Self::Binary => "PDFs and audio",
        }
    }
}

/// What a provider's request converter does with inlined media. PDFs and audio are inlined only
/// for readers that send them on as media; the other converters paste a base64 document into
/// the prompt as text or drop it. Text documents are inlined for every reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaReader {
    TextOnly,
    /// Anthropic Messages.
    Pdf,
    /// OpenAI Chat Completions and Responses, whose `input_audio` takes only WAV and MP3.
    PdfWavMp3,
    /// Gemini, Vertex and OpenRouter.
    PdfAnyAudio,
}

impl MediaReader {
    /// The reader of the Rig client that `ModelFactory` builds for `provider_name`.
    pub fn for_provider(provider_name: &str) -> Self {
        let provider = provider_name.trim().to_ascii_lowercase();
        let client = if is_hosted_provider_name(&provider) {
            provider.strip_prefix("hosted:").unwrap_or("openrouter")
        } else {
            provider.strip_prefix("custom:").unwrap_or(&provider)
        };
        match client {
            "anthropic" => Self::Pdf,
            "openai" | "azure" | "bedrock" => Self::PdfWavMp3,
            "gemini" | "vertex" | "openrouter" => Self::PdfAnyAudio,
            _ => Self::TextOnly,
        }
    }

    fn reads_pdf(self) -> bool {
        self != Self::TextOnly
    }

    fn reads_audio(self, media_type: &AudioMediaType) -> bool {
        match self {
            Self::PdfAnyAudio => true,
            Self::PdfWavMp3 => matches!(media_type, AudioMediaType::WAV | AudioMediaType::MP3),
            Self::TextOnly | Self::Pdf => false,
        }
    }
}

struct InlinedMedia {
    data_url: String,
    bytes: u64,
}

impl InlinedMedia {
    fn binary(mime: &str, body: &[u8]) -> Self {
        Self::encode(format!("data:{mime};base64,"), body)
    }

    /// Marked as a string source, so history conversion yields a text document.
    fn text(mime: &str, body: &[u8]) -> Result<Self> {
        let text = decode_text(body)?;
        Ok(Self::encode(
            format!("data:{mime};flow-like-source=string;base64,"),
            text.as_bytes(),
        ))
    }

    fn encode(mut data_url: String, payload: &[u8]) -> Self {
        data_url.reserve(payload.len().div_ceil(3) * 4);
        STANDARD.encode_string(payload, &mut data_url);
        Self {
            data_url,
            bytes: payload.len() as u64,
        }
    }
}

/// UTF-16 by its byte order mark, else UTF-8, else Windows-1252, the encoding of legacy Windows
/// exports such as Excel's CSV. A body with NUL bytes is not text.
fn decode_text(body: &[u8]) -> Result<Cow<'_, str>> {
    if let Some(units) = body.strip_prefix(b"\xFF\xFE") {
        return Ok(decode_utf16(units, u16::from_le_bytes).into());
    }
    if let Some(units) = body.strip_prefix(b"\xFE\xFF") {
        return Ok(decode_utf16(units, u16::from_be_bytes).into());
    }
    let body = body.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(body);
    if body.contains(&0) {
        bail!("the body contains NUL bytes, so it is not text");
    }
    Ok(match std::str::from_utf8(body) {
        Ok(text) => Cow::Borrowed(text),
        Err(_) => Cow::Owned(body.iter().copied().map(windows_1252).collect()),
    })
}

fn decode_utf16(units: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    char::decode_utf16(units.chunks_exact(2).map(|pair| unit([pair[0], pair[1]])))
        .map(|decoded| decoded.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

const WINDOWS_1252_C1: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}', '\u{178}',
];

fn windows_1252(byte: u8) -> char {
    match byte {
        0x80..=0x9F => WINDOWS_1252_C1[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

/// Keyed by environment, MIME type and URL, so a tool loop that invokes the model again with the
/// same History does not download its attachments again.
type CacheKey = (&'static str, &'static str, String);

static MEDIA_CACHE: LazyLock<moka::sync::Cache<CacheKey, Arc<InlinedMedia>>> =
    LazyLock::new(|| {
        moka::sync::Cache::builder()
            .max_capacity(CACHE_BYTES)
            .weigher(|_, media: &Arc<InlinedMedia>| {
                u32::try_from(media.data_url.len()).unwrap_or(u32::MAX)
            })
            .time_to_live(CACHE_TTL)
            .build()
    });

/// Failed downloads by the same key, so a tool loop does not wait on a dead URL every call.
static FAILED_FETCHES: LazyLock<moka::sync::Cache<CacheKey, Arc<str>>> = LazyLock::new(|| {
    moka::sync::Cache::builder()
        .max_capacity(FAILURE_ENTRIES)
        .time_to_live(FAILURE_TTL)
        .build()
});

/// Inlines remote PDF, text and audio parts as base64 before `invoke`, since most providers
/// refuse document and audio URLs. Images and video stay URLs. Downloads pass the egress guard
/// of `environment`; a part that cannot be fetched keeps its URL.
pub struct RemoteMediaModel {
    inner: Arc<dyn ModelLogic>,
    environment: ExecutionEnvironment,
    reader: MediaReader,
    policy: InlinePolicy,
}

impl RemoteMediaModel {
    pub fn new(
        inner: Arc<dyn ModelLogic>,
        environment: ExecutionEnvironment,
        reader: MediaReader,
    ) -> Self {
        Self {
            inner,
            environment,
            reader,
            policy: DEFAULT_POLICY,
        }
    }
}

#[flow_like_types::async_trait]
impl ModelLogic for RemoteMediaModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        self.inner.provider().await
    }

    async fn default_model(&self) -> Option<String> {
        self.inner.default_model().await
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        self.inner.additional_params(history)
    }

    fn usage_reporting(&self) -> UsageReportingMode {
        self.inner.usage_reporting()
    }

    fn transform_history(&self, history: &mut History) {
        self.inner.transform_history(history);
    }

    fn request_params(&self, history: &History, is_streaming: bool) -> Result<Option<Value>> {
        self.inner.request_params(history, is_streaming)
    }

    fn agent_settings(&self, history: &Option<History>) -> Result<AgentSettings> {
        self.inner.agent_settings(history)
    }

    async fn dynamic_completion_model(
        &self,
        model_name: Option<&str>,
    ) -> Result<DynamicCompletionModel> {
        self.inner.dynamic_completion_model(model_name).await
    }

    #[allow(deprecated)]
    async fn completion_model_handle(
        &self,
        model_name: Option<&str>,
    ) -> Result<CompletionModelHandle<'static>> {
        self.inner.completion_model_handle(model_name).await
    }

    async fn invoke(&self, history: &History, lambda: Option<LLMCallback>) -> Result<Response> {
        match inline_remote_media(history, self.environment, self.policy, self.reader).await {
            Some(resolved) => self.inner.invoke(&resolved, lambda).await,
            None => self.inner.invoke(history, lambda).await,
        }
    }
}

struct Candidate<'a> {
    message: usize,
    part: usize,
    url: &'a str,
    mime: &'static str,
    class: MediaClass,
}

/// How a part is inlined, if at all: the canonical MIME type of the Rig media type that history
/// conversion resolves from the declared type or the URL extension, and its budget.
fn inline_as(
    content: &Content,
    policy: &InlinePolicy,
    reader: MediaReader,
) -> Option<(&'static str, MediaClass)> {
    let (Content::Document {
        document_url: url, ..
    }
    | Content::Audio { audio_url: url, .. }) = content
    else {
        return None;
    };
    if !policy.fetches(url) {
        return None;
    }
    match UserContent::from(content.clone()) {
        UserContent::Document(document) => match document.media_type? {
            DocumentMediaType::PDF => reader
                .reads_pdf()
                .then(|| (DocumentMediaType::PDF.to_mime_type(), MediaClass::Binary)),
            text => Some((text.to_mime_type(), MediaClass::Text)),
        },
        UserContent::Audio(audio) => {
            let media_type = audio.media_type?;
            reader
                .reads_audio(&media_type)
                .then(|| (media_type.to_mime_type(), MediaClass::Binary))
        }
        _ => None,
    }
}

fn candidates<'a>(
    history: &'a History,
    policy: &InlinePolicy,
    reader: MediaReader,
) -> Vec<Candidate<'a>> {
    history
        .messages
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(message, entry)| match &entry.content {
            MessageContent::Contents(parts) => Some((message, parts)),
            MessageContent::String(_) => None,
        })
        .flat_map(|(message, parts)| {
            parts.iter().enumerate().filter_map(move |(part, content)| {
                let (mime, class) = inline_as(content, policy, reader)?;
                Some(Candidate {
                    message,
                    part,
                    url: content.media_url()?,
                    mime,
                    class,
                })
            })
        })
        .take(policy.max_parts)
        .collect()
}

fn media_url_mut<'a>(history: &'a mut History, candidate: &Candidate) -> Option<&'a mut String> {
    let MessageContent::Contents(parts) = &mut history.messages.get_mut(candidate.message)?.content
    else {
        return None;
    };
    match parts.get_mut(candidate.part)? {
        Content::Document {
            document_url: url, ..
        }
        | Content::Audio { audio_url: url, .. } => Some(url),
        _ => None,
    }
}

/// A copy of `history` with its remote PDF, text and audio parts inlined, or `None` when nothing
/// was inlined. Parts download concurrently; the budget goes to the newest message first.
async fn inline_remote_media(
    history: &History,
    environment: ExecutionEnvironment,
    policy: InlinePolicy,
    reader: MediaReader,
) -> Option<History> {
    let candidates = candidates(history, &policy, reader);
    if candidates.is_empty() {
        return None;
    }
    let fetcher = match GuardedHttpClient::new(environment) {
        Ok(client) => MediaFetcher::new(client, environment, policy),
        Err(error) => {
            tracing::warn!(%error, "Keeping remote media as URLs: building the HTTP client failed");
            return None;
        }
    };
    let pending: Vec<_> = candidates
        .iter()
        .map(|candidate| fetcher.fetch(candidate))
        .collect();
    let mut fetches = stream::iter(pending).buffered(CONCURRENT_FETCHES);
    let mut resolved = history.clone();
    let mut inlined = false;
    for candidate in &candidates {
        let Some(fetched) = fetches.next().await else {
            break;
        };
        match fetched.and_then(|media| fetcher.spend(candidate.class, &media).map(|()| media)) {
            Ok(media) => {
                if let Some(url) = media_url_mut(&mut resolved, candidate) {
                    *url = media.data_url.clone();
                    inlined = true;
                }
            }
            Err(error) => tracing::warn!(
                host = %url_host(candidate.url),
                mime = candidate.mime,
                %error,
                "Keeping remote media as a URL for the model call"
            ),
        }
    }
    inlined.then_some(resolved)
}

struct MediaFetcher {
    client: GuardedHttpClient,
    environment: ExecutionEnvironment,
    policy: InlinePolicy,
    deadline: Instant,
    text_left: AtomicU64,
    binary_left: AtomicU64,
}

impl MediaFetcher {
    fn new(
        client: GuardedHttpClient,
        environment: ExecutionEnvironment,
        policy: InlinePolicy,
    ) -> Self {
        Self {
            client,
            environment,
            policy,
            deadline: Instant::now() + policy.deadline,
            text_left: AtomicU64::new(policy.text_bytes),
            binary_left: AtomicU64::new(policy.binary_bytes),
        }
    }

    fn left(&self, class: MediaClass) -> &AtomicU64 {
        match class {
            MediaClass::Text => &self.text_left,
            MediaClass::Binary => &self.binary_left,
        }
    }

    fn spend(&self, class: MediaClass, media: &InlinedMedia) -> Result<()> {
        self.left(class)
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(media.bytes)
            })
            .map(|_| ())
            .map_err(|left| {
                anyhow!(
                    "{} bytes exceed the {left} bytes left of this call's inline budget for {}",
                    media.bytes,
                    class.label()
                )
            })
    }

    /// Downloads a part, or takes it from the caches. A failure is cached unless the call's
    /// deadline or budget caused it.
    async fn fetch(&self, candidate: &Candidate<'_>) -> Result<Arc<InlinedMedia>> {
        let key = (
            self.environment.as_str(),
            candidate.mime,
            candidate.url.to_owned(),
        );
        if let Some(media) = MEDIA_CACHE.get(&key) {
            return Ok(media);
        }
        if let Some(error) = FAILED_FETCHES.get(&key) {
            bail!("{error} (a failure cached for {:?})", FAILURE_TTL);
        }
        if self.left(candidate.class).load(Ordering::Relaxed) == 0 {
            bail!(
                "this call's inline budget for {} is used up",
                candidate.class.label()
            );
        }
        let own_deadline = Instant::now() + self.policy.fetch_timeout;
        let until = own_deadline.min(self.deadline);
        let downloaded = match timeout_at(until, self.download(candidate)).await {
            Ok(downloaded) => downloaded,
            Err(_) if until < own_deadline => bail!(
                "the {:?} inline deadline of this call passed",
                self.policy.deadline
            ),
            Err(_) => Err(anyhow!(
                "download did not finish within {:?}",
                self.policy.fetch_timeout
            )),
        };
        match downloaded {
            Ok(media) => {
                let media = Arc::new(media);
                MEDIA_CACHE.insert(key, media.clone());
                Ok(media)
            }
            Err(error) => {
                FAILED_FETCHES.insert(key, Arc::from(error.to_string()));
                Err(error)
            }
        }
    }

    async fn download(&self, candidate: &Candidate<'_>) -> Result<InlinedMedia> {
        let limit = self.policy.cap(candidate.class);
        let mut response = self
            .client
            .get(candidate.url)?
            .send()
            .await
            .map_err(|error| anyhow!("GET failed: {}", error.without_url()))?;
        let origin = response.url();
        if !self.policy.fetches(origin.as_str()) {
            bail!(
                "GET ended on a {} URL on host '{}'; only https responses are inlined",
                origin.scheme(),
                origin.host_str().unwrap_or_default()
            );
        }
        let status = response.status();
        if !status.is_success() {
            bail!("GET returned HTTP {status}");
        }
        if let Some(length) = response.content_length()
            && length > limit
        {
            bail!("response of {length} bytes exceeds the inline limit of {limit} bytes");
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| anyhow!("reading the response body failed: {}", error.without_url()))?
        {
            if (body.len() + chunk.len()) as u64 > limit {
                bail!("response body exceeds the inline limit of {limit} bytes");
            }
            body.extend_from_slice(&chunk);
        }
        match candidate.class {
            MediaClass::Text => InlinedMedia::text(candidate.mime, &body),
            MediaClass::Binary => Ok(InlinedMedia::binary(candidate.mime, &body)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_model_provider::history::{ContentType, HistoryMessage, ImageUrl, Role};
    use rig::completion::Message as RigMessage;
    use rig::message::DocumentSourceKind;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const TEST_POLICY: InlinePolicy = InlinePolicy {
        binary_bytes: 1024,
        text_bytes: 64,
        max_parts: 10,
        fetch_timeout: Duration::from_secs(5),
        deadline: Duration::from_secs(10),
        allow_http: true,
    };
    const PDF: &[u8] = b"%PDF-1.7 flow-like test document";
    const LARGE: &[u8] = &[b'%'; 2048];
    const MISSING: Reply = Reply::Body {
        status: 404,
        body: b"missing",
        content_length: true,
    };

    enum Reply {
        Body {
            status: u16,
            body: &'static [u8],
            content_length: bool,
        },
        Redirect(&'static str),
        Stall,
    }

    impl Reply {
        fn ok(body: &'static [u8]) -> Self {
            Self::Body {
                status: 200,
                body,
                content_length: true,
            }
        }
    }

    struct TestServer {
        base: String,
        hits: Arc<AtomicUsize>,
    }

    impl TestServer {
        fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    async fn serve(routes: Vec<(&'static str, Reply)>) -> TestServer {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind media server");
        let base = format!("http://{}", listener.local_addr().expect("server address"));
        let hits = Arc::new(AtomicUsize::new(0));
        let routes: Arc<HashMap<_, _>> = Arc::new(routes.into_iter().collect());
        let counter = hits.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (routes, counter) = (routes.clone(), counter.clone());
                tokio::spawn(async move {
                    let (mut buffer, mut request) = ([0_u8; 1024], Vec::new());
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        match socket.read(&mut buffer).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&buffer[..read]),
                        }
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    let head = String::from_utf8_lossy(&request);
                    let target = head.split_whitespace().nth(1).unwrap_or_default();
                    let path = target.split('?').next().unwrap_or_default();
                    let (head, body): (String, &[u8]) = match routes.get(path).unwrap_or(&MISSING) {
                        Reply::Stall => return std::future::pending::<()>().await,
                        Reply::Redirect(location) => (
                            format!(
                                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n"
                            ),
                            b"",
                        ),
                        Reply::Body {
                            status,
                            body,
                            content_length,
                        } => {
                            let mut head = format!("HTTP/1.1 {status} Test\r\n");
                            if *content_length {
                                head.push_str(&format!("Content-Length: {}\r\n", body.len()));
                            }
                            (head, *body)
                        }
                    };
                    let head = format!("{head}Connection: close\r\n\r\n");
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(body).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        TestServer { base, hits }
    }

    fn text(text: &str) -> Content {
        Content::Text {
            content_type: ContentType::Text,
            text: text.to_string(),
        }
    }

    fn document(url: String, media_type: Option<&str>) -> Content {
        Content::Document {
            content_type: ContentType::DocumentUrl,
            document_url: url,
            media_type: media_type.map(ToOwned::to_owned),
            additional_params: None,
        }
    }

    fn audio(url: String, media_type: &str) -> Content {
        Content::Audio {
            content_type: ContentType::AudioUrl,
            audio_url: url,
            media_type: Some(media_type.to_string()),
            additional_params: None,
        }
    }

    fn user(parts: Vec<Content>) -> HistoryMessage {
        HistoryMessage {
            role: Role::User,
            content: MessageContent::Contents(parts),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            annotations: None,
        }
    }

    fn history(messages: Vec<HistoryMessage>) -> History {
        History::new("test-model".to_string(), messages)
    }

    fn url_at(history: &History, message: usize, part: usize) -> &str {
        let MessageContent::Contents(parts) = &history.messages[message].content else {
            panic!("message {message} has no content parts");
        };
        parts[part].media_url().expect("media part")
    }

    fn prompt_contents(history: &History) -> Vec<UserContent> {
        let (prompt, _) = history
            .extract_prompt_and_history()
            .expect("convert history into rig messages");
        let RigMessage::User { content } = prompt else {
            panic!("the prompt is a user message");
        };
        content.into_iter().collect()
    }

    fn assert_text_document(content: &UserContent, expected: &str, media_type: DocumentMediaType) {
        let UserContent::Document(document) = content else {
            panic!("expected a document, got {content:?}");
        };
        assert_eq!(document.data, DocumentSourceKind::String(expected.into()));
        assert_eq!(document.media_type, Some(media_type));
    }

    async fn inline(
        original: &History,
        policy: InlinePolicy,
        reader: MediaReader,
    ) -> Option<History> {
        inline_remote_media(original, ExecutionEnvironment::Local, policy, reader).await
    }

    #[tokio::test]
    async fn pdf_and_audio_parts_become_base64_sources() {
        let server = serve(vec![
            ("/report.pdf", Reply::ok(PDF)),
            ("/scan.pdf", Reply::ok(PDF)),
            ("/voice.mp3", Reply::ok(b"ID3 audio")),
        ])
        .await;
        let report = server.url("/report.pdf");
        let original = history(vec![user(vec![
            text("summarise these"),
            document(report.clone(), Some("application/pdf")),
            document(
                server.url("/scan.pdf?X-Amz-Signature=secret"),
                Some("binary/octet-stream"),
            ),
            audio(server.url("/voice.mp3"), "audio/mpeg"),
        ])]);

        let resolved = inline(&original, TEST_POLICY, MediaReader::PdfAnyAudio)
            .await
            .expect("remote media is inlined");

        let pdf = STANDARD.encode(PDF);
        assert_eq!(url_at(&original, 0, 1), report);
        assert_eq!(
            url_at(&resolved, 0, 1),
            format!("data:application/pdf;base64,{pdf}")
        );
        assert_eq!(
            url_at(&resolved, 0, 2),
            format!("data:application/pdf;base64,{pdf}"),
            "a PDF known only by its URL extension keeps its type"
        );
        let contents = prompt_contents(&resolved);
        for content in &contents[1..3] {
            let UserContent::Document(document) = content else {
                panic!("expected a document, got {content:?}");
            };
            assert_eq!(document.data, DocumentSourceKind::Base64(pdf.clone()));
            assert_eq!(document.media_type, Some(DocumentMediaType::PDF));
        }
        let UserContent::Audio(audio) = &contents[3] else {
            panic!("expected audio, got {:?}", contents[3]);
        };
        assert_eq!(
            audio.data,
            DocumentSourceKind::Base64(STANDARD.encode(b"ID3 audio"))
        );
        assert_eq!(audio.media_type, Some(AudioMediaType::MP3));
    }

    #[tokio::test]
    async fn textual_documents_become_string_sources() {
        let server = serve(vec![
            ("/table.csv", Reply::ok(b"a,b\n1,2\n")),
            ("/notes.txt", Reply::ok(b"plain notes")),
        ])
        .await;
        let original = history(vec![user(vec![
            text("read these"),
            document(server.url("/table.csv"), Some("text/csv")),
            document(server.url("/notes.txt"), None),
        ])]);

        let resolved = inline(&original, TEST_POLICY, MediaReader::TextOnly)
            .await
            .expect("remote media is inlined");

        let contents = prompt_contents(&resolved);
        for (content, expected, media_type) in [
            (&contents[1], "a,b\n1,2\n", DocumentMediaType::CSV),
            (&contents[2], "plain notes", DocumentMediaType::TXT),
        ] {
            assert_text_document(content, expected, media_type);
        }
    }

    #[tokio::test]
    async fn legacy_encoded_text_is_decoded_and_binary_text_keeps_its_url() {
        let server = serve(vec![
            ("/export.csv", Reply::ok(b"Name;Betrag\nM\xFCller;12\x80\n")),
            ("/notepad.txt", Reply::ok(b"\xFF\xFEh\0i\0")),
            ("/archive.txt", Reply::ok(b"PK\x03\x04\0\0zip")),
        ])
        .await;
        let archive = server.url("/archive.txt");
        let original = history(vec![user(vec![
            text("read these"),
            document(server.url("/export.csv"), Some("text/csv")),
            document(server.url("/notepad.txt"), None),
            document(archive.clone(), None),
        ])]);

        let resolved = inline(&original, TEST_POLICY, MediaReader::PdfWavMp3)
            .await
            .expect("the text documents are inlined");

        let contents = prompt_contents(&resolved);
        assert_text_document(
            &contents[1],
            "Name;Betrag\nMüller;12€\n",
            DocumentMediaType::CSV,
        );
        assert_text_document(&contents[2], "hi", DocumentMediaType::TXT);
        assert_eq!(url_at(&resolved, 0, 3), archive);
    }

    #[tokio::test]
    async fn text_beyond_the_text_budget_keeps_its_url() {
        let server = serve(vec![
            ("/server.log", Reply::ok(LARGE)),
            ("/notes.txt", Reply::ok(b"small")),
        ])
        .await;
        let log = server.url("/server.log");
        let original = history(vec![user(vec![
            text("why did it fail?"),
            document(log.clone(), Some("text/plain")),
            document(server.url("/notes.txt"), None),
        ])]);

        let resolved = inline(&original, TEST_POLICY, MediaReader::PdfAnyAudio)
            .await
            .expect("the small text document is inlined");

        assert_eq!(url_at(&resolved, 0, 1), log);
        assert_text_document(
            &prompt_contents(&resolved)[2],
            "small",
            DocumentMediaType::TXT,
        );
    }

    #[tokio::test]
    async fn readers_inline_only_the_media_their_provider_reads() {
        let server = serve(vec![
            ("/report.pdf", Reply::ok(PDF)),
            ("/voice.mp3", Reply::ok(b"ID3 audio")),
            ("/memo.m4a", Reply::ok(b"m4a audio")),
            ("/table.csv", Reply::ok(b"a,b\n")),
        ])
        .await;
        let original = history(vec![user(vec![
            text("read"),
            document(server.url("/report.pdf"), Some("application/pdf")),
            audio(server.url("/voice.mp3"), "audio/mpeg"),
            audio(server.url("/memo.m4a"), "audio/mp4"),
            document(server.url("/table.csv"), Some("text/csv")),
        ])]);

        for (reader, expected) in [
            (MediaReader::TextOnly, [false, false, false, true]),
            (MediaReader::Pdf, [true, false, false, true]),
            (MediaReader::PdfWavMp3, [true, true, false, true]),
            (MediaReader::PdfAnyAudio, [true, true, true, true]),
        ] {
            let resolved = inline(&original, TEST_POLICY, reader)
                .await
                .expect("the CSV is inlined for every reader");
            let inlined = [1, 2, 3, 4].map(|part| url_at(&resolved, 0, part).starts_with("data:"));
            assert_eq!(inlined, expected, "{reader:?}");
        }
    }

    #[test]
    fn provider_names_map_to_the_reader_of_their_client() {
        for (provider, reader) in [
            ("openai", MediaReader::PdfWavMp3),
            (" Azure ", MediaReader::PdfWavMp3),
            ("custom:bedrock", MediaReader::PdfWavMp3),
            ("hosted:openai", MediaReader::PdfWavMp3),
            ("hosted:bedrock", MediaReader::PdfWavMp3),
            ("custom:anthropic", MediaReader::Pdf),
            ("hosted", MediaReader::PdfAnyAudio),
            ("hosted:openrouter", MediaReader::PdfAnyAudio),
            ("gemini", MediaReader::PdfAnyAudio),
            ("custom:vertex", MediaReader::PdfAnyAudio),
            ("deepseek", MediaReader::TextOnly),
            ("custom:ollama", MediaReader::TextOnly),
            ("huggingface", MediaReader::TextOnly),
            ("local", MediaReader::TextOnly),
        ] {
            assert_eq!(MediaReader::for_provider(provider), reader, "{provider}");
        }
    }

    #[tokio::test]
    async fn images_video_and_unmapped_documents_are_left_alone() {
        let server = serve(vec![]).await;
        let original = history(vec![user(vec![
            text("look"),
            Content::Image {
                content_type: ContentType::ImageUrl,
                image_url: ImageUrl {
                    url: server.url("/photo.png"),
                    detail: None,
                    media_type: Some("image/png".to_string()),
                    additional_params: None,
                },
            },
            Content::Video {
                content_type: ContentType::VideoUrl,
                video_url: server.url("/clip.mp4"),
                media_type: Some("video/mp4".to_string()),
                additional_params: None,
            },
            document(
                server.url("/letter.docx"),
                Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
            ),
            document(server.url("/blob"), Some("application/octet-stream")),
            document(
                format!("data:application/pdf;base64,{}", STANDARD.encode(PDF)),
                Some("application/pdf"),
            ),
        ])]);

        let resolved = inline(&original, TEST_POLICY, MediaReader::PdfAnyAudio).await;

        assert!(resolved.is_none());
        assert_eq!(server.hits(), 0);
    }

    #[tokio::test]
    async fn failed_downloads_keep_the_url() {
        let server = serve(vec![
            ("/ok.pdf", Reply::ok(PDF)),
            ("/large.pdf", Reply::ok(LARGE)),
            (
                "/streamed.pdf",
                Reply::Body {
                    status: 200,
                    body: LARGE,
                    content_length: false,
                },
            ),
        ])
        .await;
        let failing = [
            server.url("/missing.pdf"),
            server.url("/large.pdf"),
            server.url("/streamed.pdf"),
        ];
        let mut parts = vec![text("read"), document(server.url("/ok.pdf"), None)];
        parts.extend(failing.iter().map(|url| document(url.clone(), None)));
        let original = history(vec![user(parts)]);

        let resolved = inline(&original, TEST_POLICY, MediaReader::Pdf)
            .await
            .expect("the reachable document is inlined");

        assert!(url_at(&resolved, 0, 1).starts_with("data:application/pdf;base64,"));
        for (index, url) in failing.iter().enumerate() {
            assert_eq!(url_at(&resolved, 0, index + 2), url);
        }
    }

    #[tokio::test]
    async fn per_call_budget_keeps_older_parts_as_urls() {
        let server = serve(vec![
            ("/old.pdf", Reply::ok(PDF)),
            ("/new.pdf", Reply::ok(PDF)),
        ])
        .await;
        let policy = InlinePolicy {
            binary_bytes: PDF.len() as u64 + 8,
            ..TEST_POLICY
        };
        let old = server.url("/old.pdf");
        let original = history(vec![
            user(vec![text("first"), document(old.clone(), None)]),
            HistoryMessage::from_string(Role::Assistant, "noted"),
            user(vec![text("second"), document(server.url("/new.pdf"), None)]),
        ]);

        let resolved = inline(&original, policy, MediaReader::Pdf)
            .await
            .expect("the newest document is inlined");

        assert!(url_at(&resolved, 2, 1).starts_with("data:application/pdf;base64,"));
        assert_eq!(url_at(&resolved, 0, 1), old);
    }

    #[tokio::test]
    async fn failures_are_cached_and_parts_per_call_are_capped() {
        let server = serve(vec![
            ("/ok.pdf", Reply::ok(PDF)),
            ("/stalled.pdf", Reply::Stall),
        ])
        .await;
        let policy = InlinePolicy {
            max_parts: 3,
            fetch_timeout: Duration::from_millis(200),
            ..TEST_POLICY
        };
        let oldest = server.url("/ok.pdf");
        let failing = [server.url("/missing.pdf"), server.url("/stalled.pdf")];
        let original = history(vec![
            user(vec![text("first"), document(oldest.clone(), None)]),
            HistoryMessage::from_string(Role::Assistant, "noted"),
            user(vec![
                text("second"),
                document(failing[0].clone(), None),
                document(failing[1].clone(), None),
                document(server.url("/ok.pdf?newest"), None),
            ]),
        ]);

        for call in 0..2 {
            let resolved = inline(&original, policy, MediaReader::Pdf)
                .await
                .expect("the newest document is inlined");
            assert_eq!(url_at(&resolved, 0, 1), oldest, "beyond the part cap");
            assert_eq!(url_at(&resolved, 2, 1), failing[0]);
            assert_eq!(url_at(&resolved, 2, 2), failing[1]);
            assert!(url_at(&resolved, 2, 3).starts_with("data:application/pdf;base64,"));
            assert_eq!(server.hits(), 3, "call {call} requests nothing again");
        }
    }

    #[tokio::test]
    async fn the_call_deadline_ends_stalled_downloads() {
        let server = serve(vec![
            ("/stalled.pdf", Reply::Stall),
            ("/ok.pdf", Reply::ok(PDF)),
        ])
        .await;
        let policy = InlinePolicy {
            deadline: Duration::from_millis(300),
            ..TEST_POLICY
        };
        let stalled = server.url("/stalled.pdf");
        let original = history(vec![user(vec![
            text("read"),
            document(stalled.clone(), None),
            document(server.url("/ok.pdf"), None),
        ])]);

        let started = std::time::Instant::now();
        let resolved = inline(&original, policy, MediaReader::Pdf)
            .await
            .expect("the served document is inlined");

        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(url_at(&resolved, 0, 1), stalled);
        assert!(url_at(&resolved, 0, 2).starts_with("data:application/pdf;base64,"));
        inline(&original, policy, MediaReader::Pdf).await;
        assert_eq!(
            server.hits(),
            3,
            "a download the deadline cut short is not cached as failed"
        );
    }

    #[tokio::test]
    async fn responses_not_served_over_https_are_refused() {
        let server = serve(vec![
            ("/start.pdf", Reply::Redirect("/report.pdf")),
            ("/report.pdf", Reply::ok(PDF)),
        ])
        .await;
        let url = server.url("/start.pdf");
        let candidate = Candidate {
            message: 0,
            part: 0,
            url: &url,
            mime: "application/pdf",
            class: MediaClass::Binary,
        };
        let fetcher = |policy| {
            let environment = ExecutionEnvironment::Local;
            let client = GuardedHttpClient::new(environment).expect("HTTP client");
            MediaFetcher::new(client, environment, policy)
        };

        let followed = fetcher(TEST_POLICY)
            .download(&candidate)
            .await
            .expect("the redirect is followed");
        let refused = fetcher(DEFAULT_POLICY)
            .download(&candidate)
            .await
            .err()
            .expect("an http response is not inlined");

        assert_eq!(followed.bytes, PDF.len() as u64);
        assert!(refused.to_string().contains("only https"), "{refused}");
        assert_eq!(server.hits(), 4);
    }

    #[tokio::test]
    async fn server_environment_refuses_loopback_hosts() {
        let server = serve(vec![("/report.pdf", Reply::ok(PDF))]).await;
        let original = history(vec![user(vec![
            text("read"),
            document(server.url("/report.pdf"), Some("application/pdf")),
        ])]);

        let resolved = inline_remote_media(
            &original,
            ExecutionEnvironment::Server,
            TEST_POLICY,
            MediaReader::Pdf,
        )
        .await;

        assert!(resolved.is_none());
        assert_eq!(server.hits(), 0);
    }

    #[tokio::test]
    async fn plain_http_is_not_fetched_by_default() {
        let server = serve(vec![("/report.pdf", Reply::ok(PDF))]).await;
        let original = history(vec![user(vec![
            text("read"),
            document(server.url("/report.pdf"), Some("application/pdf")),
        ])]);

        let resolved = inline(&original, DEFAULT_POLICY, MediaReader::Pdf).await;

        assert!(resolved.is_none());
        assert_eq!(server.hits(), 0);
    }

    #[derive(Default)]
    struct RecordingModel {
        histories: Mutex<Vec<History>>,
    }

    #[flow_like_types::async_trait]
    impl ModelLogic for RecordingModel {
        async fn provider(&self) -> Result<ModelConstructor> {
            Err(anyhow!("the recording model has no provider"))
        }

        async fn default_model(&self) -> Option<String> {
            Some("recorded-model".to_string())
        }

        fn usage_reporting(&self) -> UsageReportingMode {
            UsageReportingMode::OpenRouterUsageInclude
        }

        async fn invoke(
            &self,
            history: &History,
            _lambda: Option<LLMCallback>,
        ) -> Result<Response> {
            self.histories
                .lock()
                .expect("history log")
                .push(history.clone());
            Ok(Response::default())
        }
    }

    #[tokio::test]
    async fn wrapper_delegates_and_invokes_with_inlined_media() {
        let server = serve(vec![("/report.pdf", Reply::ok(PDF))]).await;
        let recorder = Arc::new(RecordingModel::default());
        let model = RemoteMediaModel {
            inner: recorder.clone(),
            environment: ExecutionEnvironment::Local,
            reader: MediaReader::Pdf,
            policy: TEST_POLICY,
        };
        let with_media = history(vec![user(vec![
            text("read"),
            document(server.url("/report.pdf"), Some("application/pdf")),
        ])]);
        let plain = history(vec![HistoryMessage::from_string(Role::User, "hello")]);

        assert_eq!(
            model.default_model().await.as_deref(),
            Some("recorded-model")
        );
        assert_eq!(
            model.usage_reporting(),
            UsageReportingMode::OpenRouterUsageInclude
        );
        model.invoke(&with_media, None).await.expect("first invoke");
        model
            .invoke(&with_media, None)
            .await
            .expect("second invoke");
        model.invoke(&plain, None).await.expect("plain invoke");

        let histories = recorder.histories.lock().expect("history log");
        assert_eq!(histories.len(), 3);
        for received in &histories[..2] {
            assert_eq!(
                url_at(received, 0, 1),
                format!("data:application/pdf;base64,{}", STANDARD.encode(PDF))
            );
        }
        assert_eq!(histories[2].messages[0].content, plain.messages[0].content);
        assert_eq!(
            server.hits(),
            1,
            "the second invoke reuses the cached download"
        );
    }
}
