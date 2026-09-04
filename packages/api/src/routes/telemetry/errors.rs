//! Anonymous crash and error report ingest.
//!
//! PRIVACY INVARIANT: like the event ingest, this handler is anonymous by
//! construction. It must never extract `Extension(AppUser)` and never store
//! user identity or IP addresses — only the random, client-generated `anon_id`
//! and a sanitized exception payload.

use axum::{Json, extract::State, http::HeaderMap};
use flow_like_types::Value;
use sea_orm::sea_query::ExprTrait;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, Set,
    sea_query::{Expr, OnConflict},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;

use super::fingerprint::{FingerprintFrame, fingerprint};
use super::{country_from_headers, parse_client_ts, sanitize_props, validate_anon_id};
use crate::{
    entity::{telemetry_error_event, telemetry_issue, telemetry_release},
    error::ApiError,
    state::AppState,
    telemetry::sink_from_env,
};

const MAX_ERRORS_PER_BATCH: usize = 20;
const MAX_STACK_FRAMES: usize = 100;
const MAX_BREADCRUMBS: usize = 50;
const MAX_ERROR_BYTES: usize = 32 * 1024;
const MAX_KIND_LEN: usize = 128;
const MAX_TITLE_LEN: usize = 512;
const MAX_CULPRIT_LEN: usize = 256;
const MAX_LONG_STRING_LEN: usize = 512;
pub(super) const MAX_SHORT_STRING_LEN: usize = 64;
pub(super) const MAX_RELEASE_LEN: usize = 128;

const DEFAULT_LEVEL: &str = "error";
const LEVELS: [&str; 3] = ["error", "fatal", "warning"];
const ISSUE_STATUS_UNRESOLVED: &str = "unresolved";

/// Batch origins accepted by the crash and session ingest. Also the closed
/// vocabulary an admin alert rule may filter on.
pub(crate) const SOURCES: [&str; 6] = [
    "desktop",
    "desktop_core",
    "desktop_native",
    "web",
    "web_server",
    "backend",
];

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryStackFramePayload {
    #[serde(default)]
    pub function: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub lineno: Option<i64>,
    #[serde(default)]
    pub colno: Option<i64>,
    /// Whether the frame belongs to application code. Derived from the file
    /// path when omitted.
    #[serde(default)]
    pub in_app: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryBreadcrumbPayload {
    /// Breadcrumb timestamp (RFC 3339), stored verbatim.
    #[serde(default)]
    pub ts: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    /// Pre-sanitized breadcrumb message. Never user content.
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryErrorPayload {
    /// Exception type, e.g. "TypeError".
    pub kind: String,
    /// Sanitized exception message.
    pub value: String,
    /// One of "error", "fatal", "warning". Defaults to "error".
    #[serde(default)]
    pub level: Option<String>,
    /// Human readable location of the failure, e.g. a route or component.
    #[serde(default)]
    pub culprit: Option<String>,
    /// Up to 100 frames, most recent first.
    #[serde(default)]
    pub stacktrace: Option<Vec<TelemetryStackFramePayload>>,
    /// Up to 50 breadcrumbs, oldest first.
    #[serde(default)]
    pub breadcrumbs: Option<Vec<TelemetryBreadcrumbPayload>>,
    /// Free-form anonymous context object. Secret-looking keys are redacted.
    #[serde(default)]
    pub context: Option<Value>,
    /// Client-side timestamp (RFC 3339). Invalid values are stored as null.
    #[serde(default)]
    pub client_ts: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryErrorIngestPayload {
    /// Random client-generated identifier, 1-64 characters. Never a user id.
    pub anon_id: String,
    /// Origin of the batch: "desktop", "desktop_core", "desktop_native", "web" or "backend".
    pub source: String,
    #[serde(default)]
    pub app_version: Option<String>,
    /// Release identifier used for release health and symbolication.
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    /// Up to 20 errors per batch.
    pub errors: Vec<TelemetryErrorPayload>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TelemetryErrorIngestResponse {
    /// Number of error events that were stored.
    pub accepted: usize,
    /// Number of distinct issues the batch was grouped into.
    pub issues: usize,
}

#[derive(Debug, Serialize)]
struct ValidatedFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lineno: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    colno: Option<i64>,
    in_app: bool,
}

#[derive(Debug, Serialize)]
struct ValidatedBreadcrumb {
    #[serde(skip_serializing_if = "Option::is_none")]
    ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<String>,
}

#[derive(Clone)]
struct ValidatedError {
    fingerprint: String,
    kind: String,
    title: String,
    culprit: Option<String>,
    level: String,
    stacktrace: Option<Value>,
    breadcrumbs: Option<Value>,
    context: Option<Value>,
    client_ts: Option<chrono::NaiveDateTime>,
}

struct IssueGroup {
    kind: String,
    title: String,
    culprit: Option<String>,
    level: String,
    count: i32,
}

pub(super) fn validate_source(source: &str) -> Result<(), ApiError> {
    if !SOURCES.contains(&source) {
        return Err(ApiError::bad_request(format!(
            "Unknown telemetry source '{}'",
            source
        )));
    }
    Ok(())
}

fn truncated(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    value.chars().take(max).collect()
}

pub(super) fn optional_string(value: Option<String>, max: usize) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(truncated(trimmed, max))
}

fn normalize_level(level: Option<String>) -> String {
    level
        .map(|level| level.trim().to_ascii_lowercase())
        .filter(|level| LEVELS.contains(&level.as_str()))
        .unwrap_or_else(|| DEFAULT_LEVEL.to_string())
}

/// Frames without an explicit flag count as application code unless they come
/// from a vendored dependency — the same heuristic symbolication applies.
fn in_app_default(file: Option<&str>) -> bool {
    file.is_some_and(|file| !file.contains("node_modules"))
}

fn validate_frames(frames: Option<Vec<TelemetryStackFramePayload>>) -> Vec<ValidatedFrame> {
    frames
        .unwrap_or_default()
        .into_iter()
        .take(MAX_STACK_FRAMES)
        .map(|frame| {
            let file = optional_string(frame.file, MAX_LONG_STRING_LEN);
            ValidatedFrame {
                in_app: frame
                    .in_app
                    .unwrap_or_else(|| in_app_default(file.as_deref())),
                function: optional_string(frame.function, MAX_LONG_STRING_LEN),
                file,
                lineno: frame.lineno.filter(|line| *line >= 0),
                colno: frame.colno.filter(|col| *col >= 0),
            }
        })
        .collect()
}

/// Keeps the most recent breadcrumbs, which are the ones leading to the crash.
fn validate_breadcrumbs(
    breadcrumbs: Option<Vec<TelemetryBreadcrumbPayload>>,
) -> Vec<ValidatedBreadcrumb> {
    let breadcrumbs = breadcrumbs.unwrap_or_default();
    let skip = breadcrumbs.len().saturating_sub(MAX_BREADCRUMBS);
    breadcrumbs
        .into_iter()
        .skip(skip)
        .map(|crumb| ValidatedBreadcrumb {
            ts: optional_string(crumb.ts, MAX_SHORT_STRING_LEN),
            category: optional_string(crumb.category, MAX_SHORT_STRING_LEN),
            message: optional_string(crumb.message, MAX_LONG_STRING_LEN),
            level: optional_string(crumb.level, MAX_SHORT_STRING_LEN),
        })
        .collect()
}

fn validate_context(context: Option<Value>) -> Option<Value> {
    let mut context = context?;
    if !context.is_object() {
        return None;
    }
    sanitize_props(&mut context);
    Some(context)
}

fn json_bytes(value: Option<&Value>) -> usize {
    match value {
        Some(value) => serde_json::to_vec(value)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX),
        None => 0,
    }
}

fn error_payload_bytes(error: &ValidatedError) -> usize {
    error
        .kind
        .len()
        .saturating_add(error.title.len())
        .saturating_add(error.culprit.as_ref().map_or(0, |culprit| culprit.len()))
        .saturating_add(json_bytes(error.stacktrace.as_ref()))
        .saturating_add(json_bytes(error.breadcrumbs.as_ref()))
        .saturating_add(json_bytes(error.context.as_ref()))
}

fn to_json(values: &[impl Serialize]) -> Option<Value> {
    if values.is_empty() {
        return None;
    }
    serde_json::to_value(values).ok()
}

/// Issues are per source. `TelemetryIssue.fingerprint` is globally unique and
/// an issue is stamped with the source of the batch that created it, so without
/// the source in the fingerprint input the same stack-less failure reported by
/// the web and the desktop app would collapse into a single issue carrying
/// whichever source arrived first — and every source-filtered read would then
/// return the wrong set. `source` comes from the closed `SOURCES` vocabulary,
/// so the separator can not be forged.
fn issue_fingerprint(
    source: &str,
    kind: &str,
    title: &str,
    frames: &[FingerprintFrame<'_>],
) -> String {
    fingerprint(&format!("{}|{}", source, kind), title, frames)
}

fn validate_error(error: TelemetryErrorPayload, source: &str) -> Option<ValidatedError> {
    let kind = optional_string(Some(error.kind), MAX_KIND_LEN)?;
    let title = optional_string(Some(error.value), MAX_TITLE_LEN).unwrap_or_else(|| kind.clone());
    let frames = validate_frames(error.stacktrace);
    let breadcrumbs = validate_breadcrumbs(error.breadcrumbs);

    let fingerprint_frames: Vec<FingerprintFrame<'_>> = frames
        .iter()
        .map(|frame| FingerprintFrame {
            function: frame.function.as_deref(),
            file: frame.file.as_deref(),
            in_app: frame.in_app,
        })
        .collect();

    let validated = ValidatedError {
        fingerprint: issue_fingerprint(source, &kind, &title, &fingerprint_frames),
        kind,
        title,
        culprit: optional_string(error.culprit, MAX_CULPRIT_LEN),
        level: normalize_level(error.level),
        stacktrace: to_json(&frames),
        breadcrumbs: to_json(&breadcrumbs),
        context: validate_context(error.context),
        client_ts: parse_client_ts(error.client_ts.as_deref()),
    };

    if error_payload_bytes(&validated) > MAX_ERROR_BYTES {
        return None;
    }
    Some(validated)
}

fn validate_errors(errors: Vec<TelemetryErrorPayload>, source: &str) -> Vec<ValidatedError> {
    errors
        .into_iter()
        .filter_map(|error| validate_error(error, source))
        .collect()
}

fn group_issues(errors: &[ValidatedError]) -> BTreeMap<String, IssueGroup> {
    let mut groups: BTreeMap<String, IssueGroup> = BTreeMap::new();
    for error in errors {
        groups
            .entry(error.fingerprint.clone())
            .and_modify(|group| group.count = group.count.saturating_add(1))
            .or_insert_with(|| IssueGroup {
                kind: error.kind.clone(),
                title: error.title.clone(),
                culprit: error.culprit.clone(),
                level: error.level.clone(),
                count: 1,
            });
    }
    groups
}

/// The batch-level columns every stored row carries, owned so a retried
/// transaction can rebuild its rows from scratch.
#[derive(Clone)]
struct BatchContext {
    anon_id: String,
    source: String,
    platform: Option<String>,
    app_version: Option<String>,
    release: Option<String>,
    country: Option<String>,
}

/// Records a release the first time it reports in. Shared with the session
/// ingest so release health works without any error report.
pub(super) async fn upsert_release<C: ConnectionTrait>(
    db: &C,
    version: &str,
    source: &str,
    now: chrono::NaiveDateTime,
) -> Result<(), DbErr> {
    telemetry_release::Entity::insert(telemetry_release::ActiveModel {
        id: Set(flow_like_types::create_id()),
        version: Set(version.to_string()),
        source: Set(source.to_string()),
        commit_sha: Set(None),
        first_seen_at: Set(now),
        created_at: Set(now),
    })
    .on_conflict(
        OnConflict::columns([
            telemetry_release::Column::Version,
            telemetry_release::Column::Source,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

fn new_issue(
    id: &str,
    fingerprint: &str,
    group: &IssueGroup,
    payload: &BatchContext,
    now: chrono::NaiveDateTime,
) -> telemetry_issue::ActiveModel {
    telemetry_issue::ActiveModel {
        id: Set(id.to_string()),
        fingerprint: Set(fingerprint.to_string()),
        kind: Set(group.kind.clone()),
        title: Set(group.title.clone()),
        culprit: Set(group.culprit.clone()),
        level: Set(group.level.clone()),
        source: Set(payload.source.clone()),
        platform: Set(payload.platform.clone()),
        status: Set(ISSUE_STATUS_UNRESOLVED.to_string()),
        resolved_in_release: Set(None),
        first_seen: Set(now),
        last_seen: Set(now),
        event_count: Set(group.count),
        install_count: Set(0),
        first_release: Set(payload.release.clone()),
        last_release: Set(payload.release.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    }
}

/// `installCount` is deliberately not maintained here — it is recomputed as a
/// distinct install count when an issue is read.
async fn bump_issue<C: ConnectionTrait>(
    db: &C,
    issue_id: &str,
    count: i32,
    release: Option<&str>,
    now: chrono::NaiveDateTime,
) -> Result<(), DbErr> {
    let mut update = telemetry_issue::Entity::update_many()
        .col_expr(
            telemetry_issue::Column::EventCount,
            Expr::col(telemetry_issue::Column::EventCount).add(count),
        )
        .col_expr(telemetry_issue::Column::LastSeen, Expr::value(now))
        .col_expr(telemetry_issue::Column::UpdatedAt, Expr::value(now))
        .filter(telemetry_issue::Column::Id.eq(issue_id));

    if let Some(release) = release {
        update = update.col_expr(telemetry_issue::Column::LastRelease, Expr::value(release));
    }

    update.exec(db).await?;
    Ok(())
}

/// Finds or creates one issue per fingerprint and returns the fingerprint to
/// issue id mapping. Issues that already existed have their counters bumped.
async fn resolve_issues<C: ConnectionTrait>(
    db: &C,
    groups: &BTreeMap<String, IssueGroup>,
    payload: &BatchContext,
    now: chrono::NaiveDateTime,
) -> Result<HashMap<String, String>, DbErr> {
    let fingerprints: Vec<String> = groups.keys().cloned().collect();
    let existing = telemetry_issue::Entity::find()
        .filter(telemetry_issue::Column::Fingerprint.is_in(fingerprints))
        .all(db)
        .await?;

    let mut ids: HashMap<String, String> = HashMap::new();
    let mut planned: HashMap<String, String> = HashMap::new();
    let mut inserts = Vec::new();

    for issue in existing {
        if let Some(group) = groups.get(&issue.fingerprint) {
            bump_issue(db, &issue.id, group.count, payload.release.as_deref(), now).await?;
        }
        ids.insert(issue.fingerprint, issue.id);
    }

    for (fingerprint, group) in groups {
        if ids.contains_key(fingerprint) {
            continue;
        }
        let id = flow_like_types::create_id();
        inserts.push(new_issue(&id, fingerprint, group, payload, now));
        planned.insert(fingerprint.clone(), id);
    }

    if inserts.is_empty() {
        return Ok(ids);
    }

    telemetry_issue::Entity::insert_many(inserts)
        .on_conflict(
            OnConflict::column(telemetry_issue::Column::Fingerprint)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;

    let created = telemetry_issue::Entity::find()
        .filter(
            telemetry_issue::Column::Fingerprint.is_in(planned.keys().cloned().collect::<Vec<_>>()),
        )
        .all(db)
        .await?;

    for issue in created {
        let lost_race = planned
            .get(&issue.fingerprint)
            .is_some_and(|planned_id| *planned_id != issue.id);
        if lost_race && let Some(group) = groups.get(&issue.fingerprint) {
            bump_issue(db, &issue.id, group.count, payload.release.as_deref(), now).await?;
        }
        ids.insert(issue.fingerprint, issue.id);
    }

    Ok(ids)
}

/// The release upsert, the issue counters and the events land in one
/// transaction: `eventCount` is a stored counter that can not be recomputed
/// from the surviving rows, so a partial write would inflate it permanently
/// while `installCount` — derived on read — kept telling the truth. The
/// counter bump is also what makes a crash storm contend, so the whole batch
/// is retried on a lost commit race with fresh ids.
async fn persist_errors(
    state: &AppState,
    payload: &TelemetryErrorIngestPayload,
    errors: Vec<ValidatedError>,
    country: Option<String>,
) -> Result<TelemetryErrorIngestResponse, DbErr> {
    if errors.is_empty() {
        return Ok(TelemetryErrorIngestResponse {
            accepted: 0,
            issues: 0,
        });
    }

    let context = BatchContext {
        anon_id: payload.anon_id.clone(),
        source: payload.source.clone(),
        platform: payload.platform.clone(),
        app_version: payload.app_version.clone(),
        release: payload.release.clone(),
        country,
    };
    let now = chrono::Utc::now().naive_utc();

    state
        .transaction(|txn| {
            let context = context.clone();
            let errors = errors.clone();
            Box::pin(async move { persist_batch(txn, &context, errors, now).await })
        })
        .await
}

async fn persist_batch<C: ConnectionTrait>(
    txn: &C,
    context: &BatchContext,
    errors: Vec<ValidatedError>,
    now: chrono::NaiveDateTime,
) -> Result<TelemetryErrorIngestResponse, DbErr> {
    if let Some(release) = context.release.as_deref() {
        upsert_release(txn, release, &context.source, now).await?;
    }

    let groups = group_issues(&errors);
    let ids = resolve_issues(txn, &groups, context, now).await?;

    let models: Vec<telemetry_error_event::ActiveModel> = errors
        .into_iter()
        .filter_map(|error| {
            let issue_id = ids.get(&error.fingerprint)?;
            Some(telemetry_error_event::ActiveModel {
                id: Set(flow_like_types::create_id()),
                issue_id: Set(issue_id.clone()),
                anon_id: Set(context.anon_id.clone()),
                source: Set(context.source.clone()),
                platform: Set(context.platform.clone()),
                app_version: Set(context.app_version.clone()),
                release: Set(context.release.clone()),
                kind: Set(error.kind),
                title: Set(error.title),
                culprit: Set(error.culprit),
                level: Set(error.level),
                stacktrace: Set(error.stacktrace),
                breadcrumbs: Set(error.breadcrumbs),
                context: Set(error.context),
                country: Set(context.country.clone()),
                client_ts: Set(error.client_ts),
                created_at: Set(now),
            })
        })
        .collect();

    let accepted = models.len();
    telemetry_error_event::Entity::insert_many(models)
        .exec_without_returning(txn)
        .await?;

    Ok(TelemetryErrorIngestResponse {
        accepted,
        issues: ids.len(),
    })
}

/// Anonymous by construction: this handler intentionally never extracts
/// `Extension(AppUser)` and never persists user identity or IP addresses.
/// The stored country is derived exclusively from proxy geolocation headers
/// (CloudFront/Cloudflare/Vercel); the client IP is never read or stored.
#[utoipa::path(
    post,
    path = "/telemetry/errors",
    tag = "telemetry",
    request_body = TelemetryErrorIngestPayload,
    responses(
        (status = 200, description = "Number of error reports that were accepted and the issues they were grouped into", body = TelemetryErrorIngestResponse),
        (status = 400, description = "Invalid batch"),
        (status = 404, description = "Telemetry is disabled on this platform")
    ),
    description = "Submit a batch of anonymous crash and error reports. No account, user identity or IP address is ever stored — only a random client-generated identifier and a sanitized exception payload."
)]
#[tracing::instrument(name = "POST /telemetry/errors", skip(state, headers, payload))]
pub async fn ingest_errors(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<TelemetryErrorIngestPayload>,
) -> Result<Json<TelemetryErrorIngestResponse>, ApiError> {
    if !state.platform_config.features.telemetry {
        return Err(ApiError::NOT_FOUND);
    }

    validate_anon_id(&payload.anon_id)?;
    validate_source(&payload.source)?;

    if payload.errors.len() > MAX_ERRORS_PER_BATCH {
        return Err(ApiError::bad_request(format!(
            "A telemetry error batch may contain at most {} errors",
            MAX_ERRORS_PER_BATCH
        )));
    }

    payload.app_version = optional_string(payload.app_version.take(), MAX_SHORT_STRING_LEN);
    payload.release = optional_string(payload.release.take(), MAX_RELEASE_LEN);
    payload.platform = optional_string(payload.platform.take(), MAX_SHORT_STRING_LEN);

    let validated = validate_errors(std::mem::take(&mut payload.errors), &payload.source);
    if validated.is_empty() {
        return Ok(Json(TelemetryErrorIngestResponse {
            accepted: 0,
            issues: 0,
        }));
    }

    let groups = validated
        .iter()
        .map(|error| error.fingerprint.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let sink = sink_from_env();

    if sink == "none" {
        return Ok(Json(TelemetryErrorIngestResponse {
            accepted: validated.len(),
            issues: groups,
        }));
    }

    if sink == "log" {
        tracing::info!(
            source = %payload.source,
            anon_id = %payload.anon_id,
            release = payload.release.as_deref().unwrap_or(""),
            platform = payload.platform.as_deref().unwrap_or(""),
            errors = validated.len(),
            issues = groups,
            kinds = ?validated.iter().map(|error| error.kind.as_str()).collect::<Vec<_>>(),
            "telemetry error batch"
        );
        return Ok(Json(TelemetryErrorIngestResponse {
            accepted: validated.len(),
            issues: groups,
        }));
    }

    let country = country_from_headers(&headers);

    match persist_errors(&state, &payload, validated, country).await {
        Ok(response) => Ok(Json(response)),
        Err(e) => {
            tracing::error!("Failed to persist telemetry error batch: {}", e);
            Ok(Json(TelemetryErrorIngestResponse {
                accepted: 0,
                issues: 0,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::QueryTrait;
    use serde_json::json;
    use std::collections::HashSet;

    const SOURCE: &str = "web";

    fn validated(error: TelemetryErrorPayload) -> Option<ValidatedError> {
        validate_error(error, SOURCE)
    }

    fn frame(file: &str, lineno: i64) -> TelemetryStackFramePayload {
        TelemetryStackFramePayload {
            function: Some("renderBoard".to_string()),
            file: Some(file.to_string()),
            lineno: Some(lineno),
            colno: Some(12),
            in_app: Some(true),
        }
    }

    fn error(
        value: &str,
        stacktrace: Option<Vec<TelemetryStackFramePayload>>,
    ) -> TelemetryErrorPayload {
        TelemetryErrorPayload {
            kind: "TypeError".to_string(),
            value: value.to_string(),
            level: None,
            culprit: None,
            stacktrace,
            breadcrumbs: None,
            context: None,
            client_ts: None,
        }
    }

    #[test]
    fn rejects_unknown_sources() {
        assert!(validate_source("desktop").is_ok());
        assert!(validate_source("desktop_native").is_ok());
        assert!(validate_source("web").is_ok());
        assert!(validate_source("web_server").is_ok());
        assert!(validate_source("mobile").is_err());
        assert!(validate_source("").is_err());
    }

    #[test]
    fn caps_the_stacktrace_at_a_hundred_frames() {
        let frames = (0..150).map(|line| frame("board.ts", line)).collect();
        let validated = validated(error("boom", Some(frames))).unwrap();
        let stored = validated.stacktrace.unwrap();
        assert_eq!(stored.as_array().unwrap().len(), MAX_STACK_FRAMES);
    }

    #[test]
    fn keeps_the_most_recent_fifty_breadcrumbs() {
        let breadcrumbs = (0..80)
            .map(|index| TelemetryBreadcrumbPayload {
                ts: None,
                category: Some("nav".to_string()),
                message: Some(format!("step-{}", index)),
                level: None,
            })
            .collect();
        let mut payload = error("boom", None);
        payload.breadcrumbs = Some(breadcrumbs);
        let validated = validated(payload).unwrap();
        let stored = validated.breadcrumbs.unwrap();
        let crumbs = stored.as_array().unwrap();
        assert_eq!(crumbs.len(), MAX_BREADCRUMBS);
        assert_eq!(crumbs[0]["message"], "step-30");
        assert_eq!(crumbs[MAX_BREADCRUMBS - 1]["message"], "step-79");
    }

    #[test]
    fn drops_errors_over_the_size_cap() {
        let mut oversized = error("boom", None);
        oversized.context = Some(json!({ "blob": "x".repeat(MAX_ERROR_BYTES + 1) }));
        assert!(validated(oversized).is_none());

        let mut small = error("boom", None);
        small.context = Some(json!({ "blob": "x".repeat(1024) }));
        assert!(validated(small).is_some());
    }

    #[test]
    fn drops_errors_without_a_kind() {
        let mut blank = error("boom", None);
        blank.kind = "   ".to_string();
        assert!(validated(blank).is_none());
        assert_eq!(validate_errors(vec![error("boom", None)], SOURCE).len(), 1);
    }

    #[test]
    fn falls_back_to_the_kind_when_the_value_is_blank() {
        let validated = validated(error("  ", None)).unwrap();
        assert_eq!(validated.title, "TypeError");
    }

    #[test]
    fn redacts_secret_keys_in_context_at_any_depth() {
        let mut payload = error("boom", None);
        payload.context = Some(json!({
            "route": "/library",
            "nested": { "API_KEY": "abc", "list": [{ "refresh_token": "xyz" }] }
        }));
        let context = validated(payload).unwrap().context.unwrap();
        assert_eq!(context["nested"]["API_KEY"], "[REDACTED]");
        assert_eq!(context["nested"]["list"][0]["refresh_token"], "[REDACTED]");
        assert_eq!(context["route"], "/library");
    }

    #[test]
    fn ignores_non_object_context() {
        let mut payload = error("boom", None);
        payload.context = Some(json!(["not", "an", "object"]));
        assert!(validated(payload).unwrap().context.is_none());
    }

    #[test]
    fn normalizes_levels() {
        assert_eq!(normalize_level(Some(" FATAL ".to_string())), "fatal");
        assert_eq!(normalize_level(Some("debug".to_string())), DEFAULT_LEVEL);
        assert_eq!(normalize_level(None), DEFAULT_LEVEL);
    }

    #[test]
    fn line_drift_does_not_split_an_issue() {
        let first = validated(error("boom", Some(vec![frame("board.ts", 42)]))).unwrap();
        let second = validated(error("boom", Some(vec![frame("board.ts", 4711)]))).unwrap();
        assert_eq!(first.fingerprint, second.fingerprint);
    }

    #[test]
    fn the_same_failure_from_two_sources_is_two_issues() {
        let web = validate_error(error("Failed to fetch", None), "web").unwrap();
        let desktop = validate_error(error("Failed to fetch", None), "desktop").unwrap();
        assert_ne!(web.fingerprint, desktop.fingerprint);

        let stacked_web =
            validate_error(error("boom", Some(vec![frame("board.ts", 42)])), "web").unwrap();
        let stacked_desktop =
            validate_error(error("boom", Some(vec![frame("board.ts", 42)])), "desktop").unwrap();
        assert_ne!(stacked_web.fingerprint, stacked_desktop.fingerprint);
    }

    #[test]
    fn no_two_sources_share_a_fingerprint() {
        let fingerprints: HashSet<String> = SOURCES
            .iter()
            .map(|source| {
                validate_error(error("Failed to fetch", None), source)
                    .unwrap()
                    .fingerprint
            })
            .collect();
        assert_eq!(fingerprints.len(), SOURCES.len());
    }

    #[test]
    fn the_same_failure_from_one_source_stays_one_issue() {
        let first = validate_error(error("Failed to fetch", None), "desktop_native").unwrap();
        let second = validate_error(error("Failed to fetch", None), "desktop_native").unwrap();
        assert_eq!(first.fingerprint, second.fingerprint);

        let stacked = validate_error(
            error("boom", Some(vec![frame("board.ts", 42)])),
            "desktop_native",
        )
        .unwrap();
        let drifted = validate_error(
            error("boom", Some(vec![frame("board.ts", 4711)])),
            "desktop_native",
        )
        .unwrap();
        assert_eq!(stacked.fingerprint, drifted.fingerprint);
    }

    #[test]
    fn different_stacks_produce_different_issues() {
        let first = validated(error("boom", Some(vec![frame("board.ts", 1)]))).unwrap();
        let second = validated(error("boom", Some(vec![frame("editor.ts", 1)]))).unwrap();
        assert_ne!(first.fingerprint, second.fingerprint);
    }

    #[test]
    fn derives_in_app_from_the_file_path() {
        assert!(in_app_default(Some("src/board.ts")));
        assert!(!in_app_default(Some("node_modules/react-dom/index.js")));
        assert!(!in_app_default(None));
    }

    #[test]
    fn groups_events_by_fingerprint() {
        let errors = validate_errors(
            vec![
                error("boom", Some(vec![frame("board.ts", 1)])),
                error("boom", Some(vec![frame("board.ts", 99)])),
                error("boom", Some(vec![frame("editor.ts", 1)])),
            ],
            SOURCE,
        );
        let groups = group_issues(&errors);
        assert_eq!(groups.len(), 2);
        let mut counts: Vec<i32> = groups.values().map(|group| group.count).collect();
        counts.sort_unstable();
        assert_eq!(counts, vec![1, 2]);
    }

    #[test]
    fn issue_counters_are_bumped_atomically() {
        let sql = telemetry_issue::Entity::update_many()
            .col_expr(
                telemetry_issue::Column::EventCount,
                Expr::col(telemetry_issue::Column::EventCount).add(3),
            )
            .filter(telemetry_issue::Column::Id.eq("issue-1"))
            .build(sea_orm::DbBackend::Postgres)
            .to_string();
        assert!(
            sql.contains(r#""eventCount" = "eventCount" + 3"#),
            "{}",
            sql
        );
        assert!(!sql.contains("installCount"), "{}", sql);
    }

    #[test]
    fn truncates_oversized_strings() {
        let long = "x".repeat(MAX_TITLE_LEN + 10);
        let validated = validated(error(&long, None)).unwrap();
        assert_eq!(validated.title.len(), MAX_TITLE_LEN);
        assert_eq!(optional_string(Some("  ".to_string()), 10), None);
    }
}
