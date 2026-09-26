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
use flow_like_storage::contracts::database::DatabaseSelector;
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct SetPrimaryKeyPayload {
    /// Required column with unique values; string, int32, int64, uint32, uint64 or binary.
    pub column: String,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/db/{table}/primary-key",
    tag = "database",
    description = "Mark a column as the table key so concurrent upserts on it never create duplicate rows. The key cannot be changed or removed; setting the current key again succeeds.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name"),
        ("scope" = Option<String>, Query, description = "Use 'user' for a user-scoped database")
    ),
    request_body = SetPrimaryKeyPayload,
    responses(
        (status = 200, description = "The column is the table key", body = ()),
        (status = 400, description = "The column cannot be the key: it is missing, optional, of an unsupported type or holds duplicate values, or the table already has another key"),
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
    name = "PUT /apps/{app_id}/db/{table}/primary-key",
    skip(state, user, scope, payload)
)]
pub async fn set_primary_key(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Query(selector): Query<DatabaseSelector>,
    Json(payload): Json<SetPrimaryKeyPayload>,
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
    let db = LanceDBVectorStore::from_connection_with_selector(connection, table, selector).await?;
    db.set_primary_key(&payload.column)
        .await
        .map_err(super::table_key_error)?;

    Ok(Json(()))
}
