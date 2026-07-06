use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_connection},
    state::AppState,
};
use axum::{
    Extension,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::lancegraph;

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/graph/{overlay_id}",
    tag = "graph",
    description = "Delete a graph overlay (metadata only, does not drop underlying tables).",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    responses(
        (status = 200, description = "Overlay deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Overlay not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/graph/{overlay_id}", skip(state, user))]
pub async fn delete_overlay(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
) -> Result<(), ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    lancegraph::delete_overlay(&connection, &overlay_id).await?;

    Ok(())
}
