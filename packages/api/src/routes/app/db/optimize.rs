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
use flow_like_storage::contracts::database::{DatabaseCleanupStats, DatabaseSelector};
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};
use utoipa::ToSchema;

const DEFAULT_RETENTION_DAYS: u64 = 7;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct OptimizePayload {
    /// Retain every table version. Set this to false to prune old versions after compaction and
    /// index maintenance finish.
    #[serde(default = "default_keep_versions")]
    pub keep_versions: bool,
    /// When pruning, keep this many days of history (at least 1, default 7). Tagged versions and
    /// branches are always kept.
    #[serde(default)]
    pub older_than_days: Option<u64>,
}

fn default_keep_versions() -> bool {
    true
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/{table}/optimize",
    tag = "database",
    description = "Compact table storage and update indices. Version history is retained by default; optional cleanup removes versions older than the chosen number of days (default 7), keeping tagged versions and branches.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = OptimizePayload,
    responses(
        (status = 200, description = "Table optimized. When old versions were pruned, the body counts what was removed; otherwise it is null.", body = Object),
        (status = 400, description = "Retention shorter than one day"),
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
    Query(selector): Query<DatabaseSelector>,
    Json(payload): Json<OptimizePayload>,
) -> Result<Json<Option<DatabaseCleanupStats>>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;
    super::validate_writable_selector(&selector)?;

    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection_with_selector(connection, table, selector).await?;

    if payload.keep_versions {
        db.optimize(true).await?;
        return Ok(Json(None));
    }
    let days = payload.older_than_days.unwrap_or(DEFAULT_RETENTION_DAYS);
    if days == 0 {
        return Err(ApiError::bad_request("older_than_days must be at least 1"));
    }
    Ok(Json(Some(db.optimize_and_cleanup(days).await?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_keep_versions_defaults_to_retention() {
        let omitted: OptimizePayload =
            serde_json::from_value(serde_json::json!({})).expect("payload should deserialize");
        assert!(omitted.keep_versions);
        assert_eq!(omitted.older_than_days, None);

        let explicit_cleanup: OptimizePayload =
            serde_json::from_value(serde_json::json!({ "keep_versions": false }))
                .expect("payload should deserialize");
        assert!(!explicit_cleanup.keep_versions);

        let chosen_retention: OptimizePayload = serde_json::from_value(
            serde_json::json!({ "keep_versions": false, "older_than_days": 1 }),
        )
        .expect("payload should deserialize");
        assert_eq!(chosen_retention.older_than_days, Some(1));
    }
}
