//! The regression-suite runner: replays a suite's cases against a candidate
//! board version and grades verdict-vs-baseline.
//!
//! Every case dispatches the board-invoke shape with `shadow: true` signed
//! into the executor JWT, `CredentialsAccess::ShadowExecute` scoped
//! credentials, `RunVariant::Regression` on the run row (plus
//! `regressionRunId`), and `DispatchTrigger::System`. Suite runs live in
//! `RegressionSuiteRun` + `RegressionCaseResult` — on cloud those tables are
//! THE store, no bucket archive is written.

pub mod gate;

use std::sync::Arc;
use std::time::Duration;

use flow_like::app::App;
use flow_like::flow::board::Board;
use flow_like::flow::execution::LogLevel;
use flow_like::flow::execution::log::StoredLogMessage;
use flow_like::flow::regression::{
    CaseOutcome, FixtureBaseline, GateMode, RegressionSuite as CoreRegressionSuite,
    RunGradeEvidence, SuiteCase, compare_to_expectation, error_class_of, grade_run,
    plan_suite_cases,
};
use flow_like_storage::Path as StoragePath;
use flow_like_storage::arrow_array::RecordBatch;
use flow_like_storage::lancedb::query::{ExecutableQuery, QueryBase};
use flow_like_storage::serde_arrow;
use flow_like_types::{anyhow, create_id, tokio};
use futures::{StreamExt, TryStreamExt};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, sea_query::Expr,
};

use crate::{
    credentials::CredentialsAccess,
    db::{DEFAULT_WRITE_CHUNK, update_in_batches},
    entity::{
        event_sink, execution_run, regression_case_result, regression_suite, regression_suite_run,
        sea_orm_active_enums::{RunMode, RunStatus, RunVariant},
    },
    error::ApiError,
    execution::{
        DispatchRequest, DispatchTrigger, ExecutionBackend, ExecutionJwtParams, TokenType,
        collect_generic_result, collect_generic_result_bytes, format_run_version,
        is_jwt_configured, resolve_wasm_packages, sign_execution_jwt, update_run_on_completion,
    },
    state::AppState,
};

/// Cases dispatched concurrently per suite run.
const SUITE_CONCURRENCY: usize = 4;
/// Concurrency when the candidate board carries WASM nodes — their executor
/// compiles per-request bundles, so replays are throttled harder.
const SUITE_CONCURRENCY_WASM: usize = 2;
/// Per-case dispatch-to-completion timeout.
const CASE_TIMEOUT: Duration = Duration::from_secs(120);
/// Whole-suite wall clock; the run is flipped to `errored` past it.
pub const SUITE_WALL_CLOCK: Duration = Duration::from_secs(15 * 60);
/// Liveness grace on top of the wall clock: a `running` SuiteRun older than
/// wall + grace is flipped to `errored` by the RegressionSuites maintenance
/// job (`run_sweeper` only touches `ExecutionRun` and never sees these rows).
pub const SUITE_LIVENESS_GRACE: Duration = Duration::from_secs(5 * 60);
/// Bounded Lance retries while the executor's log flush lands.
const GRADE_RETRIES: usize = 3;
const GRADE_RETRY_DELAY: Duration = Duration::from_secs(2);
/// Due schedules dispatched per maintenance tick.
const SCHEDULE_DISPATCH_CAP: u64 = 25;

/// Log rows fetched per grading query — the exact limits of the TS twin's
/// `collectRunEvidence` (`board-tests.ts`), so both graders see the same
/// evidence.
pub(crate) const GRADE_ASSERT_LOG_LIMIT: usize = 100;
pub(crate) const GRADE_ERROR_LOG_LIMIT: usize = 10;

/// Inserted but not yet executing: the deployment's detached tasks are
/// unreliable (see [`detached_tasks_unreliable`]) and the RegressionSuites
/// maintenance job will pick the run up.
pub const SUITE_RUN_QUEUED: &str = "queued";
pub const SUITE_RUN_RUNNING: &str = "running";
pub const SUITE_RUN_COMPLETED: &str = "completed";
pub const SUITE_RUN_ERRORED: &str = "errored";

/// `DETACHED_TASKS_UNRELIABLE=1` marks deployments whose detached tasks die
/// with the request — the AWS `lambda_http` API, where a `tokio::spawn`ed
/// suite run would be frozen mid-flight when the invocation ends. Suite runs
/// are then inserted [`SUITE_RUN_QUEUED`] and executed inline by the
/// RegressionSuites maintenance job instead of on a spawned task.
fn detached_tasks_unreliable() -> bool {
    std::env::var("DETACHED_TASKS_UNRELIABLE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Board-version label of a draft (unpublished head) suite run. Draft runs
/// never feed the publish/promote gate — the gate keys on pinned versions.
pub const DRAFT_VERSION_LABEL: &str = "draft";

const CASE_KIND_RECORDED: &str = "recorded_fixture";
const CASE_KIND_AUTHORED: &str = "authored_test";

const OUTCOME_SKIPPED: &str = "skipped";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuiteRunTrigger {
    Manual,
    Publish,
    Schedule,
}

impl SuiteRunTrigger {
    pub const fn as_str(self) -> &'static str {
        match self {
            SuiteRunTrigger::Manual => "manual",
            SuiteRunTrigger::Publish => "publish",
            SuiteRunTrigger::Schedule => "schedule",
        }
    }
}

/// Which board version the suite replays into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateVersion {
    Pinned((u32, u32, u32)),
    /// The board's newest published version, resolved at spawn time.
    LatestPublished,
    /// The live head, unpinned. Never feeds the gate.
    Draft,
}

/// Who the replay runs as. Manual runs carry the caller's sub, publish runs
/// the publisher's; scheduled runs resolve the event's sink PAT subject and
/// fall back to a `regression:{suite_id}` placeholder with `userId` NULL.
#[derive(Clone, Debug, Default)]
pub struct SuiteRunActor {
    pub sub: Option<String>,
    pub user_context: Option<flow_like::flow::execution::UserExecutionContext>,
}

/// Projection-row string of a [`GateMode`]; the inverse of [`parse_gate_mode`].
pub(crate) fn gate_mode_as_str(mode: GateMode) -> &'static str {
    match mode {
        GateMode::Off => "Off",
        GateMode::Warn => "Warn",
        GateMode::Block => "Block",
    }
}

pub(crate) fn parse_gate_mode(raw: &str) -> Option<GateMode> {
    match raw {
        "Off" => Some(GateMode::Off),
        "Warn" => Some(GateMode::Warn),
        "Block" => Some(GateMode::Block),
        _ => None,
    }
}

pub(crate) fn datetime_micros(dt: chrono::DateTime<chrono::FixedOffset>) -> u64 {
    dt.timestamp_micros().max(0) as u64
}

/// Rebuild a suite config from its Postgres projection row — the degraded
/// read for a torn bucket write.
pub(crate) fn suite_from_projection(row: &regression_suite::Model) -> CoreRegressionSuite {
    CoreRegressionSuite {
        id: row.id.clone(),
        board_id: row.board_id.clone(),
        event_id: row.event_id.clone(),
        node_id: row.node_id.clone(),
        trigger_on_publish: row.trigger_on_publish,
        schedule: row.schedule.clone(),
        gate_mode: parse_gate_mode(&row.gate_mode).unwrap_or_default(),
        allow_live_side_effects: row.allow_live_side_effects,
        created_at: datetime_micros(row.created_at),
        updated_at: datetime_micros(row.updated_at),
    }
}

/// The bucket object is the config authority; a missing object (torn first
/// write) degrades to the projection row so reads stay usable.
pub(crate) async fn load_core_suite(
    app: &App,
    row: &regression_suite::Model,
) -> CoreRegressionSuite {
    match CoreRegressionSuite::load(app, &row.id).await {
        Ok(suite) => suite,
        Err(error) => {
            tracing::warn!(%error, suite_id = %row.id, "Suite config missing from the bucket; projecting from the database row");
            suite_from_projection(row)
        }
    }
}

/// Open the board's Lance runs database. Returns the connection plus the
/// `runs` summary table when at least one run has ever flushed.
pub(crate) async fn open_runs_db(
    state: &AppState,
    sub: &str,
    app_id: &str,
    board_id: &str,
) -> Result<
    (
        flow_like_storage::lancedb::Connection,
        Option<flow_like_storage::lancedb::Table>,
    ),
    ApiError,
> {
    let credentials = state
        .scoped_credentials(sub, app_id, CredentialsAccess::ReadLogs)
        .await?;
    let logs_db_builder = credentials
        .into_shared_credentials()
        .to_logs_db_builder()
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to create logs db builder: {}", e))
        })?;
    let base_path = StoragePath::from("runs").child(app_id).child(board_id);
    let db = logs_db_builder(base_path.clone())
        .execute()
        .await
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to open runs database at {base_path}: {e}"))
        })?;
    let table_names = db.table_names().execute().await.map_err(|e| {
        ApiError::internal_error(anyhow!(
            "Failed to list run tables for board {board_id}: {e}"
        ))
    })?;
    if !table_names.iter().any(|name| name == "runs") {
        return Ok((db, None));
    }
    let table = db.open_table("runs").execute().await.map_err(|e| {
        ApiError::internal_error(anyhow!(
            "Failed to open runs table for board {board_id}: {e}"
        ))
    })?;
    Ok((db, Some(table)))
}

/// One grading query against a run's own log table.
pub(crate) async fn run_log_messages(
    db: &flow_like_storage::lancedb::Connection,
    run_id: &str,
    filter: &str,
    limit: usize,
) -> flow_like_types::Result<Vec<String>> {
    let table = db
        .open_table(run_id)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open log table for run {run_id}: {e}"))?;
    let batches: Vec<RecordBatch> = table
        .query()
        .only_if(filter)
        .limit(limit)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to query logs of run {run_id}: {e}"))?
        .try_collect()
        .await
        .map_err(|e| anyhow!("Failed to collect logs of run {run_id}: {e}"))?;
    let mut messages = Vec::new();
    for batch in &batches {
        let stored: Vec<StoredLogMessage> =
            serde_arrow::from_record_batch(batch).unwrap_or_default();
        messages.extend(stored.into_iter().map(|log| log.message));
    }
    Ok(messages)
}

/// Gather the grading evidence for a run from its stored Lance artifacts: one
/// `ASSERT_%` query and one error-log query on the run's own log table (the
/// TS twin's `collectRunEvidence` shape). A missing or unreadable log table
/// sets `log_query_failed`, so the grade becomes `error` — never a green
/// light.
pub(crate) async fn collect_lance_evidence(
    db: &flow_like_storage::lancedb::Connection,
    run_id: &str,
) -> RunGradeEvidence {
    let mut evidence = RunGradeEvidence {
        metadata_present: true,
        ..RunGradeEvidence::default()
    };
    let has_table = match db.table_names().execute().await {
        Ok(names) => names.iter().any(|name| name == run_id),
        Err(error) => {
            tracing::warn!(%error, run_id = %run_id, "Failed to list run tables while grading");
            evidence.log_query_failed = true;
            false
        }
    };
    if has_table {
        match run_log_messages(
            db,
            run_id,
            "message LIKE 'ASSERT_%'",
            GRADE_ASSERT_LOG_LIMIT,
        )
        .await
        {
            Ok(messages) => evidence.assert_logs = messages,
            Err(error) => {
                tracing::warn!(%error, run_id = %run_id, "Assert-log query failed while grading");
                evidence.log_query_failed = true;
            }
        }
        match run_log_messages(db, run_id, "log_level >= 3", GRADE_ERROR_LOG_LIMIT).await {
            Ok(messages) => evidence.error_logs = messages,
            Err(error) => {
                tracing::warn!(%error, run_id = %run_id, "Error-log query failed while grading");
                evidence.log_query_failed = true;
            }
        }
    } else if !evidence.log_query_failed {
        tracing::warn!(run_id = %run_id, "Run has no log table; the run is ungradable");
        evidence.log_query_failed = true;
    }
    evidence
}

/// Replay grading has to tolerate the gap between the executor's `completed`
/// event and its Lance log flush: retry the evidence read a few times before
/// accepting `log_query_failed`.
async fn collect_replay_evidence(
    db: &flow_like_storage::lancedb::Connection,
    run_id: &str,
) -> RunGradeEvidence {
    let mut evidence = collect_lance_evidence(db, run_id).await;
    let mut attempts = 0;
    while evidence.log_query_failed && attempts < GRADE_RETRIES {
        attempts += 1;
        tokio::time::sleep(GRADE_RETRY_DELAY).await;
        evidence = collect_lance_evidence(db, run_id).await;
    }
    evidence
}

fn outcome_label(outcome: &CaseOutcome) -> &'static str {
    match outcome {
        CaseOutcome::Ok => "ok",
        CaseOutcome::Regressed => "regressed",
        CaseOutcome::StillFailing { .. } => "still_failing",
        CaseOutcome::Fixed => "fixed",
    }
}

fn now_naive() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::Utc::now().fixed_offset()
}

/// Resolve the candidate into the dispatch pin and the `SuiteRun.boardVersion`
/// label (dotted `major.minor.patch`, or [`DRAFT_VERSION_LABEL`]).
async fn resolve_candidate(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    candidate: CandidateVersion,
) -> Result<(Option<(u32, u32, u32)>, String), ApiError> {
    match candidate {
        CandidateVersion::Pinned(version) => {
            state
                .master_board_shared(app_id, board_id, state, Some(version))
                .await
                .map_err(|e| {
                    ApiError::bad_request(format!(
                        "Board {board_id} has no published version {}.{}.{}: {e}",
                        version.0, version.1, version.2
                    ))
                })?;
            Ok((
                Some(version),
                crate::routes::app::events::dotted_version_key(version),
            ))
        }
        CandidateVersion::LatestPublished => {
            let head = state
                .master_board_shared(app_id, board_id, state, None)
                .await
                .map_err(ApiError::internal_error)?;
            let version = head.board.version;
            state
                .master_board_shared(app_id, board_id, state, Some(version))
                .await
                .map_err(|e| {
                    ApiError::bad_request(format!(
                        "Board {board_id} has no published snapshot yet ({}.{}.{}): {e}",
                        version.0, version.1, version.2
                    ))
                })?;
            Ok((
                Some(version),
                crate::routes::app::events::dotted_version_key(version),
            ))
        }
        CandidateVersion::Draft => Ok((None, DRAFT_VERSION_LABEL.to_string())),
    }
}

/// Validate, insert the SuiteRun row and detach the case executor. Returns
/// the pollable `suite_run_id` plus the row's initial status; any later
/// failure lands on the row as `errored`.
///
/// Fire-and-forget by default; when [`detached_tasks_unreliable`] the row is
/// inserted [`SUITE_RUN_QUEUED`] instead and nothing is spawned — the
/// RegressionSuites maintenance job executes it inline.
pub async fn spawn_suite_run(
    state: AppState,
    app_id: String,
    suite: CoreRegressionSuite,
    candidate: CandidateVersion,
    trigger: SuiteRunTrigger,
    actor: SuiteRunActor,
) -> Result<(String, &'static str), ApiError> {
    if !suite.allow_live_side_effects {
        return Err(ApiError::conflict(
            "This suite has not acknowledged live side effects. Replays execute real nodes — outbound HTTP from native nodes cannot be suppressed (shadow isolation only guards storage and WASM). Set allow_live_side_effects: true on the suite before running it.",
        ));
    }
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured (missing BACKEND_KEY/BACKEND_PUB)"
        )));
    }
    if matches!(trigger, SuiteRunTrigger::Manual | SuiteRunTrigger::Publish) && actor.sub.is_none()
    {
        return Err(ApiError::internal_error(anyhow!(
            "{} suite runs require the caller's subject",
            trigger.as_str()
        )));
    }

    let (dispatch_version, version_label) =
        resolve_candidate(&state, &app_id, &suite.board_id, candidate).await?;

    let queued = detached_tasks_unreliable();
    let suite_run_id = create_id();
    let now = now_naive();
    regression_suite_run::ActiveModel {
        id: Set(suite_run_id.clone()),
        app_id: Set(app_id.clone()),
        suite_id: Set(suite.id.clone()),
        board_id: Set(suite.board_id.clone()),
        board_version: Set(version_label),
        trigger: Set(trigger.as_str().to_string()),
        status: Set(if queued {
            SUITE_RUN_QUEUED
        } else {
            SUITE_RUN_RUNNING
        }
        .to_string()),
        regressed: Set(0),
        fixed: Set(0),
        still_failing: Set(0),
        ok: Set(0),
        skipped: Set(0),
        started_at: Set(if queued { None } else { Some(now) }),
        completed_at: Set(None),
        error: Set(None),
        created_at: Set(now),
    }
    .insert(&state.db)
    .await
    .map_err(|e| ApiError::internal_error(anyhow!("Failed to insert suite run row: {e}")))?;

    if queued {
        return Ok((suite_run_id, SUITE_RUN_QUEUED));
    }

    let spawned_id = suite_run_id.clone();
    tokio::spawn(async move {
        run_suite_to_completion(
            &state,
            &app_id,
            &suite,
            &spawned_id,
            dispatch_version,
            actor,
        )
        .await;
    });

    Ok((suite_run_id, SUITE_RUN_RUNNING))
}

/// Drive one already-`running` SuiteRun to a terminal row: execute the cases
/// under the wall clock, then finalize with the aggregated tallies.
async fn run_suite_to_completion(
    state: &AppState,
    app_id: &str,
    suite: &CoreRegressionSuite,
    suite_run_id: &str,
    dispatch_version: Option<(u32, u32, u32)>,
    actor: SuiteRunActor,
) {
    let body = execute_suite_run(state, app_id, suite, suite_run_id, dispatch_version, actor);
    let outcome = match tokio::time::timeout(SUITE_WALL_CLOCK, body).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!(
            "suite run exceeded the {} minute wall clock",
            SUITE_WALL_CLOCK.as_secs() / 60
        )),
    };
    if let Err(error) = finalize_suite_run(state, suite_run_id, outcome).await {
        tracing::error!(suite_run_id = %suite_run_id, %error, "Failed to finalize suite run");
    }
}

/// Fold the persisted case rows into the SuiteRun tallies. Aggregating from
/// the rows (instead of in-memory counters) keeps the tallies honest when the
/// wall clock cut the case stream short.
async fn aggregate_case_tallies(
    db: &sea_orm::DatabaseConnection,
    suite_run_id: &str,
) -> Result<(i32, i32, i32, i32, i32), sea_orm::DbErr> {
    let outcomes: Vec<String> = regression_case_result::Entity::find()
        .select_only()
        .column(regression_case_result::Column::Outcome)
        .filter(regression_case_result::Column::SuiteRunId.eq(suite_run_id))
        .into_tuple()
        .all(db)
        .await?;
    let mut tallies = (0, 0, 0, 0, 0);
    for outcome in outcomes {
        match outcome.as_str() {
            "regressed" => tallies.0 += 1,
            "fixed" => tallies.1 += 1,
            "still_failing" => tallies.2 += 1,
            "ok" => tallies.3 += 1,
            OUTCOME_SKIPPED => tallies.4 += 1,
            other => tracing::warn!(suite_run_id, outcome = other, "Unknown case outcome"),
        }
    }
    Ok(tallies)
}

async fn finalize_suite_run(
    state: &AppState,
    suite_run_id: &str,
    outcome: Result<(), String>,
) -> Result<(), ApiError> {
    let (regressed, fixed, still_failing, ok, skipped) =
        aggregate_case_tallies(&state.db, suite_run_id)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to tally case rows: {e}")))?;
    let (status, error) = match outcome {
        Ok(()) => (SUITE_RUN_COMPLETED, None),
        Err(message) => (SUITE_RUN_ERRORED, Some(message)),
    };
    regression_suite_run::Entity::update_many()
        .set(regression_suite_run::ActiveModel {
            status: Set(status.to_string()),
            regressed: Set(regressed),
            fixed: Set(fixed),
            still_failing: Set(still_failing),
            ok: Set(ok),
            skipped: Set(skipped),
            completed_at: Set(Some(now_naive())),
            error: Set(error),
            ..Default::default()
        })
        .filter(regression_suite_run::Column::Id.eq(suite_run_id))
        .filter(regression_suite_run::Column::Status.eq(SUITE_RUN_RUNNING))
        .exec(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to finalize suite run: {e}")))?;
    Ok(())
}

/// The scheduled-trigger identity: the event's sink PAT subject when one
/// resolves, else a `regression:{suite_id}` placeholder (mirroring the
/// `sink:{id}` / `inbound:{event_id}` convention) with `userId` NULL.
async fn resolve_schedule_identity(
    state: &AppState,
    app_id: &str,
    suite: &CoreRegressionSuite,
) -> Result<(String, Option<String>, Option<event_sink::Model>), ApiError> {
    let placeholder = format!("regression:{}", suite.id);
    let Some(event_id) = suite.event_id.as_deref() else {
        return Ok((placeholder, None, None));
    };
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(app_id))
        .filter(event_sink::Column::EventId.eq(event_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to load event sink: {e}")))?;
    let Some(sink) = sink else {
        return Ok((placeholder, None, None));
    };
    let token = sink.pat_encrypted.as_ref().and_then(|encrypted| {
        crate::routes::app::events::db::decrypt_token(encrypted, &state.encryption_key)
    });
    let pat_user =
        crate::routes::sink::trigger::resolve_sink_pat_user_id(state, &sink, token.as_deref())
            .await?;
    match pat_user {
        Some(user_id) => Ok((user_id.clone(), Some(user_id), Some(sink))),
        None => Ok((placeholder, None, Some(sink))),
    }
}

struct CaseDispatchContext {
    app_id: String,
    suite_run_id: String,
    event_id: Option<String>,
    dispatch_version: Option<(u32, u32, u32)>,
    subject: String,
    run_user_id: Option<String>,
    user_context: Option<flow_like::flow::execution::UserExecutionContext>,
    credentials_json: String,
    callback_url: String,
    profile: Option<serde_json::Value>,
    wasm_packages:
        Option<std::collections::HashMap<String, flow_like_types::dispatch::WasmPackageRef>>,
    board_id: String,
    grading_blind: bool,
}

async fn execute_suite_run(
    state: &AppState,
    app_id: &str,
    suite: &CoreRegressionSuite,
    suite_run_id: &str,
    dispatch_version: Option<(u32, u32, u32)>,
    actor: SuiteRunActor,
) -> Result<(), ApiError> {
    let app = state.master_app("regression", app_id, state).await?;

    // Manual and publish runs carry the actor's sub; scheduled runs — and
    // queued pickups, where the actor was never persisted on the row —
    // resolve the schedule identity (sink PAT subject or placeholder).
    let (subject, run_user_id, sink) = match actor.sub.clone() {
        Some(sub) => (sub.clone(), Some(sub), None),
        None => resolve_schedule_identity(state, app_id, suite).await?,
    };

    let cached = state
        .master_board_shared(app_id, &suite.board_id, state, dispatch_version)
        .await
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to load the candidate board: {e}"))
        })?;
    let board: &Board = &cached.board;
    let concurrency = if board.has_wasm_nodes() {
        SUITE_CONCURRENCY_WASM
    } else {
        SUITE_CONCURRENCY
    };
    let grading_blind = board.log_level.to_u8() > LogLevel::Info.to_u8();

    let fixtures = suite
        .list_fixtures(&app)
        .await
        .map_err(ApiError::internal_error)?;
    let plan = plan_suite_cases(&fixtures, board, None);

    for fixture_id in &plan.skipped_missing_node {
        insert_case_result(
            &state.db,
            suite_run_id,
            CASE_KIND_RECORDED,
            fixture_id,
            None,
            OUTCOME_SKIPPED,
            "skipped",
            Some(serde_json::json!({
                "reason": "source node absent from the candidate board version"
            })),
            None,
        )
        .await;
    }
    if plan.truncated > 0 {
        tracing::warn!(
            suite_run_id,
            truncated = plan.truncated,
            "Suite case list truncated at the case cap"
        );
    }
    if plan.cases.is_empty() {
        return Ok(());
    }

    let credentials = state
        .scoped_credentials(&subject, app_id, CredentialsAccess::ShadowExecute)
        .await?;
    let credentials_json = serde_json::to_string(&credentials.into_shared_credentials())
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize credentials: {e}")))?;

    let profile = match &sink {
        Some(sink) => crate::routes::sink::trigger::hydrated_sink_profile(state, sink).await,
        None => {
            crate::execution::fetch_profile_for_dispatch(state, &subject, None, app_id, true).await
        }
    };
    let wasm_packages = resolve_wasm_packages(state, app_id).await;
    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let (logs_db, _runs_table) = open_runs_db(state, &subject, app_id, &suite.board_id).await?;

    let context = Arc::new(CaseDispatchContext {
        app_id: app_id.to_string(),
        suite_run_id: suite_run_id.to_string(),
        event_id: suite.event_id.clone(),
        dispatch_version,
        subject,
        run_user_id,
        user_context: actor.user_context,
        credentials_json,
        callback_url,
        profile,
        wasm_packages,
        board_id: suite.board_id.clone(),
        grading_blind,
    });

    futures::stream::iter(plan.cases.into_iter().map(|case| {
        let state = state.clone();
        let context = context.clone();
        let logs_db = logs_db.clone();
        async move { run_case(&state, &context, &logs_db, case).await }
    }))
    .buffer_unordered(concurrency)
    .collect::<Vec<()>>()
    .await;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_case_result(
    db: &sea_orm::DatabaseConnection,
    suite_run_id: &str,
    case_kind: &str,
    case_ref: &str,
    replay_run_id: Option<String>,
    outcome: &str,
    grade_verdict: &str,
    detail: Option<serde_json::Value>,
    duration_ms: Option<i32>,
) {
    let result = regression_case_result::ActiveModel {
        id: Set(create_id()),
        suite_run_id: Set(suite_run_id.to_string()),
        case_kind: Set(case_kind.to_string()),
        case_ref: Set(case_ref.to_string()),
        replay_run_id: Set(replay_run_id),
        outcome: Set(outcome.to_string()),
        grade_verdict: Set(grade_verdict.to_string()),
        detail: Set(detail),
        duration_ms: Set(duration_ms),
        created_at: Set(now_naive()),
    }
    .insert(db)
    .await;
    if let Err(error) = result {
        tracing::error!(suite_run_id, case_ref, %error, "Failed to insert case result row");
    }
}

/// Dispatch one case (board-invoke shape, shadow-signed), wait for completion
/// and grade verdict-vs-baseline. Every failure mode lands as a graded case
/// row — a case never disappears silently.
async fn run_case(
    state: &AppState,
    context: &CaseDispatchContext,
    logs_db: &flow_like_storage::lancedb::Connection,
    case: SuiteCase,
) {
    let (case_kind, case_ref, node_id, payload, baseline, alias) = match case {
        SuiteCase::RecordedFixture {
            fixture_id,
            payload,
            source_node_id,
            baseline,
        } => (
            CASE_KIND_RECORDED,
            fixture_id,
            source_node_id,
            Some(payload),
            baseline,
            None,
        ),
        SuiteCase::AuthoredTest { node_id, alias } => (
            CASE_KIND_AUTHORED,
            node_id.clone(),
            node_id,
            None,
            FixtureBaseline::pass_expectation(datetime_micros(now_naive())),
            Some(alias),
        ),
    };

    let started = std::time::Instant::now();
    let (replay_run_id, dispatch_error) = dispatch_case(state, context, &node_id, payload).await;

    let mut evidence = match &replay_run_id {
        Some(run_id) => collect_replay_evidence(logs_db, run_id).await,
        None => RunGradeEvidence::default(),
    };

    // Fold the run row's terminal status into the evidence: a replay that
    // failed, timed out or never completed must not grade on quiet logs.
    if let Some(error) = dispatch_error {
        evidence.execution_error = Some(error);
    } else if let Some(run_id) = &replay_run_id {
        match execution_run::Entity::find_by_id(run_id)
            .one(&state.db)
            .await
        {
            Ok(Some(run)) => match run.status {
                RunStatus::Completed => {}
                RunStatus::Pending | RunStatus::Running => {
                    evidence.execution_error.get_or_insert_with(|| {
                        format!(
                            "replay did not complete within the {}s case timeout",
                            CASE_TIMEOUT.as_secs()
                        )
                    });
                }
                status => {
                    evidence
                        .execution_error
                        .get_or_insert_with(|| format!("replay ended with status {status:?}"));
                }
            },
            Ok(None) => {
                evidence
                    .execution_error
                    .get_or_insert_with(|| "replay run row disappeared".to_string());
            }
            Err(error) => {
                tracing::warn!(run_id = %run_id, %error, "Failed to read the replay run row");
                evidence.log_query_failed = true;
            }
        }
    }

    let grade = grade_run(evidence);
    let outcome = compare_to_expectation(&baseline, &grade);
    let duration_ms = i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX);

    let mut detail = serde_json::json!({
        "error_class": error_class_of(&grade),
        "baseline_verdict": baseline.verdict.as_str(),
        "baseline_error_class": baseline.error_class,
        "assert_ok": grade.assert_ok,
        "assert_fail": grade.assert_fail,
        "failed_assertions": grade.failed_assertions,
        "execution_error": grade.execution_error,
    });
    if let CaseOutcome::StillFailing {
        error_class_changed,
    } = outcome
    {
        detail["error_class_changed"] = serde_json::Value::Bool(error_class_changed);
    }
    if context.grading_blind {
        detail["grading_blind"] = serde_json::Value::Bool(true);
    }
    if let Some(alias) = alias {
        detail["alias"] = serde_json::Value::String(alias);
    }

    insert_case_result(
        &state.db,
        &context.suite_run_id,
        case_kind,
        &case_ref,
        replay_run_id,
        outcome_label(&outcome),
        grade.verdict.as_str(),
        Some(detail),
        Some(duration_ms),
    )
    .await;
}

/// Insert the replay's `ExecutionRun` row and dispatch it. Returns the run id
/// (once the row exists) plus an error string when the dispatch never made it
/// to a graded completion.
async fn dispatch_case(
    state: &AppState,
    context: &CaseDispatchContext,
    node_id: &str,
    payload: Option<serde_json::Value>,
) -> (Option<String>, Option<String>) {
    let run_id = create_id();
    let now = now_naive();
    let input_payload_len = payload
        .as_ref()
        .and_then(|p| serde_json::to_string(p).ok())
        .map(|s| s.len() as i64)
        .unwrap_or(0);

    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(context.board_id.clone()),
        version: Set(context.dispatch_version.map(format_run_version)),
        event_id: Set(context.event_id.clone()),
        node_id: Set(Some(node_id.to_string())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Http),
        run_variant: Set(RunVariant::Regression),
        variant_name: Set(None),
        shadow_of_run_id: Set(None),
        regression_run_id: Set(Some(context.suite_run_id.clone())),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(now + chrono::Duration::hours(24))),
        user_id: Set(context.run_user_id.clone()),
        technical_user_id: Set(None),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(context.app_id.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    };
    if let Err(error) = run.insert(&state.db).await {
        return (None, Some(format!("failed to create the run row: {error}")));
    }

    if let Err(error) = crate::audit::record_execution_dispatch(state, &run_id, "regression").await
    {
        fail_replay_run(state, &run_id).await;
        return (
            Some(run_id),
            Some(format!("failed to record execution audit: {error}")),
        );
    }

    let executor_jwt = match sign_execution_jwt(ExecutionJwtParams {
        user_id: context.subject.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: context.app_id.clone(),
        board_id: context.board_id.clone(),
        event_id: None,
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(&run_id)),
        callback_url: context.callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(60 * 60),
        shadow: Some(true),
    }) {
        Ok(jwt) => jwt,
        Err(error) => {
            fail_replay_run(state, &run_id).await;
            return (
                Some(run_id),
                Some(format!("failed to sign the executor JWT: {error}")),
            );
        }
    };

    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: context.app_id.clone(),
        board_id: context.board_id.clone(),
        board_version: context.dispatch_version,
        board_etag: None,
        node_id: node_id.to_string(),
        event_json: None,
        payload,
        user_id: context.subject.clone(),
        credentials_json: context.credentials_json.clone(),
        jwt: executor_jwt,
        callback_url: context.callback_url.clone(),
        token: None,
        oauth_tokens: None,
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::Sync),
        runtime_variables: None,
        user_context: context.user_context.clone(),
        profile: context.profile.clone(),
        wasm_packages: context.wasm_packages.clone(),
        channel: None,
        trigger: DispatchTrigger::System,
        shadow: true,
        artifact: None,
    };

    let db_arc = crate::audit::ExecutionAuditContext::from(state);
    let dispatch_error = match state.dispatcher.backend() {
        ExecutionBackend::LambdaStream => {
            match state.dispatcher.dispatch_streaming(request).await {
                Ok((_response, byte_stream)) => {
                    collect_generic_result_bytes(
                        byte_stream,
                        run_id.clone(),
                        Some(db_arc),
                        CASE_TIMEOUT,
                    )
                    .await;
                    None
                }
                Err(error) => Some(format!("dispatch failed: {error}")),
            }
        }
        _ => match state.dispatcher.dispatch_http_sse(request).await {
            Ok((_response, executor_response)) => {
                collect_generic_result(
                    executor_response,
                    run_id.clone(),
                    Some(db_arc),
                    CASE_TIMEOUT,
                )
                .await;
                None
            }
            Err(error) => Some(format!("dispatch failed: {error}")),
        },
    };
    if dispatch_error.is_some() {
        fail_replay_run(state, &run_id).await;
    }

    (Some(run_id), dispatch_error)
}

/// Mark a replay run failed when its dispatch never reached the executor, so
/// the row does not sit `Pending` until the run sweeper finds it.
async fn fail_replay_run(state: &AppState, run_id: &str) {
    if let Err(error) = update_run_on_completion(
        &crate::audit::ExecutionAuditContext::from(state),
        run_id,
        RunStatus::Failed,
        0,
    )
    .await
    {
        tracing::warn!(run_id, %error, "Failed to mark the replay run as failed");
    }
}

// ---------------------------------------------------------------------------
// Maintenance: due schedules + suite-run liveness.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct RegressionMaintenanceOutcome {
    /// Scheduled suite runs dispatched this tick.
    pub dispatched: u64,
    /// `running` suite runs flipped to `errored` past wall clock + grace.
    pub swept: u64,
    /// Queued suite runs picked up and executed inline this tick.
    pub executed: u64,
}

/// One maintenance pass: flip dead `running` SuiteRuns to `errored`, dispatch
/// every due schedule (`nextRunAt <= now`) — advancing `nextRunAt` with a
/// conditional update so concurrent tickers dispatch each occurrence exactly
/// once — then execute queued suite runs inline (oldest first, small batch).
pub async fn maintenance_tick(state: &AppState) -> Result<RegressionMaintenanceOutcome, ApiError> {
    let mut outcome = RegressionMaintenanceOutcome::default();
    let now = now_naive();

    let liveness_cutoff = now
        - chrono::Duration::from_std(SUITE_WALL_CLOCK + SUITE_LIVENESS_GRACE)
            .unwrap_or_else(|_| chrono::Duration::minutes(20));
    // Keyed on startedAt, not createdAt: a queued run may start long after
    // its row was inserted, and every `running` row has a startedAt. Dead
    // rows are flipped in primary-key pages, each its own transaction.
    outcome.swept = update_in_batches::<regression_suite_run::Entity>(
        &state.db,
        state.db_dialect,
        Condition::all()
            .add(regression_suite_run::Column::Status.eq(SUITE_RUN_RUNNING))
            .add(regression_suite_run::Column::StartedAt.lt(liveness_cutoff)),
        vec![
            (
                regression_suite_run::Column::Status,
                Expr::value(SUITE_RUN_ERRORED),
            ),
            (
                regression_suite_run::Column::CompletedAt,
                Expr::value(Some(now)),
            ),
            (
                regression_suite_run::Column::Error,
                Expr::value(Some(
                    "Suite run exceeded the wall clock plus grace without completing".to_string(),
                )),
            ),
        ],
        DEFAULT_WRITE_CHUNK,
    )
    .await
    .map_err(|e| ApiError::internal_error(anyhow!("Suite-run liveness sweep failed: {e}")))?;

    let due: Vec<regression_suite::Model> = regression_suite::Entity::find()
        .filter(regression_suite::Column::Schedule.is_not_null())
        .filter(regression_suite::Column::NextRunAt.is_not_null())
        .filter(regression_suite::Column::NextRunAt.lte(now))
        .order_by_asc(regression_suite::Column::NextRunAt)
        .limit(SCHEDULE_DISPATCH_CAP)
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to load due suites: {e}")))?;

    for row in due {
        let Some(schedule) = row.schedule.as_deref() else {
            continue;
        };
        let next = match flow_like_sinks::scheduler::next_cron_occurrence_utc(schedule) {
            Ok(next) => next.fixed_offset(),
            Err(error) => {
                tracing::warn!(suite_id = %row.id, %error, "Stored cron schedule no longer parses; skipping");
                continue;
            }
        };
        // Claim the occurrence: only the ticker whose conditional update wins
        // dispatches it.
        let claimed = regression_suite::Entity::update_many()
            .set(regression_suite::ActiveModel {
                next_run_at: Set(Some(next)),
                ..Default::default()
            })
            .filter(regression_suite::Column::Id.eq(&row.id))
            .filter(regression_suite::Column::NextRunAt.eq(row.next_run_at))
            .exec(&state.db)
            .await
            .map_err(|e| {
                ApiError::internal_error(anyhow!("Failed to advance a suite schedule: {e}"))
            })?;
        if claimed.rows_affected == 0 {
            continue;
        }

        let app = match state.master_app("regression", &row.app_id, state).await {
            Ok(app) => app,
            Err(error) => {
                tracing::warn!(suite_id = %row.id, app_id = %row.app_id, %error, "Failed to load app for a scheduled suite run");
                continue;
            }
        };
        let suite = load_core_suite(&app, &row).await;
        match spawn_suite_run(
            state.clone(),
            row.app_id.clone(),
            suite,
            CandidateVersion::LatestPublished,
            SuiteRunTrigger::Schedule,
            SuiteRunActor::default(),
        )
        .await
        {
            Ok((suite_run_id, _)) => {
                outcome.dispatched += 1;
                tracing::info!(suite_id = %row.id, suite_run_id = %suite_run_id, "Dispatched scheduled regression suite run");
            }
            Err(error) => {
                tracing::warn!(suite_id = %row.id, %error, "Scheduled regression suite run refused");
            }
        }
    }

    outcome.executed = drain_queued_suite_runs(state).await?;

    Ok(outcome)
}

/// Queued suite runs executed inline per maintenance tick. Deliberately
/// small: on stateless deployments the whole tick runs inside one
/// `POST /maintenance/run` invocation.
const QUEUED_PICKUP_CAP: u64 = 3;

/// Execute queued suite runs, oldest first. Each run is claimed with a
/// conditional `queued → running` update so concurrent tickers execute it
/// exactly once.
async fn drain_queued_suite_runs(state: &AppState) -> Result<u64, ApiError> {
    let queued: Vec<regression_suite_run::Model> = regression_suite_run::Entity::find()
        .filter(regression_suite_run::Column::Status.eq(SUITE_RUN_QUEUED))
        .order_by_asc(regression_suite_run::Column::CreatedAt)
        .limit(QUEUED_PICKUP_CAP)
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to load queued suite runs: {e}")))?;

    let mut executed = 0;
    for row in queued {
        let claimed = regression_suite_run::Entity::update_many()
            .set(regression_suite_run::ActiveModel {
                status: Set(SUITE_RUN_RUNNING.to_string()),
                started_at: Set(Some(now_naive())),
                ..Default::default()
            })
            .filter(regression_suite_run::Column::Id.eq(&row.id))
            .filter(regression_suite_run::Column::Status.eq(SUITE_RUN_QUEUED))
            .exec(&state.db)
            .await
            .map_err(|e| {
                ApiError::internal_error(anyhow!("Failed to claim a queued suite run: {e}"))
            })?;
        if claimed.rows_affected == 0 {
            continue;
        }
        execute_claimed_suite_run(state, &row).await;
        executed += 1;
    }
    Ok(executed)
}

/// Execute one claimed (`queued → running`) suite run inline. The actor was
/// never persisted on the row, so the replay identity resolves like a
/// scheduled run's; every setup failure lands on the row as `errored`.
async fn execute_claimed_suite_run(state: &AppState, row: &regression_suite_run::Model) {
    let setup: Result<(CoreRegressionSuite, Option<(u32, u32, u32)>), String> = async {
        let suite_row = regression_suite::Entity::find_by_id(&row.suite_id)
            .one(&state.db)
            .await
            .map_err(|e| format!("failed to load the suite row: {e}"))?
            .ok_or_else(|| "the suite was deleted while the run was queued".to_string())?;
        let app = state
            .master_app("regression", &row.app_id, state)
            .await
            .map_err(|e| format!("failed to load the app: {e}"))?;
        let suite = load_core_suite(&app, &suite_row).await;
        let dispatch_version = if row.board_version == DRAFT_VERSION_LABEL {
            None
        } else {
            Some(
                crate::routes::app::events::parse_version_tuple(&row.board_version).ok_or_else(
                    || format!("unparseable stored board version '{}'", row.board_version),
                )?,
            )
        };
        Ok((suite, dispatch_version))
    }
    .await;

    match setup {
        Ok((suite, dispatch_version)) => {
            run_suite_to_completion(
                state,
                &row.app_id,
                &suite,
                &row.id,
                dispatch_version,
                SuiteRunActor::default(),
            )
            .await;
        }
        Err(message) => {
            if let Err(error) = finalize_suite_run(state, &row.id, Err(message)).await {
                tracing::error!(suite_run_id = %row.id, %error, "Failed to finalize a queued suite run");
            }
        }
    }
}

const DEFAULT_WORKER_INTERVAL_SECS: u64 = 60;

/// Spawn the RegressionSuites maintenance worker on long-running API
/// processes — the same places `spawn_run_sweeper` is spawned. Stateless
/// deployments drive the identical pass (schedules, liveness sweep, queued
/// pickup) through `POST /maintenance/run` (`job: regression_suites`)
/// instead.
///
/// Env: `REGRESSION_SUITES_DISABLED=1` skips it,
/// `REGRESSION_SUITES_INTERVAL_SECS` overrides the 60s tick.
pub fn spawn_regression_suites_worker(
    state: AppState,
) -> Option<flow_like_types::tokio::task::JoinHandle<()>> {
    if std::env::var("REGRESSION_SUITES_DISABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        tracing::info!("Regression suites worker disabled via REGRESSION_SUITES_DISABLED");
        return None;
    }
    let interval = std::env::var("REGRESSION_SUITES_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_WORKER_INTERVAL_SECS);

    tracing::info!(
        interval_secs = interval,
        "Spawning regression suites worker"
    );

    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match maintenance_tick(&state).await {
                Ok(outcome)
                    if outcome.dispatched > 0 || outcome.swept > 0 || outcome.executed > 0 =>
                {
                    tracing::info!(
                        dispatched = outcome.dispatched,
                        swept = outcome.swept,
                        executed = outcome.executed,
                        "Regression suites maintenance tick completed"
                    );
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(%error, "Regression suites maintenance tick failed");
                }
            }
        }
    }))
}

/// Publish hook for `PATCH /apps/{app_id}/board/{board_id}`: one indexed
/// projection query on `(appId, boardId, triggerOnPublish)`, run entirely on a
/// detached task so it can never block or fail the publish.
pub fn spawn_publish_triggered_suites(
    state: &AppState,
    publisher_sub: &str,
    app_id: &str,
    board_id: &str,
    published: (u32, u32, u32),
) {
    let state = state.clone();
    let publisher_sub = publisher_sub.to_string();
    let app_id = app_id.to_string();
    let board_id = board_id.to_string();
    tokio::spawn(async move {
        let rows = regression_suite::Entity::find()
            .filter(regression_suite::Column::AppId.eq(&app_id))
            .filter(regression_suite::Column::BoardId.eq(&board_id))
            .filter(regression_suite::Column::TriggerOnPublish.eq(true))
            .all(&state.db)
            .await;
        let rows = match rows {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(app_id = %app_id, board_id = %board_id, %error, "Publish-triggered suite lookup failed");
                return;
            }
        };
        if rows.is_empty() {
            return;
        }
        let app = match state.master_app(&publisher_sub, &app_id, &state).await {
            Ok(app) => app,
            Err(error) => {
                tracing::warn!(app_id = %app_id, %error, "Failed to load app for publish-triggered suites");
                return;
            }
        };
        for row in rows {
            let suite = load_core_suite(&app, &row).await;
            match spawn_suite_run(
                state.clone(),
                app_id.clone(),
                suite,
                CandidateVersion::Pinned(published),
                SuiteRunTrigger::Publish,
                SuiteRunActor {
                    sub: Some(publisher_sub.clone()),
                    user_context: None,
                },
            )
            .await
            {
                Ok((suite_run_id, _)) => {
                    tracing::info!(suite_id = %row.id, suite_run_id = %suite_run_id, "Publish-triggered regression suite run dispatched");
                }
                Err(error) => {
                    tracing::warn!(suite_id = %row.id, %error, "Publish-triggered regression suite run refused");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_mode_round_trips_the_projection_strings() {
        for mode in [GateMode::Off, GateMode::Warn, GateMode::Block] {
            assert_eq!(parse_gate_mode(gate_mode_as_str(mode)), Some(mode));
        }
        assert_eq!(parse_gate_mode("off"), None);
        assert_eq!(parse_gate_mode("BLOCK"), None);
    }

    #[test]
    fn outcome_labels_match_the_case_result_contract() {
        assert_eq!(outcome_label(&CaseOutcome::Ok), "ok");
        assert_eq!(outcome_label(&CaseOutcome::Regressed), "regressed");
        assert_eq!(
            outcome_label(&CaseOutcome::StillFailing {
                error_class_changed: true
            }),
            "still_failing"
        );
        assert_eq!(outcome_label(&CaseOutcome::Fixed), "fixed");
    }

    #[test]
    fn triggers_serialize_to_the_run_row_strings() {
        assert_eq!(SuiteRunTrigger::Manual.as_str(), "manual");
        assert_eq!(SuiteRunTrigger::Publish.as_str(), "publish");
        assert_eq!(SuiteRunTrigger::Schedule.as_str(), "schedule");
    }
}
