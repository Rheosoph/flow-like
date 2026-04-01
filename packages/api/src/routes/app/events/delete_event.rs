use crate::{
    audit_branch, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::anyhow;

use super::db::delete_event_with_sink;

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/events/{event_id}",
    tag = "events",
    description = "Delete an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Event deleted", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/events/{event_id}", skip(state, user))]
pub async fn delete_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;

    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    // Try to delete from bucket, but don't fail if file doesn't exist
    // The event might only exist in the database (e.g., if bucket sync failed)
    if let Err(e) = app.delete_event(&event_id).await {
        tracing::warn!("Failed to delete event from bucket (may not exist): {}", e);
    }

    // Always delete from database and external schedulers
    delete_event_with_sink(&state.db, &state, &event_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete event from database: {}", e);
            ApiError::internal_error(anyhow!(e))
        })?;

    audit_branch!(
        state,
        user,
        app_id,
        "event.delete",
        "Event",
        event_id,
        "Event deleted"
    );
    Ok(Json(()))
}
