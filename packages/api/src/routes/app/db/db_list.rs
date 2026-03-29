use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopedPaginationParams, resolve_connection, validate_table_name},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db/{table}",
    tag = "database",
    description = "List rows from a table with pagination.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name"),
        ("limit" = Option<u64>, Query, description = "Max results (default 25, max 250)"),
        ("offset" = Option<u64>, Query, description = "Result offset")
    ),
    responses(
        (status = 200, description = "List table items", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/db/{table}", skip(state, user))]
pub async fn list_items(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(params): Query<ScopedPaginationParams>,
) -> Result<Json<Vec<flow_like_types::Value>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadFiles);
    validate_table_name(&table)?;

    let offset = params.offset.unwrap_or(0).min(100_000) as usize;
    let limit = params.limit.unwrap_or(25).min(250) as usize;

    let connection = resolve_connection(&state, &user, &app_id, &params.scope_params()).await?;
    let db = LanceDBVectorStore::from_connection(connection, table).await;

    let items = db.list(None, limit, offset).await?;

    Ok(Json(items))
}
