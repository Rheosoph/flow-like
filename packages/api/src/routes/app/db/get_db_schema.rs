use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
    routes::app::db::{ScopeParams, resolve_connection, validate_table_name},
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::{
    arrow_schema::Schema,
    databases::vector::{VectorStore, lancedb::LanceDBVectorStore},
};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db/{table}/schema",
    tag = "database",
    description = "Get the table schema.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    responses(
        (status = 200, description = "Table schema (Arrow JSON)", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/db/{table}/schema", skip(state, user))]
pub async fn get_db_schema(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<Schema>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadFiles);
    validate_table_name(&table)?;

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection(connection, table).await;

    let schema = db.schema().await?;

    Ok(Json(schema))
}
