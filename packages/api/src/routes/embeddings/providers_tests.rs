use super::*;
use flow_like::flow_like_model_provider::provider::{ModelProvider, Pooling, Prefix};

fn provider() -> EmbeddingModelProvider {
    EmbeddingModelProvider {
        languages: vec!["en".into()],
        vector_length: 2,
        input_length: 8192,
        prefix: Prefix {
            query: "query: ".into(),
            paragraph: "passage: ".into(),
        },
        pooling: Pooling::Mean,
        provider: ModelProvider {
            provider_name: "Local".into(),
            model_id: Some("local-model".into()),
            version: None,
            api_surface: None,
            params: None,
        },
        remote: None,
    }
}

fn config(implementation: RemoteEmbeddingProvider) -> RemoteExecutionConfig {
    RemoteExecutionConfig {
        implementation: Some(implementation),
        model_id: Some("upstream-model".into()),
        ..Default::default()
    }
}

fn payload() -> EmbedRequest {
    EmbedRequest {
        model: "bit-id".into(),
        input: vec!["first".into(), "second".into()],
        embed_type: EmbedType::Query,
    }
}

#[test]
fn builds_cloudflare_and_openai_urls_without_using_configured_origins() {
    let cloudflare = endpoint_url(
        &RemoteEmbeddingProvider::CloudflareWorkersAI,
        Some("https://untrusted.example"),
        Some("0123456789abcdef0123456789abcdef"),
    )
    .unwrap();
    assert_eq!(
        cloudflare.as_str(),
        "https://api.cloudflare.com/client/v4/accounts/0123456789abcdef0123456789abcdef/ai/v1/embeddings"
    );
    assert_eq!(
        endpoint_url(
            &RemoteEmbeddingProvider::OpenAI,
            Some("https://untrusted.example"),
            None,
        )
        .unwrap()
        .as_str(),
        "https://api.openai.com/v1/embeddings"
    );
    for account in [None, Some("account/../../escape"), Some("short")] {
        assert!(
            endpoint_url(&RemoteEmbeddingProvider::CloudflareWorkersAI, None, account).is_err()
        );
    }
    for (provider, expected) in [
        (
            RemoteEmbeddingProvider::Cohere,
            "https://api.cohere.com/v2/embed",
        ),
        (
            RemoteEmbeddingProvider::VoyageAI,
            "https://api.voyageai.com/v1/embeddings",
        ),
    ] {
        assert_eq!(
            endpoint_url(&provider, Some("https://untrusted.example"), None)
                .unwrap()
                .as_str(),
            expected,
        );
    }
}

#[test]
fn cloudflare_bge_cannot_silently_mix_mean_vectors_with_a_cls_bit() {
    let mut provider = provider();
    let mut config = config(RemoteEmbeddingProvider::CloudflareWorkersAI);
    config.model_id = Some("@cf/baai/bge-large-en-v1.5".into());
    provider.pooling = Pooling::CLS;
    assert!(validate_request(&provider, &config, &payload()).is_err());
    provider.pooling = Pooling::Mean;
    validate_request(&provider, &config, &payload()).unwrap();
}

#[test]
fn normalizes_server_endpoint_roots_and_api_paths() {
    for variant in [
        RemoteEmbeddingProvider::Internal,
        RemoteEmbeddingProvider::HuggingfaceEndpoint,
        RemoteEmbeddingProvider::OpenAICompatible,
    ] {
        for endpoint in [
            "https://embed.example/proxy/",
            "https://embed.example/proxy/v1/",
            "https://embed.example/proxy/v1/embeddings/",
        ] {
            assert_eq!(
                endpoint_url(&variant, Some(endpoint), None)
                    .unwrap()
                    .as_str(),
                "https://embed.example/proxy/v1/embeddings"
            );
        }
    }
    for endpoint in [
        "https://azure.example",
        "https://azure.example/openai",
        "https://azure.example/openai/v1/",
        "https://azure.example/openai/v1/embeddings",
    ] {
        assert_eq!(
            endpoint_url(&RemoteEmbeddingProvider::AzureOpenAI, Some(endpoint), None)
                .unwrap()
                .as_str(),
            "https://azure.example/openai/v1/embeddings"
        );
    }
}

#[test]
fn rejects_urls_containing_credentials_queries_fragments_or_unsupported_schemes() {
    for endpoint in [
        "https://user:password@embed.example",
        "https://embed.example?api_key=secret",
        "https://embed.example#fragment",
        "file:///tmp/embeddings",
        "garbage",
    ] {
        let error = endpoint_url(
            &RemoteEmbeddingProvider::OpenAICompatible,
            Some(endpoint),
            None,
        )
        .unwrap_err();
        assert!(!error.to_string().contains(endpoint));
    }
    assert!(
        endpoint_url(
            &RemoteEmbeddingProvider::AzureOpenAI,
            Some("http://azure.example"),
            None,
        )
        .is_err()
    );
}

#[test]
fn validates_before_admission_and_applies_exact_query_or_document_prefix() {
    let provider = provider();
    let config = config(RemoteEmbeddingProvider::CloudflareWorkersAI);
    let mut input = payload();
    validate_request(&provider, &config, &input).unwrap();
    assert_eq!(
        request_body(&provider, &config, &input),
        serde_json::json!({
            "model": "upstream-model", "input": ["query: first", "query: second"],
            "encoding_format": "float"
        })
    );
    input.embed_type = EmbedType::Document;
    assert_eq!(
        request_body(&provider, &config, &input)["input"],
        serde_json::json!(["passage: first", "passage: second"])
    );
    for invalid in [
        vec![],
        vec![String::new()],
        vec!["x".repeat(MAX_INPUT_BYTES)],
    ] {
        input.input = invalid;
        assert!(validate_request(&provider, &config, &input).is_err());
    }
    input.input = vec!["x".into(); MAX_BATCH_SIZE + 1];
    assert!(validate_request(&provider, &config, &input).is_err());
    let mut invalid_provider = provider;
    invalid_provider.vector_length = 0;
    assert!(validate_request(&invalid_provider, &config, &payload()).is_err());
}

#[test]
fn deprecated_bit_url_is_ignored_and_fixed_provider_overrides_fail_closed() {
    let mut config = config(RemoteEmbeddingProvider::CloudflareWorkersAI);
    config.endpoint = Some("https://untrusted.example".into());
    validate_request(&provider(), &config, &payload()).unwrap();
    config.endpoint_secret_name = Some("OTHER_ENDPOINT".into());
    assert!(validate_request(&provider(), &config, &payload()).is_err());
    config.implementation = Some(RemoteEmbeddingProvider::OpenAI);
    assert!(validate_request(&provider(), &config, &payload()).is_err());
    config.implementation = Some(RemoteEmbeddingProvider::Cohere);
    assert!(validate_request(&provider(), &config, &payload()).is_err());
    config.implementation = Some(RemoteEmbeddingProvider::VoyageAI);
    assert!(validate_request(&provider(), &config, &payload()).is_err());
    config.implementation = Some(RemoteEmbeddingProvider::OpenAICompatible);
    validate_request(&provider(), &config, &payload()).unwrap();
    config.model_id = Some("  ".into());
    assert!(validate_request(&provider(), &config, &payload()).is_err());
}

#[test]
fn native_providers_preserve_prefixes_set_retrieval_type_and_disable_truncation() {
    for (implementation, field, query_type, document_type) in [
        (
            RemoteEmbeddingProvider::Cohere,
            "texts",
            "search_query",
            "search_document",
        ),
        (
            RemoteEmbeddingProvider::VoyageAI,
            "input",
            "query",
            "document",
        ),
    ] {
        let config = config(implementation);
        let mut payload = payload();
        let body = request_body(&provider(), &config, &payload);
        assert_eq!(
            body[field],
            serde_json::json!(["query: first", "query: second"])
        );
        assert_eq!(body["input_type"], query_type);
        assert!(body.get("encoding_format").is_none());
        if implementation == RemoteEmbeddingProvider::Cohere {
            assert_eq!(body["embedding_types"], serde_json::json!(["float"]));
            assert_eq!(body["truncate"], "NONE");
        } else {
            assert_eq!(body["output_dtype"], "float");
            assert_eq!(body["truncation"], false);
        }
        payload.embed_type = EmbedType::Document;
        let body = request_body(&provider(), &config, &payload);
        assert_eq!(body["input_type"], document_type);
        assert_eq!(
            body[field],
            serde_json::json!(["passage: first", "passage: second"])
        );
    }
}

#[test]
fn native_providers_enforce_documented_batch_sizes_before_admission() {
    for (implementation, max_batch) in [
        (RemoteEmbeddingProvider::Cohere, 96),
        (RemoteEmbeddingProvider::VoyageAI, 1000),
    ] {
        let config = config(implementation);
        let mut payload = payload();
        payload.input = vec!["text".into(); max_batch];
        validate_request(&provider(), &config, &payload).unwrap();
        payload.input.push("one too many".into());
        assert!(validate_request(&provider(), &config, &payload).is_err());
    }
}

#[test]
fn cloudflare_models_enforce_documented_batch_limits_before_admission() {
    // Limits from each model's published input schema, including BGE's
    // synchronous schema rather than its separate asynchronous batch API.
    for (model, max_batch) in [
        ("@cf/qwen/qwen3-embedding-0.6b", 32),
        ("@cf/google/embeddinggemma-300m", 100),
        ("@cf/baai/bge-large-en-v1.5", 100),
    ] {
        let mut config = config(RemoteEmbeddingProvider::CloudflareWorkersAI);
        config.model_id = Some(format!(" {model} "));
        let mut payload = payload();
        payload.input = vec!["text".into(); max_batch];
        validate_request(&provider(), &config, &payload).unwrap();
        payload.input.push("one too many".into());
        assert!(validate_request(&provider(), &config, &payload).is_err());
        config.implementation = Some(RemoteEmbeddingProvider::Internal);
        validate_request(&provider(), &config, &payload).unwrap();
    }
}

#[test]
fn requests_configured_dimensions_only_from_supported_native_models() {
    for (implementation, model, supports_dimensions) in [
        (RemoteEmbeddingProvider::Cohere, "embed-v4.0", true),
        (RemoteEmbeddingProvider::Cohere, "embed-v5.0-fast", true),
        (RemoteEmbeddingProvider::Cohere, "embed-english-v3.0", false),
        (RemoteEmbeddingProvider::VoyageAI, "voyage-4-large", true),
        (RemoteEmbeddingProvider::VoyageAI, "voyage-code-4", true),
        (RemoteEmbeddingProvider::VoyageAI, "voyage-3.5-lite", true),
        (RemoteEmbeddingProvider::VoyageAI, "voyage-law-2", false),
    ] {
        let mut config = config(implementation);
        config.model_id = Some(model.into());
        let mut provider = provider();
        provider.vector_length = 512;
        let body = request_body(&provider, &config, &payload());
        if supports_dimensions {
            assert_eq!(body["output_dimension"], 512);
        } else {
            assert!(body.get("output_dimension").is_none());
        }
    }
}

#[test]
fn validates_known_dimensions_and_requests_openai_reductions() {
    for (implementation, model, accepted, rejected) in [
        (RemoteEmbeddingProvider::Cohere, "embed-v4.0", 1024, 768),
        (RemoteEmbeddingProvider::Cohere, "embed-v5.0-pro", 768, 128),
        (
            RemoteEmbeddingProvider::Cohere,
            "embed-english-v3.0",
            1024,
            512,
        ),
        (
            RemoteEmbeddingProvider::Cohere,
            "embed-multilingual-light-v3.0",
            384,
            1024,
        ),
        (
            RemoteEmbeddingProvider::VoyageAI,
            "voyage-4-large",
            512,
            1536,
        ),
        (
            RemoteEmbeddingProvider::OpenAI,
            "text-embedding-3-small",
            512,
            1537,
        ),
        (
            RemoteEmbeddingProvider::OpenAI,
            "text-embedding-3-large",
            2048,
            3073,
        ),
        (
            RemoteEmbeddingProvider::OpenAI,
            "text-embedding-ada-002",
            1536,
            512,
        ),
    ] {
        let mut config = config(implementation);
        config.model_id = Some(model.into());
        let mut provider = provider();
        provider.vector_length = accepted;
        validate_request(&provider, &config, &payload()).unwrap();
        let body = request_body(&provider, &config, &payload());
        if implementation == RemoteEmbeddingProvider::OpenAI
            && model.starts_with("text-embedding-3")
        {
            assert_eq!(body["dimensions"], accepted);
        } else {
            assert!(body.get("dimensions").is_none());
        }
        provider.vector_length = rejected;
        assert!(validate_request(&provider, &config, &payload()).is_err());
    }
    let mut azure = config(RemoteEmbeddingProvider::AzureOpenAI);
    azure.model_id = Some("custom-deployment".into());
    assert!(
        request_body(&provider(), &azure, &payload())
            .get("dimensions")
            .is_none()
    );
}

#[test]
fn invalid_credentials_are_rejected_and_authorization_headers_are_sensitive() {
    for implementation in [
        RemoteEmbeddingProvider::OpenAI,
        RemoteEmbeddingProvider::AzureOpenAI,
    ] {
        let (_, header) = credential_header(&implementation, "key").unwrap();
        assert!(header.is_sensitive());
        assert!(credential_header(&implementation, "key\nwith-header-injection").is_err());
    }
}

#[test]
fn cohere_normalizes_ordered_vectors_and_billed_units() {
    let response = serde_json::json!({
        "id": "cohere-request",
        "embeddings": {"float": [[0.1, 0.2], [0.3, 0.4]]},
        "meta": {"billed_units": {"input_tokens": 7.0}, "tokens": {"input_tokens": 9}},
        "texts": ["private text", "more private text"],
    });
    let result = decode_cohere_response(&serde_json::to_vec(&response).unwrap(), 2, 2).unwrap();
    assert_eq!(result.embeddings, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);
    assert_eq!(
        result.provider_request_id.as_deref(),
        Some("cohere-request")
    );
    let usage = result.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 7);
    assert_eq!(usage.total_tokens, 7);
    assert_eq!(
        result.raw_usage.unwrap(),
        serde_json::json!({"billed_units":{"input_tokens":7.0}})
    );
    assert!(decode_cohere_response(&serde_json::to_vec(&response).unwrap(), 1, 2).is_err());
    assert!(decode_cohere_response(&serde_json::to_vec(&response).unwrap(), 2, 3).is_err());
}

#[test]
fn cohere_does_not_substitute_unbilled_tokens_and_rejects_invalid_billed_counts() {
    let mut response = serde_json::json!({
        "embeddings": {"float": [[0.1, 0.2]]},
        "meta": {"tokens": {"input_tokens": 9}},
    });
    assert!(
        decode_cohere_response(&serde_json::to_vec(&response).unwrap(), 1, 2)
            .unwrap()
            .usage
            .is_none()
    );
    for value in [
        serde_json::json!(-1),
        serde_json::json!(0.5),
        serde_json::json!("7"),
        serde_json::json!(1e20),
    ] {
        response["meta"]["billed_units"] = serde_json::json!({"input_tokens": value});
        assert!(decode_cohere_response(&serde_json::to_vec(&response).unwrap(), 1, 2).is_err());
    }
    assert!(decode_cohere_response(br#"{"embeddings":{"float":[[1e39, 0.2]]}}"#, 1, 2).is_err());
}

#[test]
fn sorts_indexed_vectors_and_preserves_reported_usage() {
    let response = serde_json::json!({
        "data": [
            {"index": 1, "embedding": [0.3, 0.4]},
            {"index": 0, "embedding": [0.1, 0.2]}
        ],
        "model": "served-model",
        "request_id": "upstream-request",
        "usage": {"prompt_tokens": 4, "total_tokens": 4}
    });
    let result = decode_response(&serde_json::to_vec(&response).unwrap(), 2, 2).unwrap();
    assert_eq!(result.embeddings, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);
    assert_eq!(result.model.as_deref(), Some("served-model"));
    assert_eq!(
        result.provider_request_id.as_deref(),
        Some("upstream-request")
    );
    assert_eq!(result.usage.unwrap().total_tokens, 4);
    assert_eq!(result.raw_usage.unwrap()["prompt_tokens"], 4);
}

#[test]
fn rejects_incomplete_duplicate_out_of_range_and_wrong_dimension_vectors() {
    for data in [
        serde_json::json!([{ "index": 0, "embedding": [0.1, 0.2] }]),
        serde_json::json!([
            { "index": 0, "embedding": [0.1, 0.2] },
            { "index": 0, "embedding": [0.1, 0.2] }
        ]),
        serde_json::json!([
            { "index": 0, "embedding": [0.1, 0.2] },
            { "index": 2, "embedding": [0.1, 0.2] }
        ]),
        serde_json::json!([
            { "index": 0, "embedding": [0.1] },
            { "index": 1, "embedding": [0.1, 0.2] }
        ]),
    ] {
        assert!(
            decode_response(
                &serde_json::to_vec(&serde_json::json!({"data": data})).unwrap(),
                2,
                2,
            )
            .is_err()
        );
    }
    // This is a finite f64 JSON number that overflows f32.
    assert!(decode_response(br#"{"data":[{"index":0,"embedding":[1e39]}]}"#, 1, 1).is_err());
}

#[test]
fn accepts_missing_or_partial_usage_and_rejects_invalid_accounting() {
    for usage in [serde_json::Value::Null, serde_json::json!({})] {
        let response = serde_json::json!({
            "data": [{"index": 0, "embedding": [0.5]}], "usage": usage,
        });
        assert!(
            decode_response(&serde_json::to_vec(&response).unwrap(), 1, 1)
                .unwrap()
                .usage
                .is_none()
        );
    }
    for usage in [
        serde_json::json!({"prompt_tokens": 3}),
        serde_json::json!({"total_tokens": 3}),
    ] {
        let response = serde_json::json!({
            "data": [{"index": 0, "embedding": [0.5]}], "usage": usage,
        });
        let usage = decode_response(&serde_json::to_vec(&response).unwrap(), 1, 1)
            .unwrap()
            .usage
            .unwrap();
        assert_eq!(usage.prompt_tokens, 3);
        assert_eq!(usage.total_tokens, 3);
    }
    for usage in [
        serde_json::json!({"prompt_tokens": -1}),
        serde_json::json!({"prompt_tokens": 3, "total_tokens": 2}),
        serde_json::json!({"total_tokens": "three"}),
    ] {
        let response = serde_json::json!({
            "data": [{"index": 0, "embedding": [0.5]}], "usage": usage,
        });
        assert!(decode_response(&serde_json::to_vec(&response).unwrap(), 1, 1).is_err());
    }
}

#[tokio::test]
async fn sends_expected_json_and_provider_authentication_to_http_stub() {
    use axum::{Json, Router, http::HeaderMap, routing::post};

    for implementation in [
        RemoteEmbeddingProvider::Internal,
        RemoteEmbeddingProvider::CloudflareWorkersAI,
        RemoteEmbeddingProvider::OpenAI,
        RemoteEmbeddingProvider::AzureOpenAI,
        RemoteEmbeddingProvider::HuggingfaceEndpoint,
        RemoteEmbeddingProvider::OpenAICompatible,
        RemoteEmbeddingProvider::Cohere,
        RemoteEmbeddingProvider::VoyageAI,
    ] {
        let cohere = implementation == RemoteEmbeddingProvider::Cohere;
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let router = Router::new().route(
            "/v1/embeddings",
            post(
                move |headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
                    sender.send((headers, body)).await.unwrap();
                    (
                        [("x-request-id", "request-header")],
                        Json(if cohere {
                            serde_json::json!({
                                "embeddings": {"float": [[0.1, 0.2]]},
                                "meta": {"billed_units": {"input_tokens": 5}},
                            })
                        } else {
                            serde_json::json!({
                                "data": [{ "index": 0, "embedding": [0.1, 0.2] }],
                                "usage": {"prompt_tokens": 5, "total_tokens": 5}
                            })
                        }),
                    )
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut payload = payload();
        payload.input = vec!["private test input".into()];
        let body = request_body(&provider(), &config(implementation.clone()), &payload);
        let result = send_request(
            &implementation,
            Url::parse(&format!("http://{address}/v1/embeddings")).unwrap(),
            "test-key",
            &body,
            1,
            2,
            2000,
        )
        .await
        .unwrap();
        let (headers, sent) = receiver.recv().await.unwrap();
        assert_eq!(sent, body);
        if implementation == RemoteEmbeddingProvider::AzureOpenAI {
            assert_eq!(headers["api-key"], "test-key");
            assert!(!headers.contains_key("authorization"));
        } else {
            assert_eq!(headers["authorization"], "Bearer test-key");
            assert!(!headers.contains_key("api-key"));
        }
        assert_eq!(
            result.provider_request_id.as_deref(),
            Some("request-header")
        );
        assert_eq!(result.embeddings, vec![vec![0.1, 0.2]]);
        assert_eq!(result.usage.unwrap().total_tokens, 5);
        server.abort();
    }
}

#[tokio::test]
async fn does_not_follow_redirects_retry_failures_or_echo_error_bodies() {
    use axum::{Router, http::StatusCode, response::Redirect, routing::post};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let calls = Arc::new(AtomicUsize::new(0));
    let redirects = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route("/redirect", post(|| async { Redirect::temporary("/sink") }))
        .route(
            "/sink",
            post({
                let redirects = redirects.clone();
                move || async move {
                    redirects.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        )
        .route(
            "/failed",
            post({
                let calls = calls.clone();
                move || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        "private-input secret-api-key",
                    )
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    for path in ["redirect", "failed"] {
        let result = send_request(
            &RemoteEmbeddingProvider::OpenAICompatible,
            Url::parse(&format!("http://{address}/{path}")).unwrap(),
            "secret-api-key",
            &serde_json::json!({"input": ["private-input"]}),
            1,
            2,
            2000,
        )
        .await;
        let error = result
            .err()
            .expect("upstream error must be returned")
            .to_string();
        assert!(!error.contains("private-input"));
        assert!(!error.contains("secret-api-key"));
        assert!(!error.contains(&address.to_string()));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(redirects.load(Ordering::SeqCst), 0);
    server.abort();
}
