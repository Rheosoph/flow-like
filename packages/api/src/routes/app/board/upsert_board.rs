use crate::{
    audit_branch,
    db::{DEFAULT_WRITE_CHUNK, insert_in_chunks},
    ensure_permission,
    entity::page,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::board::secrets::filter_board_secrets,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::{
    a2ui::widget::Page,
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

    let rows = instantiated_page_rows(app_id, board_id, pages, chrono::Utc::now().naive_utc());

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
    now: chrono::NaiveDateTime,
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
    Json(params): Json<UpsertBoard>,
) -> Result<Json<UpsertBoardResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;
    let mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;

    let mut app = state.master_app(&sub, &app_id, &state).await?;
    let mut id = board_id.clone();
    let created_board = !app.boards.contains(&board_id);
    let had_template = params.template.is_some();
    if created_board {
        // Instantiate through the same core path as offline boards. Besides remapping node/pin
        // IDs, this runs catalog schema migrations and keeps compact schema refs resolvable before
        // the first board snapshot is saved.
        let created = app
            .create_board(Some(board_id.clone()), params.template)
            .await?;
        mutation_guard.ensure_held()?;
        app.save().await?;
        id = created.board_id;
        persist_instantiated_pages(&state, &app_id, &id, &created.pages).await?;
    }

    let board = app.open_board(id, Some(false), None).await?;
    let mut board = board.lock().await;

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

    if let Err(err) =
        crate::routes::app::board::scoring::persist_board_score(&state.db, &app_id, &board).await
    {
        tracing::warn!(
            "failed to persist board score for {app_id}/{}: {err:?}",
            board.id
        );
    }

    audit_branch!(
        state,
        user,
        app_id,
        "board.update",
        "Board",
        board.id,
        "Board created or updated"
    );
    let response_board = (created_board && had_template).then(|| {
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
        let now = chrono::Utc::now().naive_utc();
        let rows = instantiated_page_rows("app", "board", &pages, now);

        assert_eq!(rows.len(), pages.len());
        let chunks = rows.chunks(DEFAULT_WRITE_CHUNK).collect::<Vec<_>>();
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|chunk| chunk.len() <= DEFAULT_WRITE_CHUNK));
    }

    #[test]
    fn instantiated_page_rows_are_scoped_to_the_created_board() {
        let now = chrono::Utc::now().naive_utc();
        let rows = instantiated_page_rows("app", "board", &[page("a")], now);
        let row = rows.first().expect("one row");
        assert_eq!(row.app_id, Set("app".to_string()));
        assert_eq!(row.board_id, Set(Some("board".to_string())));
        assert_eq!(row.created_at, Set(now));
    }
}
