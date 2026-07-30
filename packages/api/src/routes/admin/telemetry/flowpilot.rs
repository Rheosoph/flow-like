//! FlowPilot workflow-generation funnel metrics.
//!
//! Windows of at most 48 hours fold the raw counter events. Longer windows read
//! `TelemetryFlowpilotDaily`, which stores one pre-summed row per UTC day, so a
//! 30 day funnel is 30 rows instead of a capped scan over every counter event.

use super::overview::{GRANULARITY_DAILY, GRANULARITY_RAW, day_window, reads_raw, window_bucket};
use super::{bucket_slots, bucket_step, trunc_to_bucket};
use crate::entity::{telemetry_event, telemetry_flowpilot_daily};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use utoipa::{IntoParams, ToSchema};

const FLOWPILOT_METRICS_EVENT: &str = "flowpilot_generation_metrics";
/// Safety bound on the raw fold, which only ever runs inside the 48 hour raw
/// window now that longer windows read the rollup.
const FLOWPILOT_ROW_CAP: u64 = 100_000;

/// Single source of truth for the FlowPilot counter field list: hands it to the
/// accumulator macro that knows how to read one kind of source row.
macro_rules! flowpilot_counters {
    ($accumulate:ident, $($arg:tt),* $(,)?) => {
        $accumulate!(
            $($arg,)*
            [
                runs_started,
                runs_succeeded,
                runs_failed,
                runs_cancelled,
                plans_assessed,
                plans_feasible,
                plans_infeasible,
                attempts_total,
                attempts_parse_valid,
                attempts_typed_valid,
                attempts_reconcile_valid,
                attempts_applied,
                queued_reviews,
                apply_dispositions,
                dismissed_dispositions,
                stale_dispositions,
                error_dispositions,
                diagnostic_occurrences,
                repeated_diagnostic_occurrences,
                validation_regressions,
                boards_inspected,
                empty_boards_after_run,
            ]
        )
    };
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct TelemetryFlowPilotQuery {
    /// Lookback window in hours. Default 720 (30 days), clamped to 1..=2160.
    #[serde(default)]
    pub hours: Option<i64>,
}

#[derive(Debug, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowPilotTotals {
    pub runs_started: i64,
    pub runs_succeeded: i64,
    pub runs_failed: i64,
    pub runs_cancelled: i64,
    pub plans_assessed: i64,
    pub plans_feasible: i64,
    pub plans_infeasible: i64,
    pub attempts_total: i64,
    pub attempts_parse_valid: i64,
    pub attempts_typed_valid: i64,
    pub attempts_reconcile_valid: i64,
    pub attempts_applied: i64,
    pub queued_reviews: i64,
    pub apply_dispositions: i64,
    pub dismissed_dispositions: i64,
    pub stale_dispositions: i64,
    pub error_dispositions: i64,
    pub diagnostic_occurrences: i64,
    pub repeated_diagnostic_occurrences: i64,
    pub validation_regressions: i64,
    pub boards_inspected: i64,
    pub empty_boards_after_run: i64,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowPilotTrendPoint {
    /// ISO-8601 timestamp at the start of the bucket.
    pub ts: String,
    pub runs_started: i64,
    pub runs_succeeded: i64,
    pub runs_failed: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryFlowPilotResponse {
    pub hours: i64,
    /// "raw" when the funnel is folded from individual counter events, "daily"
    /// when it is summed from the daily rollup.
    pub granularity: String,
    pub installs: i64,
    pub totals: FlowPilotTotals,
    pub trend: Vec<FlowPilotTrendPoint>,
}

#[derive(Debug, FromQueryResult)]
struct FlowPilotRow {
    anon_id: String,
    props: Option<serde_json::Value>,
    created_at: NaiveDateTime,
}

fn counter_value(props: Option<&serde_json::Value>, key: &str) -> i64 {
    let Some(value) = props.and_then(|p| p.get(key)) else {
        return 0;
    };
    value
        .as_i64()
        .or_else(|| value.as_u64().map(|v| v.min(i64::MAX as u64) as i64))
        .unwrap_or(0)
        .max(0)
}

macro_rules! sum_counters {
    ($totals:expr, $props:expr, [$($field:ident),* $(,)?]) => {
        $( $totals.$field = $totals.$field.saturating_add(counter_value($props, stringify!($field))); )*
    };
}

macro_rules! sum_daily_counters {
    ($totals:expr, $row:expr, [$($field:ident),* $(,)?]) => {
        $( $totals.$field = $totals.$field.saturating_add($row.$field.max(0)); )*
    };
}

fn fold_flowpilot_totals(rows: &[FlowPilotRow]) -> FlowPilotTotals {
    let mut totals = FlowPilotTotals::default();
    for row in rows {
        let props = row.props.as_ref();
        flowpilot_counters!(sum_counters, totals, props);
    }
    totals
}

fn fold_flowpilot_trend(
    rows: &[FlowPilotRow],
    cutoff: NaiveDateTime,
    now: NaiveDateTime,
    bucket: &str,
) -> Vec<FlowPilotTrendPoint> {
    let step = bucket_step(bucket);
    let mut buckets: BTreeMap<NaiveDateTime, (i64, i64, i64)> = BTreeMap::new();
    let mut slot = trunc_to_bucket(cutoff, bucket);
    let end = trunc_to_bucket(now, bucket);
    while slot <= end {
        buckets.insert(slot, (0, 0, 0));
        slot += step;
    }
    for row in rows {
        if let Some(counts) = buckets.get_mut(&trunc_to_bucket(row.created_at, bucket)) {
            let props = row.props.as_ref();
            counts.0 = counts
                .0
                .saturating_add(counter_value(props, "runs_started"));
            counts.1 = counts
                .1
                .saturating_add(counter_value(props, "runs_succeeded"));
            counts.2 = counts.2.saturating_add(counter_value(props, "runs_failed"));
        }
    }
    buckets
        .into_iter()
        .map(|(ts, (started, succeeded, failed))| FlowPilotTrendPoint {
            ts: DateTime::<Utc>::from_naive_utc_and_offset(ts, Utc).to_rfc3339(),
            runs_started: started,
            runs_succeeded: succeeded,
            runs_failed: failed,
        })
        .collect()
}

/// Sums the pre-aggregated days. `installs` adds up the daily distinct installs
/// and is therefore an upper bound on the distinct installs over the window.
fn fold_daily_totals(rows: &[telemetry_flowpilot_daily::Model]) -> (FlowPilotTotals, i64) {
    let mut totals = FlowPilotTotals::default();
    let mut installs = 0i64;
    for row in rows {
        flowpilot_counters!(sum_daily_counters, totals, row);
        installs = installs.saturating_add(i64::from(row.installs.max(0)));
    }
    (totals, installs)
}

fn fold_daily_trend(
    rows: &[telemetry_flowpilot_daily::Model],
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Vec<FlowPilotTrendPoint> {
    let counts: BTreeMap<NaiveDateTime, (i64, i64, i64)> = rows
        .iter()
        .map(|row| {
            (
                row.day,
                (
                    row.runs_started.max(0),
                    row.runs_succeeded.max(0),
                    row.runs_failed.max(0),
                ),
            )
        })
        .collect();

    bucket_slots(start, end, "day")
        .into_iter()
        .map(|slot| {
            let (started, succeeded, failed) = counts.get(&slot).copied().unwrap_or((0, 0, 0));
            FlowPilotTrendPoint {
                ts: DateTime::<Utc>::from_naive_utc_and_offset(slot, Utc).to_rfc3339(),
                runs_started: started,
                runs_succeeded: succeeded,
                runs_failed: failed,
            }
        })
        .collect()
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/flowpilot",
    tag = "admin",
    params(TelemetryFlowPilotQuery),
    responses(
        (status = 200, description = "Aggregated FlowPilot generation funnel metrics", body = TelemetryFlowPilotResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "FlowPilot workflow-generation funnel: run outcomes, plan feasibility, attempt validity, review dispositions and diagnostics, aggregated from anonymous counter events. Windows of up to 48 hours fold individual events (granularity \"raw\"); longer windows sum the daily rollup (granularity \"daily\") and are aligned to whole UTC days. In \"daily\" mode the install count is the sum of the daily distinct installs, an upper bound on the distinct installs over the whole window. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/flowpilot", skip_all)]
pub async fn telemetry_flowpilot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<TelemetryFlowPilotQuery>,
) -> Result<Json<TelemetryFlowPilotResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let hours = q.hours.unwrap_or(720).clamp(1, 2160);
    let now = Utc::now().naive_utc();

    if reads_raw(hours) {
        let bucket = window_bucket(hours, None);
        let cutoff = now - Duration::hours(hours);

        let rows = telemetry_event::Entity::find()
            .select_only()
            .column_as(telemetry_event::Column::AnonId, "anon_id")
            .column_as(telemetry_event::Column::Props, "props")
            .column_as(telemetry_event::Column::CreatedAt, "created_at")
            .filter(telemetry_event::Column::Name.eq(FLOWPILOT_METRICS_EVENT))
            .filter(telemetry_event::Column::CreatedAt.gte(cutoff))
            .order_by_desc(telemetry_event::Column::CreatedAt)
            .limit(FLOWPILOT_ROW_CAP)
            .into_model::<FlowPilotRow>()
            .all(&state.db)
            .await?;
        if rows.len() as u64 >= FLOWPILOT_ROW_CAP {
            tracing::warn!(
                cap = FLOWPILOT_ROW_CAP,
                "telemetry flowpilot query hit its row cap; totals may be incomplete"
            );
        }

        let installs = rows
            .iter()
            .map(|row| row.anon_id.as_str())
            .collect::<HashSet<_>>()
            .len() as i64;

        return Ok(Json(TelemetryFlowPilotResponse {
            hours,
            granularity: GRANULARITY_RAW.to_string(),
            installs,
            totals: fold_flowpilot_totals(&rows),
            trend: fold_flowpilot_trend(&rows, cutoff, now, bucket),
        }));
    }

    let (start, end) = day_window(now, hours);
    let rows = telemetry_flowpilot_daily::Entity::find()
        .filter(telemetry_flowpilot_daily::Column::Day.gte(start))
        .filter(telemetry_flowpilot_daily::Column::Day.lte(end))
        .order_by_asc(telemetry_flowpilot_daily::Column::Day)
        .all(&state.db)
        .await?;

    let (totals, installs) = fold_daily_totals(&rows);

    Ok(Json(TelemetryFlowPilotResponse {
        hours,
        granularity: GRANULARITY_DAILY.to_string(),
        installs,
        totals,
        trend: fold_daily_trend(&rows, start, end),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::json;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn flowpilot_row(
        anon: &str,
        props: Option<serde_json::Value>,
        created_at: NaiveDateTime,
    ) -> FlowPilotRow {
        FlowPilotRow {
            anon_id: anon.to_string(),
            props,
            created_at,
        }
    }

    fn ts(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        day(y, m, d).and_hms_opt(h, min, 0).unwrap()
    }

    fn daily(
        y: i32,
        m: u32,
        d: u32,
        runs_started: i64,
        runs_succeeded: i64,
        runs_failed: i64,
        installs: i32,
    ) -> telemetry_flowpilot_daily::Model {
        telemetry_flowpilot_daily::Model {
            id: format!("{y}-{m}-{d}"),
            day: ts(y, m, d, 0, 0),
            runs_started,
            runs_succeeded,
            runs_failed,
            runs_cancelled: 0,
            plans_assessed: 0,
            plans_feasible: 0,
            plans_infeasible: 0,
            attempts_total: 0,
            attempts_parse_valid: 0,
            attempts_typed_valid: 0,
            attempts_reconcile_valid: 0,
            attempts_applied: 0,
            queued_reviews: 0,
            apply_dispositions: 0,
            dismissed_dispositions: 0,
            stale_dispositions: 0,
            error_dispositions: 0,
            diagnostic_occurrences: 0,
            repeated_diagnostic_occurrences: 0,
            validation_regressions: 0,
            boards_inspected: 0,
            empty_boards_after_run: 0,
            installs,
            created_at: ts(y, m, d, 0, 0),
            updated_at: ts(y, m, d, 0, 0),
        }
    }

    #[test]
    fn the_window_decides_between_raw_events_and_the_daily_rollup() {
        assert!(reads_raw(47));
        assert!(reads_raw(48));
        assert!(!reads_raw(49));
        assert!(!reads_raw(720));
    }

    #[test]
    fn flowpilot_totals_are_lenient_about_props() {
        let rows = vec![
            flowpilot_row(
                "a",
                Some(json!({
                    "runs_started": 2,
                    "runs_succeeded": 1,
                    "boards_inspected": 3
                })),
                ts(2026, 7, 26, 10, 0),
            ),
            flowpilot_row(
                "b",
                Some(json!({
                    "runs_started": "junk",
                    "runs_failed": 2.5,
                    "runs_cancelled": 1
                })),
                ts(2026, 7, 26, 11, 0),
            ),
            flowpilot_row("c", None, ts(2026, 7, 26, 12, 0)),
        ];
        let totals = fold_flowpilot_totals(&rows);
        assert_eq!(totals.runs_started, 2);
        assert_eq!(totals.runs_succeeded, 1);
        assert_eq!(totals.runs_failed, 0);
        assert_eq!(totals.runs_cancelled, 1);
        assert_eq!(totals.boards_inspected, 3);
        assert_eq!(totals.attempts_total, 0);
    }

    #[test]
    fn flowpilot_counters_clamp_negatives_and_saturate() {
        let rows = vec![
            flowpilot_row(
                "a",
                Some(json!({ "runs_started": i64::MAX, "runs_failed": -5 })),
                ts(2026, 7, 26, 10, 0),
            ),
            flowpilot_row(
                "b",
                Some(json!({ "runs_started": i64::MAX })),
                ts(2026, 7, 26, 10, 30),
            ),
        ];
        let totals = fold_flowpilot_totals(&rows);
        assert_eq!(totals.runs_started, i64::MAX);
        assert_eq!(totals.runs_failed, 0);

        let trend = fold_flowpilot_trend(
            &rows,
            ts(2026, 7, 26, 10, 0),
            ts(2026, 7, 26, 10, 59),
            "hour",
        );
        assert_eq!(trend.len(), 1);
        assert_eq!(trend[0].runs_started, i64::MAX);
        assert_eq!(trend[0].runs_failed, 0);
    }

    #[test]
    fn flowpilot_trend_zero_fills_hour_buckets() {
        let cutoff = ts(2026, 7, 26, 10, 30);
        let now = ts(2026, 7, 26, 13, 10);
        let rows = vec![
            flowpilot_row(
                "a",
                Some(json!({ "runs_started": 1 })),
                ts(2026, 7, 26, 10, 45),
            ),
            flowpilot_row(
                "b",
                Some(json!({ "runs_started": 2, "runs_failed": 1 })),
                ts(2026, 7, 26, 12, 5),
            ),
        ];
        let trend = fold_flowpilot_trend(&rows, cutoff, now, "hour");
        assert_eq!(trend.len(), 4);
        assert_eq!(trend[0].ts, "2026-07-26T10:00:00+00:00");
        assert_eq!(trend[0].runs_started, 1);
        assert_eq!(trend[1].runs_started, 0);
        assert_eq!(trend[2].runs_started, 2);
        assert_eq!(trend[2].runs_failed, 1);
        assert_eq!(trend[3].runs_started, 0);
    }

    #[test]
    fn daily_rollup_days_are_summed_and_negatives_ignored() {
        let rows = vec![
            daily(2026, 7, 24, 5, 4, 1, 3),
            daily(2026, 7, 25, 7, 6, -2, 4),
        ];
        let (totals, installs) = fold_daily_totals(&rows);

        assert_eq!(totals.runs_started, 12);
        assert_eq!(totals.runs_succeeded, 10);
        assert_eq!(totals.runs_failed, 1);
        assert_eq!(totals.attempts_total, 0);
        assert_eq!(installs, 7);
    }

    #[test]
    fn daily_rollup_totals_saturate_instead_of_overflowing() {
        let rows = vec![
            daily(2026, 7, 24, i64::MAX, 0, 0, 0),
            daily(2026, 7, 25, i64::MAX, 0, 0, 0),
        ];
        let (totals, _) = fold_daily_totals(&rows);
        assert_eq!(totals.runs_started, i64::MAX);
    }

    #[test]
    fn daily_trend_zero_fills_missing_days() {
        let rows = vec![daily(2026, 7, 25, 3, 2, 1, 2)];
        let trend = fold_daily_trend(&rows, ts(2026, 7, 24, 0, 0), ts(2026, 7, 26, 0, 0));

        assert_eq!(
            trend,
            vec![
                FlowPilotTrendPoint {
                    ts: "2026-07-24T00:00:00+00:00".to_string(),
                    runs_started: 0,
                    runs_succeeded: 0,
                    runs_failed: 0,
                },
                FlowPilotTrendPoint {
                    ts: "2026-07-25T00:00:00+00:00".to_string(),
                    runs_started: 3,
                    runs_succeeded: 2,
                    runs_failed: 1,
                },
                FlowPilotTrendPoint {
                    ts: "2026-07-26T00:00:00+00:00".to_string(),
                    runs_started: 0,
                    runs_succeeded: 0,
                    runs_failed: 0,
                },
            ]
        );
    }
}
