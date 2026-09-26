use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{
        ScopedPaginationParams, resolve_connection, table_input_error, table_rejection_message,
        validate_table_name,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::contracts::database::DatabaseSelector;
use flow_like_storage::{
    databases::vector::{
        VectorStore,
        lancedb::{LanceDBVectorStore, record_batches_to_vec},
    },
    datafusion::{arrow::error::ArrowError, error::DataFusionError, prelude::SessionContext},
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
    Query(selector): Query<DatabaseSelector>,
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
    selector
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;

    let offset = params.offset.unwrap_or(0).min(100_000) as usize;
    let limit = params.limit.unwrap_or(25).min(250) as usize;

    let connection = resolve_connection(&state, &user, &app_id, &params.scope_params()).await?;
    let db = LanceDBVectorStore::from_connection_with_selector(connection, table.clone(), selector)
        .await?;

    if let Some(sql) = payload.sql {
        let items = run_sql_query(&db, table, &sql, &payload.sql_params).await?;
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
                .await
                .map_err(table_input_error)?;
            return Ok(Json(items));
        }
        (None, Some(fts_term), filter) => {
            let filter_str = filter.as_deref();
            let items = db
                .fts_search(&fts_term, filter_str, payload.select, None, limit, offset)
                .await
                .map_err(table_input_error)?;
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
                .await
                .map_err(table_input_error)?;
            return Ok(Json(items));
        }
        (None, None, Some(filter)) => {
            let items = db
                .filter(&filter, payload.select, limit, offset)
                .await
                .map_err(table_input_error)?;
            return Ok(Json(items));
        }
        _ => {
            return Err(ApiError::bad_request(
                "No valid query parameters provided".to_string(),
            ));
        }
    }
}

async fn run_sql_query(
    db: &LanceDBVectorStore,
    table: String,
    sql: &str,
    sql_params: &flow_like_types::Value,
) -> Result<Vec<flow_like_types::Value>, ApiError> {
    // The registered provider supports DML, but this endpoint is gated by read
    // permissions only — reject anything but a single SELECT before planning.
    flow_like_storage::databases::sql_guard::validate_readonly_sql(sql)
        .map_err(|error| ApiError::bad_request(format!("Invalid query SQL: {error}")))?;
    let context = SessionContext::new();
    flow_like_storage::geometry::register_geo_functions(&context);
    let fusion = db.to_datafusion().await?;
    context.register_table(table, fusion)?;
    let param_values = flow_like_storage::databases::sql_params::bind_params(sql_params)
        .map_err(|error| ApiError::bad_request(format!("Invalid query parameters: {error}")))?;
    let df = context
        .sql(sql)
        .await
        .and_then(|df| df.with_param_values(param_values))
        .map_err(|error| invalid_query(&error))?;
    let batches = df.collect().await.map_err(query_execution_error)?;
    record_batches_to_vec(Some(batches)).map_err(|error| ApiError::bad_request(error.to_string()))
}

fn invalid_query(error: &DataFusionError) -> ApiError {
    ApiError::bad_request(format!("Invalid query: {}", error.strip_backtrace()))
}

/// Unplannable statements and function or cast failures on the query's own values surface only
/// once execution starts (DataFusion defers failing constant expressions); those stay the
/// caller's to fix, while storage and resource failures stay internal.
fn query_execution_error(error: DataFusionError) -> ApiError {
    let causes = std::iter::successors(
        Some(&error as &(dyn std::error::Error + 'static)),
        |cause| cause.source(),
    );
    if let Some(message) = table_rejection_message(causes) {
        return ApiError::bad_request(message);
    }
    match error.find_root() {
        DataFusionError::Plan(_)
        | DataFusionError::SchemaError(..)
        | DataFusionError::SQL(..)
        | DataFusionError::NotImplemented(_)
        | DataFusionError::Execution(_) => invalid_query(&error),
        DataFusionError::ArrowError(arrow_error, _) if is_value_error(arrow_error) => {
            invalid_query(&error)
        }
        _ => ApiError::from(error),
    }
}

fn is_value_error(error: &ArrowError) -> bool {
    matches!(
        error,
        ArrowError::CastError(_)
            | ArrowError::ParseError(_)
            | ArrowError::DivideByZero
            | ArrowError::ArithmeticOverflow(_)
            | ArrowError::InvalidArgumentError(_)
            | ArrowError::ComputeError(_)
    )
}

#[cfg(test)]
mod tests {
    use super::query_execution_error;
    use axum::http::StatusCode;
    use flow_like_storage::databases::vector::schema::TableInputRejected;
    use flow_like_storage::datafusion::{arrow::error::ArrowError, error::DataFusionError};

    const FEATURE_COLLECTION_REJECTION: &str =
        "ST_GeomFromGeoJSON: received a GeoJSON FeatureCollection; write one row per feature";

    #[test]
    fn query_execution_error_treats_planning_failures_as_invalid_queries() {
        let error = DataFusionError::Plan("Invalid function 'st_asgeojsn'".to_string())
            .context("Failed to execute query on table 'sites'");

        let api_error = query_execution_error(error);

        assert_eq!(api_error.status(), StatusCode::BAD_REQUEST);
        let message = api_error.public_message().unwrap_or_default();
        assert!(message.starts_with("Invalid query: "), "{message}");
        assert!(
            message.contains("Invalid function 'st_asgeojsn'"),
            "{message}"
        );
    }

    #[test]
    fn query_execution_error_treats_function_input_failures_as_invalid_queries() {
        for error in [
            DataFusionError::Execution(FEATURE_COLLECTION_REJECTION.to_string()),
            DataFusionError::ArrowError(
                Box::new(ArrowError::CastError(
                    "Cannot cast string 'north' to value of Int32 type".into(),
                )),
                None,
            ),
            DataFusionError::ArrowError(Box::new(ArrowError::DivideByZero), None),
        ] {
            let expected = error.strip_backtrace();
            let api_error = query_execution_error(error.context("Query on table 'sites' failed"));

            assert_eq!(api_error.status(), StatusCode::BAD_REQUEST);
            let message = api_error.public_message().unwrap_or_default();
            assert!(message.starts_with("Invalid query: "), "{message}");
            assert!(message.contains(&expected), "{message}");
        }
    }

    #[test]
    fn query_execution_error_reports_table_input_rejections_with_their_own_message() {
        let error = DataFusionError::External(Box::new(TableInputRejected(
            FEATURE_COLLECTION_REJECTION.to_string(),
        )))
        .context("Query on table 'sites' failed");

        let api_error = query_execution_error(error);

        assert_eq!(api_error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            api_error.public_message(),
            Some(FEATURE_COLLECTION_REJECTION)
        );
    }

    #[test]
    fn query_execution_error_keeps_runtime_failures_internal() {
        for error in [
            DataFusionError::External(Box::new(std::io::Error::other(
                "object store read of bucket 'apps-prod' failed",
            ))),
            DataFusionError::IoError(std::io::Error::other("connection reset")),
            DataFusionError::ArrowError(
                Box::new(ArrowError::IoError(
                    "fragment read failed".into(),
                    std::io::Error::other("connection reset"),
                )),
                None,
            ),
            DataFusionError::ResourcesExhausted("memory pool exhausted".into()),
            DataFusionError::Internal("unexpected plan shape".into()),
        ] {
            let api_error = query_execution_error(error);

            assert_eq!(api_error.status(), StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(api_error.public_message(), None);
        }
    }
}
