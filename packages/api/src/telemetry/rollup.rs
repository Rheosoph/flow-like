//! Daily rollup of the raw telemetry tables.
//!
//! Raw telemetry is written per event and swept on a retention schedule. Every
//! admin query whose window is longer than 48h reads these day-granular
//! aggregates instead, so long-range dashboards never scan (or silently
//! truncate) raw rows. The job mirrors `telemetry::sweeper`: an in-process
//! ticker for long-lived deployments, and `POST /admin/telemetry/rollup` for
//! serverless ones, both running the exact same `rollup_once`.
//!
//! Idempotency: every pass recomputes each day in the backfill window from raw
//! rows and UPSERTs the result, so running it twice can never double-count.
//! Late-arriving rows (the desktop client buffers events while offline) are
//! picked up because the window always covers the last N days.
//!
//! Bucketing: rows are assigned to a day by `createdAt`, the server-side ingest
//! timestamp. That is deliberately the same column the sweeper deletes by — a
//! row can therefore never be swept before the day it was rolled into, and a
//! late-arriving row always lands inside the backfill window.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, NaiveDateTime, Utc};
use flow_like_types::create_id;
use flow_like_types::tokio::{self, task::JoinHandle};
use sea_orm::sea_query::{
    Alias, Expr, Func, OnConflict, Order as SeaOrder, Query as SeaQuery, SimpleExpr,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, DbErr,
    EntityTrait, FromQueryResult, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Set,
    Statement,
};

use crate::entity::{
    telemetry_dimension_daily, telemetry_error_event, telemetry_event, telemetry_event_daily,
    telemetry_flowpilot_daily, telemetry_install_daily, telemetry_llm_call, telemetry_llm_daily,
    telemetry_perf_daily, telemetry_perf_metric, telemetry_session, telemetry_session_daily,
};

const DEFAULT_INTERVAL_SECS: u64 = 3600;
const MIN_INTERVAL_SECS: u64 = 60;
const DEFAULT_BACKFILL_DAYS: i64 = 3;
const MIN_BACKFILL_DAYS: i64 = 1;
const MAX_BACKFILL_DAYS: i64 = 90;

/// Distinct keys a day may keep before the remainder is folded together.
pub const ROLLUP_TOP_N: usize = 200;
/// Key the folded long tail is stored under.
pub const OTHER_KEY: &str = "__other__";
/// Stand-in for a `NULL` dimension value or an unknown release.
pub const UNKNOWN_VALUE: &str = "unknown";
/// Synthetic anon id used by server-side events; never counted as an install.
const BACKEND_ANON_ID: &str = "backend";
/// Dimensions broken out into `TelemetryDimensionDaily`.
pub const ROLLUP_DIMENSIONS: [&str; 4] = ["platform", "country", "app_version", "source"];

const FLOWPILOT_METRICS_EVENT: &str = "flowpilot_generation_metrics";
const CRASHED_STATUS: &str = "crashed";
/// Non-crash unhealthy session statuses. Together with `ok` and `crashed` this
/// partitions the session status vocabulary exactly once.
const ERRORED_STATUSES: [&str; 2] = ["errored", "abnormal"];
const LLM_ERROR_STATUS: &str = "error";

/// Upper bound on the grouped rows a single day fetches. Only affects which
/// keys win the top-`ROLLUP_TOP_N` ranking — totals stay exact because the
/// remainder is aggregated by a separate query.
const GROUP_ROW_CAP: u64 = 5_000;
/// Upper bound on the distinct installs a single day materialises in memory.
const INSTALL_ROW_CAP: u64 = 250_000;
const FLOWPILOT_ROW_CAP: u64 = 100_000;
/// Only used by the non-Postgres percentile fallback.
const PERF_ROW_CAP: u64 = 200_000;
const INSERT_CHUNK: usize = 500;

/// Rows upserted by a single rollup pass, per rollup table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TelemetryRollupResult {
    pub days: u64,
    pub installs_upserted: u64,
    pub events_upserted: u64,
    pub dimensions_upserted: u64,
    pub sessions_upserted: u64,
    pub llm_upserted: u64,
    pub perf_upserted: u64,
    pub flowpilot_upserted: u64,
}

impl TelemetryRollupResult {
    pub fn total(&self) -> u64 {
        self.installs_upserted
            + self.events_upserted
            + self.dimensions_upserted
            + self.sessions_upserted
            + self.llm_upserted
            + self.perf_upserted
            + self.flowpilot_upserted
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

/// Configuration for the telemetry rollup job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TelemetryRollupConfig {
    pub interval: Duration,
    /// Days recomputed every pass, counting back from today.
    pub backfill_days: i64,
}

impl Default for TelemetryRollupConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
            backfill_days: DEFAULT_BACKFILL_DAYS,
        }
    }
}

impl TelemetryRollupConfig {
    /// Build config from environment variables.
    /// - `FLOW_LIKE_TELEMETRY_ROLLUP_INTERVAL_SECS`: how often to roll up (default 3600, minimum 60)
    /// - `FLOW_LIKE_TELEMETRY_ROLLUP_BACKFILL_DAYS`: days recomputed per pass (default 3, 1..=90)
    pub fn from_env() -> Self {
        Self {
            interval: parse_interval(
                std::env::var("FLOW_LIKE_TELEMETRY_ROLLUP_INTERVAL_SECS")
                    .ok()
                    .as_deref(),
            ),
            backfill_days: parse_backfill_days(
                std::env::var("FLOW_LIKE_TELEMETRY_ROLLUP_BACKFILL_DAYS")
                    .ok()
                    .as_deref(),
            ),
        }
    }
}

/// Whether the operator switched the rollup job off.
///
/// The sweeper reads this too: with rollups disabled nobody will ever aggregate
/// the raw rows, so holding them back forever would only grow the tables the
/// job exists to bound.
pub fn rollup_disabled() -> bool {
    std::env::var("FLOW_LIKE_TELEMETRY_ROLLUP_DISABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Spawn the telemetry rollup job as a background task.
///
/// Returns `None` if `FLOW_LIKE_TELEMETRY_ROLLUP_DISABLED=1` is set, otherwise
/// the join handle of the spawned task. The task runs forever and is expected
/// to be aborted on process shutdown.
pub fn spawn_telemetry_rollup(
    db: Arc<DatabaseConnection>,
    config: TelemetryRollupConfig,
) -> Option<JoinHandle<()>> {
    if rollup_disabled() {
        tracing::info!("Telemetry rollup disabled via FLOW_LIKE_TELEMETRY_ROLLUP_DISABLED");
        return None;
    }

    tracing::info!(
        interval_secs = config.interval.as_secs(),
        backfill_days = config.backfill_days,
        "Spawning telemetry rollup job"
    );

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // First tick fires immediately; let services come up before we hit the DB.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            match rollup_once(db.as_ref(), &config).await {
                Ok(result) if result.is_empty() => {}
                Ok(result) => tracing::info!(
                    days = result.days,
                    installs = result.installs_upserted,
                    events = result.events_upserted,
                    dimensions = result.dimensions_upserted,
                    sessions = result.sessions_upserted,
                    llm = result.llm_upserted,
                    perf = result.perf_upserted,
                    flowpilot = result.flowpilot_upserted,
                    "Telemetry rollup refreshed daily aggregates"
                ),
                Err(e) => tracing::error!(error = %e, "Telemetry rollup iteration failed"),
            }
        }
    });

    Some(handle)
}

/// Recompute and upsert every daily rollup for each day in the backfill window.
///
/// Exposed for tests, for the spawned task, and for the Admin-gated
/// `POST /admin/telemetry/rollup` endpoint used by serverless deployments.
pub async fn rollup_once(
    db: &DatabaseConnection,
    config: &TelemetryRollupConfig,
) -> Result<TelemetryRollupResult, DbErr> {
    let now = Utc::now().naive_utc();
    let mut result = TelemetryRollupResult::default();

    for day in rollup_days(now, config.backfill_days) {
        let next = day + ChronoDuration::days(1);
        result.days += 1;
        result.installs_upserted += rollup_installs(db, day, next).await?;
        result.events_upserted += rollup_events(db, day, next).await?;
        result.dimensions_upserted += rollup_dimensions(db, day, next).await?;
        result.sessions_upserted += rollup_sessions(db, day, next).await?;
        result.llm_upserted += rollup_llm(db, day, next).await?;
        result.perf_upserted += rollup_perf(db, day, next).await?;
        result.flowpilot_upserted += rollup_flowpilot(db, day, next).await?;
    }

    Ok(result)
}

/// Most recent day that has an install rollup, i.e. how far the aggregates have
/// caught up with the raw tables. The sweeper clamps its retention cutoffs to
/// this so raw rows can never be deleted before they were aggregated.
pub async fn latest_rolled_up_day<C: ConnectionTrait>(
    db: &C,
) -> Result<Option<NaiveDateTime>, DbErr> {
    Ok(telemetry_install_daily::Entity::find()
        .select_only()
        .column_as(telemetry_install_daily::Column::Day.max(), "day")
        .into_model::<MaxDayRow>()
        .one(db)
        .await?
        .and_then(|row| row.day))
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// UTC midnight of the day `ts` falls in.
pub fn day_start(ts: NaiveDateTime) -> NaiveDateTime {
    ts.date()
        .and_hms_opt(0, 0, 0)
        .unwrap_or_else(|| ts.date().and_time(chrono::NaiveTime::MIN))
}

/// Days recomputed by one pass, oldest first, always including today.
pub(crate) fn rollup_days(now: NaiveDateTime, backfill_days: i64) -> Vec<NaiveDateTime> {
    let days = backfill_days.clamp(MIN_BACKFILL_DAYS, MAX_BACKFILL_DAYS);
    let today = day_start(now);
    (0..days)
        .rev()
        .map(|offset| today - ChronoDuration::days(offset))
        .collect()
}

pub(crate) fn parse_interval(raw: Option<&str>) -> Duration {
    let secs = raw
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(MIN_INTERVAL_SECS);
    Duration::from_secs(secs)
}

pub(crate) fn parse_backfill_days(raw: Option<&str>) -> i64 {
    raw.and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(DEFAULT_BACKFILL_DAYS)
        .clamp(MIN_BACKFILL_DAYS, MAX_BACKFILL_DAYS)
}

/// Saturating narrowing for counter columns that are `Int` in the schema.
pub(crate) fn to_i32(value: i64) -> i32 {
    value.clamp(0, i32::MAX as i64) as i32
}

/// Linear-interpolating percentile over an ascending slice, matching the
/// semantics of SQL `percentile_cont` so the Postgres and fallback paths agree.
pub(crate) fn percentile_cont(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }

    let pos = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lower = pos.floor();
    let upper = pos.ceil();
    if (upper - lower).abs() < f64::EPSILON {
        return sorted[lower as usize];
    }

    let low = sorted[lower as usize];
    let high = sorted[upper as usize];
    low + (pos - lower) * (high - low)
}

pub(crate) fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// One grouped `(key, bucket)` count as read back from the database.
#[derive(Clone, Debug, PartialEq, Eq, FromQueryResult)]
pub(crate) struct GroupedCount {
    pub key: String,
    pub bucket: String,
    pub cnt: i64,
    pub installs: i64,
}

#[derive(Debug, FromQueryResult)]
struct BucketCount {
    bucket: String,
    cnt: i64,
    installs: i64,
}

#[derive(Debug, Default, FromQueryResult)]
struct TotalCount {
    cnt: i64,
    installs: i64,
}

#[derive(Debug, FromQueryResult)]
struct MaxDayRow {
    day: Option<NaiveDateTime>,
}

#[derive(Debug, FromQueryResult)]
struct InstallRow {
    anon_id: String,
    source: String,
}

/// Which keys a day keeps under their own name and how many were folded away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LongTailSplit {
    pub kept: Vec<GroupedCount>,
    pub kept_keys: Vec<String>,
    pub folded_keys: usize,
}

/// Bound the long tail: rank keys by their total count across buckets, keep the
/// top `cap`, and report the rest so the caller can aggregate them into a
/// single `__other__` row. Ties break on the key so the split is deterministic
/// and therefore idempotent across passes.
pub(crate) fn split_long_tail(rows: Vec<GroupedCount>, cap: usize) -> LongTailSplit {
    let mut totals: HashMap<String, i64> = HashMap::new();
    for row in &rows {
        *totals.entry(row.key.clone()).or_default() += row.cnt;
    }

    if totals.len() <= cap {
        let mut kept_keys: Vec<String> = totals.into_keys().collect();
        kept_keys.sort();
        return LongTailSplit {
            kept: rows,
            kept_keys,
            folded_keys: 0,
        };
    }

    let mut ranked: Vec<(String, i64)> = totals.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let folded_keys = ranked.len() - cap;
    let kept_set: HashSet<String> = ranked.into_iter().take(cap).map(|(key, _)| key).collect();
    let mut kept_keys: Vec<String> = kept_set.iter().cloned().collect();
    kept_keys.sort();

    let kept = rows
        .into_iter()
        .filter(|row| kept_set.contains(&row.key))
        .collect();

    LongTailSplit {
        kept,
        kept_keys,
        folded_keys,
    }
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

macro_rules! flowpilot_counters {
    ($($field:ident),* $(,)?) => {
        /// Daily sums of the counters an `IFlowPilotProductionMetrics` payload
        /// carries. Field names match the props keys the client emits.
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub(crate) struct FlowPilotCounters {
            $(pub $field: i64,)*
        }

        impl FlowPilotCounters {
            fn add(&mut self, props: Option<&serde_json::Value>) {
                $(
                    self.$field = self
                        .$field
                        .saturating_add(counter_value(props, stringify!($field)));
                )*
            }
        }
    };
}

flowpilot_counters!(
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
);

#[derive(Debug, FromQueryResult)]
struct FlowPilotRow {
    anon_id: String,
    props: Option<serde_json::Value>,
}

fn fold_flowpilot(rows: &[FlowPilotRow]) -> (FlowPilotCounters, i64) {
    let mut counters = FlowPilotCounters::default();
    let mut installs: HashSet<&str> = HashSet::new();
    for row in rows {
        counters.add(row.props.as_ref());
        if row.anon_id != BACKEND_ANON_ID {
            installs.insert(row.anon_id.as_str());
        }
    }
    (counters, installs.len() as i64)
}

#[derive(Debug, FromQueryResult)]
struct PerfSampleRow {
    metric: String,
    source: String,
    value: f64,
}

#[derive(Debug, FromQueryResult)]
struct PerfDailyRow {
    metric: String,
    source: String,
    cnt: i64,
    p50: f64,
    p75: f64,
    p95: f64,
}

/// Percentiles per `(metric, source)` from raw samples, matching the SQL path.
fn fold_perf_samples(rows: Vec<PerfSampleRow>) -> Vec<PerfDailyRow> {
    let mut grouped: HashMap<(String, String), Vec<f64>> = HashMap::new();
    for row in rows {
        grouped
            .entry((row.metric, row.source))
            .or_default()
            .push(row.value);
    }

    let mut folded: Vec<PerfDailyRow> = grouped
        .into_iter()
        .map(|((metric, source), mut values)| {
            values.sort_by(|a, b| a.total_cmp(b));
            PerfDailyRow {
                metric,
                source,
                cnt: values.len() as i64,
                p50: percentile_cont(&values, 0.5),
                p75: percentile_cont(&values, 0.75),
                p95: percentile_cont(&values, 0.95),
            }
        })
        .collect();
    folded.sort_by(|a, b| {
        a.metric
            .cmp(&b.metric)
            .then_with(|| a.source.cmp(&b.source))
    });
    folded
}

// ---------------------------------------------------------------------------
// Shared query fragments
// ---------------------------------------------------------------------------

/// `COUNT(DISTINCT anonId)` over telemetry events, ignoring the synthetic
/// backend id so server-side events never inflate install counts.
fn event_installs_expr() -> SimpleExpr {
    Func::count_distinct(
        Expr::case(
            Expr::col(telemetry_event::Column::AnonId).eq(BACKEND_ANON_ID),
            sea_orm::Value::String(None),
        )
        .finally(Expr::col(telemetry_event::Column::AnonId)),
    )
    .into()
}

fn event_count_expr() -> SimpleExpr {
    Expr::col(telemetry_event::Column::Id).count()
}

/// `COALESCE(<column>, 'unknown')` — used identically in the projection, the
/// `GROUP BY` and the long-tail `NOT IN`, so all three agree on the key.
fn coalesced_key(column: impl sea_orm::sea_query::IntoColumnRef) -> SimpleExpr {
    Func::coalesce([
        Expr::col(column).into(),
        Expr::val(UNKNOWN_VALUE.to_string()).into(),
    ])
    .into()
}

fn dimension_column(dimension: &str) -> Option<telemetry_event::Column> {
    match dimension {
        "platform" => Some(telemetry_event::Column::Platform),
        "country" => Some(telemetry_event::Column::Country),
        "app_version" => Some(telemetry_event::Column::AppVersion),
        "source" => Some(telemetry_event::Column::Source),
        _ => None,
    }
}

/// `COUNT([DISTINCT] CASE WHEN <status matches> THEN <column> END)`.
fn session_status_count(
    column: telemetry_session::Column,
    statuses: &[&str],
    distinct: bool,
) -> SimpleExpr {
    let case = Expr::case(
        Expr::col(telemetry_session::Column::Status).is_in(statuses.iter().map(|s| s.to_string())),
        Expr::col(column),
    )
    .finally(sea_orm::Value::String(None));
    if distinct {
        Expr::expr(case).count_distinct()
    } else {
        Expr::expr(case).count()
    }
}

/// `CAST(COALESCE(SUM(<column>), 0) AS BIGINT)` — CockroachDB returns DECIMAL
/// for `SUM` over an integer column, which would not deserialize into `i64`.
fn sum_bigint(column: telemetry_llm_call::Column) -> SimpleExpr {
    Expr::expr(Func::coalesce([
        Func::sum(Expr::col(column)).into(),
        Expr::val(0i64).into(),
    ]))
    .cast_as(Alias::new("BIGINT"))
}

async fn upsert_chunked<E, A, C>(db: &C, models: Vec<A>, conflict: OnConflict) -> Result<u64, DbErr>
where
    E: EntityTrait,
    E::Model: IntoActiveModel<A>,
    A: ActiveModelTrait<Entity = E> + Clone + Send,
    C: ConnectionTrait,
{
    let total = models.len() as u64;
    for chunk in models.chunks(INSERT_CHUNK) {
        E::insert_many(chunk.to_vec())
            .on_conflict(conflict.clone())
            .exec_without_returning(db)
            .await?;
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// TelemetryInstallDaily
// ---------------------------------------------------------------------------

async fn install_pairs<C: ConnectionTrait>(
    db: &C,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<Vec<(String, String)>, DbErr> {
    let mut pairs: HashSet<(String, String)> = HashSet::new();

    let events = telemetry_event::Entity::find()
        .select_only()
        .column_as(telemetry_event::Column::AnonId, "anon_id")
        .column_as(telemetry_event::Column::Source, "source")
        .filter(telemetry_event::Column::CreatedAt.gte(day))
        .filter(telemetry_event::Column::CreatedAt.lt(next))
        .filter(telemetry_event::Column::AnonId.ne(BACKEND_ANON_ID))
        .distinct()
        .limit(INSTALL_ROW_CAP)
        .into_model::<InstallRow>()
        .all(db)
        .await?;

    let sessions = telemetry_session::Entity::find()
        .select_only()
        .column_as(telemetry_session::Column::AnonId, "anon_id")
        .column_as(telemetry_session::Column::Source, "source")
        .filter(telemetry_session::Column::CreatedAt.gte(day))
        .filter(telemetry_session::Column::CreatedAt.lt(next))
        .filter(telemetry_session::Column::AnonId.ne(BACKEND_ANON_ID))
        .distinct()
        .limit(INSTALL_ROW_CAP)
        .into_model::<InstallRow>()
        .all(db)
        .await?;

    let errors = telemetry_error_event::Entity::find()
        .select_only()
        .column_as(telemetry_error_event::Column::AnonId, "anon_id")
        .column_as(telemetry_error_event::Column::Source, "source")
        .filter(telemetry_error_event::Column::CreatedAt.gte(day))
        .filter(telemetry_error_event::Column::CreatedAt.lt(next))
        .filter(telemetry_error_event::Column::AnonId.ne(BACKEND_ANON_ID))
        .distinct()
        .limit(INSTALL_ROW_CAP)
        .into_model::<InstallRow>()
        .all(db)
        .await?;

    for row in events.into_iter().chain(sessions).chain(errors) {
        pairs.insert((row.anon_id, row.source));
    }

    if pairs.len() as u64 >= INSTALL_ROW_CAP {
        tracing::warn!(
            cap = INSTALL_ROW_CAP,
            day = %day,
            "Telemetry install rollup hit its row cap; the day is incomplete"
        );
    }

    let mut pairs: Vec<(String, String)> = pairs.into_iter().collect();
    pairs.sort();
    Ok(pairs)
}

async fn rollup_installs(
    db: &DatabaseConnection,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<u64, DbErr> {
    let pairs = install_pairs(db, day, next).await?;
    if pairs.is_empty() {
        return Ok(0);
    }

    let now = Utc::now().naive_utc();
    let models: Vec<telemetry_install_daily::ActiveModel> = pairs
        .into_iter()
        .map(|(anon_id, source)| telemetry_install_daily::ActiveModel {
            id: Set(create_id()),
            day: Set(day),
            anon_id: Set(anon_id),
            source: Set(source),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .collect();

    // The row carries no value beyond its key, so a conflict has nothing to
    // update — `DO NOTHING` keeps the pass idempotent without rewriting rows.
    upsert_chunked(db, models, install_on_conflict()).await
}

fn install_on_conflict() -> OnConflict {
    OnConflict::columns([
        telemetry_install_daily::Column::Day,
        telemetry_install_daily::Column::AnonId,
        telemetry_install_daily::Column::Source,
    ])
    .do_nothing()
    .to_owned()
}

// ---------------------------------------------------------------------------
// TelemetryEventDaily
// ---------------------------------------------------------------------------

async fn event_name_counts<C: ConnectionTrait>(
    db: &C,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<Vec<GroupedCount>, DbErr> {
    let mut query = SeaQuery::select();
    query
        .from(telemetry_event::Entity)
        .expr_as(Expr::col(telemetry_event::Column::Name), Alias::new("key"))
        .expr_as(
            Expr::col(telemetry_event::Column::Source),
            Alias::new("bucket"),
        )
        .expr_as(event_count_expr(), Alias::new("cnt"))
        .expr_as(event_installs_expr(), Alias::new("installs"))
        .and_where(Expr::col(telemetry_event::Column::CreatedAt).gte(day))
        .and_where(Expr::col(telemetry_event::Column::CreatedAt).lt(next))
        .add_group_by([
            Expr::col(telemetry_event::Column::Name).into(),
            Expr::col(telemetry_event::Column::Source).into(),
        ])
        .order_by_expr(event_count_expr(), SeaOrder::Desc)
        .limit(GROUP_ROW_CAP);

    let stmt = db.get_database_backend().build(&query);
    GroupedCount::find_by_statement(stmt).all(db).await
}

async fn event_name_tail<C: ConnectionTrait>(
    db: &C,
    day: NaiveDateTime,
    next: NaiveDateTime,
    kept_keys: &[String],
) -> Result<Vec<BucketCount>, DbErr> {
    let mut query = SeaQuery::select();
    query
        .from(telemetry_event::Entity)
        .expr_as(
            Expr::col(telemetry_event::Column::Source),
            Alias::new("bucket"),
        )
        .expr_as(event_count_expr(), Alias::new("cnt"))
        .expr_as(event_installs_expr(), Alias::new("installs"))
        .and_where(Expr::col(telemetry_event::Column::CreatedAt).gte(day))
        .and_where(Expr::col(telemetry_event::Column::CreatedAt).lt(next))
        .and_where(Expr::col(telemetry_event::Column::Name).is_not_in(kept_keys.to_vec()))
        .add_group_by([Expr::col(telemetry_event::Column::Source).into()]);

    let stmt = db.get_database_backend().build(&query);
    BucketCount::find_by_statement(stmt).all(db).await
}

async fn rollup_events(
    db: &DatabaseConnection,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<u64, DbErr> {
    let grouped = event_name_counts(db, day, next).await?;
    // The grouped read is capped, so a day past the cap has keys the split
    // never saw. Both cases route the remainder through the exact tail query.
    let capped = grouped.len() as u64 >= GROUP_ROW_CAP;
    let split = split_long_tail(grouped, ROLLUP_TOP_N);
    let mut rows = split.kept;

    if split.folded_keys > 0 || capped {
        tracing::debug!(
            day = %day,
            folded_keys = split.folded_keys,
            capped,
            "Folding telemetry event long tail into {}",
            OTHER_KEY
        );
        for tail in event_name_tail(db, day, next, &split.kept_keys).await? {
            rows.push(GroupedCount {
                key: OTHER_KEY.to_string(),
                bucket: tail.bucket,
                cnt: tail.cnt,
                installs: tail.installs,
            });
        }
    }

    if rows.is_empty() {
        return Ok(0);
    }

    let now = Utc::now().naive_utc();
    let models: Vec<telemetry_event_daily::ActiveModel> = rows
        .into_iter()
        .map(|row| telemetry_event_daily::ActiveModel {
            id: Set(create_id()),
            day: Set(day),
            name: Set(row.key),
            source: Set(row.bucket),
            count: Set(to_i32(row.cnt)),
            installs: Set(to_i32(row.installs)),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .collect();

    upsert_chunked(db, models, event_daily_on_conflict()).await
}

fn event_daily_on_conflict() -> OnConflict {
    OnConflict::columns([
        telemetry_event_daily::Column::Day,
        telemetry_event_daily::Column::Name,
        telemetry_event_daily::Column::Source,
    ])
    .update_columns([
        telemetry_event_daily::Column::Count,
        telemetry_event_daily::Column::Installs,
        telemetry_event_daily::Column::UpdatedAt,
    ])
    .to_owned()
}

// ---------------------------------------------------------------------------
// TelemetryDimensionDaily
// ---------------------------------------------------------------------------

async fn dimension_counts<C: ConnectionTrait>(
    db: &C,
    day: NaiveDateTime,
    next: NaiveDateTime,
    dimension: &str,
    column: telemetry_event::Column,
) -> Result<Vec<GroupedCount>, DbErr> {
    let mut query = SeaQuery::select();
    query
        .from(telemetry_event::Entity)
        .expr_as(coalesced_key(column), Alias::new("key"))
        .expr_as(
            Expr::val(dimension.to_string()).cast_as(Alias::new("TEXT")),
            Alias::new("bucket"),
        )
        .expr_as(event_count_expr(), Alias::new("cnt"))
        .expr_as(event_installs_expr(), Alias::new("installs"))
        .and_where(Expr::col(telemetry_event::Column::CreatedAt).gte(day))
        .and_where(Expr::col(telemetry_event::Column::CreatedAt).lt(next))
        .add_group_by([coalesced_key(column)])
        .order_by_expr(event_count_expr(), SeaOrder::Desc)
        .limit(GROUP_ROW_CAP);

    let stmt = db.get_database_backend().build(&query);
    GroupedCount::find_by_statement(stmt).all(db).await
}

async fn dimension_tail<C: ConnectionTrait>(
    db: &C,
    day: NaiveDateTime,
    next: NaiveDateTime,
    column: telemetry_event::Column,
    kept_keys: &[String],
) -> Result<TotalCount, DbErr> {
    let mut query = SeaQuery::select();
    query
        .from(telemetry_event::Entity)
        .expr_as(event_count_expr(), Alias::new("cnt"))
        .expr_as(event_installs_expr(), Alias::new("installs"))
        .and_where(Expr::col(telemetry_event::Column::CreatedAt).gte(day))
        .and_where(Expr::col(telemetry_event::Column::CreatedAt).lt(next))
        .and_where(Expr::expr(coalesced_key(column)).is_not_in(kept_keys.to_vec()));

    let stmt = db.get_database_backend().build(&query);
    Ok(TotalCount::find_by_statement(stmt)
        .one(db)
        .await?
        .unwrap_or_default())
}

async fn rollup_dimensions(
    db: &DatabaseConnection,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<u64, DbErr> {
    let now = Utc::now().naive_utc();
    let mut models: Vec<telemetry_dimension_daily::ActiveModel> = Vec::new();

    for dimension in ROLLUP_DIMENSIONS {
        let Some(column) = dimension_column(dimension) else {
            continue;
        };

        let grouped = dimension_counts(db, day, next, dimension, column).await?;
        let capped = grouped.len() as u64 >= GROUP_ROW_CAP;
        let split = split_long_tail(grouped, ROLLUP_TOP_N);
        let mut rows = split.kept;

        if split.folded_keys > 0 || capped {
            let tail = dimension_tail(db, day, next, column, &split.kept_keys).await?;
            if tail.cnt > 0 {
                rows.push(GroupedCount {
                    key: OTHER_KEY.to_string(),
                    bucket: dimension.to_string(),
                    cnt: tail.cnt,
                    installs: tail.installs,
                });
            }
        }

        models.extend(
            rows.into_iter()
                .map(|row| telemetry_dimension_daily::ActiveModel {
                    id: Set(create_id()),
                    day: Set(day),
                    dimension: Set(row.bucket),
                    value: Set(row.key),
                    count: Set(to_i32(row.cnt)),
                    installs: Set(to_i32(row.installs)),
                    created_at: Set(now),
                    updated_at: Set(now),
                }),
        );
    }

    if models.is_empty() {
        return Ok(0);
    }

    upsert_chunked(db, models, dimension_daily_on_conflict()).await
}

fn dimension_daily_on_conflict() -> OnConflict {
    OnConflict::columns([
        telemetry_dimension_daily::Column::Day,
        telemetry_dimension_daily::Column::Dimension,
        telemetry_dimension_daily::Column::Value,
    ])
    .update_columns([
        telemetry_dimension_daily::Column::Count,
        telemetry_dimension_daily::Column::Installs,
        telemetry_dimension_daily::Column::UpdatedAt,
    ])
    .to_owned()
}

// ---------------------------------------------------------------------------
// TelemetrySessionDaily
// ---------------------------------------------------------------------------

#[derive(Debug, FromQueryResult)]
struct SessionDailyRow {
    release: String,
    source: String,
    sessions: i64,
    crashed_sessions: i64,
    errored_sessions: i64,
    installs: i64,
    crashed_installs: i64,
}

async fn session_counts<C: ConnectionTrait>(
    db: &C,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<Vec<SessionDailyRow>, DbErr> {
    let mut query = SeaQuery::select();
    query
        .from(telemetry_session::Entity)
        .expr_as(
            coalesced_key(telemetry_session::Column::Release),
            Alias::new("release"),
        )
        .expr_as(
            Expr::col(telemetry_session::Column::Source),
            Alias::new("source"),
        )
        .expr_as(
            Expr::col(telemetry_session::Column::Id).count(),
            Alias::new("sessions"),
        )
        .expr_as(
            session_status_count(telemetry_session::Column::Id, &[CRASHED_STATUS], false),
            Alias::new("crashed_sessions"),
        )
        .expr_as(
            session_status_count(telemetry_session::Column::Id, &ERRORED_STATUSES, false),
            Alias::new("errored_sessions"),
        )
        .expr_as(
            Expr::col(telemetry_session::Column::AnonId).count_distinct(),
            Alias::new("installs"),
        )
        .expr_as(
            session_status_count(telemetry_session::Column::AnonId, &[CRASHED_STATUS], true),
            Alias::new("crashed_installs"),
        )
        .and_where(Expr::col(telemetry_session::Column::CreatedAt).gte(day))
        .and_where(Expr::col(telemetry_session::Column::CreatedAt).lt(next))
        .add_group_by([
            coalesced_key(telemetry_session::Column::Release),
            Expr::col(telemetry_session::Column::Source).into(),
        ])
        .limit(GROUP_ROW_CAP);

    let stmt = db.get_database_backend().build(&query);
    SessionDailyRow::find_by_statement(stmt).all(db).await
}

async fn rollup_sessions(
    db: &DatabaseConnection,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<u64, DbErr> {
    let rows = session_counts(db, day, next).await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let now = Utc::now().naive_utc();
    let models: Vec<telemetry_session_daily::ActiveModel> = rows
        .into_iter()
        .map(|row| telemetry_session_daily::ActiveModel {
            id: Set(create_id()),
            day: Set(day),
            release: Set(row.release),
            source: Set(row.source),
            sessions: Set(to_i32(row.sessions)),
            crashed_sessions: Set(to_i32(row.crashed_sessions)),
            errored_sessions: Set(to_i32(row.errored_sessions)),
            installs: Set(to_i32(row.installs)),
            crashed_installs: Set(to_i32(row.crashed_installs)),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .collect();

    upsert_chunked(db, models, session_daily_on_conflict()).await
}

fn session_daily_on_conflict() -> OnConflict {
    OnConflict::columns([
        telemetry_session_daily::Column::Day,
        telemetry_session_daily::Column::Release,
        telemetry_session_daily::Column::Source,
    ])
    .update_columns([
        telemetry_session_daily::Column::Sessions,
        telemetry_session_daily::Column::CrashedSessions,
        telemetry_session_daily::Column::ErroredSessions,
        telemetry_session_daily::Column::Installs,
        telemetry_session_daily::Column::CrashedInstalls,
        telemetry_session_daily::Column::UpdatedAt,
    ])
    .to_owned()
}

// ---------------------------------------------------------------------------
// TelemetryLlmDaily
// ---------------------------------------------------------------------------

#[derive(Debug, FromQueryResult)]
struct LlmDailyRow {
    provider: String,
    model: String,
    source: String,
    calls: i64,
    errors: i64,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    duration_sum_ms: i64,
    duration_max_ms: i64,
}

async fn llm_counts<C: ConnectionTrait>(
    db: &C,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<Vec<LlmDailyRow>, DbErr> {
    let error_case = Expr::case(
        Expr::col(telemetry_llm_call::Column::Status).eq(LLM_ERROR_STATUS),
        Expr::col(telemetry_llm_call::Column::Id),
    )
    .finally(sea_orm::Value::String(None));

    let mut query = SeaQuery::select();
    query
        .from(telemetry_llm_call::Entity)
        .expr_as(
            Expr::col(telemetry_llm_call::Column::Provider),
            Alias::new("provider"),
        )
        .expr_as(
            Expr::col(telemetry_llm_call::Column::Model),
            Alias::new("model"),
        )
        .expr_as(
            Expr::col(telemetry_llm_call::Column::Source),
            Alias::new("source"),
        )
        .expr_as(
            Expr::col(telemetry_llm_call::Column::Id).count(),
            Alias::new("calls"),
        )
        .expr_as(Expr::expr(error_case).count(), Alias::new("errors"))
        .expr_as(
            sum_bigint(telemetry_llm_call::Column::PromptTokens),
            Alias::new("prompt_tokens"),
        )
        .expr_as(
            sum_bigint(telemetry_llm_call::Column::CompletionTokens),
            Alias::new("completion_tokens"),
        )
        .expr_as(
            sum_bigint(telemetry_llm_call::Column::TotalTokens),
            Alias::new("total_tokens"),
        )
        .expr_as(
            sum_bigint(telemetry_llm_call::Column::DurationMs),
            Alias::new("duration_sum_ms"),
        )
        .expr_as(
            Expr::expr(Func::coalesce([
                Func::max(Expr::col(telemetry_llm_call::Column::DurationMs)).into(),
                Expr::val(0i64).into(),
            ]))
            .cast_as(Alias::new("BIGINT")),
            Alias::new("duration_max_ms"),
        )
        .and_where(Expr::col(telemetry_llm_call::Column::CreatedAt).gte(day))
        .and_where(Expr::col(telemetry_llm_call::Column::CreatedAt).lt(next))
        .add_group_by([
            Expr::col(telemetry_llm_call::Column::Provider).into(),
            Expr::col(telemetry_llm_call::Column::Model).into(),
            Expr::col(telemetry_llm_call::Column::Source).into(),
        ])
        .limit(GROUP_ROW_CAP);

    let stmt = db.get_database_backend().build(&query);
    LlmDailyRow::find_by_statement(stmt).all(db).await
}

async fn rollup_llm(
    db: &DatabaseConnection,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<u64, DbErr> {
    let rows = llm_counts(db, day, next).await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let now = Utc::now().naive_utc();
    let models: Vec<telemetry_llm_daily::ActiveModel> = rows
        .into_iter()
        .map(|row| telemetry_llm_daily::ActiveModel {
            id: Set(create_id()),
            day: Set(day),
            provider: Set(row.provider),
            model: Set(row.model),
            source: Set(row.source),
            calls: Set(to_i32(row.calls)),
            errors: Set(to_i32(row.errors)),
            prompt_tokens: Set(row.prompt_tokens.max(0)),
            completion_tokens: Set(row.completion_tokens.max(0)),
            total_tokens: Set(row.total_tokens.max(0)),
            duration_sum_ms: Set(row.duration_sum_ms.max(0)),
            duration_max_ms: Set(to_i32(row.duration_max_ms)),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .collect();

    upsert_chunked(db, models, llm_daily_on_conflict()).await
}

fn llm_daily_on_conflict() -> OnConflict {
    OnConflict::columns([
        telemetry_llm_daily::Column::Day,
        telemetry_llm_daily::Column::Provider,
        telemetry_llm_daily::Column::Model,
        telemetry_llm_daily::Column::Source,
    ])
    .update_columns([
        telemetry_llm_daily::Column::Calls,
        telemetry_llm_daily::Column::Errors,
        telemetry_llm_daily::Column::PromptTokens,
        telemetry_llm_daily::Column::CompletionTokens,
        telemetry_llm_daily::Column::TotalTokens,
        telemetry_llm_daily::Column::DurationSumMs,
        telemetry_llm_daily::Column::DurationMaxMs,
        telemetry_llm_daily::Column::UpdatedAt,
    ])
    .to_owned()
}

// ---------------------------------------------------------------------------
// TelemetryPerfDaily
// ---------------------------------------------------------------------------

async fn perf_percentiles_sql<C: ConnectionTrait>(
    db: &C,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<Vec<PerfDailyRow>, DbErr> {
    let sql = r#"SELECT "metric" AS metric,
                        "source" AS source,
                        CAST(COUNT(*) AS BIGINT) AS cnt,
                        percentile_cont(0.5::float8) WITHIN GROUP (ORDER BY "value") AS p50,
                        percentile_cont(0.75::float8) WITHIN GROUP (ORDER BY "value") AS p75,
                        percentile_cont(0.95::float8) WITHIN GROUP (ORDER BY "value") AS p95
                 FROM "TelemetryPerfMetric"
                 WHERE "createdAt" >= $1 AND "createdAt" < $2
                 GROUP BY "metric", "source"
                 ORDER BY "metric" ASC, "source" ASC"#;

    PerfDailyRow::find_by_statement(Statement::from_sql_and_values(
        DbBackend::Postgres,
        sql,
        [day.into(), next.into()],
    ))
    .all(db)
    .await
}

async fn perf_percentiles_fold<C: ConnectionTrait>(
    db: &C,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<Vec<PerfDailyRow>, DbErr> {
    let rows = telemetry_perf_metric::Entity::find()
        .select_only()
        .column_as(telemetry_perf_metric::Column::Metric, "metric")
        .column_as(telemetry_perf_metric::Column::Source, "source")
        .column_as(telemetry_perf_metric::Column::Value, "value")
        .filter(telemetry_perf_metric::Column::CreatedAt.gte(day))
        .filter(telemetry_perf_metric::Column::CreatedAt.lt(next))
        .order_by_asc(telemetry_perf_metric::Column::CreatedAt)
        .limit(PERF_ROW_CAP)
        .into_model::<PerfSampleRow>()
        .all(db)
        .await?;

    if rows.len() as u64 >= PERF_ROW_CAP {
        tracing::warn!(
            cap = PERF_ROW_CAP,
            day = %day,
            "Telemetry perf rollup hit its sample cap; percentiles are approximate"
        );
    }

    Ok(fold_perf_samples(rows))
}

async fn rollup_perf(
    db: &DatabaseConnection,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<u64, DbErr> {
    // `percentile_cont` is a Postgres ordered-set aggregate; other backends and
    // Postgres-compatible engines that do not implement it fall back to folding
    // the raw samples with identical interpolation semantics.
    let rows = if db.get_database_backend() == DbBackend::Postgres {
        match perf_percentiles_sql(db, day, next).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "percentile_cont unavailable, folding telemetry perf samples in memory"
                );
                perf_percentiles_fold(db, day, next).await?
            }
        }
    } else {
        perf_percentiles_fold(db, day, next).await?
    };

    if rows.is_empty() {
        return Ok(0);
    }

    let now = Utc::now().naive_utc();
    let models: Vec<telemetry_perf_daily::ActiveModel> = rows
        .into_iter()
        .map(|row| telemetry_perf_daily::ActiveModel {
            id: Set(create_id()),
            day: Set(day),
            metric: Set(row.metric),
            source: Set(row.source),
            count: Set(to_i32(row.cnt)),
            p50: Set(round3(row.p50)),
            p75: Set(round3(row.p75)),
            p95: Set(round3(row.p95)),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .collect();

    upsert_chunked(db, models, perf_daily_on_conflict()).await
}

fn perf_daily_on_conflict() -> OnConflict {
    OnConflict::columns([
        telemetry_perf_daily::Column::Day,
        telemetry_perf_daily::Column::Metric,
        telemetry_perf_daily::Column::Source,
    ])
    .update_columns([
        telemetry_perf_daily::Column::Count,
        telemetry_perf_daily::Column::P50,
        telemetry_perf_daily::Column::P75,
        telemetry_perf_daily::Column::P95,
        telemetry_perf_daily::Column::UpdatedAt,
    ])
    .to_owned()
}

// ---------------------------------------------------------------------------
// TelemetryFlowpilotDaily
// ---------------------------------------------------------------------------

async fn rollup_flowpilot(
    db: &DatabaseConnection,
    day: NaiveDateTime,
    next: NaiveDateTime,
) -> Result<u64, DbErr> {
    let rows = telemetry_event::Entity::find()
        .select_only()
        .column_as(telemetry_event::Column::AnonId, "anon_id")
        .column_as(telemetry_event::Column::Props, "props")
        .filter(telemetry_event::Column::Name.eq(FLOWPILOT_METRICS_EVENT))
        .filter(telemetry_event::Column::CreatedAt.gte(day))
        .filter(telemetry_event::Column::CreatedAt.lt(next))
        .limit(FLOWPILOT_ROW_CAP)
        .into_model::<FlowPilotRow>()
        .all(db)
        .await?;

    if rows.is_empty() {
        return Ok(0);
    }
    if rows.len() as u64 >= FLOWPILOT_ROW_CAP {
        tracing::warn!(
            cap = FLOWPILOT_ROW_CAP,
            day = %day,
            "Telemetry FlowPilot rollup hit its row cap; counters are incomplete"
        );
    }

    let (counters, installs) = fold_flowpilot(&rows);
    let now = Utc::now().naive_utc();
    let model = telemetry_flowpilot_daily::ActiveModel {
        id: Set(create_id()),
        day: Set(day),
        runs_started: Set(counters.runs_started),
        runs_succeeded: Set(counters.runs_succeeded),
        runs_failed: Set(counters.runs_failed),
        runs_cancelled: Set(counters.runs_cancelled),
        plans_assessed: Set(counters.plans_assessed),
        plans_feasible: Set(counters.plans_feasible),
        plans_infeasible: Set(counters.plans_infeasible),
        attempts_total: Set(counters.attempts_total),
        attempts_parse_valid: Set(counters.attempts_parse_valid),
        attempts_typed_valid: Set(counters.attempts_typed_valid),
        attempts_reconcile_valid: Set(counters.attempts_reconcile_valid),
        attempts_applied: Set(counters.attempts_applied),
        queued_reviews: Set(counters.queued_reviews),
        apply_dispositions: Set(counters.apply_dispositions),
        dismissed_dispositions: Set(counters.dismissed_dispositions),
        stale_dispositions: Set(counters.stale_dispositions),
        error_dispositions: Set(counters.error_dispositions),
        diagnostic_occurrences: Set(counters.diagnostic_occurrences),
        repeated_diagnostic_occurrences: Set(counters.repeated_diagnostic_occurrences),
        validation_regressions: Set(counters.validation_regressions),
        boards_inspected: Set(counters.boards_inspected),
        empty_boards_after_run: Set(counters.empty_boards_after_run),
        installs: Set(to_i32(installs)),
        created_at: Set(now),
        updated_at: Set(now),
    };

    upsert_chunked(db, vec![model], flowpilot_daily_on_conflict()).await
}

fn flowpilot_daily_on_conflict() -> OnConflict {
    OnConflict::column(telemetry_flowpilot_daily::Column::Day)
        .update_columns([
            telemetry_flowpilot_daily::Column::RunsStarted,
            telemetry_flowpilot_daily::Column::RunsSucceeded,
            telemetry_flowpilot_daily::Column::RunsFailed,
            telemetry_flowpilot_daily::Column::RunsCancelled,
            telemetry_flowpilot_daily::Column::PlansAssessed,
            telemetry_flowpilot_daily::Column::PlansFeasible,
            telemetry_flowpilot_daily::Column::PlansInfeasible,
            telemetry_flowpilot_daily::Column::AttemptsTotal,
            telemetry_flowpilot_daily::Column::AttemptsParseValid,
            telemetry_flowpilot_daily::Column::AttemptsTypedValid,
            telemetry_flowpilot_daily::Column::AttemptsReconcileValid,
            telemetry_flowpilot_daily::Column::AttemptsApplied,
            telemetry_flowpilot_daily::Column::QueuedReviews,
            telemetry_flowpilot_daily::Column::ApplyDispositions,
            telemetry_flowpilot_daily::Column::DismissedDispositions,
            telemetry_flowpilot_daily::Column::StaleDispositions,
            telemetry_flowpilot_daily::Column::ErrorDispositions,
            telemetry_flowpilot_daily::Column::DiagnosticOccurrences,
            telemetry_flowpilot_daily::Column::RepeatedDiagnosticOccurrences,
            telemetry_flowpilot_daily::Column::ValidationRegressions,
            telemetry_flowpilot_daily::Column::BoardsInspected,
            telemetry_flowpilot_daily::Column::EmptyBoardsAfterRun,
            telemetry_flowpilot_daily::Column::Installs,
            telemetry_flowpilot_daily::Column::UpdatedAt,
        ])
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use sea_orm::QueryTrait;
    use sea_orm::sea_query::PostgresQueryBuilder;
    use serde_json::json;

    fn ts(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 7, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    fn grouped(key: &str, bucket: &str, cnt: i64, installs: i64) -> GroupedCount {
        GroupedCount {
            key: key.to_string(),
            bucket: bucket.to_string(),
            cnt,
            installs,
        }
    }

    fn flowpilot_row(anon: &str, props: Option<serde_json::Value>) -> FlowPilotRow {
        FlowPilotRow {
            anon_id: anon.to_string(),
            props,
        }
    }

    fn perf_sample(metric: &str, source: &str, value: f64) -> PerfSampleRow {
        PerfSampleRow {
            metric: metric.to_string(),
            source: source.to_string(),
            value,
        }
    }

    #[test]
    fn day_start_truncates_to_utc_midnight() {
        assert_eq!(day_start(ts(20, 13, 45)), ts(20, 0, 0));
        assert_eq!(day_start(ts(20, 0, 0)), ts(20, 0, 0));
    }

    #[test]
    fn rollup_window_is_oldest_first_and_always_contains_today() {
        assert_eq!(
            rollup_days(ts(20, 5, 0), 3),
            vec![ts(18, 0, 0), ts(19, 0, 0), ts(20, 0, 0)]
        );
        assert_eq!(rollup_days(ts(20, 5, 0), 1), vec![ts(20, 0, 0)]);
    }

    #[test]
    fn rollup_window_clamps_out_of_range_backfills() {
        assert_eq!(rollup_days(ts(20, 5, 0), 0), vec![ts(20, 0, 0)]);
        assert_eq!(rollup_days(ts(20, 5, 0), -7), vec![ts(20, 0, 0)]);
        assert_eq!(rollup_days(ts(20, 5, 0), 5_000).len(), 90);
    }

    #[test]
    fn backfill_days_default_and_clamp() {
        assert_eq!(parse_backfill_days(None), DEFAULT_BACKFILL_DAYS);
        assert_eq!(parse_backfill_days(Some("")), DEFAULT_BACKFILL_DAYS);
        assert_eq!(parse_backfill_days(Some("later")), DEFAULT_BACKFILL_DAYS);
        assert_eq!(parse_backfill_days(Some("2.5")), DEFAULT_BACKFILL_DAYS);
        assert_eq!(parse_backfill_days(Some("0")), MIN_BACKFILL_DAYS);
        assert_eq!(parse_backfill_days(Some("-3")), MIN_BACKFILL_DAYS);
        assert_eq!(parse_backfill_days(Some(" 14 ")), 14);
        assert_eq!(parse_backfill_days(Some("365")), MAX_BACKFILL_DAYS);
    }

    #[test]
    fn rollup_interval_defaults_and_never_drops_below_a_minute() {
        assert_eq!(parse_interval(None), Duration::from_secs(3600));
        assert_eq!(parse_interval(Some("nope")), Duration::from_secs(3600));
        assert_eq!(parse_interval(Some("0")), Duration::from_secs(60));
        assert_eq!(parse_interval(Some("30")), Duration::from_secs(60));
        assert_eq!(parse_interval(Some(" 900 ")), Duration::from_secs(900));
    }

    #[test]
    fn default_config_matches_the_documented_defaults() {
        let config = TelemetryRollupConfig::default();
        assert_eq!(config.interval, Duration::from_secs(3600));
        assert_eq!(config.backfill_days, 3);
        assert!(TelemetryRollupResult::default().is_empty());
    }

    #[test]
    fn counters_narrow_without_wrapping() {
        assert_eq!(to_i32(0), 0);
        assert_eq!(to_i32(-5), 0);
        assert_eq!(to_i32(42), 42);
        assert_eq!(to_i32(i64::MAX), i32::MAX);
    }

    #[test]
    fn long_tail_keeps_everything_below_the_cap() {
        let rows = vec![
            grouped("a", "web", 5, 2),
            grouped("b", "web", 3, 1),
            grouped("a", "desktop", 1, 1),
        ];
        let split = split_long_tail(rows.clone(), ROLLUP_TOP_N);
        assert_eq!(split.folded_keys, 0);
        assert_eq!(split.kept, rows);
        assert_eq!(split.kept_keys, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn long_tail_folds_everything_past_the_two_hundredth_key() {
        // 250 names: the first 200 are hot, the remaining 50 are the tail.
        let mut rows: Vec<GroupedCount> = (0..250)
            .map(|i| {
                let count = if i < 200 { 1_000 - i } else { 1 };
                grouped(&format!("event_{i:03}"), "web", count, 1)
            })
            .collect();
        rows.push(grouped("event_000", "desktop", 7, 3));

        let split = split_long_tail(rows, ROLLUP_TOP_N);
        assert_eq!(split.folded_keys, 50);
        assert_eq!(split.kept_keys.len(), ROLLUP_TOP_N);
        assert!(split.kept_keys.contains(&"event_000".to_string()));
        assert!(!split.kept_keys.contains(&"event_200".to_string()));
        // Both rows of the hottest name survive, one per bucket.
        assert_eq!(
            split
                .kept
                .iter()
                .filter(|row| row.key == "event_000")
                .count(),
            2
        );
        assert!(split.kept.iter().all(|row| row.key.as_str() < "event_200"));
    }

    #[test]
    fn long_tail_ranks_on_the_total_across_buckets_and_breaks_ties_on_the_key() {
        let rows = vec![
            grouped("split", "web", 4, 1),
            grouped("split", "desktop", 4, 1),
            grouped("single", "web", 7, 1),
            grouped("zzz", "web", 7, 1),
        ];
        let split = split_long_tail(rows, 2);
        assert_eq!(split.folded_keys, 1);
        // "split" totals 8 and wins; "single" and "zzz" tie at 7, key order decides.
        assert_eq!(
            split.kept_keys,
            vec!["single".to_string(), "split".to_string()]
        );
    }

    #[test]
    fn long_tail_split_is_deterministic_so_a_second_pass_upserts_the_same_rows() {
        let rows: Vec<GroupedCount> = (0..300)
            .map(|i| grouped(&format!("name_{i:03}"), "web", (i % 7) as i64 + 1, 1))
            .collect();
        let first = split_long_tail(rows.clone(), ROLLUP_TOP_N);
        let second = split_long_tail(rows, ROLLUP_TOP_N);
        assert_eq!(first, second);
    }

    #[test]
    fn percentiles_interpolate_like_percentile_cont() {
        let values = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile_cont(&values, 0.5), 2.5);
        assert_eq!(percentile_cont(&values, 0.75), 3.25);
        assert_eq!(round3(percentile_cont(&values, 0.95)), 3.85);
        assert_eq!(percentile_cont(&values, 0.0), 1.0);
        assert_eq!(percentile_cont(&values, 1.0), 4.0);
    }

    #[test]
    fn percentiles_handle_degenerate_inputs() {
        assert_eq!(percentile_cont(&[], 0.5), 0.0);
        assert_eq!(percentile_cont(&[42.0], 0.95), 42.0);
        assert_eq!(percentile_cont(&[1.0, 2.0], -1.0), 1.0);
        assert_eq!(percentile_cont(&[1.0, 2.0], 5.0), 2.0);
    }

    #[test]
    fn perf_fold_groups_by_metric_and_source_and_sorts_samples() {
        let rows = vec![
            perf_sample("lcp", "web", 400.0),
            perf_sample("lcp", "web", 100.0),
            perf_sample("lcp", "web", 300.0),
            perf_sample("lcp", "web", 200.0),
            perf_sample("lcp", "desktop", 50.0),
            perf_sample("cls", "web", 0.1),
        ];
        let folded = fold_perf_samples(rows);
        assert_eq!(folded.len(), 3);
        assert_eq!(folded[0].metric, "cls");
        assert_eq!(folded[1].metric, "lcp");
        assert_eq!(folded[1].source, "desktop");
        let web = &folded[2];
        assert_eq!(web.source, "web");
        assert_eq!(web.cnt, 4);
        assert_eq!(web.p50, 250.0);
        assert_eq!(web.p75, 325.0);
        assert_eq!(round3(web.p95), 385.0);
    }

    #[test]
    fn perf_fold_is_idempotent_over_the_same_samples() {
        let samples = || {
            vec![
                perf_sample("inp", "web", 12.5),
                perf_sample("inp", "web", 7.25),
                perf_sample("inp", "web", 30.0),
            ]
        };
        let first = fold_perf_samples(samples());
        let second = fold_perf_samples(samples());
        assert_eq!(first.len(), second.len());
        assert_eq!(first[0].cnt, second[0].cnt);
        assert_eq!(first[0].p50, second[0].p50);
        assert_eq!(first[0].p75, second[0].p75);
        assert_eq!(first[0].p95, second[0].p95);
    }

    #[test]
    fn flowpilot_counters_sum_and_ignore_junk_props() {
        let rows = vec![
            flowpilot_row(
                "a",
                Some(json!({ "runs_started": 2, "runs_succeeded": 1, "boards_inspected": 3 })),
            ),
            flowpilot_row(
                "b",
                Some(json!({ "runs_started": "junk", "runs_failed": 2.5, "runs_cancelled": 1 })),
            ),
            flowpilot_row("a", None),
            flowpilot_row(BACKEND_ANON_ID, Some(json!({ "runs_started": 5 }))),
        ];
        let (counters, installs) = fold_flowpilot(&rows);
        assert_eq!(counters.runs_started, 7);
        assert_eq!(counters.runs_succeeded, 1);
        assert_eq!(counters.runs_failed, 0);
        assert_eq!(counters.runs_cancelled, 1);
        assert_eq!(counters.boards_inspected, 3);
        assert_eq!(counters.attempts_total, 0);
        assert_eq!(installs, 2);
    }

    #[test]
    fn flowpilot_counters_clamp_negatives_and_saturate() {
        let rows = vec![
            flowpilot_row(
                "a",
                Some(json!({ "runs_started": i64::MAX, "runs_failed": -5 })),
            ),
            flowpilot_row("b", Some(json!({ "runs_started": i64::MAX }))),
        ];
        let (counters, _) = fold_flowpilot(&rows);
        assert_eq!(counters.runs_started, i64::MAX);
        assert_eq!(counters.runs_failed, 0);
    }

    #[test]
    fn flowpilot_fold_is_idempotent() {
        let rows = vec![
            flowpilot_row(
                "a",
                Some(json!({ "runs_started": 3, "attempts_applied": 2 })),
            ),
            flowpilot_row("b", Some(json!({ "runs_started": 1 }))),
        ];
        assert_eq!(fold_flowpilot(&rows), fold_flowpilot(&rows));
    }

    /// Every rollup UPSERTs on exactly the unique key of its table and never
    /// touches `id` or `createdAt`, which is what makes a second pass over the
    /// same day overwrite instead of double-count.
    #[test]
    fn every_rollup_upserts_on_its_unique_key() {
        let now = ts(20, 12, 0);
        let day = ts(20, 0, 0);

        let event_sql = telemetry_event_daily::Entity::insert(telemetry_event_daily::ActiveModel {
            id: Set("e1".to_string()),
            day: Set(day),
            name: Set("app_opened".to_string()),
            source: Set("web".to_string()),
            count: Set(3),
            installs: Set(2),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .on_conflict(event_daily_on_conflict())
        .build(DbBackend::Postgres)
        .to_string();
        assert!(
            event_sql.contains(r#"ON CONFLICT ("day", "name", "source") DO UPDATE SET"#),
            "{}",
            event_sql
        );
        assert!(
            event_sql.contains(r#""count" = "excluded"."count""#),
            "{}",
            event_sql
        );
        assert!(
            event_sql.contains(r#""installs" = "excluded"."installs""#),
            "{}",
            event_sql
        );

        let dimension_sql =
            telemetry_dimension_daily::Entity::insert(telemetry_dimension_daily::ActiveModel {
                id: Set("d1".to_string()),
                day: Set(day),
                dimension: Set("platform".to_string()),
                value: Set("macos".to_string()),
                count: Set(3),
                installs: Set(2),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .on_conflict(dimension_daily_on_conflict())
            .build(DbBackend::Postgres)
            .to_string();
        assert!(
            dimension_sql.contains(r#"ON CONFLICT ("day", "dimension", "value") DO UPDATE SET"#),
            "{}",
            dimension_sql
        );

        let install_sql =
            telemetry_install_daily::Entity::insert(telemetry_install_daily::ActiveModel {
                id: Set("i1".to_string()),
                day: Set(day),
                anon_id: Set("anon".to_string()),
                source: Set("web".to_string()),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .on_conflict(install_on_conflict())
            .build(DbBackend::Postgres)
            .to_string();
        assert!(
            install_sql.contains(r#"ON CONFLICT ("day", "anonId", "source") DO NOTHING"#),
            "{}",
            install_sql
        );

        let session_sql =
            telemetry_session_daily::Entity::insert(telemetry_session_daily::ActiveModel {
                id: Set("s1".to_string()),
                day: Set(day),
                release: Set(UNKNOWN_VALUE.to_string()),
                source: Set("desktop".to_string()),
                sessions: Set(10),
                crashed_sessions: Set(1),
                errored_sessions: Set(2),
                installs: Set(5),
                crashed_installs: Set(1),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .on_conflict(session_daily_on_conflict())
            .build(DbBackend::Postgres)
            .to_string();
        assert!(
            session_sql.contains(r#"ON CONFLICT ("day", "release", "source") DO UPDATE SET"#),
            "{}",
            session_sql
        );

        let llm_sql = telemetry_llm_daily::Entity::insert(telemetry_llm_daily::ActiveModel {
            id: Set("l1".to_string()),
            day: Set(day),
            provider: Set("openai".to_string()),
            model: Set("gpt".to_string()),
            source: Set("backend".to_string()),
            calls: Set(4),
            errors: Set(1),
            prompt_tokens: Set(10),
            completion_tokens: Set(20),
            total_tokens: Set(30),
            duration_sum_ms: Set(400),
            duration_max_ms: Set(200),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .on_conflict(llm_daily_on_conflict())
        .build(DbBackend::Postgres)
        .to_string();
        assert!(
            llm_sql.contains(r#"ON CONFLICT ("day", "provider", "model", "source") DO UPDATE SET"#),
            "{}",
            llm_sql
        );

        let perf_sql = telemetry_perf_daily::Entity::insert(telemetry_perf_daily::ActiveModel {
            id: Set("p1".to_string()),
            day: Set(day),
            metric: Set("lcp".to_string()),
            source: Set("web".to_string()),
            count: Set(9),
            p50: Set(1.0),
            p75: Set(2.0),
            p95: Set(3.0),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .on_conflict(perf_daily_on_conflict())
        .build(DbBackend::Postgres)
        .to_string();
        assert!(
            perf_sql.contains(r#"ON CONFLICT ("day", "metric", "source") DO UPDATE SET"#),
            "{}",
            perf_sql
        );

        for sql in [
            &event_sql,
            &dimension_sql,
            &session_sql,
            &llm_sql,
            &perf_sql,
        ] {
            assert!(!sql.contains(r#""id" = "excluded"."id""#), "{}", sql);
            assert!(
                !sql.contains(r#""createdAt" = "excluded"."createdAt""#),
                "{}",
                sql
            );
        }
    }

    #[test]
    fn flowpilot_upsert_keys_on_the_day_alone() {
        let now = ts(20, 12, 0);
        let sql =
            telemetry_flowpilot_daily::Entity::insert(telemetry_flowpilot_daily::ActiveModel {
                id: Set("f1".to_string()),
                day: Set(ts(20, 0, 0)),
                runs_started: Set(1),
                runs_succeeded: Set(1),
                runs_failed: Set(0),
                runs_cancelled: Set(0),
                plans_assessed: Set(0),
                plans_feasible: Set(0),
                plans_infeasible: Set(0),
                attempts_total: Set(0),
                attempts_parse_valid: Set(0),
                attempts_typed_valid: Set(0),
                attempts_reconcile_valid: Set(0),
                attempts_applied: Set(0),
                queued_reviews: Set(0),
                apply_dispositions: Set(0),
                dismissed_dispositions: Set(0),
                stale_dispositions: Set(0),
                error_dispositions: Set(0),
                diagnostic_occurrences: Set(0),
                repeated_diagnostic_occurrences: Set(0),
                validation_regressions: Set(0),
                boards_inspected: Set(0),
                empty_boards_after_run: Set(0),
                installs: Set(1),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .on_conflict(flowpilot_daily_on_conflict())
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains(r#"ON CONFLICT ("day") DO UPDATE SET"#),
            "{}",
            sql
        );
        assert!(
            sql.contains(r#""runsStarted" = "excluded"."runsStarted""#),
            "{}",
            sql
        );
        assert!(
            sql.contains(r#""emptyBoardsAfterRun" = "excluded"."emptyBoardsAfterRun""#),
            "{}",
            sql
        );
        assert!(!sql.contains(r#""id" = "excluded"."id""#), "{}", sql);
    }

    /// The kept keys drive the `NOT IN` of the tail query, so the tail and the
    /// kept rows partition the day exactly once — that is what keeps
    /// `__other__` from double-counting a name that was also stored on its own.
    #[test]
    fn tail_filter_excludes_exactly_the_kept_keys() {
        let rows: Vec<GroupedCount> = (0..205)
            .map(|i| grouped(&format!("k{i:03}"), "web", 205 - i, 1))
            .collect();
        let split = split_long_tail(rows, ROLLUP_TOP_N);

        let sql = SeaQuery::select()
            .from(telemetry_event::Entity)
            .expr_as(event_count_expr(), Alias::new("cnt"))
            .and_where(Expr::col(telemetry_event::Column::Name).is_not_in(split.kept_keys.clone()))
            .to_string(PostgresQueryBuilder);

        assert!(sql.contains(r#""name" NOT IN ("#), "{}", sql);
        assert!(sql.contains("'k000'"), "{}", sql);
        assert!(!sql.contains("'k204'"), "{}", sql);
    }

    #[test]
    fn dimension_vocabulary_maps_to_event_columns() {
        for dimension in ROLLUP_DIMENSIONS {
            assert!(
                dimension_column(dimension).is_some(),
                "missing column for {dimension}"
            );
        }
        assert!(dimension_column("release").is_none());
    }

    #[test]
    fn session_statuses_partition_into_crashed_and_errored() {
        let sql = SeaQuery::select()
            .from(telemetry_session::Entity)
            .expr_as(
                session_status_count(telemetry_session::Column::Id, &[CRASHED_STATUS], false),
                Alias::new("crashed_sessions"),
            )
            .expr_as(
                session_status_count(telemetry_session::Column::Id, &ERRORED_STATUSES, false),
                Alias::new("errored_sessions"),
            )
            .expr_as(
                session_status_count(telemetry_session::Column::AnonId, &[CRASHED_STATUS], true),
                Alias::new("crashed_installs"),
            )
            .to_string(PostgresQueryBuilder);

        assert!(sql.contains("'crashed'"), "{}", sql);
        assert!(sql.contains("'errored', 'abnormal'"), "{}", sql);
        assert!(sql.contains("COUNT(DISTINCT"), "{}", sql);
        // "ok" is the remainder: sessions - crashed - errored, never queried.
        assert!(!sql.contains("'ok'"), "{}", sql);
    }

    #[test]
    fn sums_are_cast_so_cockroach_decimals_still_deserialize() {
        let sql = SeaQuery::select()
            .from(telemetry_llm_call::Entity)
            .expr_as(
                sum_bigint(telemetry_llm_call::Column::TotalTokens),
                Alias::new("total_tokens"),
            )
            .to_string(PostgresQueryBuilder);

        assert!(sql.contains(r#"SUM("totalTokens")"#), "{}", sql);
        assert!(sql.contains("AS BIGINT"), "{}", sql);
    }

    #[test]
    fn install_counts_never_include_the_synthetic_backend_id() {
        let sql = SeaQuery::select()
            .from(telemetry_event::Entity)
            .expr_as(event_installs_expr(), Alias::new("installs"))
            .to_string(PostgresQueryBuilder);

        assert!(sql.contains("COUNT(DISTINCT"), "{}", sql);
        assert!(sql.contains("'backend'"), "{}", sql);
        assert!(sql.contains("THEN NULL"), "{}", sql);
    }

    #[test]
    fn dimension_keys_coalesce_nulls_identically_in_projection_and_filter() {
        let sql = SeaQuery::select()
            .from(telemetry_event::Entity)
            .expr_as(
                coalesced_key(telemetry_event::Column::Platform),
                Alias::new("key"),
            )
            .and_where(
                Expr::expr(coalesced_key(telemetry_event::Column::Platform))
                    .is_not_in(["macos".to_string()]),
            )
            .to_string(PostgresQueryBuilder);

        assert_eq!(sql.matches(r#"COALESCE("platform", 'unknown')"#).count(), 2);
    }
}
