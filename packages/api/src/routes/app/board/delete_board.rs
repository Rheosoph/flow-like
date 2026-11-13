use super::board_cleanup::{
    BoardCleanupSteps, clean_up_board, delete_page_folder, delete_page_rows, events_of_board,
    ontology_action_events, refuse_ontology_actions,
};
use crate::{
    audit_branch, ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::events::delete_event::remove_event,
    state::{AppState, BoardMutationGuard},
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::{app::App, state::FlowLikeState};
use flow_like_types::anyhow;

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/board/{board_id}",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    responses(
        (status = 200, description = "Board deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden. Deleting a board that still has events also needs the permission to write events."),
        (status = 404, description = "Board not found"),
        (status = 409, description = "The board still runs ontology actions (code BOARD_HAS_ONTOLOGY_ACTIONS). Remove those actions in Data Studio first; nothing was deleted."),
        (status = 500, description = "A clean-up step failed (code BOARD_CLEANUP_FAILED). The board is still in place and the message names the step and id; retrying the identical request resumes the clean-up."),
        (status = 423, description = "Another writer holds this board's mutation lease (code BOARD_LOCKED). Nothing was written; retry the identical request shortly.")
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/board/{board_id}", skip(state, user))]
pub async fn delete_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;
    let mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;

    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    let may_delete_events = permission.has_permission(RolePermissions::WriteEvents);
    remove_board_and_dependants(
        &state,
        &mut app,
        &app_id,
        &board_id,
        &mutation_guard,
        may_delete_events,
    )
    .await?;
    // A re-created board id must not inherit the deleted board's prerun manifests.
    let manifest_prefix = format!("{app_id}:{board_id}:");
    let _ = state
        .prerun_manifest_cache
        .invalidate_entries_if(move |key, _| key.starts_with(&manifest_prefix));
    mutation_guard.ensure_held()?;
    app.save().await?;

    if let Err(err) =
        crate::routes::app::board::scoring::delete_board_score(&state.db, &app_id, &board_id).await
    {
        tracing::warn!("failed to delete board score for {app_id}/{board_id}: {err:?}");
    }

    audit_branch!(state, user, app_id, "board.delete", "Board", board_id);
    Ok(Json(()))
}

struct RouteCleanup<'a> {
    state: &'a AppState,
    app: &'a mut App,
    app_id: &'a str,
    board_id: &'a str,
}

impl BoardCleanupSteps for RouteCleanup<'_> {
    async fn remove_event(&mut self, event_id: &str) -> Result<(), ApiError> {
        remove_event(self.state, self.app, event_id).await
    }

    async fn remove_pages(&mut self) -> Result<(), ApiError> {
        let flow_state = self
            .app
            .app_state
            .clone()
            .ok_or_else(|| ApiError::internal_error(anyhow!("App state not found")))?;
        let store = FlowLikeState::project_meta_store(&flow_state)
            .await?
            .as_generic();
        delete_page_folder(store.as_ref(), self.app_id, self.board_id).await?;
        delete_page_rows(
            &self.state.db,
            self.state.db_dialect,
            self.app_id,
            self.board_id,
        )
        .await
    }
}

async fn remove_board_and_dependants(
    state: &AppState,
    app: &mut App,
    app_id: &str,
    board_id: &str,
    mutation_guard: &BoardMutationGuard,
    may_delete_events: bool,
) -> Result<(), ApiError> {
    let event_ids = events_of_board(&state.db, app_id, board_id).await?;
    if !event_ids.is_empty() && !may_delete_events {
        return Err(ApiError::forbidden(format!(
            "Deleting board {board_id} also deletes its {} events, which needs the permission to write events",
            event_ids.len()
        )));
    }
    let managed = ontology_action_events(&state.db, app_id, &event_ids).await?;
    refuse_ontology_actions(board_id, &managed)?;
    let mut steps = RouteCleanup {
        state,
        app,
        app_id,
        board_id,
    };
    clean_up_board(&mut steps, board_id, &event_ids).await?;
    mutation_guard.ensure_held()?;
    app.delete_board(board_id).await?;
    Ok(())
}
