//! Vertex AI responses as Rig completions, whole or streamed as server-sent events.

use bytes::Bytes;
use futures::{Stream, StreamExt};
use rig::OneOrMany;
use rig::completion::{CompletionError, CompletionResponse, GetTokenUsage, Usage};
use rig::message::{
    AssistantContent, ImageDetail, ImageMediaType, MimeType, Reasoning, ReasoningContent, ToolCall,
    ToolFunction,
};
use rig::streaming::{RawStreamingChoice, RawStreamingToolCall};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::media::{describe_http_error, scrub_links};
use super::wire::{Candidate, GenerateContentResponse, Part, PromptFeedback, UsageMetadata};

/// Finish reasons that mean the model broke the tool protocol, not that it answered.
const TOOL_PROTOCOL_FAILURES: [&str; 5] = [
    "MALFORMED_FUNCTION_CALL",
    "UNEXPECTED_TOOL_CALL",
    "MISSING_THOUGHT_SIGNATURE",
    "TOO_MANY_TOOL_CALLS",
    "MALFORMED_RESPONSE",
];

fn tool_protocol_failure(candidate: &Candidate) -> Option<CompletionError> {
    let reason = candidate.finish_reason.as_deref()?;
    TOOL_PROTOCOL_FAILURES.contains(&reason).then(|| {
        CompletionError::ResponseError(format!(
            "Vertex AI stopped with finish reason {reason}: {}",
            candidate
                .finish_message
                .as_deref()
                .map_or_else(|| "no finish message".to_string(), scrub_links)
        ))
    })
}

fn blocked(feedback: Option<&PromptFeedback>) -> CompletionError {
    match feedback.and_then(|feedback| feedback.block_reason.as_deref()) {
        Some(reason) => CompletionError::ResponseError(format!(
            "Vertex AI blocked the prompt ({reason}){}",
            feedback
                .and_then(|feedback| feedback.block_reason_message.as_deref())
                .map(|message| format!(": {}", scrub_links(message)))
                .unwrap_or_default()
        )),
        None => CompletionError::ResponseError("Vertex AI returned no candidates".to_string()),
    }
}

fn tool_arguments(args: &Value) -> Value {
    if args.is_null() {
        json!({})
    } else {
        args.clone()
    }
}

fn assistant_content(part: &Part) -> Option<AssistantContent> {
    if let Some(call) = &part.function_call {
        return Some(AssistantContent::ToolCall(
            ToolCall::new(
                call.name.clone(),
                ToolFunction::new(call.name.clone(), tool_arguments(&call.args)),
            )
            .with_signature(part.thought_signature.clone()),
        ));
    }
    if let Some(text) = &part.text {
        return Some(if part.thought {
            AssistantContent::Reasoning(Reasoning::new_with_signature(
                text,
                part.thought_signature.clone(),
            ))
        } else {
            AssistantContent::text(text)
        });
    }
    if let Some(blob) = &part.inline_data {
        let media_type = ImageMediaType::from_mime_type(&blob.mime_type.to_ascii_lowercase());
        if media_type.is_none() {
            tracing::warn!(
                mime = %blob.mime_type,
                "Vertex AI returned inline data that is not an image this client reads; leaving it out"
            );
        }
        return media_type.map(|media_type| {
            AssistantContent::image_base64(
                blob.data.clone(),
                Some(media_type),
                Some(ImageDetail::default()),
            )
        });
    }
    tracing::debug!("Vertex AI returned a part kind this client does not read");
    None
}

pub fn completion_response(
    response: GenerateContentResponse,
) -> Result<CompletionResponse<GenerateContentResponse>, CompletionError> {
    let candidate = response
        .candidates
        .first()
        .ok_or_else(|| blocked(response.prompt_feedback.as_ref()))?;
    if let Some(error) = tool_protocol_failure(candidate) {
        return Err(error);
    }
    let content: Vec<AssistantContent> = candidate
        .content
        .iter()
        .flat_map(|content| &content.parts)
        .filter_map(assistant_content)
        .collect();
    let choice = OneOrMany::many(content).map_err(|_| {
        CompletionError::ResponseError(format!(
            "Vertex AI returned no text or tool call (finish reason {}{})",
            candidate.finish_reason.as_deref().unwrap_or("unspecified"),
            candidate
                .finish_message
                .as_deref()
                .map(|message| format!(": {}", scrub_links(message)))
                .unwrap_or_default()
        ))
    })?;
    let usage = response
        .usage_metadata
        .map(UsageMetadata::to_rig)
        .unwrap_or_default();
    Ok(CompletionResponse {
        choice,
        usage,
        raw_response: response,
        message_id: None,
    })
}

/// The end of a stream. `finish_reason` is Vertex AI's (`STOP`, `MAX_TOKENS`, ...).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StreamingResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_metadata: Option<UsageMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_version: Option<String>,
}

impl GetTokenUsage for StreamingResponse {
    fn token_usage(&self) -> Option<Usage> {
        self.usage_metadata.map(UsageMetadata::to_rig)
    }
}

pub type StreamItem = RawStreamingChoice<StreamingResponse>;

/// Splits a server-sent event stream into the data of its events.
#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
    data: Vec<u8>,
    has_data: bool,
}

impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut start = 0;
        while let Some(offset) = self.buffer[start..].iter().position(|byte| *byte == b'\n') {
            let end = start + offset;
            let line = self.buffer[start..end].to_vec();
            self.line(line.strip_suffix(b"\r").unwrap_or(&line), &mut events);
            start = end + 1;
        }
        self.buffer.drain(..start);
        events
    }

    /// The events left when the stream ends, including one without its closing blank line.
    pub fn finish(&mut self) -> Vec<String> {
        let mut events = self.push(b"\n");
        self.line(b"", &mut events);
        events
    }

    fn line(&mut self, line: &[u8], events: &mut Vec<String>) {
        if line.is_empty() {
            if std::mem::take(&mut self.has_data) {
                events.push(String::from_utf8_lossy(&std::mem::take(&mut self.data)).into_owned());
            }
            return;
        }
        if line.starts_with(b":") {
            return;
        }
        let (field, value) = match line.iter().position(|byte| *byte == b':') {
            Some(colon) => (&line[..colon], &line[colon + 1..]),
            None => (line, &line[line.len()..]),
        };
        if field == b"data" {
            if self.has_data {
                self.data.push(b'\n');
            }
            self.data
                .extend_from_slice(value.strip_prefix(b" ").unwrap_or(value));
            self.has_data = true;
        }
    }
}

/// Turns each `streamGenerateContent` chunk into Rig stream items.
#[derive(Debug, Default)]
pub struct StreamState {
    end: StreamingResponse,
}

impl StreamState {
    pub fn accept(&mut self, data: &str) -> Result<Vec<StreamItem>, CompletionError> {
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return Ok(Vec::new());
        }
        let value: Value = serde_json::from_str(data)?;
        if let Some(error) = value.get("error") {
            return Err(CompletionError::ProviderError(format!(
                "Vertex AI stream failed: {}",
                provider_message(error)
            )));
        }
        let chunk: GenerateContentResponse = serde_json::from_value(value)?;
        if chunk.usage_metadata.is_some() {
            self.end.usage_metadata = chunk.usage_metadata;
        }
        if chunk.model_version.is_some() {
            self.end.model_version = chunk.model_version;
        }
        let Some(candidate) = chunk.candidates.into_iter().next() else {
            if chunk
                .prompt_feedback
                .as_ref()
                .is_some_and(|feedback| feedback.block_reason.is_some())
            {
                return Err(blocked(chunk.prompt_feedback.as_ref()));
            }
            return Ok(Vec::new());
        };
        if let Some(error) = tool_protocol_failure(&candidate) {
            return Err(error);
        }
        if candidate.finish_reason.is_some() {
            self.end.finish_reason = candidate.finish_reason;
            self.end.finish_message = candidate.finish_message;
        }
        let parts = candidate
            .content
            .map(|content| content.parts)
            .unwrap_or_default();
        Ok(parts.into_iter().filter_map(stream_item).collect())
    }

    pub fn finish(self) -> StreamItem {
        RawStreamingChoice::FinalResponse(self.end)
    }
}

fn stream_item(part: Part) -> Option<StreamItem> {
    if let Some(call) = part.function_call {
        return Some(RawStreamingChoice::ToolCall(
            RawStreamingToolCall::new(call.name.clone(), call.name, tool_arguments(&call.args))
                .with_signature(part.thought_signature),
        ));
    }
    match (part.text, part.thought) {
        (Some(text), true) if part.thought_signature.is_some() => {
            Some(RawStreamingChoice::Reasoning {
                id: None,
                content: ReasoningContent::Text {
                    text,
                    signature: part.thought_signature,
                },
            })
        }
        (Some(text), true) if !text.is_empty() => Some(RawStreamingChoice::ReasoningDelta {
            id: None,
            reasoning: text,
        }),
        (Some(text), false) if !text.is_empty() => Some(RawStreamingChoice::Message(text)),
        (Some(_), _) => None,
        (None, _) => {
            if part.inline_data.is_some() {
                tracing::warn!(
                    "Vertex AI streamed inline media, which Rig streams cannot carry; leaving it out"
                );
            }
            None
        }
    }
}

/// The message of a Google API error object, with its links scrubbed.
pub fn provider_message(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string());
    let status = error.get("status").and_then(Value::as_str);
    scrub_links(&match status {
        Some(status) => format!("{status}: {message}"),
        None => message,
    })
}

/// The Rig stream of a `streamGenerateContent?alt=sse` response body.
pub fn sse_items<S>(bytes: S) -> impl Stream<Item = Result<StreamItem, CompletionError>> + Send
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
{
    async_stream::stream! {
        let mut bytes = Box::pin(bytes);
        let mut decoder = SseDecoder::default();
        let mut state = StreamState::default();
        loop {
            let (events, ended) = match bytes.next().await {
                Some(Ok(chunk)) => (decoder.push(&chunk), false),
                Some(Err(error)) => {
                    yield Err(CompletionError::ProviderError(format!(
                        "The Vertex AI stream broke off: {}",
                        describe_http_error(error)
                    )));
                    return;
                }
                None => (decoder.finish(), true),
            };
            for event in events {
                match state.accept(&event) {
                    Ok(items) => {
                        for item in items {
                            yield Ok(item);
                        }
                    }
                    Err(error) => {
                        yield Err(error);
                        return;
                    }
                }
            }
            if ended {
                break;
            }
        }
        yield Ok(state.finish());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::streaming::RawStreamingChoice as Item;

    fn response(value: Value) -> GenerateContentResponse {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn whole_responses_become_rig_completions() {
        let completion = completion_response(response(json!({
            "candidates": [{
                "content": {"role": "model", "parts": [
                    {"text": "Plan the call.", "thought": true, "thoughtSignature": "sig-0"},
                    {"text": "Checking."},
                    {"functionCall": {"name": "get_weather", "args": {"city": "Berlin"}}, "thoughtSignature": "sig-1"},
                    {"inlineData": {"mimeType": "image/png", "data": "aW1n"}},
                    {"executableCode": {"language": "PYTHON", "code": "1"}},
                ]},
                "finishReason": "STOP",
                "safetyRatings": [{"category": "HARM_CATEGORY_HATE_SPEECH", "probability": "NEGLIGIBLE", "probabilityScore": 0.1}],
                "citationMetadata": {"citations": [{"uri": "https://example.com"}]},
            }],
            "usageMetadata": {
                "promptTokenCount": 10, "candidatesTokenCount": 5, "totalTokenCount": 21,
                "thoughtsTokenCount": 6, "cachedContentTokenCount": 2,
            },
            "modelVersion": "gemini-2.5-flash",
            "createTime": "2026-10-28T12:00:00Z",
            "responseId": "r-1",
        })))
        .unwrap();
        let choice: Vec<AssistantContent> = completion.choice.into_iter().collect();
        assert_eq!(choice.len(), 4);
        assert!(
            matches!(&choice[0], AssistantContent::Reasoning(reasoning) if reasoning.first_signature() == Some("sig-0"))
        );
        assert!(matches!(&choice[1], AssistantContent::Text(text) if text.text == "Checking."));
        let AssistantContent::ToolCall(call) = &choice[2] else {
            panic!("tool call expected");
        };
        assert_eq!(call.id, "get_weather");
        assert_eq!(call.function.arguments, json!({"city": "Berlin"}));
        assert_eq!(call.signature.as_deref(), Some("sig-1"));
        assert!(matches!(&choice[3], AssistantContent::Image(_)));
        assert_eq!(completion.usage.input_tokens, 10);
        assert_eq!(completion.usage.output_tokens, 5);
        assert_eq!(completion.usage.total_tokens, 21);
        assert_eq!(completion.usage.reasoning_tokens, 6);
        assert_eq!(completion.usage.cached_input_tokens, 2);
        let raw = serde_json::to_value(&completion.raw_response).unwrap();
        assert_eq!(
            raw.pointer("/candidates/0/finishReason"),
            Some(&json!("STOP"))
        );
    }

    #[test]
    fn blocked_and_broken_responses_fail_with_their_reason() {
        let blocked = completion_response(response(json!({
            "promptFeedback": {"blockReason": "PROHIBITED_CONTENT", "blockReasonMessage": "no"},
        })))
        .unwrap_err()
        .to_string();
        assert!(blocked.contains("PROHIBITED_CONTENT"), "{blocked}");
        let malformed = completion_response(response(json!({
            "candidates": [{"finishReason": "MALFORMED_FUNCTION_CALL", "finishMessage": "bad call"}],
        })))
        .unwrap_err()
        .to_string();
        assert!(
            malformed.contains("MALFORMED_FUNCTION_CALL: bad call"),
            "{malformed}"
        );
        let safety = completion_response(response(json!({
            "candidates": [{"finishReason": "SAFETY"}],
        })))
        .unwrap_err()
        .to_string();
        assert!(safety.contains("SAFETY"), "{safety}");
    }

    #[test]
    fn the_decoder_reassembles_events_split_anywhere() {
        let stream = b": keepalive\r\ndata: {\"a\":1}\r\n\r\ndata: {\"b\":\r\ndata: 2}\n\nevent: x\ndata:{\"c\":3}";
        for split in 0..stream.len() {
            let mut decoder = SseDecoder::default();
            let mut events = decoder.push(&stream[..split]);
            events.extend(decoder.push(&stream[split..]));
            events.extend(decoder.finish());
            assert_eq!(
                events,
                vec!["{\"a\":1}", "{\"b\":\n2}", "{\"c\":3}"],
                "split at {split}"
            );
        }
    }

    #[test]
    fn chunks_become_text_reasoning_tool_calls_and_a_final_response() {
        let mut state = StreamState::default();
        let mut items = Vec::new();
        for chunk in [
            json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "Hmm", "thought": true}]}}]}),
            json!({"candidates": [{"content": {"role": "model", "parts": [{"text": " done.", "thought": true, "thoughtSignature": "sig-0"}]}}]}),
            json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "Hello"}, {"text": ""}]}}]}),
            json!({"candidates": [{"content": {"role": "model", "parts": [
                {"functionCall": {"name": "lookup", "args": {"q": "x"}}, "thoughtSignature": "sig-1"},
            ]}, "finishReason": "STOP"}],
                "usageMetadata": {"promptTokenCount": 7, "candidatesTokenCount": 3, "totalTokenCount": 10},
                "modelVersion": "gemini-2.5-pro"}),
        ] {
            items.extend(state.accept(&chunk.to_string()).unwrap());
        }
        items.push(state.finish());
        assert_eq!(items.len(), 5);
        assert!(matches!(&items[0], Item::ReasoningDelta { reasoning, .. } if reasoning == "Hmm"));
        assert!(matches!(
            &items[1],
            Item::Reasoning { content: ReasoningContent::Text { text, signature }, .. }
                if text == " done." && signature.as_deref() == Some("sig-0")
        ));
        assert!(matches!(&items[2], Item::Message(text) if text == "Hello"));
        let Item::ToolCall(call) = &items[3] else {
            panic!("tool call expected");
        };
        assert_eq!((call.id.as_str(), call.name.as_str()), ("lookup", "lookup"));
        assert_eq!(call.arguments, json!({"q": "x"}));
        assert_eq!(call.signature.as_deref(), Some("sig-1"));
        let Item::FinalResponse(end) = &items[4] else {
            panic!("final response expected");
        };
        assert_eq!(end.finish_reason.as_deref(), Some("STOP"));
        assert_eq!(end.model_version.as_deref(), Some("gemini-2.5-pro"));
        let usage = end.token_usage().unwrap();
        assert_eq!(
            (usage.input_tokens, usage.output_tokens, usage.total_tokens),
            (7, 3, 10)
        );
        assert_eq!(
            serde_json::to_value(end).unwrap()["finish_reason"],
            json!("STOP")
        );
    }

    #[test]
    fn stream_errors_and_tool_protocol_failures_end_the_stream() {
        let mut state = StreamState::default();
        let error = state
            .accept(r#"{"error": {"code": 429, "status": "RESOURCE_EXHAUSTED", "message": "Quota exceeded, see https://console.cloud.google.com/iam?project=p"}}"#)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("RESOURCE_EXHAUSTED: Quota exceeded"),
            "{error}"
        );
        assert!(!error.contains("project=p"), "{error}");
        let malformed = state
            .accept(r#"{"candidates": [{"finishReason": "MALFORMED_FUNCTION_CALL"}]}"#)
            .unwrap_err()
            .to_string();
        assert!(malformed.contains("MALFORMED_FUNCTION_CALL"), "{malformed}");
        assert!(state.accept("not json").is_err());
        assert!(state.accept("[DONE]").unwrap().is_empty());
    }

    #[tokio::test]
    async fn sse_bodies_stream_items_and_end_with_usage() {
        let body = concat!(
            "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hel\"}]}}]}\r\n\r\n",
            "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"lo\"}]},\"finishReason\":\"MAX_TOKENS\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":2,\"totalTokenCount\":6}}\r\n\r\n",
        );
        let chunks: Vec<Result<Bytes, reqwest::Error>> = body
            .as_bytes()
            .chunks(7)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect();
        let items: Vec<StreamItem> = sse_items(futures::stream::iter(chunks))
            .map(Result::unwrap)
            .collect()
            .await;
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], Item::Message(text) if text == "Hel"));
        assert!(matches!(&items[1], Item::Message(text) if text == "lo"));
        let Item::FinalResponse(end) = &items[2] else {
            panic!("final response expected");
        };
        assert_eq!(end.finish_reason.as_deref(), Some("MAX_TOKENS"));
        assert_eq!(end.token_usage().unwrap().output_tokens, 2);
    }
}
