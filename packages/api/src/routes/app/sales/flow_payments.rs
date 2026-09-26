use crate::{
    entity::profile, error::ApiError, middleware::jwt::AppUser, payments::sql, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

use super::overview::{change_percent, verify_revenue_access};

const DAY_MS: i64 = 86_400_000;
const PERIOD_DAYS: i64 = 30;
const MAX_RANGE_DAYS: i64 = 366;

/// Settled payments a flow collected for this app: accepted attempts only, so
/// late payments that are being refunded automatically never count as income.
const SETTLED: &str =
    r#""appId"=$1 AND "sourceType"='REQUEST' AND status='PAID' AND orphaned=FALSE AND livemode=$2"#;

#[derive(Debug, Deserialize, IntoParams)]
pub struct FlowPaymentsQuery {
    /// First day of the daily breakdown (YYYY-MM-DD). Defaults to 30 days before `end_date`.
    pub start_date: Option<String>,
    /// Last day of the daily breakdown (YYYY-MM-DD). Defaults to today.
    pub end_date: Option<String>,
    /// Number of recent payments to return (max 100).
    #[serde(default = "default_limit")]
    pub limit: u64,
}

fn default_limit() -> u64 {
    20
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowPaymentDay {
    pub date: String,
    /// Collected minus refunded (cents)
    pub revenue: i64,
    pub collected: i64,
    pub refunded: i64,
    pub payments: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowPaymentItem {
    pub id: String,
    pub product_name: Option<String>,
    pub reference: Option<String>,
    pub payer_user_id: Option<String>,
    pub payer_name: Option<String>,
    pub payer_avatar: Option<String>,
    pub board_id: Option<String>,
    pub run_id: Option<String>,
    pub amount: i64,
    pub collected: i64,
    pub refunded: i64,
    pub currency: String,
    /// Unix milliseconds
    pub created_at: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowPaymentsReport {
    /// Lifetime collected minus refunded (cents), before payment provider and platform fees
    pub total_revenue: i64,
    pub total_collected: i64,
    pub total_refunded: i64,
    pub total_payments: i64,
    pub refunded_payments: i64,
    pub unique_payers: i64,
    /// Revenue of the last 30 days
    pub period_revenue: i64,
    pub period_payments: i64,
    pub revenue_change_percent: Option<f64>,
    pub payments_change_percent: Option<f64>,
    pub daily_stats: Vec<FlowPaymentDay>,
    pub recent_payments: Vec<FlowPaymentItem>,
}

/// GET /apps/{app_id}/sales/flow-payments
#[utoipa::path(
    get,
    path = "/apps/{app_id}/sales/flow-payments",
    tag = "sales",
    description = "Get the money this app's flows collected through payment nodes: lifetime totals, a daily breakdown and the most recent payments. Only the app owner can read it.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        FlowPaymentsQuery
    ),
    responses(
        (status = 200, description = "Flow payment revenue", body = FlowPaymentsReport),
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
#[tracing::instrument(
    name = "GET /apps/{app_id}/sales/flow-payments",
    skip(state, user, query)
)]
pub async fn get_flow_payments(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<FlowPaymentsQuery>,
) -> Result<Json<FlowPaymentsReport>, ApiError> {
    verify_revenue_access(&state, &user, &app_id).await?;

    let (start_date, end_date) = date_range(&query, Utc::now().date_naive());
    let livemode = state.platform_config.payments.livemode;
    let now_ms = Utc::now().timestamp_millis();
    let period_start = now_ms - PERIOD_DAYS * DAY_MS;
    let prev_period_start = period_start - PERIOD_DAYS * DAY_MS;

    let totals = state
        .db
        .query_one_raw(sql(
            &format!(
                r#"SELECT
                    COUNT(*)::BIGINT AS total_payments,
                    COALESCE(SUM("capturedAmount"),0)::BIGINT AS total_collected,
                    COALESCE(SUM("refundedAmount"),0)::BIGINT AS total_refunded,
                    COALESCE(SUM(CASE WHEN "refundedAmount">0 THEN 1 ELSE 0 END),0)::BIGINT AS refunded_payments,
                    COUNT(DISTINCT "payerUserId")::BIGINT AS unique_payers,
                    COALESCE(SUM(CASE WHEN "createdAt">=$3 THEN 1 ELSE 0 END),0)::BIGINT AS period_payments,
                    COALESCE(SUM(CASE WHEN "createdAt">=$3 THEN "capturedAmount"-"refundedAmount" ELSE 0 END),0)::BIGINT AS period_revenue,
                    COALESCE(SUM(CASE WHEN "createdAt">=$4 AND "createdAt"<$3 THEN 1 ELSE 0 END),0)::BIGINT AS prev_period_payments,
                    COALESCE(SUM(CASE WHEN "createdAt">=$4 AND "createdAt"<$3 THEN "capturedAmount"-"refundedAmount" ELSE 0 END),0)::BIGINT AS prev_period_revenue
                FROM "PaymentAttempt" WHERE {SETTLED}"#
            ),
            vec![
                app_id.clone().into(),
                livemode.into(),
                period_start.into(),
                prev_period_start.into(),
            ],
        ))
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let total = |column: &str| totals.try_get::<i64>("", column);

    let day_rows = state
        .db
        .query_all_raw(sql(
            &format!(
                r#"SELECT ("createdAt"/{DAY_MS})::BIGINT AS day,
                    COUNT(*)::BIGINT AS payments,
                    COALESCE(SUM("capturedAmount"),0)::BIGINT AS collected,
                    COALESCE(SUM("refundedAmount"),0)::BIGINT AS refunded
                FROM "PaymentAttempt" WHERE {SETTLED} AND "createdAt">=$3 AND "createdAt"<$4
                GROUP BY 1"#
            ),
            vec![
                app_id.clone().into(),
                livemode.into(),
                day_start_ms(start_date).into(),
                (day_start_ms(end_date) + DAY_MS).into(),
            ],
        ))
        .await?;
    let mut days = HashMap::new();
    for row in day_rows {
        days.insert(
            row.try_get::<i64>("", "day")?,
            (
                row.try_get::<i64>("", "collected")?,
                row.try_get::<i64>("", "refunded")?,
                row.try_get::<i64>("", "payments")?,
            ),
        );
    }

    let total_collected = total("total_collected")?;
    let total_refunded = total("total_refunded")?;
    let period_revenue = total("period_revenue")?;
    let period_payments = total("period_payments")?;

    Ok(Json(FlowPaymentsReport {
        total_revenue: total_collected - total_refunded,
        total_collected,
        total_refunded,
        total_payments: total("total_payments")?,
        refunded_payments: total("refunded_payments")?,
        unique_payers: total("unique_payers")?,
        period_revenue,
        period_payments,
        revenue_change_percent: change_percent(period_revenue, total("prev_period_revenue")?),
        payments_change_percent: change_percent(period_payments, total("prev_period_payments")?),
        daily_stats: dense_days(start_date, end_date, &days),
        recent_payments: recent_payments(&state, &app_id, livemode, query.limit.min(100)).await?,
    }))
}

async fn recent_payments(
    state: &AppState,
    app_id: &str,
    livemode: bool,
    limit: u64,
) -> Result<Vec<FlowPaymentItem>, ApiError> {
    let rows = state
        .db
        .query_all_raw(sql(
            &format!(
                r#"SELECT a.id,a."payerUserId",a.amount,a."capturedAmount",a."refundedAmount",a.currency,a."createdAt",
                    r."productName",r.reference,r."boardId",r."runId"
                FROM (SELECT * FROM "PaymentAttempt" WHERE {SETTLED} ORDER BY "createdAt" DESC,id DESC LIMIT $3) a
                LEFT JOIN "PaymentRequest" r ON r.id=a."sourceId"
                ORDER BY a."createdAt" DESC,a.id DESC"#
            ),
            vec![app_id.into(), livemode.into(), (limit as i64).into()],
        ))
        .await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(FlowPaymentItem {
            id: row.try_get("", "id")?,
            product_name: row.try_get("", "productName")?,
            reference: row.try_get("", "reference")?,
            payer_user_id: row.try_get("", "payerUserId")?,
            payer_name: None,
            payer_avatar: None,
            board_id: row.try_get("", "boardId")?,
            run_id: row.try_get("", "runId")?,
            amount: row.try_get("", "amount")?,
            collected: row.try_get("", "capturedAmount")?,
            refunded: row.try_get("", "refundedAmount")?,
            currency: row.try_get("", "currency")?,
            created_at: row.try_get("", "createdAt")?,
        });
    }

    let payer_ids: Vec<String> = items
        .iter()
        .filter_map(|item| item.payer_user_id.clone())
        .collect();
    if payer_ids.is_empty() {
        return Ok(items);
    }
    let profiles: HashMap<String, profile::Model> = profile::Entity::find()
        .filter(profile::Column::UserId.is_in(payer_ids))
        .filter(profile::Column::DeletedAt.is_null())
        .all(&state.db)
        .await?
        .into_iter()
        .map(|profile| (profile.user_id.clone(), profile))
        .collect();
    for item in &mut items {
        if let Some(profile) = item.payer_user_id.as_ref().and_then(|id| profiles.get(id)) {
            item.payer_name = Some(profile.name.clone());
            item.payer_avatar = profile.thumbnail.clone();
        }
    }
    Ok(items)
}

fn parse_day(value: Option<&String>) -> Option<NaiveDate> {
    value.and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
}

/// Inclusive day range, capped to a year so a hostile range cannot build an
/// unbounded day list.
fn date_range(query: &FlowPaymentsQuery, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let end = parse_day(query.end_date.as_ref()).unwrap_or(today);
    let start = parse_day(query.start_date.as_ref())
        .unwrap_or_else(|| end - Duration::days(PERIOD_DAYS))
        .min(end);
    (start.max(end - Duration::days(MAX_RANGE_DAYS)), end)
}

fn day_start_ms(day: NaiveDate) -> i64 {
    day.and_hms_opt(0, 0, 0)
        .map(|stamp| stamp.and_utc().timestamp_millis())
        .unwrap_or_default()
}

fn day_from_index(index: i64) -> Option<NaiveDate> {
    DateTime::from_timestamp_millis(index * DAY_MS).map(|stamp| stamp.date_naive())
}

/// One row per UTC day in the range, zero-filled, so the chart lines up with
/// the store series.
fn dense_days(
    start: NaiveDate,
    end: NaiveDate,
    days: &HashMap<i64, (i64, i64, i64)>,
) -> Vec<FlowPaymentDay> {
    let mut stats = Vec::new();
    let mut index = day_start_ms(start) / DAY_MS;
    let last = day_start_ms(end) / DAY_MS;
    while index <= last {
        if let Some(date) = day_from_index(index) {
            let (collected, refunded, payments) = days.get(&index).copied().unwrap_or_default();
            stats.push(FlowPaymentDay {
                date: date.format("%Y-%m-%d").to_string(),
                revenue: collected - refunded,
                collected,
                refunded,
                payments,
            });
        }
        index += 1;
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn query(start: Option<&str>, end: Option<&str>) -> FlowPaymentsQuery {
        FlowPaymentsQuery {
            start_date: start.map(str::to_owned),
            end_date: end.map(str::to_owned),
            limit: default_limit(),
        }
    }

    #[test]
    fn range_defaults_to_the_thirty_days_before_the_end() {
        let (start, end) = date_range(&query(None, None), day("2026-09-21"));
        assert_eq!(start, day("2026-08-22"));
        assert_eq!(end, day("2026-09-21"));
    }

    #[test]
    fn range_is_capped_and_never_inverted() {
        let (start, _) = date_range(
            &query(Some("2000-01-01"), Some("2026-09-21")),
            day("2026-09-21"),
        );
        assert_eq!(start, day("2026-09-21") - Duration::days(MAX_RANGE_DAYS));

        let (start, end) = date_range(
            &query(Some("2026-09-25"), Some("2026-09-21")),
            day("2026-09-21"),
        );
        assert_eq!(start, end);
    }

    #[test]
    fn days_are_zero_filled_and_keyed_by_utc_day() {
        let start = day("2026-09-19");
        let paid_day = day_start_ms(day("2026-09-20")) / DAY_MS;
        let days = HashMap::from([(paid_day, (1500, 500, 2))]);

        let stats = dense_days(start, day("2026-09-21"), &days);

        assert_eq!(
            stats.iter().map(|s| s.date.as_str()).collect::<Vec<_>>(),
            ["2026-09-19", "2026-09-20", "2026-09-21"]
        );
        assert_eq!(stats[0].payments, 0);
        assert_eq!(stats[1].revenue, 1000);
        assert_eq!(stats[1].collected, 1500);
        assert_eq!(stats[1].payments, 2);
    }

    #[test]
    fn a_payment_just_before_midnight_stays_on_its_day() {
        let late = day_start_ms(day("2026-09-20")) + DAY_MS - 1;
        assert_eq!(day_from_index(late / DAY_MS), Some(day("2026-09-20")));
    }
}
