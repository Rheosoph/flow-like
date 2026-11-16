use bytes::Bytes;
use rig::{
    http_client::{
        self, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
    },
    wasm_compat::WasmCompatSend,
};
use serde_json::{Value, json};

/// ChatGPT accepts assistant history as output_text, including turns without a provider ID.
/// Rig 0.38.2 emits input_text for those turns, which the subscription endpoint rejects.
#[derive(Clone, Debug, Default)]
pub(super) struct CodexHttpClient(pub(super) reqwest_rig::Client);

impl HttpClientExt for CodexHttpClient {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let request = assistant_output_text(request);
        let client = self.0.clone();
        async move { client.send::<Bytes, U>(request?).await }
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
        let request = assistant_output_text(request);
        let client = self.0.clone();
        async move { client.send_streaming(request?).await }
    }
}

fn assistant_output_text<T: Into<Bytes>>(
    request: Request<T>,
) -> http_client::Result<Request<Bytes>> {
    let (mut parts, body) = request.into_parts();
    let body = body.into();
    if !parts.uri.path().ends_with("/responses") {
        return Ok(Request::from_parts(parts, body));
    }
    let error = || {
        http_client::Error::Instance(Box::new(std::io::Error::other(
            "Could not encode Codex assistant history",
        )))
    };
    let mut value: Value = serde_json::from_slice(&body).map_err(|_| error())?;
    let mut changed = false;
    for item in value
        .get_mut("input")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if item.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        for content in item
            .get_mut("content")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if content.get("type").and_then(Value::as_str) == Some("input_text") {
                content["type"] = json!("output_text");
                if let Some(content) = content.as_object_mut() {
                    content.entry("annotations").or_insert_with(|| json!([]));
                }
                changed = true;
            }
        }
    }
    if !changed {
        return Ok(Request::from_parts(parts, body));
    }
    parts.headers.remove(http::header::CONTENT_LENGTH);
    Ok(Request::from_parts(
        parts,
        Bytes::from(serde_json::to_vec(&value).map_err(|_| error())?),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{
        responses_tools::NonStrictToolsClient,
        test_support::{responses_completion, serve_once, sse_response},
    };
    use futures::StreamExt;
    use rig::{
        OneOrMany,
        client::CompletionClient,
        completion::{CompletionModel, CompletionRequest, Message},
        message::{AssistantContent, Reasoning},
    };

    fn replay() -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: Some("Follow the request.".into()),
            chat_history: OneOrMany::many(vec![
                Message::assistant("Previously reviewed the board."),
                Message::user("Read its details."),
                Message::Assistant {
                    id: None,
                    content: OneOrMany::many(vec![
                        AssistantContent::Reasoning(Reasoning::summaries(vec![
                            "A prior summary.".into(),
                        ])),
                        AssistantContent::text("I will read the app."),
                        AssistantContent::tool_call_with_call_id(
                            "fc_item",
                            "call_actual".into(),
                            "get_app_detail",
                            json!({"app_id":"fixture"}),
                        ),
                    ])
                    .unwrap(),
                },
                Message::tool_result_with_call_id(
                    "fc_item",
                    Some("call_actual".into()),
                    "fixture nonce",
                ),
            ])
            .unwrap(),
            documents: vec![],
            tools: vec![],
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        }
    }

    #[tokio::test]
    async fn codex_replays_board_history_and_distinct_tool_call_ids_on_both_transports() {
        for streaming in [false, true] {
            let response = responses_completion("gpt-test");
            let (endpoint, server) = serve_once(sse_response(&[
                json!({"type":"response.completed","response":response}),
            ]))
            .await;
            let provider = crate::provider::ModelProvider {
                provider_name: "custom:codex".into(),
                model_id: Some("gpt-test".into()),
                version: None,
                api_surface: None,
                params: Some(std::collections::HashMap::from([(
                    "access_token".into(),
                    json!("test-token"),
                )])),
            };
            let client = super::super::codex_client_builder(&provider)
                .unwrap()
                .base_url(&endpoint)
                .build()
                .unwrap();
            let model = NonStrictToolsClient(client).completion_model("gpt-test");
            if streaming {
                let mut response = model.stream(replay()).await.unwrap();
                while let Some(chunk) = response.next().await {
                    chunk.unwrap();
                }
            } else {
                model.completion(replay()).await.unwrap();
            }
            let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
            let input = body["input"].as_array().unwrap();
            let assistant: Vec<_> = input
                .iter()
                .filter(|item| item["role"] == "assistant")
                .collect();
            assert_eq!(assistant.len(), 2, "{body}");
            assert!(
                assistant
                    .iter()
                    .all(|item| item["content"][0]["type"] == "output_text"
                        && item.get("id").is_none()),
                "{body}"
            );
            assert_eq!(
                assistant[0]["content"][0]["text"],
                "Previously reviewed the board."
            );
            assert_eq!(
                input.iter().find(|item| item["role"] == "user").unwrap()["content"][0]["type"],
                "input_text"
            );
            let call = input
                .iter()
                .find(|item| item["type"] == "function_call")
                .unwrap();
            let result = input
                .iter()
                .find(|item| item["type"] == "function_call_output")
                .unwrap();
            assert_eq!(call["id"], "fc_item");
            assert_eq!(call["call_id"], "call_actual");
            assert_eq!(result["call_id"], "call_actual");
            assert_eq!(result["output"], "fixture nonce");
        }
    }

    #[test]
    fn wire_fix_preserves_other_roles_and_non_responses_requests() {
        let original = json!({"input":[{"role":"user","content":[{"type":"input_text","text":"User"}]},{"role":"assistant","content":[{"type":"output_text","text":"Assistant","annotations":[{"type":"citation"}]}]}]}).to_string();
        let request = Request::builder()
            .uri("https://example.test/responses")
            .body(original.clone())
            .unwrap();
        assert_eq!(
            assistant_output_text(request).unwrap().body().as_ref(),
            original.as_bytes()
        );
        let request = Request::builder()
            .uri("https://example.test/models")
            .body("unchanged")
            .unwrap();
        assert_eq!(
            assistant_output_text(request).unwrap().body().as_ref(),
            b"unchanged"
        );
    }
}
