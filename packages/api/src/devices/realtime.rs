//! A device's doorbell: receive-only credentials for wakeups that tell it to poll its routes now.
//! A wakeup carries no data. Everything the device acts on still comes from its own authenticated
//! requests, so a forged wakeup costs it at most an early poll.

use crate::{error::ApiError, routes::realtime::issue_realtime_grant, state::AppState};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::post,
};
use flow_like_types::realtime::{
    RealtimeConnectRequest, RealtimeConnectResponse, RealtimeTransportPreference,
};

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/{id}/realtime/connect", post(connect))
}

#[utoipa::path(
    post,
    path = "/devices/{id}/realtime/connect",
    tag = "devices",
    description = "A device asks for receive-only credentials to the channel that tells it when to poll its routes again. Wakeups carry no data.",
    params(("id" = String, Path, description = "Device ID")),
    request_body(content = Object, description = "RealtimeConnectRequest: transport is configured (default) or http."),
    responses(
        (status = 200, description = "Receive-only grant for this device's wakeups with its expiry. HTTP when the deployment has no push channel or its provider cannot issue one right now", body = Object),
        (status = 401, description = "The device session or request proof is invalid or revoked"),
        (status = 403, description = "The session belongs to another device"),
        (status = 503, description = "Device enrollment is not enabled on this hub")
    )
)]
#[tracing::instrument(name = "POST /devices/{id}/realtime/connect", skip_all)]
pub(crate) async fn connect(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<RealtimeConnectRequest>,
) -> Result<(HeaderMap, Json<RealtimeConnectResponse>), ApiError> {
    super::device_principal(
        &super::context(&state),
        &id,
        &headers,
        "POST",
        &format!("/devices/{id}/realtime/connect"),
        false,
    )
    .await?;
    let prefer_http = request.transport == RealtimeTransportPreference::Http;
    let channels = &state.channels;
    issue_realtime_grant(channels.transport(), prefer_http, |http| {
        channels.realtime_connect_device(&id, http)
    })
    .await
}
