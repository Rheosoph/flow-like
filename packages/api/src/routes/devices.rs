use crate::{
    devices::{
        self,
        view::{DeviceEnrollmentView, DeviceUsageView, DeviceView, EnrollmentFilter},
    },
    error::ApiError,
    instances,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{Next, from_fn},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use flow_like_device_protocol::*;
use serde::Deserialize;
use std::result::Result;
use utoipa::{IntoParams, ToSchema};

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
        .route("/usage", get(usage))
        .route("/enrollments", get(list_enrollments).post(create))
        .route("/enrollments/{id}", delete(cancel))
        .route("/enrollments/{id}/challenge", post(challenge))
        .route("/enrollments/{id}/redeem", post(redeem))
        .route("/token", post(token))
        .route("/{id}", get(status).delete(revoke).patch(rename))
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

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EnrollmentQuery {
    /// `open` (default) lists setup packages waiting to be started, including ones that
    /// expired unused; `recent` also lists cancelled ones.
    #[serde(default)]
    #[param(inline)]
    state: EnrollmentFilter,
}

#[utoipa::path(
    get,
    path = "/devices/enrollments",
    tag = "devices",
    description = "Lists your setup packages that have not been started on a device yet.",
    params(EnrollmentQuery),
    responses(
        (status = 200, description = "Setup packages, newest first, at most 200", body = [DeviceEnrollmentView]),
        (status = 400, description = "Unknown state filter"),
        (status = 401, description = "Sign-in required"),
        (status = 403, description = "The account or access token cannot manage devices"),
        (status = 503, description = "Device enrollment is not enabled on this hub")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn list_enrollments(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<EnrollmentQuery>,
) -> Result<Json<Vec<DeviceEnrollmentView>>, ApiError> {
    let context = devices::context(&state);
    devices::enabled(&context)?;
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        devices::view::enrollments(&context, &owner, query.state).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/devices",
    tag = "devices",
    description = "Lists the devices you own, devices shared with you, and devices you approved cloud access for.",
    responses(
        (status = 200, description = "Devices you may see, your own first", body = [DeviceView]),
        (status = 401, description = "Sign-in required"),
        (status = 403, description = "The account or access token cannot manage devices"),
        (status = 503, description = "Device enrollment is not enabled on this hub")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<DeviceView>>, ApiError> {
    let context = devices::context(&state);
    devices::enabled(&context)?;
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(devices::view::list(&context, &owner).await?))
}

#[utoipa::path(
    get,
    path = "/devices/usage",
    tag = "devices",
    description = "Shows how many devices, setup packages and account backups this hub allows you and how many you use.",
    responses(
        (status = 200, description = "Your limits, what you use of them and the hub's current time", body = DeviceUsageView),
        (status = 401, description = "Sign-in required"),
        (status = 403, description = "The account or access token cannot manage devices"),
        (status = 503, description = "Device enrollment is not enabled on this hub")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn usage(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<DeviceUsageView>, ApiError> {
    let context = devices::context(&state);
    devices::enabled(&context)?;
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(devices::view::usage(&context, &owner).await?))
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

/// `null` removes the name; a body without the key is malformed, never a removal.
fn stated<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Option::deserialize(deserializer)
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RenameDeviceRequest {
    /// The label to show instead of the setup name: 1 to 64 characters, no control
    /// characters. `null` removes it.
    #[serde(deserialize_with = "stated")]
    #[schema(required = true)]
    display_name: Option<String>,
}

#[utoipa::path(
    patch,
    path = "/devices/{id}",
    tag = "devices",
    description = "Gives one of your devices a display name, or removes it. The name chosen at setup stays unchanged.",
    params(("id" = String, Path, description = "Device ID")),
    request_body = RenameDeviceRequest,
    responses(
        (status = 200, description = "The device with its new display name", body = DeviceView),
        (status = 400, description = "The display name is empty, too long or contains control characters"),
        (status = 401, description = "Sign-in required"),
        (status = 403, description = "The account or access token cannot manage devices"),
        (status = 404, description = "No active device of yours has this ID"),
        (status = 422, description = "The request does not state a display name"),
        (status = 503, description = "Device enrollment is not enabled on this hub")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn rename(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(request): Json<RenameDeviceRequest>,
) -> Result<Json<DeviceView>, ApiError> {
    let context = devices::context(&state);
    devices::enabled(&context)?;
    let owner = devices::human_owner(&state, &user).await?;
    Ok(Json(
        devices::rename(&context, &owner, &id, request.display_name.as_deref()).await?,
    ))
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

#[utoipa::path(
    get,
    path = "/devices/{id}",
    tag = "devices",
    description = "Shows one device you own, have shared access to, or approved cloud access for.",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, description = "The device as you may see it", body = DeviceView),
        (status = 401, description = "Sign-in required"),
        (status = 403, description = "The account or access token cannot manage devices"),
        (status = 404, description = "No such device, or you may not see it"),
        (status = 503, description = "Device enrollment is not enabled on this hub")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
async fn status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let context = devices::context(&state);
    devices::enabled(&context)?;
    if matches!(user, AppUser::OpenID(_) | AppUser::PAT(_)) {
        let owner = devices::human_owner(&state, &user).await?;
        return Ok(Json(devices::view::get(&context, &owner, &id).await?).into_response());
    }
    // Agents parse this response strictly, so a device proof is answered with the bare
    // registration status and never with the view.
    Ok(Json(
        devices::device_principal(
            &context,
            &id,
            &headers,
            "GET",
            &format!("/devices/{id}"),
            false,
        )
        .await?
        .status,
    )
    .into_response())
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
    use super::*;

    /// Two merged routers claiming the same path and method panic here instead of when
    /// the server starts.
    #[test]
    fn merged_device_routers_do_not_overlap() {
        let _ = routes();
    }

    #[test]
    fn a_rename_states_the_display_name_explicitly() {
        let parse = |body: &str| serde_json::from_str::<RenameDeviceRequest>(body);
        assert_eq!(
            parse(r#"{"display_name":"Lab GPU"}"#).unwrap().display_name,
            Some("Lab GPU".into())
        );
        assert_eq!(
            parse(r#"{"display_name":null}"#).unwrap().display_name,
            None
        );
        assert!(parse("{}").is_err());
        assert!(parse(r#"{"display_name":"Lab GPU","name":"lab-gpu-02"}"#).is_err());
        assert!(parse(r#"{"display_name":7}"#).is_err());
    }

    #[test]
    fn enrollment_lists_default_to_open_packages() {
        let parse = |query: &str| {
            let uri = format!("/devices/enrollments{query}").parse().unwrap();
            Query::<EnrollmentQuery>::try_from_uri(&uri).map(|Query(query)| query.state)
        };
        assert_eq!(parse("").unwrap(), EnrollmentFilter::Open);
        assert_eq!(parse("?state=open").unwrap(), EnrollmentFilter::Open);
        assert_eq!(parse("?state=recent").unwrap(), EnrollmentFilter::Recent);
        assert!(parse("?state=consumed").is_err());
    }
}
