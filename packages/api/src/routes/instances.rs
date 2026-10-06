use crate::{devices, error::ApiError, instances, state::AppState};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, HeaderValue},
    middleware::{Next, from_fn},
    response::Response,
    routing::{get, post},
};
use flow_like_device_protocol::{
    InstanceReceipt, InstanceTokenRequest, InstanceTokenResponse, ReceiptRequest,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{id}/token", post(token))
        .route("/{id}/project-token", post(project_token))
        .route("/{id}/receipt", post(receipt))
        .route("/project/app", get(project_app))
        .route("/project/storage", post(storage))
        .route(
            "/project/schedules",
            post(schedules).layer(DefaultBodyLimit::max(
                instances::schedules::CLAIM_BODY_LIMIT_BYTES,
            )),
        )
        .route(
            "/project/offline/replay",
            post(offline_replay).layer(DefaultBodyLimit::max(
                flow_like_device_protocol::MAX_OFFLINE_REPLAY_HTTP_BYTES,
            )),
        )
        .route(
            "/project/boards/{board}/versions/{major}/{minor}/{patch}/pages/{page}",
            get(project_page),
        )
        .route(
            "/project/{kind}/{id}/versions/{major}/{minor}/{patch}",
            get(artifact),
        )
        .route(
            "/chat/completions",
            post(crate::routes::chat::completions::invoke_instance_llm),
        )
        .route(
            "/responses",
            post(crate::routes::chat::responses::invoke_instance_responses),
        )
        .route(
            "/systemone",
            post(crate::routes::chat::systemone::invoke_instance_systemone),
        )
        .route(
            "/embeddings/embed",
            post(crate::routes::embeddings::embed::embed_instance_text),
        )
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(from_fn(no_store))
}

/// Takes the raw request so the device proof is verified before the body is buffered.
async fn offline_replay(
    State(state): State<AppState>,
    request: Request,
) -> Result<Json<flow_like_device_protocol::OfflineReplayResponse>, ApiError> {
    Ok(Json(instances::offline::replay(&state, request).await?))
}

async fn project_token(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<InstanceTokenRequest>,
) -> Result<Json<InstanceTokenResponse>, ApiError> {
    Ok(Json(
        instances::project::token(&devices::context(&state), &id, request).await?,
    ))
}
async fn storage(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<flow_like_device_protocol::InstanceStorageLease>, ApiError> {
    Ok(Json(instances::project::storage(&state, &headers).await?))
}
async fn schedules(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<instances::schedules::ScheduleClaimResponse>, ApiError> {
    Ok(Json(
        instances::schedules::claim(&state, &headers, &body).await?,
    ))
}
async fn artifact(
    State(state): State<AppState>,
    Path((kind, id, major, minor, patch)): Path<(String, String, u32, u32, u32)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        instances::project::artifact(&state, &headers, &kind, &id, (major, minor, patch)).await?,
    ))
}
async fn project_app(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(instances::project::app(&state, &headers).await?))
}

async fn project_page(
    State(state): State<AppState>,
    Path((board, major, minor, patch, page)): Path<(String, u32, u32, u32, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        instances::project::page(&state, &headers, &board, &page, (major, minor, patch)).await?,
    ))
}

async fn no_store(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("pragma", HeaderValue::from_static("no-cache"));
    response
}
async fn token(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<InstanceTokenRequest>,
) -> Result<Json<InstanceTokenResponse>, ApiError> {
    Ok(Json(
        instances::token(&devices::context(&state), &id, request).await?,
    ))
}
async fn receipt(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ReceiptRequest>,
) -> Result<Json<InstanceReceipt>, ApiError> {
    Ok(Json(
        instances::receipt(&devices::context(&state), &id, request).await?,
    ))
}
