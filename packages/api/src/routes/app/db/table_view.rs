use crate::{
    ensure_any_permission,
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
use flow_like_storage::{
    arrow_schema::Schema,
    databases::vector::{VectorStore, lancedb::LanceDBVectorStore},
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TableViewResponse {
    pub schema: Schema,
    pub count: usize,
    pub items: Vec<flow_like_types::Value>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db/{table}/view",
    tag = "database",
    description = "Get combined table view: schema + count + items. Reuses a single DB connection for all three operations.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name"),
        ("limit" = Option<u64>, Query, description = "Max results (default 25, max 250)"),
        ("offset" = Option<u64>, Query, description = "Result offset")
    ),
    responses(
        (status = 200, description = "Table view with schema, count, and items", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/db/{table}/view", skip(state, user))]
pub async fn table_view(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(params): Query<ScopedPaginationParams>,
) -> Result<Json<TableViewResponse>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );
    validate_table_name(&table)?;

    let offset = params.offset.unwrap_or(0).min(100_000) as usize;
    let limit = params.limit.unwrap_or(25).min(250) as usize;

    // Single connection reused for all 3 operations
    let connection = resolve_connection(&state, &user, &app_id, &params.scope_params()).await?;
    let db = LanceDBVectorStore::from_connection(connection, table).await;

    let schema = db.schema().await?;
    let count = db.count(None).await?;
    let items = db.list(None, limit, offset).await?;

    Ok(Json(TableViewResponse {
        schema,
        count,
        items,
    }))
}
