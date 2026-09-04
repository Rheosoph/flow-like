//! Threshold and anomaly alerting over the anonymous telemetry tables.
//!
//! Rules are evaluated on a fixed interval by an in-process ticker, mirroring
//! `telemetry::sweeper`. Deployments without a long-lived process (AWS Lambda)
//! drive the exact same `evaluate_once` through
//! `POST /api/v1/maintenance/run` (or the Admin-only manual endpoint).
//!
//! A firing rule always appends a row to the in-app inbox; that row is the
//! source of truth. Rules that opt into a notification channel additionally
//! hand the committed transition to `telemetry::notify`, which delivers it
//! best-effort and can never fail the pass. Every value the engine reads is an
//! aggregate over already-anonymous rows, so an alert can never carry identity.

use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, Utc};
use flow_like_types::tokio::{self, task::JoinHandle};
use sea_orm::sea_query::{Expr, IntoColumnRef, NullOrdering, SimpleExpr};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait,
    FromQueryResult, IntoActiveModel, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    Set, Statement,
};

use crate::db::DbDialect;
use crate::entity::{
    telemetry_alert_event, telemetry_alert_rule, telemetry_error_event, telemetry_event,
    telemetry_llm_call, telemetry_session, telemetry_span,
};
use crate::state::AppState;
use crate::telemetry::notify::notify_alert_transition;
use crate::telemetry::percentiles_in_sql;

/// Metrics a rule may watch.
pub const ALERT_METRICS: [&str; 6] = [
    "error_rate",
    "latency_p95",
    "crash_free_rate",
    "event_count",
    "span_error_rate",
    "llm_error_rate",
];
/// Directions a threshold rule may breach in.
pub const ALERT_COMPARATORS: [&str; 2] = ["gt", "lt"];
/// Evaluation strategies a rule may use.
pub const ALERT_MODES: [&str; 2] = ["threshold", "anomaly"];

pub const ALERT_MODE_THRESHOLD: &str = "threshold";
pub const ALERT_MODE_ANOMALY: &str = "anomaly";
pub const ALERT_STATUS_TRIGGERED: &str = "triggered";
pub const ALERT_STATUS_RESOLVED: &str = "resolved";

/// Standard deviations an anomaly rule tolerates when it does not set its own.
pub const DEFAULT_SENSITIVITY: f64 = 3.0;
/// Baseline windows an anomaly rule needs before it may fire.
pub const DEFAULT_MIN_SAMPLES: i32 = 5;
pub const MIN_MIN_SAMPLES: i32 = 2;
pub const MAX_MIN_SAMPLES: i32 = 100;
pub const MIN_WINDOW_MINUTES: i32 = 1;
pub const MAX_WINDOW_MINUTES: i32 = 1440;
pub const DEFAULT_WINDOW_MINUTES: i32 = 60;

const DEFAULT_INTERVAL_SECS: u64 = 300;
const MIN_INTERVAL_SECS: u64 = 30;
const DEFAULT_RULE_CAP: u64 = 200;
const CRASHED_STATUS: &str = "crashed";
const ERROR_STATUS: &str = "error";
static ALERT_EVALUATION_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Upper bound on the spans folded in Rust when the backend has no
/// `percentile_cont`.
const SPAN_ROW_CAP: u64 = 100_000;

pub fn is_valid_metric(metric: &str) -> bool {
    ALERT_METRICS.contains(&metric)
}

pub fn is_valid_comparator(comparator: &str) -> bool {
    ALERT_COMPARATORS.contains(&comparator)
}

pub fn is_valid_mode(mode: &str) -> bool {
    ALERT_MODES.contains(&mode)
}

/// Mean and population standard deviation over the baseline windows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BaselineStats {
    pub samples: usize,
    pub mean: f64,
    pub stddev: f64,
}

/// What a single evaluation of a rule did to the inbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertTransition {
    None,
    Trigger,
    Resolve,
}

/// Number of rules touched by a single evaluation pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AlertEvaluationResult {
    pub evaluated: u64,
    pub triggered: u64,
    pub resolved: u64,
}

impl AlertEvaluationResult {
    pub fn is_empty(&self) -> bool {
        self.triggered == 0 && self.resolved == 0
    }
}

/// Configuration for the telemetry alert evaluator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TelemetryAlertConfig {
    pub interval: Duration,
    /// Upper bound on the enabled rules a single pass evaluates.
    pub rule_cap: u64,
}

impl Default for TelemetryAlertConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
            rule_cap: DEFAULT_RULE_CAP,
        }
    }
}

impl TelemetryAlertConfig {
    /// Build config from environment variables.
    /// - `FLOW_LIKE_TELEMETRY_ALERT_INTERVAL_SECS`: how often rules are evaluated (default 300, minimum 30)
    pub fn from_env() -> Self {
        Self {
            interval: parse_interval(
                std::env::var("FLOW_LIKE_TELEMETRY_ALERT_INTERVAL_SECS")
                    .ok()
                    .as_deref(),
            ),
            rule_cap: DEFAULT_RULE_CAP,
        }
    }
}

/// Spawn the telemetry alert evaluator as a background task.
///
/// Returns `None` if `FLOW_LIKE_TELEMETRY_ALERTS_DISABLED=1` is set, otherwise
/// the join handle of the spawned task. The task runs forever and is expected
/// to be aborted on process shutdown.
pub fn spawn_telemetry_alert_evaluator(
    state: AppState,
    config: TelemetryAlertConfig,
) -> Option<JoinHandle<()>> {
    if alerts_disabled() {
        tracing::info!("Telemetry alerts disabled via FLOW_LIKE_TELEMETRY_ALERTS_DISABLED");
        return None;
    }

    tracing::info!(
        interval_secs = config.interval.as_secs(),
        rule_cap = config.rule_cap,
        "Spawning telemetry alert evaluator"
    );

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // First tick fires immediately; let services come up before we hit the DB.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            match evaluate_once(&state, &config).await {
                Ok(result) if result.is_empty() => {}
                Ok(result) => tracing::info!(
                    evaluated = result.evaluated,
                    triggered = result.triggered,
                    resolved = result.resolved,
                    "Telemetry alert evaluation changed rule states"
                ),
                Err(e) => tracing::error!(error = %e, "Telemetry alert evaluation failed"),
            }
        }
    });

    Some(handle)
}

/// Evaluate every enabled rule once. Returns how many rules were evaluated and
/// how many opened or closed an alert.
///
/// Exposed for tests, for the spawned task, and for the service-authenticated
/// maintenance endpoint used by serverless deployments. A process mutex avoids
/// redundant local passes, while each rule is evaluated inside a retried
/// transaction that locks its row. On blocking engines the lock serializes the
/// rule across API replicas; on optimistic ones both replicas evaluate and the
/// loser re-runs, sees `lastEvaluatedAt` advanced by the winner and skips, so a
/// transition is never produced twice. A rule that fails to evaluate is logged
/// and skipped so one bad rule cannot stop the pass, and a transition is only
/// notified once its transaction commits.
pub async fn evaluate_once(
    state: &AppState,
    config: &TelemetryAlertConfig,
) -> Result<AlertEvaluationResult, DbErr> {
    // Avoid doing the same expensive metric aggregation twice in this process.
    let _local_guard = ALERT_EVALUATION_MUTEX.lock().await;

    let rules = telemetry_alert_rule::Entity::find()
        .filter(telemetry_alert_rule::Column::Enabled.eq(true))
        // The cap is a per-pass work budget, not a permanent subset. Rules
        // never evaluated come first, then the least recently evaluated.
        .order_by_with_nulls(
            telemetry_alert_rule::Column::LastEvaluatedAt,
            Order::Asc,
            NullOrdering::First,
        )
        .order_by_asc(telemetry_alert_rule::Column::CreatedAt)
        .order_by_asc(telemetry_alert_rule::Column::Id)
        .limit(config.rule_cap)
        .all(&state.db)
        .await?;

    let dialect = state.db_dialect;
    let mut result = AlertEvaluationResult::default();
    for listed_rule in rules {
        let outcome = state
            .transaction(|txn| {
                let listed_rule = listed_rule.clone();
                Box::pin(async move { evaluate_listed_rule(txn, dialect, listed_rule).await })
            })
            .await;

        let (rule, evaluation) = match outcome {
            Ok(RuleOutcome::Evaluated { rule, evaluation }) => (rule, evaluation),
            Ok(RuleOutcome::Skipped) => continue,
            Err(error) => {
                result.evaluated += 1;
                tracing::error!(
                    rule_id = %listed_rule.id,
                    metric = %listed_rule.metric,
                    error = %error,
                    "Telemetry alert rule evaluation failed"
                );
                continue;
            }
        };

        result.evaluated += 1;
        match evaluation.transition {
            AlertTransition::Trigger => result.triggered += 1,
            AlertTransition::Resolve => result.resolved += 1,
            AlertTransition::None => {}
        }
        if let Some(event) = evaluation.event {
            notify_alert_transition(state, &rule, &event).await;
        }
    }

    Ok(result)
}

/// What one transaction attempt did with a listed rule.
enum RuleOutcome {
    /// The rule vanished, was disabled, or another replica evaluated it since
    /// the pass listed it.
    Skipped,
    Evaluated {
        rule: telemetry_alert_rule::Model,
        evaluation: RuleEvaluation,
    },
}

async fn evaluate_listed_rule(
    txn: &DatabaseTransaction,
    dialect: DbDialect,
    listed_rule: telemetry_alert_rule::Model,
) -> Result<RuleOutcome, DbErr> {
    let Some(rule) = telemetry_alert_rule::Entity::find_by_id(&listed_rule.id)
        .lock_exclusive()
        .one(txn)
        .await?
    else {
        return Ok(RuleOutcome::Skipped);
    };

    // A rule can be disabled after the initial bounded list but before its
    // row lock is acquired, and a replica that lost the commit race re-reads
    // the winner's evaluation stamp here.
    if !rule.enabled || rule.last_evaluated_at > listed_rule.last_evaluated_at {
        return Ok(RuleOutcome::Skipped);
    }

    let evaluation = evaluate_rule(txn, dialect, &rule).await?;
    Ok(RuleOutcome::Evaluated { rule, evaluation })
}

/// What a single rule evaluation changed: the transition and, when there was
/// one, the committed inbox row the notification channels carry out of band.
#[derive(Clone, Debug)]
struct RuleEvaluation {
    transition: AlertTransition,
    event: Option<telemetry_alert_event::Model>,
}

async fn evaluate_rule<C: ConnectionTrait>(
    db: &C,
    dialect: DbDialect,
    rule: &telemetry_alert_rule::Model,
) -> Result<RuleEvaluation, DbErr> {
    let now = Utc::now().fixed_offset();
    let window_minutes = rule
        .window_minutes
        .clamp(MIN_WINDOW_MINUTES, MAX_WINDOW_MINUTES);
    let window = ChronoDuration::minutes(window_minutes as i64);
    let source = rule.source.as_deref().filter(|value| !value.is_empty());
    let value = metric_value(db, dialect, &rule.metric, source, now - window, now).await?;

    let mut baseline = BaselineStats::default();
    let sensitivity = rule.sensitivity.unwrap_or(DEFAULT_SENSITIVITY);
    let min_samples = rule.min_samples.clamp(MIN_MIN_SAMPLES, MAX_MIN_SAMPLES) as usize;

    let fires = match value {
        None => false,
        Some(value) if rule.mode == ALERT_MODE_ANOMALY => {
            let samples =
                baseline_samples(db, dialect, &rule.metric, source, now, window, min_samples)
                    .await?;
            baseline = baseline_stats(&samples);
            anomaly_fires(value, &samples, sensitivity, min_samples)
        }
        Some(value) => rule
            .threshold
            .map(|threshold| comparator_fires(&rule.comparator, value, threshold))
            .unwrap_or(false),
    };

    let latest = telemetry_alert_event::Entity::find()
        .filter(telemetry_alert_event::Column::RuleId.eq(&rule.id))
        .order_by_desc(telemetry_alert_event::Column::CreatedAt)
        .order_by_desc(telemetry_alert_event::Column::Id)
        .one(db)
        .await?;
    let transition = next_transition(fires, is_open(latest.as_ref().map(|e| e.status.as_str())));
    let mut event = None;

    if transition != AlertTransition::None {
        let (status, message) = match transition {
            AlertTransition::Trigger if rule.mode == ALERT_MODE_ANOMALY => (
                ALERT_STATUS_TRIGGERED,
                anomaly_message(
                    &rule.metric,
                    value.unwrap_or_default(),
                    baseline,
                    sensitivity,
                    window_minutes,
                ),
            ),
            AlertTransition::Trigger => (
                ALERT_STATUS_TRIGGERED,
                threshold_message(
                    &rule.metric,
                    &rule.comparator,
                    value.unwrap_or_default(),
                    rule.threshold.unwrap_or_default(),
                    window_minutes,
                ),
            ),
            _ => (
                ALERT_STATUS_RESOLVED,
                resolved_message(&rule.metric, value, window_minutes),
            ),
        };

        event = Some(
            telemetry_alert_event::ActiveModel {
                id: Set(flow_like_types::create_id()),
                rule_id: Set(rule.id.clone()),
                rule_name: Set(rule.name.clone()),
                status: Set(status.to_string()),
                value: Set(value.unwrap_or_default()),
                threshold: Set(rule.threshold),
                message: Set(message),
                acknowledged_at: Set(None),
                created_at: Set(now),
            }
            .insert(db)
            .await?,
        );
    }

    let mut active = rule.clone().into_active_model();
    active.last_evaluated_at = Set(Some(now));
    active.last_value = Set(value);
    if transition == AlertTransition::Trigger {
        active.last_triggered_at = Set(Some(now));
    }
    active.updated_at = Set(now);
    active.update(db).await?;

    Ok(RuleEvaluation { transition, event })
}

/// The `min_samples` windows of the same length that precede the current one.
///
/// Windows without data contribute no sample, which keeps a sparse history from
/// producing a baseline the rule could fire against.
async fn baseline_samples<C: ConnectionTrait>(
    db: &C,
    dialect: DbDialect,
    metric: &str,
    source: Option<&str>,
    now: DateTime<FixedOffset>,
    window: ChronoDuration,
    min_samples: usize,
) -> Result<Vec<f64>, DbErr> {
    let mut samples = Vec::with_capacity(min_samples);
    for step in 1..=min_samples {
        let to = now - window * step as i32;
        if let Some(value) = metric_value(db, dialect, metric, source, to - window, to).await? {
            samples.push(value);
        }
    }
    Ok(samples)
}

/// The value of `metric` over `[from, to)`, or `None` when the window holds no
/// samples at all. A metric without data never fires a rule.
async fn metric_value<C: ConnectionTrait>(
    db: &C,
    dialect: DbDialect,
    metric: &str,
    source: Option<&str>,
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
) -> Result<Option<f64>, DbErr> {
    match metric {
        "event_count" => Ok(Some(event_count(db, source, from, to).await? as f64)),
        "error_rate" => {
            let events = event_count(db, source, from, to).await?;
            if events <= 0 {
                return Ok(None);
            }
            let errors = error_event_count(db, source, from, to).await?;
            Ok(Some(errors as f64 / events as f64))
        }
        "crash_free_rate" => {
            let pair = session_pair(db, source, from, to).await?;
            Ok(rate(pair.total - pair.matched, pair.total))
        }
        "span_error_rate" => {
            let pair = span_pair(db, source, from, to).await?;
            Ok(rate(pair.matched, pair.total))
        }
        "llm_error_rate" => {
            let pair = llm_pair(db, source, from, to).await?;
            Ok(rate(pair.matched, pair.total))
        }
        "latency_p95" => span_p95(db, dialect, source, from, to).await,
        _ => Ok(None),
    }
}

#[derive(Debug, Default, FromQueryResult)]
struct PairRow {
    total: i64,
    matched: i64,
}

#[derive(Debug, FromQueryResult)]
struct P95Row {
    p95: Option<f64>,
    cnt: i64,
}

#[derive(Debug, FromQueryResult)]
struct DurationRow {
    duration_ms: i32,
}

/// `COUNT(CASE WHEN <status column> = <status> THEN <id column> END)`, portable
/// across backends.
fn status_match_count<S, I>(status_column: S, id_column: I, status: &str) -> SimpleExpr
where
    S: IntoColumnRef,
    I: IntoColumnRef,
{
    use sea_orm::sea_query::ExprTrait;

    let case = Expr::case(Expr::col(status_column).eq(status), Expr::col(id_column))
        .finally(sea_orm::Value::String(None));
    Expr::expr(case).count()
}

async fn event_count<C: ConnectionTrait>(
    db: &C,
    source: Option<&str>,
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
) -> Result<i64, DbErr> {
    let mut select = telemetry_event::Entity::find()
        .filter(telemetry_event::Column::CreatedAt.gte(from))
        .filter(telemetry_event::Column::CreatedAt.lt(to));
    if let Some(source) = source {
        select = select.filter(telemetry_event::Column::Source.eq(source));
    }
    Ok(select.count(db).await? as i64)
}

async fn error_event_count<C: ConnectionTrait>(
    db: &C,
    source: Option<&str>,
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
) -> Result<i64, DbErr> {
    let mut select = telemetry_error_event::Entity::find()
        .filter(telemetry_error_event::Column::CreatedAt.gte(from))
        .filter(telemetry_error_event::Column::CreatedAt.lt(to));
    if let Some(source) = source {
        select = select.filter(telemetry_error_event::Column::Source.eq(source));
    }
    Ok(select.count(db).await? as i64)
}

async fn session_pair<C: ConnectionTrait>(
    db: &C,
    source: Option<&str>,
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
) -> Result<PairRow, DbErr> {
    use sea_orm::sea_query::ExprTrait;

    let mut select = telemetry_session::Entity::find()
        .select_only()
        .column_as(Expr::col(telemetry_session::Column::Id).count(), "total")
        .column_as(
            status_match_count(
                telemetry_session::Column::Status,
                telemetry_session::Column::Id,
                CRASHED_STATUS,
            ),
            "matched",
        )
        .filter(telemetry_session::Column::StartedAt.gte(from))
        .filter(telemetry_session::Column::StartedAt.lt(to));
    if let Some(source) = source {
        select = select.filter(telemetry_session::Column::Source.eq(source));
    }
    Ok(select
        .into_model::<PairRow>()
        .one(db)
        .await?
        .unwrap_or_default())
}

async fn span_pair<C: ConnectionTrait>(
    db: &C,
    source: Option<&str>,
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
) -> Result<PairRow, DbErr> {
    use sea_orm::sea_query::ExprTrait;

    let mut select = telemetry_span::Entity::find()
        .select_only()
        .column_as(Expr::col(telemetry_span::Column::Id).count(), "total")
        .column_as(
            status_match_count(
                telemetry_span::Column::Status,
                telemetry_span::Column::Id,
                ERROR_STATUS,
            ),
            "matched",
        )
        .filter(telemetry_span::Column::StartedAt.gte(from))
        .filter(telemetry_span::Column::StartedAt.lt(to));
    if let Some(source) = source {
        select = select.filter(telemetry_span::Column::Source.eq(source));
    }
    Ok(select
        .into_model::<PairRow>()
        .one(db)
        .await?
        .unwrap_or_default())
}

async fn llm_pair<C: ConnectionTrait>(
    db: &C,
    source: Option<&str>,
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
) -> Result<PairRow, DbErr> {
    use sea_orm::sea_query::ExprTrait;

    let mut select = telemetry_llm_call::Entity::find()
        .select_only()
        .column_as(Expr::col(telemetry_llm_call::Column::Id).count(), "total")
        .column_as(
            status_match_count(
                telemetry_llm_call::Column::Status,
                telemetry_llm_call::Column::Id,
                ERROR_STATUS,
            ),
            "matched",
        )
        .filter(telemetry_llm_call::Column::CreatedAt.gte(from))
        .filter(telemetry_llm_call::Column::CreatedAt.lt(to));
    if let Some(source) = source {
        select = select.filter(telemetry_llm_call::Column::Source.eq(source));
    }
    Ok(select
        .into_model::<PairRow>()
        .one(db)
        .await?
        .unwrap_or_default())
}

/// p95 span duration in the window: `percentile_cont` where the engine has
/// ordered-set aggregates, a capped fold in Rust everywhere else.
async fn span_p95<C: ConnectionTrait>(
    db: &C,
    dialect: DbDialect,
    source: Option<&str>,
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
) -> Result<Option<f64>, DbErr> {
    let backend = db.get_database_backend();
    if !percentiles_in_sql(backend, dialect) {
        let mut select = telemetry_span::Entity::find()
            .select_only()
            .column_as(telemetry_span::Column::DurationMs, "duration_ms")
            .filter(telemetry_span::Column::StartedAt.gte(from))
            .filter(telemetry_span::Column::StartedAt.lt(to));
        if let Some(source) = source {
            select = select.filter(telemetry_span::Column::Source.eq(source));
        }

        let rows = select
            .limit(SPAN_ROW_CAP)
            .into_model::<DurationRow>()
            .all(db)
            .await?;
        if rows.is_empty() {
            return Ok(None);
        }

        let mut durations: Vec<f64> = rows.into_iter().map(|row| row.duration_ms as f64).collect();
        durations.sort_by(|a, b| a.total_cmp(b));
        return Ok(Some(percentile(&durations, 0.95)));
    }

    let mut values: Vec<sea_orm::Value> = vec![from.into(), to.into()];
    let mut conditions = r#""startedAt" >= $1 AND "startedAt" < $2"#.to_string();
    if let Some(source) = source {
        values.push(source.to_string().into());
        conditions.push_str(&format!(r#" AND "source" = ${}"#, values.len()));
    }

    let sql = format!(
        r#"SELECT percentile_cont(0.95::float8) WITHIN GROUP (ORDER BY "durationMs"::float8) AS p95,
                  CAST(COUNT(*) AS BIGINT) AS cnt
           FROM "TelemetrySpan"
           WHERE {conditions}"#
    );

    let row = P95Row::find_by_statement(Statement::from_sql_and_values(backend, sql, values))
        .one(db)
        .await?;

    Ok(row.filter(|row| row.cnt > 0).and_then(|row| row.p95))
}

fn alerts_disabled() -> bool {
    std::env::var("FLOW_LIKE_TELEMETRY_ALERTS_DISABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub(crate) fn parse_interval(raw: Option<&str>) -> Duration {
    let secs = raw
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(MIN_INTERVAL_SECS);
    Duration::from_secs(secs)
}

/// Share of `total` that `part` accounts for, `None` without samples.
pub fn rate(part: i64, total: i64) -> Option<f64> {
    if total <= 0 {
        return None;
    }
    Some(part as f64 / total as f64)
}

/// Mean and population standard deviation of the baseline windows.
pub fn baseline_stats(samples: &[f64]) -> BaselineStats {
    if samples.is_empty() {
        return BaselineStats::default();
    }

    let count = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / count;
    let variance = samples
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / count;

    BaselineStats {
        samples: samples.len(),
        mean,
        stddev: variance.sqrt(),
    }
}

/// Whether the observed value breaches the threshold. A value sitting exactly
/// on the threshold never fires, and an unknown comparator never fires.
pub fn comparator_fires(comparator: &str, value: f64, threshold: f64) -> bool {
    match comparator {
        "gt" => value > threshold,
        "lt" => value < threshold,
        _ => false,
    }
}

/// Whether the observed value is further than `sensitivity` population standard
/// deviations from the baseline mean.
///
/// Never fires on a short baseline, a flat one (stddev 0) or a nonsensical
/// sensitivity — insufficient data must never look like an incident.
pub fn anomaly_fires(value: f64, baseline: &[f64], sensitivity: f64, min_samples: usize) -> bool {
    let stats = baseline_stats(baseline);
    if stats.samples < min_samples.max(1) {
        return false;
    }
    if !stats.stddev.is_finite() || stats.stddev <= 0.0 {
        return false;
    }
    if !sensitivity.is_finite() || sensitivity <= 0.0 {
        return false;
    }
    (value - stats.mean).abs() > sensitivity * stats.stddev
}

/// A rule is open while the newest event it produced is a trigger.
pub fn is_open(latest_status: Option<&str>) -> bool {
    latest_status == Some(ALERT_STATUS_TRIGGERED)
}

/// The state machine that keeps the inbox free of duplicate consecutive
/// triggers: a rule only triggers while nothing is open, and only resolves once
/// something is.
pub fn next_transition(fires: bool, open: bool) -> AlertTransition {
    match (fires, open) {
        (true, false) => AlertTransition::Trigger,
        (false, true) => AlertTransition::Resolve,
        _ => AlertTransition::None,
    }
}

/// Linear-interpolating percentile over an ascending slice, matching the
/// semantics of SQL `percentile_cont`.
pub(crate) fn percentile(sorted: &[f64], q: f64) -> f64 {
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

pub(crate) fn format_metric_value(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        return format!("{value:.0}");
    }
    format!("{value:.3}")
}

pub(crate) fn threshold_message(
    metric: &str,
    comparator: &str,
    value: f64,
    threshold: f64,
    window_minutes: i32,
) -> String {
    let direction = if comparator == "lt" { "below" } else { "above" };
    format!(
        "{metric} is {} over the last {window_minutes} min, {direction} the threshold of {}",
        format_metric_value(value),
        format_metric_value(threshold)
    )
}

pub(crate) fn anomaly_message(
    metric: &str,
    value: f64,
    baseline: BaselineStats,
    sensitivity: f64,
    window_minutes: i32,
) -> String {
    let deviation = if baseline.stddev > 0.0 {
        (value - baseline.mean).abs() / baseline.stddev
    } else {
        0.0
    };
    format!(
        "{metric} is {} over the last {window_minutes} min, {} standard deviations from the baseline mean of {} (stddev {}, {} windows, sensitivity {})",
        format_metric_value(value),
        format_metric_value(deviation),
        format_metric_value(baseline.mean),
        format_metric_value(baseline.stddev),
        baseline.samples,
        format_metric_value(sensitivity)
    )
}

pub(crate) fn resolved_message(metric: &str, value: Option<f64>, window_minutes: i32) -> String {
    match value {
        Some(value) => format!(
            "{metric} recovered to {} over the last {window_minutes} min",
            format_metric_value(value)
        ),
        None => format!("{metric} has no samples in the last {window_minutes} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_defaults_and_never_drops_below_thirty_seconds() {
        assert_eq!(parse_interval(None), Duration::from_secs(300));
        assert_eq!(parse_interval(Some("")), Duration::from_secs(300));
        assert_eq!(parse_interval(Some("nope")), Duration::from_secs(300));
        assert_eq!(parse_interval(Some("0")), Duration::from_secs(30));
        assert_eq!(parse_interval(Some("5")), Duration::from_secs(30));
        assert_eq!(parse_interval(Some(" 900 ")), Duration::from_secs(900));
    }

    #[test]
    fn default_config_matches_the_documented_interval() {
        let config = TelemetryAlertConfig::default();
        assert_eq!(config.interval, Duration::from_secs(300));
        assert_eq!(config.rule_cap, 200);
        assert!(AlertEvaluationResult::default().is_empty());
    }

    #[test]
    fn vocabulary_is_closed() {
        assert!(is_valid_metric("error_rate"));
        assert!(is_valid_metric("llm_error_rate"));
        assert!(!is_valid_metric("cpu"));
        assert!(is_valid_comparator("gt"));
        assert!(is_valid_comparator("lt"));
        assert!(!is_valid_comparator("gte"));
        assert!(is_valid_mode(ALERT_MODE_THRESHOLD));
        assert!(is_valid_mode(ALERT_MODE_ANOMALY));
        assert!(!is_valid_mode("ml"));
    }

    #[test]
    fn rates_are_none_without_samples() {
        assert_eq!(rate(1, 0), None);
        assert_eq!(rate(0, -1), None);
        assert_eq!(rate(1, 4), Some(0.25));
        assert_eq!(rate(4, 4), Some(1.0));
        assert_eq!(rate(0, 4), Some(0.0));
    }

    #[test]
    fn baseline_stats_use_the_population_standard_deviation() {
        let stats = baseline_stats(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
        assert_eq!(stats.samples, 8);
        assert_eq!(stats.mean, 5.0);
        assert_eq!(stats.stddev, 2.0);
    }

    #[test]
    fn baseline_stats_of_an_empty_or_flat_series() {
        assert_eq!(baseline_stats(&[]), BaselineStats::default());

        let single = baseline_stats(&[3.0]);
        assert_eq!(single.samples, 1);
        assert_eq!(single.mean, 3.0);
        assert_eq!(single.stddev, 0.0);

        let flat = baseline_stats(&[7.0, 7.0, 7.0]);
        assert_eq!(flat.mean, 7.0);
        assert_eq!(flat.stddev, 0.0);
    }

    #[test]
    fn a_value_exactly_at_the_threshold_never_fires() {
        assert!(!comparator_fires("gt", 0.05, 0.05));
        assert!(!comparator_fires("lt", 0.05, 0.05));
        assert!(comparator_fires("gt", 0.051, 0.05));
        assert!(!comparator_fires("gt", 0.049, 0.05));
        assert!(comparator_fires("lt", 0.049, 0.05));
        assert!(!comparator_fires("lt", 0.051, 0.05));
    }

    #[test]
    fn an_unknown_comparator_never_fires() {
        assert!(!comparator_fires("gte", 1.0, 0.0));
        assert!(!comparator_fires("", 1.0, 0.0));
    }

    #[test]
    fn anomalies_need_the_full_baseline() {
        let baseline = [10.0, 11.0, 9.0, 10.0];
        assert!(!anomaly_fires(100.0, &baseline, 3.0, 5));
        assert!(!anomaly_fires(100.0, &[], 3.0, 5));
        assert!(anomaly_fires(100.0, &baseline, 3.0, 4));
    }

    #[test]
    fn a_flat_baseline_never_fires() {
        let baseline = [10.0, 10.0, 10.0, 10.0, 10.0];
        assert!(!anomaly_fires(10.0, &baseline, 3.0, 5));
        assert!(!anomaly_fires(9999.0, &baseline, 3.0, 5));
    }

    #[test]
    fn anomalies_fire_on_both_sides_of_the_baseline() {
        let baseline = [10.0, 12.0, 8.0, 10.0, 10.0];
        let stats = baseline_stats(&baseline);
        assert!(stats.stddev > 0.0);

        let high = stats.mean + 3.1 * stats.stddev;
        let low = stats.mean - 3.1 * stats.stddev;
        assert!(anomaly_fires(high, &baseline, 3.0, 5));
        assert!(anomaly_fires(low, &baseline, 3.0, 5));

        let inside = stats.mean + 2.9 * stats.stddev;
        assert!(!anomaly_fires(inside, &baseline, 3.0, 5));
        assert!(!anomaly_fires(stats.mean, &baseline, 3.0, 5));
    }

    #[test]
    fn a_value_exactly_at_the_sensitivity_boundary_never_fires() {
        // mean 10, population stddev exactly 2, so the boundary is exactly 16.
        let baseline = [8.0, 12.0, 8.0, 12.0];
        let stats = baseline_stats(&baseline);
        assert_eq!(stats.mean, 10.0);
        assert_eq!(stats.stddev, 2.0);

        assert!(!anomaly_fires(16.0, &baseline, 3.0, 4));
        assert!(!anomaly_fires(4.0, &baseline, 3.0, 4));
        assert!(anomaly_fires(16.001, &baseline, 3.0, 4));
        assert!(anomaly_fires(3.999, &baseline, 3.0, 4));
    }

    #[test]
    fn a_nonsensical_sensitivity_never_fires() {
        let baseline = [10.0, 12.0, 8.0, 10.0, 10.0];
        assert!(!anomaly_fires(1000.0, &baseline, 0.0, 5));
        assert!(!anomaly_fires(1000.0, &baseline, -3.0, 5));
        assert!(!anomaly_fires(1000.0, &baseline, f64::NAN, 5));
    }

    #[test]
    fn open_rules_are_the_ones_whose_latest_event_is_a_trigger() {
        assert!(is_open(Some(ALERT_STATUS_TRIGGERED)));
        assert!(!is_open(Some(ALERT_STATUS_RESOLVED)));
        assert!(!is_open(None));
    }

    #[test]
    fn triggers_are_deduped_until_the_rule_resolves() {
        let mut latest: Option<&str> = None;
        let mut inbox: Vec<&str> = Vec::new();

        for fires in [true, true, true, false, false, true] {
            match next_transition(fires, is_open(latest)) {
                AlertTransition::Trigger => {
                    inbox.push(ALERT_STATUS_TRIGGERED);
                    latest = Some(ALERT_STATUS_TRIGGERED);
                }
                AlertTransition::Resolve => {
                    inbox.push(ALERT_STATUS_RESOLVED);
                    latest = Some(ALERT_STATUS_RESOLVED);
                }
                AlertTransition::None => {}
            }
        }

        assert_eq!(
            inbox,
            vec![
                ALERT_STATUS_TRIGGERED,
                ALERT_STATUS_RESOLVED,
                ALERT_STATUS_TRIGGERED
            ]
        );
    }

    #[test]
    fn a_rule_that_never_fired_never_resolves() {
        assert_eq!(next_transition(false, false), AlertTransition::None);
        assert_eq!(next_transition(false, true), AlertTransition::Resolve);
        assert_eq!(next_transition(true, false), AlertTransition::Trigger);
        assert_eq!(next_transition(true, true), AlertTransition::None);
    }

    #[test]
    fn percentiles_interpolate_like_percentile_cont() {
        assert_eq!(percentile(&[], 0.95), 0.0);
        assert_eq!(percentile(&[42.0], 0.95), 42.0);
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0, 5.0], 0.5), 3.0);
        assert_eq!(percentile(&[10.0, 20.0], 0.95), 19.5);
    }

    #[test]
    fn messages_state_the_metric_and_the_numbers_without_identity() {
        let threshold = threshold_message("error_rate", "gt", 0.1234, 0.05, 15);
        assert_eq!(
            threshold,
            "error_rate is 0.123 over the last 15 min, above the threshold of 0.050"
        );

        let below = threshold_message("crash_free_rate", "lt", 0.9, 0.99, 60);
        assert!(below.contains("below the threshold of 0.990"), "{below}");

        let counts = threshold_message("event_count", "lt", 0.0, 100.0, 60);
        assert!(counts.contains("is 0 over the last 60 min"), "{counts}");

        let anomaly = anomaly_message(
            "latency_p95",
            900.0,
            baseline_stats(&[100.0, 200.0]),
            3.0,
            5,
        );
        assert!(anomaly.contains("latency_p95 is 900"), "{anomaly}");
        assert!(anomaly.contains("2 windows"), "{anomaly}");

        assert_eq!(
            resolved_message("error_rate", Some(0.01), 15),
            "error_rate recovered to 0.010 over the last 15 min"
        );
        assert_eq!(
            resolved_message("error_rate", None, 15),
            "error_rate has no samples in the last 15 min"
        );
    }
}
