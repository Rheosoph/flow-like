use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::execution::LogMeta;
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    ensure_permission, entity::execution_run, error::ApiError,
    execution::run_summary::log_meta_from_run, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};

use super::db::get_event_from_db_opt;
use super::get_event::map_missing_event_artifact;

const DEFAULT_RUNS_LIMIT: u64 = 100;
/// Hard cap on one page of run summaries.
const MAX_RUNS_LIMIT: u64 = 200;

#[derive(Serialize, ToSchema)]
pub struct EventRunsResponse {
    /// Run summaries shaped like the board runs listing, newest first.
    /// `event_version` is dotted `MAJOR.MINOR.PATCH`, matching the timeline's
    /// `version_key`.
    #[schema(value_type = Vec<Object>)]
    pub runs: Vec<LogMeta>,
    /// Boards the listing covered.
    pub boards_queried: Vec<String>,
}

/// Board, event and run ids end up in storage paths and log filters, so only
/// `create_id()`-shaped values may pass. Shared with the regression corpus
/// routes.
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
    let limit = limit.min(MAX_RUNS_LIMIT);

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

    let runs = execution_run::Entity::find()
        .filter(execution_run::Column::AppId.eq(&app_id))
        .filter(execution_run::Column::EventId.eq(&event_id))
        .filter(execution_run::Column::BoardId.is_in(board_ids.clone()))
        .order_by_desc(execution_run::Column::CreatedAt)
        .order_by_desc(execution_run::Column::Id)
        .offset(offset)
        .limit(limit)
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to query event runs: {e}")))?;

    Ok(Json(EventRunsResponse {
        runs: runs.iter().map(log_meta_from_run).collect(),
        boards_queried: board_ids,
    }))
}
