//! Regression-suite surfaces: corpus listing, fixture promotion, suite config.
//!
//! The corpus is the plaintext `LogMeta.payload` column of the event board's
//! Lance runs table — never the encrypted `ExecutionRun.inputPayloadKey`
//! blobs. Storage authority split (do not add dual writes): the bucket under
//! `apps/{app_id}/regression/` owns the suite config + fixtures (via the core
//! helpers, shared with desktop); the Postgres `RegressionSuite` row is a
//! *projection* written by the same PUT; suite runs live in
//! `RegressionSuiteRun` + `RegressionCaseResult` only.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use flow_like::app::App;
use flow_like::flow::event::Event as CoreEvent;
use flow_like::flow::execution::{LogLevel, LogMeta, StoredLogMeta};
use flow_like::flow::regression::{
    CAVEAT_CALLER_OAUTH_TOKENS, CAVEAT_GRADING_BLIND, CorpusCandidate, FIXTURE_PAYLOAD_CAP_BYTES,
    FixtureBaseline, GateMode, REPLAY_EXCLUSION_SUITE_RUNS, RegressionFixture,
    RegressionSuite as CoreRegressionSuite, SUITE_CASE_CAP, TestVerdict, drop_raw_body_duplicates,
    error_class_of, grade_run, payload_preview, prepare_fixture_payload, redact_by_key_name,
    select_corpus_window, shape_hash,
};
use flow_like_storage::arrow_array::RecordBatch;
use flow_like_storage::lancedb::query::{ExecutableQuery, QueryBase};
use flow_like_storage::serde_arrow;
use flow_like_types::{Value, anyhow, create_id};
use futures::TryStreamExt;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    ensure_permission,
    entity::{
        event_sink, execution_run, regression_case_result, regression_suite, regression_suite_run,
    },
    error::ApiError,
    execution::regression::{
        CandidateVersion, SuiteRunActor, SuiteRunTrigger, collect_lance_evidence, datetime_micros,
        gate_mode_as_str, load_core_suite, open_runs_db, parse_gate_mode, spawn_suite_run,
    },
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};

use super::db::get_event_from_db_opt;
use super::get_event::map_missing_event_artifact;
use super::get_event_runs::is_safe_id;

const DEFAULT_CORPUS_LIMIT: usize = 25;

/// Corpus-entry caveat: the recorded row is a rejected trigger (`start == end`
/// and an empty visited-node set — the two markers `RejectedRun` writes).
const CAVEAT_REJECTED: &str = "rejected";
/// Corpus-entry caveat: the redacted payload exceeds the fixture cap, so this
/// run cannot be promoted.
const CAVEAT_TOO_LARGE: &str = "too_large";
/// Corpus-entry caveat: the run recorded no payload.
const CAVEAT_EMPTY: &str = "empty";

/// Belt-and-braces bound on how many replay-tagged `ExecutionRun` ids feed the
/// corpus exclusion set — mirrors the primary exclusion's bound of
/// [`REPLAY_EXCLUSION_SUITE_RUNS`] suite runs × [`SUITE_CASE_CAP`] cases.
const REGRESSION_RUN_ID_SCAN_CAP: u64 = (REPLAY_EXCLUSION_SUITE_RUNS * SUITE_CASE_CAP) as u64;

/// Suite runs returned by the history listing.
const SUITE_RUN_HISTORY_CAP: u64 = 50;

fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or_default()
}

/// Board version label on a run row (`v{major}-{minor}-{patch}`; event rows
/// occasionally store dotted/underscore labels). `None` for etag-bound or
/// unparseable labels — callers fall back to the live board.
fn parse_run_version_label(label: &str) -> Option<(u32, u32, u32)> {
    let trimmed = label.trim();
    let parts: Vec<&str> = match trimmed.strip_prefix('v') {
        Some(rest) => rest.split('-').collect(),
        None => trimmed.split(['.', '_']).collect(),
    };
    match parts.as_slice() {
        [major, minor, patch] => Some((
            major.parse().ok()?,
            minor.parse().ok()?,
            patch.parse().ok()?,
        )),
        _ => None,
    }
}

/// Page-target and `ontology_action` events are excluded from regression
/// suites entirely: page payloads are sealed to their page session and cannot
/// be replayed, and ontology actions are generated machinery with a governed
/// endpoint of their own.
fn ensure_regression_capable_event(event: &CoreEvent) -> Result<(), ApiError> {
    if event.default_page_id.is_some() {
        return Err(ApiError::bad_request(
            "Page events are excluded from regression suites — their payloads are sealed to the page session and cannot be replayed",
        ));
    }
    if event.event_type == "ontology_action" {
        return Err(ApiError::bad_request(
            "Ontology action events are excluded from regression suites",
        ));
    }
    Ok(())
}

async fn resolve_event(
    state: &AppState,
    app: &App,
    app_id: &str,
    event_id: &str,
) -> Result<CoreEvent, ApiError> {
    let event = match get_event_from_db_opt(&state.db, event_id, app_id).await? {
        Some(event) => event,
        None => app
            .get_event(event_id, None)
            .await
            .map_err(|error| map_missing_event_artifact(event_id, error))?,
    };
    Ok(event)
}

/// The event's current board — the one board the corpus and fixture routes
/// read from and promote against.
fn event_board_id(app: &App, event: &CoreEvent) -> Result<String, ApiError> {
    let board_id = event.board_id.clone();
    if board_id.is_empty() {
        return Err(ApiError::bad_request(
            "Event has no board target; regression suites need a board to replay into",
        ));
    }
    if !is_safe_id(&board_id) {
        return Err(ApiError::bad_request(
            "Board IDs may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    if !app.boards.contains(&board_id) {
        return Err(ApiError::bad_request(format!(
            "Board {board_id} does not belong to this app"
        )));
    }
    Ok(board_id)
}

/// The replay-exclusion set: every case replay run id of the last
/// [`REPLAY_EXCLUSION_SUITE_RUNS`] suite runs, plus — belt and braces, since
/// the DB handle is already held — every run row tagged with a
/// `regressionRunId`. Without it, "newest per shape" dedupe preferentially
/// keeps a nightly suite's own replays and the suite converges on testing
/// itself.
async fn replay_exclusion_set(
    db: &sea_orm::DatabaseConnection,
    app_id: &str,
    board_id: &str,
    suite_id: Option<&str>,
) -> Result<HashSet<String>, ApiError> {
    let mut excluded = HashSet::new();

    if let Some(suite_id) = suite_id {
        let suite_run_ids: Vec<String> = regression_suite_run::Entity::find()
            .select_only()
            .column(regression_suite_run::Column::Id)
            .filter(regression_suite_run::Column::AppId.eq(app_id))
            .filter(regression_suite_run::Column::SuiteId.eq(suite_id))
            .order_by_desc(regression_suite_run::Column::CreatedAt)
            .limit(REPLAY_EXCLUSION_SUITE_RUNS as u64)
            .into_tuple()
            .all(db)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to load suite runs: {e}")))?;
        if !suite_run_ids.is_empty() {
            let replay_ids: Vec<String> = regression_case_result::Entity::find()
                .select_only()
                .column(regression_case_result::Column::ReplayRunId)
                .filter(regression_case_result::Column::SuiteRunId.is_in(suite_run_ids))
                .filter(regression_case_result::Column::ReplayRunId.is_not_null())
                .into_tuple()
                .all(db)
                .await
                .map_err(|e| {
                    ApiError::internal_error(anyhow!("Failed to load suite case results: {e}"))
                })?;
            excluded.extend(replay_ids);
        }
    }

    let tagged: Vec<String> = execution_run::Entity::find()
        .select_only()
        .column(execution_run::Column::Id)
        .filter(execution_run::Column::AppId.eq(app_id))
        .filter(execution_run::Column::BoardId.eq(board_id))
        .filter(execution_run::Column::RegressionRunId.is_not_null())
        .order_by_desc(execution_run::Column::CreatedAt)
        .limit(REGRESSION_RUN_ID_SCAN_CAP)
        .into_tuple()
        .all(db)
        .await
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to load regression-tagged runs: {e}"))
        })?;
    excluded.extend(tagged);

    Ok(excluded)
}

async fn find_suite_row(
    db: &sea_orm::DatabaseConnection,
    app_id: &str,
    event_id: &str,
) -> Result<Option<regression_suite::Model>, ApiError> {
    regression_suite::Entity::find()
        .filter(regression_suite::Column::AppId.eq(app_id))
        .filter(regression_suite::Column::EventId.eq(event_id))
        .one(db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to load regression suite row: {e}")))
}

fn require_suite_row(
    row: Option<regression_suite::Model>,
) -> Result<regression_suite::Model, ApiError> {
    row.ok_or_else(|| {
        ApiError::not_found(
            "No regression suite is configured for this event — save one first via PUT .../regression/suite",
        )
    })
}

/// The two markers `RejectedRun::log_meta` writes: a zero-length duration and
/// an empty visited-node set. Every executed run visits at least one node.
fn is_rejected_summary(start: u64, end: u64, visited_nodes: usize) -> bool {
    start == end && visited_nodes == 0
}

fn payload_value(payload: &[u8]) -> Value {
    if payload.is_empty() {
        Value::Null
    } else {
        flow_like_types::json::from_slice(payload).unwrap_or(Value::Null)
    }
}

/// Whether the event's recorded runs carried caller OAuth tokens (the sink
/// stores them per event). Such tokens are per-caller and not part of any
/// fixture, so replays diverge for reasons unrelated to the board — a suite
/// containing such a fixture cannot be scheduled.
async fn event_carries_caller_oauth(
    db: &sea_orm::DatabaseConnection,
    app_id: &str,
    event_id: &str,
) -> Result<bool, ApiError> {
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(app_id))
        .filter(event_sink::Column::EventId.eq(event_id))
        .one(db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to load event sink: {e}")))?;
    Ok(sink.is_some_and(|sink| sink.oauth_tokens_encrypted.is_some()))
}

// ---------------------------------------------------------------------------
// GET /{event_id}/corpus
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct CorpusQuery {
    /// Maximum corpus entries to return (default 25, capped at 100).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CorpusEntry {
    pub run_id: String,
    /// Unix micros.
    pub start: u64,
    /// Unix micros.
    pub end: u64,
    /// The run's highest log level (3 = error, 4 = fatal).
    pub log_level: u8,
    /// Board version label as stored on the run row (`v{major}-{minor}-{patch}`).
    pub board_version: String,
    /// Dotted event version, when the run recorded one.
    pub event_version: Option<String>,
    /// The node the run was dispatched into — the node a replay must target.
    pub node_id: String,
    /// Raw recorded payload size in bytes, pre-redaction.
    pub payload_len: usize,
    /// Structural hash of the payload shape (key paths + types; values never
    /// enter it) — the shape-dedupe key.
    pub shape_hash: String,
    /// Redacted preview, capped at 2 KiB.
    pub preview: String,
    /// Any of `rejected` (the trigger never executed), `too_large` (redacted
    /// payload exceeds the fixture cap) and `empty` (no payload recorded).
    pub caveats: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventCorpusResponse {
    /// Selected entries, newest first, failing inputs never selected away.
    pub entries: Vec<CorpusEntry>,
    /// The board whose runs table was scanned.
    pub board_id: String,
    /// The scan window the selection was drawn from, in seconds.
    pub window_secs: u64,
    /// Raw rows the final scan returned, pre-refinement.
    pub scanned_rows: usize,
    /// The final scan hit the row cap; the selection is an arbitrary subset
    /// of the window.
    pub scan_capped: bool,
}

struct CorpusRowMeta {
    version: String,
    event_version: Option<String>,
    log_level: u8,
    payload_len: usize,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/corpus",
    tag = "events",
    description = "List recent real inputs recorded for this event, deduplicated by payload shape with failing inputs preserved — the candidates for regression-fixture promotion. Payload previews are redacted.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        CorpusQuery
    ),
    responses(
        (status = 200, description = "Corpus candidates for the event", body = EventCorpusResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/corpus",
    skip(state, user, query)
)]
pub async fn get_event_corpus(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<CorpusQuery>,
) -> Result<Json<EventCorpusResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);
    if !permission.has_permission(RolePermissions::ReadLogs) {
        return Err(ApiError::FORBIDDEN);
    }
    let sub = permission.sub()?;

    if !is_safe_id(&event_id) {
        return Err(ApiError::bad_request(
            "Event ID may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    let target = query
        .limit
        .unwrap_or(DEFAULT_CORPUS_LIMIT)
        .clamp(1, SUITE_CASE_CAP);

    let app = state.master_app(&sub, &app_id, &state).await?;
    let event = resolve_event(&state, &app, &app_id, &event_id).await?;
    ensure_regression_capable_event(&event)?;
    let board_id = event_board_id(&app, &event)?;

    let suite_row = find_suite_row(&state.db, &app_id, &event_id).await?;
    let excluded = replay_exclusion_set(
        &state.db,
        &app_id,
        &board_id,
        suite_row.as_ref().map(|row| row.id.as_str()),
    )
    .await?;

    let (_db, table) = open_runs_db(&state, &sub, &app_id, &board_id).await?;
    let Some(table) = table else {
        // No run has ever flushed for this board — an empty corpus, not an error.
        return Ok(Json(EventCorpusResponse {
            entries: Vec::new(),
            board_id,
            window_secs: 0,
            scanned_rows: 0,
            scan_capped: false,
        }));
    };

    let side_meta: Arc<Mutex<HashMap<String, CorpusRowMeta>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let scan_event_id = event_id.clone();
    let scan_meta = side_meta.clone();
    let selection = select_corpus_window(target, &excluded, move |window: Duration, cap: usize| {
        let table = table.clone();
        let event_id = scan_event_id.clone();
        let side_meta = scan_meta.clone();
        async move {
            let cutoff = now_micros().saturating_sub(window.as_micros() as u64);
            let filter = format!("event_id = '{event_id}' AND start >= {cutoff}");
            let batches: Vec<RecordBatch> = table
                .query()
                .only_if(&filter)
                .limit(cap)
                .execute()
                .await
                .map_err(|e| anyhow!("Failed to scan the corpus window: {e}"))?
                .try_collect()
                .await
                .map_err(|e| anyhow!("Failed to collect corpus rows: {e}"))?;
            let mut rows = Vec::new();
            for batch in &batches {
                let stored: Vec<StoredLogMeta> =
                    serde_arrow::from_record_batch(batch).unwrap_or_default();
                for row in stored {
                    side_meta.lock().expect("corpus meta lock").insert(
                        row.run_id.clone(),
                        CorpusRowMeta {
                            version: row.version.clone(),
                            event_version: row.event_version.clone(),
                            log_level: row.log_level,
                            payload_len: row.payload.len(),
                        },
                    );
                    rows.push(CorpusCandidate::from_log_meta(&LogMeta::from(row)));
                }
            }
            Ok(rows)
        }
    })
    .await
    .map_err(ApiError::internal_error)?;

    let metas = side_meta.lock().expect("corpus meta lock");
    let entries = selection
        .candidates
        .iter()
        .map(|candidate| {
            let meta = metas.get(&candidate.run_id);
            let mut caveats = Vec::new();
            if is_rejected_summary(
                candidate.start,
                candidate.end,
                candidate.visited_node_ids.len(),
            ) {
                caveats.push(CAVEAT_REJECTED.to_string());
            }
            if candidate.payload.is_null() {
                caveats.push(CAVEAT_EMPTY.to_string());
            } else {
                let mut redacted = candidate.payload.clone();
                redact_by_key_name(&mut redacted);
                drop_raw_body_duplicates(&mut redacted);
                let redacted_len = flow_like_types::json::to_vec(&redacted)
                    .map(|bytes| bytes.len())
                    .unwrap_or(usize::MAX);
                if redacted_len > FIXTURE_PAYLOAD_CAP_BYTES {
                    caveats.push(CAVEAT_TOO_LARGE.to_string());
                }
            }
            CorpusEntry {
                run_id: candidate.run_id.clone(),
                start: candidate.start,
                end: candidate.end,
                log_level: meta.map(|m| m.log_level).unwrap_or_default(),
                board_version: meta.map(|m| m.version.clone()).unwrap_or_default(),
                event_version: meta.and_then(|m| m.event_version.clone()),
                node_id: candidate.source_node_id.clone(),
                payload_len: meta.map(|m| m.payload_len).unwrap_or_default(),
                shape_hash: shape_hash(&candidate.payload),
                preview: payload_preview(&candidate.payload),
                caveats,
            }
        })
        .collect();

    Ok(Json(EventCorpusResponse {
        entries,
        board_id,
        window_secs: selection.window.as_secs(),
        scanned_rows: selection.scanned_rows,
        scan_capped: selection.scan_capped,
    }))
}

// ---------------------------------------------------------------------------
// GET /{event_id}/corpus/{run_id}/payload
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct CorpusPayloadResponse {
    pub run_id: String,
    /// The node the run was dispatched into — resolve Re-Run against this,
    /// never against the event id the run row carries.
    pub node_id: String,
    pub board_id: String,
    /// The recorded payload, redacted by leaf key name across the whole
    /// document.
    #[schema(value_type = Object)]
    pub payload: Value,
}

/// Load one run's summary row from the runs table, newest first (a double
/// flush writes the same run twice). The `event_id` filter scopes the lookup
/// to the event being queried.
async fn load_corpus_row(
    table: &flow_like_storage::lancedb::Table,
    event_id: &str,
    run_id: &str,
) -> Result<Option<StoredLogMeta>, ApiError> {
    let filter = format!("run_id = '{run_id}' AND event_id = '{event_id}'");
    let batches: Vec<RecordBatch> = table
        .query()
        .only_if(&filter)
        .limit(4)
        .execute()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to query run {run_id}: {e}")))?
        .try_collect()
        .await
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to collect run row for {run_id}: {e}"))
        })?;
    let mut rows: Vec<StoredLogMeta> = Vec::new();
    for batch in &batches {
        rows.extend(
            serde_arrow::from_record_batch::<Vec<StoredLogMeta>>(batch).unwrap_or_default(),
        );
    }
    Ok(rows.into_iter().max_by_key(|row| row.start))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/corpus/{run_id}/payload",
    tag = "events",
    description = "Fetch one recorded run's full input payload (redacted) plus the node it was dispatched into — the data needed to re-run or promote it.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("run_id" = String, Path, description = "Run ID from the corpus listing")
    ),
    responses(
        (status = 200, description = "The redacted payload and its start node", body = CorpusPayloadResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/corpus/{run_id}/payload",
    skip(state, user)
)]
pub async fn get_corpus_payload(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id, run_id)): Path<(String, String, String)>,
) -> Result<Json<CorpusPayloadResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);
    if !permission.has_permission(RolePermissions::ReadLogs) {
        return Err(ApiError::FORBIDDEN);
    }
    let sub = permission.sub()?;

    if !is_safe_id(&event_id) || !is_safe_id(&run_id) {
        return Err(ApiError::bad_request(
            "IDs may only contain alphanumeric characters, '-' and '_'",
        ));
    }

    let app = state.master_app(&sub, &app_id, &state).await?;
    let event = resolve_event(&state, &app, &app_id, &event_id).await?;
    ensure_regression_capable_event(&event)?;
    let board_id = event_board_id(&app, &event)?;

    let (_db, table) = open_runs_db(&state, &sub, &app_id, &board_id).await?;
    let table = table.ok_or_else(|| ApiError::not_found("No runs recorded for this board"))?;
    let row = load_corpus_row(&table, &event_id, &run_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Run {run_id} not found for this event")))?;

    let mut payload = payload_value(&row.payload);
    redact_by_key_name(&mut payload);
    // Raw-body duplicates (body_bytes / body_text) defeat key-name redaction;
    // the parsed `body` object is the redacted, replayable form.
    drop_raw_body_duplicates(&mut payload);

    Ok(Json(CorpusPayloadResponse {
        run_id: row.run_id,
        node_id: row.node_id,
        board_id,
        payload,
    }))
}

// ---------------------------------------------------------------------------
// POST /{event_id}/regression/fixtures
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct PromoteFixtureRequest {
    /// Run id from the corpus listing.
    pub run_id: String,
    /// Optional baseline override: `pass` or `fail`. Omit to grade the
    /// recorded run and use its verdict as the baseline.
    #[serde(default)]
    pub expectation: Option<String>,
    /// Must be `true` to promote a rejected trigger — a run that never
    /// executed a node.
    #[serde(default)]
    pub acknowledge_rejected: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FixtureSummary {
    pub id: String,
    /// The node a replay of this fixture dispatches into.
    pub source_node_id: String,
    pub source_board_id: String,
    /// The verdict recorded at promotion — what replays are compared against.
    #[schema(value_type = Object)]
    pub baseline: FixtureBaseline,
    pub promoted_by: String,
    /// Well-known values: `grading_blind`, `caller_oauth_tokens`.
    pub caveats: Vec<String>,
}

fn fixture_summary(fixture: &RegressionFixture) -> FixtureSummary {
    FixtureSummary {
        id: fixture.id.clone(),
        source_node_id: fixture.source_node_id.clone(),
        source_board_id: fixture.source_board_id.clone(),
        baseline: fixture.baseline.clone(),
        promoted_by: fixture.promoted_by.clone(),
        caveats: fixture.caveats.clone(),
    }
}

/// Whether the recorded run's board (at the recorded version) discarded
/// `ASSERT_OK` markers: `log_level` above `Info` means the baseline cannot
/// justify a green verdict — the fixture is stamped `grading_blind`.
async fn board_grading_blind(app: &App, board_id: &str, version_label: &str) -> bool {
    let version = parse_run_version_label(version_label);
    match app
        .open_board_authoritative(board_id.to_string(), version)
        .await
    {
        Ok(board) => board.lock().await.log_level.to_u8() > LogLevel::Info.to_u8(),
        Err(error) => {
            tracing::warn!(%error, board_id = %board_id, version = %version_label, "Could not load the recorded run's board to check its log level");
            false
        }
    }
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/regression/fixtures",
    tag = "events",
    description = "Promote a recorded run into a regression fixture: its payload is redacted and capped, and the run is graded to capture the baseline verdict future replays are compared against.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = PromoteFixtureRequest,
    responses(
        (status = 200, description = "The promoted fixture", body = FixtureSummary),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Conflict with the suite's schedule")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/events/{event_id}/regression/fixtures",
    skip(state, user, body)
)]
pub async fn promote_regression_fixture(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(body): Json<PromoteFixtureRequest>,
) -> Result<Json<FixtureSummary>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;

    if !is_safe_id(&event_id) || !is_safe_id(&body.run_id) {
        return Err(ApiError::bad_request(
            "IDs may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    let expectation = match body.expectation.as_deref() {
        None => None,
        Some("pass") => Some(TestVerdict::Pass),
        Some("fail") => Some(TestVerdict::Fail),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Unknown expectation '{other}'; expected pass or fail"
            )));
        }
    };

    let app = state.master_app(&sub, &app_id, &state).await?;
    let event = resolve_event(&state, &app, &app_id, &event_id).await?;
    ensure_regression_capable_event(&event)?;
    let board_id = event_board_id(&app, &event)?;

    let suite_row = require_suite_row(find_suite_row(&state.db, &app_id, &event_id).await?)?;
    let suite = load_core_suite(&app, &suite_row).await;
    let fixtures = suite
        .list_fixtures(&app)
        .await
        .map_err(ApiError::internal_error)?;
    if fixtures.len() >= SUITE_CASE_CAP {
        return Err(ApiError::bad_request(format!(
            "This suite already holds {SUITE_CASE_CAP} fixtures — delete one before promoting another run"
        )));
    }

    let caller_oauth = event_carries_caller_oauth(&state.db, &app_id, &event_id).await?;
    if caller_oauth && suite_row.schedule.is_some() {
        return Err(ApiError::conflict(
            "This event's recorded runs carry caller OAuth tokens, which are not part of any fixture — a scheduled suite cannot replay them faithfully. Remove the suite's schedule before promoting this run.",
        ));
    }

    let (db, table) = open_runs_db(&state, &sub, &app_id, &board_id).await?;
    let table = table.ok_or_else(|| ApiError::not_found("No runs recorded for this board"))?;
    let row = load_corpus_row(&table, &event_id, &body.run_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!("Run {} not found for this event", body.run_id))
        })?;

    let visited_node_ids: Vec<String> = row
        .nodes
        .iter()
        .flatten()
        .map(|(node_id, _)| node_id.clone())
        .collect();
    if is_rejected_summary(row.start, row.end, visited_node_ids.len()) && !body.acknowledge_rejected
    {
        return Err(ApiError::bad_request(
            "This run was rejected before execution — its payload never reached the flow. Pass acknowledge_rejected: true to promote it anyway.",
        ));
    }

    let payload = prepare_fixture_payload(payload_value(&row.payload))
        .map_err(|error| ApiError::bad_request(error.to_string()))?;

    // Grade the recorded run from its stored Lance artifacts — the baseline
    // future replays are compared against. A missing or unreadable log table
    // yields an `error` baseline, never a green light.
    let grade = grade_run(collect_lance_evidence(&db, &body.run_id).await);
    let verdict = expectation.unwrap_or(grade.verdict);
    let error_class = if verdict == TestVerdict::Pass {
        None
    } else if verdict == grade.verdict {
        error_class_of(&grade)
    } else {
        // The operator overrode the graded verdict; the graded class does not
        // describe the expected failure.
        None
    };

    let mut caveats = Vec::new();
    if board_grading_blind(&app, &board_id, &row.version).await {
        caveats.push(CAVEAT_GRADING_BLIND.to_string());
    }
    if caller_oauth {
        caveats.push(CAVEAT_CALLER_OAUTH_TOKENS.to_string());
    }

    let fixture = RegressionFixture {
        id: create_id(),
        payload,
        source_node_id: row.node_id.clone(),
        source_board_id: board_id,
        baseline: FixtureBaseline {
            verdict,
            error_class,
            visited_node_ids,
            recorded_at: row.start,
        },
        promoted_by: sub,
        caveats,
    };
    suite
        .save_fixture(&app, &fixture)
        .await
        .map_err(ApiError::internal_error)?;

    Ok(Json(fixture_summary(&fixture)))
}

// ---------------------------------------------------------------------------
// DELETE /{event_id}/regression/fixtures/{fixture_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/events/{event_id}/regression/fixtures/{fixture_id}",
    tag = "events",
    description = "Delete a regression fixture and its stored payload.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("fixture_id" = String, Path, description = "Fixture ID")
    ),
    responses(
        (status = 200, description = "Fixture deleted"),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "DELETE /apps/{app_id}/events/{event_id}/regression/fixtures/{fixture_id}",
    skip(state, user)
)]
pub async fn delete_regression_fixture(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id, fixture_id)): Path<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;

    if !is_safe_id(&event_id) || !is_safe_id(&fixture_id) {
        return Err(ApiError::bad_request(
            "IDs may only contain alphanumeric characters, '-' and '_'",
        ));
    }

    let app = state.master_app(&sub, &app_id, &state).await?;
    let suite_row = require_suite_row(find_suite_row(&state.db, &app_id, &event_id).await?)?;
    let suite = load_core_suite(&app, &suite_row).await;
    suite
        .delete_fixture(&app, &fixture_id)
        .await
        .map_err(|error| {
            ApiError::internal_error(anyhow!("Failed to delete fixture {fixture_id}: {error}"))
        })?;

    Ok(Json(
        flow_like_types::json::json!({ "deleted": fixture_id }),
    ))
}

// ---------------------------------------------------------------------------
// GET + PUT /{event_id}/regression/suite
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct RegressionSuiteResponse {
    /// The suite configuration (bucket authority).
    #[schema(value_type = Object)]
    pub suite: CoreRegressionSuite,
    /// Next scheduled run (RFC 3339, cloud scheduler state), when a schedule
    /// is set.
    pub next_run_at: Option<String>,
    /// Promoted fixtures, without payloads.
    pub fixtures: Vec<FixtureSummary>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutRegressionSuiteRequest {
    /// Run the suite automatically when a new board version is published.
    #[serde(default)]
    pub trigger_on_publish: bool,
    /// Cron expression (5, 6 or 7 fields) for scheduled runs; omit or null to
    /// clear the schedule.
    #[serde(default)]
    pub schedule: Option<String>,
    /// Publish/promote gate behavior: `Off` (default), `Warn` or `Block`.
    #[serde(default)]
    pub gate_mode: Option<String>,
    /// Acknowledgement that replays execute live side effects — outbound HTTP
    /// from native nodes cannot be suppressed. Suite runs are refused while
    /// `false`.
    #[serde(default)]
    pub allow_live_side_effects: bool,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/regression/suite",
    tag = "events",
    description = "Read the event's regression-suite configuration and its promoted fixtures.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "The suite configuration", body = RegressionSuiteResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/regression/suite",
    skip(state, user)
)]
pub async fn get_regression_suite(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
) -> Result<Json<RegressionSuiteResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);
    let sub = permission.sub()?;

    let suite_row = require_suite_row(find_suite_row(&state.db, &app_id, &event_id).await?)?;
    let app = state.master_app(&sub, &app_id, &state).await?;
    let suite = load_core_suite(&app, &suite_row).await;
    let fixtures = suite
        .list_fixtures(&app)
        .await
        .map_err(ApiError::internal_error)?;

    Ok(Json(RegressionSuiteResponse {
        suite,
        next_run_at: suite_row.next_run_at.map(|at| at.and_utc().to_rfc3339()),
        fixtures: fixtures.iter().map(fixture_summary).collect(),
    }))
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/events/{event_id}/regression/suite",
    tag = "events",
    description = "Create or update the event's regression suite: publish trigger, schedule, gate mode and the live-side-effects acknowledgement. Writes the bucket config and the database projection row together.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = PutRegressionSuiteRequest,
    responses(
        (status = 200, description = "The stored suite configuration", body = RegressionSuiteResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Schedule conflicts with a fixture caveat")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/events/{event_id}/regression/suite",
    skip(state, user, body)
)]
pub async fn put_regression_suite(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(body): Json<PutRegressionSuiteRequest>,
) -> Result<Json<RegressionSuiteResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;

    if !is_safe_id(&event_id) {
        return Err(ApiError::bad_request(
            "Event ID may only contain alphanumeric characters, '-' and '_'",
        ));
    }

    let gate_mode = match body.gate_mode.as_deref() {
        None => GateMode::Off,
        Some(raw) => parse_gate_mode(raw).ok_or_else(|| {
            ApiError::bad_request(format!(
                "Unknown gate mode '{raw}'; expected Off, Warn or Block"
            ))
        })?,
    };
    let schedule = body
        .schedule
        .as_deref()
        .map(str::trim)
        .filter(|expr| !expr.is_empty())
        .map(str::to_string);
    let next_run_at = match &schedule {
        Some(expr) => Some(
            flow_like_sinks::scheduler::next_cron_occurrence_utc(expr)
                .map_err(|error| {
                    ApiError::bad_request(format!("Invalid cron schedule '{expr}': {error}"))
                })?
                .naive_utc(),
        ),
        None => None,
    };

    let app = state.master_app(&sub, &app_id, &state).await?;
    let event = resolve_event(&state, &app, &app_id, &event_id).await?;
    ensure_regression_capable_event(&event)?;
    let board_id = event_board_id(&app, &event)?;

    let existing = find_suite_row(&state.db, &app_id, &event_id).await?;
    let suite_id = existing
        .as_ref()
        .map(|row| row.id.clone())
        .unwrap_or_else(create_id);

    let mut fixtures = Vec::new();
    if let Some(row) = &existing {
        let current = load_core_suite(&app, row).await;
        fixtures = current
            .list_fixtures(&app)
            .await
            .map_err(ApiError::internal_error)?;
    }
    if schedule.is_some()
        && fixtures.iter().any(|fixture| {
            fixture
                .caveats
                .iter()
                .any(|c| c == CAVEAT_CALLER_OAUTH_TOKENS)
        })
    {
        return Err(ApiError::conflict(
            "This suite contains fixtures whose recorded runs carried caller OAuth tokens — those tokens are per-caller and not part of the fixture, so a scheduled replay would diverge for reasons unrelated to the board. Remove those fixtures before scheduling.",
        ));
    }

    let now = now_micros();
    let created_at_micros = match &existing {
        Some(row) => match CoreRegressionSuite::load(&app, &row.id).await {
            Ok(stored) => stored.created_at,
            Err(_) => datetime_micros(row.created_at),
        },
        None => now,
    };
    let suite = CoreRegressionSuite {
        id: suite_id.clone(),
        board_id: board_id.clone(),
        event_id: Some(event_id.clone()),
        node_id: event.node_id.clone(),
        trigger_on_publish: body.trigger_on_publish,
        schedule: schedule.clone(),
        gate_mode,
        allow_live_side_effects: body.allow_live_side_effects,
        created_at: created_at_micros,
        updated_at: now,
    };
    suite.save(&app).await.map_err(ApiError::internal_error)?;

    let now_naive = chrono::Utc::now().naive_utc();
    match existing {
        Some(row) => {
            let mut active: regression_suite::ActiveModel = row.into();
            active.board_id = Set(board_id);
            active.node_id = Set(event.node_id.clone());
            active.trigger_on_publish = Set(body.trigger_on_publish);
            active.schedule = Set(schedule);
            active.gate_mode = Set(gate_mode_as_str(gate_mode).to_string());
            active.allow_live_side_effects = Set(body.allow_live_side_effects);
            active.next_run_at = Set(next_run_at);
            active.updated_at = Set(now_naive);
            active.update(&state.db).await.map_err(|e| {
                ApiError::internal_error(anyhow!("Failed to update suite projection row: {e}"))
            })?;
        }
        None => {
            let active = regression_suite::ActiveModel {
                id: Set(suite_id.clone()),
                app_id: Set(app_id.clone()),
                board_id: Set(board_id),
                event_id: Set(Some(event_id.clone())),
                node_id: Set(event.node_id.clone()),
                trigger_on_publish: Set(body.trigger_on_publish),
                schedule: Set(schedule),
                gate_mode: Set(gate_mode_as_str(gate_mode).to_string()),
                allow_live_side_effects: Set(body.allow_live_side_effects),
                next_run_at: Set(next_run_at),
                created_at: Set(now_naive),
                updated_at: Set(now_naive),
            };
            active.insert(&state.db).await.map_err(|e| {
                ApiError::internal_error(anyhow!("Failed to insert suite projection row: {e}"))
            })?;
        }
    }

    Ok(Json(RegressionSuiteResponse {
        suite,
        next_run_at: next_run_at.map(|at| at.and_utc().to_rfc3339()),
        fixtures: fixtures.iter().map(fixture_summary).collect(),
    }))
}

// ---------------------------------------------------------------------------
// POST /{event_id}/regression/run + GET /{event_id}/regression/runs[/{id}]
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct RunRegressionSuiteRequest {
    /// Candidate board version as (major, minor, patch). Omit to run against
    /// the board's newest published version.
    #[serde(default)]
    pub board_version: Option<(u32, u32, u32)>,
    /// When no `board_version` is given, run against the live draft head
    /// instead of the newest published version. Draft runs never feed the
    /// publish/promote gate.
    #[serde(default)]
    pub allow_draft: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RunRegressionSuiteAccepted {
    /// Poll `GET .../regression/runs/{suite_run_id}` for progress.
    pub suite_run_id: String,
    /// `running`, or `queued` on deployments where the maintenance job
    /// executes suite runs.
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuiteRunSummary {
    pub id: String,
    /// Candidate board version (`major.minor.patch`, or `draft`).
    pub board_version: String,
    /// `manual`, `publish` or `schedule`.
    pub trigger: String,
    /// `queued`, `running`, `completed` or `errored`.
    pub status: String,
    pub regressed: i32,
    pub fixed: i32,
    pub still_failing: i32,
    pub ok: i32,
    pub skipped: i32,
    /// RFC 3339.
    pub started_at: Option<String>,
    /// RFC 3339.
    pub completed_at: Option<String>,
    pub error: Option<String>,
    /// RFC 3339.
    pub created_at: String,
}

fn suite_run_summary(row: regression_suite_run::Model) -> SuiteRunSummary {
    SuiteRunSummary {
        id: row.id,
        board_version: row.board_version,
        trigger: row.trigger,
        status: row.status,
        regressed: row.regressed,
        fixed: row.fixed,
        still_failing: row.still_failing,
        ok: row.ok,
        skipped: row.skipped,
        started_at: row.started_at.map(|at| at.and_utc().to_rfc3339()),
        completed_at: row.completed_at.map(|at| at.and_utc().to_rfc3339()),
        error: row.error,
        created_at: row.created_at.and_utc().to_rfc3339(),
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuiteCaseResultView {
    pub id: String,
    /// `recorded_fixture` or `authored_test`.
    pub case_kind: String,
    /// Fixture id (recorded) or start node id (authored).
    pub case_ref: String,
    /// The replay's execution run id; `null` when the case was skipped.
    pub replay_run_id: Option<String>,
    /// `ok`, `regressed`, `still_failing`, `fixed` or `skipped`.
    pub outcome: String,
    /// Raw grader verdict of the replay: `pass`, `fail`, `error` (or
    /// `skipped`).
    pub grade_verdict: String,
    /// Diagnostics: error classes, failed assertions, grading-blind stamp.
    #[schema(value_type = Option<Object>)]
    pub detail: Option<Value>,
    pub duration_ms: Option<i32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuiteRunDetailResponse {
    pub run: SuiteRunSummary,
    pub cases: Vec<SuiteCaseResultView>,
}

/// Start a suite run against a candidate board version.
///
/// On deployments whose detached tasks die with the request — the AWS
/// `lambda_http` API sets `DETACHED_TASKS_UNRELIABLE=1` — the run is inserted
/// `queued` instead of being spawned, and the RegressionSuites maintenance
/// job executes it on its next tick; the 202 then reports `status: queued`.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/regression/run",
    tag = "events",
    description = "Start a regression-suite run against a candidate board version. Returns 202 immediately; poll the runs endpoint for progress. Replays execute live side effects — the suite must have acknowledged that.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = RunRegressionSuiteRequest,
    responses(
        (status = 202, description = "Suite run started", body = RunRegressionSuiteAccepted),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 409, description = "The suite has not acknowledged live side effects")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/events/{event_id}/regression/run",
    skip(state, user, body)
)]
pub async fn run_regression_suite(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(body): Json<RunRegressionSuiteRequest>,
) -> Result<Response, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    if !permission.has_permission(RolePermissions::ReadLogs) {
        // Grading reads the board's run logs; without ReadLogs the runner
        // could not even score its own replays.
        return Err(ApiError::FORBIDDEN);
    }
    let sub = permission.effective_user_id().map_err(|_| {
        ApiError::forbidden("Running a suite requires a caller that is linked to a user account")
    })?;
    let user_context = permission.to_user_context();

    if !is_safe_id(&event_id) {
        return Err(ApiError::bad_request(
            "Event ID may only contain alphanumeric characters, '-' and '_'",
        ));
    }

    let app = state.master_app(&sub, &app_id, &state).await?;
    let event = resolve_event(&state, &app, &app_id, &event_id).await?;
    ensure_regression_capable_event(&event)?;
    event_board_id(&app, &event)?;

    let suite_row = require_suite_row(find_suite_row(&state.db, &app_id, &event_id).await?)?;
    let suite = load_core_suite(&app, &suite_row).await;

    let candidate = match body.board_version {
        Some(version) => CandidateVersion::Pinned(version),
        None if body.allow_draft => CandidateVersion::Draft,
        None => CandidateVersion::LatestPublished,
    };

    let (suite_run_id, status) = spawn_suite_run(
        state,
        app_id,
        suite,
        candidate,
        SuiteRunTrigger::Manual,
        SuiteRunActor {
            sub: Some(sub),
            user_context: Some(user_context),
        },
    )
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(RunRegressionSuiteAccepted {
            suite_run_id,
            status: status.to_string(),
        }),
    )
        .into_response())
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/regression/runs",
    tag = "events",
    description = "List the event's regression-suite runs, newest first.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Suite runs, newest first", body = Vec<SuiteRunSummary>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/regression/runs",
    skip(state, user)
)]
pub async fn list_regression_runs(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
) -> Result<Json<Vec<SuiteRunSummary>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);

    let suite_row = require_suite_row(find_suite_row(&state.db, &app_id, &event_id).await?)?;
    let rows = regression_suite_run::Entity::find()
        .filter(regression_suite_run::Column::AppId.eq(&app_id))
        .filter(regression_suite_run::Column::SuiteId.eq(&suite_row.id))
        .order_by_desc(regression_suite_run::Column::CreatedAt)
        .limit(SUITE_RUN_HISTORY_CAP)
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to list suite runs: {e}")))?;

    Ok(Json(rows.into_iter().map(suite_run_summary).collect()))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/regression/runs/{suite_run_id}",
    tag = "events",
    description = "Read one regression-suite run with its per-case verdicts.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("suite_run_id" = String, Path, description = "Suite run ID")
    ),
    responses(
        (status = 200, description = "The suite run and its case results", body = SuiteRunDetailResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/regression/runs/{suite_run_id}",
    skip(state, user)
)]
pub async fn get_regression_run(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id, suite_run_id)): Path<(String, String, String)>,
) -> Result<Json<SuiteRunDetailResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);
    if !permission.has_permission(RolePermissions::ReadLogs) {
        // Case details carry log-derived strings (error classes, failed
        // assertions), so this route gates like every log-reading surface.
        return Err(ApiError::FORBIDDEN);
    }

    let suite_row = require_suite_row(find_suite_row(&state.db, &app_id, &event_id).await?)?;
    let run = regression_suite_run::Entity::find_by_id(&suite_run_id)
        .filter(regression_suite_run::Column::AppId.eq(&app_id))
        .filter(regression_suite_run::Column::SuiteId.eq(&suite_row.id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to load suite run: {e}")))?
        .ok_or_else(|| ApiError::not_found(format!("Suite run {suite_run_id} not found")))?;

    let cases = regression_case_result::Entity::find()
        .filter(regression_case_result::Column::SuiteRunId.eq(&suite_run_id))
        .order_by_asc(regression_case_result::Column::CreatedAt)
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to load case results: {e}")))?;

    Ok(Json(SuiteRunDetailResponse {
        run: suite_run_summary(run),
        cases: cases
            .into_iter()
            .map(|case| SuiteCaseResultView {
                id: case.id,
                case_kind: case.case_kind,
                case_ref: case.case_ref,
                replay_run_id: case.replay_run_id,
                outcome: case.outcome,
                grade_verdict: case.grade_verdict,
                detail: case.detail,
                duration_ms: case.duration_ms,
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_version_labels_parse_all_three_formats() {
        assert_eq!(parse_run_version_label("v1-2-3"), Some((1, 2, 3)));
        assert_eq!(parse_run_version_label("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_run_version_label("1_2_3"), Some((1, 2, 3)));
        assert_eq!(parse_run_version_label("etag:abc"), None);
        assert_eq!(parse_run_version_label(""), None);
    }

    #[test]
    fn rejected_summaries_are_the_two_documented_markers() {
        assert!(is_rejected_summary(100, 100, 0));
        assert!(!is_rejected_summary(100, 200, 0));
        assert!(!is_rejected_summary(100, 100, 1));
    }
}
