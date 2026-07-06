use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_connection, validate_table_name},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct DropColumnsPayload {
    pub columns: Vec<String>,
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/db/{table}/columns",
    tag = "database",
    description = "Remove columns from a table.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = DropColumnsPayload,
    responses(
        (status = 200, description = "Columns dropped", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/db/{table}/columns", skip(state, user))]
pub async fn drop_columns(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<DropColumnsPayload>,
) -> Result<Json<()>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection(connection, table).await;

    let column_refs: Vec<&str> = payload.columns.iter().map(|s| s.as_str()).collect();
    db.drop_columns(&column_refs).await?;

    Ok(Json(()))
}
