use crate::{
    ensure_permission, entity::app_board_score, entity::page, error::ApiError,
    middleware::jwt::AppUser, permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::board::{ExecutionMode, ExecutionStage};
use flow_like::flow::execution::LogLevel;
use futures::{StreamExt, stream};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use super::super::page::get_pages::PageInfo;
use super::scoring::{
    BoardScores, BoardSummaryMeta, FlaggedPattern, board_summary_meta, compute_board_score,
    persist_board_score_with,
};
use flow_like::flow::board::summary::{BoardEntryNode, BoardSummaryMetrics};

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoardSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    #[schema(value_type = String)]
    pub stage: ExecutionStage,
    #[schema(value_type = String)]
    pub execution_mode: ExecutionMode,
    #[schema(value_type = i32)]
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
    /// Only present when requested with `include=node_types`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_types: Option<Vec<String>>,
    /// Only present when requested with `include=node_types`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub entry_nodes: Option<Vec<BoardEntryNode>>,
    /// The board's last modification time. Absent only for summaries cached before it was
    /// recorded; those refresh on the next score persist.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub updated_at: Option<std::time::SystemTime>,
    /// Only present when requested with `include=metrics`: how many scorable nodes carry scores.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scored_node_count: Option<u32>,
    /// Only present when requested with `include=metrics`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flagged_patterns: Option<Vec<FlaggedPattern>>,
    /// Only present when requested with `include=metrics`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub metrics: Option<BoardSummaryMetrics>,
}

#[derive(Deserialize, ToSchema, Default)]
pub struct SummariesQuery {
    /// Comma-separated extras. Supported: `node_types` (distinct node type names + entry nodes),
    /// `metrics` (scored node count, flagged patterns, wasm/variable/layer breakdowns).
    pub include: Option<String>,
}

impl SummariesQuery {
    fn wants(&self, extra: &str) -> bool {
        self.include
            .as_deref()
            .map(|include| include.split(',').any(|part| part.trim() == extra))
            .unwrap_or(false)
    }
}

#[derive(Clone, Copy)]
struct Includes {
    node_types: bool,
    metrics: bool,
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

#[allow(clippy::too_many_arguments)]
fn summary_from_meta(
    board_id: &str,
    meta: BoardSummaryMeta,
    node_count: u32,
    scores: Option<BoardScores>,
    pages: Vec<PageInfo>,
    includes: Includes,
    scored_node_count: u32,
    flagged_patterns: Vec<FlaggedPattern>,
) -> BoardSummary {
    let with_node_types = includes.node_types;
    BoardSummary {
        id: board_id.to_string(),
        name: meta.name,
        description: meta.description,
        stage: meta.stage,
        execution_mode: meta.execution_mode,
        log_level: meta.log_level,
        version: meta.version,
        node_count,
        connection_count: meta.connection_count,
        variable_count: meta.variable_count,
        layer_count: meta.layer_count,
        comment_count: meta.comment_count,
        scores,
        pages,
        node_types: with_node_types.then_some(meta.node_types).flatten(),
        entry_nodes: with_node_types.then_some(meta.entry_nodes).flatten(),
        updated_at: meta.updated_at,
        scored_node_count: includes.metrics.then_some(scored_node_count),
        flagged_patterns: includes.metrics.then_some(flagged_patterns),
        metrics: includes.metrics.then_some(meta.metrics).flatten(),
    }
}

/// Build a [`BoardSummary`] from a cached DB row. Returns `None` when the row
/// predates the cached `summary` metadata (or, when node types were requested,
/// predates that field) so the caller can fall back to S3 and backfill.
fn summary_from_row(
    row: &app_board_score::Model,
    pages: Vec<PageInfo>,
    includes: Includes,
) -> Option<BoardSummary> {
    let meta: BoardSummaryMeta = serde_json::from_value(row.summary.clone()?).ok()?;
    if includes.node_types && (meta.node_types.is_none() || meta.entry_nodes.is_none()) {
        return None;
    }
    if includes.metrics && meta.metrics.is_none() {
        return None;
    }
    let flagged_patterns: Vec<FlaggedPattern> = if includes.metrics {
        row.flagged_patterns
            .clone()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Some(summary_from_meta(
        &row.board_id,
        meta,
        row.node_count.max(0) as u32,
        scores_from_row(row),
        pages,
        includes,
        row.scored_node_count.max(0) as u32,
        flagged_patterns,
    ))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/summaries",
    tag = "boards",
    description = "Get lightweight summaries for all boards including stats, scores, and pages. Add `include=node_types` for distinct node types and entry nodes, `include=metrics` for scored node count, flagged patterns and wasm/variable/layer breakdowns.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("include" = Option<String>, Query, description = "Comma-separated extras: node_types, metrics")
    ),
    responses(
        (status = 200, description = "Board summaries with stats and pages", body = Vec<BoardSummary>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/board/summaries", skip(state, user, query))]
pub async fn board_summaries(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<SummariesQuery>,
) -> Result<Json<Vec<BoardSummary>>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;
    let includes = Includes {
        node_types: query.wants("node_types"),
        metrics: query.wants("metrics"),
    };

    let app = state.master_app(&sub, &app_id, &state).await?;

    let all_pages: Vec<page::Model> = page::Entity::find()
        .filter(page::Column::AppId.eq(&app_id))
        .all(&state.db)
        .await?;

    let mut pages_by_board: HashMap<String, Vec<PageInfo>> = HashMap::new();
    for p in &all_pages {
        if let Some(ref board_id) = p.board_id {
            pages_by_board
                .entry(board_id.clone())
                .or_default()
                .push(PageInfo::from_row(&app_id, p));
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

    // Fast path: serve every board that has a complete cached row. Boards without one (never
    // persisted, or persisted before a field this request needs existed) are collected for the
    // storage fallback below.
    let mut summaries: Vec<Option<BoardSummary>> = vec![None; app.boards.len()];
    let mut missing: Vec<(usize, String, Vec<PageInfo>)> = Vec::new();
    for (index, board_id) in app.boards.iter().enumerate() {
        let pages = pages_by_board.remove(board_id).unwrap_or_default();
        if let Some(row) = cached.remove(board_id)
            && let Some(summary) = summary_from_row(&row, pages.clone(), includes)
        {
            summaries[index] = Some(summary);
            continue;
        }
        missing.push((index, board_id.clone(), pages));
    }

    // Backwards-compatible fallback: load from storage, compute, and patch the DB so the next
    // request is served from the row. Loads go through the ETag-validated board cache and run a
    // few at a time — an app with many large, never-edited boards must not turn its first
    // summaries request into a serial walk over all of them.
    const FALLBACK_CONCURRENCY: usize = 4;
    let backfilled: Vec<Option<(usize, BoardSummary)>> = stream::iter(missing)
        .map(|(index, board_id, pages)| {
            let state = state.clone();
            let app_id = app_id.clone();
            async move {
                let cached = state
                    .master_board_shared(&app_id, &board_id, &state, None)
                    .await
                    .ok()?;
                let board = &cached.board;
                let computation = compute_board_score(board);
                let meta = board_summary_meta(board, computation.connection_count);
                if let Err(err) =
                    persist_board_score_with(&state.db, &app_id, board, &computation).await
                {
                    tracing::warn!(
                        "failed to backfill board score for {app_id}/{board_id}: {err:?}"
                    );
                }
                Some((
                    index,
                    summary_from_meta(
                        &board.id,
                        meta,
                        computation.node_count,
                        computation.scores,
                        pages,
                        includes,
                        computation.scored_node_count,
                        computation.flagged_patterns,
                    ),
                ))
            }
        })
        .buffer_unordered(FALLBACK_CONCURRENCY)
        .collect()
        .await;
    for (index, summary) in backfilled.into_iter().flatten() {
        summaries[index] = Some(summary);
    }

    Ok(Json(summaries.into_iter().flatten().collect()))
}
