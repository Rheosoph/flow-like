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
    bot_flow::bot_from_technical_id,
    ensure_permission,
    entity::{bot, execution_run},
    error::ApiError,
    execution::run_summary::log_meta_from_run,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
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
    pub runs: Vec<RunRow>,
    /// Boards the listing covered.
    pub boards_queried: Vec<String>,
}

/// Who started a run when it was not a person: today a bot, acting for its owner.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct RunActor {
    pub kind: &'static str,
    pub id: String,
    pub name: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct RunRow {
    #[serde(flatten)]
    pub meta: LogMeta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<RunActor>,
}

pub(crate) fn run_actor(
    technical_user_id: Option<&str>,
    names: &std::collections::HashMap<String, Option<String>>,
) -> Option<RunActor> {
    let bot = bot_from_technical_id(technical_user_id?)?;
    Some(RunActor {
        kind: "bot",
        name: names.get(&bot.bot_id).cloned().flatten(),
        id: bot.bot_id,
    })
}

async fn bot_names(
    state: &AppState,
    runs: &[execution_run::Model],
) -> Result<std::collections::HashMap<String, Option<String>>, ApiError> {
    let ids: std::collections::BTreeSet<String> = runs
        .iter()
        .filter_map(|run| bot_from_technical_id(run.technical_user_id.as_deref()?))
        .map(|bot| bot.bot_id)
        .collect();
    if ids.is_empty() {
        return Ok(Default::default());
    }
    Ok(bot::Entity::find()
        .filter(bot::Column::Id.is_in(ids))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|bot| (bot.id, bot.name))
        .collect())
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

    let names = bot_names(&state, &runs).await?;
    Ok(Json(EventRunsResponse {
        runs: runs
            .iter()
            .map(|run| RunRow {
                meta: log_meta_from_run(run),
                actor: run_actor(run.technical_user_id.as_deref(), &names),
            })
            .collect(),
        boards_queried: board_ids,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bot_started_run_names_the_bot_and_a_key_run_names_nobody() {
        let names = [("bot_1".to_string(), Some("Ada".to_string()))].into();
        let actor = run_actor(Some("bot_1"), &names).unwrap();
        assert_eq!(actor.kind, "bot");
        assert_eq!(actor.name.as_deref(), Some("Ada"));
        assert!(run_actor(Some("key-1"), &names).is_none());
        assert!(run_actor(None, &names).is_none());
    }

    #[test]
    fn a_retired_bot_keeps_its_id_without_a_name() {
        let actor = run_actor(Some("bot_gone"), &Default::default()).unwrap();
        assert_eq!(actor.id, "bot_gone");
        assert!(actor.name.is_none());
    }

    #[test]
    fn the_row_flattens_the_actor_next_to_the_run_summary() {
        let run = serde_json::json!({
            "app_id": "a", "run_id": "r", "board_id": "b", "start": 1, "end": 2,
            "log_level": 0, "version": "", "nodes": null, "logs": null, "node_id": "",
            "event_version": null, "event_id": "e", "payload": []
        });
        let meta: LogMeta = serde_json::from_value(run).unwrap();
        let row = RunRow {
            meta,
            actor: run_actor(Some("bot_1"), &Default::default()),
        };
        let value = serde_json::to_value(row).unwrap();
        assert_eq!(value["run_id"], "r");
        assert_eq!(value["actor"]["kind"], "bot");
    }
}
