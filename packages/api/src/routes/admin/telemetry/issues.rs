//! Crash and error issues: grouped occurrences, detail with symbolicated stack
//! frames, and triage status updates.

use super::overview::PlatformBucket;
use super::symbolicate::{SourceMapEntry, StackFrame, basename, symbolicate_frames};
use super::{TOP_LIST_LIMIT, bucket_for, bucket_slots};
use crate::entity::{telemetry_error_event, telemetry_issue, telemetry_source_map};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use sea_orm::sea_query::{Expr, Func};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DbBackend, EntityTrait,
    FromQueryResult, IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Select,
    Set, Statement,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeSet, HashMap};
use utoipa::{IntoParams, ToSchema};

const ISSUE_STATUSES: [&str; 3] = ["unresolved", "resolved", "ignored"];
/// Window used for the occurrence chart and the release/platform breakdowns.
const ISSUE_DETAIL_HOURS: i64 = 24 * 30;
/// Upper bound on how many stored maps a single symbolication pass may load.
const MAX_SYMBOLICATION_MAPS: u64 = 25;
/// Upper bound on the map index scanned to find candidates for a stack trace.
const SOURCE_MAP_INDEX_CAP: u64 = 500;

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListTelemetryIssuesQuery {
    /// Lookback window in hours over the last occurrence. Default 720 (30 days).
    #[serde(default)]
    pub hours: Option<i64>,
    /// Filter by triage status: "unresolved", "resolved" or "ignored".
    #[serde(default)]
    pub status: Option<String>,
    /// Filter by source: "desktop", "web", "desktop_native" or "backend".
    #[serde(default)]
    pub source: Option<String>,
    /// Filter by level: "error", "fatal" or "warning".
    #[serde(default)]
    pub level: Option<String>,
    /// Case-insensitive substring match on the issue title and culprit.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub page: Option<u64>,
    /// Page size, capped at 100. Default 25.
    #[serde(default)]
    pub page_size: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryIssueRecord {
    pub id: String,
    pub fingerprint: String,
    pub kind: String,
    pub title: String,
    pub culprit: Option<String>,
    pub level: String,
    pub source: String,
    pub platform: Option<String>,
    pub status: String,
    pub resolved_in_release: Option<String>,
    pub first_seen: String,
    pub last_seen: String,
    pub event_count: i64,
    /// Distinct anonymous installs affected, recomputed on read.
    pub install_count: i64,
    pub first_release: Option<String>,
    pub last_release: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTelemetryIssuesResponse {
    pub issues: Vec<TelemetryIssueRecord>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryIssueEvent {
    pub id: String,
    pub anon_id: String,
    pub source: String,
    pub platform: Option<String>,
    pub app_version: Option<String>,
    pub release: Option<String>,
    pub stacktrace: Vec<StackFrame>,
    pub breadcrumbs: Option<serde_json::Value>,
    pub context: Option<serde_json::Value>,
    pub country: Option<String>,
    pub client_ts: Option<String>,
    pub created_at: String,
    /// True when at least one frame was resolved through a stored source map.
    pub symbolicated: bool,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueTimeseriesPoint {
    /// ISO-8601 timestamp at the start of the bucket.
    pub ts: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueReleaseBucket {
    pub release: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryIssueDetailResponse {
    pub issue: TelemetryIssueRecord,
    pub latest_event: Option<TelemetryIssueEvent>,
    pub timeseries: Vec<IssueTimeseriesPoint>,
    pub releases: Vec<IssueReleaseBucket>,
    pub platforms: Vec<PlatformBucket>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTelemetryIssuePayload {
    /// New triage status: "unresolved", "resolved" or "ignored".
    #[serde(default)]
    pub status: Option<String>,
    /// Release that fixes the issue. Send null to clear it.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<String>)]
    pub resolved_in_release: Option<Option<String>>,
}

/// Distinguishes an absent field (`None`) from an explicit `null` (`Some(None)`).
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

#[derive(Debug, FromQueryResult)]
struct IssueInstallRow {
    issue_id: String,
    installs: i64,
}

#[derive(Debug, FromQueryResult)]
struct KeyCountRow {
    key: Option<String>,
    cnt: i64,
}

#[derive(Debug, FromQueryResult)]
struct IssueBucketRow {
    bucket: NaiveDateTime,
    cnt: i64,
}

#[derive(Debug, FromQueryResult)]
struct SourceMapRef {
    id: String,
    file_name: String,
}

/// Escapes LIKE wildcards so a user query never turns into a pattern.
fn like_pattern(term: &str) -> String {
    let escaped = term
        .to_lowercase()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{}%", escaped)
}

fn validate_status(status: &str) -> Result<(), ApiError> {
    if ISSUE_STATUSES.contains(&status) {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "Unknown issue status '{}', expected one of {}",
        status,
        ISSUE_STATUSES.join(", ")
    )))
}

fn issue_record(model: telemetry_issue::Model, install_count: i64) -> TelemetryIssueRecord {
    TelemetryIssueRecord {
        id: model.id,
        fingerprint: model.fingerprint,
        kind: model.kind,
        title: model.title,
        culprit: model.culprit,
        level: model.level,
        source: model.source,
        platform: model.platform,
        status: model.status,
        resolved_in_release: model.resolved_in_release,
        first_seen: model.first_seen.and_utc().to_rfc3339(),
        last_seen: model.last_seen.and_utc().to_rfc3339(),
        event_count: model.event_count as i64,
        install_count,
        first_release: model.first_release,
        last_release: model.last_release,
    }
}

/// One grouped query for the whole page: distinct installs per issue.
fn install_counts_query(issue_ids: Vec<String>) -> Select<telemetry_error_event::Entity> {
    telemetry_error_event::Entity::find()
        .select_only()
        .column_as(telemetry_error_event::Column::IssueId, "issue_id")
        .column_as(
            Expr::col(telemetry_error_event::Column::AnonId).count_distinct(),
            "installs",
        )
        .filter(telemetry_error_event::Column::IssueId.is_in(issue_ids))
        .group_by(telemetry_error_event::Column::IssueId)
}

async fn install_counts<C: ConnectionTrait>(
    db: &C,
    issue_ids: Vec<String>,
) -> Result<HashMap<String, i64>, ApiError> {
    if issue_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = install_counts_query(issue_ids)
        .into_model::<IssueInstallRow>()
        .all(db)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.issue_id, row.installs))
        .collect())
}

fn breakdown_query(
    column: telemetry_error_event::Column,
    issue_id: &str,
    cutoff: NaiveDateTime,
) -> Select<telemetry_error_event::Entity> {
    telemetry_error_event::Entity::find()
        .select_only()
        .column_as(column, "key")
        .column_as(Expr::col(telemetry_error_event::Column::Id).count(), "cnt")
        .filter(telemetry_error_event::Column::IssueId.eq(issue_id))
        .filter(telemetry_error_event::Column::CreatedAt.gte(cutoff))
        .group_by(column)
        .order_by_desc(Expr::col(telemetry_error_event::Column::Id).count())
        .limit(TOP_LIST_LIMIT)
}

async fn breakdown<C: ConnectionTrait>(
    db: &C,
    column: telemetry_error_event::Column,
    issue_id: &str,
    cutoff: NaiveDateTime,
) -> Result<Vec<KeyCountRow>, ApiError> {
    let rows = breakdown_query(column, issue_id, cutoff)
        .into_model::<KeyCountRow>()
        .all(db)
        .await?;
    Ok(rows)
}

fn fill_timeseries(
    rows: Vec<IssueBucketRow>,
    cutoff: NaiveDateTime,
    now: NaiveDateTime,
    bucket: &str,
) -> Vec<IssueTimeseriesPoint> {
    let counts: HashMap<NaiveDateTime, i64> =
        rows.into_iter().map(|row| (row.bucket, row.cnt)).collect();
    bucket_slots(cutoff, now, bucket)
        .into_iter()
        .map(|slot| IssueTimeseriesPoint {
            ts: DateTime::<Utc>::from_naive_utc_and_offset(slot, Utc).to_rfc3339(),
            count: counts.get(&slot).copied().unwrap_or(0),
        })
        .collect()
}

async fn issue_timeseries<C: ConnectionTrait>(
    db: &C,
    issue_id: &str,
    cutoff: NaiveDateTime,
    now: NaiveDateTime,
    bucket: &str,
) -> Result<Vec<IssueTimeseriesPoint>, ApiError> {
    let backend = db.get_database_backend();
    let sql = match backend {
        DbBackend::Postgres => format!(
            r#"SELECT date_trunc('{bucket}', "createdAt") AS bucket, COUNT(*) AS cnt
               FROM "TelemetryErrorEvent"
               WHERE "issueId" = $1 AND "createdAt" >= $2
               GROUP BY bucket
               ORDER BY bucket ASC"#,
        ),
        _ => format!(
            r#"SELECT date_trunc('{bucket}', created_at) AS bucket, COUNT(*) AS cnt
               FROM telemetry_error_event
               WHERE issue_id = $1 AND created_at >= $2
               GROUP BY bucket
               ORDER BY bucket ASC"#,
        ),
    };

    let stmt = Statement::from_sql_and_values(backend, sql, [issue_id.into(), cutoff.into()]);
    let rows = IssueBucketRow::find_by_statement(stmt).all(db).await?;
    Ok(fill_timeseries(rows, cutoff, now, bucket))
}

/// Loads only the stored maps whose file name matches a frame in this stack
/// trace; source maps are large enough that fetching a whole release is not an
/// option.
async fn source_maps_for_frames<C: ConnectionTrait>(
    db: &C,
    release: &str,
    source: &str,
    frames: &[StackFrame],
) -> Result<Vec<SourceMapEntry>, ApiError> {
    let wanted: BTreeSet<&str> = frames
        .iter()
        .filter_map(|frame| frame.file.as_deref())
        .map(basename)
        .collect();
    if wanted.is_empty() {
        return Ok(Vec::new());
    }

    let candidates = telemetry_source_map::Entity::find()
        .select_only()
        .column_as(telemetry_source_map::Column::Id, "id")
        .column_as(telemetry_source_map::Column::FileName, "file_name")
        .filter(telemetry_source_map::Column::Release.eq(release))
        .filter(telemetry_source_map::Column::Source.eq(source))
        .limit(SOURCE_MAP_INDEX_CAP)
        .into_model::<SourceMapRef>()
        .all(db)
        .await?;

    let ids: Vec<String> = candidates
        .into_iter()
        .filter(|row| wanted.contains(basename(&row.file_name)))
        .map(|row| row.id)
        .take(MAX_SYMBOLICATION_MAPS as usize)
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let maps = telemetry_source_map::Entity::find()
        .filter(telemetry_source_map::Column::Id.is_in(ids))
        .limit(MAX_SYMBOLICATION_MAPS)
        .all(db)
        .await?;

    Ok(maps
        .into_iter()
        .map(|model| SourceMapEntry {
            file_name: model.file_name,
            map: model.map,
        })
        .collect())
}

async fn latest_event<C: ConnectionTrait>(
    db: &C,
    issue_id: &str,
) -> Result<Option<TelemetryIssueEvent>, ApiError> {
    let Some(model) = telemetry_error_event::Entity::find()
        .filter(telemetry_error_event::Column::IssueId.eq(issue_id))
        .order_by_desc(telemetry_error_event::Column::CreatedAt)
        .one(db)
        .await?
    else {
        return Ok(None);
    };

    let frames: Vec<StackFrame> = model
        .stacktrace
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();

    let (stacktrace, symbolicated) = match model.release.as_deref() {
        Some(release) if !frames.is_empty() => {
            let maps = source_maps_for_frames(db, release, &model.source, &frames).await?;
            symbolicate_frames(frames, &maps)
        }
        _ => (frames, false),
    };

    Ok(Some(TelemetryIssueEvent {
        id: model.id,
        anon_id: model.anon_id,
        source: model.source,
        platform: model.platform,
        app_version: model.app_version,
        release: model.release,
        stacktrace,
        breadcrumbs: model.breadcrumbs,
        context: model.context,
        country: model.country,
        client_ts: model.client_ts.map(|ts| ts.and_utc().to_rfc3339()),
        created_at: model.created_at.and_utc().to_rfc3339(),
        symbolicated,
    }))
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/issues",
    tag = "admin",
    params(ListTelemetryIssuesQuery),
    responses(
        (status = 200, description = "Paginated list of grouped crash and error issues, most recent first", body = ListTelemetryIssuesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "List grouped crash and error issues with the number of events and affected anonymous installs. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/issues", skip_all)]
pub async fn list_telemetry_issues(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<ListTelemetryIssuesQuery>,
) -> Result<Json<ListTelemetryIssuesResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let hours = q.hours.unwrap_or(24 * 30).clamp(1, 24 * 90);
    let page = q.page.unwrap_or(0);
    let page_size = q.page_size.unwrap_or(25).clamp(1, 100);
    let cutoff = Utc::now().naive_utc() - Duration::hours(hours);

    let mut select =
        telemetry_issue::Entity::find().filter(telemetry_issue::Column::LastSeen.gte(cutoff));

    if let Some(status) = &q.status
        && !status.is_empty()
    {
        validate_status(status)?;
        select = select.filter(telemetry_issue::Column::Status.eq(status));
    }

    if let Some(source) = &q.source
        && !source.is_empty()
    {
        select = select.filter(telemetry_issue::Column::Source.eq(source));
    }

    if let Some(level) = &q.level
        && !level.is_empty()
    {
        select = select.filter(telemetry_issue::Column::Level.eq(level));
    }

    if let Some(term) = &q.query
        && !term.trim().is_empty()
    {
        let pattern = like_pattern(term.trim());
        select = select.filter(
            Condition::any()
                .add(
                    Expr::expr(Func::lower(Expr::col(telemetry_issue::Column::Title)))
                        .like(pattern.clone()),
                )
                .add(
                    Expr::expr(Func::lower(Expr::col(telemetry_issue::Column::Culprit)))
                        .like(pattern),
                ),
        );
    }

    let total = select.clone().count(&state.db).await?;

    let models = select
        .order_by_desc(telemetry_issue::Column::LastSeen)
        .paginate(&state.db, page_size)
        .fetch_page(page)
        .await?;

    let installs = install_counts(
        &state.db,
        models.iter().map(|model| model.id.clone()).collect(),
    )
    .await?;

    let issues = models
        .into_iter()
        .map(|model| {
            let install_count = installs.get(&model.id).copied().unwrap_or(0);
            issue_record(model, install_count)
        })
        .collect();

    Ok(Json(ListTelemetryIssuesResponse {
        issues,
        total,
        page,
        page_size,
    }))
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/issues/{issue_id}",
    tag = "admin",
    params(("issue_id" = String, Path, description = "Issue identifier")),
    responses(
        (status = 200, description = "Issue detail with the latest symbolicated event and breakdowns", body = TelemetryIssueDetailResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Issue not found")
    ),
    description = "Inspect a single crash or error issue: the most recent event with source-mapped stack frames, its occurrence chart and the affected releases and platforms. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/issues/{issue_id}", skip(state, user))]
pub async fn get_telemetry_issue(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(issue_id): Path<String>,
) -> Result<Json<TelemetryIssueDetailResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let model = telemetry_issue::Entity::find_by_id(&issue_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let now = Utc::now().naive_utc();
    let cutoff = now - Duration::hours(ISSUE_DETAIL_HOURS);
    let bucket = bucket_for(ISSUE_DETAIL_HOURS, None);

    let installs = install_counts(&state.db, vec![issue_id.clone()])
        .await?
        .get(&issue_id)
        .copied()
        .unwrap_or(0);

    let latest_event = latest_event(&state.db, &issue_id).await?;
    let timeseries = issue_timeseries(&state.db, &issue_id, cutoff, now, bucket).await?;

    let releases = breakdown(
        &state.db,
        telemetry_error_event::Column::Release,
        &issue_id,
        cutoff,
    )
    .await?
    .into_iter()
    .map(|row| IssueReleaseBucket {
        release: row.key.unwrap_or_else(|| "unknown".to_string()),
        count: row.cnt,
    })
    .collect();

    let platforms = breakdown(
        &state.db,
        telemetry_error_event::Column::Platform,
        &issue_id,
        cutoff,
    )
    .await?
    .into_iter()
    .map(|row| PlatformBucket {
        platform: row.key.unwrap_or_else(|| "unknown".to_string()),
        count: row.cnt,
    })
    .collect();

    Ok(Json(TelemetryIssueDetailResponse {
        issue: issue_record(model, installs),
        latest_event,
        timeseries,
        releases,
        platforms,
    }))
}

#[utoipa::path(
    patch,
    path = "/admin/telemetry/issues/{issue_id}",
    tag = "admin",
    params(("issue_id" = String, Path, description = "Issue identifier")),
    request_body = UpdateTelemetryIssuePayload,
    responses(
        (status = 200, description = "The updated issue", body = TelemetryIssueRecord),
        (status = 400, description = "Unknown status"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Issue not found")
    ),
    description = "Triage a crash or error issue by marking it resolved, ignored or unresolved and recording the release that fixed it. Requires Admin permission."
)]
#[tracing::instrument(
    name = "PATCH /admin/telemetry/issues/{issue_id}",
    skip(state, user, payload)
)]
pub async fn update_telemetry_issue(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(issue_id): Path<String>,
    Json(payload): Json<UpdateTelemetryIssuePayload>,
) -> Result<Json<TelemetryIssueRecord>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let model = telemetry_issue::Entity::find_by_id(&issue_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut active = model.into_active_model();

    if let Some(status) = payload.status {
        validate_status(&status)?;
        active.status = Set(status);
    }

    if let Some(resolved_in_release) = payload.resolved_in_release {
        active.resolved_in_release = Set(resolved_in_release.filter(|v| !v.trim().is_empty()));
    }

    active.updated_at = Set(Utc::now().naive_utc());

    let model = active.update(&state.db).await?;
    let installs = install_counts(&state.db, vec![issue_id.clone()])
        .await?
        .get(&issue_id)
        .copied()
        .unwrap_or(0);

    Ok(Json(issue_record(model, installs)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use sea_orm::QueryTrait;

    fn ts(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    #[test]
    fn install_counts_are_one_grouped_query_for_the_whole_page() {
        let sql = install_counts_query(vec!["a".to_string(), "b".to_string()])
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains(r#"COUNT(DISTINCT "anonId") AS "installs""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#"WHERE "TelemetryErrorEvent"."issueId" IN ('a', 'b')"#),
            "{sql}"
        );
        assert!(
            sql.contains(r#"GROUP BY "TelemetryErrorEvent"."issueId""#),
            "{sql}"
        );
    }

    #[test]
    fn breakdowns_group_by_column_and_rank_by_occurrences() {
        let sql = breakdown_query(
            telemetry_error_event::Column::Platform,
            "issue-1",
            ts(2026, 7, 26, 0),
        )
        .build(DbBackend::Postgres)
        .to_string();

        assert!(
            sql.contains(r#""TelemetryErrorEvent"."platform" AS "key""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#"GROUP BY "TelemetryErrorEvent"."platform""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#"ORDER BY COUNT("id") DESC LIMIT 10"#),
            "{sql}"
        );
    }

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(like_pattern("Boom"), "%boom%");
        assert_eq!(like_pattern("100% CPU"), "%100\\% cpu%");
        assert_eq!(like_pattern("a_b"), "%a\\_b%");
        assert_eq!(like_pattern("back\\slash"), "%back\\\\slash%");
    }

    #[test]
    fn only_known_statuses_are_accepted() {
        for status in ISSUE_STATUSES {
            assert!(validate_status(status).is_ok());
        }
        assert!(validate_status("done").is_err());
        assert!(validate_status("").is_err());
    }

    #[test]
    fn timeseries_is_zero_filled_across_the_window() {
        let rows = vec![
            IssueBucketRow {
                bucket: ts(2026, 7, 26, 10),
                cnt: 3,
            },
            IssueBucketRow {
                bucket: ts(2026, 7, 26, 12),
                cnt: 1,
            },
        ];
        let points = fill_timeseries(rows, ts(2026, 7, 26, 10), ts(2026, 7, 26, 13), "hour");
        assert_eq!(
            points,
            vec![
                IssueTimeseriesPoint {
                    ts: "2026-07-26T10:00:00+00:00".to_string(),
                    count: 3
                },
                IssueTimeseriesPoint {
                    ts: "2026-07-26T11:00:00+00:00".to_string(),
                    count: 0
                },
                IssueTimeseriesPoint {
                    ts: "2026-07-26T12:00:00+00:00".to_string(),
                    count: 1
                },
                IssueTimeseriesPoint {
                    ts: "2026-07-26T13:00:00+00:00".to_string(),
                    count: 0
                },
            ]
        );
    }

    #[test]
    fn resolved_in_release_distinguishes_absent_from_null() {
        let absent: UpdateTelemetryIssuePayload =
            serde_json::from_str(r#"{"status":"resolved"}"#).unwrap();
        assert_eq!(absent.resolved_in_release, None);

        let cleared: UpdateTelemetryIssuePayload =
            serde_json::from_str(r#"{"resolved_in_release":null}"#).unwrap();
        assert_eq!(cleared.resolved_in_release, Some(None));

        let set: UpdateTelemetryIssuePayload =
            serde_json::from_str(r#"{"resolved_in_release":"1.2.3"}"#).unwrap();
        assert_eq!(set.resolved_in_release, Some(Some("1.2.3".to_string())));
    }
}
