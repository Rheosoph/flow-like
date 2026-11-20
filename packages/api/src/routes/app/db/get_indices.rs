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
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Serialize, ToSchema)]
pub struct IndexConfigResponse {
    pub name: String,
    pub index_type: String,
    pub columns: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db/{table}/indices",
    tag = "database",
    description = "List indices for a table.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    responses(
        (status = 200, description = "Table indices", body = Vec<IndexConfigResponse>),
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
    name = "GET /apps/{app_id}/db/{table}/indices",
    skip(state, user, scope)
)]
pub async fn get_db_indices(
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

    let indices = read_table(
        &state,
        &user,
        &permission,
        &app_id,
        &scope,
        1,
        TableOperation::Indices { table, selector },
    )
    .await?;
    Ok(Json(indices))
}
