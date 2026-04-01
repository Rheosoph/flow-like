use crate::{
    entity::page,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::app::{
        board::summaries::{BoardScores, BoardSummary},
        events::db::{filter_event_list_execution, filter_event_secrets, get_events_for_app},
        page::get_pages::PageInfo,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub board_id: String,
    pub event_type: String,
    pub active: bool,
    pub priority: u32,
    pub route: Option<String>,
    pub is_default: bool,
    pub version: (u32, u32, u32),
    pub default_page_id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppContentResponse {
    pub boards: Vec<BoardSummary>,
    pub events: Vec<EventSummary>,
    pub pages: Vec<PageInfo>,
}

#[utoipa::path(
    get,
    path = "/admin/publication/apps/{app_id}/content",
    tag = "admin",
    description = "Get board summaries, events, and pages for an app under publication review.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "App content overview", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "GET /admin/publication/apps/{app_id}/content",
    skip(state, user)
)]
pub async fn get_app_content(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<AppContentResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    // Load the app from master storage
    let app = state.master_app("admin", &app_id, &state).await?;

    // Fetch pages from DB
    let all_pages: Vec<page::Model> = page::Entity::find()
        .filter(page::Column::AppId.eq(&app_id))
        .all(&state.db)
        .await?;

    let pages: Vec<PageInfo> = all_pages
        .iter()
        .map(|p| PageInfo {
            app_id: app_id.clone(),
            page_id: p.id.clone(),
            board_id: p.board_id.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
        })
        .collect();

    let mut pages_by_board: HashMap<String, Vec<PageInfo>> = HashMap::new();
    for p in &all_pages {
        if let Some(ref board_id) = p.board_id {
            pages_by_board
                .entry(board_id.clone())
                .or_default()
                .push(PageInfo {
                    app_id: app_id.clone(),
                    page_id: p.id.clone(),
                    board_id: p.board_id.clone(),
                    name: p.name.clone(),
                    description: p.description.clone(),
                });
        }
    }

    // Build board summaries
    let mut boards = Vec::with_capacity(app.boards.len());
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

        boards.push(BoardSummary {
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

    // Fetch events and strip secrets
    let events: Vec<EventSummary> = get_events_for_app(&state.db, &app_id)
        .await?
        .into_iter()
        .map(filter_event_secrets)
        .map(filter_event_list_execution)
        .map(|e| EventSummary {
            id: e.id,
            name: e.name,
            description: e.description,
            board_id: e.board_id,
            event_type: e.event_type,
            active: e.active,
            priority: e.priority,
            route: e.route,
            is_default: e.is_default,
            version: e.event_version,
            default_page_id: e.default_page_id,
        })
        .collect();

    Ok(Json(AppContentResponse {
        boards,
        events,
        pages,
    }))
}
