use crate::{
    audit_branch,
    db::{DEFAULT_WRITE_CHUNK, insert_in_chunks},
    ensure_permission,
    entity::page,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::board::{secrets::filter_board_secrets, sink_rebind::rebind_latest_sinks},
    state::{AppState, BoardMutationGuard},
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use flow_like::{
    a2ui::widget::Page,
    app::App,
    flow::{
        board::{Board, ExecutionMode, ExecutionStage},
        execution::LogLevel,
    },
};
use sea_orm::{ActiveValue::Set, sea_query::OnConflict};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use utoipa::ToSchema;

#[derive(Clone, Deserialize, ToSchema)]
pub struct UpsertBoard {
    pub name: Option<String>,
    pub description: Option<String>,
    #[schema(value_type = Option<String>)]
    pub stage: Option<ExecutionStage>,
    #[schema(value_type = Option<i32>)]
    pub log_level: Option<LogLevel>,
    #[schema(value_type = Option<String>)]
    pub execution_mode: Option<ExecutionMode>,
    /// Board to instantiate the new board from. Only valid when `board_id` does not exist yet;
    /// an existing board's graph is edited through `POST /apps/{app_id}/board/{board_id}`.
    #[schema(value_type = Option<Object>)]
    pub template: Option<Board>,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct UpsertBoardResponse {
    pub id: String,
    #[schema(value_type = Object)]
    pub updated_at: SystemTime,
    /// Present when a board was instantiated from a template so clients can cache the exact
    /// server-generated node and pin IDs without instantiating the template a second time.
    #[schema(value_type = Option<Object>)]
    pub board: Option<Board>,
}

/// Register the pages a template instantiation just wrote to storage.
///
/// Instantiated pages are minted inside the core board path, so no page-upload call ever describes
/// them. Without a row each, `GET /apps/{app_id}/pages` cannot see them and a later fork — which is
/// driven off the `Page` table — drops them silently.
///
/// A template carries an unbounded number of pages, so the insert is chunked: each chunk commits in
/// its own retried transaction, which both stays under a bounded engine's per-statement row cap and
/// absorbs the serialization conflicts a bare `exec(&state.db)` would surface as a `409`. Page ids
/// come from the instantiation, so `DO NOTHING` makes a replayed chunk harmless.
async fn persist_instantiated_pages(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    pages: &[Page],
) -> Result<(), ApiError> {
    if pages.is_empty() {
        return Ok(());
    }

    let rows = instantiated_page_rows(app_id, board_id, pages, chrono::Utc::now().fixed_offset());

    insert_in_chunks(
        &state.db,
        state.db_dialect,
        rows,
        DEFAULT_WRITE_CHUNK,
        Some(OnConflict::new().do_nothing().to_owned()),
    )
    .await?;
    Ok(())
}

fn instantiated_page_rows(
    app_id: &str,
    board_id: &str,
    pages: &[Page],
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Vec<page::ActiveModel> {
    pages
        .iter()
        .map(|page| page::ActiveModel {
            id: Set(page.id.clone()),
            name: Set(page.name.clone()),
            description: Set(page.title.clone()),
            app_id: Set(app_id.to_string()),
            board_id: Set(Some(board_id.to_string())),
            version: Set(page
                .version
                .map(|version| format!("{}.{}.{}", version.0, version.1, version.2))),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .collect()
}

/// Instantiation remaps every node and pin id and mints pages, so it cannot double as an in-place
/// graph replacement: events bound to the board's node ids would dangle, and a round-tripped board
/// would arrive with its secrets already stripped by the read path.
fn ensure_template_targets_new_board(
    created_board: bool,
    has_template: bool,
    app_id: &str,
    board_id: &str,
) -> Result<(), ApiError> {
    if created_board || !has_template {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::CONFLICT,
        "BOARD_EXISTS",
        format!(
            "Board {board_id} already exists in app {app_id}; `template` only instantiates new boards and nothing was written. Edit the existing graph with POST /apps/{app_id}/board/{board_id} (board commands) or POST /apps/{app_id}/board/{board_id}/flowscript/apply, or PUT without `template` to update metadata only."
        ),
    ))
}

/// Creates `board_id` when the app does not list it yet. Returns whether it was instantiated from
/// `template`, in which case the caller hands the exact server-minted graph back.
async fn create_missing_board(
    state: &AppState,
    app: &mut App,
    app_id: &str,
    board_id: &str,
    template: Option<Board>,
    mutation_guard: &BoardMutationGuard,
) -> Result<bool, ApiError> {
    let created_board = !app.boards.iter().any(|id| id == board_id);
    let has_template = template.is_some();
    ensure_template_targets_new_board(created_board, has_template, app_id, board_id)?;
    if !created_board {
        return Ok(false);
    }
    // Instantiate through the same core path as offline boards. Besides remapping node/pin
    // IDs, this runs catalog schema migrations and keeps compact schema refs resolvable before
    // the first board snapshot is saved.
    let created = app
        .create_board(Some(board_id.to_string()), template)
        .await?;
    mutation_guard.ensure_held()?;
    app.save().await?;
    persist_instantiated_pages(state, app_id, &created.board_id, &created.pages).await?;
    Ok(has_template)
}

async fn save_metadata(
    board: &mut Board,
    params: UpsertBoard,
    mutation_guard: &BoardMutationGuard,
) -> Result<(), ApiError> {
    board.name = params.name.unwrap_or(board.name.clone());
    board.description = params.description.unwrap_or(board.description.clone());
    board.stage = params.stage.unwrap_or(board.stage.clone());
    board.log_level = params.log_level.unwrap_or(board.log_level);
    board.execution_mode = params
        .execution_mode
        .unwrap_or(board.execution_mode.clone());
    board.mark_changed();
    mutation_guard.ensure_held()?;
    board.save(None).await?;
    Ok(())
}

async fn record_board_score(state: &AppState, app_id: &str, board: &Board) {
    if let Err(err) =
        crate::routes::app::board::scoring::persist_board_score(&state.db, app_id, board).await
    {
        tracing::warn!(
            "failed to persist board score for {app_id}/{}: {err:?}",
            board.id
        );
    }
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/board/{board_id}",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = UpsertBoard,
    responses(
        (status = 200, description = "Board created or updated", body = UpsertBoardResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 409, description = "The board already exists and the request carried a `template` (code BOARD_EXISTS). Nothing was written; edit an existing board's graph with board commands instead."),
        (status = 423, description = "Another writer holds this board's mutation lease (code BOARD_LOCKED). Nothing was written; retry the identical request shortly.")
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/board/{board_id}",
    skip(state, user, params)
)]
pub async fn upsert_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(mut params): Json<UpsertBoard>,
) -> Result<Json<UpsertBoardResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;
    let mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;

    let mut app = state.master_app(&sub, &app_id, &state).await?;
    let template = params.template.take();
    rebind_latest_sinks(&state, &app_id, &board_id, &user).await;
    let instantiated = create_missing_board(
        &state,
        &mut app,
        &app_id,
        &board_id,
        template,
        &mutation_guard,
    )
    .await?;

    let board = app.open_board(board_id, Some(false), None).await?;
    let mut board = board.write().await;
    save_metadata(&mut board, params, &mutation_guard).await?;
    record_board_score(&state, &app_id, &board).await;
    rebind_latest_sinks(&state, &app_id, &board.id, &user).await;

    audit_branch!(state, user, app_id, "board.update", "Board", board.id);
    let response_board = instantiated.then(|| {
        let mut response_board = (*board).clone();
        filter_board_secrets(&mut response_board);
        response_board
    });
    Ok(Json(UpsertBoardResponse {
        id: board.id.clone(),
        updated_at: board.updated_at,
        board: response_board,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_db::DEFAULT_WRITE_CHUNK;

    fn page(id: &str) -> Page {
        Page::new(id, id, format!("/{id}"))
    }

    /// L2: the instantiation used to send one `insert_many` per template, unbounded by page count
    /// and outside any retry wrapper. Chunking is what keeps it under a bounded engine's
    /// per-statement row cap.
    #[test]
    fn instantiated_pages_chunk_below_the_write_cap() {
        let pages = (0..DEFAULT_WRITE_CHUNK * 2 + 7)
            .map(|index| page(&format!("page-{index}")))
            .collect::<Vec<_>>();
        let now = chrono::Utc::now().fixed_offset();
        let rows = instantiated_page_rows("app", "board", &pages, now);

        assert_eq!(rows.len(), pages.len());
        let chunks = rows.chunks(DEFAULT_WRITE_CHUNK).collect::<Vec<_>>();
        assert_eq!(chunks.len(), 3);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.len() <= DEFAULT_WRITE_CHUNK)
        );
    }

    #[test]
    fn instantiated_page_rows_are_scoped_to_the_created_board() {
        let now = chrono::Utc::now().fixed_offset();
        let rows = instantiated_page_rows("app", "board", &[page("a")], now);
        let row = rows.first().expect("one row");
        assert_eq!(row.app_id, Set("app".to_string()));
        assert_eq!(row.board_id, Set(Some("board".to_string())));
        assert_eq!(row.created_at, Set(now));
    }

    #[test]
    fn template_on_existing_board_is_rejected_not_ignored() {
        let error = ensure_template_targets_new_board(false, true, "app", "board")
            .expect_err("template on an existing board must fail");
        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(error.public_code(), "BOARD_EXISTS");
        assert!(
            error
                .public_message()
                .is_some_and(|message| message.contains("nothing was written"))
        );
    }

    #[test]
    fn metadata_updates_and_template_creates_pass() {
        assert!(ensure_template_targets_new_board(false, false, "app", "board").is_ok());
        assert!(ensure_template_targets_new_board(true, true, "app", "board").is_ok());
        assert!(ensure_template_targets_new_board(true, false, "app", "board").is_ok());
    }
}
