//! The hop between a server executor and the API's private-model relay.
//!
//! Requests are OpenAI Chat Completions as rig's completions client emits them, with
//! `model` = the private Bit's id and `"thinking": "off" | "low" | "mid" | "high"`. The media
//! parts of a user message are Flow-Like content parts instead, so a link travels as the run
//! holds it and only media that exists as bytes travels inline:
//!
//! - `{"type":"image_url","image_url":{"url":V,"detail":"auto|low|high","media_type":M}}`
//! - `{"type":"document_url","document_url":V,"media_type":M}` for PDFs and text documents
//! - `{"type":"audio_url","audio_url":V,"media_type":M}`
//! - `{"type":"video_url","video_url":V,"media_type":M}`
//!
//! `V` is an `https` (or `http`) link, which the relay resolves the way a direct call with the
//! same Bit would, or a base64 `data:` URL. In a data URL, `;flow-like-source=string` marks a
//! text document whose payload is its UTF-8 text and `;flow-like-source=raw` raw bytes. `M` is a
//! MIME type; it, `detail` and `additional_params` are optional. A request is at most
//! [`MAX_REQUEST_BYTES`], inline media included.
//!
//! `stream: true` answers with SSE chunk frames, `: keepalive` comments and a closing
//! `data: [DONE]`; `stream: false` answers with a `chat.completion` body that may follow
//! whitespace keepalives. Once the relay has committed its response, a failure arrives in band
//! as `{"error":{"message","type"}}`, as the body or as a stream frame.

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use rig::completion::{CompletionRequest, Message as RigMessage};
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, Request, Response as HttpResponse,
    StreamingResponse, sse::BoxedStream,
};
use rig::message::{Document, DocumentSourceKind, ToolChoice as RigToolChoice, UserContent};
use rig::providers::openai::completion::Message as WireMessage;
use rig::wasm_compat::WasmCompatSend;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

use super::{ModelConstructor, ModelLogic, UsageReportingMode, merge_additional_params};
use crate::authorization::AuthorizedHttpClient;
use crate::history::{
    Content, ContentType, History, HistoryFunction, HistoryFunctionParameters,
    HistoryJSONSchemaDefine, HistoryJSONSchemaType, HistoryMessage, HistoryThinking, ImageUrl,
    MessageContent, ResponseFormat, Role, Tool, ToolChoice, ToolType,
};
use crate::response::{Choice, Response, Usage};
use crate::response_chunk::{ResponseChunk, ResponseChunkChoice};

/// Chat route of the relay, under the API's `/api/v1/execution` router. [`RelayModel`] posts
/// here when its base URL is `{api}/api/v1/execution/apps/{app_id}/models`.
pub const RELAY_ROUTE_CHAT: &str = "/apps/{app_id}/models/chat/completions";
/// SystemOne route of the relay, under the API's `/api/v1/execution` router.
pub const RELAY_ROUTE_SYSTEMONE: &str = "/apps/{app_id}/models/systemone";

/// The largest request the relay takes, inline media included: AWS Lambda's synchronous
/// payload limit.
pub const MAX_REQUEST_BYTES: usize = 6_000_000;

/// Prefix of the file id that carries a media part from the request fixup to
/// [`RelayHttpClient`].
const MEDIA_CARRIER: &str = "flow-like-relay-media:";

/// A private Bit that the API relay calls with its owner's key. The executor holds no provider
/// secret; it authenticates with its run token.
pub struct RelayModel {
    client: rig::providers::openai::CompletionsClient<RelayHttpClient>,
    bit_id: String,
}

impl RelayModel {
    /// `base_url` is `{api}/api/v1/execution/apps/{app_id}/models`; `token` is the executor JWT.
    pub fn new(base_url: &str, token: &str, bit_id: &str) -> Result<Self> {
        let base_url = base_url.trim().trim_end_matches('/');
        if base_url.is_empty() || token.trim().is_empty() || bit_id.trim().is_empty() {
            bail!("The model relay needs a base URL, a run token and a Bit id");
        }
        let client = rig::providers::openai::CompletionsClient::builder()
            .api_key(token)
            .http_client(RelayHttpClient::default())
            .base_url(base_url)
            .build()?;
        Ok(Self {
            client,
            bit_id: bit_id.to_string(),
        })
    }
}

#[async_trait]
impl ModelLogic for RelayModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor::with_request_fixup(
            self.client.clone(),
            relay_request,
        ))
    }

    async fn default_model(&self) -> Option<String> {
        Some(self.bit_id.clone())
    }

    fn usage_reporting(&self) -> UsageReportingMode {
        UsageReportingMode::OpenAIStreamOptions
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        merge_additional_params(
            history.build_additional_params().ok().flatten(),
            history
                .thinking
                .map(|thinking| json!({ "thinking": thinking })),
        )
    }
}

fn relay_request(model: &str, request: CompletionRequest) -> CompletionRequest {
    carry_media(require_named_tools(model, request))
}

/// Rig's converter refuses media links, video and most inline documents, so each media part of a
/// user message reaches [`RelayHttpClient`] as a file id carrying its hop part, which
/// [`splice_media`] puts back in its place.
fn carry_media(mut request: CompletionRequest) -> CompletionRequest {
    for message in request.chat_history.iter_mut() {
        let RigMessage::User { content } = message else {
            continue;
        };
        for part in content.iter_mut() {
            if matches!(
                part,
                UserContent::Image(_)
                    | UserContent::Audio(_)
                    | UserContent::Video(_)
                    | UserContent::Document(_)
            ) {
                let media = std::mem::replace(part, UserContent::text(String::new()));
                *part = media_carrier(media);
            }
        }
    }
    request
}

fn media_carrier(media: UserContent) -> UserContent {
    let part = serde_json::to_string(&HopPart::from(Content::from(media))).unwrap_or_default();
    UserContent::Document(Document {
        data: DocumentSourceKind::FileId(format!("{MEDIA_CARRIER}{part}")),
        media_type: None,
        additional_params: None,
    })
}

/// A content part of a user message on the hop.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HopPart {
    Text {
        text: String,
    },
    ImageUrl {
        image_url: ImageUrl,
    },
    DocumentUrl {
        document_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        additional_params: Option<Value>,
    },
    AudioUrl {
        audio_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        additional_params: Option<Value>,
    },
    VideoUrl {
        video_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        additional_params: Option<Value>,
    },
}

impl HopPart {
    /// The kind and value of a media part.
    fn media(&self) -> Option<(&'static str, &str)> {
        match self {
            Self::Text { .. } => None,
            Self::ImageUrl { image_url } => Some(("images", &image_url.url)),
            Self::DocumentUrl { document_url, .. } => Some(("documents", document_url)),
            Self::AudioUrl { audio_url, .. } => Some(("audio", audio_url)),
            Self::VideoUrl { video_url, .. } => Some(("video", video_url)),
        }
    }
}

impl From<Content> for HopPart {
    fn from(content: Content) -> Self {
        match content {
            Content::Text { text, .. } => Self::Text { text },
            Content::Image { image_url, .. } => Self::ImageUrl { image_url },
            Content::Document {
                document_url,
                media_type,
                additional_params,
                ..
            } => Self::DocumentUrl {
                document_url,
                media_type,
                additional_params,
            },
            Content::Audio {
                audio_url,
                media_type,
                additional_params,
                ..
            } => Self::AudioUrl {
                audio_url,
                media_type,
                additional_params,
            },
            Content::Video {
                video_url,
                media_type,
                additional_params,
                ..
            } => Self::VideoUrl {
                video_url,
                media_type,
                additional_params,
            },
        }
    }
}

impl TryFrom<HopPart> for Content {
    type Error = anyhow::Error;

    fn try_from(part: HopPart) -> Result<Self> {
        if let Some((kind, value)) = part.media() {
            ensure_relayable(kind, value)?;
        }
        Ok(match part {
            HopPart::Text { text } => Self::Text {
                content_type: ContentType::Text,
                text,
            },
            HopPart::ImageUrl { image_url } => Self::Image {
                content_type: ContentType::ImageUrl,
                image_url,
            },
            HopPart::DocumentUrl {
                document_url,
                media_type,
                additional_params,
            } => Self::Document {
                content_type: ContentType::DocumentUrl,
                document_url,
                media_type,
                additional_params,
            },
            HopPart::AudioUrl {
                audio_url,
                media_type,
                additional_params,
            } => Self::Audio {
                content_type: ContentType::AudioUrl,
                audio_url,
                media_type,
                additional_params,
            },
            HopPart::VideoUrl {
                video_url,
                media_type,
                additional_params,
            } => Self::Video {
                content_type: ContentType::VideoUrl,
                video_url,
                media_type,
                additional_params,
            },
        })
    }
}

/// The hop carries media as an http(s) link or base64 data. The value is never echoed: a signed
/// link is a credential.
fn ensure_relayable(kind: &str, value: &str) -> Result<()> {
    let refused = |form: &str| {
        anyhow!("The model relay takes {kind} as http(s) links or base64 data: URLs, not as {form}")
    };
    if let Some(data) = value.strip_prefix("data:") {
        return match data.split_once(',') {
            Some((metadata, _)) if metadata.ends_with(";base64") => Ok(()),
            _ => Err(refused("a data: URL that is not base64")),
        };
    }
    if value.starts_with("file_id:") {
        return Err(refused("a provider file id"));
    }
    match reqwest::Url::parse(value) {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some_and(|host| !host.is_empty()) =>
        {
            Ok(())
        }
        Ok(url) => Err(refused(&format!("a {} link", url.scheme()))),
        Err(_) => Err(refused("a value that is neither")),
    }
}

/// The request body as the hop sends it: carried media back in place, within
/// [`MAX_REQUEST_BYTES`].
fn hop_request<T: Into<Bytes>>(request: Request<T>) -> http_client::Result<Request<Bytes>> {
    let (parts, body) = request.into_parts();
    let body = hop_body(body.into()).map_err(|error| {
        http_client::Error::Instance(Box::new(std::io::Error::other(error.to_string())))
    })?;
    Ok(Request::from_parts(parts, body))
}

fn hop_body(body: Bytes) -> Result<Bytes> {
    let carries_media = body
        .windows(MEDIA_CARRIER.len())
        .any(|window| window == MEDIA_CARRIER.as_bytes());
    let (body, inline) = if carries_media {
        splice_media(&body)?
    } else {
        (body, 0)
    };
    if body.len() <= MAX_REQUEST_BYTES {
        return Ok(body);
    }
    if inline > 0 {
        bail!(
            "The private model call is {} bytes, more than the {MAX_REQUEST_BYTES} bytes the \
             model relay takes; {inline} bytes of it are media sent inline because the run holds \
             them only as bytes. Pass large files as links, such as signed storage URLs, instead",
            body.len()
        );
    }
    bail!(
        "The private model call is {} bytes, more than the {MAX_REQUEST_BYTES} bytes the model \
         relay takes",
        body.len()
    )
}

/// Puts each carried media part back in its message and counts the bytes of inline media.
fn splice_media(body: &[u8]) -> Result<(Bytes, usize)> {
    let mut request: Value = serde_json::from_slice(body)?;
    let mut inline = 0;
    let parts = request
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(|message| message.get_mut("content").and_then(Value::as_array_mut))
        .flatten();
    for part in parts {
        let Some(carried) = part
            .pointer("/file/file_id")
            .and_then(Value::as_str)
            .and_then(|file_id| file_id.strip_prefix(MEDIA_CARRIER))
        else {
            continue;
        };
        let media: HopPart = serde_json::from_str(carried)?;
        if let Some((kind, value)) = media.media() {
            ensure_relayable(kind, value)?;
            if value.starts_with("data:") {
                inline += value.len();
            }
        }
        *part = serde_json::to_value(&media)?;
    }
    Ok((Bytes::from(serde_json::to_vec(&request)?), inline))
}

/// Rig's OpenAI client refuses a specific tool choice, so the request keeps only the named tools
/// and requires one of them, which every provider behind the relay accepts. Names that match no
/// tool stay specific, and rig refuses the request before it is sent.
fn require_named_tools(_model: &str, mut request: CompletionRequest) -> CompletionRequest {
    let Some(RigToolChoice::Specific { function_names }) = request.tool_choice.clone() else {
        return request;
    };
    let all_defined = function_names
        .iter()
        .all(|name| request.tools.iter().any(|tool| &tool.name == name));
    if function_names.is_empty() || !all_defined {
        return request;
    }
    request
        .tools
        .retain(|tool| function_names.contains(&tool.name));
    request.tool_choice = Some(RigToolChoice::Required);
    request
}

/// Puts the carried media of a request back in place and keeps it within
/// [`MAX_REQUEST_BYTES`]. Rig's OpenAI client skips a stream frame it cannot parse, misreads an
/// error body and takes a stream that stops early for a finished answer, so the relay's in-band
/// errors and a stream that ends before `data: [DONE]` become request errors here.
#[derive(Clone, Debug, Default)]
struct RelayHttpClient(AuthorizedHttpClient);

impl HttpClientExt for RelayHttpClient {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<HttpResponse<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let request = hop_request(request);
        let client = self.0.clone();
        async move {
            let response = client.send::<Bytes, Bytes>(request?).await?;
            let (parts, body) = response.into_parts();
            let body: LazyBody<U> = Box::pin(async move {
                let body = body.await?;
                match in_band_error(body.trim_ascii_start()) {
                    Some(error) => Err(error),
                    None => Ok(U::from(body)),
                }
            });
            Ok(HttpResponse::from_parts(parts, body))
        }
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
        let request = hop_request(request);
        let client = self.0.clone();
        async move {
            let response = client.send_streaming(request?).await?;
            Ok(response.map(surface_stream_errors))
        }
    }
}

/// Passes the stream through until a complete `data: {"error":…}` line, which ends it with
/// that error after the lines before it. A stream that ends without `data: [DONE]` fails too.
fn surface_stream_errors(mut body: BoxedStream) -> BoxedStream {
    Box::pin(async_stream::stream! {
        let mut line = Vec::new();
        let mut done = false;
        while let Some(chunk) = body.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };
            let mut start = 0;
            let line_ends = chunk
                .iter()
                .enumerate()
                .filter_map(|(index, byte)| (*byte == b'\n').then_some(index));
            for end in line_ends {
                line.extend_from_slice(&chunk[start..end]);
                if let Some(error) = sse_data(&line).and_then(in_band_error) {
                    if start > 0 {
                        yield Ok(chunk.slice(..start));
                    }
                    yield Err(error);
                    return;
                }
                done |= closes_stream(&line);
                line.clear();
                start = end + 1;
            }
            line.extend_from_slice(&chunk[start..]);
            yield Ok(chunk);
        }
        if !done && !closes_stream(&line) {
            yield Err(http_client::Error::Instance(Box::new(std::io::Error::other(
                "Model relay failed: the stream ended before its closing data: [DONE], so the \
                 answer is incomplete",
            ))));
        }
    })
}

fn sse_data(line: &[u8]) -> Option<&[u8]> {
    line.strip_prefix(b"data:").map(<[u8]>::trim_ascii)
}

fn closes_stream(line: &[u8]) -> bool {
    sse_data(line).is_some_and(|data| data == b"[DONE]")
}

fn in_band_error(json: &[u8]) -> Option<http_client::Error> {
    #[derive(Deserialize)]
    struct Envelope {
        error: Value,
    }

    let opens_with_error = json
        .strip_prefix(b"{")
        .is_some_and(|rest| rest.trim_ascii_start().starts_with(b"\"error\""));
    if !opens_with_error {
        return None;
    }
    let Envelope { error } = serde_json::from_slice(json).ok()?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or("no message");
    let message = match error.get("type").and_then(Value::as_str) {
        Some(kind) => format!("Model relay failed ({kind}): {message}"),
        None => format!("Model relay failed: {message}"),
    };
    Some(http_client::Error::Instance(Box::new(
        std::io::Error::other(message),
    )))
}

#[derive(Deserialize)]
struct RelayRequest {
    model: String,
    messages: Vec<Value>,
    tools: Option<Vec<RelayTool>>,
    tool_choice: Option<Value>,
    response_format: Option<Value>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>,
    stop: Option<StopSequences>,
    seed: Option<u32>,
    presence_penalty: Option<f32>,
    frequency_penalty: Option<f32>,
    thinking: Option<String>,
    stream: Option<bool>,
}

#[derive(Deserialize)]
struct RelayTool {
    function: RelayFunction,
}

#[derive(Deserialize)]
struct RelayFunction {
    name: String,
    description: Option<String>,
    parameters: Option<Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StopSequences {
    One(String),
    Many(Vec<String>),
}

/// Reads a hop request into the History the API invokes the private Bit with, and whether the
/// executor asked for a stream. `History::model` carries the requested Bit id.
///
/// Media parts keep their links and inline data as the executor sent them; any other media value
/// is refused. Tool schemas keep what History can express (`integer` becomes `number`). The hop
/// carries no signed reasoning, so thinking is turned off once an assistant turn has tool calls:
/// providers such as Anthropic refuse a tool continuation whose thinking lost its signature.
pub fn history_from_relay_request(body: &Value) -> Result<(History, bool)> {
    let request = RelayRequest::deserialize(body)
        .map_err(|error| anyhow!("Invalid model relay request: {error}"))?;
    if request.model.trim().is_empty() {
        bail!("The model relay request names no model");
    }
    if request.messages.is_empty() {
        bail!("The model relay request has no messages");
    }
    let messages = request
        .messages
        .into_iter()
        .map(history_message)
        .collect::<Result<Vec<_>>>()?;
    let has_tool_turn = messages.iter().any(|message| {
        message.role == Role::Assistant
            && message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| !calls.is_empty())
    });
    let thinking = request
        .thinking
        .as_deref()
        .map(str::parse::<HistoryThinking>)
        .transpose()
        .map_err(|error| anyhow!("Invalid model relay request: {error}"))?;
    let thinking = match thinking {
        Some(requested) if requested != HistoryThinking::Off && has_tool_turn => {
            tracing::debug!(
                ?requested,
                "Relayed tool calls carry no signed reasoning; turning thinking off"
            );
            Some(HistoryThinking::Off)
        }
        thinking => thinking,
    };
    let tools: Vec<Tool> = request
        .tools
        .unwrap_or_default()
        .into_iter()
        .map(history_tool)
        .collect();
    let tool_choice = request
        .tool_choice
        .as_ref()
        .map(|choice| history_tool_choice(choice, &tools))
        .transpose()?;
    let stream = request.stream.unwrap_or(false);

    let mut history = History::new(request.model, messages);
    history.stream = Some(stream);
    history.temperature = request.temperature;
    history.top_p = request.top_p;
    history.max_completion_tokens = request.max_completion_tokens.or(request.max_tokens);
    history.stop = request.stop.map(|stop| match stop {
        StopSequences::One(stop) => vec![stop],
        StopSequences::Many(stops) => stops,
    });
    history.seed = request.seed;
    history.presence_penalty = request.presence_penalty;
    history.frequency_penalty = request.frequency_penalty;
    history.response_format = request
        .response_format
        .map(history_response_format)
        .transpose()?;
    history.thinking = thinking;
    history.tools = (!tools.is_empty()).then_some(tools);
    history.tool_choice = tool_choice;
    Ok((history, stream))
}

fn history_message(message: Value) -> Result<HistoryMessage> {
    if message.get("role").and_then(Value::as_str) == Some("user") {
        return user_message(&message);
    }
    let message = WireMessage::deserialize(message)
        .map_err(|error| anyhow!("Invalid model relay message: {error}"))?;
    if let WireMessage::System { content, .. } = &message {
        let text = content
            .iter()
            .map(|part| part.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(HistoryMessage::from_string(Role::System, &text));
    }
    let message = RigMessage::try_from(message)
        .map_err(|error| anyhow!("Invalid model relay message: {error}"))?;
    Ok(message.into())
}

/// A user turn with its text and media parts as the hop carries them.
fn user_message(message: &Value) -> Result<HistoryMessage> {
    let content = match message.get("content") {
        Some(Value::String(text)) => MessageContent::String(text.clone()),
        Some(Value::Array(parts)) if !parts.is_empty() => {
            let parts = parts
                .iter()
                .map(|part| {
                    HopPart::deserialize(part)
                        .map_err(|error| anyhow!("Invalid model relay message: {error}"))
                        .and_then(Content::try_from)
                })
                .collect::<Result<Vec<_>>>()?;
            match parts.as_slice() {
                [Content::Text { text, .. }] => MessageContent::String(text.clone()),
                _ => MessageContent::Contents(parts),
            }
        }
        _ => bail!("Invalid model relay message: a user message needs text or content parts"),
    };
    Ok(HistoryMessage {
        role: Role::User,
        content,
        name: None,
        tool_calls: None,
        tool_call_id: None,
        annotations: None,
    })
}

fn history_tool(tool: RelayTool) -> Tool {
    let RelayFunction {
        name,
        description,
        parameters,
    } = tool.function;
    let parameters = parameters.unwrap_or_default();
    Tool {
        tool_type: ToolType::Function,
        function: HistoryFunction {
            name,
            description: description.filter(|description| !description.is_empty()),
            parameters: HistoryFunctionParameters {
                schema_type: schema_type(&parameters).unwrap_or(HistoryJSONSchemaType::Object),
                properties: schema_properties(&parameters),
                required: schema_required(&parameters),
            },
        },
    }
}

fn schema_define(schema: &Value) -> HistoryJSONSchemaDefine {
    let enum_values: Vec<String> = schema
        .get("enum")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    HistoryJSONSchemaDefine {
        schema_type: schema_type(schema),
        description: schema
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        default: schema.get("default").cloned(),
        enum_values: (!enum_values.is_empty()).then_some(enum_values),
        properties: schema_properties(schema),
        required: schema_required(schema),
        items: schema
            .get("items")
            .map(|items| Box::new(schema_define(items))),
    }
}

fn schema_type(schema: &Value) -> Option<HistoryJSONSchemaType> {
    let name = match schema.get("type")? {
        Value::String(name) => name.as_str(),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .find(|name| *name != "null")?,
        _ => return None,
    };
    Some(match name {
        "object" => HistoryJSONSchemaType::Object,
        "number" | "integer" => HistoryJSONSchemaType::Number,
        "string" => HistoryJSONSchemaType::String,
        "array" => HistoryJSONSchemaType::Array,
        "boolean" => HistoryJSONSchemaType::Boolean,
        "null" => HistoryJSONSchemaType::Null,
        _ => return None,
    })
}

fn schema_properties(schema: &Value) -> Option<HashMap<String, Box<HistoryJSONSchemaDefine>>> {
    let properties = schema.get("properties")?.as_object()?;
    Some(
        properties
            .iter()
            .map(|(name, property)| (name.clone(), Box::new(schema_define(property))))
            .collect(),
    )
}

fn schema_required(schema: &Value) -> Option<Vec<String>> {
    let required = schema.get("required")?.as_array()?;
    Some(
        required
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    )
}

fn history_tool_choice(choice: &Value, tools: &[Tool]) -> Result<ToolChoice> {
    if let Some(name) = choice.pointer("/function/name").and_then(Value::as_str) {
        let tool = tools
            .iter()
            .find(|tool| tool.function.name == name)
            .ok_or_else(|| {
                anyhow!(
                    "The model relay request forces the tool `{name}`, which it does not define"
                )
            })?;
        return Ok(ToolChoice::Specific {
            r#type: ToolType::Function,
            function: tool.function.clone(),
        });
    }
    match choice.as_str() {
        Some("none") => Ok(ToolChoice::None),
        Some("auto") => Ok(ToolChoice::Auto),
        Some("required") => Ok(ToolChoice::Required),
        _ => Err(anyhow!(
            "The model relay request has an unknown tool_choice: {choice}"
        )),
    }
}

fn history_response_format(format: Value) -> Result<ResponseFormat> {
    match format {
        Value::String(format) => Ok(ResponseFormat::String(format)),
        format @ Value::Object(_) => Ok(ResponseFormat::Object(format)),
        _ => Err(anyhow!(
            "The model relay request's response_format must be an object"
        )),
    }
}

/// The hop frame for one chunk of a relayed stream. `reasoning` travels as `reasoning_content`;
/// media parts are dropped. Rig reports each tool call's deltas and then the finished call, and
/// some providers reuse call ids, so only finished calls travel: whole, indexed by their place in
/// the frame and with `finish_reason: tool_calls`, which makes rig complete them at once.
pub fn relay_chunk(chunk: &ResponseChunk) -> Value {
    let mut frame = Map::new();
    frame.insert("id".into(), json!(chunk.id));
    frame.insert(
        "model".into(),
        json!(chunk.model.as_deref().unwrap_or_default()),
    );
    frame.insert(
        "choices".into(),
        chunk.choices.iter().map(relay_chunk_choice).collect(),
    );
    if let Some(usage) = &chunk.usage {
        frame.insert("usage".into(), relay_usage(usage));
    }
    Value::Object(frame)
}

fn relay_chunk_choice(choice: &ResponseChunkChoice) -> Value {
    let mut delta = Map::new();
    let mut finish_reason = choice.finish_reason.clone();
    if let Some(chunk_delta) = &choice.delta {
        for (key, value) in [
            ("role", &chunk_delta.role),
            ("content", &chunk_delta.content),
            ("reasoning_content", &chunk_delta.reasoning),
        ] {
            if let Some(value) = value {
                delta.insert(key.into(), json!(value));
            }
        }
        let tool_calls: Vec<Value> = chunk_delta
            .tool_calls
            .iter()
            .flatten()
            .filter_map(|call| {
                Some(json!({
                    "id": call.id,
                    "type": "function",
                    "function": {
                        "name": call.function.name.as_deref()?,
                        "arguments": call.function.arguments.as_deref()?,
                    },
                }))
            })
            .enumerate()
            .map(|(index, mut call)| {
                call["index"] = json!(index);
                call
            })
            .collect();
        if !tool_calls.is_empty() {
            delta.insert("tool_calls".into(), Value::Array(tool_calls));
            finish_reason = Some("tool_calls".into());
        }
    }
    let mut frame_choice = Map::new();
    frame_choice.insert("index".into(), json!(choice.index.max(0)));
    frame_choice.insert("delta".into(), Value::Object(delta));
    if let Some(finish_reason) = finish_reason {
        frame_choice.insert("finish_reason".into(), json!(finish_reason));
    }
    Value::Object(frame_choice)
}

/// The hop's `chat.completion` body for a finished response. Media parts are dropped.
pub fn relay_completion(response: &Response) -> Value {
    json!({
        "id": response.id.as_deref().unwrap_or_default(),
        "object": "chat.completion",
        "created": response.created.unwrap_or_default(),
        "model": response.model.as_deref().unwrap_or_default(),
        "choices": response.choices.iter().map(relay_choice).collect::<Vec<_>>(),
        "usage": relay_usage(&response.usage),
    })
}

fn relay_choice(choice: &Choice) -> Value {
    let message = &choice.message;
    let mut relayed = Map::new();
    relayed.insert("role".into(), json!("assistant"));
    if let Some(content) = &message.content {
        relayed.insert("content".into(), json!(content));
    }
    if let Some(reasoning) = message.reasoning.as_ref().filter(|text| !text.is_empty()) {
        relayed.insert("reasoning_content".into(), json!(reasoning));
    }
    if !message.tool_calls.is_empty() {
        relayed.insert(
            "tool_calls".into(),
            message
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.function.name,
                            "arguments": call.function.arguments,
                        },
                    })
                })
                .collect(),
        );
    }
    let finish_reason = match choice.finish_reason.as_str() {
        "" => "stop",
        reason => reason,
    };
    json!({
        "index": choice.index.max(0),
        "message": relayed,
        "finish_reason": finish_reason,
    })
}

/// The relay's error body, also sent in band once its response is committed.
pub fn relay_error(kind: &str, message: &str) -> Value {
    json!({ "error": { "message": message, "type": kind } })
}

/// Rig reads completion tokens as `total - prompt`, so the total is exactly their sum.
fn relay_usage(usage: &Usage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.prompt_tokens.saturating_add(usage.completion_tokens),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{ToolCall, ToolCallFunction};
    use crate::llm::LLMCallback;
    use rig::OneOrMany;
    use rig::completion::Usage as RigUsage;
    use rig::message::AssistantContent;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::task::JoinHandle;

    const BIT: &str = "my-claude";
    const TOKEN: &str = "exec-jwt";
    const IMAGE: &str = "data:image/png;base64,iVBORw0KGgo=";
    const PDF: &str = "data:application/pdf;base64,JVBERi0xLjQK";
    const WAV: &str = "data:audio/wav;base64,UklGRiQAAABXQVZF";

    /// Answers one request with `response` at the relay base of app `app-1`, yielding the
    /// request head and JSON body.
    async fn relay_endpoint(response: String) -> (RelayModel, JoinHandle<(String, Value)>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!(
            "http://{}/api/v1/execution/apps/app-1/models",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 16 * 1024];
            let (head_len, body_len) = loop {
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0, "client closed before sending its headers");
                request.extend_from_slice(&buffer[..read]);
                if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let body_len = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break (end + 4, body_len);
                }
            };
            while request.len() < head_len + body_len {
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0, "client closed before sending its body");
                request.extend_from_slice(&buffer[..read]);
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            let head = String::from_utf8_lossy(&request[..head_len]).into_owned();
            let body = serde_json::from_slice(&request[head_len..head_len + body_len]).unwrap();
            (head, body)
        });
        (RelayModel::new(&base, TOKEN, BIT).unwrap(), server)
    }

    fn reply(content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\nconnection: close\r\n\r\n{body}"
        )
    }

    fn sse(frames: &[Value]) -> String {
        reply(
            "text/event-stream",
            &format!("{}: keepalive\n\ndata: [DONE]\n\n", sse_frames(frames)),
        )
    }

    fn sse_frames(frames: &[Value]) -> String {
        frames
            .iter()
            .map(|frame| format!(": keepalive\n\ndata: {frame}\n\n"))
            .collect()
    }

    fn completion(response: &Response) -> String {
        reply(
            "application/json",
            &format!(" \n  \n {}", relay_completion(response)),
        )
    }

    fn recording_callback() -> (LLMCallback, Arc<Mutex<Vec<ResponseChunk>>>) {
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let sink = chunks.clone();
        let callback: LLMCallback = Arc::new(move |chunk| {
            sink.lock().unwrap().push(chunk);
            Box::pin(async { Ok(()) })
        });
        (callback, chunks)
    }

    fn message(role: Role, content: MessageContent) -> HistoryMessage {
        HistoryMessage {
            role,
            content,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            annotations: None,
        }
    }

    fn tool(name: &str) -> Tool {
        serde_json::from_value(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": format!("Runs {name}"),
                "parameters": {
                    "type": "object",
                    "properties": {
                        "city": {"type": "string", "description": "City name"},
                        "days": {"type": "number", "default": 1},
                        "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]}
                    },
                    "required": ["city"]
                }
            }
        }))
        .unwrap()
    }

    /// System prompt and a user turn with text, image, PDF and wav.
    fn first_turn() -> History {
        let mut history = History::new(
            BIT.to_string(),
            vec![
                HistoryMessage::from_string(Role::System, "Answer briefly."),
                message(
                    Role::User,
                    MessageContent::Contents(vec![
                        Content::Text {
                            content_type: ContentType::Text,
                            text: "Compare Paris and Rome.".to_string(),
                        },
                        Content::Image {
                            content_type: ContentType::ImageUrl,
                            image_url: ImageUrl {
                                url: IMAGE.to_string(),
                                detail: None,
                                media_type: None,
                                additional_params: None,
                            },
                        },
                        Content::Document {
                            content_type: ContentType::DocumentUrl,
                            document_url: PDF.to_string(),
                            media_type: None,
                            additional_params: None,
                        },
                        Content::Audio {
                            content_type: ContentType::AudioUrl,
                            audio_url: WAV.to_string(),
                            media_type: None,
                            additional_params: None,
                        },
                    ]),
                ),
            ],
        );
        history.tools = Some(vec![tool("get_weather"), tool("get_forecast")]);
        history.thinking = Some(HistoryThinking::High);
        history
    }

    /// The first turn answered by a `get_weather` call and its result.
    fn tool_turn() -> History {
        let mut history = first_turn();
        history.push_message(HistoryMessage {
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                r#type: "function".to_string(),
                function: ToolCallFunction {
                    name: "get_weather".to_string(),
                    arguments: r#"{"city":"Paris"}"#.to_string(),
                },
            }]),
            ..message(Role::Assistant, MessageContent::String(String::new()))
        });
        history.push_message(HistoryMessage {
            tool_call_id: Some("call_1".to_string()),
            ..message(
                Role::Tool,
                MessageContent::String("Sunny, 24°C".to_string()),
            )
        });
        history
    }

    fn sent_history(body: &Value) -> History {
        let (history, _) = history_from_relay_request(body).unwrap();
        history
    }

    fn weather_call(id: &str, city: &str) -> rig::message::ToolCall {
        let AssistantContent::ToolCall(call) =
            AssistantContent::tool_call(id, "get_weather", json!({ "city": city }))
        else {
            unreachable!("tool_call builds a tool call")
        };
        call
    }

    fn assert_two_weather_calls(response: &Response) {
        let calls: Vec<(&str, &str, Value)> = response.choices[0]
            .message
            .tool_calls
            .iter()
            .map(|call| {
                (
                    call.id.as_str(),
                    call.function.name.as_str(),
                    serde_json::from_str(&call.function.arguments).unwrap(),
                )
            })
            .collect();
        assert_eq!(
            calls,
            vec![
                ("call_a", "get_weather", json!({"city": "Paris"})),
                ("call_b", "get_weather", json!({"city": "Rome"})),
            ]
        );
    }

    #[tokio::test]
    async fn history_round_trips_through_the_hop() {
        let mut history = tool_turn();
        history.push_message(HistoryMessage::from_string(Role::User, "And tomorrow?"));
        history.tool_choice = Some(ToolChoice::Required);
        history.response_format = Some(ResponseFormat::Object(json!({
            "type": "json_schema",
            "json_schema": {"name": "answer", "schema": {"type": "object"}},
        })));
        history.temperature = Some(0.2);
        history.top_p = Some(0.9);
        history.max_completion_tokens = Some(512);
        history.stop = Some(vec!["END".to_string()]);
        history.seed = Some(7);
        history.presence_penalty = Some(0.5);
        history.frequency_penalty = Some(0.25);
        let (model, server) =
            relay_endpoint(completion(&Response::from_text("ok", "claude"))).await;

        model.invoke(&history, None).await.unwrap();

        let (head, body) = server.await.unwrap();
        let route = RELAY_ROUTE_CHAT.replace("{app_id}", "app-1");
        assert!(
            head.starts_with(&format!("POST /api/v1/execution{route} ")),
            "{head}"
        );
        assert!(
            head.to_ascii_lowercase()
                .contains(&format!("authorization: bearer {TOKEN}\r\n")),
            "{head}"
        );
        assert_eq!(body["model"], BIT);
        assert_eq!(body["thinking"], "high");
        let (sent, stream) = history_from_relay_request(&body).unwrap();
        assert!(!stream);
        assert_eq!(sent.model, BIT);
        assert_eq!(
            sent.to_rig_messages().unwrap(),
            history.to_rig_messages().unwrap()
        );
        assert!(sent.messages.iter().any(|message| matches!(
            &message.content,
            MessageContent::Contents(parts) if parts.iter().any(|part| matches!(
                part,
                Content::Audio { audio_url, .. } if audio_url == WAV
            ))
        )));
        assert_eq!(
            sent.tools_to_rig().unwrap(),
            history.tools_to_rig().unwrap()
        );
        assert!(matches!(sent.tool_choice, Some(ToolChoice::Required)));
        assert_eq!(
            serde_json::to_value(&sent.response_format).unwrap(),
            serde_json::to_value(&history.response_format).unwrap()
        );
        assert_eq!(sent.temperature, Some(0.2));
        assert_eq!(sent.top_p, Some(0.9));
        assert_eq!(sent.max_completion_tokens, Some(512));
        assert_eq!(sent.stop, Some(vec!["END".to_string()]));
        assert_eq!(sent.seed, Some(7));
        assert_eq!(sent.presence_penalty, Some(0.5));
        assert_eq!(sent.frequency_penalty, Some(0.25));
        assert_eq!(
            sent.thinking,
            Some(HistoryThinking::Off),
            "an assistant tool turn without signed reasoning turns thinking off"
        );
    }

    #[tokio::test]
    async fn thinking_turns_off_on_the_second_turn_of_a_tool_loop() {
        let (model, server) =
            relay_endpoint(completion(&Response::from_text("ok", "claude"))).await;
        model.invoke(&first_turn(), None).await.unwrap();
        let (_, first) = server.await.unwrap();

        let (model, server) =
            relay_endpoint(completion(&Response::from_text("ok", "claude"))).await;
        model.invoke(&tool_turn(), None).await.unwrap();
        let (_, second) = server.await.unwrap();

        assert_eq!(first["thinking"], "high");
        assert_eq!(sent_history(&first).thinking, Some(HistoryThinking::High));
        assert_eq!(second["thinking"], "high");
        assert_eq!(sent_history(&second).thinking, Some(HistoryThinking::Off));
    }

    #[tokio::test]
    async fn a_forced_tool_travels_as_the_only_required_tool() {
        let mut history = first_turn();
        history.tool_choice = Some(ToolChoice::Specific {
            r#type: ToolType::Function,
            function: tool("get_forecast").function,
        });
        let (model, server) =
            relay_endpoint(completion(&Response::from_text("ok", "claude"))).await;

        model.invoke(&history, None).await.unwrap();

        let (_, body) = server.await.unwrap();
        assert_eq!(body["tool_choice"], "required");
        let sent = sent_history(&body);
        let names: Vec<_> = sent
            .tools
            .iter()
            .flatten()
            .map(|tool| tool.function.name.as_str())
            .collect();
        assert_eq!(names, ["get_forecast"]);
        assert!(matches!(sent.tool_choice, Some(ToolChoice::Required)));

        let mut openai_style = body;
        openai_style["tools"] = json!(
            history
                .tools_to_rig()
                .unwrap()
                .iter()
                .map(|tool| json!({
                    "type": "function",
                    "function": {"name": tool.name, "parameters": tool.parameters},
                }))
                .collect::<Vec<_>>()
        );
        openai_style["tool_choice"] =
            json!({"type": "function", "function": {"name": "get_weather"}});
        assert!(matches!(
            sent_history(&openai_style).tool_choice,
            Some(ToolChoice::Specific { function, .. }) if function.name == "get_weather"
        ));
        openai_style["tool_choice"] = json!({"type": "function", "function": {"name": "missing"}});
        assert!(history_from_relay_request(&openai_style).is_err());
    }

    #[test]
    fn tool_schemas_keep_what_history_can_express() {
        let request = json!({
            "model": BIT,
            "messages": [{"role": "user", "content": "Find it"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "search",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "limit": {"type": "integer", "minimum": 1},
                            "query": {"type": ["string", "null"], "description": "Text"},
                            "mode": {"enum": ["fast", 3]},
                            "tags": {"type": "array", "items": {"type": "string"}},
                            "filter": {"anyOf": [{"type": "string"}, {"type": "null"}]}
                        },
                        "required": ["query"],
                        "additionalProperties": false
                    }
                }
            }, {
                "type": "function",
                "function": {"name": "ping"}
            }]
        });

        let tools = sent_history(&request).tools_to_rig().unwrap();

        assert_eq!(
            tools[0].parameters,
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "number"},
                    "query": {"type": "string", "description": "Text"},
                    "mode": {"enum": ["fast"]},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "filter": {}
                },
                "required": ["query"]
            })
        );
        assert_eq!(tools[1].parameters, json!({"type": "object"}));
    }

    fn media(kind: &str, url: &str, media_type: Option<&str>) -> Content {
        let media_type = media_type.map(ToOwned::to_owned);
        let url = url.to_string();
        match kind {
            "image" => Content::Image {
                content_type: ContentType::ImageUrl,
                image_url: ImageUrl {
                    url,
                    detail: None,
                    media_type,
                    additional_params: None,
                },
            },
            "document" => Content::Document {
                content_type: ContentType::DocumentUrl,
                document_url: url,
                media_type,
                additional_params: None,
            },
            "audio" => Content::Audio {
                content_type: ContentType::AudioUrl,
                audio_url: url,
                media_type,
                additional_params: None,
            },
            _ => Content::Video {
                content_type: ContentType::VideoUrl,
                video_url: url,
                media_type,
                additional_params: None,
            },
        }
    }

    fn asking(parts: Vec<Content>) -> History {
        let mut contents = vec![Content::Text {
            content_type: ContentType::Text,
            text: "What is in these?".to_string(),
        }];
        contents.extend(parts);
        History::new(
            BIT.to_string(),
            vec![message(Role::User, MessageContent::Contents(contents))],
        )
    }

    fn user_parts(history: &History) -> Vec<Content> {
        history
            .messages
            .iter()
            .filter(|message| message.role == Role::User)
            .flat_map(|message| match &message.content {
                MessageContent::Contents(parts) => parts.clone(),
                MessageContent::String(_) => Vec::new(),
            })
            .collect()
    }

    /// Links as the run holds them and bytes as base64 data, of every media kind.
    fn every_kind_as_link_and_inline() -> Vec<Content> {
        const SIGNED: &str = "?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Signature=deadbeef";
        vec![
            media(
                "image",
                &format!("https://files.example.com/cat.png{SIGNED}"),
                Some("image/png"),
            ),
            media("image", IMAGE, None),
            media(
                "document",
                &format!("https://files.example.com/report.pdf{SIGNED}"),
                Some("application/pdf"),
            ),
            media("document", PDF, None),
            media(
                "document",
                "https://files.example.com/table.csv",
                Some("text/csv"),
            ),
            media(
                "document",
                "data:text/csv;flow-like-source=string;base64,YSxiCjEsMgo=",
                Some("text/csv"),
            ),
            media(
                "audio",
                "http://rustfs:9000/content/tmp/user/u/memo.m4a",
                Some("audio/mp4"),
            ),
            media("audio", WAV, None),
            media(
                "video",
                "https://files.example.com/clip.mp4",
                Some("video/mp4"),
            ),
            media("video", "data:video/mp4;base64,AAAAIGZ0eXA=", None),
        ]
    }

    #[tokio::test]
    async fn every_media_kind_travels_as_its_link_or_as_inline_data() {
        let history = asking(every_kind_as_link_and_inline());
        let (model, server) =
            relay_endpoint(completion(&Response::from_text("ok", "claude"))).await;

        model.invoke(&history, None).await.unwrap();

        let (_, body) = server.await.unwrap();
        let wire = body.to_string();
        assert!(!wire.contains("attachment omitted"), "{wire}");
        assert!(!wire.contains(MEDIA_CARRIER), "{wire}");
        let types: Vec<&str> = body["messages"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|part| part["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types,
            [
                "text",
                "image_url",
                "image_url",
                "document_url",
                "document_url",
                "document_url",
                "document_url",
                "audio_url",
                "audio_url",
                "video_url",
                "video_url",
            ]
        );
        let sent = sent_history(&body);
        let values = |history: &History| -> Vec<String> {
            user_parts(history)
                .iter()
                .filter_map(|part| part.media_url().map(ToOwned::to_owned))
                .collect()
        };
        assert_eq!(values(&sent), values(&history));
        assert_eq!(
            sent.to_rig_messages().unwrap(),
            history.to_rig_messages().unwrap()
        );
    }

    #[tokio::test]
    async fn agents_send_media_as_the_run_holds_it() {
        let (model, server) =
            relay_endpoint(completion(&Response::from_text("ok", "claude"))).await;
        let agent_model = model.dynamic_completion_model(None).await.unwrap();
        let clip = "https://files.example.com/clip.mp4?sig=secret";
        let prompt = RigMessage::User {
            content: OneOrMany::many(vec![
                UserContent::text("Describe the clip."),
                UserContent::Video(rig::message::Video {
                    data: DocumentSourceKind::Url(clip.to_string()),
                    media_type: Some(rig::message::VideoMediaType::MP4),
                    additional_params: None,
                }),
            ])
            .unwrap(),
        };
        let request =
            rig::completion::CompletionRequestBuilder::new(agent_model.clone(), prompt).build();

        rig::completion::CompletionModel::completion(&agent_model, request)
            .await
            .unwrap();

        let (_, body) = server.await.unwrap();
        assert_eq!(
            body["messages"][0]["content"][1],
            json!({"type": "video_url", "video_url": clip, "media_type": "video/mp4"})
        );
    }

    #[tokio::test]
    async fn a_call_over_the_relay_limit_fails_with_its_size_and_is_never_sent() {
        let large = format!(
            "data:application/pdf;base64,{}",
            "A".repeat(MAX_REQUEST_BYTES)
        );
        let (model, server) =
            relay_endpoint(completion(&Response::from_text("ok", "claude"))).await;

        let error = model
            .invoke(&asking(vec![media("document", &large, None)]), None)
            .await
            .expect_err("the hop is over the relay's limit")
            .to_string();

        assert!(error.contains(&MAX_REQUEST_BYTES.to_string()), "{error}");
        assert!(error.contains("inline"), "{error}");
        assert!(!server.is_finished(), "nothing reached the relay");
        server.abort();
    }

    #[tokio::test]
    async fn media_the_hop_cannot_carry_fails_the_call_before_it_is_sent() {
        for (part, reason) in [
            (
                media("document", "file_id:file-123", Some("application/pdf")),
                "provider file id",
            ),
            (
                media("image", "gs://bucket/cat.png", Some("image/png")),
                "gs link",
            ),
        ] {
            let (model, server) =
                relay_endpoint(completion(&Response::from_text("ok", "claude"))).await;

            let error = model
                .invoke(&asking(vec![part]), None)
                .await
                .expect_err("the hop carries links and base64 data only")
                .to_string();

            assert!(error.contains(reason), "{error}");
            assert!(!error.contains("bucket") && !error.contains("file-123"));
            assert!(!server.is_finished(), "nothing reached the relay");
            server.abort();
        }
    }

    #[test]
    fn the_relay_reads_links_and_base64_data_and_refuses_other_media() {
        let request = |part: Value| {
            json!({
                "model": BIT,
                "messages": [{"role": "user", "content": [{"type": "text", "text": "Look"}, part]}],
            })
        };

        for part in [
            json!({"type": "image_url", "image_url": {"url": "data:image/svg+xml,<svg/>"}}),
            json!({"type": "image_url", "image_url": {"url": "ftp://example.com/cat.png"}}),
            json!({"type": "document_url", "document_url": "file_id:file-123"}),
            json!({"type": "document_url", "document_url": "asset://localhost/%2Fhome%2Fa.pdf"}),
            json!({"type": "audio_url", "audio_url": ""}),
            json!({"type": "video_url", "video_url": "not a link"}),
            json!({"type": "file", "file": {"file_data": PDF}}),
            json!({"type": "input_audio", "input_audio": {"data": "SUQz", "format": "mp3"}}),
        ] {
            assert!(
                history_from_relay_request(&request(part.clone())).is_err(),
                "{part}"
            );
        }
        for part in [
            json!({"type": "image_url", "image_url": {"url": IMAGE}}),
            json!({"type": "image_url", "image_url": {"url": "https://example.com/cat.png", "media_type": "image/png"}}),
            json!({"type": "document_url", "document_url": PDF, "media_type": "application/pdf"}),
            json!({"type": "document_url", "document_url": "http://rustfs:9000/content/a.pdf?X-Amz-Signature=s"}),
            json!({"type": "audio_url", "audio_url": WAV}),
            json!({"type": "video_url", "video_url": "https://example.com/clip.mp4"}),
        ] {
            let (history, _) = history_from_relay_request(&request(part.clone())).unwrap();
            let MessageContent::Contents(parts) = &history.messages[0].content else {
                panic!("{part}: the media part is kept");
            };
            assert_eq!(serde_json::to_value(&parts[1]).unwrap(), part);
        }
    }

    #[test]
    fn frames_follow_the_hop_wire_format() {
        let mut chunk = ResponseChunk::from_reasoning("Weighing", "claude");
        chunk.id = "chunk-1".to_string();
        assert_eq!(
            relay_chunk(&chunk),
            json!({
                "id": "chunk-1",
                "model": "claude",
                "choices": [{"index": 0, "delta": {"role": "assistant", "reasoning_content": "Weighing"}}],
            })
        );

        assert_eq!(
            relay_chunk(&ResponseChunk::from_tool_call_delta(
                "call_a", "{\"ci", "claude"
            ))["choices"][0],
            json!({"index": 0, "delta": {"role": "assistant"}}),
            "argument deltas wait for the finished call"
        );

        let call = relay_chunk(&ResponseChunk::from_tool_call(
            &weather_call("call_a", "Paris"),
            "claude",
        ));
        assert_eq!(
            call["choices"][0],
            json!({
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_a",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": r#"{"city":"Paris"}"#},
                    }],
                },
                "finish_reason": "tool_calls",
            })
        );

        let usage = RigUsage {
            input_tokens: 11,
            output_tokens: 7,
            total_tokens: 30,
            ..RigUsage::new()
        };
        let finish = relay_chunk(&ResponseChunk::finish(
            "claude",
            Some(&usage),
            Some("length"),
        ));
        assert_eq!(
            finish["choices"][0],
            json!({"index": 0, "delta": {}, "finish_reason": "length"})
        );
        assert_eq!(
            finish["usage"],
            json!({"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18})
        );
    }

    fn streamed_frames() -> Vec<Value> {
        let usage = RigUsage {
            input_tokens: 11,
            output_tokens: 7,
            total_tokens: 18,
            ..RigUsage::new()
        };
        [
            ResponseChunk::from_reasoning("Comparing ", "claude"),
            ResponseChunk::from_reasoning("both cities", "claude"),
            ResponseChunk::from_text("Checking ", "claude"),
            ResponseChunk::from_text("both.", "claude"),
            ResponseChunk::from_tool_call_name_delta("call_a", "get_weather", "claude"),
            ResponseChunk::from_tool_call_delta("call_a", r#"{"city":"Paris"}"#, "claude"),
            ResponseChunk::from_tool_call(&weather_call("call_a", "Paris"), "claude"),
            ResponseChunk::from_tool_call(&weather_call("call_b", "Rome"), "claude"),
            ResponseChunk::finish("claude", Some(&usage), Some("tool_calls")),
        ]
        .iter()
        .map(relay_chunk)
        .collect()
    }

    #[tokio::test]
    async fn streamed_frames_reach_the_caller_through_rigs_parser() {
        let (model, server) = relay_endpoint(sse(&streamed_frames())).await;
        let (callback, chunks) = recording_callback();

        let response = model.invoke(&first_turn(), Some(callback)).await.unwrap();

        let (_, body) = server.await.unwrap();
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(response.content().as_deref(), Some("Checking both."));
        assert_eq!(
            response.choices[0].message.reasoning.as_deref(),
            Some("Comparing both cities")
        );
        assert_two_weather_calls(&response);
        assert_eq!(response.choices[0].finish_reason, "tool_calls");
        assert_eq!(
            (
                response.usage.prompt_tokens,
                response.usage.completion_tokens,
                response.usage.total_tokens
            ),
            (11, 7, 18)
        );
        let chunks = chunks.lock().unwrap();
        assert_eq!(
            chunks
                .last()
                .and_then(ResponseChunk::finish_reason)
                .as_deref(),
            Some("tool_calls")
        );
    }

    #[tokio::test]
    async fn calls_that_share_an_id_stay_separate() {
        let frames: Vec<Value> = [
            ResponseChunk::from_tool_call(&weather_call("get_weather", "Paris"), "gemini"),
            ResponseChunk::from_tool_call(&weather_call("get_weather", "Rome"), "gemini"),
            ResponseChunk::finish("gemini", None, Some("tool_calls")),
        ]
        .iter()
        .map(relay_chunk)
        .collect();

        let (model, server) = relay_endpoint(sse(&frames)).await;
        let (callback, _) = recording_callback();
        let response = model.invoke(&first_turn(), Some(callback)).await.unwrap();
        server.await.unwrap();
        let invoked: Vec<Value> = response.choices[0]
            .message
            .tool_calls
            .iter()
            .map(|call| {
                serde_json::from_str::<Value>(&call.function.arguments).unwrap()["city"].clone()
            })
            .collect();
        assert_eq!(invoked, [json!("Paris"), json!("Rome")]);

        let (model, server) = relay_endpoint(sse(&frames)).await;
        let agent_model = model.dynamic_completion_model(None).await.unwrap();
        let request = rig::completion::CompletionRequestBuilder::new(
            agent_model.clone(),
            RigMessage::user("Weather in Paris and Rome?"),
        )
        .build();

        let mut stream = rig::completion::CompletionModel::stream(&agent_model, request)
            .await
            .unwrap();
        let mut streamed = Vec::new();
        while let Some(item) = stream.next().await {
            if let rig::streaming::StreamedAssistantContent::ToolCall { tool_call, .. } =
                item.unwrap()
            {
                streamed.push(tool_call.function.arguments["city"].clone());
            }
        }
        server.await.unwrap();
        assert_eq!(streamed, [json!("Paris"), json!("Rome")]);
    }

    #[tokio::test]
    async fn a_completion_after_whitespace_keepalives_reaches_the_caller() {
        let mut response = Response::from_rig_message(RigMessage::Assistant {
            id: None,
            content: OneOrMany::many(vec![
                AssistantContent::reasoning("Comparing both cities"),
                AssistantContent::text("Checking both."),
                AssistantContent::ToolCall(weather_call("call_a", "Paris")),
                AssistantContent::ToolCall(weather_call("call_b", "Rome")),
            ])
            .unwrap(),
        })
        .unwrap();
        response.id = Some("relay-1".to_string());
        response.model = Some("claude".to_string());
        response.usage = Usage {
            prompt_tokens: 11,
            completion_tokens: 7,
            total_tokens: 18,
            ..Usage::default()
        };
        response.choices[0].finish_reason = "tool_calls".to_string();

        for callback in [None, Some(recording_callback().0)] {
            let mut history = first_turn();
            history.stream = Some(false);
            let (model, server) = relay_endpoint(completion(&response)).await;

            let relayed = model.invoke(&history, callback).await.unwrap();

            let (_, body) = server.await.unwrap();
            assert_eq!(body["stream"], false);
            assert_eq!(relayed.content().as_deref(), Some("Checking both."));
            assert_eq!(
                relayed.choices[0].message.reasoning.as_deref(),
                Some("Comparing both cities")
            );
            assert_two_weather_calls(&relayed);
            assert_eq!(relayed.choices[0].finish_reason, "tool_calls");
            assert_eq!(
                (relayed.usage.prompt_tokens, relayed.usage.completion_tokens),
                (11, 7)
            );
        }
    }

    #[tokio::test]
    async fn an_in_band_error_fails_the_call() {
        let error = relay_error("rate_limited", "Upstream rate limited");
        let frames = [
            relay_chunk(&ResponseChunk::from_text("Partial", "claude")),
            error.clone(),
        ];
        let (model, server) = relay_endpoint(sse(&frames)).await;
        let (callback, _) = recording_callback();

        let streamed = model.invoke(&first_turn(), Some(callback)).await;

        server.await.unwrap();
        let streamed = streamed.expect_err("an in-band error frame fails the stream");
        assert!(
            streamed.to_string().contains("Upstream rate limited"),
            "{streamed}"
        );

        let (model, server) =
            relay_endpoint(reply("application/json", &format!("   \n{error}"))).await;
        let mut history = first_turn();
        history.stream = Some(false);

        let completed = model.invoke(&history, None).await;

        server.await.unwrap();
        let completed = completed.expect_err("an in-band error body fails the completion");
        assert!(
            completed.to_string().contains("Upstream rate limited"),
            "{completed}"
        );
    }

    #[tokio::test]
    async fn a_stream_that_ends_before_done_fails_the_call() {
        let frames = streamed_frames();
        let (model, server) =
            relay_endpoint(reply("text/event-stream", &sse_frames(&frames[..4]))).await;
        let (callback, _) = recording_callback();

        let cut = model.invoke(&first_turn(), Some(callback)).await;

        server.await.unwrap();
        let cut = cut.expect_err("a stream without data: [DONE] was cut short");
        assert!(cut.to_string().contains("closing data: [DONE]"), "{cut}");

        let (model, server) = relay_endpoint(reply(
            "text/event-stream",
            &format!("{}data: [DONE]", sse_frames(&frames)),
        ))
        .await;
        let (callback, _) = recording_callback();

        let finished = model.invoke(&first_turn(), Some(callback)).await.unwrap();

        server.await.unwrap();
        assert_two_weather_calls(&finished);
    }

    #[test]
    fn a_relay_model_needs_its_base_token_and_bit() {
        assert!(RelayModel::new("", TOKEN, BIT).is_err());
        assert!(RelayModel::new("https://api.example.test/models", " ", BIT).is_err());
        assert!(RelayModel::new("https://api.example.test/models", TOKEN, "").is_err());
    }
}
