use crate::{
    audit_branch, ensure_any_permission,
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
use flow_like_storage::contracts::database::DatabaseSelector;
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdatePayload {
    pub filter: String,
    pub updates: HashMap<String, flow_like_types::Value>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/db/{table}/update",
    tag = "database",
    description = "Update rows matching a filter.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = String,
    responses(
        (status = 200, description = "Items updated", body = ()),
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
    name = "PUT /apps/{app_id}/db/{table}/update",
    skip(state, user, scope, payload)
)]
pub async fn update_table(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Query(selector): Query<DatabaseSelector>,
    Json(payload): Json<UpdatePayload>,
) -> Result<Json<()>, ApiError> {
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
    let db = LanceDBVectorStore::from_connection_with_selector(connection, table.clone(), selector)
        .await?;

    let column_count = payload.updates.len();
    db.update(&payload.filter, payload.updates).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "database.rows.update",
        "DatabaseTable",
        table,
        serde_json::json!({
            "column_count": column_count,
            "user_scoped": scope.is_user_scoped(),
        })
    );

    Ok(Json(()))
}
