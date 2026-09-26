use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{
    discounts::{DiscountResponse, discounts_for_app},
    overview::{
        SalesOverview, SalesStats, StatsQuery, count_members, sales_overview, sales_stats,
        verify_sales_access,
    },
    purchases::{PurchasesQuery, PurchasesResponse, purchases_page},
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
    // One access check, app row and member count for all four sections.
    let app = verify_sales_access(&state, &user, &app_id).await?;
    let total_members = count_members(&state, &app_id).await?;

    let overview = sales_overview(&state, &app_id, app.price, total_members).await?;

    let stats = sales_stats(
        &state,
        &app_id,
        app.price,
        total_members,
        StatsQuery {
            start_date: query.start_date,
            end_date: query.end_date,
            period: query.period,
        },
    )
    .await?;

    let recent_purchases = purchases_page(
        &state,
        &app_id,
        PurchasesQuery {
            status: None,
            offset: query.purchases_offset,
            limit: query.purchases_limit,
        },
    )
    .await?;

    let discounts = discounts_for_app(&state, &app_id, false).await?;

    Ok(Json(SalesDashboardResponse {
        overview,
        stats,
        recent_purchases,
        discounts,
    }))
}
