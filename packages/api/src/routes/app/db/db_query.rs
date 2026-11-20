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
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct VectorQueryPayload {
    pub vector: Vec<f64>,
}

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct QueryTablePayload {
    sql: Option<String>,
    /// Values for the `sql` field's `$placeholders`, keyed by placeholder name without the
    /// `$`. Bound by the planner, so a caller never has to build a value into the statement.
    #[serde(default)]
    sql_params: flow_like_types::Value,
    vector_query: Option<VectorQueryPayload>,
    filter: Option<String>,
    fts_term: Option<String>,
    rerank: Option<bool>,
    select: Option<Vec<String>>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/{table}/query",
    tag = "database",
    description = "Query a table using SQL, vector, full-text, or hybrid search.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name"),
        ("limit" = Option<u64>, Query, description = "Max results (default 25, max 250)"),
        ("offset" = Option<u64>, Query, description = "Result offset")
    ),
    request_body = QueryTablePayload,
    responses(
        (status = 200, description = "Query results", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request"),
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
    name = "POST /apps/{app_id}/db/{table}/query",
    skip(state, user, params, payload)
)]
pub async fn query_table(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(params): Query<ScopedPaginationParams>,
    Query(selector): Query<DatabaseSelector>,
    Json(payload): Json<QueryTablePayload>,
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

    let operation = TableOperation::Query {
        table,
        selector,
        sql: payload.sql,
        sql_params: payload.sql_params,
        vector: payload.vector_query.map(|query| query.vector),
        filter: payload.filter,
        fts_term: payload.fts_term,
        rerank: payload.rerank,
        select: payload.select,
        limit,
        offset,
    };
    let items = read_table(
        &state,
        &user,
        &permission,
        &app_id,
        &params.scope_params(),
        limit,
        operation,
    )
    .await?;
    Ok(Json(items))
}
