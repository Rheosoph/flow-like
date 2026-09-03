//! Release health: crash-free session and install rates, adoption and trend.
//!
//! Windows of at most 48 hours aggregate the raw `TelemetrySession` rows.
//! Longer windows — including the fixed 30 day window behind the release list —
//! read `TelemetrySessionDaily`, which stores sessions, crashed sessions,
//! installs and crashed installs per (day, release, source). Both crash-free
//! rates stay derivable from those columns, so the numbers do not change shape
//! across the boundary.
//!
//! Error counts have no rollup and are always read from the raw error events;
//! they are therefore bounded by the error retention window.

use super::bucket_slots;
use super::overview::{day_window, granularity_for, reads_raw, sum_i64, window_bucket};
use crate::entity::{
    telemetry_error_event, telemetry_release, telemetry_session, telemetry_session_daily,
};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use sea_orm::sea_query::ExprTrait;
use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Select, Statement,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};

const CRASHED_STATUS: &str = "crashed";
/// Window used by the release list, which has no explicit range parameter.
const RELEASES_WINDOW_HOURS: i64 = 24 * 30;
const RELEASES_DEFAULT_LIMIT: u64 = 20;
const RELEASES_MAX_LIMIT: u64 = 50;
/// Releases shown alongside the aggregated release-health numbers.
const RELEASE_HEALTH_TOP: usize = 20;
const RELEASE_META_CAP: u64 = 200;

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListTelemetryReleasesQuery {
    /// Filter by source: "desktop", "web", "desktop_native" or "backend".
    #[serde(default)]
    pub source: Option<String>,
    /// Maximum number of releases. Default 20, capped at 50.
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct TelemetryReleaseHealthQuery {
    /// Lookback window in hours. Default 168 (7 days).
    #[serde(default)]
    pub hours: Option<i64>,
    /// Filter by source: "desktop", "web", "desktop_native" or "backend".
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryReleaseRow {
    pub version: String,
    pub source: String,
    pub commit_sha: Option<String>,
    /// ISO-8601 timestamp, null while the release has not been registered.
    pub first_seen_at: Option<String>,
    pub installs: i64,
    pub sessions: i64,
    pub crashed_sessions: i64,
    /// 1 - crashed sessions / sessions, null without sessions in the window.
    pub crash_free_session_rate: Option<f64>,
    /// 1 - installs with a crash / installs, null without sessions in the window.
    pub crash_free_install_rate: Option<f64>,
    pub error_count: i64,
    /// Share of all installs in the window that ran this release.
    pub adoption: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTelemetryReleasesResponse {
    /// Always "daily": the release list covers a fixed 30 day window.
    pub granularity: String,
    pub releases: Vec<TelemetryReleaseRow>,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseHealthTrendPoint {
    /// ISO-8601 timestamp at the start of the bucket.
    pub ts: String,
    pub sessions: i64,
    pub crashed_sessions: i64,
    pub crash_free_session_rate: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryReleaseHealthResponse {
    pub hours: i64,
    /// "raw" when the rates come from individual sessions, "daily" when they
    /// come from the daily session rollup.
    pub granularity: String,
    pub total_sessions: i64,
    pub crashed_sessions: i64,
    pub crash_free_session_rate: Option<f64>,
    pub crash_free_install_rate: Option<f64>,
    pub total_installs: i64,
    pub trend: Vec<ReleaseHealthTrendPoint>,
    pub releases: Vec<TelemetryReleaseRow>,
}

#[derive(Debug, Default, FromQueryResult)]
struct SessionTotalsRow {
    sessions: i64,
    crashed_sessions: i64,
    installs: i64,
    crashed_installs: i64,
}

/// `SUM` over an empty day range yields NULL, which never decodes into `i64`.
#[derive(Debug, Default, FromQueryResult)]
struct DailySessionTotalsRow {
    sessions: Option<i64>,
    crashed_sessions: Option<i64>,
    installs: Option<i64>,
    crashed_installs: Option<i64>,
}

impl From<DailySessionTotalsRow> for SessionTotalsRow {
    fn from(row: DailySessionTotalsRow) -> Self {
        Self {
            sessions: row.sessions.unwrap_or(0),
            crashed_sessions: row.crashed_sessions.unwrap_or(0),
            installs: row.installs.unwrap_or(0),
            crashed_installs: row.crashed_installs.unwrap_or(0),
        }
    }
}

#[derive(Debug, FromQueryResult)]
struct ReleaseSessionRow {
    release: Option<String>,
    source: String,
    sessions: i64,
    crashed_sessions: i64,
    installs: i64,
    crashed_installs: i64,
}

#[derive(Debug, FromQueryResult)]
struct ReleaseErrorRow {
    release: Option<String>,
    source: String,
    cnt: i64,
}

#[derive(Debug, FromQueryResult)]
struct TrendRow {
    bucket: NaiveDateTime,
    sessions: i64,
    crashed_sessions: i64,
}

#[derive(Debug, Clone)]
struct ReleaseMeta {
    version: String,
    source: String,
    commit_sha: Option<String>,
    first_seen_at: NaiveDateTime,
}

#[derive(Debug, Default)]
struct ReleaseAggregate {
    sessions: i64,
    crashed_sessions: i64,
    installs: i64,
    crashed_installs: i64,
    error_count: i64,
    commit_sha: Option<String>,
    first_seen_at: Option<NaiveDateTime>,
}

/// Which store answers a request, resolved once from the requested window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionWindow {
    Raw {
        cutoff: NaiveDateTime,
    },
    Daily {
        start: NaiveDateTime,
        end: NaiveDateTime,
    },
}

impl SessionWindow {
    fn new(now: NaiveDateTime, hours: i64) -> Self {
        if reads_raw(hours) {
            Self::Raw {
                cutoff: now - Duration::hours(hours),
            }
        } else {
            let (start, end) = day_window(now, hours);
            Self::Daily { start, end }
        }
    }

    /// Cutoff for the raw error events, which have no rollup of their own.
    fn error_cutoff(&self) -> NaiveDateTime {
        match self {
            Self::Raw { cutoff } => *cutoff,
            Self::Daily { start, .. } => *start,
        }
    }
}

fn ratio(part: i64, total: i64) -> Option<f64> {
    if total <= 0 {
        return None;
    }
    Some((part as f64 / total as f64).clamp(0.0, 1.0))
}

fn crash_free(total: i64, crashed: i64) -> Option<f64> {
    ratio(total - crashed, total)
}

/// `COUNT(CASE WHEN status = 'crashed' THEN <column> END)`, portable across backends.
fn crashed_count(column: telemetry_session::Column, distinct: bool) -> SimpleExpr {
    let case = Expr::case(
        Expr::col(telemetry_session::Column::Status).eq(CRASHED_STATUS),
        Expr::col(column),
    )
    .finally(sea_orm::Value::String(None));
    if distinct {
        Expr::expr(case).count_distinct()
    } else {
        Expr::expr(case).count()
    }
}

fn session_totals_query(
    cutoff: NaiveDateTime,
    source: Option<&str>,
) -> Select<telemetry_session::Entity> {
    let mut select = telemetry_session::Entity::find()
        .select_only()
        .column_as(Expr::col(telemetry_session::Column::Id).count(), "sessions")
        .column_as(
            crashed_count(telemetry_session::Column::Id, false),
            "crashed_sessions",
        )
        .column_as(
            Expr::col(telemetry_session::Column::AnonId).count_distinct(),
            "installs",
        )
        .column_as(
            crashed_count(telemetry_session::Column::AnonId, true),
            "crashed_installs",
        )
        .filter(telemetry_session::Column::StartedAt.gte(cutoff));

    if let Some(source) = source {
        select = select.filter(telemetry_session::Column::Source.eq(source));
    }
    select
}

fn daily_session_totals_query(
    start: NaiveDateTime,
    end: NaiveDateTime,
    source: Option<&str>,
) -> Select<telemetry_session_daily::Entity> {
    let mut select = telemetry_session_daily::Entity::find()
        .select_only()
        .column_as(
            sum_i64(telemetry_session_daily::Column::Sessions),
            "sessions",
        )
        .column_as(
            sum_i64(telemetry_session_daily::Column::CrashedSessions),
            "crashed_sessions",
        )
        .column_as(
            sum_i64(telemetry_session_daily::Column::Installs),
            "installs",
        )
        .column_as(
            sum_i64(telemetry_session_daily::Column::CrashedInstalls),
            "crashed_installs",
        )
        .filter(telemetry_session_daily::Column::Day.gte(start))
        .filter(telemetry_session_daily::Column::Day.lte(end));

    if let Some(source) = source {
        select = select.filter(telemetry_session_daily::Column::Source.eq(source));
    }
    select
}

async fn session_totals<C: ConnectionTrait>(
    db: &C,
    window: SessionWindow,
    source: Option<&str>,
) -> Result<SessionTotalsRow, ApiError> {
    match window {
        SessionWindow::Raw { cutoff } => Ok(session_totals_query(cutoff, source)
            .into_model::<SessionTotalsRow>()
            .one(db)
            .await?
            .unwrap_or_default()),
        SessionWindow::Daily { start, end } => Ok(daily_session_totals_query(start, end, source)
            .into_model::<DailySessionTotalsRow>()
            .one(db)
            .await?
            .unwrap_or_default()
            .into()),
    }
}

fn release_sessions_query(
    cutoff: NaiveDateTime,
    source: Option<&str>,
) -> Select<telemetry_session::Entity> {
    let mut select = telemetry_session::Entity::find()
        .select_only()
        .column_as(telemetry_session::Column::Release, "release")
        .column_as(telemetry_session::Column::Source, "source")
        .column_as(Expr::col(telemetry_session::Column::Id).count(), "sessions")
        .column_as(
            crashed_count(telemetry_session::Column::Id, false),
            "crashed_sessions",
        )
        .column_as(
            Expr::col(telemetry_session::Column::AnonId).count_distinct(),
            "installs",
        )
        .column_as(
            crashed_count(telemetry_session::Column::AnonId, true),
            "crashed_installs",
        )
        .filter(telemetry_session::Column::StartedAt.gte(cutoff))
        .filter(telemetry_session::Column::Release.is_not_null())
        .group_by(telemetry_session::Column::Release)
        .group_by(telemetry_session::Column::Source);

    if let Some(source) = source {
        select = select.filter(telemetry_session::Column::Source.eq(source));
    }
    select
}

fn daily_release_sessions_query(
    start: NaiveDateTime,
    end: NaiveDateTime,
    source: Option<&str>,
) -> Select<telemetry_session_daily::Entity> {
    let mut select = telemetry_session_daily::Entity::find()
        .select_only()
        .column_as(telemetry_session_daily::Column::Release, "release")
        .column_as(telemetry_session_daily::Column::Source, "source")
        .column_as(
            sum_i64(telemetry_session_daily::Column::Sessions),
            "sessions",
        )
        .column_as(
            sum_i64(telemetry_session_daily::Column::CrashedSessions),
            "crashed_sessions",
        )
        .column_as(
            sum_i64(telemetry_session_daily::Column::Installs),
            "installs",
        )
        .column_as(
            sum_i64(telemetry_session_daily::Column::CrashedInstalls),
            "crashed_installs",
        )
        .filter(telemetry_session_daily::Column::Day.gte(start))
        .filter(telemetry_session_daily::Column::Day.lte(end))
        .group_by(telemetry_session_daily::Column::Release)
        .group_by(telemetry_session_daily::Column::Source);

    if let Some(source) = source {
        select = select.filter(telemetry_session_daily::Column::Source.eq(source));
    }
    select
}

async fn release_sessions<C: ConnectionTrait>(
    db: &C,
    window: SessionWindow,
    source: Option<&str>,
) -> Result<Vec<ReleaseSessionRow>, ApiError> {
    match window {
        SessionWindow::Raw { cutoff } => Ok(release_sessions_query(cutoff, source)
            .into_model::<ReleaseSessionRow>()
            .all(db)
            .await?),
        SessionWindow::Daily { start, end } => Ok(daily_release_sessions_query(start, end, source)
            .into_model::<ReleaseSessionRow>()
            .all(db)
            .await?),
    }
}

fn release_errors_query(
    cutoff: NaiveDateTime,
    source: Option<&str>,
) -> Select<telemetry_error_event::Entity> {
    let mut select = telemetry_error_event::Entity::find()
        .select_only()
        .column_as(telemetry_error_event::Column::Release, "release")
        .column_as(telemetry_error_event::Column::Source, "source")
        .column_as(Expr::col(telemetry_error_event::Column::Id).count(), "cnt")
        .filter(telemetry_error_event::Column::CreatedAt.gte(cutoff))
        .filter(telemetry_error_event::Column::Release.is_not_null())
        .group_by(telemetry_error_event::Column::Release)
        .group_by(telemetry_error_event::Column::Source);

    if let Some(source) = source {
        select = select.filter(telemetry_error_event::Column::Source.eq(source));
    }
    select
}

async fn release_errors<C: ConnectionTrait>(
    db: &C,
    cutoff: NaiveDateTime,
    source: Option<&str>,
) -> Result<Vec<ReleaseErrorRow>, ApiError> {
    Ok(release_errors_query(cutoff, source)
        .into_model::<ReleaseErrorRow>()
        .all(db)
        .await?)
}

async fn release_meta<C: ConnectionTrait>(
    db: &C,
    source: Option<&str>,
) -> Result<Vec<ReleaseMeta>, ApiError> {
    let mut select = telemetry_release::Entity::find();
    if let Some(source) = source {
        select = select.filter(telemetry_release::Column::Source.eq(source));
    }

    let models = select
        .order_by_desc(telemetry_release::Column::FirstSeenAt)
        .limit(RELEASE_META_CAP)
        .all(db)
        .await?;

    Ok(models
        .into_iter()
        .map(|model| ReleaseMeta {
            version: model.version,
            source: model.source,
            commit_sha: model.commit_sha,
            first_seen_at: model.first_seen_at,
        })
        .collect())
}

async fn raw_trend<C: ConnectionTrait>(
    db: &C,
    cutoff: NaiveDateTime,
    now: NaiveDateTime,
    bucket: &str,
    source: Option<&str>,
) -> Result<Vec<ReleaseHealthTrendPoint>, ApiError> {
    let backend = db.get_database_backend();
    let mut values: Vec<sea_orm::Value> = vec![cutoff.into()];
    if let Some(source) = source {
        values.push(source.into());
    }

    let sql = match backend {
        DbBackend::Postgres => {
            let mut conditions = r#""startedAt" >= $1"#.to_string();
            if source.is_some() {
                conditions.push_str(r#" AND "source" = $2"#);
            }
            format!(
                r#"SELECT date_trunc('{bucket}', "startedAt") AS bucket,
                          COUNT(*) AS sessions,
                          COUNT(*) FILTER (WHERE "status" = '{CRASHED_STATUS}') AS crashed_sessions
                   FROM "TelemetrySession"
                   WHERE {conditions}
                   GROUP BY bucket
                   ORDER BY bucket ASC"#,
            )
        }
        _ => {
            let mut conditions = "started_at >= $1".to_string();
            if source.is_some() {
                conditions.push_str(" AND source = $2");
            }
            format!(
                r#"SELECT date_trunc('{bucket}', started_at) AS bucket,
                          COUNT(*) AS sessions,
                          COUNT(CASE WHEN status = '{CRASHED_STATUS}' THEN id ELSE NULL END) AS crashed_sessions
                   FROM telemetry_session
                   WHERE {conditions}
                   GROUP BY bucket
                   ORDER BY bucket ASC"#,
            )
        }
    };

    let stmt = Statement::from_sql_and_values(backend, sql, values);
    let rows = TrendRow::find_by_statement(stmt).all(db).await?;
    Ok(fill_trend(rows, cutoff, now, bucket))
}

fn daily_trend_query(
    start: NaiveDateTime,
    end: NaiveDateTime,
    source: Option<&str>,
) -> Select<telemetry_session_daily::Entity> {
    let mut select = telemetry_session_daily::Entity::find()
        .select_only()
        .column_as(telemetry_session_daily::Column::Day, "bucket")
        .column_as(
            sum_i64(telemetry_session_daily::Column::Sessions),
            "sessions",
        )
        .column_as(
            sum_i64(telemetry_session_daily::Column::CrashedSessions),
            "crashed_sessions",
        )
        .filter(telemetry_session_daily::Column::Day.gte(start))
        .filter(telemetry_session_daily::Column::Day.lte(end))
        .group_by(telemetry_session_daily::Column::Day)
        .order_by_asc(telemetry_session_daily::Column::Day);

    if let Some(source) = source {
        select = select.filter(telemetry_session_daily::Column::Source.eq(source));
    }
    select
}

async fn release_trend<C: ConnectionTrait>(
    db: &C,
    window: SessionWindow,
    now: NaiveDateTime,
    bucket: &str,
    source: Option<&str>,
) -> Result<Vec<ReleaseHealthTrendPoint>, ApiError> {
    match window {
        SessionWindow::Raw { cutoff } => raw_trend(db, cutoff, now, bucket, source).await,
        SessionWindow::Daily { start, end } => {
            let rows = daily_trend_query(start, end, source)
                .into_model::<TrendRow>()
                .all(db)
                .await?;
            Ok(fill_trend(rows, start, end, "day"))
        }
    }
}

fn fill_trend(
    rows: Vec<TrendRow>,
    cutoff: NaiveDateTime,
    now: NaiveDateTime,
    bucket: &str,
) -> Vec<ReleaseHealthTrendPoint> {
    let counts: BTreeMap<NaiveDateTime, (i64, i64)> = rows
        .into_iter()
        .map(|row| (row.bucket, (row.sessions, row.crashed_sessions)))
        .collect();
    bucket_slots(cutoff, now, bucket)
        .into_iter()
        .map(|slot| {
            let (sessions, crashed_sessions) = counts.get(&slot).copied().unwrap_or((0, 0));
            ReleaseHealthTrendPoint {
                ts: DateTime::<Utc>::from_naive_utc_and_offset(slot, Utc).to_rfc3339(),
                sessions,
                crashed_sessions,
                crash_free_session_rate: crash_free(sessions, crashed_sessions),
            }
        })
        .collect()
}

fn build_release_rows(
    sessions: Vec<ReleaseSessionRow>,
    errors: Vec<ReleaseErrorRow>,
    meta: Vec<ReleaseMeta>,
    total_installs: i64,
    limit: usize,
) -> Vec<TelemetryReleaseRow> {
    let mut aggregates: BTreeMap<(String, String), ReleaseAggregate> = BTreeMap::new();

    for row in sessions {
        let Some(release) = row.release else {
            continue;
        };
        let entry = aggregates.entry((release, row.source)).or_default();
        entry.sessions = row.sessions;
        entry.crashed_sessions = row.crashed_sessions;
        entry.installs = row.installs;
        entry.crashed_installs = row.crashed_installs;
    }

    for row in errors {
        let Some(release) = row.release else {
            continue;
        };
        aggregates
            .entry((release, row.source))
            .or_default()
            .error_count = row.cnt;
    }

    for row in meta {
        let entry = aggregates.entry((row.version, row.source)).or_default();
        entry.commit_sha = row.commit_sha;
        entry.first_seen_at = Some(row.first_seen_at);
    }

    let mut rows: Vec<(Option<NaiveDateTime>, i64, String, TelemetryReleaseRow)> = aggregates
        .into_iter()
        .map(|((version, source), aggregate)| {
            let row = TelemetryReleaseRow {
                version: version.clone(),
                source,
                commit_sha: aggregate.commit_sha,
                first_seen_at: aggregate.first_seen_at.map(|ts| ts.and_utc().to_rfc3339()),
                installs: aggregate.installs,
                sessions: aggregate.sessions,
                crashed_sessions: aggregate.crashed_sessions,
                crash_free_session_rate: crash_free(aggregate.sessions, aggregate.crashed_sessions),
                crash_free_install_rate: crash_free(aggregate.installs, aggregate.crashed_installs),
                error_count: aggregate.error_count,
                adoption: ratio(aggregate.installs, total_installs),
            };
            (aggregate.first_seen_at, aggregate.sessions, version, row)
        })
        .collect();

    rows.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
    });

    rows.into_iter()
        .take(limit)
        .map(|(_, _, _, row)| row)
        .collect()
}

async fn release_rows<C: ConnectionTrait>(
    db: &C,
    window: SessionWindow,
    source: Option<&str>,
    total_installs: i64,
    limit: usize,
) -> Result<Vec<TelemetryReleaseRow>, ApiError> {
    let sessions = release_sessions(db, window, source).await?;
    let errors = release_errors(db, window.error_cutoff(), source).await?;
    let meta = release_meta(db, source).await?;
    Ok(build_release_rows(
        sessions,
        errors,
        meta,
        total_installs,
        limit,
    ))
}

fn source_filter(source: &Option<String>) -> Option<&str> {
    source
        .as_deref()
        .map(str::trim)
        .filter(|source| !source.is_empty())
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/releases",
    tag = "admin",
    params(ListTelemetryReleasesQuery),
    responses(
        (status = 200, description = "Releases with adoption and crash-free rates over the last 30 days", body = ListTelemetryReleasesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "List the observed application releases with their adoption, crash-free session and install rates and error counts over the last 30 days. Session numbers come from the daily session rollup and are aligned to whole UTC days; error counts come from the raw error events and therefore only cover the days those are still retained. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/releases", skip_all)]
pub async fn list_telemetry_releases(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<ListTelemetryReleasesQuery>,
) -> Result<Json<ListTelemetryReleasesResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let limit = q
        .limit
        .unwrap_or(RELEASES_DEFAULT_LIMIT)
        .clamp(1, RELEASES_MAX_LIMIT) as usize;
    let source = source_filter(&q.source);
    let window = SessionWindow::new(Utc::now().naive_utc(), RELEASES_WINDOW_HOURS);

    let totals = session_totals(&state.db, window, source).await?;
    let releases = release_rows(&state.db, window, source, totals.installs, limit).await?;

    Ok(Json(ListTelemetryReleasesResponse {
        granularity: granularity_for(RELEASES_WINDOW_HOURS).to_string(),
        releases,
    }))
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/release-health",
    tag = "admin",
    params(TelemetryReleaseHealthQuery),
    responses(
        (status = 200, description = "Crash-free session and install rates with a trend and per-release breakdown", body = TelemetryReleaseHealthResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Release health over anonymous sessions: crash-free session and install rates, their trend over the selected window and the per-release breakdown. Windows of up to 48 hours aggregate individual sessions (granularity \"raw\"); longer windows read the daily session rollup (granularity \"daily\"), which forces whole UTC day buckets. Session and crash counts stay exact across the boundary; in \"daily\" mode the install counts are the sum of the daily distinct installs, so the crash-free install rate is a ratio of daily counts rather than of installs unique across the whole window. Error counts always come from the raw error events. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/release-health", skip_all)]
pub async fn telemetry_release_health(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<TelemetryReleaseHealthQuery>,
) -> Result<Json<TelemetryReleaseHealthResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let hours = q.hours.unwrap_or(168).clamp(1, 24 * 90);
    let bucket = window_bucket(hours, None);
    let source = source_filter(&q.source);
    let now = Utc::now().naive_utc();
    let window = SessionWindow::new(now, hours);

    let totals = session_totals(&state.db, window, source).await?;
    let trend = release_trend(&state.db, window, now, bucket, source).await?;
    let releases = release_rows(
        &state.db,
        window,
        source,
        totals.installs,
        RELEASE_HEALTH_TOP,
    )
    .await?;

    Ok(Json(TelemetryReleaseHealthResponse {
        hours,
        granularity: granularity_for(hours).to_string(),
        total_sessions: totals.sessions,
        crashed_sessions: totals.crashed_sessions,
        crash_free_session_rate: crash_free(totals.sessions, totals.crashed_sessions),
        crash_free_install_rate: crash_free(totals.installs, totals.crashed_installs),
        total_installs: totals.installs,
        trend,
        releases,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::admin::telemetry::overview::{GRANULARITY_DAILY, GRANULARITY_RAW};
    use chrono::NaiveDate;
    use sea_orm::QueryTrait;

    fn ts(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    fn session_row(
        release: &str,
        sessions: i64,
        crashed_sessions: i64,
        installs: i64,
        crashed_installs: i64,
    ) -> ReleaseSessionRow {
        ReleaseSessionRow {
            release: Some(release.to_string()),
            source: "desktop".to_string(),
            sessions,
            crashed_sessions,
            installs,
            crashed_installs,
        }
    }

    fn meta(version: &str, y: i32, m: u32, d: u32) -> ReleaseMeta {
        ReleaseMeta {
            version: version.to_string(),
            source: "desktop".to_string(),
            commit_sha: Some(format!("sha-{version}")),
            first_seen_at: ts(y, m, d, 0),
        }
    }

    #[test]
    fn the_window_switches_to_the_rollup_after_forty_eight_hours() {
        let now = ts(2026, 7, 27, 10);

        assert_eq!(
            SessionWindow::new(now, 47),
            SessionWindow::Raw {
                cutoff: now - Duration::hours(47)
            }
        );
        assert_eq!(
            SessionWindow::new(now, 48),
            SessionWindow::Raw {
                cutoff: now - Duration::hours(48)
            }
        );
        assert_eq!(
            SessionWindow::new(now, 49),
            SessionWindow::Daily {
                start: ts(2026, 7, 25, 0),
                end: ts(2026, 7, 27, 0)
            }
        );
        assert_eq!(granularity_for(48), GRANULARITY_RAW);
        assert_eq!(granularity_for(49), GRANULARITY_DAILY);
        assert_eq!(granularity_for(RELEASES_WINDOW_HOURS), GRANULARITY_DAILY);
    }

    #[test]
    fn the_error_cutoff_follows_the_window_start() {
        let now = ts(2026, 7, 27, 10);

        assert_eq!(
            SessionWindow::new(now, 24).error_cutoff(),
            now - Duration::hours(24)
        );
        assert_eq!(
            SessionWindow::new(now, 24 * 7).error_cutoff(),
            ts(2026, 7, 20, 0)
        );
    }

    #[test]
    fn totals_are_a_single_ungrouped_aggregate() {
        let sql = session_totals_query(ts(2026, 7, 26, 0), Some("desktop"))
            .build(DbBackend::Postgres)
            .to_string();

        assert!(!sql.contains("GROUP BY"), "{sql}");
        assert!(
            sql.contains(r#"COUNT(DISTINCT "anonId") AS "installs""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#""TelemetrySession"."source" = 'desktop'"#),
            "{sql}"
        );
    }

    #[test]
    fn daily_totals_sum_the_rollup_columns() {
        let sql = daily_session_totals_query(ts(2026, 7, 20, 0), ts(2026, 7, 27, 0), Some("web"))
            .build(DbBackend::Postgres)
            .to_string();

        assert!(!sql.contains("GROUP BY"), "{sql}");
        assert!(
            sql.contains(r#"CAST(SUM("sessions") AS BIGINT) AS "sessions""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#"CAST(SUM("crashedInstalls") AS BIGINT) AS "crashed_installs""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#""TelemetrySessionDaily"."source" = 'web'"#),
            "{sql}"
        );
    }

    #[test]
    fn daily_release_aggregates_group_by_release_and_source() {
        let sql = daily_release_sessions_query(ts(2026, 7, 20, 0), ts(2026, 7, 27, 0), None)
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains(
                r#"GROUP BY "TelemetrySessionDaily"."release", "TelemetrySessionDaily"."source""#
            ),
            "{sql}"
        );
    }

    #[test]
    fn release_aggregates_count_crashes_in_the_same_query() {
        let sql = release_sessions_query(ts(2026, 7, 26, 0), None)
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains(
                r#"COUNT((CASE WHEN ("status" = 'crashed') THEN "id" ELSE NULL END)) AS "crashed_sessions""#
            ),
            "{sql}"
        );
        assert!(
            sql.contains(
                r#"COUNT(DISTINCT (CASE WHEN ("status" = 'crashed') THEN "anonId" ELSE NULL END)) AS "crashed_installs""#
            ),
            "{sql}"
        );
        assert!(
            sql.contains(r#""TelemetrySession"."release" IS NOT NULL"#),
            "{sql}"
        );
        assert!(
            sql.contains(r#"GROUP BY "TelemetrySession"."release", "TelemetrySession"."source""#),
            "{sql}"
        );
    }

    #[test]
    fn rates_are_null_without_sessions() {
        assert_eq!(crash_free(0, 0), None);
        assert_eq!(ratio(3, 0), None);
        assert_eq!(crash_free(10, 1), Some(0.9));
        assert_eq!(crash_free(4, 4), Some(0.0));
        assert_eq!(ratio(2, 8), Some(0.25));
    }

    #[test]
    fn empty_daily_totals_read_as_zero_rather_than_null() {
        let totals: SessionTotalsRow = DailySessionTotalsRow::default().into();

        assert_eq!(totals.sessions, 0);
        assert_eq!(totals.crashed_sessions, 0);
        assert_eq!(totals.installs, 0);
        assert_eq!(totals.crashed_installs, 0);
        assert_eq!(crash_free(totals.sessions, totals.crashed_sessions), None);
    }

    #[test]
    fn release_rows_carry_rates_adoption_and_errors() {
        let rows = build_release_rows(
            vec![session_row("1.2.0", 100, 5, 40, 2)],
            vec![ReleaseErrorRow {
                release: Some("1.2.0".to_string()),
                source: "desktop".to_string(),
                cnt: 12,
            }],
            vec![meta("1.2.0", 2026, 7, 20)],
            50,
            10,
        );

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.version, "1.2.0");
        assert_eq!(row.commit_sha.as_deref(), Some("sha-1.2.0"));
        assert_eq!(
            row.first_seen_at.as_deref(),
            Some("2026-07-20T00:00:00+00:00")
        );
        assert_eq!(row.crash_free_session_rate, Some(0.95));
        assert_eq!(row.crash_free_install_rate, Some(0.95));
        assert_eq!(row.adoption, Some(0.8));
        assert_eq!(row.error_count, 12);
    }

    #[test]
    fn registered_releases_without_sessions_still_appear() {
        let rows = build_release_rows(
            Vec::new(),
            Vec::new(),
            vec![meta("2.0.0", 2026, 7, 25)],
            0,
            10,
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sessions, 0);
        assert_eq!(rows[0].crash_free_session_rate, None);
        assert_eq!(rows[0].adoption, None);
    }

    #[test]
    fn releases_sort_newest_first_and_respect_the_limit() {
        let rows = build_release_rows(
            vec![
                session_row("1.0.0", 10, 0, 5, 0),
                session_row("1.1.0", 20, 1, 8, 1),
                session_row("nightly", 99, 0, 30, 0),
            ],
            Vec::new(),
            vec![meta("1.0.0", 2026, 7, 1), meta("1.1.0", 2026, 7, 20)],
            40,
            2,
        );

        assert_eq!(
            rows.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(),
            vec!["1.1.0", "1.0.0"]
        );
    }

    #[test]
    fn trend_is_zero_filled_and_carries_the_crash_free_rate() {
        let rows = vec![TrendRow {
            bucket: ts(2026, 7, 26, 10),
            sessions: 4,
            crashed_sessions: 1,
        }];
        let trend = fill_trend(rows, ts(2026, 7, 26, 10), ts(2026, 7, 26, 11), "hour");

        assert_eq!(
            trend,
            vec![
                ReleaseHealthTrendPoint {
                    ts: "2026-07-26T10:00:00+00:00".to_string(),
                    sessions: 4,
                    crashed_sessions: 1,
                    crash_free_session_rate: Some(0.75),
                },
                ReleaseHealthTrendPoint {
                    ts: "2026-07-26T11:00:00+00:00".to_string(),
                    sessions: 0,
                    crashed_sessions: 0,
                    crash_free_session_rate: None,
                },
            ]
        );
    }

    #[test]
    fn the_daily_trend_is_zero_filled_across_whole_days() {
        let rows = vec![TrendRow {
            bucket: ts(2026, 7, 26, 0),
            sessions: 10,
            crashed_sessions: 2,
        }];
        let trend = fill_trend(rows, ts(2026, 7, 25, 0), ts(2026, 7, 27, 0), "day");

        assert_eq!(trend.len(), 3);
        assert_eq!(trend[0].sessions, 0);
        assert_eq!(trend[1].ts, "2026-07-26T00:00:00+00:00");
        assert_eq!(trend[1].crash_free_session_rate, Some(0.8));
        assert_eq!(trend[2].sessions, 0);
    }
}
