use std::{any::Any, sync::Arc};

use super::{
    ModelLogic, UsageReportingMode, extract_headers,
    media::{self, MediaDialect},
    merge_additional_params, output_budget_as_body_param,
    user_media::{self, leading_bytes},
};
use crate::authorization::AuthorizedHttpClient;
use crate::provider::random_provider;
use crate::{
    egress::ProviderEgress,
    history::History,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_types_contracts::Cacheable;
use flow_like_types_contracts::authorization::{RequestAuthorizer, ResourceAudience};
use rig::completion::{CompletionRequest, Message as RigMessage};
use rig::message::{DocumentSourceKind, UserContent, VideoMediaType};
use serde_json::json;

pub struct OpenRouterModel {
    client: rig::providers::openrouter::Client<AuthorizedHttpClient>,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl OpenRouterModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let openrouter_config = random_provider(&config.openrouter_config)?;
        let api_key = openrouter_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::openrouter::Client::builder()
            .api_key(&api_key)
            .http_client(AuthorizedHttpClient::default());

        if let Some(endpoint) = openrouter_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(OpenRouterModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(
        provider: &ModelProvider,
        egress: ProviderEgress,
    ) -> anyhow::Result<Self> {
        Self::with_http_client(
            provider,
            AuthorizedHttpClient::for_provider(provider, egress)?,
        )
    }

    pub async fn from_provider_with_authorizer(
        provider: &ModelProvider,
        authorizer: Option<Arc<dyn RequestAuthorizer>>,
    ) -> anyhow::Result<Self> {
        let http_client = match authorizer {
            Some(authorizer) => AuthorizedHttpClient::new(
                authorizer,
                ResourceAudience::HostedModels,
                provider
                    .params
                    .as_ref()
                    .and_then(|params| params.get("endpoint"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("Hosted authorization requires an explicit endpoint")
                    })?,
            )?,
            None => AuthorizedHttpClient::default(),
        };
        Self::with_http_client(provider, http_client)
    }

    fn with_http_client(
        provider: &ModelProvider,
        http_client: AuthorizedHttpClient,
    ) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        let endpoint = params.get("endpoint").and_then(|v| v.as_str());
        let custom_headers = extract_headers(&params);

        let mut builder = rig::providers::openrouter::Client::builder()
            .api_key(api_key)
            .http_client(http_client);
        if let Some(endpoint) = endpoint {
            builder = builder.base_url(endpoint);
        }
        if !custom_headers.is_empty() {
            builder = builder.http_headers(custom_headers);
        }

        let client = builder.build()?;

        Ok(OpenRouterModel {
            client,
            default_model: model_id.clone(),
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for OpenRouterModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for OpenRouterModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(user_media::constructor(
            self.client.clone(),
            |model, request| {
                media::retain_supported(
                    MediaDialect::OpenRouter,
                    model,
                    video_bytes_as_data_urls(output_budget_as_body_param(request, "max_tokens")),
                )
            },
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn usage_reporting(&self) -> UsageReportingMode {
        UsageReportingMode::OpenRouterUsageInclude
    }

    fn additional_params(&self, history: &Option<History>) -> Option<serde_json::Value> {
        let history = history.as_ref()?;
        let base = history.build_additional_params().ok().flatten();
        let reasoning = history.thinking.map(|thinking| {
            json!({
                "reasoning": {
                    "effort": thinking.openai_reasoning_effort(),
                }
            })
        });

        merge_additional_params(base, reasoning)
    }
}

/// OpenRouter forwards a video link only to upstreams that fetch it (Google AI Studio fetches
/// YouTube links only), so links stay as they are. Video held as bytes travels as a typed base64
/// `data:` URL, which every video upstream reads; Rig refuses raw bytes and untyped base64.
fn video_bytes_as_data_urls(mut request: CompletionRequest) -> CompletionRequest {
    for message in request.chat_history.iter_mut() {
        let RigMessage::User { content } = message else {
            continue;
        };
        for part in content.iter_mut() {
            let UserContent::Video(video) = part else {
                continue;
            };
            if let DocumentSourceKind::Raw(bytes) = &video.data {
                video.data = DocumentSourceKind::Base64(STANDARD.encode(bytes));
            }
            if video.media_type.is_none()
                && let DocumentSourceKind::Base64(data) = &video.data
            {
                video.media_type = sniffed_video(&leading_bytes(data, 64));
            }
        }
    }
    request
}

/// The container of a video, from its first bytes: the formats OpenRouter lists.
fn sniffed_video(bytes: &[u8]) -> Option<VideoMediaType> {
    let at = |range: std::ops::Range<usize>| bytes.get(range).unwrap_or_default();
    match bytes {
        _ if at(4..8) == b"ftyp" && at(8..12) == b"qt  " => Some(VideoMediaType::MOV),
        _ if at(4..8) == b"ftyp" => Some(VideoMediaType::MP4),
        [0x1A, 0x45, 0xDF, 0xA3, ..] if bytes.windows(4).any(|window| window == b"webm") => {
            Some(VideoMediaType::WEBM)
        }
        [0x00, 0x00, 0x01, 0xB3 | 0xBA, ..] => Some(VideoMediaType::MPEG),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Content, ContentType, HistoryMessage, MessageContent, Role};
    use crate::llm::LLMCallback;
    use crate::llm::test_support::{chat_completion, json_response, serve_once, sse_response};
    use crate::response_chunk::ResponseChunk;
    use rig::completion::Completion;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const MODEL: &str = "anthropic/claude-sonnet-4.5";
    const BUDGET: u32 = 100_000;

    fn stream_chunk(delta: Value, finish_reason: Value) -> Value {
        json!({
            "id": "gen-stream",
            "model": MODEL,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
        })
    }

    fn stream_usage(completion_tokens: u32) -> Value {
        json!({
            "id": "gen-stream",
            "model": MODEL,
            "choices": [],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": completion_tokens,
                "total_tokens": completion_tokens + 12,
            },
        })
    }

    async fn model(endpoint: &str) -> OpenRouterModel {
        OpenRouterModel::from_provider(
            &ModelProvider {
                provider_name: "openrouter".to_string(),
                model_id: None,
                version: None,
                api_surface: None,
                params: Some(HashMap::from([
                    ("api_key".to_string(), json!("test-key")),
                    ("model_id".to_string(), json!(MODEL)),
                    ("endpoint".to_string(), json!(endpoint)),
                ])),
            },
            ProviderEgress::Open,
        )
        .await
        .unwrap()
    }

    fn budgeted_history() -> History {
        let mut history = History::new(
            MODEL.to_string(),
            vec![HistoryMessage::from_string(
                Role::User,
                "Plan the next step, then call finalize.",
            )],
        );
        history.max_completion_tokens = Some(BUDGET);
        history
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

    fn wire_body(body: &str) -> Value {
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            body["max_tokens"], BUDGET,
            "History.max_completion_tokens must reach the OpenRouter request body"
        );
        body
    }

    #[tokio::test]
    async fn non_streaming_forwards_the_budget_and_reports_upstream_truncation() {
        let (endpoint, server) = serve_once(json_response(json!({
            "id": "gen-truncated",
            "object": "chat.completion",
            "created": 1,
            "model": MODEL,
            "choices": [{
                "index": 0,
                "finish_reason": "length",
                "native_finish_reason": "max_tokens",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "reasoning": "Still weighing which element to click",
                },
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 4096, "total_tokens": 4108},
        })))
        .await;
        let (callback, chunks) = recording_callback();
        let mut history = budgeted_history();
        history.stream = Some(false);

        let response = model(&endpoint)
            .await
            .invoke(&history, Some(callback))
            .await
            .unwrap();

        assert_eq!(wire_body(&server.await.unwrap())["stream"], false);
        assert_eq!(response.choices[0].finish_reason, "length");
        assert!(response.choices[0].message.tool_calls.is_empty());
        assert_eq!(
            chunks
                .lock()
                .unwrap()
                .last()
                .and_then(ResponseChunk::finish_reason)
                .as_deref(),
            Some("length")
        );
    }

    #[tokio::test]
    async fn streaming_forwards_the_budget_and_reports_exhaustion_as_length() {
        let (endpoint, server) = serve_once(sse_response(&[
            stream_chunk(
                json!({"role": "assistant", "reasoning": "Still weighing"}),
                Value::Null,
            ),
            stream_chunk(
                json!({"reasoning": " which element to click"}),
                json!("length"),
            ),
            stream_usage(BUDGET),
        ]))
        .await;
        let (callback, chunks) = recording_callback();

        let response = model(&endpoint)
            .await
            .invoke(&budgeted_history(), Some(callback))
            .await
            .unwrap();

        assert_eq!(wire_body(&server.await.unwrap())["stream"], true);
        assert_eq!(response.choices[0].finish_reason, "length");
        assert_eq!(
            response.choices[0].message.reasoning.as_deref(),
            Some("Still weighing which element to click")
        );
        assert_eq!(
            chunks
                .lock()
                .unwrap()
                .last()
                .and_then(ResponseChunk::finish_reason)
                .as_deref(),
            Some("length")
        );
    }

    #[tokio::test]
    async fn streaming_within_the_budget_reports_stop() {
        let (endpoint, server) = serve_once(sse_response(&[
            stream_chunk(
                json!({"role": "assistant", "content": "Done."}),
                json!("stop"),
            ),
            stream_usage(42),
        ]))
        .await;
        let (callback, _) = recording_callback();

        let response = model(&endpoint)
            .await
            .invoke(&budgeted_history(), Some(callback))
            .await
            .unwrap();

        wire_body(&server.await.unwrap());
        assert_eq!(response.choices[0].finish_reason, "stop");
        assert_eq!(response.content().as_deref(), Some("Done."));
    }

    #[tokio::test]
    async fn agent_requests_forward_the_budget_without_forcing_a_stream() {
        let (endpoint, server) = serve_once(json_response(chat_completion(MODEL, "stop", 1))).await;
        let model = model(&endpoint).await;
        let mut history = budgeted_history();
        history.temperature = Some(0.25);
        assert_eq!(history.stream, Some(true));
        let settings = model.agent_settings(&Some(history)).unwrap();
        let agent = model
            .provider()
            .await
            .unwrap()
            .into_client()
            .agent(MODEL)
            .additional_params(settings.params.unwrap())
            .temperature(settings.temperature.unwrap())
            .max_tokens(settings.max_tokens.unwrap())
            .build();

        let response = agent
            .completion(
                "Plan the next step.",
                Vec::<rig::completion::Message>::new(),
            )
            .await
            .unwrap()
            .send()
            .await
            .unwrap();

        let body = wire_body(&server.await.unwrap());
        assert!(
            body.get("stream").is_none(),
            "a non-streaming agent call must not ask for SSE: {body}"
        );
        assert_eq!(body["usage"]["include"], true);
        assert_eq!(body["temperature"], 0.25);
        assert_eq!(response.raw_response.finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test]
    #[allow(deprecated)]
    async fn prebuilt_requests_carry_the_history_thinking() {
        use crate::history::HistoryThinking;
        use crate::llm::lenient_settings::LenientSettings;
        use rig::completion::CompletionModel;

        let (endpoint, server) = serve_once(json_response(chat_completion(MODEL, "stop", 1))).await;
        let model = model(&endpoint).await;
        let mut history = History::new(MODEL.to_string(), vec![]);
        history.thinking = Some(HistoryThinking::Low);
        let params = model.agent_settings(&Some(history)).unwrap().params;
        let handle = model.completion_model_handle(None).await.unwrap();

        LenientSettings::new(handle, params)
            .completion_request("Read this page.")
            .send()
            .await
            .unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["reasoning"]["effort"], "low");
        assert_eq!(body["usage"]["include"], true);
    }

    #[tokio::test]
    async fn linked_audio_the_run_did_not_inline_fails_naming_its_host() {
        let (endpoint, server) = serve_once(json_response(chat_completion(MODEL, "stop", 1))).await;
        let mut history = budgeted_history();
        history.stream = Some(false);
        history.messages.push(HistoryMessage {
            role: Role::User,
            content: MessageContent::Contents(vec![Content::Audio {
                content_type: ContentType::AudioUrl,
                audio_url: "https://bucket.s3.amazonaws.com/voice.mp3?X-Amz-Signature=secret"
                    .to_string(),
                media_type: Some("audio/mpeg".to_string()),
                additional_params: None,
            }]),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            annotations: None,
        });

        let error = model(&endpoint)
            .await
            .invoke(&history, None)
            .await
            .unwrap_err()
            .to_string();

        assert!(
            error.contains(
                "The audio (audio/mp3) from host 'bucket.s3.amazonaws.com' cannot be sent to \
                 OpenRouter as a link"
            ),
            "{error}"
        );
        assert!(
            !error.contains("X-Amz") && !error.contains("voice.mp3"),
            "{error}"
        );
        assert!(!server.is_finished(), "nothing reached OpenRouter");
        server.abort();
    }

    fn video(url: &str, media_type: Option<&str>) -> Content {
        Content::Video {
            content_type: ContentType::VideoUrl,
            video_url: url.to_string(),
            media_type: media_type.map(str::to_string),
            additional_params: None,
        }
    }

    /// Sends `videos` in a user message and returns the request body.
    async fn sending_videos(videos: Vec<Content>) -> Value {
        let (endpoint, server) = serve_once(json_response(chat_completion(MODEL, "stop", 1))).await;
        let mut history = budgeted_history();
        history.stream = Some(false);
        history.messages.push(HistoryMessage {
            role: Role::User,
            content: MessageContent::Contents(videos),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            annotations: None,
        });
        model(&endpoint).await.invoke(&history, None).await.unwrap();
        wire_body(&server.await.unwrap())
    }

    #[tokio::test]
    async fn video_links_stay_links_and_video_bytes_travel_as_typed_data_urls() {
        const YOUTUBE: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        const CLIP: &str = "https://cdn.example.com/clips/launch.mp4";
        const RAW: &str = "flow-like-source=raw;base64";
        let mp4 = STANDARD.encode(b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2");
        let mov = STANDARD.encode(b"\0\0\0\x14ftypqt  \0\0\x02\0qt  ");
        let webm = STANDARD.encode(b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\x82\x84webm");
        let mpeg = STANDARD.encode(b"\0\0\x01\xba\x44\0\x04\0\x04\x01");
        let unknown = STANDARD.encode(b"not a video at all");

        let body = sending_videos(vec![
            video(YOUTUBE, None),
            video(CLIP, Some("video/mp4")),
            video(&format!("data:application/octet-stream;base64,{mp4}"), None),
            video(&format!("data:application/octet-stream;{RAW},{mov}"), None),
            video(&format!("data:;base64,{webm}"), None),
            video(&format!("data:video/mpeg;{RAW},{mpeg}"), None),
            video(
                &format!("data:application/octet-stream;base64,{unknown}"),
                None,
            ),
        ])
        .await;

        let videos: Vec<&str> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["content"].as_array().into_iter().flatten())
            .filter(|part| part["type"] == "video_url")
            .map(|part| part["video_url"]["url"].as_str().unwrap())
            .collect();
        assert_eq!(
            videos,
            [
                YOUTUBE.to_string(),
                CLIP.to_string(),
                format!("data:video/mp4;base64,{mp4}"),
                format!("data:video/mov;base64,{mov}"),
                format!("data:video/webm;base64,{webm}"),
                format!("data:video/mpeg;base64,{mpeg}"),
            ]
        );
        assert!(
            body.to_string()
                .contains("[attachment omitted: unsupported file type video]"),
            "{body}"
        );
    }
}
