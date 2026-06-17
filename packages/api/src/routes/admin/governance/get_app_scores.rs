use crate::{
    entity::{app_board_score, meta},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::app::board::scoring::FlaggedPattern,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoardScoreItem {
    pub board_id: String,
    pub security: i32,
    pub privacy: i32,
    pub performance: i32,
    pub governance: i32,
    pub reliability: i32,
    pub cost: i32,
    pub worst_score: i32,
    pub node_count: i32,
    pub scored_node_count: i32,
    pub flagged_patterns: Vec<FlaggedPattern>,
    pub computed_at: String,
    pub updated_at: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppScoreDetailResponse {
    pub app_id: String,
    pub app_name: Option<String>,
    pub boards: Vec<BoardScoreItem>,
}

#[utoipa::path(
    get,
    path = "/admin/governance/scores/{app_id}",
    tag = "admin",
    description = "Per-board governance score breakdown for a single app, including flagged low-score nodes.",
    params(("app_id" = String, Path, description = "App id")),
    responses(
        (status = 200, description = "Per-board score breakdown", body = AppScoreDetailResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /admin/governance/scores/{app_id}", skip(state, user))]
pub async fn get_app_scores(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<AppScoreDetailResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    let rows = app_board_score::Entity::find()
        .filter(app_board_score::Column::AppId.eq(app_id.clone()))
        .order_by_asc(app_board_score::Column::WorstScore)
        .all(&state.db)
        .await?;

    let app_name = meta::Entity::find()
        .filter(meta::Column::AppId.eq(app_id.clone()))
        .filter(meta::Column::Lang.eq("en"))
        .one(&state.db)
        .await?
        .map(|m| m.name);

    let boards = rows
        .into_iter()
        .map(|row| {
            let flagged_patterns = row
                .flagged_patterns
                .as_ref()
                .and_then(|json| serde_json::from_value(json.clone()).ok())
                .unwrap_or_default();
            BoardScoreItem {
                board_id: row.board_id,
                security: row.security,
                privacy: row.privacy,
                performance: row.performance,
                governance: row.governance,
                reliability: row.reliability,
                cost: row.cost,
                worst_score: row.worst_score,
                node_count: row.node_count,
                scored_node_count: row.scored_node_count,
                flagged_patterns,
                computed_at: row.computed_at.and_utc().to_rfc3339(),
                updated_at: row.updated_at.and_utc().to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(AppScoreDetailResponse {
        app_id,
        app_name,
        boards,
    }))
}
