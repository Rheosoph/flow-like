use axum::{
    body::{Body, to_bytes},
    extract::Request,
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rmcp::model::{ClientJsonRpcMessage, ClientRequest};

const MAX_INITIALIZE_BODY_BYTES: usize = 4 * 1024 * 1024;

pub(super) async fn validate_session_start(request: Request, next: Next) -> Response {
    let has_session = request
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .is_some();
    if request.method() != Method::POST || has_session {
        return next.run(request).await;
    }

    // RMCP 1.7 allocates a session before rejecting a non-initialize message and
    // leaves that entry in its table. Validate before handing it the request.
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_INITIALIZE_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let Ok(ClientJsonRpcMessage::Request(request)) =
        serde_json::from_slice::<ClientJsonRpcMessage>(&body)
    else {
        return (
            StatusCode::BAD_REQUEST,
            "Expected an MCP initialize request",
        )
            .into_response();
    };
    let ClientRequest::InitializeRequest(initialize) = request.request else {
        return (
            StatusCode::BAD_REQUEST,
            "Expected an MCP initialize request",
        )
            .into_response();
    };
    if let Some(version) = parts.headers.get("mcp-protocol-version")
        && version.to_str().ok() != Some(initialize.params.protocol_version.as_str())
    {
        return (
            StatusCode::BAD_REQUEST,
            "MCP protocol versions do not match",
        )
            .into_response();
    }
    next.run(Request::from_parts(parts, Body::from(body))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use rmcp::{
        ServerHandler,
        transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    };
    use std::{sync::Arc, time::Duration};
    use tower::ServiceExt;

    #[derive(Clone)]
    struct TestServer;
    impl ServerHandler for TestServer {}

    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"security-test","version":"1"}}}"#;

    fn request(body: &str) -> Request {
        Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header("host", "127.0.0.1")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(Body::from(body.to_owned()))
            .unwrap()
    }

    #[tokio::test]
    async fn invalid_session_starts_do_not_allocate_and_valid_initialize_still_works() {
        let sessions = Arc::new(LocalSessionManager::default());
        let config = StreamableHttpServerConfig::default();
        let cancellation = config.cancellation_token.clone();
        let service = StreamableHttpService::new(|| Ok(TestServer), sessions.clone(), config);
        let router = axum::Router::new()
            .nest_service("/mcp", service)
            .layer(axum::middleware::from_fn(validate_session_start));

        for body in [
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "invalid json",
        ] {
            for invalid_session_header in [false, true] {
                let mut request = request(body);
                if invalid_session_header {
                    request
                        .headers_mut()
                        .insert("mcp-session-id", HeaderValue::from_bytes(&[0xff]).unwrap());
                }
                let response = router.clone().oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::BAD_REQUEST);
                assert!(sessions.sessions.read().await.is_empty());
            }
        }

        let oversized = " ".repeat(MAX_INITIALIZE_BODY_BYTES + 1);
        let response = router.clone().oneshot(request(&oversized)).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(sessions.sessions.read().await.is_empty());

        let mut interrupted = request("");
        *interrupted.body_mut() = Body::from_stream(futures::stream::pending::<
            Result<axum::body::Bytes, std::io::Error>,
        >());
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                router.clone().oneshot(interrupted)
            )
            .await
            .is_err()
        );
        assert!(sessions.sessions.read().await.is_empty());

        let mut mismatched_version = request(INITIALIZE);
        mismatched_version.headers_mut().insert(
            "mcp-protocol-version",
            HeaderValue::from_static("2024-11-05"),
        );
        assert_eq!(
            router
                .clone()
                .oneshot(mismatched_version)
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(sessions.sessions.read().await.is_empty());

        let mut wrong_content_type = request(INITIALIZE);
        wrong_content_type
            .headers_mut()
            .insert("content-type", HeaderValue::from_static("text/plain"));
        assert!(
            router
                .clone()
                .oneshot(wrong_content_type)
                .await
                .unwrap()
                .status()
                .is_client_error()
        );
        assert!(sessions.sessions.read().await.is_empty());

        let response = router.clone().oneshot(request(INITIALIZE)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let session_id = response.headers().get("mcp-session-id").unwrap().clone();
        assert_eq!(sessions.sessions.read().await.len(), 1);
        drop(response);

        let delete = Request::builder()
            .method(Method::DELETE)
            .uri("/mcp")
            .header("host", "127.0.0.1")
            .header("mcp-session-id", session_id)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router.oneshot(delete).await.unwrap().status(),
            StatusCode::ACCEPTED
        );
        assert!(sessions.sessions.read().await.is_empty());
        cancellation.cancel();
    }

    #[tokio::test]
    async fn abandoned_valid_initialize_is_removed_by_the_session_worker() {
        let mut manager = LocalSessionManager::default();
        manager.session_config.keep_alive = Some(Duration::from_millis(20));
        let sessions = Arc::new(manager);
        let service = StreamableHttpService::new(
            || Ok(TestServer),
            sessions.clone(),
            StreamableHttpServerConfig::default(),
        );
        let router = axum::Router::new()
            .nest_service("/mcp", service)
            .layer(axum::middleware::from_fn(validate_session_start));
        let response = router.oneshot(request(INITIALIZE)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        drop(response);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !sessions.sessions.read().await.is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("abandoned initialize must not leave a permanent session entry");
    }
}
