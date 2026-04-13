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
    ensure_permission, entity::execution_run, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct ListRunsQuery {
    pub node_id: Option<String>,
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub status: Option<u8>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
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
#[tracing::instrument(name = "GET /apps/{app_id}/board/{board_id}/runs", skip(state, user))]
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
    let to_datetime = |ts: u64| -> Option<chrono::NaiveDateTime> {
        // If timestamp is >= 10^15, it's in microseconds, convert to millis
        let millis = if ts >= 1_000_000_000_000_000 {
            (ts / 1000) as i64
        } else {
            ts as i64
        };
        chrono::DateTime::from_timestamp_millis(millis).map(|dt| dt.naive_utc())
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

    let log_metas: Vec<LogMeta> = runs
        .into_iter()
        .map(|run| {
            // Convert to microseconds to match local LanceDB format
            let start = run
                .started_at
                .map(|dt: chrono::NaiveDateTime| dt.and_utc().timestamp_micros() as u64)
                .unwrap_or_else(|| run.created_at.and_utc().timestamp_micros() as u64);
            let end = run
                .completed_at
                .map(|dt: chrono::NaiveDateTime| dt.and_utc().timestamp_micros() as u64)
                .unwrap_or_else(|| run.updated_at.and_utc().timestamp_micros() as u64);

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

    tracing::info!("Returning {} runs for board {}", log_metas.len(), board_id);

    Ok(Json(log_metas))
}
