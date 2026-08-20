use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};

/// Available event versions, each as (major, minor, patch).
pub type EventVersionList = Vec<(u32, u32, u32)>;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/versions",
    tag = "events",
    description = "List available versions for an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Event versions", body = Vec<(u32, u32, u32)>),
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
    name = "GET /apps/{app_id}/events/{event_id}/versions",
    skip(state, user)
)]
pub async fn get_event_versions(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
) -> Result<Json<EventVersionList>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);
    let sub = permission.sub()?;

    let app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    let versions = app.get_event_versions(&event_id).await?;

    Ok(Json(versions))
}
