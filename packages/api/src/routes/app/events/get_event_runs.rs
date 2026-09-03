use std::collections::HashMap;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::execution::LogMeta;
use flow_like_storage::Path as StoragePath;
use flow_like_storage::arrow_array::RecordBatch;
use flow_like_storage::lancedb::query::{ExecutableQuery, QueryBase, Select};
use flow_like_storage::serde_arrow;
use flow_like_types::anyhow;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    credentials::CredentialsAccess, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};

use super::db::get_event_from_db_opt;
use super::get_event::map_missing_event_artifact;

const DEFAULT_RUNS_LIMIT: u64 = 100;
/// Hard cap on one page of merged run summaries.
const MAX_RUNS_LIMIT: u64 = 200;
/// Bound on how many `(run_id, start)` ordering rows one board contributes.
/// The Lance query API cannot sort, so ordering rows are fetched narrow (two
/// columns) and sorted in-process; a board past this cap cannot page deeper.
const RUNS_ORDER_SCAN_CAP: usize = 50_000;

/// Every `StoredLogMeta` column except `payload` — run inputs never leave the
/// log store through this listing.
const SUMMARY_COLUMNS: &[&str] = &[
    "app_id",
    "run_id",
    "board_id",
    "start",
    "end",
    "log_level",
    "version",
    "nodes",
    "logs",
    "node_id",
    "event_version",
    "event_id",
];

#[derive(Serialize, ToSchema)]
pub struct EventRunsResponse {
    /// Run summaries shaped like the board runs listing, newest first.
    /// `event_version` is dotted `MAJOR.MINOR.PATCH`, matching the timeline's
    /// `version_key`.
    #[schema(value_type = Vec<Object>)]
    pub runs: Vec<LogMeta>,
    /// Boards whose Lance `runs` tables were successfully queried.
    pub boards_queried: Vec<String>,
}

#[derive(Deserialize)]
struct RunOrderRow {
    run_id: String,
    start: u64,
}

/// `StoredLogMeta` minus `payload`, matching [`SUMMARY_COLUMNS`].
#[derive(Deserialize)]
struct RunSummaryRow {
    app_id: String,
    run_id: String,
    board_id: String,
    start: u64,
    end: u64,
    log_level: u8,
    version: String,
    nodes: Option<Vec<(String, u8)>>,
    logs: Option<u64>,
    node_id: String,
    event_version: Option<String>,
    event_id: String,
}

impl From<RunSummaryRow> for LogMeta {
    fn from(row: RunSummaryRow) -> Self {
        LogMeta {
            app_id: row.app_id,
            run_id: row.run_id,
            board_id: row.board_id,
            start: row.start,
            end: row.end,
            log_level: row.log_level,
            version: row.version,
            nodes: row.nodes,
            logs: row.logs,
            node_id: row.node_id,
            event_version: row.event_version,
            event_id: row.event_id,
            payload: Vec::new(),
            is_remote: true,
        }
    }
}

/// IDs are inlined into Lance filter strings, so only `create_id()`-shaped
/// values may pass — same allowlist as the board runs hydration. Shared with
/// the regression corpus routes, which inline the same id kinds.
pub(super) fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/runs",
    tag = "events",
    description = "List recent runs for an event across the boards its versions targeted, newest first.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("board_id" = Option<Vec<String>>, Query, description = "Boards to search — repeat the parameter per board, sourced from the timeline's board list. Defaults to the event's current board."),
        ("limit" = Option<u64>, Query, description = "Maximum runs to return (default 100, capped at 200)"),
        ("offset" = Option<u64>, Query, description = "Number of newest runs to skip for paging")
    ),
    responses(
        (status = 200, description = "Run summaries for the event", body = EventRunsResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/runs",
    skip(state, user, params)
)]
pub async fn get_event_runs(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(params): Query<Vec<(String, String)>>,
) -> Result<Json<EventRunsResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);
    if !permission.has_permission(RolePermissions::ReadBoards) {
        return Err(ApiError::FORBIDDEN);
    }
    let sub = permission.sub()?;

    if !is_safe_id(&event_id) {
        return Err(ApiError::bad_request(
            "Event ID may only contain alphanumeric characters, '-' and '_'",
        ));
    }

    let mut board_ids: Vec<String> = Vec::new();
    let mut limit = DEFAULT_RUNS_LIMIT;
    let mut offset: u64 = 0;
    for (key, value) in params {
        match key.as_str() {
            "board_id" => {
                if !board_ids.contains(&value) {
                    board_ids.push(value);
                }
            }
            "limit" => {
                limit = value
                    .parse()
                    .map_err(|_| ApiError::bad_request("limit must be a non-negative integer"))?;
            }
            "offset" => {
                offset = value
                    .parse()
                    .map_err(|_| ApiError::bad_request("offset must be a non-negative integer"))?;
            }
            _ => {}
        }
    }
    let limit = limit.min(MAX_RUNS_LIMIT) as usize;
    let offset = offset as usize;

    let app = state.master_app(&sub, &app_id, &state).await?;

    if board_ids.is_empty() {
        // The timeline's board list is the intended source of the board set;
        // without it, fall back to the live event's current board.
        let live = match get_event_from_db_opt(&state.db, &event_id, &app_id).await? {
            Some(event) => event,
            None => app
                .get_event(&event_id, None)
                .await
                .map_err(|error| map_missing_event_artifact(&event_id, error))?,
        };
        if live.board_id.is_empty() {
            return Err(ApiError::bad_request(
                "Event has no board target; pass at least one board_id query parameter",
            ));
        }
        board_ids.push(live.board_id);
    }

    for board_id in &board_ids {
        if !is_safe_id(board_id) {
            return Err(ApiError::bad_request(
                "Board IDs may only contain alphanumeric characters, '-' and '_'",
            ));
        }
        if !app.boards.contains(board_id) {
            return Err(ApiError::bad_request(format!(
                "Board {board_id} does not belong to this app"
            )));
        }
    }

    let credentials = state
        .scoped_credentials(&sub, &app_id, CredentialsAccess::ReadLogs)
        .await?;
    let logs_db_builder = credentials
        .into_shared_credentials()
        .to_logs_db_builder()
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to create logs db builder: {}", e))
        })?;

    let event_filter = format!("event_id = '{event_id}'");
    let mut boards_queried: Vec<String> = Vec::new();
    let mut tables: Vec<(String, flow_like_storage::lancedb::Table)> = Vec::new();
    // (table index, run_id, start) across every queried board.
    let mut order_rows: Vec<(usize, String, u64)> = Vec::new();

    // Per-board failures skip that board rather than failing the listing —
    // one board with an unreadable log store must not hide the others.
    for board_id in &board_ids {
        let base_path = StoragePath::from("runs")
            .child(app_id.as_str())
            .child(board_id.as_str());
        let db = match logs_db_builder(base_path.clone()).execute().await {
            Ok(db) => db,
            Err(error) => {
                tracing::warn!(%error, board_id = %board_id, path = %base_path, "Failed to open runs database for board; skipping");
                continue;
            }
        };
        let table_names = match db.table_names().execute().await {
            Ok(names) => names,
            Err(error) => {
                tracing::warn!(%error, board_id = %board_id, "Failed to list run tables for board; skipping");
                continue;
            }
        };
        if !table_names.iter().any(|name| name == "runs") {
            // No run has ever flushed for this board — queried, zero rows.
            boards_queried.push(board_id.clone());
            continue;
        }
        let table = match db.open_table("runs").execute().await {
            Ok(table) => table,
            Err(error) => {
                tracing::warn!(%error, board_id = %board_id, "Failed to open runs table for board; skipping");
                continue;
            }
        };
        let batches: Result<Vec<RecordBatch>, _> = match table
            .query()
            .only_if(&event_filter)
            .select(Select::columns(&["run_id", "start"]))
            .limit(RUNS_ORDER_SCAN_CAP)
            .execute()
            .await
        {
            Ok(stream) => stream.try_collect().await,
            Err(error) => Err(error),
        };
        let batches = match batches {
            Ok(batches) => batches,
            Err(error) => {
                tracing::warn!(%error, board_id = %board_id, "Failed to query runs for board; skipping");
                continue;
            }
        };

        let table_index = tables.len();
        for batch in &batches {
            let rows: Vec<RunOrderRow> = serde_arrow::from_record_batch(batch).unwrap_or_default();
            order_rows.extend(
                rows.into_iter()
                    .map(|row| (table_index, row.run_id, row.start)),
            );
        }
        tables.push((board_id.clone(), table));
        boards_queried.push(board_id.clone());
    }

    // Dedupe by run id (a double flush keeps the newest row), newest first.
    let mut newest_by_run: HashMap<String, (usize, u64)> = HashMap::new();
    for (table_index, run_id, start) in order_rows {
        match newest_by_run.get(&run_id) {
            Some((_, existing)) if *existing >= start => {}
            _ => {
                newest_by_run.insert(run_id, (table_index, start));
            }
        }
    }
    let mut ordered: Vec<(String, usize, u64)> = newest_by_run
        .into_iter()
        .map(|(run_id, (table_index, start))| (run_id, table_index, start))
        .collect();
    ordered.sort_unstable_by(|a, b| b.2.cmp(&a.2).then_with(|| b.0.cmp(&a.0)));

    let page: Vec<(String, usize)> = ordered
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(run_id, table_index, _)| (run_id, table_index))
        .collect();

    // Second pass: fetch the page's full summaries per board. Run ids come
    // out of the log store, so they pass the same allowlist before being
    // inlined into the filter.
    let mut ids_by_table: HashMap<usize, Vec<&str>> = HashMap::new();
    for (run_id, table_index) in &page {
        if is_safe_id(run_id) {
            ids_by_table
                .entry(*table_index)
                .or_default()
                .push(run_id.as_str());
        }
    }

    let mut summaries: HashMap<String, LogMeta> = HashMap::new();
    for (table_index, ids) in ids_by_table {
        let (board_id, table) = &tables[table_index];
        let filter = format!(
            "run_id IN ({})",
            ids.iter()
                .map(|id| format!("'{id}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let batches: Result<Vec<RecordBatch>, _> = match table
            .query()
            .only_if(&filter)
            .select(Select::columns(SUMMARY_COLUMNS))
            .limit(ids.len())
            .execute()
            .await
        {
            Ok(stream) => stream.try_collect().await,
            Err(error) => Err(error),
        };
        let batches = match batches {
            Ok(batches) => batches,
            Err(error) => {
                tracing::warn!(%error, board_id = %board_id, "Failed to load run summaries for board");
                continue;
            }
        };
        for batch in &batches {
            let rows: Vec<RunSummaryRow> =
                serde_arrow::from_record_batch(batch).unwrap_or_default();
            for row in rows {
                summaries.insert(row.run_id.clone(), LogMeta::from(row));
            }
        }
    }

    let runs: Vec<LogMeta> = page
        .into_iter()
        .filter_map(|(run_id, _)| summaries.remove(&run_id))
        .collect();

    Ok(Json(EventRunsResponse {
        runs,
        boards_queried,
    }))
}
