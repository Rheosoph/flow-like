use super::*;
use crate::history::{History, HistoryMessage, Role};
use crate::llm::test_support::{json_response, serve_once, sse_response};
use crate::llm::{DynamicResponse, DynamicStreamingResponse};
use futures::StreamExt;
use rig::completion::{
    Completion, CompletionModel, CompletionRequest, CompletionRequestBuilder, Message,
};
use rig::streaming::StreamedAssistantContent;
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};

#[derive(Clone, Copy, Debug)]
struct Case {
    name: &'static str,
    surface: Option<ModelApiSurface>,
    azure: bool,
}

fn cases() -> Vec<Case> {
    let mut cases: Vec<_> = [
        "custom:anthropic",
        "custom:bedrock",
        "custom:cohere",
        "custom:deepseek",
        "custom:galadriel",
        "custom:gemini",
        "custom:groq",
        "custom:huggingface",
        "custom:hyperbolic",
        "custom:lmstudio",
        "custom:mira",
        "custom:mistral",
        "custom:moonshot",
        "custom:mozilla",
        "custom:ollama",
        "custom:openrouter",
        "custom:perplexity",
        "custom:together",
        "custom:xai",
    ]
    .into_iter()
    .map(|name| Case {
        name,
        surface: None,
        azure: false,
    })
    .collect();
    for (surface, azure) in [
        (ModelApiSurface::Responses, false),
        (ModelApiSurface::ChatCompletions, false),
        (ModelApiSurface::ChatCompletions, true),
    ] {
        cases.push(Case {
            name: "custom:openai",
            surface: Some(surface),
            azure,
        });
    }
    cases
}

fn provider(case: Case, endpoint: &str, request_params: Value) -> ModelProvider {
    ModelProvider {
        provider_name: case.name.into(),
        model_id: Some("test-model".into()),
        version: None,
        api_surface: case.surface,
        params: Some(HashMap::from([
            ("model_id".into(), json!("test-model")),
            ("api_key".into(), json!("request-parameter-test-key")),
            ("endpoint".into(), json!(endpoint)),
            ("is_azure".into(), json!(case.azure)),
            ("request_params".into(), request_params),
        ])),
    }
}

async fn capture_agent_body(
    case: Case,
    request_params: Value,
    additional_params: Value,
    streaming: bool,
) -> Value {
    // The response is deliberately generic: this test inspects the serialized request,
    // regardless of which provider-specific response schema rejects the empty object.
    let (endpoint, mut server) = serve_once(json_response(json!({}))).await;
    let model = build(
        case.name,
        &provider(case, &endpoint, request_params),
        ProviderEgress::Open,
        AmbientCredentials::Forbidden,
    )
    .await
    .unwrap_or_else(|error| panic!("{case:?}: {error:#}"));
    let agent = model
        .provider()
        .await
        .unwrap()
        .into_client()
        .agent("test-model")
        .max_tokens(512)
        .temperature(0.25)
        .additional_params(additional_params)
        .build();
    let request = agent
        .completion("Hello", Vec::<Message>::new())
        .await
        .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(5), async {
        if streaming {
            match request.stream().await {
                Ok(mut stream) => format!("{:?}", stream.next().await.map(|chunk| chunk.err())),
                Err(error) => error.to_string(),
            }
        } else {
            request
                .send()
                .await
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default()
        }
    })
    .await;
    let sent = tokio::time::timeout(Duration::from_secs(5), &mut server).await;
    let body = match sent {
        Ok(sent) => sent.unwrap(),
        Err(_) => {
            server.abort();
            panic!("{case:?}, stream={streaming} sent no request: {outcome:?}");
        }
    };
    assert!(outcome.is_ok(), "{case:?}, stream={streaming}: {outcome:?}");
    serde_json::from_str(&body).unwrap()
}

#[tokio::test]
async fn custom_parameters_reach_every_http_adapter_for_both_transports() {
    for case in cases() {
        let (budget_path, overrides) = match case.name {
            "custom:gemini" => (
                "/generationConfig/maxOutputTokens",
                json!({"generationConfig": {"maxOutputTokens": 8192}}),
            ),
            "custom:ollama" => (
                "/options/num_predict",
                json!({"options": {"num_predict": 8192}}),
            ),
            "custom:xai" | "custom:mozilla" => {
                ("/max_output_tokens", json!({"max_output_tokens": 8192}))
            }
            "custom:openai" if case.surface == Some(ModelApiSurface::Responses) => {
                ("/max_output_tokens", json!({"max_output_tokens": 8192}))
            }
            _ => ("/max_tokens", json!({"max_tokens": 8192})),
        };
        for streaming in [false, true] {
            let mut overrides = overrides.clone();
            overrides["futureNativeOption"] = json!({"enabled": false, "items": [1, 2]});
            overrides["temperature"] = json!(0.75);
            overrides["optionalNativeValue"] = Value::Null;
            let body = capture_agent_body(case, overrides, json!({}), streaming).await;

            assert_eq!(
                body.pointer(budget_path),
                Some(&json!(8192)),
                "{case:?}: {body}"
            );
            assert_eq!(body["temperature"], 0.75, "{case:?}: {body}");
            assert_eq!(
                body["futureNativeOption"],
                json!({"enabled": false, "items": [1, 2]}),
                "{case:?}: {body}"
            );
            assert_eq!(body.get("optionalNativeValue"), Some(&Value::Null));
            for connection_field in [
                "request_params",
                "api_key",
                "endpoint",
                "is_azure",
                "headers",
            ] {
                assert!(body.get(connection_field).is_none(), "{case:?}: {body}");
            }
            assert!(!body.to_string().contains("request-parameter-test-key"));
            if let Some(stream) = body.get("stream") {
                assert_eq!(stream.as_bool(), Some(streaming), "{case:?}: {body}");
            }
        }
    }
}

#[tokio::test]
async fn gemini_native_overrides_preserve_generated_fields_and_unknown_nested_fields() {
    let case = Case {
        name: "custom:gemini",
        surface: None,
        azure: false,
    };
    for streaming in [false, true] {
        let body = capture_agent_body(
            case,
            json!({"generationConfig": {
                "maxOutputTokens": 8192,
                "temperature": 0.75,
                "thinkingConfig": {"futureThinkingOption": false},
                "futureGenerationOption": ["first", "second"],
            }}),
            json!({"generationConfig": {
                "topP": 0.8,
                "thinkingConfig": {"includeThoughts": true},
            }}),
            streaming,
        )
        .await;

        let config = &body["generationConfig"];
        assert_eq!(config["maxOutputTokens"], 8192);
        assert_eq!(config["temperature"], 0.75);
        assert_eq!(config["topP"], 0.8);
        assert_eq!(config["thinkingConfig"]["includeThoughts"], true);
        assert_eq!(config["thinkingConfig"]["futureThinkingOption"], false);
        assert_eq!(config["futureGenerationOption"], json!(["first", "second"]));
    }
}

#[tokio::test]
async fn gemini_history_invocation_also_applies_native_overrides() {
    let case = Case {
        name: "custom:gemini",
        surface: None,
        azure: false,
    };
    let (endpoint, server) = serve_once(json_response(json!({
        "responseId": "test-response",
        "candidates": [{
            "content": {"role": "model", "parts": [{"text": "Finished"}]},
            "finishReason": "STOP",
        }],
        "usageMetadata": {
            "promptTokenCount": 12, "candidatesTokenCount": 3000, "totalTokenCount": 3012,
        },
    })))
    .await;
    let model = build(
        case.name,
        &provider(
            case,
            &endpoint,
            json!({"generationConfig": {
                "maxOutputTokens": 8192, "temperature": 0.75, "futureOption": true,
            }}),
        ),
        ProviderEgress::Open,
        AmbientCredentials::Forbidden,
    )
    .await
    .unwrap();
    let mut history = History::new(
        "test-model".into(),
        vec![HistoryMessage::from_string(Role::User, "Hello")],
    );
    history.max_completion_tokens = Some(512);
    history.temperature = Some(0.25);
    let response = tokio::time::timeout(Duration::from_secs(5), model.invoke(&history, None))
        .await
        .expect("Gemini history invocation finishes")
        .unwrap();
    let body = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("Gemini history invocation reaches the endpoint")
        .unwrap();
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 8192);
    assert_eq!(body["generationConfig"]["temperature"], 0.75);
    assert_eq!(body["generationConfig"]["futureOption"], true);
    assert_eq!(response.choices[0].finish_reason, "stop");
}

#[tokio::test]
async fn invalid_native_parameters_are_rejected_when_building_custom_clients() {
    for case in cases() {
        for value in [
            json!([]),
            json!("{}"),
            Value::Null,
            json!({"stream": false}),
        ] {
            let result = build(
                case.name,
                &provider(case, "http://127.0.0.1:1", value),
                ProviderEgress::Open,
                AmbientCredentials::Forbidden,
            )
            .await;
            let error = result
                .err()
                .unwrap_or_else(|| panic!("{case:?} accepted invalid parameters"));
            assert!(
                error.to_string().contains("Request parameters"),
                "{case:?}: {error:#}"
            );
        }
    }
}

fn anthropic_response(model: &str, tokens: u64, streaming: bool) -> String {
    let message = json!({
        "id": "msg_test", "type": "message", "role": "assistant", "model": model,
        "content": [{"type": "text", "text": "Finished"}],
        "stop_reason": "end_turn", "usage": {"input_tokens": 12, "output_tokens": tokens},
    });
    if streaming {
        sse_response(&[
            json!({"type": "message_start", "message": message}),
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": "Finished"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
                "usage": {"output_tokens": tokens}}),
        ])
    } else {
        json_response(message)
    }
}

fn request_with_budget(budget: u64) -> CompletionRequest {
    CompletionRequest {
        model: None,
        preamble: None,
        chat_history: rig::OneOrMany::one(Message::user("Hello")),
        documents: vec![],
        tools: vec![],
        temperature: None,
        max_tokens: Some(budget),
        tool_choice: None,
        additional_params: None,
        output_schema: None,
    }
}

async fn completion_finish_reason<M>(
    request: CompletionRequestBuilder<M>,
    streaming: bool,
) -> String
where
    M: CompletionModel<Response = DynamicResponse, StreamingResponse = DynamicStreamingResponse>,
{
    if streaming {
        let mut stream = request.stream().await.unwrap();
        let mut finish_reason = None;
        while let Some(item) = stream.next().await {
            if let StreamedAssistantContent::Final(response) = item.unwrap() {
                finish_reason = response.finish_reason;
            }
        }
        finish_reason.expect("stream reports its finish reason")
    } else {
        request
            .send()
            .await
            .unwrap()
            .raw_response
            .finish_reason
            .unwrap()
    }
}

#[tokio::test]
async fn finish_reasons_use_the_native_limit_for_agents_dynamic_models_and_history() {
    let case = Case {
        name: "custom:anthropic",
        surface: None,
        azure: false,
    };
    for (configured, original, used, expected) in [
        (json!(8192), 2048, 3000, "stop"),
        (json!(1536), 8192, 1536, "length"),
        (Value::Null, 2048, 3000, "stop"),
        (json!(u64::from(u32::MAX) + 1), 2048, 3000, "stop"),
    ] {
        for path in ["agent", "dynamic", "history"] {
            for streaming in [false, true] {
                let (endpoint, server) =
                    serve_once(anthropic_response("test-model", used, streaming)).await;
                let model = build(
                    case.name,
                    &provider(case, &endpoint, json!({"max_tokens": configured})),
                    ProviderEgress::Open,
                    AmbientCredentials::Forbidden,
                )
                .await
                .unwrap();
                let reason = match path {
                    "agent" => {
                        let agent = model
                            .provider()
                            .await
                            .unwrap()
                            .into_client()
                            .agent("test-model")
                            .max_tokens(original)
                            .build();
                        completion_finish_reason(
                            agent
                                .completion("Hello", Vec::<Message>::new())
                                .await
                                .unwrap(),
                            streaming,
                        )
                        .await
                    }
                    "dynamic" => {
                        let dynamic = model.dynamic_completion_model(None).await.unwrap();
                        completion_finish_reason(
                            CompletionModel::completion_request(&dynamic, Message::user("Hello"))
                                .max_tokens(original),
                            streaming,
                        )
                        .await
                    }
                    _ => {
                        let mut history = History::new(
                            "test-model".into(),
                            vec![HistoryMessage::from_string(Role::User, "Hello")],
                        );
                        history.max_completion_tokens = Some(original as u32);
                        let callback = streaming.then(|| -> LLMCallback {
                            Arc::new(move |_| {
                                Box::pin(async move {
                                    // A nested model request in a callback uses its own budget.
                                    assert_eq!(
                                        crate::llm::requested_output_budget(&request_with_budget(
                                            original
                                        )),
                                        Some(original),
                                    );
                                    Ok(())
                                })
                            })
                        });
                        model.invoke(&history, callback).await.unwrap().choices[0]
                            .finish_reason
                            .clone()
                    }
                };
                let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
                assert_eq!(body["max_tokens"], configured, "{path}, stream={streaming}");
                assert_eq!(
                    reason, expected,
                    "{path}, stream={streaming}, limit={configured}"
                );
            }
        }
    }
}

#[tokio::test]
async fn native_anthropic_limit_is_applied_before_the_thinking_budget() {
    let case = Case {
        name: "custom:anthropic",
        surface: None,
        azure: false,
    };
    let model_id = "claude-sonnet-4-5";
    for path in ["agent", "history"] {
        let (endpoint, server) = serve_once(anthropic_response(model_id, 1, false)).await;
        let mut config = provider(case, &endpoint, json!({"max_tokens": 1536}));
        config.model_id = Some(model_id.into());
        config
            .params
            .as_mut()
            .unwrap()
            .insert("model_id".into(), json!(model_id));
        let model = build(
            case.name,
            &config,
            ProviderEgress::Open,
            AmbientCredentials::Forbidden,
        )
        .await
        .unwrap();
        let mut history = History::new(
            model_id.into(),
            vec![HistoryMessage::from_string(Role::User, "Hello")],
        );
        history.max_completion_tokens = Some(8192);
        history.thinking = Some(crate::history::HistoryThinking::High);
        if path == "agent" {
            let settings = model.agent_settings(&Some(history)).unwrap();
            let agent = model
                .provider()
                .await
                .unwrap()
                .into_client()
                .agent(model_id)
                .max_tokens(8192)
                .additional_params(settings.params.unwrap())
                .build();
            agent
                .completion("Hello", Vec::<Message>::new())
                .await
                .unwrap()
                .send()
                .await
                .unwrap();
        } else {
            model.invoke(&history, None).await.unwrap();
        }
        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["max_tokens"], 1536, "{path}");
        assert_eq!(body["thinking"]["budget_tokens"], 1535, "{path}");
    }
}

#[tokio::test]
async fn configured_budget_scopes_are_nested_and_isolated_between_concurrent_calls() {
    let request = request_with_budget(512);
    let budget = || crate::llm::requested_output_budget(&request);
    assert_eq!(budget(), Some(512));
    let barrier = tokio::sync::Barrier::new(3);
    tokio::join!(
        with_output_budget(Some(8192), async {
            barrier.wait().await;
            tokio::task::yield_now().await;
            assert_eq!(budget(), Some(8192));
            with_output_budget(None, async {
                tokio::task::yield_now().await;
                assert_eq!(budget(), None);
            })
            .await;
            assert_eq!(budget(), Some(8192));
        }),
        with_output_budget(Some(1536), async {
            barrier.wait().await;
            tokio::task::yield_now().await;
            assert_eq!(budget(), Some(1536));
        }),
        async {
            barrier.wait().await;
            tokio::task::yield_now().await;
            assert_eq!(budget(), Some(512));
        },
    );
    assert_eq!(budget(), Some(512));
}
