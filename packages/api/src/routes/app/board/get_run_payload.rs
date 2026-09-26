use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::execution::run_index::read_run_payload;
use flow_like_types::{Value, anyhow};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::{
    ensure_permission,
    entity::{execution_run, sea_orm_active_enums::RunMode},
    error::ApiError,
    execution::run_summary::logs_store,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/runs/{run_id}/payload",
    tag = "execution",
    description = "Fetch the input a run was started with, so the run can be started again with the same input. Runs that started without an input have no payload.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        ("run_id" = String, Path, description = "Run ID from the runs listing")
    ),
    responses(
        (status = 200, description = "The recorded input payload", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: requires both ReadBoards and ReadLogs"),
        (status = 404, description = "Run not found, it recorded no input, or it is an event run (use the event corpus payload instead)")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/runs/{run_id}/payload",
    skip(state, user)
)]
pub async fn get_run_payload(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id, run_id)): Path<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    if !permission.has_permission(RolePermissions::ReadLogs) {
        return Err(ApiError::FORBIDDEN);
    }
    let sub = permission.sub()?;

    // Self-reported local rows never have a cloud sidecar, and report_run lets
    // callers mint them under any free id.
    let run = execution_run::Entity::find_by_id(&run_id)
        .filter(execution_run::Column::AppId.eq(&app_id))
        .filter(execution_run::Column::BoardId.eq(&board_id))
        .filter(execution_run::Column::Mode.ne(RunMode::Local))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if run.event_id.as_deref().is_some_and(|id| !id.is_empty()) {
        return Err(ApiError::not_found(
            "Event run inputs are served redacted by /apps/{app_id}/events/{event_id}/corpus/{run_id}/payload",
        ));
    }

    let store = logs_store(&state, &sub, &app_id).await?;
    let payload = read_run_payload(&store, &app_id, &board_id, &run_id)
        .await
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to read the payload of run {run_id}: {e}"))
        })?
        .ok_or_else(|| ApiError::not_found("This run recorded no input payload"))?;
    let payload: Value = flow_like_types::json::from_slice(&payload).map_err(|e| {
        ApiError::internal_error(anyhow!("Recorded payload of run {run_id} is not JSON: {e}"))
    })?;

    Ok(Json(payload))
}
