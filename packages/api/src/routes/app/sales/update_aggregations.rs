//! Utility to update daily sales aggregations
//! This can be called by a cron job or triggered after purchases

use crate::{
    entity::{app_purchase, app_sales_daily, sea_orm_active_enums::PurchaseStatus},
    error::ApiError,
    state::AppState,
};
use chrono::{Duration, NaiveDate, Utc};
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use std::collections::HashSet;

/// Update daily aggregations for a specific app and date
#[allow(dead_code)]
pub async fn update_daily_aggregation(
    state: &AppState,
    app_id: &str,
    date: NaiveDate,
) -> Result<(), ApiError> {
    // Get all purchases for this app on this date
    let start_of_day = date.and_hms_opt(0, 0, 0).unwrap();
    let end_of_day = date.and_hms_opt(23, 59, 59).unwrap();

    let purchases = app_purchase::Entity::find()
        .filter(app_purchase::Column::AppId.eq(app_id))
        .filter(app_purchase::Column::CompletedAt.gte(start_of_day))
        .filter(app_purchase::Column::CompletedAt.lte(end_of_day))
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

    // Check if aggregation exists for this date
    let existing = app_sales_daily::Entity::find()
        .filter(app_sales_daily::Column::AppId.eq(app_id))
        .filter(app_sales_daily::Column::Date.eq(date))
        .one(&state.db)
        .await?;

    let now = Utc::now().naive_utc();

    if let Some(existing) = existing {
        // Update existing
        let mut active: app_sales_daily::ActiveModel = existing.into();
        active.total_revenue = Set(total_revenue);
        active.gross_revenue = Set(gross_revenue);
        active.total_discounts = Set(total_discounts);
        active.purchase_count = Set(completed.len() as i64);
        active.refund_count = Set(refunded.len() as i64);
        active.refund_amount = Set(refund_amount);
        active.unique_buyers = Set(unique_buyers.len() as i64);
        active.avg_order_value = Set(avg_order_value);
        active.discount_codes_used = Set(discount_codes_used as i64);
        active.updated_at = Set(now);

        active.update(&state.db).await?;
    } else {
        // Create new
        let id = create_id();
        let new_agg = app_sales_daily::ActiveModel {
            id: Set(id),
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
        };

        new_agg.insert(&state.db).await?;
    }

    Ok(())
}

/// Backfill aggregations for an app for the last N days
#[allow(dead_code)]
pub async fn backfill_aggregations(
    state: &AppState,
    app_id: &str,
    days: i64,
) -> Result<u32, ApiError> {
    let today = Utc::now().date_naive();
    let mut count = 0;

    for i in 0..days {
        let date = today - Duration::days(i);
        update_daily_aggregation(state, app_id, date).await?;
        count += 1;
    }

    Ok(count)
}
