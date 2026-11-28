use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopedPaginationParams, read_table, validate_table_name},
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
#[tracing::instrument(name = "GET /apps/{app_id}/db/{table}/view", skip(state, user, params))]
pub async fn table_view(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(params): Query<ScopedPaginationParams>,
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

    let offset = params.offset.unwrap_or(0).min(100_000) as usize;
    let limit = params.limit.unwrap_or(25).min(250) as usize;

    let view = read_table(
        &state,
        &user,
        &permission,
        &app_id,
        &params.scope_params(),
        limit,
        TableOperation::View {
            table,
            selector,
            limit,
            offset,
        },
    )
    .await?;
    Ok(Json(view))
}
