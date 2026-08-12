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
use utoipa::{IntoParams, ToSchema};

#[derive(FromQueryResult)]
struct AppScoreAgg {
    app_id: String,
    security: i32,
    privacy: i32,
    performance: i32,
    governance: i32,
    reliability: i32,
    cost: i32,
    worst_score: i32,
    board_count: i64,
    node_count: i64,
    scored_node_count: i64,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppScoreItem {
    pub app_id: String,
    pub app_name: Option<String>,
    pub security: i32,
    pub privacy: i32,
    pub performance: i32,
    pub governance: i32,
    pub reliability: i32,
    pub cost: i32,
    pub worst_score: i32,
    pub board_count: i64,
    pub node_count: i64,
    pub scored_node_count: i64,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListAppScoresResponse {
    pub apps: Vec<AppScoreItem>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
    pub has_more: bool,
}

#[derive(Clone, Deserialize, Debug, IntoParams, ToSchema)]
pub struct ListAppScoresQuery {
    /// Free-text search over app id or localized app name.
    pub search: Option<String>,
    /// Only include apps whose score in this category is at or below `threshold`.
    pub category: Option<String>,
    /// Upper bound for the selected `category` (or `worstScore` when no category given).
    pub threshold: Option<i32>,
    /// Sort key: one of the six categories, `worstScore` (default).
    pub sort: Option<String>,
    /// Sort direction: `asc` (default, worst first) or `desc`.
    pub direction: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
}

fn category_value(item: &AppScoreItem, key: &str) -> i32 {
    match key {
        "security" => item.security,
        "privacy" => item.privacy,
        "performance" => item.performance,
        "governance" => item.governance,
        "reliability" => item.reliability,
        "cost" => item.cost,
        _ => item.worst_score,
    }
}

#[utoipa::path(
    get,
    path = "/admin/governance/scores",
    tag = "admin",
    description = "List apps with their persisted governance/quality scores, aggregated from board scores. Sorted worst-first by default.",
    params(ListAppScoresQuery),
    responses(
        (status = 200, description = "Ranked app scores", body = ListAppScoresResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /admin/governance/scores", skip_all)]
pub async fn list_scores(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    axum::extract::Query(query): axum::extract::Query<ListAppScoresQuery>,
) -> Result<Json<ListAppScoresResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(25).clamp(1, 100);

    // Aggregate per-app scores in SQL (MIN per category, MIN worst, counts).
    let aggregates: Vec<AppScoreAgg> = app_board_score::Entity::find()
        .select_only()
        .column_as(app_board_score::Column::AppId, "app_id")
        .column_as(
            Expr::col(app_board_score::Column::Security).min(),
            "security",
        )
        .column_as(Expr::col(app_board_score::Column::Privacy).min(), "privacy")
        .column_as(
            Expr::col(app_board_score::Column::Performance).min(),
            "performance",
        )
        .column_as(
            Expr::col(app_board_score::Column::Governance).min(),
            "governance",
        )
        .column_as(
            Expr::col(app_board_score::Column::Reliability).min(),
            "reliability",
        )
        .column_as(Expr::col(app_board_score::Column::Cost).min(), "cost")
        .column_as(
            Expr::col(app_board_score::Column::WorstScore).min(),
            "worst_score",
        )
        .column_as(
            Expr::col(app_board_score::Column::BoardId).count(),
            "board_count",
        )
        .column_as(
            Expr::col(app_board_score::Column::NodeCount).sum(),
            "node_count",
        )
        .column_as(
            Expr::col(app_board_score::Column::ScoredNodeCount).sum(),
            "scored_node_count",
        )
        .group_by(app_board_score::Column::AppId)
        .into_model::<AppScoreAgg>()
        .all(&state.db)
        .await?;

    // Resolve localized (English) app names for display & search.
    let app_ids: Vec<String> = aggregates.iter().map(|a| a.app_id.clone()).collect();
    let names: HashMap<String, String> = if app_ids.is_empty() {
        HashMap::new()
    } else {
        meta::Entity::find()
            .filter(meta::Column::AppId.is_in(app_ids.clone()))
            .filter(meta::Column::Lang.eq("en"))
            .all(&state.db)
            .await?
            .into_iter()
            .filter_map(|m| m.app_id.clone().map(|aid| (aid, m.name)))
            .collect()
    };

    let mut items: Vec<AppScoreItem> = aggregates
        .into_iter()
        .map(|a| AppScoreItem {
            app_name: names.get(&a.app_id).cloned(),
            app_id: a.app_id,
            security: a.security,
            privacy: a.privacy,
            performance: a.performance,
            governance: a.governance,
            reliability: a.reliability,
            cost: a.cost,
            worst_score: a.worst_score,
            board_count: a.board_count,
            node_count: a.node_count,
            scored_node_count: a.scored_node_count,
        })
        .collect();

    // Search filter (app id or name, case-insensitive).
    if let Some(search) = query.search.as_ref().map(|s| s.trim().to_lowercase())
        && !search.is_empty()
    {
        items.retain(|item| {
            item.app_id.to_lowercase().contains(&search)
                || item
                    .app_name
                    .as_ref()
                    .map(|n| n.to_lowercase().contains(&search))
                    .unwrap_or(false)
        });
    }

    // Category / threshold filter.
    if let Some(threshold) = query.threshold {
        let key = query
            .category
            .clone()
            .unwrap_or_else(|| "worstScore".into());
        items.retain(|item| category_value(item, &key) <= threshold);
    }

    // Sort (default: worstScore ascending = worst first).
    let sort_key = query.sort.clone().unwrap_or_else(|| "worstScore".into());
    let descending = query
        .direction
        .as_deref()
        .map(|d| d.eq_ignore_ascii_case("desc"))
        .unwrap_or(false);
    items.sort_by(|a, b| {
        let ord = category_value(a, &sort_key)
            .cmp(&category_value(b, &sort_key))
            .then_with(|| a.app_id.cmp(&b.app_id));
        if descending { ord.reverse() } else { ord }
    });

    let total = items.len() as u64;
    let offset = ((page - 1) * limit) as usize;
    let paged: Vec<AppScoreItem> = items
        .into_iter()
        .skip(offset)
        .take(limit as usize)
        .collect();
    let has_more = (offset as u64) + (paged.len() as u64) < total;

    Ok(Json(ListAppScoresResponse {
        apps: paged,
        total,
        page,
        limit,
        has_more,
    }))
}
