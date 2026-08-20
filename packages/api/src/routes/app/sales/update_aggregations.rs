//! Daily sales aggregation for `AppSalesDaily`.
//!
//! `GET /apps/{app_id}/sales/stats` reads these day-granular rows instead of
//! scanning `AppPurchase` on every request. They are filled lazily, on read, by
//! [`ensure_sales_aggregations_current`] — the same pattern the analytics twin
//! in `app/analytics/update_aggregations.rs` uses, and the only one that works
//! on every deployment target (the AWS backend is a Lambda and hosts no
//! ticker).
//!
//! Only complete days are ever aggregated. Today is recomputed live from
//! `AppPurchase` by the read path, so a purchase that completes while the
//! dashboard is open shows up on the next refresh.
//!
//! A day is always recomputed from raw rows and upserted, so running the
//! backfill twice can never double-count.

use crate::{
    entity::{app_purchase, app_sales_daily, sea_orm_active_enums::PurchaseStatus},
    error::ApiError,
    state::AppState,
};
use chrono::{Duration, NaiveDate, Utc};
use flow_like_types::create_id;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    sea_query::OnConflict,
};
use std::collections::HashSet;

/// Days recomputed when an app has no aggregations yet. Matches the widest
/// range the sales dashboard offers ("Last 90 days").
const MAX_BACKFILL_DAYS: i64 = 90;

/// Ensure aggregations are up to date through yesterday.
///
/// Finds the latest aggregated date and fills every missing day up to
/// yesterday, capped at [`MAX_BACKFILL_DAYS`] so a first load can never walk
/// the whole history. Apps that have never completed a purchase are skipped
/// entirely instead of being filled with 90 zero rows — the read path falls
/// back to raw purchases and reports the same zeros.
///
/// Callers must verify access before calling this: it writes.
pub async fn ensure_sales_aggregations_current(
    state: &AppState,
    app_id: &str,
) -> Result<(), ApiError> {
    let yesterday = Utc::now().date_naive() - Duration::days(1);
    let earliest_allowed = yesterday - Duration::days(MAX_BACKFILL_DAYS - 1);

    let latest = app_sales_daily::Entity::find()
        .filter(app_sales_daily::Column::AppId.eq(app_id))
        .order_by_desc(app_sales_daily::Column::Date)
        .one(&state.db)
        .await?;

    let start_date = match latest {
        Some(ref row) if row.date >= yesterday => return Ok(()),
        Some(ref row) => row.date + Duration::days(1),
        None => {
            let completed_purchases = app_purchase::Entity::find()
                .filter(app_purchase::Column::AppId.eq(app_id))
                .filter(app_purchase::Column::CompletedAt.is_not_null())
                .count(&state.db)
                .await?;

            if completed_purchases == 0 {
                return Ok(());
            }

            earliest_allowed
        }
    };

    let mut date = start_date.max(earliest_allowed);
    while date <= yesterday {
        update_daily_aggregation(state, app_id, date).await?;
        date += Duration::days(1);
    }

    Ok(())
}

/// Recompute and upsert the aggregation for one app and one day.
///
/// Purchases are bucketed by `completedAt` over a half-open interval, matching
/// the `completed_at.date()` grouping the raw fallback in `overview.rs` uses,
/// so the aggregated and the raw source can never disagree.
pub async fn update_daily_aggregation(
    state: &AppState,
    app_id: &str,
    date: NaiveDate,
) -> Result<(), ApiError> {
    let start_of_day = date.and_hms_opt(0, 0, 0).unwrap();
    let start_of_next_day = start_of_day + Duration::days(1);

    let purchases = app_purchase::Entity::find()
        .filter(app_purchase::Column::AppId.eq(app_id))
        .filter(app_purchase::Column::CompletedAt.gte(start_of_day))
        .filter(app_purchase::Column::CompletedAt.lt(start_of_next_day))
        .all(&state.db)
        .await?;

    let completed: Vec<_> = purchases
        .iter()
        .filter(|p| p.status == PurchaseStatus::Completed)
        .collect();

    let refunded: Vec<_> = purchases
        .iter()
        .filter(|p| {
            matches!(
                p.status,
                PurchaseStatus::Refunded | PurchaseStatus::PartiallyRefunded
            )
        })
        .collect();

    let total_revenue: i64 = completed.iter().map(|p| p.price_paid).sum();
    let gross_revenue: i64 = completed.iter().map(|p| p.original_price).sum();
    let total_discounts: i64 = completed.iter().map(|p| p.discount_amount).sum();
    let refund_amount: i64 = refunded.iter().map(|p| p.price_paid).sum();

    let unique_buyers: HashSet<_> = completed.iter().map(|p| &p.user_id).collect();
    let discount_codes_used = completed.iter().filter(|p| p.discount_id.is_some()).count();

    let avg_order_value = if completed.is_empty() {
        0
    } else {
        total_revenue / completed.len() as i64
    };

    let now = Utc::now().naive_utc();

    app_sales_daily::Entity::insert(app_sales_daily::ActiveModel {
        id: Set(create_id()),
        app_id: Set(app_id.to_string()),
        date: Set(date),
        total_revenue: Set(total_revenue),
        gross_revenue: Set(gross_revenue),
        total_discounts: Set(total_discounts),
        purchase_count: Set(completed.len() as i64),
        refund_count: Set(refunded.len() as i64),
        refund_amount: Set(refund_amount),
        unique_buyers: Set(unique_buyers.len() as i64),
        avg_order_value: Set(avg_order_value),
        discount_codes_used: Set(discount_codes_used as i64),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .on_conflict(
        OnConflict::columns([
            app_sales_daily::Column::AppId,
            app_sales_daily::Column::Date,
        ])
        .update_columns([
            app_sales_daily::Column::TotalRevenue,
            app_sales_daily::Column::GrossRevenue,
            app_sales_daily::Column::TotalDiscounts,
            app_sales_daily::Column::PurchaseCount,
            app_sales_daily::Column::RefundCount,
            app_sales_daily::Column::RefundAmount,
            app_sales_daily::Column::UniqueBuyers,
            app_sales_daily::Column::AvgOrderValue,
            app_sales_daily::Column::DiscountCodesUsed,
            app_sales_daily::Column::UpdatedAt,
        ])
        .to_owned(),
    )
    .exec(&state.db)
    .await?;

    Ok(())
}
