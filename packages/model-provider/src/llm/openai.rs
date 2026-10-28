use std::{any::Any, sync::Arc};

use super::{
    ModelLogic, UsageReportingMode, body_params, drop_body_param, drop_temperature,
    extract_headers,
    media::{self, MediaDialect},
    merge_additional_params, output_budget_as_body_param,
    responses_tools::NonStrictToolsClient,
    user_media::{self, MediaFixup, leading_bytes, link_file_name},
};
use crate::authorization::AuthorizedHttpClient;
use crate::provider::{is_hosted_provider_name, random_provider};
use crate::{
    egress::ProviderEgress,
    history::{History, HistoryThinking},
    llm::ModelConstructor,
    provider::{ModelApiSurface, ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use flow_like_types_contracts::Cacheable;
use flow_like_types_contracts::authorization::{RequestAuthorizer, ResourceAudience};
use rig::OneOrMany;
use rig::completion::{CompletionError, CompletionRequest, Message as RigMessage};
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, Request, Response as HttpResponse,
    StreamingResponse,
};
use rig::message::{
    AssistantContent, Document, DocumentMediaType, DocumentSourceKind, ReasoningContent,
    UserContent,
};
use rig::wasm_compat::WasmCompatSend;
use serde_json::{Value, json};

#[derive(Clone)]
enum OpenAIClientType {
    OpenAI(rig::providers::openai::Client<InputFileHttpClient>),
    OpenAIChatCompletions(rig::providers::openai::CompletionsClient<InputFileHttpClient>),
    Azure(rig::providers::azure::Client<AuthorizedHttpClient>),
}

/// Reasoning support by model family, from the model id alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAIReasoning {
    /// gpt-4o, gpt-4.1, gpt-5-chat, o1-mini: reasoning control returns 400.
    Unsupported,
    OSeries,
    Gpt5 {
        minor: u32,
    },
    Gpt6,
    GptOss,
    /// Hosted Bit ids and other names that don't reveal the upstream model.
    Unknown,
}

impl OpenAIReasoning {
    fn of(model: &str) -> Self {
        let model = model.to_ascii_lowercase();
        let name = model.rsplit('/').next().unwrap_or(&model);
        let name = name.strip_prefix("openai.").unwrap_or(name);
        if name.contains("gpt-oss") {
            return Self::GptOss;
        }
        if name.contains("gpt-6") {
            return Self::Gpt6;
        }
        if let Some(rest) = name.split_once("gpt-5").map(|(_, rest)| rest) {
            if rest.contains("-chat") {
                return Self::Unsupported;
            }
            let minor = rest
                .strip_prefix('.')
                .map(|rest| {
                    rest.chars()
                        .take_while(char::is_ascii_digit)
                        .collect::<String>()
                })
                .and_then(|digits| digits.parse().ok())
                .unwrap_or(0);
            return Self::Gpt5 { minor };
        }
        if name.starts_with("o1-mini") {
            return Self::Unsupported;
        }
        let mut chars = name.chars();
        if chars.next() == Some('o') && chars.next().is_some_and(|c| c.is_ascii_digit()) {
            return Self::OSeries;
        }
        if ["gpt-4", "gpt-3.5", "chatgpt"]
            .iter()
            .any(|family| name.contains(family))
        {
            return Self::Unsupported;
        }
        Self::Unknown
    }

    /// Effort names differ per family: `minimal` exists only on gpt-5, `none` from gpt-5.1 on.
    fn effort(self, thinking: HistoryThinking) -> Option<&'static str> {
        match (self, thinking) {
            (Self::Unsupported, _) => None,
            (_, HistoryThinking::Low) => Some("low"),
            (_, HistoryThinking::Mid) => Some("medium"),
            (_, HistoryThinking::High) => Some("high"),
            (Self::Gpt5 { minor: 0 }, HistoryThinking::Off) => Some("minimal"),
            (Self::Gpt5 { .. } | Self::Unknown, HistoryThinking::Off) => Some("none"),
            (Self::OSeries | Self::Gpt6 | Self::GptOss, HistoryThinking::Off) => Some("low"),
        }
    }

    /// o-series and GPT-5+ reject `temperature`/`top_p`/penalties unless gpt-5.1+ runs with
    /// reasoning effort `none`.
    fn rejects_sampling(self, effort: Option<&str>) -> bool {
        match self {
            Self::OSeries | Self::Gpt6 => true,
            Self::Gpt5 { minor } => !(minor >= 1 && effort == Some("none")),
            Self::Unsupported | Self::GptOss | Self::Unknown => false,
        }
    }

    /// Chat Completions reasoning models accept only `max_completion_tokens`.
    fn needs_max_completion_tokens(self) -> bool {
        matches!(self, Self::OSeries | Self::Gpt5 { .. } | Self::Gpt6)
    }

    /// OpenAI rejects `include: ["reasoning.encrypted_content"]` for models that do not reason.
    fn returns_encrypted_reasoning(self) -> bool {
        matches!(self, Self::OSeries | Self::Gpt5 { .. } | Self::Gpt6)
    }
}

fn drop_rejected_sampling(model: &str, request: &mut CompletionRequest) {
    let effort = request.additional_params.as_ref().and_then(|params| {
        params
            .get("reasoning_effort")
            .or_else(|| params.pointer("/reasoning/effort"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    if !OpenAIReasoning::of(model).rejects_sampling(effort.as_deref()) {
        return;
    }
    drop_temperature(request, model, "rejected by reasoning models");
    for key in ["top_p", "presence_penalty", "frequency_penalty"] {
        drop_body_param(request, key, model, "rejected by reasoning models");
    }
}

fn chat_completions_constraints(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    chat_request(MediaDialect::OpenAIChat, model, request)
}

fn other_server_chat_constraints(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    chat_request(MediaDialect::OpenAICompatibleChat, model, request)
}

fn chat_request(
    dialect: MediaDialect,
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    let mut request = media::retain_supported(dialect, model, request)?;
    if OpenAIReasoning::of(model).needs_max_completion_tokens()
        && let Some(max_tokens) = request.max_tokens.take()
    {
        body_params(&mut request).insert("max_completion_tokens".into(), json!(max_tokens));
    }
    drop_rejected_sampling(model, &mut request);
    Ok(request)
}

/// Rig's Azure request omits `max_tokens`, so the budget always travels as a body field.
fn azure_constraints(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    let key = if OpenAIReasoning::of(model).needs_max_completion_tokens() {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    let mut request = output_budget_as_body_param(
        media::retain_supported(MediaDialect::OpenAIChat, model, request)?,
        key,
    );
    drop_rejected_sampling(model, &mut request);
    Ok(request)
}

fn responses_constraints(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    responses_request(MediaDialect::OpenAIResponses, model, request, false)
}

/// OpenAI's own API also reads Office files and CSV links as `input_file`; Azure takes PDFs only.
fn openai_responses_constraints(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    responses_request(MediaDialect::OpenAIResponses, model, request, true)
}

fn other_server_responses_constraints(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    responses_request(
        MediaDialect::OpenAICompatibleResponses,
        model,
        request,
        false,
    )
}

fn responses_request(
    dialect: MediaDialect,
    model: &str,
    request: CompletionRequest,
    office_files: bool,
) -> Result<CompletionRequest, CompletionError> {
    let links = carries_media_link(&request);
    let request = if office_files {
        carry_office_files(request)
    } else {
        request
    };
    let mut request = media::retain_supported(dialect, model, request)?;
    if links {
        keep_out_of_storage(model, &mut request);
    }
    drop_rejected_sampling(model, &mut request);
    Ok(request)
}

/// Who serves a client's requests, which decides the media parts they may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Server {
    /// OpenAI's own API, whose Responses surface also reads Office files and CSV links.
    OpenAI,
    /// Azure OpenAI, which reads PDFs and file ids.
    Azure,
    /// Flow-Like's hosted proxy, which relays file parts to the Bit's provider.
    HostedProxy,
    /// Any other server, such as vLLM, llama.cpp or TGI, none of which takes a file part.
    Other,
}

impl Server {
    /// Rig's client calls `api.openai.com` when no endpoint is set.
    fn of(provider_name: &str, endpoint: Option<&str>) -> Self {
        if is_hosted_provider_name(provider_name) {
            return Self::HostedProxy;
        }
        let Some(endpoint) = endpoint else {
            return Self::OpenAI;
        };
        let host = reqwest::Url::parse(endpoint)
            .ok()
            .filter(|url| url.scheme() == "https")
            .and_then(|url| url.host_str().map(str::to_string));
        match host.as_deref() {
            Some(host) if host == "api.openai.com" || host.ends_with(".api.openai.com") => {
                Self::OpenAI
            }
            Some(host) if host.ends_with(".openai.azure.com") => Self::Azure,
            _ => Self::Other,
        }
    }
}

pub struct OpenAIModel {
    client: OpenAIClientType,
    default_model: Option<String>,
    server: Server,
}

impl OpenAIModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let openai_config = random_provider(&config.openai_config)?;
        let api_key = openai_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let (client, server) = if provider.provider_name == "azure" {
            let endpoint = openai_config.endpoint.clone().unwrap_or_default();
            let endpoint = if endpoint.ends_with('/') {
                endpoint.to_string()
            } else {
                format!("{}/", endpoint)
            };

            let auth = rig::providers::azure::AzureOpenAIAuth::ApiKey(api_key.clone());
            let mut builder = rig::providers::azure::Client::builder()
                .api_key(auth)
                .http_client(AuthorizedHttpClient::default())
                .azure_endpoint(endpoint);
            if let Some(version) = provider.version.as_deref() {
                builder = builder.api_version(version);
            }

            (OpenAIClientType::Azure(builder.build()?), Server::Azure)
        } else {
            let mut builder = rig::providers::openai::Client::builder()
                .api_key(&api_key)
                .http_client(InputFileHttpClient::default());
            if let Some(endpoint) = openai_config.endpoint.as_deref() {
                builder = builder.base_url(endpoint);
            }

            (
                OpenAIClientType::OpenAI(builder.build()?),
                Server::of(&provider.provider_name, openai_config.endpoint.as_deref()),
            )
        };

        Ok(OpenAIModel {
            client,
            default_model: model_id,
            server,
        })
    }

    pub async fn from_provider(
        provider: &ModelProvider,
        egress: ProviderEgress,
    ) -> anyhow::Result<Self> {
        Self::from_params(provider, None, egress)
    }

    pub async fn from_provider_with_authorizer(
        provider: &ModelProvider,
        authorizer: Option<Arc<dyn RequestAuthorizer>>,
    ) -> anyhow::Result<Self> {
        Self::from_params(provider, authorizer, ProviderEgress::Open)
    }

    #[allow(clippy::cognitive_complexity)]
    fn from_params(
        provider: &ModelProvider,
        authorizer: Option<Arc<dyn RequestAuthorizer>>,
        egress: ProviderEgress,
    ) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        let is_azure = params.get("is_azure").cloned();
        let endpoint = params.get("endpoint").cloned();
        let custom_headers = extract_headers(&params);

        let is_azure = match is_azure {
            Some(val) => val.as_bool().unwrap_or(false),
            None => false,
        };

        if is_azure && authorizer.is_some() {
            anyhow::bail!("Renewable hosted authorization requires the Flow-Like proxy endpoint");
        }

        if is_azure && endpoint.is_none() {
            return Err(anyhow::anyhow!("Azure OpenAI requires an endpoint"));
        }

        if is_azure && model_id.is_none() {
            return Err(anyhow::anyhow!(
                "Azure OpenAI requires a model_id (deployment name)"
            ));
        }

        let (client, server) = if is_azure {
            let endpoint = endpoint.unwrap_or_default();
            let endpoint = endpoint.as_str().unwrap_or_default();
            let endpoint = if endpoint.ends_with('/') {
                endpoint.to_string()
            } else {
                format!("{}/", endpoint)
            };

            let auth = rig::providers::azure::AzureOpenAIAuth::ApiKey(api_key.to_string());
            let mut builder = rig::providers::azure::Client::builder()
                .api_key(auth)
                .http_client(AuthorizedHttpClient::for_egress(egress)?)
                .azure_endpoint(endpoint);
            if let Some(version_str) = params.get("version").and_then(|v| v.as_str()) {
                builder = builder.api_version(version_str);
            }
            if !custom_headers.is_empty() {
                builder = builder.http_headers(custom_headers);
            }
            (OpenAIClientType::Azure(builder.build()?), Server::Azure)
        } else {
            let http_client = match authorizer {
                Some(authorizer) => AuthorizedHttpClient::new(
                    authorizer,
                    ResourceAudience::HostedModels,
                    endpoint
                        .as_ref()
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!("Hosted authorization requires an explicit endpoint")
                        })?,
                )?,
                None => AuthorizedHttpClient::for_egress(egress)?,
            };
            let endpoint = endpoint.as_ref().and_then(Value::as_str);
            let mut builder = rig::providers::openai::Client::builder()
                .api_key(api_key)
                .http_client(InputFileHttpClient(http_client));
            if let Some(endpoint) = endpoint {
                builder = builder.base_url(endpoint);
            }
            if !custom_headers.is_empty() {
                builder = builder.http_headers(custom_headers);
            }
            (
                OpenAIClientType::OpenAI(builder.build()?),
                Server::of(&provider.provider_name, endpoint),
            )
        };

        Ok(OpenAIModel {
            client,
            default_model: model_id,
            server,
        })
    }

    /// Build an OpenAI client that uses the Chat Completions API.
    ///
    /// Rig's default OpenAI client uses the Responses API. Flow-Like's hosted
    /// model proxy exposes an OpenAI-compatible `/chat/completions` endpoint,
    /// so hosted OpenAI Bits must opt into Rig's completions client explicitly.
    /// Direct OpenAI models keep using [`Self::from_provider`] and the Responses
    /// API.
    pub async fn from_provider_chat_completions(provider: &ModelProvider) -> anyhow::Result<Self> {
        Ok(Self::from_provider(provider, ProviderEgress::Open)
            .await?
            .with_api_surface(ModelApiSurface::ChatCompletions))
    }

    /// Build the client for an explicit API surface, for an endpoint this host
    /// trusts ([`ProviderEgress::Open`]).
    ///
    /// [`ModelApiSurface::Responses`] keeps Rig's default OpenAI client, which
    /// posts to `{base_url}/responses`; [`ModelApiSurface::ChatCompletions`]
    /// swaps in the completions client, which posts to
    /// `{base_url}/chat/completions`.
    pub async fn from_provider_with_surface(
        provider: &ModelProvider,
        surface: ModelApiSurface,
    ) -> anyhow::Result<Self> {
        Ok(Self::from_provider(provider, ProviderEgress::Open)
            .await?
            .with_api_surface(surface))
    }

    pub async fn from_provider_with_surface_and_authorizer(
        provider: &ModelProvider,
        surface: ModelApiSurface,
        authorizer: Option<Arc<dyn RequestAuthorizer>>,
    ) -> anyhow::Result<Self> {
        Ok(Self::from_provider_with_authorizer(provider, authorizer)
            .await?
            .with_api_surface(surface))
    }

    /// Re-target an already built client at `surface`.
    ///
    /// Azure clients are left untouched — Rig models Azure as a single
    /// deployment client that has no Responses counterpart.
    pub fn with_api_surface(self, surface: ModelApiSurface) -> Self {
        let Self {
            client,
            default_model,
            server,
        } = self;
        let client = match (client, surface) {
            (OpenAIClientType::OpenAI(client), ModelApiSurface::ChatCompletions) => {
                OpenAIClientType::OpenAIChatCompletions(client.completions_api())
            }
            (OpenAIClientType::OpenAIChatCompletions(client), ModelApiSurface::Responses) => {
                OpenAIClientType::OpenAI(client.responses_api())
            }
            (other, _) => other,
        };

        Self {
            client,
            default_model,
            server,
        }
    }
}

impl Cacheable for OpenAIModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for OpenAIModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(match self.client.clone() {
            OpenAIClientType::OpenAI(client) => {
                let fixup: MediaFixup = match self.server {
                    Server::OpenAI => openai_responses_constraints,
                    Server::Azure | Server::HostedProxy => responses_constraints,
                    Server::Other => other_server_responses_constraints,
                };
                user_media::constructor(NonStrictToolsClient(client), fixup)
            }
            OpenAIClientType::OpenAIChatCompletions(client) => {
                let fixup: MediaFixup = match self.server {
                    Server::OpenAI | Server::Azure | Server::HostedProxy => {
                        chat_completions_constraints
                    }
                    Server::Other => other_server_chat_constraints,
                };
                user_media::constructor(client, fixup)
            }
            OpenAIClientType::Azure(client) => user_media::constructor(client, azure_constraints),
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn usage_reporting(&self) -> UsageReportingMode {
        // The Responses API rejects `stream_options` and always emits usage on
        // the terminal `response.completed` event, so only the Chat Completions
        // and Azure clients need the opt-in.
        match self.client {
            OpenAIClientType::OpenAI(_) => UsageReportingMode::None,
            OpenAIClientType::OpenAIChatCompletions(_) | OpenAIClientType::Azure(_) => {
                UsageReportingMode::OpenAIStreamOptions
            }
        }
    }

    /// Responses takes `reasoning: {effort}`; Chat Completions and Azure take a flat
    /// `reasoning_effort` and reject the object.
    fn additional_params(&self, history: &Option<History>) -> Option<serde_json::Value> {
        let history = history.as_ref()?;
        let base = history.build_additional_params().ok().flatten();
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        let reasoning = history
            .thinking
            .and_then(|thinking| OpenAIReasoning::of(model).effort(thinking))
            .map(|effort| match self.client {
                OpenAIClientType::OpenAI(_) => json!({ "reasoning": { "effort": effort } }),
                OpenAIClientType::OpenAIChatCompletions(_) | OpenAIClientType::Azure(_) => {
                    json!({ "reasoning_effort": effort })
                }
            });

        merge_additional_params(base, reasoning)
    }
}

pub(super) fn user_parts(request: &CompletionRequest) -> impl Iterator<Item = &UserContent> {
    request
        .chat_history
        .iter()
        .filter_map(|message| match message {
            RigMessage::User { content } => Some(content.iter()),
            _ => None,
        })
        .flatten()
}

/// Whether a user message carries an image, audio, video or document as a link.
pub(super) fn carries_media_link(request: &CompletionRequest) -> bool {
    user_parts(request).any(|part| {
        let source = match part {
            UserContent::Image(image) => &image.data,
            UserContent::Audio(audio) => &audio.data,
            UserContent::Video(video) => &video.data,
            UserContent::Document(document) => &document.data,
            UserContent::Text(_) | UserContent::ToolResult(_) => return false,
        };
        matches!(source, DocumentSourceKind::Url(_))
    })
}

const ENCRYPTED_REASONING: &str = "reasoning.encrypted_content";

/// OpenAI keeps a stored request, its media links included, for 30 days or more, and a link may
/// be a presigned credential. Other requests stay stored: Rig replays reasoning by item id, which
/// OpenAI resolves from the stored response unless the item carries its encrypted payload. So a
/// request kept out of storage asks reasoning models for that payload, and replayed reasoning
/// without it is dropped, as OpenAI would fail the call on it.
fn keep_out_of_storage(model: &str, request: &mut CompletionRequest) {
    let params = body_params(request);
    params.insert("store".to_string(), Value::Bool(false));
    if OpenAIReasoning::of(model).returns_encrypted_reasoning() {
        let include = params.entry("include").or_insert_with(|| json!([]));
        if let Some(include) = include.as_array_mut()
            && !include
                .iter()
                .any(|item| item.as_str() == Some(ENCRYPTED_REASONING))
        {
            include.push(json!(ENCRYPTED_REASONING));
        }
    }
    drop_stored_reasoning(request);
}

/// Removes replayed reasoning that only a stored response holds: an item with an id and no
/// encrypted payload, which a request with `store: false` cannot refer to.
pub(super) fn drop_stored_reasoning(request: &mut CompletionRequest) {
    let mut dropped = 0;
    for message in request.chat_history.iter_mut() {
        let RigMessage::Assistant { content, .. } = message else {
            continue;
        };
        if !content.iter().any(is_stored_reasoning) {
            continue;
        }
        let kept: Vec<AssistantContent> = content
            .iter()
            .filter(|part| !is_stored_reasoning(part))
            .cloned()
            .collect();
        dropped += content.len() - kept.len();
        *content =
            OneOrMany::many(kept).unwrap_or_else(|_| OneOrMany::one(AssistantContent::text("")));
    }
    if dropped > 0 {
        tracing::debug!(
            dropped,
            "Dropped replayed reasoning that only a stored response holds"
        );
    }
}

fn is_stored_reasoning(part: &AssistantContent) -> bool {
    let AssistantContent::Reasoning(reasoning) = part else {
        return false;
    };
    reasoning.id.is_some()
        && !reasoning.content.iter().any(|content| {
            matches!(
                content,
                ReasoningContent::Encrypted(_) | ReasoningContent::Redacted { .. }
            )
        })
}

/// Prefix of the file id that carries an `input_file` part from the request fixup to
/// [`InputFileHttpClient`].
const INPUT_FILE_CARRIER: &str = "flow-like-input-file:";

/// Documents OpenAI's Responses API reads as `input_file` besides PDF, which Rig sends itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OfficeFile {
    Docx,
    Xlsx,
    Pptx,
    Csv,
}

impl OfficeFile {
    fn extension(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
            Self::Csv => "csv",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Self::Csv => "text/csv",
        }
    }

    fn from_extension(extension: &str) -> Option<Self> {
        [Self::Docx, Self::Xlsx, Self::Pptx, Self::Csv]
            .into_iter()
            .find(|kind| extension.eq_ignore_ascii_case(kind.extension()))
    }
}

/// Rig sends only PDFs as `input_file`: it fails the call on an Office file link and sends inline
/// Office data as text. Such parts travel to [`InputFileHttpClient`] as carrier file ids instead.
fn carry_office_files(mut request: CompletionRequest) -> CompletionRequest {
    for message in request.chat_history.iter_mut() {
        let RigMessage::User { content } = message else {
            continue;
        };
        for part in content.iter_mut() {
            let UserContent::Document(document) = part else {
                continue;
            };
            if let Some(file) = office_input_file(document) {
                *part = UserContent::Document(Document {
                    data: DocumentSourceKind::FileId(format!("{INPUT_FILE_CARRIER}{file}")),
                    media_type: None,
                    additional_params: None,
                });
            }
        }
    }
    request
}

/// The `input_file` part of an Office file or CSV link, or of inline Office data. Inline CSV
/// stays text, which reaches the model whole; OpenAI reads 1,000 rows of a spreadsheet file.
fn office_input_file(document: &Document) -> Option<Value> {
    match (&document.data, &document.media_type) {
        (DocumentSourceKind::Url(url), media_type) => office_link(url, media_type.as_ref()),
        (DocumentSourceKind::Base64(data), None)
            if leading_bytes(data, ZIP_ENTRY.len()).starts_with(ZIP_ENTRY) =>
        {
            let kind = office_package(&STANDARD.decode(data).ok()?)?;
            Some(inline_input_file(kind, data))
        }
        (DocumentSourceKind::Raw(bytes), None) => Some(inline_input_file(
            office_package(bytes)?,
            &STANDARD.encode(bytes),
        )),
        _ => None,
    }
}

/// OpenAI fetches `https` links only; any other link keeps its note.
fn office_link(url: &str, media_type: Option<&DocumentMediaType>) -> Option<Value> {
    if !url
        .get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
    {
        return None;
    }
    let name = link_file_name(url);
    let named = name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .and_then(|(_, extension)| OfficeFile::from_extension(extension));
    let kind = match media_type {
        None => named?,
        Some(DocumentMediaType::CSV) => OfficeFile::Csv,
        Some(_) => return None,
    };
    let filename = if named == Some(kind) {
        name.to_string()
    } else {
        format!("document.{}", kind.extension())
    };
    Some(json!({"type": "input_file", "file_url": url, "filename": filename}))
}

/// Signature of a zip's first local file header, where every Office Open XML package starts.
const ZIP_ENTRY: &[u8] = b"PK\x03\x04";

fn inline_input_file(kind: OfficeFile, base64: &str) -> Value {
    json!({
        "type": "input_file",
        "file_data": format!("data:{};base64,{base64}", kind.mime()),
        "filename": format!("document.{}", kind.extension()),
    })
}

/// The Office Open XML package a zip holds, from the folder of its parts (`word/`, `xl/`,
/// `ppt/`) as its central directory lists them.
fn office_package(zip: &[u8]) -> Option<OfficeFile> {
    if !zip.starts_with(ZIP_ENTRY) {
        return None;
    }
    let names = zip_entry_names(zip)?;
    if !names.contains(&b"[Content_Types].xml".as_slice()) {
        return None;
    }
    names.into_iter().find_map(|name| {
        [
            (b"word/".as_slice(), OfficeFile::Docx),
            (b"xl/", OfficeFile::Xlsx),
            (b"ppt/", OfficeFile::Pptx),
        ]
        .into_iter()
        .find_map(|(folder, kind)| name.starts_with(folder).then_some(kind))
    })
}

/// The entry names a zip's central directory lists, in its order.
fn zip_entry_names(zip: &[u8]) -> Option<Vec<&[u8]>> {
    let search_from = zip.len().saturating_sub(22 + usize::from(u16::MAX));
    let end = zip[search_from..]
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")?;
    let directory = &zip[search_from + end..];
    let mut entries = zip.get(le32(directory, 16)?..)?;
    let mut names = Vec::new();
    for _ in 0..le16(directory, 10)? {
        let (name, rest) = directory_entry(entries)?;
        names.push(name);
        entries = rest;
    }
    Some(names)
}

/// The name of the central directory entry `entries` starts with, and the entries after it.
fn directory_entry(entries: &[u8]) -> Option<(&[u8], &[u8])> {
    if !entries.starts_with(b"PK\x01\x02") {
        return None;
    }
    let name_end = 46 + le16(entries, 28)?;
    let next = name_end + le16(entries, 30)? + le16(entries, 32)?;
    Some((entries.get(46..name_end)?, entries.get(next..)?))
}

fn le16(bytes: &[u8], at: usize) -> Option<usize> {
    let field = bytes.get(at..at + 2)?;
    Some(usize::from(u16::from_le_bytes([field[0], field[1]])))
}

fn le32(bytes: &[u8], at: usize) -> Option<usize> {
    let field = bytes.get(at..at + 4)?;
    usize::try_from(u32::from_le_bytes([field[0], field[1], field[2], field[3]])).ok()
}

/// Puts each carried `input_file` part back in place of its carrier file id. A request without
/// a carrier passes through unchanged.
#[derive(Clone, Debug, Default)]
struct InputFileHttpClient(AuthorizedHttpClient);

impl HttpClientExt for InputFileHttpClient {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<HttpResponse<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let request = with_input_files(request);
        let client = self.0.clone();
        async move { client.send::<Bytes, U>(request?).await }
    }

    fn send_multipart<U>(
        &self,
        request: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<HttpResponse<LazyBody<U>>>> + WasmCompatSend + 'static
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
        let request = with_input_files(request);
        let client = self.0.clone();
        async move { client.send_streaming(request?).await }
    }
}

fn with_input_files<T: Into<Bytes>>(request: Request<T>) -> http_client::Result<Request<Bytes>> {
    let (parts, body) = request.into_parts();
    let body = splice_input_files(body.into()).ok_or_else(|| {
        http_client::Error::Instance(Box::new(std::io::Error::other(
            "An attached file could not be put into the OpenAI Responses request",
        )))
    })?;
    Ok(Request::from_parts(parts, body))
}

/// The value of a carrier is never echoed: a file link may be a presigned credential.
fn splice_input_files(body: Bytes) -> Option<Bytes> {
    let carrier = INPUT_FILE_CARRIER.as_bytes();
    if !body.windows(carrier.len()).any(|window| window == carrier) {
        return Some(body);
    }
    let mut request: Value = serde_json::from_slice(&body).ok()?;
    let parts = request
        .get_mut("input")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get_mut("content").and_then(Value::as_array_mut))
        .flatten();
    for part in parts {
        let Some(carried) = part
            .get("file_id")
            .and_then(Value::as_str)
            .and_then(|file_id| file_id.strip_prefix(INPUT_FILE_CARRIER))
        else {
            continue;
        };
        let file: Value = serde_json::from_str(carried).ok()?;
        *part = file;
    }
    serde_json::to_vec(&request).ok().map(Bytes::from)
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use std::collections::HashMap;

    use rig::agent::MultiTurnStreamItem;
    use rig::completion::ToolDefinition;
    use rig::completion::{Chat, Message};
    use rig::message::Text;
    use rig::streaming::{StreamedAssistantContent, StreamingChat};
    use rig::tool::Tool;
    use schemars::{JsonSchema, schema_for};
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{
        history::{Content, ContentType, History, HistoryMessage, ImageUrl, MessageContent, Role},
        provider::{ModelProviderConfiguration, OpenAIConfig},
    };
    use dotenv::dotenv;

    /// Inline so vision tests don't depend on a third-party host the provider must fetch.
    const TEST_IMAGE_URL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAIAAAD8GO2jAAAAKklEQVR42mO44GBAU8QwasGoBaMWjFowasGoBaMWjFowasGoBaMWDBULAFMMAEwxv0+fAAAAAElFTkSuQmCC";

    fn proxy_provider() -> ModelProvider {
        ModelProvider {
            api_surface: None,
            provider_name: "hosted:openai".to_string(),
            model_id: Some("upstream-model".to_string()),
            version: None,
            params: Some(HashMap::from([
                (
                    "api_key".to_string(),
                    serde_json::Value::String("test-token".to_string()),
                ),
                (
                    "endpoint".to_string(),
                    serde_json::Value::String("https://proxy.example/api/v1".to_string()),
                ),
                (
                    "model_id".to_string(),
                    serde_json::Value::String("bit-id".to_string()),
                ),
            ])),
        }
    }

    #[tokio::test]
    async fn hosted_proxy_constructor_uses_openai_chat_completions() {
        let provider = proxy_provider();
        let model = OpenAIModel::from_provider_chat_completions(&provider)
            .await
            .expect("hosted proxy client should build");

        assert!(matches!(
            model.client,
            OpenAIClientType::OpenAIChatCompletions(_)
        ));
        assert_eq!(model.default_model.as_deref(), Some("bit-id"));
    }

    #[tokio::test]
    async fn direct_constructor_keeps_the_openai_responses_api() {
        let provider = proxy_provider();
        let model = OpenAIModel::from_provider(&provider, ProviderEgress::Open)
            .await
            .expect("direct OpenAI client should build");

        assert!(matches!(model.client, OpenAIClientType::OpenAI(_)));
    }

    #[tokio::test]
    #[ignore = "calls the live OpenAI API set by OPENAI_ENDPOINT and OPENAI_API_KEY"]
    async fn test_openai_model_no_stream() {
        let Some((provider, config)) = openai_provider_and_config() else {
            println!("Skipping OpenAI LLM test: OPENAI_ENDPOINT or OPENAI_API_KEY not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let mut history = History::new(
            model_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage::from_string(Role::User, "Hello"),
            ],
        );
        history.set_stream(false);

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, mut history_msgs) = history.extract_prompt_and_history().unwrap();
        let response: String = agent.chat(prompt, &mut history_msgs).await.unwrap();

        assert!(!response.is_empty());
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    #[ignore = "calls the live Azure OpenAI deployment set by AZURE_OPENAI_*"]
    async fn test_azure_openai_model_no_stream() {
        dotenv().ok();

        // Skip test if Azure LLM deployment is not configured
        let deployment_name = match std::env::var("AZURE_OPENAI_LLM_DEPLOYMENT") {
            Ok(name) => name,
            Err(_) => {
                println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
                return;
            }
        };

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some(deployment_name.clone()),
            version: Some("2024-02-15-preview".to_string()),
            provider_name: "azure".to_string(),
            params: None,
        };
        let api_key = std::env::var("AZURE_OPENAI_API_KEY").unwrap();
        let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT").unwrap();
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: Some(endpoint),
                proxy: None,
            }],
            ..Default::default()
        };

        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let mut history = History::new(
            deployment_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage::from_string(Role::User, "Hello"),
            ],
        );
        history.set_stream(false);
        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .temperature(1.0)
            .build();

        let (prompt, mut history) = history.extract_prompt_and_history().unwrap();

        let response: String = agent.chat(prompt, &mut history).await.unwrap();
        println!("Final response: {:?}", response);
        assert!(!response.is_empty());
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    #[ignore = "calls the live OpenAI API set by OPENAI_ENDPOINT and OPENAI_API_KEY"]
    async fn test_openai_model_stream() {
        let Some(config) = openai_endpoint_config() else {
            println!("Skipping OpenAI LLM test: OPENAI_ENDPOINT or OPENAI_API_KEY not set");
            return;
        };

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some("@preset/testing".to_string()),
            version: None,
            provider_name: "openai".to_string(),
            params: None,
        };

        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let history = History::new(
            "@preset/testing".to_string(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage::from_string(Role::User, "Hello"),
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, history_msgs) = history.extract_prompt_and_history().unwrap();

        use futures::StreamExt;
        let mut stream = agent.stream_chat(prompt, history_msgs).await;
        let mut chunks = 0;
        let mut response = String::new();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                    chunks += 1;
                }
                Ok(_) => {} // Ignore other stream items
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        println!("Final response: {:?}", response);
        println!("Chunks: {}", chunks);
        assert!(!response.is_empty());
        assert!(chunks > 0);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    #[ignore = "calls the live Azure OpenAI deployment set by AZURE_OPENAI_*"]
    async fn test_azure_openai_model_stream() {
        dotenv().ok();

        let deployment_name = match std::env::var("AZURE_OPENAI_LLM_DEPLOYMENT") {
            Ok(name) => name,
            Err(_) => {
                println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
                return;
            }
        };

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some(deployment_name.clone()),
            version: Some("2024-02-15-preview".to_string()),
            provider_name: "azure".to_string(),
            params: None,
        };
        let api_key = std::env::var("AZURE_OPENAI_API_KEY").unwrap();
        let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT").unwrap();
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: Some(endpoint),
                proxy: None,
            }],
            ..Default::default()
        };

        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let history = History::new(
            deployment_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage::from_string(Role::User, "Hello"),
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, history_msgs) = history.extract_prompt_and_history().unwrap();

        use futures::StreamExt;
        let mut stream = agent.stream_chat(prompt, history_msgs).await;
        let mut chunks = 0;
        let mut response = String::new();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                    chunks += 1;
                }
                Ok(_) => {} // Ignore other stream items
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        println!("Final response: {:?}", response);
        println!("Chunks: {}", chunks);
        assert!(!response.is_empty());
        assert!(chunks > 0);
    }

    // --- Helpers for new tests ---
    fn azure_provider_and_config() -> Option<(ModelProvider, ModelProviderConfiguration)> {
        dotenv().ok();

        let deployment_name = std::env::var("AZURE_OPENAI_LLM_DEPLOYMENT").ok()?;
        let api_key = std::env::var("AZURE_OPENAI_API_KEY").ok()?;
        let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT").ok()?;

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some(deployment_name),
            version: Some("2024-02-15-preview".to_string()),
            provider_name: "azure".to_string(),
            params: None,
        };
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: Some(endpoint),
                proxy: None,
            }],
            ..Default::default()
        };
        Some((provider, config))
    }

    // ========== Rig Tool Implementations ==========

    #[derive(Deserialize, JsonSchema)]
    struct WeatherArgs {
        location: String,
        unit: String,
    }

    #[derive(Debug)]
    struct WeatherError;

    impl std::fmt::Display for WeatherError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Weather tool error")
        }
    }

    impl std::error::Error for WeatherError {}

    #[derive(Deserialize, Serialize)]
    struct WeatherTool;

    impl Tool for WeatherTool {
        const NAME: &'static str = "get_current_weather";

        type Error = WeatherError;
        type Args = WeatherArgs;
        type Output = String;

        async fn definition(&self, _prompt: String) -> ToolDefinition {
            ToolDefinition {
                name: "get_current_weather".to_string(),
                description: "Get the current weather in a given location".to_string(),
                parameters: serde_json::to_value(schema_for!(WeatherArgs))
                    .expect("Failed to serialize weather args schema"),
            }
        }

        async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
            Ok(format!(
                "The weather in {} is 22 degrees {}",
                args.location, args.unit
            ))
        }
    }

    #[derive(Deserialize, JsonSchema)]
    struct ForecastArgs {
        location: String,
        days: i32,
    }

    #[derive(Debug)]
    struct ForecastError;

    impl std::fmt::Display for ForecastError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Forecast tool error")
        }
    }

    impl std::error::Error for ForecastError {}

    #[derive(Deserialize, Serialize)]
    struct ForecastTool;

    impl Tool for ForecastTool {
        const NAME: &'static str = "get_forecast";

        type Error = ForecastError;
        type Args = ForecastArgs;
        type Output = String;

        async fn definition(&self, _prompt: String) -> ToolDefinition {
            ToolDefinition {
                name: "get_forecast".to_string(),
                description: "Get the forecast for the next N days".to_string(),
                parameters: serde_json::to_value(schema_for!(ForecastArgs))
                    .expect("Failed to serialize forecast args schema"),
            }
        }

        async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
            Ok(format!(
                "The forecast for {} for the next {} days is sunny with temperatures ranging from 18-25 degrees",
                args.location, args.days
            ))
        }
    }

    // ========== Tool Tests ==========

    #[tokio::test]
    #[ignore = "calls the live Azure OpenAI deployment set by AZURE_OPENAI_*"]
    async fn test_azure_openai_tool_call_no_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat(
                "Call the tool to get the weather for San Francisco, CA in celsius.",
                &mut history,
            )
            .await
            .expect("Failed to get response");

        // The response should contain the tool's output (weather info)
        println!("Response: {}", response);
        assert!(!response.is_empty());
        assert!(
            response.contains("San Francisco")
                || response.contains("weather")
                || response.contains("22")
        );
    }

    #[tokio::test]
    #[ignore = "calls the live Azure OpenAI deployment set by AZURE_OPENAI_*"]
    async fn test_azure_openai_tool_call_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        use futures::StreamExt;
        let mut stream = agent
            .stream_chat(
                "Please call the tool to get the weather for Berlin in celsius.",
                Vec::<Message>::new(),
            )
            .await;

        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                }
                Ok(_) => {}
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        println!("Streamed response: {}", response);
        assert!(!response.is_empty());
    }

    #[tokio::test]
    #[ignore = "calls the live Azure OpenAI deployment set by AZURE_OPENAI_*"]
    async fn test_azure_openai_tool_result_roundtrip() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        // This will automatically call the tool and use its result
        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat(
                "What is the weather in Paris in celsius? Use the tool.",
                &mut history,
            )
            .await
            .expect("Failed to get response");

        println!("Roundtrip response: {}", response);
        assert!(!response.is_empty());
        // Response should mention Paris or the weather info
        assert!(
            response.contains("Paris") || response.contains("weather") || response.contains("22")
        );
    }

    #[tokio::test]
    #[ignore = "calls the live Azure OpenAI deployment set by AZURE_OPENAI_*"]
    async fn test_azure_openai_vision_no_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.clone().unwrap();

        let image_url = TEST_IMAGE_URL;
        let history = History::new(
            model_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage {
                    role: Role::User,
                    content: MessageContent::Contents(vec![
                        Content::Text {
                            content_type: ContentType::Text,
                            text: "Describe the image succinctly.".to_string(),
                        },
                        Content::Image {
                            content_type: ContentType::ImageUrl,
                            image_url: ImageUrl {
                                url: image_url.to_string(),
                                detail: None,
                                media_type: None,
                                additional_params: None,
                            },
                        },
                    ]),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    annotations: None,
                },
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, mut history_msgs) = history.extract_prompt_and_history().unwrap();

        let response: String = match agent.chat(prompt, &mut history_msgs).await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
                    eprintln!("Skipping due to rate limit: {msg}");
                    return;
                }
                if msg.contains("No endpoints found that support tool use")
                    || msg.contains("404 Not Found")
                {
                    eprintln!("Skipping: tool use unsupported on route: {msg}");
                    return;
                }
                panic!("{e}");
            }
        };
        assert!(!response.is_empty());
    }

    #[tokio::test]
    #[ignore = "calls the live Azure OpenAI deployment set by AZURE_OPENAI_*"]
    async fn test_azure_openai_vision_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.clone().unwrap();

        let image_url = TEST_IMAGE_URL;
        let history = History::new(
            model_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage {
                    role: Role::User,
                    content: MessageContent::Contents(vec![
                        Content::Text {
                            content_type: ContentType::Text,
                            text: "Describe the image.".to_string(),
                        },
                        Content::Image {
                            content_type: ContentType::ImageUrl,
                            image_url: ImageUrl {
                                url: image_url.to_string(),
                                detail: None,
                                media_type: None,
                                additional_params: None,
                            },
                        },
                    ]),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    annotations: None,
                },
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, history_msgs) = history.extract_prompt_and_history().unwrap();

        use futures::StreamExt;
        let mut stream = agent.stream_chat(prompt, history_msgs).await;

        let mut chunks = 0;
        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                    chunks += 1;
                }
                Ok(_) => {} // Ignore other stream items
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
                        eprintln!("Skipping due to rate limit: {msg}");
                        return;
                    }
                    panic!("Stream error: {}", e);
                }
            }
        }

        assert!(!response.is_empty());
        assert!(chunks > 0);
    }

    // -------- OpenAI parity tests (tool calling, roundtrip, vision) --------

    fn openai_endpoint_config() -> Option<ModelProviderConfiguration> {
        dotenv().ok();
        let endpoint = std::env::var("OPENAI_ENDPOINT").ok()?;
        let api_key = std::env::var("OPENAI_API_KEY").ok()?;
        Some(ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: Some(endpoint),
                proxy: None,
            }],
            ..Default::default()
        })
    }

    fn openai_provider_and_config() -> Option<(ModelProvider, ModelProviderConfiguration)> {
        let config = openai_endpoint_config()?;
        // Use a specific model that works with OpenRouter instead of @preset/testing
        // openai/gpt-4o-mini supports tools and is available via OpenRouter
        let model_id = if std::env::var("OPENAI_ENDPOINT")
            .unwrap_or_default()
            .contains("openrouter")
        {
            "openai/gpt-4o-mini".to_string()
        } else {
            "gpt-3.5-turbo".to_string()
        };

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some(model_id),
            version: None,
            provider_name: "openai".to_string(),
            params: None,
        };
        Some((provider, config))
    }

    #[tokio::test]
    #[ignore = "calls the live OpenAI API set by OPENAI_ENDPOINT and OPENAI_API_KEY"]
    async fn test_openai_tool_call_no_stream() {
        let Some((provider, config)) = openai_provider_and_config() else {
            println!("Skipping OpenAI LLM test: OPENAI_ENDPOINT or OPENAI_API_KEY not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or("@preset/testing".to_string()))
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat(
                "Call the tool to get the weather for San Francisco, CA in celsius.",
                &mut history,
            )
            .await
            .expect("Failed to get response");

        println!("Response: {}", response);
        assert!(!response.is_empty());
        assert!(
            response.contains("San Francisco")
                || response.contains("weather")
                || response.contains("22")
        );
    }

    #[tokio::test]
    #[ignore = "calls the live OpenAI API set by OPENAI_ENDPOINT and OPENAI_API_KEY"]
    async fn test_openai_tool_call_stream() {
        let Some((provider, config)) = openai_provider_and_config() else {
            println!("Skipping OpenAI LLM test: OPENAI_ENDPOINT or OPENAI_API_KEY not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or("@preset/testing".to_string()))
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        use futures::StreamExt;
        let mut stream = agent
            .stream_chat(
                "Please call the tool to get the weather for Berlin in celsius.",
                Vec::<Message>::new(),
            )
            .await;

        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                }
                Ok(_) => {}
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        println!("Streamed response: {}", response);
        assert!(!response.is_empty());
    }

    #[tokio::test]
    #[ignore = "calls the live OpenAI API set by OPENAI_ENDPOINT and OPENAI_API_KEY"]
    async fn test_openai_tool_result_roundtrip() {
        let Some((provider, config)) = openai_provider_and_config() else {
            println!("Skipping OpenAI LLM test: OPENAI_ENDPOINT or OPENAI_API_KEY not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or("@preset/testing".to_string()))
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat(
                "What is the weather in Paris in celsius? Use the tool.",
                &mut history,
            )
            .await
            .expect("Failed to get response");

        println!("Roundtrip response: {}", response);
        assert!(!response.is_empty());
        assert!(
            response.contains("Paris") || response.contains("weather") || response.contains("22")
        );
    }

    #[tokio::test]
    #[ignore = "calls the live OpenAI API set by OPENAI_ENDPOINT and OPENAI_API_KEY"]
    async fn test_openai_vision_no_stream() {
        let Some((provider, config)) = openai_provider_and_config() else {
            println!("Skipping OpenAI LLM test: OPENAI_ENDPOINT or OPENAI_API_KEY not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        // Use vision-capable model for OpenRouter, otherwise use default
        let model_name = if std::env::var("OPENAI_ENDPOINT")
            .unwrap_or_default()
            .contains("openrouter")
        {
            "openai/gpt-4o-mini"
        } else {
            provider.model_id.as_ref().unwrap()
        };

        let image_url = TEST_IMAGE_URL;
        let history = History::new(
            model_name.to_string(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage {
                    role: Role::User,
                    content: MessageContent::Contents(vec![
                        Content::Text {
                            content_type: ContentType::Text,
                            text: "Describe the image succinctly.".to_string(),
                        },
                        Content::Image {
                            content_type: ContentType::ImageUrl,
                            image_url: ImageUrl {
                                url: image_url.to_string(),
                                detail: None,
                                media_type: None,
                                additional_params: None,
                            },
                        },
                    ]),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    annotations: None,
                },
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, mut history_msgs) = history.extract_prompt_and_history().unwrap();

        let response: String = match agent.chat(prompt, &mut history_msgs).await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
                    eprintln!("Skipping due to rate limit: {msg}");
                    return;
                }
                eprintln!("OpenAI vision error: {msg}");
                return;
            }
        };
        assert!(!response.is_empty());
    }

    #[tokio::test]
    #[ignore = "calls the live OpenAI API set by OPENAI_ENDPOINT and OPENAI_API_KEY"]
    async fn test_openai_vision_stream() {
        let Some((provider, config)) = openai_provider_and_config() else {
            println!("Skipping OpenAI LLM test: OPENAI_ENDPOINT or OPENAI_API_KEY not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        // Use vision-capable model for OpenRouter, otherwise use default
        let model_name = if std::env::var("OPENAI_ENDPOINT")
            .unwrap_or_default()
            .contains("openrouter")
        {
            "openai/gpt-4o-mini"
        } else {
            provider.model_id.as_ref().unwrap()
        };

        let image_url = TEST_IMAGE_URL;
        let history = History::new(
            model_name.to_string(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage {
                    role: Role::User,
                    content: MessageContent::Contents(vec![
                        Content::Text {
                            content_type: ContentType::Text,
                            text: "Describe the image.".to_string(),
                        },
                        Content::Image {
                            content_type: ContentType::ImageUrl,
                            image_url: ImageUrl {
                                url: image_url.to_string(),
                                detail: None,
                                media_type: None,
                                additional_params: None,
                            },
                        },
                    ]),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    annotations: None,
                },
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, history_msgs) = history.extract_prompt_and_history().unwrap();

        use futures::StreamExt;
        let mut stream = agent.stream_chat(prompt, history_msgs).await;

        let mut chunks = 0;
        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                    chunks += 1;
                }
                Ok(_) => {} // Ignore other stream items
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
                        eprintln!("Skipping due to rate limit: {msg}");
                        return;
                    }
                    eprintln!("OpenAI vision stream error: {msg}");
                    panic!("Stream error: {}", e);
                }
            }
        }

        assert!(!response.is_empty());
        assert!(chunks > 0);
    }

    // -------- Parallel tool-calls (both providers) --------

    #[tokio::test]
    #[ignore = "calls the live Azure OpenAI deployment set by AZURE_OPENAI_*"]
    async fn test_azure_openai_parallel_tool_calls_no_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .tool(ForecastTool)
            .build();

        let prompt =
            "Call both weather and forecast tools for Berlin (3 days), return tool calls only.";
        let mut history = Vec::<Message>::new();
        let response: String = agent.chat(prompt, &mut history).await.unwrap();

        assert!(!response.is_empty());
        assert!(response.contains("Berlin") || response.contains("berlin"));
    }

    #[tokio::test]
    #[ignore = "calls the live Azure OpenAI deployment set by AZURE_OPENAI_*"]
    async fn test_azure_openai_parallel_tool_calls_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .tool(ForecastTool)
            .default_max_turns(3)
            .build();

        use futures::StreamExt;
        let prompt = "Call both weather and forecast tools for Berlin (3 days).";
        let mut stream = agent.stream_chat(prompt, Vec::<Message>::new()).await;

        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                }
                Ok(_) => {}
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        assert!(!response.is_empty());
        assert!(response.contains("Berlin") || response.contains("berlin"));
    }

    #[tokio::test]
    #[ignore = "calls the live OpenAI API set by OPENAI_ENDPOINT and OPENAI_API_KEY"]
    async fn test_openai_parallel_tool_calls_no_stream() {
        let Some((provider, config)) = openai_provider_and_config() else {
            println!("Skipping OpenAI LLM test: OPENAI_ENDPOINT or OPENAI_API_KEY not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .tool(ForecastTool)
            .build();

        let prompt =
            "Call both weather and forecast tools for Berlin (3 days), return tool calls only.";
        let mut history = Vec::<Message>::new();
        let response: String = agent.chat(prompt, &mut history).await.unwrap();

        assert!(!response.is_empty());
        assert!(response.contains("Berlin") || response.contains("berlin"));
    }

    #[tokio::test]
    #[ignore = "calls the live OpenAI API set by OPENAI_ENDPOINT and OPENAI_API_KEY"]
    async fn test_openai_parallel_tool_calls_stream() {
        let Some((provider, config)) = openai_provider_and_config() else {
            println!("Skipping OpenAI LLM test: OPENAI_ENDPOINT or OPENAI_API_KEY not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .tool(ForecastTool)
            .default_max_turns(3)
            .build();

        use futures::StreamExt;
        let prompt = "Call both weather and forecast tools for Berlin (3 days).";
        let mut stream = agent.stream_chat(prompt, Vec::<Message>::new()).await;

        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                }
                Ok(_) => {}
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        assert!(!response.is_empty());
        assert!(response.contains("Berlin") || response.contains("berlin"));
    }

    #[tokio::test]
    async fn azure_forwards_the_output_budget() {
        use crate::llm::test_support::{chat_completion, json_response, serve_once};

        let (endpoint, server) =
            serve_once(json_response(chat_completion("gpt-4o", "length", 2048))).await;
        let model = OpenAIModel::from_provider(
            &ModelProvider {
                api_surface: None,
                provider_name: "openai".to_string(),
                model_id: None,
                version: None,
                params: Some(HashMap::from([
                    ("api_key".to_string(), serde_json::json!("test-key")),
                    ("model_id".to_string(), serde_json::json!("gpt-4o")),
                    ("endpoint".to_string(), serde_json::json!(endpoint)),
                    ("is_azure".to_string(), serde_json::json!(true)),
                ])),
            },
            ProviderEgress::Open,
        )
        .await
        .unwrap();
        let mut history = History::new(
            "gpt-4o".to_string(),
            vec![HistoryMessage::from_string(
                Role::User,
                "Summarize the run.",
            )],
        );
        history.max_completion_tokens = Some(2048);

        let response = model.invoke(&history, None).await.unwrap();

        let body: serde_json::Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(response.choices[0].finish_reason, "length");
    }

    #[tokio::test]
    async fn hosted_chat_completions_fail_on_a_pdf_link_the_run_did_not_inline() {
        use crate::llm::test_support::{chat_completion, json_response, serve_once};

        let (endpoint, server) =
            serve_once(json_response(chat_completion("bit-id", "stop", 1))).await;
        let mut provider = proxy_provider();
        provider
            .params
            .as_mut()
            .unwrap()
            .insert("endpoint".to_string(), serde_json::json!(endpoint));
        let model = OpenAIModel::from_provider_chat_completions(&provider)
            .await
            .unwrap();
        let mut history = History::new(
            "bit-id".to_string(),
            vec![HistoryMessage {
                role: Role::User,
                content: MessageContent::Contents(vec![
                    Content::Text {
                        content_type: ContentType::Text,
                        text: "What does the report say?".to_string(),
                    },
                    Content::Document {
                        content_type: ContentType::DocumentUrl,
                        document_url: "https://bucket.s3.amazonaws.com/runs/r/request/teams/files/0/report.pdf?X-Amz-Signature=secret".to_string(),
                        media_type: Some("application/pdf".to_string()),
                        additional_params: None,
                    },
                ]),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                annotations: None,
            }],
        );
        history.stream = Some(false);

        let error = model.invoke(&history, None).await.unwrap_err().to_string();

        assert!(
            error.contains(
                "The document (application/pdf) from host 'bucket.s3.amazonaws.com' cannot be \
                 sent to OpenAI Chat Completions as a link"
            ),
            "{error}"
        );
        assert!(
            !error.contains("X-Amz") && !error.contains("report.pdf"),
            "{error}"
        );
        assert!(!server.is_finished(), "nothing reached the proxy");
        server.abort();
    }

    #[tokio::test]
    async fn azure_reasoning_deployment_gets_its_own_parameter_names() {
        use crate::llm::test_support::{chat_completion, json_response, serve_once};

        let (endpoint, server) =
            serve_once(json_response(chat_completion("o4-mini", "stop", 12))).await;
        let model = OpenAIModel::from_provider(
            &ModelProvider {
                api_surface: None,
                provider_name: "openai".to_string(),
                model_id: None,
                version: None,
                params: Some(HashMap::from([
                    ("api_key".to_string(), serde_json::json!("test-key")),
                    ("model_id".to_string(), serde_json::json!("o4-mini")),
                    ("endpoint".to_string(), serde_json::json!(endpoint)),
                    ("is_azure".to_string(), serde_json::json!(true)),
                ])),
            },
            ProviderEgress::Open,
        )
        .await
        .unwrap();
        let mut history = History::new(
            "o4-mini".to_string(),
            vec![HistoryMessage::from_string(
                Role::User,
                "Summarize the run.",
            )],
        );
        history.max_completion_tokens = Some(4096);
        history.thinking = Some(HistoryThinking::High);
        history.temperature = Some(0.2);
        history.top_p = Some(0.9);

        model.invoke(&history, None).await.unwrap();

        let body: serde_json::Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["max_completion_tokens"], 4096);
        assert_eq!(body["reasoning_effort"], "high");
        for rejected in ["max_tokens", "reasoning", "temperature", "top_p"] {
            assert!(body.get(rejected).is_none(), "{rejected} in {body}");
        }
    }

    fn custom_openai(endpoint: &str, model: &str, surface: ModelApiSurface) -> ModelProvider {
        ModelProvider {
            api_surface: Some(surface),
            provider_name: "custom:openai".to_string(),
            model_id: None,
            version: None,
            params: Some(HashMap::from([
                ("api_key".to_string(), serde_json::json!("test-key")),
                ("model_id".to_string(), serde_json::json!(model)),
                ("endpoint".to_string(), serde_json::json!(endpoint)),
            ])),
        }
    }

    async fn agent_on(
        endpoint: &str,
        model: &str,
        surface: ModelApiSurface,
    ) -> rig::agent::AgentBuilder<crate::llm::CompletionModelHandle<'static>> {
        OpenAIModel::from_provider_with_surface(&custom_openai(endpoint, model, surface), surface)
            .await
            .unwrap()
            .provider()
            .await
            .unwrap()
            .into_client()
            .agent(model)
    }

    #[tokio::test]
    async fn responses_stream_sends_tool_schemas_as_written() {
        use crate::llm::test_support::{
            assert_sends_tool_as_written, serve_once, sse_response, typeless_tool,
        };
        use futures::StreamExt;
        use rig::completion::Completion;

        let (endpoint, server) = serve_once(sse_response(&[])).await;
        let agent = agent_on(&endpoint, "gpt-5.3-codex", ModelApiSurface::Responses)
            .await
            .build();

        let mut stream = agent
            .completion("Fill in the form.", Vec::<Message>::new())
            .await
            .unwrap()
            .tools(vec![typeless_tool()])
            .stream()
            .await
            .unwrap();
        let _ = stream.next().await;

        let body: serde_json::Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_sends_tool_as_written(&body, &typeless_tool());
    }

    #[tokio::test]
    async fn responses_completion_sends_tool_schemas_as_written_after_the_request_fixup() {
        use crate::llm::test_support::{
            assert_sends_tool_as_written, json_response, responses_completion, serve_once,
            typeless_tool,
        };
        use rig::completion::Completion;

        let (endpoint, server) = serve_once(json_response(responses_completion("gpt-5.4"))).await;
        let agent = agent_on(&endpoint, "gpt-5.4", ModelApiSurface::Responses)
            .await
            .temperature(0.2)
            .additional_params(serde_json::json!({"top_p": 0.9}))
            .build();

        let response = agent
            .completion("Fill in the form.", Vec::<Message>::new())
            .await
            .unwrap()
            .tools(vec![typeless_tool()])
            .send()
            .await
            .unwrap();

        let body: serde_json::Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_sends_tool_as_written(&body, &typeless_tool());
        for rejected in ["temperature", "top_p"] {
            assert!(body.get(rejected).is_none(), "{rejected} in {body}");
        }
        assert_eq!(response.raw_response.finish_reason.as_deref(), Some("stop"));
    }

    async fn responses_body_forcing(
        tools: Vec<rig::completion::ToolDefinition>,
        choice: rig::message::ToolChoice,
    ) -> serde_json::Value {
        use crate::llm::test_support::{json_response, responses_completion, serve_once};
        use rig::completion::Completion;

        let (endpoint, server) = serve_once(json_response(responses_completion("gpt-5.4"))).await;
        let agent = agent_on(&endpoint, "gpt-5.4", ModelApiSurface::Responses)
            .await
            .build();

        agent
            .completion("Fill in the form.", Vec::<Message>::new())
            .await
            .unwrap()
            .tools(tools)
            .tool_choice(choice)
            .send()
            .await
            .unwrap();

        serde_json::from_str(&server.await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn responses_forced_tool_choice_keeps_the_tool_it_forces() {
        use crate::llm::test_support::{assert_sends_tool_as_written, typeless_tool};

        let body =
            responses_body_forcing(vec![typeless_tool()], rig::message::ToolChoice::Required).await;

        assert_eq!(body["tool_choice"], "required", "{body}");
        assert_sends_tool_as_written(&body, &typeless_tool());
    }

    #[tokio::test]
    async fn responses_specific_tool_choice_sends_only_the_named_tool_and_requires_it() {
        use crate::llm::test_support::{assert_sends_tool_as_written, typeless_tool};

        let other = rig::completion::ToolDefinition {
            name: "open_app_page".to_string(),
            description: "Open an app page.".to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        };
        let body = responses_body_forcing(
            vec![other, typeless_tool()],
            rig::message::ToolChoice::Specific {
                function_names: vec![typeless_tool().name],
            },
        )
        .await;

        assert_eq!(body["tool_choice"], "required", "{body}");
        assert_sends_tool_as_written(&body, &typeless_tool());
    }

    #[tokio::test]
    async fn chat_completions_sends_tool_schemas_as_written_without_strict() {
        use crate::llm::test_support::{chat_completion, json_response, serve_once, typeless_tool};
        use rig::completion::Completion;

        let (endpoint, server) =
            serve_once(json_response(chat_completion("gpt-5.4", "stop", 1))).await;
        let agent = agent_on(&endpoint, "gpt-5.4", ModelApiSurface::ChatCompletions)
            .await
            .build();

        agent
            .completion("Fill in the form.", Vec::<Message>::new())
            .await
            .unwrap()
            .tools(vec![typeless_tool()])
            .send()
            .await
            .unwrap();

        let body: serde_json::Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        let tool = typeless_tool();
        assert_eq!(
            body["tools"],
            serde_json::json!([{
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                },
            }])
        );
    }

    #[test]
    fn reasoning_family_comes_from_the_model_id() {
        for (id, expected) in [
            ("gpt-4o", OpenAIReasoning::Unsupported),
            ("gpt-4.1-mini", OpenAIReasoning::Unsupported),
            ("gpt-5-chat-latest", OpenAIReasoning::Unsupported),
            ("o1-mini", OpenAIReasoning::Unsupported),
            ("o4-mini", OpenAIReasoning::OSeries),
            ("gpt-5-mini", OpenAIReasoning::Gpt5 { minor: 0 }),
            ("openai/gpt-5.4", OpenAIReasoning::Gpt5 { minor: 4 }),
            ("gpt-6-astra", OpenAIReasoning::Gpt6),
            ("openai.gpt-oss-120b-1:0", OpenAIReasoning::GptOss),
            ("flowpilot-hosted-bit", OpenAIReasoning::Unknown),
        ] {
            assert_eq!(OpenAIReasoning::of(id), expected, "{id}");
        }
        assert_eq!(
            OpenAIReasoning::Gpt5 { minor: 0 }.effort(HistoryThinking::Off),
            Some("minimal")
        );
        assert_eq!(
            OpenAIReasoning::Gpt5 { minor: 1 }.effort(HistoryThinking::Off),
            Some("none")
        );
        assert_eq!(
            OpenAIReasoning::Gpt6.effort(HistoryThinking::Off),
            Some("low")
        );
        assert_eq!(
            OpenAIReasoning::Unsupported.effort(HistoryThinking::High),
            None
        );
    }

    #[tokio::test]
    async fn reasoning_shape_follows_the_api_surface() {
        let mut history = History::new(
            "gpt-5.4".to_string(),
            vec![HistoryMessage::from_string(Role::User, "hi")],
        );
        history.thinking = Some(HistoryThinking::Mid);
        let provider = |surface| ModelProvider {
            api_surface: Some(surface),
            provider_name: "openai".to_string(),
            model_id: None,
            version: None,
            params: Some(HashMap::from([
                ("api_key".to_string(), serde_json::json!("test-key")),
                ("model_id".to_string(), serde_json::json!("gpt-5.4")),
            ])),
        };

        let responses = OpenAIModel::from_provider_with_surface(
            &provider(ModelApiSurface::Responses),
            ModelApiSurface::Responses,
        )
        .await
        .unwrap()
        .additional_params(&Some(history.clone()))
        .unwrap();
        let chat = OpenAIModel::from_provider_with_surface(
            &provider(ModelApiSurface::ChatCompletions),
            ModelApiSurface::ChatCompletions,
        )
        .await
        .unwrap()
        .additional_params(&Some(history))
        .unwrap();

        assert_eq!(
            responses["reasoning"],
            serde_json::json!({"effort": "medium"})
        );
        assert!(responses.get("reasoning_effort").is_none());
        assert_eq!(chat["reasoning_effort"], "medium");
        assert!(chat.get("reasoning").is_none());
    }

    const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

    fn signed_link(name: &str) -> String {
        format!(
            "https://bucket.s3.amazonaws.com/runs/r/files/0/{name}?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Signature=secret"
        )
    }

    /// A zip with an empty stored entry per name, listed in its central directory.
    fn zip_of(names: &[&str]) -> Vec<u8> {
        let mut zip = Vec::new();
        let mut directory = Vec::new();
        for name in names {
            let offset = u32::try_from(zip.len()).unwrap().to_le_bytes();
            let name_len = u16::try_from(name.len()).unwrap().to_le_bytes();
            zip.extend_from_slice(b"PK\x03\x04");
            zip.extend_from_slice(&[0; 22]);
            zip.extend_from_slice(&name_len);
            zip.extend_from_slice(&[0; 2]);
            zip.extend_from_slice(name.as_bytes());
            directory.extend_from_slice(b"PK\x01\x02");
            directory.extend_from_slice(&[0; 24]);
            directory.extend_from_slice(&name_len);
            directory.extend_from_slice(&[0; 12]);
            directory.extend_from_slice(&offset);
            directory.extend_from_slice(name.as_bytes());
        }
        let entries = u16::try_from(names.len()).unwrap().to_le_bytes();
        let directory_offset = u32::try_from(zip.len()).unwrap().to_le_bytes();
        let directory_size = u32::try_from(directory.len()).unwrap().to_le_bytes();
        zip.extend_from_slice(&directory);
        zip.extend_from_slice(b"PK\x05\x06");
        zip.extend_from_slice(&[0; 4]);
        zip.extend_from_slice(&entries);
        zip.extend_from_slice(&entries);
        zip.extend_from_slice(&directory_size);
        zip.extend_from_slice(&directory_offset);
        zip.extend_from_slice(&[0; 2]);
        zip
    }

    fn text_part(text: &str) -> Content {
        Content::Text {
            content_type: ContentType::Text,
            text: text.to_string(),
        }
    }

    fn document_part(url: &str, media_type: &str) -> Content {
        Content::Document {
            content_type: ContentType::DocumentUrl,
            document_url: url.to_string(),
            media_type: Some(media_type.to_string()),
            additional_params: None,
        }
    }

    fn asking(model: &str, parts: Vec<Content>) -> History {
        let mut history = History::new(
            model.to_string(),
            vec![HistoryMessage {
                role: Role::User,
                content: MessageContent::Contents(parts),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                annotations: None,
            }],
        );
        history.stream = Some(false);
        history
    }

    async fn openai_platform_model(endpoint: &str, model: &str) -> OpenAIModel {
        let surface = ModelApiSurface::Responses;
        let mut openai = OpenAIModel::from_provider_with_surface(
            &custom_openai(endpoint, model, surface),
            surface,
        )
        .await
        .unwrap();
        openai.server = Server::OpenAI;
        openai
    }

    fn input_files(body: &Value) -> Vec<Value> {
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|item| item["content"].as_array().cloned().unwrap_or_default())
            .filter(|part| part["type"] == "input_file")
            .collect()
    }

    #[tokio::test]
    async fn openai_responses_reads_office_files_and_csv_links_as_input_files() {
        use crate::llm::test_support::{json_response, responses_completion, serve_once};

        let (endpoint, server) = serve_once(json_response(responses_completion("gpt-5.4"))).await;
        let workbook = STANDARD.encode(zip_of(&["[Content_Types].xml", "xl/workbook.xml"]));
        let history = asking(
            "gpt-5.4",
            vec![
                text_part("Compare these."),
                document_part(&signed_link("report.docx"), DOCX),
                document_part(&format!("data:{XLSX};base64,{workbook}"), XLSX),
                document_part(&signed_link("table.csv"), "text/csv"),
                document_part(&signed_link("contract.pdf"), "application/pdf"),
            ],
        );

        openai_platform_model(&endpoint, "gpt-5.4")
            .await
            .invoke(&history, None)
            .await
            .unwrap();

        let body = server.await.unwrap();
        assert!(!body.contains(INPUT_FILE_CARRIER), "{body}");
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            input_files(&body),
            vec![
                json!({"type": "input_file", "file_url": signed_link("report.docx"), "filename": "report.docx"}),
                json!({"type": "input_file", "file_data": format!("data:{XLSX};base64,{workbook}"), "filename": "document.xlsx"}),
                json!({"type": "input_file", "file_url": signed_link("table.csv"), "filename": "table.csv"}),
                json!({"type": "input_file", "file_url": signed_link("contract.pdf"), "filename": "document.pdf"}),
            ]
        );
        assert_eq!(body["store"], false, "{body}");
    }

    #[tokio::test]
    async fn streamed_openai_responses_carry_office_files_too() {
        use crate::llm::test_support::{serve_once, sse_response};
        use futures::StreamExt;
        use rig::completion::Completion;

        let (endpoint, server) = serve_once(sse_response(&[])).await;
        let agent = openai_platform_model(&endpoint, "gpt-5.4")
            .await
            .provider()
            .await
            .unwrap()
            .into_client()
            .agent("gpt-5.4")
            .build();
        let prompt = Message::User {
            content: OneOrMany::many(vec![
                UserContent::text("Summarise the deck."),
                UserContent::document_url(signed_link("deck.pptx"), None),
            ])
            .unwrap(),
        };

        let mut stream = agent
            .completion(prompt, Vec::<Message>::new())
            .await
            .unwrap()
            .stream()
            .await
            .unwrap();
        let _ = stream.next().await;

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(
            input_files(&body),
            vec![json!({
                "type": "input_file",
                "file_url": signed_link("deck.pptx"),
                "filename": "deck.pptx",
            })]
        );
    }

    #[tokio::test]
    async fn office_files_stay_notes_on_other_hosts_and_on_chat_completions() {
        use crate::llm::test_support::{
            chat_completion, json_response, responses_completion, serve_once,
        };

        let history = asking(
            "gpt-5.4",
            vec![
                text_part("Read it."),
                document_part(&signed_link("report.docx"), DOCX),
            ],
        );
        let (endpoint, server) = serve_once(json_response(responses_completion("gpt-5.4"))).await;
        let surface = ModelApiSurface::Responses;
        OpenAIModel::from_provider_with_surface(
            &custom_openai(&endpoint, "gpt-5.4", surface),
            surface,
        )
        .await
        .unwrap()
        .invoke(&history, None)
        .await
        .unwrap();
        let other_host = server.await.unwrap();

        let (endpoint, server) =
            serve_once(json_response(chat_completion("gpt-5.4", "stop", 1))).await;
        let mut chat = openai_platform_model(&endpoint, "gpt-5.4").await;
        chat = chat.with_api_surface(ModelApiSurface::ChatCompletions);
        chat.invoke(&history, None).await.unwrap();
        let chat_completions = server.await.unwrap();

        for body in [other_host, chat_completions] {
            assert!(!body.contains("X-Amz-Signature"), "{body}");
            assert!(!body.contains("input_file"), "{body}");
            assert!(
                body.contains("[attachment omitted: unsupported file type document]"),
                "{body}"
            );
        }
    }

    /// Sends a PDF and an image inline over `surface`, as `server` serves it, and returns the
    /// request body.
    async fn sending_a_pdf_and_an_image(
        surface: ModelApiSurface,
        server: Option<Server>,
    ) -> String {
        use crate::llm::test_support::{
            chat_completion, json_response, responses_completion, serve_once,
        };

        let model = "Qwen/Qwen2.5-VL-7B-Instruct";
        let reply = match surface {
            ModelApiSurface::Responses => responses_completion(model),
            _ => chat_completion(model, "stop", 1),
        };
        let (endpoint, sent) = serve_once(json_response(reply)).await;
        let mut openai = OpenAIModel::from_provider_with_surface(
            &custom_openai(&endpoint, model, surface),
            surface,
        )
        .await
        .unwrap();
        assert_eq!(openai.server, Server::Other, "{endpoint}");
        if let Some(server) = server {
            openai.server = server;
        }
        let history = asking(
            model,
            vec![
                text_part("Compare them."),
                document_part(
                    "data:application/pdf;base64,JVBERi0xLjQK",
                    "application/pdf",
                ),
                Content::Image {
                    content_type: ContentType::ImageUrl,
                    image_url: ImageUrl {
                        url: TEST_IMAGE_URL.to_string(),
                        detail: None,
                        media_type: None,
                        additional_params: None,
                    },
                },
            ],
        );

        openai.invoke(&history, None).await.unwrap();
        sent.await.unwrap()
    }

    #[tokio::test]
    async fn servers_other_than_openai_azure_and_the_proxy_get_no_file_part() {
        for surface in [ModelApiSurface::ChatCompletions, ModelApiSurface::Responses] {
            let other = sending_a_pdf_and_an_image(surface, None).await;
            assert!(!other.contains("file_data"), "{surface:?}: {other}");
            assert!(
                other.contains("[attachment omitted: unsupported file type application/pdf]"),
                "{surface:?}: {other}"
            );
            assert!(
                other.contains("data:image/png;base64,iVBORw0KGgo"),
                "{surface:?}: {other}"
            );

            for server in [Server::OpenAI, Server::Azure, Server::HostedProxy] {
                let kept = sending_a_pdf_and_an_image(surface, Some(server)).await;
                assert!(
                    kept.contains("data:application/pdf;base64,JVBERi0xLjQK"),
                    "{surface:?} {server:?}: {kept}"
                );
                assert!(
                    !kept.contains("attachment omitted"),
                    "{surface:?} {server:?}: {kept}"
                );
            }
        }
    }

    fn turn_after_a_tool_call(image_link: Option<&str>) -> CompletionRequest {
        let mut question = vec![UserContent::text("What is in the picture?")];
        if let Some(link) = image_link {
            question.push(UserContent::image_url(
                link,
                Some(rig::message::ImageMediaType::PNG),
                None,
            ));
        }
        let stored = rig::message::Reasoning::summaries(Vec::new()).with_id("rs_stored".into());
        let encrypted = rig::message::Reasoning::encrypted("opaque").with_id("rs_encrypted".into());
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: OneOrMany::many(vec![
                Message::User {
                    content: OneOrMany::many(question).unwrap(),
                },
                Message::Assistant {
                    id: Some("msg_1".to_string()),
                    content: OneOrMany::many(vec![
                        AssistantContent::Reasoning(stored),
                        AssistantContent::Reasoning(encrypted),
                        AssistantContent::tool_call_with_call_id(
                            "fc_1",
                            "call_1".to_string(),
                            "look",
                            json!({}),
                        ),
                    ])
                    .unwrap(),
                },
                Message::tool_result_with_call_id("fc_1", Some("call_1".to_string()), "a cat"),
            ])
            .unwrap(),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        }
    }

    fn replayed_reasoning(model: &str, request: CompletionRequest) -> Vec<String> {
        let wire = serde_json::to_value(
            rig::providers::openai::responses_api::CompletionRequest::try_from((
                model.to_string(),
                request,
            ))
            .unwrap(),
        )
        .unwrap();
        wire["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "reasoning")
            .map(|item| item["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn only_requests_with_media_links_leave_storage_and_replay_encrypted_reasoning() {
        let link = signed_link("cat.png");

        let stored = responses_constraints("gpt-5.4", turn_after_a_tool_call(None)).unwrap();
        assert_eq!(stored.additional_params, None);
        assert_eq!(
            replayed_reasoning("gpt-5.4", stored),
            ["rs_stored", "rs_encrypted"]
        );

        let unstored =
            responses_constraints("gpt-5.4", turn_after_a_tool_call(Some(&link))).unwrap();
        assert_eq!(
            unstored.additional_params,
            Some(json!({"store": false, "include": [ENCRYPTED_REASONING]}))
        );
        assert_eq!(replayed_reasoning("gpt-5.4", unstored), ["rs_encrypted"]);

        let without_reasoning =
            responses_constraints("gpt-4.1", turn_after_a_tool_call(Some(&link))).unwrap();
        assert_eq!(
            without_reasoning.additional_params,
            Some(json!({"store": false}))
        );
    }

    #[test]
    fn the_server_is_told_by_the_provider_and_the_endpoint_host() {
        for (provider, endpoint, expected) in [
            ("openai", None, Server::OpenAI),
            (
                "custom:openai",
                Some("https://api.openai.com/v1"),
                Server::OpenAI,
            ),
            (
                "custom:openai",
                Some("https://eu.api.openai.com/v1/"),
                Server::OpenAI,
            ),
            (
                "custom:openai",
                Some("http://api.openai.com/v1"),
                Server::Other,
            ),
            (
                "custom:openai",
                Some("https://my-resource.openai.azure.com/openai/v1"),
                Server::Azure,
            ),
            (
                "custom:openai",
                Some("https://api.openai.com.example.net/v1"),
                Server::Other,
            ),
            (
                "custom:openai",
                Some("https://openrouter.ai/api/v1"),
                Server::Other,
            ),
            (
                "custom:openai",
                Some("http://127.0.0.1:8000/v1"),
                Server::Other,
            ),
            (
                "custom:bedrock",
                Some("https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1"),
                Server::Other,
            ),
            ("custom:openai", Some("not a url"), Server::Other),
            (
                "hosted:openai",
                Some("https://api.flow-like.com/api/v1"),
                Server::HostedProxy,
            ),
            (
                "hosted",
                Some("http://localhost:8080/api/v1"),
                Server::HostedProxy,
            ),
        ] {
            assert_eq!(
                Server::of(provider, endpoint),
                expected,
                "{provider} {endpoint:?}"
            );
        }
    }

    #[test]
    fn only_https_office_links_become_input_files() {
        for (url, expected) in [
            ("https://files.example.com/report.docx", true),
            ("HTTPS://files.example.com/report.docx", true),
            ("http://files.example.com/report.docx", false),
            ("asset://localhost/%2FUsers%2Fme%2Freport.docx", false),
            ("https://files.example.com/report.odt", false),
        ] {
            let document = Document {
                data: DocumentSourceKind::Url(url.to_string()),
                media_type: None,
                additional_params: None,
            };
            assert_eq!(office_input_file(&document).is_some(), expected, "{url}");
        }
    }

    #[test]
    fn office_packages_are_told_apart_by_the_folders_of_their_parts() {
        for (names, expected) in [
            (
                vec!["[Content_Types].xml", "_rels/.rels", "word/document.xml"],
                Some(OfficeFile::Docx),
            ),
            (
                vec!["[Content_Types].xml", "xl/workbook.xml"],
                Some(OfficeFile::Xlsx),
            ),
            (
                vec!["[Content_Types].xml", "ppt/presentation.xml"],
                Some(OfficeFile::Pptx),
            ),
            (vec!["word/document.xml"], None),
            (vec!["mimetype", "content.xml"], None),
        ] {
            assert_eq!(office_package(&zip_of(&names)), expected, "{names:?}");
        }
        let docx = zip_of(&["[Content_Types].xml", "word/document.xml"]);
        assert_eq!(office_package(&docx[..docx.len() - 4]), None);
        assert_eq!(office_package(b"PK\x03\x04 but not a zip"), None);
    }

    #[test]
    fn a_carrier_that_cannot_be_read_fails_the_call_without_its_link() {
        let plain = Bytes::from_static(br#"{"input":[],"model":"gpt-5.4"}"#);
        assert_eq!(splice_input_files(plain.clone()), Some(plain));

        let broken = format!(
            r#"{{"input":[{{"type":"message","role":"user","content":[{{"type":"input_file","file_id":"{INPUT_FILE_CARRIER}{{\"file_url\":\"{}"}}]}}]}}"#,
            signed_link("report.docx")
        );
        let error = with_input_files(
            Request::post("https://api.openai.com/v1/responses")
                .body(Bytes::from(broken))
                .unwrap(),
        )
        .unwrap_err()
        .to_string();
        assert!(!error.contains("X-Amz"), "{error}");
    }
}
