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
    databases::vector::{
        VectorStore,
        lancedb::{LanceDBVectorStore, record_batches_to_vec},
    },
    datafusion::prelude::SessionContext,
};
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
    Json(payload): Json<QueryTablePayload>,
) -> Result<Json<Vec<flow_like_types::Value>>, ApiError> {
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

    let connection = resolve_connection(&state, &user, &app_id, &params.scope_params()).await?;
    let db = LanceDBVectorStore::from_connection(connection, table.clone()).await;

    if let Some(sql) = payload.sql {
        // The registered provider supports DML, but this endpoint is gated by read
        // permissions only — reject anything but a single SELECT before planning.
        flow_like_storage::databases::sql_guard::validate_readonly_sql(&sql)
            .map_err(|error| ApiError::bad_request(format!("Invalid query SQL: {error}")))?;
        let context = SessionContext::new();
        let fusion = db.to_datafusion().await?;
        context.register_table(table, fusion)?;
        let param_values =
            flow_like_storage::databases::sql_params::bind_params(&payload.sql_params)?;
        let df = context.sql(&sql).await?.with_param_values(param_values)?;
        let items = df.collect().await?;
        let items = record_batches_to_vec(Some(items))?;
        return Ok(Json(items));
    }

    match (payload.vector_query, payload.fts_term, payload.filter) {
        (Some(vector_query), None, filter) => {
            let filter_str = filter.as_deref();
            let items = db
                .vector_search(
                    vector_query.vector,
                    filter_str,
                    payload.select,
                    limit,
                    offset,
                )
                .await?;
            return Ok(Json(items));
        }
        (None, Some(fts_term), filter) => {
            let filter_str = filter.as_deref();
            let items = db
                .fts_search(&fts_term, filter_str, payload.select, None, limit, offset)
                .await?;
            return Ok(Json(items));
        }
        (Some(vector_query), Some(fts_term), filter) => {
            let filter_str = filter.as_deref();
            let items = db
                .hybrid_search(
                    vector_query.vector,
                    &fts_term,
                    filter_str,
                    payload.select,
                    None,
                    limit,
                    offset,
                    payload.rerank.unwrap_or(true),
                )
                .await?;
            return Ok(Json(items));
        }
        (None, None, Some(filter)) => {
            let items = db.filter(&filter, payload.select, limit, offset).await?;
            return Ok(Json(items));
        }
        _ => {
            return Err(ApiError::bad_request(
                "No valid query parameters provided".to_string(),
            ));
        }
    }
}
