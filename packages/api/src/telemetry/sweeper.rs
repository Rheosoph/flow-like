//! Retention sweeper for the anonymous telemetry tables.
//!
//! Every high-volume telemetry table is bounded by a retention window: raw
//! events, errors, sessions, LLM calls, spans, web-vitals samples and alert
//! inbox rows, plus the daily rollups they feed. The sweeper deletes rows older
//! than the configured window on a fixed interval, mirroring
//! `execution::run_sweeper`. Deployments without a long-lived process (AWS
//! Lambda) trigger the same work through `POST /admin/telemetry/sweep`.
//!
//! ORDERING INVARIANT: raw rows must never be deleted before the rollup job has
//! aggregated the day they belong to, otherwise the long-window admin queries —
//! which read the rollups — would lose data permanently. Every raw table that
//! feeds a rollup therefore has its cutoff additionally clamped so it can never
//! advance past the end of the last fully rolled-up day, which is one day
//! before the newest rolled-up day because that one is still receiving rows.
//! While no rollup has ever run those tables are skipped entirely; if the
//! operator disabled the rollup job the clamp is lifted, because nobody will
//! ever aggregate the rows and holding them forever would only grow the tables
//! the sweeper exists to bound.
//!
//! `TelemetryIssue`, `TelemetryRelease`, `TelemetrySourceMap`,
//! `TelemetrySavedQuery`, `TelemetryDashboard` and `TelemetryAlertRule` are
//! never swept: they are bounded or user-owned. An issue therefore keeps its
//! `eventCount`/`installCount` after the `TelemetryErrorEvent` rows they were
//! derived from age out — those counters are lifetime totals by design.

use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDateTime, Utc};
use flow_like_types::tokio::{self, task::JoinHandle};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};

use crate::entity::prelude::{
    TelemetryAlertEvent, TelemetryDimensionDaily, TelemetryErrorEvent, TelemetryEvent,
    TelemetryEventDaily, TelemetryFlowpilotDaily, TelemetryFlowpilotFailureDaily,
    TelemetryInstallDaily, TelemetryLlmCall, TelemetryLlmDaily, TelemetryPerfDaily,
    TelemetryPerfMetric, TelemetrySession, TelemetrySessionDaily, TelemetrySpan,
};
use crate::entity::{
    telemetry_alert_event, telemetry_dimension_daily, telemetry_error_event, telemetry_event,
    telemetry_event_daily, telemetry_flowpilot_daily, telemetry_flowpilot_failure_daily,
    telemetry_install_daily, telemetry_llm_call, telemetry_llm_daily, telemetry_perf_daily,
    telemetry_perf_metric, telemetry_session, telemetry_session_daily, telemetry_span,
};
use crate::telemetry::rollup::{latest_rolled_up_day, rollup_disabled};

const DEFAULT_INTERVAL_SECS: u64 = 3600;
const DEFAULT_EVENT_RETENTION_DAYS: i64 = 30;
const DEFAULT_ERROR_RETENTION_DAYS: i64 = 90;
const DEFAULT_SESSION_RETENTION_DAYS: i64 = 90;
const DEFAULT_LLM_RETENTION_DAYS: i64 = 30;
const DEFAULT_TRACE_RETENTION_DAYS: i64 = 7;
const DEFAULT_PERF_RETENTION_DAYS: i64 = 30;
const DEFAULT_ALERT_EVENT_RETENTION_DAYS: i64 = 180;
const DEFAULT_ROLLUP_RETENTION_DAYS: i64 = 400;
const MIN_RETENTION_DAYS: i64 = 1;
const MIN_INTERVAL_SECS: u64 = 1;

/// Number of rows removed by a single sweep, per table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TelemetrySweepResult {
    pub events_deleted: u64,
    pub errors_deleted: u64,
    pub sessions_deleted: u64,
    pub llm_deleted: u64,
    pub spans_deleted: u64,
    pub perf_deleted: u64,
    pub alert_events_deleted: u64,
    pub rollups_deleted: u64,
}

impl TelemetrySweepResult {
    pub fn total(&self) -> u64 {
        self.events_deleted
            + self.errors_deleted
            + self.sessions_deleted
            + self.llm_deleted
            + self.spans_deleted
            + self.perf_deleted
            + self.alert_events_deleted
            + self.rollups_deleted
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

/// Configuration for the telemetry retention sweeper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TelemetrySweeperConfig {
    pub interval: Duration,
    pub event_retention_days: i64,
    pub error_retention_days: i64,
    pub session_retention_days: i64,
    pub llm_retention_days: i64,
    pub trace_retention_days: i64,
    pub perf_retention_days: i64,
    pub alert_event_retention_days: i64,
    pub rollup_retention_days: i64,
}

impl Default for TelemetrySweeperConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
            event_retention_days: DEFAULT_EVENT_RETENTION_DAYS,
            error_retention_days: DEFAULT_ERROR_RETENTION_DAYS,
            session_retention_days: DEFAULT_SESSION_RETENTION_DAYS,
            llm_retention_days: DEFAULT_LLM_RETENTION_DAYS,
            trace_retention_days: DEFAULT_TRACE_RETENTION_DAYS,
            perf_retention_days: DEFAULT_PERF_RETENTION_DAYS,
            alert_event_retention_days: DEFAULT_ALERT_EVENT_RETENTION_DAYS,
            rollup_retention_days: DEFAULT_ROLLUP_RETENTION_DAYS,
        }
    }
}

impl TelemetrySweeperConfig {
    /// Build config from environment variables. Every window is a whole number
    /// of days and is clamped to at least one day.
    /// - `FLOW_LIKE_TELEMETRY_SWEEP_INTERVAL_SECS`: how often to sweep (default 3600)
    /// - `FLOW_LIKE_EVENT_RETENTION_DAYS`: raw product events (default 30)
    /// - `FLOW_LIKE_ERROR_RETENTION_DAYS`: raw error events (default 90)
    /// - `FLOW_LIKE_SESSION_RETENTION_DAYS`: raw sessions (default 90)
    /// - `FLOW_LIKE_LLM_RETENTION_DAYS`: raw LLM calls (default 30)
    /// - `FLOW_LIKE_TRACE_RETENTION_DAYS`: spans (default 7)
    /// - `FLOW_LIKE_PERF_RETENTION_DAYS`: web-vitals samples (default 30)
    /// - `FLOW_LIKE_ALERT_EVENT_RETENTION_DAYS`: alert inbox rows (default 180)
    /// - `FLOW_LIKE_ROLLUP_RETENTION_DAYS`: every `*Daily` rollup (default 400)
    pub fn from_env() -> Self {
        Self {
            interval: parse_interval(
                std::env::var("FLOW_LIKE_TELEMETRY_SWEEP_INTERVAL_SECS")
                    .ok()
                    .as_deref(),
            ),
            event_retention_days: retention_days_from_env(
                "FLOW_LIKE_EVENT_RETENTION_DAYS",
                DEFAULT_EVENT_RETENTION_DAYS,
            ),
            error_retention_days: retention_days_from_env(
                "FLOW_LIKE_ERROR_RETENTION_DAYS",
                DEFAULT_ERROR_RETENTION_DAYS,
            ),
            session_retention_days: retention_days_from_env(
                "FLOW_LIKE_SESSION_RETENTION_DAYS",
                DEFAULT_SESSION_RETENTION_DAYS,
            ),
            llm_retention_days: retention_days_from_env(
                "FLOW_LIKE_LLM_RETENTION_DAYS",
                DEFAULT_LLM_RETENTION_DAYS,
            ),
            trace_retention_days: retention_days_from_env(
                "FLOW_LIKE_TRACE_RETENTION_DAYS",
                DEFAULT_TRACE_RETENTION_DAYS,
            ),
            perf_retention_days: retention_days_from_env(
                "FLOW_LIKE_PERF_RETENTION_DAYS",
                DEFAULT_PERF_RETENTION_DAYS,
            ),
            alert_event_retention_days: retention_days_from_env(
                "FLOW_LIKE_ALERT_EVENT_RETENTION_DAYS",
                DEFAULT_ALERT_EVENT_RETENTION_DAYS,
            ),
            rollup_retention_days: retention_days_from_env(
                "FLOW_LIKE_ROLLUP_RETENTION_DAYS",
                DEFAULT_ROLLUP_RETENTION_DAYS,
            ),
        }
    }
}

/// How far the rollups have caught up, and therefore how far the raw tables may
/// be swept. See the ORDERING INVARIANT in the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollupFloor {
    /// Rollups exist through this day; raw rows are safe to delete up to the
    /// start of it, i.e. through the end of the previous day.
    UpTo(NaiveDateTime),
    /// The rollup job is enabled but has never produced a day. Hold everything.
    Pending,
    /// The rollup job is switched off, so retention is the operator's call.
    Disabled,
}

/// Spawn the telemetry retention sweeper as a background task.
///
/// Returns `None` if `FLOW_LIKE_TELEMETRY_SWEEP_DISABLED=1` is set, otherwise
/// the join handle of the spawned task. The task runs forever and is expected
/// to be aborted on process shutdown.
pub fn spawn_telemetry_sweeper(
    db: Arc<DatabaseConnection>,
    config: TelemetrySweeperConfig,
) -> Option<JoinHandle<()>> {
    if sweeper_disabled() {
        tracing::info!("Telemetry sweeper disabled via FLOW_LIKE_TELEMETRY_SWEEP_DISABLED");
        return None;
    }

    tracing::info!(
        interval_secs = config.interval.as_secs(),
        event_retention_days = config.event_retention_days,
        error_retention_days = config.error_retention_days,
        session_retention_days = config.session_retention_days,
        llm_retention_days = config.llm_retention_days,
        trace_retention_days = config.trace_retention_days,
        perf_retention_days = config.perf_retention_days,
        alert_event_retention_days = config.alert_event_retention_days,
        rollup_retention_days = config.rollup_retention_days,
        "Spawning telemetry retention sweeper"
    );

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // First tick fires immediately; let services come up before we hit the DB.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            match sweep_once(db.as_ref(), &config).await {
                Ok(result) if result.is_empty() => {}
                Ok(result) => tracing::info!(
                    events_deleted = result.events_deleted,
                    errors_deleted = result.errors_deleted,
                    sessions_deleted = result.sessions_deleted,
                    llm_deleted = result.llm_deleted,
                    spans_deleted = result.spans_deleted,
                    perf_deleted = result.perf_deleted,
                    alert_events_deleted = result.alert_events_deleted,
                    rollups_deleted = result.rollups_deleted,
                    "Telemetry sweeper removed expired rows"
                ),
                Err(e) => tracing::error!(error = %e, "Telemetry sweeper iteration failed"),
            }
        }
    });

    Some(handle)
}

/// Run one retention sweep. Returns the number of rows deleted per table.
///
/// Exposed for tests, for the spawned task, and for the Admin-gated
/// `POST /admin/telemetry/sweep` endpoint used by serverless deployments.
pub async fn sweep_once(
    db: &DatabaseConnection,
    config: &TelemetrySweeperConfig,
) -> Result<TelemetrySweepResult, DbErr> {
    let now = Utc::now().naive_utc();
    let floor = rollup_floor(db).await?;
    let mut result = TelemetrySweepResult::default();

    // Raw tables that feed a rollup: cutoff clamped to the rollup watermark.
    if let Some(cutoff) = clamp_cutoff(retention_cutoff(now, config.event_retention_days), floor) {
        result.events_deleted = TelemetryEvent::delete_many()
            .filter(telemetry_event::Column::CreatedAt.lt(cutoff))
            .exec(db)
            .await?
            .rows_affected;
    }

    if let Some(cutoff) = clamp_cutoff(retention_cutoff(now, config.error_retention_days), floor) {
        result.errors_deleted = TelemetryErrorEvent::delete_many()
            .filter(telemetry_error_event::Column::CreatedAt.lt(cutoff))
            .exec(db)
            .await?
            .rows_affected;
    }

    if let Some(cutoff) = clamp_cutoff(retention_cutoff(now, config.session_retention_days), floor)
    {
        result.sessions_deleted = TelemetrySession::delete_many()
            .filter(telemetry_session::Column::CreatedAt.lt(cutoff))
            .exec(db)
            .await?
            .rows_affected;
    }

    if let Some(cutoff) = clamp_cutoff(retention_cutoff(now, config.llm_retention_days), floor) {
        result.llm_deleted = TelemetryLlmCall::delete_many()
            .filter(telemetry_llm_call::Column::CreatedAt.lt(cutoff))
            .exec(db)
            .await?
            .rows_affected;
    }

    if let Some(cutoff) = clamp_cutoff(retention_cutoff(now, config.perf_retention_days), floor) {
        result.perf_deleted = TelemetryPerfMetric::delete_many()
            .filter(telemetry_perf_metric::Column::CreatedAt.lt(cutoff))
            .exec(db)
            .await?
            .rows_affected;
    }

    // Spans and alert inbox rows feed no rollup, so nothing can be lost by
    // deleting them on their own schedule.
    let span_cutoff = retention_cutoff(now, config.trace_retention_days);
    result.spans_deleted = TelemetrySpan::delete_many()
        .filter(telemetry_span::Column::CreatedAt.lt(span_cutoff))
        .exec(db)
        .await?
        .rows_affected;

    let alert_cutoff = retention_cutoff(now, config.alert_event_retention_days);
    result.alert_events_deleted = TelemetryAlertEvent::delete_many()
        .filter(telemetry_alert_event::Column::CreatedAt.lt(alert_cutoff))
        .exec(db)
        .await?
        .rows_affected;

    let rollup_cutoff = retention_cutoff(now, config.rollup_retention_days);
    result.rollups_deleted = sweep_rollups(db, rollup_cutoff).await?;

    Ok(result)
}

/// Rollups are keyed and swept by their `day`, not by insert time, so a day
/// leaves every rollup table at the same moment.
async fn sweep_rollups(db: &DatabaseConnection, cutoff: NaiveDateTime) -> Result<u64, DbErr> {
    macro_rules! delete_days_before {
        ($($entity:ty => $column:expr),+ $(,)?) => {{
            let mut deleted = 0u64;
            $(
                deleted += <$entity>::delete_many()
                    .filter($column.lt(cutoff))
                    .exec(db)
                    .await?
                    .rows_affected;
            )+
            deleted
        }};
    }

    Ok(delete_days_before!(
        TelemetryInstallDaily => telemetry_install_daily::Column::Day,
        TelemetryEventDaily => telemetry_event_daily::Column::Day,
        TelemetryDimensionDaily => telemetry_dimension_daily::Column::Day,
        TelemetrySessionDaily => telemetry_session_daily::Column::Day,
        TelemetryLlmDaily => telemetry_llm_daily::Column::Day,
        TelemetryPerfDaily => telemetry_perf_daily::Column::Day,
        TelemetryFlowpilotDaily => telemetry_flowpilot_daily::Column::Day,
        TelemetryFlowpilotFailureDaily => telemetry_flowpilot_failure_daily::Column::Day,
    ))
}

async fn rollup_floor(db: &DatabaseConnection) -> Result<RollupFloor, DbErr> {
    if rollup_disabled() {
        return Ok(RollupFloor::Disabled);
    }
    Ok(match latest_rolled_up_day(db).await? {
        Some(day) => RollupFloor::UpTo(day),
        None => RollupFloor::Pending,
    })
}

fn sweeper_disabled() -> bool {
    std::env::var("FLOW_LIKE_TELEMETRY_SWEEP_DISABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Apply the ORDERING INVARIANT to a raw retention cutoff.
///
/// Returns `None` when the table must not be swept at all this pass. The clamp
/// only ever pulls the cutoff *back*, so the failure mode of a lagging rollup
/// job is keeping data longer than configured — never losing it.
pub(crate) fn clamp_cutoff(cutoff: NaiveDateTime, floor: RollupFloor) -> Option<NaiveDateTime> {
    match floor {
        RollupFloor::Disabled => Some(cutoff),
        RollupFloor::Pending => None,
        RollupFloor::UpTo(day) => Some(cutoff.min(day - chrono::Duration::days(1))),
    }
}

pub(crate) fn retention_cutoff(now: NaiveDateTime, days: i64) -> NaiveDateTime {
    now - chrono::Duration::days(days.max(MIN_RETENTION_DAYS))
}

pub(crate) fn parse_retention_days(raw: Option<&str>, default: i64) -> i64 {
    raw.and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(default)
        .max(MIN_RETENTION_DAYS)
}

fn retention_days_from_env(key: &str, default: i64) -> i64 {
    parse_retention_days(std::env::var(key).ok().as_deref(), default)
}

pub(crate) fn parse_interval(raw: Option<&str>) -> Duration {
    let secs = raw
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(MIN_INTERVAL_SECS);
    Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn ts(day: u32, hour: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 7, day)
            .unwrap()
            .and_hms_opt(hour, 0, 0)
            .unwrap()
    }

    fn midnight(day: u32) -> NaiveDateTime {
        ts(day, 0)
    }

    #[test]
    fn cutoff_subtracts_the_retention_window() {
        assert_eq!(retention_cutoff(ts(20, 12), 7), ts(13, 12));
        assert_eq!(retention_cutoff(ts(31, 6), 30), ts(1, 6));
    }

    #[test]
    fn cutoff_clamps_non_positive_retention_to_one_day() {
        assert_eq!(retention_cutoff(ts(20, 12), 0), ts(19, 12));
        assert_eq!(retention_cutoff(ts(20, 12), -30), ts(19, 12));
    }

    #[test]
    fn retention_days_fall_back_to_the_default() {
        assert_eq!(parse_retention_days(None, 7), 7);
        assert_eq!(parse_retention_days(Some(""), 30), 30);
        assert_eq!(parse_retention_days(Some("forever"), 7), 7);
        assert_eq!(parse_retention_days(Some("3.5"), 7), 7);
    }

    #[test]
    fn retention_days_are_clamped_to_at_least_one_day() {
        assert_eq!(parse_retention_days(Some("0"), 7), 1);
        assert_eq!(parse_retention_days(Some("-14"), 7), 1);
        assert_eq!(parse_retention_days(Some(" 14 "), 7), 14);
    }

    #[test]
    fn every_retention_window_parses_and_clamps_the_same_way() {
        for default in [
            DEFAULT_EVENT_RETENTION_DAYS,
            DEFAULT_ERROR_RETENTION_DAYS,
            DEFAULT_SESSION_RETENTION_DAYS,
            DEFAULT_LLM_RETENTION_DAYS,
            DEFAULT_TRACE_RETENTION_DAYS,
            DEFAULT_PERF_RETENTION_DAYS,
            DEFAULT_ALERT_EVENT_RETENTION_DAYS,
            DEFAULT_ROLLUP_RETENTION_DAYS,
        ] {
            assert_eq!(parse_retention_days(None, default), default);
            assert_eq!(parse_retention_days(Some("not a number"), default), default);
            assert_eq!(parse_retention_days(Some("0"), default), 1);
            assert_eq!(parse_retention_days(Some("-1"), default), 1);
            assert_eq!(parse_retention_days(Some("365"), default), 365);
        }
    }

    #[test]
    fn interval_defaults_and_never_reaches_zero() {
        assert_eq!(parse_interval(None), Duration::from_secs(3600));
        assert_eq!(parse_interval(Some("nope")), Duration::from_secs(3600));
        assert_eq!(parse_interval(Some("0")), Duration::from_secs(1));
        assert_eq!(parse_interval(Some("120")), Duration::from_secs(120));
    }

    #[test]
    fn default_config_matches_the_documented_retention_windows() {
        let config = TelemetrySweeperConfig::default();
        assert_eq!(config.interval, Duration::from_secs(3600));
        assert_eq!(config.event_retention_days, 30);
        assert_eq!(config.error_retention_days, 90);
        assert_eq!(config.session_retention_days, 90);
        assert_eq!(config.llm_retention_days, 30);
        assert_eq!(config.trace_retention_days, 7);
        assert_eq!(config.perf_retention_days, 30);
        assert_eq!(config.alert_event_retention_days, 180);
        assert_eq!(config.rollup_retention_days, 400);
        assert!(TelemetrySweepResult::default().is_empty());
    }

    #[test]
    fn sweep_result_totals_every_table() {
        let result = TelemetrySweepResult {
            events_deleted: 1,
            errors_deleted: 2,
            sessions_deleted: 3,
            llm_deleted: 4,
            spans_deleted: 5,
            perf_deleted: 6,
            alert_events_deleted: 7,
            rollups_deleted: 8,
        };
        assert_eq!(result.total(), 36);
        assert!(!result.is_empty());
    }

    #[test]
    fn caught_up_rollups_never_hold_a_cutoff_back() {
        // Rolled up through today; a 30-day cutoff is nowhere near the clamp.
        let floor = RollupFloor::UpTo(midnight(26));
        let cutoff = retention_cutoff(ts(26, 9), 30);
        assert_eq!(clamp_cutoff(cutoff, floor), Some(cutoff));
    }

    #[test]
    fn lagging_rollups_pull_the_cutoff_back_to_the_last_complete_day() {
        // The rollup job last completed the 10th while the sweeper runs on the
        // 26th: a 7-day cutoff (the 19th) would delete raw rows for the 11th
        // through the 19th that were never aggregated.
        let floor = RollupFloor::UpTo(midnight(10));
        let cutoff = retention_cutoff(ts(26, 9), 7);
        assert_eq!(clamp_cutoff(cutoff, floor), Some(midnight(9)));
        assert!(clamp_cutoff(cutoff, floor).unwrap() < cutoff);
    }

    #[test]
    fn the_newest_rolled_up_day_is_never_swept_because_it_is_still_filling() {
        let floor = RollupFloor::UpTo(midnight(26));
        // A one-day retention on the same day would otherwise delete rows the
        // current rollup pass has not finished aggregating.
        let cutoff = retention_cutoff(ts(26, 23), 1);
        assert_eq!(clamp_cutoff(cutoff, floor), Some(midnight(25)));
    }

    #[test]
    fn nothing_is_swept_until_the_first_rollup_lands() {
        assert_eq!(
            clamp_cutoff(retention_cutoff(ts(26, 9), 30), RollupFloor::Pending),
            None
        );
    }

    #[test]
    fn disabling_the_rollup_job_hands_retention_back_to_the_operator() {
        let cutoff = retention_cutoff(ts(26, 9), 30);
        assert_eq!(clamp_cutoff(cutoff, RollupFloor::Disabled), Some(cutoff));
    }
}
