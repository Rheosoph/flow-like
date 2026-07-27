use crate::{
    entity::app_board_score, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, routes::app::board::scoring::FlaggedPattern,
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PatternItem {
    pub node: String,
    pub category: String,
    /// Number of distinct apps where this node/category was flagged.
    pub app_count: u64,
    /// Total number of flagged occurrences across all boards.
    pub occurrence_count: u64,
    /// Lowest score observed for this node/category.
    pub min_score: i32,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListPatternsResponse {
    pub patterns: Vec<PatternItem>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
    pub has_more: bool,
}

#[derive(Clone, Deserialize, Debug, IntoParams, ToSchema)]
pub struct ListPatternsQuery {
    /// Free-text search over node name or category.
    pub search: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Default)]
struct PatternAgg {
    apps: HashSet<String>,
    occurrences: u64,
    min_score: i32,
}

#[derive(sea_orm::FromQueryResult)]
struct FlaggedRow {
    app_id: String,
    flagged_patterns: Option<sea_orm::JsonValue>,
}

#[utoipa::path(
    get,
    path = "/admin/governance/patterns",
    tag = "admin",
    description = "Platform-wide aggregation of flagged low-score node patterns across all apps, searchable by node or category.",
    params(ListPatternsQuery),
    responses(
        (status = 200, description = "Aggregated flagged patterns", body = ListPatternsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /admin/governance/patterns", skip_all)]
pub async fn list_patterns(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    axum::extract::Query(query): axum::extract::Query<ListPatternsQuery>,
) -> Result<Json<ListPatternsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(25).clamp(1, 100);

    let rows: Vec<FlaggedRow> = app_board_score::Entity::find()
        .select_only()
        .column_as(app_board_score::Column::AppId, "app_id")
        .column_as(app_board_score::Column::FlaggedPatterns, "flagged_patterns")
        .filter(app_board_score::Column::FlaggedPatterns.is_not_null())
        .into_model::<FlaggedRow>()
        .all(&state.db)
        .await?;

    let mut aggregates: HashMap<(String, String), PatternAgg> = HashMap::new();
    for row in rows {
        let Some(json) = row.flagged_patterns else {
            continue;
        };
        let patterns: Vec<FlaggedPattern> = match serde_json::from_value(json) {
            Ok(patterns) => patterns,
            Err(_) => continue,
        };
        for pattern in patterns {
            let entry = aggregates
                .entry((pattern.node.clone(), pattern.category.clone()))
                .or_insert_with(|| PatternAgg {
                    min_score: i32::from(pattern.score),
                    ..Default::default()
                });
            entry.apps.insert(row.app_id.clone());
            entry.occurrences += u64::from(pattern.count.max(1));
            entry.min_score = entry.min_score.min(i32::from(pattern.score));
        }
    }

    let mut items: Vec<PatternItem> = aggregates
        .into_iter()
        .map(|((node, category), agg)| PatternItem {
            node,
            category,
            app_count: agg.apps.len() as u64,
            occurrence_count: agg.occurrences,
            min_score: agg.min_score,
        })
        .collect();

    if let Some(search) = query.search.as_ref().map(|s| s.trim().to_lowercase()) {
        if !search.is_empty() {
            items.retain(|item| {
                item.node.to_lowercase().contains(&search)
                    || item.category.to_lowercase().contains(&search)
            });
        }
    }

    items.sort_by(|a, b| {
        a.min_score
            .cmp(&b.min_score)
            .then_with(|| b.app_count.cmp(&a.app_count))
            .then_with(|| a.node.cmp(&b.node))
    });

    let total = items.len() as u64;
    let offset = ((page - 1) * limit) as usize;
    let paged: Vec<PatternItem> = items
        .into_iter()
        .skip(offset)
        .take(limit as usize)
        .collect();
    let has_more = (offset as u64) + (paged.len() as u64) < total;

    Ok(Json(ListPatternsResponse {
        patterns: paged,
        total,
        page,
        limit,
        has_more,
    }))
}
