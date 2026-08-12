use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{
    discounts::{DiscountResponse, ListDiscountsQuery},
    overview::{SalesOverview, SalesStats, StatsQuery},
    purchases::PurchasesResponse,
};

#[derive(Debug, Deserialize, IntoParams)]
pub struct DashboardQuery {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    #[serde(default = "default_period")]
    pub period: String,
    #[serde(default = "default_purchases_limit")]
    pub purchases_limit: u64,
    #[serde(default)]
    pub purchases_offset: u64,
}

fn default_period() -> String {
    "day".to_string()
}

fn default_purchases_limit() -> u64 {
    50
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SalesDashboardResponse {
    pub overview: SalesOverview,
    pub stats: SalesStats,
    pub recent_purchases: PurchasesResponse,
    pub discounts: Vec<DiscountResponse>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/sales/dashboard",
    tag = "sales",
    description = "Get combined sales dashboard: overview + stats + purchases + discounts in a single request.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        DashboardQuery
    ),
    responses(
        (status = 200, description = "Sales dashboard data", body = SalesDashboardResponse),
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
#[tracing::instrument(name = "GET /apps/{app_id}/sales/dashboard", skip(state, user, query))]
pub async fn dashboard(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<SalesDashboardResponse>, ApiError> {
    // Delegate to existing handlers, reusing the same extractors
    let overview = super::overview::get_sales_overview(
        State(state.clone()),
        Extension(user.clone()),
        Path(app_id.clone()),
    )
    .await?
    .0;

    let stats = super::overview::get_sales_stats(
        State(state.clone()),
        Extension(user.clone()),
        Path(app_id.clone()),
        Query(StatsQuery {
            start_date: query.start_date,
            end_date: query.end_date,
            period: query.period,
        }),
    )
    .await?
    .0;

    let recent_purchases = super::purchases::list_purchases(
        State(state.clone()),
        Extension(user.clone()),
        Path(app_id.clone()),
        Query(super::purchases::PurchasesQuery {
            status: None,
            offset: query.purchases_offset,
            limit: query.purchases_limit,
        }),
    )
    .await?
    .0;

    let discounts = super::discounts::list_discounts(
        State(state.clone()),
        Extension(user.clone()),
        Path(app_id.clone()),
        Query(ListDiscountsQuery { active_only: false }),
    )
    .await?
    .0;

    Ok(Json(SalesDashboardResponse {
        overview,
        stats,
        recent_purchases,
        discounts,
    }))
}
