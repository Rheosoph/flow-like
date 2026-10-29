//! Readies the media of a model call for its provider. [`MediaReader`] says, for each kind of
//! media, whether the provider fetches links itself, reads it only inline, or cannot read it at
//! all. A link the provider fetches stays a link when its host is public HTTPS; any other link to
//! media the provider reads is downloaded and sent inline, as base64 or, for a text document, as
//! its text. Media the provider cannot read is left to its client, which notes it. Media that has
//! to travel inline but cannot, because its download fails or it is over the provider's inline
//! caps, fails the call with an error naming its kind, MIME type and host, never its link.
//!
//! A [`MediaSource`] downloads the media and decides links. A direct call fetches through the
//! egress guard of its execution environment, plain HTTP too except in a browser sandbox, and
//! only on the user's own machine does it read desktop asset links, to files in the app's storage
//! alone. A server also fetches the links into [`OwnStorage`], the object store its storage
//! credentials name, even when that store is internal or serves plain HTTP: as they are, without
//! redirects, and never on the host plane unless the store runs on loopback. A provider that
//! fetches such a link itself receives it signed again to expire within minutes, or its media
//! inline when the store does not serve the fresh link.
//! The API's model relay fetches through its own guard and signs links into the platform's
//! storage again, so a relayed call reaches the provider with the media a direct call sends. It
//! fetches no host that is neither public nor its storage, so a run sends the media of such a
//! host inline itself.
//! `invoke` readies the History; an agent's requests are readied the same way on the Rig request.

use crate::flow::execution::{ExecutionEnvironment, egress::GuardedHttpClient};
use flow_like_model_provider::{
    egress::ProviderEgress,
    history::{Content, History, MessageContent},
    llm::{
        AgentSettings, CompletionClientDyn, CompletionModelDyn, CompletionModelHandle,
        DynamicResponse, DynamicStreamingResponse, LLMCallback, ModelConstructor, ModelLogic,
        UsageReportingMode, relay::MAX_REQUEST_BYTES,
    },
    provider::{ModelApiSurface, ModelProvider, is_hosted_provider_name},
    response::Response,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::{
    Result, Value, anyhow, bail,
    base64::{Engine as _, engine::general_purpose::STANDARD},
    egress::{is_host_plane_ip, is_host_plane_name},
    reqwest::{
        self, StatusCode,
        dns::{Addrs, Name, Resolve, Resolving},
        header::RANGE,
        redirect,
    },
    tokio::{
        sync::OnceCell,
        time::{Instant, timeout, timeout_at},
    },
};
use futures::{StreamExt, stream};
use rig::{
    agent::AgentBuilder,
    completion::{
        CompletionError, CompletionRequest, CompletionRequestBuilder, CompletionResponse, Message,
    },
    message::{DocumentMediaType, DocumentSourceKind, ImageMediaType, MimeType, UserContent},
    streaming::StreamingCompletionResponse,
    wasm_compat::WasmBoxedFuture,
};
use std::{
    borrow::Cow,
    fmt,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        Arc, LazyLock, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use url::Url;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const MB: u64 = 1_000_000;
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const CACHE_BYTES: u64 = 64 * MIB;
const FAILURE_TTL: Duration = Duration::from_secs(60);
const FAILURE_ENTRIES: u64 = 1024;
const CONCURRENT_FETCHES: usize = 4;
/// For a host's addresses to resolve when a call decides whether its provider can fetch a link.
const LINK_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// Object a store signs a link for to show where its links point. The link is never fetched.
const PROBE_OBJECT: &str = "link-scope-probe";
const PROBE_TTL: Duration = Duration::from_secs(60);
/// How long a link into the own store that a provider fetches itself lets it read the object.
const RESIGNED_TTL: Duration = Duration::from_secs(5 * 60);
/// For the own store to answer a one-byte read of a link signed again.
const RESIGNED_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// Cache key of an image that only its response names the type of.
const IMAGE_TYPED_BY_RESPONSE: &str = "image/*";
const PDF_TYPE: &str = "application/pdf";
const DOCX_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const XLSX_TYPE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const PPTX_TYPE: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
const DOC_TYPE: &str = "application/msword";

/// The image formats of the web, which OpenAI, Anthropic, Bedrock and most OpenAI-compatible APIs
/// read.
const WEB_IMAGES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];
const JPEG_PNG: &[&str] = &["image/jpeg", "image/png"];
const JPEG_PNG_WEBP: &[&str] = &["image/jpeg", "image/png", "image/webp"];
const JPEG_PNG_GIF: &[&str] = &["image/jpeg", "image/png", "image/gif"];
const MOONSHOT_IMAGES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/heic",
    "image/heif",
];
/// What OpenAI's `input_audio` takes.
const WAV_MP3: &[&str] = &["audio/wav", "audio/mp3"];
const OOXML: &[&str] = &[DOCX_TYPE, XLSX_TYPE, PPTX_TYPE];
const WORD: &[&str] = &[DOCX_TYPE, DOC_TYPE];

#[derive(Clone, Copy, Debug)]
struct InlinePolicy {
    /// Text document bytes per call. The model reads them as tokens, so this is sized by the
    /// context window.
    text_bytes: u64,
    /// Media parts inlined per call.
    max_parts: usize,
    fetch_timeout: Duration,
    /// For all downloads of one call together.
    deadline: Duration,
    allow_http: bool,
}

/// A direct call's, over HTTPS only. Where HTTP nodes fetch plain HTTP, so does a direct call.
const DIRECT_POLICY: InlinePolicy = InlinePolicy {
    text_bytes: 256 * KIB,
    max_parts: 32,
    fetch_timeout: Duration::from_secs(20),
    deadline: Duration::from_secs(30),
    allow_http: false,
};

/// The API's model relay, whose source fetches plain HTTP only from storage it owns.
const RESOLVING_POLICY: InlinePolicy = InlinePolicy {
    allow_http: true,
    ..DIRECT_POLICY
};

/// Whether runs in `environment` execute on the user's own machine, whose hosts and files are
/// the user's.
pub fn runs_on_users_machine(environment: ExecutionEnvironment) -> bool {
    matches!(
        environment,
        ExecutionEnvironment::Local | ExecutionEnvironment::Desktop | ExecutionEnvironment::Mobile
    )
}

impl InlinePolicy {
    /// What HTTP nodes in `environment` reach: on a server, every host off the host plane.
    fn direct(environment: ExecutionEnvironment) -> Self {
        Self {
            allow_http: environment != ExecutionEnvironment::BrowserSandbox,
            ..DIRECT_POLICY
        }
    }

    /// Tauri's `asset.localhost` serves the user's files to the desktop app's webview only; a
    /// source that reads them takes them from disk.
    fn fetches(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|url| {
            let scheme_allowed = match url.scheme() {
                "https" => true,
                "http" => self.allow_http,
                _ => false,
            };
            scheme_allowed && url.host_str() != Some("asset.localhost")
        })
    }
}

/// The file a desktop asset link names: `asset://localhost/…`, or `http(s)://asset.localhost/…`
/// on Windows and Android, followed by the file's absolute path, usually encoded as one segment.
fn asset_path(url: &str) -> Option<PathBuf> {
    let url = Url::parse(url).ok()?;
    let asset = match url.scheme() {
        "asset" => url.host_str() == Some("localhost"),
        "http" | "https" => url.host_str() == Some("asset.localhost"),
        _ => false,
    };
    if !asset {
        return None;
    }
    let decoded = percent_encoding::percent_decode_str(url.path())
        .decode_utf8()
        .ok()?;
    let path = decoded.trim_start_matches('/');
    if path.is_empty() {
        return None;
    }
    let windows_drive = path.as_bytes().get(1) == Some(&b':');
    Some(PathBuf::from(if windows_drive {
        path.to_owned()
    } else {
        format!("/{path}")
    }))
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
            Self::Binary => "images, documents, audio and video",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Image,
    Document,
    Audio,
    Video,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Document => "document",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

/// A part beyond what one call inlines: its size, or the bytes or time left of the call.
#[derive(Debug)]
struct BeyondBudget(String);

impl fmt::Display for BeyondBudget {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BeyondBudget {}

/// How a provider takes one kind of media.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Take {
    /// Not at all: the part is left to the provider's client, which notes it.
    Never,
    /// Only inline.
    Inline,
    /// From the public links it fetches itself; any other link goes inline.
    Link,
}

/// How a provider takes one kind of media, and of which MIME types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reads {
    take: Take,
    /// Canonical MIME types, or `None` for every type.
    types: Option<&'static [&'static str]>,
}

impl Reads {
    const NEVER: Self = Self {
        take: Take::Never,
        types: None,
    };
    const INLINE: Self = Self {
        take: Take::Inline,
        types: None,
    };
    const LINK: Self = Self {
        take: Take::Link,
        types: None,
    };

    const fn only(self, types: &'static [&'static str]) -> Self {
        Self {
            types: Some(types),
            ..self
        }
    }

    /// How the provider takes media of `mime`. An image that only its response types is taken,
    /// and its client notes it if the response names a type it cannot read.
    fn of(self, mime: Option<&str>) -> Take {
        match (self.types, mime) {
            (Some(types), Some(mime)) if !types.contains(&mime) => Take::Never,
            _ => self.take,
        }
    }
}

/// Raw bytes one call sends inline: in all, and in one part of a kind. Inline media travel as
/// base64, a third larger, within the provider's request limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InlineCaps {
    call: u64,
    image: u64,
    document: u64,
    /// Audio and video.
    media: u64,
}

impl InlineCaps {
    const fn total(call: u64) -> Self {
        Self {
            call,
            image: call,
            document: call,
            media: call,
        }
    }

    const fn per_image(self, image: u64) -> Self {
        Self { image, ..self }
    }

    const fn per_document(self, document: u64) -> Self {
        Self { document, ..self }
    }

    const fn per_media(self, media: u64) -> Self {
        Self { media, ..self }
    }

    fn part(&self, kind: Kind) -> u64 {
        let part = match kind {
            Kind::Image => self.image,
            Kind::Document => self.document,
            Kind::Audio | Kind::Video => self.media,
        };
        part.min(self.call)
    }

    /// Each cap, at most `limit`'s.
    fn within(self, limit: Self) -> Self {
        Self {
            call: self.call.min(limit.call),
            image: self.image.min(limit.image),
            document: self.document.min(limit.document),
            media: self.media.min(limit.media),
        }
    }
}

/// For providers that document no limit; sized for a 32 MB request.
const DEFAULT_CAPS: InlineCaps = InlineCaps::total(20 * MIB);
/// OpenAI: 50 MB of files per request. Its audio limit is undocumented.
const OPENAI_CAPS: InlineCaps = InlineCaps::total(50 * MB).per_media(20 * MIB);
/// Azure OpenAI: 20 MB per image and per audio file.
const AZURE_CAPS: InlineCaps = InlineCaps::total(20 * MIB)
    .per_image(20 * MB)
    .per_media(20 * MB);
/// Bedrock: about 20 MB per request, 3.75 MB per image and 4.5 MB per document.
const BEDROCK_CAPS: InlineCaps = InlineCaps::total(14 * MB)
    .per_image(3_750_000)
    .per_document(4_500_000);
/// Anthropic: 32 MB per request and 10 MB of base64 per image.
const ANTHROPIC_CAPS: InlineCaps = InlineCaps::total(20 * MIB).per_image(7_500_000);
/// Claude on Bedrock: 20 MB per request and 5 MB of base64 per image. Google allows more.
const HOSTED_CLAUDE_CAPS: InlineCaps = InlineCaps::total(14 * MB).per_image(3_750_000);
/// Gemini: 100 MB per request, of which its client keeps 90 MB for base64 media, and 50 MB per
/// document.
const GEMINI_CAPS: InlineCaps = InlineCaps::total(64 * MIB).per_document(50 * MB);
/// Vertex AI inline: 7 MB per image, 50 MB per PDF and 20 MB per audio or video file.
const VERTEX_CAPS: InlineCaps = InlineCaps::total(50 * MB)
    .per_image(7 * MB)
    .per_media(20 * MB);
/// Mistral: 10 MB per image.
const MISTRAL_CAPS: InlineCaps = InlineCaps::total(20 * MIB).per_image(10 * MB);
/// DeepSeek: 48 MiB per request and 32 MiB per image.
const DEEPSEEK_CAPS: InlineCaps = InlineCaps::total(32 * MIB);
/// Cohere: 20 MB of images per request.
const COHERE_CAPS: InlineCaps = InlineCaps::total(15 * MB);
/// Perplexity: 50 MiB of documents and 72 MiB per request.
const PERPLEXITY_CAPS: InlineCaps = InlineCaps::total(50 * MIB);
/// Text Generation Inference behind the Hugging Face router: 2 MB per request.
const TGI_CAPS: InlineCaps = InlineCaps::total(1_400_000);

const fn base64_len(bytes: u64) -> u64 {
    bytes.div_ceil(3) * 4
}

const _: () = assert!(base64_len(DEFAULT_CAPS.call) < 32 * MB);
const _: () = assert!(base64_len(ANTHROPIC_CAPS.call) < 32 * MB);
const _: () = assert!(base64_len(ANTHROPIC_CAPS.image) <= 10 * MB);
const _: () = assert!(base64_len(BEDROCK_CAPS.call) < 20 * MB);
const _: () = assert!(base64_len(HOSTED_CLAUDE_CAPS.call) < 20 * MB);
const _: () = assert!(base64_len(HOSTED_CLAUDE_CAPS.image) <= 5 * MB);
const _: () = assert!(base64_len(GEMINI_CAPS.call) < 90 * MB);
const _: () = assert!(base64_len(DEEPSEEK_CAPS.call) < 48 * MIB);
const _: () = assert!(base64_len(COHERE_CAPS.call) <= 20 * MB);
const _: () = assert!(base64_len(PERPLEXITY_CAPS.call) < 72 * MIB);
const _: () = assert!(base64_len(TGI_CAPS.call) < 2 * MB);

/// What a run sends the API's model relay inline in one call, beside the call's text documents
/// and messages within the relay's request limit.
const RELAY_CAPS: InlineCaps = InlineCaps::total(4 * MB);
const _: () = assert!(
    base64_len(RELAY_CAPS.call) + base64_len(DIRECT_POLICY.text_bytes) < MAX_REQUEST_BYTES as u64
);

/// What a provider's client does with each kind of media in a user message. Text documents are
/// always inlined as their text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaReader {
    images: Reads,
    pdfs: Take,
    /// Documents Rig has no type for, such as Office files, by their declared type.
    files: Reads,
    audio: Reads,
    video: Take,
    caps: InlineCaps,
}

/// Where a Bit of the OpenAI client sends its calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenAiHost {
    /// `api.openai.com`, Rig's default.
    Platform,
    /// A deployment through Rig's Azure client, which speaks Chat Completions only.
    AzureDeployment,
    /// Azure's OpenAI v1 API.
    AzureV1,
    /// vLLM, llama.cpp, TGI, Ollama, LM Studio or a gateway.
    Other,
}

impl OpenAiHost {
    fn of(provider: &ModelProvider) -> Self {
        if param_flag(provider, "is_azure") {
            return Self::AzureDeployment;
        }
        let Some(endpoint) = endpoint_url(provider) else {
            return Self::Platform;
        };
        let host = endpoint.host_str().unwrap_or_default();
        if endpoint.scheme() == "https"
            && (host == "api.openai.com" || host.ends_with(".api.openai.com"))
        {
            Self::Platform
        } else if host.ends_with(".openai.azure.com") {
            Self::AzureV1
        } else {
            Self::Other
        }
    }
}

fn endpoint_url(provider: &ModelProvider) -> Option<Url> {
    let endpoint = provider.params.as_ref()?.get("endpoint")?.as_str()?.trim();
    Url::parse(endpoint).ok()
}

fn param_flag(provider: &ModelProvider, key: &str) -> bool {
    provider
        .params
        .as_ref()
        .and_then(|params| params.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

impl MediaReader {
    /// Reads text documents only and leaves any other media to its client.
    const TEXT_ONLY: Self = Self {
        images: Reads::NEVER,
        pdfs: Take::Never,
        files: Reads::NEVER,
        audio: Reads::NEVER,
        video: Take::Never,
        caps: DEFAULT_CAPS,
    };
    /// OpenAI's Responses API: links to web images, PDFs and Office files, and no audio.
    const OPENAI_RESPONSES: Self = Self::TEXT_ONLY
        .with_images(Reads::LINK.only(WEB_IMAGES))
        .with_pdfs(Take::Link)
        .with_files(Reads::LINK.only(OOXML))
        .with_caps(OPENAI_CAPS);
    /// OpenAI's Chat Completions: image links, PDFs as file data, WAV and MP3 audio.
    const OPENAI_CHAT: Self = Self::TEXT_ONLY
        .with_images(Reads::LINK.only(WEB_IMAGES))
        .with_pdfs(Take::Inline)
        .with_audio(Reads::INLINE.only(WAV_MP3))
        .with_caps(OPENAI_CAPS);
    const AZURE_CHAT: Self = Self::OPENAI_CHAT.with_caps(AZURE_CAPS);
    /// Azure's v1 Responses API, which reads PDFs but no Office files or audio.
    const AZURE_RESPONSES: Self = Self::TEXT_ONLY
        .with_images(Reads::LINK.only(WEB_IMAGES))
        .with_pdfs(Take::Link)
        .with_caps(AZURE_CAPS);
    /// A self-hosted server behind the OpenAI client: none fetches links the way the hosted APIs
    /// do, and none takes a file part.
    const OPENAI_COMPATIBLE_RESPONSES: Self = Self::TEXT_ONLY.with_images(Reads::INLINE);
    const OPENAI_COMPATIBLE_CHAT: Self =
        Self::OPENAI_COMPATIBLE_RESPONSES.with_audio(Reads::INLINE.only(WAV_MP3));
    /// Bedrock fetches no links.
    const BEDROCK: Self = Self::TEXT_ONLY
        .with_images(Reads::INLINE.only(WEB_IMAGES))
        .with_pdfs(Take::Inline)
        .with_caps(BEDROCK_CAPS);
    /// Anthropic fetches image links; PDFs travel inline.
    const ANTHROPIC: Self = Self::TEXT_ONLY
        .with_images(Reads::LINK.only(WEB_IMAGES))
        .with_pdfs(Take::Inline)
        .with_caps(ANTHROPIC_CAPS);
    /// Claude on Bedrock, Google Cloud or a proxy, which fetch no links.
    const HOSTED_CLAUDE: Self = Self::ANTHROPIC
        .with_images(Reads::INLINE.only(WEB_IMAGES))
        .with_caps(HOSTED_CLAUDE_CAPS);
    /// Gemini's client passes the public links Gemini fetches and downloads the rest itself.
    const GEMINI: Self = Self::TEXT_ONLY
        .with_images(Reads::LINK)
        .with_pdfs(Take::Link)
        .with_audio(Reads::LINK)
        .with_video(Take::Link)
        .with_caps(GEMINI_CAPS);
    /// Vertex's client, like Gemini's, decides which public links Vertex fetches.
    const VERTEX: Self = Self::GEMINI.with_caps(VERTEX_CAPS);
    /// OpenRouter fetches image and PDF links; audio and any video but YouTube travel inline.
    const OPENROUTER: Self = Self::TEXT_ONLY
        .with_images(Reads::LINK)
        .with_pdfs(Take::Link)
        .with_audio(Reads::INLINE)
        .with_video(Take::Inline);
    /// Mistral fetches image and PDF links; Voxtral takes audio inline only.
    const MISTRAL: Self = Self::TEXT_ONLY
        .with_images(Reads::LINK.only(WEB_IMAGES))
        .with_pdfs(Take::Link)
        .with_audio(Reads::INLINE)
        .with_caps(MISTRAL_CAPS);
    /// Perplexity fetches image links, but PDF and Word files travel inline: it refuses links
    /// with credentials, which a signed link may count as.
    const PERPLEXITY: Self = Self::TEXT_ONLY
        .with_images(Reads::LINK.only(WEB_IMAGES))
        .with_pdfs(Take::Inline)
        .with_files(Reads::INLINE.only(WORD))
        .with_caps(PERPLEXITY_CAPS);
    /// xAI fetches JPEG, PNG and PDF links.
    const XAI: Self = Self::TEXT_ONLY
        .with_images(Reads::LINK.only(JPEG_PNG))
        .with_pdfs(Take::Link);

    const fn with_images(self, images: Reads) -> Self {
        Self { images, ..self }
    }

    const fn with_pdfs(self, pdfs: Take) -> Self {
        Self { pdfs, ..self }
    }

    const fn with_files(self, files: Reads) -> Self {
        Self { files, ..self }
    }

    const fn with_audio(self, audio: Reads) -> Self {
        Self { audio, ..self }
    }

    const fn with_video(self, video: Take) -> Self {
        Self { video, ..self }
    }

    const fn with_caps(self, caps: InlineCaps) -> Self {
        Self { caps, ..self }
    }

    /// A provider that reads image links of `types` and no other media.
    const fn image_links(types: &'static [&'static str], caps: InlineCaps) -> Self {
        Self::TEXT_ONLY
            .with_images(Reads::LINK.only(types))
            .with_caps(caps)
    }

    /// A local server that reads images of `types` inline and no other media.
    const fn inline_images(types: &'static [&'static str]) -> Self {
        Self::TEXT_ONLY.with_images(Reads::INLINE.only(types))
    }

    /// The reader of the client `ModelFactory` builds for `provider`, by its name, API surface
    /// and endpoint.
    pub fn for_provider(provider: &ModelProvider) -> Self {
        let name = provider.provider_name.trim().to_ascii_lowercase();
        if is_hosted_provider_name(&name) {
            return match name.strip_prefix("hosted:").unwrap_or("openrouter") {
                "openai" => Self::openai(OpenAiHost::Platform, provider.api_surface_or_default()),
                "bedrock" => Self::BEDROCK,
                _ => Self::OPENROUTER,
            };
        }
        let surface = provider.api_surface.unwrap_or(ModelApiSurface::Responses);
        match name.strip_prefix("custom:").unwrap_or(&name) {
            "openai" => Self::openai(OpenAiHost::of(provider), surface),
            "azure" => Self::AZURE_CHAT,
            "bedrock" => Self::BEDROCK,
            "anthropic"
                if endpoint_url(provider)
                    .is_none_or(|url| url.host_str() == Some("api.anthropic.com")) =>
            {
                Self::ANTHROPIC
            }
            "anthropic" => Self::HOSTED_CLAUDE,
            "gemini" => Self::GEMINI,
            "vertex" => Self::VERTEX,
            "openrouter" => Self::OPENROUTER,
            "mistral" => Self::MISTRAL,
            "perplexity" => Self::PERPLEXITY,
            "xai" => Self::XAI,
            "deepseek" => Self::image_links(WEB_IMAGES, DEEPSEEK_CAPS),
            "cohere" => Self::image_links(WEB_IMAGES, COHERE_CAPS),
            "groq" | "together" => Self::image_links(WEB_IMAGES, DEFAULT_CAPS),
            "huggingface" => Self::TEXT_ONLY.with_images(Reads::LINK).with_caps(TGI_CAPS),
            "moonshot" => Self::inline_images(MOONSHOT_IMAGES),
            "ollama" => Self::inline_images(JPEG_PNG_WEBP),
            "lmstudio" => Self::inline_images(JPEG_PNG),
            "mozilla" => Self::inline_images(JPEG_PNG_GIF),
            _ => Self::TEXT_ONLY,
        }
    }

    fn openai(host: OpenAiHost, surface: ModelApiSurface) -> Self {
        match (host, surface.is_responses()) {
            (OpenAiHost::Platform, true) => Self::OPENAI_RESPONSES,
            (OpenAiHost::Platform, false) => Self::OPENAI_CHAT,
            (OpenAiHost::AzureV1, true) => Self::AZURE_RESPONSES,
            (OpenAiHost::AzureDeployment | OpenAiHost::AzureV1, _) => Self::AZURE_CHAT,
            (OpenAiHost::Other, true) => Self::OPENAI_COMPATIBLE_RESPONSES,
            (OpenAiHost::Other, false) => Self::OPENAI_COMPATIBLE_CHAT,
        }
    }
}

/// The image type a `Content-Type` names, if Rig can send it.
fn image_type(content_type: &str) -> Option<ImageMediaType> {
    let essence = content_type.split(';').next()?.trim().to_ascii_lowercase();
    match essence.as_str() {
        "image/jpg" => Some(ImageMediaType::JPEG),
        essence => ImageMediaType::from_mime_type(essence),
    }
}

struct InlinedMedia {
    data_url: String,
    /// Where the base64 payload of `data_url` starts.
    payload: usize,
    bytes: u64,
    /// A text document, whose payload is its UTF-8 text.
    text: bool,
}

impl InlinedMedia {
    fn binary(mime: &str, body: &[u8]) -> Self {
        Self::encode(format!("data:{mime};base64,"), body, false)
    }

    /// Marked as a string source, so history conversion yields a text document.
    fn text(mime: &str, body: &[u8]) -> Result<Self> {
        let text = decode_text(body)?;
        Ok(Self::encode(
            format!("data:{mime};flow-like-source=string;base64,"),
            text.as_bytes(),
            true,
        ))
    }

    fn encode(mut data_url: String, payload: &[u8], text: bool) -> Self {
        let start = data_url.len();
        data_url.reserve(payload.len().div_ceil(3) * 4);
        STANDARD.encode_string(payload, &mut data_url);
        Self {
            data_url,
            payload: start,
            bytes: payload.len() as u64,
            text,
        }
    }

    /// The MIME type `data_url` names.
    fn mime(&self) -> &str {
        self.data_url["data:".len()..self.payload]
            .split([';', ','])
            .next()
            .unwrap_or_default()
    }

    /// The source Rig sends: a text document as its text, any other part as base64.
    fn source(&self) -> DocumentSourceKind {
        let base64 = &self.data_url[self.payload..];
        if self.text {
            let text = STANDARD.decode(base64).unwrap_or_default();
            DocumentSourceKind::String(String::from_utf8_lossy(&text).into_owned())
        } else {
            DocumentSourceKind::Base64(base64.to_owned())
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
type CacheKey = (String, String, String);

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

#[derive(Clone)]
struct FailedFetch {
    message: Arc<str>,
    beyond_budget: bool,
}

impl FailedFetch {
    fn of(error: &flow_like_types::Error) -> Self {
        Self {
            message: Arc::from(error.to_string()),
            beyond_budget: error.is::<BeyondBudget>(),
        }
    }

    fn error(&self) -> flow_like_types::Error {
        let message = format!("{} (a failure cached for {FAILURE_TTL:?})", self.message);
        if self.beyond_budget {
            BeyondBudget(message).into()
        } else {
            anyhow!(message)
        }
    }
}

/// Failed downloads by the same key and size limit, so a tool loop does not wait on a dead URL
/// every call.
static FAILED_FETCHES: LazyLock<moka::sync::Cache<(CacheKey, u64), FailedFetch>> =
    LazyLock::new(|| {
        moka::sync::Cache::builder()
            .max_capacity(FAILURE_ENTRIES)
            .time_to_live(FAILURE_TTL)
            .build()
    });

/// Readies the media of each call as [`MediaReader`] and its [`MediaSource`] decide: `invoke`
/// on its History, and agents, which build their requests from [`ModelLogic::provider`], on
/// each Rig request.
pub struct RemoteMediaModel {
    inner: Arc<dyn ModelLogic>,
    media: Arc<MediaPreparation>,
}

impl RemoteMediaModel {
    /// A direct call. Downloads pass the egress guard of `environment`, and a link the provider
    /// fetches itself stays a link when its host is public HTTPS.
    pub fn new(
        inner: Arc<dyn ModelLogic>,
        environment: ExecutionEnvironment,
        reader: MediaReader,
    ) -> Self {
        let source = EnvironmentSource::new(environment);
        Self::with(
            inner,
            Arc::new(source),
            reader,
            InlinePolicy::direct(environment),
        )
    }

    /// A direct call that also reads the desktop asset links of files under `storage_roots`, the
    /// app's own storage on this machine, and fetches the links into `own_storage`, the object
    /// store of a server's storage credentials. A link to any other file fails the call.
    pub fn direct(
        inner: Arc<dyn ModelLogic>,
        environment: ExecutionEnvironment,
        reader: MediaReader,
        storage_roots: Vec<PathBuf>,
        own_storage: OwnStorage,
    ) -> Self {
        let source = EnvironmentSource::reading(environment, storage_roots, own_storage);
        Self::with(
            inner,
            Arc::new(source),
            reader,
            InlinePolicy::direct(environment),
        )
    }

    /// A call the API's model relay makes: `source` downloads the media and decides links.
    pub fn resolving(
        inner: Arc<dyn ModelLogic>,
        source: Arc<dyn MediaSource>,
        reader: MediaReader,
    ) -> Self {
        Self::with(inner, source, reader, RESOLVING_POLICY)
    }

    /// A call a run in `environment` makes through the API's model relay. Links travel as the
    /// run holds them, except a link to a host that is neither public nor in `own_storage`: the
    /// relay fetches no such host, so the run sends its media inline, within the relay's request
    /// limit.
    pub fn relayed(
        inner: Arc<dyn ModelLogic>,
        environment: ExecutionEnvironment,
        reader: MediaReader,
        own_storage: OwnStorage,
    ) -> Self {
        let source = RelayedSource {
            downloads: EnvironmentSource::new(environment),
            own_storage,
        };
        let reader = MediaReader {
            caps: reader.caps.within(RELAY_CAPS),
            ..reader
        };
        Self::with(
            inner,
            Arc::new(source),
            reader,
            InlinePolicy::direct(environment),
        )
    }

    fn with(
        inner: Arc<dyn ModelLogic>,
        source: Arc<dyn MediaSource>,
        reader: MediaReader,
        policy: InlinePolicy,
    ) -> Self {
        Self {
            inner,
            media: Arc::new(MediaPreparation {
                source,
                reader,
                policy,
            }),
        }
    }
}

#[flow_like_types::async_trait]
impl ModelLogic for RemoteMediaModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        let constructor = self.inner.provider().await?;
        Ok(ModelConstructor {
            inner: Box::new(MediaClient {
                inner: constructor.inner,
                media: self.media.clone(),
            }),
        })
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

    async fn invoke(&self, history: &History, lambda: Option<LLMCallback>) -> Result<Response> {
        match self.media.history(history).await? {
            Some(prepared) => self.inner.invoke(&prepared, lambda).await,
            None => self.inner.invoke(history, lambda).await,
        }
    }
}

/// How the calls of one model ready their media.
struct MediaPreparation {
    source: Arc<dyn MediaSource>,
    reader: MediaReader,
    policy: InlinePolicy,
}

impl MediaPreparation {
    /// Whether a link is downloaded at all: over HTTP as the policy allows, into the source's own
    /// storage, or a desktop asset link the source reads from disk.
    fn reads(&self, url: &str) -> bool {
        self.policy.fetches(url) || self.source.owns(url) || self.source.reads_file(url)
    }

    /// A copy of `history` with its media ready, or `None` when nothing changed.
    async fn history(&self, history: &History) -> Result<Option<History>> {
        let candidates = history_candidates(history, self);
        let resolved = resolve(candidates, self).await?;
        if resolved.is_empty() {
            return Ok(None);
        }
        let mut prepared = history.clone();
        for (candidate, resolution) in &resolved {
            if let Some(url) = media_url_mut(&mut prepared, candidate.message, candidate.part) {
                resolution.link().clone_into(url);
            }
        }
        Ok(Some(prepared))
    }

    async fn request(
        &self,
        mut request: CompletionRequest,
    ) -> std::result::Result<CompletionRequest, CompletionError> {
        let candidates = request_candidates(&request, self);
        let resolved = resolve(candidates, self)
            .await
            .map_err(|error| CompletionError::RequestError(error.into()))?;
        for (candidate, resolution) in &resolved {
            if let Some(part) = user_part_mut(&mut request, candidate.message, candidate.part) {
                resolution.apply(part);
            }
        }
        Ok(request)
    }
}

/// A provider client whose every request, from `invoke` or an agent, has its media ready first.
struct MediaClient {
    inner: Box<dyn CompletionClientDyn + Send + Sync>,
    media: Arc<MediaPreparation>,
}

impl MediaClient {
    fn model<'a>(&self, model: &str) -> MediaRequests<'a> {
        MediaRequests {
            inner: Arc::from(self.inner.completion_model(model)),
            media: self.media.clone(),
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
struct MediaRequests<'a> {
    inner: Arc<dyn CompletionModelDyn + 'a>,
    media: Arc<MediaPreparation>,
}

impl CompletionModelDyn for MediaRequests<'_> {
    fn completion(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<
        '_,
        std::result::Result<CompletionResponse<DynamicResponse>, CompletionError>,
    > {
        Box::pin(async move {
            self.inner
                .completion(self.media.request(request).await?)
                .await
        })
    }

    fn stream(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<
        '_,
        std::result::Result<StreamingCompletionResponse<DynamicStreamingResponse>, CompletionError>,
    > {
        Box::pin(async move { self.inner.stream(self.media.request(request).await?).await })
    }

    fn completion_request(
        &self,
        prompt: Message,
    ) -> CompletionRequestBuilder<CompletionModelHandle<'_>> {
        CompletionRequestBuilder::new(CompletionModelHandle::new(Arc::new(self.clone())), prompt)
    }
}

/// Downloads the media links of model calls and decides what becomes of a link the provider
/// fetches itself.
#[flow_like_types::async_trait]
pub trait MediaSource: Send + Sync {
    /// Keeps downloads through different guards apart in the caches.
    fn cache_scope(&self) -> &str;

    /// Readies the downloads of a call, before its download deadline starts.
    async fn prepare(&self) -> Result<()> {
        Ok(())
    }

    /// Sends the GET of `url`. Errors name its host at most, never the URL.
    async fn send(&self, url: &str) -> Result<reqwest::Response>;

    /// What the provider receives for a link it fetches itself.
    async fn link(&self, url: &str) -> Result<Link>;

    /// What the provider receives for a link to media it reads only inline: the media, inline,
    /// unless the source leaves the link to the next hop.
    async fn inline_only(&self, _url: &str) -> Result<Link> {
        Ok(Link::Inline)
    }

    /// Whether `url` links into this source's own object store, which it fetches without
    /// redirects even when the store is internal or serves plain HTTP.
    fn owns(&self, _url: &str) -> bool {
        false
    }

    /// Whether this source reads `url`, a desktop asset link, from this machine's disk.
    fn reads_file(&self, _url: &str) -> bool {
        false
    }

    /// The file a desktop asset link names, refused once it exceeds `limit` bytes. Errors never
    /// name its path.
    async fn read_file(&self, _url: &str, _limit: u64) -> Result<Vec<u8>> {
        bail!("this server reads no file of the machine that holds the link")
    }
}

/// What the provider receives for a link it fetches itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// The link as the call holds it.
    Keep,
    /// Another link to the same media.
    Replace(String),
    /// The media, inline.
    Inline,
}

/// Whether a provider can fetch `url` itself: HTTPS, and every address its host resolves to
/// public.
pub async fn publicly_reachable(url: &str) -> bool {
    timeout(
        LINK_CHECK_TIMEOUT,
        ProviderEgress::Public.ensure_endpoint_allowed(url),
    )
    .await
    .is_ok_and(|checked| checked.is_ok())
}

/// Where a store points the links it signs: their origin (scheme, host and port) and the path in
/// front of the object key, the bucket's for a path-style store.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StorageScope {
    origin: url::Origin,
    path_prefix: String,
}

impl StorageScope {
    /// `None` unless the probe link is HTTP or HTTPS and its path ends in the probe's key.
    fn from_probe(probe: &Url) -> Option<Self> {
        if !matches!(probe.scheme(), "http" | "https") {
            return None;
        }
        let path_prefix = probe
            .path()
            .strip_suffix(PROBE_OBJECT)
            .filter(|prefix| prefix.ends_with('/'))?;
        Some(Self {
            origin: probe.origin(),
            path_prefix: path_prefix.to_owned(),
        })
    }

    /// The object key a link into this scope names, as the link encodes it.
    fn key<'a>(&self, url: &'a Url) -> Option<&'a str> {
        if url.origin() != self.origin || !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        url.path()
            .strip_prefix(self.path_prefix.as_str())
            .filter(|key| is_object_key(key))
    }
}

/// Non-empty segments of URL path characters and percent escapes. An escaped separator or NUL,
/// or a dot segment, could make the store read another key, so none counts.
fn is_object_key(key: &str) -> bool {
    key.split('/').all(|segment| {
        let decoded: Cow<'_, [u8]> = percent_encoding::percent_decode_str(segment).into();
        !segment.is_empty()
            && !matches!(decoded.as_ref(), b"." | b"..")
            && !decoded.iter().any(|byte| matches!(byte, b'/' | b'\\' | 0))
            && segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:@%".contains(ch))
    })
}

#[derive(Clone, Debug)]
struct OwnStore {
    scope: StorageScope,
    store: FlowLikeStore,
    /// Whether a provider can fetch the store's links itself, asked on first use.
    reachable: OnceCell<bool>,
}

impl OwnStore {
    fn new(scope: StorageScope, store: FlowLikeStore) -> Self {
        Self {
            scope,
            store,
            reachable: OnceCell::new(),
        }
    }

    /// HTTPS, and every address the store's host resolves to public.
    async fn reachable(&self) -> bool {
        let origin = self.scope.origin.ascii_serialization();
        *self
            .reachable
            .get_or_init(|| async move { publicly_reachable(&origin).await })
            .await
    }

    /// A link to the object `key` names that expires after [`RESIGNED_TTL`], or `None` when the
    /// store cannot sign or would sign a link to another object than `url`.
    async fn resign(&self, url: &Url, key: &str) -> Option<String> {
        let path = flow_like_storage::Path::from_url_path(key).ok()?;
        let fresh = self
            .store
            .sign("GET", &path, RESIGNED_TTL)
            .await
            .inspect_err(|_| {
                tracing::warn!(
                    host = url.host_str().unwrap_or_default(),
                    "Signing a link into the own storage again failed; its media is sent inline"
                );
            })
            .ok()?;
        let same_object = fresh.origin() == url.origin() && fresh.path() == url.path();
        same_object.then(|| fresh.into())
    }
}

/// The object store a server reaches through its storage credentials, by where the links its
/// stores sign point. Self-hosted deployments run it on their own network, often over plain
/// HTTP, and the links a run holds, such as uploads, chat attachments and the files of its app,
/// point there.
#[derive(Clone, Debug, Default)]
pub struct OwnStorage {
    stores: Vec<OwnStore>,
}

impl OwnStorage {
    /// Where `stores` point the links they sign. Only S3-compatible stores are asked: they sign
    /// without a network call, and they are the stores deployments run on their own network.
    /// Azure Blob Storage and Google Cloud Storage links are public HTTPS, which every call
    /// fetches already.
    pub async fn signed_by(stores: impl IntoIterator<Item = FlowLikeStore>) -> Self {
        let probe = flow_like_storage::Path::from(PROBE_OBJECT);
        let mut own: Vec<OwnStore> = Vec::new();
        for store in stores {
            if !matches!(store, FlowLikeStore::AWS(_)) {
                continue;
            }
            let Some(scope) = store
                .sign("GET", &probe, PROBE_TTL)
                .await
                .ok()
                .and_then(|signed| StorageScope::from_probe(&signed))
            else {
                continue;
            };
            if !own.iter().any(|known| known.scope == scope) {
                own.push(OwnStore::new(scope, store));
            }
        }
        Self { stores: own }
    }

    fn is_empty(&self) -> bool {
        self.stores.is_empty()
    }

    /// The store `url` links into and the object key it names there, as the link encodes it.
    fn store_of<'a>(&self, url: &'a Url) -> Option<(&OwnStore, &'a str)> {
        self.stores
            .iter()
            .find_map(|own| Some((own, own.scope.key(url)?)))
    }

    /// Whether `url` links into the store: its origin, an object key under its path, and no
    /// credentials of the link's own.
    fn contains(&self, url: &Url) -> bool {
        self.store_of(url).is_some()
    }

    /// Downloads that may come from the store are cached apart from those that may not.
    fn cache_scope(&self, environment: ExecutionEnvironment) -> String {
        self.stores
            .iter()
            .fold(environment.as_str().to_owned(), |scope, own| {
                format!(
                    "{scope} {}{}",
                    own.scope.origin.ascii_serialization(),
                    own.scope.path_prefix
                )
            })
    }
}

/// Whether the own store may not be reached at `ip`: an address on the host plane, other than
/// loopback, where a deployment may run its store beside the executor.
fn refused_for_storage(ip: IpAddr) -> bool {
    is_host_plane_ip(ip) && !ip.to_canonical().is_loopback()
}

/// Refuses a link into the own store whose host is on the host plane, unless it is loopback.
/// A name is checked here by convention and by its addresses when the client connects.
fn ensure_storage_host(url: &Url) -> Result<()> {
    let refused = match url.host() {
        Some(url::Host::Ipv4(ip)) => refused_for_storage(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => refused_for_storage(IpAddr::V6(ip)),
        Some(url::Host::Domain(host)) => {
            let loopback = matches!(
                host.trim_end_matches('.').to_ascii_lowercase().as_str(),
                "localhost" | "ip6-localhost" | "ip6-loopback"
            );
            is_host_plane_name(host) && !loopback
        }
        None => true,
    };
    if refused {
        bail!(
            "the request was refused: the storage host '{}' is on this executor's own network \
             plane (link-local, metadata and other host addresses)",
            url.host_str().unwrap_or_default()
        );
    }
    Ok(())
}

/// The addresses the own store's `host` resolved to, refused when one is on the host plane
/// other than loopback.
fn storage_addresses(
    host: &str,
    addrs: impl IntoIterator<Item = SocketAddr>,
) -> std::result::Result<Vec<SocketAddr>, String> {
    let addrs: Vec<SocketAddr> = addrs.into_iter().collect();
    if addrs.is_empty() {
        return Err(format!("the storage host '{host}' resolved to no address"));
    }
    match addrs.iter().find(|addr| refused_for_storage(addr.ip())) {
        Some(refused) => Err(format!(
            "the storage host '{host}' resolves to {}, on this executor's own network plane",
            refused.ip()
        )),
        None => Ok(addrs),
    }
}

/// Resolves the own store's name when the client connects, so a name that comes to resolve onto
/// the host plane is refused then.
struct StorageResolver;

impl Resolve for StorageResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addrs = flow_like_types::tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|_| format!("the storage host '{host}' could not be resolved"))?;
            let addrs = storage_addresses(&host, addrs)?;
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// A transport error and its causes, such as a refusal of the resolver, without the URL.
fn transport_error(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut message = error.to_string();
    let mut cause = std::error::Error::source(&error);
    while let Some(inner) = cause {
        message = format!("{message}: {inner}");
        cause = inner.source();
    }
    message
}

/// A direct call's downloads, through the egress guard of its environment, and from the own store
/// of a server as they are. A link the provider fetches itself stays a link when the provider can
/// reach it; a link into the own store is signed again to expire after [`RESIGNED_TTL`] first, or
/// sent inline when the store does not serve the fresh link.
struct EnvironmentSource {
    environment: ExecutionEnvironment,
    client: OnceLock<std::result::Result<GuardedHttpClient, String>>,
    /// The app's storage on this machine, the only files its desktop asset links are read from.
    storage_roots: Vec<PathBuf>,
    own_storage: OwnStorage,
    /// Follows no redirect and honours the proxy settings, through which a sandboxed executor
    /// reaches its store.
    storage_client: OnceLock<std::result::Result<reqwest::Client, String>>,
    scope: String,
}

impl EnvironmentSource {
    fn new(environment: ExecutionEnvironment) -> Self {
        Self::reading(environment, Vec::new(), OwnStorage::default())
    }

    fn reading(
        environment: ExecutionEnvironment,
        storage_roots: Vec<PathBuf>,
        own_storage: OwnStorage,
    ) -> Self {
        Self {
            environment,
            client: OnceLock::new(),
            storage_roots,
            scope: own_storage.cache_scope(environment),
            own_storage,
            storage_client: OnceLock::new(),
        }
    }

    fn client(&self) -> Result<&GuardedHttpClient> {
        self.client
            .get_or_init(|| GuardedHttpClient::new(self.environment).map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|error| anyhow!("building the HTTP client for inline media failed: {error}"))
    }

    fn storage_client(&self) -> Result<&reqwest::Client> {
        self.storage_client
            .get_or_init(|| {
                reqwest::Client::builder()
                    .redirect(redirect::Policy::none())
                    .dns_resolver(Arc::new(StorageResolver))
                    .build()
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|error| anyhow!("building the HTTP client for the storage failed: {error}"))
    }

    /// A GET of a link into the own store, whose answer is taken as it is: no redirect is
    /// followed.
    fn storage_get(&self, url: &str) -> Result<reqwest::RequestBuilder> {
        let url = Url::parse(url).map_err(|_| anyhow!("the link is not a valid URL"))?;
        ensure_storage_host(&url)?;
        Ok(self.storage_client()?.get(url))
    }

    /// The GET of a link into the own store: a redirect fails the download.
    async fn send_to_storage(&self, url: &str) -> Result<reqwest::Response> {
        self.storage_get(url)?
            .send()
            .await
            .map_err(|error| anyhow!("GET failed: {}", transport_error(error)))
    }

    /// How the own store answers a one-byte read of `url`, or `None` when it does not in time.
    async fn storage_status(&self, url: &str) -> Option<StatusCode> {
        let request = self
            .storage_get(url)
            .ok()?
            .header(RANGE, "bytes=0-0")
            .send();
        Some(
            timeout(RESIGNED_CHECK_TIMEOUT, request)
                .await
                .ok()?
                .ok()?
                .status(),
        )
    }

    /// What a provider that reaches the own store gets for `link` into it: the object signed
    /// again for [`RESIGNED_TTL`] when the store serves that link, else the media inline, read
    /// with the link the run holds. The run's credentials may not read every object of the
    /// store, such as one a connected app signed a link for.
    async fn fresh_link(&self, own: &OwnStore, link: &Url, key: &str) -> Link {
        let Some(fresh) = own.resign(link, key).await else {
            return Link::Inline;
        };
        let status = self.storage_status(&fresh).await;
        if matches!(status, Some(StatusCode::OK | StatusCode::PARTIAL_CONTENT)) {
            return Link::Replace(fresh);
        }
        tracing::debug!(
            host = link.host_str().unwrap_or_default(),
            status = ?status,
            "The own storage does not serve a link signed again; its media is sent inline"
        );
        Link::Inline
    }

    /// The file `path` resolves to, after links and `..`, when it lies in the app's storage.
    async fn stored_file(&self, path: &std::path::Path) -> Result<PathBuf> {
        let file = flow_like_types::tokio::fs::canonicalize(path)
            .await
            .map_err(|error| anyhow!("the file cannot be opened: {}", error.kind()))?;
        for root in &self.storage_roots {
            if let Ok(root) = flow_like_types::tokio::fs::canonicalize(root).await
                && file.starts_with(&root)
            {
                return Ok(file);
            }
        }
        bail!("the file is outside this app's storage, the only files a model call reads")
    }
}

#[flow_like_types::async_trait]
impl MediaSource for EnvironmentSource {
    fn cache_scope(&self) -> &str {
        &self.scope
    }

    async fn prepare(&self) -> Result<()> {
        self.client()?;
        if !self.own_storage.is_empty() {
            self.storage_client()?;
        }
        Ok(())
    }

    async fn send(&self, url: &str) -> Result<reqwest::Response> {
        if self.owns(url) {
            return self.send_to_storage(url).await;
        }
        self.client()?
            .get(url)?
            .send()
            .await
            .map_err(|error| anyhow!("GET failed: {}", error.without_url()))
    }

    async fn link(&self, url: &str) -> Result<Link> {
        if let Ok(link) = Url::parse(url)
            && let Some((own, key)) = self.own_storage.store_of(&link)
        {
            return Ok(if own.reachable().await {
                self.fresh_link(own, &link, key).await
            } else {
                Link::Inline
            });
        }
        Ok(if publicly_reachable(url).await {
            Link::Keep
        } else {
            Link::Inline
        })
    }

    fn owns(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|url| self.own_storage.contains(&url))
    }

    fn reads_file(&self, url: &str) -> bool {
        !self.storage_roots.is_empty() && asset_path(url).is_some()
    }

    async fn read_file(&self, url: &str, limit: u64) -> Result<Vec<u8>> {
        let path = asset_path(url).ok_or_else(|| anyhow!("the link names no file"))?;
        read_capped_file(&self.stored_file(&path).await?, limit).await
    }
}

/// A run's side of a call through the API's model relay, which resolves public links and links
/// into the platform's storage the way a direct call does and fetches no other host. A link to
/// any other host the run downloads itself, through the egress guard of its environment, and
/// sends inline: no provider reaches it either. Every other link travels as the run holds it.
struct RelayedSource {
    downloads: EnvironmentSource,
    /// The store the relay reads with the run's own links.
    own_storage: OwnStorage,
}

impl RelayedSource {
    async fn resolved_by_relay(&self, url: &str) -> Link {
        let own = Url::parse(url).is_ok_and(|link| self.own_storage.contains(&link));
        if own || publicly_reachable(url).await {
            Link::Keep
        } else {
            Link::Inline
        }
    }
}

#[flow_like_types::async_trait]
impl MediaSource for RelayedSource {
    fn cache_scope(&self) -> &str {
        self.downloads.cache_scope()
    }

    async fn prepare(&self) -> Result<()> {
        self.downloads.prepare().await
    }

    async fn send(&self, url: &str) -> Result<reqwest::Response> {
        self.downloads.send(url).await
    }

    async fn link(&self, url: &str) -> Result<Link> {
        Ok(self.resolved_by_relay(url).await)
    }

    async fn inline_only(&self, url: &str) -> Result<Link> {
        Ok(self.resolved_by_relay(url).await)
    }
}

/// A file's bytes, refused as beyond the budget once they exceed `limit`.
async fn read_capped_file(file: &std::path::Path, limit: u64) -> Result<Vec<u8>> {
    use flow_like_types::tokio::{fs, io::AsyncReadExt};
    let unreadable = |error: std::io::Error| anyhow!("the file cannot be read: {}", error.kind());
    let over_limit = |length: u64| {
        BeyondBudget(format!(
            "it is {length} bytes, over the {limit}-byte limit for one part sent inline"
        ))
    };
    let metadata = fs::metadata(file).await.map_err(unreadable)?;
    if !metadata.is_file() {
        bail!("the link names no file");
    }
    if metadata.len() > limit {
        return Err(over_limit(metadata.len()).into());
    }
    let mut body = Vec::new();
    fs::File::open(file)
        .await
        .map_err(unreadable)?
        .take(limit + 1)
        .read_to_end(&mut body)
        .await
        .map_err(unreadable)?;
    if body.len() as u64 > limit {
        return Err(over_limit(body.len() as u64).into());
    }
    Ok(body)
}

/// What a media part is to the reader.
struct Classified {
    kind: Kind,
    /// `None` for an image that only its response names the type of.
    mime: Option<String>,
    class: MediaClass,
    take: Take,
}

impl Classified {
    fn binary(kind: Kind, mime: Option<&str>, take: Take) -> Option<Self> {
        (take != Take::Never).then(|| Self {
            kind,
            mime: mime.map(ToOwned::to_owned),
            class: MediaClass::Binary,
            take,
        })
    }

    fn text(mime: impl Into<String>) -> Self {
        Self {
            kind: Kind::Document,
            mime: Some(mime.into()),
            class: MediaClass::Text,
            take: Take::Inline,
        }
    }
}

/// How the provider takes a part, or `None` when the part is left to its client. `declared` is
/// the MIME type the call declares for a document, which Rig drops when it has no type for it.
fn classify(
    part: &UserContent,
    declared: Option<&str>,
    url: &str,
    reader: &MediaReader,
) -> Option<Classified> {
    match part {
        UserContent::Image(image) => {
            let mime = image.media_type.as_ref().map(MimeType::to_mime_type);
            Classified::binary(Kind::Image, mime, reader.images.of(mime))
        }
        UserContent::Document(document) => match document.media_type.as_ref() {
            Some(DocumentMediaType::PDF) => {
                Classified::binary(Kind::Document, Some(PDF_TYPE), reader.pdfs)
            }
            Some(text) => Some(Classified::text(text.to_mime_type())),
            None => classify_untyped_document(untyped_document_type(declared, url)?, reader),
        },
        UserContent::Audio(audio) => {
            let mime = audio.media_type.as_ref()?.to_mime_type();
            Classified::binary(Kind::Audio, Some(mime), reader.audio.of(Some(mime)))
        }
        UserContent::Video(video) if !is_youtube(url) => {
            let mime = video.media_type.as_ref()?.to_mime_type();
            Classified::binary(Kind::Video, Some(mime), reader.video)
        }
        _ => None,
    }
}

/// A document of `mime`, a type Rig has none for: text and JSON as text, a PDF typed with
/// parameters as a PDF, any other file as the provider reads files.
fn classify_untyped_document(mime: String, reader: &MediaReader) -> Option<Classified> {
    if mime == PDF_TYPE {
        Classified::binary(Kind::Document, Some(PDF_TYPE), reader.pdfs)
    } else if is_text_type(&mime) {
        Some(Classified::text(mime))
    } else {
        let take = reader.files.of(Some(mime.as_str()));
        Classified::binary(Kind::Document, Some(mime.as_str()), take)
    }
}

/// The declared type of a document Rig has no type for, else the type its link's file extension
/// names. A generic binary type names none.
fn untyped_document_type(declared: Option<&str>, url: &str) -> Option<String> {
    declared
        .map(essence)
        .filter(|mime| names_a_type(mime))
        .or_else(|| extension_type(url))
}

fn essence(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn names_a_type(mime: &str) -> bool {
    !mime.is_empty() && !mime.ends_with("/octet-stream")
}

fn extension_type(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    let name = url.path_segments()?.next_back()?;
    let (_, extension) = name.rsplit_once('.')?;
    let guessed = mime_guess::from_ext(extension).first()?;
    Some(essence(guessed.essence_str())).filter(|mime| names_a_type(mime))
}

/// Text the model reads as such: `text/*` and JSON.
fn is_text_type(mime: &str) -> bool {
    mime.starts_with("text/") || mime == "application/json" || mime.ends_with("+json")
}

/// Gemini and OpenRouter take YouTube videos only as links.
fn is_youtube(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| {
        matches!(
            url.host_str(),
            Some("www.youtube.com" | "youtube.com" | "m.youtube.com" | "youtu.be")
        )
    })
}

/// A media link of the call that the provider reads, by its position.
struct Candidate {
    message: usize,
    part: usize,
    url: String,
    kind: Kind,
    /// `None` for an image that only its response names the type of.
    mime: Option<String>,
    class: MediaClass,
    /// The provider fetches this link itself, so the source decides what it receives.
    link: bool,
}

impl Candidate {
    fn new(message: usize, part: usize, url: &str, classified: Classified) -> Self {
        Self {
            message,
            part,
            url: url.to_owned(),
            kind: classified.kind,
            mime: classified.mime,
            class: classified.class,
            link: classified.take == Take::Link,
        }
    }

    /// Its kind and type, as errors name it.
    fn describe(&self) -> String {
        format!(
            "{} ({})",
            self.kind.label(),
            self.mime.as_deref().unwrap_or("type unknown")
        )
    }

    fn part_label(&self) -> String {
        match self.class {
            MediaClass::Text => "text documents in one call".to_owned(),
            MediaClass::Binary => format!("one {} sent inline", self.kind.label()),
        }
    }

    fn failed(&self, error: flow_like_types::Error) -> flow_like_types::Error {
        anyhow!(
            "The {} at host '{}' could not be sent to the model inline: {error}",
            self.describe(),
            url_host(&self.url)
        )
    }

    fn refused(&self, error: flow_like_types::Error) -> flow_like_types::Error {
        anyhow!(
            "The {} at host '{}' cannot be sent to the model: {error}",
            self.describe(),
            url_host(&self.url)
        )
    }
}

fn declared_type(content: &Content) -> Option<&str> {
    match content {
        Content::Document { media_type, .. } => media_type.as_deref(),
        _ => None,
    }
}

/// The media links of `history` the provider reads, newest first.
fn history_candidates(history: &History, media: &MediaPreparation) -> Vec<Candidate> {
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
                let url = content.media_url().filter(|url| media.reads(url))?;
                let part_content = UserContent::from(content.clone());
                let classified =
                    classify(&part_content, declared_type(content), url, &media.reader)?;
                Some(Candidate::new(message, part, url, classified))
            })
        })
        .collect()
}

/// The media links in the user messages of `request` the provider reads, newest first.
fn request_candidates(request: &CompletionRequest, media: &MediaPreparation) -> Vec<Candidate> {
    let messages: Vec<&Message> = request.chat_history.iter().collect();
    messages
        .into_iter()
        .enumerate()
        .rev()
        .filter_map(|(message, entry)| match entry {
            Message::User { content } => Some((message, content)),
            _ => None,
        })
        .flat_map(|(message, content)| {
            content
                .iter()
                .enumerate()
                .filter_map(move |(part, part_content)| {
                    let url = link_of(part_content).filter(|url| media.reads(url))?;
                    let classified = classify(part_content, None, url, &media.reader)?;
                    Some(Candidate::new(message, part, url, classified))
                })
        })
        .collect()
}

fn link_of(part: &UserContent) -> Option<&str> {
    let data = match part {
        UserContent::Image(image) => &image.data,
        UserContent::Document(document) => &document.data,
        UserContent::Audio(audio) => &audio.data,
        UserContent::Video(video) => &video.data,
        UserContent::Text(_) | UserContent::ToolResult(_) => return None,
    };
    match data {
        DocumentSourceKind::Url(url) => Some(url),
        _ => None,
    }
}

fn media_url_mut(history: &mut History, message: usize, part: usize) -> Option<&mut String> {
    let MessageContent::Contents(parts) = &mut history.messages.get_mut(message)?.content else {
        return None;
    };
    match parts.get_mut(part)? {
        Content::Image { image_url, .. } => Some(&mut image_url.url),
        Content::Document {
            document_url: url, ..
        }
        | Content::Audio { audio_url: url, .. }
        | Content::Video { video_url: url, .. } => Some(url),
        Content::Text { .. } => None,
    }
}

fn user_part_mut(
    request: &mut CompletionRequest,
    message: usize,
    part: usize,
) -> Option<&mut UserContent> {
    match request.chat_history.iter_mut().nth(message)? {
        Message::User { content } => content.iter_mut().nth(part),
        _ => None,
    }
}

/// What the provider receives in place of a link.
enum Resolution {
    Inline(Arc<InlinedMedia>),
    Replace(String),
}

impl Resolution {
    /// The value a History part holds instead of its link.
    fn link(&self) -> &str {
        match self {
            Self::Inline(media) => &media.data_url,
            Self::Replace(url) => url,
        }
    }

    /// Sets a Rig part's source, and the type of an image that only its response typed.
    fn apply(&self, part: &mut UserContent) {
        let source = match self {
            Self::Inline(media) => media.source(),
            Self::Replace(url) => DocumentSourceKind::Url(url.clone()),
        };
        match part {
            UserContent::Image(image) => {
                if let Self::Inline(media) = self
                    && image.media_type.is_none()
                {
                    image.media_type = image_type(media.mime());
                }
                image.data = source;
            }
            UserContent::Document(document) => document.data = source,
            UserContent::Audio(audio) => audio.data = source,
            UserContent::Video(video) => video.data = source,
            UserContent::Text(_) | UserContent::ToolResult(_) => {}
        }
    }
}

/// What the provider receives for each candidate that changes. The first part that can be
/// neither linked nor inlined fails the call.
async fn resolve(
    candidates: Vec<Candidate>,
    media: &MediaPreparation,
) -> Result<Vec<(Candidate, Resolution)>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let source = media.source.as_ref();
    let pending: Vec<_> = candidates
        .iter()
        .map(|candidate| async move {
            if candidate.link {
                source.link(&candidate.url).await
            } else {
                source.inline_only(&candidate.url).await
            }
        })
        .collect();
    let decisions: Vec<Result<Link>> = stream::iter(pending)
        .buffered(CONCURRENT_FETCHES)
        .collect()
        .await;
    let mut resolved = Vec::new();
    let mut inline = Vec::new();
    for (candidate, decision) in candidates.into_iter().zip(decisions) {
        match decision.map_err(|error| candidate.refused(error))? {
            Link::Keep => {}
            Link::Replace(url) => resolved.push((candidate, Resolution::Replace(url))),
            Link::Inline => inline.push(candidate),
        }
    }
    let fetched = fetch_media(inline, source, media.policy, media.reader.caps).await?;
    resolved.extend(
        fetched
            .into_iter()
            .map(|(candidate, media)| (candidate, Resolution::Inline(media))),
    );
    Ok(resolved)
}

/// Downloads the candidates' media concurrently, newest message first. Each part holds its bytes
/// of the call's budget once its length is declared or its bytes arrive, so the downloads together
/// never hold more than the budget. A part that cannot be fetched or is beyond the call's budgets
/// fails the call.
async fn fetch_media(
    candidates: Vec<Candidate>,
    source: &dyn MediaSource,
    policy: InlinePolicy,
    caps: InlineCaps,
) -> Result<Vec<(Candidate, Arc<InlinedMedia>)>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    if candidates.len() > policy.max_parts {
        bail!(
            "The call has {} media parts to send inline, more than the {} one call sends",
            candidates.len(),
            policy.max_parts
        );
    }
    source
        .prepare()
        .await
        .map_err(|error| anyhow!("Media for the model could not be fetched: {error}"))?;
    let fetcher = MediaFetcher::new(source, policy, caps);
    let mut inlined = Vec::with_capacity(candidates.len());
    {
        let pending: Vec<_> = candidates
            .iter()
            .map(|candidate| fetcher.fetch(candidate))
            .collect();
        let mut fetches = stream::iter(pending).buffered(CONCURRENT_FETCHES);
        for candidate in &candidates {
            let Some(fetched) = fetches.next().await else {
                break;
            };
            inlined.push(fetched.map_err(|error| candidate.failed(error))?);
        }
    }
    Ok(candidates.into_iter().zip(inlined).collect())
}

struct MediaFetcher<'a> {
    source: &'a dyn MediaSource,
    policy: InlinePolicy,
    caps: InlineCaps,
    deadline: Instant,
    text_left: AtomicU64,
    binary_left: AtomicU64,
}

impl<'a> MediaFetcher<'a> {
    fn new(source: &'a dyn MediaSource, policy: InlinePolicy, caps: InlineCaps) -> Self {
        Self {
            source,
            policy,
            caps,
            deadline: Instant::now() + policy.deadline,
            text_left: AtomicU64::new(policy.text_bytes),
            binary_left: AtomicU64::new(caps.call),
        }
    }

    fn left(&self, class: MediaClass) -> &AtomicU64 {
        match class {
            MediaClass::Text => &self.text_left,
            MediaClass::Binary => &self.binary_left,
        }
    }

    fn call_limit(&self, class: MediaClass) -> u64 {
        match class {
            MediaClass::Text => self.policy.text_bytes,
            MediaClass::Binary => self.caps.call,
        }
    }

    fn part_limit(&self, candidate: &Candidate) -> u64 {
        match candidate.class {
            MediaClass::Text => self.policy.text_bytes,
            MediaClass::Binary => self.caps.part(candidate.kind),
        }
    }

    fn ensure_budget_left(&self, class: MediaClass) -> Result<()> {
        if self.left(class).load(Ordering::Relaxed) == 0 {
            return Err(BeyondBudget(format!(
                "this call's {}-byte inline limit for {} is used up",
                self.call_limit(class),
                class.label()
            ))
            .into());
        }
        Ok(())
    }

    /// Nothing held yet of the call's budget for `class`.
    fn charge(&self, class: MediaClass) -> Charge<'_> {
        Charge {
            left: self.left(class),
            call: self.call_limit(class),
            class,
            held: 0,
            refused: false,
        }
    }

    /// Keeps a part's bytes of the call's budget. A part from the cache is checked against this
    /// provider's limit for one part as well.
    fn spend(&self, candidate: &Candidate, media: &InlinedMedia, charge: Charge<'_>) -> Result<()> {
        let limit = self.part_limit(candidate);
        if media.bytes > limit {
            return Err(BeyondBudget(format!(
                "it is {} bytes, over the {limit}-byte limit for {}",
                media.bytes,
                candidate.part_label()
            ))
            .into());
        }
        charge.settle(media.bytes)
    }

    /// Downloads a part, or takes it from the caches, and keeps its bytes of the call's budget.
    /// A failure is cached unless the call's deadline or budget caused it. Files on this machine
    /// are read again each call, so every read passes the storage check.
    async fn fetch(&self, candidate: &Candidate) -> Result<Arc<InlinedMedia>> {
        let cached = !self.source.reads_file(&candidate.url);
        let key = (
            self.source.cache_scope().to_owned(),
            candidate
                .mime
                .clone()
                .unwrap_or_else(|| IMAGE_TYPED_BY_RESPONSE.to_owned()),
            candidate.url.clone(),
        );
        let mut charge = self.charge(candidate.class);
        if cached && let Some(media) = MEDIA_CACHE.get(&key) {
            self.spend(candidate, &media, charge)?;
            return Ok(media);
        }
        let limit = self.part_limit(candidate);
        let failure_key = (key.clone(), limit);
        if cached && let Some(failed) = FAILED_FETCHES.get(&failure_key) {
            return Err(failed.error());
        }
        self.ensure_budget_left(candidate.class)?;
        let own_deadline = Instant::now() + self.policy.fetch_timeout;
        let until = own_deadline.min(self.deadline);
        let downloaded = match timeout_at(until, self.download(candidate, limit, &mut charge)).await
        {
            Ok(downloaded) => downloaded,
            Err(_) if until < own_deadline => {
                return Err(BeyondBudget(format!(
                    "the {:?} inline deadline of this call passed",
                    self.policy.deadline
                ))
                .into());
            }
            Err(_) => Err(anyhow!(
                "download did not finish within {:?}",
                self.policy.fetch_timeout
            )),
        };
        let media = Arc::new(downloaded.inspect_err(|error| {
            if cached && !charge.refused {
                FAILED_FETCHES.insert(failure_key, FailedFetch::of(error));
            }
        })?);
        if cached {
            MEDIA_CACHE.insert(key, media.clone());
        }
        self.spend(candidate, &media, charge)?;
        Ok(media)
    }

    async fn download(
        &self,
        candidate: &Candidate,
        limit: u64,
        charge: &mut Charge<'_>,
    ) -> Result<InlinedMedia> {
        let (mime, body) = if self.source.reads_file(&candidate.url) {
            let body = self.source.read_file(&candidate.url, limit).await?;
            charge.hold(body.len() as u64, true)?;
            let mime = match &candidate.mime {
                Some(mime) => mime.clone(),
                None => file_image_type(&candidate.url)?.to_mime_type().to_owned(),
            };
            (mime, body)
        } else {
            let response = self.source.send(&candidate.url).await?;
            self.ensure_inlinable(&candidate.url, &response)?;
            let mime = match &candidate.mime {
                Some(mime) => mime.clone(),
                None => response_image_type(&response)?.to_mime_type().to_owned(),
            };
            (
                mime,
                read_within(response, limit, &candidate.part_label(), charge).await?,
            )
        };
        match candidate.class {
            MediaClass::Text => InlinedMedia::text(&mime, &body),
            MediaClass::Binary => Ok(InlinedMedia::binary(&mime, &body)),
        }
    }

    /// A successful response that ended, after any redirects, on a URL the policy fetches, or
    /// that the source's own store gave for `url` itself.
    fn ensure_inlinable(&self, url: &str, response: &reqwest::Response) -> Result<()> {
        let origin = response.url();
        let from_storage =
            self.source.owns(url) && Url::parse(url).is_ok_and(|link| &link == origin);
        if !from_storage && !self.policy.fetches(origin.as_str()) {
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
        Ok(())
    }
}

/// The type of an image that only its response names.
fn response_image_type(response: &reqwest::Response) -> Result<ImageMediaType> {
    response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(image_type)
        .ok_or_else(|| anyhow!("the response names no image type a model accepts"))
}

/// The type of an image file that only its file extension names.
fn file_image_type(url: &str) -> Result<ImageMediaType> {
    asset_path(url)
        .and_then(|path| mime_guess::from_path(path).first())
        .and_then(|mime| image_type(mime.essence_str()))
        .ok_or_else(|| anyhow!("the file names no image type a model accepts"))
}

/// The response body, refused as beyond the budget once it exceeds `limit` bytes or what `charge`
/// can hold. A declared length is held before the body is read.
async fn read_within(
    mut response: reqwest::Response,
    limit: u64,
    label: &str,
    charge: &mut Charge<'_>,
) -> Result<Vec<u8>> {
    if let Some(length) = response.content_length() {
        if length > limit {
            return Err(BeyondBudget(format!(
                "it is {length} bytes, over the {limit}-byte limit for {label}"
            ))
            .into());
        }
        charge.hold(length, true)?;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| anyhow!("reading the response body failed: {}", error.without_url()))?
    {
        let read = (body.len() + chunk.len()) as u64;
        if read > limit {
            return Err(
                BeyondBudget(format!("it is over the {limit}-byte limit for {label}")).into(),
            );
        }
        charge.hold(read, false)?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The bytes of a call's budget that one part holds while it is read: its declared length, or
/// what arrived of it. Given back when the part fails, kept once it is spent.
struct Charge<'a> {
    left: &'a AtomicU64,
    call: u64,
    class: MediaClass,
    held: u64,
    /// The call's budget refused the part, which a call with other parts may not.
    refused: bool,
}

impl Charge<'_> {
    /// Holds `bytes` in all, `whole` when they are the part's size rather than what arrived of it.
    fn hold(&mut self, bytes: u64, whole: bool) -> Result<()> {
        let more = bytes.saturating_sub(self.held);
        if more == 0 {
            return Ok(());
        }
        match self
            .left
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(more)
            }) {
            Ok(_) => {
                self.held = bytes;
                Ok(())
            }
            Err(left) => {
                self.refused = true;
                let size = if whole {
                    format!("its {bytes} bytes exceed")
                } else {
                    "it exceeds".to_owned()
                };
                Err(BeyondBudget(format!(
                    "{size} the {} bytes left of this call's {}-byte inline limit for {}",
                    left + self.held,
                    self.call,
                    self.class.label()
                ))
                .into())
            }
        }
    }

    /// Keeps exactly `bytes` for a part that is sent and gives back the rest.
    fn settle(mut self, bytes: u64) -> Result<()> {
        self.hold(bytes, true)?;
        self.left.fetch_add(self.held - bytes, Ordering::Relaxed);
        self.held = 0;
        Ok(())
    }
}

impl Drop for Charge<'_> {
    fn drop(&mut self) {
        self.left.fetch_add(self.held, Ordering::Relaxed);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flow_like_model_provider::history::{ContentType, HistoryMessage, ImageUrl, Role};
    use rig::OneOrMany;
    use rig::completion::CompletionModel;
    use rig::message::AudioMediaType;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    const TEST_POLICY: InlinePolicy = InlinePolicy {
        text_bytes: 64,
        max_parts: 10,
        fetch_timeout: Duration::from_secs(5),
        deadline: Duration::from_secs(10),
        allow_http: true,
    };
    const PDF: &[u8] = b"%PDF-1.7 flow-like test document";
    pub(crate) const PNG: &[u8] = b"\x89PNG\r\n\x1a\n flow-like test image";
    const DOCX: &[u8] = b"PK\x03\x04 word/document.xml";
    const MP3: &[u8] = b"ID3 audio";
    const MP4: &[u8] = b"mp4 video";
    const LARGE: &[u8] = &[b'%'; 2048];
    const PUBLIC: &str = "https://93.184.216.34";
    const MISSING: Reply = Reply::Body {
        status: 404,
        body: b"missing",
        content_length: true,
    };
    const DENIED: Reply = Reply::Body {
        status: 403,
        body: b"denied",
        content_length: true,
    };

    pub(crate) enum Reply {
        Body {
            status: u16,
            body: &'static [u8],
            content_length: bool,
        },
        Typed(&'static str, &'static [u8]),
        /// Declares a body of this many bytes and sends none.
        Declared(u64),
        /// Declares a body of this many bytes and stalls before sending it.
        Withheld(u64),
        /// 403 for a request whose query holds the text, the body for any other.
        Refusing(&'static str, &'static [u8]),
        Redirect(&'static str),
        Stall,
    }

    impl Reply {
        pub(crate) fn ok(body: &'static [u8]) -> Self {
            Self::Body {
                status: 200,
                body,
                content_length: true,
            }
        }

        /// The head and body of every reply but a stall.
        fn response(&self) -> (String, &[u8]) {
            match self {
                Self::Redirect(location) => (
                    format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n"),
                    b"",
                ),
                Self::Typed(content_type, body) => (
                    format!(
                        "HTTP/1.1 200 Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
                        body.len()
                    ),
                    *body,
                ),
                Self::Declared(length) | Self::Withheld(length) => (
                    format!("HTTP/1.1 200 Test\r\nContent-Length: {length}\r\n"),
                    b"",
                ),
                Self::Refusing(_, body) => (
                    format!("HTTP/1.1 200 Test\r\nContent-Length: {}\r\n", body.len()),
                    *body,
                ),
                Self::Body {
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
                Self::Stall => (String::new(), b""),
            }
        }
    }

    pub(crate) struct TestServer {
        pub(crate) base: String,
        hits: Arc<AtomicUsize>,
    }

    impl TestServer {
        pub(crate) fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }

        pub(crate) fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    pub(crate) async fn serve(routes: Vec<(&str, Reply)>) -> TestServer {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind media server");
        let base = format!("http://{}", listener.local_addr().expect("server address"));
        let hits = Arc::new(AtomicUsize::new(0));
        let routes: Arc<HashMap<String, Reply>> = Arc::new(
            routes
                .into_iter()
                .map(|(path, reply)| (path.to_string(), reply))
                .collect(),
        );
        let counter = hits.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(answer(socket, routes.clone(), counter.clone()));
            }
        });
        TestServer { base, hits }
    }

    async fn answer(
        mut socket: TcpStream,
        routes: Arc<HashMap<String, Reply>>,
        hits: Arc<AtomicUsize>,
    ) {
        let (mut buffer, mut request) = ([0_u8; 1024], Vec::new());
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => request.extend_from_slice(&buffer[..read]),
            }
        }
        hits.fetch_add(1, Ordering::SeqCst);
        let head = String::from_utf8_lossy(&request);
        let target = head.split_whitespace().nth(1).unwrap_or_default();
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let reply = match routes.get(path).unwrap_or(&MISSING) {
            Reply::Refusing(refused, _) if query.contains(refused) => &DENIED,
            reply => reply,
        };
        if matches!(reply, Reply::Stall) {
            return std::future::pending::<()>().await;
        }
        let (head, body) = reply.response();
        let head = format!("{head}Connection: close\r\n\r\n");
        let _ = socket.write_all(head.as_bytes()).await;
        if matches!(reply, Reply::Withheld(_)) {
            return std::future::pending::<()>().await;
        }
        let _ = socket.write_all(body).await;
        let _ = socket.shutdown().await;
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

    fn image(url: String, media_type: Option<&str>) -> Content {
        Content::Image {
            content_type: ContentType::ImageUrl,
            image_url: ImageUrl {
                url,
                detail: None,
                media_type: media_type.map(ToOwned::to_owned),
                additional_params: None,
            },
        }
    }

    fn video(url: String) -> Content {
        Content::Video {
            content_type: ContentType::VideoUrl,
            video_url: url,
            media_type: Some("video/mp4".to_string()),
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
        let Message::User { content } = prompt else {
            panic!("the prompt is a user message");
        };
        content.into_iter().collect()
    }

    fn rig_request(history: &History) -> CompletionRequest {
        let (prompt, mut messages) = history
            .extract_prompt_and_history()
            .expect("convert history into rig messages");
        messages.push(prompt);
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: OneOrMany::many(messages).expect("a request has a prompt"),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        }
    }

    fn prompt_parts(request: &CompletionRequest) -> Vec<UserContent> {
        let Message::User { content } = request.chat_history.last_ref() else {
            panic!("the prompt is a user message");
        };
        content.iter().cloned().collect()
    }

    fn assert_text_document(content: &UserContent, expected: &str, media_type: DocumentMediaType) {
        let UserContent::Document(document) = content else {
            panic!("expected a document, got {content:?}");
        };
        assert_eq!(document.data, DocumentSourceKind::String(expected.into()));
        assert_eq!(document.media_type, Some(media_type));
    }

    fn provider(name: &str) -> ModelProvider {
        ModelProvider {
            provider_name: name.to_string(),
            model_id: None,
            version: None,
            api_surface: None,
            params: None,
        }
    }

    fn provider_with(name: &str, params: Value) -> ModelProvider {
        ModelProvider {
            params: flow_like_types::json::from_value(params).ok(),
            ..provider(name)
        }
    }

    fn chat(provider: ModelProvider) -> ModelProvider {
        ModelProvider {
            api_surface: Some(ModelApiSurface::ChatCompletions),
            ..provider
        }
    }

    fn reader(name: &str) -> MediaReader {
        MediaReader::for_provider(&provider(name))
    }

    /// A local direct call's preparation.
    fn direct(reader: MediaReader, policy: InlinePolicy) -> MediaPreparation {
        MediaPreparation {
            source: Arc::new(EnvironmentSource::new(ExecutionEnvironment::Local)),
            reader,
            policy,
        }
    }

    async fn prepare(original: &History, reader: MediaReader) -> Result<Option<History>> {
        direct(reader, TEST_POLICY).history(original).await
    }

    async fn prepared(original: &History, reader: MediaReader) -> History {
        prepare(original, reader)
            .await
            .expect("every part can be sent")
            .expect("parts changed")
    }

    async fn failure(original: &History, preparation: MediaPreparation) -> String {
        preparation
            .history(original)
            .await
            .expect_err("the call fails")
            .to_string()
    }

    /// Downloads like a local direct call and decides each link the provider fetches itself by
    /// its path: `/keep…` stays, `/move…` moves to `moved.example`, `/refuse…` fails, and any
    /// other is inlined.
    struct DecidingSource(EnvironmentSource);

    impl DecidingSource {
        fn new() -> Self {
            Self(EnvironmentSource::new(ExecutionEnvironment::Local))
        }
    }

    #[flow_like_types::async_trait]
    impl MediaSource for DecidingSource {
        fn cache_scope(&self) -> &str {
            "test-deciding"
        }

        async fn prepare(&self) -> Result<()> {
            self.0.prepare().await
        }

        async fn send(&self, url: &str) -> Result<reqwest::Response> {
            self.0.send(url).await
        }

        async fn link(&self, url: &str) -> Result<Link> {
            let path = Url::parse(url)?.path().to_owned();
            if path.starts_with("/refuse") {
                bail!("the source refuses it");
            }
            Ok(if path.starts_with("/keep") {
                Link::Keep
            } else if path.starts_with("/move") {
                Link::Replace(format!("https://moved.example{path}"))
            } else {
                Link::Inline
            })
        }
    }

    fn deciding(reader: MediaReader) -> MediaPreparation {
        MediaPreparation {
            source: Arc::new(DecidingSource::new()),
            reader,
            policy: TEST_POLICY,
        }
    }

    #[tokio::test]
    async fn pdf_and_audio_parts_become_base64_sources() {
        let server = serve(vec![
            ("/report.pdf", Reply::ok(PDF)),
            ("/scan.pdf", Reply::ok(PDF)),
            ("/voice.mp3", Reply::ok(MP3)),
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

        let resolved = prepared(&original, reader("custom:openrouter")).await;

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
        assert_eq!(audio.data, DocumentSourceKind::Base64(STANDARD.encode(MP3)));
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

        let resolved = prepared(&original, reader("custom:galadriel")).await;

        let contents = prompt_contents(&resolved);
        for (content, expected, media_type) in [
            (&contents[1], "a,b\n1,2\n", DocumentMediaType::CSV),
            (&contents[2], "plain notes", DocumentMediaType::TXT),
        ] {
            assert_text_document(content, expected, media_type);
        }
    }

    #[tokio::test]
    async fn untyped_json_and_text_are_inlined_as_text_and_office_files_keep_their_type() {
        const JSON: &[u8] = br#"{"rows":[1,2]}"#;
        let server = serve(vec![
            ("/export", Reply::ok(JSON)),
            ("/config.yaml", Reply::ok(b"name: flow-like\n")),
            ("/letter.docx", Reply::ok(DOCX)),
        ])
        .await;
        let letter = server.url("/letter.docx");
        let original = history(vec![user(vec![
            text("read these"),
            document(
                server.url("/export"),
                Some("application/json; charset=utf-8"),
            ),
            document(server.url("/config.yaml"), None),
            document(letter.clone(), Some(DOCX_TYPE)),
        ])]);

        let resolved = prepared(&original, reader("custom:openai")).await;

        assert_eq!(
            url_at(&resolved, 0, 1),
            format!(
                "data:application/json;flow-like-source=string;base64,{}",
                STANDARD.encode(JSON)
            )
        );
        assert!(
            url_at(&resolved, 0, 2).starts_with("data:text/x-yaml;flow-like-source=string;"),
            "{}",
            url_at(&resolved, 0, 2)
        );
        assert_eq!(
            url_at(&resolved, 0, 3),
            format!("data:{DOCX_TYPE};base64,{}", STANDARD.encode(DOCX))
        );
        let contents = prompt_contents(&resolved);
        let UserContent::Document(json) = &contents[1] else {
            panic!("expected a document, got {:?}", contents[1]);
        };
        assert_eq!(
            json.data,
            DocumentSourceKind::String(String::from_utf8_lossy(JSON).into_owned())
        );

        let for_claude = prepared(&original, reader("custom:anthropic")).await;
        assert!(url_at(&for_claude, 0, 1).starts_with("data:application/json;"));
        assert_eq!(
            url_at(&for_claude, 0, 3),
            letter,
            "Anthropic reads no Office file, so its client notes it"
        );
    }

    #[tokio::test]
    async fn legacy_encoded_text_is_decoded_and_binary_text_fails_the_call() {
        let server = serve(vec![
            ("/export.csv", Reply::ok(b"Name;Betrag\nM\xFCller;12\x80\n")),
            ("/notepad.txt", Reply::ok(b"\xFF\xFEh\0i\0")),
            ("/archive.txt", Reply::ok(b"PK\x03\x04\0\0zip")),
        ])
        .await;
        let original = history(vec![user(vec![
            text("read these"),
            document(server.url("/export.csv"), Some("text/csv")),
            document(server.url("/notepad.txt"), None),
        ])]);

        let resolved = prepared(&original, reader("custom:openai")).await;

        let contents = prompt_contents(&resolved);
        assert_text_document(
            &contents[1],
            "Name;Betrag\nMüller;12€\n",
            DocumentMediaType::CSV,
        );
        assert_text_document(&contents[2], "hi", DocumentMediaType::TXT);

        let archive = history(vec![user(vec![
            text("read this"),
            document(server.url("/archive.txt"), None),
        ])]);
        let error = failure(&archive, direct(reader("custom:openai"), TEST_POLICY)).await;
        assert!(error.contains("document (text/plain)"), "{error}");
        assert!(error.contains("host '127.0.0.1'"), "{error}");
        assert!(error.contains("NUL bytes"), "{error}");
        assert!(!error.contains("archive"), "{error}");
    }

    #[tokio::test]
    async fn text_over_the_text_limit_fails_the_call() {
        let server = serve(vec![("/server.log", Reply::ok(LARGE))]).await;
        let original = history(vec![user(vec![
            text("why did it fail?"),
            document(server.url("/server.log"), Some("text/plain")),
        ])]);

        let error = failure(&original, direct(reader("custom:anthropic"), TEST_POLICY)).await;

        assert!(
            error.contains("2048 bytes, over the 64-byte limit for text documents in one call"),
            "{error}"
        );
        assert!(
            error.contains("document (text/plain) at host '127.0.0.1'"),
            "{error}"
        );
        assert!(!error.contains("server.log"), "{error}");
    }

    fn takes(reader: MediaReader) -> [Take; 5] {
        [
            reader.images.take,
            reader.pdfs,
            reader.files.take,
            reader.audio.take,
            reader.video,
        ]
    }

    #[test]
    fn readers_follow_each_provider_and_api_surface() {
        use Take::{Inline as I, Link as L, Never as N};
        let azure_deployment = provider_with(
            "custom:openai",
            flow_like_types::json::json!({"is_azure": true, "endpoint": "https://acme.openai.azure.com"}),
        );
        let self_hosted = provider_with(
            "custom:openai",
            flow_like_types::json::json!({"endpoint": "http://10.0.0.5:8000/v1"}),
        );
        for (provider, expected) in [
            (provider("openai"), [L, L, L, N, N]),
            (provider("custom:openai"), [L, L, L, N, N]),
            (
                provider_with(
                    "custom:openai",
                    flow_like_types::json::json!({"endpoint": "https://eu.api.openai.com/v1"}),
                ),
                [L, L, L, N, N],
            ),
            (chat(provider("custom:openai")), [L, I, N, I, N]),
            (provider("hosted:openai"), [L, I, N, I, N]),
            (provider("azure"), [L, I, N, I, N]),
            (azure_deployment, [L, I, N, I, N]),
            (
                provider_with(
                    "custom:openai",
                    flow_like_types::json::json!({"endpoint": "https://acme.openai.azure.com/openai/v1"}),
                ),
                [L, L, N, N, N],
            ),
            (self_hosted.clone(), [I, N, N, N, N]),
            (chat(self_hosted), [I, N, N, I, N]),
            (provider("custom:bedrock"), [I, I, N, N, N]),
            (provider("hosted:bedrock"), [I, I, N, N, N]),
            (provider("anthropic"), [L, I, N, N, N]),
            (
                provider_with(
                    "custom:anthropic",
                    flow_like_types::json::json!({"endpoint": "https://api.anthropic.com"}),
                ),
                [L, I, N, N, N],
            ),
            (
                provider_with(
                    "custom:anthropic",
                    flow_like_types::json::json!({"endpoint": "https://claude.proxy.example/v1"}),
                ),
                [I, I, N, N, N],
            ),
            (provider("gemini"), [L, L, N, L, L]),
            (provider("custom:vertex"), [L, L, N, L, L]),
            (provider("hosted"), [L, L, N, I, I]),
            (provider("custom:openrouter"), [L, L, N, I, I]),
            (provider("mistral"), [L, L, N, I, N]),
            (provider("custom:perplexity"), [L, I, I, N, N]),
            (provider("xai"), [L, L, N, N, N]),
            (provider("deepseek"), [L, N, N, N, N]),
            (provider("custom:cohere"), [L, N, N, N, N]),
            (provider("groq"), [L, N, N, N, N]),
            (provider("together"), [L, N, N, N, N]),
            (provider("huggingface"), [L, N, N, N, N]),
            (provider("custom:moonshot"), [I, N, N, N, N]),
            (provider("ollama"), [I, N, N, N, N]),
            (provider("custom:lmstudio"), [I, N, N, N, N]),
            (provider("custom:mozilla"), [I, N, N, N, N]),
            (provider("hyperbolic"), [N, N, N, N, N]),
            (provider("custom:galadriel"), [N, N, N, N, N]),
            (provider("llama.cpp"), [N, N, N, N, N]),
            (provider("local"), [N, N, N, N, N]),
        ] {
            assert_eq!(
                takes(MediaReader::for_provider(&provider)),
                expected,
                "{provider:?}"
            );
        }

        for (name, types) in [
            ("custom:ollama", JPEG_PNG_WEBP),
            ("custom:lmstudio", JPEG_PNG),
            ("custom:mozilla", JPEG_PNG_GIF),
            ("custom:moonshot", MOONSHOT_IMAGES),
            ("custom:xai", JPEG_PNG),
            ("custom:anthropic", WEB_IMAGES),
        ] {
            assert_eq!(reader(name).images.types, Some(types), "{name}");
        }
        assert_eq!(reader("custom:perplexity").files.types, Some(WORD));
        assert_eq!(reader("custom:openai").files.types, Some(OOXML));
        assert_eq!(reader("gemini").images.types, None);
    }

    #[test]
    fn inline_caps_follow_each_provider() {
        for (name, kind, expected) in [
            ("custom:bedrock", Kind::Image, 3_750_000),
            ("custom:bedrock", Kind::Document, 4_500_000),
            ("anthropic", Kind::Image, 7_500_000),
            ("anthropic", Kind::Document, 20 * MIB),
            ("custom:vertex", Kind::Image, 7 * MB),
            ("custom:vertex", Kind::Document, 50 * MB),
            ("custom:vertex", Kind::Video, 20 * MB),
            ("gemini", Kind::Document, 50 * MB),
            ("gemini", Kind::Video, 64 * MIB),
            ("custom:huggingface", Kind::Image, 1_400_000),
            ("mistral", Kind::Image, 10 * MB),
            ("deepseek", Kind::Image, 32 * MIB),
            ("openai", Kind::Document, 50 * MB),
            ("azure", Kind::Image, 20 * MB),
            ("custom:ollama", Kind::Image, 20 * MIB),
        ] {
            assert_eq!(reader(name).caps.part(kind), expected, "{name} {kind:?}");
        }
        let claude_elsewhere = MediaReader::for_provider(&provider_with(
            "custom:anthropic",
            flow_like_types::json::json!({"endpoint": "https://bedrock-runtime.us-east-1.amazonaws.com"}),
        ));
        assert_eq!(claude_elsewhere.caps.part(Kind::Image), 3_750_000);
        assert_eq!(claude_elsewhere.caps.call, 14 * MB);
    }

    #[tokio::test]
    async fn each_provider_gets_inline_what_it_reads_from_links_it_cannot_reach() {
        let server = serve(vec![
            ("/photo.png", Reply::ok(PNG)),
            ("/report.pdf", Reply::ok(PDF)),
            ("/letter.docx", Reply::ok(DOCX)),
            ("/voice.mp3", Reply::ok(MP3)),
            ("/clip.mp4", Reply::ok(MP4)),
            ("/table.csv", Reply::ok(b"a,b\n")),
        ])
        .await;
        let original = history(vec![user(vec![
            text("read"),
            image(server.url("/photo.png"), None),
            document(server.url("/report.pdf"), Some("application/pdf")),
            document(server.url("/letter.docx"), Some(DOCX_TYPE)),
            audio(server.url("/voice.mp3"), "audio/mpeg"),
            video(server.url("/clip.mp4")),
            document(server.url("/table.csv"), Some("text/csv")),
        ])]);

        for (provider, expected) in [
            (
                provider("custom:openai"),
                [true, true, true, false, false, true],
            ),
            (
                chat(provider("custom:openai")),
                [true, true, false, true, false, true],
            ),
            (
                provider("custom:anthropic"),
                [true, true, false, false, false, true],
            ),
            (
                provider("custom:gemini"),
                [true, true, false, true, true, true],
            ),
            (
                provider("custom:vertex"),
                [true, true, false, true, true, true],
            ),
            (
                provider("custom:openrouter"),
                [true, true, false, true, true, true],
            ),
            (
                provider("custom:mistral"),
                [true, true, false, true, false, true],
            ),
            (
                provider("custom:perplexity"),
                [true, true, true, false, false, true],
            ),
            (
                provider("custom:deepseek"),
                [true, false, false, false, false, true],
            ),
            (
                provider("custom:bedrock"),
                [true, true, false, false, false, true],
            ),
            (
                provider("custom:ollama"),
                [true, false, false, false, false, true],
            ),
            (
                provider("custom:galadriel"),
                [false, false, false, false, false, true],
            ),
        ] {
            let resolved = prepared(&original, MediaReader::for_provider(&provider)).await;
            for (offset, inlined) in expected.into_iter().enumerate() {
                let part = offset + 1;
                let sent = url_at(&resolved, 0, part);
                if inlined {
                    assert!(
                        sent.starts_with("data:"),
                        "{provider:?} part {part}: {sent}"
                    );
                } else {
                    assert_eq!(sent, url_at(&original, 0, part), "{provider:?} part {part}");
                }
            }
        }
    }

    #[tokio::test]
    async fn public_links_stay_links_for_providers_that_fetch_them() {
        let links = history(vec![user(vec![
            text("look"),
            image(format!("{PUBLIC}/cat.png"), None),
            document(format!("{PUBLIC}/report.pdf"), Some("application/pdf")),
            audio(format!("{PUBLIC}/voice.mp3"), "audio/mpeg"),
            video(format!("{PUBLIC}/clip.mp4")),
        ])]);
        for name in ["custom:gemini", "custom:vertex"] {
            assert!(
                prepare(&links, reader(name))
                    .await
                    .expect("public links need no download")
                    .is_none(),
                "{name}"
            );
        }
        for name in [
            "custom:openai",
            "custom:openrouter",
            "custom:mistral",
            "xai",
        ] {
            let image_and_pdf = history(vec![user(vec![
                text("look"),
                image(format!("{PUBLIC}/cat.png"), None),
                document(format!("{PUBLIC}/report.pdf"), Some("application/pdf")),
            ])]);
            assert!(
                prepare(&image_and_pdf, reader(name))
                    .await
                    .expect("public links need no download")
                    .is_none(),
                "{name}"
            );
        }
        let youtube = history(vec![user(vec![
            text("watch"),
            video("https://www.youtube.com/watch?v=flow-like".to_string()),
        ])]);
        assert!(
            prepare(&youtube, reader("custom:openrouter"))
                .await
                .expect("YouTube videos stay links")
                .is_none()
        );
    }

    #[tokio::test]
    async fn gemini_and_vertex_get_links_they_cannot_reach_inline() {
        let server = serve(vec![
            ("/photo.png", Reply::ok(PNG)),
            ("/clip.mp4", Reply::ok(MP4)),
        ])
        .await;
        let original = history(vec![user(vec![
            text("describe"),
            image(server.url("/photo.png"), None),
            video(server.url("/clip.mp4")),
            image(format!("{PUBLIC}/cat.png"), Some("image/png")),
        ])]);

        for name in ["custom:gemini", "custom:vertex"] {
            let resolved = prepared(&original, reader(name)).await;
            assert_eq!(
                url_at(&resolved, 0, 1),
                format!("data:image/png;base64,{}", STANDARD.encode(PNG)),
                "{name}"
            );
            assert_eq!(
                url_at(&resolved, 0, 2),
                format!("data:video/mp4;base64,{}", STANDARD.encode(MP4)),
                "{name}"
            );
            assert_eq!(
                url_at(&resolved, 0, 3),
                format!("{PUBLIC}/cat.png"),
                "{name}"
            );
        }

        let gone = history(vec![user(vec![
            text("describe"),
            image(server.url("/gone.png"), None),
        ])]);
        let error = failure(&gone, direct(reader("custom:gemini"), TEST_POLICY)).await;
        assert!(
            error.contains("image (image/png) at host '127.0.0.1'"),
            "{error}"
        );
        assert!(error.contains("HTTP 404"), "{error}");
        assert!(!error.contains("gone"), "{error}");
    }

    #[tokio::test]
    async fn image_types_a_provider_cannot_read_are_left_to_its_client() {
        let server = serve(vec![
            ("/anim.gif", Reply::ok(b"GIF89a")),
            ("/render", Reply::Typed("image/png", PNG)),
            ("/photo.webp", Reply::ok(b"RIFF webp")),
        ])
        .await;
        let original = history(vec![user(vec![
            text("look"),
            image(server.url("/anim.gif"), None),
            image(server.url("/render?size=2"), None),
            image(server.url("/photo.webp"), None),
        ])]);

        let for_ollama = prepared(&original, reader("custom:ollama")).await;

        assert_eq!(url_at(&for_ollama, 0, 1), server.url("/anim.gif"));
        assert_eq!(
            url_at(&for_ollama, 0, 2),
            format!("data:image/png;base64,{}", STANDARD.encode(PNG)),
            "an image only its response types is downloaded and typed by it"
        );
        assert!(url_at(&for_ollama, 0, 3).starts_with("data:image/webp;base64,"));
        assert_eq!(server.hits(), 2);

        let for_lm_studio = prepared(&original, reader("custom:lmstudio")).await;
        assert_eq!(url_at(&for_lm_studio, 0, 3), server.url("/photo.webp"));
    }

    #[test]
    fn a_content_type_names_an_image_by_its_essence() {
        for (content_type, expected) in [
            ("image/JPG; q=1", Some(ImageMediaType::JPEG)),
            (" image/png ", Some(ImageMediaType::PNG)),
            ("image/svg+xml;charset=utf-8", Some(ImageMediaType::SVG)),
            ("application/octet-stream", None),
            ("", None),
        ] {
            assert_eq!(image_type(content_type), expected, "{content_type}");
        }
    }

    #[tokio::test]
    async fn a_provider_can_reach_only_public_https_hosts() {
        for (url, reachable) in [
            ("https://93.184.216.34/cat.png", true),
            ("http://93.184.216.34/cat.png", false),
            ("https://127.0.0.1/cat.png", false),
            ("https://10.0.0.5/cat.png", false),
            ("https://localhost/cat.png", false),
            ("asset://localhost/%2FUsers%2Fme%2Fcat.png", false),
        ] {
            assert_eq!(publicly_reachable(url).await, reachable, "{url}");
        }
    }

    #[tokio::test]
    async fn a_source_decides_what_becomes_of_the_links_a_provider_reads() {
        let server = serve(vec![
            ("/keep.png", Reply::ok(PNG)),
            ("/move.png", Reply::ok(PNG)),
            ("/render", Reply::Typed("Image/WebP; charset=binary", PNG)),
            ("/clip.mp4", Reply::ok(MP4)),
            ("/scan.pdf", Reply::ok(PDF)),
        ])
        .await;
        let keep = server.url("/keep.png");
        let original = history(vec![user(vec![
            text("compare"),
            image(keep.clone(), None),
            image(server.url("/move.png?X-Amz-Signature=old"), None),
            image(server.url("/render?X-Amz-Signature=secret"), None),
            video(server.url("/clip.mp4")),
            document(server.url("/scan.pdf"), None),
        ])]);

        let resolved = deciding(reader("custom:gemini"))
            .history(&original)
            .await
            .expect("every part resolves")
            .expect("the source changed parts");

        assert_eq!(url_at(&resolved, 0, 1), keep);
        assert_eq!(url_at(&resolved, 0, 2), "https://moved.example/move.png");
        for (part, expected) in [
            (
                3,
                format!("data:image/webp;base64,{}", STANDARD.encode(PNG)),
            ),
            (4, format!("data:video/mp4;base64,{}", STANDARD.encode(MP4))),
            (
                5,
                format!("data:application/pdf;base64,{}", STANDARD.encode(PDF)),
            ),
        ] {
            assert_eq!(url_at(&resolved, 0, part), expected);
        }
        assert_eq!(server.hits(), 3, "kept and moved links are never fetched");
    }

    #[tokio::test]
    async fn a_call_fails_on_media_it_cannot_send_naming_kind_type_and_host() {
        let server = serve(vec![
            ("/photo.png", Reply::ok(PNG)),
            ("/untyped", Reply::ok(PNG)),
        ])
        .await;
        for (part, described, reason) in [
            (
                image(server.url("/gone.png"), None),
                "image (image/png)",
                "HTTP 404",
            ),
            (
                image(server.url("/untyped"), None),
                "image (type unknown)",
                "names no image type",
            ),
            (
                document(server.url("/gone.pdf"), Some("text/plain")),
                "document (text/plain)",
                "HTTP 404",
            ),
            (
                image(server.url("/refuse.png"), None),
                "image (image/png)",
                "the source refuses it",
            ),
        ] {
            let original = history(vec![user(vec![
                text("look"),
                image(server.url("/photo.png"), None),
                part,
            ])]);

            let error = failure(&original, deciding(reader("custom:anthropic"))).await;

            assert!(error.contains(reason), "{error}");
            assert!(
                error.contains(&format!("{described} at host '127.0.0.1'")),
                "{error}"
            );
            for path in ["/gone", "/untyped", "/refuse"] {
                assert!(!error.contains(path), "{error}");
            }
        }
    }

    #[tokio::test]
    async fn media_over_the_provider_caps_fails_the_call() {
        let server = serve(vec![
            ("/huge.png", Reply::Declared(4_000_000)),
            ("/first.png", Reply::ok(PNG)),
            ("/second.png", Reply::ok(PNG)),
        ])
        .await;
        let huge = history(vec![user(vec![
            text("look"),
            image(server.url("/huge.png"), None),
        ])]);
        let error = failure(&huge, direct(reader("custom:bedrock"), TEST_POLICY)).await;
        assert!(
            error.contains(
                "The image (image/png) at host '127.0.0.1' could not be sent to the model inline: \
                 it is 4000000 bytes, over the 3750000-byte limit for one image sent inline"
            ),
            "{error}"
        );

        let both = history(vec![user(vec![
            text("both"),
            image(server.url("/first.png"), None),
            image(server.url("/second.png"), None),
        ])]);
        let small = MediaReader {
            caps: InlineCaps::total(PNG.len() as u64 + 8),
            ..reader("custom:bedrock")
        };
        let error = failure(&both, direct(small, TEST_POLICY)).await;
        assert!(
            error.contains(&format!(
                "its {} bytes exceed the 8 bytes left of this call's {}-byte inline limit",
                PNG.len(),
                PNG.len() + 8
            )),
            "{error}"
        );

        let one_part = InlinePolicy {
            max_parts: 1,
            ..TEST_POLICY
        };
        let error = failure(&both, direct(reader("custom:bedrock"), one_part)).await;
        assert!(
            error.contains("2 media parts to send inline, more than the 1"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn media_a_provider_cannot_read_is_left_alone() {
        let server = serve(vec![]).await;
        let original = history(vec![user(vec![
            text("look"),
            video(server.url("/clip.mp4")),
            document(server.url("/letter.docx"), Some(DOCX_TYPE)),
            document(server.url("/blob"), Some("application/octet-stream")),
            audio(server.url("/memo.m4a"), "audio/mp4"),
            document(
                format!("data:application/pdf;base64,{}", STANDARD.encode(PDF)),
                Some("application/pdf"),
            ),
            image("gs://bucket/cat.png".to_string(), None),
        ])]);

        let resolved = prepare(&original, reader("custom:anthropic"))
            .await
            .expect("nothing is downloaded");

        assert!(resolved.is_none());
        assert_eq!(server.hits(), 0);
    }

    #[tokio::test]
    async fn failed_downloads_are_cached_and_fail_every_call() {
        let server = serve(vec![("/ok.pdf", Reply::ok(PDF))]).await;
        let original = history(vec![user(vec![
            text("read"),
            document(server.url("/ok.pdf"), None),
            document(server.url("/missing.pdf"), None),
        ])]);

        for call in 0..2 {
            let error = failure(&original, direct(reader("custom:anthropic"), TEST_POLICY)).await;
            assert!(error.contains("HTTP 404"), "call {call}: {error}");
            assert!(
                error.contains("document (application/pdf) at host '127.0.0.1'"),
                "call {call}: {error}"
            );
            assert_eq!(server.hits(), 2, "call {call} requests nothing again");
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
        let original = history(vec![user(vec![
            text("read"),
            document(server.url("/stalled.pdf"), None),
            document(server.url("/ok.pdf"), None),
        ])]);

        let started = std::time::Instant::now();
        let error = failure(&original, direct(reader("custom:anthropic"), policy)).await;

        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            error.contains("inline deadline of this call passed"),
            "{error}"
        );
        failure(&original, direct(reader("custom:anthropic"), policy)).await;
        assert_eq!(
            server.hits(),
            3,
            "a download the deadline cut short is not cached as failed"
        );
    }

    #[tokio::test]
    async fn responses_not_served_over_https_are_refused_where_http_is_not_fetched() {
        let server = serve(vec![
            ("/start.pdf", Reply::Redirect("/report.pdf")),
            ("/report.pdf", Reply::ok(PDF)),
        ])
        .await;
        let candidate = Candidate {
            message: 0,
            part: 0,
            url: server.url("/start.pdf"),
            kind: Kind::Document,
            mime: Some(PDF_TYPE.to_string()),
            class: MediaClass::Binary,
            link: false,
        };
        let source = EnvironmentSource::new(ExecutionEnvironment::Local);
        let local = MediaFetcher::new(&source, TEST_POLICY, DEFAULT_CAPS);
        let remote = MediaFetcher::new(
            &source,
            InlinePolicy::direct(ExecutionEnvironment::BrowserSandbox),
            DEFAULT_CAPS,
        );

        let followed = local
            .download(&candidate, 1024, &mut local.charge(MediaClass::Binary))
            .await
            .expect("the redirect is followed");
        let refused = remote
            .download(&candidate, 1024, &mut remote.charge(MediaClass::Binary))
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
        let preparation = MediaPreparation {
            source: Arc::new(EnvironmentSource::new(ExecutionEnvironment::Server)),
            reader: reader("custom:anthropic"),
            policy: TEST_POLICY,
        };

        let error = failure(&original, preparation).await;

        assert!(error.contains("host '127.0.0.1'"), "{error}");
        assert!(error.contains("refused"), "{error}");
        assert_eq!(server.hits(), 0);
    }

    #[tokio::test]
    async fn plain_http_is_fetched_where_http_nodes_fetch_it() {
        for (environment, fetched) in [
            (ExecutionEnvironment::Local, true),
            (ExecutionEnvironment::Desktop, true),
            (ExecutionEnvironment::Mobile, true),
            (ExecutionEnvironment::Server, true),
            (ExecutionEnvironment::BrowserSandbox, false),
        ] {
            assert_eq!(
                InlinePolicy::direct(environment).allow_http,
                fetched,
                "{environment:?}"
            );
        }
        let policy = InlinePolicy::direct(ExecutionEnvironment::Local);
        assert!(!policy.fetches("http://asset.localhost/%2FUsers%2Fme%2Fcat.png"));
        assert!(!policy.fetches("asset://localhost/%2FUsers%2Fme%2Fcat.png"));

        let server = serve(vec![("/report.pdf", Reply::ok(PDF))]).await;
        let original = history(vec![user(vec![
            text("read"),
            document(server.url("/report.pdf"), Some("application/pdf")),
        ])]);
        let on_server = MediaPreparation {
            source: Arc::new(EnvironmentSource::new(ExecutionEnvironment::Server)),
            reader: reader("custom:anthropic"),
            policy: InlinePolicy::direct(ExecutionEnvironment::Server),
        };
        let error = failure(&original, on_server).await;
        assert!(
            error.contains("refused"),
            "the server's egress guard refuses loopback: {error}"
        );
        assert_eq!(server.hits(), 0);
        let on_desktop = direct(
            reader("custom:anthropic"),
            InlinePolicy::direct(ExecutionEnvironment::Desktop),
        );
        let resolved = on_desktop.history(&original).await.unwrap().unwrap();
        assert!(url_at(&resolved, 0, 1).starts_with("data:application/pdf;base64,"));
    }

    /// A store of a server's storage credentials at `endpoint`, built as the executor builds it.
    pub(crate) fn s3_store(endpoint: &str) -> FlowLikeStore {
        FlowLikeStore::AWS(Arc::new(
            flow_like_storage::object_store::aws::AmazonS3Builder::new()
                .with_bucket_name("content")
                .with_region("us-east-1")
                .with_endpoint(endpoint)
                .with_allow_http(true)
                .with_access_key_id("test-access-key")
                .with_secret_access_key("test-secret-key")
                .build()
                .expect("an S3-compatible store"),
        ))
    }

    async fn own_store(endpoint: &str) -> OwnStorage {
        let own = OwnStorage::signed_by([s3_store(endpoint)]).await;
        assert!(!own.is_empty(), "{endpoint} signs links");
        own
    }

    /// A server call whose storage credentials name `own_storage`.
    fn on_server(own_storage: OwnStorage) -> MediaPreparation {
        MediaPreparation {
            source: Arc::new(EnvironmentSource::reading(
                ExecutionEnvironment::Server,
                Vec::new(),
                own_storage,
            )),
            reader: reader("custom:anthropic"),
            policy: InlinePolicy::direct(ExecutionEnvironment::Server),
        }
    }

    fn pdf_at(url: String) -> History {
        history(vec![user(vec![
            text("read"),
            document(url, Some(PDF_TYPE)),
        ])])
    }

    #[tokio::test]
    async fn a_server_fetches_links_into_its_own_store_even_over_plain_http() {
        let store = serve(vec![
            ("/content/tmp/user/me/report.pdf", Reply::ok(PDF)),
            ("/content/tmp/user/me/photo.png", Reply::ok(PNG)),
        ])
        .await;
        let own = own_store(&store.base).await;
        let call = history(vec![user(vec![
            text("read"),
            document(
                store.url("/content/tmp/user/me/report.pdf?X-Amz-Signature=secret"),
                Some(PDF_TYPE),
            ),
            image(
                store.url("/content/tmp/user/me/photo.png?X-Amz-Signature=secret"),
                None,
            ),
        ])]);

        let resolved = on_server(own.clone())
            .history(&call)
            .await
            .expect("links into the own store are fetched")
            .expect("parts changed");

        assert_eq!(
            url_at(&resolved, 0, 1),
            format!("data:application/pdf;base64,{}", STANDARD.encode(PDF))
        );
        assert_eq!(
            url_at(&resolved, 0, 2),
            format!("data:image/png;base64,{}", STANDARD.encode(PNG))
        );
        assert_eq!(store.hits(), 2);
        let error = failure(&call, on_server(OwnStorage::default())).await;
        assert!(
            error.contains("refused"),
            "a server whose credentials name another store fetches through its egress guard, \
             which refuses loopback: {error}"
        );
        assert_eq!(store.hits(), 2);
        assert_ne!(
            EnvironmentSource::reading(ExecutionEnvironment::Server, Vec::new(), own).cache_scope(),
            EnvironmentSource::new(ExecutionEnvironment::Server).cache_scope(),
            "downloads that may come from the store are cached apart"
        );
    }

    #[tokio::test]
    async fn a_server_reads_only_its_own_store_past_its_egress_guard() {
        let store = serve(vec![("/content/report.pdf", Reply::ok(PDF))]).await;
        let other = serve(vec![("/content/report.pdf", Reply::ok(PDF))]).await;
        let own = own_store(&store.base).await;
        let authority = store.base.trim_start_matches("http://");
        let port = authority.rsplit(':').next().expect("a port");
        let source =
            EnvironmentSource::reading(ExecutionEnvironment::Server, Vec::new(), own.clone());
        let on_loopback = [
            other.url("/content/report.pdf"),
            store.url("/other-bucket/report.pdf"),
            store.url("/minio/v2/metrics/cluster"),
            store.url("/content/a%2F..%2F..%2Fother-bucket/report.pdf"),
            format!("http://localhost:{port}/content/report.pdf"),
            format!("http://key:secret@{authority}/content/report.pdf"),
        ];

        for link in on_loopback.iter().cloned().chain([
            "http://minio:9000/content/report.pdf".to_string(),
            "http://10.0.0.5:9000/content/report.pdf".to_string(),
        ]) {
            assert!(!source.owns(&link), "{link}");
        }
        for link in on_loopback {
            let error = failure(&pdf_at(link.clone()), on_server(own.clone())).await;
            assert!(error.contains("refused"), "{link}: {error}");
            assert!(!error.contains("secret"), "{error}");
        }
        assert_eq!(store.hits() + other.hits(), 0);
    }

    #[tokio::test]
    async fn a_server_follows_no_redirect_of_its_own_store() {
        let store = serve(vec![
            ("/content/moved.pdf", Reply::Redirect("/content/report.pdf")),
            (
                "/content/metadata.pdf",
                Reply::Redirect("http://169.254.169.254/latest/meta-data/"),
            ),
            ("/content/report.pdf", Reply::ok(PDF)),
        ])
        .await;
        let own = own_store(&store.base).await;

        for (requests, path) in [(1, "/content/moved.pdf"), (2, "/content/metadata.pdf")] {
            let call = pdf_at(store.url(&format!("{path}?X-Amz-Signature=secret")));

            let error = failure(&call, on_server(own.clone())).await;

            assert!(error.contains("HTTP 302"), "{error}");
            assert!(
                error.contains("document (application/pdf) at host '127.0.0.1'"),
                "{error}"
            );
            for hidden in ["content/", "secret", "meta-data"] {
                assert!(!error.contains(hidden), "{error}");
            }
            assert_eq!(
                store.hits(),
                requests,
                "only the redirect itself is requested"
            );
        }
    }

    #[tokio::test]
    async fn a_store_on_the_host_plane_is_refused_even_when_configured() {
        for (endpoint, host) in [
            ("http://169.254.169.254", "169.254.169.254"),
            ("http://169.254.170.2:9000", "169.254.170.2"),
            ("http://[fd00:ec2::254]", "[fd00:ec2::254]"),
            ("http://0.0.0.0:9000", "0.0.0.0"),
            (
                "http://metadata.google.internal",
                "metadata.google.internal",
            ),
            ("http://host.docker.internal:9000", "host.docker.internal"),
        ] {
            let own = own_store(endpoint).await;
            let call = pdf_at(format!(
                "{endpoint}/content/latest/meta-data?X-Amz-Signature=secret"
            ));

            let error = failure(&call, on_server(own)).await;

            assert!(error.contains("refused"), "{error}");
            assert!(
                error.contains(&format!("storage host '{host}' is on this executor's own")),
                "{error}"
            );
            assert!(
                !error.contains("meta-data") && !error.contains("secret"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_store_name_that_resolves_onto_the_host_plane_is_refused() {
        let at = |ip: &str| SocketAddr::new(ip.parse().expect("an address"), 0);
        assert!(storage_addresses("rustfs", [at("10.0.0.5")]).is_ok());
        assert!(
            storage_addresses(
                "s3.localhost",
                [at("127.0.0.1"), at("::1"), at("::ffff:127.0.0.1")]
            )
            .is_ok(),
            "a store beside the executor answers on loopback"
        );
        for refused in [
            "169.254.169.254",
            "fd00:ec2::254",
            "0.0.0.0",
            "::ffff:169.254.169.254",
        ] {
            let error = storage_addresses("rebound.example", [at("10.0.0.5"), at(refused)])
                .expect_err(refused);
            assert!(
                error.contains("'rebound.example' resolves to") && error.contains(refused),
                "{error}"
            );
        }
        assert!(storage_addresses("nowhere.example", []).is_err());
    }

    #[tokio::test]
    async fn a_store_named_on_loopback_is_fetched_and_its_failures_name_no_link() {
        let store = serve(vec![("/content/report.pdf", Reply::ok(PDF))]).await;
        let port = store.base.rsplit(':').next().expect("a port");
        let named = format!("http://localhost:{port}");
        let resolved = on_server(own_store(&named).await)
            .history(&pdf_at(format!("{named}/content/report.pdf")))
            .await
            .expect("a store named on loopback is fetched")
            .expect("parts changed");
        assert!(url_at(&resolved, 0, 1).starts_with("data:application/pdf;base64,"));

        let closed = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind a port")
            .local_addr()
            .expect("its address");
        let down = format!("http://{closed}");
        let call = pdf_at(format!("{down}/content/report.pdf?X-Amz-Signature=secret"));
        let error = failure(&call, on_server(own_store(&down).await)).await;
        assert!(
            error.contains("GET failed: error sending request"),
            "{error}"
        );
        assert!(error.contains("tcp connect error"), "{error}");
        assert!(
            !error.contains("content/") && !error.contains("secret"),
            "{error}"
        );
    }

    #[test]
    fn a_store_scope_is_read_from_its_probe_link() {
        let url = |raw: &str| Url::parse(raw).expect("a URL");
        assert_eq!(
            StorageScope::from_probe(&url(&format!(
                "http://rustfs:9000/content/{PROBE_OBJECT}?X-Amz-Signature=secret"
            ))),
            Some(StorageScope {
                origin: url("http://rustfs:9000").origin(),
                path_prefix: "/content/".to_owned(),
            })
        );
        assert_eq!(
            StorageScope::from_probe(&url(&format!(
                "https://content.s3.us-east-1.amazonaws.com/{PROBE_OBJECT}"
            ))),
            Some(StorageScope {
                origin: url("https://content.s3.us-east-1.amazonaws.com").origin(),
                path_prefix: "/".to_owned(),
            })
        );
        for probe in [
            "http://rustfs:9000/content/other-object".to_owned(),
            format!("http://rustfs:9000/content/x{PROBE_OBJECT}"),
            format!("asset://localhost/content/{PROBE_OBJECT}"),
        ] {
            assert_eq!(StorageScope::from_probe(&url(&probe)), None, "{probe}");
        }
    }

    #[tokio::test]
    async fn only_object_keys_under_the_stores_bucket_link_into_it() {
        let endpoint = "http://rustfs:9000";
        let own = own_store(endpoint).await;
        let fixture = signed_link(
            &s3_store(endpoint),
            "tmp/user/me/Übersicht (2)#1.pdf",
            Duration::from_secs(600),
        )
        .await;
        assert!(own.contains(&fixture), "{fixture}");
        for owned in [
            "http://rustfs:9000/content/tmp/user/me/report.pdf?X-Amz-Signature=secret",
            "http://rustfs:9000/content/apps/app-1/storage/a%20b%23c.pdf",
        ] {
            assert!(own.contains(&Url::parse(owned).unwrap()), "{owned}");
        }
        for foreign in [
            "http://rustfs:9000/other-bucket/x.txt",
            "http://rustfs:9000/minio/v2/metrics/cluster",
            "http://rustfs:9000/contents/x.txt",
            "http://rustfs:9000/content",
            "http://rustfs:9000/content/",
            "http://rustfs:9000/content//x.txt",
            "http://rustfs:9000/content/x.txt/",
            "http://rustfs:9000/content/a%2F..%2F..%2Fother-bucket/x.txt",
            "http://rustfs:9000/content/a%5C..%5Cx.txt",
            "http://rustfs:9000/content/a%00.txt",
            "http://rustfs:9000/content/a/%2E%2E%2Fx.txt",
            "http://key:secret@rustfs:9000/content/x.txt",
            "https://rustfs:9000/content/x.txt",
            "http://rustfs:9001/content/x.txt",
        ] {
            assert!(!own.contains(&Url::parse(foreign).unwrap()), "{foreign}");
        }
        for dotted in ["a/./b", "a/../b", "%2e/b", "a/%2E%2e"] {
            assert!(!is_object_key(dotted), "{dotted}");
        }
    }

    async fn signed_link(store: &FlowLikeStore, key: &str, ttl: Duration) -> Url {
        store
            .sign("GET", &flow_like_storage::Path::from(key), ttl)
            .await
            .expect("a signed link")
    }

    fn expiry(link: &Url) -> Option<String> {
        link.query_pairs()
            .find(|(name, _)| name == "X-Amz-Expires")
            .map(|(_, value)| value.into_owned())
    }

    const WEEK: Duration = Duration::from_secs(7 * 24 * 60 * 60);

    /// The own store at `endpoint`, taken to be one a provider reaches, as it reaches public
    /// HTTPS.
    async fn reachable_store(endpoint: &str) -> OwnStorage {
        let mut own = own_store(endpoint).await;
        for store in &mut own.stores {
            store.reachable = OnceCell::new_with(Some(true));
        }
        own
    }

    /// A server call through `own_storage` to a provider that fetches image and PDF links.
    fn linking_on_server(own_storage: OwnStorage) -> MediaPreparation {
        MediaPreparation {
            source: Arc::new(EnvironmentSource::reading(
                ExecutionEnvironment::Server,
                Vec::new(),
                own_storage,
            )),
            reader: reader("custom:openai"),
            policy: InlinePolicy::direct(ExecutionEnvironment::Server),
        }
    }

    #[tokio::test]
    async fn a_provider_fetches_links_into_the_own_store_signed_again_to_expire_soon() {
        let photo_key = "tmp/user/me/Übersicht (2)#1.png";
        let photo_path = signed_link(&s3_store("http://127.0.0.1"), photo_key, PROBE_TTL)
            .await
            .path()
            .to_owned();
        let store = serve(vec![
            (photo_path.as_str(), Reply::ok(PNG)),
            ("/content/apps/app-1/storage/report.pdf", Reply::ok(PDF)),
        ])
        .await;
        let signer = s3_store(&store.base);
        let photo = signed_link(&signer, photo_key, WEEK).await;
        let report = signed_link(&signer, "apps/app-1/storage/report.pdf", WEEK).await;
        let foreign = format!("{PUBLIC}/other-bucket/cat.png");
        let call = history(vec![user(vec![
            text("look"),
            image(photo.to_string(), None),
            document(report.to_string(), Some(PDF_TYPE)),
            image(foreign.clone(), None),
        ])]);

        let resolved = linking_on_server(reachable_store(&store.base).await)
            .history(&call)
            .await
            .expect("nothing is downloaded")
            .expect("parts changed");

        for (part, held) in [(1, &photo), (2, &report)] {
            let sent = Url::parse(url_at(&resolved, 0, part)).expect("a link");
            assert_eq!(
                (sent.origin(), sent.path()),
                (held.origin(), held.path()),
                "part {part} names the same object"
            );
            assert_eq!(expiry(held).as_deref(), Some("604800"));
            assert_eq!(expiry(&sent).as_deref(), Some("300"), "part {part}");
        }
        assert_eq!(
            url_at(&resolved, 0, 3),
            foreign,
            "a public link into another bucket stays as it is"
        );
        assert_eq!(
            store.hits(),
            2,
            "one byte is read of each link signed again"
        );
    }

    #[tokio::test]
    async fn a_link_signed_again_that_the_store_refuses_is_read_inline_with_the_runs_link() {
        let key = "apps/other-app/storage/photo.png";
        let store = serve(vec![(
            "/content/apps/other-app/storage/photo.png",
            Reply::Refusing("X-Amz-Expires=300", PNG),
        )])
        .await;
        let held = signed_link(&s3_store(&store.base), key, WEEK).await;
        let call = history(vec![user(vec![
            text("look"),
            image(held.to_string(), None),
        ])]);

        let resolved = linking_on_server(reachable_store(&store.base).await)
            .history(&call)
            .await
            .expect("the media is read with the run's link")
            .expect("parts changed");

        assert_eq!(
            url_at(&resolved, 0, 1),
            format!("data:image/png;base64,{}", STANDARD.encode(PNG))
        );
        assert_eq!(
            store.hits(),
            2,
            "the store refused the link signed again and served the run's own"
        );
    }

    #[tokio::test]
    async fn a_link_into_the_own_store_that_cannot_be_signed_again_is_sent_inline() {
        let own = OwnStorage::signed_by([s3_store(PUBLIC)]).await;
        let unsigned = OwnStorage {
            stores: own
                .stores
                .iter()
                .map(|known| {
                    OwnStore::new(
                        known.scope.clone(),
                        FlowLikeStore::Memory(Arc::new(
                            flow_like_storage::object_store::memory::InMemory::new(),
                        )),
                    )
                })
                .collect(),
        };
        let photo = format!("{PUBLIC}/content/tmp/user/me/photo.png?X-Amz-Expires=604800");

        for (own, link, why) in [
            (
                own,
                format!("{PUBLIC}/content/a(1).png"),
                "it signs another form",
            ),
            (unsigned, photo, "it cannot sign"),
        ] {
            let source = EnvironmentSource::reading(ExecutionEnvironment::Server, Vec::new(), own);
            assert_eq!(source.link(&link).await.unwrap(), Link::Inline, "{why}");
        }
    }

    #[tokio::test]
    async fn a_relayed_call_sends_private_media_inline_and_every_other_link_as_held() {
        let intranet = serve(vec![
            ("/scans/cat.png", Reply::ok(PNG)),
            ("/reports/q3.pdf", Reply::ok(PDF)),
            ("/reports/big.pdf", Reply::Declared(5_000_000)),
        ])
        .await;
        let store = serve(vec![]).await;
        let call = history(vec![user(vec![
            text("read"),
            image(intranet.url("/scans/cat.png"), None),
            document(intranet.url("/reports/q3.pdf"), Some(PDF_TYPE)),
            image(format!("{PUBLIC}/cat.png"), None),
            document(format!("{PUBLIC}/report.pdf"), Some(PDF_TYPE)),
            document(
                store.url("/content/tmp/user/me/report.pdf?X-Amz-Signature=secret"),
                Some(PDF_TYPE),
            ),
        ])]);
        let model = RemoteMediaModel::relayed(
            Arc::new(RecordingModel::default()),
            ExecutionEnvironment::Local,
            reader("custom:anthropic"),
            own_store(&store.base).await,
        );

        let resolved = model
            .media
            .history(&call)
            .await
            .expect("the private media is fetched")
            .expect("parts changed");

        for (part, inline) in [
            (1, format!("data:image/png;base64,{}", STANDARD.encode(PNG))),
            (
                2,
                format!("data:application/pdf;base64,{}", STANDARD.encode(PDF)),
            ),
        ] {
            assert_eq!(
                url_at(&resolved, 0, part),
                inline,
                "no provider and not the relay reaches a private host"
            );
        }
        for part in 3..=5 {
            assert_eq!(
                url_at(&resolved, 0, part),
                url_at(&call, 0, part),
                "the relay resolves public links and links into its storage"
            );
        }
        assert_eq!((intranet.hits(), store.hits()), (2, 0));
        assert_eq!(model.media.reader.caps.call, RELAY_CAPS.call);

        let error = model
            .media
            .history(&pdf_at(intranet.url("/reports/big.pdf")))
            .await
            .expect_err("the PDF is over the relay's inline limit")
            .to_string();
        assert!(
            error.contains("document (application/pdf) at host '127.0.0.1'"),
            "{error}"
        );
        assert!(error.contains("over the 4000000-byte limit"), "{error}");
        assert!(!error.contains("big.pdf"), "{error}");
    }

    #[tokio::test]
    async fn downloads_hold_their_declared_length_of_the_call_budget_before_the_body_arrives() {
        let server = serve(vec![
            ("/first.pdf", Reply::ok(LARGE)),
            ("/second.pdf", Reply::ok(LARGE)),
            ("/withheld.pdf", Reply::Withheld(LARGE.len() as u64)),
        ])
        .await;
        let part = LARGE.len() as u64;
        let budget = |call: u64| MediaReader {
            caps: InlineCaps::total(call),
            ..reader("custom:anthropic")
        };
        let held_back = history(vec![user(vec![
            text("read"),
            document(server.url("/first.pdf"), Some(PDF_TYPE)),
            document(server.url("/withheld.pdf"), Some(PDF_TYPE)),
        ])]);

        let started = std::time::Instant::now();
        let error = failure(&held_back, direct(budget(part + 952), TEST_POLICY)).await;

        assert!(started.elapsed() < Duration::from_secs(3), "{error}");
        assert!(
            error.contains(&format!(
                "its {part} bytes exceed the 952 bytes left of this call's {}-byte inline limit",
                part + 952
            )),
            "{error}"
        );
        let fitting = history(vec![user(vec![
            text("read"),
            document(server.url("/first.pdf"), Some(PDF_TYPE)),
            document(server.url("/second.pdf"), Some(PDF_TYPE)),
        ])]);
        let resolved = direct(budget(2 * part), TEST_POLICY)
            .history(&fitting)
            .await
            .expect("parts that fill the budget exactly are sent")
            .expect("parts changed");
        assert!(url_at(&resolved, 0, 2).starts_with("data:application/pdf;base64,"));
    }

    fn asset_link(base: &str, path: &std::path::Path) -> String {
        let path = path.to_str().expect("UTF-8 path");
        let encoded =
            percent_encoding::utf8_percent_encode(path, percent_encoding::NON_ALPHANUMERIC);
        format!("{base}{encoded}")
    }

    #[test]
    fn asset_links_name_the_files_of_this_machine() {
        for (url, path) in [
            (
                "asset://localhost/%2FUsers%2Fme%2Fcat.png",
                Some("/Users/me/cat.png"),
            ),
            (
                "asset://localhost/Users/me/report%20(2).pdf?filename=a.pdf",
                Some("/Users/me/report (2).pdf"),
            ),
            (
                "http://asset.localhost/C%3A%5CUsers%5Cme%5Ccat.png",
                Some("C:\\Users\\me\\cat.png"),
            ),
            ("https://example.com/cat.png", None),
            ("asset://elsewhere/%2Fetc%2Fpasswd", None),
            ("asset://localhost/", None),
        ] {
            assert_eq!(asset_path(url), path.map(PathBuf::from), "{url}");
        }
    }

    /// A desktop call that reads the files under `roots`.
    fn on_desktop(roots: Vec<PathBuf>) -> MediaPreparation {
        MediaPreparation {
            source: Arc::new(EnvironmentSource {
                storage_roots: roots,
                ..EnvironmentSource::new(ExecutionEnvironment::Desktop)
            }),
            reader: reader("custom:anthropic"),
            policy: InlinePolicy::direct(ExecutionEnvironment::Desktop),
        }
    }

    #[tokio::test]
    async fn desktop_asset_links_into_the_apps_storage_are_read_from_disk() {
        let storage = tempfile::tempdir().expect("storage");
        std::fs::write(storage.path().join("report.pdf"), PDF).expect("stored file");
        std::fs::write(storage.path().join("cat.png"), PNG).expect("stored image");
        let original = history(vec![user(vec![
            text("read"),
            document(
                asset_link("asset://localhost/", &storage.path().join("report.pdf")),
                Some("application/pdf"),
            ),
            image(
                asset_link("http://asset.localhost/", &storage.path().join("cat.png")),
                None,
            ),
        ])]);

        let resolved = on_desktop(vec![storage.path().to_path_buf()])
            .history(&original)
            .await
            .expect("files in storage are read")
            .expect("parts changed");

        assert_eq!(
            url_at(&resolved, 0, 1),
            format!("data:application/pdf;base64,{}", STANDARD.encode(PDF))
        );
        assert_eq!(
            url_at(&resolved, 0, 2),
            format!("data:image/png;base64,{}", STANDARD.encode(PNG))
        );
        assert!(
            on_desktop(Vec::new())
                .history(&original)
                .await
                .expect("without storage no file is read")
                .is_none()
        );
    }

    #[tokio::test]
    async fn desktop_asset_links_outside_the_apps_storage_fail_the_call() {
        let storage = tempfile::tempdir().expect("storage");
        let elsewhere = tempfile::tempdir().expect("elsewhere");
        std::fs::write(elsewhere.path().join("secret.pdf"), PDF).expect("outside file");
        let traversal = storage.path().join("..").join(
            elsewhere
                .path()
                .file_name()
                .expect("temporary directory name"),
        );
        for outside in [
            elsewhere.path().join("secret.pdf"),
            traversal.join("secret.pdf"),
        ] {
            let call = history(vec![user(vec![
                text("read"),
                document(
                    asset_link("asset://localhost/", &outside),
                    Some("application/pdf"),
                ),
            ])]);
            let error = failure(&call, on_desktop(vec![storage.path().to_path_buf()])).await;
            assert!(error.contains("outside this app's storage"), "{error}");
            assert!(
                error.contains("document (application/pdf) at host 'localhost'"),
                "{error}"
            );
            assert!(!error.contains("secret"), "{error}");
        }
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
        let model = RemoteMediaModel::with(
            recorder.clone(),
            Arc::new(EnvironmentSource::new(ExecutionEnvironment::Local)),
            reader("custom:anthropic"),
            TEST_POLICY,
        );
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

    #[tokio::test]
    async fn a_resolving_model_invokes_with_every_link_resolved() {
        let server = serve(vec![("/scan.pdf", Reply::ok(PDF))]).await;
        let recorder = Arc::new(RecordingModel::default());
        let resolving = RemoteMediaModel::resolving(
            recorder.clone(),
            Arc::new(DecidingSource::new()),
            reader("custom:anthropic"),
        );
        assert!(resolving.media.policy.allow_http);
        assert_eq!(resolving.media.policy.max_parts, 32);
        let call = history(vec![user(vec![
            text("read"),
            document(server.url("/scan.pdf"), None),
            image(server.url("/move.png"), None),
        ])]);

        resolving
            .invoke(&call, None)
            .await
            .expect("every link resolves");

        let histories = recorder.histories.lock().expect("history log");
        assert!(url_at(&histories[0], 0, 1).starts_with("data:application/pdf;base64,"));
        assert_eq!(
            url_at(&histories[0], 0, 2),
            "https://moved.example/move.png"
        );
    }

    /// Records each request a client sends and fails it.
    #[derive(Clone, Default)]
    struct RecordingClient {
        requests: Arc<Mutex<Vec<CompletionRequest>>>,
    }

    impl RecordingClient {
        fn record(&self, request: CompletionRequest) -> CompletionError {
            self.requests.lock().expect("request log").push(request);
            CompletionError::ProviderError("recorded".to_string())
        }
    }

    impl CompletionClientDyn for RecordingClient {
        fn completion_model<'a>(&self, _model: &str) -> Box<dyn CompletionModelDyn + 'a> {
            Box::new(self.clone())
        }

        fn agent<'a>(&self, _model: &str) -> AgentBuilder<CompletionModelHandle<'a>> {
            AgentBuilder::new(CompletionModelHandle::new(Arc::new(self.clone())))
        }
    }

    impl CompletionModelDyn for RecordingClient {
        fn completion(
            &self,
            request: CompletionRequest,
        ) -> WasmBoxedFuture<
            '_,
            std::result::Result<CompletionResponse<DynamicResponse>, CompletionError>,
        > {
            let error = self.record(request);
            Box::pin(async move { Err(error) })
        }

        fn stream(
            &self,
            request: CompletionRequest,
        ) -> WasmBoxedFuture<
            '_,
            std::result::Result<
                StreamingCompletionResponse<DynamicStreamingResponse>,
                CompletionError,
            >,
        > {
            let error = self.record(request);
            Box::pin(async move { Err(error) })
        }

        fn completion_request(
            &self,
            prompt: Message,
        ) -> CompletionRequestBuilder<CompletionModelHandle<'_>> {
            CompletionRequestBuilder::new(
                CompletionModelHandle::new(Arc::new(self.clone())),
                prompt,
            )
        }
    }

    /// A model whose client is a [`RecordingClient`], as agents reach it.
    struct AgentModel(RecordingClient);

    #[flow_like_types::async_trait]
    impl ModelLogic for AgentModel {
        async fn provider(&self) -> Result<ModelConstructor> {
            Ok(ModelConstructor {
                inner: Box::new(self.0.clone()),
            })
        }

        async fn default_model(&self) -> Option<String> {
            Some("agent-model".to_string())
        }
    }

    fn agent_model(client: &RecordingClient, reader: MediaReader) -> RemoteMediaModel {
        RemoteMediaModel::with(
            Arc::new(AgentModel(client.clone())),
            Arc::new(EnvironmentSource::new(ExecutionEnvironment::Local)),
            reader,
            TEST_POLICY,
        )
    }

    #[tokio::test]
    async fn agents_get_their_media_ready_like_invoke() {
        let server = serve(vec![
            ("/report.pdf", Reply::ok(PDF)),
            ("/notes.txt", Reply::ok(b"plain notes")),
            ("/render", Reply::Typed("image/png", PNG)),
        ])
        .await;
        let call = history(vec![user(vec![
            text("read"),
            document(server.url("/report.pdf"), Some("application/pdf")),
            document(server.url("/notes.txt"), None),
            image(server.url("/render"), None),
            image(format!("{PUBLIC}/cat.png"), None),
        ])]);
        let client = RecordingClient::default();
        let model = agent_model(&client, reader("custom:anthropic"));

        let completion = model
            .provider()
            .await
            .expect("the provider's client")
            .into_client()
            .completion_model("agent-model");
        completion
            .completion(rig_request(&call))
            .await
            .expect_err("the recording client fails every request");
        let dynamic = model
            .dynamic_completion_model(None)
            .await
            .expect("a dynamic model");
        CompletionModel::stream(&dynamic, rig_request(&call))
            .await
            .err()
            .expect("the recording client fails every request");

        let requests = client.requests.lock().expect("request log");
        assert_eq!(requests.len(), 2);
        for sent in requests.iter() {
            let parts = prompt_parts(sent);
            let UserContent::Document(pdf) = &parts[1] else {
                panic!("expected a document, got {:?}", parts[1]);
            };
            assert_eq!(pdf.data, DocumentSourceKind::Base64(STANDARD.encode(PDF)));
            assert_eq!(pdf.media_type, Some(DocumentMediaType::PDF));
            assert_text_document(&parts[2], "plain notes", DocumentMediaType::TXT);
            let UserContent::Image(rendered) = &parts[3] else {
                panic!("expected an image, got {:?}", parts[3]);
            };
            assert_eq!(
                rendered.data,
                DocumentSourceKind::Base64(STANDARD.encode(PNG))
            );
            assert_eq!(rendered.media_type, Some(ImageMediaType::PNG));
            let UserContent::Image(public) = &parts[4] else {
                panic!("expected an image, got {:?}", parts[4]);
            };
            assert_eq!(
                public.data,
                DocumentSourceKind::Url(format!("{PUBLIC}/cat.png"))
            );
        }
        assert_eq!(server.hits(), 3, "the second request reuses the downloads");
    }

    #[tokio::test]
    async fn an_agent_request_with_media_it_cannot_send_fails_naming_the_host() {
        let server = serve(vec![]).await;
        let call = history(vec![user(vec![
            text("read"),
            document(server.url("/gone.pdf?X-Amz-Signature=secret"), None),
        ])]);
        let client = RecordingClient::default();
        let model = agent_model(&client, reader("custom:anthropic"));

        let error = model
            .provider()
            .await
            .expect("the provider's client")
            .into_client()
            .completion_model("agent-model")
            .completion(rig_request(&call))
            .await
            .expect_err("the PDF cannot be fetched")
            .to_string();

        assert!(
            error.contains("document (application/pdf) at host '127.0.0.1'"),
            "{error}"
        );
        assert!(error.contains("HTTP 404"), "{error}");
        assert!(
            !error.contains("gone") && !error.contains("secret"),
            "{error}"
        );
        assert!(
            client.requests.lock().expect("request log").is_empty(),
            "the request never reaches the provider"
        );
    }
}
