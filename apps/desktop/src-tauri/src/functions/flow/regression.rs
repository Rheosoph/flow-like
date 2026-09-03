//! Desktop regression suites (Track D, lane F): corpus listing, fixture
//! promotion and suite config over the local Lance runs tables and the local
//! meta store, via the core `flow::regression` helpers — the same bucket
//! layout as cloud (`apps/{app_id}/regression/`).
//!
//! The desktop split: no scheduling, no publish gate, and the suite RUNNER is
//! client-side TS (`components/tauri-provider/regression-runner.ts`) — cases
//! execute through the ordinary local `execute_board` path, so desktop
//! replays are fully LIVE runs with no shadow isolation at all. These
//! commands only plan cases, grade baselines and persist the JSON run
//! archives (newest [`flow_like::flow::regression::DESKTOP_RUN_ARCHIVE_CAP`],
//! the desktop's one store for suite runs — no Postgres rows here).

use flow_like::{
    app::App,
    flow::{
        event::Event,
        execution::{LogLevel, LogMeta, StoredLogMeta},
        regression::{
            CAVEAT_GRADING_BLIND, CorpusCandidate, FIXTURE_PAYLOAD_CAP_BYTES, FixtureBaseline,
            GateMode, RegressionFixture, RegressionSuite, RunGradeEvidence, SUITE_CASE_CAP,
            SuiteCase, TestVerdict, drop_raw_body_duplicates, error_class_of, grade_run, payload_preview,
            plan_suite_cases, prepare_fixture_payload, redact_by_key_name, select_corpus_window,
            shape_hash,
        },
    },
    flow_like_storage::{
        lancedb::query::{ExecutableQuery, QueryBase},
        serde_arrow,
    },
    state::FlowLikeState,
};
use flow_like_types::{Value, create_id};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

use super::event::is_safe_id;
use crate::{functions::TauriFunctionError, state::TauriFlowLikeState};

const DEFAULT_CORPUS_LIMIT: usize = 25;

/// Corpus-entry caveats — the same literals the cloud route serves.
const CAVEAT_REJECTED: &str = "rejected";
const CAVEAT_TOO_LARGE: &str = "too_large";
const CAVEAT_EMPTY: &str = "empty";

/// Log rows fetched per grading query — the exact limits of the grading twins
/// (`collectRunEvidence` in board-tests.ts and the cloud runner), so every
/// grader sees the same evidence.
const GRADE_ASSERT_LOG_LIMIT: usize = 100;
const GRADE_ERROR_LOG_LIMIT: usize = 10;

/// Fixtures promoted on this device: local runs are owner-equivalent and the
/// desktop has no stable subject to record.
const LOCAL_PROMOTER: &str = "local";

fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or_default()
}

/// Board version label on a run row (`v{major}-{minor}-{patch}`; event rows
/// occasionally store dotted/underscore labels). `None` for etag-bound or
/// unparseable labels — callers fall back to the live board. Copy of the
/// cloud route's parser (the desktop cannot depend on `packages/api`).
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

/// Page-target and `ontology_action` events are excluded from regression
/// suites entirely — the same rule as the cloud route.
fn ensure_regression_capable_event(event: &Event) -> Result<(), TauriFunctionError> {
    if event.default_page_id.is_some() {
        return Err(TauriFunctionError::new(
            "Page events are excluded from regression suites — their payloads are sealed to the page session and cannot be replayed",
        ));
    }
    if event.event_type == "ontology_action" {
        return Err(TauriFunctionError::new(
            "Ontology action events are excluded from regression suites",
        ));
    }
    Ok(())
}

fn event_board_id(app: &App, event: &Event) -> Result<String, TauriFunctionError> {
    let board_id = event.board_id.clone();
    if board_id.is_empty() {
        return Err(TauriFunctionError::new(
            "Event has no board target; regression suites need a board to replay into",
        ));
    }
    if !is_safe_id(&board_id) {
        return Err(TauriFunctionError::new(
            "Board IDs may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    if !app.boards.contains(&board_id) {
        return Err(TauriFunctionError::new(&format!(
            "Board {board_id} does not belong to this app"
        )));
    }
    Ok(board_id)
}

async fn load_app(handler: &AppHandle, app_id: &str) -> Result<App, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(handler).await?;
    App::load(app_id.to_string(), flow_like_state)
        .await
        .map_err(|_| TauriFunctionError::new("App not found"))
}

/// The desktop has no projection row — resolve an event's suite by loading
/// the app's stored suite configs (bounded: a handful per app).
async fn find_suite_for_event(
    app: &App,
    event_id: &str,
) -> Result<Option<RegressionSuite>, TauriFunctionError> {
    for suite_id in RegressionSuite::list_suite_ids(app).await? {
        match RegressionSuite::load(app, &suite_id).await {
            Ok(suite) if suite.event_id.as_deref() == Some(event_id) => return Ok(Some(suite)),
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, suite_id = %suite_id, "Failed to load a stored regression suite; skipping");
            }
        }
    }
    Ok(None)
}

fn require_suite(suite: Option<RegressionSuite>) -> Result<RegressionSuite, TauriFunctionError> {
    suite.ok_or_else(|| {
        TauriFunctionError::new(
            "No regression suite is configured for this event — save one first from the event's Quality section",
        )
    })
}

/// Open the board's local Lance `runs` summary table; `None` when no run has
/// ever flushed for the board.
async fn open_runs_table_opt(
    state: &Arc<FlowLikeState>,
    app_id: &str,
    board_id: &str,
) -> flow_like_types::Result<Option<flow_like::flow_like_storage::lancedb::Table>> {
    let db = super::run::open_runs_db(state, app_id, board_id).await?;
    let table_names = db.table_names().execute().await.map_err(|e| {
        flow_like_types::anyhow!("Failed to list run tables for board {board_id}: {e}")
    })?;
    if !table_names.iter().any(|name| name == "runs") {
        return Ok(None);
    }
    let table = db.open_table("runs").execute().await.map_err(|e| {
        flow_like_types::anyhow!("Failed to open runs table for board {board_id}: {e}")
    })?;
    Ok(Some(table))
}

/// The replay-exclusion set: every case replay run id recorded in the suite's
/// archived runs (the archive is pruned to the newest
/// [`flow_like::flow::regression::DESKTOP_RUN_ARCHIVE_CAP`], which is exactly
/// the exclusion window). Without it, "newest per shape" dedupe preferentially
/// keeps the suite's own replays and the suite converges on testing itself.
/// The desktop has no run database, so there is no belt-and-braces
/// `regressionRunId` filter here — the archive is the only source.
async fn replay_exclusion_set(app: &App, suite: Option<&RegressionSuite>) -> HashSet<String> {
    let mut excluded = HashSet::new();
    let Some(suite) = suite else {
        return excluded;
    };
    let ids = match RegressionSuite::list_run_archive_ids(app, &suite.id).await {
        Ok(ids) => ids,
        Err(error) => {
            tracing::warn!(%error, suite_id = %suite.id, "Failed to list suite-run archives for the replay exclusion set");
            return excluded;
        }
    };
    for suite_run_id in ids {
        match RegressionSuite::load_run_archive::<SuiteRunArchive>(app, &suite.id, &suite_run_id)
            .await
        {
            Ok(archive) => {
                excluded.extend(
                    archive
                        .cases
                        .into_iter()
                        .filter_map(|case| case.replay_run_id),
                );
            }
            Err(error) => {
                tracing::warn!(%error, suite_run_id = %suite_run_id, "Failed to load a suite-run archive for the replay exclusion set");
            }
        }
    }
    excluded
}

/// Load one run's summary row from the runs table, newest first (a double
/// flush writes the same run twice). The `event_id` filter scopes the lookup
/// to the event being queried.
async fn load_corpus_row(
    table: &flow_like::flow_like_storage::lancedb::Table,
    event_id: &str,
    run_id: &str,
) -> Result<Option<StoredLogMeta>, TauriFunctionError> {
    let filter = format!("run_id = '{run_id}' AND event_id = '{event_id}'");
    let batches = table
        .query()
        .only_if(&filter)
        .limit(4)
        .execute()
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to query run {run_id}: {e}"))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to collect run row for {run_id}: {e}"))?;
    let mut rows: Vec<StoredLogMeta> = Vec::new();
    for batch in &batches {
        rows.extend(
            serde_arrow::from_record_batch::<Vec<StoredLogMeta>>(batch).unwrap_or_default(),
        );
    }
    Ok(rows.into_iter().max_by_key(|row| row.start))
}

/// Gather grading evidence for a recorded run from the local log store: one
/// `ASSERT_%` query and one error-log query on the run's own log table (the
/// grading twins' shape). A missing or unreadable log table sets
/// `log_query_failed`, so the grade becomes `error` — never a green light.
async fn collect_local_evidence(state: &Arc<FlowLikeState>, meta: &LogMeta) -> RunGradeEvidence {
    let mut evidence = RunGradeEvidence {
        metadata_present: true,
        ..RunGradeEvidence::default()
    };
    match state
        .query_run(
            meta,
            "message LIKE 'ASSERT_%'",
            Some(GRADE_ASSERT_LOG_LIMIT),
            Some(0),
        )
        .await
    {
        Ok(messages) => {
            evidence.assert_logs = messages.into_iter().map(|log| log.message).collect()
        }
        Err(error) => {
            tracing::warn!(%error, run_id = %meta.run_id, "Assert-log query failed while grading");
            evidence.log_query_failed = true;
        }
    }
    match state
        .query_run(meta, "log_level >= 3", Some(GRADE_ERROR_LOG_LIMIT), Some(0))
        .await
    {
        Ok(messages) => evidence.error_logs = messages.into_iter().map(|log| log.message).collect(),
        Err(error) => {
            tracing::warn!(%error, run_id = %meta.run_id, "Error-log query failed while grading");
            evidence.log_query_failed = true;
        }
    }
    evidence
}

/// Whether the board (at the given version) discards `ASSERT_OK` markers:
/// `log_level` above `Info` means a grade cannot justify a green verdict.
async fn board_grading_blind(app: &App, board_id: &str, version: Option<(u32, u32, u32)>) -> bool {
    match app
        .open_board(board_id.to_string(), Some(false), version)
        .await
    {
        Ok(board) => board.lock().await.log_level.to_u8() > LogLevel::Info.to_u8(),
        Err(error) => {
            tracing::warn!(%error, board_id = %board_id, ?version, "Could not load the board to check its log level");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Wire twins of the cloud responses — field names and casing must stay
// identical so the shared UI renders both transports unchanged.
// ---------------------------------------------------------------------------

#[derive(Serialize, Debug, Clone)]
pub struct CorpusEntry {
    pub run_id: String,
    pub start: u64,
    pub end: u64,
    pub log_level: u8,
    pub board_version: String,
    pub event_version: Option<String>,
    /// The node the run was dispatched into — the node a replay must target.
    pub node_id: String,
    /// Raw recorded payload size in bytes, pre-redaction.
    pub payload_len: usize,
    pub shape_hash: String,
    /// Redacted preview, capped at 2 KiB.
    pub preview: String,
    pub caveats: Vec<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct EventCorpusResponse {
    pub entries: Vec<CorpusEntry>,
    pub board_id: String,
    pub window_secs: u64,
    pub scanned_rows: usize,
    pub scan_capped: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct CorpusPayloadResponse {
    pub run_id: String,
    pub node_id: String,
    pub board_id: String,
    /// The recorded payload, redacted by leaf key name.
    pub payload: Value,
}

#[derive(Serialize, Debug, Clone)]
pub struct FixtureSummary {
    pub id: String,
    pub source_node_id: String,
    pub source_board_id: String,
    pub baseline: FixtureBaseline,
    pub promoted_by: String,
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

#[derive(Serialize, Debug, Clone)]
pub struct RegressionSuiteResponse {
    pub suite: RegressionSuite,
    /// Always `None` — desktop suites are never scheduled.
    pub next_run_at: Option<String>,
    pub fixtures: Vec<FixtureSummary>,
}

async fn suite_response(
    app: &App,
    suite: RegressionSuite,
) -> Result<RegressionSuiteResponse, TauriFunctionError> {
    let fixtures = suite.list_fixtures(app).await?;
    Ok(RegressionSuiteResponse {
        suite,
        next_run_at: None,
        fixtures: fixtures.iter().map(fixture_summary).collect(),
    })
}

/// One archived suite run: the wire twin of the cloud's
/// `SuiteRunDetailResponse`, persisted as compressed JSON by the client-side
/// runner. The archive is the desktop's ONLY store for suite runs.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SuiteRunArchive {
    pub run: SuiteRunSummary,
    pub cases: Vec<SuiteCaseResult>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SuiteRunSummary {
    pub id: String,
    /// Candidate board version (`major.minor.patch`, or `draft`).
    pub board_version: String,
    /// Always `manual` on desktop — there is no schedule and no publish hook.
    pub trigger: String,
    /// `running`, `completed` or `errored`.
    pub status: String,
    pub regressed: i64,
    pub fixed: i64,
    pub still_failing: i64,
    pub ok: i64,
    pub skipped: i64,
    /// RFC 3339.
    pub started_at: Option<String>,
    /// RFC 3339.
    pub completed_at: Option<String>,
    pub error: Option<String>,
    /// RFC 3339.
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SuiteCaseResult {
    pub id: String,
    /// `recorded_fixture` or `authored_test`.
    pub case_kind: String,
    /// Fixture id (recorded) or start node id (authored).
    pub case_ref: String,
    pub replay_run_id: Option<String>,
    /// `ok`, `regressed`, `still_failing`, `fixed` or `skipped`.
    pub outcome: String,
    pub grade_verdict: String,
    pub detail: Option<Value>,
    pub duration_ms: Option<i64>,
}

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

struct CorpusRowMeta {
    version: String,
    event_version: Option<String>,
    log_level: u8,
    payload_len: usize,
}

#[tauri::command(async)]
pub async fn list_regression_corpus(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    limit: Option<usize>,
) -> Result<EventCorpusResponse, TauriFunctionError> {
    if !is_safe_id(&event_id) {
        return Err(TauriFunctionError::new(
            "Event ID may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    let target = limit
        .unwrap_or(DEFAULT_CORPUS_LIMIT)
        .clamp(1, SUITE_CASE_CAP);

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state.clone())
        .await
        .map_err(|_| TauriFunctionError::new("App not found"))?;
    let event = app.get_event(&event_id, None).await?;
    ensure_regression_capable_event(&event)?;
    let board_id = event_board_id(&app, &event)?;

    let suite = find_suite_for_event(&app, &event_id).await?;
    let excluded = replay_exclusion_set(&app, suite.as_ref()).await;

    let Some(table) = open_runs_table_opt(&flow_like_state, &app_id, &board_id).await? else {
        // No run has ever flushed for this board — an empty corpus, not an error.
        return Ok(EventCorpusResponse {
            entries: Vec::new(),
            board_id,
            window_secs: 0,
            scanned_rows: 0,
            scan_capped: false,
        });
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
            let batches = table
                .query()
                .only_if(&filter)
                .limit(cap)
                .execute()
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to scan the corpus window: {e}"))?
                .try_collect::<Vec<_>>()
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to collect corpus rows: {e}"))?;
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
    .await?;

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

    Ok(EventCorpusResponse {
        entries,
        board_id,
        window_secs: selection.window.as_secs(),
        scanned_rows: selection.scanned_rows,
        scan_capped: selection.scan_capped,
    })
}

#[tauri::command(async)]
pub async fn get_regression_corpus_payload(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    run_id: String,
) -> Result<CorpusPayloadResponse, TauriFunctionError> {
    if !is_safe_id(&event_id) || !is_safe_id(&run_id) {
        return Err(TauriFunctionError::new(
            "IDs may only contain alphanumeric characters, '-' and '_'",
        ));
    }

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state.clone())
        .await
        .map_err(|_| TauriFunctionError::new("App not found"))?;
    let event = app.get_event(&event_id, None).await?;
    ensure_regression_capable_event(&event)?;
    let board_id = event_board_id(&app, &event)?;

    let table = open_runs_table_opt(&flow_like_state, &app_id, &board_id)
        .await?
        .ok_or_else(|| TauriFunctionError::new("No runs recorded for this board"))?;
    let row = load_corpus_row(&table, &event_id, &run_id)
        .await?
        .ok_or_else(|| {
            TauriFunctionError::new(&format!("Run {run_id} not found for this event"))
        })?;

    let mut payload = payload_value(&row.payload);
    redact_by_key_name(&mut payload);
    // Raw-body duplicates (body_bytes / body_text) defeat key-name redaction;
    // the parsed `body` object is the redacted, replayable form.
    drop_raw_body_duplicates(&mut payload);

    Ok(CorpusPayloadResponse {
        run_id: row.run_id,
        node_id: row.node_id,
        board_id,
        payload,
    })
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

#[tauri::command(async)]
pub async fn promote_regression_fixture(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    run_id: String,
    expectation: Option<String>,
    acknowledge_rejected: Option<bool>,
) -> Result<FixtureSummary, TauriFunctionError> {
    if !is_safe_id(&event_id) || !is_safe_id(&run_id) {
        return Err(TauriFunctionError::new(
            "IDs may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    let expectation = match expectation.as_deref() {
        None => None,
        Some("pass") => Some(TestVerdict::Pass),
        Some("fail") => Some(TestVerdict::Fail),
        Some(other) => {
            return Err(TauriFunctionError::new(&format!(
                "Unknown expectation '{other}'; expected pass or fail"
            )));
        }
    };

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state.clone())
        .await
        .map_err(|_| TauriFunctionError::new("App not found"))?;
    let event = app.get_event(&event_id, None).await?;
    ensure_regression_capable_event(&event)?;
    let board_id = event_board_id(&app, &event)?;

    let suite = require_suite(find_suite_for_event(&app, &event_id).await?)?;
    let fixtures = suite.list_fixtures(&app).await?;
    if fixtures.len() >= SUITE_CASE_CAP {
        return Err(TauriFunctionError::new(&format!(
            "This suite already holds {SUITE_CASE_CAP} fixtures — delete one before promoting another run"
        )));
    }

    let table = open_runs_table_opt(&flow_like_state, &app_id, &board_id)
        .await?
        .ok_or_else(|| TauriFunctionError::new("No runs recorded for this board"))?;
    let row = load_corpus_row(&table, &event_id, &run_id)
        .await?
        .ok_or_else(|| {
            TauriFunctionError::new(&format!("Run {run_id} not found for this event"))
        })?;

    let visited_node_ids: Vec<String> = row
        .nodes
        .iter()
        .flatten()
        .map(|(node_id, _)| node_id.clone())
        .collect();
    if is_rejected_summary(row.start, row.end, visited_node_ids.len())
        && !acknowledge_rejected.unwrap_or(false)
    {
        return Err(TauriFunctionError::new(
            "This run was rejected before execution — its payload never reached the flow. Pass acknowledge_rejected: true to promote it anyway.",
        ));
    }

    let recorded_version = row.version.clone();
    let recorded_start = row.start;
    let source_node_id = row.node_id.clone();
    let payload = prepare_fixture_payload(payload_value(&row.payload))?;

    // Grade the recorded run from its stored Lance artifacts — the baseline
    // future replays are compared against. A missing or unreadable log table
    // yields an `error` baseline, never a green light.
    let meta = LogMeta::from(row);
    let grade = grade_run(collect_local_evidence(&flow_like_state, &meta).await);
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
    if board_grading_blind(&app, &board_id, parse_run_version_label(&recorded_version)).await {
        caveats.push(CAVEAT_GRADING_BLIND.to_string());
    }

    let fixture = RegressionFixture {
        id: create_id(),
        payload,
        source_node_id,
        source_board_id: board_id,
        baseline: FixtureBaseline {
            verdict,
            error_class,
            visited_node_ids,
            recorded_at: recorded_start,
        },
        promoted_by: LOCAL_PROMOTER.to_string(),
        caveats,
    };
    suite.save_fixture(&app, &fixture).await?;

    Ok(fixture_summary(&fixture))
}

#[tauri::command(async)]
pub async fn delete_regression_fixture(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    fixture_id: String,
) -> Result<(), TauriFunctionError> {
    if !is_safe_id(&event_id) || !is_safe_id(&fixture_id) {
        return Err(TauriFunctionError::new(
            "IDs may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    let app = load_app(&handler, &app_id).await?;
    let suite = require_suite(find_suite_for_event(&app, &event_id).await?)?;
    suite.delete_fixture(&app, &fixture_id).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Suite config
// ---------------------------------------------------------------------------

#[tauri::command(async)]
pub async fn get_regression_suite(
    handler: AppHandle,
    app_id: String,
    event_id: String,
) -> Result<Option<RegressionSuiteResponse>, TauriFunctionError> {
    let app = load_app(&handler, &app_id).await?;
    match find_suite_for_event(&app, &event_id).await? {
        Some(suite) => Ok(Some(suite_response(&app, suite).await?)),
        None => Ok(None),
    }
}

#[tauri::command(async)]
pub async fn upsert_regression_suite(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    trigger_on_publish: Option<bool>,
    schedule: Option<String>,
    gate_mode: Option<String>,
    allow_live_side_effects: Option<bool>,
) -> Result<RegressionSuiteResponse, TauriFunctionError> {
    if !is_safe_id(&event_id) {
        return Err(TauriFunctionError::new(
            "Event ID may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    if schedule
        .as_deref()
        .map(str::trim)
        .is_some_and(|expr| !expr.is_empty())
    {
        return Err(TauriFunctionError::new(
            "Scheduling is cloud-only — desktop suites run manually from this device",
        ));
    }
    let gate_mode = match gate_mode.as_deref() {
        None => GateMode::Off,
        Some("Off") => GateMode::Off,
        Some("Warn") => GateMode::Warn,
        Some("Block") => GateMode::Block,
        Some(other) => {
            return Err(TauriFunctionError::new(&format!(
                "Unknown gate mode '{other}'; expected Off, Warn or Block"
            )));
        }
    };

    let app = load_app(&handler, &app_id).await?;
    let event = app.get_event(&event_id, None).await?;
    ensure_regression_capable_event(&event)?;
    let board_id = event_board_id(&app, &event)?;

    let existing = find_suite_for_event(&app, &event_id).await?;
    let now = now_micros();
    let suite = RegressionSuite {
        id: existing
            .as_ref()
            .map(|suite| suite.id.clone())
            .unwrap_or_else(create_id),
        board_id,
        event_id: Some(event_id.clone()),
        node_id: event.node_id.clone(),
        trigger_on_publish: trigger_on_publish.unwrap_or(false),
        schedule: None,
        gate_mode,
        allow_live_side_effects: allow_live_side_effects.unwrap_or(false),
        created_at: existing.map(|suite| suite.created_at).unwrap_or(now),
        updated_at: now,
    };
    suite.save(&app).await?;

    suite_response(&app, suite).await
}

// ---------------------------------------------------------------------------
// Suite runs — planned here, executed by the client-side TS runner, archived
// back through `persist_regression_suite_run`.
// ---------------------------------------------------------------------------

#[derive(Serialize, Debug, Clone)]
pub struct RegressionSuitePlanResponse {
    pub suite_id: String,
    pub board_id: String,
    /// Every runnable case, fixtures first then authored `test*` events —
    /// core `plan_suite_cases` semantics (copilot filter/cap included).
    pub cases: Vec<SuiteCase>,
    /// Fixture ids skipped because the candidate version no longer contains
    /// their `source_node_id`.
    pub skipped_missing_node: Vec<String>,
    /// Cases dropped by the suite case cap.
    pub truncated: usize,
    /// The candidate board's `log_level` discards `ASSERT_OK` markers — green
    /// verdicts cannot be justified and every case is stamped.
    pub grading_blind: bool,
}

#[tauri::command(async)]
pub async fn plan_regression_suite_run(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    board_version: Option<(u32, u32, u32)>,
) -> Result<RegressionSuitePlanResponse, TauriFunctionError> {
    if !is_safe_id(&event_id) {
        return Err(TauriFunctionError::new(
            "Event ID may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    let app = load_app(&handler, &app_id).await?;
    let event = app.get_event(&event_id, None).await?;
    ensure_regression_capable_event(&event)?;

    let suite = require_suite(find_suite_for_event(&app, &event_id).await?)?;
    if !suite.allow_live_side_effects {
        return Err(TauriFunctionError::new(
            "This suite has not acknowledged live side effects. Desktop replays run fully live on this device — every node executes for real, with no isolation of storage, network or WASM. Acknowledge live side effects on the suite before running it.",
        ));
    }

    let board = app
        .open_board(suite.board_id.clone(), Some(false), board_version)
        .await
        .map_err(|error| {
            TauriFunctionError::new(&format!(
                "Candidate board {} (version {:?}) could not be loaded: {}",
                suite.board_id, board_version, error
            ))
        })?;
    let board = board.lock().await;

    let fixtures = suite.list_fixtures(&app).await?;
    let plan = plan_suite_cases(&fixtures, &board, None);
    let grading_blind = board.log_level.to_u8() > LogLevel::Info.to_u8();

    Ok(RegressionSuitePlanResponse {
        suite_id: suite.id,
        board_id: suite.board_id,
        cases: plan.cases,
        skipped_missing_node: plan.skipped_missing_node,
        truncated: plan.truncated,
        grading_blind,
    })
}

#[tauri::command(async)]
pub async fn persist_regression_suite_run(
    handler: AppHandle,
    app_id: String,
    suite_id: String,
    run: SuiteRunArchive,
) -> Result<(), TauriFunctionError> {
    if !is_safe_id(&suite_id) || !is_safe_id(&run.run.id) {
        return Err(TauriFunctionError::new(
            "IDs may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    let app = load_app(&handler, &app_id).await?;
    let suite = RegressionSuite::load(&app, &suite_id)
        .await
        .map_err(|_| TauriFunctionError::new("Regression suite not found"))?;
    let suite_run_id = run.run.id.clone();
    suite.archive_run(&app, &suite_run_id, &run).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn list_regression_suite_runs(
    handler: AppHandle,
    app_id: String,
    event_id: String,
) -> Result<Vec<SuiteRunSummary>, TauriFunctionError> {
    let app = load_app(&handler, &app_id).await?;
    let suite = require_suite(find_suite_for_event(&app, &event_id).await?)?;

    let mut summaries = Vec::new();
    for suite_run_id in RegressionSuite::list_run_archive_ids(&app, &suite.id).await? {
        match RegressionSuite::load_run_archive::<SuiteRunArchive>(&app, &suite.id, &suite_run_id)
            .await
        {
            Ok(archive) => summaries.push(archive.run),
            Err(error) => {
                tracing::warn!(%error, suite_run_id = %suite_run_id, "Failed to load a suite-run archive; skipping");
            }
        }
    }
    // The runner writes `created_at` as an ISO 8601 UTC instant, so the
    // lexicographic order is the chronological order.
    summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(summaries)
}

#[tauri::command(async)]
pub async fn get_regression_suite_run(
    handler: AppHandle,
    app_id: String,
    event_id: String,
    suite_run_id: String,
) -> Result<SuiteRunArchive, TauriFunctionError> {
    if !is_safe_id(&suite_run_id) {
        return Err(TauriFunctionError::new(
            "Suite run ID may only contain alphanumeric characters, '-' and '_'",
        ));
    }
    let app = load_app(&handler, &app_id).await?;
    let suite = require_suite(find_suite_for_event(&app, &event_id).await?)?;
    RegressionSuite::load_run_archive::<SuiteRunArchive>(&app, &suite.id, &suite_run_id)
        .await
        .map_err(|_| TauriFunctionError::new(&format!("Suite run {suite_run_id} not found")))
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
