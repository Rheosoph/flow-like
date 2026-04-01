use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::overview::{AnalyticsOverview, AnalyticsStats, AnalyticsStatsQuery};

#[derive(Debug, Deserialize, IntoParams)]
pub struct AnalyticsDashboardQuery {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    #[serde(default = "default_period")]
    pub period: String,
}

fn default_period() -> String {
    "day".to_string()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AnalyticsDashboardResponse {
    pub overview: AnalyticsOverview,
    pub stats: AnalyticsStats,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/analytics/dashboard",
    tag = "analytics",
    description = "Get combined analytics dashboard: overview + daily stats in a single request.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        AnalyticsDashboardQuery
    ),
    responses(
        (status = 200, description = "Analytics dashboard data", body = AnalyticsDashboardResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/analytics/dashboard", skip(state, user))]
pub async fn dashboard(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<AnalyticsDashboardQuery>,
) -> Result<Json<AnalyticsDashboardResponse>, ApiError> {
    let overview = super::overview::get_analytics_overview(
        State(state.clone()),
        Extension(user.clone()),
        Path(app_id.clone()),
    )
    .await?
    .0;

    let stats = super::overview::get_analytics_stats(
        State(state.clone()),
        Extension(user.clone()),
        Path(app_id.clone()),
        Query(AnalyticsStatsQuery {
            start_date: query.start_date,
            end_date: query.end_date,
            period: query.period,
        }),
    )
    .await?
    .0;

    Ok(Json(AnalyticsDashboardResponse { overview, stats }))
}
