use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, TableListParams, read_table},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_query::table::TableOperation;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db",
    tag = "database",
    description = "List the tables in the app database. Pass `detail=summary` to get each table's row count, schema, indexes, storage footprint and the ontology objects, actions and saved queries that read it, instead of just the names.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("detail" = Option<String>, Query, description = "Use 'summary' for full per-table summaries")
    ),
    responses(
        (status = 200, description = "Table names, or full summaries when detail=summary", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/db", skip(state, user))]
pub async fn list_tables(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(params): Query<TableListParams>,
) -> Result<Json<flow_like_types::Value>, ApiError> {
    let permission = ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let tables = read_table(
        &state,
        &user,
        &permission,
        &app_id,
        &ScopeParams::default(),
        1,
        TableOperation::ListTables {
            summary: params.wants_summary(),
        },
    )
    .await?;
    Ok(Json(tables))
}
