pub mod context;
pub mod offline_replay;
pub mod presign;

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    middleware::{Next, from_fn},
    response::{IntoResponse, Response},
    routing::{MethodRouter, get, post},
};
use flow_like_device_protocol::MAX_OFFLINE_REPLAY_HTTP_BYTES;

use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/presign", get(presign::presign))
        .route("/context", get(context::execution_context))
        .route(
            "/offline/replay",
            replay_route(post(offline_replay::offline_replay)),
        )
        .route(
            "/offline/capabilities",
            get(offline_replay::offline_capabilities),
        )
}

fn replay_route<S: Clone + Send + Sync + 'static>(handler: MethodRouter<S>) -> MethodRouter<S> {
    handler
        .layer(DefaultBodyLimit::max(
            offline_replay::desktop_limits()
                .max_request_bytes
                .unwrap_or(MAX_OFFLINE_REPLAY_HTTP_BYTES),
        ))
        .layer(from_fn(reject_anonymous))
}

/// The auth middleware passes anonymous requests through. Reject them before the
/// handler buffers and parses a body of up to the offline replay limit.
async fn reject_anonymous(request: Request, next: Next) -> Response {
    match request.extensions().get::<AppUser>() {
        None | Some(AppUser::Unauthorized) => ApiError::UNAUTHORIZED.into_response(),
        Some(_) => next.run(request).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::OpenIDUser;
    use axum::{
        Json,
        body::{Body, Bytes},
        http::StatusCode,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tower::ServiceExt;

    async fn accept(Json(_): Json<serde_json::Value>) -> StatusCode {
        StatusCode::NO_CONTENT
    }

    fn replay(user: Option<AppUser>, body: Body) -> axum::http::Request<Body> {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri("/offline/replay")
            .header("content-type", "application/json")
            .body(body)
            .unwrap();
        if let Some(user) = user {
            request.extensions_mut().insert(user);
        }
        request
    }

    fn router() -> Router {
        Router::new().route("/offline/replay", replay_route(post(accept)))
    }

    #[tokio::test]
    async fn anonymous_replays_are_rejected_before_the_body_is_read() {
        for user in [None, Some(AppUser::Unauthorized)] {
            let polled = Arc::new(AtomicBool::new(false));
            let flag = polled.clone();
            let body = Body::from_stream(futures::stream::once(async move {
                flag.store(true, Ordering::SeqCst);
                Ok::<_, std::convert::Infallible>(Bytes::from_static(b"[0,0,0]"))
            }));
            let response = router().oneshot(replay(user, body)).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert!(!polled.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn signed_in_replays_keep_the_offline_body_limit() {
        let user = || {
            Some(AppUser::OpenID(OpenIDUser {
                sub: "account".into(),
                access_token: "token".into(),
            }))
        };
        let accepted = router()
            .oneshot(replay(user(), Body::from("{}")))
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::NO_CONTENT);
        let limit = offline_replay::desktop_limits()
            .max_request_bytes
            .unwrap_or(MAX_OFFLINE_REPLAY_HTTP_BYTES);
        let oversized = router()
            .oneshot(replay(user(), Body::from(vec![b' '; limit + 1])))
            .await
            .unwrap();
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
