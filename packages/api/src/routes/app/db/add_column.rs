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
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct AddColumnPayload {
    pub name: String,
    pub sql_expression: String,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/{table}/columns",
    tag = "database",
    description = "Add a computed column to a table.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = AddColumnPayload,
    responses(
        (status = 200, description = "Column added", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/db/{table}/columns", skip(state, user))]
pub async fn add_column(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<AddColumnPayload>,
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

    db.add_column(&payload.name, &payload.sql_expression)
        .await?;

    Ok(Json(()))
}
