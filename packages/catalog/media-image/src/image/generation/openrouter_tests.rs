use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn request() -> ImageGenerationRequest {
    ImageGenerationRequest {
        prompt: "A lighthouse".into(),
        system_prompt: None,
        negative_prompt: None,
        count: 1,
        aspect_ratio: None,
        size: Some("1536x1024".into()),
        quality: None,
        output_format: "png".into(),
        seed: None,
        background: None,
        provider_options: HashMap::new(),
    }
}

fn capabilities() -> Value {
    json!({
        "id": "vendor/painter",
        "endpoints": [{
            "provider_name": "Provider",
            "provider_slug": "provider",
            "provider_tag": "provider",
            "supported_parameters": {
                "resolution": {"type": "enum", "values": ["1K", "2K"]},
                "aspect_ratio": {"type": "enum", "values": ["1:1", "3:2", "2:3"]},
                "n": {"type": "range", "min": 1, "max": 4},
                "seed": {"type": "boolean"}
            },
            "supports_streaming": false,
            "allowed_passthrough_parameters": []
        }]
    })
}

fn image_response() -> Value {
    json!({"data": [
        {"b64_json": STANDARD.encode(b"\x89PNG\r\n\x1a\n"), "media_type": "image/png"},
        {"b64_json": STANDARD.encode(b"\xff\xd8\xff"), "media_type": "image/png"}
    ]})
}

#[test]
fn formats_are_explicit_only_when_requested_by_node_options() {
    let request = request();
    let fields = openrouter_fields(&request, &ImageGenerationProviderOptions::Default);
    assert!(fields.get("output_format").is_none());
    assert_eq!(fields["size"], "1536x1024");
    assert!(fields.get("aspect_ratio").is_none());
    let fields = openrouter_fields(
        &request,
        &ImageGenerationProviderOptions::OpenAi(OpenAiImageOptions {
            output_format: ImageOutputFormat::Webp,
            ..Default::default()
        }),
    );
    assert_eq!(fields["output_format"], "webp");
}

#[test]
fn inline_image_responses_keep_the_actual_format_and_drop_encoded_metadata() {
    let images = parse_openrouter_images(&image_response()).unwrap();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].mime_type.as_deref(), Some("image/png"));
    assert_eq!(images[1].mime_type.as_deref(), Some("image/jpeg"));
    assert_eq!(images[1].bytes, b"\xff\xd8\xff");
    assert!(
        images
            .iter()
            .all(|image| image.provider_metadata.get("b64_json").is_none())
    );

    let inferred = parse_openrouter_images(&json!({"data": [{
        "b64_json": STANDARD.encode(b"RIFF\x00\x00\x00\x00WEBP"), "media_type": "image/png"
    }]}))
    .unwrap();
    assert_eq!(inferred[0].mime_type.as_deref(), Some("image/webp"));
}

#[test]
fn invalid_or_non_raster_image_responses_are_refused() {
    for value in [
        json!({"data": []}),
        json!({"choices": [{"message": {"content": "no image"}}]}),
        json!({"data": [{"url": "https://example.invalid/image.png"}]}),
        json!({"data": [{"b64_json": "%%%"}]}),
        json!({"data": [{"b64_json": STANDARD.encode(b"<svg></svg>"), "media_type": "image/svg+xml"}]}),
    ] {
        assert!(parse_openrouter_images(&value).is_err(), "{value}");
    }
}

async fn serve(
    responses: Vec<(u16, Value)>,
) -> (String, tokio::task::JoinHandle<Vec<(String, Value)>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let (head_end, content_length) = loop {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                received.extend_from_slice(&chunk[..read]);
                if let Some(position) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&received[..position]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    break (position + 4, length);
                }
            };
            while received.len() < head_end + content_length {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                received.extend_from_slice(&chunk[..read]);
            }
            let first_line = String::from_utf8_lossy(&received[..head_end])
                .lines()
                .next()
                .unwrap()
                .to_owned();
            let payload = if content_length == 0 {
                Value::Null
            } else {
                flow_like_types::json::from_slice(&received[head_end..head_end + content_length])
                    .unwrap()
            };
            requests.push((first_line, payload));
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
        requests
    });
    (format!("http://{address}/api/v1"), server)
}

fn provider(endpoint: &str) -> ModelProvider {
    ModelProvider {
        provider_name: PROVIDER_OPENROUTER.into(),
        model_id: Some("vendor/painter".into()),
        version: None,
        api_surface: None,
        params: Some(HashMap::from([
            ("endpoint".into(), json!(endpoint)),
            ("api_key".into(), json!("test-key")),
        ])),
    }
}

fn client() -> GuardedHttpClient {
    GuardedHttpClient::configured_without_redirects(ExecutionEnvironment::Local, |builder| {
        builder.no_proxy().retry(reqwest::retry::never())
    })
    .unwrap()
}

#[tokio::test]
async fn direct_images_use_discovery_and_the_image_api_without_changing_the_model() {
    let (endpoint, server) = serve(vec![(200, capabilities()), (200, image_response())]).await;
    let mut request = request();
    request.count = 2;
    let images = tokio::time::timeout(
        Duration::from_secs(5),
        generate_openrouter(
            &client(),
            &provider(&endpoint),
            &request,
            &ImageGenerationProviderOptions::Default,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(images.len(), 2);
    let requests = server.await.unwrap();
    assert_eq!(
        requests[0].0,
        "GET /api/v1/images/models/vendor/painter/endpoints HTTP/1.1"
    );
    assert_eq!(requests[1].0, "POST /api/v1/images HTTP/1.1");
    let body = &requests[1].1;
    assert_eq!(body["model"], "vendor/painter");
    assert_eq!(body["prompt"], "A lighthouse");
    assert_eq!(body["n"], 2);
    assert_eq!(body["aspect_ratio"], "3:2");
    assert!([json!("1K"), json!("2K")].contains(&body["resolution"]));
    for key in [
        "size",
        "modalities",
        "messages",
        "image_config",
        "output_format",
    ] {
        assert!(body.get(key).is_none(), "{key}");
    }
}

#[tokio::test]
async fn unsupported_explicit_options_stop_before_generation() {
    let (endpoint, server) = serve(vec![(200, capabilities())]).await;
    let mut request = request();
    request.background = Some("transparent".into());
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        generate_openrouter(
            &client(),
            &provider(&endpoint),
            &request,
            &ImageGenerationProviderOptions::Default,
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("background"), "{error}");
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn failed_capability_discovery_stops_before_generation() {
    let (endpoint, server) = serve(vec![(502, json!({"error": "unavailable"}))]).await;
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        generate_openrouter(
            &client(),
            &provider(&endpoint),
            &request(),
            &ImageGenerationProviderOptions::Default,
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(
        error.to_string().contains("capabilities returned HTTP 502"),
        "{error}"
    );
    assert_eq!(server.await.unwrap().len(), 1);
}
