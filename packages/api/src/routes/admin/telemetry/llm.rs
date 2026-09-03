//! LLM observability: call volume, failure rate, token spend and latency.
//!
//! Windows of at most 48 hours are computed over the individual calls: with
//! `percentile_cont` on Postgres, and with a single capped fetch folded in Rust
//! on other backends. Longer windows read `TelemetryLlmDaily`.
//!
//! Percentiles are not summable, so a rollup window reports no p95 at all
//! (`p95DurationMs` is null) rather than a fabricated one. What the rollup does
//! carry is exact: the mean derived from `durationSumMs / calls` and the true
//! maximum from `durationMaxMs`.
//!
//! The rollup has no `operation` or `errorKind` dimension, so those two
//! breakdowns are always read as grouped aggregates over the raw calls, clipped
//! to the retention window the sweeper keeps them for. `breakdownWindowHours`
//! reports how much they actually cover.

use super::overview::{
    GRANULARITY_DAILY, GRANULARITY_RAW, day_window, reads_raw, retention_days, window_bucket,
};
use super::performance::{percentile_cont, round3};
use super::{bucket_slots, trunc_to_bucket};
use crate::entity::{telemetry_llm_call, telemetry_llm_daily};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use crate::telemetry::llm::LLM_STATUS_ERROR;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Select, Statement,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};

const DEFAULT_LLM_HOURS: i64 = 24;
const MAX_LLM_HOURS: i64 = 24 * 90;
/// Rows returned by every breakdown list.
const LLM_TOP_LIMIT: usize = 20;
/// Upper bound on the calls folded in Rust when the backend has no
/// `percentile_cont`.
const LLM_ROW_CAP: u64 = 100_000;
/// Mirrors the telemetry sweeper's default raw LLM call retention.
const DEFAULT_LLM_RETENTION_DAYS: i64 = 30;
const LLM_RETENTION_VAR: &str = "FLOW_LIKE_LLM_RETENTION_DAYS";

#[derive(Debug, Deserialize, IntoParams)]
pub struct TelemetryLlmQuery {
    /// Lookback window in hours. Default 24.
    #[serde(default)]
    pub hours: Option<i64>,
    /// Filter by provider, e.g. "openai".
    #[serde(default)]
    pub provider: Option<String>,
    /// Filter by model identifier.
    #[serde(default)]
    pub model: Option<String>,
    /// Filter by source: "desktop", "web", "desktop_native" or "backend".
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmTotals {
    pub calls: i64,
    pub errors: i64,
    /// Failed calls divided by all calls, between 0 and 1.
    pub error_rate: f64,
    pub total_tokens: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub avg_duration_ms: f64,
    /// Null outside the raw window: percentiles cannot be recomputed from daily
    /// aggregates, so none is reported rather than an invented one.
    pub p95_duration_ms: Option<f64>,
    pub max_duration_ms: f64,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmModelStats {
    pub provider: String,
    pub model: String,
    pub calls: i64,
    pub errors: i64,
    pub error_rate: f64,
    pub avg_duration_ms: f64,
    /// Null outside the raw window.
    pub p95_duration_ms: Option<f64>,
    pub max_duration_ms: f64,
    pub total_tokens: i64,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderStats {
    pub provider: String,
    pub calls: i64,
    pub errors: i64,
    pub error_rate: f64,
    pub avg_duration_ms: f64,
    /// Null outside the raw window.
    pub p95_duration_ms: Option<f64>,
    pub max_duration_ms: f64,
    pub total_tokens: i64,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmOperationStats {
    /// One of "chat", "embed" or "tool".
    pub operation: String,
    pub calls: i64,
    pub error_rate: f64,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmErrorKindStats {
    pub error_kind: String,
    pub count: i64,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LlmTrendPoint {
    /// ISO-8601 timestamp at the start of the bucket.
    pub ts: String,
    pub calls: i64,
    pub errors: i64,
    /// Null outside the raw window.
    pub p95_duration_ms: Option<f64>,
    pub max_duration_ms: f64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryLlmResponse {
    pub hours: i64,
    /// "raw" when the numbers come from individual calls, "daily" when they come
    /// from the daily rollup. Only "raw" carries latency percentiles.
    pub granularity: String,
    pub totals: LlmTotals,
    pub by_model: Vec<LlmModelStats>,
    pub by_provider: Vec<LlmProviderStats>,
    pub by_operation: Vec<LlmOperationStats>,
    pub top_errors: Vec<LlmErrorKindStats>,
    pub trend: Vec<LlmTrendPoint>,
    /// Hours of raw calls the operation and error-kind breakdowns could read.
    pub breakdown_window_hours: i64,
}

#[derive(Clone, Debug, FromQueryResult)]
struct LlmSampleRow {
    provider: String,
    model: String,
    operation: String,
    status: String,
    error_kind: Option<String>,
    duration_ms: i32,
    prompt_tokens: Option<i32>,
    completion_tokens: Option<i32>,
    total_tokens: Option<i32>,
    created_at: NaiveDateTime,
}

#[derive(Debug, FromQueryResult)]
struct TotalsRow {
    calls: i64,
    errors: i64,
    total_tokens: i64,
    prompt_tokens: i64,
    completion_tokens: i64,
    avg_duration_ms: f64,
    p95_duration_ms: f64,
    max_duration_ms: f64,
}

#[derive(Debug, FromQueryResult)]
struct ModelRow {
    provider: String,
    model: String,
    calls: i64,
    errors: i64,
    total_tokens: i64,
    avg_duration_ms: f64,
    p95_duration_ms: f64,
    max_duration_ms: f64,
}

#[derive(Debug, FromQueryResult)]
struct ProviderRow {
    provider: String,
    calls: i64,
    errors: i64,
    total_tokens: i64,
    avg_duration_ms: f64,
    p95_duration_ms: f64,
    max_duration_ms: f64,
}

#[derive(Debug, FromQueryResult)]
struct OperationRow {
    operation: String,
    calls: i64,
    errors: i64,
}

#[derive(Debug, FromQueryResult)]
struct ErrorKindRow {
    error_kind: String,
    count: i64,
}

#[derive(Debug, FromQueryResult)]
struct LlmTrendRow {
    bucket: NaiveDateTime,
    calls: i64,
    errors: i64,
    p95_duration_ms: f64,
    max_duration_ms: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct TrendBucket {
    calls: i64,
    errors: i64,
    p95_duration_ms: Option<f64>,
    max_duration_ms: f64,
}

#[derive(Debug, Default)]
struct LlmFold {
    totals: LlmTotals,
    by_model: Vec<LlmModelStats>,
    by_provider: Vec<LlmProviderStats>,
    by_operation: Vec<LlmOperationStats>,
    top_errors: Vec<LlmErrorKindStats>,
    trend: Vec<LlmTrendPoint>,
}

/// Zero calls must read as a healthy 0.0, never as a NaN division.
fn error_rate(errors: i64, calls: i64) -> f64 {
    if calls <= 0 {
        return 0.0;
    }
    round3((errors as f64 / calls as f64).clamp(0.0, 1.0))
}

/// A row that reports no total falls back to the sum of its parts, so a client
/// that only knows prompt and completion counts still contributes to spend.
fn effective_total_tokens(prompt: Option<i32>, completion: Option<i32>, total: Option<i32>) -> i64 {
    match total {
        Some(total) => i64::from(total),
        None => i64::from(prompt.unwrap_or(0)) + i64::from(completion.unwrap_or(0)),
    }
}

/// An empty raw window still knows its latency is zero; an empty rollup window
/// has no percentile to report at all.
fn empty_raw_totals() -> LlmTotals {
    LlmTotals {
        p95_duration_ms: Some(0.0),
        ..Default::default()
    }
}

#[derive(Debug, Default)]
struct LlmAccumulator {
    durations: Vec<f64>,
    errors: i64,
    duration_total: f64,
    duration_max: f64,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
}

impl LlmAccumulator {
    fn push(&mut self, row: &LlmSampleRow) {
        let duration = f64::from(row.duration_ms);
        self.durations.push(duration);
        self.duration_total += duration;
        self.duration_max = self.duration_max.max(duration);
        if row.status == LLM_STATUS_ERROR {
            self.errors += 1;
        }
        self.prompt_tokens += i64::from(row.prompt_tokens.unwrap_or(0));
        self.completion_tokens += i64::from(row.completion_tokens.unwrap_or(0));
        self.total_tokens +=
            effective_total_tokens(row.prompt_tokens, row.completion_tokens, row.total_tokens);
    }

    fn calls(&self) -> i64 {
        self.durations.len() as i64
    }

    fn avg_duration_ms(&self) -> f64 {
        if self.durations.is_empty() {
            return 0.0;
        }
        round3(self.duration_total / self.durations.len() as f64)
    }

    fn p95_duration_ms(&mut self) -> f64 {
        self.durations.sort_by(|a, b| a.total_cmp(b));
        round3(percentile_cont(&self.durations, 0.95))
    }
}

/// Accumulates pre-aggregated days. Sums stay exact; the maximum is the true
/// maximum because it is a maximum of daily maxima.
#[derive(Debug, Default)]
struct DailyAccumulator {
    calls: i64,
    errors: i64,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    duration_sum_ms: i64,
    duration_max_ms: i64,
}

impl DailyAccumulator {
    fn push(&mut self, row: &telemetry_llm_daily::Model) {
        self.calls = self.calls.saturating_add(i64::from(row.calls.max(0)));
        self.errors = self.errors.saturating_add(i64::from(row.errors.max(0)));
        self.prompt_tokens = self.prompt_tokens.saturating_add(row.prompt_tokens.max(0));
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(row.completion_tokens.max(0));
        self.total_tokens = self.total_tokens.saturating_add(row.total_tokens.max(0));
        self.duration_sum_ms = self
            .duration_sum_ms
            .saturating_add(row.duration_sum_ms.max(0));
        self.duration_max_ms = self
            .duration_max_ms
            .max(i64::from(row.duration_max_ms.max(0)));
    }

    fn avg_duration_ms(&self) -> f64 {
        if self.calls <= 0 {
            return 0.0;
        }
        round3(self.duration_sum_ms as f64 / self.calls as f64)
    }

    fn max_duration_ms(&self) -> f64 {
        self.duration_max_ms as f64
    }
}

fn rank_models(mut rows: Vec<LlmModelStats>) -> Vec<LlmModelStats> {
    rows.sort_by(|a, b| {
        b.calls
            .cmp(&a.calls)
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.model.cmp(&b.model))
    });
    rows.truncate(LLM_TOP_LIMIT);
    rows
}

fn rank_providers(mut rows: Vec<LlmProviderStats>) -> Vec<LlmProviderStats> {
    rows.sort_by(|a, b| {
        b.calls
            .cmp(&a.calls)
            .then_with(|| a.provider.cmp(&b.provider))
    });
    rows.truncate(LLM_TOP_LIMIT);
    rows
}

fn rank_operations(mut rows: Vec<LlmOperationStats>) -> Vec<LlmOperationStats> {
    rows.sort_by(|a, b| {
        b.calls
            .cmp(&a.calls)
            .then_with(|| a.operation.cmp(&b.operation))
    });
    rows.truncate(LLM_TOP_LIMIT);
    rows
}

fn rank_error_kinds(mut rows: Vec<LlmErrorKindStats>) -> Vec<LlmErrorKindStats> {
    rows.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.error_kind.cmp(&b.error_kind))
    });
    rows.truncate(LLM_TOP_LIMIT);
    rows
}

fn fill_trend(
    buckets: BTreeMap<NaiveDateTime, TrendBucket>,
    cutoff: NaiveDateTime,
    now: NaiveDateTime,
    bucket: &str,
    empty: TrendBucket,
) -> Vec<LlmTrendPoint> {
    bucket_slots(cutoff, now, bucket)
        .into_iter()
        .map(|slot| {
            let point = buckets.get(&slot).copied().unwrap_or(empty);
            LlmTrendPoint {
                ts: DateTime::<Utc>::from_naive_utc_and_offset(slot, Utc).to_rfc3339(),
                calls: point.calls,
                errors: point.errors,
                p95_duration_ms: point.p95_duration_ms,
                max_duration_ms: point.max_duration_ms,
            }
        })
        .collect()
}

/// Zero-filled slots inside the raw window report a latency of zero, matching
/// the buckets that do have calls.
fn empty_raw_bucket() -> TrendBucket {
    TrendBucket {
        p95_duration_ms: Some(0.0),
        ..Default::default()
    }
}

fn fold_llm_samples(
    rows: Vec<LlmSampleRow>,
    cutoff: NaiveDateTime,
    now: NaiveDateTime,
    bucket: &str,
) -> LlmFold {
    let mut totals = LlmAccumulator::default();
    let mut per_model: BTreeMap<(String, String), LlmAccumulator> = BTreeMap::new();
    let mut per_provider: BTreeMap<String, LlmAccumulator> = BTreeMap::new();
    let mut per_operation: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut per_error: BTreeMap<String, i64> = BTreeMap::new();
    let mut per_bucket: BTreeMap<NaiveDateTime, LlmAccumulator> = BTreeMap::new();

    for row in &rows {
        totals.push(row);
        per_model
            .entry((row.provider.clone(), row.model.clone()))
            .or_default()
            .push(row);
        per_provider
            .entry(row.provider.clone())
            .or_default()
            .push(row);
        per_bucket
            .entry(trunc_to_bucket(row.created_at, bucket))
            .or_default()
            .push(row);

        let operation = per_operation.entry(row.operation.clone()).or_default();
        operation.0 += 1;
        if row.status == LLM_STATUS_ERROR {
            operation.1 += 1;
            if let Some(kind) = row.error_kind.as_deref().filter(|kind| !kind.is_empty()) {
                *per_error.entry(kind.to_string()).or_default() += 1;
            }
        }
    }

    let by_model = per_model
        .into_iter()
        .map(|((provider, model), mut acc)| LlmModelStats {
            provider,
            model,
            calls: acc.calls(),
            errors: acc.errors,
            error_rate: error_rate(acc.errors, acc.calls()),
            avg_duration_ms: acc.avg_duration_ms(),
            p95_duration_ms: Some(acc.p95_duration_ms()),
            max_duration_ms: acc.duration_max,
            total_tokens: acc.total_tokens,
        })
        .collect();

    let by_provider = per_provider
        .into_iter()
        .map(|(provider, mut acc)| LlmProviderStats {
            provider,
            calls: acc.calls(),
            errors: acc.errors,
            error_rate: error_rate(acc.errors, acc.calls()),
            avg_duration_ms: acc.avg_duration_ms(),
            p95_duration_ms: Some(acc.p95_duration_ms()),
            max_duration_ms: acc.duration_max,
            total_tokens: acc.total_tokens,
        })
        .collect();

    let by_operation = per_operation
        .into_iter()
        .map(|(operation, (calls, errors))| LlmOperationStats {
            operation,
            calls,
            error_rate: error_rate(errors, calls),
        })
        .collect();

    let top_errors = per_error
        .into_iter()
        .map(|(error_kind, count)| LlmErrorKindStats { error_kind, count })
        .collect();

    let trend_buckets = per_bucket
        .into_iter()
        .map(|(slot, mut acc)| {
            (
                slot,
                TrendBucket {
                    calls: acc.calls(),
                    errors: acc.errors,
                    p95_duration_ms: Some(acc.p95_duration_ms()),
                    max_duration_ms: acc.duration_max,
                },
            )
        })
        .collect();

    LlmFold {
        totals: LlmTotals {
            calls: totals.calls(),
            errors: totals.errors,
            error_rate: error_rate(totals.errors, totals.calls()),
            total_tokens: totals.total_tokens,
            prompt_tokens: totals.prompt_tokens,
            completion_tokens: totals.completion_tokens,
            avg_duration_ms: totals.avg_duration_ms(),
            p95_duration_ms: Some(totals.p95_duration_ms()),
            max_duration_ms: totals.duration_max,
        },
        by_model: rank_models(by_model),
        by_provider: rank_providers(by_provider),
        by_operation: rank_operations(by_operation),
        top_errors: rank_error_kinds(top_errors),
        trend: fill_trend(trend_buckets, cutoff, now, bucket, empty_raw_bucket()),
    }
}

/// Folds the daily rollup. Everything reported here is exact except that there
/// is deliberately no p95: daily percentiles cannot be recombined.
fn fold_llm_daily(
    rows: &[telemetry_llm_daily::Model],
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> LlmFold {
    let mut totals = DailyAccumulator::default();
    let mut per_model: BTreeMap<(&str, &str), DailyAccumulator> = BTreeMap::new();
    let mut per_provider: BTreeMap<&str, DailyAccumulator> = BTreeMap::new();
    let mut per_day: BTreeMap<NaiveDateTime, DailyAccumulator> = BTreeMap::new();

    for row in rows {
        totals.push(row);
        per_model
            .entry((row.provider.as_str(), row.model.as_str()))
            .or_default()
            .push(row);
        per_provider
            .entry(row.provider.as_str())
            .or_default()
            .push(row);
        per_day.entry(row.day).or_default().push(row);
    }

    let by_model = per_model
        .into_iter()
        .map(|((provider, model), acc)| LlmModelStats {
            provider: provider.to_string(),
            model: model.to_string(),
            calls: acc.calls,
            errors: acc.errors,
            error_rate: error_rate(acc.errors, acc.calls),
            avg_duration_ms: acc.avg_duration_ms(),
            p95_duration_ms: None,
            max_duration_ms: acc.max_duration_ms(),
            total_tokens: acc.total_tokens,
        })
        .collect();

    let by_provider = per_provider
        .into_iter()
        .map(|(provider, acc)| LlmProviderStats {
            provider: provider.to_string(),
            calls: acc.calls,
            errors: acc.errors,
            error_rate: error_rate(acc.errors, acc.calls),
            avg_duration_ms: acc.avg_duration_ms(),
            p95_duration_ms: None,
            max_duration_ms: acc.max_duration_ms(),
            total_tokens: acc.total_tokens,
        })
        .collect();

    let trend_buckets = per_day
        .into_iter()
        .map(|(day, acc)| {
            (
                day,
                TrendBucket {
                    calls: acc.calls,
                    errors: acc.errors,
                    p95_duration_ms: None,
                    max_duration_ms: acc.max_duration_ms(),
                },
            )
        })
        .collect();

    LlmFold {
        totals: LlmTotals {
            calls: totals.calls,
            errors: totals.errors,
            error_rate: error_rate(totals.errors, totals.calls),
            total_tokens: totals.total_tokens,
            prompt_tokens: totals.prompt_tokens,
            completion_tokens: totals.completion_tokens,
            avg_duration_ms: totals.avg_duration_ms(),
            p95_duration_ms: None,
            max_duration_ms: totals.max_duration_ms(),
        },
        by_model: rank_models(by_model),
        by_provider: rank_providers(by_provider),
        by_operation: Vec::new(),
        top_errors: Vec::new(),
        trend: fill_trend(trend_buckets, start, end, "day", TrendBucket::default()),
    }
}

struct LlmFilters {
    cutoff: NaiveDateTime,
    provider: Option<String>,
    model: Option<String>,
    source: Option<String>,
}

impl LlmFilters {
    fn new(q: &TelemetryLlmQuery, cutoff: NaiveDateTime) -> Self {
        Self {
            cutoff,
            provider: q
                .provider
                .clone()
                .map(|value| value.to_ascii_lowercase())
                .filter(|value| !value.is_empty()),
            model: q.model.clone().filter(|value| !value.is_empty()),
            source: q.source.clone().filter(|value| !value.is_empty()),
        }
    }
}

/// Raw calls only exist for as long as the sweeper keeps them, so a raw read is
/// clipped to that retention and reports the hours it really covers.
fn raw_window(now: NaiveDateTime, hours: i64, retention_hours: i64) -> (NaiveDateTime, i64) {
    let effective = hours.min(retention_hours);
    (now - Duration::hours(effective), effective)
}

fn llm_retention_hours() -> i64 {
    retention_days(LLM_RETENTION_VAR, DEFAULT_LLM_RETENTION_DAYS) * 24
}

/// Shared `WHERE` fragment plus its bound values, in placeholder order. Filter
/// values are always bound parameters, never interpolated into the SQL text.
fn llm_where(filters: &LlmFilters) -> (String, Vec<sea_orm::Value>) {
    let mut values: Vec<sea_orm::Value> = vec![filters.cutoff.into()];
    let mut clauses = vec![r#""createdAt" >= $1"#.to_string()];

    if let Some(value) = &filters.provider {
        values.push(value.clone().into());
        clauses.push(format!(r#""provider" = ${}"#, values.len()));
    }

    if let Some(value) = &filters.model {
        values.push(value.clone().into());
        clauses.push(format!(r#""model" = ${}"#, values.len()));
    }

    if let Some(value) = &filters.source {
        values.push(value.clone().into());
        clauses.push(format!(r#""source" = ${}"#, values.len()));
    }

    (clauses.join(" AND "), values)
}

/// `totalTokens` is nullable; fall back to the sum of prompt and completion so
/// the SQL path matches [`effective_total_tokens`].
const TOTAL_TOKENS_SQL: &str =
    r#"COALESCE("totalTokens", COALESCE("promptTokens", 0) + COALESCE("completionTokens", 0))"#;

async fn llm_from_sql<C: ConnectionTrait>(
    db: &C,
    filters: &LlmFilters,
    now: NaiveDateTime,
    bucket: &str,
) -> Result<LlmFold, ApiError> {
    let backend = db.get_database_backend();
    let (where_sql, values) = llm_where(filters);
    let errors = format!(r#"COUNT(*) FILTER (WHERE "status" = '{LLM_STATUS_ERROR}')"#);
    let p95 = r#"COALESCE(percentile_cont(0.95::float8) WITHIN GROUP (ORDER BY "durationMs"::float8), 0)"#;
    let avg = r#"COALESCE(AVG("durationMs"::float8), 0)"#;
    let max = r#"COALESCE(MAX("durationMs")::float8, 0)"#;

    let totals_sql = format!(
        r#"SELECT CAST(COUNT(*) AS BIGINT) AS calls,
                  CAST({errors} AS BIGINT) AS errors,
                  CAST(COALESCE(SUM({TOTAL_TOKENS_SQL}), 0) AS BIGINT) AS total_tokens,
                  CAST(COALESCE(SUM("promptTokens"), 0) AS BIGINT) AS prompt_tokens,
                  CAST(COALESCE(SUM("completionTokens"), 0) AS BIGINT) AS completion_tokens,
                  {avg} AS avg_duration_ms,
                  {p95} AS p95_duration_ms,
                  {max} AS max_duration_ms
           FROM "TelemetryLlmCall"
           WHERE {where_sql}"#
    );
    let totals = TotalsRow::find_by_statement(Statement::from_sql_and_values(
        backend,
        totals_sql,
        values.clone(),
    ))
    .one(db)
    .await?;

    let model_sql = format!(
        r#"SELECT "provider" AS provider,
                  "model" AS model,
                  CAST(COUNT(*) AS BIGINT) AS calls,
                  CAST({errors} AS BIGINT) AS errors,
                  CAST(COALESCE(SUM({TOTAL_TOKENS_SQL}), 0) AS BIGINT) AS total_tokens,
                  {avg} AS avg_duration_ms,
                  {p95} AS p95_duration_ms,
                  {max} AS max_duration_ms
           FROM "TelemetryLlmCall"
           WHERE {where_sql}
           GROUP BY "provider", "model"
           ORDER BY calls DESC, provider ASC, model ASC
           LIMIT {LLM_TOP_LIMIT}"#
    );
    let model_rows = ModelRow::find_by_statement(Statement::from_sql_and_values(
        backend,
        model_sql,
        values.clone(),
    ))
    .all(db)
    .await?;

    let provider_sql = format!(
        r#"SELECT "provider" AS provider,
                  CAST(COUNT(*) AS BIGINT) AS calls,
                  CAST({errors} AS BIGINT) AS errors,
                  CAST(COALESCE(SUM({TOTAL_TOKENS_SQL}), 0) AS BIGINT) AS total_tokens,
                  {avg} AS avg_duration_ms,
                  {p95} AS p95_duration_ms,
                  {max} AS max_duration_ms
           FROM "TelemetryLlmCall"
           WHERE {where_sql}
           GROUP BY "provider"
           ORDER BY calls DESC, provider ASC
           LIMIT {LLM_TOP_LIMIT}"#
    );
    let provider_rows = ProviderRow::find_by_statement(Statement::from_sql_and_values(
        backend,
        provider_sql,
        values.clone(),
    ))
    .all(db)
    .await?;

    let trend_sql = format!(
        r#"SELECT date_trunc('{bucket}', "createdAt") AS bucket,
                  CAST(COUNT(*) AS BIGINT) AS calls,
                  CAST({errors} AS BIGINT) AS errors,
                  {p95} AS p95_duration_ms,
                  {max} AS max_duration_ms
           FROM "TelemetryLlmCall"
           WHERE {where_sql}
           GROUP BY bucket
           ORDER BY bucket ASC"#
    );
    let trend_rows =
        LlmTrendRow::find_by_statement(Statement::from_sql_and_values(backend, trend_sql, values))
            .all(db)
            .await?;

    let trend_buckets = trend_rows
        .into_iter()
        .map(|row| {
            (
                row.bucket,
                TrendBucket {
                    calls: row.calls,
                    errors: row.errors,
                    p95_duration_ms: Some(round3(row.p95_duration_ms)),
                    max_duration_ms: round3(row.max_duration_ms),
                },
            )
        })
        .collect();

    Ok(LlmFold {
        totals: totals
            .map(|row| LlmTotals {
                calls: row.calls,
                errors: row.errors,
                error_rate: error_rate(row.errors, row.calls),
                total_tokens: row.total_tokens,
                prompt_tokens: row.prompt_tokens,
                completion_tokens: row.completion_tokens,
                avg_duration_ms: round3(row.avg_duration_ms),
                p95_duration_ms: Some(round3(row.p95_duration_ms)),
                max_duration_ms: round3(row.max_duration_ms),
            })
            .unwrap_or_else(empty_raw_totals),
        by_model: model_rows
            .into_iter()
            .map(|row| LlmModelStats {
                provider: row.provider,
                model: row.model,
                calls: row.calls,
                errors: row.errors,
                error_rate: error_rate(row.errors, row.calls),
                avg_duration_ms: round3(row.avg_duration_ms),
                p95_duration_ms: Some(round3(row.p95_duration_ms)),
                max_duration_ms: round3(row.max_duration_ms),
                total_tokens: row.total_tokens,
            })
            .collect(),
        by_provider: provider_rows
            .into_iter()
            .map(|row| LlmProviderStats {
                provider: row.provider,
                calls: row.calls,
                errors: row.errors,
                error_rate: error_rate(row.errors, row.calls),
                avg_duration_ms: round3(row.avg_duration_ms),
                p95_duration_ms: Some(round3(row.p95_duration_ms)),
                max_duration_ms: round3(row.max_duration_ms),
                total_tokens: row.total_tokens,
            })
            .collect(),
        by_operation: Vec::new(),
        top_errors: Vec::new(),
        trend: fill_trend(
            trend_buckets,
            filters.cutoff,
            now,
            bucket,
            empty_raw_bucket(),
        ),
    })
}

async fn llm_from_fold<C: ConnectionTrait>(
    db: &C,
    filters: &LlmFilters,
    now: NaiveDateTime,
    bucket: &str,
) -> Result<LlmFold, ApiError> {
    let rows = llm_samples_query(filters)
        .limit(LLM_ROW_CAP)
        .into_model::<LlmSampleRow>()
        .all(db)
        .await?;

    if rows.len() as u64 == LLM_ROW_CAP {
        tracing::warn!(
            cap = LLM_ROW_CAP,
            "LLM percentile fold hit the row cap; results are truncated"
        );
    }

    Ok(fold_llm_samples(rows, filters.cutoff, now, bucket))
}

fn llm_samples_query(filters: &LlmFilters) -> Select<telemetry_llm_call::Entity> {
    let mut select = telemetry_llm_call::Entity::find()
        .select_only()
        .column_as(telemetry_llm_call::Column::Provider, "provider")
        .column_as(telemetry_llm_call::Column::Model, "model")
        .column_as(telemetry_llm_call::Column::Operation, "operation")
        .column_as(telemetry_llm_call::Column::Status, "status")
        .column_as(telemetry_llm_call::Column::ErrorKind, "error_kind")
        .column_as(telemetry_llm_call::Column::DurationMs, "duration_ms")
        .column_as(telemetry_llm_call::Column::PromptTokens, "prompt_tokens")
        .column_as(
            telemetry_llm_call::Column::CompletionTokens,
            "completion_tokens",
        )
        .column_as(telemetry_llm_call::Column::TotalTokens, "total_tokens")
        .column_as(telemetry_llm_call::Column::CreatedAt, "created_at")
        .filter(telemetry_llm_call::Column::CreatedAt.gte(filters.cutoff));

    if let Some(value) = &filters.provider {
        select = select.filter(telemetry_llm_call::Column::Provider.eq(value));
    }
    if let Some(value) = &filters.model {
        select = select.filter(telemetry_llm_call::Column::Model.eq(value));
    }
    if let Some(value) = &filters.source {
        select = select.filter(telemetry_llm_call::Column::Source.eq(value));
    }
    select
}

/// `COUNT(CASE WHEN status = 'error' THEN id END)`, portable across backends.
fn error_count() -> SimpleExpr {
    use sea_orm::sea_query::ExprTrait;
    Expr::expr(
        Expr::case(
            Expr::col(telemetry_llm_call::Column::Status).eq(LLM_STATUS_ERROR),
            Expr::col(telemetry_llm_call::Column::Id),
        )
        .finally(sea_orm::Value::String(None)),
    )
    .count()
}

fn apply_llm_filters(
    mut select: Select<telemetry_llm_call::Entity>,
    filters: &LlmFilters,
) -> Select<telemetry_llm_call::Entity> {
    select = select.filter(telemetry_llm_call::Column::CreatedAt.gte(filters.cutoff));
    if let Some(value) = &filters.provider {
        select = select.filter(telemetry_llm_call::Column::Provider.eq(value));
    }
    if let Some(value) = &filters.model {
        select = select.filter(telemetry_llm_call::Column::Model.eq(value));
    }
    if let Some(value) = &filters.source {
        select = select.filter(telemetry_llm_call::Column::Source.eq(value));
    }
    select
}

fn operation_stats_query(filters: &LlmFilters) -> Select<telemetry_llm_call::Entity> {
    use sea_orm::sea_query::ExprTrait;
    apply_llm_filters(
        telemetry_llm_call::Entity::find()
            .select_only()
            .column_as(telemetry_llm_call::Column::Operation, "operation")
            .column_as(Expr::col(telemetry_llm_call::Column::Id).count(), "calls")
            .column_as(error_count(), "errors")
            .group_by(telemetry_llm_call::Column::Operation)
            .order_by_desc(Expr::col(telemetry_llm_call::Column::Id).count())
            .limit(LLM_TOP_LIMIT as u64),
        filters,
    )
}

fn error_kind_stats_query(filters: &LlmFilters) -> Select<telemetry_llm_call::Entity> {
    use sea_orm::sea_query::ExprTrait;
    apply_llm_filters(
        telemetry_llm_call::Entity::find()
            .select_only()
            .column_as(telemetry_llm_call::Column::ErrorKind, "error_kind")
            .column_as(Expr::col(telemetry_llm_call::Column::Id).count(), "count")
            .filter(telemetry_llm_call::Column::Status.eq(LLM_STATUS_ERROR))
            .filter(telemetry_llm_call::Column::ErrorKind.is_not_null())
            .group_by(telemetry_llm_call::Column::ErrorKind)
            .order_by_desc(Expr::col(telemetry_llm_call::Column::Id).count())
            .limit(LLM_TOP_LIMIT as u64),
        filters,
    )
}

/// Operation and error-kind breakdowns have no rollup dimension, so they are
/// grouped over the raw calls. These are aggregates, not row fetches: they are
/// bounded by retention, never silently truncated.
async fn raw_breakdowns<C: ConnectionTrait>(
    db: &C,
    filters: &LlmFilters,
) -> Result<(Vec<LlmOperationStats>, Vec<LlmErrorKindStats>), ApiError> {
    let operations = operation_stats_query(filters)
        .into_model::<OperationRow>()
        .all(db)
        .await?
        .into_iter()
        .map(|row| LlmOperationStats {
            operation: row.operation,
            calls: row.calls,
            error_rate: error_rate(row.errors, row.calls),
        })
        .collect();

    let error_kinds = error_kind_stats_query(filters)
        .into_model::<ErrorKindRow>()
        .all(db)
        .await?
        .into_iter()
        .map(|row| LlmErrorKindStats {
            error_kind: row.error_kind,
            count: row.count,
        })
        .collect();

    Ok((rank_operations(operations), rank_error_kinds(error_kinds)))
}

fn daily_llm_query(
    start: NaiveDateTime,
    end: NaiveDateTime,
    filters: &LlmFilters,
) -> Select<telemetry_llm_daily::Entity> {
    let mut select = telemetry_llm_daily::Entity::find()
        .filter(telemetry_llm_daily::Column::Day.gte(start))
        .filter(telemetry_llm_daily::Column::Day.lte(end))
        .order_by_asc(telemetry_llm_daily::Column::Day);

    if let Some(value) = &filters.provider {
        select = select.filter(telemetry_llm_daily::Column::Provider.eq(value));
    }
    if let Some(value) = &filters.model {
        select = select.filter(telemetry_llm_daily::Column::Model.eq(value));
    }
    if let Some(value) = &filters.source {
        select = select.filter(telemetry_llm_daily::Column::Source.eq(value));
    }
    select
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/llm",
    tag = "admin",
    params(TelemetryLlmQuery),
    responses(
        (status = 200, description = "LLM call volume, failure rate, token spend and latency", body = TelemetryLlmResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Aggregate LLM usage across the platform: call volume, error rate, token spend and latency, broken down by model, provider and kind of call, with the most frequent failure kinds and a trend over time. Windows of up to 48 hours are computed over individual calls (granularity \"raw\") and include the p95 latency. Longer windows read the daily rollup (granularity \"daily\"): call counts, errors and token spend stay exact, the mean latency is derived from the stored duration sum and \"maxDurationMs\" is the true maximum, but \"p95DurationMs\" is null because percentiles cannot be recomputed from daily aggregates. The per-operation and per-error-kind breakdowns are never rolled up and only cover the days raw calls are still kept for, reported as \"breakdownWindowHours\". Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/llm", skip_all)]
pub async fn telemetry_llm(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<TelemetryLlmQuery>,
) -> Result<Json<TelemetryLlmResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let hours = q.hours.unwrap_or(DEFAULT_LLM_HOURS).clamp(1, MAX_LLM_HOURS);
    let bucket = window_bucket(hours, None);
    let now = Utc::now().naive_utc();
    let (raw_cutoff, breakdown_window_hours) = raw_window(now, hours, llm_retention_hours());
    let filters = LlmFilters::new(&q, raw_cutoff);

    if reads_raw(hours) {
        let fold = match state.db.get_database_backend() {
            DbBackend::Postgres => {
                let mut fold = llm_from_sql(&state.db, &filters, now, bucket).await?;
                let (by_operation, top_errors) = raw_breakdowns(&state.db, &filters).await?;
                fold.by_operation = by_operation;
                fold.top_errors = top_errors;
                fold
            }
            _ => llm_from_fold(&state.db, &filters, now, bucket).await?,
        };

        return Ok(Json(TelemetryLlmResponse {
            hours,
            granularity: GRANULARITY_RAW.to_string(),
            totals: fold.totals,
            by_model: fold.by_model,
            by_provider: fold.by_provider,
            by_operation: fold.by_operation,
            top_errors: fold.top_errors,
            trend: fold.trend,
            breakdown_window_hours,
        }));
    }

    let (start, end) = day_window(now, hours);
    let rows = daily_llm_query(start, end, &filters).all(&state.db).await?;
    let fold = fold_llm_daily(&rows, start, end);
    let (by_operation, top_errors) = raw_breakdowns(&state.db, &filters).await?;

    Ok(Json(TelemetryLlmResponse {
        hours,
        granularity: GRANULARITY_DAILY.to_string(),
        totals: fold.totals,
        by_model: fold.by_model,
        by_provider: fold.by_provider,
        by_operation,
        top_errors,
        trend: fold.trend,
        breakdown_window_hours,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use sea_orm::QueryTrait;

    fn ts(hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 7, 26)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn sample(model: &str, duration_ms: i32, status: &str, hour: u32) -> LlmSampleRow {
        LlmSampleRow {
            provider: "openai".to_string(),
            model: model.to_string(),
            operation: "chat".to_string(),
            status: status.to_string(),
            error_kind: None,
            duration_ms,
            prompt_tokens: Some(100),
            completion_tokens: Some(20),
            total_tokens: None,
            created_at: ts(hour, 30),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn daily(
        y: i32,
        m: u32,
        d: u32,
        provider: &str,
        model: &str,
        calls: i32,
        errors: i32,
        duration_sum_ms: i64,
        duration_max_ms: i32,
    ) -> telemetry_llm_daily::Model {
        telemetry_llm_daily::Model {
            id: format!("{y}-{m}-{d}-{provider}-{model}"),
            day: day(y, m, d),
            provider: provider.to_string(),
            model: model.to_string(),
            source: "desktop".to_string(),
            calls,
            errors,
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            duration_sum_ms,
            duration_max_ms,
            created_at: day(y, m, d),
            updated_at: day(y, m, d),
        }
    }

    #[test]
    fn the_window_decides_between_raw_calls_and_the_daily_rollup() {
        assert!(reads_raw(47));
        assert!(reads_raw(48));
        assert!(!reads_raw(49));
        assert_eq!(window_bucket(48, None), "hour");
        assert_eq!(window_bucket(49, None), "day");
    }

    #[test]
    fn a_raw_read_is_clipped_to_the_call_retention() {
        let now = ts(12, 0);

        assert_eq!(
            raw_window(now, 24, 24 * 30),
            (now - Duration::hours(24), 24)
        );
        assert_eq!(
            raw_window(now, 24 * 90, 24 * 30),
            (now - Duration::hours(24 * 30), 24 * 30)
        );
    }

    #[test]
    fn error_rates_never_divide_by_zero() {
        assert_eq!(error_rate(0, 0), 0.0);
        assert!(error_rate(0, 0).is_finite());
        assert_eq!(error_rate(5, 0), 0.0);
        assert_eq!(error_rate(0, 4), 0.0);
        assert_eq!(error_rate(4, 4), 1.0);
        assert_eq!(error_rate(1, 3), 0.333);
    }

    #[test]
    fn an_empty_window_yields_zeroed_totals_and_no_nan() {
        let fold = fold_llm_samples(vec![], ts(10, 0), ts(11, 0), "hour");

        assert_eq!(fold.totals, empty_raw_totals());
        assert_eq!(fold.totals.error_rate, 0.0);
        assert_eq!(fold.totals.avg_duration_ms, 0.0);
        assert_eq!(fold.totals.p95_duration_ms, Some(0.0));
        assert_eq!(fold.totals.max_duration_ms, 0.0);
        assert!(fold.by_model.is_empty());
        assert!(fold.by_provider.is_empty());
        assert!(fold.by_operation.is_empty());
        assert!(fold.top_errors.is_empty());
        assert_eq!(fold.trend.len(), 2);
        assert_eq!(fold.trend[0].calls, 0);
        assert_eq!(fold.trend[0].p95_duration_ms, Some(0.0));
    }

    #[test]
    fn percentiles_and_averages_fold_per_model() {
        let rows = vec![
            sample("gpt-5-mini", 100, "ok", 10),
            sample("gpt-5-mini", 200, "ok", 10),
            sample("gpt-5-mini", 300, "error", 10),
            sample("gpt-5", 1000, "ok", 11),
        ];

        let fold = fold_llm_samples(rows, ts(10, 0), ts(11, 0), "hour");

        assert_eq!(fold.totals.calls, 4);
        assert_eq!(fold.totals.errors, 1);
        assert_eq!(fold.totals.error_rate, 0.25);
        assert_eq!(fold.totals.prompt_tokens, 400);
        assert_eq!(fold.totals.completion_tokens, 80);
        assert_eq!(fold.totals.total_tokens, 480);
        assert_eq!(fold.totals.avg_duration_ms, 400.0);
        assert_eq!(fold.totals.p95_duration_ms, Some(895.0));
        assert_eq!(fold.totals.max_duration_ms, 1000.0);

        assert_eq!(fold.by_model.len(), 2);
        assert_eq!(fold.by_model[0].model, "gpt-5-mini");
        assert_eq!(fold.by_model[0].calls, 3);
        assert_eq!(fold.by_model[0].error_rate, 0.333);
        assert_eq!(fold.by_model[0].avg_duration_ms, 200.0);
        assert_eq!(fold.by_model[0].p95_duration_ms, Some(290.0));
        assert_eq!(fold.by_model[0].max_duration_ms, 300.0);
        assert_eq!(fold.by_model[0].total_tokens, 360);
        assert_eq!(fold.by_model[1].model, "gpt-5");

        assert_eq!(fold.by_provider.len(), 1);
        assert_eq!(fold.by_provider[0].provider, "openai");
        assert_eq!(fold.by_provider[0].calls, 4);

        assert_eq!(
            fold.by_operation,
            vec![LlmOperationStats {
                operation: "chat".to_string(),
                calls: 4,
                error_rate: 0.25,
            }]
        );
    }

    #[test]
    fn an_explicit_total_wins_over_the_derived_sum() {
        assert_eq!(effective_total_tokens(Some(10), Some(5), Some(99)), 99);
        assert_eq!(effective_total_tokens(Some(10), Some(5), None), 15);
        assert_eq!(effective_total_tokens(Some(10), None, None), 10);
        assert_eq!(effective_total_tokens(None, None, None), 0);
    }

    #[test]
    fn error_kinds_are_counted_only_for_failed_calls() {
        let rows = vec![
            LlmSampleRow {
                error_kind: Some("rate_limit".to_string()),
                ..sample("gpt-5", 10, LLM_STATUS_ERROR, 10)
            },
            LlmSampleRow {
                error_kind: Some("rate_limit".to_string()),
                ..sample("gpt-5", 10, LLM_STATUS_ERROR, 10)
            },
            LlmSampleRow {
                error_kind: Some("timeout".to_string()),
                ..sample("gpt-5", 10, LLM_STATUS_ERROR, 10)
            },
            LlmSampleRow {
                error_kind: Some("ignored".to_string()),
                ..sample("gpt-5", 10, "ok", 10)
            },
        ];

        let fold = fold_llm_samples(rows, ts(10, 0), ts(10, 30), "hour");

        assert_eq!(
            fold.top_errors,
            vec![
                LlmErrorKindStats {
                    error_kind: "rate_limit".to_string(),
                    count: 2,
                },
                LlmErrorKindStats {
                    error_kind: "timeout".to_string(),
                    count: 1,
                },
            ]
        );
    }

    #[test]
    fn the_trend_is_zero_filled_across_the_window() {
        let rows = vec![
            sample("gpt-5", 100, "ok", 10),
            sample("gpt-5", 300, "error", 12),
        ];

        let fold = fold_llm_samples(rows, ts(10, 0), ts(12, 0), "hour");

        assert_eq!(
            fold.trend,
            vec![
                LlmTrendPoint {
                    ts: "2026-07-26T10:00:00+00:00".to_string(),
                    calls: 1,
                    errors: 0,
                    p95_duration_ms: Some(100.0),
                    max_duration_ms: 100.0,
                },
                LlmTrendPoint {
                    ts: "2026-07-26T11:00:00+00:00".to_string(),
                    calls: 0,
                    errors: 0,
                    p95_duration_ms: Some(0.0),
                    max_duration_ms: 0.0,
                },
                LlmTrendPoint {
                    ts: "2026-07-26T12:00:00+00:00".to_string(),
                    calls: 1,
                    errors: 1,
                    p95_duration_ms: Some(300.0),
                    max_duration_ms: 300.0,
                },
            ]
        );
    }

    #[test]
    fn breakdowns_are_capped_at_the_top_limit() {
        let rows: Vec<LlmSampleRow> = (0..30)
            .flat_map(|i| {
                (0..=i).map(move |_| {
                    let mut row = sample(&format!("model-{i:02}"), 10, "ok", 10);
                    row.provider = format!("provider-{i:02}");
                    row.operation = format!("operation-{i:02}");
                    row.error_kind = Some(format!("kind-{i:02}"));
                    row.status = LLM_STATUS_ERROR.to_string();
                    row
                })
            })
            .collect();

        let fold = fold_llm_samples(rows, ts(10, 0), ts(10, 30), "hour");

        assert_eq!(fold.by_model.len(), LLM_TOP_LIMIT);
        assert_eq!(fold.by_model[0].model, "model-29");
        assert_eq!(fold.by_provider.len(), LLM_TOP_LIMIT);
        assert_eq!(fold.by_provider[0].provider, "provider-29");
        assert_eq!(fold.by_operation.len(), LLM_TOP_LIMIT);
        assert_eq!(fold.by_operation[0].operation, "operation-29");
        assert_eq!(fold.top_errors.len(), LLM_TOP_LIMIT);
        assert_eq!(fold.top_errors[0].error_kind, "kind-29");
    }

    #[test]
    fn the_daily_rollup_sums_exactly_and_reports_no_percentile() {
        let rows = vec![
            daily(2026, 7, 25, "openai", "gpt-5", 10, 1, 5_000, 900),
            daily(2026, 7, 26, "openai", "gpt-5", 30, 3, 15_000, 1_500),
            daily(2026, 7, 26, "anthropic", "claude", 20, 0, 40_000, 4_000),
        ];

        let fold = fold_llm_daily(&rows, day(2026, 7, 25), day(2026, 7, 26));

        assert_eq!(fold.totals.calls, 60);
        assert_eq!(fold.totals.errors, 4);
        assert_eq!(fold.totals.error_rate, 0.067);
        assert_eq!(fold.totals.total_tokens, 360);
        assert_eq!(fold.totals.avg_duration_ms, 1000.0);
        assert_eq!(fold.totals.p95_duration_ms, None);
        assert_eq!(fold.totals.max_duration_ms, 4000.0);

        assert_eq!(fold.by_model.len(), 2);
        assert_eq!(fold.by_model[0].model, "gpt-5");
        assert_eq!(fold.by_model[0].calls, 40);
        assert_eq!(fold.by_model[0].avg_duration_ms, 500.0);
        assert_eq!(fold.by_model[0].max_duration_ms, 1500.0);
        assert_eq!(fold.by_model[0].p95_duration_ms, None);

        assert_eq!(fold.by_provider.len(), 2);
        assert_eq!(fold.by_provider[0].provider, "openai");
        assert_eq!(fold.by_provider[0].calls, 40);

        assert!(fold.by_operation.is_empty());
        assert!(fold.top_errors.is_empty());
    }

    #[test]
    fn the_daily_trend_is_zero_filled_and_never_invents_a_percentile() {
        let rows = vec![daily(2026, 7, 26, "openai", "gpt-5", 4, 1, 800, 400)];
        let fold = fold_llm_daily(&rows, day(2026, 7, 25), day(2026, 7, 27));

        assert_eq!(
            fold.trend,
            vec![
                LlmTrendPoint {
                    ts: "2026-07-25T00:00:00+00:00".to_string(),
                    calls: 0,
                    errors: 0,
                    p95_duration_ms: None,
                    max_duration_ms: 0.0,
                },
                LlmTrendPoint {
                    ts: "2026-07-26T00:00:00+00:00".to_string(),
                    calls: 4,
                    errors: 1,
                    p95_duration_ms: None,
                    max_duration_ms: 400.0,
                },
                LlmTrendPoint {
                    ts: "2026-07-27T00:00:00+00:00".to_string(),
                    calls: 0,
                    errors: 0,
                    p95_duration_ms: None,
                    max_duration_ms: 0.0,
                },
            ]
        );
    }

    #[test]
    fn an_empty_rollup_window_never_divides_by_zero() {
        let fold = fold_llm_daily(&[], day(2026, 7, 26), day(2026, 7, 26));

        assert_eq!(fold.totals, LlmTotals::default());
        assert_eq!(fold.totals.avg_duration_ms, 0.0);
        assert_eq!(fold.totals.p95_duration_ms, None);
        assert_eq!(fold.trend.len(), 1);
    }

    #[test]
    fn the_daily_query_keeps_the_filters_and_the_day_range() {
        let filters = LlmFilters::new(
            &TelemetryLlmQuery {
                hours: None,
                provider: Some("OpenAI".to_string()),
                model: Some("gpt-5".to_string()),
                source: Some("web".to_string()),
            },
            ts(10, 0),
        );
        let sql = daily_llm_query(day(2026, 7, 20), day(2026, 7, 27), &filters)
            .build(DbBackend::Postgres)
            .to_string();

        assert!(sql.contains(r#""TelemetryLlmDaily""#), "{sql}");
        assert!(sql.contains(r#""provider" = 'openai'"#), "{sql}");
        assert!(sql.contains(r#""model" = 'gpt-5'"#), "{sql}");
        assert!(sql.contains(r#""source" = 'web'"#), "{sql}");
    }

    #[test]
    fn the_operation_breakdown_counts_errors_in_the_same_grouped_query() {
        let filters = LlmFilters::new(
            &TelemetryLlmQuery {
                hours: None,
                provider: None,
                model: None,
                source: None,
            },
            ts(10, 0),
        );
        let sql = operation_stats_query(&filters)
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains(
                r#"COUNT((CASE WHEN ("status" = 'error') THEN "id" ELSE NULL END)) AS "errors""#
            ),
            "{sql}"
        );
        assert!(
            sql.contains(r#"GROUP BY "TelemetryLlmCall"."operation""#),
            "{sql}"
        );
        assert!(sql.contains("LIMIT 20"), "{sql}");
    }

    #[test]
    fn the_error_kind_breakdown_only_counts_failed_calls() {
        let filters = LlmFilters::new(
            &TelemetryLlmQuery {
                hours: None,
                provider: None,
                model: None,
                source: None,
            },
            ts(10, 0),
        );
        let sql = error_kind_stats_query(&filters)
            .build(DbBackend::Postgres)
            .to_string();

        assert!(sql.contains(r#""status" = 'error'"#), "{sql}");
        assert!(sql.contains(r#""errorKind" IS NOT NULL"#), "{sql}");
        assert!(
            sql.contains(r#"GROUP BY "TelemetryLlmCall"."errorKind""#),
            "{sql}"
        );
    }

    #[test]
    fn filter_values_are_bound_parameters_and_never_reach_the_sql_text() {
        let filters = LlmFilters::new(
            &TelemetryLlmQuery {
                hours: None,
                provider: Some("OpenAI'; DROP TABLE \"TelemetryLlmCall\"; --".to_string()),
                model: Some("gpt-5' OR '1'='1".to_string()),
                source: Some("web\"; DELETE FROM \"TelemetryEvent\"; --".to_string()),
            },
            ts(10, 0),
        );
        let (sql, values) = llm_where(&filters);

        assert_eq!(values.len(), 4);
        assert_eq!(
            sql,
            r#""createdAt" >= $1 AND "provider" = $2 AND "model" = $3 AND "source" = $4"#
        );
        assert!(!sql.contains("DROP"), "{sql}");
        assert!(!sql.contains("DELETE"), "{sql}");
        assert!(!sql.contains('\''), "{sql}");
    }

    #[test]
    fn omitted_filters_do_not_shift_placeholders() {
        let filters = LlmFilters::new(
            &TelemetryLlmQuery {
                hours: None,
                provider: None,
                model: Some("gpt-5".to_string()),
                source: None,
            },
            ts(10, 0),
        );
        let (sql, values) = llm_where(&filters);

        assert_eq!(values.len(), 2);
        assert_eq!(sql, r#""createdAt" >= $1 AND "model" = $2"#);
    }

    #[test]
    fn empty_filters_are_treated_as_absent_and_providers_are_lowercased() {
        let filters = LlmFilters::new(
            &TelemetryLlmQuery {
                hours: None,
                provider: Some("OpenAI".to_string()),
                model: Some(String::new()),
                source: Some(String::new()),
            },
            ts(10, 0),
        );

        assert_eq!(filters.provider.as_deref(), Some("openai"));
        assert_eq!(filters.model, None);
        assert_eq!(filters.source, None);
    }
}
