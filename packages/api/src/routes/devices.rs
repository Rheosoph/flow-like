use crate::{devices, error::ApiError, instances, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{Next, from_fn},
    response::Response,
    routing::{delete, get, post},
};
use flow_like_device_protocol::*;
use std::result::Result;

pub fn routes() -> Router<AppState> {
    Router::new()
        .merge(devices::management::routes())
        .merge(devices::archives::routes())
        .merge(devices::certificates::routes())
        .merge(devices::inventory::routes())
        .merge(devices::fleet::routes())
        .merge(instances::routes::routes())
        .route("/", get(list))
        .route("/setup", get(devices::readiness::get))
        .route("/enrollments", post(create))
        .route("/enrollments/{id}", delete(cancel))
        .route("/enrollments/{id}/challenge", post(challenge))
        .route("/enrollments/{id}/redeem", post(redeem))
        .route("/token", post(token))
        .route("/{id}", get(status).delete(revoke))
        .route("/{id}/heartbeat", post(heartbeat))
        .route("/{id}/receipt", post(receipt))
        .layer(DefaultBodyLimit::max(96 * 1024))
        .layer(from_fn(no_store))
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

async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<CreateEnrollmentRequest>,
) -> Result<Json<CreateEnrollmentResponse>, ApiError> {
    devices::enabled(&devices::context(&state))?;
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        devices::create_enrollment(&devices::context(&state), &owner, body).await?,
    ))
}

async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<DeviceStatus>>, ApiError> {
    devices::enabled(&devices::context(&state))?;
    let owner = devices::human_owner(&state, &user).await?;
    let mut devices = devices::repository(&devices::context(&state))
        .list(&owner)
        .await?;
    devices.extend(devices::management::shared_devices(&state, &owner).await?);
    for retained in instances::consent_devices(&devices::context(&state), &owner).await? {
        if !devices
            .iter()
            .any(|device| device.device_id == retained.device_id)
        {
            devices.push(retained);
        }
    }
    Ok(Json(devices))
}

async fn cancel(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    devices::enabled(&devices::context(&state))?;
    let owner = devices::human_owner(&state, &user).await?;
    devices::repository(&devices::context(&state))
        .cancel_enrollment(&owner, &id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    devices::enabled(&devices::context(&state))?;
    let owner = devices::human_owner(&state, &user).await?;
    devices::repository(&devices::context(&state))
        .revoke(&owner, &id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn challenge(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ChallengeRequest>,
) -> Result<Json<EnrollmentChallenge>, ApiError> {
    Ok(Json(
        devices::challenge(&devices::context(&state), &id, body).await?,
    ))
}

async fn redeem(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RedeemEnrollmentRequest>,
) -> Result<Json<DeviceReceipt>, ApiError> {
    Ok(Json(
        devices::redeem(&devices::context(&state), &id, body).await?,
    ))
}

async fn token(
    State(state): State<AppState>,
    Json(body): Json<DeviceTokenRequest>,
) -> Result<Json<DeviceTokenResponse>, ApiError> {
    Ok(Json(devices::token(&devices::context(&state), body).await?))
}

async fn receipt(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ReceiptRequest>,
) -> Result<Json<DeviceReceipt>, ApiError> {
    Ok(Json(
        devices::recover_receipt(&devices::context(&state), &id, body).await?,
    ))
}

async fn status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<DeviceStatus>, ApiError> {
    devices::enabled(&devices::context(&state))?;
    if matches!(user, AppUser::OpenID(_) | AppUser::PAT(_)) {
        let owner = devices::human_owner(&state, &user).await?;
        let current = devices::repository(&devices::context(&state))
            .device(&id)
            .await?;
        if current.status.owner_id != owner {
            return Ok(Json(
                devices::management::admitted_device(&state, &owner, &id)
                    .await?
                    .0
                    .status,
            ));
        }
        return Ok(Json(current.status));
    }
    Ok(Json(
        devices::device_principal(
            &devices::context(&state),
            &id,
            &headers,
            "GET",
            &format!("/devices/{id}"),
            false,
        )
        .await?
        .status,
    ))
}

async fn heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<DeviceHeartbeat>,
) -> Result<Json<DeviceStatus>, ApiError> {
    if body.version != PROTOCOL_VERSION
        || body.boot_id.is_empty()
        || body.boot_id.len() > 128
        || body.agent_version.is_empty()
        || body.agent_version.len() > 128
    {
        return Err(ApiError::bad_request("Invalid device heartbeat"));
    }
    // Only reachability is stored here. Detailed workload and host telemetry
    // belongs in the end-to-end encrypted management stream.
    Ok(Json(
        devices::device_principal(
            &devices::context(&state),
            &id,
            &headers,
            "POST",
            &format!("/devices/{id}/heartbeat"),
            true,
        )
        .await?
        .status,
    ))
}

#[cfg(test)]
mod tests {
    /// Two merged routers claiming one path and method, or naming a path parameter
    /// differently, panic here instead of when the server starts.
    #[test]
    fn merged_device_routers_do_not_overlap() {
        let _ = super::routes();
    }
}
