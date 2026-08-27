use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_write_connection, validate_table_name},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct OptimizePayload {
    /// Retain every table version. Set this to false to prune versions older than seven days
    /// after compaction and index maintenance finish.
    #[serde(default = "default_keep_versions")]
    pub keep_versions: bool,
}

fn default_keep_versions() -> bool {
    true
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/{table}/optimize",
    tag = "database",
    description = "Compact table storage and update indices. Version history is retained by default; optional cleanup prunes versions older than seven days.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = OptimizePayload,
    responses(
        (status = 200, description = "Table optimized", body = ()),
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
    name = "POST /apps/{app_id}/db/{table}/optimize",
    skip(state, user, scope, payload)
)]
pub async fn optimize_table(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<OptimizePayload>,
) -> Result<Json<()>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;

    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection(connection, table).await;

    db.optimize(payload.keep_versions).await?;

    Ok(Json(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_keep_versions_defaults_to_retention() {
        let omitted: OptimizePayload =
            serde_json::from_value(serde_json::json!({})).expect("payload should deserialize");
        assert!(omitted.keep_versions);

        let explicit_cleanup: OptimizePayload =
            serde_json::from_value(serde_json::json!({ "keep_versions": false }))
                .expect("payload should deserialize");
        assert!(!explicit_cleanup.keep_versions);
    }
}
