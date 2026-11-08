use super::*;
use crate::llm::test_support::{json_response, serve_once};
use flow_like_types_contracts::authorization::{
    AuthorizationError, AuthorizationFuture, AuthorizationRequest, RequestAuthorization,
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aGZkAAAAASUVORK5CYII=";

fn request() -> HostedImageRequest {
    HostedImageRequest {
        model: "catalog-image-bit".into(),
        prompt: "A lighthouse on a clear day".into(),
        ..Default::default()
    }
}

fn response() -> Value {
    json!({"model": "catalog-image-bit", "data": [{"b64_json": PNG, "mime_type": "image/png"}]})
}

#[test]
fn requests_bound_work_and_reject_routing_or_batch_overrides() {
    request().validate().unwrap();
    for (key, value) in [
        ("n", json!(2)),
        ("provider", json!({"only": ["unapproved"]})),
        ("endpoint", json!("https://other.example")),
        ("api_key", json!("private-key")),
        ("extra_body", json!({"n": 10})),
    ] {
        let mut body = serde_json::to_value(request()).unwrap();
        body[key] = value;
        assert!(serde_json::from_value::<HostedImageRequest>(body).is_err());
    }
    for (key, value) in [
        ("model", json!(" ")),
        ("prompt", json!(" ")),
        ("prompt", json!("é".repeat(2049))),
        ("negative_prompt", json!("x".repeat(2049))),
        ("size", json!("4096x4096")),
        ("size", json!("255x256")),
        ("size", json!("1024x-1")),
        ("seed", json!(u64::from(u32::MAX) + 1)),
        ("quality", json!("premium")),
        ("output_format", json!("svg")),
        ("background", json!("erase")),
        ("aspect_ratio", json!("99:1")),
    ] {
        let mut body = serde_json::to_value(request()).unwrap();
        body[key] = value;
        assert!(
            serde_json::from_value::<HostedImageRequest>(body)
                .unwrap()
                .validate()
                .is_err(),
            "{key}"
        );
    }
    let mut maximum = request();
    maximum.prompt = "é".repeat(2048);
    maximum.size = Some("2048x2048".into());
    maximum.seed = Some(u64::from(u32::MAX));
    maximum.validate().unwrap();
}

#[tokio::test]
async fn sends_catalog_identity_options_and_current_user_attribution() {
    let client = HostedImageClient::new("https://api.example/api/v1/", "executor-token")
        .unwrap()
        .with_usage_headers(vec![
            ("x-flow-like-app-id".into(), "project".into()),
            ("x-flow-like-run-id".into(), "run".into()),
        ]);
    let mut input = request();
    input.size = Some("1024x1024".into());
    input.seed = Some(0);
    let request = client.build_request(&input).await.unwrap();
    assert_eq!(
        request.url().as_str(),
        "https://api.example/api/v1/images/generations"
    );
    assert_eq!(request.headers()["authorization"], "Bearer executor-token");
    assert_eq!(request.headers()["x-flow-like-app-id"], "project");
    assert_eq!(request.headers()["x-flow-like-run-id"], "run");
    let body: Value = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
    assert_eq!(
        body,
        json!({"model": "catalog-image-bit", "prompt": input.prompt, "size": "1024x1024", "seed": 0})
    );
}

struct RotatingAuthorizer(AtomicUsize);

impl RequestAuthorizer for RotatingAuthorizer {
    fn attribution(&self) -> AuthorizationAttribution {
        AuthorizationAttribution::InstanceGrant
    }

    fn resource_base_url(&self, audience: ResourceAudience) -> Option<String> {
        assert_eq!(audience, ResourceAudience::HostedModels);
        Some("https://broker.example/custom/instances".into())
    }

    fn authorize<'a>(&'a self, request: AuthorizationRequest<'a>) -> AuthorizationFuture<'a> {
        Box::pin(async move {
            assert_eq!(request.audience, ResourceAudience::HostedModels);
            assert_eq!(request.method, "POST");
            assert_eq!(
                request.url,
                "https://broker.example/custom/instances/images/generations"
            );
            let generation = self.0.load(Ordering::SeqCst);
            if generation == 2 {
                return Err(AuthorizationError::Denied);
            }
            RequestAuthorization::new(
                format!("DPoP token-{generation}"),
                Some(format!("proof-{generation}")),
                std::time::SystemTime::now() + Duration::from_secs(60),
            )
        })
    }
}

#[tokio::test]
async fn instance_client_uses_exact_broker_path_fresh_lease_and_server_attribution() {
    let authorizer = Arc::new(RotatingAuthorizer(AtomicUsize::new(0)));
    let client = HostedImageClient::new("https://stale.example/api/v1", "stale-token")
        .unwrap()
        .with_usage_headers(vec![("x-flow-like-app-id".into(), "caller-app".into())])
        .with_authorizer(authorizer.clone())
        .unwrap();
    for generation in 0..2 {
        authorizer.0.store(generation, Ordering::SeqCst);
        let request = client.build_request(&request()).await.unwrap();
        assert_eq!(
            request.headers()["authorization"],
            format!("DPoP token-{generation}")
        );
        assert_eq!(request.headers()["dpop"], format!("proof-{generation}"));
        assert!(!request.headers().contains_key("x-flow-like-app-id"));
    }
    authorizer.0.store(2, Ordering::SeqCst);
    assert!(client.build_request(&request()).await.is_err());
    assert!(client.bearer.is_empty());
}

#[tokio::test]
async fn decodes_image_bytes_and_rejects_empty_mismatched_or_invalid_results() {
    let (url, server) = serve_once(json_response(response())).await;
    let image = HostedImageClient::new(&url, "token")
        .unwrap()
        .generate(&request())
        .await
        .unwrap();
    assert_eq!(
        image.data[0].decode().unwrap(),
        STANDARD.decode(PNG).unwrap()
    );
    assert_eq!(
        serde_json::from_str::<Value>(&server.await.unwrap()).unwrap()["model"],
        "catalog-image-bit"
    );

    for bad in [
        json!({"model": "other-bit", "data": response()["data"]}),
        json!({"model": "catalog-image-bit", "data": []}),
        json!({"model": "catalog-image-bit", "data": [response()["data"][0], response()["data"][0]]}),
        json!({"model": "catalog-image-bit", "data": [{"b64_json": "bad-base64", "mime_type": "image/png"}]}),
        json!({"model": "catalog-image-bit", "data": [{"b64_json": PNG, "mime_type": "image/jpeg"}]}),
        json!({"error": {"message": "worker failed"}}),
    ] {
        let (url, server) = serve_once(json_response(bad)).await;
        assert!(
            HostedImageClient::new(&url, "token")
                .unwrap()
                .generate(&request())
                .await
                .is_err()
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn redirects_do_not_replay_a_paid_request_or_forward_credentials() {
    let (url, server) = serve_once(
        "HTTP/1.1 307 Temporary Redirect\r\nlocation: http://127.0.0.1:1/leaked\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".into(),
    ).await;
    let error = HostedImageClient::new(&url, "private-token")
        .unwrap()
        .generate(&request())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("307"));
    assert!(!error.to_string().contains("private-token"));
    server.await.unwrap();
}

#[tokio::test]
async fn returns_the_gateway_option_error_without_dumping_other_response_fields() {
    let message = "Requested output format is unsupported by this Cloudflare model";
    let response = json_response(json!({
        "error": {"code": "BAD_REQUEST", "message": message},
        "unrelated": "do-not-echo"
    }))
    .replacen("200 OK", "400 Bad Request", 1);
    let (url, server) = serve_once(response).await;
    let error = HostedImageClient::new(&url, "token")
        .unwrap()
        .generate(&request())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains(message));
    assert!(!error.contains("do-not-echo"));
    server.await.unwrap();
}

#[tokio::test]
async fn html_and_truncated_errors_keep_the_http_status_without_the_response_body() {
    for (status, body, missing_bytes) in [
        (
            "502 Bad Gateway",
            "<html>private-upstream-host private-token</html>",
            0,
        ),
        (
            "503 Service Unavailable",
            r#"{"error":{"message":"private-token""#,
            100,
        ),
    ] {
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len() + missing_bytes
        );
        let (url, server) = serve_once(response).await;
        let error = HostedImageClient::new(&url, "private-token")
            .unwrap()
            .generate(&request())
            .await
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains(status), "{message}");
        assert!(!message.contains("private-token"), "{message}");
        assert!(!message.contains("private-upstream-host"), "{message}");
        assert!(!message.contains("<html>"), "{message}");
        if missing_bytes > 0 {
            assert!(message.contains("interrupted"), "{message}");
        }
        server.await.unwrap();
    }
}

#[test]
fn endpoint_accepts_api_bases_without_embedded_credentials() {
    for base in ["https://api.example", "https://api.example/api/v1/"] {
        assert_eq!(
            endpoint(base, false).unwrap().as_str(),
            "https://api.example/api/v1/images/generations"
        );
    }
    for base in [
        "file:///tmp/images",
        "https://user:key@api.example",
        "https://api.example?key=x",
        "https://api.example#fragment",
    ] {
        assert!(endpoint(base, false).is_err());
    }
}
