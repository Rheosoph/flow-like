pub mod context;
pub mod offline_replay;
pub mod presign;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use flow_like_device_protocol::MAX_OFFLINE_REPLAY_HTTP_BYTES;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/presign", get(presign::presign))
        .route("/context", get(context::execution_context))
        .route(
            "/offline/replay",
            post(offline_replay::offline_replay).layer(DefaultBodyLimit::max(
                offline_replay::desktop_limits()
                    .max_request_bytes
                    .unwrap_or(MAX_OFFLINE_REPLAY_HTTP_BYTES),
            )),
        )
        .route(
            "/offline/capabilities",
            get(offline_replay::offline_capabilities),
        )
}
