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

fn gemma2() -> (EmbeddingModelProvider, RemoteExecutionConfig) {
    let mut provider = provider();
    provider.vector_length = 768;
    let mut config = config(RemoteEmbeddingProvider::Internal);
    config.model_id = Some("embeddinggemma-2".into());
    (provider, config)
}

fn openrouter_gemini() -> (EmbeddingModelProvider, RemoteExecutionConfig) {
    let mut provider = provider();
    provider.vector_length = 768;
    let mut config = config(RemoteEmbeddingProvider::OpenRouter);
    config.model_id = Some("google/gemini-embedding-2".into());
    (provider, config)
}

#[test]
fn openrouter_preserves_order_and_uses_embedding_media_parts_for_a_mixed_batch() {
    let (provider, config) = openrouter_gemini();
    let input = request(serde_json::json!([
        "plain text",
        {"type": "content", "parts": [
            {"type": "image", "source": "https://media.example/a.png?signature=a%2Fb"},
            {"type": "text", "text": "Compare these"},
            {"type": "image", "source": "data:image/jpeg;base64,AAAA"},
            {"type": "text", "text": " and this recording"},
            {"type": "audio", "source": "data:audio/mpeg;base64,AAAA"},
            {"type": "video", "source": "data:video/quicktime;base64,AAAA"},
            {"type": "file", "source": "AAAA", "mime_type": "application/pdf", "filename": "report.pdf"}
        ]}
    ]));
    validate_request(&provider, &config, &input).unwrap();
    let body = request_body(&provider, &config, &input);
    assert_eq!(
        body,
        serde_json::json!({
            "model": "google/gemini-embedding-2", "dimensions": 768,
            "encoding_format": "float", "input_type": "search_query",
            "input": [
                {"content": [{"type": "text", "text": "query: plain text"}]},
                {"content": [
                    {"type": "image_url", "image_url": {"url": "https://media.example/a.png?signature=a%2Fb"}},
                    {"type": "text", "text": "query: Compare these"},
                    {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,AAAA"}},
                    {"type": "text", "text": " and this recording"},
                    {"type": "input_audio", "input_audio": {"data": "data:audio/mpeg;base64,AAAA", "format": "mp3"}},
                    {"type": "input_video", "input_video": {"data": "data:video/quicktime;base64,AAAA", "format": "mov"}},
                    {"type": "input_file", "input_file": {"data": "data:application/pdf;base64,AAAA", "format": "pdf"}}
                ]}
            ]
        })
    );
    assert_eq!(media_metering_details(&input.input)["inlineMediaCount"], 4);
    assert_eq!(media_metering_details(&input.input)["mediaUrlCount"], 1);
}

#[test]
fn openrouter_text_and_legacy_media_keep_batch_and_task_semantics() {
    let (provider, config) = openrouter_gemini();
    let mut input = request(serde_json::json!({"type":"content", "parts": [
        {"type":"text", "text":"first"}, {"type":"text", "text":" second"}
    ]}));
    input.embed_type = EmbedType::Document;
    let body = request_body(&provider, &config, &input);
    assert_eq!(body["input"], serde_json::json!(["passage: first second"]));
    assert_eq!(body["input_type"], "search_document");
    assert_eq!(
        input_metered_bytes(&input.input[0], "passage: "),
        "passage: first second".len() as i64
    );
    for (kind, field, expected) in [
        ("image", "image", "image_url"),
        ("audio", "audio", "input_audio"),
        ("video", "video", "input_video"),
    ] {
        let input = request(serde_json::json!({"type": kind, field: "AAAA"}));
        validate_request(&provider, &config, &input).unwrap();
        assert_eq!(
            request_body(&provider, &config, &input)["input"][0]["content"][0]["type"],
            expected
        );
    }
}

#[test]
fn openrouter_validates_model_dimensions_media_formats_and_fixed_origin() {
    let (mut provider, mut config) = openrouter_gemini();
    assert_eq!(
        key_secret(&RemoteEmbeddingProvider::OpenRouter),
        "OPENROUTER_API_KEY"
    );
    assert_eq!(
        endpoint_url(
            &RemoteEmbeddingProvider::OpenRouter,
            Some("https://untrusted.example"),
            None
        )
        .unwrap()
        .as_str(),
        "https://openrouter.ai/api/v1/embeddings"
    );
    let image = request(serde_json::json!({"type":"image", "image":"AAAA"}));
    for dimensions in [128, 768, 1536, 3072] {
        provider.vector_length = dimensions;
        validate_request(&provider, &config, &image).unwrap();
        assert_eq!(
            request_body(&provider, &config, &image)["dimensions"],
            dimensions
        );
    }
    for dimensions in [0, 127, 3073] {
        provider.vector_length = dimensions;
        assert!(validate_request(&provider, &config, &image).is_err());
    }
    provider.vector_length = 768;
    for invalid in [
        serde_json::json!({"type":"content", "parts":[]}),
        serde_json::json!({"type":"content", "parts":vec![serde_json::json!({"type":"image", "source":"AAAA"});7]}),
        serde_json::json!({"type":"audio", "audio":"https://media.example/a.wav"}),
        serde_json::json!({"type":"video", "video":"https://media.example/a.mp4"}),
        serde_json::json!({"type":"image", "image":"data:image/svg+xml;base64,AAAA"}),
        serde_json::json!({"type":"audio", "audio":"data:image/png;base64,AAAA"}),
        serde_json::json!({"type":"audio", "audio":"badbase64"}),
        serde_json::json!({"type":"file", "source":"AAAA", "mime_type":"text/plain"}),
        serde_json::json!({"type":"file", "source":"data:image/png;base64,AAAA", "mime_type":"application/pdf"}),
    ] {
        assert!(
            validate_request(&provider, &config, &request(invalid.clone())).is_err(),
            "accepted {invalid}"
        );
    }
    config.endpoint_secret_name = Some("OTHER_ENDPOINT".into());
    assert!(validate_request(&provider, &config, &image).is_err());
    config.endpoint_secret_name = None;
    config.model_id = Some("openai/text-embedding-3-small".into());
    assert!(validate_request(&provider, &config, &image).is_err());
    validate_request(&provider, &config, &request(serde_json::json!("text"))).unwrap();
}

#[tokio::test]
async fn openrouter_bit_can_disable_dimension_requests_without_relaxing_response_validation() {
    use axum::{Json, Router, http::StatusCode, routing::post};

    let router = Router::new().route(
        "/v1/embeddings",
        post(|Json(body): Json<serde_json::Value>| async move {
            if body.get("dimensions").is_some() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Model does not support matryoshka representation"})),
                );
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "data": [{"index": 0, "embedding": vec![0.1; 1024]}],
                    "usage": {"prompt_tokens": 5, "total_tokens": 5, "cost": 0.00000005}
                })),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Url::parse(&format!(
        "http://{}/v1/embeddings",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut config = config(RemoteEmbeddingProvider::OpenRouter);
    config.model_id = Some("example/fixed-width-embedding".into());
    config.supports_dimensions = Some(false);
    let mut provider = provider();
    provider.vector_length = 1024;
    provider.input_length = 512;
    let input = request(serde_json::json!("test passage"));
    validate_request(&provider, &config, &input).unwrap();
    let body = request_body(&provider, &config, &input);
    let result = send_request(
        &RemoteEmbeddingProvider::OpenRouter,
        endpoint.clone(),
        "test-key",
        &body,
        1,
        provider.vector_length as usize,
        2000,
    )
    .await
    .unwrap();
    assert_eq!(result.embeddings.len(), 1);
    assert_eq!(result.embeddings[0].len(), 1024);
    assert!(
        openrouter::decode_response(
            &serde_json::to_vec(&serde_json::json!({
                "data": [{"index": 0, "embedding": vec![0.1; 512]}]
            }))
            .unwrap(),
            1,
            provider.vector_length as usize,
        )
        .is_err()
    );
    server.abort();
}

#[test]
fn openrouter_dimension_requests_follow_bit_configuration() {
    let mut config = config(RemoteEmbeddingProvider::OpenRouter);
    config.model_id = Some("example/embedding-model".into());
    let mut provider = provider();
    provider.vector_length = 768;
    for supports_dimensions in [None, Some(true), Some(false)] {
        config.supports_dimensions = supports_dimensions;
        let body = request_body(&provider, &config, &payload());
        if supports_dimensions == Some(false) {
            assert!(body.get("dimensions").is_none());
        } else {
            assert_eq!(body["dimensions"], 768);
        }
    }
}

fn request(input: serde_json::Value) -> EmbedRequest {
    serde_json::from_value(serde_json::json!({"model": "bit-id", "input": input})).unwrap()
}

#[test]
fn accepts_single_text_structured_inputs_and_legacy_or_mixed_batches() {
    for input in [
        serde_json::json!("text"),
        serde_json::json!({"type": "text", "text": "text"}),
        serde_json::json!({"type": "image", "image": "AAAA"}),
        serde_json::json!({"type": "audio", "audio": "data:audio/wav;base64,AAAA"}),
        serde_json::json!({"type": "video", "video": "https://media.example/video.mp4"}),
    ] {
        assert_eq!(request(input).input.len(), 1);
    }
    let mixed = request(serde_json::json!([
        "text", {"type": "text", "text": "second"},
        {"type": "multimodal", "text": "Describe <|image|>", "image": "AAAA"}
    ]));
    assert_eq!(mixed.input.len(), 3);
    let (provider, config) = gemma2();
    validate_request(&provider, &config, &mixed).unwrap();
    assert_eq!(
        request(serde_json::json!(["first", "second"])).input.len(),
        2
    );
    for invalid in [
        serde_json::json!({"type": "image", "audio": "AAAA"}),
        serde_json::json!({"type": "text", "text": "text", "image": "AAAA"}),
        serde_json::json!(42),
    ] {
        assert!(
            serde_json::from_value::<EmbedRequest>(serde_json::json!({
                "model": "bit-id", "input": invalid
            }))
            .is_err()
        );
    }
}

#[test]
fn rejects_media_for_every_other_provider_model_and_wrong_gemma_dimensions() {
    let input = request(serde_json::json!({"type": "image", "image": "AAAA"}));
    let multimodal_text = request(serde_json::json!({"type": "multimodal", "text": "hello"}));
    let (mut provider, mut config) = gemma2();
    validate_request(&provider, &config, &input).unwrap();
    for implementation in [
        RemoteEmbeddingProvider::CloudflareWorkersAI,
        RemoteEmbeddingProvider::OpenAI,
        RemoteEmbeddingProvider::OpenRouter,
        RemoteEmbeddingProvider::AzureOpenAI,
        RemoteEmbeddingProvider::HuggingfaceEndpoint,
        RemoteEmbeddingProvider::OpenAICompatible,
        RemoteEmbeddingProvider::Cohere,
        RemoteEmbeddingProvider::VoyageAI,
    ] {
        config.implementation = Some(implementation);
        assert!(validate_request(&provider, &config, &input).is_err());
        assert_eq!(
            validate_request(&provider, &config, &multimodal_text).is_err(),
            implementation != RemoteEmbeddingProvider::OpenRouter,
        );
    }
    config.implementation = Some(RemoteEmbeddingProvider::Internal);
    config.model_id = Some("gte-multilingual-base".into());
    assert!(validate_request(&provider, &config, &input).is_err());
    config.model_id = Some("embeddinggemma-2".into());
    provider.vector_length = 256;
    assert!(validate_request(&provider, &config, &input).is_err());
}

#[test]
fn preserves_signed_media_and_placeholders_while_prefixing_text_only() {
    let (provider, config) = gemma2();
    let signed = "https://media.example/video.mp4?X-Signature=a%2Fb%2Bc&part=2&part=1";
    let mut payload = request(serde_json::json!([
        "plain",
        {"type": "image", "image": "AAAA"},
        {"type": "multimodal", "text": "Hear <|audio|> then see <|video|>",
         "audio": "data:audio/wav;base64,AAAA", "video": signed}
    ]));
    validate_request(&provider, &config, &payload).unwrap();
    let body = request_body(&provider, &config, &payload);
    assert_eq!(body["input"][0], "query: plain");
    assert_eq!(
        body["input"][1],
        serde_json::json!({"type": "image", "image": "AAAA"})
    );
    assert_eq!(
        body["input"][2]["text"],
        "query: Hear <|audio|> then see <|video|>"
    );
    assert_eq!(body["input"][2]["video"], signed);
    assert_eq!(body["input"][2]["audio"], "data:audio/wav;base64,AAAA");
    payload.embed_type = EmbedType::Document;
    assert_eq!(
        request_body(&provider, &config, &payload)["input"][2]["text"],
        "passage: Hear <|audio|> then see <|video|>"
    );
    let text = request(serde_json::json!({"type": "text", "text": "structured"}));
    assert_eq!(
        request_body(&provider, &config, &text)["input"][0],
        serde_json::json!({"type": "text", "text": "query: structured"})
    );
    assert_eq!(
        request_body(
            &provider,
            &self::config(RemoteEmbeddingProvider::OpenAICompatible),
            &text
        )["input"][0],
        "query: structured"
    );
}

#[test]
fn validates_empty_media_invalid_schemes_and_inline_limits_without_fetching() {
    let (provider, config) = gemma2();
    for input in [
        serde_json::json!({"type": "multimodal"}),
        serde_json::json!({"type": "image", "image": ""}),
        serde_json::json!({"type": "image", "image": "file:///tmp/image.png"}),
        serde_json::json!({"type": "audio", "audio": "data:audio/wav,AAAA"}),
        serde_json::json!({"type": "video", "video": "data:video/mp4;base64,"}),
    ] {
        assert!(validate_request(&provider, &config, &request(input)).is_err());
    }
    // A URL cannot be contacted during validation, including hosts that do not resolve.
    validate_request(
        &provider,
        &config,
        &request(serde_json::json!({
            "type": "image", "image": "https://unresolved.invalid/image?signature=keep%2Bme"
        })),
    )
    .unwrap();
    assert!(validate_media(&"A".repeat(MAX_ENCODED_MEDIA_BYTES + 1)).is_err());
}

#[test]
fn media_accounting_includes_inline_data_and_reserves_download_bounds_for_urls() {
    let payload = request(serde_json::json!({
        "type": "multimodal", "text": "<|image|> <|audio|>",
        "image": "data:image/png;base64,AAAA", "audio": "https://media.example/a.wav?token=x"
    }));
    let text_bytes = "query: <|image|> <|audio|>".len();
    assert_eq!(
        input_metered_bytes(&payload.input[0], "query: "),
        (text_bytes
            + "data:image/png;base64,AAAA".len()
            + "https://media.example/a.wav?token=x".len()) as i64
    );
    assert_eq!(
        reservation_input_bytes(&payload.input, "query: "),
        (text_bytes + "data:image/png;base64,AAAA".len() + MAX_ENCODED_MEDIA_BYTES) as i64
    );
    let media_only =
        request(serde_json::json!({"type": "image", "image": "https://media.example/a"}));
    assert!(input_metered_bytes(&media_only.input[0], "") > 0);
    assert_eq!(
        reservation_input_bytes(&media_only.input, ""),
        MAX_ENCODED_MEDIA_BYTES as i64
    );
    assert_eq!(
        media_metering_details(&payload.input),
        serde_json::json!({
            "inlineMediaCount": 1,
            "mediaUrlCount": 1,
            "mediaByteBasis": "supplied_encoded_bytes_and_url_strings",
            "downloadedMediaBytesAvailable": false
        })
    );
}

#[tokio::test]
async fn sends_mixed_gemma2_batch_to_gateway_and_restores_index_order() {
    use axum::{Json, Router, http::HeaderMap, routing::post};
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let router = Router::new().route(
        "/v1/embeddings",
        post(
            move |headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
                sender.send((headers, body)).await.unwrap();
                Json(serde_json::json!({
                    "model": "embeddinggemma-2",
                    "data": [
                        {"index": 1, "embedding": vec![0.2; 768]},
                        {"index": 0, "embedding": vec![0.1; 768]}
                    ], "usage": {"prompt_tokens": 3, "total_tokens": 3}
                }))
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (provider, config) = gemma2();
    let payload = request(serde_json::json!([
        "plain", {"type": "video", "video": "https://media.invalid/v?key=a%2Bb&x=2"}
    ]));
    validate_request(&provider, &config, &payload).unwrap();
    let body = request_body(&provider, &config, &payload);
    let result = send_request(
        &RemoteEmbeddingProvider::Internal,
        Url::parse(&format!("http://{address}/v1/embeddings")).unwrap(),
        "test-gateway-key",
        &body,
        2,
        768,
        2000,
    )
    .await
    .unwrap();
    let (headers, sent) = receiver.recv().await.unwrap();
    assert_eq!(headers["authorization"], "Bearer test-gateway-key");
    assert_eq!(
        sent["input"][1]["video"],
        "https://media.invalid/v?key=a%2Bb&x=2"
    );
    assert_eq!(sent, body);
    assert_eq!(result.embeddings, vec![vec![0.1; 768], vec![0.2; 768]]);
    server.abort();
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

fn cloudflare_gemma2() -> (EmbeddingModelProvider, RemoteExecutionConfig) {
    let (mut provider, mut config) = gemma2();
    provider.prefix.query = "task: search result | query: ".into();
    provider.prefix.paragraph = "title: none | text: ".into();
    config.implementation = Some(RemoteEmbeddingProvider::CloudflareWorkersAI);
    config.model_id = Some("@cf/google/embeddinggemma-2".into());
    (provider, config)
}

#[test]
fn cloudflare_gemma2_uses_native_endpoint_and_requested_text_dimensions() {
    assert_eq!(
        cloudflare::endpoint_url(Some("0123456789abcdef0123456789abcdef"))
            .unwrap()
            .as_str(),
        "https://api.cloudflare.com/client/v4/accounts/0123456789abcdef0123456789abcdef/ai/run/@cf/google/embeddinggemma-2"
    );
    assert!(cloudflare::endpoint_url(Some("account/../../escape")).is_err());
    let (mut provider, mut config) = cloudflare_gemma2();
    config.model_id = Some(" @cf/google/embeddinggemma-2 ".into());
    let mut input = request(serde_json::json!([
        "first", {"type":"text", "text":"second"},
        {"type":"content", "parts":[{"type":"text", "text":"third"},{"type":"text", "text":" fourth"}]}
    ]));
    for dimensions in [128, 256, 512, 768] {
        provider.vector_length = dimensions;
        validate_request(&provider, &config, &input).unwrap();
        assert_eq!(
            request_body(&provider, &config, &input),
            serde_json::json!({
                "input": ["task: search result | query: first", "task: search result | query: second", "task: search result | query: third fourth"],
                "dimensions": dimensions, "encoding_format": "float"
            })
        );
    }
    input.embed_type = EmbedType::Document;
    assert_eq!(
        request_body(&provider, &config, &input)["input"][0],
        "title: none | text: first"
    );
    for dimensions in [0, 127, 384, 769] {
        provider.vector_length = dimensions;
        assert!(validate_request(&provider, &config, &input).is_err());
    }
}

#[test]
fn cloudflare_gemma2_validates_media_before_admission() {
    let (mut provider, mut config) = cloudflare_gemma2();
    for invalid in [
        serde_json::json!({"type":"content", "parts":[]}),
        serde_json::json!({"type":"multimodal"}),
        serde_json::json!({"type":"image", "image":"https://media.example/a.png"}),
        serde_json::json!({"type":"audio", "audio":"https://media.example/a.wav"}),
        serde_json::json!({"type":"video", "video":"https://media.example/a.mp4"}),
        serde_json::json!({"type":"audio", "audio":"data:image/png;base64,AAAA"}),
        serde_json::json!({"type":"video", "video":"data:video/mp4;base64,badbase64"}),
        serde_json::json!({"type":"image", "image":"data:image/png,AAAA"}),
        serde_json::json!({"type":"file", "source":"AAAA", "mime_type":"application/pdf"}),
        serde_json::json!({"type":"content", "parts":[{"type":"file", "source":"AAAA", "mime_type":"application/pdf"}]}),
    ] {
        assert!(
            validate_request(&provider, &config, &request(invalid.clone())).is_err(),
            "accepted {invalid}"
        );
    }
    let image = request(serde_json::json!({"type":"image", "image":"data:image/png;base64,AAAA"}));
    for input_length in [0, 8193] {
        provider.input_length = input_length;
        assert!(validate_request(&provider, &config, &image).is_err());
    }
    provider.input_length = 8192;
    config.endpoint_secret_name = Some("OTHER_ENDPOINT".into());
    assert!(validate_request(&provider, &config, &image).is_err());
    config.endpoint_secret_name = None;
    config.model_id = Some("@cf/google/embeddinggemma-300m".into());
    assert!(validate_request(&provider, &config, &image).is_err());
}

#[tokio::test]
async fn cloudflare_gemma2_sends_ordered_media_batches_and_decodes_native_envelopes() {
    use axum::{Json, Router, http::HeaderMap, routing::post};
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let router = Router::new().route(
        "/ai/run/@cf/google/embeddinggemma-2",
        post(move |headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
            sender.send((headers, body)).await.unwrap();
            Json(serde_json::json!({"success":true, "errors":[], "result": {
                "id":"embd-test", "model":"@cf/google/embeddinggemma-2",
                "data":[{"index":1,"embedding":vec![0.2;256]}, {"index":0,"embedding":vec![0.1;256]}],
                "usage":{"prompt_tokens":266,"total_tokens":266}
            }}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (mut provider, config) = cloudflare_gemma2();
    provider.vector_length = 256;
    let input = request(serde_json::json!([
        {"type":"content", "parts":[
            {"type":"image", "source":"data:image/jpeg;base64,AAAA"},
            {"type":"text", "text":"Compare"},
            {"type":"image", "source":"AAAA"},
            {"type":"text", "text":" these"},
            {"type":"audio", "source":"data:audio/wav;base64,AAAA"},
            {"type":"video", "source":"data:video/mp4;base64,AAAA"}
        ]}, "plain"
    ]));
    validate_request(&provider, &config, &input).unwrap();
    let body = request_body(&provider, &config, &input);
    assert_eq!(
        body,
        serde_json::json!({"dimensions":256,"encoding_format":"float","messages":[
            [{"role":"user","content":[
                {"type":"image_url","image_url":{"url":"data:image/jpeg;base64,AAAA"}},
                {"type":"text","text":"task: search result | query: Compare"},
                {"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}},
                {"type":"text","text":" these"},
                {"type":"input_audio","input_audio":{"data":"AAAA","format":"wav"}},
                {"type":"video_url","video_url":{"url":"data:video/mp4;base64,AAAA"}}
            ]}],
            [{"role":"user","content":[{"type":"text","text":"task: search result | query: plain"}]}]
        ]})
    );
    let result = send_request(
        &RemoteEmbeddingProvider::CloudflareWorkersAI,
        Url::parse(&format!(
            "http://{address}/ai/run/@cf/google/embeddinggemma-2"
        ))
        .unwrap(),
        "test-cloudflare-token",
        &body,
        2,
        256,
        2000,
    )
    .await
    .unwrap();
    let (headers, sent) = receiver.recv().await.unwrap();
    assert_eq!(headers["authorization"], "Bearer test-cloudflare-token");
    assert_eq!(sent, body);
    assert_eq!(result.embeddings, vec![vec![0.1; 256], vec![0.2; 256]]);
    assert_eq!(result.usage.unwrap().total_tokens, 266);
    assert_eq!(result.provider_request_id.as_deref(), Some("embd-test"));
    server.abort();
}

#[test]
fn cloudflare_accepts_flat_results_but_rejects_failed_or_invalid_envelopes() {
    let result = serde_json::json!({"data":[{"index":0,"embedding":[0.1,0.2]}]});
    assert!(cloudflare::decode_response(&serde_json::to_vec(&result).unwrap(), 1, 2).is_ok());
    for invalid in [
        serde_json::json!({"success":false,"result":result}),
        serde_json::json!({"result":result}),
        serde_json::json!({"success":true,"result":{"data":[{"index":0,"embedding":[0.1]}]}}),
        serde_json::json!({"success":true,"result":{"data":[{"index":1,"embedding":[0.1,0.2]}]}}),
    ] {
        assert!(cloudflare::decode_response(&serde_json::to_vec(&invalid).unwrap(), 1, 2).is_err());
    }
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
        input.input = invalid.into_iter().map(Into::into).collect();
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
        RemoteEmbeddingProvider::OpenRouter,
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
fn openrouter_accepts_unindexed_vectors_and_rejects_ambiguous_order() {
    let response = serde_json::json!({
        "data": [{"embedding": [0.1, 0.2]}, {"embedding": [0.3, 0.4]}],
        "usage": {"prompt_tokens": 8, "total_tokens": 8, "cost": 0.001},
    });
    let result =
        openrouter::decode_response(&serde_json::to_vec(&response).unwrap(), 2, 2).unwrap();
    assert_eq!(result.embeddings, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);
    assert_eq!(result.raw_usage.unwrap()["cost"], 0.001);
    for data in [
        serde_json::json!([
            {"index": 1, "embedding": [0.1, 0.2]}, {"embedding": [0.3, 0.4]},
        ]),
        serde_json::json!([
            {"index": 0, "embedding": [0.1, 0.2]}, {"index": 0, "embedding": [0.3, 0.4]},
        ]),
        serde_json::json!([{"embedding": [0.1, 0.2]}]),
        serde_json::json!([{"embedding": [0.1]}, {"embedding": [0.3, 0.4]}]),
    ] {
        assert!(
            openrouter::decode_response(
                &serde_json::to_vec(&serde_json::json!({"data": data})).unwrap(),
                2,
                2
            )
            .is_err()
        );
    }
    // Existing providers still require their documented indexes.
    assert!(decode_response(&serde_json::to_vec(&response).unwrap(), 2, 2).is_err());
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
        RemoteEmbeddingProvider::OpenRouter,
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
