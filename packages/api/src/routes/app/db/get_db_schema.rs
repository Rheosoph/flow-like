use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, read_table, validate_table_name},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_query::table::TableOperation;
use flow_like_storage::contracts::database::DatabaseSelector;

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
#[tracing::instrument(
    name = "GET /apps/{app_id}/db/{table}/schema",
    skip(state, user, scope)
)]
pub async fn get_db_schema(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Query(selector): Query<DatabaseSelector>,
) -> Result<Json<flow_like_types::Value>, ApiError> {
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );
    validate_table_name(&table)?;
    selector
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;

    let schema = read_table(
        &state,
        &user,
        &permission,
        &app_id,
        &scope,
        1,
        TableOperation::Schema { table, selector },
    )
    .await?;
    Ok(Json(schema))
}
