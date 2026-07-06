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
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct DeleteFromDBPayload {
    pub query: String,
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/db/{table}",
    tag = "database",
    description = "Delete rows matching a filter.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = DeleteFromDBPayload,
    responses(
        (status = 200, description = "Items deleted", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/db/{table}", skip(state, user))]
pub async fn delete_from_table(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<DeleteFromDBPayload>,
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

    db.delete(&payload.query).await?;

    Ok(Json(()))
}
