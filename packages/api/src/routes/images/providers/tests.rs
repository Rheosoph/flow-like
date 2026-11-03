use super::*;

fn request() -> HostedImageRequest {
    serde_json::from_value(json!({"model":"catalog-bit","prompt":"a mountain"})).unwrap()
}

#[test]
fn openrouter_wire_locks_model_count_and_streaming() {
    let body = wire_body(Provider::OpenRouter, "google/test-image", &request()).unwrap();
    assert_eq!(body["model"], "google/test-image");
    assert_eq!(body["n"], 1);
    assert_eq!(body["stream"], false);
    assert!(body.get("provider").is_none());
    assert!(body.get("input_references").is_none());
    let mut negative = request();
    negative.negative_prompt = Some("clouds".into());
    assert!(wire_body(Provider::OpenRouter, "google/test-image", &negative).is_err());
    for field in [
        "n",
        "stream",
        "provider",
        "api_key",
        "endpoint",
        "input_references",
    ] {
        let mut payload = json!({"model":"catalog-bit","prompt":"a mountain"});
        payload[field] = json!("injected");
        assert!(
            serde_json::from_value::<HostedImageRequest>(payload).is_err(),
            "{field}"
        );
    }
}

#[test]
fn cloudflare_uses_fixed_steps_and_native_dimensions() {
    let mut payload = request();
    payload.seed = Some(12);
    assert_eq!(
        wire_body(Provider::Cloudflare, FLUX, &payload).unwrap(),
        json!({"prompt":"a mountain","seed":12,"steps":4})
    );
    payload.aspect_ratio = Some("16:9".into());
    assert!(wire_body(Provider::Cloudflare, FLUX, &payload).is_err());
    let body = wire_body(Provider::Cloudflare, SDXL, &payload).unwrap();
    assert_eq!(body["width"], 1024);
    assert_eq!(body["height"], 576);
    assert_eq!(body["num_steps"], 20);
    payload.negative_prompt = Some("clouds".into());
    payload.size = Some("768x1024".into());
    payload.aspect_ratio = None;
    let body = wire_body(Provider::Cloudflare, SDXL, &payload).unwrap();
    assert_eq!(body["negative_prompt"], "clouds");
    assert_eq!(body["width"], 768);
    payload.quality = Some("high".into());
    assert!(wire_body(Provider::Cloudflare, SDXL, &payload).is_err());
    let mut format = request();
    format.output_format = Some("png".into());
    assert!(wire_body(Provider::Cloudflare, FLUX, &format).is_err());
    assert!(wire_body(Provider::Cloudflare, SDXL, &format).is_ok());
    assert!(
        wire_body(
            Provider::Cloudflare,
            "@cf/black-forest-labs/flux-2-dev",
            &request()
        )
        .is_err()
    );
}

#[test]
fn cloudflare_account_and_model_cannot_escape_the_endpoint() {
    let account = "a".repeat(32);
    assert_eq!(
        cloudflare_url(&account, FLUX).unwrap(),
        format!("https://api.cloudflare.com/client/v4/accounts/{account}/ai/run/{FLUX}")
    );
    for invalid in [
        "https://attacker.invalid",
        "account/../other",
        "a?token=secret",
    ] {
        assert!(cloudflare_url(invalid, FLUX).is_err());
        assert!(cloudflare_url(&account, invalid).is_err());
    }
}

#[test]
fn binary_and_base64_responses_report_the_actual_image_format() {
    let png = b"\x89PNG\r\n\x1a\nfixture";
    let image = parse_response(Provider::Cloudflare, "image/png", png).unwrap();
    assert_eq!(image.image.mime_type, "image/png");
    let jpeg = b"\xff\xd8\xfffixture";
    let bytes =
        serde_json::to_vec(&json!({"success":true,"result":{"image":STANDARD.encode(jpeg)}}))
            .unwrap();
    let image = parse_response(Provider::Cloudflare, "application/json", &bytes).unwrap();
    assert_eq!(image.image.mime_type, "image/jpeg");
    let bytes = serde_json::to_vec(
        &json!({"data":[{"b64_json":STANDARD.encode(png),"media_type":"image/jpeg"}],
        "usage":{"cost":0.01}}),
    )
    .unwrap();
    let image = parse_response(Provider::OpenRouter, "application/json", &bytes).unwrap();
    assert_eq!(image.image.mime_type, "image/png");
    assert_eq!(image.usage.unwrap()["cost"], 0.01);
}

#[test]
fn responses_require_one_inline_raster_image_and_never_fetch_urls() {
    for value in [
        json!({"data":[{"url":"https://private.invalid/image"}]}),
        json!({"data":[]}),
        json!({"data":[{},{}]}),
        json!({"data":[{"b64_json":"not base64"}]}),
        json!({"data":[{"b64_json":STANDARD.encode(b"<svg onload='alert(1)'/>")}]}),
        json!({"error":{"message":"secret provider error"}}),
    ] {
        let error = parse_response(
            Provider::OpenRouter,
            "application/json",
            &serde_json::to_vec(&value).unwrap(),
        )
        .err()
        .unwrap();
        assert!(!error.to_string().contains("secret provider error"));
    }
    assert!(image_data(&vec![0; MAX_IMAGE_BYTES + 1]).is_err());
}

#[test]
fn settlement_uses_reported_cost_without_adding_the_reserved_maximum() {
    assert_eq!(
        settlement_cost(
            Provider::OpenRouter,
            Some(&json!({"cost":0.0123451})),
            100_000
        ),
        Some(12_346)
    );
    assert_eq!(
        settlement_cost(Provider::OpenRouter, Some(&json!({"cost":0})), 100_000),
        Some(0)
    );
    for usage in [
        None,
        Some(json!({})),
        Some(json!({"cost":null})),
        Some(json!({"cost":-1})),
        Some(json!({"cost":"0.01"})),
        Some(json!({"cost":1e100})),
    ] {
        assert_eq!(
            settlement_cost(Provider::OpenRouter, usage.as_ref(), 100_000),
            None
        );
    }
    assert_eq!(
        settlement_cost(Provider::Cloudflare, None, 5_000),
        Some(5_000)
    );
}

#[test]
fn accounting_metadata_drops_echoed_prompts_and_arbitrary_provider_fields() {
    assert_eq!(
        sanitized_usage(Some(&json!({"cost":0.1,"prompt_tokens":12,
        "prompt":"private prompt", "images":["bytes"],"provider_debug":{"api_key":"secret"}}))),
        Some(json!({"cost":0.1,"prompt_tokens":12}))
    );
    assert_eq!(
        sanitized_usage(Some(&json!({"cost":"secret echoed text"}))),
        Some(json!({}))
    );
}

#[tokio::test]
async fn openrouter_transport_sends_one_bounded_request_and_normalizes_the_response() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let length = socket.read(&mut buffer).await.unwrap();
            assert!(length > 0);
            received.extend_from_slice(&buffer[..length]);
            if let Some(offset) = received.windows(4).position(|value| value == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&received[..offset]).to_ascii_lowercase();
                let body_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                if received.len() >= offset + 4 + body_length {
                    break;
                }
            }
        }
        let body = serde_json::to_string(
            &json!({"data":[{"b64_json":STANDARD.encode(b"\x89PNG\r\n\x1a\nfixture")}],
            "usage":{"cost":0.01}}),
        )
        .unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        received
    });
    let mut started = false;
    let result = call(
        Connection {
            url: format!("http://{address}/api/v1/images"),
            key: "server-owned-key".into(),
        },
        Provider::OpenRouter,
        "google/image",
        &request(),
        2000,
        &mut started,
    )
    .await
    .unwrap();
    assert_eq!(result.image.mime_type, "image/png");
    assert!(started);
    let received = String::from_utf8(server.await.unwrap()).unwrap();
    assert!(received.starts_with("POST /api/v1/images HTTP/1.1"));
    assert!(
        received
            .to_ascii_lowercase()
            .contains("authorization: bearer server-owned-key\r\n")
    );
    let body: Value = serde_json::from_str(received.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["model"], "google/image");
    assert_eq!(body["n"], 1);
    assert_eq!(body["stream"], false);
}

#[tokio::test]
async fn invalid_request_headers_do_not_mark_a_provider_request_started() {
    let mut started = false;
    let result = call(
        Connection {
            url: "http://127.0.0.1:9/images".into(),
            key: "invalid\nkey".into(),
        },
        Provider::OpenRouter,
        "google/image",
        &request(),
        100,
        &mut started,
    )
    .await;
    assert!(result.is_err());
    assert!(!started);
}

#[tokio::test]
async fn image_credentials_are_never_forwarded_on_redirect() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        socket.read(&mut buffer).await.unwrap();
        let response = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{address}/credential-leak\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        drop(socket);
        assert!(
            tokio::time::timeout(Duration::from_millis(150), listener.accept())
                .await
                .is_err()
        );
    });
    let mut started = false;
    let result = call(
        Connection {
            url: format!("http://{address}/images"),
            key: "server-owned-key".into(),
        },
        Provider::OpenRouter,
        "google/image",
        &request(),
        2000,
        &mut started,
    )
    .await;
    assert!(result.is_err());
    assert!(started);
    server.await.unwrap();
}
