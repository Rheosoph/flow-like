use crate::{
    entity::{app, app_purchase, app_sales_daily, membership, sea_orm_active_enums::Visibility},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::{Duration, NaiveDate, Utc};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct StatsQuery {
    /// Start date for the stats period (YYYY-MM-DD)
    pub start_date: Option<String>,
    /// End date for the stats period (YYYY-MM-DD)
    pub end_date: Option<String>,
    /// Aggregation period: "day", "week", "month"
    #[serde(default = "default_period")]
    #[allow(dead_code)]
    // OpenAPI/serde query parameter — registered in openapi.rs, documented in utoipa params()
    pub period: String,
}

fn default_period() -> String {
    "day".to_string()
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SalesOverview {
    /// Total lifetime revenue (cents)
    pub total_revenue: i64,
    /// Total number of purchases
    pub total_purchases: i64,
    /// Total number of refunds
    pub total_refunds: i64,
    /// Total refund amount (cents)
    pub refund_amount: i64,
    /// Net revenue (total - refunds)
    pub net_revenue: i64,
    /// Total unique buyers
    pub unique_buyers: i64,
    /// Average order value (cents)
    pub avg_order_value: i64,
    /// Current price (cents)
    pub current_price: i64,
    /// Total discount amount given (cents)
    pub total_discounts: i64,
    /// Total team members
    pub total_members: i64,
    /// Revenue this period
    pub period_revenue: i64,
    /// Purchases this period
    pub period_purchases: i64,
    /// Period comparison (percentage change from previous period)
    pub revenue_change_percent: Option<f64>,
    pub purchases_change_percent: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DailyStat {
    pub date: String,
    pub revenue: i64,
    pub gross_revenue: i64,
    pub discounts: i64,
    pub purchases: i64,
    pub refunds: i64,
    pub refund_amount: i64,
    pub unique_buyers: i64,
    pub avg_order_value: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SalesStats {
    pub daily_stats: Vec<DailyStat>,
    pub summary: SalesOverview,
}

/// GET /apps/{app_id}/sales - Get sales overview for an app
#[utoipa::path(
    get,
    path = "/apps/{app_id}/sales",
    tag = "sales",
    description = "Get sales overview for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Sales overview", body = SalesOverview),
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
#[tracing::instrument(name = "GET /apps/{app_id}/sales", skip(state, user))]
pub async fn get_sales_overview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<SalesOverview>, ApiError> {
    let sub = user.sub()?;

    // Verify user has access (must be owner/admin of the app)
    verify_sales_access(&state, &sub, &app_id).await?;

    let app = app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    // Get total stats from purchases
    let purchases: Vec<app_purchase::Model> = app_purchase::Entity::find()
        .filter(app_purchase::Column::AppId.eq(&app_id))
        .all(&state.db)
        .await?;

    let total_purchases = purchases.len() as i64;
    let completed_purchases: Vec<_> = purchases
        .iter()
        .filter(|p| p.status == crate::entity::sea_orm_active_enums::PurchaseStatus::Completed)
        .collect();

    let total_revenue: i64 = completed_purchases.iter().map(|p| p.price_paid).sum();
    let total_discounts: i64 = completed_purchases.iter().map(|p| p.discount_amount).sum();

    let refunded_purchases: Vec<_> = purchases
        .iter()
        .filter(|p| {
            matches!(
                p.status,
                crate::entity::sea_orm_active_enums::PurchaseStatus::Refunded
                    | crate::entity::sea_orm_active_enums::PurchaseStatus::PartiallyRefunded
            )
        })
        .collect();

    let total_refunds = refunded_purchases.len() as i64;
    let refund_amount: i64 = refunded_purchases.iter().map(|p| p.price_paid).sum();

    let net_revenue = total_revenue - refund_amount;

    // Unique buyers
    let unique_buyer_ids: std::collections::HashSet<_> =
        completed_purchases.iter().map(|p| &p.user_id).collect();
    let unique_buyers = unique_buyer_ids.len() as i64;

    let avg_order_value = if completed_purchases.is_empty() {
        0
    } else {
        total_revenue / completed_purchases.len() as i64
    };

    // Team members count
    let total_members = membership::Entity::find()
        .filter(membership::Column::AppId.eq(&app_id))
        .count(&state.db)
        .await? as i64;

    // Period stats (last 30 days vs previous 30 days)
    let now = Utc::now().date_naive();
    let thirty_days_ago = now - Duration::days(30);
    let sixty_days_ago = now - Duration::days(60);

    let period_purchases: Vec<_> = completed_purchases
        .iter()
        .filter(|p| {
            p.completed_at
                .map(|d| d.date() >= thirty_days_ago)
                .unwrap_or(false)
        })
        .collect();
    let period_revenue: i64 = period_purchases.iter().map(|p| p.price_paid).sum();
    let period_purchase_count = period_purchases.len() as i64;

    let prev_period_purchases: Vec<_> = completed_purchases
        .iter()
        .filter(|p| {
            p.completed_at
                .map(|d| d.date() >= sixty_days_ago && d.date() < thirty_days_ago)
                .unwrap_or(false)
        })
        .collect();
    let prev_period_revenue: i64 = prev_period_purchases.iter().map(|p| p.price_paid).sum();
    let prev_period_purchase_count = prev_period_purchases.len() as i64;

    let revenue_change_percent = if prev_period_revenue > 0 {
        Some(((period_revenue - prev_period_revenue) as f64 / prev_period_revenue as f64) * 100.0)
    } else if period_revenue > 0 {
        Some(100.0)
    } else {
        None
    };

    let purchases_change_percent = if prev_period_purchase_count > 0 {
        Some(
            ((period_purchase_count - prev_period_purchase_count) as f64
                / prev_period_purchase_count as f64)
                * 100.0,
        )
    } else if period_purchase_count > 0 {
        Some(100.0)
    } else {
        None
    };

    Ok(Json(SalesOverview {
        total_revenue,
        total_purchases,
        total_refunds,
        refund_amount,
        net_revenue,
        unique_buyers,
        avg_order_value,
        current_price: app.price,
        total_discounts,
        total_members,
        period_revenue,
        period_purchases: period_purchase_count,
        revenue_change_percent,
        purchases_change_percent,
    }))
}

/// GET /apps/{app_id}/sales/stats - Get detailed sales statistics with daily breakdown
#[utoipa::path(
    get,
    path = "/apps/{app_id}/sales/stats",
    tag = "sales",
    description = "Get sales statistics with daily breakdown.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("start_date" = Option<String>, Query, description = "Start date (YYYY-MM-DD)"),
        ("end_date" = Option<String>, Query, description = "End date (YYYY-MM-DD)"),
        ("period" = String, Query, description = "Aggregation period: day, week, month")
    ),
    responses(
        (status = 200, description = "Sales stats", body = SalesStats),
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
#[tracing::instrument(name = "GET /apps/{app_id}/sales/stats", skip(state, user, query))]
pub async fn get_sales_stats(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<SalesStats>, ApiError> {
    let sub = user.sub()?;

    verify_sales_access(&state, &sub, &app_id).await?;

    // Parse date range
    let end_date = query
        .end_date
        .as_ref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .unwrap_or_else(|| Utc::now().date_naive());

    let start_date = query
        .start_date
        .as_ref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .unwrap_or_else(|| end_date - Duration::days(30));

    // Try to get pre-aggregated daily stats
    let daily_aggregates = app_sales_daily::Entity::find()
        .filter(app_sales_daily::Column::AppId.eq(&app_id))
        .filter(app_sales_daily::Column::Date.gte(start_date))
        .filter(app_sales_daily::Column::Date.lte(end_date))
        .order_by_asc(app_sales_daily::Column::Date)
        .all(&state.db)
        .await?;

    let daily_stats: Vec<DailyStat> = if daily_aggregates.is_empty() {
        // Fall back to computing from purchases if no aggregates exist
        compute_daily_stats_from_purchases(&state, &app_id, start_date, end_date).await?
    } else {
        daily_aggregates
            .into_iter()
            .map(|d| DailyStat {
                date: d.date.format("%Y-%m-%d").to_string(),
                revenue: d.total_revenue,
                gross_revenue: d.gross_revenue,
                discounts: d.total_discounts,
                purchases: d.purchase_count,
                refunds: d.refund_count,
                refund_amount: d.refund_amount,
                unique_buyers: d.unique_buyers,
                avg_order_value: d.avg_order_value,
            })
            .collect()
    };

    // Calculate summary
    let total_revenue: i64 = daily_stats.iter().map(|d| d.revenue).sum();
    let total_purchases: i64 = daily_stats.iter().map(|d| d.purchases).sum();
    let total_refunds: i64 = daily_stats.iter().map(|d| d.refunds).sum();
    let refund_amount: i64 = daily_stats.iter().map(|d| d.refund_amount).sum();
    let total_discounts: i64 = daily_stats.iter().map(|d| d.discounts).sum();
    let unique_buyers: i64 = daily_stats.iter().map(|d| d.unique_buyers).sum();

    let app = app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let total_members = membership::Entity::find()
        .filter(membership::Column::AppId.eq(&app_id))
        .count(&state.db)
        .await? as i64;

    let avg_order_value = if total_purchases > 0 {
        total_revenue / total_purchases
    } else {
        0
    };

    Ok(Json(SalesStats {
        daily_stats,
        summary: SalesOverview {
            total_revenue,
            total_purchases,
            total_refunds,
            refund_amount,
            net_revenue: total_revenue - refund_amount,
            unique_buyers,
            avg_order_value,
            current_price: app.price,
            total_discounts,
            total_members,
            period_revenue: total_revenue,
            period_purchases: total_purchases,
            revenue_change_percent: None,
            purchases_change_percent: None,
        },
    }))
}

/// Helper to compute daily stats from raw purchases (fallback when no aggregates)
async fn compute_daily_stats_from_purchases(
    state: &AppState,
    app_id: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<DailyStat>, ApiError> {
    use std::collections::HashMap;

    let purchases = app_purchase::Entity::find()
        .filter(app_purchase::Column::AppId.eq(app_id))
        .all(&state.db)
        .await?;

    // Group by date
    let mut daily_map: HashMap<NaiveDate, Vec<&app_purchase::Model>> = HashMap::new();

    for purchase in &purchases {
        if let Some(completed_at) = purchase.completed_at {
            let date = completed_at.date();
            if date >= start_date && date <= end_date {
                daily_map.entry(date).or_default().push(purchase);
            }
        }
    }

    // Build stats for each day in range
    let mut stats = Vec::new();
    let mut current = start_date;
    while current <= end_date {
        let day_purchases = daily_map.get(&current).map(|v| v.as_slice()).unwrap_or(&[]);

        let completed: Vec<_> = day_purchases
            .iter()
            .filter(|p| p.status == crate::entity::sea_orm_active_enums::PurchaseStatus::Completed)
            .collect();

        let refunded: Vec<_> = day_purchases
            .iter()
            .filter(|p| {
                matches!(
                    p.status,
                    crate::entity::sea_orm_active_enums::PurchaseStatus::Refunded
                        | crate::entity::sea_orm_active_enums::PurchaseStatus::PartiallyRefunded
                )
            })
            .collect();

        let revenue: i64 = completed.iter().map(|p| p.price_paid).sum();
        let gross_revenue: i64 = completed.iter().map(|p| p.original_price).sum();
        let discounts: i64 = completed.iter().map(|p| p.discount_amount).sum();
        let refund_amt: i64 = refunded.iter().map(|p| p.price_paid).sum();

        let unique_buyers: std::collections::HashSet<_> =
            completed.iter().map(|p| &p.user_id).collect();

        stats.push(DailyStat {
            date: current.format("%Y-%m-%d").to_string(),
            revenue,
            gross_revenue,
            discounts,
            purchases: completed.len() as i64,
            refunds: refunded.len() as i64,
            refund_amount: refund_amt,
            unique_buyers: unique_buyers.len() as i64,
            avg_order_value: if completed.is_empty() {
                0
            } else {
                revenue / completed.len() as i64
            },
        });

        current += Duration::days(1);
    }

    Ok(stats)
}

/// Verify the user has owner/admin access to view sales for this app
pub(crate) async fn verify_sales_access(
    state: &AppState,
    user_id: &str,
    app_id: &str,
) -> Result<(), ApiError> {
    use crate::entity::role;

    // Check if user has a membership with owner role
    let membership = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id))
        .filter(membership::Column::UserId.eq(user_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;

    // Get the app to check owner role
    let app = app::Entity::find_by_id(app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    // Verify app is public/public_restricted (sales only make sense for these)
    if !matches!(
        app.visibility,
        Visibility::Public | Visibility::PublicRequestAccess
    ) {
        return Err(ApiError::bad_request(
            "Sales dashboard is only available for public apps".to_string(),
        ));
    }

    // Check if user has owner role
    if let Some(owner_role_id) = &app.owner_role_id
        && &membership.role_id == owner_role_id
    {
        return Ok(());
    }

    // Check if role has sales permission (for future extensibility)
    let role = role::Entity::find_by_id(&membership.role_id)
        .one(&state.db)
        .await?;

    if let Some(role) = role {
        // For now, only "owner" role name gets access
        // This can be extended with a proper permission system
        if role.name.to_lowercase() == "owner" {
            return Ok(());
        }
    }

    Err(ApiError::FORBIDDEN)
}
