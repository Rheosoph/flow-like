//! Install engagement analytics: DAU/WAU/MAU, retention cohorts, churn and drop-off.
//!
//! Every install metric is read from `TelemetryInstallDaily`, the one-row-per
//! install-per-day rollup. That is both cheaper and more correct than folding
//! raw events: the rollup is retained far longer than the raw events, so a 90
//! day cohort no longer silently loses installs whose events have aged out, and
//! there is no row cap to truncate against.
//!
//! Drop-off paths are the one exception — they need the `path` property of the
//! last raw `page_view`, which no rollup carries. They are therefore bounded to
//! the raw event retention window and the response says how much of the window
//! they actually cover.

use super::TOP_LIST_LIMIT;
use super::overview::{granularity_for, retention_days};
use crate::entity::{telemetry_event, telemetry_install_daily};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};

const FIRST_SEEN_HORIZON_DAYS: i64 = 90;
const RETENTION_MAX_COHORTS: usize = 8;
const RETENTION_MAX_WEEKS: i64 = 8;
/// Safety bound on the raw `page_view` fetch behind the drop-off list. Hitting
/// it is reported as `dropOffTruncated` instead of being swallowed.
const PAGE_VIEW_ROW_CAP: u64 = 50_000;
/// Mirrors the telemetry sweeper's default raw event retention. Drop-off paths
/// cannot look further back than the sweeper keeps raw events.
const DEFAULT_EVENT_RETENTION_DAYS: i64 = 30;
const EVENT_RETENTION_VAR: &str = "FLOW_LIKE_EVENT_RETENTION_DAYS";

const BACKEND_ANON_ID: &str = "backend";
const BACKEND_SOURCE: &str = "backend";
const PAGE_VIEW_EVENT: &str = "page_view";

#[derive(Debug, Deserialize, IntoParams)]
pub struct TelemetryEngagementQuery {
    /// Lookback window in days. Default 30, clamped to 7..=90.
    #[serde(default)]
    pub days: Option<i64>,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DauPoint {
    /// ISO-8601 timestamp at the start of the UTC day.
    pub ts: String,
    pub installs: i64,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetentionCohort {
    /// ISO-8601 timestamp at the start of the cohort's ISO week (Monday).
    pub cohort_week: String,
    pub cohort_size: i64,
    /// Fraction of the cohort active per week offset, starting at offset 0.
    pub weeks: Vec<f64>,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DropOffPath {
    pub path: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryEngagementResponse {
    pub days: i64,
    /// Always "daily": engagement is computed from the per-install daily rollup.
    pub granularity: String,
    pub dau: Vec<DauPoint>,
    pub wau: i64,
    pub mau: i64,
    pub previous_wau: i64,
    pub previous_mau: i64,
    pub new_installs: i64,
    pub returning_installs: i64,
    pub churned_installs: i64,
    pub churn_rate: Option<f64>,
    pub retention: Vec<RetentionCohort>,
    pub drop_off_paths: Vec<DropOffPath>,
    /// Days of raw `page_view` history the drop-off list could actually read.
    /// Zero means raw events no longer reach back before the selected window.
    pub drop_off_window_days: i64,
    /// True when the raw `page_view` fetch hit its safety bound, so the
    /// drop-off list is a sample rather than the full picture.
    pub drop_off_truncated: bool,
}

#[derive(Debug, FromQueryResult)]
struct InstallDayRow {
    anon_id: String,
    day: NaiveDateTime,
}

#[derive(Debug, FromQueryResult)]
struct PageViewRow {
    anon_id: String,
    props: Option<serde_json::Value>,
}

struct EngagementFold {
    dau: Vec<DauPoint>,
    wau: i64,
    mau: i64,
    previous_wau: i64,
    previous_mau: i64,
    new_installs: i64,
    returning_installs: i64,
    churned: BTreeSet<String>,
    churn_rate: Option<f64>,
    retention: Vec<RetentionCohort>,
}

fn day_ts(day: NaiveDate) -> String {
    DateTime::<Utc>::from_naive_utc_and_offset(day.and_time(NaiveTime::MIN), Utc).to_rfc3339()
}

fn week_start(day: NaiveDate) -> NaiveDate {
    day - Duration::days(day.weekday().num_days_from_monday() as i64)
}

/// Raw `page_view` range behind the drop-off list: everything between the
/// cohort horizon and the start of the selected window, clipped to the days the
/// sweeper still keeps raw events for. `None` when nothing is left to read.
fn drop_off_window(
    today: NaiveDate,
    days: i64,
    horizon_days: i64,
    retention_days: i64,
) -> Option<(NaiveDateTime, NaiveDateTime)> {
    let end = (today - Duration::days(days - 1)).and_time(NaiveTime::MIN);
    let horizon = today - Duration::days(horizon_days - 1);
    let retained = today - Duration::days(retention_days - 1);
    let start = horizon.max(retained).and_time(NaiveTime::MIN);
    (start < end).then_some((start, end))
}

fn distinct_between(
    pairs: &[(String, NaiveDate)],
    start: NaiveDate,
    end: NaiveDate,
) -> HashSet<&str> {
    pairs
        .iter()
        .filter(|(_, day)| *day >= start && *day <= end)
        .map(|(anon, _)| anon.as_str())
        .collect()
}

fn fold_engagement(pairs: &[(String, NaiveDate)], today: NaiveDate, days: i64) -> EngagementFold {
    let window_start = today - Duration::days(days - 1);
    let prev_window_start = window_start - Duration::days(days);
    let prev_window_end = window_start - Duration::days(1);

    let mut per_day: BTreeMap<NaiveDate, HashSet<&str>> = BTreeMap::new();
    for (anon, day) in pairs {
        if *day >= window_start && *day <= today {
            per_day.entry(*day).or_default().insert(anon.as_str());
        }
    }
    let dau = (0..days)
        .map(|offset| {
            let day = window_start + Duration::days(offset);
            DauPoint {
                ts: day_ts(day),
                installs: per_day.get(&day).map(|set| set.len() as i64).unwrap_or(0),
            }
        })
        .collect();

    let wau = distinct_between(pairs, today - Duration::days(6), today).len() as i64;
    let previous_wau =
        distinct_between(pairs, today - Duration::days(13), today - Duration::days(7)).len() as i64;
    let mau = distinct_between(pairs, today - Duration::days(29), today).len() as i64;
    let previous_mau = distinct_between(
        pairs,
        today - Duration::days(59),
        today - Duration::days(30),
    )
    .len() as i64;

    let mut first_seen: HashMap<&str, NaiveDate> = HashMap::new();
    for (anon, day) in pairs {
        first_seen
            .entry(anon.as_str())
            .and_modify(|seen| {
                if *day < *seen {
                    *seen = *day;
                }
            })
            .or_insert(*day);
    }

    let active_current = distinct_between(pairs, window_start, today);
    let new_installs = active_current
        .iter()
        .filter(|anon| first_seen.get(**anon).is_some_and(|d| *d >= window_start))
        .count() as i64;
    let returning_installs = active_current.len() as i64 - new_installs;

    let active_previous = distinct_between(pairs, prev_window_start, prev_window_end);
    let churned: BTreeSet<String> = active_previous
        .iter()
        .filter(|anon| !active_current.contains(**anon))
        .map(|anon| anon.to_string())
        .collect();
    let churn_rate = if active_previous.is_empty() {
        None
    } else {
        Some(churned.len() as f64 / active_previous.len() as f64)
    };

    let mut active_weeks: HashMap<&str, HashSet<NaiveDate>> = HashMap::new();
    for (anon, day) in pairs {
        active_weeks
            .entry(anon.as_str())
            .or_default()
            .insert(week_start(*day));
    }
    let mut cohorts: BTreeMap<NaiveDate, Vec<&str>> = BTreeMap::new();
    for (anon, seen) in &first_seen {
        cohorts.entry(week_start(*seen)).or_default().push(*anon);
    }
    let current_week = week_start(today);
    let skip = cohorts.len().saturating_sub(RETENTION_MAX_COHORTS);
    let retention = cohorts
        .iter()
        .skip(skip)
        .map(|(cohort_week, members)| {
            let size = members.len() as i64;
            let elapsed =
                ((current_week - *cohort_week).num_days() / 7 + 1).min(RETENTION_MAX_WEEKS);
            let weeks = (0..elapsed)
                .map(|offset| {
                    let target = *cohort_week + Duration::days(offset * 7);
                    let active = members
                        .iter()
                        .filter(|anon| {
                            active_weeks
                                .get(**anon)
                                .is_some_and(|weeks| weeks.contains(&target))
                        })
                        .count();
                    active as f64 / size as f64
                })
                .collect();
            RetentionCohort {
                cohort_week: day_ts(*cohort_week),
                cohort_size: size,
                weeks,
            }
        })
        .collect();

    EngagementFold {
        dau,
        wau,
        mau,
        previous_wau,
        previous_mau,
        new_installs,
        returning_installs,
        churned,
        churn_rate,
        retention,
    }
}

/// Rows must be ordered newest-first so the first row per install is its last page view.
fn fold_drop_off_paths(rows: &[PageViewRow], churned: &BTreeSet<String>) -> Vec<DropOffPath> {
    let mut latest: HashMap<&str, Option<&str>> = HashMap::new();
    for row in rows {
        if !churned.contains(&row.anon_id) {
            continue;
        }
        latest.entry(row.anon_id.as_str()).or_insert_with(|| {
            row.props
                .as_ref()
                .and_then(|props| props.get("path"))
                .and_then(|path| path.as_str())
        });
    }
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for path in latest.values().flatten() {
        *counts.entry(*path).or_default() += 1;
    }
    let mut paths: Vec<DropOffPath> = counts
        .into_iter()
        .map(|(path, count)| DropOffPath {
            path: path.to_string(),
            count,
        })
        .collect();
    paths.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.path.cmp(&b.path)));
    paths.truncate(TOP_LIST_LIMIT as usize);
    paths
}

/// Distinct `(install, day)` pairs straight out of the rollup.
async fn install_days<C: ConnectionTrait>(
    db: &C,
    horizon_start: NaiveDateTime,
) -> Result<Vec<(String, NaiveDate)>, ApiError> {
    let rows = telemetry_install_daily::Entity::find()
        .select_only()
        .column_as(telemetry_install_daily::Column::AnonId, "anon_id")
        .column_as(telemetry_install_daily::Column::Day, "day")
        .filter(telemetry_install_daily::Column::Day.gte(horizon_start))
        .filter(telemetry_install_daily::Column::AnonId.ne(BACKEND_ANON_ID))
        .filter(telemetry_install_daily::Column::Source.ne(BACKEND_SOURCE))
        .distinct()
        .into_model::<InstallDayRow>()
        .all(db)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.anon_id, row.day.date()))
        .collect())
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/engagement",
    tag = "admin",
    params(TelemetryEngagementQuery),
    responses(
        (status = 200, description = "Install engagement metrics: DAU/WAU/MAU, retention cohorts and churn", body = TelemetryEngagementResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Engagement analytics over anonymous installs: daily active installs, WAU/MAU with previous windows, new vs returning installs, churn and weekly retention cohorts. All of these are computed from the daily per-install rollup, so they are exact for the whole window and are not affected by raw event retention. The drop-off list is the exception: it needs the route of the last raw page view, so it only covers the days raw events are still kept for, reported as \"dropOffWindowDays\", and sets \"dropOffTruncated\" when the underlying fetch hit its safety bound. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/engagement", skip_all)]
pub async fn telemetry_engagement(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<TelemetryEngagementQuery>,
) -> Result<Json<TelemetryEngagementResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let days = q.days.unwrap_or(30).clamp(7, 90);
    let today = Utc::now().date_naive();
    let horizon_days = (days * 2).max(FIRST_SEEN_HORIZON_DAYS);
    let horizon_start = (today - Duration::days(horizon_days - 1)).and_time(NaiveTime::MIN);

    let pairs = install_days(&state.db, horizon_start).await?;
    let fold = fold_engagement(&pairs, today, days);

    let mut drop_off_paths = Vec::new();
    let mut drop_off_window_days = 0;
    let mut drop_off_truncated = false;

    if let Some((start, end)) = drop_off_window(
        today,
        days,
        horizon_days,
        retention_days(EVENT_RETENTION_VAR, DEFAULT_EVENT_RETENTION_DAYS),
    ) {
        drop_off_window_days = (end - start).num_days();
        if !fold.churned.is_empty() {
            let rows = telemetry_event::Entity::find()
                .select_only()
                .column_as(telemetry_event::Column::AnonId, "anon_id")
                .column_as(telemetry_event::Column::Props, "props")
                .filter(telemetry_event::Column::Name.eq(PAGE_VIEW_EVENT))
                .filter(telemetry_event::Column::AnonId.ne(BACKEND_ANON_ID))
                .filter(telemetry_event::Column::Source.ne(BACKEND_SOURCE))
                .filter(telemetry_event::Column::CreatedAt.gte(start))
                .filter(telemetry_event::Column::CreatedAt.lt(end))
                .order_by_desc(telemetry_event::Column::CreatedAt)
                .limit(PAGE_VIEW_ROW_CAP)
                .into_model::<PageViewRow>()
                .all(&state.db)
                .await?;
            drop_off_truncated = rows.len() as u64 >= PAGE_VIEW_ROW_CAP;
            if drop_off_truncated {
                tracing::warn!(
                    cap = PAGE_VIEW_ROW_CAP,
                    "telemetry engagement page_view query hit its row cap; drop-off paths are a sample"
                );
            }
            drop_off_paths = fold_drop_off_paths(&rows, &fold.churned);
        }
    }

    Ok(Json(TelemetryEngagementResponse {
        days,
        granularity: granularity_for(days.saturating_mul(24)).to_string(),
        dau: fold.dau,
        wau: fold.wau,
        mau: fold.mau,
        previous_wau: fold.previous_wau,
        previous_mau: fold.previous_mau,
        new_installs: fold.new_installs,
        returning_installs: fold.returning_installs,
        churned_installs: fold.churned.len() as i64,
        churn_rate: fold.churn_rate,
        retention: fold.retention,
        drop_off_paths,
        drop_off_window_days,
        drop_off_truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::admin::telemetry::overview::GRANULARITY_DAILY;
    use sea_orm::{DbBackend, QueryTrait};
    use serde_json::json;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn pair(anon: &str, y: i32, m: u32, d: u32) -> (String, NaiveDate) {
        (anon.to_string(), day(y, m, d))
    }

    #[test]
    fn engagement_always_reads_the_daily_rollup() {
        for days in [7, 30, 90] {
            assert_eq!(granularity_for(days * 24), GRANULARITY_DAILY);
        }
    }

    #[test]
    fn install_days_selects_distinct_pairs_and_skips_the_backend() {
        let sql = telemetry_install_daily::Entity::find()
            .select_only()
            .column_as(telemetry_install_daily::Column::AnonId, "anon_id")
            .column_as(telemetry_install_daily::Column::Day, "day")
            .filter(
                telemetry_install_daily::Column::Day.gte(day(2026, 4, 28).and_time(NaiveTime::MIN)),
            )
            .filter(telemetry_install_daily::Column::AnonId.ne(BACKEND_ANON_ID))
            .filter(telemetry_install_daily::Column::Source.ne(BACKEND_SOURCE))
            .distinct()
            .build(DbBackend::Postgres)
            .to_string();

        assert!(sql.starts_with("SELECT DISTINCT"), "{sql}");
        assert!(sql.contains(r#""TelemetryInstallDaily""#), "{sql}");
        assert!(!sql.contains("LIMIT"), "{sql}");
    }

    #[test]
    fn the_drop_off_window_is_clipped_to_the_raw_event_retention() {
        let today = day(2026, 7, 26);

        let (start, end) = drop_off_window(today, 7, 90, 30).unwrap();
        assert_eq!(start, day(2026, 6, 27).and_time(NaiveTime::MIN));
        assert_eq!(end, day(2026, 7, 20).and_time(NaiveTime::MIN));
        assert_eq!((end - start).num_days(), 23);

        let (start, _) = drop_off_window(today, 7, 90, 400).unwrap();
        assert_eq!(start, day(2026, 4, 28).and_time(NaiveTime::MIN));
    }

    #[test]
    fn the_drop_off_window_is_empty_when_raw_events_no_longer_reach_back() {
        assert_eq!(drop_off_window(day(2026, 7, 26), 90, 180, 30), None);
        assert_eq!(drop_off_window(day(2026, 7, 26), 30, 90, 1), None);
    }

    #[test]
    fn dau_is_zero_filled_over_the_window() {
        let pairs = vec![
            pair("a", 2026, 7, 20),
            pair("b", 2026, 7, 20),
            pair("a", 2026, 7, 23),
        ];
        let fold = fold_engagement(&pairs, day(2026, 7, 26), 7);
        assert_eq!(fold.dau.len(), 7);
        assert_eq!(fold.dau[0].ts, "2026-07-20T00:00:00+00:00");
        assert_eq!(
            fold.dau.iter().map(|p| p.installs).collect::<Vec<_>>(),
            vec![2, 0, 0, 1, 0, 0, 0]
        );
        assert_eq!(fold.dau[6].ts, "2026-07-26T00:00:00+00:00");
    }

    #[test]
    fn splits_new_and_returning_installs() {
        let pairs = vec![
            pair("old", 2026, 7, 1),
            pair("old", 2026, 7, 21),
            pair("new", 2026, 7, 22),
        ];
        let fold = fold_engagement(&pairs, day(2026, 7, 26), 7);
        assert_eq!(fold.new_installs, 1);
        assert_eq!(fold.returning_installs, 1);
    }

    #[test]
    fn returning_installs_survive_the_ninety_day_window() {
        let pairs = vec![
            pair("old", 2026, 3, 28),
            pair("old", 2026, 7, 25),
            pair("new", 2026, 7, 20),
        ];
        let fold = fold_engagement(&pairs, day(2026, 7, 26), 90);
        assert_eq!(fold.new_installs, 1);
        assert_eq!(fold.returning_installs, 1);
    }

    #[test]
    fn detects_churned_installs_and_rate() {
        let pairs = vec![
            pair("gone", 2026, 7, 15),
            pair("stay", 2026, 7, 15),
            pair("stay", 2026, 7, 22),
            pair("fresh", 2026, 7, 23),
        ];
        let fold = fold_engagement(&pairs, day(2026, 7, 26), 7);
        assert_eq!(
            fold.churned.iter().cloned().collect::<Vec<_>>(),
            vec!["gone".to_string()]
        );
        assert_eq!(fold.churn_rate, Some(0.5));
    }

    #[test]
    fn churn_rate_is_null_without_previous_activity() {
        let pairs = vec![pair("only", 2026, 7, 22)];
        let fold = fold_engagement(&pairs, day(2026, 7, 26), 7);
        assert!(fold.churned.is_empty());
        assert_eq!(fold.churn_rate, None);
    }

    #[test]
    fn retention_cohorts_report_weekly_ratios() {
        let pairs = vec![
            pair("a", 2026, 7, 13),
            pair("b", 2026, 7, 13),
            pair("a", 2026, 7, 20),
        ];
        let fold = fold_engagement(&pairs, day(2026, 7, 26), 7);
        assert_eq!(fold.retention.len(), 1);
        let cohort = &fold.retention[0];
        assert_eq!(cohort.cohort_week, "2026-07-13T00:00:00+00:00");
        assert_eq!(cohort.cohort_size, 2);
        assert_eq!(cohort.weeks, vec![1.0, 0.5]);
    }

    fn page_view(anon: &str, props: Option<serde_json::Value>) -> PageViewRow {
        PageViewRow {
            anon_id: anon.to_string(),
            props,
        }
    }

    #[test]
    fn drop_off_uses_last_page_view_per_churned_install() {
        let churned: BTreeSet<String> = ["g1", "g2", "g3"].iter().map(|s| s.to_string()).collect();
        let rows = vec![
            page_view("g1", Some(json!({ "path": "/b" }))),
            page_view("other", Some(json!({ "path": "/x" }))),
            page_view("g1", Some(json!({ "path": "/a" }))),
            page_view("g2", None),
            page_view("g2", Some(json!({ "path": "/a" }))),
            page_view("g3", Some(json!({ "path": "/b" }))),
        ];
        let paths = fold_drop_off_paths(&rows, &churned);
        assert_eq!(
            paths,
            vec![DropOffPath {
                path: "/b".to_string(),
                count: 2
            }]
        );
    }
}
