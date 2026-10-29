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
use flow_like::flow::event::Event as CoreEvent;
use flow_like::flow::execution::LogLevel;
use flow_like::flow::execution::log::StoredLogMessage;
use flow_like::flow::execution::run_index::runs_base_path;
use flow_like::flow::regression::{
    CaseOutcome, FixtureBaseline, GateMode, RegressionSuite as CoreRegressionSuite, RunGrade,
    RunGradeEvidence, SuiteCase, compare_to_expectation, error_class_of, grade_run,
    plan_suite_cases,
};
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
        DispatchRequest, DispatchTrigger, ExecutionBackend, ExecutionJwtError, ExecutionJwtParams,
        TokenType, collect_generic_result, collect_generic_result_bytes, format_run_version,
        is_jwt_configured, resolve_wasm_packages, sign_execution_jwt,
        sign_unattended_execution_jwt, subject::account_user_id, update_run_on_completion,
    },
    middleware::jwt::SHADOW_SIDE_EFFECT_REFUSED,
    permission::role_permission::RolePermissions,
    routes::sink::authority::{RunAsDenial, RunAsVerdict, run_as_authority},
    routes::sink::trigger::recheck_fire,
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

/// The status fold of a replay that failed without a reason on its run row: its board
/// reported node errors, and its error logs say why.
const REPLAY_FAILED: &str = "replay ended with status Failed";
const REFUSAL_LEAD: &str = "Shadow and regression runs cannot ";
const REFUSED_EFFECT_MAX_CHARS: usize = 200;

const ACTOR_MISSING: &str = "The run was queued without the person who started it; start it again.";
const SCHEDULE_DRAFT_REFUSED: &str = "Scheduled suites replay published versions only.";

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

    /// The trigger a run row stores; the inverse of [`Self::as_str`].
    pub fn from_row(raw: &str) -> Option<Self> {
        [Self::Manual, Self::Publish, Self::Schedule]
            .into_iter()
            .find(|trigger| trigger.as_str() == raw)
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
/// the publisher's; scheduled runs act as the run-as user of the event's sink
/// where that sink runs the candidate, else as a `regression:{suite_id}`
/// placeholder with `userId` NULL.
#[derive(Clone, Debug, Default)]
pub struct SuiteRunActor {
    pub sub: Option<String>,
    /// The API key a manual run was started with. Its replays act with the key's role, as
    /// the key's own runs do, and lend its creator's private Bits like them. A queued row
    /// stores only the account, so a queued pickup replays with the account's role.
    pub technical_user_id: Option<String>,
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

/// Open the board's Lance log database — the connection its per-run log
/// tables are read through.
pub(crate) async fn open_runs_db(
    state: &AppState,
    sub: &str,
    app_id: &str,
    board_id: &str,
) -> Result<flow_like_storage::lancedb::Connection, ApiError> {
    let credentials = state
        .scoped_credentials(sub, app_id, CredentialsAccess::ReadLogs)
        .await?;
    let logs_db_builder = credentials
        .into_shared_credentials()
        .to_logs_db_builder()
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to create logs db builder: {}", e))
        })?;
    let base_path = runs_base_path(app_id, board_id);
    logs_db_builder(base_path.clone())
        .execute()
        .await
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to open runs database at {base_path}: {e}"))
        })
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

/// A suite that replays an event must not run it where the event may not run: a device-only
/// event executes on its deployed devices, never on the hub. Checked when a run starts and
/// again when a queued run is picked up, since the event can change in between.
async fn ensure_suite_event_may_run_here(
    state: &AppState,
    app_id: &str,
    suite: &CoreRegressionSuite,
) -> Result<(), ApiError> {
    let Some(event_id) = suite.event_id.as_deref() else {
        return Ok(());
    };
    match crate::routes::app::events::db::get_event_from_db_opt(&state.db, event_id, app_id).await?
    {
        Some(event) => crate::routes::app::events::ensure_source_execution_allowed(&event),
        None => Ok(()),
    }
}

const LIVE_SIDE_EFFECTS_UNACKNOWLEDGED: &str = "This suite has not acknowledged live side effects. Replays cannot write app storage, change the app cache, send notifications, call connected apps, change locked files, send platform mail or Teams messages, take payments, or refresh OAuth handles, but outbound HTTP from native nodes still runs. On storage providers without per-prefix access control (R2), storage isolation is enforced in the runtime only. Set allow_live_side_effects: true on the suite before running it.";

/// A suite replays only while it acknowledges live side effects. Checked when a run starts
/// and again when a queued run is picked up, since the suite can change while the run waits.
fn ensure_live_side_effects_acknowledged(suite: &CoreRegressionSuite) -> Result<(), ApiError> {
    if suite.allow_live_side_effects {
        return Ok(());
    }
    Err(ApiError::conflict(LIVE_SIDE_EFFECTS_UNACKNOWLEDGED))
}

/// The account a suite run records as the person who started it. Manual and publish runs
/// replay as that account; a placeholder or pseudonym never names one.
fn actor_account(
    trigger: SuiteRunTrigger,
    actor: &SuiteRunActor,
) -> Result<Option<String>, ApiError> {
    let account = actor
        .sub
        .as_deref()
        .and_then(account_user_id)
        .map(ToOwned::to_owned);
    if account.is_none() && matches!(trigger, SuiteRunTrigger::Manual | SuiteRunTrigger::Publish) {
        return Err(ApiError::internal_error(anyhow!(
            "{} suite runs require the caller's account, got subject {:?}",
            trigger.as_str(),
            actor.sub
        )));
    }
    Ok(account)
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
    ensure_live_side_effects_acknowledged(&suite)?;
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured (missing BACKEND_KEY/BACKEND_PUB)"
        )));
    }
    let actor_user_id = actor_account(trigger, &actor)?;

    ensure_suite_event_may_run_here(&state, &app_id, &suite).await?;

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
        actor_user_id: Set(actor_user_id),
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
    let body = async {
        let identity = replay_identity(state, app_id, suite, dispatch_version, actor).await?;
        execute_suite_run(
            state,
            app_id,
            suite,
            suite_run_id,
            dispatch_version,
            identity,
        )
        .await
        .map_err(|error| suite_run_error(&error))
    };
    let outcome = match tokio::time::timeout(SUITE_WALL_CLOCK, body).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "suite run exceeded the {} minute wall clock",
            SUITE_WALL_CLOCK.as_secs() / 60
        )),
    };
    if let Err(error) = finalize_suite_run(state, suite_run_id, outcome).await {
        tracing::error!(suite_run_id = %suite_run_id, %error, "Failed to finalize suite run");
    }
}

/// What a suite-run row records for an API error: its public message when it has one.
fn suite_run_error(error: &ApiError) -> String {
    error
        .public_message()
        .map_or_else(|| error.to_string(), str::to_owned)
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

/// Who a suite run's cases dispatch as, and the sink whose profile they carry.
struct ReplayIdentity {
    subject: String,
    /// The account the replays act as and lend their private Bits; `None` for the
    /// placeholder.
    run_user_id: Option<String>,
    /// The API key that started the run, whose role bounds the replays' callbacks.
    technical_user_id: Option<String>,
    user_context: Option<flow_like::flow::execution::UserExecutionContext>,
    sink: Option<event_sink::Model>,
    /// No person started the run: a scheduled run replays as the user the event runs as.
    unattended: bool,
}

/// Manual and publish runs replay as the person who started them, checked again when the
/// run starts, whether it was spawned or picked up from the queue.
async fn replay_identity(
    state: &AppState,
    app_id: &str,
    suite: &CoreRegressionSuite,
    dispatch_version: Option<(u32, u32, u32)>,
    actor: SuiteRunActor,
) -> Result<ReplayIdentity, String> {
    let Some(sub) = actor.sub else {
        return resolve_schedule_identity(state, app_id, suite, dispatch_version).await;
    };
    ensure_actor_may_replay(state, app_id, &sub).await?;
    Ok(ReplayIdentity {
        subject: sub.clone(),
        run_user_id: Some(sub),
        technical_user_id: actor.technical_user_id,
        user_context: actor.user_context,
        sink: None,
        unattended: false,
    })
}

/// The scheduled-trigger identity: the run-as user of the event's sink where that sink
/// already runs the candidate as them ([`sink_runs_candidate`]), else a
/// `regression:{suite_id}` placeholder (mirroring the `sink:{id}` /
/// `inbound:{event_id}` convention) with `userId` NULL, also for a legacy sink
/// that only stores a PAT. Read-only: a refusal errors this suite run, never
/// pauses the sink and records nothing on the event.
async fn resolve_schedule_identity(
    state: &AppState,
    app_id: &str,
    suite: &CoreRegressionSuite,
    dispatch_version: Option<(u32, u32, u32)>,
) -> Result<ReplayIdentity, String> {
    let candidate = dispatch_version.ok_or_else(|| SCHEDULE_DRAFT_REFUSED.to_string())?;
    let (sink, run_as) = schedule_sink(state, app_id, suite, candidate).await?;
    let verdict = run_as_authority(
        &state.db,
        app_id,
        run_as.as_deref(),
        RolePermissions::ExecuteEvents,
    )
    .await
    .map_err(|e| format!("failed to check who the event runs as: {e}"))?;
    match verdict {
        RunAsVerdict::Allowed(_) => Ok(ReplayIdentity {
            subject: run_as
                .clone()
                .unwrap_or_else(|| format!("regression:{}", suite.id)),
            run_user_id: run_as,
            technical_user_id: None,
            user_context: None,
            sink,
            unattended: true,
        }),
        RunAsVerdict::Denied(denial) => Err(denial.reason().to_string()),
        RunAsVerdict::Transient(reason) => {
            Err(format!("could not check who the event runs as: {reason}"))
        }
    }
}

/// The suite event's sink, and its run-as user where the sink already runs the candidate as
/// them. The event and the sink are read together when a scheduled run starts.
async fn schedule_sink(
    state: &AppState,
    app_id: &str,
    suite: &CoreRegressionSuite,
    candidate: (u32, u32, u32),
) -> Result<(Option<event_sink::Model>, Option<String>), String> {
    let Some(event_id) = suite.event_id.as_deref() else {
        return Ok((None, None));
    };
    let event = crate::routes::app::events::db::get_event_from_db_opt(&state.db, event_id, app_id)
        .await
        .map_err(|e| format!("failed to load the event: {e}"))?;
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(app_id))
        .filter(event_sink::Column::EventId.eq(event_id))
        .one(&state.db)
        .await
        .map_err(|e| format!("failed to load the event sink: {e}"))?;
    let run_as = match (&event, &sink) {
        (Some(event), Some(sink))
            if sink_runs_candidate(event, sink.active, &suite.board_id, candidate) =>
        {
            sink.run_as_user_id.clone()
        }
        _ => None,
    };
    Ok((sink, run_as))
}

/// Whether the sink's trigger already runs `candidate` of `board_id` as its run-as user,
/// through the event's primary target or one of its variants. A pinned target runs exactly
/// its version, and only a save of the event, which binds the person saving, moves the pin.
/// A target on latest runs what its run-as user changed last only while the sink is on,
/// because board edits rebind active sinks only.
fn sink_runs_candidate(
    event: &CoreEvent,
    sink_active: bool,
    board_id: &str,
    candidate: (u32, u32, u32),
) -> bool {
    let variants = event.variant_set();
    std::iter::once((event.board_id.as_str(), event.board_version))
        .chain(
            variants
                .iter()
                .map(|variant| (variant.board_id.as_str(), variant.board_version)),
        )
        .any(|(target, version)| {
            target == board_id && version.map_or(sink_active, |pinned| pinned == candidate)
        })
}

/// A sink's profile follows its run-as user; an actor carries their own profile and the
/// placeholder none.
async fn replay_profile(
    state: &AppState,
    app_id: &str,
    identity: &ReplayIdentity,
) -> Option<serde_json::Value> {
    match (&identity.sink, identity.run_user_id.as_deref()) {
        (Some(sink), run_as) => crate::execution::sink_dispatch_profile(state, sink, run_as).await,
        (None, Some(user_id)) => {
            crate::execution::fetch_profile_for_dispatch(state, user_id, None, app_id).await
        }
        (None, None) => None,
    }
}

/// The sink a scheduled run borrows its run-as user from while that sink is on, as the run
/// read it. A board edit or a publication rebinds an active sink only, so each case reads it
/// again before it dispatches.
fn lent_sink(identity: &ReplayIdentity) -> Option<event_sink::Model> {
    identity
        .sink
        .as_ref()
        .filter(|sink| {
            sink.active
                && identity.run_user_id.is_some()
                && sink.run_as_user_id == identity.run_user_id
        })
        .cloned()
}

/// Who a queued suite run replays as once the maintenance job picks it up.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PickupIdentity {
    /// The person who started a manual or publish run.
    Actor(String),
    /// The run-as user of the event's sink, or the placeholder.
    Schedule,
}

/// A manual or publish run replays as the person who started it and never falls back to
/// the schedule identity; a scheduled run never replays a draft.
fn pickup_identity(
    trigger: SuiteRunTrigger,
    actor: Option<&str>,
    candidate: CandidateVersion,
) -> Result<PickupIdentity, &str> {
    match trigger {
        SuiteRunTrigger::Manual | SuiteRunTrigger::Publish => actor
            .filter(|actor| !actor.trim().is_empty())
            .map(|actor| PickupIdentity::Actor(actor.to_string()))
            .ok_or(ACTOR_MISSING),
        SuiteRunTrigger::Schedule if candidate == CandidateVersion::Draft => {
            Err(SCHEDULE_DRAFT_REFUSED)
        }
        SuiteRunTrigger::Schedule => Ok(PickupIdentity::Schedule),
    }
}

/// Why the person who started a queued run may not replay it now. Replaying needs an
/// active account that is a member of the active project, with a role that executes
/// events and reads the logs grading needs.
fn actor_refusal(verdict: RunAsVerdict) -> Option<String> {
    let reason = match verdict {
        RunAsVerdict::Allowed(_) => return None,
        RunAsVerdict::Transient(reason) => {
            return Some(format!(
                "could not check the person who started the run: {reason}"
            ));
        }
        RunAsVerdict::Denied(RunAsDenial::AppInactive) => RunAsDenial::AppInactive.reason(),
        RunAsVerdict::Denied(RunAsDenial::AccountInactive) => {
            "The account of the person who started the run is not active."
        }
        RunAsVerdict::Denied(RunAsDenial::NotMember) => {
            "The person who started the run is not a member of this project."
        }
        RunAsVerdict::Denied(RunAsDenial::CannotTrigger | RunAsDenial::TokenInvalid) => {
            "The role of the person who started the run does not allow executing events and reading logs."
        }
    };
    Some(reason.to_string())
}

async fn ensure_actor_may_replay(
    state: &AppState,
    app_id: &str,
    actor: &str,
) -> Result<(), String> {
    let verdict = run_as_authority(
        &state.db,
        app_id,
        Some(actor),
        RolePermissions::ExecuteEvents | RolePermissions::ReadLogs,
    )
    .await
    .map_err(|e| format!("failed to check the person who started the run: {e}"))?;
    actor_refusal(verdict).map_or(Ok(()), Err)
}

struct CaseDispatchContext {
    app_id: String,
    suite_run_id: String,
    event_id: Option<String>,
    dispatch_version: Option<(u32, u32, u32)>,
    subject: String,
    run_user_id: Option<String>,
    technical_user_id: Option<String>,
    user_context: Option<flow_like::flow::execution::UserExecutionContext>,
    credentials_json: String,
    callback_url: String,
    profile: Option<serde_json::Value>,
    wasm_packages:
        Option<std::collections::HashMap<String, flow_like_types::dispatch::WasmPackageRef>>,
    board_id: String,
    grading_blind: bool,
    /// See [`lent_sink`].
    run_as_sink: Option<event_sink::Model>,
    /// See [`ReplayIdentity::unattended`].
    unattended: bool,
}

async fn execute_suite_run(
    state: &AppState,
    app_id: &str,
    suite: &CoreRegressionSuite,
    suite_run_id: &str,
    dispatch_version: Option<(u32, u32, u32)>,
    identity: ReplayIdentity,
) -> Result<(), ApiError> {
    let app = state.master_app("regression", app_id, state).await?;

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
        .scoped_credentials(&identity.subject, app_id, CredentialsAccess::ShadowExecute)
        .await?;
    let credentials_json = serde_json::to_string(&credentials.into_shared_credentials())
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize credentials: {e}")))?;

    let profile = replay_profile(state, app_id, &identity).await;
    let wasm_packages = resolve_wasm_packages(state, app_id).await;
    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let logs_db = open_runs_db(state, &identity.subject, app_id, &suite.board_id).await?;

    let run_as_sink = lent_sink(&identity);
    let context = Arc::new(CaseDispatchContext {
        app_id: app_id.to_string(),
        suite_run_id: suite_run_id.to_string(),
        event_id: suite.event_id.clone(),
        dispatch_version,
        subject: identity.subject,
        run_user_id: identity.run_user_id,
        technical_user_id: identity.technical_user_id,
        user_context: identity.user_context,
        credentials_json,
        callback_url,
        profile,
        wasm_packages,
        board_id: suite.board_id.clone(),
        grading_blind,
        run_as_sink,
        unattended: identity.unattended,
    });

    futures::stream::iter(plan.cases.into_iter().map(|case| {
        let state = state.clone();
        let context = context.clone();
        let logs_db = logs_db.clone();
        async move { run_case(&state, &context, &logs_db, case).await }
    }))
    .buffer_unordered(concurrency)
    .collect::<Vec<Result<(), ApiError>>>()
    .await
    .into_iter()
    .collect()
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

/// A case dispatches only while the sink whose run-as user the run borrowed still runs as that
/// user, as a sink fire does. Otherwise the case is skipped with the retryable refusal, which
/// errors the suite run.
async fn recheck_lent_sink(
    state: &AppState,
    context: &CaseDispatchContext,
    case_kind: &str,
    case_ref: &str,
) -> Result<(), ApiError> {
    let Some(sink) = &context.run_as_sink else {
        return Ok(());
    };
    let Err(overtaken) = recheck_fire(state, sink).await else {
        return Ok(());
    };
    insert_case_result(
        &state.db,
        &context.suite_run_id,
        case_kind,
        case_ref,
        None,
        OUTCOME_SKIPPED,
        "skipped",
        Some(serde_json::json!({ "reason": suite_run_error(&overtaken) })),
        None,
    )
    .await;
    Err(overtaken)
}

/// Dispatch one case (board-invoke shape, shadow-signed), wait for completion
/// and grade verdict-vs-baseline. Every failure mode lands as a graded case
/// row — a case never disappears silently.
async fn run_case(
    state: &AppState,
    context: &CaseDispatchContext,
    logs_db: &flow_like_storage::lancedb::Connection,
    case: SuiteCase,
) -> Result<(), ApiError> {
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

    recheck_lent_sink(state, context, case_kind, &case_ref).await?;
    let started = std::time::Instant::now();
    let (replay_run_id, dispatch_error) = dispatch_case(state, context, &node_id, payload).await;

    let mut evidence = match &replay_run_id {
        Some(run_id) => collect_replay_evidence(logs_db, run_id).await,
        None => RunGradeEvidence::default(),
    };

    let refusal = match (dispatch_error, &replay_run_id) {
        (Some(error), _) => {
            evidence.execution_error = Some(error);
            None
        }
        (None, Some(run_id)) => {
            fold_run_status(state, run_id, &mut evidence).await;
            refusal_reason(logs_db, run_id, &evidence).await
        }
        (None, None) => None,
    };

    let grade = grade_run(evidence);
    let outcome = compare_to_expectation(&baseline, &grade);
    let duration_ms = i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX);
    let label = if refusal.is_some() {
        OUTCOME_SKIPPED
    } else {
        outcome_label(&outcome)
    };
    let detail = case_detail(
        &grade,
        &baseline,
        &outcome,
        context.grading_blind,
        alias,
        refusal,
    );

    insert_case_result(
        &state.db,
        &context.suite_run_id,
        case_kind,
        &case_ref,
        replay_run_id,
        label,
        grade.verdict.as_str(),
        Some(detail),
        Some(duration_ms),
    )
    .await;
    Ok(())
}

/// What a case row records besides its outcome: error classes, failed assertions, the
/// grading-blind stamp, the authored test's alias and why a refused replay was skipped.
fn case_detail(
    grade: &RunGrade,
    baseline: &FixtureBaseline,
    outcome: &CaseOutcome,
    grading_blind: bool,
    alias: Option<String>,
    refusal: Option<String>,
) -> serde_json::Value {
    let mut detail = serde_json::json!({
        "error_class": error_class_of(grade),
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
        detail["error_class_changed"] = serde_json::Value::Bool(*error_class_changed);
    }
    if grading_blind {
        detail["grading_blind"] = serde_json::Value::Bool(true);
    }
    if let Some(alias) = alias {
        detail["alias"] = serde_json::Value::String(alias);
    }
    if let Some(reason) = refusal {
        detail["reason"] = serde_json::Value::String(reason);
    }
    detail
}

/// Fold the run row's terminal status into the evidence: a replay that failed,
/// timed out or never completed must not grade on quiet logs.
async fn fold_run_status(state: &AppState, run_id: &str, evidence: &mut RunGradeEvidence) {
    match execution_run::Entity::find_by_id(run_id)
        .one(&state.db)
        .await
    {
        Ok(Some(run)) => {
            if let Some(error) = status_error(&run.status, run.error_message.as_deref()) {
                evidence.execution_error.get_or_insert(error);
            }
        }
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

/// A failed run's own reason, such as an executor crash, outranks [`REPLAY_FAILED`].
fn status_error(status: &RunStatus, error_message: Option<&str>) -> Option<String> {
    match status {
        RunStatus::Completed => None,
        RunStatus::Pending | RunStatus::Running => Some(format!(
            "replay did not complete within the {}s case timeout",
            CASE_TIMEOUT.as_secs()
        )),
        RunStatus::Failed => Some(error_message.unwrap_or(REPLAY_FAILED).to_string()),
        status => Some(format!("replay ended with status {status:?}")),
    }
}

/// D6: why a replay grades as skipped when replay isolation refusing a side effect is its
/// only failure. Every error log and the execution error carry
/// [`SHADOW_SIDE_EFFECT_REFUSED`] and no assertion failed; a run that ended `Failed`
/// without a reason of its own counts when its error logs explain it.
fn refused_side_effect_only(evidence: &RunGradeEvidence) -> Option<String> {
    if !evidence.metadata_present || evidence.log_query_failed {
        return None;
    }
    let refused = |message: &str| message.contains(SHADOW_SIDE_EFFECT_REFUSED);
    let assert_failed = evidence
        .assert_logs
        .iter()
        .any(|message| message.starts_with("ASSERT_FAIL"));
    let execution_error_refused = match evidence.execution_error.as_deref() {
        None => true,
        Some(REPLAY_FAILED) => !evidence.error_logs.is_empty(),
        Some(error) => refused(error),
    };
    if assert_failed
        || !execution_error_refused
        || !evidence.error_logs.iter().all(|message| refused(message))
    {
        return None;
    }
    let effect = evidence
        .error_logs
        .iter()
        .map(String::as_str)
        .chain(evidence.execution_error.as_deref())
        .find_map(refused_effect)?;
    Some(format!("side effect refused: {effect}"))
}

/// The effect a refusal names, such as "change the app cache", in the API's answer, the
/// catalog's error or a log line quoting either. Detail after a colon, such as the path of
/// a blocked app-storage write, is not part of the effect.
fn refused_effect(message: &str) -> Option<String> {
    let (_, after_code) = message.split_once(SHADOW_SIDE_EFFECT_REFUSED)?;
    let named = message
        .split_once(REFUSAL_LEAD)
        .map_or(after_code, |(_, effect)| effect);
    let effect: String = named
        .trim_start_matches([':', ' '])
        .split(['"', '\n', ':'])
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches('.')
        .chars()
        .take(REFUSED_EFFECT_MAX_CHARS)
        .collect();
    Some(if effect.is_empty() {
        "unnamed side effect".to_string()
    } else {
        effect
    })
}

/// [`refused_side_effect_only`], confirmed against the whole log table when the error-log
/// sample is full, so a failure past the sample cannot hide behind refusals.
async fn refusal_reason(
    db: &flow_like_storage::lancedb::Connection,
    run_id: &str,
    evidence: &RunGradeEvidence,
) -> Option<String> {
    let reason = refused_side_effect_only(evidence)?;
    if evidence.error_logs.len() < GRADE_ERROR_LOG_LIMIT {
        return Some(reason);
    }
    let unrefused = format!("log_level >= 3 AND message NOT LIKE '%{SHADOW_SIDE_EFFECT_REFUSED}%'");
    match run_log_messages(db, run_id, &unrefused, 1).await {
        Ok(messages) if messages.is_empty() => Some(reason),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(%error, run_id, "Unrefused error-log query failed while grading");
            None
        }
    }
}

/// The replay's `ExecutionRun` row. Only an account fills its `User` foreign key; the API
/// key that started the suite run is recorded as its technical user.
fn replay_run_row(
    context: &CaseDispatchContext,
    run_id: &str,
    node_id: &str,
    input_payload_len: i64,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> execution_run::ActiveModel {
    execution_run::ActiveModel {
        id: Set(run_id.to_string()),
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
        user_id: Set(context
            .run_user_id
            .as_deref()
            .and_then(account_user_id)
            .map(ToOwned::to_owned)),
        technical_user_id: Set(context.technical_user_id.clone()),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.to_string())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(context.app_id.clone()),
        created_at: Set(now),
        updated_at: Set(now),
        event_version: Set(None),
        nodes: Set(None),
        logs_count: Set(None),
    }
}

/// The replay's executor token, signed as a shadow run. When an API key started the suite
/// run, the key's role bounds the replay's callbacks, as it bounds the key's own runs.
fn replay_jwt_params(context: &CaseDispatchContext, run_id: &str) -> ExecutionJwtParams {
    ExecutionJwtParams {
        user_id: context.subject.clone(),
        technical_user_id: context.technical_user_id.clone(),
        run_id: run_id.to_string(),
        app_id: context.app_id.clone(),
        board_id: context.board_id.clone(),
        event_id: None,
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(run_id)),
        callback_url: context.callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(60 * 60),
        shadow: Some(true),
    }
}

/// A scheduled replay, which no person started, is signed unattended.
fn sign_replay_jwt(
    context: &CaseDispatchContext,
    run_id: &str,
) -> Result<String, ExecutionJwtError> {
    let params = replay_jwt_params(context, run_id);
    if context.unattended {
        sign_unattended_execution_jwt(params)
    } else {
        sign_execution_jwt(params)
    }
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
    let input_payload_len = payload
        .as_ref()
        .and_then(|p| serde_json::to_string(p).ok())
        .map(|s| s.len() as i64)
        .unwrap_or(0);

    let run = replay_run_row(context, &run_id, node_id, input_payload_len, now_naive());
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

    let executor_jwt = match sign_replay_jwt(context, &run_id) {
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
        oauth_tokens: None,
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::Sync),
        runtime_variables: None,
        user_context: context.user_context.clone(),
        profile: context.profile.clone(),
        custom_bit_owner: context.run_user_id.clone(),
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

/// Execute one claimed (`queued → running`) suite run inline. Manual and
/// publish runs replay as the person who started them, checked again now;
/// scheduled runs resolve the schedule identity. Every setup failure lands on
/// the row as `errored`.
async fn execute_claimed_suite_run(state: &AppState, row: &regression_suite_run::Model) {
    match claimed_run_setup(state, row).await {
        Ok(claimed) => {
            run_suite_to_completion(
                state,
                &row.app_id,
                &claimed.suite,
                &row.id,
                claimed.dispatch_version,
                claimed.actor,
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

struct ClaimedRun {
    suite: CoreRegressionSuite,
    dispatch_version: Option<(u32, u32, u32)>,
    actor: SuiteRunActor,
}

async fn claimed_run_setup(
    state: &AppState,
    row: &regression_suite_run::Model,
) -> Result<ClaimedRun, String> {
    let (candidate, actor) = claimed_identity(row)?;
    let suite = claimed_suite(state, row).await?;
    let dispatch_version = match candidate {
        CandidateVersion::Pinned(version) => Some(version),
        _ => None,
    };
    Ok(ClaimedRun {
        suite,
        dispatch_version,
        actor,
    })
}

/// The stored candidate and who replays it, settled before anything else is loaded.
fn claimed_identity(
    row: &regression_suite_run::Model,
) -> Result<(CandidateVersion, SuiteRunActor), String> {
    let trigger = SuiteRunTrigger::from_row(&row.trigger)
        .ok_or_else(|| format!("unknown suite run trigger '{}'", row.trigger))?;
    let candidate = stored_candidate(&row.board_version)
        .ok_or_else(|| format!("unparseable stored board version '{}'", row.board_version))?;
    let actor = match pickup_identity(trigger, row.actor_user_id.as_deref(), candidate)? {
        PickupIdentity::Actor(sub) => SuiteRunActor {
            sub: Some(sub),
            technical_user_id: None,
            user_context: None,
        },
        PickupIdentity::Schedule => SuiteRunActor::default(),
    };
    Ok((candidate, actor))
}

async fn claimed_suite(
    state: &AppState,
    row: &regression_suite_run::Model,
) -> Result<CoreRegressionSuite, String> {
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
    ensure_live_side_effects_acknowledged(&suite).map_err(|e| suite_run_error(&e))?;
    ensure_suite_event_may_run_here(state, &row.app_id, &suite)
        .await
        .map_err(|e| suite_run_error(&e))?;
    Ok(suite)
}

/// The candidate a queued row stores as its `boardVersion` label.
fn stored_candidate(label: &str) -> Option<CandidateVersion> {
    if label == DRAFT_VERSION_LABEL {
        return Some(CandidateVersion::Draft);
    }
    crate::routes::app::events::parse_version_tuple(label).map(CandidateVersion::Pinned)
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
                    technical_user_id: None,
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
    use flow_like::flow::event::{CanaryEvent, EventVariant, EventVariantMode};
    use std::collections::HashMap;
    use std::time::SystemTime;

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
        for trigger in [
            SuiteRunTrigger::Manual,
            SuiteRunTrigger::Publish,
            SuiteRunTrigger::Schedule,
        ] {
            assert_eq!(SuiteRunTrigger::from_row(trigger.as_str()), Some(trigger));
        }
        assert_eq!(SuiteRunTrigger::from_row("Manual"), None);
    }

    #[test]
    fn queued_runs_replay_as_the_person_who_started_them() {
        use SuiteRunTrigger::{Manual, Publish, Schedule};
        let pinned = CandidateVersion::Pinned((1, 2, 3));
        let draft = CandidateVersion::Draft;
        let alice = Ok(PickupIdentity::Actor("alice".to_string()));
        let schedule = Ok(PickupIdentity::Schedule);
        for (trigger, actor, candidate, expected) in [
            (Manual, Some("alice"), pinned, alice.clone()),
            (Manual, Some("alice"), draft, alice.clone()),
            (Publish, Some("alice"), pinned, alice.clone()),
            (Manual, None, pinned, Err(ACTOR_MISSING)),
            (Manual, None, draft, Err(ACTOR_MISSING)),
            (Publish, None, pinned, Err(ACTOR_MISSING)),
            (Publish, Some(" "), pinned, Err(ACTOR_MISSING)),
            (Schedule, None, pinned, schedule.clone()),
            (Schedule, Some("alice"), pinned, schedule),
            (Schedule, None, draft, Err(SCHEDULE_DRAFT_REFUSED)),
            (Schedule, Some("alice"), draft, Err(SCHEDULE_DRAFT_REFUSED)),
        ] {
            assert_eq!(
                pickup_identity(trigger, actor, candidate),
                expected,
                "{trigger:?} {actor:?} {candidate:?}"
            );
        }
    }

    #[test]
    fn suite_runs_record_only_an_account_as_the_person_who_started_them() {
        use SuiteRunTrigger::{Manual, Publish, Schedule};
        let actor = |sub: Option<&str>| SuiteRunActor {
            sub: sub.map(str::to_string),
            ..SuiteRunActor::default()
        };
        let refused = Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        for (trigger, sub, expected) in [
            (Manual, Some("alice"), Ok(Some("alice"))),
            (Publish, Some("alice"), Ok(Some("alice"))),
            (Schedule, None, Ok(None)),
            (Schedule, Some("regression:suite"), Ok(None)),
            (Manual, None, refused),
            (Manual, Some("frontend:event:run-run-1"), refused),
            (Publish, Some("regression:suite"), refused),
            (Manual, Some(" "), refused),
        ] {
            let recorded = actor_account(trigger, &actor(sub)).map_err(|error| error.status());
            assert_eq!(
                recorded,
                expected.map(|account| account.map(str::to_string)),
                "{trigger:?} {sub:?}"
            );
        }
    }

    #[test]
    fn stored_board_versions_name_the_candidate() {
        assert_eq!(stored_candidate("draft"), Some(CandidateVersion::Draft));
        assert_eq!(
            stored_candidate("1.2.3"),
            Some(CandidateVersion::Pinned((1, 2, 3)))
        );
        assert_eq!(stored_candidate("latest"), None);
    }

    fn suite_config(allow_live_side_effects: bool) -> CoreRegressionSuite {
        CoreRegressionSuite {
            id: "suite".to_string(),
            board_id: "board".to_string(),
            event_id: Some("event".to_string()),
            node_id: "node".to_string(),
            trigger_on_publish: false,
            schedule: None,
            gate_mode: GateMode::Block,
            allow_live_side_effects,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn a_suite_replays_only_while_it_acknowledges_live_side_effects() {
        assert!(ensure_live_side_effects_acknowledged(&suite_config(true)).is_ok());
        let refusal = ensure_live_side_effects_acknowledged(&suite_config(false)).unwrap_err();
        assert_eq!(refusal.status(), axum::http::StatusCode::CONFLICT);
        assert_eq!(
            refusal.public_message(),
            Some(LIVE_SIDE_EFFECTS_UNACKNOWLEDGED)
        );
        // A queued run picked up after the acknowledgement was withdrawn is errored with
        // the words the 409 uses.
        assert_eq!(suite_run_error(&refusal), LIVE_SIDE_EFFECTS_UNACKNOWLEDGED);
    }

    #[test]
    fn a_queued_pickup_checks_the_acknowledgement_again() {
        let source = include_str!("mod.rs");
        let pickup = source
            .split_once("\nasync fn claimed_suite(")
            .expect("queued runs load their suite in claimed_suite")
            .1;
        let pickup = pickup.split_once("\n}\n").map_or(pickup, |(body, _)| body);
        assert!(pickup.contains("ensure_live_side_effects_acknowledged(&suite)"));
    }

    #[test]
    fn a_queued_actor_needs_an_active_member_who_executes_and_reads_logs() {
        let both = RolePermissions::ExecuteEvents | RolePermissions::ReadLogs;
        assert_eq!(actor_refusal(RunAsVerdict::Allowed(both)), None);
        assert_eq!(
            actor_refusal(RunAsVerdict::Denied(RunAsDenial::AppInactive)).as_deref(),
            Some(RunAsDenial::AppInactive.reason())
        );
        for denial in [
            RunAsDenial::AccountInactive,
            RunAsDenial::NotMember,
            RunAsDenial::CannotTrigger,
            RunAsDenial::TokenInvalid,
        ] {
            let reason = actor_refusal(RunAsVerdict::Denied(denial)).unwrap();
            assert!(reason.contains("person who started the run"), "{reason}");
        }
        let transient = actor_refusal(RunAsVerdict::Transient("Invalid role permission bits"));
        assert!(transient.unwrap().ends_with("Invalid role permission bits"));
    }

    fn case_context(
        run_user_id: Option<&str>,
        technical_user_id: Option<&str>,
    ) -> CaseDispatchContext {
        CaseDispatchContext {
            app_id: "app".to_string(),
            suite_run_id: "suite-run".to_string(),
            event_id: Some("event".to_string()),
            dispatch_version: Some((1, 2, 3)),
            subject: run_user_id.unwrap_or("regression:suite").to_string(),
            run_user_id: run_user_id.map(str::to_string),
            technical_user_id: technical_user_id.map(str::to_string),
            user_context: None,
            credentials_json: String::new(),
            callback_url: "http://localhost:8080".to_string(),
            profile: None,
            wasm_packages: None,
            board_id: "board".to_string(),
            grading_blind: false,
            run_as_sink: None,
            unattended: false,
        }
    }

    fn schedule_sink_row(active: bool, run_as: Option<&str>) -> event_sink::Model {
        let now = now_naive();
        event_sink::Model {
            id: "sink".into(),
            event_id: "event".into(),
            app_id: "app".into(),
            sink_type: "cron".into(),
            active,
            path: None,
            auth_token: None,
            webhook_secret: None,
            cron_expression: None,
            cron_timezone: None,
            pat_encrypted: None,
            oauth_tokens_encrypted: None,
            profile_json: None,
            created_at: now,
            updated_at: now,
            method: None,
            run_as_user_id: run_as.map(str::to_owned),
            run_as_released_at: None,
        }
    }

    fn schedule_identity(
        run_user_id: Option<&str>,
        sink: Option<event_sink::Model>,
    ) -> ReplayIdentity {
        ReplayIdentity {
            subject: run_user_id.unwrap_or("regression:suite").to_string(),
            run_user_id: run_user_id.map(str::to_string),
            technical_user_id: None,
            user_context: None,
            sink,
            unattended: true,
        }
    }

    /// Only a sink that is on is rebound by a board edit or a publication, so a run reads its
    /// sink again before each case only while it borrows the run-as user of such a sink.
    #[test]
    fn a_borrowed_run_as_is_checked_again_only_while_its_sink_is_on() {
        let owner = Some("owner");
        let cases = [
            (
                "the run-as user of an active sink",
                schedule_identity(owner, Some(schedule_sink_row(true, owner))),
                true,
            ),
            (
                "the run-as user of a paused sink's pinned target",
                schedule_identity(owner, Some(schedule_sink_row(false, owner))),
                false,
            ),
            (
                "the placeholder beside an active sink",
                schedule_identity(None, Some(schedule_sink_row(true, owner))),
                false,
            ),
            (
                "the placeholder beside a sink without a user",
                schedule_identity(None, Some(schedule_sink_row(true, None))),
                false,
            ),
            (
                "the person who started the run",
                schedule_identity(owner, None),
                false,
            ),
        ];
        for (case, identity, checked) in cases {
            assert_eq!(lent_sink(&identity).is_some(), checked, "{case}");
        }
    }

    /// A case dispatches only while the sink whose run-as user the run borrowed still runs as
    /// that user. A changed sink skips the case with the retryable refusal, which errors the
    /// suite run, as a sink fire is refused.
    #[test]
    fn each_case_checks_the_borrowed_run_as_before_it_dispatches() {
        let source = include_str!("mod.rs");
        let body = |signature: &str| {
            let body = source
                .split_once(signature)
                .unwrap_or_else(|| panic!("{signature} exists"))
                .1;
            body.split_once("\n}\n").map_or(body, |(body, _)| body)
        };
        let in_order = |name: &str, steps: &[&str]| {
            let function = body(&format!("\nasync fn {name}("));
            let mut from = 0;
            for step in steps {
                let found = function[from..]
                    .find(step)
                    .unwrap_or_else(|| panic!("{name} calls {step} after the steps before it"));
                from += found + step.len();
            }
        };
        in_order(
            "recheck_lent_sink",
            &[
                "let Some(sink) = &context.run_as_sink else",
                "let Err(overtaken) = recheck_fire(state, sink).await else",
                "OUTCOME_SKIPPED,",
                "Err(overtaken)",
            ],
        );
        in_order(
            "run_case",
            &[
                "recheck_lent_sink(state, context, case_kind, &case_ref).await?;",
                "dispatch_case(state, context, &node_id, payload)",
            ],
        );
        let execute = body("\nasync fn execute_suite_run(");
        assert!(execute.contains("let run_as_sink = lent_sink(&identity);"));
        assert!(execute.contains(".collect::<Vec<Result<(), ApiError>>>()"));
    }

    #[test]
    fn replays_name_only_accounts_and_act_with_the_key_that_started_them() {
        for (run_user_id, technical_user_id, account) in [
            (Some("alice"), None, Some("alice")),
            (Some("alice"), Some("key-1"), Some("alice")),
            (Some("frontend:event:run-run-1"), None, None),
            (Some("regression:suite"), None, None),
            (None, None, None),
        ] {
            let context = case_context(run_user_id, technical_user_id);
            let row = replay_run_row(&context, "run-1", "node", 0, now_naive());
            assert_eq!(
                row.user_id,
                Set(account.map(str::to_string)),
                "{run_user_id:?}"
            );
            assert_eq!(
                row.technical_user_id,
                Set(technical_user_id.map(str::to_string))
            );
            assert_eq!(row.regression_run_id, Set(Some("suite-run".to_string())));
            let token = replay_jwt_params(&context, "run-1");
            assert_eq!(token.user_id, context.subject);
            assert_eq!(token.technical_user_id.as_deref(), technical_user_id);
            assert_eq!(token.shadow, Some(true));
            assert!(token.app_chain.is_none());
        }
    }

    /// A scheduled replay, which no person started, is signed unattended; a replay a person
    /// started is not. Both stay shadow runs.
    #[test]
    fn only_scheduled_replays_are_signed_unattended() {
        crate::backend_jwt::init_for_tests();
        for (case, unattended) in [("scheduled", true), ("started by a person", false)] {
            let context = CaseDispatchContext {
                unattended,
                ..case_context(Some("alice"), None)
            };
            let token = sign_replay_jwt(&context, "run-1").unwrap();
            let claims = crate::execution::verify_execution_jwt(&token).unwrap();
            assert_eq!(claims.unattended, unattended, "{case}");
            assert_eq!(claims.shadow, Some(true), "{case}");
            assert_eq!(claims.sub, "alice", "{case}");
        }
        let source = include_str!("mod.rs");
        let body = |signature: &str| {
            let body = source
                .split_once(signature)
                .unwrap_or_else(|| panic!("{signature} exists"))
                .1;
            body.split_once("\n}\n").map_or(body, |(body, _)| body)
        };
        let actor = body("\nasync fn replay_identity(");
        assert!(actor.contains("unattended: false,"));
        let schedule = body("\nasync fn resolve_schedule_identity(");
        assert!(schedule.contains("unattended: true,"));
        let dispatch = body("\nasync fn dispatch_case(");
        assert!(dispatch.contains("sign_replay_jwt(context, &run_id)"));
    }

    const CATALOG_REFUSAL: &str =
        "SHADOW_SIDE_EFFECT_REFUSED: Shadow and regression runs cannot change the app cache";
    const NODE_FAILURE: &str = "Failed to execute node: SHADOW_SIDE_EFFECT_REFUSED: Shadow and regression runs cannot change the app cache";
    const API_ANSWER: &str = r#"Cache write failed with status 403 Forbidden: {"error":{"code":"SHADOW_SIDE_EFFECT_REFUSED","message":"Shadow and regression runs cannot change the app cache"}}"#;
    const OAUTH_ANSWER: &str = r#"Token refresh failed with status 403 Forbidden: {"error":"SHADOW_SIDE_EFFECT_REFUSED","error_description":"Shadow and regression runs cannot refresh OAuth tokens"}"#;
    const EXECUTOR_CRASH: &str =
        "the run exceeded the executor's memory limit (Runtime.OutOfMemory)";

    fn evidence(
        error_logs: &[&str],
        execution_error: Option<&str>,
        assert_logs: &[&str],
    ) -> RunGradeEvidence {
        RunGradeEvidence {
            metadata_present: true,
            assert_logs: assert_logs.iter().map(|log| log.to_string()).collect(),
            error_logs: error_logs.iter().map(|log| log.to_string()).collect(),
            execution_error: execution_error.map(str::to_string),
            log_query_failed: false,
        }
    }

    #[test]
    fn only_refused_side_effects_grade_as_skipped() {
        let cache = Some("side effect refused: change the app cache".to_string());
        for (error_logs, execution_error, assert_logs, expected) in [
            (
                vec![CATALOG_REFUSAL, NODE_FAILURE],
                Some(REPLAY_FAILED),
                vec!["ASSERT_OK sum"],
                cache.clone(),
            ),
            (vec![API_ANSWER], None, vec![], cache.clone()),
            (vec![NODE_FAILURE], Some(CATALOG_REFUSAL), vec![], cache),
            (
                vec![OAUTH_ANSWER],
                Some(REPLAY_FAILED),
                vec![],
                Some("side effect refused: refresh OAuth tokens".to_string()),
            ),
            (
                vec![CATALOG_REFUSAL, "Failed to execute node: division by zero"],
                Some(REPLAY_FAILED),
                vec![],
                None,
            ),
            (vec![CATALOG_REFUSAL], None, vec!["ASSERT_FAIL total"], None),
            (
                vec![CATALOG_REFUSAL],
                Some("replay ended with status Timeout"),
                vec![],
                None,
            ),
            (
                vec![CATALOG_REFUSAL],
                Some("dispatch failed: refused"),
                vec![],
                None,
            ),
            (vec![], Some(REPLAY_FAILED), vec![], None),
            (vec![], None, vec!["ASSERT_OK sum"], None),
        ] {
            assert_eq!(
                refused_side_effect_only(&evidence(&error_logs, execution_error, &assert_logs)),
                expected,
                "{error_logs:?} {execution_error:?} {assert_logs:?}"
            );
        }
    }

    #[test]
    fn ungradable_replays_are_never_skipped() {
        let mut unreadable = evidence(&[CATALOG_REFUSAL], None, &[]);
        unreadable.log_query_failed = true;
        assert_eq!(refused_side_effect_only(&unreadable), None);
        let mut without_metadata = evidence(&[CATALOG_REFUSAL], None, &[]);
        without_metadata.metadata_present = false;
        assert_eq!(refused_side_effect_only(&without_metadata), None);
    }

    #[test]
    fn refused_effects_are_named_from_every_answer_shape() {
        for (message, effect) in [
            (CATALOG_REFUSAL, Some("change the app cache")),
            (NODE_FAILURE, Some("change the app cache")),
            (API_ANSWER, Some("change the app cache")),
            (OAUTH_ANSWER, Some("refresh OAuth tokens")),
            (
                "SHADOW_SIDE_EFFECT_REFUSED: send notifications.",
                Some("send notifications"),
            ),
            (
                r#"{"error":"SHADOW_SIDE_EFFECT_REFUSED"}"#,
                Some("unnamed side effect"),
            ),
            ("Failed to execute node: division by zero", None),
        ] {
            assert_eq!(refused_effect(message).as_deref(), effect, "{message}");
        }
    }

    #[tokio::test]
    async fn a_replay_blocked_by_the_read_only_store_is_a_refused_side_effect() {
        use flow_like_storage::{
            Path, files::store::FlowLikeStore, object_store::memory::InMemory,
        };
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new())).read_only();
        let blocked = store
            .put(
                &Path::from("apps/app-1/storage/report.csv"),
                b"total".to_vec(),
            )
            .await
            .unwrap_err();
        let node_failure = format!("Failed to execute node: {blocked:?}");
        assert_eq!(
            refused_effect(&node_failure).as_deref(),
            Some("write app storage")
        );
        assert_eq!(
            refused_side_effect_only(&evidence(&[&node_failure], Some(REPLAY_FAILED), &[])),
            Some("side effect refused: write app storage".to_string())
        );
    }

    #[test]
    fn a_skipped_refusal_records_why_and_keeps_its_grade() {
        let refused = evidence(&[CATALOG_REFUSAL, NODE_FAILURE], Some(REPLAY_FAILED), &[]);
        let reason = refused_side_effect_only(&refused);
        let baseline = FixtureBaseline::pass_expectation(0);
        let grade = grade_run(refused);
        let outcome = compare_to_expectation(&baseline, &grade);
        let detail = case_detail(&grade, &baseline, &outcome, false, None, reason);
        assert_eq!(
            detail["reason"],
            "side effect refused: change the app cache"
        );
        assert_eq!(detail["execution_error"], REPLAY_FAILED);
        assert_eq!(detail["baseline_verdict"], "pass");
        assert!(detail.get("grading_blind").is_none());
    }

    #[test]
    fn a_failed_replay_grades_on_its_own_reason() {
        assert_eq!(
            status_error(&RunStatus::Failed, None).as_deref(),
            Some(REPLAY_FAILED)
        );
        assert_eq!(
            status_error(&RunStatus::Failed, Some(EXECUTOR_CRASH)).as_deref(),
            Some(EXECUTOR_CRASH)
        );
        assert_eq!(status_error(&RunStatus::Completed, None), None);
        assert_eq!(
            status_error(&RunStatus::Timeout, None).as_deref(),
            Some("replay ended with status Timeout")
        );
        let refused_logs = [CATALOG_REFUSAL, NODE_FAILURE];
        let failed = status_error(&RunStatus::Failed, None);
        assert!(
            refused_side_effect_only(&evidence(&refused_logs, failed.as_deref(), &[])).is_some()
        );
        let crashed = status_error(&RunStatus::Failed, Some(EXECUTOR_CRASH));
        assert_eq!(
            refused_side_effect_only(&evidence(&refused_logs, crashed.as_deref(), &[])),
            None
        );
    }

    const CANDIDATE: (u32, u32, u32) = (2, 0, 0);
    const OLDER: Option<(u32, u32, u32)> = Some((1, 0, 0));

    fn event(board_id: &str, board_version: Option<(u32, u32, u32)>) -> CoreEvent {
        CoreEvent {
            id: "event".to_string(),
            name: "Nightly".to_string(),
            description: String::new(),
            board_id: board_id.to_string(),
            board_version,
            node_id: "node".to_string(),
            variables: HashMap::new(),
            config: Vec::new(),
            active: true,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: "cron".to_string(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
            default_page_id: None,
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode: Default::default(),
            exposure: Default::default(),
            correlation_mappings: None,
        }
    }

    /// An event pinned to an older version of "board" that runs one variant.
    fn variant_event(
        board_id: &str,
        board_version: Option<(u32, u32, u32)>,
        mode: EventVariantMode,
    ) -> CoreEvent {
        let mut event = event("board", OLDER);
        event.variants.push(EventVariant {
            name: "canary".to_string(),
            board_id: board_id.to_string(),
            board_version,
            node_id: "node".to_string(),
            variables: HashMap::new(),
            default_page_id: None,
            mode,
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        });
        event
    }

    fn runs_candidate(event: &CoreEvent, sink_active: bool) -> bool {
        sink_runs_candidate(event, sink_active, "board", CANDIDATE)
    }

    #[test]
    fn a_scheduled_replay_acts_as_the_sink_where_its_target_runs_the_candidate() {
        for (index, (case, sink_active, expected)) in [
            (event("board", None), true, true),
            (event("board", None), false, false),
            (event("board", Some(CANDIDATE)), true, true),
            (event("board", Some(CANDIDATE)), false, true),
            (event("board", OLDER), true, false),
            (event("other", None), true, false),
            (event("other", Some(CANDIDATE)), true, false),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(runs_candidate(&case, sink_active), expected, "case {index}");
        }
    }

    #[test]
    fn a_scheduled_replay_acts_as_the_sink_where_a_variant_runs_the_candidate() {
        let live = || EventVariantMode::Live { weight: 0.1 };
        let shadow = EventVariantMode::Shadow { sample_rate: 0.1 };
        let mut legacy_canary = event("board", OLDER);
        legacy_canary.canary = Some(CanaryEvent {
            weight: 0.1,
            variables: HashMap::new(),
            board_id: "board".to_string(),
            board_version: None,
            node_id: "node".to_string(),
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        });
        for (index, (case, sink_active, expected)) in [
            (variant_event("board", None, live()), true, true),
            (variant_event("board", None, live()), false, false),
            (variant_event("board", Some(CANDIDATE), shadow), false, true),
            (variant_event("board", OLDER, live()), true, false),
            (variant_event("other", Some(CANDIDATE), live()), true, false),
            (legacy_canary, true, true),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(runs_candidate(&case, sink_active), expected, "case {index}");
        }
    }
}
