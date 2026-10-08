use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{MethodRouter, post},
};

use crate::state::AppState;

pub mod embed;
mod providers;

// The gateway accepts 256 MiB of media. Base64 expands the JSON envelope by 4/3.
pub(crate) const MAX_EMBEDDING_BODY_BYTES: usize = 256 * 1024 * 1024 * 4 / 3 + 1024 * 1024;

pub fn routes() -> Router<AppState> {
    embedding_routes(post(embed::embed_text))
}

fn embedding_routes<S: Clone + Send + Sync + 'static>(handler: MethodRouter<S>) -> Router<S> {
    Router::new()
        .route(
            "/embed",
            handler.layer(DefaultBodyLimit::max(MAX_EMBEDDING_BODY_BYTES)),
        )
        .route_layer(axum::middleware::from_fn(require_embedding_principal))
}

async fn require_embedding_principal(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use crate::{error::ApiError, middleware::jwt::AppUser};
    use axum::response::IntoResponse;

    // JWT middleware permits anonymous catalog reads. Reject anonymous media
    // submissions before buffering the larger embedding request envelope.
    match request.extensions().get::<AppUser>() {
        None | Some(AppUser::Unauthorized) => ApiError::UNAUTHORIZED.into_response(),
        Some(_) => next.run(request).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json,
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    fn authorized_request(body: impl Into<Body>) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri("/embed")
            .header("content-type", "application/json")
            .body(body.into())
            .unwrap();
        request
            .extensions_mut()
            .insert(crate::middleware::jwt::AppUser::OpenID(
                crate::middleware::jwt::OpenIDUser {
                    sub: "test-user".into(),
                    access_token: "test-token".into(),
                },
            ));
        request
    }

    async fn extract_embedding(Json(payload): Json<embed::EmbedRequest>) -> StatusCode {
        assert_eq!(payload.input.len(), 1);
        StatusCode::NO_CONTENT
    }

    #[tokio::test]
    async fn media_route_overrides_the_default_two_mebibyte_json_limit() {
        let json = serde_json::to_vec(&serde_json::json!({
            "model": "gemma2-bit", "input": {"type": "image", "image": "A".repeat(3 * 1024 * 1024)}
        }))
        .unwrap();
        let request = || authorized_request(json.clone());
        let default = Router::new()
            .route("/embed", post(extract_embedding))
            .oneshot(request())
            .await
            .unwrap();
        assert_eq!(default.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let media = embedding_routes(post(extract_embedding))
            .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
            .oneshot(request())
            .await
            .unwrap();
        assert_eq!(media.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn anonymous_media_requests_are_rejected_before_reading_the_body() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        for principal in [None, Some(crate::middleware::jwt::AppUser::Unauthorized)] {
            let polled = Arc::new(AtomicBool::new(false));
            let flag = polled.clone();
            let body = Body::from_stream(futures::stream::once(async move {
                flag.store(true, Ordering::SeqCst);
                Ok::<_, std::convert::Infallible>(axum::body::Bytes::from_static(b"{}"))
            }));
            let mut request = Request::builder()
                .method("POST")
                .uri("/embed")
                .header("content-type", "application/json")
                .body(body)
                .unwrap();
            if let Some(principal) = principal {
                request.extensions_mut().insert(principal);
            }
            let response = embedding_routes(post(extract_embedding))
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert!(!polled.load(Ordering::SeqCst));
        }
    }
}
