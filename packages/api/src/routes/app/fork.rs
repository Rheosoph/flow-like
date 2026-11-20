use crate::state::AppState;
use axum::{
    Router,
    routing::{get, post},
};

pub mod begin_offline;
pub mod begin_online;
pub mod finalize_online;
pub mod jobs;
pub mod offline_files;
pub mod online_fork;
pub mod preview;
pub mod upload_online;

/// Per-app fork endpoints. Mounted under `/apps/{app_id}/fork`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/preview", get(preview::get_fork_preview))
        .route(
            "/offline/begin",
            post(begin_offline::begin_offline_fork).route_layer(axum::middleware::from_fn(
                super::board::capabilities::negotiate_board_format,
            )),
        )
        .route("/offline/files", get(offline_files::offline_fork_files))
        .route("/online/upload-urls", post(upload_online::upload_urls))
        .route("/online/adopt", post(upload_online::adopt_upload))
        .route(
            "/online/finalize",
            post(finalize_online::finalize_online_fork),
        )
}

/// App-root fork endpoints. Mounted at `/apps/...` (sibling to the
/// `/apps/{app_id}/...` nest) for actions that don't have a server-side
/// source app — i.e. offline → online forks where the source lives on
/// the desktop.
pub fn root_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/fork/online/begin",
            post(begin_online::begin_online_fork).layer(axum::extract::DefaultBodyLimit::max(
                begin_online::BEGIN_BODY_LIMIT,
            )),
        )
        .route("/fork/jobs/{job_id}", get(jobs::get_fork_job))
}
