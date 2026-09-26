//! Persisted board edit jobs, approval, and delivery leases.

use super::board_commits::{
    ApplyFlowIrCommitResult, FlowIrCommitDisposition, RecoveredBoardEditBatch,
    flow_ir_durable_receipt_ref_key, flowpilot_apply_flow_ir_commit_with_recovery,
    flowpilot_flow_ir_commit_disposition, typed_commit_destructive_review_items,
};
use super::stream_events::utf8_prefix;
use crate::{
    functions::ai::copilot_sdk_tools::{
        flow_ir_draft_snapshot_dir, persist_recovery_snapshot, retained_flow_ir_draft_store,
    },
    state::TauriFlowLikeState,
};
use flow_like::{
    app::{App, AppVisibility},
    copilot::FlowIrCommitToken,
    flow::copilot::BoardCommand,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex as StdMutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};

/// Provider-neutral lifecycle for an exact compiled board edit. The model transport stops at
/// `AwaitingApproval`; only the native host can advance the retained token through Apply/Deny.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoardEditJobPhase {
    Preparing,
    AwaitingApproval,
    Applying,
    /// Native board mutation succeeded, but the renderer has not yet acknowledged replaying the
    /// exact receipt into its sync/history layer. This remains durable and listable across reload.
    AppliedPendingDelivery,
    Applied,
    Denied,
    Stale,
    Failed,
    Cancelled,
}

impl BoardEditJobPhase {
    pub(super) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Applied | Self::Denied | Self::Stale | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardEditJobReview {
    pub command_count: usize,
    pub command_counts: BTreeMap<String, usize>,
    pub command_summaries: Vec<String>,
    pub replacement_mode: bool,
    pub destructive_effects: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardEditJob {
    pub schema_version: String,
    pub job_id: String,
    pub app_id: String,
    pub board_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_principal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_hub: Option<String>,
    pub phase: BoardEditJobPhase,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub expires_at_ms: u64,
    pub token: FlowIrCommitToken,
    pub approval: flow_like::flow::copilot::tool_spec::ResolvedToolApproval,
    pub review: BoardEditJobReview,
    /// Compact post-apply evidence survives delivery without retaining command vectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persisted_board_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ApplyFlowIrCommitResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BoardEditJob {
    pub(super) fn clear_apply_result(&mut self) {
        self.result = None;
        self.persisted_board_fingerprint = None;
    }

    pub(super) fn record_apply_result(&mut self, mut result: ApplyFlowIrCommitResult) {
        if self.phase == BoardEditJobPhase::AppliedPendingDelivery && result.status == "applied" {
            // The board owns the command receipt. Keep only enough evidence to verify an already
            // delivered job after another presenter wins the delivery lease or the app restarts.
            self.persisted_board_fingerprint = result.persisted_board_fingerprint;
            self.result = None;
        } else {
            self.persisted_board_fingerprint = None;
            result.persisted_board_fingerprint = None;
            result.commands.clear();
            result.board_commands.clear();
            self.result = Some(result);
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardEditJobResolution {
    pub job: BoardEditJob,
    /// False for an idempotent replay of a previously settled job. Callers use this to avoid
    /// repeating host-side synchronization work.
    pub transitioned: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardEditJobDeliveryClaim {
    pub job: BoardEditJob,
    pub claimed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_lease_id: Option<String>,
}

#[derive(Clone)]
pub(super) struct BoardEditJobDeliveryLease {
    pub(super) lease_id: String,
    pub(super) expires_at: Instant,
}

#[derive(Clone)]
pub(super) struct BoardEditJobRecord {
    pub(super) job: BoardEditJob,
    /// The exact host-compiled batch reviewed by the user. Unlike the retained FlowScript draft
    /// store, this is persisted with the job so an approved review remains applicable after a
    /// desktop restart.
    pub(super) board_commands: Vec<BoardCommand>,
    /// The compiler's replacement policy is part of the reviewed artifact, not something that may
    /// be inferred from a later/re-hydrated draft.
    pub(super) replacement_mode: bool,
    pub(super) touched_at: Instant,
    /// Serialize Apply/Deny for this exact retained claim without holding the global registry
    /// mutex across board IO or a native confirmation dialog. Duplicate presenters wait here and
    /// then observe the first resolver's terminal result.
    pub(super) resolution_lock: Arc<tokio::sync::Mutex<()>>,
    pub(super) delivery_lease: Option<BoardEditJobDeliveryLease>,
}

pub(super) const BOARD_EDIT_JOB_SCHEMA_VERSION: &str = "flowpilot.board-edit-job/v1";

pub(super) const BOARD_EDIT_JOB_TTL: Duration = Duration::from_secs(2 * 60 * 60);

const BOARD_EDIT_DELIVERY_DISPLAY_TTL_MS: u64 = 10 * 365 * 24 * 60 * 60 * 1_000;

pub(super) const BOARD_EDIT_JOB_MAX_ENTRIES: usize = 256;

const BOARD_EDIT_JOB_DELIVERY_LEASE: Duration = Duration::from_secs(60);

const BOARD_EDIT_JOB_SNAPSHOT_FILE: &str = "board-edit-jobs-v1.json";

/// Keep one pathological compiler artifact from monopolizing the recovery registry. The lower
/// executed-command transport cap is checked after apply, because undo metadata can change size.
const BOARD_EDIT_JOB_MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;

/// Hard cap for the complete persisted registry, including applied delivery receipts. Entry count
/// alone is insufficient because command payloads vary by orders of magnitude.
const BOARD_EDIT_JOB_MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

/// Mirrors the renderer's atomic command envelope. A compiler command such as RemoveNode can
/// expand to an undo command containing an entire large node, so this is checked on the actual
/// executed GenericCommand before persistence. The shared validator also checks JSON-string
/// escaping for the Lambda request event and echoed command response.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const BOARD_EDIT_JOB_MAX_REMOTE_COMMAND_BYTES: usize =
    crate::functions::flow::board::REMOTE_BOARD_COMMAND_BATCH_MAX_BYTES;

/// Bound the full durable replay payload independently from compact BoardCommand input size.
pub(super) const BOARD_EDIT_JOB_MAX_APPLY_RECEIPT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
struct BoardEditJobSnapshot {
    schema_version: String,
    jobs: Vec<PersistedBoardEditJobEntry>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct PersistedBoardEditJobRecord {
    pub(super) job: BoardEditJob,
    #[serde(default)]
    pub(super) board_commands: Vec<BoardCommand>,
    #[serde(default)]
    pub(super) replacement_mode: bool,
}

/// Accept the original v1 snapshot shape during migration. Such a record has no recoverable exact
/// batch and is therefore made stale below instead of being unsafely reconstructed.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub(super) enum PersistedBoardEditJobEntry {
    Current(PersistedBoardEditJobRecord),
    Legacy(BoardEditJob),
}

fn board_edit_job_snapshot_path() -> Option<PathBuf> {
    Some(flow_ir_draft_snapshot_dir()?.join(BOARD_EDIT_JOB_SNAPSHOT_FILE))
}

pub(super) fn board_edit_job_record_from_persisted(
    persisted: PersistedBoardEditJobEntry,
) -> Option<BoardEditJobRecord> {
    let (mut job, board_commands, replacement_mode) = match persisted {
        PersistedBoardEditJobEntry::Current(record) => {
            (record.job, record.board_commands, record.replacement_mode)
        }
        PersistedBoardEditJobEntry::Legacy(job) => (job, Vec::new(), false),
    };
    if job.schema_version != BOARD_EDIT_JOB_SCHEMA_VERSION
        || job.job_id.trim().is_empty()
        || job.app_id.trim().is_empty()
        || job.board_id.trim().is_empty()
    {
        return None;
    }
    let now = Instant::now();
    let now_ms = wall_clock_ms();
    if matches!(
        job.phase,
        BoardEditJobPhase::Preparing | BoardEditJobPhase::Applying
    ) {
        job.phase = BoardEditJobPhase::Failed;
        job.updated_at_ms = now_ms;
        job.error = Some(
            "The desktop process restarted during the native apply transition. Retry this exact retained review; a durable board receipt prevents duplicate mutation if persistence already completed."
                .to_string(),
        );
    }
    if board_commands.is_empty()
        && matches!(
            job.phase,
            BoardEditJobPhase::Preparing
                | BoardEditJobPhase::AwaitingApproval
                | BoardEditJobPhase::Applying
                | BoardEditJobPhase::Failed
        )
    {
        job.phase = BoardEditJobPhase::Stale;
        job.updated_at_ms = now_ms;
        job.error = Some(
            "This review predates crash-durable exact batches and cannot be applied safely after restart. Regenerate it against the current board."
                .to_string(),
        );
    }
    if !board_commands.is_empty() {
        // Display metadata is never an authority boundary. Recompute it from the exact persisted
        // host batch so a partial/corrupt snapshot cannot pair benign review text with a different
        // apply artifact.
        job.review = board_command_review(&board_commands, replacement_mode);
    }
    if job.phase == BoardEditJobPhase::AppliedPendingDelivery {
        job.expires_at_ms = now_ms.saturating_add(BOARD_EDIT_DELIVERY_DISPLAY_TTL_MS);
    }
    if !matches!(
        job.phase,
        BoardEditJobPhase::AppliedPendingDelivery | BoardEditJobPhase::Applied
    ) {
        job.persisted_board_fingerprint = None;
    }
    let age_ms = now_ms.saturating_sub(job.updated_at_ms);
    let touched_at = now
        .checked_sub(Duration::from_millis(age_ms))
        .unwrap_or(now);
    Some(BoardEditJobRecord {
        job,
        board_commands,
        replacement_mode,
        touched_at,
        resolution_lock: Arc::new(tokio::sync::Mutex::new(())),
        delivery_lease: None,
    })
}

fn load_board_edit_jobs() -> HashMap<String, BoardEditJobRecord> {
    let Some(path) = board_edit_job_snapshot_path() else {
        return HashMap::new();
    };
    if std::fs::metadata(&path)
        .is_ok_and(|metadata| metadata.len() > BOARD_EDIT_JOB_MAX_SNAPSHOT_BYTES as u64)
    {
        tracing::warn!(
            "ignoring oversized FlowPilot board-edit recovery snapshot at {}",
            path.display()
        );
        return HashMap::new();
    }
    let Ok(encoded) = std::fs::read(path) else {
        return HashMap::new();
    };
    let Ok(snapshot) = serde_json::from_slice::<BoardEditJobSnapshot>(&encoded) else {
        return HashMap::new();
    };
    if snapshot.schema_version != BOARD_EDIT_JOB_SCHEMA_VERSION {
        return HashMap::new();
    }
    let jobs = snapshot
        .jobs
        .into_iter()
        .filter_map(|persisted| {
            let id = match &persisted {
                PersistedBoardEditJobEntry::Current(record) => record.job.job_id.clone(),
                PersistedBoardEditJobEntry::Legacy(job) => job.job_id.clone(),
            };
            board_edit_job_record_from_persisted(persisted).map(|record| (id, record))
        })
        .collect::<HashMap<_, _>>();
    // Hydration may convert an interrupted Applying transition to retryable Failed, migrate a
    // legacy entry to fail-closed Stale, or recompute review metadata. Persist that normalized
    // state immediately so repeated restarts cannot resurrect the pre-normalized lifecycle.
    if let Err(error) = persist_board_edit_jobs(&jobs) {
        tracing::error!("could not persist normalized FlowPilot review jobs: {error}");
    }
    jobs
}

fn persist_board_edit_jobs(jobs: &HashMap<String, BoardEditJobRecord>) -> Result<(), String> {
    let Some(path) = board_edit_job_snapshot_path() else {
        return Err("The FlowPilot review snapshot directory is unavailable.".to_string());
    };
    let Some(parent) = path.parent() else {
        return Err("The FlowPilot review snapshot path is invalid.".to_string());
    };
    std::fs::create_dir_all(parent).map_err(|error| {
        format!("Could not create the FlowPilot review snapshot directory: {error}")
    })?;
    let mut retained = jobs
        .values()
        .map(|record| {
            let mut job = record.job.clone();
            if job.phase == BoardEditJobPhase::AppliedPendingDelivery {
                // The board-embedded receipt is the authoritative crash-recovery payload. Keeping
                // another potentially huge GenericCommand vector here made registry persistence
                // fail after the board had already committed.
                job.result = None;
            }
            PersistedBoardEditJobEntry::Current(PersistedBoardEditJobRecord {
                job,
                board_commands: record.board_commands.clone(),
                replacement_mode: record.replacement_mode,
            })
        })
        .collect::<Vec<_>>();
    retained.sort_by_key(|entry| match entry {
        PersistedBoardEditJobEntry::Current(record) => {
            (record.job.created_at_ms, record.job.job_id.clone())
        }
        PersistedBoardEditJobEntry::Legacy(job) => (job.created_at_ms, job.job_id.clone()),
    });
    let snapshot = BoardEditJobSnapshot {
        schema_version: BOARD_EDIT_JOB_SCHEMA_VERSION.to_string(),
        jobs: retained,
    };
    let encoded = serde_json::to_vec(&snapshot)
        .map_err(|error| format!("Could not serialize FlowPilot review recovery state: {error}"))?;
    if encoded.len() > BOARD_EDIT_JOB_MAX_SNAPSHOT_BYTES {
        return Err(format!(
            "FlowPilot review recovery storage is full ({} MiB limit). Resolve delivered reviews before creating more large board edits.",
            BOARD_EDIT_JOB_MAX_SNAPSHOT_BYTES / (1024 * 1024)
        ));
    }
    persist_recovery_snapshot(&path, &encoded)
        .map_err(|error| format!("Could not commit FlowPilot review recovery state: {error}"))?;
    Ok(())
}

pub(super) static BOARD_EDIT_JOBS: LazyLock<StdMutex<HashMap<String, BoardEditJobRecord>>> =
    LazyLock::new(|| StdMutex::new(load_board_edit_jobs()));

/// Reject an ordinary native board mutation while FlowPilot owns the same board's apply/delivery
/// window. Callers must invoke this only after acquiring the live board mutex. That lock order
/// makes the check race-free with job resolution: either the ordinary edit already owns the board
/// and completes first, or resolution publishes `Applying` before it waits and this check fails.
pub(crate) fn ensure_board_mutation_not_reserved_by_flowpilot(
    app_id: &str,
    board_id: &str,
) -> Result<(), String> {
    let app_id = app_id.trim();
    let board_id = board_id.trim();
    let mut jobs = BOARD_EDIT_JOBS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if prune_board_edit_jobs(&mut jobs, Instant::now()) {
        persist_board_edit_jobs(&jobs)?;
    }
    let reserved = board_mutation_is_reserved(&jobs, app_id, board_id);
    if reserved {
        Err(
            "FlowPilot has an applying, failed-recovery, or pending-delivery edit for this board. Retry/dismiss it or finish receipt delivery before making another board mutation."
                .to_string(),
        )
    } else {
        Ok(())
    }
}

pub(super) fn board_mutation_is_reserved(
    jobs: &HashMap<String, BoardEditJobRecord>,
    app_id: &str,
    board_id: &str,
) -> bool {
    jobs.values().any(|record| {
        record.job.app_id == app_id
            && record.job.board_id == board_id
            && board_edit_job_phase_reserves_mutation(record.job.phase)
    })
}

pub(super) fn another_board_edit_job_reserves_mutation(
    jobs: &HashMap<String, BoardEditJobRecord>,
    current_job_id: &str,
    app_id: &str,
    board_id: &str,
) -> bool {
    jobs.iter().any(|(job_id, record)| {
        job_id != current_job_id
            && record.job.app_id == app_id
            && record.job.board_id == board_id
            && board_edit_job_phase_reserves_mutation(record.job.phase)
    })
}

fn board_edit_job_phase_reserves_mutation(phase: BoardEditJobPhase) -> bool {
    matches!(
        phase,
        BoardEditJobPhase::Applying
            | BoardEditJobPhase::AppliedPendingDelivery
            | BoardEditJobPhase::Failed
    )
}

pub(super) fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(super) fn flow_ir_commit_identity(token: &FlowIrCommitToken) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        token.board_id, token.draft_id, token.revision, token.base_fingerprint, token.claim_id
    )
}

pub(super) fn board_command_review(
    commands: &[BoardCommand],
    replacement_mode: bool,
) -> BoardEditJobReview {
    let mut command_counts = BTreeMap::<String, usize>::new();
    let mut command_summaries = Vec::new();
    for command in commands {
        let serialized = serde_json::to_value(command).unwrap_or_default();
        let kind = serialized
            .get("command_type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Unknown")
            .to_string();
        *command_counts.entry(kind).or_default() += 1;
        if command_summaries.len() < 24
            && let Some(summary) = serialized
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|summary| !summary.is_empty())
        {
            command_summaries.push(utf8_prefix(summary, 240).to_string());
        }
    }
    BoardEditJobReview {
        command_count: commands.len(),
        command_counts,
        command_summaries,
        replacement_mode,
        destructive_effects: typed_commit_destructive_review_items(replacement_mode, commands),
    }
}

fn release_board_edit_job_claim(record: &BoardEditJobRecord) {
    let token = &record.job.token;
    if let Some(store) = retained_flow_ir_draft_store(&token.board_id) {
        let _ = store.release_commit_if_matches(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        );
    }
}

pub(super) fn prune_board_edit_jobs(
    jobs: &mut HashMap<String, BoardEditJobRecord>,
    now: Instant,
) -> bool {
    let now_ms = wall_clock_ms();
    let mut persisted_state_changed = false;
    for record in jobs.values_mut() {
        if record
            .delivery_lease
            .as_ref()
            .is_some_and(|lease| lease.expires_at <= now)
        {
            record.delivery_lease = None;
        }
        if !record.job.phase.is_terminal()
            && !matches!(
                record.job.phase,
                BoardEditJobPhase::Applying | BoardEditJobPhase::AppliedPendingDelivery
            )
            && now.saturating_duration_since(record.touched_at) > BOARD_EDIT_JOB_TTL
        {
            // An expired review is no longer presentable. Release only its exact claim so it
            // cannot become a hidden, permanently queued compiler artifact after a renderer
            // reload. A concurrently changed claim is left untouched by the store-side CAS.
            release_board_edit_job_claim(record);
            record.job.phase = BoardEditJobPhase::Stale;
            record.job.updated_at_ms = now_ms;
            record.job.error = Some(
                "The compiled board-edit review expired before it was resolved; regenerate it against the current board."
                    .to_string(),
            );
            record.board_commands.clear();
            persisted_state_changed = true;
        }
    }
    while jobs.len() > BOARD_EDIT_JOB_MAX_ENTRIES {
        let removable = jobs
            .iter()
            .filter(|(_, record)| record.job.phase.is_terminal())
            .min_by_key(|(_, record)| record.touched_at)
            .map(|(id, _)| id.clone());
        let Some(removable) = removable else {
            break;
        };
        if let Some(record) = jobs.remove(&removable) {
            // Only terminal jobs are capacity-evicted. Unresolved and post-apply delivery jobs
            // are never sacrificed to admit a newer review.
            release_board_edit_job_claim(&record);
            persisted_state_changed = true;
        }
    }
    persisted_state_changed
}

/// Convert an exact retained compiler claim into a host-owned review job. This function knows
/// nothing about the model provider; Bits, GitHub Copilot, Codex and Claude all converge here.
#[tauri::command]
pub async fn flowpilot_create_board_edit_job(
    app_handle: AppHandle,
    app_id: String,
    request_id: Option<String>,
    token: FlowIrCommitToken,
) -> Result<BoardEditJob, String> {
    let app_id = app_id.trim().to_string();
    if app_id.is_empty()
        || token.board_id.trim().is_empty()
        || token.draft_id.trim().is_empty()
        || token.base_fingerprint.trim().is_empty()
        || token.claim_id.trim().is_empty()
    {
        return Err("The compiled board-edit token or app id is incomplete.".to_string());
    }
    // Expiry may release an exact compiler claim. Perform that cleanup before validating the
    // token, otherwise a pre-prune validation could incorrectly reopen the just-released job.
    {
        let mut jobs = BOARD_EDIT_JOBS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if prune_board_edit_jobs(&mut jobs, Instant::now()) {
            persist_board_edit_jobs(&jobs)?;
        }
    }
    let Some(store) = retained_flow_ir_draft_store(&token.board_id) else {
        return Err("The compiled board-edit artifact is no longer retained.".to_string());
    };
    let Some(state) = app_handle.try_state::<TauriFlowLikeState>() else {
        return Err("The live board registry is unavailable.".to_string());
    };
    let flow_like_state = state.0.clone();
    let app = App::load(app_id.clone(), flow_like_state.clone())
        .await
        .map_err(|error| format!("The review app could not be loaded: {error}"))?;
    if !app.boards.contains(&token.board_id) {
        return Err("The review board does not belong to the requested app.".to_string());
    }
    let live_board = flow_like_state
        .get_board(&token.board_id, None)
        .map_err(|_| "The review board is not open in this desktop process.".to_string())?;
    let board = live_board.write().await;
    let commands = store
        .pending_commands_if_current(
            &board,
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        )
        .ok_or_else(|| {
            "The retained compiler claim is stale relative to the live board.".to_string()
        })?;
    let replacement_mode = store
        .pending_commit_requires_destructive_approval(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        )
        .ok_or_else(|| "The retained compiler review policy is no longer current.".to_string())?;
    let batch_bytes = serde_json::to_vec(&commands)
        .map_err(|error| format!("The compiled board-edit batch could not be retained: {error}"))?
        .len();
    if batch_bytes > BOARD_EDIT_JOB_MAX_BATCH_BYTES {
        let _ = store.release_commit_if_matches(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        );
        return Err(format!(
            "The compiled board-edit artifact is too large for crash-safe review ({} bytes; {} MiB limit). Split the workflow change into smaller reviews.",
            batch_bytes,
            BOARD_EDIT_JOB_MAX_BATCH_BYTES / (1024 * 1024)
        ));
    }
    let review = board_command_review(&commands, replacement_mode);
    let approval_spec =
        flow_like::flow::copilot::tool_spec::find_global_tool_spec("flowpilot_board")
            .ok_or_else(|| "The shared board-edit approval policy is unavailable.".to_string())?;
    let approval = flow_like::flow::copilot::tool_spec::resolve_tool_apply_approval(
        &approval_spec,
        &serde_json::json!({
            "mode": "edit",
            "app_id": app_id,
            "board_id": token.board_id,
        }),
    );
    drop(board);

    let identity = flow_ir_commit_identity(&token);
    let now = Instant::now();
    let now_ms = wall_clock_ms();
    let mut jobs = BOARD_EDIT_JOBS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing_id) = jobs
        .iter()
        .find(|(_, record)| {
            record.job.app_id == app_id && flow_ir_commit_identity(&record.job.token) == identity
        })
        .map(|(id, _)| id.clone())
    {
        let previous = jobs
            .get(&existing_id)
            .cloned()
            .expect("board-edit job existed while registry lock was held");
        let existing = jobs
            .get_mut(&existing_id)
            .expect("board-edit job existed while registry lock was held");
        // A renderer can return long after a TTL sweep, and a retryable native Apply failure keeps
        // the exact compiler claim retained. Successful preflight above proves that these states
        // can safely be offered for review again.
        if existing.job.phase == BoardEditJobPhase::Stale {
            existing.job.phase = BoardEditJobPhase::AwaitingApproval;
            existing.job.clear_apply_result();
            existing.job.error = None;
            existing.job.review = review;
            existing.job.approval = approval;
            existing.delivery_lease = None;
        }
        // Preflight above re-established this exact claim against the live board. Refresh the
        // durable artifact as well as the display-only review metadata.
        existing.board_commands = commands;
        existing.replacement_mode = replacement_mode;
        if existing.job.request_id.is_none() {
            existing.job.request_id = request_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
        }
        existing.job.updated_at_ms = now_ms;
        existing.job.expires_at_ms = now_ms.saturating_add(BOARD_EDIT_JOB_TTL.as_millis() as u64);
        existing.touched_at = now;
        let job = existing.job.clone();
        if let Err(error) = persist_board_edit_jobs(&jobs) {
            jobs.insert(existing_id, previous);
            return Err(error);
        }
        return Ok(job);
    }
    let evicted_terminal = if jobs.len() >= BOARD_EDIT_JOB_MAX_ENTRIES
        && let Some(oldest_terminal) = jobs
            .iter()
            .filter(|(_, record)| record.job.phase.is_terminal())
            .min_by_key(|(_, record)| record.touched_at)
            .map(|(id, _)| id.clone())
    {
        jobs.remove(&oldest_terminal)
            .map(|record| (oldest_terminal, record))
    } else {
        None
    };
    if jobs.len() >= BOARD_EDIT_JOB_MAX_ENTRIES {
        return Err(
            "FlowPilot has too many unresolved board-edit reviews. Resolve or dismiss an existing review before creating another."
                .to_string(),
        );
    }
    let job_id = uuid::Uuid::new_v4().to_string();
    let job = BoardEditJob {
        schema_version: BOARD_EDIT_JOB_SCHEMA_VERSION.to_string(),
        job_id: job_id.clone(),
        app_id,
        board_id: token.board_id.clone(),
        request_id: request_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        remote_profile_id: None,
        remote_principal_id: None,
        remote_hub: None,
        phase: BoardEditJobPhase::AwaitingApproval,
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        expires_at_ms: now_ms.saturating_add(BOARD_EDIT_JOB_TTL.as_millis() as u64),
        token,
        approval,
        review,
        persisted_board_fingerprint: None,
        result: None,
        error: None,
    };
    jobs.insert(
        job_id,
        BoardEditJobRecord {
            job: job.clone(),
            board_commands: commands,
            replacement_mode,
            touched_at: now,
            resolution_lock: Arc::new(tokio::sync::Mutex::new(())),
            delivery_lease: None,
        },
    );
    if let Err(error) = persist_board_edit_jobs(&jobs) {
        jobs.remove(&job.job_id);
        if let Some((id, record)) = evicted_terminal {
            jobs.insert(id, record);
        }
        return Err(error);
    }
    if let Some((_, record)) = evicted_terminal {
        release_board_edit_job_claim(&record);
    }
    Ok(job)
}

#[tauri::command]
pub fn flowpilot_list_board_edit_jobs(
    app_id: Option<String>,
    board_id: Option<String>,
    include_terminal: Option<bool>,
) -> Vec<BoardEditJob> {
    let now = Instant::now();
    let mut jobs = BOARD_EDIT_JOBS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if prune_board_edit_jobs(&mut jobs, now)
        && let Err(error) = persist_board_edit_jobs(&jobs)
    {
        tracing::error!("could not persist pruned FlowPilot review jobs: {error}");
    }
    let include_terminal = include_terminal.unwrap_or(false);
    let app_id = app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let board_id = board_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut result = jobs
        .values()
        .filter(|record| {
            (include_terminal || !record.job.phase.is_terminal())
                && app_id.is_none_or(|app_id| record.job.app_id == app_id)
                && board_id.is_none_or(|board_id| record.job.board_id == board_id)
        })
        .map(|record| record.job.clone())
        .collect::<Vec<_>>();
    result.sort_by_key(|job| (job.created_at_ms, job.job_id.clone()));
    result
}

#[tauri::command]
pub fn flowpilot_get_board_edit_job(job_id: String) -> Option<BoardEditJob> {
    let mut jobs = BOARD_EDIT_JOBS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if prune_board_edit_jobs(&mut jobs, Instant::now())
        && let Err(error) = persist_board_edit_jobs(&jobs)
    {
        tracing::error!("could not persist pruned FlowPilot review jobs: {error}");
    }
    jobs.get(job_id.trim()).map(|record| record.job.clone())
}

/// Atomically claim a pending review and resolve it. Duplicate callers observe the same native job
/// instead of applying the batch twice; exact-batch CAS remains enforced by the existing Apply API.
///
/// `destructive_preapproved` reports that the destructive review was already presented to and
/// accepted by the user in the renderer — either through the approval card or through the auto-mode
/// waiver they armed themselves. It suppresses only the redundant second confirmation; every exact
/// claim, base-fingerprint, and batch-equality check below still runs unchanged.
#[tauri::command]
pub async fn flowpilot_resolve_board_edit_job(
    app_handle: AppHandle,
    job_id: String,
    approved: bool,
    destructive_preapproved: Option<bool>,
    remote_profile_id: Option<String>,
    remote_principal_id: Option<String>,
    remote_hub: Option<String>,
) -> Result<BoardEditJobResolution, String> {
    let job_id = job_id.trim().to_string();
    let (resolution_lock, review_app_id, review_board_id) = {
        let mut jobs = BOARD_EDIT_JOBS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if prune_board_edit_jobs(&mut jobs, Instant::now()) {
            persist_board_edit_jobs(&jobs)?;
        }
        jobs.get(&job_id)
            .map(|record| {
                (
                    record.resolution_lock.clone(),
                    record.job.app_id.clone(),
                    record.job.board_id.clone(),
                )
            })
            .ok_or_else(|| "The board-edit review job is no longer retained.".to_string())?
    };
    let _resolution_guard = resolution_lock.lock().await;
    let requires_remote_identity = if approved {
        let Some(state) = app_handle.try_state::<TauriFlowLikeState>() else {
            return Err("The live board registry is unavailable.".to_string());
        };
        let app = App::load(review_app_id.clone(), state.0.clone())
            .await
            .map_err(|error| format!("The review app could not be loaded: {error}"))?;
        if !app.boards.contains(&review_board_id) {
            return Err("The review board does not belong to the requested app.".to_string());
        }
        !matches!(app.visibility, AppVisibility::Offline)
    } else {
        false
    };
    let (app_id, token, recovered_batch) = {
        let mut jobs = BOARD_EDIT_JOBS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if prune_board_edit_jobs(&mut jobs, Instant::now()) {
            persist_board_edit_jobs(&jobs)?;
        }
        let previous = jobs
            .get(&job_id)
            .cloned()
            .ok_or_else(|| "The board-edit review job is no longer retained.".to_string())?;
        if approved
            && another_board_edit_job_reserves_mutation(
                &jobs,
                &job_id,
                &previous.job.app_id,
                &previous.job.board_id,
            )
        {
            return Err(
                "Another FlowPilot edit for this board is applying, awaiting recovery, or pending receipt delivery. Retry/dismiss or deliver that job before approving this review."
                    .to_string(),
            );
        }
        let record = jobs
            .get_mut(&job_id)
            .expect("board-edit job existed while registry lock was held");
        let can_resolve = matches!(
            record.job.phase,
            BoardEditJobPhase::AwaitingApproval | BoardEditJobPhase::Failed
        ) || (approved && record.job.phase == BoardEditJobPhase::Applying)
            || (!approved && record.job.phase == BoardEditJobPhase::Stale);
        if !can_resolve {
            return Ok(BoardEditJobResolution {
                job: record.job.clone(),
                transitioned: false,
            });
        }
        if approved {
            let remote_profile_id = remote_profile_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let remote_principal_id = remote_principal_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let remote_hub = remote_hub
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let incoming_identity = (
                remote_profile_id.as_ref(),
                remote_principal_id.as_ref(),
                remote_hub.as_ref(),
            );
            let incoming_is_unbound = incoming_identity.0.is_none()
                && incoming_identity.1.is_none()
                && incoming_identity.2.is_none();
            let incoming_is_complete = incoming_identity.0.is_some()
                && incoming_identity.1.is_some()
                && incoming_identity.2.is_some();
            if !incoming_is_unbound && !incoming_is_complete {
                return Err(
                    "A remote board-edit review requires a complete profile, account, and Hub identity."
                        .to_string(),
                );
            }

            let existing_identity = (
                record.job.remote_profile_id.as_ref(),
                record.job.remote_principal_id.as_ref(),
                record.job.remote_hub.as_ref(),
            );
            let existing_is_unbound = existing_identity.0.is_none()
                && existing_identity.1.is_none()
                && existing_identity.2.is_none();
            if existing_is_unbound && incoming_is_complete != requires_remote_identity {
                return Err(
                    "The board-edit review identity does not match the app's authoritative local-only/shared visibility. Refresh the app and retry."
                        .to_string(),
                );
            }
            if !existing_is_unbound && existing_identity != incoming_identity {
                return Err(
                    "The board-edit review is already bound to a different remote profile, account, or Hub."
                        .to_string(),
                );
            }
            if existing_is_unbound && incoming_is_complete {
                record.job.remote_profile_id = remote_profile_id;
                record.job.remote_principal_id = remote_principal_id;
                record.job.remote_hub = remote_hub;
            }
        }
        record.job.phase = if approved {
            BoardEditJobPhase::Applying
        } else {
            // Reserve the transition while the exact claim is released below. The per-job lock
            // prevents a duplicate resolver from observing this provisional value.
            BoardEditJobPhase::Denied
        };
        record.job.updated_at_ms = wall_clock_ms();
        record.job.clear_apply_result();
        record.job.error = None;
        record.touched_at = Instant::now();
        let transition = (
            record.job.app_id.clone(),
            record.job.token.clone(),
            RecoveredBoardEditBatch {
                board_commands: record.board_commands.clone(),
                replacement_mode: record.replacement_mode,
            },
        );
        if let Err(error) = persist_board_edit_jobs(&jobs) {
            jobs.insert(job_id.clone(), previous);
            return Err(error);
        }
        transition
    };

    if approved {
        let result = flowpilot_apply_flow_ir_commit_with_recovery(
            app_handle,
            app_id,
            token,
            Some(recovered_batch),
            destructive_preapproved.unwrap_or(false),
        )
        .await;
        let phase = match result.status.as_str() {
            "applied" => BoardEditJobPhase::AppliedPendingDelivery,
            "stale" => BoardEditJobPhase::Stale,
            _ => BoardEditJobPhase::Failed,
        };
        let mut jobs = BOARD_EDIT_JOBS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = jobs
            .get(&job_id)
            .cloned()
            .ok_or_else(|| "The board-edit review job disappeared while applying.".to_string())?;
        let record = jobs
            .get_mut(&job_id)
            .expect("board-edit job existed while registry lock was held");
        record.job.phase = phase;
        record.job.updated_at_ms = wall_clock_ms();
        if phase == BoardEditJobPhase::AppliedPendingDelivery {
            record.job.expires_at_ms = record
                .job
                .updated_at_ms
                .saturating_add(BOARD_EDIT_DELIVERY_DISPLAY_TTL_MS);
        }
        record.job.error = matches!(phase, BoardEditJobPhase::Stale | BoardEditJobPhase::Failed)
            .then(|| result.message.clone());
        record.job.record_apply_result(result);
        record.delivery_lease = None;
        if matches!(
            phase,
            BoardEditJobPhase::AppliedPendingDelivery | BoardEditJobPhase::Stale
        ) {
            // Pending delivery recovers from the board-embedded receipt; stale work is terminal.
            // Do not retain a second large copy of the exact batch.
            record.board_commands.clear();
        }
        record.touched_at = Instant::now();
        let job = record.job.clone();
        if let Err(error) = persist_board_edit_jobs(&jobs) {
            // The board apply may already be durable. Restore `Applying` in memory and let the
            // next resolution replay the board-embedded receipt instead of stranding a half-
            // persisted lifecycle transition.
            jobs.insert(job_id.clone(), previous);
            return Err(error);
        }
        if matches!(phase, BoardEditJobPhase::Stale | BoardEditJobPhase::Failed)
            && let Some(record) = jobs.get(&job_id)
        {
            // A failed apply must not keep holding the board's single pending-commit claim: the
            // store would redeliver the same doomed batch and refuse every repair commit with
            // FLOWSCRIPT_BOARD_COMMIT_PENDING, wedging the whole loop. The job retains its
            // recovered batch, so an explicit retry replays independently of the released claim.
            release_board_edit_job_claim(record);
        }
        return Ok(BoardEditJobResolution {
            job,
            transitioned: true,
        });
    }

    let disposition =
        flowpilot_flow_ir_commit_disposition(app_handle, token, FlowIrCommitDisposition::Dismissed)
            .await;
    let dismissed = disposition.status == "dismissed"
        || disposition.code.as_deref() == Some("IR_COMMIT_TOKEN_INVALID");
    let mut jobs = BOARD_EDIT_JOBS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = jobs
        .get(&job_id)
        .cloned()
        .ok_or_else(|| "The board-edit review job disappeared while denying it.".to_string())?;
    let record = jobs
        .get_mut(&job_id)
        .expect("board-edit job existed while registry lock was held");
    record.job.phase = if dismissed {
        BoardEditJobPhase::Denied
    } else {
        BoardEditJobPhase::Failed
    };
    record.job.updated_at_ms = wall_clock_ms();
    record.job.error = (!dismissed).then_some(disposition.message);
    if dismissed {
        record.board_commands.clear();
    }
    record.touched_at = Instant::now();
    let job = record.job.clone();
    if let Err(error) = persist_board_edit_jobs(&jobs) {
        jobs.insert(job_id, previous);
        if let Some(record) = jobs.get(job.job_id.as_str()) {
            // The durable state is already Denied from the provisional transition. Do not leave
            // its exact compiler claim hidden if persisting the follow-up diagnostic failed.
            release_board_edit_job_claim(record);
        }
        return Err(error);
    }
    Ok(BoardEditJobResolution {
        job,
        transitioned: true,
    })
}

/// Lease delivery of an already-applied native receipt to exactly one renderer. The lease makes
/// concurrent direct/global presenters harmless and expires so a renderer crash can be recovered
/// after reload without reapplying the native board mutation.
#[tauri::command]
pub fn flowpilot_claim_board_edit_job_delivery(
    job_id: String,
) -> Result<BoardEditJobDeliveryClaim, String> {
    let job_id = job_id.trim();
    let now = Instant::now();
    let mut jobs = BOARD_EDIT_JOBS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if prune_board_edit_jobs(&mut jobs, now) {
        persist_board_edit_jobs(&jobs)?;
    }
    let record = jobs
        .get_mut(job_id)
        .ok_or_else(|| "The board-edit delivery job is no longer retained.".to_string())?;
    if record.job.phase != BoardEditJobPhase::AppliedPendingDelivery {
        return Ok(BoardEditJobDeliveryClaim {
            job: record.job.clone(),
            claimed: false,
            delivery_lease_id: None,
        });
    }
    if record.delivery_lease.is_some() {
        return Ok(BoardEditJobDeliveryClaim {
            job: record.job.clone(),
            claimed: false,
            delivery_lease_id: None,
        });
    }

    let lease_id = uuid::Uuid::new_v4().to_string();
    record.delivery_lease = Some(BoardEditJobDeliveryLease {
        lease_id: lease_id.clone(),
        expires_at: now + BOARD_EDIT_JOB_DELIVERY_LEASE,
    });
    record.touched_at = now;
    record.job.updated_at_ms = wall_clock_ms();
    Ok(BoardEditJobDeliveryClaim {
        job: record.job.clone(),
        claimed: true,
        delivery_lease_id: Some(lease_id),
    })
}

/// Acknowledge that the renderer replayed the native receipt through its sync/history layer.
/// Until this exact lease is acknowledged the job remains nonterminal and discoverable.
#[tauri::command]
pub async fn flowpilot_ack_board_edit_job_delivery(
    app_handle: AppHandle,
    job_id: String,
    delivery_lease_id: String,
) -> Result<BoardEditJob, String> {
    let job_id = job_id.trim();
    let delivery_lease_id = delivery_lease_id.trim();
    let now = Instant::now();
    let job = {
        let mut jobs = BOARD_EDIT_JOBS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if prune_board_edit_jobs(&mut jobs, now) {
            persist_board_edit_jobs(&jobs)?;
        }
        let previous = jobs
            .get(job_id)
            .cloned()
            .ok_or_else(|| "The board-edit delivery job is no longer retained.".to_string())?;
        let record = jobs
            .get_mut(job_id)
            .expect("board-edit job existed while registry lock was held");
        if record.job.phase == BoardEditJobPhase::Applied {
            return Ok(record.job.clone());
        }
        if record.job.phase != BoardEditJobPhase::AppliedPendingDelivery {
            return Err(
                "The board-edit job does not have an applied receipt to deliver.".to_string(),
            );
        }
        let lease_matches = record
            .delivery_lease
            .as_ref()
            .is_some_and(|lease| lease.lease_id == delivery_lease_id && lease.expires_at > now);
        if !lease_matches {
            return Err(
                "The board-edit delivery lease expired or belongs to another presenter."
                    .to_string(),
            );
        }
        record.delivery_lease = None;
        record.job.phase = BoardEditJobPhase::Applied;
        // Delivery has completed; neither replay payload nor pre-apply batch belongs in the
        // long-lived terminal registry entry. Keep its small graph fingerprint for later readback.
        record.job.result = None;
        record.board_commands.clear();
        record.job.updated_at_ms = wall_clock_ms();
        record.touched_at = now;
        let job = record.job.clone();
        if let Err(error) = persist_board_edit_jobs(&jobs) {
            jobs.insert(job_id.to_string(), previous);
            return Err(error);
        }
        job
    };

    // The terminal job snapshot now prevents any future receipt replay. Remove the potentially
    // large board-embedded recovery value best-effort; a crash here merely leaves harmless
    // bookkeeping that the bounded receipt set will eventually replace.
    if let Some(state) = app_handle.try_state::<TauriFlowLikeState>()
        && let Ok(live_board) = state.0.get_board(&job.board_id, None)
    {
        let mut board = live_board.write().await;
        let key = flow_ir_durable_receipt_ref_key(&job.app_id, &job.token);
        if let Some(encoded) = board.remove_internal_ref(&key) {
            match TauriFlowLikeState::get_project_meta_store(&app_handle).await {
                Ok(store) => {
                    if let Err(error) = board.save(Some(store)).await {
                        let _ = board.insert_internal_ref(key, encoded);
                        tracing::warn!(
                            "could not prune delivered FlowPilot board receipt: {error}"
                        );
                    }
                }
                Err(error) => {
                    let _ = board.insert_internal_ref(key, encoded);
                    tracing::warn!(
                        "could not open board store to prune FlowPilot receipt: {error}"
                    );
                }
            }
        }
    }
    Ok(job)
}
