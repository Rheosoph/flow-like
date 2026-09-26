use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::execution::log::LogMessage;
use flow_like::flow::execution::log_query::{
    LogQuery, MAX_PAGE_SIZE, count_logs, query_log_page, scan_log_summary,
};
use flow_like::flow::execution::log_summary::{LogSummary, read_run_summary};
use flow_like_types::anyhow;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::query_logs::open_run_log_table;
use crate::{
    ensure_permission, error::ApiError, execution::run_summary::logs_store,
    middleware::jwt::AppUser, permission::role_permission::RolePermissions, state::AppState,
};

#[derive(Debug, Deserialize, ToSchema)]
pub struct QueryRunLogsRequest {
    pub run_id: String,
    #[serde(default)]
    pub query: LogQuery,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_page_size")]
    pub limit: usize,
}

fn default_page_size() -> usize {
    100
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CountRunLogsRequest {
    pub run_id: String,
    #[serde(default)]
    pub query: LogQuery,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CountRunLogsResponse {
    pub count: usize,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct RunLogSummaryParams {
    pub run_id: String,
}

fn query_failed(run_id: &str, action: &str, error: flow_like_types::Error) -> ApiError {
    tracing::error!(run_id = %run_id, error = %error, "Failed to {action}");
    ApiError::internal_error(anyhow!("Failed to {action} for run {run_id}: {error}"))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/logs/query",
    tag = "execution",
    description = "One page of a run's logs, oldest first, matching a structured filter: levels, nodes, text, time range and repeat groups, each includable or excludable.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = QueryRunLogsRequest,
    responses(
        (status = 200, description = "Log messages for the page", body = Vec<Object>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: requires both ReadBoards and ReadLogs"),
        (status = 500, description = "Failed to query logs")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/logs/query",
    skip(state, user, request)
)]
pub async fn query_run_logs(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(request): Json<QueryRunLogsRequest>,
) -> Result<Json<Vec<LogMessage>>, ApiError> {
    let _permission = ensure_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadBoards | RolePermissions::ReadLogs
    );
    let sub = user.sub()?;
    if request.limit > MAX_PAGE_SIZE {
        return Err(ApiError::bad_request(format!(
            "A log page holds at most {MAX_PAGE_SIZE} messages, got a limit of {}",
            request.limit
        )));
    }

    let Some(table) = open_run_log_table(&state, &sub, &app_id, &board_id, &request.run_id).await?
    else {
        return Ok(Json(Vec::new()));
    };
    let logs = query_log_page(&table, &request.query, request.offset, request.limit)
        .await
        .map_err(|e| query_failed(&request.run_id, "query logs", e))?;
    Ok(Json(logs))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/logs/count",
    tag = "execution",
    description = "How many of a run's logs match a structured filter.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = CountRunLogsRequest,
    responses(
        (status = 200, description = "The number of matching logs", body = CountRunLogsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: requires both ReadBoards and ReadLogs"),
        (status = 500, description = "Failed to count logs")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/logs/count",
    skip(state, user, request)
)]
pub async fn count_run_logs(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(request): Json<CountRunLogsRequest>,
) -> Result<Json<CountRunLogsResponse>, ApiError> {
    let _permission = ensure_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadBoards | RolePermissions::ReadLogs
    );
    let sub = user.sub()?;

    let Some(table) = open_run_log_table(&state, &sub, &app_id, &board_id, &request.run_id).await?
    else {
        return Ok(Json(CountRunLogsResponse { count: 0 }));
    };
    let count = count_logs(&table, &request.query)
        .await
        .map_err(|e| query_failed(&request.run_id, "count logs", e))?;
    Ok(Json(CountRunLogsResponse { count }))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/logs/summary",
    tag = "execution",
    description = "Counts per level and node, the first error, the nodes that ran and the most frequent repeated messages of one run. Runs recorded before summaries existed get one computed from their logs.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        RunLogSummaryParams
    ),
    responses(
        (status = 200, description = "The run's log summary, or null when it has no logs", body = Option<LogSummary>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: requires both ReadBoards and ReadLogs"),
        (status = 500, description = "Failed to read the summary")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/logs/summary",
    skip(state, user, params)
)]
pub async fn get_run_log_summary(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(params): Query<RunLogSummaryParams>,
) -> Result<Json<Option<LogSummary>>, ApiError> {
    let _permission = ensure_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadBoards | RolePermissions::ReadLogs
    );
    let sub = user.sub()?;

    let store = logs_store(&state, &sub, &app_id).await?;
    if let Some(summary) = read_run_summary(&store, &app_id, &board_id, &params.run_id)
        .await
        .map_err(|e| query_failed(&params.run_id, "read the log summary", e))?
    {
        return Ok(Json(Some(summary)));
    }

    let Some(table) = open_run_log_table(&state, &sub, &app_id, &board_id, &params.run_id).await?
    else {
        return Ok(Json(None));
    };
    let summary = scan_log_summary(&table, None)
        .await
        .map_err(|e| query_failed(&params.run_id, "summarize logs", e))?;
    Ok(Json(Some(summary)))
}
