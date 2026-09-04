use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::execution::LogMeta;
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::{
    credentials::CredentialsAccess, ensure_permission, entity::execution_run, error::ApiError,
    middleware::jwt::AppUser, permission::role_permission::RolePermissions, state::AppState,
};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct ListRunsQuery {
    pub node_id: Option<String>,
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub status: Option<u8>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    /// Also load each run's per-node activity summary (visited nodes + max
    /// severity) from the log store. Opt-in: it costs an extra object-store
    /// query, so clients request it only when rendering the activity heatmap.
    pub include_nodes: Option<bool>,
}

/// Fills per-node activity (`nodes`, `logs`) from the board's LanceDB run
/// summaries, which the executor persists on run completion. Best-effort: the
/// SQL rows stay authoritative for the run list — runs without a persisted
/// summary (still in flight, or logs never flushed) simply keep `nodes: None`.
async fn hydrate_node_summaries(
    state: &AppState,
    sub: &str,
    app_id: &str,
    board_id: &str,
    log_metas: &mut [LogMeta],
) -> flow_like_types::Result<()> {
    use flow_like::flow::execution::StoredLogMeta;
    use flow_like_storage::Path as StoragePath;
    use flow_like_storage::arrow_array::RecordBatch;
    use flow_like_storage::lancedb::query::{ExecutableQuery, QueryBase};
    use flow_like_storage::serde_arrow;
    use futures::TryStreamExt;
    use std::collections::HashMap;

    // Run ids are internal create_id() values; anything else is skipped so
    // the ids can be inlined into the lance filter safely.
    let ids: Vec<&str> = log_metas
        .iter()
        .map(|meta| meta.run_id.as_str())
        .filter(|id| {
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .collect();
    if ids.is_empty() {
        return Ok(());
    }

    let credentials = state
        .scoped_credentials(sub, app_id, CredentialsAccess::ReadLogs)
        .await?;
    let logs_db_builder = credentials.into_shared_credentials().to_logs_db_builder()?;
    let base_path = StoragePath::from("runs").child(app_id).child(board_id);
    let db = logs_db_builder(base_path).execute().await?;
    let table = db.open_table("runs").execute().await?;

    let filter = format!(
        "run_id IN ({})",
        ids.iter()
            .map(|id| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let batches: Vec<RecordBatch> = table
        .query()
        .only_if(&filter)
        .limit(ids.len())
        .execute()
        .await?
        .try_collect()
        .await?;

    let mut by_run: HashMap<String, StoredLogMeta> = HashMap::new();
    for batch in batches {
        let stored: Vec<StoredLogMeta> = serde_arrow::from_record_batch(&batch).unwrap_or_default();
        for summary in stored {
            by_run.insert(summary.run_id.clone(), summary);
        }
    }

    for meta in log_metas.iter_mut() {
        if let Some(stored) = by_run.remove(&meta.run_id) {
            let summary = LogMeta::from(stored);
            meta.nodes = summary.nodes;
            meta.logs = summary.logs;
            meta.log_level = meta.log_level.max(summary.log_level);
        }
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/runs",
    tag = "execution",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        ListRunsQuery
    ),
    responses(
        (status = 200, description = "List of execution runs", body = Vec<Object>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/runs",
    skip(state, user, query)
)]
pub async fn get_runs(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(query): Query<ListRunsQuery>,
) -> Result<Json<Vec<LogMeta>>, ApiError> {
    let _permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    // Helper to convert timestamp - handles both microseconds (16+ digits) and milliseconds (13 digits)
    let to_datetime = |ts: u64| -> Option<chrono::DateTime<chrono::FixedOffset>> {
        // If timestamp is >= 10^15, it's in microseconds, convert to millis
        let millis = if ts >= 1_000_000_000_000_000 {
            (ts / 1000) as i64
        } else {
            ts as i64
        };
        chrono::DateTime::from_timestamp_millis(millis).map(|dt| dt.fixed_offset())
    };

    let mut db_query = execution_run::Entity::find()
        .filter(execution_run::Column::BoardId.eq(&board_id))
        .filter(execution_run::Column::AppId.eq(&app_id));

    if let Some(node_id) = &query.node_id {
        db_query = db_query.filter(execution_run::Column::NodeId.eq(node_id));
    }

    if let Some(from) = query.from
        && let Some(dt) = to_datetime(from)
    {
        db_query = db_query.filter(execution_run::Column::CreatedAt.gte(dt));
    }

    if let Some(to) = query.to
        && let Some(dt) = to_datetime(to)
    {
        db_query = db_query.filter(execution_run::Column::CreatedAt.lte(dt));
    }

    if let Some(status) = query.status {
        if status == 0 {
            db_query = db_query.filter(execution_run::Column::LogLevel.lte(1));
        } else {
            db_query = db_query.filter(execution_run::Column::LogLevel.eq(status as i32));
        }
    }

    let runs = db_query
        .order_by_desc(execution_run::Column::CreatedAt)
        .limit(limit)
        .offset(offset)
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to query runs");
            ApiError::internal_error(anyhow!("Failed to query runs: {}", e))
        })?;

    let mut log_metas: Vec<LogMeta> = runs
        .into_iter()
        .map(|run| {
            // Convert to microseconds to match local LanceDB format
            let start = run
                .started_at
                .map(|dt: chrono::DateTime<chrono::FixedOffset>| dt.timestamp_micros() as u64)
                .unwrap_or_else(|| run.created_at.timestamp_micros() as u64);
            let end = run
                .completed_at
                .map(|dt: chrono::DateTime<chrono::FixedOffset>| dt.timestamp_micros() as u64)
                .unwrap_or_else(|| run.updated_at.timestamp_micros() as u64);

            LogMeta {
                app_id: run.app_id,
                run_id: run.id,
                board_id: run.board_id,
                start,
                end,
                log_level: run.log_level as u8,
                version: run.version.unwrap_or_default(),
                nodes: None,
                logs: None,
                node_id: run.node_id.unwrap_or_default(),
                event_version: None,
                event_id: run.event_id.unwrap_or_default(),
                payload: vec![],
                is_remote: true,
            }
        })
        .collect();

    // Opt-in node-activity hydration for the board heatmap: skipped entirely
    // unless requested, and never fails the run list itself.
    if query.include_nodes.unwrap_or(false)
        && !log_metas.is_empty()
        && let Ok(sub) = user.sub()
        && let Err(err) =
            hydrate_node_summaries(&state, &sub, &app_id, &board_id, &mut log_metas).await
    {
        tracing::debug!(
            error = %err,
            "Run summaries unavailable — returning runs without node activity"
        );
    }

    tracing::info!("Returning {} runs for board {}", log_metas.len(), board_id);

    Ok(Json(log_metas))
}
