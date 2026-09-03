//! Anonymous release-health session ingest.
//!
//! PRIVACY INVARIANT: like the event ingest, this handler is anonymous by
//! construction. It must never extract `Extension(AppUser)` and never store
//! user identity or IP addresses — only the random, client-generated `anon_id`
//! and the coarse outcome of a session.

use axum::{Json, extract::State};
use sea_orm::{
    ConnectionTrait, DbErr, EntityTrait, Set,
    sea_query::{Alias, CaseStatement, Expr, Func, OnConflict, SimpleExpr},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

use super::errors::{
    MAX_RELEASE_LEN, MAX_SHORT_STRING_LEN, optional_string, upsert_release, validate_source,
};
use super::{parse_client_ts, validate_anon_id};
use crate::{
    entity::telemetry_session, error::ApiError, state::AppState, telemetry::sink_from_env,
};

const MAX_SESSIONS_PER_BATCH: usize = 50;
const MAX_SESSION_ID_LEN: usize = 64;
/// How far ahead of the server clock a reported start may sit. Release health
/// windows are open ended (`startedAt >= cutoff`), and sessions are never swept,
/// so a single future-dated row would count towards every window from now until
/// that timestamp passes.
const MAX_CLOCK_SKEW_MINUTES: i64 = 5;
/// Oldest start still accepted. A wrong client clock must not be able to
/// backfill release health arbitrarily far into the past.
const MAX_SESSION_AGE_DAYS: i64 = 90;

/// Session outcomes ordered by severity. A session never moves back down this
/// ladder, so a late "ok" report can not overwrite a crash.
const SESSION_STATUS_PRECEDENCE: [(&str, i32); 4] =
    [("ok", 0), ("errored", 1), ("abnormal", 2), ("crashed", 3)];

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetrySessionPayload {
    /// Client-generated session identifier, 1-64 characters.
    pub session_id: String,
    /// One of "ok", "errored", "abnormal", "crashed".
    pub status: String,
    /// Session start (RFC 3339). Skipped when it is more than 5 minutes ahead of
    /// the server clock or more than 90 days old.
    pub started_at: String,
    #[serde(default)]
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetrySessionIngestPayload {
    /// Random client-generated identifier, 1-64 characters. Never a user id.
    pub anon_id: String,
    /// Origin of the batch: "desktop", "desktop_core", "desktop_native", "web" or "backend".
    pub source: String,
    /// Release identifier the sessions belong to.
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    /// Up to 50 sessions per batch.
    pub sessions: Vec<TelemetrySessionPayload>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TelemetrySessionIngestResponse {
    /// Number of sessions that were stored.
    pub accepted: usize,
    /// Sessions the batch reported that were dropped: an unknown status, an
    /// unusable session id, or a start outside the accepted clock range.
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct ValidatedSession {
    session_id: String,
    status: String,
    started_at: chrono::NaiveDateTime,
    duration_ms: Option<i32>,
}

impl ValidatedSession {
    fn merge(&mut self, other: ValidatedSession) {
        let status = merge_status(&self.status, &other.status).to_string();
        self.status = status;
        self.started_at = self.started_at.min(other.started_at);
        self.duration_ms = match (self.duration_ms, other.duration_ms) {
            (Some(current), Some(next)) => Some(current.max(next)),
            (current, next) => current.or(next),
        };
    }
}

fn status_rank(status: &str) -> i32 {
    SESSION_STATUS_PRECEDENCE
        .iter()
        .find(|(name, _)| *name == status)
        .map(|(_, rank)| *rank)
        .unwrap_or(0)
}

fn is_known_status(status: &str) -> bool {
    SESSION_STATUS_PRECEDENCE
        .iter()
        .any(|(name, _)| *name == status)
}

fn merge_status<'a>(stored: &'a str, incoming: &'a str) -> &'a str {
    if status_rank(incoming) >= status_rank(stored) {
        incoming
    } else {
        stored
    }
}

/// A start is only usable while it sits inside the window the release-health
/// queries can still reach: a session is never swept and every window filters
/// with an open upper bound, so one future-dated row would depress crash-free
/// rates for every window, permanently.
fn started_at_in_range(started_at: chrono::NaiveDateTime, now: chrono::NaiveDateTime) -> bool {
    started_at <= now + chrono::Duration::minutes(MAX_CLOCK_SKEW_MINUTES)
        && started_at >= now - chrono::Duration::days(MAX_SESSION_AGE_DAYS)
}

fn validate_session(
    session: TelemetrySessionPayload,
    now: chrono::NaiveDateTime,
) -> Option<ValidatedSession> {
    let session_id = session.session_id.trim();
    if session_id.is_empty() || session_id.len() > MAX_SESSION_ID_LEN {
        return None;
    }

    let status = session.status.trim().to_ascii_lowercase();
    if !is_known_status(&status) {
        return None;
    }

    let started_at = parse_client_ts(Some(&session.started_at))?;
    if !started_at_in_range(started_at, now) {
        return None;
    }

    Some(ValidatedSession {
        session_id: session_id.to_string(),
        status,
        started_at,
        duration_ms: session
            .duration_ms
            .filter(|duration| *duration >= 0)
            .map(|duration| duration.min(i32::MAX as i64) as i32),
    })
}

/// The sessions a batch contributes plus the entries that were dropped, so the
/// client learns that something it reported never made it into release health.
struct ValidatedBatch {
    sessions: Vec<ValidatedSession>,
    skipped: usize,
}

/// Collapses repeats of the same session inside one batch so a single upsert
/// carries the most severe status the client reported.
fn validate_sessions(
    sessions: Vec<TelemetrySessionPayload>,
    now: chrono::NaiveDateTime,
) -> ValidatedBatch {
    let mut merged: BTreeMap<String, ValidatedSession> = BTreeMap::new();
    let mut skipped = 0;
    for session in sessions {
        let Some(validated) = validate_session(session, now) else {
            skipped += 1;
            continue;
        };
        match merged.get_mut(&validated.session_id) {
            Some(existing) => existing.merge(validated),
            None => {
                merged.insert(validated.session_id.clone(), validated);
            }
        }
    }
    ValidatedBatch {
        sessions: merged.into_values().collect(),
        skipped,
    }
}

fn excluded_col(column: telemetry_session::Column) -> SimpleExpr {
    Expr::col((Alias::new("excluded"), column)).into()
}

// Inside `ON CONFLICT DO UPDATE` a bare column name is ambiguous between the
// target row and `excluded`, so the stored side must be table-qualified.
fn stored_col(column: telemetry_session::Column) -> SimpleExpr {
    Expr::col((telemetry_session::Entity, column)).into()
}

fn status_rank_expr(column: SimpleExpr) -> SimpleExpr {
    use sea_orm::sea_query::ExprTrait;

    SESSION_STATUS_PRECEDENCE
        .iter()
        .fold(CaseStatement::new(), |case, (status, rank)| {
            case.case(column.clone().eq(Expr::value(*status)), *rank)
        })
        .finally(0)
        .into()
}

/// Upserts by session id and never downgrades an outcome or shortens a
/// duration, so an out-of-order "ok" heartbeat can not erase a crash reported
/// by the same install. `GREATEST` ignores NULL operands on Postgres and
/// CockroachDB, so a session without a duration keeps the stored one.
fn session_on_conflict() -> OnConflict {
    use sea_orm::sea_query::ExprTrait;

    let incoming = status_rank_expr(excluded_col(telemetry_session::Column::Status));
    let stored = status_rank_expr(stored_col(telemetry_session::Column::Status));
    let status: SimpleExpr = Expr::case(
        Expr::expr(incoming).gte(stored),
        excluded_col(telemetry_session::Column::Status),
    )
    .finally(stored_col(telemetry_session::Column::Status))
    .into();

    OnConflict::column(telemetry_session::Column::Id)
        .value(telemetry_session::Column::Status, status)
        .value(
            telemetry_session::Column::DurationMs,
            Func::cust(Alias::new("GREATEST")).args([
                excluded_col(telemetry_session::Column::DurationMs),
                stored_col(telemetry_session::Column::DurationMs),
            ]),
        )
        .value(
            telemetry_session::Column::Release,
            Func::coalesce([
                excluded_col(telemetry_session::Column::Release),
                stored_col(telemetry_session::Column::Release),
            ]),
        )
        .value(
            telemetry_session::Column::Platform,
            Func::coalesce([
                excluded_col(telemetry_session::Column::Platform),
                stored_col(telemetry_session::Column::Platform),
            ]),
        )
        .update_column(telemetry_session::Column::UpdatedAt)
        .to_owned()
}

async fn persist_sessions<C: ConnectionTrait>(
    db: &C,
    payload: &TelemetrySessionIngestPayload,
    sessions: Vec<ValidatedSession>,
    now: chrono::NaiveDateTime,
) -> Result<usize, DbErr> {
    if let Some(release) = payload.release.as_deref() {
        upsert_release(db, release, &payload.source, now).await?;
    }

    let accepted = sessions.len();
    let models: Vec<telemetry_session::ActiveModel> = sessions
        .into_iter()
        .map(|session| telemetry_session::ActiveModel {
            id: Set(session.session_id),
            anon_id: Set(payload.anon_id.clone()),
            source: Set(payload.source.clone()),
            release: Set(payload.release.clone()),
            platform: Set(payload.platform.clone()),
            status: Set(session.status),
            started_at: Set(session.started_at),
            duration_ms: Set(session.duration_ms),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .collect();

    telemetry_session::Entity::insert_many(models)
        .on_conflict(session_on_conflict())
        .exec_without_returning(db)
        .await?;

    Ok(accepted)
}

/// Anonymous by construction: this handler intentionally never extracts
/// `Extension(AppUser)` and never persists user identity or IP addresses.
#[utoipa::path(
    post,
    path = "/telemetry/sessions",
    tag = "telemetry",
    request_body = TelemetrySessionIngestPayload,
    responses(
        (status = 200, description = "Number of sessions that were accepted and how many were skipped", body = TelemetrySessionIngestResponse),
        (status = 400, description = "Invalid batch"),
        (status = 404, description = "Telemetry is disabled on this platform")
    ),
    description = "Submit a batch of anonymous session outcomes used to compute crash-free rates per release. Sessions whose start is more than 5 minutes ahead of the server clock or more than 90 days old are skipped. No account, user identity or IP address is ever stored — only a random client-generated identifier."
)]
#[tracing::instrument(name = "POST /telemetry/sessions", skip(state, payload))]
pub async fn ingest_sessions(
    State(state): State<AppState>,
    Json(mut payload): Json<TelemetrySessionIngestPayload>,
) -> Result<Json<TelemetrySessionIngestResponse>, ApiError> {
    if !state.platform_config.features.telemetry {
        return Err(ApiError::NOT_FOUND);
    }

    validate_anon_id(&payload.anon_id)?;
    validate_source(&payload.source)?;

    if payload.sessions.len() > MAX_SESSIONS_PER_BATCH {
        return Err(ApiError::bad_request(format!(
            "A telemetry session batch may contain at most {} sessions",
            MAX_SESSIONS_PER_BATCH
        )));
    }

    payload.release = optional_string(payload.release.take(), MAX_RELEASE_LEN);
    payload.platform = optional_string(payload.platform.take(), MAX_SHORT_STRING_LEN);

    let now = chrono::Utc::now().naive_utc();
    let ValidatedBatch {
        sessions: validated,
        skipped,
    } = validate_sessions(std::mem::take(&mut payload.sessions), now);

    if skipped > 0 {
        tracing::debug!(
            source = %payload.source,
            skipped,
            "Skipped telemetry sessions that failed validation"
        );
    }

    if validated.is_empty() {
        return Ok(Json(TelemetrySessionIngestResponse {
            accepted: 0,
            skipped,
        }));
    }

    let sink = sink_from_env();

    if sink == "none" {
        return Ok(Json(TelemetrySessionIngestResponse {
            accepted: validated.len(),
            skipped,
        }));
    }

    if sink == "log" {
        tracing::info!(
            source = %payload.source,
            anon_id = %payload.anon_id,
            release = payload.release.as_deref().unwrap_or(""),
            platform = payload.platform.as_deref().unwrap_or(""),
            sessions = validated.len(),
            skipped,
            statuses = ?validated.iter().map(|session| session.status.as_str()).collect::<Vec<_>>(),
            "telemetry session batch"
        );
        return Ok(Json(TelemetrySessionIngestResponse {
            accepted: validated.len(),
            skipped,
        }));
    }

    match persist_sessions(&state.db, &payload, validated, now).await {
        Ok(accepted) => Ok(Json(TelemetrySessionIngestResponse { accepted, skipped })),
        Err(e) => {
            tracing::error!("Failed to persist telemetry session batch: {}", e);
            Ok(Json(TelemetrySessionIngestResponse {
                accepted: 0,
                skipped,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::QueryTrait;

    fn session(session_id: &str, status: &str, started_at: &str) -> TelemetrySessionPayload {
        TelemetrySessionPayload {
            session_id: session_id.to_string(),
            status: status.to_string(),
            started_at: started_at.to_string(),
            duration_ms: None,
        }
    }

    const START: &str = "2026-07-26T10:00:00Z";
    const NOW: &str = "2026-07-26T12:00:00Z";

    fn now() -> chrono::NaiveDateTime {
        parse_client_ts(Some(NOW)).unwrap()
    }

    /// An RFC 3339 start `offset` away from the fixed test clock.
    fn offset_start(offset: chrono::Duration) -> String {
        (now() + offset).and_utc().to_rfc3339()
    }

    fn validated(session: TelemetrySessionPayload) -> Option<ValidatedSession> {
        validate_session(session, now())
    }

    #[test]
    fn severity_ladder_is_ordered() {
        assert!(status_rank("ok") < status_rank("errored"));
        assert!(status_rank("errored") < status_rank("abnormal"));
        assert!(status_rank("abnormal") < status_rank("crashed"));
        assert_eq!(status_rank("nonsense"), status_rank("ok"));
    }

    #[test]
    fn a_crash_is_never_downgraded() {
        for later in ["ok", "errored", "abnormal"] {
            assert_eq!(merge_status("crashed", later), "crashed");
        }
        assert_eq!(merge_status("crashed", "crashed"), "crashed");
    }

    #[test]
    fn later_status_wins_when_it_is_at_least_as_severe() {
        assert_eq!(merge_status("ok", "errored"), "errored");
        assert_eq!(merge_status("ok", "crashed"), "crashed");
        assert_eq!(merge_status("errored", "abnormal"), "abnormal");
        assert_eq!(merge_status("abnormal", "errored"), "abnormal");
        assert_eq!(merge_status("ok", "ok"), "ok");
    }

    #[test]
    fn duplicates_in_a_batch_merge_by_precedence() {
        let batch = validate_sessions(
            vec![
                session("s1", "ok", START),
                session("s1", "crashed", START),
                session("s1", "ok", START),
                session("s2", "ok", START),
            ],
            now(),
        );
        let merged = batch.sessions;
        assert_eq!(batch.skipped, 0);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].session_id, "s1");
        assert_eq!(merged[0].status, "crashed");
        assert_eq!(merged[1].status, "ok");
    }

    #[test]
    fn merging_keeps_the_earliest_start_and_longest_duration() {
        let mut first = validated(TelemetrySessionPayload {
            duration_ms: Some(500),
            ..session("s1", "ok", "2026-07-26T10:00:05Z")
        })
        .unwrap();
        let second = validated(TelemetrySessionPayload {
            duration_ms: Some(120),
            ..session("s1", "errored", START)
        })
        .unwrap();
        first.merge(second);
        assert_eq!(first.status, "errored");
        assert_eq!(first.duration_ms, Some(500));
        assert_eq!(first.started_at, parse_client_ts(Some(START)).unwrap());
    }

    #[test]
    fn drops_sessions_with_an_unknown_status() {
        assert!(validated(session("s1", "weird", START)).is_none());
        assert!(validated(session("s1", " CRASHED ", START)).is_some());
    }

    #[test]
    fn drops_sessions_with_an_invalid_id_or_start() {
        assert!(validated(session("", "ok", START)).is_none());
        assert!(validated(session(&"s".repeat(MAX_SESSION_ID_LEN + 1), "ok", START)).is_none());
        assert!(validated(session("s1", "ok", "yesterday")).is_none());
    }

    #[test]
    fn drops_starts_beyond_the_clock_skew_tolerance() {
        let tolerance = chrono::Duration::minutes(MAX_CLOCK_SKEW_MINUTES);
        assert!(validated(session("s1", "ok", &offset_start(tolerance))).is_some());
        assert!(
            validated(session(
                "s1",
                "ok",
                &offset_start(tolerance - chrono::Duration::seconds(1))
            ))
            .is_some()
        );
        assert!(
            validated(session(
                "s1",
                "ok",
                &offset_start(tolerance + chrono::Duration::seconds(1))
            ))
            .is_none()
        );
        assert!(
            validated(session(
                "s1",
                "ok",
                &offset_start(chrono::Duration::days(1))
            ))
            .is_none()
        );
        assert!(
            validated(session(
                "s1",
                "ok",
                &offset_start(chrono::Duration::days(3650))
            ))
            .is_none()
        );
    }

    #[test]
    fn drops_starts_older_than_the_retention_horizon() {
        let horizon = chrono::Duration::days(MAX_SESSION_AGE_DAYS);
        assert!(validated(session("s1", "ok", &offset_start(-horizon))).is_some());
        assert!(
            validated(session(
                "s1",
                "ok",
                &offset_start(-horizon + chrono::Duration::seconds(1))
            ))
            .is_some()
        );
        assert!(
            validated(session(
                "s1",
                "ok",
                &offset_start(-horizon - chrono::Duration::seconds(1))
            ))
            .is_none()
        );
        assert!(
            validated(session(
                "s1",
                "ok",
                &offset_start(-chrono::Duration::days(365))
            ))
            .is_none()
        );
    }

    #[test]
    fn a_batch_reports_how_many_sessions_it_dropped() {
        let batch = validate_sessions(
            vec![
                session("s1", "ok", START),
                session("s2", "ok", &offset_start(chrono::Duration::days(1))),
                session("s3", "ok", &offset_start(-chrono::Duration::days(120))),
                session("s4", "weird", START),
                session("s5", "ok", "yesterday"),
            ],
            now(),
        );
        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].session_id, "s1");
        assert_eq!(batch.skipped, 4);
    }

    #[test]
    fn clamps_durations_into_the_stored_range() {
        let negative = validated(TelemetrySessionPayload {
            duration_ms: Some(-1),
            ..session("s1", "ok", START)
        })
        .unwrap();
        assert_eq!(negative.duration_ms, None);

        let huge = validated(TelemetrySessionPayload {
            duration_ms: Some(i64::MAX),
            ..session("s1", "ok", START)
        })
        .unwrap();
        assert_eq!(huge.duration_ms, Some(i32::MAX));
    }

    #[test]
    fn upsert_sql_never_downgrades_a_stored_status() {
        let model = telemetry_session::ActiveModel {
            id: Set("s1".to_string()),
            anon_id: Set("anon".to_string()),
            source: Set("desktop".to_string()),
            release: Set(Some("1.2.3".to_string())),
            platform: Set(Some("macos".to_string())),
            status: Set("ok".to_string()),
            started_at: Set(parse_client_ts(Some(START)).unwrap()),
            duration_ms: Set(Some(10)),
            created_at: Set(parse_client_ts(Some(START)).unwrap()),
            updated_at: Set(parse_client_ts(Some(START)).unwrap()),
        };
        let sql = telemetry_session::Entity::insert(model)
            .on_conflict(session_on_conflict())
            .build(sea_orm::DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains(r#"ON CONFLICT ("id") DO UPDATE SET"#),
            "{}",
            sql
        );
        assert!(
            sql.contains(r#"WHEN ("excluded"."status" = 'crashed') THEN 3"#),
            "{}",
            sql
        );
        assert!(
            sql.contains(r#"WHEN ("TelemetrySession"."status" = 'crashed') THEN 3"#),
            "{}",
            sql
        );
        assert!(
            sql.contains(r#"THEN "excluded"."status" ELSE "TelemetrySession"."status" END)"#),
            "{}",
            sql
        );
        assert!(
            sql.contains(
                r#""durationMs" = GREATEST("excluded"."durationMs", "TelemetrySession"."durationMs")"#
            ),
            "{}",
            sql
        );
        // Postgres rejects a bare column in DO UPDATE SET as ambiguous with `excluded`.
        assert!(
            !sql.contains(r#"ELSE "status" END"#) && !sql.contains(r#", "durationMs")"#),
            "{}",
            sql
        );
        assert!(
            sql.contains(r#""updatedAt" = "excluded"."updatedAt""#),
            "{}",
            sql
        );
        assert!(
            !sql.contains(r#""startedAt" = "excluded"."startedAt""#),
            "{}",
            sql
        );
    }

    #[test]
    fn merges_the_full_batch_cap_without_losing_a_crash() {
        let mut batch: Vec<TelemetrySessionPayload> = (0..MAX_SESSIONS_PER_BATCH - 1)
            .map(|index| session(&format!("s{}", index), "ok", START))
            .collect();
        batch.insert(0, session("s0", "crashed", START));
        let merged = validate_sessions(batch, now()).sessions;
        assert_eq!(merged.len(), MAX_SESSIONS_PER_BATCH - 1);
        assert_eq!(merged[0].session_id, "s0");
        assert_eq!(merged[0].status, "crashed");
    }
}
