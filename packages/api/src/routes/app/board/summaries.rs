use crate::{
    ensure_permission, entity::app_board_score, entity::page, error::ApiError,
    middleware::jwt::AppUser, permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::board::{ExecutionMode, ExecutionStage};
use flow_like::flow::execution::LogLevel;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use std::collections::HashMap;

use super::super::page::get_pages::PageInfo;
use super::scoring::{
    BoardScores, board_summary_meta, compute_board_score, persist_board_score_with,
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub stage: ExecutionStage,
    pub execution_mode: ExecutionMode,
    pub log_level: LogLevel,
    pub version: (u32, u32, u32),
    pub node_count: u32,
    pub connection_count: u32,
    pub variable_count: u32,
    pub layer_count: u32,
    pub comment_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scores: Option<BoardScores>,
    pub pages: Vec<PageInfo>,
}

/// Reconstruct the persisted score columns into [`BoardScores`].
/// Returns `None` when the board has no scored nodes (matching live computation).
fn scores_from_row(row: &app_board_score::Model) -> Option<BoardScores> {
    if row.scored_node_count <= 0 {
        return None;
    }
    Some(BoardScores {
        security: row.security as u8,
        privacy: row.privacy as u8,
        performance: row.performance as u8,
        governance: row.governance as u8,
        reliability: row.reliability as u8,
        cost: row.cost as u8,
    })
}

/// Build a [`BoardSummary`] from a cached DB row. Returns `None` when the row
/// predates the cached `summary` metadata so the caller can fall back to S3.
fn summary_from_row(row: &app_board_score::Model, pages: Vec<PageInfo>) -> Option<BoardSummary> {
    let meta = row.summary.clone()?;

    Some(BoardSummary {
        id: row.board_id.clone(),
        name: meta.name,
        description: meta.description,
        stage: meta.stage,
        execution_mode: meta.execution_mode,
        log_level: meta.log_level,
        version: meta.version,
        node_count: row.node_count.max(0) as u32,
        connection_count: meta.connection_count,
        variable_count: meta.variable_count,
        layer_count: meta.layer_count,
        comment_count: meta.comment_count,
        scores: scores_from_row(row),
        pages,
    })
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/summaries",
    tag = "boards",
    description = "Get lightweight summaries for all boards including stats, scores, and pages",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Board summaries with stats and pages", body = Vec<Object>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/board/summaries", skip(state, user))]
pub async fn board_summaries(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<BoardSummary>>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    let app = state.master_app(&sub, &app_id, &state).await?;

    let all_pages: Vec<page::Model> = page::Entity::find()
        .filter(page::Column::AppId.eq(&app_id))
        .all(&state.db)
        .await?;

    let mut pages_by_board: HashMap<String, Vec<PageInfo>> = HashMap::new();
    for p in all_pages {
        if let Some(ref board_id) = p.board_id {
            pages_by_board
                .entry(board_id.clone())
                .or_default()
                .push(PageInfo {
                    app_id: app_id.clone(),
                    page_id: p.id,
                    board_id: p.board_id,
                    name: p.name,
                    description: p.description,
                });
        }
    }

    // Cached board metadata (scores + summary) avoids reading every board from
    // object storage. Rows missing the cached `summary` fall back to S3.
    let mut cached: HashMap<String, app_board_score::Model> = app_board_score::Entity::find()
        .filter(app_board_score::Column::AppId.eq(&app_id))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|row| (row.board_id.clone(), row))
        .collect();

    let mut summaries = Vec::with_capacity(app.boards.len());
    for board_id in app.boards.iter() {
        let pages = pages_by_board.remove(board_id).unwrap_or_default();

        // Fast path: serve from the DB cache.
        if let Some(row) = cached.remove(board_id) {
            if let Some(summary) = summary_from_row(&row, pages.clone()) {
                summaries.push(summary);
                continue;
            }
        }

        // Backwards-compatible fallback: load from S3, compute, and patch the DB.
        let board = match app.open_board(board_id.clone(), Some(false), None).await {
            Ok(b) => b,
            Err(_) => continue,
        };
        let board = board.lock().await;

        let computation = compute_board_score(&board);
        let meta = board_summary_meta(&board, computation.connection_count);

        if let Err(err) = persist_board_score_with(&state.db, &app_id, &board, &computation).await {
            tracing::warn!("failed to backfill board score for {app_id}/{board_id}: {err:?}");
        }

        summaries.push(BoardSummary {
            id: board.id.clone(),
            name: meta.name,
            description: meta.description,
            stage: meta.stage,
            execution_mode: meta.execution_mode,
            log_level: meta.log_level,
            version: meta.version,
            node_count: computation.node_count,
            connection_count: computation.connection_count,
            variable_count: meta.variable_count,
            layer_count: meta.layer_count,
            comment_count: meta.comment_count,
            scores: computation.scores,
            pages,
        });
    }

    Ok(Json(summaries))
}
