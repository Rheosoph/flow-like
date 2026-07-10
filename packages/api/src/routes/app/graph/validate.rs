use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_connection},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::lancegraph;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ValidationResult {
    pub ok: bool,
    pub issues: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{overlay_id}/validate",
    tag = "graph",
    description = "Validate a graph overlay definition (checks tables exist, columns exist, types are OK).",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    responses(
        (status = 200, description = "Validation result", body = ValidationResult),
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
#[tracing::instrument(
    name = "POST /apps/{app_id}/graph/{overlay_id}/validate",
    skip(state, user)
)]
pub async fn validate_overlay(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<ValidationResult>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let overlay = lancegraph::load_overlay(&connection, &overlay_id).await?;

    let validation = match lancegraph::LanceGraphStore::new(connection, overlay, None).await {
        Ok(_) => ValidationResult {
            ok: true,
            issues: Vec::new(),
        },
        Err(error) => ValidationResult {
            ok: false,
            issues: vec![error.to_string()],
        },
    };

    Ok(Json(validation))
}
