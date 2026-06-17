use crate::{
    entity::{app_board_score, meta},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(FromQueryResult)]
struct AppScoreAgg {
    app_id: String,
    security: i32,
    privacy: i32,
    worst_score: i32,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorstAppItem {
    pub app_id: String,
    pub app_name: Option<String>,
    pub worst_score: i32,
    pub security: i32,
    pub privacy: i32,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceScoresSummary {
    pub critical_apps: u64,
    pub flagged_apps: u64,
    pub total_apps: u64,
    pub worst_apps: Vec<WorstAppItem>,
}

#[utoipa::path(
    get,
    path = "/admin/governance/scores/summary",
    tag = "admin",
    description = "Summary of governance scores for the admin dashboard: counts of critical/flagged apps and top worst apps requiring attention.",
    responses(
        (status = 200, description = "Governance scores summary", body = GovernanceScoresSummary),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /admin/governance/scores/summary", skip(state, user))]
pub async fn get_scores_summary(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<GovernanceScoresSummary>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    // Aggregate per-app worst scores in SQL.
    let aggregates: Vec<AppScoreAgg> = app_board_score::Entity::find()
        .select_only()
        .column_as(app_board_score::Column::AppId, "app_id")
        .column_as(
            Expr::col(app_board_score::Column::Security).min(),
            "security",
        )
        .column_as(Expr::col(app_board_score::Column::Privacy).min(), "privacy")
        .column_as(
            Expr::col(app_board_score::Column::WorstScore).min(),
            "worst_score",
        )
        .group_by(app_board_score::Column::AppId)
        .into_model::<AppScoreAgg>()
        .all(&state.db)
        .await?;

    let total_apps = aggregates.len() as u64;
    let critical_apps = aggregates.iter().filter(|a| a.worst_score <= 3).count() as u64;
    let flagged_apps = aggregates
        .iter()
        .filter(|a| a.worst_score <= 6 && a.worst_score > 3)
        .count() as u64;

    // Get worst apps (limit to 5 with worst_score <= 6).
    let mut worst: Vec<AppScoreAgg> = aggregates
        .into_iter()
        .filter(|a| a.worst_score <= 6)
        .collect();
    worst.sort_by_key(|a| a.worst_score);
    worst.truncate(5);

    // Resolve app names for display.
    let app_ids: Vec<String> = worst.iter().map(|a| a.app_id.clone()).collect();
    let names: HashMap<String, String> = if app_ids.is_empty() {
        HashMap::new()
    } else {
        meta::Entity::find()
            .filter(meta::Column::AppId.is_in(app_ids))
            .filter(meta::Column::Lang.eq("en"))
            .all(&state.db)
            .await?
            .into_iter()
            .filter_map(|m| m.app_id.clone().map(|aid| (aid, m.name)))
            .collect()
    };

    let worst_apps = worst
        .into_iter()
        .map(|a| WorstAppItem {
            app_name: names.get(&a.app_id).cloned(),
            app_id: a.app_id,
            worst_score: a.worst_score,
            security: a.security,
            privacy: a.privacy,
        })
        .collect();

    Ok(Json(GovernanceScoresSummary {
        critical_apps,
        flagged_apps,
        total_apps,
        worst_apps,
    }))
}
