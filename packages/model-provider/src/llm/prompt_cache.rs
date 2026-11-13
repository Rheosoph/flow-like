//! Anthropic prompt caching for a system prompt whose head stays the same from call to call.
//!
//! A bot turn's system prompt opens with sections that only change when the bot does and ends with
//! its per-turn context (time, memory). The caller that built it runs its model calls inside
//! [`with_stable_system_prefix`]. Clients that read explicit cache breakpoints then send the stable
//! head as its own system block marked `cache_control` and the rest as a second, unmarked block, so
//! the tools and the head are read from cache by the next call. Automatic caching also keeps the
//! growing message history reusable within a turn. Anthropic's Messages API reads these settings,
//! and OpenRouter does for Claude models. Other requests leave their client unchanged.

use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
    sse::BoxedStream,
};
use rig::wasm_compat::WasmCompatSend;
use serde_json::{Map, Value, json};

use crate::authorization::AuthorizedHttpClient;

tokio::task_local! {
    static STABLE_SYSTEM_PREFIX: Arc<str>;
    static CACHE_SESSION_KEY: Option<String>;
}

/// Runs `future` with `prefix` as the stable head of its system prompts. The head ends where a
/// section ends: the system text either is `prefix` or continues with a blank line after it.
pub async fn with_stable_system_prefix<F: Future>(prefix: String, future: F) -> F::Output {
    if prefix.trim().is_empty() {
        return future.await;
    }
    STABLE_SYSTEM_PREFIX.scope(Arc::from(prefix), future).await
}

/// Keeps OpenRouter's provider routing attached to a conversation even when its opening context
/// changes. Only a hash of the caller's conversation identity is sent to the provider.
pub async fn with_conversation_cache<F: Future>(
    prefix: String,
    conversation: Option<String>,
    future: F,
) -> F::Output {
    let session = conversation
        .map(|identity| format!("flow-like-{}", blake3::hash(identity.as_bytes()).to_hex()));
    CACHE_SESSION_KEY
        .scope(session, with_stable_system_prefix(prefix, future))
        .await
}

fn stable_system_prefix() -> Option<Arc<str>> {
    STABLE_SYSTEM_PREFIX.try_with(Arc::clone).ok()
}

/// Whether OpenRouter routes `model` to a Claude model.
pub fn is_openrouter_claude(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("anthropic/")
}

/// Where a provider's request body holds the system prompt, and whether it reads breakpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheDialect {
    /// Anthropic's Messages API: a top-level `system` array of text blocks.
    Messages,
    /// OpenRouter chat completions: system messages of text parts. Only Claude models read the
    /// breakpoint. A hosted Bit's request names the Bit, not its model, so `claude` carries what
    /// the Bit's catalog entry names.
    OpenRouter { claude: bool },
}

/// Marks the stable head in `body`. Returns whether the body changed.
fn mark(dialect: CacheDialect, body: &mut Value, prefix: &str) -> bool {
    // An explicit caller configuration owns its breakpoint budget and TTL ordering.
    if body.get("cache_control").is_some() || cache_markers(body) > 0 {
        return false;
    }
    let marked = match dialect {
        CacheDialect::Messages => body
            .get_mut("system")
            .is_some_and(|system| mark_blocks(system, prefix)),
        CacheDialect::OpenRouter { claude } => {
            let to_claude = claude
                || body
                    .get("model")
                    .and_then(Value::as_str)
                    .is_some_and(is_openrouter_claude);
            to_claude
                && body
                    .get_mut("messages")
                    .and_then(Value::as_array_mut)
                    .is_some_and(|messages| {
                        messages
                            .iter_mut()
                            .filter(|message| {
                                message.get("role").and_then(Value::as_str) == Some("system")
                            })
                            .filter_map(|message| message.get_mut("content"))
                            .any(|content| mark_blocks(content, prefix))
                    })
        }
    };
    if marked {
        // The fixed system breakpoint survives changed turn context; the automatic breakpoint
        // moves with the conversation so repeated tool rounds also reuse their history.
        // Tool-free calls cannot continue this loop; avoid paying to cache their varying tail.
        // A tool-enabled loop uses two of Anthropic's four breakpoint slots.
        if body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty())
        {
            body["cache_control"] = json!({"type": "ephemeral"});
        }
        if matches!(dialect, CacheDialect::OpenRouter { .. })
            && body.get("session_id").is_none()
            && body.get("prompt_cache_key").is_none()
            && let Ok(Some(session)) = CACHE_SESSION_KEY.try_with(Clone::clone)
        {
            body["session_id"] = Value::String(session);
        }
    }
    marked
}

fn cache_markers(value: &Value) -> usize {
    let marked = |block: &Value| {
        usize::from(
            block
                .get("cache_control")
                .is_some_and(|value| !value.is_null()),
        )
    };
    let blocks = |content: Option<&Value>| {
        content
            .and_then(Value::as_array)
            .map_or(0, |blocks| blocks.iter().map(marked).sum::<usize>())
    };
    marked(value)
        + blocks(value.get("tools"))
        + blocks(value.get("system"))
        + value
            .get("messages")
            .and_then(Value::as_array)
            .map_or(0, |messages| {
                messages
                    .iter()
                    .map(|message| marked(message) + blocks(message.get("content")))
                    .sum::<usize>()
            })
}

/// Splits the first unmarked text block of `content` that opens with `prefix`. Text content
/// becomes blocks.
fn mark_blocks(content: &mut Value, prefix: &str) -> bool {
    if let Some(text) = content.as_str() {
        let Some(blocks) = split_blocks(text, prefix) else {
            return false;
        };
        *content = Value::Array(blocks);
        return true;
    }
    let Some(blocks) = content.as_array_mut() else {
        return false;
    };
    let Some((index, split)) = blocks.iter().enumerate().find_map(|(index, block)| {
        let text = block
            .get("text")
            .and_then(Value::as_str)
            .filter(|_| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter(|_| block.get("cache_control").is_none_or(Value::is_null))?;
        Some((index, split_blocks(text, prefix)?))
    }) else {
        return false;
    };
    blocks.splice(index..=index, split);
    true
}

/// `prefix` as a block marked as a cache breakpoint, then the rest of `text` after the blank line
/// that ends `prefix`, unmarked.
fn split_blocks(text: &str, prefix: &str) -> Option<Vec<Value>> {
    let rest = text.strip_prefix(prefix)?;
    let tail = match rest {
        "" => None,
        rest => Some(rest.strip_prefix("\n\n")?),
    };
    let mut blocks = vec![json!({
        "type": "text",
        "text": prefix,
        "cache_control": {"type": "ephemeral"},
    })];
    blocks.extend(
        tail.filter(|tail| !tail.trim().is_empty())
            .map(|tail| json!({"type": "text", "text": tail})),
    );
    Some(blocks)
}

/// Rig's HTTP client for Anthropic and OpenRouter. It places the breakpoint of
/// [`with_stable_system_prefix`] and, on the Messages API, keeps a stream's cache counts in its
/// closing usage.
#[derive(Clone, Debug, Default)]
pub(crate) struct CacheBreakpoints {
    inner: AuthorizedHttpClient,
    dialect: Option<CacheDialect>,
}

impl CacheBreakpoints {
    pub(crate) fn new(inner: AuthorizedHttpClient, dialect: CacheDialect) -> Self {
        Self {
            inner,
            dialect: Some(dialect),
        }
    }

    fn marked<T: Into<Bytes>>(&self, request: Request<T>) -> Request<Bytes> {
        let request = request.map(|body| -> Bytes { body.into() });
        let (Some(dialect), Some(prefix)) = (self.dialect, stable_system_prefix()) else {
            return request;
        };
        if request.method() != http::Method::POST {
            return request;
        }
        let Ok(mut body) = serde_json::from_slice::<Value>(request.body()) else {
            return request;
        };
        if !mark(dialect, &mut body, &prefix) {
            return request;
        }
        let Ok(body) = serde_json::to_vec(&body) else {
            return request;
        };
        let (mut parts, _) = request.into_parts();
        parts.headers.remove(http::header::CONTENT_LENGTH);
        Request::from_parts(parts, Bytes::from(body))
    }
}

impl HttpClientExt for CacheBreakpoints {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.inner.send(self.marked(request))
    }

    fn send_multipart<U>(
        &self,
        request: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.inner.send_multipart(request)
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        let request = self.marked(request);
        let client = self.inner.clone();
        let messages_api = self.dialect == Some(CacheDialect::Messages);
        async move {
            let response = client.send_streaming(request).await?;
            Ok(if messages_api {
                response.map(backfill_cache_counts)
            } else {
                response
            })
        }
    }
}

const CACHE_COUNTS: [&str; 2] = ["cache_creation_input_tokens", "cache_read_input_tokens"];

/// Rig 0.38.2 reads a Messages stream's cache counts from its closing `message_delta` alone. The
/// API states them in `message_start` and may leave them out of the delta, so they are copied
/// over where the delta lacks them.
fn backfill_cache_counts(mut body: BoxedStream) -> BoxedStream {
    Box::pin(async_stream::stream! {
        let mut line = Vec::new();
        let mut counts = Map::new();
        while let Some(chunk) = body.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };
            let mut out = Vec::with_capacity(chunk.len());
            for piece in chunk.split_inclusive(|byte| *byte == b'\n') {
                line.extend_from_slice(piece);
                if line.ends_with(b"\n") {
                    match backfilled(&line, &mut counts) {
                        Some(rewritten) => out.extend_from_slice(&rewritten),
                        None => out.extend_from_slice(&line),
                    }
                    line.clear();
                }
            }
            if !out.is_empty() {
                yield Ok(Bytes::from(out));
            }
        }
        if !line.is_empty() {
            yield Ok(Bytes::from(line));
        }
    })
}

/// The `line` with the remembered cache counts added, when it is a `message_delta` without them.
/// A `message_start` line updates `counts`.
fn backfilled(line: &[u8], counts: &mut Map<String, Value>) -> Option<Vec<u8>> {
    let content = line.trim_ascii_end();
    let data = content.strip_prefix(b"data:")?.trim_ascii();
    if !names_a_message_event(data) {
        return None;
    }
    let mut event: Value = serde_json::from_slice(data).ok()?;
    let kind = event.get("type").and_then(Value::as_str)?.to_owned();
    match kind.as_str() {
        "message_start" => {
            remember_cache_counts(&event, counts);
            None
        }
        "message_delta" if add_cache_counts(&mut event, counts) => {
            let mut rewritten = format!("data: {event}").into_bytes();
            rewritten.extend_from_slice(&line[content.len()..]);
            Some(rewritten)
        }
        _ => None,
    }
}

fn names_a_message_event(data: &[u8]) -> bool {
    [b"message_start".as_slice(), b"message_delta".as_slice()]
        .iter()
        .any(|name| data.windows(name.len()).any(|window| window == *name))
}

fn remember_cache_counts(event: &Value, counts: &mut Map<String, Value>) {
    let Some(usage) = event.pointer("/message/usage") else {
        return;
    };
    for name in CACHE_COUNTS {
        if let Some(count) = usage.get(name).filter(|count| count.is_u64()) {
            counts.insert(name.to_string(), count.clone());
        }
    }
}

/// Adds the remembered counts the event's usage lacks. Returns whether it added any.
fn add_cache_counts(event: &mut Value, counts: &Map<String, Value>) -> bool {
    let Some(usage) = event.get_mut("usage").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut added = false;
    for (name, count) in counts {
        if usage.get(name).is_none_or(Value::is_null) {
            usage.insert(name.clone(), count.clone());
            added = true;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "You are Ada.\n\n## WHAT YOU CAN DO\nSearch the web.";
    const TAIL: &str = "## CONTEXT\nIt is 09:30.";

    fn prompt() -> String {
        format!("{PREFIX}\n\n{TAIL}")
    }

    fn split() -> Value {
        json!([
            {"type": "text", "text": PREFIX, "cache_control": {"type": "ephemeral"}},
            {"type": "text", "text": TAIL},
        ])
    }

    #[test]
    fn messages_system_splits_after_the_stable_head() {
        let mut body = json!({"system": [{"type": "text", "text": prompt()}], "messages": [], "tools": [{"name": "search"}]});
        assert!(mark(CacheDialect::Messages, &mut body, PREFIX));
        assert_eq!(body["system"], split());
        assert_eq!(body["messages"], json!([]));
        assert_eq!(body["cache_control"], json!({"type": "ephemeral"}));
    }

    #[test]
    fn growing_tool_history_uses_automatic_caching_without_rewriting_messages() {
        let mut history = vec![json!({"role": "user", "content": "Find item 7"})];
        for round in 0..3 {
            let mut body =
                json!({"system": prompt(), "messages": history, "tools": [{"name": "search"}]});
            assert!(mark(CacheDialect::Messages, &mut body, PREFIX));
            assert_eq!(body["system"], split());
            assert_eq!(body["cache_control"], json!({"type": "ephemeral"}));
            assert_eq!(body["messages"], json!(history));
            history.push(json!({"role": "assistant", "content": [{
                "type": "tool_use", "id": format!("call-{round}"), "name": "search",
                "input": {"item": 7},
            }]}));
            history.push(json!({"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": format!("call-{round}"),
                "content": format!("Result {round}"),
            }]}));
        }
    }

    #[test]
    fn tool_free_calls_cache_only_the_stable_system_prefix() {
        for tools in [None, Some(json!([])), Some(Value::Null)] {
            let mut body =
                json!({"system": prompt(), "messages": [{"role": "user", "content": "Conclude"}]});
            if let Some(tools) = tools {
                body["tools"] = tools;
            }
            assert!(mark(CacheDialect::Messages, &mut body, PREFIX));
            assert_eq!(body["system"], split());
            assert!(body.get("cache_control").is_none());
            assert_eq!(cache_markers(&body), 1);
        }
    }

    #[test]
    fn caller_cache_configuration_keeps_its_breakpoints_and_ttl() {
        for extra in [
            json!({"cache_control": {"type": "ephemeral", "ttl": "1h"}}),
            json!({"cache_control": null}),
            json!({"messages": [{"role": "user", "content": [
                {"type": "text", "text": "A", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "B", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "C", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "D", "cache_control": {"type": "ephemeral"}},
            ]}]}),
        ] {
            let mut body = json!({"system": prompt()});
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let before = body.clone();
            assert!(!mark(CacheDialect::Messages, &mut body, PREFIX));
            assert_eq!(body, before);
        }
    }

    #[test]
    fn a_tool_data_field_named_cache_control_does_not_disable_caching() {
        let mut body = json!({
            "system": prompt(),
            "tools": [{"name": "set_cache", "input_schema": {
                "type": "object", "properties": {"cache_control": {"type": "string"}}
            }}],
            "messages": [{"role": "assistant", "content": [{
                "type": "tool_use", "id": "call", "name": "set_cache",
                "input": {"cache_control": "public"}
            }]}]
        });
        assert!(mark(CacheDialect::Messages, &mut body, PREFIX));
        assert_eq!(cache_markers(&body), 2);
        assert_eq!(
            body["messages"][0]["content"][0]["input"]["cache_control"],
            "public"
        );
    }

    #[test]
    fn a_prompt_that_is_only_the_head_is_one_marked_block() {
        let mut body = json!({"system": [{"type": "text", "text": PREFIX}]});
        assert!(mark(CacheDialect::Messages, &mut body, PREFIX));
        assert_eq!(body["system"], json!([split()[0].clone()]));
    }

    #[test]
    fn a_head_that_ends_inside_a_section_or_differs_marks_nothing() {
        for system in [
            format!("{PREFIX} and more"),
            "Another prompt entirely.".to_string(),
        ] {
            let mut body = json!({"system": [{"type": "text", "text": system}]});
            let before = body.clone();
            assert!(!mark(CacheDialect::Messages, &mut body, PREFIX));
            assert_eq!(body, before);
        }
    }

    #[test]
    fn earlier_system_blocks_stay_in_front() {
        let mut body = json!({"system": [
            {"type": "text", "text": "Respond by calling the `submit` tool."},
            {"type": "text", "text": prompt()},
        ]});
        assert!(mark(CacheDialect::Messages, &mut body, PREFIX));
        let system = body["system"].as_array().unwrap();
        assert_eq!(system[0]["text"], "Respond by calling the `submit` tool.");
        assert!(system[0].get("cache_control").is_none());
        assert_eq!(Value::Array(system[1..].to_vec()), split());
    }

    #[test]
    fn openrouter_marks_claude_models_and_hosted_claude_bits_only() {
        let body = |model: &str| {
            json!({
                "model": model,
                "messages": [
                    {"role": "system", "content": [{"type": "text", "text": prompt()}]},
                    {"role": "user", "content": "Hi"},
                ],
            })
        };
        let mut claude = body("anthropic/claude-sonnet-4.5");
        assert!(mark(
            CacheDialect::OpenRouter { claude: false },
            &mut claude,
            PREFIX
        ));
        assert_eq!(claude["messages"][0]["content"], split());
        assert_eq!(claude["messages"][1]["content"], "Hi");

        let mut other = body("openai/gpt-4o-mini");
        assert!(!mark(
            CacheDialect::OpenRouter { claude: false },
            &mut other,
            PREFIX
        ));
        assert_eq!(other, body("openai/gpt-4o-mini"));

        let mut hosted = body("bit-id");
        assert!(mark(
            CacheDialect::OpenRouter { claude: true },
            &mut hosted,
            PREFIX
        ));
        assert_eq!(hosted["messages"][0]["content"], split());
    }

    #[test]
    fn openrouter_string_content_becomes_parts() {
        let mut body = json!({
            "model": "anthropic/claude-opus-4.5",
            "messages": [{"role": "system", "content": prompt()}],
        });
        assert!(mark(
            CacheDialect::OpenRouter { claude: false },
            &mut body,
            PREFIX
        ));
        assert_eq!(body["messages"][0]["content"], split());
    }

    #[tokio::test]
    async fn requests_outside_the_scope_are_sent_as_rig_wrote_them() {
        let client = CacheBreakpoints::new(AuthorizedHttpClient::default(), CacheDialect::Messages);
        let body =
            serde_json::to_vec(&json!({"system": [{"type": "text", "text": prompt()}]})).unwrap();
        let request = || {
            Request::post("https://api.example.test/v1/messages")
                .body(body.clone())
                .unwrap()
        };

        assert_eq!(client.marked(request()).body().as_ref(), body.as_slice());
        let marked =
            with_stable_system_prefix(PREFIX.to_string(), async { client.marked(request()) }).await;
        let sent: Value = serde_json::from_slice(marked.body()).unwrap();
        assert_eq!(sent["system"], split());
        let blank =
            with_stable_system_prefix(" ".to_string(), async { client.marked(request()) }).await;
        assert_eq!(blank.body().as_ref(), body.as_slice());
    }

    #[tokio::test]
    async fn nested_workers_use_their_own_prefix_and_restore_the_parent() {
        with_stable_system_prefix(PREFIX.to_string(), async {
            assert_eq!(stable_system_prefix().as_deref(), Some(PREFIX));
            with_stable_system_prefix("Worker instructions".to_string(), async {
                let client =
                    CacheBreakpoints::new(AuthorizedHttpClient::default(), CacheDialect::Messages);
                let request = Request::post("https://api.example.test/v1/messages")
                    .body(
                        serde_json::to_vec(&json!({
                            "system": "Worker instructions\n\nWorker context", "messages": [],
                            "tools": [{"name": "search"}],
                        }))
                        .unwrap(),
                    )
                    .unwrap();
                let sent: Value = serde_json::from_slice(client.marked(request).body()).unwrap();
                assert_eq!(sent["system"][0]["text"], "Worker instructions");
                assert_eq!(cache_markers(&sent), 2);
            })
            .await;
            assert_eq!(stable_system_prefix().as_deref(), Some(PREFIX));
        })
        .await;
        assert!(stable_system_prefix().is_none());
    }

    #[tokio::test]
    async fn openrouter_affinity_uses_conversation_identity_without_exposing_it() {
        async fn request(
            conversation: &str,
            context: &str,
            override_key: Option<(&str, &str)>,
        ) -> Value {
            with_conversation_cache(PREFIX.into(), Some(conversation.into()), async {
                let mut body = json!({
                    "model": "anthropic/claude-opus-5-5",
                    "messages": [{"role": "system", "content": format!("{PREFIX}\n\n{context}")}],
                });
                if let Some((key, value)) = override_key {
                    body[key] = json!(value);
                }
                assert!(mark(
                    CacheDialect::OpenRouter { claude: false },
                    &mut body,
                    PREFIX
                ));
                body
            })
            .await
        }
        let first = request("owner\nbot\nroom", "09:30", None).await;
        let next = request("owner\nbot\nroom", "10:00", None).await;
        assert_eq!(first["session_id"], next["session_id"]);
        assert_ne!(first["session_id"], "owner\nbot\nroom");
        assert_ne!(
            first["session_id"],
            request("another-room", "09:30", None).await["session_id"]
        );
        assert_eq!(
            request("owner\nbot\nroom", "09:30", Some(("session_id", "custom"))).await["session_id"],
            "custom"
        );
        let keyed = request(
            "owner\nbot\nroom",
            "09:30",
            Some(("prompt_cache_key", "custom")),
        )
        .await;
        assert_eq!(keyed["prompt_cache_key"], "custom");
        assert!(keyed.get("session_id").is_none());
    }

    /// The body an OpenAI agent call with the bot prompt put on the wire.
    async fn openai_body(stable: Option<&str>) -> String {
        use crate::egress::ProviderEgress;
        use crate::llm::ModelLogic;
        use crate::llm::openai::OpenAIModel;
        use crate::llm::test_support::{json_response, responses_completion, serve_once};
        use crate::provider::ModelProvider;
        use rig::completion::Completion;
        use std::collections::HashMap;

        let (endpoint, server) = serve_once(json_response(responses_completion("gpt-5"))).await;
        let params = [
            ("api_key", "test-key"),
            ("model_id", "gpt-5"),
            ("endpoint", endpoint.as_str()),
        ]
        .map(|(name, value)| (name.to_string(), json!(value)));
        let provider = ModelProvider {
            provider_name: "openai".to_string(),
            model_id: None,
            version: None,
            api_surface: None,
            params: Some(HashMap::from(params)),
        };
        let model = OpenAIModel::from_provider(&provider, ProviderEgress::Open)
            .await
            .unwrap();
        let agent = model
            .provider()
            .await
            .unwrap()
            .into_client()
            .agent("gpt-5")
            .preamble(&prompt())
            .build();
        let call = async {
            agent
                .completion("Hi", Vec::<rig::completion::Message>::new())
                .await
                .unwrap()
                .send()
                .await
                .unwrap();
        };
        match stable {
            Some(stable) => with_stable_system_prefix(stable.to_string(), call).await,
            None => call.await,
        }
        server.await.unwrap()
    }

    #[tokio::test]
    async fn openai_requests_are_byte_identical_with_or_without_a_stable_head() {
        let plain = openai_body(None).await;
        assert_eq!(plain, openai_body(Some(PREFIX)).await);
        assert!(plain.contains("## CONTEXT"), "{plain}");
        assert!(!plain.contains("cache_control"), "{plain}");
    }

    #[tokio::test]
    async fn stream_cache_counts_reach_the_closing_delta_across_chunk_boundaries() {
        let start = json!({"type": "message_start", "message": {"usage": {
            "input_tokens": 12, "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 4096, "output_tokens": 1,
        }}});
        let delta = json!({
            "type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 5},
        });
        let text = json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "message_delta is a word here"}});
        let wire = format!(
            "event: message_start\ndata: {start}\n\ndata: {text}\r\n\r\nevent: message_delta\ndata: {delta}\n\n"
        );
        let chunks: Vec<http_client::Result<Bytes>> = wire
            .as_bytes()
            .chunks(7)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect();

        let out: Vec<u8> = backfill_cache_counts(Box::pin(futures::stream::iter(chunks)))
            .map(|chunk| chunk.unwrap().to_vec())
            .concat()
            .await;
        let out = String::from_utf8(out).unwrap();

        let (before, closing) = out.rsplit_once("data: ").unwrap();
        assert!(wire.starts_with(before), "{out}");
        let closing: Value = serde_json::from_str(closing.trim()).unwrap();
        assert_eq!(closing["usage"]["cache_read_input_tokens"], 4096);
        assert_eq!(closing["usage"]["cache_creation_input_tokens"], 0);
        assert_eq!(closing["usage"]["output_tokens"], 5);
        assert!(out.ends_with("\n\n"));

        let complete = json!({
            "type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 5, "cache_read_input_tokens": 4096,
                "cache_creation_input_tokens": 0},
        });
        let untouched = format!("data: {start}\n\ndata: {complete}\n\n");
        let chunk: http_client::Result<Bytes> = Ok(Bytes::from(untouched.clone()));
        let out: Vec<u8> = backfill_cache_counts(Box::pin(futures::stream::iter([chunk])))
            .map(|chunk| chunk.unwrap().to_vec())
            .concat()
            .await;
        assert_eq!(String::from_utf8(out).unwrap(), untouched);
    }
}
