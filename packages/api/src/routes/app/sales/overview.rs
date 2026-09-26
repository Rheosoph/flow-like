use crate::utils::time::utc_midnight;
use crate::{
    entity::{
        app, app_purchase, app_sales_daily, membership,
        sea_orm_active_enums::{PurchaseStatus, Visibility},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
    utils::stats_period::StatsPeriod,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::{Duration, NaiveDate, Utc};
use sea_orm::sea_query::{Alias, Expr, ExprTrait, Func, SimpleExpr};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::update_aggregations::ensure_sales_aggregations_current;

#[derive(Debug, Deserialize, ToSchema)]
pub struct StatsQuery {
    /// Start date for the stats period (YYYY-MM-DD)
    pub start_date: Option<String>,
    /// End date for the stats period (YYYY-MM-DD)
    pub end_date: Option<String>,
    /// Aggregation period: "day" (default), "week" or "month". Week buckets
    /// start on Monday; every bucket is labelled with its first day.
    #[serde(default = "default_period")]
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
    // Verify user has access (must be owner/admin of the app)
    let app = verify_sales_access(&state, &user, &app_id).await?;
    let total_members = count_members(&state, &app_id).await?;

    Ok(Json(
        sales_overview(&state, &app_id, app.price, total_members).await?,
    ))
}

#[derive(Default, FromQueryResult)]
struct PurchaseTotals {
    total_purchases: i64,
    completed_purchases: i64,
    total_revenue: i64,
    total_discounts: i64,
    unique_buyers: i64,
    total_refunds: i64,
    refund_amount: i64,
    period_purchases: i64,
    period_revenue: i64,
    prev_period_purchases: i64,
    prev_period_revenue: i64,
}

/// `CASE WHEN <condition> THEN <column> END`, the aggregate input that limits a
/// `COUNT`/`SUM` to the matching purchases.
fn purchase_case(condition: SimpleExpr, column: app_purchase::Column) -> SimpleExpr {
    Expr::expr(Expr::case(condition, Expr::col(column)))
}

fn count_purchases(condition: SimpleExpr) -> SimpleExpr {
    purchase_case(condition, app_purchase::Column::Id).count()
}

/// SUM over BIGINT comes back as NUMERIC/DECIMAL, hence the cast.
fn sum_purchases(condition: SimpleExpr, column: app_purchase::Column) -> SimpleExpr {
    Expr::expr(Func::coalesce([
        purchase_case(condition, column).sum(),
        Expr::val(0i64),
    ]))
    .cast_as(Alias::new("BIGINT"))
}

pub(super) async fn count_members(state: &AppState, app_id: &str) -> Result<i64, ApiError> {
    Ok(membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id))
        .count(&state.db)
        .await? as i64)
}

/// Lifetime totals plus the last-30-days vs previous-30-days comparison, from a
/// single aggregate over the app's purchases.
pub(super) async fn sales_overview(
    state: &AppState,
    app_id: &str,
    current_price: i64,
    total_members: i64,
) -> Result<SalesOverview, ApiError> {
    let now = Utc::now().date_naive();
    let period_start = utc_midnight(now - Duration::days(30));
    let prev_period_start = utc_midnight(now - Duration::days(60));

    let completed = || app_purchase::Column::Status.eq(PurchaseStatus::Completed);
    let refunded = || {
        app_purchase::Column::Status
            .is_in([PurchaseStatus::Refunded, PurchaseStatus::PartiallyRefunded])
    };
    let in_period = || completed().and(app_purchase::Column::CompletedAt.gte(period_start));
    let in_prev_period = || {
        completed()
            .and(app_purchase::Column::CompletedAt.gte(prev_period_start))
            .and(app_purchase::Column::CompletedAt.lt(period_start))
    };

    let totals = app_purchase::Entity::find()
        .filter(app_purchase::Column::AppId.eq(app_id))
        .select_only()
        .expr_as(
            Expr::col(app_purchase::Column::Id).count(),
            "total_purchases",
        )
        .expr_as(count_purchases(completed()), "completed_purchases")
        .expr_as(
            sum_purchases(completed(), app_purchase::Column::PricePaid),
            "total_revenue",
        )
        .expr_as(
            sum_purchases(completed(), app_purchase::Column::DiscountAmount),
            "total_discounts",
        )
        .expr_as(
            purchase_case(completed(), app_purchase::Column::UserId).count_distinct(),
            "unique_buyers",
        )
        .expr_as(count_purchases(refunded()), "total_refunds")
        .expr_as(
            sum_purchases(refunded(), app_purchase::Column::PricePaid),
            "refund_amount",
        )
        .expr_as(count_purchases(in_period()), "period_purchases")
        .expr_as(
            sum_purchases(in_period(), app_purchase::Column::PricePaid),
            "period_revenue",
        )
        .expr_as(count_purchases(in_prev_period()), "prev_period_purchases")
        .expr_as(
            sum_purchases(in_prev_period(), app_purchase::Column::PricePaid),
            "prev_period_revenue",
        )
        .into_model::<PurchaseTotals>()
        .one(&state.db)
        .await?
        .unwrap_or_default();

    let total_purchases = totals.total_purchases;
    let total_revenue = totals.total_revenue;
    let total_discounts = totals.total_discounts;
    let total_refunds = totals.total_refunds;
    let refund_amount = totals.refund_amount;
    let net_revenue = total_revenue - refund_amount;
    let unique_buyers = totals.unique_buyers;

    let avg_order_value = if totals.completed_purchases == 0 {
        0
    } else {
        total_revenue / totals.completed_purchases
    };

    let period_revenue = totals.period_revenue;
    let period_purchase_count = totals.period_purchases;
    let revenue_change_percent = change_percent(period_revenue, totals.prev_period_revenue);
    let purchases_change_percent =
        change_percent(period_purchase_count, totals.prev_period_purchases);

    Ok(SalesOverview {
        total_revenue,
        total_purchases,
        total_refunds,
        refund_amount,
        net_revenue,
        unique_buyers,
        avg_order_value,
        current_price,
        total_discounts,
        total_members,
        period_revenue,
        period_purchases: period_purchase_count,
        revenue_change_percent,
        purchases_change_percent,
    })
}

/// GET /apps/{app_id}/sales/stats - Get detailed sales statistics with daily breakdown
#[utoipa::path(
    get,
    path = "/apps/{app_id}/sales/stats",
    tag = "sales",
    description = "Get sales statistics bucketed by the requested period. Buckets are labelled with their first day; week buckets start on Monday.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("start_date" = Option<String>, Query, description = "Start date (YYYY-MM-DD)"),
        ("end_date" = Option<String>, Query, description = "End date (YYYY-MM-DD)"),
        ("period" = String, Query, description = "Aggregation period: day (default), week or month. Unrecognised values fall back to day.")
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
    let app = verify_sales_access(&state, &user, &app_id).await?;
    let total_members = count_members(&state, &app_id).await?;

    Ok(Json(
        sales_stats(&state, &app_id, app.price, total_members, query).await?,
    ))
}

/// Day rows for the requested range folded into `query.period` buckets, plus
/// their summary. Callers must verify access first: the backfill writes.
pub(super) async fn sales_stats(
    state: &AppState,
    app_id: &str,
    current_price: i64,
    total_members: i64,
    query: StatsQuery,
) -> Result<SalesStats, ApiError> {
    let period = StatsPeriod::parse(&query.period);

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

    // Fill any day still missing from the pre-aggregated table, through
    // yesterday. Runs after verify_sales_access - this writes.
    ensure_sales_aggregations_current(state, app_id).await?;

    let daily_aggregates = app_sales_daily::Entity::find()
        .filter(app_sales_daily::Column::AppId.eq(app_id))
        .filter(app_sales_daily::Column::Date.gte(start_date))
        .filter(app_sales_daily::Column::Date.lte(end_date))
        .order_by_asc(app_sales_daily::Column::Date)
        .all(&state.db)
        .await?;

    let computed_from_raw = daily_aggregates.is_empty();
    let mut daily_stats: Vec<DailyStat> = if computed_from_raw {
        // No aggregates for this window (app never sold, or the window predates
        // the backfill cap) - compute the whole range from raw purchases.
        compute_daily_stats_from_purchases(state, app_id, start_date, end_date).await?
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

    // Aggregates only ever cover complete days; today is always live.
    let today = Utc::now().date_naive();
    if !computed_from_raw && start_date <= today && end_date >= today {
        daily_stats.extend(compute_daily_stats_from_purchases(state, app_id, today, today).await?);
    }

    let daily_stats =
        fold_daily_stats(state, app_id, daily_stats, start_date, end_date, period).await?;

    // Calculate summary
    let total_revenue: i64 = daily_stats.iter().map(|d| d.revenue).sum();
    let total_purchases: i64 = daily_stats.iter().map(|d| d.purchases).sum();
    let total_refunds: i64 = daily_stats.iter().map(|d| d.refunds).sum();
    let refund_amount: i64 = daily_stats.iter().map(|d| d.refund_amount).sum();
    let total_discounts: i64 = daily_stats.iter().map(|d| d.discounts).sum();
    let unique_buyers: i64 = daily_stats.iter().map(|d| d.unique_buyers).sum();

    let avg_order_value = if total_purchases > 0 {
        total_revenue / total_purchases
    } else {
        0
    };

    Ok(SalesStats {
        daily_stats,
        summary: SalesOverview {
            total_revenue,
            total_purchases,
            total_refunds,
            refund_amount,
            net_revenue: total_revenue - refund_amount,
            unique_buyers,
            avg_order_value,
            current_price,
            total_discounts,
            total_members,
            period_revenue: total_revenue,
            period_purchases: total_purchases,
            revenue_change_percent: None,
            purchases_change_percent: None,
        },
    })
}

#[derive(FromQueryResult)]
struct BucketCount {
    bucket: String,
    cnt: i64,
}

/// Distinct buyers of completed purchases per bucket, counted in SQL so a buyer
/// who bought on two days inside one bucket is counted once.
async fn count_unique_buyers_by_bucket(
    state: &AppState,
    app_id: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
    period: StatsPeriod,
) -> Result<std::collections::HashMap<NaiveDate, i64>, ApiError> {
    use std::collections::HashMap;

    let bucket_expr = period.bucket_expr(state.db.get_database_backend(), "completedAt");
    let start_of_range = utc_midnight(start_date);
    let end_of_range = utc_midnight(end_date) + Duration::days(1);

    let rows = app_purchase::Entity::find()
        .filter(app_purchase::Column::AppId.eq(app_id))
        .filter(app_purchase::Column::Status.eq(PurchaseStatus::Completed))
        .filter(app_purchase::Column::CompletedAt.gte(start_of_range))
        .filter(app_purchase::Column::CompletedAt.lt(end_of_range))
        .select_only()
        .expr_as(bucket_expr.clone(), "bucket")
        .expr_as(
            Expr::col(app_purchase::Column::UserId).count_distinct(),
            "cnt",
        )
        .group_by(bucket_expr)
        .into_model::<BucketCount>()
        .all(&state.db)
        .await?;

    let mut per_bucket: HashMap<NaiveDate, i64> = HashMap::new();
    for row in rows {
        if let Ok(date) = NaiveDate::parse_from_str(&row.bucket, "%Y-%m-%d") {
            per_bucket.insert(date, row.cnt);
        }
    }

    Ok(per_bucket)
}

/// Fold day rows into `period` buckets keyed by the first day of the bucket.
///
/// Money and counts add up, and `avg_order_value` is re-derived from the folded
/// totals, which is the purchase-weighted average of the daily values. Distinct
/// buyers cannot be folded this way - a buyer active on two days of the bucket
/// would be counted twice - so that field is left untouched here and refilled
/// from SQL by [`fold_daily_stats`].
fn fold_bucket_totals(stats: Vec<DailyStat>, period: StatsPeriod) -> Vec<DailyStat> {
    use std::collections::BTreeMap;
    use std::collections::btree_map::Entry;

    if period == StatsPeriod::Day {
        return stats;
    }

    let mut buckets: BTreeMap<NaiveDate, DailyStat> = BTreeMap::new();
    for stat in stats {
        let Ok(date) = NaiveDate::parse_from_str(&stat.date, "%Y-%m-%d") else {
            continue;
        };
        let bucket_start = period.bucket_start(date);
        match buckets.entry(bucket_start) {
            Entry::Vacant(slot) => {
                slot.insert(DailyStat {
                    date: bucket_start.format("%Y-%m-%d").to_string(),
                    ..stat
                });
            }
            Entry::Occupied(mut slot) => {
                let target = slot.get_mut();
                target.revenue += stat.revenue;
                target.gross_revenue += stat.gross_revenue;
                target.discounts += stat.discounts;
                target.purchases += stat.purchases;
                target.refunds += stat.refunds;
                target.refund_amount += stat.refund_amount;
            }
        }
    }

    let mut folded: Vec<DailyStat> = buckets.into_values().collect();
    for stat in &mut folded {
        stat.avg_order_value = if stat.purchases > 0 {
            stat.revenue / stat.purchases
        } else {
            0
        };
    }

    folded
}

/// Fold day rows into `period` buckets and re-count distinct buyers per bucket.
async fn fold_daily_stats(
    state: &AppState,
    app_id: &str,
    stats: Vec<DailyStat>,
    start_date: NaiveDate,
    end_date: NaiveDate,
    period: StatsPeriod,
) -> Result<Vec<DailyStat>, ApiError> {
    if period == StatsPeriod::Day {
        return Ok(stats);
    }

    let mut folded = fold_bucket_totals(stats, period);
    let unique_buyers =
        count_unique_buyers_by_bucket(state, app_id, start_date, end_date, period).await?;

    for stat in &mut folded {
        if let Ok(date) = NaiveDate::parse_from_str(&stat.date, "%Y-%m-%d") {
            stat.unique_buyers = unique_buyers.get(&date).copied().unwrap_or(0);
        }
    }

    Ok(folded)
}

/// Helper to compute daily stats from raw purchases (fallback when no aggregates)
async fn compute_daily_stats_from_purchases(
    state: &AppState,
    app_id: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<DailyStat>, ApiError> {
    use std::collections::HashMap;

    let start_of_range = utc_midnight(start_date);
    let end_of_range = utc_midnight(end_date) + Duration::days(1);

    let purchases = app_purchase::Entity::find()
        .filter(app_purchase::Column::AppId.eq(app_id))
        .filter(app_purchase::Column::CompletedAt.gte(start_of_range))
        .filter(app_purchase::Column::CompletedAt.lt(end_of_range))
        .all(&state.db)
        .await?;

    // Group by date
    let mut daily_map: HashMap<NaiveDate, Vec<&app_purchase::Model>> = HashMap::new();

    for purchase in &purchases {
        if let Some(completed_at) = purchase.completed_at {
            let date = completed_at.date_naive();
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

/// Percentage change from the previous period; `None` when both are empty.
pub(super) fn change_percent(current: i64, previous: i64) -> Option<f64> {
    if previous > 0 {
        Some(((current - previous) as f64 / previous as f64) * 100.0)
    } else if current > 0 {
        Some(100.0)
    } else {
        None
    }
}

/// Verify the user holds the app's designated owner role and return the app
/// row the check loaded. Flow payments can be collected at any visibility, so
/// this is the gate for revenue that does not come from a store listing.
pub(crate) async fn verify_revenue_access(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
) -> Result<app::Model, ApiError> {
    // Revenue is user-only: `app_permission` alone would also admit API keys
    // and connected apps.
    user.sub()?;

    // Prices, discounts and purchase lists must not outlive a demotion, so the
    // membership role is read fresh instead of from the permission cache.
    let role = user.app_permission_fresh(app_id, state).await?.role;

    let app = app::Entity::find_by_id(app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if app.owner_role_id.as_deref() == Some(role.id.as_str()) {
        return Ok(app);
    }

    Err(ApiError::FORBIDDEN)
}

/// [`verify_revenue_access`] for store sales, which only exist for apps listed
/// in the store.
pub(crate) async fn verify_sales_access(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
) -> Result<app::Model, ApiError> {
    let app = verify_revenue_access(state, user, app_id).await?;

    if !matches!(
        app.visibility,
        Visibility::Public | Visibility::PublicRequestAccess
    ) {
        return Err(ApiError::bad_request(
            "Sales dashboard is only available for public apps".to_string(),
        ));
    }

    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(date: &str, revenue: i64, purchases: i64, unique_buyers: i64) -> DailyStat {
        DailyStat {
            date: date.to_string(),
            revenue,
            gross_revenue: revenue + 10,
            discounts: 10,
            purchases,
            refunds: 1,
            refund_amount: 5,
            unique_buyers,
            avg_order_value: if purchases > 0 {
                revenue / purchases
            } else {
                0
            },
        }
    }

    fn labels(stats: &[DailyStat]) -> Vec<&str> {
        stats.iter().map(|s| s.date.as_str()).collect()
    }

    #[test]
    fn day_period_returns_the_rows_untouched() {
        let rows = vec![stat("2026-08-19", 200, 1, 1), stat("2026-08-20", 300, 3, 2)];
        let folded = fold_bucket_totals(rows, StatsPeriod::Day);

        assert_eq!(labels(&folded), ["2026-08-19", "2026-08-20"]);
        assert_eq!(folded[0].revenue, 200);
        assert_eq!(folded[1].avg_order_value, 100);
    }

    #[test]
    fn week_buckets_sum_counts_and_reweight_the_average_order_value() {
        let rows = vec![
            stat("2026-08-17", 300, 3, 2),
            stat("2026-08-19", 200, 1, 1),
            stat("2026-08-23", 0, 0, 0),
        ];
        let folded = fold_bucket_totals(rows, StatsPeriod::Week);

        assert_eq!(labels(&folded), ["2026-08-17"]);
        let week = &folded[0];
        assert_eq!(week.revenue, 500);
        assert_eq!(week.gross_revenue, 530);
        assert_eq!(week.discounts, 30);
        assert_eq!(week.purchases, 4);
        assert_eq!(week.refunds, 3);
        assert_eq!(week.refund_amount, 15);
        // Purchase-weighted, not the mean of the daily averages (which is 100).
        assert_eq!(week.avg_order_value, 125);
    }

    #[test]
    fn empty_buckets_report_a_zero_average_order_value() {
        let folded = fold_bucket_totals(vec![stat("2026-08-19", 0, 0, 0)], StatsPeriod::Week);

        assert_eq!(folded[0].purchases, 0);
        assert_eq!(folded[0].avg_order_value, 0);
    }

    #[test]
    fn week_buckets_cross_month_and_year_boundaries() {
        let rows = vec![
            stat("2025-12-31", 100, 1, 1),
            stat("2026-01-01", 100, 1, 1),
            stat("2026-01-04", 100, 1, 1),
            stat("2026-01-05", 100, 1, 1),
            stat("2026-08-31", 100, 1, 1),
            stat("2026-09-01", 100, 1, 1),
        ];
        let folded = fold_bucket_totals(rows, StatsPeriod::Week);

        assert_eq!(
            labels(&folded),
            ["2025-12-29", "2026-01-05", "2026-08-31"],
            "weeks are labelled with their Monday, even across a month or year boundary"
        );
        assert_eq!(folded[0].purchases, 3);
        assert_eq!(folded[1].purchases, 1);
        assert_eq!(folded[2].purchases, 2);
    }

    #[test]
    fn month_buckets_are_labelled_with_the_first_of_the_month() {
        let rows = vec![
            stat("2026-08-17", 100, 1, 1),
            stat("2026-08-31", 100, 1, 1),
            stat("2026-09-01", 100, 1, 1),
        ];
        let folded = fold_bucket_totals(rows, StatsPeriod::Month);

        assert_eq!(labels(&folded), ["2026-08-01", "2026-09-01"]);
        assert_eq!(folded[0].revenue, 200);
        assert_eq!(folded[1].revenue, 100);
    }

    #[test]
    fn distinct_buyers_are_never_summed_by_the_fold() {
        let rows = vec![stat("2026-08-17", 100, 1, 1), stat("2026-08-19", 100, 1, 1)];
        let folded = fold_bucket_totals(rows, StatsPeriod::Week);

        assert_eq!(folded.len(), 1);
        assert_eq!(
            folded[0].unique_buyers, 1,
            "the same buyer on two days must not become two buyers; fold_daily_stats re-queries this"
        );
    }
}
