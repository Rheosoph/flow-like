use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::execution::log::LogMessage;
use flow_like_storage::Path as StoragePath;
use flow_like_storage::arrow_array::RecordBatch;
use flow_like_storage::lancedb::query::{ExecutableQuery, QueryBase};
use flow_like_types::anyhow;
use futures::TryStreamExt;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::{
    credentials::CredentialsAccess, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct QueryLogsRequest {
    pub run_id: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/logs",
    tag = "execution",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        QueryLogsRequest
    ),
    responses(
        (status = 200, description = "Log messages for the run", body = Vec<Object>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Failed to query logs")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/logs",
    skip(state, user, params)
)]
pub async fn query_logs(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(params): Query<QueryLogsRequest>,
) -> Result<Json<Vec<LogMessage>>, ApiError> {
    let _permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let sub = user.sub()?;
    let limit = params.limit.unwrap_or(100);
    let offset = params.offset.unwrap_or(0);
    let query = params.query.unwrap_or_default();

    // Get scoped credentials with read access to logs
    let credentials = state
        .scoped_credentials(&sub, &app_id, CredentialsAccess::ReadLogs)
        .await?;

    // Convert to SharedCredentials and build the logs database connection
    let shared_credentials = credentials.into_shared_credentials();
    let logs_db_builder = shared_credentials.to_logs_db_builder().map_err(|e| {
        ApiError::internal_error(anyhow!("Failed to create logs db builder: {}", e))
    })?;

    let base_path = StoragePath::from("runs")
        .child(app_id.as_str())
        .child(board_id.as_str());

    let db = logs_db_builder(base_path.clone())
        .execute()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, path = %base_path, "Failed to open log database");
            ApiError::internal_error(anyhow!("Failed to open log database: {}", e))
        })?;

    // A run row is created before its executor has emitted the first log. If
    // setup fails (for example, because a pinned board object is missing), no
    // per-run table is ever created. Treat that state like an empty log stream
    // instead of turning an otherwise inspectable run into a 500 response.
    let table_names = db.table_names().execute().await.map_err(|e| {
        tracing::error!(error = %e, path = %base_path, "Failed to list run tables");
        ApiError::internal_error(anyhow!("Failed to list run tables: {}", e))
    })?;
    if !table_names.iter().any(|name| name == &params.run_id) {
        tracing::debug!(run_id = %params.run_id, "Run has no log table yet");
        return Ok(Json(Vec::new()));
    }

    let table = db.open_table(&params.run_id).execute().await.map_err(|e| {
        tracing::error!(error = %e, run_id = %params.run_id, "Failed to open run table");
        ApiError::internal_error(anyhow!("Failed to open run table: {}", e))
    })?;

    let mut q = table.query();

    if !query.is_empty() {
        q = q.only_if(&query);
    }

    let results: Vec<RecordBatch> = q
        .offset(offset)
        .limit(limit)
        .execute()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to execute query");
            ApiError::internal_error(anyhow!("Failed to execute query: {}", e))
        })?
        .try_collect()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to collect query results");
            ApiError::internal_error(anyhow!("Failed to collect query results: {}", e))
        })?;

    use flow_like::flow::execution::log::StoredLogMessage;
    use flow_like_storage::serde_arrow;

    let mut log_messages = Vec::with_capacity(results.len() * 10);
    for result in results {
        let stored: Vec<StoredLogMessage> =
            serde_arrow::from_record_batch(&result).unwrap_or_default();
        let messages: Vec<LogMessage> = stored.into_iter().map(|log| log.into()).collect();
        log_messages.extend(messages);
    }

    tracing::info!(
        run_id = %params.run_id,
        count = log_messages.len(),
        "Returning logs for run"
    );

    Ok(Json(log_messages))
}
