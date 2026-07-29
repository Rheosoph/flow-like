use crate::{
    ensure_any_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::db::ScopeParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::graph::lancegraph;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MappingValidation {
    pub kind: String,
    pub label: String,
    pub ok: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ValidationResult {
    pub ok: bool,
    pub issues: Vec<String>,
    pub mappings: Vec<MappingValidation>,
}

impl From<lancegraph::ValidationReport> for ValidationResult {
    fn from(report: lancegraph::ValidationReport) -> Self {
        ValidationResult {
            ok: report.ok,
            issues: report.issues,
            mappings: report
                .mappings
                .into_iter()
                .map(|mapping| MappingValidation {
                    kind: mapping.kind,
                    label: mapping.label,
                    ok: mapping.ok,
                    issues: mapping.issues,
                })
                .collect(),
        }
    }
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/graph/{overlay_id}/validate",
    tag = "graph",
    description = "Validate a graph overlay against the live database: tables and columns must exist, labels must be unique and queryable, and links must reference declared object types. Send a draft overlay in the body to validate unsaved changes.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("overlay_id" = String, Path, description = "Overlay ID (ignored when a draft body is sent)"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body(content = Object, description = "Optional draft overlay to validate before saving"),
    responses(
        (status = 200, description = "Per-mapping validation result", body = ValidationResult),
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
    skip(state, user, scope, draft)
)]
pub async fn validate_overlay(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, overlay_id)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    draft: Option<Json<lancegraph::GraphOverlayDef>>,
) -> Result<Json<ValidationResult>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let (connection, overlay) = match draft {
        // Validate an unsaved draft against the scoped database.
        Some(Json(draft)) => {
            let connection =
                crate::routes::app::db::resolve_connection(&state, &user, &app_id, &scope).await?;
            (connection, draft)
        }
        None => super::load_scoped_overlay(&state, &user, &app_id, &overlay_id, &scope).await?,
    };

    let report = lancegraph::validate_overlay_definition(&connection, &overlay)
        .await
        .map_err(|error| ApiError::internal(format!("Overlay validation failed: {error}")))?;

    Ok(Json(report.into()))
}
