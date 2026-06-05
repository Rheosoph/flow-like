use crate::{
    entity::app, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::app::board::scoring::persist_board_score, state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecomputeScoresRequest {
    /// Optional app id to scope the recompute. When omitted, all apps are recomputed.
    pub app_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecomputeScoresResponse {
    pub apps_processed: u64,
    pub boards_processed: u64,
    pub failures: u64,
}

#[utoipa::path(
    post,
    path = "/admin/governance/scores/recompute",
    tag = "admin",
    description = "Recompute and persist board governance scores for a single app or the entire platform (admin only).",
    request_body = RecomputeScoresRequest,
    responses(
        (status = 200, description = "Recompute summary", body = RecomputeScoresResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "POST /admin/governance/scores/recompute", skip(state, user))]
pub async fn recompute_scores(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<RecomputeScoresRequest>,
) -> Result<Json<RecomputeScoresResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let app_ids: Vec<String> = match payload.app_id {
        Some(app_id) => vec![app_id],
        None => app::Entity::find()
            .all(&state.db)
            .await?
            .into_iter()
            .map(|a| a.id)
            .collect(),
    };

    let mut apps_processed = 0u64;
    let mut boards_processed = 0u64;
    let mut failures = 0u64;

    for app_id in app_ids {
        let app = match state.master_app("admin", &app_id, &state).await {
            Ok(app) => app,
            Err(err) => {
                tracing::warn!("failed to load app {app_id} for recompute: {err:?}");
                failures += 1;
                continue;
            }
        };
        apps_processed += 1;

        for board_id in app.boards.iter() {
            let board = match app.open_board(board_id.clone(), Some(false), None).await {
                Ok(board) => board,
                Err(err) => {
                    tracing::warn!("failed to open board {board_id} of {app_id}: {err:?}");
                    failures += 1;
                    continue;
                }
            };
            let board = board.lock().await;
            if let Err(err) = persist_board_score(&state.db, &app_id, &board).await {
                tracing::warn!("failed to persist score for {app_id}/{board_id}: {err:?}");
                failures += 1;
                continue;
            }
            boards_processed += 1;
        }
    }

    Ok(Json(RecomputeScoresResponse {
        apps_processed,
        boards_processed,
        failures,
    }))
}
