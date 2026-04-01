use crate::{
    ensure_permission, entity::page, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardScores {
    pub security: u8,
    pub privacy: u8,
    pub performance: u8,
    pub governance: u8,
    pub reliability: u8,
    pub cost: u8,
}

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

    let mut summaries = Vec::with_capacity(app.boards.len());
    for board_id in app.boards.iter() {
        let board = match app.open_board(board_id.clone(), Some(false), None).await {
            Ok(b) => b,
            Err(_) => continue,
        };
        let board = board.lock().await;

        let mut node_count = 0u32;
        let mut connection_count = 0u32;
        let mut min_scores: Option<BoardScores> = None;

        for node in board.nodes.values() {
            if node.name == "reroute" {
                continue;
            }
            node_count += 1;
            for pin in node.pins.values() {
                connection_count += pin.connected_to.len() as u32;
            }

            if let Some(ref s) = node.scores {
                min_scores = Some(match min_scores {
                    None => BoardScores {
                        security: s.security,
                        privacy: s.privacy,
                        performance: s.performance,
                        governance: s.governance,
                        reliability: s.reliability,
                        cost: s.cost,
                    },
                    Some(prev) => BoardScores {
                        security: prev.security.min(s.security),
                        privacy: prev.privacy.min(s.privacy),
                        performance: prev.performance.min(s.performance),
                        governance: prev.governance.min(s.governance),
                        reliability: prev.reliability.min(s.reliability),
                        cost: prev.cost.min(s.cost),
                    },
                });
            }
        }
        connection_count /= 2;

        summaries.push(BoardSummary {
            id: board.id.clone(),
            name: board.name.clone(),
            description: board.description.clone(),
            stage: board.stage.clone(),
            execution_mode: board.execution_mode.clone(),
            log_level: board.log_level,
            version: board.version,
            node_count,
            connection_count,
            variable_count: board.variables.len() as u32,
            layer_count: board.layers.len() as u32,
            comment_count: board.comments.len() as u32,
            scores: min_scores,
            pages: pages_by_board.remove(&board.id).unwrap_or_default(),
        });
    }

    Ok(Json(summaries))
}
