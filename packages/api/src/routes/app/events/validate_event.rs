use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use utoipa::ToSchema;

use super::db::validate_event_schedule;

#[derive(Deserialize, Debug, ToSchema)]
pub struct VersionQuery {
    /// expected format: "MAJOR_MINOR_PATCH", e.g. "1_0_3"
    pub version: Option<String>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/validate",
    tag = "events",
    description = "Validate an event configuration.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("version" = Option<String>, Query, description = "Version in MAJOR_MINOR_PATCH format")
    ),
    responses(
        (status = 200, description = "Validation succeeded", body = ()),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/events/{event_id}/validate",
    skip(state, user, query)
)]
pub async fn validate_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<VersionQuery>,
) -> Result<Json<()>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;

    let version_opt =
        match query.version.as_deref() {
            Some(ver_str) => Some(super::parse_version_tuple(ver_str).ok_or_else(|| {
                ApiError::bad_request("version must be in MAJOR_MINOR_PATCH format")
            })?),
            None => None,
        };

    let app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    let event = app.get_event(&event_id, version_opt).await?;
    event.validate_event_references(&app).await?;

    validate_event_schedule(&state, &event)
        .await
        .map_err(|error| match error {
            flow_like_sinks::SchedulerError::InvalidCronExpression(message) => {
                ApiError::bad_request(message)
            }
            other => ApiError::service_unavailable(format!(
                "Failed to validate cron schedule: {}",
                other
            )),
        })?;

    Ok(Json(()))
}
