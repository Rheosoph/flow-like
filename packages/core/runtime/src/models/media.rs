use crate::flow::execution::{ExecutionEnvironment, egress::GuardedHttpClient};
use flow_like_model_provider::{
    history::{Content, History, MessageContent},
    llm::{
        AgentSettings, CompletionModelHandle, DynamicCompletionModel, LLMCallback,
        ModelConstructor, ModelLogic, UsageReportingMode,
    },
    response::Response,
};
use flow_like_types::{
    Result, Value, anyhow, bail,
    base64::{Engine as _, engine::general_purpose::STANDARD},
    tokio::time::timeout,
};
use rig::message::{MimeType, UserContent};
use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};
use url::Url;

const MIB: u64 = 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CACHE_BYTES: u64 = 64 * MIB;

#[derive(Clone, Copy, Debug)]
struct InlinePolicy {
    file_bytes: u64,
    total_bytes: u64,
    allow_http: bool,
}

const DEFAULT_POLICY: InlinePolicy = InlinePolicy {
    file_bytes: 25 * MIB,
    total_bytes: 40 * MIB,
    allow_http: false,
};

impl InlinePolicy {
    fn fetches(&self, url: &str) -> bool {
        let scheme_allowed =
            has_scheme(url, "https://") || (self.allow_http && has_scheme(url, "http://"));
        scheme_allowed && Url::parse(url).is_ok()
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

struct InlinedMedia {
    data_url: String,
    bytes: u64,
}

impl InlinedMedia {
    fn encode(mime: &str, body: &[u8]) -> Self {
        let mut data_url = format!("data:{mime};base64,");
        data_url.reserve(body.len().div_ceil(3) * 4);
        STANDARD.encode_string(body, &mut data_url);
        Self {
            data_url,
            bytes: body.len() as u64,
        }
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

/// Inlines remote PDF, text and audio parts as base64 before `invoke`, since most providers
/// refuse document and audio URLs. Images and video stay URLs. Downloads pass the egress guard
/// of `environment`; a part that cannot be fetched keeps its URL.
pub struct RemoteMediaModel {
    inner: Arc<dyn ModelLogic>,
    environment: ExecutionEnvironment,
    policy: InlinePolicy,
}

impl RemoteMediaModel {
    pub fn new(inner: Arc<dyn ModelLogic>, environment: ExecutionEnvironment) -> Self {
        Self {
            inner,
            environment,
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
        match inline_remote_media(history, self.environment, self.policy).await {
            Some(resolved) => self.inner.invoke(&resolved, lambda).await,
            None => self.inner.invoke(history, lambda).await,
        }
    }
}

struct Candidate {
    message: usize,
    part: usize,
    mime: &'static str,
}

/// The MIME type a part is inlined as: the canonical one of the Rig media type that history
/// conversion resolves from the declared type or the URL extension.
fn inline_mime(content: &Content, policy: &InlinePolicy) -> Option<&'static str> {
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
        UserContent::Document(document) => document.media_type.map(|media| media.to_mime_type()),
        UserContent::Audio(audio) => audio.media_type.map(|media| media.to_mime_type()),
        _ => None,
    }
}

fn candidates(history: &History, policy: &InlinePolicy) -> Vec<Candidate> {
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
                inline_mime(content, policy).map(|mime| Candidate {
                    message,
                    part,
                    mime,
                })
            })
        })
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

/// A copy of `history` with its remote PDF, text and audio parts inlined, newest message first,
/// or `None` when nothing was inlined.
async fn inline_remote_media(
    history: &History,
    environment: ExecutionEnvironment,
    policy: InlinePolicy,
) -> Option<History> {
    let candidates = candidates(history, &policy);
    if candidates.is_empty() {
        return None;
    }
    let client = match GuardedHttpClient::new(environment) {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, "Keeping remote media as URLs: building the HTTP client failed");
            return None;
        }
    };
    let mut fetcher = MediaFetcher {
        client,
        environment,
        policy,
        remaining: policy.total_bytes,
    };
    let mut resolved = history.clone();
    let mut inlined = false;
    for candidate in &candidates {
        let Some(url) = media_url_mut(&mut resolved, candidate) else {
            continue;
        };
        match fetcher.data_url(url, candidate.mime).await {
            Ok(data_url) => {
                *url = data_url;
                inlined = true;
            }
            Err(error) => tracing::warn!(
                host = %url_host(url),
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
    remaining: u64,
}

impl MediaFetcher {
    async fn data_url(&mut self, url: &str, mime: &'static str) -> Result<String> {
        let limit = self.policy.file_bytes.min(self.remaining);
        if limit == 0 {
            bail!(
                "the per-call inline budget of {} bytes is used up",
                self.policy.total_bytes
            );
        }
        let key = (self.environment.as_str(), mime, url.to_owned());
        let media = match MEDIA_CACHE.get(&key) {
            Some(media) => media,
            None => {
                let body = timeout(FETCH_TIMEOUT, download(&self.client, url, limit))
                    .await
                    .map_err(|_| {
                        anyhow!(
                            "download did not finish within {} s",
                            FETCH_TIMEOUT.as_secs()
                        )
                    })??;
                let media = Arc::new(InlinedMedia::encode(mime, &body));
                MEDIA_CACHE.insert(key, media.clone());
                media
            }
        };
        if media.bytes > limit {
            bail!(
                "{} bytes exceed the remaining inline limit of {limit} bytes",
                media.bytes
            );
        }
        self.remaining -= media.bytes;
        Ok(media.data_url.clone())
    }
}

async fn download(client: &GuardedHttpClient, url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut response = client
        .get(url)?
        .send()
        .await
        .map_err(|error| anyhow!("GET failed: {}", error.without_url()))?;
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
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_model_provider::history::{ContentType, HistoryMessage, ImageUrl, Role};
    use rig::completion::Message as RigMessage;
    use rig::message::{AudioMediaType, DocumentMediaType, DocumentSourceKind};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const TEST_POLICY: InlinePolicy = InlinePolicy {
        file_bytes: 1024,
        total_bytes: 4096,
        allow_http: true,
    };
    const PDF: &[u8] = b"%PDF-1.7 flow-like test document";
    const LARGE: &[u8] = &[b'%'; 2048];

    struct Reply {
        status: u16,
        body: &'static [u8],
        content_length: bool,
    }

    impl Reply {
        fn ok(body: &'static [u8]) -> Self {
            Self {
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
                    let missing = Reply {
                        status: 404,
                        body: b"missing",
                        content_length: true,
                    };
                    let reply = routes.get(path).unwrap_or(&missing);
                    let mut response =
                        format!("HTTP/1.1 {} Test\r\nConnection: close\r\n", reply.status);
                    if reply.content_length {
                        response.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
                    }
                    response.push_str("\r\n");
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.write_all(reply.body).await;
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

        let resolved = inline_remote_media(&original, ExecutionEnvironment::Local, TEST_POLICY)
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

        let resolved = inline_remote_media(&original, ExecutionEnvironment::Local, TEST_POLICY)
            .await
            .expect("remote media is inlined");

        let contents = prompt_contents(&resolved);
        for (content, expected, media_type) in [
            (&contents[1], "a,b\n1,2\n", DocumentMediaType::CSV),
            (&contents[2], "plain notes", DocumentMediaType::TXT),
        ] {
            let UserContent::Document(document) = content else {
                panic!("expected a document, got {content:?}");
            };
            assert_eq!(document.data, DocumentSourceKind::String(expected.into()));
            assert_eq!(document.media_type, Some(media_type));
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

        let resolved =
            inline_remote_media(&original, ExecutionEnvironment::Local, TEST_POLICY).await;

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
                Reply {
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

        let resolved = inline_remote_media(&original, ExecutionEnvironment::Local, TEST_POLICY)
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
            total_bytes: PDF.len() as u64 + 8,
            ..TEST_POLICY
        };
        let old = server.url("/old.pdf");
        let original = history(vec![
            user(vec![text("first"), document(old.clone(), None)]),
            HistoryMessage::from_string(Role::Assistant, "noted"),
            user(vec![text("second"), document(server.url("/new.pdf"), None)]),
        ]);

        let resolved = inline_remote_media(&original, ExecutionEnvironment::Local, policy)
            .await
            .expect("the newest document is inlined");

        assert!(url_at(&resolved, 2, 1).starts_with("data:application/pdf;base64,"));
        assert_eq!(url_at(&resolved, 0, 1), old);
    }

    #[tokio::test]
    async fn server_environment_refuses_loopback_hosts() {
        let server = serve(vec![("/report.pdf", Reply::ok(PDF))]).await;
        let original = history(vec![user(vec![
            text("read"),
            document(server.url("/report.pdf"), Some("application/pdf")),
        ])]);

        let resolved =
            inline_remote_media(&original, ExecutionEnvironment::Server, TEST_POLICY).await;

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

        let resolved =
            inline_remote_media(&original, ExecutionEnvironment::Local, DEFAULT_POLICY).await;

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
