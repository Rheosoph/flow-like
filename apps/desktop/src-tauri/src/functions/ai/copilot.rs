use super::copilot_sdk_tools::{
    SideEffectCommandQueue, retained_flow_ir_draft_store, retained_flow_ir_draft_store_for_board,
};
use super::frontend_tool_bridge::FrontendToolContext;
use crate::state::{TauriFlowLikeState, TauriSettingsState};
use async_trait::async_trait;
use dashmap::DashMap;
use flow_like::a2ui::SurfaceComponent;
use flow_like::app::{App, AppVisibility};
use flow_like::copilot::FlowIrCommitToken;
use flow_like::copilot::{
    ChatImage, CopilotScope, UIActionContext, UnifiedChatMessage, UnifiedContext, UnifiedCopilot,
    UnifiedCopilotResponse,
};
use flow_like::flow::board::Board;
use flow_like::flow::board::commands::GenericCommand;
use flow_like::flow::copilot::memory::{AssistantMemory, MemoryEntry, MemoryStatus};
use flow_like::flow::copilot::platform::PlatformToolBridge;
use flow_like::flow::copilot::{
    AttachmentManifestEntry, BoardCommand, CatalogProvider, EmitCommandsArgs,
    FlowScriptCandidateRegression, FlowScriptPendingDelivery, FlowScriptRepairTracker,
    GlobalDataStudioContext, GlobalOpenBoardContext, GraphContext, NodeMetadata, PinMetadata,
    PlatformContextInput, RunContext, build_platform_context, emit_validation_requires_flowscript,
    enrich_node_metadata, flowscript_workspace_envelope, global_assistant_system_prompt,
    profile_flowscript_candidate, render_flowscript_modular_partial_result, run_platform_chat,
    score_catalog_metadata, validate_model_facing_emit_commands_scope,
};
use flow_like::flow::node::Node;
use flow_like::flow::pin::{Pin, PinType};
use flow_like::flow::variable::VariableType;
use flow_like::models::llm::ModelUsageContext;
use flow_like_catalog::get_catalog;
use flow_like_types::tokio_util::sync::CancellationToken;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, LazyLock, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering},
    },
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Manager, State,
    ipc::{Channel, InvokeResponseBody},
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tokio::sync::{Semaphore, watch};

const EXTERNAL_AGENT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const EXTERNAL_AGENT_HANDLER_QUIESCENCE_TIMEOUT: Duration = Duration::from_secs(15);
const EXTERNAL_AGENT_STDERR_MAX_BYTES: usize = 256 * 1024;
const EXTERNAL_AGENT_TEXT_MAX_BYTES: usize = 2 * 1024 * 1024;
const EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES: usize = 256;
const MCP_TOOL_PROGRESS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const SDK_CONTROL_RPC_TIMEOUT: Duration = Duration::from_secs(30);
const SDK_CHAT_ABORT_TIMEOUT: Duration = Duration::from_secs(5);
// A direct SDK session previously waited forever when the CLI/event transport disappeared after a
// tool start. This is deliberately longer than the frontend bridge's 120-second approval/tool
// deadline, and resets after every received event.
const SDK_EVENT_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(180);
const SDK_RESPONSE_MAX_BYTES: usize = 2 * 1024 * 1024;
const SDK_USAGE_CALLS_MAX_ENTRIES: usize = 256;

#[derive(Clone)]
struct ActiveCopilotRun {
    generation: uuid::Uuid,
    cancellation: CancellationToken,
}

static ACTIVE_COPILOT_RUNS: LazyLock<DashMap<String, ActiveCopilotRun>> =
    LazyLock::new(DashMap::new);

struct ActiveCopilotRunGuard {
    request_id: Option<String>,
    generation: uuid::Uuid,
    cancellation: CancellationToken,
}

impl Drop for ActiveCopilotRunGuard {
    fn drop(&mut self) {
        // Tool handlers registered with the SDK can outlive the async chat future. Always cancel
        // the run token before removing its registry entry so their blocking frontend waits stop
        // on every return path, including explicit cancellation and provider errors.
        self.cancellation.cancel();
        let Some(request_id) = self.request_id.as_deref() else {
            return;
        };
        ACTIVE_COPILOT_RUNS.remove_if(request_id, |_, run| run.generation == self.generation);
    }
}

fn register_copilot_run(request_id: Option<&str>) -> (CancellationToken, ActiveCopilotRunGuard) {
    let cancellation = CancellationToken::new();
    let generation = uuid::Uuid::new_v4();
    let request_id = request_id
        .map(str::trim)
        .filter(|request_id| !request_id.is_empty())
        .map(str::to_string);
    if let Some(request_id) = request_id.as_ref() {
        if let Some(previous) = ACTIVE_COPILOT_RUNS.insert(
            request_id.clone(),
            ActiveCopilotRun {
                generation,
                cancellation: cancellation.clone(),
            },
        ) {
            // Request ids are expected to be unique. If a caller reuses one, stop the stale run
            // before replacing it so a late completion cannot keep mutating the same board.
            previous.cancellation.cancel();
        }
    }
    (
        cancellation.clone(),
        ActiveCopilotRunGuard {
            request_id,
            generation,
            cancellation: cancellation.clone(),
        },
    )
}

/// Cancel a detached/nested FlowPilot agent run by the frontend bridge request id. Cancellation is
/// cooperative for SDK calls and forceful for external CLI processes; the run remains registered
/// until its RAII cleanup finishes.
#[tauri::command]
pub fn cancel_copilot_chat(request_id: String) -> Result<bool, String> {
    let request_id = request_id.trim();
    if request_id.is_empty() {
        return Err("FlowPilot cancellation requires a non-empty request id".to_string());
    }
    let Some(run) = ACTIVE_COPILOT_RUNS.get(request_id) else {
        return Ok(false);
    };
    run.cancellation.cancel();
    Ok(true)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowIrCommitDisposition {
    Preflight,
    Applied,
    Dismissed,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowIrCommitDispositionResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct ApplyFlowIrCommitResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    pub commands: Vec<GenericCommand>,
    pub board_commands: Vec<BoardCommand>,
    pub diagnostics: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_board_node_count: Option<usize>,
}

const FLOW_IR_APPLIED_RECEIPT_TTL: Duration = Duration::from_secs(2 * 60 * 60);
const FLOW_IR_APPLIED_RECEIPT_MAX_ENTRIES: usize = 512;
static FLOW_IR_APPLIED_RECEIPTS: LazyLock<
    StdMutex<HashMap<String, (Instant, ApplyFlowIrCommitResult)>>,
> = LazyLock::new(|| StdMutex::new(HashMap::new()));

fn flow_ir_applied_receipt_key(app_id: &str, token: &FlowIrCommitToken) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        app_id,
        token.board_id,
        token.draft_id,
        token.revision,
        token.base_fingerprint,
        token.claim_id
    )
}

fn prune_flow_ir_applied_receipts(
    receipts: &mut HashMap<String, (Instant, ApplyFlowIrCommitResult)>,
    now: Instant,
) {
    receipts.retain(|_, (created_at, _)| {
        now.saturating_duration_since(*created_at) <= FLOW_IR_APPLIED_RECEIPT_TTL
    });
    while receipts.len() >= FLOW_IR_APPLIED_RECEIPT_MAX_ENTRIES {
        let Some(oldest) = receipts
            .iter()
            .min_by_key(|(_, (created_at, _))| *created_at)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        receipts.remove(&oldest);
    }
}

fn replay_flow_ir_applied_receipt(
    app_id: &str,
    token: &FlowIrCommitToken,
) -> Option<ApplyFlowIrCommitResult> {
    let now = Instant::now();
    let mut receipts = FLOW_IR_APPLIED_RECEIPTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    prune_flow_ir_applied_receipts(&mut receipts, now);
    receipts
        .get(&flow_ir_applied_receipt_key(app_id, token))
        .map(|(_, result)| {
            let mut replay = result.clone();
            replay.message = format!("{} (idempotent replay)", replay.message);
            replay
        })
}

fn retain_flow_ir_applied_receipt(
    app_id: &str,
    token: &FlowIrCommitToken,
    result: &ApplyFlowIrCommitResult,
) {
    let now = Instant::now();
    let mut receipts = FLOW_IR_APPLIED_RECEIPTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    prune_flow_ir_applied_receipts(&mut receipts, now);
    receipts.insert(
        flow_ir_applied_receipt_key(app_id, token),
        (now, result.clone()),
    );
}

impl ApplyFlowIrCommitResult {
    fn empty(status: &str, code: &str, message: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            code: Some(code.to_string()),
            message: message.into(),
            commands: Vec::new(),
            board_commands: Vec::new(),
            diagnostics: Vec::new(),
            final_board_node_count: None,
        }
    }

    fn apply_error(
        code: &str,
        message: impl Into<String>,
        board_commands: Vec<BoardCommand>,
        diagnostics: Vec<String>,
    ) -> Self {
        Self {
            status: "error".to_string(),
            code: Some(code.to_string()),
            message: message.into(),
            commands: Vec::new(),
            board_commands,
            diagnostics,
            final_board_node_count: None,
        }
    }
}

impl FlowIrCommitDispositionResult {
    fn success(status: &str, message: &str) -> Self {
        Self {
            status: status.to_string(),
            code: None,
            message: message.to_string(),
        }
    }

    fn error(code: &str, message: &str) -> Self {
        Self {
            status: "error".to_string(),
            code: Some(code.to_string()),
            message: message.to_string(),
        }
    }
}

/// Resolve the exact compiled-workflow review carried by a FlowPilot response. Preflight remains
/// available for review UX and Dismissed releases the exact revision. Applied is deliberately
/// rejected: only `flowpilot_apply_flow_ir_commit` may mutate and acknowledge a compiled batch
/// atomically.
#[tauri::command]
pub async fn flowpilot_flow_ir_commit_disposition(
    app_handle: AppHandle,
    token: FlowIrCommitToken,
    disposition: FlowIrCommitDisposition,
) -> FlowIrCommitDispositionResult {
    if token.board_id.trim().is_empty()
        || token.draft_id.trim().is_empty()
        || token.base_fingerprint.trim().is_empty()
        || token.claim_id.trim().is_empty()
    {
        return FlowIrCommitDispositionResult::error(
            "IR_COMMIT_TOKEN_INVALID",
            "The compiled workflow review token is incomplete.",
        );
    }
    let Some(store) = retained_flow_ir_draft_store(&token.board_id) else {
        return FlowIrCommitDispositionResult::error(
            "IR_COMMIT_TOKEN_INVALID",
            "The compiled workflow review is no longer retained by this desktop process.",
        );
    };

    if matches!(disposition, FlowIrCommitDisposition::Dismissed) {
        // Serialize Dismiss with the native atomic Apply command. Dismiss remains available when
        // the board has already closed, but while it is live it must not revoke a claim between
        // exact-batch verification and acknowledgement under the board write lock.
        let dismiss_live_board = app_handle
            .try_state::<TauriFlowLikeState>()
            .and_then(|state| state.0.get_board(&token.board_id, None).ok());
        let _live_board_guard = match dismiss_live_board.as_ref() {
            Some(live_board) => Some(live_board.lock().await),
            None => None,
        };
        return if store.release_commit_if_matches(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        ) {
            FlowIrCommitDispositionResult::success(
                "dismissed",
                "The compiled workflow review was dismissed and its exact revision was released.",
            )
        } else {
            FlowIrCommitDispositionResult::error(
                "IR_COMMIT_TOKEN_INVALID",
                "The compiled workflow review token no longer identifies a pending revision.",
            )
        };
    }
    if matches!(disposition, FlowIrCommitDisposition::Applied) {
        return FlowIrCommitDispositionResult::error(
            "IR_COMMIT_ATOMIC_APPLY_REQUIRED",
            "Compiled workflow changes must be applied through the native atomic Apply command; a separate applied acknowledgement is not accepted.",
        );
    }

    let Some(state) = app_handle.try_state::<TauriFlowLikeState>() else {
        return FlowIrCommitDispositionResult::error(
            "IR_COMMIT_BOARD_UNAVAILABLE",
            "The live board registry is unavailable; the review was not resolved.",
        );
    };
    let Ok(live_board) = state.0.get_board(&token.board_id, None) else {
        return FlowIrCommitDispositionResult::error(
            "IR_COMMIT_BOARD_UNAVAILABLE",
            "The review board is not open in this desktop process; the review was not resolved.",
        );
    };
    let board = live_board.lock().await;
    match disposition {
        FlowIrCommitDisposition::Preflight => {
            if store.pending_commit_is_current(
                &board,
                &token.draft_id,
                token.revision,
                &token.base_fingerprint,
                &token.claim_id,
            ) {
                FlowIrCommitDispositionResult::success(
                    "current",
                    "The compiled workflow review still matches the live board and may be applied.",
                )
            } else {
                FlowIrCommitDispositionResult::error(
                    "IR_COMMIT_REVIEW_STALE",
                    "The live board or retained compiled revision changed after this review was generated. Dismiss it and regenerate against the current board.",
                )
            }
        }
        FlowIrCommitDisposition::Applied => unreachable!("legacy applied disposition rejected"),
        FlowIrCommitDisposition::Dismissed => unreachable!("dismiss handled before board lookup"),
    }
}

/// Atomically apply the exact command batch retained behind a compiled-workflow review token.
///
/// The client cannot supply or alter the commands. The live board write lock spans token/base
/// validation, retained-batch lookup, rollback-safe application, persistence, and exact claim
/// acknowledgement, closing the preflight/apply TOCTOU window.
#[tauri::command]
pub async fn flowpilot_apply_flow_ir_commit(
    app_handle: AppHandle,
    app_id: String,
    token: FlowIrCommitToken,
) -> ApplyFlowIrCommitResult {
    if app_id.trim().is_empty()
        || token.board_id.trim().is_empty()
        || token.draft_id.trim().is_empty()
        || token.base_fingerprint.trim().is_empty()
        || token.claim_id.trim().is_empty()
    {
        return ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_TOKEN_INVALID",
            "The compiled workflow review token or app id is incomplete.",
        );
    }

    if let Some(receipt) = replay_flow_ir_applied_receipt(&app_id, &token) {
        return receipt;
    }

    let Some(store) = retained_flow_ir_draft_store(&token.board_id) else {
        return ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_TOKEN_INVALID",
            "The compiled workflow review is no longer retained by this desktop process.",
        );
    };
    let Some(managed_state) = app_handle.try_state::<TauriFlowLikeState>() else {
        return ApplyFlowIrCommitResult::empty(
            "error",
            "IR_COMMIT_BOARD_UNAVAILABLE",
            "The live board registry is unavailable; nothing was applied.",
        );
    };
    let flow_like_state = managed_state.0.clone();
    let Ok(live_board) = flow_like_state.get_board(&token.board_id, None) else {
        return ApplyFlowIrCommitResult::empty(
            "error",
            "IR_COMMIT_BOARD_UNAVAILABLE",
            "The review board is not open in this desktop process; nothing was applied.",
        );
    };
    let project_store = match TauriFlowLikeState::get_project_meta_store(&app_handle).await {
        Ok(store) => store,
        Err(error) => {
            return ApplyFlowIrCommitResult::empty(
                "error",
                "IR_COMMIT_PERSISTENCE_UNAVAILABLE",
                format!("The board store is unavailable; nothing was applied: {error}"),
            );
        }
    };

    // Build the app-scoped catalog from the authoritative native registry before taking the board
    // write lock. Renderer-supplied Node/WASM metadata is intentionally ignored: package ids alone
    // do not authenticate pin schemas, versions, permissions, or executable module metadata.
    let all_nodes = match flow_like_state.node_registry.read().await.get_nodes() {
        Ok(nodes) => nodes,
        Err(error) => {
            return ApplyFlowIrCommitResult::empty(
                "error",
                "IR_COMMIT_CATALOG_UNAVAILABLE",
                format!("The live node catalog is unavailable; nothing was applied: {error}"),
            );
        }
    };
    let app = match App::load(app_id.clone(), flow_like_state.clone()).await {
        Ok(app) => app,
        Err(error) => {
            return ApplyFlowIrCommitResult::empty(
                "error",
                "IR_COMMIT_APP_UNAVAILABLE",
                format!("The target app could not be loaded; nothing was applied: {error}"),
            );
        }
    };
    if !app.boards.contains(&token.board_id) {
        return ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_APP_BOARD_MISMATCH",
            "The review board does not belong to the requested app; nothing was applied.",
        );
    }
    let allowed_packages = app.packages.keys().cloned().collect::<HashSet<_>>();
    let app_catalog = all_nodes
        .into_iter()
        .filter(|node| match &node.wasm {
            None => true,
            Some(wasm) => allowed_packages.contains(&wasm.package_id),
        })
        .collect::<Vec<_>>();

    let mut board = live_board.lock().await;
    let Some(mut board_commands) = store.pending_commands_if_current(
        &board,
        &token.draft_id,
        token.revision,
        &token.base_fingerprint,
        &token.claim_id,
    ) else {
        return ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_REVIEW_STALE",
            "The live board or retained compiled revision changed after this review was generated. Nothing was applied.",
        );
    };
    let Some(replacement_mode) = store.pending_commit_requires_destructive_approval(
        &token.draft_id,
        token.revision,
        &token.base_fingerprint,
        &token.claim_id,
    ) else {
        return ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_REVIEW_STALE",
            "The retained compiled revision changed while its review policy was being checked. Nothing was applied.",
        );
    };
    let destructive_review_items =
        typed_commit_destructive_review_items(replacement_mode, &board_commands);
    if !destructive_review_items.is_empty() {
        // Renderer state is not an authorization boundary. Release the live-board lock while the
        // operating-system dialog is open, then reacquire it and repeat the exact claim/base/batch
        // checks before applying. A compromised renderer can request this dialog, but it cannot
        // synthesize the native user's answer or race an approved answer onto another revision.
        drop(board);
        if !confirm_destructive_flow_ir_commit(app_handle.clone(), destructive_review_items.clone())
            .await
        {
            return ApplyFlowIrCommitResult::apply_error(
                "IR_COMMIT_DESTRUCTIVE_APPROVAL_DENIED",
                "The native destructive workflow confirmation was denied or unavailable. Nothing was applied and the exact claim remains pending.",
                board_commands,
                destructive_review_items,
            );
        }

        board = live_board.lock().await;
        let Some(revalidated_commands) = store.pending_commands_if_current(
            &board,
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        ) else {
            return ApplyFlowIrCommitResult::empty(
                "stale",
                "IR_COMMIT_REVIEW_STALE",
                "The live board or retained compiled revision changed while native destructive confirmation was open. Nothing was applied.",
            );
        };
        let Some(revalidated_replacement_mode) = store
            .pending_commit_requires_destructive_approval(
                &token.draft_id,
                token.revision,
                &token.base_fingerprint,
                &token.claim_id,
            )
        else {
            return ApplyFlowIrCommitResult::empty(
                "stale",
                "IR_COMMIT_REVIEW_STALE",
                "The retained compiled revision changed while native destructive confirmation was open. Nothing was applied.",
            );
        };
        let revalidated_review_items = typed_commit_destructive_review_items(
            revalidated_replacement_mode,
            &revalidated_commands,
        );
        let exact_batch_unchanged =
            exact_board_command_batch_matches(&board_commands, &revalidated_commands);
        if !exact_batch_unchanged || revalidated_review_items != destructive_review_items {
            return ApplyFlowIrCommitResult::empty(
                "stale",
                "IR_COMMIT_REVIEW_STALE",
                "The exact destructive batch changed while native confirmation was open. Nothing was applied.",
            );
        }
        board_commands = revalidated_commands;
    }

    let original_board = board.clone();
    let retained_commands = board_commands.clone();
    let apply_result = match flow_like::flow::ast::apply_board_commands_to_board(
        &mut board,
        board_commands,
        &app_catalog,
        flow_like_state.clone(),
        None,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            // Core rolls back every executed prefix. Restore the exact snapshot as a final host
            // guard so an unexpected planner/rollback defect cannot leak a partial live mutation.
            *board = original_board;
            return ApplyFlowIrCommitResult::apply_error(
                "IR_COMMIT_APPLY_FAILED",
                format!(
                    "The exact compiled workflow batch could not be applied and remains retryable: {error}"
                ),
                retained_commands,
                vec![error.to_string()],
            );
        }
    };

    if apply_result.commands.is_empty() || !apply_result.diagnostics.is_empty() {
        *board = original_board;
        let diagnostics = if apply_result.diagnostics.is_empty() {
            vec!["The exact compiled workflow batch produced no executed commands.".to_string()]
        } else {
            apply_result.diagnostics
        };
        return ApplyFlowIrCommitResult::apply_error(
            "IR_COMMIT_APPLY_FAILED",
            "The exact compiled workflow batch did not complete; its claim remains available for retry or dismissal.",
            apply_result.board_commands,
            diagnostics,
        );
    }

    if let Err(error) = board.save(Some(project_store.clone())).await {
        let rollback_error = board
            .undo(apply_result.commands.clone(), flow_like_state.clone())
            .await
            .err()
            .map(|error| error.to_string());
        *board = original_board;
        let restore_error = board
            .save(Some(project_store.clone()))
            .await
            .err()
            .map(|error| error.to_string());
        let mut diagnostics = vec![format!("Board persistence failed: {error}")];
        if let Some(error) = rollback_error {
            diagnostics.push(format!("Command rollback reported: {error}"));
        }
        if let Some(error) = restore_error {
            diagnostics.push(format!(
                "Restoring the persisted board snapshot reported: {error}"
            ));
        }
        return ApplyFlowIrCommitResult::apply_error(
            "IR_COMMIT_SAVE_FAILED",
            "The compiled workflow batch could not be persisted. The live board was restored and the claim remains retryable.",
            apply_result.board_commands,
            diagnostics,
        );
    }

    let acknowledged = store.acknowledge_applied_commit(
        &board,
        &token.draft_id,
        token.revision,
        &token.base_fingerprint,
        &token.claim_id,
    );
    // The exact claim/base/batch was validated under this continuous board lock before execution,
    // so a failed acknowledgement only means the claim BOOKKEEPING was resolved concurrently (a
    // dismissal issued after a lost response channel, TTL cleanup, or a duplicate disposition).
    // The applied and persisted board is correct and must not be rolled back — destroying it here
    // previously forced full rebuild cycles of an identical batch. Best-effort release keeps the
    // store from redelivering the already-applied review.
    let mut diagnostics = apply_result.diagnostics;
    let mut code = None;
    if !acknowledged {
        let released = store.release_commit_if_matches(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        );
        code = Some("IR_COMMIT_ACK_RACED".to_string());
        diagnostics.push(flow_ir_ack_race_diagnostic(released));
    }
    let result = ApplyFlowIrCommitResult {
        status: "applied".to_string(),
        code,
        message: format!(
            "Applied and persisted {} exact compiled workflow board command(s).",
            apply_result.commands.len()
        ),
        commands: apply_result.commands,
        board_commands: apply_result.board_commands,
        diagnostics,
        final_board_node_count: Some(board_total_node_count(&board)),
    };
    retain_flow_ir_applied_receipt(&app_id, &token, &result);
    result
}

/// Human-readable trace for an apply whose claim acknowledgement raced a concurrent disposition.
/// The applied, persisted board is kept either way; this only records how the retained-store
/// bookkeeping was resolved.
fn flow_ir_ack_race_diagnostic(released: bool) -> String {
    if released {
        "The exact claim was resolved concurrently while this apply was executing; the leftover pending review was released after the batch was applied and persisted."
            .to_string()
    } else {
        "The exact claim was resolved concurrently while this apply was executing (for example a dismissal after a lost response channel); the applied and persisted board was kept."
            .to_string()
    }
}

fn typed_commit_destructive_review_items(
    replacement_mode: bool,
    commands: &[BoardCommand],
) -> Vec<String> {
    let mut items = flow_like::flow::ast::destructive_flowscript_command_summaries(commands);
    if replacement_mode && items.is_empty() {
        items.push("The draft uses full-board replacement semantics.".to_string());
    }
    items
}

/// Fail-closed structural equality for a batch reviewed across an unlocked native-dialog window.
/// `BoardCommand` intentionally has no semantic `PartialEq`; its tagged wire form is the exact
/// retained contract shared with review/telemetry, and any serialization failure denies Apply.
fn exact_board_command_batch_matches(
    reviewed: &[BoardCommand],
    revalidated: &[BoardCommand],
) -> bool {
    match (
        serde_json::to_vec(reviewed),
        serde_json::to_vec(revalidated),
    ) {
        (Ok(reviewed), Ok(revalidated)) => reviewed == revalidated,
        _ => false,
    }
}

/// Count identities across the root board and function/collapsed layers. Layer nodes are not
/// guaranteed to be duplicated in `Board::nodes`, so a root-only count can report a substantial
/// generated workflow as nearly empty in production evaluation telemetry.
fn board_total_node_count(board: &Board) -> usize {
    let mut node_ids = board
        .nodes
        .keys()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for layer in board.layers.values() {
        node_ids.extend(layer.nodes.keys().map(String::as_str));
    }
    node_ids.len()
}

async fn confirm_destructive_flow_ir_commit(
    app_handle: AppHandle,
    destructive_review_items: Vec<String>,
) -> bool {
    let mut message = String::from(
        "FlowPilot is about to replace or remove existing workflow state. Review the exact destructive effects below:\n\n",
    );
    for item in destructive_review_items.iter().take(12) {
        message.push_str("\u{2022} ");
        message.push_str(item);
        message.push('\n');
    }
    if destructive_review_items.len() > 12 {
        message.push_str(&format!(
            "\u{2022} ... and {} more destructive effect(s)\n",
            destructive_review_items.len() - 12
        ));
    }
    message.push_str("\nOnly choose Replace and apply if these effects are intended.");

    tokio::task::spawn_blocking(move || {
        app_handle
            .dialog()
            .message(message)
            .title("Approve destructive workflow change")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Replace and apply".to_string(),
                "Cancel".to_string(),
            ))
            .blocking_show()
    })
    .await
    .unwrap_or(false)
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

/// Append text while keeping the retained copy bounded. Streaming still forwards each delta to
/// the frontend; this cap only prevents a long-running agent from retaining an unbounded duplicate
/// in the native process.
fn append_bounded_text(target: &mut String, value: &str, max_bytes: usize) -> bool {
    const TRUNCATED: &str = "\n[FlowPilot output truncated in native retention]";
    if value.is_empty() {
        return true;
    }
    if target.len().saturating_add(value.len()) <= max_bytes {
        target.push_str(value);
        return true;
    }
    if target.len() >= max_bytes {
        return false;
    }

    let available = max_bytes - target.len();
    let content_bytes = available.saturating_sub(TRUNCATED.len());
    target.push_str(utf8_prefix(value, content_bytes));
    target.push_str(utf8_prefix(TRUNCATED, max_bytes - target.len()));
    false
}

fn append_bounded_tail(target: &mut String, value: &str, max_bytes: usize) {
    if value.is_empty() || max_bytes == 0 {
        return;
    }
    target.push_str(value);
    if target.len() <= max_bytes {
        return;
    }
    let mut keep_from = target.len() - max_bytes;
    while keep_from < target.len() && !target.is_char_boundary(keep_from) {
        keep_from += 1;
    }
    target.drain(..keep_from);
}

/// Avoid emitting verbose FlowPilot lifecycle traces in production. User-visible stream frames,
/// tool results, warnings, and errors use separate paths and remain available in every build.
macro_rules! flowpilot_debug_log {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            println!($($arg)*);
        }
    };
}

macro_rules! flowpilot_debug_trace {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            tracing::debug!($($arg)*);
        }
    };
}

/// Desktop implementation of the catalog provider for node search
struct DesktopCatalogProvider {
    nodes: Arc<Vec<Node>>,
}

impl DesktopCatalogProvider {
    fn new(injected_nodes: Option<Vec<Node>>) -> Self {
        let mut nodes = static_catalog_nodes();

        if let Some(injected_nodes) = injected_nodes {
            let mut wasm_node_keys: HashSet<(String, String)> = nodes
                .iter()
                .filter_map(|node| {
                    node.wasm
                        .as_ref()
                        .map(|wasm| (wasm.package_id.clone(), node.name.clone()))
                })
                .collect();

            for node in injected_nodes {
                let Some(wasm) = node.wasm.as_ref() else {
                    continue;
                };

                if wasm_node_keys.insert((wasm.package_id.clone(), node.name.clone())) {
                    nodes.push(node);
                }
            }
        }

        Self {
            nodes: Arc::new(nodes),
        }
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }

    fn all_metadata(&self) -> Vec<NodeMetadata> {
        self.nodes.iter().map(node_to_metadata).collect()
    }
}

fn static_catalog_nodes() -> Vec<Node> {
    get_catalog()
        .into_iter()
        .map(|logic| logic.get_node())
        .collect()
}

async fn authoritative_app_catalog_nodes(
    app_handle: &AppHandle,
    app_id: Option<&str>,
) -> Option<Vec<Node>> {
    let app_id = app_id.map(str::trim).filter(|app_id| !app_id.is_empty())?;
    let managed_state = app_handle.try_state::<TauriFlowLikeState>()?;
    let flow_like_state = managed_state.0.clone();
    let app = App::load(app_id.to_string(), flow_like_state.clone())
        .await
        .ok()?;
    let allowed_packages = app.packages.keys().cloned().collect::<HashSet<_>>();
    let nodes = flow_like_state
        .node_registry
        .read()
        .await
        .get_nodes()
        .ok()?;
    Some(
        nodes
            .into_iter()
            .filter(|node| match &node.wasm {
                None => true,
                Some(wasm) => allowed_packages.contains(&wasm.package_id),
            })
            .collect(),
    )
}

fn pin_to_metadata(p: &Pin) -> PinMetadata {
    let is_generic = p.data_type == VariableType::Generic;
    let enforce_schema = p
        .options
        .as_ref()
        .and_then(|o| o.enforce_schema)
        .unwrap_or(false);
    let valid_values = p.options.as_ref().and_then(|o| o.valid_values.clone());

    PinMetadata {
        name: p.name.clone(),
        friendly_name: p.friendly_name.clone(),
        description: p.description.clone(),
        data_type: format!("{:?}", p.data_type),
        value_type: format!("{:?}", p.value_type),
        default_value: p
            .default_value
            .as_ref()
            .map(|value| String::from_utf8_lossy(value).to_string())
            .filter(|value| !value.is_empty() && value != "null"),
        schema: p.schema.clone(),
        is_generic,
        valid_values,
        enforce_schema,
    }
}

fn node_to_metadata(node: &Node) -> NodeMetadata {
    let derived_category = node
        .name
        .to_lowercase()
        .split("::")
        .nth(1)
        .unwrap_or("")
        .to_string();
    let category = if derived_category.is_empty() {
        node.category.clone()
    } else {
        derived_category
    };

    let mut inputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Input)
        .collect();
    inputs.sort_by_key(|p| (p.index, p.name.clone()));

    let mut outputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output)
        .collect();
    outputs.sort_by_key(|p| (p.index, p.name.clone()));

    enrich_node_metadata(NodeMetadata {
        name: node.name.clone(),
        friendly_name: node.friendly_name.clone(),
        description: node.description.clone(),
        inputs: inputs.into_iter().map(pin_to_metadata).collect(),
        outputs: outputs.into_iter().map(pin_to_metadata).collect(),
        category: Some(category),
        required_inputs: Vec::new(),
        companion_nodes: Vec::new(),
        capability_tags: Vec::new(),
    })
}

#[async_trait]
impl CatalogProvider for DesktopCatalogProvider {
    async fn search(&self, query: &str) -> Vec<NodeMetadata> {
        let mut scored_matches: Vec<(i32, NodeMetadata)> = Vec::new();

        for node in self.nodes.iter() {
            let metadata = node_to_metadata(node);
            let score = score_catalog_metadata(&metadata, query);

            if score > 0 {
                scored_matches.push((score, metadata));
            }
        }

        scored_matches.sort_by(|a, b| b.0.cmp(&a.0));
        scored_matches
            .into_iter()
            .take(10)
            .map(|(_, meta)| meta)
            .collect()
    }

    async fn search_by_pin_type(&self, pin_type: &str, is_input: bool) -> Vec<NodeMetadata> {
        let pin_type = pin_type.to_lowercase();
        let mut matches = Vec::new();

        for node in self.nodes.iter() {
            let has_matching_pin = node.pins.values().any(|p| {
                let is_correct_direction = if is_input {
                    p.pin_type == PinType::Input
                } else {
                    p.pin_type == PinType::Output
                };
                is_correct_direction
                    && format!("{:?}", p.data_type)
                        .to_lowercase()
                        .contains(&pin_type)
            });

            if has_matching_pin {
                matches.push(node_to_metadata(node));
            }
            if matches.len() >= 10 {
                break;
            }
        }
        matches
    }

    async fn filter_by_category(&self, category_prefix: &str) -> Vec<NodeMetadata> {
        let category_prefix = category_prefix.to_lowercase();
        let mut matches = Vec::new();

        for node in self.nodes.iter() {
            let name_lower = node.name.to_lowercase();
            let category = name_lower.split("::").nth(1).unwrap_or("");

            if category.starts_with(&category_prefix) || name_lower.contains(&category_prefix) {
                matches.push(node_to_metadata(node));
            }
            if matches.len() >= 15 {
                break;
            }
        }
        matches
    }

    async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata> {
        self.nodes
            .iter()
            .find(|node| node.name == node_type)
            .map(node_to_metadata)
    }

    async fn get_all_nodes(&self) -> Vec<String> {
        self.nodes.iter().map(|node| node.name.clone()).collect()
    }

    async fn get_all_metadata(&self) -> Vec<NodeMetadata> {
        self.all_metadata()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlowPilotAgentBackendKind {
    GithubCopilot,
    Codex,
    ClaudeCode,
}

impl FlowPilotAgentBackendKind {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "copilot" | "github" | "github-copilot" | "github_copilot" => Some(Self::GithubCopilot),
            "codex" | "openai-codex" | "openai_codex" => Some(Self::Codex),
            "claude" | "claude-code" | "claude_code" => Some(Self::ClaudeCode),
            _ => None,
        }
    }

    fn from_model_prefix(value: &str) -> Option<(Self, &str)> {
        for (prefix, backend) in [
            ("copilot:", Self::GithubCopilot),
            ("github-copilot:", Self::GithubCopilot),
            ("codex:", Self::Codex),
            ("claude-code:", Self::ClaudeCode),
            ("claude:", Self::ClaudeCode),
        ] {
            if let Some(model_id) = value.strip_prefix(prefix) {
                return Some((backend, model_id));
            }
        }

        None
    }

    fn label(self) -> &'static str {
        match self {
            Self::GithubCopilot => "GitHub Copilot",
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
        }
    }

    fn cli_name(self) -> &'static str {
        match self {
            Self::GithubCopilot => "copilot",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude",
        }
    }

    fn env_path_var(self) -> &'static str {
        match self {
            Self::GithubCopilot => "COPILOT_CLI_PATH",
            Self::Codex => "CODEX_CLI_PATH",
            Self::ClaudeCode => "CLAUDE_CODE_CLI_PATH",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowPilotChatBackend {
    Bits,
    Agent(FlowPilotAgentBackendKind),
}

#[derive(Debug, Clone)]
struct FlowPilotModelSelection {
    backend: FlowPilotChatBackend,
    model_id: Option<String>,
}

impl FlowPilotModelSelection {
    fn parse(model_id: Option<String>) -> Self {
        let Some(model_id) = model_id else {
            return Self {
                backend: FlowPilotChatBackend::Bits,
                model_id: None,
            };
        };

        if let Some((backend, stripped_model_id)) =
            FlowPilotAgentBackendKind::from_model_prefix(&model_id)
        {
            return Self {
                backend: FlowPilotChatBackend::Agent(backend),
                model_id: Some(stripped_model_id.to_string()),
            };
        }

        Self {
            backend: FlowPilotChatBackend::Bits,
            model_id: Some(model_id),
        }
    }
}

fn copilot_attachment_extension(media_type: &str) -> &'static str {
    match media_type.to_lowercase().as_str() {
        "image/jpeg" | "jpeg" | "jpg" => "jpg",
        "image/png" | "png" => "png",
        "image/gif" | "gif" => "gif",
        "image/webp" | "webp" => "webp",
        _ => "bin",
    }
}

const MAX_PROMPT_IMAGE_BYTES: usize = 64 * 1024 * 1024;

/// Decode base64 prompt images and persist them as hash-deduped temp files.
/// Shared by every provider that attaches images by path (GitHub Copilot
/// SDK attachments, Codex `--image` flags).
fn write_chat_image_temp_files(images: &[ChatImage]) -> Result<Vec<std::path::PathBuf>, String> {
    use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};

    let attachment_dir = std::env::temp_dir().join("flow-like-copilot-attachments");
    std::fs::create_dir_all(&attachment_dir)
        .map_err(|e| format!("Failed to create attachment directory: {}", e))?;

    images
        .iter()
        .enumerate()
        .map(|(index, image)| {
            // Bound the decoded size before allocating: base64 inflates by 4/3.
            let estimated_bytes = image.data.len() / 4 * 3;
            if estimated_bytes > MAX_PROMPT_IMAGE_BYTES {
                return Err(format!(
                    "Prompt image {} is too large ({} MB, max {} MB)",
                    index + 1,
                    estimated_bytes / (1024 * 1024),
                    MAX_PROMPT_IMAGE_BYTES / (1024 * 1024)
                ));
            }
            let bytes = STANDARD
                .decode(&image.data)
                .map_err(|e| format!("Failed to decode prompt image {}: {}", index + 1, e))?;
            let extension = copilot_attachment_extension(&image.media_type);
            let file_name = format!("{}.{}", blake3::hash(&bytes).to_hex(), extension);
            let file_path = attachment_dir.join(file_name);

            if !file_path.exists() {
                std::fs::write(&file_path, &bytes).map_err(|e| {
                    format!("Failed to write attachment {}: {}", file_path.display(), e)
                })?;
            }

            Ok(file_path)
        })
        .collect()
}

fn build_copilot_attachments(images: &[ChatImage]) -> Result<Vec<UserMessageAttachment>, String> {
    let paths = write_chat_image_temp_files(images)?;
    Ok(paths
        .into_iter()
        .enumerate()
        .map(|(index, file_path)| {
            let extension = file_path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_else(|| "bin".to_string());
            UserMessageAttachment {
                attachment_type: AttachmentType::File,
                path: file_path.to_string_lossy().into_owned(),
                display_name: format!("prompt-image-{}.{}", index + 1, extension),
            }
        })
        .collect())
}

fn resolve_copilot_app_id(
    explicit_app_id: Option<&str>,
    run_context_app_id: Option<&str>,
    action_context_app_id: Option<&str>,
) -> Result<Option<String>, String> {
    let mut resolved: Option<&str> = None;

    for candidate in [explicit_app_id, run_context_app_id, action_context_app_id]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
    {
        if resolved.is_some_and(|existing| existing != candidate) {
            return Err("Conflicting app IDs in copilot request context".to_string());
        }
        resolved = Some(candidate);
    }

    Ok(resolved.map(str::to_string))
}

/// The active profile, with the user's WHOLE custom-model library hydrated
/// instead of only the bits the profile activated. The model pickers offer that
/// library independent of profile membership, so an explicitly selected model
/// must resolve; automatic "best model" selection stays scoped to the profile's
/// `bits` inside `Profile`.
async fn copilot_profile(app_handle: &AppHandle) -> Option<Arc<flow_like::profile::Profile>> {
    let mut profile = TauriSettingsState::current_profile(app_handle).await.ok()?;

    if let Ok(settings) = TauriSettingsState::construct(app_handle).await {
        let settings = settings.lock().await;
        profile.hub_profile.custom_bits = settings
            .custom_bits
            .iter()
            .cloned()
            .map(flow_like::profile::ProfileCustomBit)
            .collect();
    }

    Some(Arc::new(profile.hub_profile))
}

/// Unified copilot chat command that handles both board and UI generation
#[tauri::command]
pub async fn copilot_chat(
    app_handle: AppHandle,
    state: State<'_, TauriFlowLikeState>,
    // Scope selection
    scope: CopilotScope,
    // Board context (optional for Frontend scope)
    board: Option<Board>,
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: Option<Vec<String>>,
    // UI context (optional for Board scope)
    current_surface: Option<Vec<SurfaceComponent>>,
    selected_component_ids: Option<Vec<String>>,
    // Common parameters
    user_prompt: String,
    current_images: Option<Vec<ChatImage>>,
    history: Option<Vec<UnifiedChatMessage>>,
    model_id: Option<String>,
    reasoning_effort: Option<String>,
    token: Option<String>,
    // Extended context
    run_context: Option<RunContext>,
    action_context: Option<UIActionContext>,
    // Sub-agent run spawned while another Copilot session is mid-turn (needs its own CLI)
    nested: Option<bool>,
    // Read-only sub-run (flowpilot_board explain): answer questions about the board without editing.
    read_only: Option<bool>,
    // App scope for hosted-model usage attribution. Omit for genuine global chat.
    app_id: Option<String>,
    // Runtime tools in a detached nested specialist are scoped by the frontend, not model input.
    tool_context: Option<FrontendToolContext>,
    // Stable frontend request id used for end-to-end cancellation of detached agent runs.
    request_id: Option<String>,
    // Immutable user-authored request, separate from host-added mode/run-context guidance.
    raw_user_prompt: Option<String>,
    // Streaming channel
    channel: Channel<String>,
) -> Result<UnifiedCopilotResponse, String> {
    let read_only = read_only.unwrap_or(false);
    let nested = nested.unwrap_or(false);
    let raw_user_prompt = raw_user_prompt
        .filter(|prompt| !prompt.trim().is_empty())
        .or_else(|| {
            tool_context
                .as_ref()
                .and_then(|context| context.source_user_prompt.clone())
                .filter(|prompt| !prompt.trim().is_empty())
        })
        .unwrap_or_else(|| user_prompt.clone());
    // The retained-draft identity and acceptance contract must survive across nested runs spawned
    // from one user turn. Delegated specialist instructions differ per nested run, so identity
    // binds to the outer chat's immutable source prompt whenever the tool context carries it,
    // scoped by the owning conversation id so identical prompt text from another conversation
    // never shares a draft lease; a genuinely different user request still produces a different
    // identity.
    let request_identity_prompt =
        request_identity_prompt_for(tool_context.as_ref(), &raw_user_prompt);
    let host_context_guidance = run_context.as_ref().map(|context| {
        format!(
            "## HOST RUN CONTEXT\nThe user is asking about execution run `{}` for app `{}` and board `{}`. Use the run/log query tools and ground the answer in that run.",
            context.run_id, context.app_id, context.board_id
        )
    });
    if !read_only
        && matches!(scope, CopilotScope::Board | CopilotScope::Both)
        && let Some(board) = board.as_ref()
        && let Some(delivery) =
            pending_flowscript_redelivery_for_request(&app_handle, board, &request_identity_prompt)
                .await
    {
        let parent_request_id = scoped_parent_request_id(tool_context.as_ref());
        let workspace_status = if delivery.stale_board {
            "stale"
        } else {
            "queued"
        };
        send_correlated_stream_json_event(
            &channel,
            "flowscript_workspace",
            &serde_json::json!({
                "source": &delivery.source,
                "status": workspace_status,
            }),
            parent_request_id.as_deref(),
        );
        send_commands_event(&channel, &delivery.commands);
        return Ok(pending_flowscript_redelivery_response(scope, delivery));
    }
    // Full Node/WASM definitions received over IPC are display data, not an authority boundary.
    // Resolve the live app package catalog from the native registry for every board agent path.
    let _renderer_catalog_nodes = catalog_nodes;
    let catalog_nodes = if matches!(scope, CopilotScope::Board | CopilotScope::Both) {
        authoritative_app_catalog_nodes(
            &app_handle,
            tool_context
                .as_ref()
                .and_then(|context| context.app_id.as_deref()),
        )
        .await
    } else {
        None
    };
    let model_selection = FlowPilotModelSelection::parse(model_id);
    if let FlowPilotChatBackend::Agent(agent_backend) = model_selection.backend {
        return match agent_backend {
            FlowPilotAgentBackendKind::GithubCopilot => {
                let model_id = model_selection
                    .model_id
                    .as_deref()
                    .filter(|model_id| !model_id.trim().is_empty())
                    .ok_or_else(|| "GitHub Copilot backend requires a model id".to_string())?;

                copilot_sdk_chat_internal(
                    app_handle.clone(),
                    model_id,
                    reasoning_effort.as_deref(),
                    scope,
                    board.as_ref(),
                    catalog_nodes,
                    selected_node_ids.as_deref().unwrap_or(&[]),
                    current_surface.as_ref(),
                    user_prompt,
                    raw_user_prompt,
                    request_identity_prompt,
                    host_context_guidance,
                    current_images,
                    history.unwrap_or_default(),
                    channel,
                    None,
                    None,
                    tool_context,
                    request_id,
                    nested,
                    read_only,
                )
                .await
            }
            FlowPilotAgentBackendKind::Codex | FlowPilotAgentBackendKind::ClaudeCode => {
                let model_id = model_selection
                    .model_id
                    .clone()
                    .unwrap_or_else(|| "default".to_string());

                external_code_agent_chat_internal(
                    app_handle.clone(),
                    agent_backend,
                    &model_id,
                    reasoning_effort.as_deref(),
                    scope,
                    board.as_ref(),
                    catalog_nodes,
                    selected_node_ids.as_deref().unwrap_or(&[]),
                    current_surface.as_ref(),
                    user_prompt,
                    raw_user_prompt,
                    request_identity_prompt,
                    host_context_guidance,
                    current_images,
                    history.unwrap_or_default(),
                    channel,
                    None,
                    None,
                    tool_context,
                    request_id,
                    nested,
                    read_only,
                )
                .await
            }
        };
    }

    // The Bits/rig backend drives the specialized board/UI copilots, which have no data-layer
    // toolset. The Data Studio agent needs a tool-calling agent backend (Claude Code / Codex /
    // GitHub Copilot). Return a clear message rather than falling through to the board copilot.
    if matches!(scope, CopilotScope::DataStudio) {
        let message = "The Data Studio agent needs a tool-capable model (Claude Code, Codex, or GitHub Copilot). Select one of those FlowPilot models to work with your data, then ask again.".to_string();
        let _ = channel.send(message.clone());
        return Ok(UnifiedCopilotResponse {
            message,
            commands: Vec::new(),
            components: Vec::new(),
            canvas_settings: None,
            root_component_id: None,
            flowscript_workspace: None,
            flow_ir_commit: None,
            suggestions: Vec::new(),
            active_scope: scope,
        });
    }

    flowpilot_debug_log!(
        "[copilot_chat] Called with scope: {:?}, run_context: {:?}",
        scope,
        run_context
    );

    let selected_node_ids = selected_node_ids.unwrap_or_default();
    let selected_component_ids = selected_component_ids.unwrap_or_default();
    let history = history.unwrap_or_default();

    let state_clone = state.0.clone();

    let profile = copilot_profile(&app_handle).await;

    let attribution_app_id = resolve_copilot_app_id(
        app_id.as_deref(),
        run_context.as_ref().map(|context| context.app_id.as_str()),
        action_context
            .as_ref()
            .map(|context| context.app_id.as_str()),
    )?;
    let usage_context = match attribution_app_id.as_deref() {
        Some(app_id) => {
            let app = App::load(app_id.to_string(), state_clone.clone())
                .await
                .map_err(|error| {
                    format!("Failed to resolve app for copilot usage attribution: {error}")
                })?;
            Some(ModelUsageContext {
                app_id: if matches!(app.visibility, AppVisibility::Offline) {
                    None
                } else {
                    Some(app_id.to_string())
                },
                run_id: run_context.as_ref().map(|context| context.run_id.clone()),
            })
        }
        None => None,
    };

    // Only create catalog provider if we might need it (Board or Both scope)
    let catalog_provider: Option<Arc<dyn CatalogProvider>> = match scope {
        CopilotScope::Frontend => None,
        _ => Some(Arc::new(DesktopCatalogProvider::new(catalog_nodes))),
    };

    // Profile/Bits board runs use the core rig loop rather than the SDK/MCP adapters. Attach the
    // same frontend execution bridge explicitly so provider choice does not remove runtime
    // verification tools. Detached nested board specialists must use the global bridge listener.
    let stream_parent_request_id = scoped_parent_request_id(tool_context.as_ref());
    let (run_cancellation, _run_registration) = register_copilot_run(
        request_id
            .as_deref()
            .or(stream_parent_request_id.as_deref()),
    );
    // The in-process Bits/rig loop shares the board-scoped retained draft stores with the agent
    // backends, so its nested runs take the same per-board gate (same-board runs serialize,
    // different boards proceed concurrently). Held for the entire run.
    let _nested_run_permit = if nested {
        Some(
            acquire_nested_copilot_run_permit(
                nested_copilot_run_gate(&nested_copilot_run_gate_key(
                    board.as_ref(),
                    tool_context.as_ref(),
                )),
                run_cancellation.clone(),
            )
            .await?,
        )
    } else {
        None
    };
    let runtime_frontend_bridge = if nested {
        super::frontend_tool_bridge::FrontendToolBridge::new_with_event(
            app_handle.clone(),
            super::frontend_tool_bridge::GLOBAL_FRONTEND_TOOL_EVENT,
        )
    } else {
        super::frontend_tool_bridge::FrontendToolBridge::new(app_handle.clone())
    }
    .with_context(tool_context);
    let runtime_bridge: Arc<dyn PlatformToolBridge> = Arc::new(DesktopPlatformBridge {
        bridge: runtime_frontend_bridge,
        tool_set: FrontendPlatformToolSet::BoardRuntime,
        cancellation: run_cancellation.clone(),
    });

    let mut run_summary = WorkflowRunSummaryEmitter::new(
        channel.clone(),
        stream_parent_request_id.clone(),
        "bits",
        model_selection.model_id.as_deref().unwrap_or("default"),
        run_cancellation.clone(),
    );
    run_summary.record_phase();

    let copilot_init =
        UnifiedCopilot::new(state_clone, catalog_provider, profile, None, usage_context);
    let mut copilot = tokio::select! {
        result = copilot_init => result.map_err(|error| error.to_string())?,
        _ = run_cancellation.cancelled() => {
            return Err("FlowPilot Bits run was cancelled during initialization".to_string());
        }
    }
    .with_runtime_bridge(runtime_bridge)
    // Bind draft/acceptance identity to the same conversation-scoped request identity the SDK and
    // external agent backends use, while `raw_user_prompt` keeps serving routing/classification.
    .with_request_identity_prompt(Some(request_identity_prompt));

    if !read_only
        && !matches!(scope, CopilotScope::Frontend)
        && let Some(board) = board.as_ref()
    {
        let flow_ir_drafts = retained_flow_ir_draft_store_for_board(board)?;
        copilot = copilot.with_flow_ir_draft_store(flow_ir_drafts);
    }

    let on_token = Some(move |token: String| {
        let token = correlate_stream_frame(&token, stream_parent_request_id.as_deref());
        let _ = channel.send(token);
    });

    // Build unified context
    let context = if run_context.is_some() || action_context.is_some() {
        Some(UnifiedContext {
            scope,
            run_context,
            action_context,
        })
    } else {
        None
    };

    let chat = copilot.chat_with_raw_user_prompt(
        scope,
        board.as_ref(),
        &selected_node_ids,
        current_surface.as_ref(),
        &selected_component_ids,
        user_prompt,
        Some(raw_user_prompt),
        current_images,
        history,
        model_selection.model_id,
        token,
        context,
        on_token,
    );
    let chat_result = tokio::select! {
        result = chat => result.map_err(|error| error.to_string()),
        _ = run_cancellation.cancelled() => {
            Err("FlowPilot Bits run was cancelled".to_string())
        }
    };
    if let Ok(response) = &chat_result {
        run_summary.set_applied_commands(response.commands.len());
        run_summary.set_outcome(
            if response.commands.is_empty() && response.flow_ir_commit.is_none() {
                "completed"
            } else {
                "committed"
            },
        );
    }
    chat_result
}

/// Collects self-awareness context for the global assistant: the signed-in user (supplied by the
/// frontend), the active profile, the names of the user's other profiles, and — when the user has a
/// board open — that board's identity. Gathers the Tauri-owned data (profiles, active profile) and
/// delegates the shared rendering to [`build_platform_context`] so desktop and server produce the
/// same context wording.
async fn build_global_agent_context(
    app_handle: &AppHandle,
    user_context: Option<&str>,
    open_board: Option<&GlobalOpenBoardContext>,
    open_data_studio: Option<&GlobalDataStudioContext>,
    attachments: &[AttachmentManifestEntry],
) -> String {
    let active = TauriSettingsState::current_profile(app_handle)
        .await
        .ok()
        .map(|current| {
            let profile = &current.hub_profile;
            (profile.name.clone(), profile.id.clone())
        });

    let switchable: Vec<String> =
        match crate::functions::settings::profiles::get_profiles(app_handle.clone()).await {
            Ok(profiles) => profiles
                .values()
                .map(|profile| {
                    let name = profile.hub_profile.name.trim();
                    if name.is_empty() {
                        profile.hub_profile.id.clone()
                    } else {
                        name.to_string()
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        };

    build_platform_context(PlatformContextInput {
        user_context,
        active_profile: active
            .as_ref()
            .map(|(name, id)| (name.as_str(), id.as_str())),
        switchable_profiles: &switchable,
        open_board,
        open_data_studio,
        attachments,
    })
}

fn attachment_media_type(url: &str) -> String {
    let name_hint = url
        .split_once("filename=")
        .map(|(_, rest)| rest.split('&').next().unwrap_or(rest))
        .map(|encoded| urlencoding::decode(encoded).unwrap_or_default().to_string())
        .unwrap_or_else(|| url.split('?').next().unwrap_or(url).to_string());

    match name_hint.rsplit('.').next().map(str::to_ascii_lowercase) {
        Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg".to_string(),
        Some(ext) if ext == "gif" => "image/gif".to_string(),
        Some(ext) if ext == "webp" => "image/webp".to_string(),
        _ => "image/png".to_string(),
    }
}

/// Convert a Tauri asset-protocol URL (produced by `convertFileSrc`) back to the local file path.
fn local_asset_path(url: &str) -> Option<PathBuf> {
    let without_query = url.split('?').next().unwrap_or(url);
    let encoded_path = without_query
        .strip_prefix("asset://localhost/")
        .or_else(|| {
            without_query
                .split_once("asset.localhost/")
                .map(|(_, rest)| rest)
        })?;
    let decoded = urlencoding::decode(encoded_path).ok()?.to_string();
    // On unix the leading slash is consumed by the host split; restore it when missing.
    let path = if decoded.starts_with('/') || decoded.contains(":\\") || decoded.contains(":/") {
        PathBuf::from(decoded)
    } else {
        PathBuf::from(format!("/{decoded}"))
    };
    path.is_file().then_some(path)
}

/// Maximum size of a single fetched attachment; larger ones are skipped to bound memory use.
const MAX_ATTACHMENT_BYTES: u64 = 512 * 1024 * 1024;

/// Resolve chat attachment URLs (local tmp files via the asset protocol, or presigned tmp uploads)
/// into base64 `ChatImage`s for the model — mirrors the simple chat's attachment handling, keeping
/// large blobs out of the frontend store and IPC payloads.
async fn resolve_attachment_images(urls: &[String]) -> Vec<ChatImage> {
    use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};

    let mut images = Vec::with_capacity(urls.len());
    for url in urls {
        let bytes = if let Some(path) = local_asset_path(url) {
            match tokio::fs::read(&path).await {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    eprintln!("[global_chat] failed to read attachment {path:?}: {error}");
                    None
                }
            }
        } else if url.starts_with("http://") || url.starts_with("https://") {
            match flow_like_types::reqwest::get(url).await {
                Ok(response) => {
                    // Reject oversized attachments by Content-Length before buffering the body into
                    // memory (a malicious URL could otherwise OOM the process).
                    if response
                        .content_length()
                        .is_some_and(|len| len > MAX_ATTACHMENT_BYTES)
                    {
                        eprintln!("[global_chat] attachment exceeds size limit, skipped: {url}");
                        None
                    } else {
                        match response.bytes().await {
                            Ok(bytes) if bytes.len() as u64 <= MAX_ATTACHMENT_BYTES => {
                                Some(bytes.to_vec())
                            }
                            Ok(_) => {
                                eprintln!(
                                    "[global_chat] attachment exceeds size limit, skipped: {url}"
                                );
                                None
                            }
                            Err(error) => {
                                eprintln!("[global_chat] failed to read attachment {url}: {error}");
                                None
                            }
                        }
                    }
                }
                Err(error) => {
                    eprintln!("[global_chat] failed to fetch attachment {url}: {error}");
                    None
                }
            }
        } else {
            None
        };

        if let Some(bytes) = bytes {
            images.push(ChatImage {
                data: STANDARD.encode(&bytes),
                media_type: attachment_media_type(url),
            });
        }
    }
    images
}

/// Global FlowPilot assistant chat: a separate platform-level agent loop.
///
/// How long a finished/aborted global-chat run stays resumable after completion, so a client that
/// reloads or reconnects a moment after the turn ended can still replay the full transcript.
const GLOBAL_CHAT_RUN_TTL_SECS: u64 = 120;
const GLOBAL_CHAT_RUN_MAX_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const GLOBAL_CHAT_RUN_MAX_CHUNKS: usize = 8_192;

#[derive(Default)]
struct GlobalChatRunBuffer {
    chunks: Vec<String>,
    bytes: usize,
    truncated: bool,
}

impl GlobalChatRunBuffer {
    fn push(&mut self, chunk: &str) {
        const TRUNCATED_FRAME: &str =
            "\n[FlowPilot resumable stream buffer reached its native retention limit]";
        if self.truncated {
            return;
        }
        if self.chunks.len() >= GLOBAL_CHAT_RUN_MAX_CHUNKS
            || self.bytes.saturating_add(chunk.len()) > GLOBAL_CHAT_RUN_MAX_BUFFER_BYTES
        {
            self.truncated = true;
            let remaining = GLOBAL_CHAT_RUN_MAX_BUFFER_BYTES.saturating_sub(self.bytes);
            if remaining > 0 && self.chunks.len() < GLOBAL_CHAT_RUN_MAX_CHUNKS {
                let notice = utf8_prefix(TRUNCATED_FRAME, remaining).to_string();
                self.bytes = self.bytes.saturating_add(notice.len());
                self.chunks.push(notice);
            }
            return;
        }
        self.bytes = self.bytes.saturating_add(chunk.len());
        self.chunks.push(chunk.to_string());
    }
}

/// A single in-flight (or just-finished) `global_chat` generation, addressable by run id so a
/// reloaded webview can re-attach to it via `global_chat_resume`.
///
/// The webview's JS `Channel` dies on reload, but the Rust generation task keeps running — it just
/// streams into a dead channel. This handle mirrors every emitted chunk into an ordered `buffer`
/// (the replay log) and forwards it to whichever `live` channel is currently attached. On resume we
/// swap `live` to the fresh channel and replay the buffer, so the client rebuilds the whole message
/// from a clean parser. `done` flips true when the turn ends, unblocking waiting resumers.
struct GlobalChatRun {
    buffer: StdMutex<GlobalChatRunBuffer>,
    live: StdMutex<Option<Channel<String>>>,
    done_tx: watch::Sender<bool>,
    done_rx: watch::Receiver<bool>,
}

/// Registry of live global-chat runs, keyed by the assistant message id the frontend generated.
static GLOBAL_CHAT_RUNS: LazyLock<DashMap<String, Arc<GlobalChatRun>>> =
    LazyLock::new(DashMap::new);

/// Register a new run and take ownership of its initial live channel.
fn register_global_chat_run(run_id: &str, live: Channel<String>) -> Arc<GlobalChatRun> {
    let (done_tx, done_rx) = watch::channel(false);
    let run = Arc::new(GlobalChatRun {
        buffer: StdMutex::new(GlobalChatRunBuffer::default()),
        live: StdMutex::new(Some(live)),
        done_tx,
        done_rx,
    });
    GLOBAL_CHAT_RUNS.insert(run_id.to_string(), run.clone());
    run
}

/// A `Channel<String>` whose sends are mirrored into the run (buffer + live forward) instead of
/// going straight to the webview. Passed to the backend in place of the raw JS channel.
fn global_chat_run_channel(run: Arc<GlobalChatRun>) -> Channel<String> {
    Channel::new(move |body: InvokeResponseBody| {
        let chunk = match &body {
            InvokeResponseBody::Json(json) => serde_json::from_str::<String>(json).ok(),
            InvokeResponseBody::Raw(bytes) => String::from_utf8(bytes.clone()).ok(),
        };
        if let Some(chunk) = chunk {
            let mut buffer = run.buffer.lock().unwrap();
            buffer.push(&chunk);
            if let Some(channel) = run.live.lock().unwrap().as_ref() {
                let _ = channel.send(chunk);
            }
        }
        Ok(())
    })
}

/// Mark a run finished and schedule its removal from the registry after the resumable TTL.
fn finish_global_chat_run(run_id: String, run: &Arc<GlobalChatRun>) {
    let _ = run.done_tx.send(true);
    let run = run.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(GLOBAL_CHAT_RUN_TTL_SECS)).await;
        // Only evict if THIS run is still registered — a retry / regeneration of the same message
        // id may have re-registered the run_id meanwhile, and we must not drop that newer run.
        GLOBAL_CHAT_RUNS.remove_if(&run_id, |_, entry| Arc::ptr_eq(entry, &run));
    });
}

#[derive(Serialize)]
pub struct GlobalChatResumeResult {
    /// True when a live/recent run was found and its transcript replayed onto the new channel.
    pub attached: bool,
}

/// Re-attach a reloaded webview to an in-flight (or just-finished) `global_chat` run: swaps the
/// run's live channel to the caller's, replays the full buffer, then blocks until the turn ends so
/// the frontend's awaited invoke resolves exactly like the original send. Returns `attached: false`
/// when no run exists (already GC'd or never registered) — the client then keeps its local
/// checkpoint as-is.
#[tauri::command]
pub async fn global_chat_resume(
    run_id: String,
    channel: Channel<String>,
) -> Result<GlobalChatResumeResult, String> {
    let run = match GLOBAL_CHAT_RUNS.get(&run_id) {
        Some(entry) => entry.value().clone(),
        None => return Ok(GlobalChatResumeResult { attached: false }),
    };

    {
        // Hold the buffer lock across the swap + replay so no concurrent push can interleave: every
        // buffered chunk reaches the new channel in order, and later pushes follow it.
        let buffer = run.buffer.lock().unwrap();
        *run.live.lock().unwrap() = Some(channel.clone());
        for chunk in &buffer.chunks {
            let _ = channel.send(chunk.clone());
        }
    }

    let mut done_rx = run.done_rx.clone();
    if !*done_rx.borrow_and_update() {
        let _ = done_rx.wait_for(|done| *done).await;
    }

    Ok(GlobalChatResumeResult { attached: true })
}

/// Reuses the same backend selection as `copilot_chat` (profile Bits models plus the GitHub Copilot,
/// Codex, and Claude Code agent backends) but injects a platform system prompt, self-awareness
/// context, and the platform tool set instead of board/frontend tools.
#[tauri::command]
pub async fn global_chat(
    app_handle: AppHandle,
    state: State<'_, TauriFlowLikeState>,
    scope: CopilotScope,
    user_prompt: String,
    current_images: Option<Vec<ChatImage>>,
    history: Option<Vec<UnifiedChatMessage>>,
    model_id: Option<String>,
    reasoning_effort: Option<String>,
    token: Option<String>,
    user_context: Option<String>,
    embedding_model_id: Option<String>,
    attachment_urls: Option<Vec<String>>,
    // Every attachment on the current message (name/type/size), including non-image files the model
    // cannot read itself — surfaced so it can hand the relevant ones to apps it calls.
    attachments_manifest: Option<Vec<AttachmentManifestEntry>>,
    board_context: Option<GlobalOpenBoardContext>,
    // The Data Studio page the user currently has open, so the assistant defaults data work to it.
    data_studio_context: Option<GlobalDataStudioContext>,
    // Frontend-generated id (the assistant message id) under which this run is registered so a
    // reloaded webview can re-attach via `global_chat_resume`. `None` disables resumability.
    run_id: Option<String>,
    channel: Channel<String>,
) -> Result<UnifiedCopilotResponse, String> {
    let model_selection = FlowPilotModelSelection::parse(model_id);
    let history = history.unwrap_or_default();
    let attachments_manifest = attachments_manifest.unwrap_or_default();
    let context = build_global_agent_context(
        &app_handle,
        user_context.as_deref(),
        board_context.as_ref(),
        data_studio_context.as_ref(),
        &attachments_manifest,
    )
    .await;

    // Attachments arrive as URLs (local tmp files / presigned uploads, like the simple chat) and
    // are resolved to base64 images here, right before the model call.
    let current_images = {
        let mut images = current_images.unwrap_or_default();
        if let Some(urls) = attachment_urls.as_deref() {
            images.extend(resolve_attachment_images(urls).await);
        }
        (!images.is_empty()).then_some(images)
    };

    let profile = copilot_profile(&app_handle).await;

    // Profile-scoped semantic memory, enabled only when the user selected an embedding model.
    // Shared by every backend so recall and the memory tools behave identically regardless of
    // the selected model.
    let memory = if let (Some(profile_arc), Some(embedding_id)) =
        (&profile, embedding_model_id.as_ref())
    {
        match profile_arc
            .find_bit(embedding_id, state.0.http_client.clone())
            .await
        {
            Ok(bit) => {
                match AssistantMemory::open(state.0.clone(), None, &profile_arc.id, &bit).await {
                    Ok(memory) => Some(Arc::new(memory)),
                    Err(error) => {
                        eprintln!("[global_chat] memory init failed: {error}");
                        None
                    }
                }
            }
            Err(error) => {
                eprintln!("[global_chat] embedding model '{embedding_id}' not found: {error}");
                None
            }
        }
    } else {
        None
    };

    // Register the run (if the frontend gave a run id) and stream through a mirror channel that
    // buffers every chunk + forwards to the live webview channel, so a reload can re-attach and
    // replay via `global_chat_resume`. Without a run id, stream straight to the raw channel.
    let run = run_id
        .as_ref()
        .map(|id| register_global_chat_run(id, channel.clone()));
    let sink = match &run {
        Some(run) => global_chat_run_channel(run.clone()),
        None => channel,
    };
    let source_user_prompt = user_prompt.clone();
    let global_tool_context = FrontendToolContext {
        source_user_prompt: Some(source_user_prompt.clone()),
        ..Default::default()
    };

    let result = async {
        match model_selection.backend {
            FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::GithubCopilot) => {
                let model_id = model_selection
                    .model_id
                    .as_deref()
                    .filter(|model_id| !model_id.trim().is_empty())
                    .ok_or_else(|| "GitHub Copilot backend requires a model id".to_string())?;
                let context = context_with_memory(context, memory.as_ref(), &user_prompt).await;

                copilot_sdk_chat_internal(
                    app_handle.clone(),
                    model_id,
                    reasoning_effort.as_deref(),
                    scope,
                    None,
                    None,
                    &[],
                    None,
                    user_prompt,
                    source_user_prompt.clone(),
                    source_user_prompt.clone(),
                    None,
                    current_images,
                    history,
                    sink,
                    Some(context),
                    memory,
                    Some(global_tool_context.clone()),
                    None,
                    false,
                    false,
                )
                .await
            }
            FlowPilotChatBackend::Agent(agent_backend) => {
                let model_id = model_selection
                    .model_id
                    .clone()
                    .unwrap_or_else(|| "default".to_string());
                let context = context_with_memory(context, memory.as_ref(), &user_prompt).await;

                external_code_agent_chat_internal(
                    app_handle.clone(),
                    agent_backend,
                    &model_id,
                    reasoning_effort.as_deref(),
                    scope,
                    None,
                    None,
                    &[],
                    None,
                    user_prompt,
                    source_user_prompt.clone(),
                    source_user_prompt,
                    None,
                    current_images,
                    history,
                    sink,
                    Some(context),
                    memory,
                    Some(global_tool_context.clone()),
                    None,
                    false,
                    false,
                )
                .await
            }
            FlowPilotChatBackend::Bits => {
                // Profile ("Bits") models are made tool-capable via the same rig machinery the board
                // copilot uses for Bits (get_model + rig agent + manual tool loop), but with the platform
                // tools + global prompt. Platform tools run through the frontend bridge (GLOBAL event).
                // The whole loop lives in core (`run_platform_chat`); the desktop only supplies the
                // Tauri-backed tool bridge and token sink. Memory recall happens inside the loop.
                let (run_cancellation, _run_registration) = register_copilot_run(run_id.as_deref());
                let bridge: Arc<dyn PlatformToolBridge> = Arc::new(DesktopPlatformBridge {
                    bridge: super::frontend_tool_bridge::FrontendToolBridge::new_with_event(
                        app_handle.clone(),
                        super::frontend_tool_bridge::GLOBAL_FRONTEND_TOOL_EVENT,
                    )
                    .with_context(Some(global_tool_context)),
                    tool_set: FrontendPlatformToolSet::Global,
                    cancellation: run_cancellation.clone(),
                });

                let board_history: Vec<flow_like::flow::copilot::ChatMessage> = history
                    .into_iter()
                    .map(|m| flow_like::flow::copilot::ChatMessage {
                        role: m.role,
                        content: m.content,
                        images: m.images,
                    })
                    .collect();

                let on_token = move |token: String| {
                    let _ = sink.send(token);
                };

                let platform_chat = run_platform_chat(
                    state.0.clone(),
                    profile,
                    context,
                    user_prompt,
                    current_images,
                    board_history,
                    model_selection.model_id,
                    token,
                    bridge,
                    memory,
                    Some(on_token),
                );
                let message = tokio::select! {
                    result = platform_chat => result.map_err(|error| error.to_string())?,
                    _ = run_cancellation.cancelled() => {
                        return Err("FlowPilot Bits run was cancelled".to_string());
                    }
                };

                Ok(UnifiedCopilotResponse {
                    message,
                    commands: Vec::new(),
                    suggestions: Vec::new(),
                    components: Vec::new(),
                    canvas_settings: None,
                    root_component_id: None,
                    flowscript_workspace: None,
                    flow_ir_commit: None,
                    active_scope: scope,
                })
            }
        }
    }
    .await;

    // Mark the run finished (unblocking any resumer waiting on completion) and schedule its removal
    // after the resumable TTL. Runs on both the success and error paths so the registry never leaks.
    if let (Some(run_id), Some(run)) = (run_id, run) {
        finish_global_chat_run(run_id, &run);
    }

    result
}

/// Append the shared memory recall/instruction sections to the platform context for the agent
/// backends, whose system prompt is assembled here (the Bits path does the same inside
/// `PlatformCopilot::chat`).
async fn context_with_memory(
    context: String,
    memory: Option<&Arc<AssistantMemory>>,
    user_prompt: &str,
) -> String {
    match memory {
        Some(memory) => format!("{context}{}", memory.prompt_sections(user_prompt).await),
        None => context,
    }
}

/// Stored-memory count for a profile + the embedding model that produced them, so the UI can warn
/// before switching to an incompatible embedding model.
#[tauri::command]
pub async fn global_chat_memory_status(
    state: State<'_, TauriFlowLikeState>,
    profile_id: String,
) -> Result<MemoryStatus, String> {
    AssistantMemory::status(state.0.clone(), None, &profile_id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete all memories for a profile (used when the user switches the embedding model).
#[tauri::command]
pub async fn global_chat_clear_memory(
    state: State<'_, TauriFlowLikeState>,
    profile_id: String,
) -> Result<(), String> {
    AssistantMemory::clear(state.0.clone(), None, &profile_id)
        .await
        .map_err(|e| e.to_string())
}

/// List a profile's saved memories (newest first) so the UI can review and manage them.
#[tauri::command]
pub async fn global_chat_list_memories(
    state: State<'_, TauriFlowLikeState>,
    profile_id: String,
) -> Result<Vec<MemoryEntry>, String> {
    AssistantMemory::list(state.0.clone(), None, &profile_id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete a single saved memory by id.
#[tauri::command]
pub async fn global_chat_delete_memory(
    state: State<'_, TauriFlowLikeState>,
    profile_id: String,
    id: String,
) -> Result<(), String> {
    AssistantMemory::delete_entry(state.0.clone(), None, &profile_id, &id)
        .await
        .map_err(|e| e.to_string())
}

/// Select which shared tool-spec surface validates and authorizes calls before they cross the
/// desktop frontend bridge. Board copilots intentionally use the scoped runtime definitions:
/// their `app_id` is injected by `FrontendToolContext`, whereas the global definitions require the
/// model to provide it explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontendPlatformToolSet {
    Global,
    BoardRuntime,
}

fn frontend_platform_tool_spec(
    tool_set: FrontendPlatformToolSet,
    tool_name: &str,
) -> Option<flow_like::flow::copilot::tool_spec::PlatformToolSpec> {
    use flow_like::flow::copilot::tool_spec::{
        find_global_tool_spec, find_runtime_execution_tool_spec,
    };

    match tool_set {
        FrontendPlatformToolSet::Global => find_global_tool_spec(tool_name),
        FrontendPlatformToolSet::BoardRuntime => find_runtime_execution_tool_spec(tool_name),
    }
}

fn global_orchestrator_tool_scope_error(
    tool_set: FrontendPlatformToolSet,
    tool_name: &str,
) -> Option<String> {
    use flow_like::flow::copilot::tool_spec::{
        ARCHIVE_LOOKUP_TOOL, INTERNET_SEARCH_TOOL, OPEN_URL_TOOL,
    };

    (tool_set != FrontendPlatformToolSet::Global
        && matches!(
            tool_name,
            INTERNET_SEARCH_TOOL | OPEN_URL_TOOL | ARCHIVE_LOOKUP_TOOL
        ))
    .then(|| {
        serde_json::json!({
            "status": "error",
            "code": "global_orchestrator_tool_only",
            "tool": tool_name,
            "message": "Public-web research is available only to the top-level FlowPilot orchestrator."
        })
        .to_string()
    })
}

/// Desktop implementation of the platform tool bridge. Calls are validated and assigned an
/// approval policy from the selected shared spec set, then routed over the configured Tauri event
/// without blocking an async runtime worker.
struct DesktopPlatformBridge {
    bridge: super::frontend_tool_bridge::FrontendToolBridge,
    tool_set: FrontendPlatformToolSet,
    cancellation: CancellationToken,
}

#[async_trait]
impl PlatformToolBridge for DesktopPlatformBridge {
    async fn call(&self, tool_name: &str, arguments: serde_json::Value) -> String {
        use super::copilot_sdk_tools::approval_from_spec;
        use super::frontend_tool_bridge::FrontendToolApproval;
        use flow_like::flow::copilot::tool_spec::missing_required_args;

        // Do not even enqueue a frontend event after the owning model run has ended. Cancellation
        // is checked again inside the blocking bridge scope to close the race after this preflight.
        if self.cancellation.is_cancelled() {
            return serde_json::json!({
                "status": "cancelled",
                "tool": tool_name,
                "message": "The owning FlowPilot run was cancelled before this tool could execute."
            })
            .to_string();
        }
        if let Some(error) = global_orchestrator_tool_scope_error(self.tool_set, tool_name) {
            return error;
        }
        let spec = frontend_platform_tool_spec(self.tool_set, tool_name);

        // Reject calls with missing required arguments before any approval dialog or dispatch,
        // so the model retries with complete arguments (same guard as the SDK/MCP backends).
        if let Some(spec) = &spec
            && let Some(error) = missing_required_args(spec, &arguments)
        {
            return serde_json::json!({ "status": "error", "error": error }).to_string();
        }

        // Approval + timeout come from the shared platform tool spec, so the Bits path enforces
        // exactly the same policy as the Copilot SDK / MCP backends.
        let (approval, timeout) = match spec {
            Some(spec) => (
                approval_from_spec(&spec, &arguments),
                Duration::from_secs(spec.timeout_secs),
            ),
            None => (FrontendToolApproval::none(), Duration::from_secs(120)),
        };

        let bridge = self.bridge.clone();
        let name = tool_name.to_string();
        let cancellation = self.cancellation.clone();
        match tokio::task::spawn_blocking(move || {
            super::frontend_tool_bridge::with_frontend_tool_execution_scope(
                cancellation,
                None,
                || bridge.call_with_timeout(name, arguments, approval, timeout),
            )
        })
        .await
        {
            Ok(value) => serde_json::to_string(&value)
                .unwrap_or_else(|_| "{\"status\":\"error\"}".to_string()),
            Err(err) => {
                serde_json::json!({ "status": "error", "error": err.to_string() }).to_string()
            }
        }
    }
}

const EXTERNAL_AGENT_TOOL_CALL_ID: &str = "external-agent";

/// Result of one external agent CLI run. `error` carries a non-fatal failure (agent error event,
/// non-zero exit) when partial text was still produced, so callers can surface both.
struct ExternalAgentRunOutput {
    text: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalAgentExitKind {
    UserCancelled,
    TransientInfrastructure,
    Permanent,
}

fn classify_external_agent_failure(error: &str, cancelled: bool) -> ExternalAgentExitKind {
    if cancelled {
        return ExternalAgentExitKind::UserCancelled;
    }
    let normalized = error.to_ascii_lowercase();
    let permanent_markers = [
        "cancelled by user",
        "canceled by user",
        "user aborted",
        "authentication",
        "unauthorized",
        "forbidden",
        "invalid api key",
        "permission denied",
        "billing",
        "unsupported",
        "not installed",
        "executable was not found",
        "invalid request",
        "context length",
        "prompt is too long",
        "request too large",
    ];
    if permanent_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentExitKind::Permanent;
    }
    let transient_markers = [
        "connection reset",
        "connection refused",
        "connection closed",
        "connection lost",
        "disconnected",
        "broken pipe",
        "unexpected eof",
        "end of stream",
        "stream closed",
        "transport",
        "timed out",
        "timeout",
        "temporarily unavailable",
        "overloaded",
        "rate limit",
        "network error",
        "dns error",
        "http 429",
        "http 502",
        "http 503",
        "http 504",
        "http 529",
    ];
    if transient_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        ExternalAgentExitKind::TransientInfrastructure
    } else {
        ExternalAgentExitKind::Permanent
    }
}

fn external_agent_run_failure(result: &Result<ExternalAgentRunOutput, String>) -> Option<&str> {
    match result {
        Ok(output) => output.error.as_deref(),
        Err(error) => Some(error.as_str()),
    }
}

fn can_resume_external_workflow_after_failure(
    snapshot: Option<&WorkflowToolLoopSnapshot>,
    error: &str,
    cancelled: bool,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    !snapshot.queued
        && classify_external_agent_failure(error, cancelled)
            == ExternalAgentExitKind::TransientInfrastructure
}

async fn external_code_agent_chat_internal(
    app_handle: AppHandle,
    backend: FlowPilotAgentBackendKind,
    model_id: &str,
    reasoning_effort: Option<&str>,
    scope: CopilotScope,
    board: Option<&Board>,
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: &[String],
    current_surface: Option<&Vec<SurfaceComponent>>,
    user_prompt: String,
    raw_user_prompt: String,
    request_identity_prompt: String,
    host_context_guidance: Option<String>,
    current_images: Option<Vec<ChatImage>>,
    history: Vec<UnifiedChatMessage>,
    channel: Channel<String>,
    global: Option<String>,
    memory: Option<Arc<AssistantMemory>>,
    tool_context: Option<FrontendToolContext>,
    request_id: Option<String>,
    nested: bool,
    read_only: bool,
) -> Result<UnifiedCopilotResponse, String> {
    let live_board = live_board_handle(&app_handle, board);
    let live_board_snapshot = match live_board.as_ref() {
        Some(live_board) => Some(live_board.lock().await.clone()),
        None => None,
    };
    let authoritative_board = live_board_snapshot.as_ref().or(board);
    let mut surface = build_flowpilot_agent_surface(
        scope,
        authoritative_board,
        catalog_nodes,
        selected_node_ids,
        current_surface,
        &history,
        &raw_user_prompt,
        &request_identity_prompt,
        host_context_guidance.as_deref(),
        global.as_deref(),
        read_only,
    );
    surface.live_board = live_board;
    surface.capabilities.tool_protocol = FlowPilotAgentTransportKind::Mcp;
    let _side_effect_cleanup = SideEffectCommandQueueCleanup(surface.side_effect_commands.clone());

    let cli = find_cli_resolution(backend, Some(&app_handle)).ok_or_else(|| {
        format!(
            "{} CLI was not found. Install it or set {} to the executable path.",
            backend.label(),
            backend.env_path_var()
        )
    })?;

    let workflow_edit_request = surface.workflow_edit_request;
    let parent_request_id = scoped_parent_request_id(tool_context.as_ref());
    let (run_cancellation, _run_registration) =
        register_copilot_run(request_id.as_deref().or(parent_request_id.as_deref()));
    // Codex/Claude Code CLI processes are already per-invocation, so no process pool is needed
    // here; the per-board gate alone gives nested runs the same same-board serialization as the
    // SDK and Bits paths (retained draft base-fingerprint integrity). Held for the entire run.
    let _nested_run_permit = if nested {
        Some(
            acquire_nested_copilot_run_permit(
                nested_copilot_run_gate(&nested_copilot_run_gate_key(board, tool_context.as_ref())),
                run_cancellation.clone(),
            )
            .await?,
        )
    } else {
        None
    };
    // Started after the same-board gate so serialized queue time does not consume the budget.
    let nested_wall_clock_deadline = nested.then(|| Instant::now() + NESTED_RUN_WALL_CLOCK_BUDGET);
    let mut tools = build_flowpilot_sdk_tools(
        app_handle,
        scope,
        &surface,
        global.is_some(),
        nested,
        tool_context,
        memory,
        &raw_user_prompt,
    );
    if read_only {
        tools.retain(|(tool, _)| !is_flowpilot_mutation_tool(&tool.name));
    } else if workflow_edit_request {
        // A live FlowScript already contains the graph structure. Hiding legacy/manual discovery
        // tools removes the strongest attractors for code-agent search loops and keeps the exposed
        // MCP surface focused on one declaration batch plus iterative text edits.
        tools.retain(|(tool, _)| {
            !matches!(
                tool.name.as_str(),
                "catalog_search"
                    | "list_board_nodes"
                    | "get_node_details"
                    | "get_unconfigured_nodes"
                    | "emit_commands"
            )
        });
    }
    let tool_names = tools
        .iter()
        .map(|(tool, _)| tool.name.clone())
        .collect::<Vec<_>>();
    let tool_name_summary = tool_names.join(", ");

    send_correlated_stream_json_event(
        &channel,
        "tool_start",
        &serde_json::json!({
            "tool_call_id": EXTERNAL_AGENT_TOOL_CALL_ID,
            "tool": backend.cli_name(),
            "status": "running",
            "summary": format!("Starting {}", backend.label()),
        }),
        parent_request_id.as_deref(),
    );
    send_external_progress_event(
        &channel,
        EXTERNAL_AGENT_TOOL_CALL_ID,
        &format!(
            "Starting {} with shared FlowPilot MCP tools: {}",
            backend.label(),
            tool_name_summary
        ),
        parent_request_id.as_deref(),
    );
    let workflow_state = workflow_edit_request.then(|| {
        Arc::new(StdMutex::new(
            WorkflowToolLoopState::from_flowscript_recovery(surface.flowscript_recovery.as_ref()),
        ))
    });
    let tool_activity = Arc::new(StdMutex::new(McpToolActivityState::default()));
    let mut run_summary = WorkflowRunSummaryEmitter::new(
        channel.clone(),
        parent_request_id.clone(),
        backend.cli_name(),
        model_id,
        run_cancellation.clone(),
    );
    run_summary.attach_workflow_state(workflow_state.clone());
    let mut final_workflow_snapshot = None;
    let mut last_successful_mutation = None;
    let mut continuation = 0u8;
    let mut zero_activity_restarts = 0u8;
    let mut previous_exhausted_budget: Option<String> = None;
    let mut prompt =
        build_external_agent_prompt(&surface.system_content, &user_prompt, workflow_edit_request);
    let agent_result = loop {
        if nested_wall_clock_exhausted(nested_wall_clock_deadline) {
            run_summary.mark_budget_incomplete();
            break Err(nested_wall_clock_incomplete_error(
                final_workflow_snapshot.as_ref(),
                continuation,
            ));
        }
        run_summary.record_phase();
        let phase_start_tool_calls = mcp_total_tool_calls(&tool_activity);
        // A fresh MCP server per provider phase is deliberate. It makes the phase URL an epoch:
        // delayed requests from a killed CLI cannot register as work owned by the next repair.
        let mcp_bridge = match FlowPilotMcpBridge::start(
            tools.clone(),
            workflow_state.clone(),
            tool_activity.clone(),
        )
        .await
        {
            Ok(bridge) => bridge,
            Err(error) => break Err(error),
        };
        let mcp_url = mcp_bridge.url.clone();
        let invocation = match ExternalAgentInvocation::new(
            backend,
            cli.clone(),
            model_id,
            reasoning_effort,
            &mcp_url,
            prompt,
            tool_names.clone(),
            current_images.as_deref().unwrap_or_default(),
        ) {
            Ok(invocation) => invocation,
            Err(error) => {
                let _ = mcp_bridge.finish_phase().await;
                break Err(error);
            }
        };
        send_external_progress_event(
            &channel,
            EXTERNAL_AGENT_TOOL_CALL_ID,
            &format!("Using {} via {}", backend.label(), mcp_url),
            parent_request_id.as_deref(),
        );
        // A nested run's wall-clock deadline cancels only this invocation's child token: the CLI
        // process is killed through the existing forceful-cancellation machinery, while the run
        // itself stays alive to report a graceful, terminal incomplete result below.
        let invocation_cancellation = run_cancellation.child_token();
        let wall_clock_watchdog = nested_wall_clock_deadline.map(|deadline| {
            let cancel_invocation = invocation_cancellation.clone();
            tokio::spawn(async move {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                cancel_invocation.cancel();
            })
        });
        let predraft_checkpoint_fired = Arc::new(AtomicBool::new(false));
        let predraft_checkpoint_watchdog = workflow_state.as_ref().map(|state| {
            let state = state.clone();
            let cancel_invocation = invocation_cancellation.clone();
            let fired = predraft_checkpoint_fired.clone();
            tokio::spawn(async move {
                let mut ready_since: Option<Instant> = None;
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let waiting = match state.lock() {
                        Ok(state) => workflow_waiting_for_initial_source_checkpoint(&state),
                        Err(_) => return,
                    };
                    if !waiting {
                        if ready_since.is_some() {
                            // A source operation started or a draft was retained; the soft
                            // checkpoint did its job and must not interfere with validation.
                            return;
                        }
                        continue;
                    }
                    let started = ready_since.get_or_insert_with(Instant::now);
                    if started.elapsed() >= EXTERNAL_PREDRAFT_SOURCE_CHECKPOINT_BUDGET {
                        fired.store(true, AtomicOrdering::Relaxed);
                        cancel_invocation.cancel();
                        return;
                    }
                }
            })
        });
        let mut run_result = run_external_agent_invocation(
            invocation,
            channel.clone(),
            parent_request_id.clone(),
            invocation_cancellation,
        )
        .await;
        if let Some(watchdog) = wall_clock_watchdog {
            watchdog.abort();
        }
        if let Some(watchdog) = predraft_checkpoint_watchdog {
            watchdog.abort();
        }
        if predraft_checkpoint_fired.load(AtomicOrdering::Relaxed) {
            if let Some(state) = workflow_state.as_ref()
                && let Ok(mut state) = state.lock()
                && !state.flowscript_draft_retained
            {
                state.last_status = Some("declarations_ready_no_source".to_string());
            }
            run_result = Err(format!(
                "FlowPilot pre-draft source checkpoint timed out after {} seconds with usable declarations but no source operation; continue in a fresh bounded phase and call write_flowscript immediately",
                EXTERNAL_PREDRAFT_SOURCE_CHECKPOINT_BUDGET.as_secs()
            ));
        }

        let phase_outcome = match mcp_bridge.finish_phase().await {
            Ok(outcome) => outcome,
            Err(error) => break Err(error),
        };
        final_workflow_snapshot = phase_outcome.workflow_snapshot;
        last_successful_mutation = phase_outcome.last_successful_mutation;
        let queued = final_workflow_snapshot
            .as_ref()
            .is_some_and(|state| state.queued);
        let run_failure = external_agent_run_failure(&run_result).map(str::to_string);
        // A phase that managed to queue its batch before the deadline still returns normally; an
        // externally cancelled run keeps its own terminal reporting.
        if nested_wall_clock_exhausted(nested_wall_clock_deadline)
            && !queued
            && !run_cancellation.is_cancelled()
        {
            run_summary.mark_budget_incomplete();
            break Err(nested_wall_clock_incomplete_error(
                final_workflow_snapshot.as_ref(),
                continuation,
            ));
        }
        if !workflow_edit_request || queued || run_cancellation.is_cancelled() {
            break run_result;
        }
        if run_failure.as_deref().is_some_and(|error| {
            !can_resume_external_workflow_after_failure(
                final_workflow_snapshot.as_ref(),
                error,
                run_cancellation.is_cancelled(),
            )
        }) {
            break run_result;
        }

        let exhausted_budget = final_workflow_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.exhausted_budget.clone());
        if exhausted_budget.is_some() && exhausted_budget == previous_exhausted_budget {
            // The previous continuation already received a fresh bounded slice for this exact
            // budget and burned it again. Another phase would arrive equally dead; stop honestly.
            run_summary.mark_budget_incomplete();
            break Err(external_workflow_incomplete_error(
                final_workflow_snapshot.as_ref(),
                continuation,
            ));
        }

        let phase_tool_calls =
            mcp_total_tool_calls(&tool_activity).saturating_sub(phase_start_tool_calls);
        if run_failure.is_some() && phase_tool_calls == 0 {
            // A transient provider/infrastructure failure before the first tool call did no
            // workflow work. Retry it on its own bounded counter instead of consuming one of the
            // workflow continuations the repair loop needs.
            if zero_activity_restarts >= MAX_EXTERNAL_ZERO_ACTIVITY_RESTARTS {
                break run_result;
            }
            zero_activity_restarts = zero_activity_restarts.saturating_add(1);
        } else {
            if continuation >= MAX_EXTERNAL_WORKFLOW_CONTINUATIONS {
                run_summary.mark_budget_incomplete();
                break Err(external_workflow_incomplete_error(
                    final_workflow_snapshot.as_ref(),
                    continuation,
                ));
            }
            continuation = continuation.saturating_add(1);
            run_summary.record_continuation();
            if let Some(workflow_state) = workflow_state.as_ref()
                && let Ok(mut state) = workflow_state.lock()
            {
                state.grant_continuation_slice();
            }
            previous_exhausted_budget = exhausted_budget;
        }

        if run_failure.is_some() {
            // Give a transient provider/transport failure a moment to clear before restarting the
            // phase, without ignoring an end-to-end cancellation while waiting.
            let cancelled_during_backoff = tokio::select! {
                _ = tokio::time::sleep(EXTERNAL_TRANSIENT_RESTART_BACKOFF) => false,
                _ = run_cancellation.cancelled() => true,
            };
            if cancelled_during_backoff {
                break run_result;
            }
        }

        let mut repair_request = build_external_workflow_continuation_prompt(
            &raw_user_prompt,
            final_workflow_snapshot.as_ref(),
            continuation.max(1),
        );
        if let Some(error) = run_failure.as_deref() {
            let recovery_action = if final_workflow_snapshot.as_ref().is_some_and(|state| {
                state.flowscript_draft_retained && state.last_flowscript.is_some()
            }) {
                "The host retained an exact FlowScript source revision. Continue that draft/revision and do not duplicate a queued commit."
            } else if final_workflow_snapshot
                .as_ref()
                .is_some_and(|state| state.typed_draft_retained)
            {
                "The host retained an exact typed draft revision. Continue that draft/revision and do not start a second mutation path."
            } else {
                "No draft revision was retained. Resume the bounded pre-draft loop from host-retained declaration/read state, then create the first draft with write_flowscript."
            };
            repair_request.push_str(&format!(
                "\n\nINTERNAL TRANSIENT RECOVERY: the previous provider/transport phase ended with `{}`. The host opened a fresh bounded phase. {recovery_action}",
                flow_like::flow::copilot::stream::safe_text_preview(error, 600),
            ));
        }
        prompt = build_external_agent_prompt(&surface.system_content, &repair_request, true);
        send_external_progress_event(
            &channel,
            EXTERNAL_AGENT_TOOL_CALL_ID,
            &format!(
                "{} ended before queueing changes; continuing the bounded workflow run ({continuation}/{MAX_EXTERNAL_WORKFLOW_CONTINUATIONS})",
                backend.label()
            ),
            parent_request_id.as_deref(),
        );
    };

    if run_cancellation.is_cancelled() {
        abandon_side_effect_commands(&surface.side_effect_commands);
    }

    let error_note = match &agent_result {
        Ok(output) => output.error.clone(),
        Err(error) => Some(error.clone()),
    };
    let debug_error_note = error_note
        .as_deref()
        .map(|error| flow_like::flow::copilot::stream::safe_text_preview(error, 1_200));
    send_correlated_stream_json_event(
        &channel,
        "tool_end",
        &serde_json::json!({
            "tool_call_id": EXTERNAL_AGENT_TOOL_CALL_ID,
            "tool": backend.cli_name(),
            "status": if error_note.is_some() { "error" } else { "done" },
            "result_summary": debug_error_note
                .clone()
                .unwrap_or_else(|| format!("{} finished", backend.label())),
            "error": debug_error_note,
        }),
        parent_request_id.as_deref(),
    );
    run_summary.resolve_outcome(error_note.is_some(), workflow_edit_request);

    let has_retained_candidate = final_workflow_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.last_flowscript.as_ref())
        .is_some();
    let agent_output = match agent_result {
        Ok(output) => output,
        Err(error) if last_successful_mutation.is_some() => ExternalAgentRunOutput {
            text: render_recovered_mutation_message(
                last_successful_mutation
                    .as_ref()
                    .expect("guarded by is_some"),
            ),
            error: Some(error),
        },
        Err(error) if workflow_edit_request && has_retained_candidate => ExternalAgentRunOutput {
            text: String::new(),
            error: Some(error),
        },
        Err(error) => return Err(error),
    };
    let text = agent_output.text.trim().to_string();
    let message = match (agent_output.error, text.is_empty()) {
        (Some(error), true) if has_retained_candidate => format!(
            "{} retained the most complete FlowScript draft for repair, but did not queue it because validation is still failing: {error}",
            backend.label()
        ),
        (Some(error), true) => return Err(format!("{} failed: {error}", backend.label())),
        (Some(error), false) => format!(
            "{text}\n\n> Note: {} ended with an error after this partial response: {error}",
            backend.label()
        ),
        (None, true) => format!(
            "{} completed without a final text response.",
            backend.label()
        ),
        (None, false) => text,
    };
    let message = if final_workflow_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.modular_fallback.as_ref())
        .is_some()
    {
        "Queued an independently runnable partial working slice for review. The requested application is still incomplete; the fuller failed FlowScript remains retained for another repair pass. Do not treat this as full completion."
            .to_string()
    } else {
        message
    };

    // `emit_ui` results are invisible to the MCP transport, so rendered surfaces are drained from
    // the shared store — the LAST successful emit wins, matching the SDK path's extraction.
    let emitted_surface = surface
        .emitted_surfaces
        .lock()
        .ok()
        .and_then(|mut surfaces| surfaces.drain(..).last());
    let (components, canvas_settings, root_component_id) = match emitted_surface {
        Some(emitted) => (
            serde_json::from_value::<Vec<SurfaceComponent>>(emitted.components).unwrap_or_default(),
            Some(emitted.canvas_settings),
            Some(emitted.root_component_id),
        ),
        None => (Vec::new(), None, None),
    };
    if !components.is_empty() {
        let comp_event = format!(
            "<components>{}</components>",
            serde_json::to_string(&components).unwrap_or_default()
        );
        let _ = channel.send(comp_event);
        if let Some(canvas) = &canvas_settings {
            let canvas_event = format!(
                "<canvas_settings>{}</canvas_settings>",
                serde_json::to_string(canvas).unwrap_or_default()
            );
            let _ = channel.send(canvas_event);
        }
    }

    let queued_workspace = surface
        .queued_flowscript
        .lock()
        .ok()
        .and_then(|workspace| workspace.clone());
    let flowscript_workspace = queued_workspace
        .as_deref()
        .map(|source| {
            flowscript_response_workspace_envelope(
                source,
                "queued",
                final_workflow_snapshot.as_ref(),
            )
        })
        .or_else(|| {
            final_workflow_snapshot.as_ref().and_then(|snapshot| {
                snapshot.last_flowscript.as_deref().map(|source| {
                    flowscript_workspace_envelope(
                        source,
                        snapshot
                            .last_status
                            .as_deref()
                            .unwrap_or("validation_errors"),
                    )
                })
            })
        });

    let (commands, flow_ir_commit) = take_side_effect_delivery(&surface.side_effect_commands);
    run_summary.set_applied_commands(commands.len());
    Ok(UnifiedCopilotResponse {
        message,
        commands,
        suggestions: Vec::new(),
        components,
        canvas_settings,
        root_component_id,
        flowscript_workspace,
        flow_ir_commit,
        active_scope: scope,
    })
}

/// Internal function to handle Copilot SDK chat
async fn copilot_sdk_chat_internal(
    app_handle: AppHandle,
    model_id: &str,
    reasoning_effort: Option<&str>,
    scope: CopilotScope,
    board: Option<&Board>,
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: &[String],
    current_surface: Option<&Vec<SurfaceComponent>>,
    user_prompt: String,
    raw_user_prompt: String,
    request_identity_prompt: String,
    host_context_guidance: Option<String>,
    current_images: Option<Vec<ChatImage>>,
    history: Vec<UnifiedChatMessage>,
    channel: Channel<String>,
    global: Option<String>,
    memory: Option<Arc<AssistantMemory>>,
    tool_context: Option<FrontendToolContext>,
    request_id: Option<String>,
    nested: bool,
    read_only: bool,
) -> Result<UnifiedCopilotResponse, String> {
    use copilot_sdk::SessionEventData;

    const MAX_WORKFLOW_IDLE_CONTINUATIONS: u8 = 2;

    let parent_request_id = scoped_parent_request_id(tool_context.as_ref());
    let nested_gate_key = nested_copilot_run_gate_key(board, tool_context.as_ref());
    let (run_cancellation, _run_registration) =
        register_copilot_run(request_id.as_deref().or(parent_request_id.as_deref()));

    let live_board = live_board_handle(&app_handle, board);
    let live_board_snapshot = match live_board.as_ref() {
        Some(live_board) => Some(live_board.lock().await.clone()),
        None => None,
    };
    let authoritative_board = live_board_snapshot.as_ref().or(board);
    let mut surface = build_flowpilot_agent_surface(
        scope,
        authoritative_board,
        catalog_nodes,
        selected_node_ids,
        current_surface,
        &history,
        &raw_user_prompt,
        &request_identity_prompt,
        host_context_guidance.as_deref(),
        global.as_deref(),
        read_only,
    );
    surface.live_board = live_board;
    let side_effect_commands = surface.side_effect_commands.clone();
    let _side_effect_cleanup = SideEffectCommandQueueCleanup(side_effect_commands.clone());
    let queued_flowscript = surface.queued_flowscript.clone();
    let emitted_surfaces = surface.emitted_surfaces.clone();
    let workflow_edit_request = surface.workflow_edit_request;
    let workflow_state = workflow_edit_request.then(|| {
        Arc::new(StdMutex::new(
            WorkflowToolLoopState::from_flowscript_recovery(surface.flowscript_recovery.as_ref()),
        ))
    });
    let mut run_summary = WorkflowRunSummaryEmitter::new(
        channel.clone(),
        parent_request_id.clone(),
        "github-copilot",
        model_id,
        run_cancellation.clone(),
    );
    run_summary.set_continuation_limit(u32::from(MAX_WORKFLOW_IDLE_CONTINUATIONS));
    run_summary.attach_workflow_state(workflow_state.clone());
    run_summary.record_phase();

    let mut tools = build_flowpilot_sdk_tools(
        app_handle,
        scope,
        &surface,
        global.is_some(),
        nested,
        tool_context,
        memory,
        &raw_user_prompt,
    );
    if read_only {
        tools.retain(|(tool, _)| !is_flowpilot_mutation_tool(&tool.name));
    } else if workflow_edit_request {
        tools.retain(|(tool, _)| {
            !matches!(
                tool.name.as_str(),
                "catalog_search"
                    | "list_board_nodes"
                    | "get_node_details"
                    | "get_unconfigured_nodes"
                    | "emit_commands"
            )
        });
        tools = guard_sdk_workflow_tools(
            tools,
            workflow_state
                .as_ref()
                .expect("workflow state exists for mutation sessions")
                .clone(),
        );
    }
    tools = scope_sdk_tool_handlers(tools, run_cancellation.clone());

    // Extract just the Tool definitions for SessionConfig
    let tool_defs: Vec<copilot_sdk::Tool> = tools.iter().map(|(t, _)| t.clone()).collect();

    // Names of our reviewed custom tools. The CLI may surface a permission request for these
    // before running them; we approve those and deny everything else (built-in file/shell tools).
    let allowed_tool_names: std::collections::HashSet<String> =
        tool_defs.iter().map(|t| t.name.clone()).collect();
    let available_tools = Some(allowed_tool_names.iter().cloned().collect::<Vec<_>>());
    let permission_allowed_tool_names = allowed_tool_names.clone();

    // Whitelist reviewed custom tools and also exclude known built-ins as a defense in depth.
    // This keeps FlowPilot in its virtual workflow/UI workspace and prevents file/shell draft
    // attempts from surfacing as permission errors.
    let excluded_tools = Some(vec![
        "Read".to_string(),
        "Edit".to_string(),
        "Write".to_string(),
        "Glob".to_string(),
        "LS".to_string(),
        "Task".to_string(),
        "WebFetch".to_string(),
        "WebSearch".to_string(),
        "NotebookEdit".to_string(),
        "shell".to_string(),
        "powershell".to_string(),
        "bash".to_string(),
        "Grep".to_string(),
        "listDir".to_string(),
        "list_dir".to_string(),
        "read_file".to_string(),
        "write_file".to_string(),
        "edit_file".to_string(),
        "create_file".to_string(),
        "Search".to_string(),
        "Insert".to_string(),
        "Replace".to_string(),
        "CreateFile".to_string(),
    ]);

    let config = copilot_sdk::SessionConfig {
        model: Some(model_id.to_string()),
        reasoning_effort: explicit_reasoning_effort(reasoning_effort).map(str::to_string),
        streaming: true,
        tools: tool_defs,
        available_tools,
        excluded_tools,
        request_permission: Some(true),
        system_message: Some(copilot_sdk::SystemMessageConfig {
            content: Some(surface.system_content),
            mode: Some(copilot_sdk::SystemMessageMode::Replace),
        }),
        infinite_sessions: Some(copilot_sdk::InfiniteSessionConfig::enabled()),
        ..Default::default()
    };

    flowpilot_debug_log!(
        "[copilot_sdk_chat] start (model: {model_id}, global: {}, nested: {nested}, tools: {})",
        global.is_some(),
        allowed_tool_names.len()
    );

    // Same-board nested runs must not interleave (retained draft base-fingerprint integrity).
    // Keep the per-board permit for the entire run. Queueing behind the current owner has no
    // arbitrary timeout, but explicit cancellation wins.
    let nested_run_permit = if nested {
        Some(
            acquire_nested_copilot_run_permit(
                nested_copilot_run_gate(&nested_gate_key),
                run_cancellation.clone(),
            )
            .await?,
        )
    } else {
        None
    };

    // A nested run checks a dedicated CLI process out of the pool (exclusive ownership keeps the
    // per-process one-request constraint). The main client slot is cloned before awaiting any RPC:
    // a wedged create_session must not hold the global mutex and block stop/status/recovery calls.
    let nested_client_lease = if nested {
        Some(checkout_nested_copilot_client(run_cancellation.clone()).await?)
    } else {
        None
    };
    let client = match nested_client_lease.as_ref() {
        Some(lease) => lease.client(),
        None => COPILOT_CLIENT
            .lock()
            .await
            .clone()
            .ok_or("Copilot SDK not running. Please start it first.")?,
    };

    let create_session = client.create_session(config);
    let session_result = tokio::select! {
        result = tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, create_session) => {
            let result = result.map_err(|_| format!(
                "{} Copilot session creation exceeded {} seconds",
                if nested { "Nested" } else { "GitHub" },
                SDK_CONTROL_RPC_TIMEOUT.as_secs(),
            ));
            result.and_then(|result| result.map_err(|error| {
                if nested {
                    format!("Failed to create nested session: {error}")
                } else {
                    format!("Failed to create session: {error}")
                }
            }))
        }
        _ = run_cancellation.cancelled() => {
            Err("FlowPilot Copilot run was cancelled during session creation".to_string())
        }
    };
    let session = match session_result {
        Ok(session) => session,
        Err(error) => {
            if nested {
                quarantine_nested_copilot_client(&client).await;
            }
            return Err(error);
        }
    };
    struct CopilotSessionCleanup {
        client: Arc<Client>,
        session: Arc<copilot_sdk::Session>,
        session_id: String,
        nested_run_permit: Option<tokio::sync::OwnedSemaphorePermit>,
        nested_client_lease: Option<NestedCopilotClientLease>,
    }
    impl Drop for CopilotSessionCleanup {
        fn drop(&mut self) {
            let client = self.client.clone();
            let session = self.session.clone();
            let session_id = self.session_id.clone();
            let nested_run_permit = self.nested_run_permit.take();
            let nested_client_lease = self.nested_client_lease.take();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _nested_run_permit = nested_run_permit;
                    // Held past session deletion: a pooled client may only rejoin the idle pool
                    // once its previous session is gone, or the next checkout could deadlock the
                    // CLI process with a second concurrent session.
                    let _nested_client_lease = nested_client_lease;
                    let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, session.abort()).await;
                    // Client::delete_session also evicts the SDK's local Arc<Session> cache.
                    let _ = tokio::time::timeout(
                        SDK_CHAT_ABORT_TIMEOUT,
                        client.delete_session(&session_id),
                    )
                    .await;
                });
            } else if let Some(lease) = nested_client_lease {
                // Without a runtime the pending session cannot be cleaned up; drop the process
                // from the pool instead of re-pooling a client with an undeleted session.
                lease.deregister();
            }
        }
    }
    // The SDK client keeps every session in its internal map until destroy succeeds. Ensure every
    // return path (including cancellation and parser errors) gets bounded best-effort cleanup.
    let _session_cleanup = CopilotSessionCleanup {
        client: client.clone(),
        session: session.clone(),
        session_id: session.session_id().to_string(),
        nested_run_permit,
        nested_client_lease,
    };
    if nested {
        flowpilot_debug_log!("[copilot_sdk_chat] creating session on the nested CLI");
    } else {
        flowpilot_debug_log!("[copilot_sdk_chat] client lock acquired; creating session");
    }
    flowpilot_debug_log!(
        "[copilot_sdk_chat] session {} created",
        session.session_id()
    );
    // Register tool handlers
    for (tool, handler) in tools {
        tokio::select! {
            _ = session.register_tool_with_handler(tool, Some(handler)) => {}
            _ = run_cancellation.cancelled() => {
                return Err("FlowPilot Copilot run was cancelled while registering tools".to_string());
            }
            _ = tokio::time::sleep(SDK_CONTROL_RPC_TIMEOUT) => {
                return Err(format!(
                    "FlowPilot Copilot tool registration exceeded {} seconds",
                    SDK_CONTROL_RPC_TIMEOUT.as_secs(),
                ));
            }
        }
    }

    // FlowPilot only exposes reviewed custom tools. Approve permission requests for those
    // tools (the CLI surfaces one before invoking them) and deny anything else so built-in
    // file/shell tools cannot run.
    let register_permission = session.register_permission_handler(move |req| {
        let tool_name = req.extension_data.get("toolName").and_then(|v| v.as_str());
        match tool_name {
            Some(name) if permission_allowed_tool_names.contains(name) => {
                copilot_sdk::PermissionRequestResult::approved()
            }
            _ => copilot_sdk::PermissionRequestResult::denied(),
        }
    });
    tokio::select! {
        _ = register_permission => {}
        _ = run_cancellation.cancelled() => {
            return Err("FlowPilot Copilot run was cancelled while configuring permissions".to_string());
        }
        _ = tokio::time::sleep(SDK_CONTROL_RPC_TIMEOUT) => {
            return Err(format!(
                "FlowPilot Copilot permission registration exceeded {} seconds",
                SDK_CONTROL_RPC_TIMEOUT.as_secs(),
            ));
        }
    }

    let mut events = session.subscribe();
    let attachments = current_images
        .as_ref()
        .filter(|images| !images.is_empty())
        .map(|images| build_copilot_attachments(images))
        .transpose()?;

    let send_message = session.send(MessageOptions {
        prompt: user_prompt,
        attachments,
        mode: None,
    });
    tokio::select! {
        result = send_message => result.map_err(|e| format!("Failed to send message: {e}"))?,
        _ = run_cancellation.cancelled() => {
            return Err("FlowPilot Copilot run was cancelled while sending its prompt".to_string());
        }
        _ = tokio::time::sleep(SDK_CONTROL_RPC_TIMEOUT) => {
            return Err(format!(
                "FlowPilot Copilot prompt delivery exceeded {} seconds",
                SDK_CONTROL_RPC_TIMEOUT.as_secs(),
            ));
        }
    };
    flowpilot_debug_log!(
        "[copilot_sdk_chat] prompt sent on session {}; streaming events",
        session.session_id()
    );

    let mut full_response = String::new();
    let mut extracted_commands: Vec<BoardCommand> = Vec::new();
    let mut extracted_components: Vec<SurfaceComponent> = Vec::new();
    let mut extracted_canvas_settings: Option<serde_json::Value> = None;
    let mut extracted_root_component_id: Option<String> = None;
    let mut extracted_flowscript_workspace: Option<String> = None;
    let mut last_validated_commands: Option<Vec<BoardCommand>> = None;
    let mut last_validated_components: Option<(
        Vec<SurfaceComponent>,
        Option<serde_json::Value>,
        Option<String>,
    )> = None;
    let mut workflow_idle_continuations = 0u8;
    // Budget name that already received a bounded continuation slice, mirroring the external
    // phase loop: granting the same exhausted budget a second slice would only loop.
    let mut previous_idle_exhausted_budget: Option<String> = None;
    let mut tool_names_by_call_id: HashMap<String, String> = HashMap::new();
    let mut open_tool_call_ids: HashSet<String> = HashSet::new();
    let mut session_error_note: Option<String> = None;
    // Most recent mutating tool call that failed validation: (tool name, errors). Cleared when a
    // later call queues/renders. Feeds the idle-continuation nudge so a model that stops after a
    // failed edit gets told exactly what to fix instead of a generic "try again".
    let mut last_validation_errors: Option<(String, Vec<String>)> = None;
    // Token usage the SDK reports per turn (assistant.usage) — accumulated into one usage_stat frame
    // so the chat shows the agent's own model usage (mirrors the Bits/rig path in platform.rs).
    let mut usage_prompt_tokens: u64 = 0;
    let mut usage_completion_tokens: u64 = 0;
    let mut usage_cost: f64 = 0.0;
    let mut usage_has_cost = false;
    let mut usage_model: Option<String> = None;
    let mut usage_calls: Vec<serde_json::Value> = Vec::new();

    loop {
        let next_event = tokio::select! {
            result = events.recv() => result,
            _ = run_cancellation.cancelled() => {
                let note = "the FlowPilot Copilot run was cancelled";
                let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, session.abort()).await;
                if nested {
                    quarantine_nested_copilot_client(&client).await;
                }
                close_pending_tool_steps(
                    &channel,
                    &mut open_tool_call_ids,
                    &tool_names_by_call_id,
                    "error",
                    Some(note),
                    parent_request_id.as_deref(),
                );
                if full_response.trim().is_empty()
                    && extracted_commands.is_empty()
                    && extracted_components.is_empty()
                    && extracted_flowscript_workspace.is_none()
                {
                    return Err("FlowPilot Copilot run was cancelled".to_string());
                }
                session_error_note = Some(note.to_string());
                break;
            }
            _ = tokio::time::sleep(SDK_EVENT_INACTIVITY_TIMEOUT) => {
                let note = format!(
                    "the Copilot SDK event stream produced no activity for {} seconds",
                    SDK_EVENT_INACTIVITY_TIMEOUT.as_secs()
                );
                let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, session.abort()).await;
                if nested {
                    quarantine_nested_copilot_client(&client).await;
                }
                close_pending_tool_steps(
                    &channel,
                    &mut open_tool_call_ids,
                    &tool_names_by_call_id,
                    "error",
                    Some(&note),
                    parent_request_id.as_deref(),
                );
                if full_response.trim().is_empty()
                    && extracted_commands.is_empty()
                    && extracted_components.is_empty()
                    && extracted_flowscript_workspace.is_none()
                    && !workflow_state_has_retained_candidate(workflow_state.as_ref())
                {
                    return Err(format!("FlowPilot Copilot session timed out: {note}"));
                }
                // Preserve any exact retained draft/diagnostics so the bounded outer workflow
                // continuation can resume them instead of pretending the timed-out phase queued.
                session_error_note = Some(note);
                break;
            }
        };
        match next_event {
            Ok(event) => match &event.data {
                SessionEventData::AssistantMessageDelta(delta) => {
                    append_bounded_text(
                        &mut full_response,
                        &delta.delta_content,
                        SDK_RESPONSE_MAX_BYTES,
                    );
                    if !workflow_edit_request {
                        let _ = channel.send(delta.delta_content.clone());
                    }
                }
                SessionEventData::AssistantMessage(msg) => {
                    // Don't overwrite accumulated content unless it's truly final
                    if full_response.is_empty() {
                        append_bounded_text(
                            &mut full_response,
                            &msg.content,
                            SDK_RESPONSE_MAX_BYTES,
                        );
                    }
                }
                SessionEventData::AssistantUsage(data) => {
                    let input = data.input_tokens.unwrap_or(0.0).max(0.0).round() as u64;
                    let output = data.output_tokens.unwrap_or(0.0).max(0.0).round() as u64;
                    if input > 0 || output > 0 {
                        usage_prompt_tokens += input;
                        usage_completion_tokens += output;
                        if let Some(cost) = data.cost {
                            usage_cost += cost;
                            usage_has_cost = true;
                        }
                        if data.model.is_some() {
                            usage_model = data.model.clone();
                        }
                        if usage_calls.len() < SDK_USAGE_CALLS_MAX_ENTRIES {
                            usage_calls.push(serde_json::json!({
                                "model": data.model.clone().unwrap_or_default(),
                                "usage": {
                                    "prompt_tokens": input,
                                    "completion_tokens": output,
                                    "total_tokens": input + output,
                                    "cost": data.cost,
                                },
                            }));
                        }
                    }
                }
                SessionEventData::ToolExecutionStart(tool_event) => {
                    let newly_announced = tool_names_by_call_id
                        .insert(
                            tool_event.tool_call_id.clone(),
                            tool_event.tool_name.clone(),
                        )
                        .is_none();
                    open_tool_call_ids.insert(tool_event.tool_call_id.clone());
                    // The same call may already have been announced via the protocol v3
                    // external_tool.requested broadcast — don't emit a second tool_start.
                    if newly_announced {
                        announce_tool_start(
                            &channel,
                            &tool_event.tool_call_id,
                            &tool_event.tool_name,
                            tool_event.arguments.as_ref(),
                            &mut extracted_flowscript_workspace,
                            parent_request_id.as_deref(),
                        );
                    }
                }
                SessionEventData::ExternalToolRequested(request) => {
                    // Protocol v3 broadcasts custom tool calls as external_tool.requested and may
                    // never emit tool.execution_start for them — announce the step here so custom
                    // FlowPilot tools stream reliably.
                    let Some(tool_call_id) =
                        request.tool_call_id.clone().filter(|id| !id.is_empty())
                    else {
                        continue;
                    };
                    if tool_names_by_call_id.contains_key(&tool_call_id) {
                        continue;
                    }
                    let tool_name = request
                        .tool_name
                        .clone()
                        .unwrap_or_else(|| "tool".to_string());
                    tool_names_by_call_id.insert(tool_call_id.clone(), tool_name.clone());
                    open_tool_call_ids.insert(tool_call_id.clone());
                    announce_tool_start(
                        &channel,
                        &tool_call_id,
                        &tool_name,
                        request.arguments.as_ref(),
                        &mut extracted_flowscript_workspace,
                        parent_request_id.as_deref(),
                    );
                }
                SessionEventData::ToolExecutionProgress(progress) => {
                    send_correlated_stream_json_event(
                        &channel,
                        "tool_progress",
                        &serde_json::json!({
                            "tool_call_id": progress.tool_call_id,
                            "tool": tool_names_by_call_id.get(&progress.tool_call_id),
                            "message": flow_like::flow::copilot::stream::safe_text_preview(&progress.progress_message, 1_200),
                        }),
                        parent_request_id.as_deref(),
                    );
                }
                SessionEventData::ToolExecutionPartialResult(partial) => {
                    send_correlated_stream_json_event(
                        &channel,
                        "tool_progress",
                        &serde_json::json!({
                            "tool_call_id": partial.tool_call_id,
                            "tool": tool_names_by_call_id.get(&partial.tool_call_id),
                            "message": flow_like::flow::copilot::stream::safe_text_preview(&partial.partial_output, 1_200),
                        }),
                        parent_request_id.as_deref(),
                    );
                }
                SessionEventData::ToolExecutionComplete(tool_complete) => {
                    open_tool_call_ids.remove(&tool_complete.tool_call_id);
                    let completed_tool_name = tool_names_by_call_id
                        .remove(&tool_complete.tool_call_id)
                        .or_else(|| tool_complete.mcp_tool_name.clone())
                        .unwrap_or_else(|| "tool".to_string());
                    let result_content = tool_complete
                        .result
                        .as_ref()
                        .map(|result| result.content.as_str());

                    if let Some(ref result) = tool_complete.result
                        && let Ok(parsed) =
                            serde_json::from_str::<serde_json::Value>(&result.content)
                    {
                        let status = parsed.get("status").and_then(|s| s.as_str());

                        if let Some(mut payload) = flowscript_workspace_result_payload(
                            &completed_tool_name,
                            &parsed,
                            extracted_flowscript_workspace.as_deref(),
                        ) {
                            if let Some(object) = payload.as_object_mut() {
                                object.insert(
                                    "tool_call_id".to_string(),
                                    serde_json::Value::String(
                                        tool_complete.tool_call_id.to_string(),
                                    ),
                                );
                            }
                            if let Some(workspace) =
                                payload.get("source").and_then(serde_json::Value::as_str)
                            {
                                extracted_flowscript_workspace = Some(workspace.to_string());
                            }
                            send_stream_json_event(&channel, "flowscript_workspace", &payload);
                        }

                        // Some models, especially Claude/Sonnet variants, stop after a
                        // successful validate_* call. Remember valid payloads so idle
                        // handling can still surface the reviewable action to the board.
                        if status == Some("valid") {
                            if let Some(cmds) = parsed.get("commands")
                                && let Ok(commands) =
                                    serde_json::from_value::<Vec<BoardCommand>>(cmds.clone())
                            {
                                last_validated_commands = Some(commands);
                            }

                            if let Some(comps) = parsed.get("components")
                                && let Ok(components) =
                                    serde_json::from_value::<Vec<SurfaceComponent>>(comps.clone())
                            {
                                let canvas = parsed.get("canvasSettings").cloned();
                                let root_id = parsed
                                    .get("rootComponentId")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string);
                                last_validated_components = Some((components, canvas, root_id));
                            }
                        } else if status == Some("validation_errors") {
                            if parsed.get("commands").is_some() {
                                last_validated_commands = None;
                            }
                            if parsed.get("components").is_some() {
                                last_validated_components = None;
                            }
                        }

                        // Track raw and typed validation outcomes so idle handling can nudge with
                        // exact structured diagnostics instead of a representation-specific retry.
                        let diagnostics = workflow_result_diagnostics(Some(&parsed));
                        if workflow_result_requires_repair(&parsed, &diagnostics) {
                            let errors = if diagnostics.is_empty() {
                                workflow_result_fallback_message(&parsed)
                                    .into_iter()
                                    .collect()
                            } else {
                                diagnostics
                            };
                            last_validation_errors = Some((completed_tool_name.clone(), errors));
                        } else if workflow_result_clears_repair(&parsed) {
                            last_validation_errors = None;
                        }

                        // Queued board commands travel via the side-effect store. Direct/legacy
                        // batches can stream immediately; retained FlowScript batches stay in the
                        // queue until commands and their exact review token can be taken together.
                        if status == Some("queued") {
                            let commands =
                                drain_streamable_side_effect_commands(&side_effect_commands);
                            if !commands.is_empty() {
                                send_commands_event(&channel, &commands);
                                extracted_commands.extend(commands);
                                last_validated_commands = None;
                            }
                        }
                        // Rendered UI travels via the emitted-surfaces store (tool results no
                        // longer echo the tree). Drain the newest surface; keep the legacy
                        // result-echo parse as a fallback.
                        if status == Some("rendered") {
                            let emitted = emitted_surfaces
                                .lock()
                                .ok()
                                .and_then(|mut surfaces| surfaces.drain(..).last());
                            let (components, canvas, root_id) = match emitted {
                                Some(surface) => (
                                    serde_json::from_value::<Vec<SurfaceComponent>>(
                                        surface.components,
                                    )
                                    .unwrap_or_default(),
                                    Some(surface.canvas_settings),
                                    Some(surface.root_component_id),
                                ),
                                None => (
                                    parsed
                                        .get("components")
                                        .cloned()
                                        .and_then(|comps| {
                                            serde_json::from_value::<Vec<SurfaceComponent>>(comps)
                                                .ok()
                                        })
                                        .unwrap_or_default(),
                                    parsed.get("canvasSettings").cloned(),
                                    parsed
                                        .get("rootComponentId")
                                        .and_then(|v| v.as_str())
                                        .map(str::to_string),
                                ),
                            };
                            if let Some(canvas) = canvas {
                                extracted_canvas_settings = Some(canvas);
                            }
                            if let Some(root_id) = root_id {
                                extracted_root_component_id = Some(root_id);
                            }
                            if !components.is_empty() {
                                let comp_event = format!(
                                    "<components>{}</components>",
                                    serde_json::to_string(&components).unwrap_or_default()
                                );
                                let _ = channel.send(comp_event);
                                if let Some(ref canvas) = extracted_canvas_settings {
                                    let canvas_event = format!(
                                        "<canvas_settings>{}</canvas_settings>",
                                        serde_json::to_string(canvas).unwrap_or_default()
                                    );
                                    let _ = channel.send(canvas_event);
                                }
                                extracted_components.extend(components);
                                last_validated_components = None;
                            }
                        }
                    }

                    // Send tool completion event to frontend
                    let terminal_status = result_content.and_then(extract_json_status);
                    let status = if !tool_complete.success {
                        "error"
                    } else {
                        result_content
                            .map(direct_sdk_tool_result_stream_status)
                            .unwrap_or("done")
                    };
                    let error_message = tool_complete.error.as_ref().map(|error| {
                        if error.message.is_empty() {
                            "Tool failed".to_string()
                        } else {
                            flow_like::flow::copilot::stream::safe_text_preview(&error.message, 600)
                        }
                    });
                    send_correlated_stream_json_event(
                        &channel,
                        "tool_end",
                        &serde_json::json!({
                            "tool_call_id": tool_complete.tool_call_id,
                            "tool": completed_tool_name,
                            "status": status,
                            "terminal_status": terminal_status,
                            // Kept for older clients while the detailed report uses terminal_status.
                            "result_status": terminal_status,
                            "result_summary": summarize_tool_result(result_content, error_message.as_deref()),
                            "result_preview": result_content.map(|content| preview_tool_result(content)),
                            "error": error_message,
                        }),
                        parent_request_id.as_deref(),
                    );
                }
                SessionEventData::SessionIdle(_) => {
                    // v3 external tool calls may never get a tool.execution_complete event —
                    // close any still-open steps so the frontend doesn't keep spinners alive.
                    close_pending_tool_steps(
                        &channel,
                        &mut open_tool_call_ids,
                        &tool_names_by_call_id,
                        "done",
                        None,
                        parent_request_id.as_deref(),
                    );

                    if extracted_commands.is_empty()
                        && let Some(commands) = last_validated_commands.take()
                    {
                        send_commands_event(&channel, &commands);
                        extracted_commands.extend(commands);
                    }

                    if extracted_commands.is_empty() {
                        let commands = drain_streamable_side_effect_commands(&side_effect_commands);
                        if !commands.is_empty() {
                            send_commands_event(&channel, &commands);
                            extracted_commands.extend(commands);
                        }
                    }

                    if extracted_components.is_empty()
                        && let Some((components, canvas_settings, root_component_id)) =
                            last_validated_components.take()
                    {
                        let comp_event = format!(
                            "<components>{}</components>",
                            serde_json::to_string(&components).unwrap_or_default()
                        );
                        let _ = channel.send(comp_event);

                        if let Some(canvas) = canvas_settings {
                            let canvas_event = format!(
                                "<canvas_settings>{}</canvas_settings>",
                                serde_json::to_string(&canvas).unwrap_or_default()
                            );
                            let _ = channel.send(canvas_event);
                            extracted_canvas_settings = Some(canvas);
                        }

                        extracted_root_component_id = root_component_id;
                        extracted_components = components;
                    }

                    // Nudge the model to finish when it stalled mid-task: either a workflow-edit
                    // request that queued nothing, or ANY scope whose last mutating call failed
                    // validation and produced no successful follow-up (models often stop right
                    // after a failed edit_flowscript/emit_ui instead of fixing and retrying).
                    let failed_attempt_pending = last_validation_errors.is_some()
                        && extracted_commands.is_empty()
                        && extracted_components.is_empty();
                    let workflow_mutation_is_terminal = workflow_state
                        .as_ref()
                        .and_then(|state| state.lock().ok())
                        .is_some_and(|state| state.queued);
                    if ((workflow_edit_request
                        && extracted_commands.is_empty()
                        && !workflow_mutation_is_terminal)
                        || failed_attempt_pending)
                        && workflow_idle_continuations < MAX_WORKFLOW_IDLE_CONTINUATIONS
                    {
                        // A continuation that demands more edits must be executable on arrival:
                        // grant an exhausted loop budget the same bounded slice the external
                        // phase loop grants, and stop honestly when that exact budget was already
                        // granted one and burned it again.
                        match prepare_sdk_idle_continuation_budget(
                            workflow_state.as_ref(),
                            previous_idle_exhausted_budget.as_deref(),
                        ) {
                            IdleContinuationBudget::Terminal(reason) => {
                                session_error_note =
                                    Some(format!("stopped without queueing changes: {reason}"));
                                break;
                            }
                            IdleContinuationBudget::SliceGranted(budget) => {
                                previous_idle_exhausted_budget = Some(budget);
                            }
                            IdleContinuationBudget::Executable => {
                                previous_idle_exhausted_budget = None;
                            }
                        }
                        workflow_idle_continuations = workflow_idle_continuations.saturating_add(1);
                        run_summary.record_continuation();
                        run_summary.record_phase();
                        full_response.clear();
                        let prompt = workflow_edit_continuation_prompt(
                            &raw_user_prompt,
                            extracted_flowscript_workspace.as_deref(),
                            workflow_idle_continuations,
                            last_validation_errors.as_ref(),
                        );
                        let continuation_send = session.send(MessageOptions {
                            prompt,
                            attachments: None,
                            mode: None,
                        });
                        let continuation_result = tokio::select! {
                            result = continuation_send => result.map_err(|error| error.to_string()),
                            _ = run_cancellation.cancelled() => {
                                Err("run cancelled before the continuation was sent".to_string())
                            }
                            _ = tokio::time::sleep(SDK_CONTROL_RPC_TIMEOUT) => {
                                Err(format!(
                                    "workflow continuation delivery exceeded {} seconds",
                                    SDK_CONTROL_RPC_TIMEOUT.as_secs(),
                                ))
                            }
                        };
                        match continuation_result {
                            Ok(_) => continue,
                            Err(e) => {
                                // Degrade instead of aborting: keep whatever the session already
                                // produced and surface the continuation failure as a note.
                                session_error_note = Some(format!(
                                    "failed to continue the workflow edit session: {e}"
                                ));
                                break;
                            }
                        }
                    }

                    break;
                }
                SessionEventData::SessionError(err) => {
                    let error_text = if err.message.trim().is_empty() {
                        err.error_type.clone()
                    } else {
                        format!("{}: {}", err.error_type, err.message)
                    };
                    close_pending_tool_steps(
                        &channel,
                        &mut open_tool_call_ids,
                        &tool_names_by_call_id,
                        "error",
                        Some(&error_text),
                        parent_request_id.as_deref(),
                    );
                    let has_partial_output = !full_response.trim().is_empty()
                        || !extracted_commands.is_empty()
                        || !extracted_components.is_empty()
                        || extracted_flowscript_workspace.is_some();
                    if !has_partial_output
                        && !workflow_state_has_retained_candidate(workflow_state.as_ref())
                    {
                        return Err(format!("Session error: {error_text}"));
                    }
                    session_error_note = Some(error_text);
                    break;
                }
                SessionEventData::SessionShutdown(_) | SessionEventData::Abort(_) => {
                    let note = "the Copilot session ended before the response completed";
                    close_pending_tool_steps(
                        &channel,
                        &mut open_tool_call_ids,
                        &tool_names_by_call_id,
                        "error",
                        Some(note),
                        parent_request_id.as_deref(),
                    );
                    if full_response.trim().is_empty()
                        && extracted_commands.is_empty()
                        && extracted_components.is_empty()
                        && !workflow_state_has_retained_candidate(workflow_state.as_ref())
                    {
                        return Err(
                            "GitHub Copilot session ended before producing a response.".to_string()
                        );
                    }
                    session_error_note = Some(note.to_string());
                    break;
                }
                _ => {}
            },
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                // The event buffer overflowed; skipping events is recoverable — terminating here
                // would silently kill the run mid-stream.
                eprintln!(
                    "[copilot_sdk_chat] Event stream lagged, skipped {skipped} events; continuing"
                );
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                let note = "the Copilot event stream closed before the session finished";
                flowpilot_debug_log!("[copilot_sdk_chat] Event stream closed before session idle");
                close_pending_tool_steps(
                    &channel,
                    &mut open_tool_call_ids,
                    &tool_names_by_call_id,
                    "error",
                    Some(note),
                    parent_request_id.as_deref(),
                );
                if full_response.trim().is_empty()
                    && extracted_commands.is_empty()
                    && extracted_components.is_empty()
                    && !workflow_state_has_retained_candidate(workflow_state.as_ref())
                {
                    return Err(
                        "GitHub Copilot stopped before producing a response (event stream closed)."
                            .to_string(),
                    );
                }
                session_error_note = Some(note.to_string());
                break;
            }
        }
    }

    if run_cancellation.is_cancelled() {
        // A retained commit is held in the queue with its commands until final delivery.
        // Cancellation is not a successful response handoff: abandon both atomically so the exact
        // revision is reopened instead of returning an orphaned command batch or token.
        abandon_side_effect_commands(&side_effect_commands);
    }

    // Collect the final native tail and its exact review token under one queue lock. Retained
    // batches are never eligible for the streaming drains above, so a poisoned/failing final lock
    // cannot expose their commands without the matching token.
    let (commands, flow_ir_commit) = take_side_effect_delivery(&side_effect_commands);
    if !commands.is_empty() {
        send_commands_event(&channel, &commands);
        extracted_commands.extend(commands);
    }

    // Same fallback for rendered UI: if the session ended before the "rendered" tool event was
    // observed, the emitted-surfaces store still holds the tree.
    if extracted_components.is_empty()
        && let Some(surface) = emitted_surfaces
            .lock()
            .ok()
            .and_then(|mut surfaces| surfaces.drain(..).last())
    {
        let components =
            serde_json::from_value::<Vec<SurfaceComponent>>(surface.components).unwrap_or_default();
        if !components.is_empty() {
            let comp_event = format!(
                "<components>{}</components>",
                serde_json::to_string(&components).unwrap_or_default()
            );
            let _ = channel.send(comp_event);
            let canvas_event = format!(
                "<canvas_settings>{}</canvas_settings>",
                serde_json::to_string(&surface.canvas_settings).unwrap_or_default()
            );
            let _ = channel.send(canvas_event);
            extracted_canvas_settings = Some(surface.canvas_settings);
            extracted_root_component_id = Some(surface.root_component_id);
            extracted_components = components;
        }
    }

    // Publish the session's own token usage as a usage_stat frame (once, after the loop so
    // workflow-edit continuations aggregate into a single stat). Labeled by role so nested board/UI
    // sub-runs are distinguishable from the top-level assistant in the stats sheet.
    if !usage_calls.is_empty() {
        let step_name = if global.is_some() {
            "Assistant"
        } else {
            match scope {
                CopilotScope::Board => "Board copilot",
                CopilotScope::Frontend => "UI copilot",
                CopilotScope::Both => "Copilot",
                CopilotScope::DataStudio => "Data Studio agent",
            }
        };
        send_correlated_stream_json_event(
            &channel,
            "usage_stat",
            &serde_json::json!({
                "step_name": step_name,
                "stats": {
                    "usage": {
                        "prompt_tokens": usage_prompt_tokens,
                        "completion_tokens": usage_completion_tokens,
                        "total_tokens": usage_prompt_tokens + usage_completion_tokens,
                        "cost": usage_has_cost.then_some(usage_cost),
                    },
                    "model": usage_model,
                    "iterations": usage_calls.len(),
                    "calls": usage_calls,
                },
            }),
            parent_request_id.as_deref(),
        );
    }

    // ── Fallback: if the model didn't call emit_ui but dumped JSON in the
    // response text, extract components from there so they still show up.
    if extracted_components.is_empty()
        && matches!(scope, CopilotScope::Frontend | CopilotScope::Both)
    {
        let surface = flow_like::a2ui::copilot::extract_surface_from_response(&full_response);
        if !surface.components.is_empty() {
            flowpilot_debug_log!(
                "[copilot_sdk_chat] Fallback: extracted {} components from text response",
                surface.components.len()
            );
            // Forward to frontend via channel so streaming UI picks them up
            let comp_event = format!(
                "<components>{}</components>",
                serde_json::to_string(&surface.components).unwrap_or_default()
            );
            let _ = channel.send(comp_event);
            if let Some(ref canvas) = surface.canvas_settings {
                let canvas_event = format!(
                    "<canvas_settings>{}</canvas_settings>",
                    serde_json::to_string(canvas).unwrap_or_default()
                );
                let _ = channel.send(canvas_event);
            }

            extracted_components = surface.components;
            if extracted_canvas_settings.is_none() {
                extracted_canvas_settings = surface.canvas_settings;
            }
            if extracted_root_component_id.is_none() {
                extracted_root_component_id = surface.root_component_id;
            }
        }
    }

    let workflow_snapshot = workflow_state
        .as_ref()
        .and_then(|state| state.lock().ok().map(|state| state.snapshot()));
    let has_retained_workflow_candidate = workflow_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.last_flowscript.as_ref())
        .is_some();

    let modular_fallback_queued = workflow_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.modular_fallback.as_ref())
        .is_some();
    let final_message = if workflow_edit_request {
        if !extracted_commands.is_empty() && modular_fallback_queued {
            "Queued an independently runnable partial working slice for review. The requested application is still incomplete; the fuller failed FlowScript remains retained for another repair pass. Do not treat this as full completion."
                .to_string()
        } else if !extracted_commands.is_empty() {
            "Queued workflow changes for review. Fill placeholder secrets before running."
                .to_string()
        } else if workflow_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.queued)
        {
            "This workflow draft revision was already queued earlier, so FlowPilot did not enqueue duplicate board commands."
                .to_string()
        } else if extracted_flowscript_workspace.is_some() || has_retained_workflow_candidate {
            "Workflow draft needs attention: no board commands were queued. Check the latest FlowScript compiler diagnostics, repair the retained source revision, and commit that same draft."
                .to_string()
        } else {
            "FlowPilot could not produce board commands or retain a workflow draft for this request."
                .to_string()
        }
    } else {
        full_response
    };

    let final_message = match &session_error_note {
        Some(note) if final_message.trim().is_empty() => {
            format!("The run ended early: {note}")
        }
        Some(note) => format!("{final_message}\n\n> Note: the run ended early ({note})."),
        None => final_message,
    };

    run_summary.set_applied_commands(extracted_commands.len());
    run_summary.resolve_outcome(session_error_note.is_some(), workflow_edit_request);

    // Preserve the best failed candidate for another repair turn, but pair source and status in one
    // envelope. The frontend applies only explicit `queued`; validation candidates remain visible
    // without becoming board mutations.
    let queued_workspace = queued_flowscript
        .lock()
        .ok()
        .and_then(|workspace| workspace.clone());
    let validated_flowscript_workspace = queued_workspace
        .as_deref()
        .map(|source| {
            flowscript_response_workspace_envelope(source, "queued", workflow_snapshot.as_ref())
        })
        .or_else(|| {
            workflow_snapshot.as_ref().and_then(|snapshot| {
                snapshot.last_flowscript.as_deref().map(|source| {
                    flowscript_workspace_envelope(
                        source,
                        snapshot
                            .last_status
                            .as_deref()
                            .unwrap_or("validation_errors"),
                    )
                })
            })
        });

    Ok(UnifiedCopilotResponse {
        message: final_message,
        commands: extracted_commands,
        suggestions: vec![],
        components: extracted_components,
        canvas_settings: extracted_canvas_settings,
        root_component_id: extracted_root_component_id,
        flowscript_workspace: validated_flowscript_workspace,
        flow_ir_commit,
        active_scope: scope,
    })
}

fn flowscript_response_workspace_envelope(
    source: &str,
    status: &str,
    snapshot: Option<&WorkflowToolLoopSnapshot>,
) -> String {
    let Some(snapshot) = snapshot else {
        return flowscript_workspace_envelope(source, status);
    };
    let Some(regression) = snapshot.modular_fallback.as_ref() else {
        return flowscript_workspace_envelope(source, status);
    };
    serde_json::json!({
        "source": source,
        "status": status,
        "completion": "partial_working_slice",
        "retained_full_source": snapshot.retained_full_source.as_deref(),
        "regression": {
            "previous_call_sites": regression.previous_call_sites,
            "candidate_call_sites": regression.candidate_call_sites,
            "previous_statements": regression.previous_statements,
            "candidate_statements": regression.candidate_statements,
            "previous_scope_symbols": regression.previous_scope_symbols,
            "retained_scope_symbols": regression.retained_scope_symbols,
        }
    })
    .to_string()
}

fn send_stream_json_event(channel: &Channel<String>, tag: &str, payload: &serde_json::Value) {
    send_correlated_stream_json_event(channel, tag, payload, None);
}

fn scoped_parent_request_id(context: Option<&FrontendToolContext>) -> Option<String> {
    context
        .and_then(|context| context.parent_request_id.as_deref())
        .map(str::trim)
        .filter(|request_id| !request_id.is_empty())
        .map(str::to_string)
}

/// Derive the immutable request identity that owns retained drafts and the acceptance contract.
///
/// Nested runs spawned from one user turn bind to the outer chat's source prompt instead of their
/// per-run specialist instruction, so a follow-up repair run can resume the retained draft. The
/// prompt text alone is not a safe identity: two conversations can send identical short prompts
/// ("yes, build it") against the same board inside the draft-store lease window, so the owning
/// conversation id is folded in whenever the host supplies one. Runs without a tool context (the
/// board panel copilot) keep their raw-prompt identity unchanged.
fn request_identity_prompt_for(
    tool_context: Option<&FrontendToolContext>,
    raw_user_prompt: &str,
) -> String {
    let source_prompt = tool_context
        .and_then(|context| context.source_user_prompt.as_deref())
        .filter(|prompt| !prompt.trim().is_empty())
        .unwrap_or(raw_user_prompt);
    let conversation_id = tool_context
        .and_then(|context| context.conversation_id.as_deref())
        .map(str::trim)
        .filter(|conversation_id| !conversation_id.is_empty());
    match conversation_id {
        Some(conversation_id) => format!("{conversation_id}\n{source_prompt}"),
        None => source_prompt.to_string(),
    }
}

fn correlated_stream_payload(
    payload: &serde_json::Value,
    parent_request_id: Option<&str>,
) -> serde_json::Value {
    let mut payload = payload.clone();
    if let (Some(parent_request_id), Some(object)) = (parent_request_id, payload.as_object_mut()) {
        object
            .entry("parent_request_id".to_string())
            .or_insert_with(|| serde_json::Value::String(parent_request_id.to_string()));
    }
    payload
}

fn send_correlated_stream_json_event(
    channel: &Channel<String>,
    tag: &str,
    payload: &serde_json::Value,
    parent_request_id: Option<&str>,
) {
    let payload = correlated_stream_payload(payload, parent_request_id);
    let event = format!(
        "<{tag}>{}</{tag}>",
        serde_json::to_string(&payload).unwrap_or_default()
    );
    let _ = channel.send(event);
}

/// Add nested-run correlation to a core/provider frame without touching plain assistant text.
/// Core emits fully framed strings, whereas the desktop SDK adapters emit structured payloads.
fn correlate_stream_frame(frame: &str, parent_request_id: Option<&str>) -> String {
    let Some(parent_request_id) = parent_request_id else {
        return frame.to_string();
    };
    let Some(tag_end) = frame.find('>') else {
        return frame.to_string();
    };
    if !frame.starts_with('<') {
        return frame.to_string();
    }
    let tag = &frame[1..tag_end];
    if !matches!(
        tag,
        "tool_start"
            | "tool_progress"
            | "tool_end"
            | "plan_step"
            | "usage_stat"
            | "flowscript_workspace"
    ) {
        return frame.to_string();
    }
    let close_tag = format!("</{tag}>");
    let Some(payload_text) = frame
        .strip_prefix(&frame[..=tag_end])
        .and_then(|rest| rest.strip_suffix(&close_tag))
    else {
        return frame.to_string();
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(payload_text) else {
        return frame.to_string();
    };
    let payload = correlated_stream_payload(&payload, Some(parent_request_id));
    format!(
        "<{tag}>{}</{tag}>",
        serde_json::to_string(&payload).unwrap_or_default()
    )
}

/// Build the workspace status frame emitted when a FlowScript lifecycle tool finishes. Retained
/// source tools return the exact model-authored `source`; the legacy one-shot edit falls back to
/// the source captured at tool start. Typed-draft results remain readable for old sessions, but
/// are no longer the advertised authoring path. The UI applies only `queued` workspaces and keeps
/// every other status visible for repair.
fn flowscript_workspace_result_payload(
    tool_name: &str,
    result: &serde_json::Value,
    latest_submitted: Option<&str>,
) -> Option<serde_json::Value> {
    let explicit_workspace = result
        .get("flowscript_workspace")
        .and_then(serde_json::Value::as_str);
    let retained_workspace = matches!(
        tool_name,
        "write_flowscript"
            | "patch_flowscript"
            | "check_flowscript"
            | "commit_flowscript"
            | "begin_flow_ir_draft"
            | "update_flow_ir_draft"
            | "upsert_flow_ir_module"
            | "validate_flow_ir_draft"
            | "commit_flow_ir_draft"
    )
    .then(|| {
        result
            .get("source")
            .or_else(|| result.get("flowscript"))
            .and_then(serde_json::Value::as_str)
    })
    .flatten();
    let workspace = explicit_workspace.or(retained_workspace).or_else(|| {
        (tool_name == "edit_flowscript")
            .then_some(latest_submitted)
            .flatten()
    })?;
    let status = result
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");

    let mut payload = serde_json::json!({
        "source": workspace,
        "status": status,
    });
    if let Some(object) = payload.as_object_mut() {
        for key in ["draft_id", "revision", "base_fingerprint"] {
            if let Some(value) = result.get(key) {
                object.insert(key.to_string(), value.clone());
            }
        }
    }
    Some(payload)
}

/// Emit the tool_start frame (plus a workspace preview for full-source authoring tools) for a tool
/// call announced either via tool.execution_start or the protocol v3 external_tool.requested
/// broadcast.
fn announce_tool_start(
    channel: &Channel<String>,
    tool_call_id: &str,
    tool_name: &str,
    arguments: Option<&serde_json::Value>,
    extracted_flowscript_workspace: &mut Option<String>,
    parent_request_id: Option<&str>,
) {
    if flow_like::flow::copilot::stream::is_flowscript_authoring_tool(tool_name)
        && let Some(workspace) =
            arguments.and_then(flow_like::flow::copilot::stream::source_argument)
    {
        *extracted_flowscript_workspace = Some(workspace.to_string());
        send_stream_json_event(
            channel,
            "flowscript_workspace",
            &serde_json::json!({
                "source": workspace,
                "status": "submitted",
                "tool_call_id": tool_call_id,
            }),
        );
    }

    send_correlated_stream_json_event(
        channel,
        "tool_start",
        &serde_json::json!({
            "tool_call_id": tool_call_id,
            "tool": tool_name,
            "status": "running",
            "summary": flow_like::flow::copilot::stream::safe_text_preview(
                &summarize_tool_arguments(tool_name, arguments),
                600,
            ),
            "arguments_preview": preview_tool_arguments(tool_name, arguments),
        }),
        parent_request_id,
    );
}

/// Close every tool step that got a tool_start but never a completion event, so the frontend does
/// not keep spinners alive after the session ends (idle, error, or stream loss).
fn close_pending_tool_steps(
    channel: &Channel<String>,
    open_tool_call_ids: &mut HashSet<String>,
    tool_names_by_call_id: &HashMap<String, String>,
    status: &str,
    error: Option<&str>,
    parent_request_id: Option<&str>,
) {
    let safe_error =
        error.map(|error| flow_like::flow::copilot::stream::safe_text_preview(error, 600));
    for tool_call_id in open_tool_call_ids.drain() {
        let tool = tool_names_by_call_id
            .get(&tool_call_id)
            .cloned()
            .unwrap_or_else(|| "tool".to_string());
        send_correlated_stream_json_event(
            channel,
            "tool_end",
            &serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool": tool,
                "status": status,
                "result_summary": safe_error.as_deref().unwrap_or("completed"),
                "error": safe_error.as_deref(),
            }),
            parent_request_id,
        );
    }
}

fn truncate_for_preview(value: &str, max_chars: usize) -> String {
    let mut result = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            result.push_str("...");
            break;
        }
        result.push(ch);
    }
    result
}

fn line_count(value: &str) -> usize {
    value.lines().count().max(usize::from(!value.is_empty()))
}

fn summarize_tool_arguments(tool_name: &str, arguments: Option<&serde_json::Value>) -> String {
    let Some(arguments) = arguments else {
        return "No arguments".to_string();
    };

    match tool_name {
        "get_declarations" | "catalog_search" | "search_by_pin" => arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(|query| format!("query: {query}"))
            .unwrap_or_else(|| "Searching".to_string()),
        "internet_search" => arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(|query| format!("query: {query}"))
            .unwrap_or_else(|| "Searching web".to_string()),
        "database_tool" | "storage_tool" => arguments
            .get("operation")
            .and_then(|value| value.as_str())
            .map(|operation| {
                let target = arguments
                    .get("table_name")
                    .or_else(|| arguments.get("tableName"))
                    .or_else(|| arguments.get("path"))
                    .or_else(|| arguments.get("prefix"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                if target.is_empty() {
                    operation.to_string()
                } else {
                    format!("{operation}: {target}")
                }
            })
            .unwrap_or_else(|| "Preparing frontend operation".to_string()),
        "execute_event" => arguments
            .get("event_id")
            .or_else(|| arguments.get("eventId"))
            .and_then(|value| value.as_str())
            .map(|event_id| format!("event: {event_id}"))
            .unwrap_or_else(|| "Executing event".to_string()),
        "ask_user" => arguments
            .get("question")
            .and_then(|value| value.as_str())
            .map(|question| truncate_for_preview(question, 180))
            .unwrap_or_else(|| "Requesting user input".to_string()),
        "edit_flowscript" | "write_flowscript" => arguments
            .get("source")
            .or_else(|| arguments.get("flowscript"))
            .or_else(|| arguments.get("script"))
            .or_else(|| arguments.get("content"))
            .and_then(|value| value.as_str())
            .map(|flowscript| {
                format!(
                    "{} lines, {} chars",
                    line_count(flowscript),
                    flowscript.chars().count()
                )
            })
            .unwrap_or_else(|| "Submitting FlowScript".to_string()),
        "patch_flowscript" => {
            let old_chars = arguments
                .get("old_text")
                .or_else(|| arguments.get("search"))
                .and_then(serde_json::Value::as_str)
                .map(|value| value.chars().count())
                .unwrap_or_default();
            let new_chars = arguments
                .get("new_text")
                .or_else(|| arguments.get("replacement"))
                .and_then(serde_json::Value::as_str)
                .map(|value| value.chars().count())
                .unwrap_or_default();
            format!("replace {old_chars} chars with {new_chars} chars")
        }
        "check_flowscript" | "commit_flowscript" => {
            let draft_id = arguments
                .get("draft_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<draft>");
            let revision = arguments
                .get("expected_revision")
                .and_then(serde_json::Value::as_u64)
                .map(|revision| revision.to_string())
                .unwrap_or_else(|| "?".to_string());
            format!("draft {draft_id}, revision {revision}")
        }
        "emit_commands" | "validate_commands" => arguments
            .get("commands")
            .and_then(|value| value.as_array())
            .map(|commands| format!("{} command(s)", commands.len()))
            .unwrap_or_else(|| "Preparing commands".to_string()),
        "emit_ui" | "validate_ui" => arguments
            .get("components")
            .and_then(|value| value.as_array())
            .map(|components| format!("{} component(s)", components.len()))
            .unwrap_or_else(|| "Preparing UI".to_string()),
        _ => preview_tool_arguments(tool_name, Some(arguments)),
    }
}

fn preview_tool_arguments(_tool_name: &str, arguments: Option<&serde_json::Value>) -> String {
    let Some(arguments) = arguments else {
        return "{}".to_string();
    };

    flow_like::flow::copilot::stream::safe_json_preview(
        arguments,
        flow_like::flow::copilot::stream::TOOL_ARGUMENT_PREVIEW_CHARS,
    )
}

fn extract_json_status(content: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| {
            value
                .get("status")
                .and_then(|status| status.as_str().map(str::to_string))
        })
}

fn direct_sdk_tool_result_stream_status(content: &str) -> &'static str {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) else {
        return flow_like::flow::copilot::stream::tool_result_stream_status(content);
    };
    let status = parsed
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if matches!(
        status,
        "draft_started" | "draft_updated" | "module_validated"
    ) && !workflow_result_has_explicit_diagnostics(&parsed)
    {
        // Missing expected modules are normal staged progress here. Preserve them for an idle
        // continuation, but do not paint a successful begin/upsert tool call as failed.
        return "done";
    }
    let diagnostics = workflow_result_diagnostics(Some(&parsed));
    if workflow_result_requires_repair(&parsed, &diagnostics) {
        return "error";
    }
    match status {
        "draft_started" | "draft_updated" | "module_validated" | "draft_valid" | "rendered"
        | "queued" | "already_queued" | "valid" => "done",
        _ => flow_like::flow::copilot::stream::tool_result_stream_status(content),
    }
}

fn summarize_tool_result(content: Option<&str>, error: Option<&str>) -> String {
    if let Some(error) = error {
        return error.to_string();
    }

    let Some(content) = content else {
        return "Completed".to_string();
    };

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) {
        let status = parsed
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("done");
        let command_count = parsed
            .get("commands")
            .and_then(|value| value.as_array())
            .map(Vec::len);
        let component_count = parsed
            .get("components")
            .and_then(|value| value.as_array())
            .map(Vec::len);
        let error_count = parsed
            .get("errors")
            .and_then(|value| value.as_array())
            .map(Vec::len);
        let diagnostic_count = parsed
            .get("diagnostics")
            .and_then(|value| value.as_array())
            .map(Vec::len);

        let mut parts = vec![status.replace('_', " ")];
        if let Some(count) = command_count {
            parts.push(format!("{count} command(s)"));
        }
        if let Some(count) = component_count {
            parts.push(format!("{count} component(s)"));
        }
        if let Some(count) = error_count.filter(|count| *count > 0) {
            parts.push(format!("{count} error(s)"));
        }
        if let Some(count) = diagnostic_count.filter(|count| *count > 0) {
            parts.push(format!("{count} diagnostic(s)"));
        }
        return parts.join(" · ");
    }

    truncate_for_preview(content.trim(), 240)
}

fn render_recovered_mutation_message(completion: &McpToolCompletion) -> String {
    let summary = summarize_tool_result(Some(&completion.result_text), None);
    let preview = preview_tool_result(&completion.result_text);
    format!(
        "`{}` completed successfully before the provider process exited ({summary}). The completed tool result was preserved:\n\n{preview}",
        completion.tool_name
    )
}

fn preview_tool_result(content: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) {
        return flow_like::flow::copilot::stream::safe_json_preview(
            &parsed,
            flow_like::flow::copilot::stream::TOOL_RESULT_PREVIEW_CHARS,
        );
    }

    flow_like::flow::copilot::stream::safe_text_preview(
        content,
        flow_like::flow::copilot::stream::TOOL_RESULT_PREVIEW_CHARS,
    )
}

fn is_workflow_edit_request(prompt: &str) -> bool {
    let prompt = prompt.to_lowercase();
    if is_read_only_workflow_request(&prompt) {
        return false;
    }

    let edit_verbs = [
        "add",
        "apply",
        "automate",
        "build",
        "connect",
        "configur",
        "create",
        "draft",
        "embed",
        "fetch",
        "fix",
        "generate",
        "insert",
        "make",
        "modify",
        "repair",
        "schedule",
        "set up",
        "store",
        "translate",
        "update",
        "wire",
        // German UI prompts are common in FlowPilot. Use stems so natural inflections such as
        // "Bau", "baue", "erstelle" and "automatisiere" enter the same guarded edit loop.
        "bau",
        "erstell",
        "hinzuf",
        "füge",
        "automatisier",
        "änder",
        "anpass",
        "reparier",
        "verbind",
        "implementier",
        "konfigurier",
        "plan",
        "speicher",
    ];
    let workflow_terms = [
        "automation",
        "board",
        "cron",
        "database",
        "db",
        "email",
        "flow",
        "flowscript",
        "gmail",
        "imap",
        "lancedb",
        "mail",
        "node",
        "nodes",
        "open database",
        "pipeline",
        "smtp",
        "vector",
        "workflow",
        "api call",
        "edge",
        "edges",
        "execution",
        "event",
        "pin",
        "pins",
        "schedule",
        "scheduler",
        "trigger",
        "automatisierung",
        "auslöser",
        "datenbank",
        "ereignis",
        "knoten",
        "schnittstelle",
        "zeitplan",
        "success output",
        "error output",
    ];

    edit_verbs.iter().any(|verb| prompt.contains(verb))
        && workflow_terms.iter().any(|term| prompt.contains(term))
}

fn is_read_only_workflow_request(prompt: &str) -> bool {
    let read_only_terms = [
        "are these",
        "can this",
        "check",
        "debug",
        "diagnose",
        "does this",
        "error",
        "explain",
        "how does",
        "inspect",
        "is this",
        "issue",
        "not working",
        "problem",
        "review",
        "show me",
        "tell me",
        "what does",
        "what is",
        "what's wrong",
        "where",
        "which",
        "why",
        "erklär",
        "warum",
        "wie funktioniert",
        "prüf",
        "untersuch",
        "fehler",
        "problem",
        "zeige",
        "welche",
        "wo ",
    ];
    if !read_only_terms.iter().any(|term| prompt.contains(term)) {
        return false;
    }

    let mutation_terms = [
        "add",
        "apply",
        "automate",
        "build",
        "change",
        "create",
        "delete",
        "draft",
        "fix",
        "generate",
        "insert",
        "make",
        "modify",
        "remove",
        "repair",
        "store",
        "translate",
        "update",
        "bau",
        "erstell",
        "hinzuf",
        "füge",
        "automatisier",
        "änder",
        "anpass",
        "reparier",
        "verbind",
        "implementier",
    ];

    !mutation_terms.iter().any(|term| prompt.contains(term))
}

fn drain_streamable_side_effect_commands(
    store: &Arc<StdMutex<SideEffectCommandQueue>>,
) -> Vec<BoardCommand> {
    match store.lock() {
        Ok(mut queue) => queue.drain_streamable(),
        Err(poisoned) => {
            let mut queue = poisoned.into_inner();
            queue.abandon();
            Vec::new()
        }
    }
}

fn take_side_effect_delivery(
    store: &Arc<StdMutex<SideEffectCommandQueue>>,
) -> (Vec<BoardCommand>, Option<FlowIrCommitToken>) {
    match store.lock() {
        Ok(mut queue) => queue.take_delivery(),
        Err(poisoned) => {
            let mut queue = poisoned.into_inner();
            queue.abandon();
            (Vec::new(), None)
        }
    }
}

fn abandon_side_effect_commands(store: &Arc<StdMutex<SideEffectCommandQueue>>) {
    match store.lock() {
        // The response/delivery channel for this run is gone, but a checked+committed batch stays
        // pending in the retained draft store so the next same-request run redelivers its exact
        // Apply/Dismiss token instead of burning a full rebuild cycle on identical commands.
        Ok(mut queue) => queue.abandon_preserving_retained_review(),
        // A poisoned queue cannot vouch for its claim state; fail closed and reopen the revision.
        Err(poisoned) => poisoned.into_inner().abandon(),
    }
}

/// Every early-return path (provider error, cancellation, closed stream, or host teardown) must
/// abandon a batch that was never transferred into the response. Successfully drained queues are
/// empty, so normal completion makes this cleanup a no-op.
struct SideEffectCommandQueueCleanup(Arc<StdMutex<SideEffectCommandQueue>>);

impl Drop for SideEffectCommandQueueCleanup {
    fn drop(&mut self) {
        abandon_side_effect_commands(&self.0);
    }
}

fn build_flowpilot_sdk_tools(
    app_handle: AppHandle,
    scope: CopilotScope,
    surface: &FlowPilotAgentSurface,
    global: bool,
    nested: bool,
    tool_context: Option<FrontendToolContext>,
    memory: Option<Arc<AssistantMemory>>,
    user_prompt: &str,
) -> Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)> {
    use super::{
        copilot_sdk_tools::{
            create_board_tools, create_data_studio_tools, create_frontend_tools,
            create_global_assistant_tools, create_runtime_tools,
        },
        frontend_tool_bridge::{FrontendToolBridge, GLOBAL_FRONTEND_TOOL_EVENT},
    };

    // The global assistant is not bound to a board/surface: it gets the curated global tool set on
    // its own bridge event so its tool requests reach the global listener, not the board copilot's.
    if global {
        let bridge = FrontendToolBridge::new_with_event(app_handle, GLOBAL_FRONTEND_TOOL_EVENT);
        return create_global_assistant_tools(bridge, memory, user_prompt);
    }

    let mut tools = match scope {
        CopilotScope::Board => create_board_tools(
            surface.graph_context.clone(),
            surface.board_arc.clone(),
            surface.live_board.clone(),
            surface.request_acceptance_prompt.as_deref(),
            surface.catalog_provider.clone(),
            Some(surface.side_effect_commands.clone()),
            Some(surface.queued_flowscript.clone()),
        ),
        CopilotScope::Frontend => create_frontend_tools(Some(surface.emitted_surfaces.clone())),
        CopilotScope::Both => {
            let mut all_tools = create_board_tools(
                surface.graph_context.clone(),
                surface.board_arc.clone(),
                surface.live_board.clone(),
                surface.request_acceptance_prompt.as_deref(),
                surface.catalog_provider.clone(),
                Some(surface.side_effect_commands.clone()),
                Some(surface.queued_flowscript.clone()),
            );
            all_tools.extend(create_frontend_tools(Some(
                surface.emitted_surfaces.clone(),
            )));
            all_tools
        }
        // Data Studio is a data-only specialist: no board/UI tools, just its graph/data tool set.
        CopilotScope::DataStudio => Vec::new(),
    };
    let runtime_bridge = if nested {
        FrontendToolBridge::new_with_event(app_handle, GLOBAL_FRONTEND_TOOL_EVENT)
    } else {
        FrontendToolBridge::new(app_handle)
    }
    .with_context(tool_context);
    if matches!(scope, CopilotScope::DataStudio) {
        tools.extend(create_data_studio_tools(runtime_bridge));
    } else {
        tools.extend(create_runtime_tools(runtime_bridge));
    }
    tools
}

#[derive(Clone)]
struct FlowPilotMcpTool {
    definition: copilot_sdk::Tool,
    handler: copilot_sdk::ToolHandler,
}

/// Cancels the synchronous tool bridge if the async MCP request future is dropped (for example,
/// when Claude/Codex disconnects its HTTP transport mid-call). `spawn_blocking` tasks are detached
/// when their JoinHandle is dropped, so aborting the async request alone is otherwise insufficient.
struct McpToolCancellationGuard {
    cancellation: CancellationToken,
    armed: bool,
}

impl McpToolCancellationGuard {
    fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for McpToolCancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancellation.cancel();
        }
    }
}

/// Delegation tools whose MCP call blocks the outer agent on a nested FlowPilot run.
fn is_delegated_agent_tool(tool_name: &str) -> bool {
    matches!(tool_name, "flowpilot_board" | "flowpilot_widget")
}

#[derive(Clone, Debug)]
struct DelegatedRunToolProgress {
    tool_name: String,
    total_tool_calls: u64,
    budget_summary: Option<String>,
}

/// Most recent tool progress reported by any FlowPilot MCP run in this process. While the outer
/// agent waits on flowpilot_board/flowpilot_widget its only signal is the progress heartbeat, so
/// this single bounded slot gives those heartbeats substance (last tool used plus loop budget
/// counts) without cross-run plumbing. Diagnostic prose only — never used for control flow.
static LATEST_DELEGATED_RUN_TOOL_PROGRESS: LazyLock<
    StdMutex<Option<(Instant, DelegatedRunToolProgress)>>,
> = LazyLock::new(|| StdMutex::new(None));

/// A stale entry (e.g. from an earlier finished run) must not narrate a hung wait as progress.
const DELEGATED_RUN_PROGRESS_FRESHNESS: Duration = Duration::from_secs(3 * 60);

fn record_delegated_run_tool_progress(
    tool_name: &str,
    total_tool_calls: u64,
    workflow_state: Option<&Arc<StdMutex<WorkflowToolLoopState>>>,
) {
    // The delegation tools themselves are what the outer agent is waiting ON; recording them
    // would overwrite the nested run's substance with the wait itself.
    if is_delegated_agent_tool(tool_name) {
        return;
    }
    let budget_summary = workflow_state
        .and_then(|state| state.lock().ok())
        .map(|state| {
            let snapshot = state.snapshot();
            format!(
                "checks {}/{MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS}, source operations {}/{MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS}, commit attempts {}/{MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS}",
                snapshot.edit_attempts,
                snapshot.flowscript_operation_attempts,
                snapshot.flowscript_commit_attempts,
            )
        });
    if let Ok(mut latest) = LATEST_DELEGATED_RUN_TOOL_PROGRESS.lock() {
        *latest = Some((
            Instant::now(),
            DelegatedRunToolProgress {
                tool_name: tool_name.to_string(),
                total_tool_calls,
                budget_summary,
            },
        ));
    }
}

/// Compose one heartbeat line for a delegation tool: the base "still running" text plus the
/// freshest nested-run tool/budget progress, so waiting turns are not blind.
fn delegated_run_heartbeat_message(base: &str) -> String {
    let progress = LATEST_DELEGATED_RUN_TOOL_PROGRESS
        .lock()
        .ok()
        .and_then(|latest| {
            latest.as_ref().and_then(|(recorded_at, progress)| {
                (recorded_at.elapsed() <= DELEGATED_RUN_PROGRESS_FRESHNESS)
                    .then(|| progress.clone())
            })
        });
    let Some(progress) = progress else {
        return base.to_string();
    };
    match progress.budget_summary.as_deref() {
        Some(budgets) => format!(
            "{base}; the delegated run last used {} (tool call {}; {budgets})",
            progress.tool_name, progress.total_tool_calls
        ),
        None => format!(
            "{base}; the delegated run last used {} (tool call {})",
            progress.tool_name, progress.total_tool_calls
        ),
    }
}

/// Keeps a long-running frontend-backed MCP call observable to clients with an idle watchdog.
/// MCP progress is opt-in: the server may only emit it when the caller supplied a progress token
/// in the request metadata. The task is tied to both the request cancellation token and this RAII
/// guard, so a completed, cancelled, or dropped request cannot leave a detached notifier behind.
struct McpProgressHeartbeat {
    cancellation: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl McpProgressHeartbeat {
    fn start(
        context: &rmcp::service::RequestContext<rmcp::RoleServer>,
        request_cancellation: CancellationToken,
        tool_name: &str,
    ) -> Option<Self> {
        let progress_token = context.meta.get_progress_token()?;
        let peer = context.peer.clone();
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let message = format!("FlowPilot {tool_name} is still running");
        let delegated_tool = is_delegated_agent_tool(tool_name);
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval_at(
                tokio::time::Instant::now() + MCP_TOOL_PROGRESS_HEARTBEAT_INTERVAL,
                MCP_TOOL_PROGRESS_HEARTBEAT_INTERVAL,
            );
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut progress = 0.0_f64;

            loop {
                tokio::select! {
                    biased;
                    _ = task_cancellation.cancelled() => break,
                    _ = request_cancellation.cancelled() => break,
                    _ = ticker.tick() => {
                        progress += 1.0;
                        // Recomputed per tick: a delegation wait should narrate the nested run's
                        // latest tool/budget movement, not repeat a blind static line.
                        let tick_message = if delegated_tool {
                            delegated_run_heartbeat_message(&message)
                        } else {
                            message.clone()
                        };
                        let notification = mcp_progress_heartbeat_notification(
                            progress_token.clone(),
                            progress,
                            &tick_message,
                        );
                        let result = tokio::select! {
                            biased;
                            _ = task_cancellation.cancelled() => break,
                            _ = request_cancellation.cancelled() => break,
                            result = peer.notify_progress(notification) => result,
                        };
                        if result.is_err() {
                            // The peer/transport is gone. Retrying forever would only retain the
                            // request state; the owning handler still has its own cancellation.
                            break;
                        }
                    }
                }
            }
        });

        Some(Self { cancellation, task })
    }
}

impl Drop for McpProgressHeartbeat {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.task.abort();
    }
}

fn mcp_progress_heartbeat_notification(
    progress_token: rmcp::model::ProgressToken,
    progress: f64,
    message: &str,
) -> rmcp::model::ProgressNotificationParam {
    rmcp::model::ProgressNotificationParam::new(progress_token, progress).with_message(message)
}

/// Wall-clock budget for one NESTED delegated FlowPilot run (flowpilot_board/flowpilot_widget).
/// It must stay well below the outer 30-minute bridge dispatch bound so budget exhaustion reaches
/// the waiting agent as a terminal, actionable incomplete result (retained draft coordinates plus
/// diagnostics) instead of an opaque outer-channel timeout after a burned turn.
const NESTED_RUN_WALL_CLOCK_BUDGET: Duration = Duration::from_secs(12 * 60);
const MAX_EXTERNAL_WORKFLOW_CONTINUATIONS: u8 = 2;
// Once a usable live declaration batch exists, a provider phase must dispatch its first source
// checkpoint promptly. If it silently composes until this soft bound, retain the discovery state
// and move to a fresh continuation instead of letting the phase consume the full nested budget.
const EXTERNAL_PREDRAFT_SOURCE_CHECKPOINT_BUDGET: Duration = Duration::from_secs(3 * 60);
const MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS: u8 = 12;
const MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS: u8 = 3;
// A continuation phase whose instructions demand more patching must actually be executable:
// exhausted stall/operation budgets receive this small bounded headroom instead of arriving dead.
const EXTERNAL_CONTINUATION_OPERATION_HEADROOM: u16 = 6;
const EXTERNAL_CONTINUATION_CHECK_HEADROOM: u8 = 2;
// Provider phases that failed transiently before the CLI issued a single tool call did no work;
// they are retried on their own bounded counter instead of consuming workflow continuations.
const MAX_EXTERNAL_ZERO_ACTIVITY_RESTARTS: u8 = 2;
const EXTERNAL_TRANSIENT_RESTART_BACKOFF: Duration = Duration::from_secs(2);
// Count every model-dispatched source lifecycle operation, not only compiler checks. Otherwise a
// provider can alternate whole-document writes and patches forever without consuming the older
// validation-attempt budget.
const MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS: u16 = 24;
const MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS: u8 = 3;
const MAX_RETAINED_STRUCTURED_DIAGNOSTICS: usize = 12;
const MAX_RETAINED_STRUCTURED_DIAGNOSTIC_BYTES: usize = 12_000;
// A typed build gets fixed lifecycle overhead plus roughly three operations per declared module
// (initial upsert and two repairs), while the fixed ceiling prevents an unbounded repair loop.
const MIN_EXTERNAL_TYPED_IR_OPERATION_BUDGET: u16 = 24;
const MAX_EXTERNAL_TYPED_IR_OPERATION_BUDGET: u16 = 64;
const MAX_EXTERNAL_TYPED_IR_STALLED_ATTEMPTS: u8 = 3;
const MAX_EXTERNAL_WORKFLOW_DECLARATION_CALLS: u8 =
    MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS.saturating_add(1);
const MAX_INITIAL_DECLARATION_ATTEMPTS: u8 = 3;
const MAX_EXTERNAL_PREDRAFT_CONTEXT_READS: u8 = 6;
const MAX_REPAIR_DECLARATION_QUERIES: usize = 12;
const MAX_REPAIR_DECLARATION_QUERY_BYTES: usize = 200;
const MAX_REPAIR_DECLARATION_ATTEMPTS_PER_KEY: u8 = 2;
const MAX_INJECTED_REPAIR_DECLARATIONS: usize = 32;
const MAX_INJECTED_REPAIR_DECLARATION_BYTES: usize = 30_000;
const MAX_RETAINED_DECLARATION_BYTES: usize = 48_000;

fn submitted_flowscript(args: &serde_json::Value) -> Option<&str> {
    args.get("flowscript")
        .or_else(|| args.get("script"))
        .or_else(|| args.get("source"))
        .or_else(|| args.get("content"))
        .and_then(serde_json::Value::as_str)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowMutationPath {
    TypedIr,
    FlowScript,
    DirectCommands,
}

#[derive(Debug, Clone, Default)]
struct WorkflowToolLoopSnapshot {
    queued: bool,
    last_flowscript: Option<String>,
    last_declarations: Option<String>,
    declaration_lookup_complete: bool,
    unresolved_declaration_queries: Vec<String>,
    /// Exact live-catalog signatures injected by the latest FlowScript validation result. These
    /// are kept separately from the flattened diagnostic messages so a subprocess continuation
    /// does not lose the machine-actionable repair context.
    last_repair_declarations: Vec<String>,
    last_status: Option<String>,
    last_errors: Vec<String>,
    edit_attempts: u8,
    flowscript_operation_attempts: u16,
    stalled_edit_attempts: u8,
    flowscript_commit_attempts: u8,
    /// Human-readable name of the loop budget that is currently exhausted, if any. A continuation
    /// phase that starts in this state would be refused-on-arrival unless the host grants a fresh
    /// bounded slice first.
    exhausted_budget: Option<String>,
    last_structured_diagnostics: Vec<serde_json::Value>,
    last_review_notes: usize,
    modular_fallback: Option<FlowScriptCandidateRegression>,
    retained_full_source: Option<String>,
    flowscript_draft_id: Option<String>,
    flowscript_draft_retained: bool,
    flowscript_revision: Option<u64>,
    typed_draft_id: Option<String>,
    typed_draft_retained: bool,
    typed_revision: Option<u64>,
    typed_operation_attempts: u16,
    typed_operation_budget: u16,
    typed_stalled_attempts: u8,
    typed_missing_modules: Vec<String>,
    mutation_path: Option<WorkflowMutationPath>,
}

#[derive(Debug, Default)]
struct WorkflowToolLoopState {
    current_reads: u8,
    predraft_context_reads: u8,
    declaration_calls: u8,
    declarations_since_edit: u8,
    declaration_lookup_in_flight: bool,
    initial_declaration_attempts: u8,
    initial_declaration_lookup_usable: bool,
    initial_declaration_lookup_complete: bool,
    unresolved_declaration_queries: Vec<String>,
    completed_repair_lookup_keys: HashSet<String>,
    in_flight_repair_lookup_keys: HashSet<String>,
    repair_lookup_attempts: HashMap<String, u8>,
    edit_attempts: u8,
    flowscript_operation_attempts: u16,
    flowscript_commit_attempts: u8,
    stalled_edit_attempts: u8,
    has_previous_validation_result: bool,
    previous_validation_diagnostics: HashSet<String>,
    flowscript_seen_repair_signatures: HashSet<String>,
    edit_in_flight: bool,
    /// Captured before invoking the frontend validator so a process/transport phase boundary cannot
    /// erase the only copy of a just-submitted rich draft.
    in_flight_flowscript: Option<String>,
    queued: bool,
    last_flowscript: Option<String>,
    repair_tracker: FlowScriptRepairTracker,
    best_failed_errors: Vec<String>,
    candidate_regression_warning: Option<String>,
    pending_modular_fallback: Option<FlowScriptCandidateRegression>,
    last_declarations: Option<String>,
    last_repair_declarations: Vec<String>,
    last_status: Option<String>,
    last_errors: Vec<String>,
    last_structured_diagnostics: Vec<serde_json::Value>,
    last_review_notes: usize,
    flowscript_draft_id: Option<String>,
    flowscript_draft_retained: bool,
    flowscript_revision: Option<u64>,
    typed_draft_id: Option<String>,
    typed_draft_retained: bool,
    typed_revision: Option<u64>,
    typed_operation_attempts: u16,
    typed_expected_modules: usize,
    typed_stalled_attempts: u8,
    typed_missing_modules: Vec<String>,
    typed_seen_repair_signatures: HashMap<String, HashSet<String>>,
    mutation_path: Option<WorkflowMutationPath>,
}

impl WorkflowToolLoopState {
    fn from_flowscript_recovery(
        recovery: Option<&flow_like::flow::copilot::FlowScriptDraftRecovery>,
    ) -> Self {
        let mut state = Self::default();
        let Some(recovery) = recovery else {
            return state;
        };
        if !matches!(
            recovery.status,
            flow_like::flow::copilot::FlowIrDraftRecoveryStatus::ExactMatch
        ) || !recovery.auto_resume
        {
            return state;
        }
        let Some(context) = recovery
            .exact_match
            .as_ref()
            .filter(|context| !context.stale_board)
        else {
            return state;
        };
        let Some(source) = context.source.as_deref() else {
            return state;
        };

        let status = if context.checked {
            "valid"
        } else {
            context.status.as_str()
        };
        let payload = serde_json::json!({
            "status": status,
            "diagnostics": &context.diagnostics,
        });
        let diagnostics = workflow_result_diagnostics(Some(&payload));
        state.last_structured_diagnostics = workflow_result_structured_diagnostics(Some(&payload));
        state.last_repair_declarations = workflow_result_repair_declarations(Some(&payload));
        state.last_status = Some(status.to_string());
        state.last_errors = diagnostics.clone();
        state.last_flowscript = Some(source.to_string());
        state.flowscript_draft_id = Some(context.draft_id.clone());
        state.flowscript_draft_retained = true;
        state.flowscript_revision = Some(context.revision);
        state.initial_declaration_lookup_usable = true;
        state.initial_declaration_lookup_complete = true;
        state.mutation_path = Some(WorkflowMutationPath::FlowScript);
        if !context.checked && !diagnostics.is_empty() {
            state
                .repair_tracker
                .record_failed_with_diagnostics(source, Some(diagnostics.len()));
            state.best_failed_errors = diagnostics.clone();
            state
                .flowscript_seen_repair_signatures
                .insert(flowscript_repair_fingerprint(
                    Some(status),
                    &diagnostics,
                    &state.last_structured_diagnostics,
                ));
        }
        state
    }

    fn needs_initial_declaration_coverage(&self) -> bool {
        // This is an explicit host-owned authorization bit. The first usable live-catalog result
        // unlocks a retained full-shape draft; complete coverage remains separate reporting data.
        // Compiler diagnostics, rather than exhaustive pre-draft discovery, drive later focused
        // lookups. Exact retained recovery seeds both bits in `from_flowscript_recovery`.
        !(self.initial_declaration_lookup_usable || self.initial_declaration_lookup_complete)
    }

    fn snapshot(&self) -> WorkflowToolLoopSnapshot {
        let (retained_flowscript, retained_status, retained_errors) = if self.queued {
            (
                self.last_flowscript.clone(),
                Some("queued".to_string()),
                self.last_errors.clone(),
            )
        } else if self.flowscript_draft_retained && self.last_flowscript.is_some() {
            // The retained source store is authoritative for the code-first lifecycle. A richer
            // earlier failed candidate is useful for regression checks, but must not replace a
            // newer exact revision (especially a `valid` one) in continuation/recovery context.
            (
                self.last_flowscript.clone(),
                self.last_status.clone(),
                self.last_errors.clone(),
            )
        } else if let Some(best_failed) = self.repair_tracker.best_failed_source() {
            let mut errors = self.best_failed_errors.clone();
            if let Some(warning) = &self.candidate_regression_warning
                && !errors.contains(warning)
            {
                errors.push(warning.clone());
            }
            (
                Some(best_failed.to_string()),
                Some("validation_errors".to_string()),
                errors,
            )
        } else if let Some(in_flight) = self.in_flight_flowscript.as_deref() {
            (
                Some(in_flight.to_string()),
                Some("edit_interrupted".to_string()),
                vec![
                    "The provider or transport ended while the validator was running; resubmit this complete draft and repair any returned diagnostics."
                        .to_string(),
                ],
            )
        } else {
            (
                self.last_flowscript.clone(),
                self.last_status.clone(),
                self.last_errors.clone(),
            )
        };
        WorkflowToolLoopSnapshot {
            queued: self.queued,
            last_flowscript: retained_flowscript,
            last_declarations: self.last_declarations.clone(),
            declaration_lookup_complete: self.initial_declaration_lookup_complete,
            unresolved_declaration_queries: self.unresolved_declaration_queries.clone(),
            last_repair_declarations: self.last_repair_declarations.clone(),
            last_status: retained_status,
            last_errors: retained_errors,
            edit_attempts: self.edit_attempts,
            flowscript_operation_attempts: self.flowscript_operation_attempts,
            stalled_edit_attempts: self.stalled_edit_attempts,
            flowscript_commit_attempts: self.flowscript_commit_attempts,
            exhausted_budget: self.exhausted_budget(),
            last_structured_diagnostics: self.last_structured_diagnostics.clone(),
            last_review_notes: self.last_review_notes,
            modular_fallback: self
                .queued
                .then(|| self.pending_modular_fallback.clone())
                .flatten(),
            retained_full_source: self
                .flowscript_draft_retained
                .then_some(self.last_flowscript.as_deref())
                .flatten()
                .or_else(|| self.repair_tracker.best_failed_source())
                .or(self.in_flight_flowscript.as_deref())
                .map(str::to_string),
            flowscript_draft_id: self.flowscript_draft_id.clone(),
            flowscript_draft_retained: self.flowscript_draft_retained,
            flowscript_revision: self.flowscript_revision,
            typed_draft_id: self.typed_draft_id.clone(),
            typed_draft_retained: self.typed_draft_retained,
            typed_revision: self.typed_revision,
            typed_operation_attempts: self.typed_operation_attempts,
            typed_operation_budget: typed_ir_operation_budget(self.typed_expected_modules),
            typed_stalled_attempts: self.typed_stalled_attempts,
            typed_missing_modules: self.typed_missing_modules.clone(),
            mutation_path: self.mutation_path,
        }
    }

    fn finish_interrupted_phase(&mut self) {
        if !self.edit_in_flight {
            return;
        }
        self.edit_in_flight = false;
        let message = match self.mutation_path {
            Some(WorkflowMutationPath::FlowScript) if self.flowscript_draft_retained => format!(
                "The provider or transport interrupted a FlowScript draft operation. Retained draft {} remains resumable at revision {}; continue that exact source revision in the next phase.",
                self.flowscript_draft_id.as_deref().unwrap_or("<unknown>"),
                self.flowscript_revision
                    .map(|revision| revision.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string())
            ),
            Some(WorkflowMutationPath::TypedIr) if self.typed_draft_retained => format!(
                "The provider or transport interrupted a typed-IR operation. The retained draft {} remains resumable at revision {}; continue from that exact revision in the next phase.",
                self.typed_draft_id.as_deref().unwrap_or("<unknown>"),
                self.typed_revision
                    .map(|revision| revision.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string())
            ),
            Some(WorkflowMutationPath::TypedIr) => "The provider or transport interrupted a typed-IR operation before draft retention could be confirmed. Do not claim a resumable draft unless the next tool response or host recovery context supplies its revision.".to_string(),
            Some(WorkflowMutationPath::FlowScript) => "The provider or transport interrupted a FlowScript operation before retained source could be confirmed. Do not claim a resumable draft unless the next tool response or host recovery context supplies its revision.".to_string(),
            _ => "The provider or transport interrupted the edit validator. The complete submitted draft was retained and must be resubmitted before any reduced fallback.".to_string(),
        };
        if let Some(interrupted) = self.in_flight_flowscript.take() {
            if self.repair_tracker.record_failed(&interrupted) {
                self.best_failed_errors = vec![message.clone()];
                self.candidate_regression_warning = None;
            }
            self.last_flowscript = Some(interrupted);
        }
        self.last_status = Some("edit_interrupted".to_string());
        self.last_errors = vec![message];
        self.pending_modular_fallback = None;
        self.declarations_since_edit = 0;
    }

    /// Name the loop budget that would refuse further source work on arrival, if any. `None`
    /// means the next phase can still dispatch operations.
    fn exhausted_budget(&self) -> Option<String> {
        if self.queued {
            return None;
        }
        if self.stalled_edit_attempts >= MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS {
            return Some(format!(
                "stalled repair progress ({}/{} repeated compiler states)",
                self.stalled_edit_attempts, MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS
            ));
        }
        if self.flowscript_operation_attempts >= MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS {
            return Some(format!(
                "FlowScript source operation budget ({}/{})",
                self.flowscript_operation_attempts, MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS
            ));
        }
        if self.edit_attempts >= MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS {
            return Some(format!(
                "FlowScript check budget ({}/{})",
                self.edit_attempts, MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS
            ));
        }
        if self.flowscript_commit_attempts >= MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS {
            return Some(format!(
                "commit retry budget ({}/{})",
                self.flowscript_commit_attempts, MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS
            ));
        }
        if self.typed_stalled_attempts >= MAX_EXTERNAL_TYPED_IR_STALLED_ATTEMPTS {
            return Some(format!(
                "typed-IR stalled repair progress ({}/{})",
                self.typed_stalled_attempts, MAX_EXTERNAL_TYPED_IR_STALLED_ATTEMPTS
            ));
        }
        let typed_budget = typed_ir_operation_budget(self.typed_expected_modules);
        if self.mutation_path == Some(WorkflowMutationPath::TypedIr)
            && self.typed_operation_attempts >= typed_budget
        {
            return Some(format!(
                "typed-IR operation budget ({}/{})",
                self.typed_operation_attempts, typed_budget
            ));
        }
        None
    }

    /// Make one host-granted continuation phase executable again. The stall detector restarts
    /// from a clean signature set and exhausted counters receive a small bounded headroom, so the
    /// continuation instructions ("repair the retained draft") are not refused-on-arrival by the
    /// budgets the previous phase burned.
    fn grant_continuation_slice(&mut self) {
        if self.queued {
            return;
        }
        self.stalled_edit_attempts = 0;
        self.flowscript_seen_repair_signatures.clear();
        self.typed_stalled_attempts = 0;
        self.typed_seen_repair_signatures.clear();
        if self.flowscript_operation_attempts >= MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS {
            self.flowscript_operation_attempts = MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS
                .saturating_sub(EXTERNAL_CONTINUATION_OPERATION_HEADROOM);
        }
        if self.edit_attempts >= MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS {
            self.edit_attempts = MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS
                .saturating_sub(EXTERNAL_CONTINUATION_CHECK_HEADROOM);
        }
        if self.flowscript_commit_attempts >= MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS {
            self.flowscript_commit_attempts =
                MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS.saturating_sub(1);
        }
        let typed_budget = typed_ir_operation_budget(self.typed_expected_modules);
        if self.typed_operation_attempts >= typed_budget {
            self.typed_operation_attempts =
                typed_budget.saturating_sub(EXTERNAL_CONTINUATION_OPERATION_HEADROOM);
        }
    }

    fn record_flowscript_repair_progress(
        &mut self,
        status: Option<&str>,
        diagnostics: &[String],
        requires_repair: bool,
    ) {
        if !requires_repair {
            self.stalled_edit_attempts = 0;
            if matches!(status, Some("valid" | "queued" | "already_queued")) {
                self.flowscript_seen_repair_signatures.clear();
            }
            return;
        }

        let fingerprint =
            flowscript_repair_fingerprint(status, diagnostics, &self.last_structured_diagnostics);
        if self.flowscript_seen_repair_signatures.insert(fingerprint) {
            self.stalled_edit_attempts = 0;
        } else {
            self.stalled_edit_attempts = self.stalled_edit_attempts.saturating_add(1);
        }
        self.declarations_since_edit = 0;
    }
}

const RUN_SUMMARY_EVENT_KIND: &str = "run_summary";

fn workflow_run_summary_budget_entry(used: u64, limit: u64) -> serde_json::Value {
    serde_json::json!({ "used": used, "limit": limit })
}

/// Build the single structured per-run summary payload from state the workflow loop already
/// tracks. The frame rides the existing `tool_end` stream tag (without a `tool_call_id`, which the
/// process-step views ignore) so every provider path reuses one pipe, and the debug report pins it
/// at maximum retention.
fn workflow_run_summary_payload(
    outcome: &str,
    provider: &str,
    model: &str,
    duration_ms: u64,
    phases: u32,
    continuations_used: u32,
    continuations_limit: u32,
    snapshot: Option<&WorkflowToolLoopSnapshot>,
    applied_commands: usize,
) -> serde_json::Value {
    let mut diagnostics_by_code = std::collections::BTreeMap::<String, u64>::new();
    for entry in snapshot
        .map(|snapshot| snapshot.last_structured_diagnostics.as_slice())
        .unwrap_or_default()
    {
        let Some(code) = entry.get("code").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let occurrences = entry
            .get("occurrences")
            .and_then(serde_json::Value::as_u64)
            .filter(|count| *count > 0)
            .unwrap_or(1);
        let total = diagnostics_by_code.entry(code.to_string()).or_default();
        *total = total.saturating_add(occurrences);
    }
    let retained_draft = snapshot
        .and_then(|snapshot| {
            if snapshot.flowscript_draft_retained {
                snapshot
                    .flowscript_draft_id
                    .as_deref()
                    .map(|id| (id, snapshot.flowscript_revision))
            } else if snapshot.typed_draft_retained {
                snapshot
                    .typed_draft_id
                    .as_deref()
                    .map(|id| (id, snapshot.typed_revision))
            } else {
                None
            }
        })
        .map(|(id, revision)| serde_json::json!({ "id": id, "revision": revision }))
        .unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "kind": RUN_SUMMARY_EVENT_KIND,
        "tool": RUN_SUMMARY_EVENT_KIND,
        "status": if matches!(outcome, "provider_failure" | "incomplete") { "error" } else { "done" },
        "outcome": outcome,
        "provider": provider,
        "model": model,
        "duration_ms": duration_ms,
        "phases": phases,
        "budget": {
            "checks": workflow_run_summary_budget_entry(
                snapshot.map_or(0, |snapshot| u64::from(snapshot.edit_attempts)),
                u64::from(MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS),
            ),
            "source_ops": workflow_run_summary_budget_entry(
                snapshot.map_or(0, |snapshot| u64::from(snapshot.flowscript_operation_attempts)),
                u64::from(MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS),
            ),
            "commits": workflow_run_summary_budget_entry(
                snapshot.map_or(0, |snapshot| u64::from(snapshot.flowscript_commit_attempts)),
                u64::from(MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS),
            ),
            "stalled": workflow_run_summary_budget_entry(
                snapshot.map_or(0, |snapshot| u64::from(snapshot.stalled_edit_attempts)),
                u64::from(MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS),
            ),
            "continuations": workflow_run_summary_budget_entry(
                u64::from(continuations_used),
                u64::from(continuations_limit),
            ),
        },
        "diagnostics_by_code": diagnostics_by_code,
        "retained_draft": retained_draft,
        "review_notes": snapshot.map_or(0, |snapshot| snapshot.last_review_notes),
        "applied_commands": applied_commands,
    })
}

/// Emits exactly one `run_summary` frame when a FlowPilot run reaches any terminal path. The
/// emission is Drop-based so early error returns, cancellations, and provider failures cannot
/// skip it; success paths set the resolved outcome and applied-command count before the emitter
/// goes out of scope.
struct WorkflowRunSummaryEmitter {
    channel: Channel<String>,
    parent_request_id: Option<String>,
    provider: String,
    model: String,
    started: Instant,
    cancellation: CancellationToken,
    workflow_state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
    phases: u32,
    continuations_used: u32,
    continuations_limit: u32,
    budget_incomplete: bool,
    outcome: Option<&'static str>,
    applied_commands: usize,
}

impl WorkflowRunSummaryEmitter {
    fn new(
        channel: Channel<String>,
        parent_request_id: Option<String>,
        provider: &str,
        model: &str,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            channel,
            parent_request_id,
            provider: provider.to_string(),
            model: model.to_string(),
            started: Instant::now(),
            cancellation,
            workflow_state: None,
            phases: 0,
            continuations_used: 0,
            continuations_limit: u32::from(MAX_EXTERNAL_WORKFLOW_CONTINUATIONS),
            budget_incomplete: false,
            outcome: None,
            applied_commands: 0,
        }
    }

    fn attach_workflow_state(&mut self, state: Option<Arc<StdMutex<WorkflowToolLoopState>>>) {
        self.workflow_state = state;
    }

    fn set_continuation_limit(&mut self, limit: u32) {
        self.continuations_limit = limit;
    }

    fn record_phase(&mut self) {
        self.phases = self.phases.saturating_add(1);
    }

    fn record_continuation(&mut self) {
        self.continuations_used = self.continuations_used.saturating_add(1);
    }

    fn mark_budget_incomplete(&mut self) {
        self.budget_incomplete = true;
    }

    fn set_applied_commands(&mut self, applied_commands: usize) {
        self.applied_commands = applied_commands;
    }

    fn set_outcome(&mut self, outcome: &'static str) {
        self.outcome = Some(outcome);
    }

    fn snapshot(&self) -> Option<WorkflowToolLoopSnapshot> {
        self.workflow_state
            .as_ref()
            .and_then(|state| state.lock().ok().map(|state| state.snapshot()))
    }

    /// Classify the terminal outcome from state the run already tracks. Queued work outranks a
    /// trailing provider error (the mutation was handed off); an edit request that ends cleanly
    /// without queueing anything is honestly incomplete, not completed.
    fn resolve_outcome(&mut self, run_error: bool, workflow_edit_request: bool) {
        let queued = self.snapshot().is_some_and(|snapshot| snapshot.queued);
        self.outcome = Some(if self.cancellation.is_cancelled() {
            "cancelled"
        } else if queued {
            "committed"
        } else if self.budget_incomplete {
            "incomplete"
        } else if run_error {
            "provider_failure"
        } else if workflow_edit_request && self.workflow_state.is_some() {
            "incomplete"
        } else {
            "completed"
        });
    }
}

impl Drop for WorkflowRunSummaryEmitter {
    fn drop(&mut self) {
        let snapshot = self.snapshot();
        let outcome = self.outcome.unwrap_or_else(|| {
            // Unset outcome means the run left through an early error return (or panic unwind).
            if self.cancellation.is_cancelled() {
                "cancelled"
            } else if snapshot.as_ref().is_some_and(|snapshot| snapshot.queued) {
                "committed"
            } else if self.budget_incomplete {
                "incomplete"
            } else {
                "provider_failure"
            }
        });
        let payload = workflow_run_summary_payload(
            outcome,
            &self.provider,
            &self.model,
            u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            self.phases.max(1),
            self.continuations_used,
            self.continuations_limit,
            snapshot.as_ref(),
            self.applied_commands,
        );
        send_correlated_stream_json_event(
            &self.channel,
            "tool_end",
            &payload,
            self.parent_request_id.as_deref(),
        );
    }
}

fn workflow_state_has_retained_candidate(
    state: Option<&Arc<StdMutex<WorkflowToolLoopState>>>,
) -> bool {
    let Some(state) = state else {
        return false;
    };
    match state.lock() {
        Ok(state) => {
            state.queued
                || state.last_flowscript.is_some()
                || state.in_flight_flowscript.is_some()
                || state.flowscript_draft_retained
                || state.typed_draft_retained
        }
        // A poisoned loop mutex means the lifecycle may have been interrupted after retaining a
        // draft. Assume there is recoverable work so outer retry logic cannot silently discard it.
        Err(_) => true,
    }
}

fn workflow_waiting_for_initial_source_checkpoint(state: &WorkflowToolLoopState) -> bool {
    state.initial_declaration_lookup_usable
        && !state.queued
        && !state.flowscript_draft_retained
        && !state.typed_draft_retained
        && state.flowscript_operation_attempts == 0
        && state.typed_operation_attempts == 0
}

/// Outcome of preparing the workflow loop budget for one SDK idle continuation.
#[derive(Debug, PartialEq, Eq)]
enum IdleContinuationBudget {
    /// No budget is exhausted; the continuation instructions are executable as-is.
    Executable,
    /// The named budget was exhausted and received the same bounded continuation slice the
    /// external phase loop grants, so the instructions are not refused on arrival.
    SliceGranted(String),
    /// Reason the continuation must not be sent: the budget already received a slice for this
    /// exact state and burned it again, or the loop state is unusable. Another continuation
    /// would arrive equally dead; stop honestly.
    Terminal(String),
}

fn prepare_sdk_idle_continuation_budget(
    workflow_state: Option<&Arc<StdMutex<WorkflowToolLoopState>>>,
    previous_exhausted_budget: Option<&str>,
) -> IdleContinuationBudget {
    let Some(state) = workflow_state else {
        return IdleContinuationBudget::Executable;
    };
    let Ok(mut state) = state.lock() else {
        return IdleContinuationBudget::Terminal(
            "the host workflow lifecycle state is unavailable".to_string(),
        );
    };
    let Some(exhausted) = state.exhausted_budget() else {
        return IdleContinuationBudget::Executable;
    };
    if Some(exhausted.as_str()) == previous_exhausted_budget {
        return IdleContinuationBudget::Terminal(format!(
            "the {exhausted} was exhausted again after its granted continuation slice"
        ));
    }
    state.grant_continuation_slice();
    IdleContinuationBudget::SliceGranted(exhausted)
}

fn workflow_loop_result(payload: serde_json::Value, is_error: bool) -> rmcp::model::CallToolResult {
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    if is_error {
        rmcp::model::CallToolResult::error(vec![rmcp::model::Content::text(text)])
    } else {
        rmcp::model::CallToolResult::success(vec![rmcp::model::Content::text(text)])
    }
}

fn workflow_loop_state_unavailable_result() -> rmcp::model::CallToolResult {
    workflow_loop_result(
        serde_json::json!({
            "status": "internal_state_unavailable",
            "code": "WORKFLOW_LOOP_STATE_UNAVAILABLE",
            "retryable": false,
            "next_action": "stop_and_resume_in_new_run",
            "message": "The host workflow lifecycle state is unavailable. No tool operation was dispatched; stop this run so a fresh host process can recover any retained draft safely."
        }),
        true,
    )
}

fn workflow_tool_preflight_sdk(
    state: &Arc<StdMutex<WorkflowToolLoopState>>,
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<copilot_sdk::ToolResultObject> {
    let result = workflow_database_setup_preflight(state, tool_name, args)
        .or_else(|| workflow_predraft_context_preflight(state, tool_name))
        .or_else(|| workflow_tool_preflight_with_args(state, tool_name, args))
        .or_else(|| workflow_candidate_preflight(state, tool_name, args))?;
    let message = result
        .content
        .iter()
        .filter_map(|content| match &content.raw {
            rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if result.is_error == Some(true) {
        Some(copilot_sdk::ToolResultObject::error(message))
    } else {
        Some(copilot_sdk::ToolResultObject::text(message))
    }
}

/// Database schema setup is useful, but it must not consume the mutation turn before the board
/// exists. Prompt guidance alone is not sufficient for code agents: a premature `create_table`
/// can open an approval dialog and wait for minutes while no recoverable FlowScript has ever been
/// submitted. Allow read-only database inspection, but require a queued board draft before schema
/// creation. The same guard is used by SDK and MCP providers.
fn workflow_database_setup_preflight(
    state: &Arc<StdMutex<WorkflowToolLoopState>>,
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<rmcp::model::CallToolResult> {
    if tool_name != "database_tool"
        || args.get("operation").and_then(serde_json::Value::as_str) != Some("create_table")
    {
        return None;
    }

    let board_draft_queued = match state.lock() {
        Ok(state) => state.queued,
        Err(_) => return Some(workflow_loop_state_unavailable_result()),
    };
    if board_draft_queued {
        return None;
    }

    Some(workflow_loop_result(
        serde_json::json!({
            "status": "deferred",
            "code": "board_draft_required_before_database_setup",
            "retryable": true,
            "next_action": "commit_workflow_draft",
            "message": "Submit and queue the complete board through commit_flowscript before creating database tables. The legacy edit_flowscript path is also accepted for compatibility. The schema request was not dispatched, no approval was opened, and no network request was made. Read-only table/schema inspection remains available."
        }),
        false,
    ))
}

/// Keep ancillary context reads from consuming the entire delegated run before any recoverable
/// source exists. The first few database/UI/storage inspections remain available for authoritative
/// context, but after that the specialist must retain a full-shape draft and let compiler
/// diagnostics drive any additional focused discovery.
fn workflow_predraft_context_preflight(
    state: &Arc<StdMutex<WorkflowToolLoopState>>,
    tool_name: &str,
) -> Option<rmcp::model::CallToolResult> {
    if !matches!(tool_name, "database_tool" | "ui_inspect" | "storage_tool") {
        return None;
    }

    let Ok(mut state) = state.lock() else {
        return Some(workflow_loop_state_unavailable_result());
    };
    if state.queued || state.flowscript_draft_retained || state.typed_draft_retained {
        return None;
    }
    if state.predraft_context_reads >= MAX_EXTERNAL_PREDRAFT_CONTEXT_READS {
        return Some(workflow_loop_result(
            serde_json::json!({
                "status": "predraft_inspection_budget_exhausted",
                "code": "PREDRAFT_INSPECTION_BUDGET_EXHAUSTED",
                "retryable": true,
                "next_action": if state.initial_declaration_lookup_usable {
                    "write_flowscript"
                } else if state.current_reads > 0 {
                    "get_declarations"
                } else {
                    "get_current_flowscript_then_get_declarations"
                },
                "inspection_calls": state.predraft_context_reads,
                "inspection_budget": MAX_EXTERNAL_PREDRAFT_CONTEXT_READS,
                "message": "The bounded ancillary inspection budget is exhausted before a recoverable workflow draft exists. Reuse the database, UI, and storage context already returned. After one usable declaration batch, call write_flowscript immediately with a full-shape draft; do not repeat or exhaustively inventory schemas and pages."
            }),
            false,
        ));
    }
    state.predraft_context_reads = state.predraft_context_reads.saturating_add(1);
    None
}

fn guard_sdk_workflow_tools(
    tools: Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)>,
    state: Arc<StdMutex<WorkflowToolLoopState>>,
) -> Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)> {
    let operation_gate = Arc::new(StdMutex::new(()));
    tools
        .into_iter()
        .map(|(tool, handler)| {
            let guarded_state = state.clone();
            let guarded_name = tool.name.clone();
            let operation_gate = operation_gate.clone();
            let guarded_handler: copilot_sdk::ToolHandler = Arc::new(move |called_name, args| {
                // The SDK may dispatch sibling tool calls concurrently. Hold a lifecycle gate
                // through preflight, handler execution, and record so a late completion cannot
                // clear or overwrite the state of a newer typed/raw mutation.
                let _operation_guard = if is_order_sensitive_workflow_tool(&guarded_name) {
                    match operation_gate.try_lock() {
                        Ok(guard) => Some(guard),
                        // A handler that panicked while holding the gate poisons it permanently;
                        // its operation was already aborted below. Refusing every later mutation
                        // with the retryable "wait" answer would strand the run, so recover the
                        // gate instead of failing closed forever.
                        Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                            operation_gate.clear_poison();
                            Some(poisoned.into_inner())
                        }
                        Err(std::sync::TryLockError::WouldBlock) => {
                            return copilot_sdk::ToolResultObject::error(
                                "Another order-sensitive workflow operation is still running. Wait for its retained revision/status before issuing the next mutation.",
                            );
                        }
                    }
                } else {
                    None
                };
                // Catch panics before they unwind past the held gate guard, and route them
                // through the same abort/cleanup as an MCP worker failure so `edit_in_flight`
                // cannot stay stuck for the rest of the session.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Some(result) =
                        workflow_tool_preflight_sdk(&guarded_state, &guarded_name, args)
                    {
                        return result;
                    }
                    let mut result = handler(called_name, args);
                    workflow_tool_record(
                        &guarded_state,
                        &guarded_name,
                        args,
                        &result.text_result_for_llm,
                    );
                    annotate_modular_fallback_result(&guarded_state, &guarded_name, &mut result);
                    suppress_unchanged_flowscript_source_echo(&guarded_name, args, &mut result);
                    result
                }));
                match outcome {
                    Ok(result) => result,
                    Err(panic) => {
                        let message = format!(
                            "FlowPilot SDK tool '{guarded_name}' failed: {}",
                            panic_payload_message(panic.as_ref())
                        );
                        workflow_tool_abort(&guarded_state, &guarded_name, &message);
                        copilot_sdk::ToolResultObject::error(message)
                    }
                }
            });
            (tool, guarded_handler)
        })
        .collect()
}

fn panic_payload_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("the tool handler panicked without a message")
}

/// Attach the owning SDK chat cancellation token to synchronous tool handlers. The Copilot SDK
/// retains handlers in its session and may still be executing one after the async event loop is
/// cancelled; the frontend bridge reads this thread-local scope to stop its bounded per-tool wait
/// and emit cancellation to the webview.
fn scope_sdk_tool_handlers(
    tools: Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)>,
    cancellation: CancellationToken,
) -> Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)> {
    tools
        .into_iter()
        .map(|(tool, handler)| {
            let handler_cancellation = cancellation.clone();
            let scoped_handler: copilot_sdk::ToolHandler = Arc::new(move |called_name, args| {
                if handler_cancellation.is_cancelled() {
                    return copilot_sdk::ToolResultObject::error(
                        "The owning FlowPilot run was cancelled before this tool could execute.",
                    );
                }
                super::frontend_tool_bridge::with_frontend_tool_execution_scope(
                    handler_cancellation.clone(),
                    None,
                    || handler(called_name, args),
                )
            });
            (tool, scoped_handler)
        })
        .collect()
}

fn is_workflow_loop_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "catalog_search"
            | "list_board_nodes"
            | "get_node_details"
            | "get_unconfigured_nodes"
            | "get_current_flowscript"
            | "get_declarations"
            | "write_flowscript"
            | "patch_flowscript"
            | "check_flowscript"
            | "commit_flowscript"
            // Compatibility-only typed IR tools. New model surfaces do not advertise these.
            | "plan_flow_ir"
            | "begin_flow_ir_draft"
            | "update_flow_ir_draft"
            | "upsert_flow_ir_module"
            | "validate_flow_ir_draft"
            | "commit_flow_ir_draft"
            | "edit_flowscript"
            | "emit_commands"
    )
}

fn is_flowpilot_mutation_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "emit_commands"
            | "edit_flowscript"
            | "write_flowscript"
            | "patch_flowscript"
            | "check_flowscript"
            | "commit_flowscript"
            | "begin_flow_ir_draft"
            | "update_flow_ir_draft"
            | "upsert_flow_ir_module"
            | "validate_flow_ir_draft"
            | "commit_flow_ir_draft"
            | "emit_ui"
    )
}

fn is_workflow_commit_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "emit_commands" | "edit_flowscript" | "commit_flowscript" | "commit_flow_ir_draft"
    )
}

fn is_flowscript_draft_operation_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "write_flowscript" | "patch_flowscript" | "check_flowscript" | "commit_flowscript"
    )
}

fn is_typed_ir_operation_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "plan_flow_ir"
            | "begin_flow_ir_draft"
            | "update_flow_ir_draft"
            | "upsert_flow_ir_module"
            | "validate_flow_ir_draft"
            | "commit_flow_ir_draft"
    )
}

fn is_order_sensitive_workflow_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "write_flowscript"
            | "patch_flowscript"
            | "check_flowscript"
            | "commit_flowscript"
            | "plan_flow_ir"
            | "begin_flow_ir_draft"
            | "update_flow_ir_draft"
            | "upsert_flow_ir_module"
            | "validate_flow_ir_draft"
            | "commit_flow_ir_draft"
            | "edit_flowscript"
            | "emit_commands"
    )
}

fn typed_ir_module_count_hint(tool_name: &str, args: &serde_json::Value) -> Option<usize> {
    let array_len = |value: Option<&serde_json::Value>| value?.as_array().map(Vec::len);
    match tool_name {
        "plan_flow_ir" => [
            array_len(args.get("modules")),
            array_len(args.get("module_estimates")),
        ]
        .into_iter()
        .flatten()
        .max(),
        "begin_flow_ir_draft" | "update_flow_ir_draft" => [
            array_len(args.get("expected_modules")),
            array_len(
                args.get("capability_plan")
                    .and_then(|plan| plan.get("modules")),
            ),
        ]
        .into_iter()
        .flatten()
        .max(),
        "upsert_flow_ir_module" | "validate_flow_ir_draft" | "commit_flow_ir_draft" => Some(0),
        _ => None,
    }
}

fn typed_ir_operation_budget(expected_modules: usize) -> u16 {
    u16::try_from(expected_modules)
        .unwrap_or(u16::MAX)
        .saturating_mul(3)
        .saturating_add(8)
        .clamp(
            MIN_EXTERNAL_TYPED_IR_OPERATION_BUDGET,
            MAX_EXTERNAL_TYPED_IR_OPERATION_BUDGET,
        )
}

fn typed_ir_operation_target(tool_name: &str, args: &serde_json::Value) -> String {
    match tool_name {
        "plan_flow_ir" => "$plan".to_string(),
        "begin_flow_ir_draft" => "$draft".to_string(),
        "update_flow_ir_draft" => "$header".to_string(),
        "upsert_flow_ir_module" => args
            .pointer("/module/name")
            .and_then(serde_json::Value::as_str)
            .map(|name| format!("module:{name}"))
            .unwrap_or_else(|| "module:<invalid>".to_string()),
        "validate_flow_ir_draft" => "$validation".to_string(),
        "commit_flow_ir_draft" => "$commit".to_string(),
        _ => tool_name.to_string(),
    }
}

fn typed_ir_result_proves_retained_draft(parsed: &serde_json::Value) -> bool {
    let revision_retained = parsed
        .get("revision")
        .and_then(serde_json::Value::as_u64)
        .is_some();
    let status = parsed.get("status").and_then(serde_json::Value::as_str);
    revision_retained
        && matches!(
            status,
            Some(
                "draft_started"
                    | "draft_updated"
                    | "draft_needs_repair"
                    | "module_validated"
                    | "module_needs_repair"
                    | "draft_valid"
                    | "scope_reduction_blocked"
                    | "candidate_regression"
                    | "resource_limit_rejected"
                    | "queued"
                    | "already_queued"
                    | "validation_errors"
                    | "infeasible"
                    | "revision_conflict"
                    | "error"
            )
        )
}

/// Enforce a short, edit-first workflow loop for external code agents. Prompt guidance alone is
/// insufficient for CLIs whose default code-agent behavior keeps searching: this per-run gate
/// makes the productive path deterministic while leaving read-only/global sessions untouched.
fn normalize_declaration_signature(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        || !value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
    {
        return None;
    }
    let normalized = value.to_ascii_lowercase();
    (!matches!(
        normalized.as_str(),
        "get_declarations" | "edit_flowscript" | "flowscript" | "function" | "event" | "events"
    ))
    .then_some(normalized)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DeclarationRepairHints {
    exact_symbols: HashSet<String>,
    topics: HashSet<String>,
}

impl DeclarationRepairHints {
    fn is_empty(&self) -> bool {
        self.exact_symbols.is_empty() && self.topics.is_empty()
    }

    fn exposed_targets(&self) -> Vec<String> {
        let mut targets = self
            .exact_symbols
            .iter()
            .map(|symbol| format!("symbol:{symbol}"))
            .chain(self.topics.iter().map(|topic| format!("topic:{topic}")))
            .collect::<Vec<_>>();
        targets.sort();
        targets
    }
}

/// Extract only repairable catalog evidence from reconcile diagnostics. In addition to explicit
/// missing-declaration messages, pin and type diagnostics name the exact node whose declaration is
/// needed. Comparison failures also justify narrowly scoped equality/conversion discovery even
/// when the reconciler cannot know the eventual catalog function name.
fn diagnostic_declaration_repair_hints(diagnostics: &[String]) -> DeclarationRepairHints {
    let mut hints = DeclarationRepairHints::default();
    for diagnostic in diagnostics {
        let lower = diagnostic.to_ascii_lowercase();

        let parts = diagnostic.split('`').collect::<Vec<_>>();
        for index in (1..parts.len()).step_by(2) {
            let context = parts[index - 1].trim_end().to_ascii_lowercase();
            let names_catalog_symbol = ["node", "on", "call", "declaration"]
                .iter()
                .any(|marker| context.ends_with(marker));
            if names_catalog_symbol
                && let Some(symbol) = normalize_declaration_signature(parts[index])
            {
                hints.exact_symbols.insert(symbol);
            }
        }

        // Some provider wrappers flatten the canonical backtick formatting. The text before this
        // exact diagnostic phrase is still an explicit signature, not a broad intent guess.
        if let Some(end) = lower.find(" does not match a catalog declaration")
            && let Some(candidate) = diagnostic[..end].split_whitespace().next_back()
            && let Some(signature) =
                normalize_declaration_signature(candidate.trim_matches(|character: char| {
                    !character.is_ascii_alphanumeric() && character != '_'
                }))
        {
            hints.exact_symbols.insert(signature);
        }

        if let Some(candidates) = lower.split("candidates are").nth(1) {
            for candidate in candidates
                .split(|character: char| {
                    character == ',' || character == ';' || character.is_whitespace()
                })
                .filter_map(|candidate| {
                    normalize_declaration_signature(candidate.trim_matches(|character: char| {
                        !character.is_ascii_alphanumeric() && character != '_'
                    }))
                })
            {
                hints.exact_symbols.insert(candidate);
            }
        }

        let comparison_failure = lower.contains("binary comparison")
            || lower.contains("ambiguous operand type")
            || lower.contains("incompatible operand type")
            || lower.contains("two-input catalog node");
        if comparison_failure {
            hints.topics.insert("comparison".to_string());
        }
        if comparison_failure
            && (lower.contains("generic")
                || lower.contains("operand type")
                || lower.contains("incompatible"))
        {
            hints.topics.insert("type_conversion".to_string());
        }
        if lower.contains("string") && (lower.contains("input pin") || lower.contains("argument")) {
            hints.topics.insert("string_operations".to_string());
        }
    }
    hints
}

fn declaration_lookup_queries(args: &serde_json::Value) -> Vec<&str> {
    let mut queries = Vec::new();
    if let Some(query) = args.get("query").and_then(serde_json::Value::as_str) {
        queries.push(query);
    }
    if let Some(batch) = args.get("queries").and_then(serde_json::Value::as_array) {
        queries.extend(batch.iter().filter_map(serde_json::Value::as_str));
    }
    queries
}

fn declaration_query_matches_signature(query: &str, signature: &str) -> bool {
    let compact_query = query
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect::<String>();
    let compact_signature = signature
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    !compact_query.is_empty() && compact_query.contains(&compact_signature)
}

fn declaration_repair_query_is_bounded(query: &str) -> bool {
    let normalized = query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    query.len() <= MAX_REPAIR_DECLARATION_QUERY_BYTES
        && query.split_whitespace().count() <= 16
        && ![
            "entire catalog",
            "whole catalog",
            "all catalog",
            "all nodes",
            "every node",
            "everything",
            "broad search",
            "search the catalog",
        ]
        .iter()
        .any(|phrase| normalized.contains(phrase))
}

fn declaration_repair_query_keys(query: &str, hints: &DeclarationRepairHints) -> HashSet<String> {
    let mut keys = hints
        .exact_symbols
        .iter()
        .filter(|symbol| declaration_query_matches_signature(query, symbol))
        .map(|symbol| format!("symbol:{symbol}"))
        .collect::<HashSet<_>>();
    let compact = query
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect::<String>();

    if hints.topics.contains("comparison")
        && ["equal", "compare", "comparison", "inequal", "sametext"]
            .iter()
            .any(|term| compact.contains(term))
    {
        keys.insert("topic:comparison".to_string());
    }
    if hints.topics.contains("type_conversion")
        && [
            "convert",
            "conversion",
            "cast",
            "coerce",
            "tostring",
            "tobool",
            "toint",
        ]
        .iter()
        .any(|term| compact.contains(term))
    {
        keys.insert("topic:type_conversion".to_string());
    }
    if hints.topics.contains("string_operations")
        && [
            "stringcontains",
            "stringtrim",
            "stringstartswith",
            "stringreplace",
            "contains",
            "trim",
            "startswith",
            "replace",
        ]
        .iter()
        .any(|term| compact.contains(term))
    {
        keys.insert("topic:string_operations".to_string());
    }
    keys
}

fn emit_commands_representation_rejected(args: &serde_json::Value) -> bool {
    let Ok(args) = serde_json::from_value::<EmitCommandsArgs>(args.clone()) else {
        return false;
    };
    let scope = validate_model_facing_emit_commands_scope(&args);
    emit_validation_requires_flowscript(&scope)
}

fn workflow_tool_preflight_with_args(
    state: &Arc<StdMutex<WorkflowToolLoopState>>,
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<rmcp::model::CallToolResult> {
    let Ok(mut state) = state.lock() else {
        return Some(workflow_loop_state_unavailable_result());
    };

    // Board commands are staged until the specialist returns and the host applies them. Executing
    // during a mutation session would therefore test the pre-edit graph and could produce a false
    // green verification result. A later read/verification turn has no workflow guard and can run
    // these tools against the persisted board normally.
    if matches!(
        tool_name,
        "execute_event" | "execute_node" | "query_execution_logs"
    ) {
        return Some(workflow_loop_result(
            serde_json::json!({
                "status": "error",
                "code": "runtime_verification_deferred",
                "retryable": true,
                "next_action": "finish_board_edit_then_run_in_a_later_turn",
                "message": "Runtime verification cannot run inside this board-mutation session because queued commands are not persisted until the turn finishes. Complete the edit, then execute the persisted node/Event and query its run logs in a later turn."
            }),
            true,
        ));
    }

    if state.queued && is_workflow_loop_tool(tool_name) {
        return Some(workflow_loop_result(
            serde_json::json!({
                "status": "already_queued",
                "next_action": "stop",
                "message": "Workflow changes are already queued. Stop workflow tools and return a brief summary. If the user also requested UI, finish it with the UI tool only."
            }),
            false,
        ));
    }

    if state.declaration_lookup_in_flight
        && (tool_name == "get_declarations" || is_order_sensitive_workflow_tool(tool_name))
    {
        return Some(workflow_loop_result(
            serde_json::json!({
                "status": "declaration_lookup_in_flight",
                "code": "DECLARATION_LOOKUP_IN_FLIGHT",
                "retryable": true,
                "next_action": "wait",
                "message": "A declaration batch is already in flight. Wait for its authoritative coverage result before starting another lookup or writing source."
            }),
            false,
        ));
    }

    let requested_path = match tool_name {
        "plan_flow_ir"
        | "begin_flow_ir_draft"
        | "update_flow_ir_draft"
        | "upsert_flow_ir_module"
        | "validate_flow_ir_draft"
        | "commit_flow_ir_draft" => Some(WorkflowMutationPath::TypedIr),
        "edit_flowscript" | "write_flowscript" | "patch_flowscript" | "check_flowscript"
        | "commit_flowscript" => Some(WorkflowMutationPath::FlowScript),
        "emit_commands" if emit_commands_representation_rejected(args) => None,
        "emit_commands" => Some(WorkflowMutationPath::DirectCommands),
        _ => None,
    };
    if tool_name == "emit_commands" && requested_path.is_none() {
        // The command tool itself returns the representation guidance. Do not let a rejected
        // executable command batch reserve an operation lease or claim a mutation path before the
        // model switches to FlowScript.
        return None;
    }
    if let (Some(active), Some(requested)) = (state.mutation_path, requested_path)
        && active != requested
    {
        return Some(workflow_loop_result(
            serde_json::json!({
                "status": "mutation_path_conflict",
                "code": "WORKFLOW_MUTATION_PATH_CONFLICT",
                "retryable": false,
                "next_action": match active {
                    WorkflowMutationPath::TypedIr => "continue_typed_draft",
                    WorkflowMutationPath::FlowScript => "continue_flowscript_draft",
                    WorkflowMutationPath::DirectCommands => "continue_direct_commands",
                },
                "message": "A workflow mutation path is already active for this change. Continue the retained FlowScript source path (or the legacy compatibility path that already owns this run); do not mix mutation representations in one atomic edit."
            }),
            true,
        ));
    }

    if is_flowscript_draft_operation_tool(tool_name) && state.flowscript_draft_retained {
        let requested_draft_id = args.get("draft_id").and_then(serde_json::Value::as_str);
        let exact_draft_id = state.flowscript_draft_id.as_deref();
        let wrong_draft = requested_draft_id.is_some() && requested_draft_id != exact_draft_id;
        let missing_draft = !args.is_null() && requested_draft_id.is_none();
        let expected_revision = args
            .get("expected_revision")
            .and_then(serde_json::Value::as_u64);
        let revision_required = matches!(
            tool_name,
            "patch_flowscript" | "check_flowscript" | "commit_flowscript"
        );
        let wrong_revision =
            revision_required && !args.is_null() && expected_revision != state.flowscript_revision;
        if wrong_draft || missing_draft || wrong_revision {
            return Some(workflow_loop_result(
                serde_json::json!({
                    "status": "retained_revision_required",
                    "code": "FLOWSCRIPT_RETAINED_REVISION_REQUIRED",
                    "retryable": true,
                    "next_action": match state.last_status.as_deref() {
                        Some("valid") => "commit_flowscript",
                        Some("validation_errors" | "error" | "no_changes") => "patch_flowscript",
                        _ => "check_flowscript",
                    },
                    "draft_id": exact_draft_id,
                    "expected_revision": state.flowscript_revision,
                    "message": "This run owns an exact retained FlowScript draft. Continue its host-authorized draft id and revision; a different or stale source session was not dispatched."
                }),
                false,
            ));
        }
    }

    if is_flowscript_draft_operation_tool(tool_name)
        && tool_name != "write_flowscript"
        && !state.flowscript_draft_retained
    {
        return Some(workflow_loop_result(
            serde_json::json!({
                "status": "flowscript_draft_required",
                "code": "FLOWSCRIPT_DRAFT_REQUIRED",
                "retryable": true,
                "next_action": if state.needs_initial_declaration_coverage() {
                    "get_declarations"
                } else {
                    "write_flowscript"
                },
                "message": "No host-authorized FlowScript draft is retained for this run. Start with live declaration coverage and write_flowscript; patch, check, and commit cannot create or guess a draft."
            }),
            false,
        ));
    }

    if state.edit_in_flight && is_order_sensitive_workflow_tool(tool_name) {
        return Some(workflow_loop_result(
            serde_json::json!({
                "status": "edit_in_flight",
                "next_action": "wait",
                "message": "Another order-sensitive workflow operation is still running. Wait for its retained revision/status before issuing the next FlowScript or compatibility mutation."
            }),
            true,
        ));
    }

    let typed_operation = is_typed_ir_operation_tool(tool_name);
    if typed_operation {
        if let Some(module_count) = typed_ir_module_count_hint(tool_name, args) {
            state.typed_expected_modules = state.typed_expected_modules.max(module_count);
        }
    }
    let typed_loop_active =
        state.mutation_path == Some(WorkflowMutationPath::TypedIr) || typed_operation;
    if typed_loop_active && is_workflow_loop_tool(tool_name) {
        let operation_budget = typed_ir_operation_budget(state.typed_expected_modules);
        if state.typed_stalled_attempts >= MAX_EXTERNAL_TYPED_IR_STALLED_ATTEMPTS {
            return Some(workflow_loop_result(
                serde_json::json!({
                    "status": "typed_repair_progress_stalled",
                    "code": "TYPED_IR_REPAIR_PROGRESS_STALLED",
                    "retryable": false,
                    "next_action": if state.typed_draft_retained {
                        "stop_and_resume_retained_draft_in_new_run"
                    } else {
                        "stop_and_report_begin_failure"
                    },
                    "draft_retained": state.typed_draft_retained,
                    "draft_id": state.typed_draft_id.as_deref(),
                    "revision": state.typed_revision,
                    "operation_attempts": state.typed_operation_attempts,
                    "operation_budget": operation_budget,
                    "stalled_attempts": state.typed_stalled_attempts,
                    "remaining_diagnostics": &state.last_errors,
                    "missing_modules": &state.typed_missing_modules,
                    "message": if state.typed_draft_retained {
                        "The same typed module or draft repair has repeated an already-seen diagnostic state. No operation was dispatched. Stop this run and report the retained draft id, revision, missing modules, and remaining diagnostics; a later run can resume that exact draft."
                    } else {
                        "The typed planner/begin loop repeated an already-seen diagnostic state before a draft was retained. No operation was dispatched. Stop this run and report the attempted draft id and remaining diagnostics."
                    }
                }),
                true,
            ));
        }
        if state.typed_operation_attempts >= operation_budget {
            return Some(workflow_loop_result(
                serde_json::json!({
                    "status": "typed_repair_budget_exhausted",
                    "code": "TYPED_IR_OPERATION_BUDGET_EXHAUSTED",
                    "retryable": false,
                    "next_action": if state.typed_draft_retained {
                        "stop_and_resume_retained_draft_in_new_run"
                    } else {
                        "stop_and_report_begin_failure"
                    },
                    "draft_retained": state.typed_draft_retained,
                    "draft_id": state.typed_draft_id.as_deref(),
                    "revision": state.typed_revision,
                    "operation_attempts": state.typed_operation_attempts,
                    "operation_budget": operation_budget,
                    "stalled_attempts": state.typed_stalled_attempts,
                    "remaining_diagnostics": &state.last_errors,
                    "missing_modules": &state.typed_missing_modules,
                    "message": if state.typed_draft_retained {
                        "The module-scaled typed-IR operation budget is exhausted. No operation was dispatched. Stop this run and report the retained draft id, revision, missing modules, and remaining diagnostics; a later run can resume that exact draft."
                    } else {
                        "The typed planner/begin operation budget is exhausted before a draft was retained. No operation was dispatched. Stop this run and report the attempted draft id and remaining diagnostics."
                    }
                }),
                true,
            ));
        }
    }

    let checked_valid_commit =
        tool_name == "commit_flowscript" && state.last_status.as_deref() == Some("valid");
    if is_flowscript_draft_operation_tool(tool_name) && !checked_valid_commit {
        if state.stalled_edit_attempts >= MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS {
            return Some(workflow_loop_result(
                serde_json::json!({
                    "status": "edit_progress_stalled",
                    "code": "FLOWSCRIPT_REPAIR_PROGRESS_STALLED",
                    "retryable": false,
                    "next_action": "stop_and_resume_retained_draft_in_new_run",
                    "draft_retained": state.flowscript_draft_retained,
                    "draft_id": state.flowscript_draft_id.as_deref(),
                    "revision": state.flowscript_revision,
                    "operation_attempts": state.flowscript_operation_attempts,
                    "operation_budget": MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
                    "errors": state.last_errors,
                    "message": "The FlowScript repair loop revisited an already-seen compiler state too many times. No source operation was dispatched. Stop this run and report the retained revision and remaining diagnostics."
                }),
                true,
            ));
        }
        if state.flowscript_operation_attempts >= MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS {
            return Some(workflow_loop_result(
                serde_json::json!({
                    "status": "edit_budget_exhausted",
                    "code": "FLOWSCRIPT_OPERATION_BUDGET_EXHAUSTED",
                    "retryable": false,
                    "next_action": "stop_and_resume_retained_draft_in_new_run",
                    "draft_retained": state.flowscript_draft_retained,
                    "draft_id": state.flowscript_draft_id.as_deref(),
                    "revision": state.flowscript_revision,
                    "operation_attempts": state.flowscript_operation_attempts,
                    "operation_budget": MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
                    "errors": state.last_errors,
                    "message": "The total FlowScript write/patch/check operation budget is exhausted. No source operation was dispatched; the latest retained revision remains available for a later run."
                }),
                true,
            ));
        }
    }

    match tool_name {
        "plan_flow_ir" => {
            state
                .mutation_path
                .get_or_insert(WorkflowMutationPath::TypedIr);
            state.typed_operation_attempts = state.typed_operation_attempts.saturating_add(1);
            None
        }
        "begin_flow_ir_draft"
        | "update_flow_ir_draft"
        | "upsert_flow_ir_module"
        | "validate_flow_ir_draft" => {
            state
                .mutation_path
                .get_or_insert(WorkflowMutationPath::TypedIr);
            state.typed_operation_attempts = state.typed_operation_attempts.saturating_add(1);
            state.edit_in_flight = true;
            None
        }
        "write_flowscript" if state.needs_initial_declaration_coverage() => {
            if state.initial_declaration_attempts >= MAX_INITIAL_DECLARATION_ATTEMPTS {
                return Some(workflow_loop_result(
                    serde_json::json!({
                        "status": "declaration_coverage_exhausted",
                        "code": "DECLARATION_COVERAGE_EXHAUSTED",
                        "retryable": false,
                        "next_action": "stop_and_report_unavailable_capabilities",
                        "attempts": state.initial_declaration_attempts,
                        "unresolved_queries": state.unresolved_declaration_queries,
                        "message": "No bounded initial declaration attempt returned a usable live signature. No FlowScript source write was dispatched; report the unavailable core capabilities instead of guessing names or pins."
                    }),
                    true,
                ));
            }
            Some(workflow_loop_result(
                serde_json::json!({
                    "status": "declaration_lookup_required",
                    "retryable": true,
                    "next_action": "get_declarations",
                    "message": "Before the first FlowScript draft, make one bounded get_declarations batch for the highest-leverage catalog calls needed to establish the end-to-end shape. Do not enumerate every utility operation. After any usable live result, write and retain the full-shape draft immediately; compiler diagnostics authorize focused later lookups."
                }),
                false,
            ))
        }
        "write_flowscript" | "patch_flowscript" => {
            state
                .mutation_path
                .get_or_insert(WorkflowMutationPath::FlowScript);
            state.flowscript_operation_attempts =
                state.flowscript_operation_attempts.saturating_add(1);
            state.edit_in_flight = true;
            if tool_name == "write_flowscript" {
                state.in_flight_flowscript = submitted_flowscript(args).map(str::to_string);
            }
            None
        }
        "check_flowscript"
            if state.stalled_edit_attempts >= MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS =>
        {
            Some(workflow_loop_result(
                serde_json::json!({
                    "status": "edit_progress_stalled",
                    "next_action": "stop",
                    "errors": state.last_errors,
                    "message": "The last FlowScript checks repeated the same unresolved compiler diagnostics. Stop this bounded loop and report those diagnostics; the complete retained source remains resumable."
                }),
                true,
            ))
        }
        "check_flowscript" if state.edit_attempts >= MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS => {
            Some(workflow_loop_result(
                serde_json::json!({
                    "status": "edit_budget_exhausted",
                    "next_action": "stop",
                    "errors": state.last_errors,
                    "message": "The bounded FlowScript check/repair budget is exhausted. Stop broad discovery and report the remaining compiler diagnostics honestly."
                }),
                true,
            ))
        }
        "check_flowscript" => {
            state
                .mutation_path
                .get_or_insert(WorkflowMutationPath::FlowScript);
            state.edit_attempts = state.edit_attempts.saturating_add(1);
            state.flowscript_operation_attempts =
                state.flowscript_operation_attempts.saturating_add(1);
            state.edit_in_flight = true;
            None
        }
        "commit_flowscript"
            if state.last_status.as_deref() != Some("valid")
                && state.stalled_edit_attempts >= MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS =>
        {
            Some(workflow_loop_result(
                serde_json::json!({
                    "status": "edit_progress_stalled",
                    "next_action": "stop",
                    "errors": state.last_errors,
                    "message": "Commit cannot bypass a repeatedly failing FlowScript check. Stop and report the retained revision and its remaining compiler diagnostics."
                }),
                true,
            ))
        }
        "commit_flowscript"
            if state.last_status.as_deref() != Some("valid")
                && state.edit_attempts >= MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS =>
        {
            Some(workflow_loop_result(
                serde_json::json!({
                    "status": "edit_budget_exhausted",
                    "next_action": "stop",
                    "errors": state.last_errors,
                    "message": "Commit requires a valid exact revision, and the bounded FlowScript check budget is exhausted. Nothing was queued."
                }),
                true,
            ))
        }
        // A successful check is the bounded validation attempt. Commit only claims that exact
        // retained revision, so a valid revision remains committable even at the check ceiling.
        "commit_flowscript" => {
            if state.flowscript_commit_attempts >= MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS {
                return Some(workflow_loop_result(
                    serde_json::json!({
                        "status": "commit_retry_budget_exhausted",
                        "code": "FLOWSCRIPT_COMMIT_RETRY_BUDGET_EXHAUSTED",
                        "retryable": false,
                        "next_action": "stop_and_resume_retained_draft_in_new_run",
                        "draft_retained": state.flowscript_draft_retained,
                        "draft_id": state.flowscript_draft_id.as_deref(),
                        "revision": state.flowscript_revision,
                        "attempts": state.flowscript_commit_attempts,
                        "message": "The exact valid revision could not complete its bounded commit attempts. Stop this run without rewriting the checked source; the retained revision can be resumed later."
                    }),
                    true,
                ));
            }
            state
                .mutation_path
                .get_or_insert(WorkflowMutationPath::FlowScript);
            state.flowscript_operation_attempts =
                state.flowscript_operation_attempts.saturating_add(1);
            state.flowscript_commit_attempts = state.flowscript_commit_attempts.saturating_add(1);
            state.edit_in_flight = true;
            None
        }
        "catalog_search" | "list_board_nodes" | "get_node_details" | "get_unconfigured_nodes" => {
            Some(workflow_loop_result(
                serde_json::json!({
                    "status": "discovery_blocked",
                    "next_action": "continue_workflow_draft",
                    "message": "This is a workflow mutation run. Broad catalog/graph discovery is disabled. Read the current FlowScript once, use one bounded get_declarations batch for the highest-leverage calls, then immediately write_flowscript, patch/check the retained source, and commit_flowscript."
                }),
                true,
            ))
        }
        "get_current_flowscript" if state.current_reads >= 1 => Some(workflow_loop_result(
            serde_json::json!({
                "status": "already_returned",
                "next_action": "write_flowscript",
                "message": "The current FlowScript was already returned in this run. Write the complete retained source document; do not fetch it again."
            }),
            true,
        )),
        "get_current_flowscript" => {
            state.current_reads = state.current_reads.saturating_add(1);
            None
        }
        "get_declarations" if state.needs_initial_declaration_coverage() => {
            let queries = declaration_lookup_queries(args);
            if !queries.iter().any(|query| !query.trim().is_empty()) {
                return Some(workflow_loop_result(
                    serde_json::json!({
                        "status": "declaration_batch_required",
                        "retryable": true,
                        "next_action": "get_declarations",
                        "unresolved_queries": state.unresolved_declaration_queries,
                        "message": "The initial declaration lookup must contain focused queries in `query` or `queries`. Submit one bounded batch for the highest-leverage catalog calls needed to establish the end-to-end shape; an empty guidance lookup does not unlock FlowScript authoring."
                    }),
                    false,
                ));
            }
            if !state.unresolved_declaration_queries.is_empty()
                && queries.iter().any(|query| {
                    !query.trim().is_empty()
                        && !state
                            .unresolved_declaration_queries
                            .iter()
                            .any(|unresolved| declaration_queries_are_related(unresolved, query))
                })
            {
                return Some(workflow_loop_result(
                    serde_json::json!({
                        "status": "declaration_follow_up_unrelated",
                        "code": "DECLARATION_FOLLOW_UP_UNRELATED",
                        "retryable": true,
                        "next_action": "get_declarations",
                        "unresolved_queries": state.unresolved_declaration_queries,
                        "message": "A partial declaration batch may be followed only by the exact unresolved capabilities or focused rephrasings that retain a distinctive capability term. Unrelated lookups do not consume an attempt and cannot unlock source authoring."
                    }),
                    false,
                ));
            }
            if state.initial_declaration_attempts >= MAX_INITIAL_DECLARATION_ATTEMPTS {
                return Some(workflow_loop_result(
                    serde_json::json!({
                        "status": "declaration_coverage_exhausted",
                        "code": "DECLARATION_COVERAGE_EXHAUSTED",
                        "retryable": false,
                        "next_action": "stop_and_report_unavailable_capabilities",
                        "attempts": state.initial_declaration_attempts,
                        "unresolved_queries": state.unresolved_declaration_queries,
                        "message": "The bounded initial declaration lookup did not return a usable live signature. No source write was dispatched. Stop and report the exact unmatched capabilities instead of guessing names or pins."
                    }),
                    true,
                ));
            }
            state.initial_declaration_attempts =
                state.initial_declaration_attempts.saturating_add(1);
            state.declaration_calls = state.declaration_calls.saturating_add(1);
            state.declarations_since_edit = state.declarations_since_edit.saturating_add(1);
            state.declaration_lookup_in_flight = true;
            None
        }
        "get_declarations"
            if state.declaration_calls == 0
                && !declaration_lookup_queries(args)
                    .iter()
                    .any(|query| !query.trim().is_empty()) =>
        {
            Some(workflow_loop_result(
                serde_json::json!({
                    "status": "declaration_batch_required",
                    "retryable": true,
                    "next_action": "get_declarations",
                    "message": "Declaration lookup requires at least one focused capability query."
                }),
                false,
            ))
        }
        "get_declarations" if state.declarations_since_edit >= 1 => Some(workflow_loop_result(
            serde_json::json!({
                "status": "discovery_budget_exhausted",
                "next_action": "write_or_patch_flowscript",
                "message": "A usable declaration batch is retained. Submit the full-shape source with write_flowscript now, or patch the retained revision. Do not chase omitted or unmatched entries before the first draft; use compiler diagnostics for focused follow-up lookups."
            }),
            false,
        )),
        "get_declarations" if state.declaration_calls == 0 => {
            state.declaration_calls = state.declaration_calls.saturating_add(1);
            state.declarations_since_edit = state.declarations_since_edit.saturating_add(1);
            state.declaration_lookup_in_flight = true;
            None
        }
        "get_declarations" => {
            if state.declaration_calls >= MAX_EXTERNAL_WORKFLOW_DECLARATION_CALLS {
                return Some(workflow_loop_result(
                    serde_json::json!({
                        "status": "discovery_budget_exhausted",
                        "next_action": "patch_flowscript",
                        "message": "The diagnostic-driven declaration lookup safety cap is exhausted. Continue repairing the retained draft with the declarations already returned."
                    }),
                    false,
                ));
            }

            let eligible = diagnostic_declaration_repair_hints(&state.last_errors);
            let queries = declaration_lookup_queries(args);
            if queries.is_empty()
                || queries.len() > MAX_REPAIR_DECLARATION_QUERIES
                || queries
                    .iter()
                    .any(|query| !declaration_repair_query_is_bounded(query))
            {
                return Some(workflow_loop_result(
                    serde_json::json!({
                        "status": "diagnostic_lookup_required",
                        "next_action": "patch_flowscript",
                        "eligible_targets": eligible.exposed_targets(),
                        "max_queries": MAX_REPAIR_DECLARATION_QUERIES,
                        "message": "Repair declaration discovery must be a short bounded batch tied to the latest validation diagnostics. Broad, oversized, or empty discovery was not dispatched."
                    }),
                    false,
                ));
            }

            let matches = queries
                .iter()
                .map(|query| declaration_repair_query_keys(query, &eligible))
                .collect::<Vec<_>>();
            let matched_query_count = matches.iter().filter(|keys| !keys.is_empty()).count();
            let requested = matches.into_iter().flatten().collect::<HashSet<_>>();
            // Permit a bounded batch to include a few plausible alternatives, but require at least
            // half of its focused searches to be justified by the current diagnostics. Completed
            // target keys prevent using the same match to reopen broad discovery after each edit.
            if eligible.is_empty()
                || requested.is_empty()
                || matched_query_count.saturating_mul(2) < queries.len()
            {
                return Some(workflow_loop_result(
                    serde_json::json!({
                        "status": "diagnostic_lookup_required",
                        "next_action": "patch_flowscript",
                        "eligible_targets": eligible.exposed_targets(),
                        "message": "A repair declaration batch must keep at least half of its focused searches tied to a node/function from the latest pin or catalog diagnostic, or to the comparison/conversion/string-operation topic identified by that diagnostic. Unrelated discovery was not dispatched."
                    }),
                    false,
                ));
            }

            let new_requested = requested
                .difference(&state.completed_repair_lookup_keys)
                .filter(|key| !state.in_flight_repair_lookup_keys.contains(*key))
                .filter(|key| {
                    state
                        .repair_lookup_attempts
                        .get(*key)
                        .copied()
                        .unwrap_or_default()
                        < MAX_REPAIR_DECLARATION_ATTEMPTS_PER_KEY
                })
                .cloned()
                .collect::<HashSet<_>>();
            if new_requested.is_empty() {
                return Some(workflow_loop_result(
                    serde_json::json!({
                        "status": "duplicate_declaration_lookup",
                        "next_action": "patch_flowscript",
                        "targets": requested,
                        "message": "These diagnostic repair targets were already resolved, definitively unavailable, or exhausted their bounded exact-signature retry. The duplicate request was not dispatched; apply retained declarations or report the unavailable capability."
                    }),
                    false,
                ));
            }

            for key in &new_requested {
                let attempts = state.repair_lookup_attempts.entry(key.clone()).or_default();
                *attempts = attempts.saturating_add(1);
            }
            state.in_flight_repair_lookup_keys = new_requested;
            state.declaration_calls = state.declaration_calls.saturating_add(1);
            state.declarations_since_edit = state.declarations_since_edit.saturating_add(1);
            state.declaration_lookup_in_flight = true;
            None
        }
        "commit_flow_ir_draft" => {
            state
                .mutation_path
                .get_or_insert(WorkflowMutationPath::TypedIr);
            state.typed_operation_attempts = state.typed_operation_attempts.saturating_add(1);
            state.edit_attempts = state.edit_attempts.saturating_add(1);
            state.edit_in_flight = true;
            None
        }
        tool if is_workflow_commit_tool(tool) && state.edit_in_flight => {
            Some(workflow_loop_result(
                serde_json::json!({
                    "status": "edit_in_flight",
                    "next_action": "wait",
                    "message": "Another workflow commit is still running. Do not submit parallel commits; wait for its validation result and revise that same typed revision or FlowScript draft if needed."
                }),
                true,
            ))
        }
        tool if is_workflow_commit_tool(tool)
            && state.stalled_edit_attempts >= MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS =>
        {
            Some(workflow_loop_result(
                serde_json::json!({
                    "status": "edit_progress_stalled",
                    "next_action": "stop",
                    "errors": state.last_errors,
                    "message": "The last repair attempts repeated the same unresolved validation diagnostics. Stop this bounded loop and report those diagnostics; the best full-scope draft remains retained."
                }),
                true,
            ))
        }
        tool if is_workflow_commit_tool(tool)
            && state.edit_attempts >= MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS =>
        {
            Some(workflow_loop_result(
                serde_json::json!({
                    "status": "edit_budget_exhausted",
                    "next_action": "stop",
                    "errors": state.last_errors,
                    "message": "The bounded FlowScript repair budget is exhausted. Stop broad discovery and report the remaining validation diagnostics honestly."
                }),
                true,
            ))
        }
        tool if is_workflow_commit_tool(tool) => {
            if let Some(requested_path) = requested_path {
                state.mutation_path.get_or_insert(requested_path);
            }
            state.edit_attempts = state.edit_attempts.saturating_add(1);
            state.edit_in_flight = true;
            None
        }
        _ => None,
    }
}

#[cfg(test)]
fn workflow_tool_preflight(
    state: &Arc<StdMutex<WorkflowToolLoopState>>,
    tool_name: &str,
) -> Option<rmcp::model::CallToolResult> {
    workflow_tool_preflight_with_args(state, tool_name, &serde_json::Value::Null)
}

/// Stop an external agent from satisfying the edit loop with a tiny valid smoke test after it has
/// already authored a substantially richer candidate. This runs after the ordinary edit preflight
/// (so the attempt is bounded) but before the reconcile handler can append commands.
fn workflow_candidate_preflight(
    state: &Arc<StdMutex<WorkflowToolLoopState>>,
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<rmcp::model::CallToolResult> {
    if !is_workflow_commit_tool(tool_name) {
        return None;
    }
    let submitted = submitted_flowscript(args)?.trim();
    if submitted.is_empty() {
        return None;
    }

    let mut state = match state.lock() {
        Ok(state) => state,
        Err(_) => return Some(workflow_loop_state_unavailable_result()),
    };
    let Some(regression) = state.repair_tracker.queued_candidate_regression(submitted) else {
        let modular_fallback = state
            .repair_tracker
            .queued_candidate_modular_fallback(submitted);
        state.pending_modular_fallback = modular_fallback;
        state.in_flight_flowscript = Some(submitted.to_string());
        return None;
    };
    let retained = state.repair_tracker.best_failed_source()?.to_string();
    let retained_profile = profile_flowscript_candidate(&retained);
    let submitted_profile = profile_flowscript_candidate(submitted);

    let message = format!(
        "The submitted FlowScript is a severe completeness regression: it has {} executable call(s) and {} distinct call type(s), while the retained repair candidate has {} executable call(s) and {} distinct call type(s). A tiny valid smoke test cannot replace the requested multi-step workflow. Revise the retained candidate and preserve its functions, event entries, variables, and behavior.",
        submitted_profile.call_sites,
        submitted_profile.call_names.len(),
        retained_profile.call_sites,
        retained_profile.call_names.len(),
    );

    // workflow_tool_preflight marked the attempt in flight. Complete it here because the actual
    // reconcile handler will not run, and retain the rich source for this process/continuations.
    state.edit_in_flight = false;
    state.in_flight_flowscript = None;
    state.last_status = Some("validation_errors".to_string());
    state.last_errors = vec![message.clone()];
    state.candidate_regression_warning = Some(message.clone());
    state.pending_modular_fallback = None;
    state.declarations_since_edit = 0;

    Some(workflow_loop_result(
        serde_json::json!({
            "status": "validation_errors",
            "code": "candidate_regression",
            "retryable": true,
            "next_action": "revise_retained_candidate",
            "errors": [message],
            "regression": {
                "previous_call_sites": regression.previous_call_sites,
                "candidate_call_sites": regression.candidate_call_sites,
                "previous_statements": regression.previous_statements,
                "candidate_statements": regression.candidate_statements,
                "previous_scope_symbols": regression.previous_scope_symbols,
                "retained_scope_symbols": regression.retained_scope_symbols,
            },
            "retained_candidate_profile": {
                "call_sites": retained_profile.call_sites,
                "meaningful_statements": retained_profile.meaningful_statements,
                "helper_functions": retained_profile.helper_functions.len(),
                "event_entries": retained_profile.event_entries,
                "top_level_variables": retained_profile.top_level_variables.len(),
            },
            "submitted_candidate_profile": {
                "call_sites": submitted_profile.call_sites,
                "meaningful_statements": submitted_profile.meaningful_statements,
                "helper_functions": submitted_profile.helper_functions.len(),
                "event_entries": submitted_profile.event_entries,
                "top_level_variables": submitted_profile.top_level_variables.len(),
            },
            "retained_flowscript": truncate_for_preview(&retained, 30_000),
            "message": "Nothing was queued. Continue from retained_flowscript and fix its diagnostics. Preserve the requested scope, or refactor real work into non-empty named helpers invoked by a separate Event; do not replace it with a smoke test or empty Event shell."
        }),
        true,
    ))
}

fn workflow_diagnostic_has_parent(entry: &serde_json::Value) -> bool {
    match entry.get("caused_by") {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::String(cause)) => !cause.trim().is_empty(),
        Some(serde_json::Value::Array(causes)) => !causes.is_empty(),
        Some(serde_json::Value::Object(cause)) => !cause.is_empty(),
        Some(_) => true,
    }
}

fn workflow_result_diagnostics(parsed: Option<&serde_json::Value>) -> Vec<String> {
    let mut diagnostics = parsed
        .map(|value| {
            [
                "errors",
                "diagnostics",
                "structured_diagnostics",
                "module_budget_violations",
            ]
            .into_iter()
            .filter_map(|key| value.get(key).and_then(serde_json::Value::as_array))
            .flat_map(|entries| entries.iter())
            .filter_map(|entry| {
                if workflow_diagnostic_has_parent(entry) {
                    return None;
                }
                entry.as_str().map(str::to_string).or_else(|| {
                    let code = entry.get("code").and_then(serde_json::Value::as_str);
                    let message = entry.get("message").and_then(serde_json::Value::as_str);
                    match (code, message) {
                        (Some(code), Some(message)) => Some(format!("[{code}] {message}")),
                        (None, Some(message)) => Some(message.to_string()),
                        _ => None,
                    }
                })
            })
            .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(value) = parsed {
        if let Some(missing_modules) = value
            .get("missing_modules")
            .and_then(serde_json::Value::as_array)
        {
            diagnostics.extend(missing_modules.iter().filter_map(|module| {
                module
                    .as_str()
                    .map(|module| format!("Missing required module: {module}"))
            }));
        }
        if let Some(budget_violations) = value
            .get("capability_plan")
            .and_then(|plan| plan.get("module_budget_violations"))
            .and_then(serde_json::Value::as_array)
        {
            diagnostics.extend(budget_violations.iter().filter_map(|entry| {
                entry.as_str().map(str::to_string).or_else(|| {
                    let code = entry.get("code").and_then(serde_json::Value::as_str);
                    let message = entry.get("message").and_then(serde_json::Value::as_str);
                    match (code, message) {
                        (Some(code), Some(message)) => Some(format!("[{code}] {message}")),
                        (None, Some(message)) => Some(message.to_string()),
                        _ => None,
                    }
                })
            }));
        }
    }
    let mut seen = HashSet::new();
    diagnostics.retain(|diagnostic| seen.insert(diagnostic.clone()));
    diagnostics
}

/// Compiler-pipeline order: an early-phase diagnostic is the likeliest root cause of later
/// cascades, so retention prefers it when the budget cannot hold everything.
fn structured_diagnostic_phase_rank(entry: &serde_json::Map<String, serde_json::Value>) -> u8 {
    match entry.get("phase").and_then(serde_json::Value::as_str) {
        Some("parse") => 0,
        Some("catalog_resolution") => 1,
        Some("type_check") => 2,
        Some("lowering") => 3,
        Some("execution_wiring") => 4,
        Some("validation" | "validate") => 5,
        _ => 6,
    }
}

fn workflow_result_structured_diagnostics(
    parsed: Option<&serde_json::Value>,
) -> Vec<serde_json::Value> {
    const RETAINED_FIELDS: &[&str] = &[
        "id",
        "code",
        "phase",
        "severity",
        "message",
        "source_span",
        "ast_path",
        "scope",
        "expected",
        "actual",
        "declaration",
        "pin",
        "fix",
        "occurrences",
        "related_messages",
    ];

    let Some(parsed) = parsed else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for entry in ["structured_diagnostics", "diagnostics"]
        .into_iter()
        .filter_map(|key| parsed.get(key).and_then(serde_json::Value::as_array))
        .flat_map(|entries| entries.iter())
    {
        let Some(source) = entry.as_object() else {
            continue;
        };
        if workflow_diagnostic_has_parent(entry) {
            continue;
        }
        let mut object = serde_json::Map::new();
        for field in RETAINED_FIELDS {
            if let Some(value) = source.get(*field) {
                object.insert((*field).to_string(), value.clone());
            }
        }
        if object.is_empty() {
            continue;
        }
        if seen.insert(serde_json::to_string(&object).unwrap_or_default()) {
            candidates.push(object);
        }
    }

    // Root causes first: earlier compiler phases outrank later ones, and the first occurrence of
    // each distinct code outranks its repeats, so truncation drops cascades instead of causes.
    let mut ordered = candidates
        .into_iter()
        .enumerate()
        .map(|(index, object)| (structured_diagnostic_phase_rank(&object), index, object))
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(phase_rank, index, _)| (*phase_rank, *index));
    let mut seen_codes = HashSet::new();
    let mut ranked = ordered
        .into_iter()
        .enumerate()
        .map(|(rank_index, (phase_rank, _, object))| {
            let repeated_code = match object.get("code") {
                Some(code) => !seen_codes.insert(code.to_string()),
                None => false,
            };
            (phase_rank, repeated_code, rank_index, object)
        })
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(phase_rank, repeated_code, rank_index, _)| {
        (*phase_rank, *repeated_code, *rank_index)
    });

    let total = ranked.len();
    let mut retained = Vec::new();
    let mut retained_bytes = 0usize;
    for (_, _, _, mut object) in ranked {
        if retained.len() >= MAX_RETAINED_STRUCTURED_DIAGNOSTICS {
            break;
        }
        let mut encoded = serde_json::to_string(&object).unwrap_or_default();
        if retained_bytes.saturating_add(encoded.len()) > MAX_RETAINED_STRUCTURED_DIAGNOSTIC_BYTES {
            // Exact repair declarations are retained separately. Prefer keeping the diagnostic's
            // location/type fields over dropping the entire item because a fix payload is large.
            object.remove("fix");
            object.remove("related_messages");
            encoded = serde_json::to_string(&object).unwrap_or_default();
        }
        if retained_bytes.saturating_add(encoded.len()) > MAX_RETAINED_STRUCTURED_DIAGNOSTIC_BYTES {
            // One oversized item must not silently discard every remaining smaller diagnostic.
            continue;
        }
        retained_bytes = retained_bytes.saturating_add(encoded.len());
        retained.push(serde_json::Value::Object(object));
    }
    if retained.len() < total {
        let omitted = total - retained.len();
        retained.push(serde_json::json!({
            "truncated": true,
            "omitted_count": omitted,
            "message": format!(
                "{omitted} additional structured diagnostic(s) exceeded the retention budget and were omitted; root-cause phases and first occurrences of each code were kept first."
            ),
        }));
    }
    retained
}

/// Preserve exact catalog signatures carried by structured FlowScript fixes. Diagnostic text is
/// intentionally flattened for progress/stall accounting, but a fresh external-agent process
/// needs the richer repair payload to avoid guessing the same declaration or pin again.
fn workflow_result_repair_declarations(parsed: Option<&serde_json::Value>) -> Vec<String> {
    let Some(parsed) = parsed else {
        return Vec::new();
    };

    let mut declarations = Vec::new();
    let mut seen = HashSet::new();
    let mut retained_bytes = 0usize;
    for diagnostic in ["diagnostics", "structured_diagnostics"]
        .into_iter()
        .filter_map(|key| parsed.get(key).and_then(serde_json::Value::as_array))
        .flat_map(|entries| entries.iter())
    {
        let Some(fix) = diagnostic.get("fix") else {
            continue;
        };
        for signatures in ["catalog_declarations", "companion_declarations"]
            .into_iter()
            .filter_map(|key| fix.get(key).and_then(serde_json::Value::as_array))
        {
            for signature in signatures.iter().filter_map(serde_json::Value::as_str) {
                let signature = signature.trim();
                if signature.is_empty() || seen.contains(signature) {
                    continue;
                }
                let next_bytes = retained_bytes.saturating_add(signature.len());
                if declarations.len() >= MAX_INJECTED_REPAIR_DECLARATIONS
                    || next_bytes > MAX_INJECTED_REPAIR_DECLARATION_BYTES
                {
                    return declarations;
                }
                seen.insert(signature.to_string());
                declarations.push(signature.to_string());
                retained_bytes = next_bytes;
            }
        }
    }
    declarations
}

/// Count of reviewer-facing notes carried by a lifecycle tool result. `None` when the result did
/// not include the field, so an unrelated follow-up result does not erase the last known count.
fn workflow_result_review_notes(parsed: Option<&serde_json::Value>) -> Option<usize> {
    parsed?
        .get("review_notes")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
}

fn typed_ir_missing_modules(parsed: Option<&serde_json::Value>) -> Vec<String> {
    let mut modules = parsed
        .and_then(|value| value.get("missing_modules"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    modules.sort_unstable();
    modules.dedup();
    modules
}

fn typed_ir_repair_fingerprint(
    status: Option<&str>,
    diagnostics: &[String],
    missing_modules: &[String],
) -> String {
    let mut normalized_diagnostics = diagnostics
        .iter()
        .map(|diagnostic| {
            diagnostic
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();
    normalized_diagnostics.sort_unstable();
    normalized_diagnostics.dedup();
    let mut normalized_modules = missing_modules
        .iter()
        .map(|module| module.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    normalized_modules.sort_unstable();
    normalized_modules.dedup();
    format!(
        "{}\u{1f}{}\u{1f}{}",
        status.unwrap_or("<missing-status>"),
        normalized_diagnostics.join("\u{1e}"),
        normalized_modules.join("\u{1e}")
    )
}

fn flowscript_repair_fingerprint(
    status: Option<&str>,
    diagnostics: &[String],
    structured_diagnostics: &[serde_json::Value],
) -> String {
    let mut normalized = diagnostics
        .iter()
        .map(|diagnostic| {
            diagnostic
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    let mut structured_subjects = structured_diagnostics
        .iter()
        .map(|diagnostic| {
            serde_json::json!({
                "code": diagnostic.get("code"),
                "message": diagnostic.get("message"),
                "ast_path": diagnostic.get("ast_path"),
                "declaration": diagnostic.get("declaration"),
                "pin": diagnostic.get("pin"),
                "occurrences": diagnostic.get("occurrences"),
            })
            .to_string()
            .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();
    structured_subjects.sort_unstable();
    structured_subjects.dedup();
    format!(
        "{}\u{1f}{}\u{1f}{}",
        status.unwrap_or("<missing-status>").to_ascii_lowercase(),
        normalized.join("\u{1e}"),
        structured_subjects.join("\u{1e}")
    )
}

fn workflow_result_has_explicit_diagnostics(parsed: &serde_json::Value) -> bool {
    [
        "errors",
        "diagnostics",
        "structured_diagnostics",
        "module_budget_violations",
    ]
    .into_iter()
    .any(|key| {
        parsed
            .get(key)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|entries| !entries.is_empty())
    }) || parsed.get("capability_plan").is_some_and(|plan| {
        plan.get("feasible").and_then(serde_json::Value::as_bool) == Some(false)
            || plan
                .get("module_budget_violations")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|entries| !entries.is_empty())
    })
}

fn workflow_result_requires_repair(parsed: &serde_json::Value, diagnostics: &[String]) -> bool {
    let status = parsed
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let failed_status = workflow_status_requires_repair(status);
    let infeasible_plan = parsed.get("feasible").and_then(serde_json::Value::as_bool)
        == Some(false)
        || parsed
            .get("capability_plan")
            .and_then(|plan| plan.get("feasible"))
            .and_then(serde_json::Value::as_bool)
            == Some(false);
    failed_status || infeasible_plan || !diagnostics.is_empty()
}

fn workflow_status_requires_repair(status: &str) -> bool {
    matches!(
        status,
        "error"
            | "cancelled"
            | "timeout"
            | "validation_error"
            | "validation_errors"
            | "no_changes"
            | "infeasible"
            | "candidate_regression"
            | "scope_reduction_blocked"
            | "resource_limit_rejected"
            | "revision_conflict"
            | "request_identity_mismatch"
            | "module_needs_repair"
            | "draft_needs_repair"
            | "discovery_blocked"
            | "discovery_budget_exhausted"
            | "edit_budget_exhausted"
            | "edit_in_flight"
            | "internal_state_unavailable"
    )
}

fn workflow_result_clears_repair(parsed: &serde_json::Value) -> bool {
    matches!(
        parsed.get("status").and_then(serde_json::Value::as_str),
        Some(
            "queued"
                | "already_queued"
                | "rendered"
                | "valid"
                | "draft_valid"
                | "draft_updated"
                | "module_validated"
                | "draft_started"
        )
    )
}

fn workflow_result_fallback_message(parsed: &serde_json::Value) -> Option<String> {
    let code = parsed.get("code").and_then(serde_json::Value::as_str);
    let message = parsed.get("message").and_then(serde_json::Value::as_str);
    match (code, message) {
        (Some(code), Some(message)) => Some(format!("[{code}] {message}")),
        (None, Some(message)) => Some(message.to_string()),
        (Some(code), None) => Some(format!("[{code}] Workflow validation needs repair.")),
        (None, None) => None,
    }
}

fn declaration_result_is_usable(result_text: &str) -> bool {
    result_text.lines().any(declaration_line_is_complete)
}

fn declaration_line_is_complete(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("declare function ")
        && line.contains('(')
        && line.contains(')')
        && line.contains(';')
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DeclarationBatchCoverage {
    processed_count: usize,
    complete: bool,
    matched_count: usize,
    matched_queries: Vec<String>,
    unmatched_queries: Vec<String>,
    output_omitted_queries: Vec<String>,
    omitted_queries: Vec<String>,
    unmatched_count: usize,
    output_omitted_count: usize,
    omitted_count: usize,
    truncated_query_count: usize,
    query_names_omitted_for_size: bool,
}

fn declaration_query_key(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn declaration_queries_are_related(left: &str, right: &str) -> bool {
    let left_key = declaration_query_key(left);
    let right_key = declaration_query_key(right);
    if left_key == right_key {
        return true;
    }
    const GENERIC_TERMS: &[&str] = &[
        "add", "approval", "build", "create", "data", "delete", "email", "fetch", "file", "find",
        "for", "from", "get", "into", "list", "mail", "make", "message", "node", "open", "read",
        "receive", "remove", "response", "run", "send", "set", "string", "the", "update", "use",
        "with", "workflow", "write",
    ];
    let distinctive = |value: &str| {
        value
            .split_whitespace()
            .filter(|token| token.len() >= 3 && !GENERIC_TERMS.contains(token))
            .map(str::to_string)
            .collect::<HashSet<_>>()
    };
    let left_terms = distinctive(&left_key);
    let right_terms = distinctive(&right_key);
    if left_terms.is_disjoint(&right_terms) {
        return false;
    }

    // Connector identity alone is not enough: `smtp send` and `smtp receive` are different
    // capabilities even though both retain the distinctive `smtp` token. When both phrasings name
    // an operation, require the operation family itself to survive the rephrase.
    const OPERATION_TERMS: &[&str] = &[
        "add", "compare", "connect", "convert", "create", "delete", "fetch", "find", "get", "list",
        "parse", "read", "receive", "remove", "replace", "search", "send", "set", "trim", "update",
        "write",
    ];
    let operations = |value: &str| {
        value
            .split_whitespace()
            .filter(|token| OPERATION_TERMS.contains(token))
            .map(str::to_string)
            .collect::<HashSet<_>>()
    };
    let left_operations = operations(&left_key);
    let right_operations = operations(&right_key);
    left_operations.is_empty()
        || right_operations.is_empty()
        || !left_operations.is_disjoint(&right_operations)
}

fn declaration_batch_coverage(result_text: &str) -> Option<DeclarationBatchCoverage> {
    const PREFIX: &str = "// flowpilot.declaration-batch/v1 ";
    let metadata = result_text
        .lines()
        .find_map(|line| line.strip_prefix(PREFIX))?;
    let value = serde_json::from_str::<serde_json::Value>(metadata).ok()?;
    let strings = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    Some(DeclarationBatchCoverage {
        processed_count: value
            .get("processed_count")
            .and_then(serde_json::Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default(),
        complete: value
            .get("complete")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        matched_count: value
            .get("matched_count")
            .and_then(serde_json::Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default(),
        matched_queries: strings("matched_queries"),
        unmatched_queries: strings("unmatched_queries"),
        output_omitted_queries: strings("output_omitted_queries"),
        omitted_queries: strings("omitted_queries"),
        unmatched_count: value
            .get("unmatched_count")
            .and_then(serde_json::Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default(),
        output_omitted_count: value
            .get("output_omitted_count")
            .and_then(serde_json::Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default(),
        omitted_count: value
            .get("omitted_count")
            .and_then(serde_json::Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default(),
        truncated_query_count: value
            .get("truncated_query_count")
            .and_then(serde_json::Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default(),
        query_names_omitted_for_size: value
            .get("query_names_omitted_for_size")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

fn complete_declaration_coverage_is_coherent(
    coverage: &DeclarationBatchCoverage,
    args: &serde_json::Value,
    result_text: &str,
) -> bool {
    if !coverage.complete {
        return true;
    }
    let requested_count = declaration_lookup_queries(args)
        .into_iter()
        .map(declaration_query_key)
        .filter(|query| !query.is_empty())
        .collect::<HashSet<_>>()
        .len();
    let exact_declaration_count = result_text
        .lines()
        .filter(|line| declaration_line_is_complete(line))
        .count();
    coverage.processed_count > 0
        && coverage.processed_count == requested_count
        && coverage.matched_count == coverage.processed_count
        && (coverage.query_names_omitted_for_size
            || coverage.matched_queries.len() == coverage.matched_count)
        && coverage.unmatched_count == 0
        && coverage.output_omitted_count == 0
        && coverage.omitted_count == 0
        && coverage.truncated_query_count == 0
        && exact_declaration_count >= coverage.matched_count
}

fn retain_declaration_result(existing: Option<&str>, result_text: &str) -> String {
    // Keep the newest bounded batch whole: its catalog-authored notes carry non-obvious ordering,
    // repeated-pin, schema-field, and companion-call guidance that signatures alone cannot encode.
    // Then retain older unique exact signatures while they fit. Never byte-truncate a declaration
    // line: a partial signature is worse than an explicit omission because it looks authoritative
    // to the next model process.
    let complete_lines = |text: &str| {
        text.lines()
            .map(str::trim)
            .filter(|line| declaration_line_is_complete(line))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let newest = complete_lines(result_text);
    let older = existing.map(complete_lines).unwrap_or_default();
    let mut seen = newest.iter().cloned().collect::<HashSet<_>>();
    let newest_full = result_text.trim();
    let mut retained = if newest_full.len() <= MAX_RETAINED_DECLARATION_BYTES {
        newest_full.to_string()
    } else {
        // Defensive fallback for a non-conforming worker: retain only whole exact signatures.
        newest.join("\n")
    };
    for line in &older {
        if !seen.insert(line.clone()) {
            continue;
        }
        let separator_bytes = usize::from(!retained.is_empty());
        if retained
            .len()
            .saturating_add(separator_bytes)
            .saturating_add(line.len())
            > MAX_RETAINED_DECLARATION_BYTES
        {
            continue;
        }
        if !retained.is_empty() {
            retained.push('\n');
        }
        retained.push_str(line);
    }
    if retained.is_empty() {
        // The caller normally invokes this only for a usable result. Keep a bounded diagnostic
        // fallback for defensive compatibility with older workers.
        truncate_for_preview(result_text, MAX_RETAINED_DECLARATION_BYTES)
    } else {
        retained
    }
}

fn workflow_tool_record(
    state: &Arc<StdMutex<WorkflowToolLoopState>>,
    tool_name: &str,
    args: &serde_json::Value,
    result_text: &str,
) {
    if tool_name == "get_declarations" {
        if let Ok(mut state) = state.lock() {
            let was_initial_lookup = state.needs_initial_declaration_coverage();
            let usable = declaration_result_is_usable(result_text);
            let mut coverage = declaration_batch_coverage(result_text);
            if let Some(parsed_coverage) = coverage.as_mut()
                && !complete_declaration_coverage_is_coherent(parsed_coverage, args, result_text)
            {
                let requested = declaration_lookup_queries(args)
                    .into_iter()
                    .filter(|query| !query.trim().is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                parsed_coverage.complete = false;
                parsed_coverage.matched_count = 0;
                parsed_coverage.matched_queries.clear();
                parsed_coverage.output_omitted_count = requested.len();
                parsed_coverage.output_omitted_queries = requested;
            }
            state.declaration_lookup_in_flight = false;
            let repair_lookup_keys = std::mem::take(&mut state.in_flight_repair_lookup_keys);
            // A parsed coverage envelope is an authoritative catalog outcome even when nothing
            // matched (or an exact signature could not fit in the bounded response). Consume the
            // diagnostic target so the model cannot reopen the same unavailable lookup forever.
            // Only a missing/unparseable result behaves like a transport failure and releases the
            // lease for an exact retry.
            let authoritative_outcome = coverage.is_some();
            let legacy_multi_query_incomplete = !authoritative_outcome
                && usable
                && !repair_lookup_keys.is_empty()
                && declaration_lookup_queries(args).len() > 1;
            let retryable_omission = legacy_multi_query_incomplete
                || coverage.as_ref().is_some_and(|coverage| {
                    coverage.output_omitted_count > 0
                        || coverage.omitted_count > 0
                        || coverage.truncated_query_count > 0
                });
            let repair_lookup_failed =
                !usable && !authoritative_outcome && !repair_lookup_keys.is_empty();
            if (usable || authoritative_outcome) && !retryable_omission {
                state
                    .completed_repair_lookup_keys
                    .extend(repair_lookup_keys.iter().cloned());
            } else if repair_lookup_failed {
                state.declaration_calls = state.declaration_calls.saturating_sub(1);
                state.declarations_since_edit = state.declarations_since_edit.saturating_sub(1);
                for key in &repair_lookup_keys {
                    let remove = if let Some(attempts) = state.repair_lookup_attempts.get_mut(key) {
                        *attempts = attempts.saturating_sub(1);
                        *attempts == 0
                    } else {
                        false
                    };
                    if remove {
                        state.repair_lookup_attempts.remove(key);
                    }
                }
            } else if retryable_omission && !repair_lookup_keys.is_empty() {
                // A large batch can omit a declaration that fits when queried alone. Release the
                // per-edit discovery lease for one focused retry while the per-key counter and
                // global call budget prevent an omission loop.
                state.declarations_since_edit = 0;
            }
            if usable {
                state.initial_declaration_lookup_usable = true;
                state.last_declarations = Some(retain_declaration_result(
                    state.last_declarations.as_deref(),
                    result_text,
                ));
            }
            if was_initial_lookup {
                match coverage {
                    Some(coverage) => {
                        let previous_unresolved =
                            std::mem::take(&mut state.unresolved_declaration_queries);
                        let mut matched_queries = coverage.matched_queries.clone();
                        if coverage.complete && coverage.query_names_omitted_for_size {
                            // The compact metadata header intentionally omits identities. The
                            // exact dispatched arguments remain host-owned and are safe to use as
                            // the matched set for this one complete batch.
                            matched_queries.extend(
                                declaration_lookup_queries(args)
                                    .into_iter()
                                    .map(str::to_string),
                            );
                        }
                        let mut processed_queries = matched_queries
                            .iter()
                            .map(String::as_str)
                            .chain(coverage.unmatched_queries.iter().map(String::as_str))
                            .collect::<Vec<_>>();
                        let mut unresolved = Vec::new();
                        for previous in previous_unresolved {
                            if let Some(index) = processed_queries
                                .iter()
                                .position(|query| declaration_queries_are_related(&previous, query))
                            {
                                // A successful focused rephrasing resolves the previous miss. An
                                // unsuccessful one replaces it with the new wording below, rather
                                // than accumulating aliases that can never all match exactly.
                                processed_queries.remove(index);
                            } else {
                                unresolved.push(previous);
                            }
                        }
                        let named_unmatched = coverage.unmatched_queries.len();
                        let named_output_omitted = coverage.output_omitted_queries.len();
                        let named_omitted = coverage.omitted_queries.len();
                        unresolved.extend(coverage.unmatched_queries);
                        unresolved.extend(coverage.output_omitted_queries);
                        unresolved.extend(coverage.omitted_queries);
                        let unnamed_unmatched =
                            coverage.unmatched_count.saturating_sub(named_unmatched);
                        let unnamed_omitted = coverage.omitted_count.saturating_sub(named_omitted);
                        let unnamed_output_omitted = coverage
                            .output_omitted_count
                            .saturating_sub(named_output_omitted);
                        if unnamed_unmatched > 0 {
                            unresolved.push(format!(
                                "{} additional unmatched declaration query or queries (names omitted for size)",
                                unnamed_unmatched
                            ));
                        }
                        if unnamed_omitted > 0 {
                            unresolved.push(format!(
                                "{} additional omitted declaration query or queries (names omitted for size)",
                                unnamed_omitted
                            ));
                        }
                        if unnamed_output_omitted > 0 {
                            unresolved.push(format!(
                                "{} additional declaration query or queries matched but their exact signatures were omitted from the bounded response",
                                unnamed_output_omitted
                            ));
                        }
                        if coverage.truncated_query_count > 0 {
                            unresolved.push(format!(
                                "{} overlong declaration query or queries must be shortened",
                                coverage.truncated_query_count
                            ));
                        }
                        if !coverage.complete
                            && unresolved.is_empty()
                            && coverage.unmatched_count == 0
                            && coverage.output_omitted_count == 0
                            && coverage.omitted_count == 0
                            && coverage.truncated_query_count == 0
                        {
                            unresolved.push(
                                "Declaration batch reported incomplete coverage without query identities."
                                    .to_string(),
                            );
                        }
                        unresolved.sort_unstable();
                        unresolved.dedup();
                        state.initial_declaration_lookup_complete =
                            coverage.complete && unresolved.is_empty();
                        state.unresolved_declaration_queries = unresolved;
                    }
                    None if usable && declaration_lookup_queries(args).len() == 1 => {
                        // Backward compatibility for direct SDK/tests and older tool workers that
                        // predate coverage metadata: one requested capability with one actual
                        // declaration remains usable. Multi-query legacy results cannot prove
                        // complete coverage and stay gated.
                        state.initial_declaration_lookup_complete = true;
                        state.unresolved_declaration_queries.clear();
                    }
                    None => {
                        state.initial_declaration_lookup_complete = false;
                        state.unresolved_declaration_queries = declaration_lookup_queries(args)
                            .into_iter()
                            .filter(|query| !query.trim().is_empty())
                            .map(str::to_string)
                            .collect();
                        if state.unresolved_declaration_queries.is_empty() {
                            state.unresolved_declaration_queries = vec![
                                "No requested capability matched a live catalog declaration."
                                    .to_string(),
                            ];
                        }
                    }
                }
                if !usable && !state.initial_declaration_lookup_complete {
                    // No usable signature was returned, so permit a bounded focused retry. Once
                    // any live signature is retained, the next checkpoint must be source; omitted
                    // and unmatched capabilities are handled from compiler diagnostics later.
                    state.declarations_since_edit = 0;
                }
            }
        }
        return;
    }
    if is_flowscript_draft_operation_tool(tool_name) {
        let parsed = serde_json::from_str::<serde_json::Value>(result_text).ok();
        let Ok(mut state) = state.lock() else {
            return;
        };
        state.edit_in_flight = false;
        let interrupted_source = state.in_flight_flowscript.take();
        let previous_status = state.last_status.clone();
        let response_status = parsed
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(serde_json::Value::as_str);
        let response_code = parsed
            .as_ref()
            .and_then(|value| value.get("code"))
            .and_then(serde_json::Value::as_str);

        if matches!(
            response_code,
            Some("FLOWSCRIPT_DRAFT_MISSING" | "FLOWSCRIPT_BASE_REVISION_CONFLICT")
        ) {
            // These responses prove that the retained coordinates can no longer be continued.
            // Release only the local authorization lease so the same request may write a fresh
            // draft id against the current board. Preserve an explicitly returned old source as
            // reference, but never synthesize retained coordinates from the rejected arguments.
            if let Some(source) = parsed
                .as_ref()
                .and_then(|value| value.get("source"))
                .and_then(serde_json::Value::as_str)
            {
                state.last_flowscript = Some(source.to_string());
            }
            state.flowscript_draft_retained = false;
            state.flowscript_draft_id = None;
            state.flowscript_revision = None;
            state.flowscript_commit_attempts = 0;
            state.last_status = response_status.map(str::to_string);
            state.last_errors = parsed
                .as_ref()
                .and_then(workflow_result_fallback_message)
                .into_iter()
                .collect();
            state.last_structured_diagnostics.clear();
            state.pending_modular_fallback = None;
            state.declarations_since_edit = 0;
            return;
        }

        if response_status == Some("request_identity_mismatch") {
            // The core deliberately returns a minimal envelope for a draft owned by another
            // immutable request. Do not reconstruct its coordinates or treat the rejected source
            // arguments as retained state. A subsequent write with a distinct draft id remains
            // possible for the current request.
            state.last_status = Some("request_identity_mismatch".to_string());
            state.last_errors = parsed
                .as_ref()
                .and_then(workflow_result_fallback_message)
                .into_iter()
                .collect();
            state.last_structured_diagnostics.clear();
            state.pending_modular_fallback = None;
            return;
        }

        let preserve_checked_valid_after_transient_commit = tool_name == "commit_flowscript"
            && previous_status.as_deref() == Some("valid")
            && response_code == Some("FLOWSCRIPT_DRAFT_STORE_UNAVAILABLE");
        if preserve_checked_valid_after_transient_commit {
            // A store-lock failure happened before the exact checked command claim could be
            // inspected or changed. Preserve the host-checked revision and its validation state;
            // the separate commit-attempt cap bounds idempotent retries.
            state.last_status = previous_status;
            return;
        }

        let response_draft_id = parsed
            .as_ref()
            .and_then(|value| value.get("draft_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let response_revision = parsed
            .as_ref()
            .and_then(|value| value.get("revision"))
            .and_then(serde_json::Value::as_u64);
        let source = parsed
            .as_ref()
            .and_then(|value| value.get("source").or_else(|| value.get("flowscript")))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| submitted_flowscript(args).map(str::to_string))
            .or(interrupted_source);
        let current_draft_id = state.flowscript_draft_id.clone();
        state.flowscript_draft_id = response_draft_id
            .clone()
            .or_else(|| {
                args.get("draft_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .or(current_draft_id);
        state.flowscript_revision = response_revision.or(state.flowscript_revision);
        if response_draft_id.is_some() && response_revision.is_some() && source.is_some() {
            state.flowscript_draft_retained = true;
        }

        state.last_status = response_status.map(str::to_string);
        if state.last_status.as_deref() == Some("valid") {
            state.flowscript_commit_attempts = 0;
        }
        if let Some(parsed) = parsed.as_ref() {
            let repair_declarations = workflow_result_repair_declarations(Some(parsed));
            if !repair_declarations.is_empty() {
                state.last_repair_declarations = repair_declarations;
            } else if workflow_result_clears_repair(parsed) {
                state.last_repair_declarations.clear();
            }
        }
        let mut diagnostics = workflow_result_diagnostics(parsed.as_ref());
        state.last_structured_diagnostics = workflow_result_structured_diagnostics(parsed.as_ref());
        if let Some(review_notes) = workflow_result_review_notes(parsed.as_ref()) {
            state.last_review_notes = review_notes;
        }
        let requires_repair = parsed.as_ref().is_none_or(|value| {
            !workflow_result_clears_repair(value)
                && workflow_result_requires_repair(value, &diagnostics)
        });
        if requires_repair && diagnostics.is_empty() {
            if let Some(message) = parsed.as_ref().and_then(workflow_result_fallback_message) {
                diagnostics.push(message);
            } else if !result_text.trim().is_empty() {
                diagnostics.push(truncate_for_preview(result_text.trim(), 2_000));
            } else {
                diagnostics.push(
                    "The FlowScript source operation failed without diagnostics.".to_string(),
                );
            }
        }

        if let Some(source) = source {
            if requires_repair {
                state.pending_modular_fallback = None;
                if state
                    .repair_tracker
                    .record_failed_with_diagnostics(&source, Some(diagnostics.len()))
                {
                    state.best_failed_errors = diagnostics.clone();
                    state.candidate_regression_warning = None;
                }
            }
            state.last_flowscript = Some(source);
        }
        state.last_errors = diagnostics;
        let status = state.last_status.clone();
        let progress_diagnostics = state.last_errors.clone();
        state.record_flowscript_repair_progress(
            status.as_deref(),
            &progress_diagnostics,
            requires_repair,
        );
        if matches!(status.as_deref(), Some("queued" | "already_queued")) {
            state.queued = true;
            state.has_previous_validation_result = false;
            state.previous_validation_diagnostics.clear();
        }
        return;
    }
    if is_typed_ir_operation_tool(tool_name) {
        let parsed = serde_json::from_str::<serde_json::Value>(result_text).ok();
        let Ok(mut state) = state.lock() else {
            return;
        };
        if is_order_sensitive_workflow_tool(tool_name) {
            state.edit_in_flight = false;
        }
        let status = parsed
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(serde_json::Value::as_str);
        if status == Some("request_identity_mismatch") {
            // A typed draft owned by another immutable request is not recovery state for this
            // loop. Ignore both rejected arguments and any coordinates a stale/custom provider
            // might return; preserve only already-authorized local coordinates.
            state.last_status = Some("request_identity_mismatch".to_string());
            state.last_errors = parsed
                .as_ref()
                .and_then(workflow_result_fallback_message)
                .into_iter()
                .collect();
            return;
        }
        let current_draft_id = state.typed_draft_id.clone();
        let response_draft_id = parsed
            .as_ref()
            .and_then(|value| value.get("draft_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if response_draft_id.is_some()
            && parsed
                .as_ref()
                .is_some_and(|value| typed_ir_result_proves_retained_draft(value))
        {
            state.typed_draft_retained = true;
        }
        state.typed_draft_id = response_draft_id
            .or_else(|| {
                args.get("draft_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .or(current_draft_id);
        state.typed_revision = parsed
            .as_ref()
            .and_then(|value| value.get("revision"))
            .and_then(serde_json::Value::as_u64)
            .or(state.typed_revision);
        let preserve_retained_draft_context =
            tool_name == "plan_flow_ir" && state.typed_draft_retained;
        if !preserve_retained_draft_context {
            state.last_status = status.map(str::to_string);
        }
        let mut diagnostics = workflow_result_diagnostics(parsed.as_ref());
        let requires_repair = parsed.as_ref().is_none_or(|value| {
            !workflow_result_clears_repair(value)
                && workflow_result_requires_repair(value, &diagnostics)
        });
        if requires_repair && diagnostics.is_empty() {
            if let Some(message) = parsed.as_ref().and_then(workflow_result_fallback_message) {
                diagnostics.push(message);
            } else if !result_text.trim().is_empty() {
                diagnostics.push(truncate_for_preview(result_text.trim(), 2_000));
            } else {
                diagnostics.push("The typed-IR operation failed without diagnostics.".to_string());
            }
        }
        let missing_modules = typed_ir_missing_modules(parsed.as_ref());
        if let Some(review_notes) = workflow_result_review_notes(parsed.as_ref()) {
            state.last_review_notes = review_notes;
        }
        if !preserve_retained_draft_context {
            state.typed_missing_modules = missing_modules.clone();
            state.last_errors = diagnostics.clone();
        }
        if let Some(flowscript) = parsed
            .as_ref()
            .and_then(|value| value.get("flowscript"))
            .and_then(serde_json::Value::as_str)
        {
            state.last_flowscript = Some(flowscript.to_string());
        }

        if requires_repair {
            let target = typed_ir_operation_target(tool_name, args);
            let fingerprint = typed_ir_repair_fingerprint(status, &diagnostics, &missing_modules);
            let is_repeated = !state
                .typed_seen_repair_signatures
                .entry(target)
                .or_default()
                .insert(fingerprint);
            if is_repeated {
                state.typed_stalled_attempts = state.typed_stalled_attempts.saturating_add(1);
            } else {
                state.typed_stalled_attempts = 0;
            }
        } else {
            state.typed_stalled_attempts = 0;
        }

        if matches!(status, Some("queued" | "already_queued")) {
            state.queued = true;
            state.typed_stalled_attempts = 0;
        }
        return;
    }
    if !is_workflow_commit_tool(tool_name) {
        return;
    }
    let Ok(mut state) = state.lock() else {
        return;
    };
    state.edit_in_flight = false;
    state.in_flight_flowscript = None;

    let parsed = serde_json::from_str::<serde_json::Value>(result_text).ok();
    state.last_status = parsed
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let parsed_errors = workflow_result_diagnostics(parsed.as_ref());
    state.last_structured_diagnostics = workflow_result_structured_diagnostics(parsed.as_ref());
    if let Some(review_notes) = workflow_result_review_notes(parsed.as_ref()) {
        state.last_review_notes = review_notes;
    }

    if tool_name == "commit_flow_ir_draft" {
        let current_draft_id = state.typed_draft_id.clone();
        state.typed_draft_id = parsed
            .as_ref()
            .and_then(|value| value.get("draft_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or(current_draft_id);
        state.typed_revision = parsed
            .as_ref()
            .and_then(|value| value.get("revision"))
            .and_then(serde_json::Value::as_u64)
            .or(state.typed_revision);
    }

    let submitted_flowscript = submitted_flowscript(args).map(str::to_string).or_else(|| {
        parsed
            .as_ref()
            .and_then(|value| value.get("flowscript"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    });
    if let Some(submitted_flowscript) = submitted_flowscript {
        let submission_failed = !matches!(
            state.last_status.as_deref(),
            Some("queued" | "already_queued")
        );
        if submission_failed {
            state.pending_modular_fallback = None;
            if state
                .repair_tracker
                .record_failed_with_diagnostics(&submitted_flowscript, Some(parsed_errors.len()))
            {
                state.best_failed_errors = parsed_errors.clone();
                state.candidate_regression_warning = None;
            }
        }
        state.last_flowscript = Some(submitted_flowscript);
    }
    state.last_errors = parsed_errors;
    let status = state.last_status.clone();
    let progress_diagnostics = state.last_errors.clone();
    let requires_repair = parsed.as_ref().is_none_or(|value| {
        !workflow_result_clears_repair(value)
            && workflow_result_requires_repair(value, &progress_diagnostics)
    });
    state.record_flowscript_repair_progress(
        status.as_deref(),
        &progress_diagnostics,
        requires_repair,
    );
    if matches!(status.as_deref(), Some("queued" | "already_queued")) {
        state.queued = true;
        state.has_previous_validation_result = false;
        state.previous_validation_diagnostics.clear();
    }
}

fn workflow_tool_abort(state: &Arc<StdMutex<WorkflowToolLoopState>>, tool_name: &str, error: &str) {
    if tool_name == "get_declarations" {
        if let Ok(mut state) = state.lock()
            && state.declaration_lookup_in_flight
        {
            let initial_lookup = state.needs_initial_declaration_coverage();
            state.declaration_lookup_in_flight = false;
            let repair_lookup_keys = std::mem::take(&mut state.in_flight_repair_lookup_keys);
            for key in repair_lookup_keys {
                let remove = if let Some(attempts) = state.repair_lookup_attempts.get_mut(&key) {
                    *attempts = attempts.saturating_sub(1);
                    *attempts == 0
                } else {
                    false
                };
                if remove {
                    state.repair_lookup_attempts.remove(&key);
                }
            }
            state.declaration_calls = state.declaration_calls.saturating_sub(1);
            state.declarations_since_edit = state.declarations_since_edit.saturating_sub(1);
            if initial_lookup {
                // Preflight reserves initial coverage before dispatch. A worker abort produced no
                // catalog evidence, so release only that attempt while preserving earlier partial
                // declarations and unresolved identities.
                state.initial_declaration_attempts =
                    state.initial_declaration_attempts.saturating_sub(1);
            }
        }
        return;
    }
    if tool_name == "commit_flowscript" {
        if let Ok(mut state) = state.lock()
            && state.edit_in_flight
            && state.last_status.as_deref() == Some("valid")
        {
            // A transport/worker abort does not invalidate the host-checked source revision.
            // Preserve its status so the bounded idempotent commit retry path remains available.
            state.edit_in_flight = false;
            state.in_flight_flowscript = None;
            return;
        }
    }
    if !is_order_sensitive_workflow_tool(tool_name) {
        return;
    }
    if let Ok(mut state) = state.lock() {
        state.edit_in_flight = false;
        if let Some(interrupted) = state.in_flight_flowscript.take() {
            if state.repair_tracker.record_failed(&interrupted) {
                state.best_failed_errors = vec![error.to_string()];
                state.candidate_regression_warning = None;
            }
            state.last_flowscript = Some(interrupted);
        }
        state.last_status = Some("error".to_string());
        state.last_errors = vec![error.to_string()];
        state.pending_modular_fallback = None;
        state.declarations_since_edit = 0;
    }
}

fn annotate_modular_fallback_result(
    state: &Arc<StdMutex<WorkflowToolLoopState>>,
    tool_name: &str,
    result: &mut copilot_sdk::ToolResultObject,
) {
    if tool_name != "edit_flowscript" || result.result_type == "error" || result.error.is_some() {
        return;
    }
    let Ok(mut payload) = serde_json::from_str::<serde_json::Value>(&result.text_result_for_llm)
    else {
        return;
    };
    if payload.get("status").and_then(serde_json::Value::as_str) != Some("queued") {
        return;
    }
    let modular_fallback = state.lock().ok().and_then(|state| {
        state.pending_modular_fallback.clone().map(|regression| {
            (
                regression,
                state
                    .repair_tracker
                    .best_failed_source()
                    .map(str::to_string),
            )
        })
    });
    if let Some((regression, retained_full_source)) = modular_fallback
        && let Some(object) = payload.as_object_mut()
    {
        let notice =
            render_flowscript_modular_partial_result(&result.text_result_for_llm, &regression);
        object.insert(
            "completion".to_string(),
            serde_json::Value::String("partial_working_slice".to_string()),
        );
        object.insert(
            "partial_working_slice_notice".to_string(),
            serde_json::Value::String(notice),
        );
        if let Some(retained_full_source) = retained_full_source {
            object.insert(
                "retained_full_source".to_string(),
                serde_json::Value::String(retained_full_source),
            );
        }
        result.text_result_for_llm =
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    }
}

fn flowscript_source_fingerprint(source: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Drop the multi-kilobyte source echo from a model-facing FlowScript tool result when the host
/// provably did not change the source the model itself just submitted or last received:
/// - `write_flowscript`: the response source is byte-identical to the submitted document.
/// - `check_flowscript` / `commit_flowscript`: the response revision equals `expected_revision`;
///   neither operation mutates the retained source.
/// `patch_flowscript` keeps its echo because the merged result is host-computed. This runs after
/// `workflow_tool_record`, so host retention/continuation state keeps the complete source.
fn suppress_unchanged_flowscript_source_echo(
    tool_name: &str,
    args: &serde_json::Value,
    result: &mut copilot_sdk::ToolResultObject,
) {
    if !matches!(
        tool_name,
        "write_flowscript" | "check_flowscript" | "commit_flowscript"
    ) {
        return;
    }
    let Ok(mut payload) = serde_json::from_str::<serde_json::Value>(&result.text_result_for_llm)
    else {
        return;
    };
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    let Some(source) = object.get("source").and_then(serde_json::Value::as_str) else {
        return;
    };
    let revision = object.get("revision").and_then(serde_json::Value::as_u64);
    let unchanged = match tool_name {
        "write_flowscript" => submitted_flowscript(args) == Some(source),
        _ => {
            revision.is_some()
                && revision
                    == args
                        .get("expected_revision")
                        .and_then(serde_json::Value::as_u64)
        }
    };
    if !unchanged {
        return;
    }
    let summary = format!(
        "Source retained at revision {} ({} lines, fingerprint {}) — unchanged from your submitted document, so it is not re-echoed.",
        revision
            .map(|revision| revision.to_string())
            .unwrap_or_else(|| "<unknown>".to_string()),
        source.lines().count(),
        flowscript_source_fingerprint(source),
    );
    object.remove("source");
    object.insert(
        "source_echo".to_string(),
        serde_json::Value::String(summary),
    );
    result.text_result_for_llm =
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
}

#[derive(Clone)]
struct FlowPilotMcpServer {
    tools: Arc<HashMap<String, FlowPilotMcpTool>>,
    workflow_state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
    tool_activity: Arc<StdMutex<McpToolActivityState>>,
    handler_quiescence: Arc<tokio::sync::Notify>,
    workflow_operation_gate: Arc<tokio::sync::Mutex<()>>,
}

impl FlowPilotMcpServer {
    fn new(
        tools: Arc<HashMap<String, FlowPilotMcpTool>>,
        workflow_state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
        tool_activity: Arc<StdMutex<McpToolActivityState>>,
        handler_quiescence: Arc<tokio::sync::Notify>,
        workflow_operation_gate: Arc<tokio::sync::Mutex<()>>,
    ) -> Self {
        Self {
            tools,
            workflow_state,
            tool_activity,
            handler_quiescence,
            workflow_operation_gate,
        }
    }

    fn to_mcp_tool(tool: &copilot_sdk::Tool) -> rmcp::model::Tool {
        let schema = match &tool.parameters_schema {
            serde_json::Value::Object(_) => tool.parameters_schema.clone(),
            _ => serde_json::json!({ "type": "object", "properties": {} }),
        };

        rmcp::model::Tool::new(
            tool.name.clone(),
            tool.description.clone(),
            rmcp::model::object(schema),
        )
    }
}

impl rmcp::ServerHandler for FlowPilotMcpServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        let instructions = if self.workflow_state.is_some() {
            "FlowPilot tools share the exact board/frontend/runtime capabilities used by Bits and GitHub Copilot. WORKFLOW BUILD LOOP: FlowScript is the sole model-authored representation for executable workflow behavior. Read get_current_flowscript, make one bounded get_declarations batch for the highest-leverage catalog calls needed to establish the end-to-end shape, then immediately retain the full-shape source with write_flowscript. Do not enumerate every utility or chase omitted queries before that checkpoint. Repair the retained source with patch_flowscript, use structured compiler diagnostics for focused declaration follow-ups, run check_flowscript, and finish with commit_flowscript at the latest revision. Before the first write, use at most six ancillary database/UI/storage inspections. Preserve every requested capability, helper, Event, and kept //@n anchor across repairs; structured compiler diagnostics are authoritative. Never replace a failed production draft with a smoke test or empty Event. Use emit_commands only for position-only MoveNode or canvas comments; it rejects executable commands and every layer mutation. Never use Read/shell/filesystem tools for FlowPilot artifacts. After commit_flowscript returns queued/already_queued, stop workflow tools; if the request also includes UI, finish it with the UI tool. Cron/schedules are app Event setup on an eventsSimple() entry, never catalog nodes."
        } else {
            "Use FlowPilot's reviewed tools for board, UI, runtime, and app operations. Do not use shell or file-edit tools for FlowPilot artifacts. For read-only requests, inspect only what is needed and answer in normal text."
        };
        rmcp::model::ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_instructions(instructions)
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::ListToolsResult, rmcp::ErrorData>> + Send + '_
    {
        let mut tools = self
            .tools
            .values()
            .map(|tool| Self::to_mcp_tool(&tool.definition))
            .collect::<Vec<_>>();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        std::future::ready(Ok(rmcp::model::ListToolsResult {
            tools,
            ..Default::default()
        }))
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.tools
            .get(name)
            .map(|tool| Self::to_mcp_tool(&tool.definition))
    }

    fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::CallToolResult, rmcp::ErrorData>> + Send + '_
    {
        let tool_name = request.name.to_string();
        let tool = self.tools.get(tool_name.as_str()).cloned();
        let args = serde_json::Value::Object(request.arguments.unwrap_or_default());

        async move {
            if let Ok(mut activity) = self.tool_activity.lock() {
                activity.total_tool_calls = activity.total_tool_calls.saturating_add(1);
            }
            let workflow_operation_guard = if self.workflow_state.is_some()
                && is_order_sensitive_workflow_tool(&tool_name)
            {
                match self.workflow_operation_gate.clone().try_lock_owned() {
                    Ok(guard) => Some(guard),
                    Err(_) => {
                        return Ok(workflow_loop_result(
                            serde_json::json!({
                                "status": "edit_in_flight",
                                "next_action": "wait",
                                "message": "Another order-sensitive workflow operation is still running. Wait for its retained revision/status before issuing the next mutation."
                            }),
                            true,
                        ));
                    }
                }
            } else {
                None
            };
            // Register before workflow preflight so a phase boundary cannot observe quiescence in
            // the small window between `edit_in_flight = true` and spawning its blocking handler.
            let cancellation = context.ct.child_token();
            let handler_cancellation = cancellation.clone();
            let active_handler = register_mcp_active_handler(
                &self.tool_activity,
                &self.handler_quiescence,
                cancellation.clone(),
            )
            .map_err(|message| rmcp::ErrorData::internal_error(message, None))?;
            let mut cancellation_guard = McpToolCancellationGuard::new(cancellation.clone());

            if let Some(state) = &self.workflow_state
                && let Some(result) = workflow_database_setup_preflight(state, &tool_name, &args)
                    .or_else(|| workflow_predraft_context_preflight(state, &tool_name))
                    .or_else(|| workflow_tool_preflight_with_args(state, &tool_name, &args))
                    .or_else(|| workflow_candidate_preflight(state, &tool_name, &args))
            {
                return Ok(result);
            }

            let Some(tool) = tool else {
                return Err(rmcp::ErrorData::invalid_params(
                    format!("Unknown FlowPilot tool: {tool_name}"),
                    None,
                ));
            };

            let _progress_heartbeat =
                McpProgressHeartbeat::start(&context, cancellation.clone(), &tool_name);
            let definition_name = tool.definition.name.clone();
            let handler = tool.handler.clone();
            let recorded_args = args.clone();
            let recorded_tool_name = tool_name.clone();
            let workflow_state = self.workflow_state.clone();
            let tool_activity = self.tool_activity.clone();
            flowpilot_debug_trace!(tool = %definition_name, "FlowPilot MCP tool call started");

            // Inherit protocol-level `notifications/cancelled` as well as HTTP future drops. A
            // child token lets the Drop guard stop only this handler without cancelling sibling
            // requests that share the rmcp connection context.
            let task_result = tokio::task::spawn_blocking(move || {
                let _workflow_operation_guard = workflow_operation_guard;
                let _active_handler = active_handler;
                let mut result = super::frontend_tool_bridge::with_frontend_tool_execution_scope(
                    handler_cancellation,
                    None,
                    || (handler)(&definition_name, &args),
                );

                // Record and annotate on the blocking worker itself. If the MCP HTTP future is
                // dropped, its JoinHandle is detached; doing this only after `.await` left
                // `edit_in_flight` stuck and let a late result overwrite the next repair phase.
                if let Some(state) = &workflow_state {
                    workflow_tool_record(
                        state,
                        &recorded_tool_name,
                        &recorded_args,
                        &result.text_result_for_llm,
                    );
                    annotate_modular_fallback_result(state, &recorded_tool_name, &mut result);
                    suppress_unchanged_flowscript_source_echo(
                        &recorded_tool_name,
                        &recorded_args,
                        &mut result,
                    );
                }

                record_delegated_run_tool_progress(
                    &recorded_tool_name,
                    mcp_total_tool_calls(&tool_activity),
                    workflow_state.as_ref(),
                );

                if is_recoverable_platform_mutation(&recorded_tool_name)
                    && !flowpilot_tool_result_is_error(&result)
                    && let Ok(mut activity) = tool_activity.lock()
                {
                    activity.last_successful_mutation = Some(McpToolCompletion {
                        tool_name: recorded_tool_name,
                        result_text: result.text_result_for_llm.clone(),
                    });
                }

                result
            })
            .await;
            // Once the synchronous handler has settled there is no orphan left to cancel. If this
            // request future is dropped while awaiting the JoinHandle, Drop keeps the guard armed.
            cancellation_guard.disarm();
            let result = match task_result {
                Ok(result) => result,
                Err(error) => {
                    let message = format!("FlowPilot MCP tool task failed: {error}");
                    if let Some(state) = &self.workflow_state {
                        workflow_tool_abort(state, &tool_name, &message);
                    }
                    return Err(rmcp::ErrorData::internal_error(message, None));
                }
            };

            if result.result_type == "error" || result.error.is_some() {
                tracing::warn!(
                    tool = %tool_name,
                    error = ?result.error,
                    "FlowPilot MCP tool call returned an error"
                );
            } else {
                flowpilot_debug_trace!(tool = %tool_name, "FlowPilot MCP tool call completed");
            }

            Ok(flowpilot_tool_result_to_mcp(result))
        }
    }
}

#[derive(Debug, Clone)]
struct McpToolCompletion {
    tool_name: String,
    result_text: String,
}

#[derive(Debug, Default)]
struct McpToolActivityState {
    last_successful_mutation: Option<McpToolCompletion>,
    /// Total tool-call arrivals across every provider phase of this run. A phase whose delta is
    /// zero proves the CLI failed before doing any work, so its restart is accounted separately
    /// from the bounded workflow continuations.
    total_tool_calls: u64,
    next_handler_id: u64,
    active_handlers: HashMap<u64, CancellationToken>,
}

fn mcp_total_tool_calls(activity: &Arc<StdMutex<McpToolActivityState>>) -> u64 {
    activity
        .lock()
        .map(|activity| activity.total_tool_calls)
        .unwrap_or_default()
}

/// Membership guard for synchronous MCP handlers that may outlive their HTTP request future.
/// `spawn_blocking` cannot be force-aborted, so a provider phase is not allowed to hand off to a
/// repair process until every registered handler has observed cancellation and left this set.
struct McpActiveHandlerGuard {
    id: u64,
    activity: Arc<StdMutex<McpToolActivityState>>,
    quiescence: Arc<tokio::sync::Notify>,
}

impl Drop for McpActiveHandlerGuard {
    fn drop(&mut self) {
        if let Ok(mut activity) = self.activity.lock() {
            activity.active_handlers.remove(&self.id);
        }
        self.quiescence.notify_waiters();
    }
}

fn register_mcp_active_handler(
    activity: &Arc<StdMutex<McpToolActivityState>>,
    quiescence: &Arc<tokio::sync::Notify>,
    cancellation: CancellationToken,
) -> Result<McpActiveHandlerGuard, String> {
    let id = {
        let mut activity = activity
            .lock()
            .map_err(|_| "FlowPilot MCP handler registry is unavailable".to_string())?;
        activity.next_handler_id = activity.next_handler_id.wrapping_add(1).max(1);
        let id = activity.next_handler_id;
        activity.active_handlers.insert(id, cancellation);
        id
    };
    Ok(McpActiveHandlerGuard {
        id,
        activity: activity.clone(),
        quiescence: quiescence.clone(),
    })
}

fn is_recoverable_platform_mutation(tool_name: &str) -> bool {
    use flow_like::flow::copilot::tool_spec::{ToolApprovalSpec, find_global_tool_spec};

    find_global_tool_spec(tool_name)
        .is_some_and(|spec| !matches!(spec.approval, ToolApprovalSpec::None))
}

fn flowpilot_tool_result_is_error(result: &copilot_sdk::ToolResultObject) -> bool {
    let semantic_error = serde_json::from_str::<serde_json::Value>(&result.text_result_for_llm)
        .ok()
        .and_then(|value| {
            value
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .is_some_and(|status| workflow_status_requires_repair(&status));

    result.result_type == "error" || result.error.is_some() || semantic_error
}

fn flowpilot_tool_result_to_mcp(
    result: copilot_sdk::ToolResultObject,
) -> rmcp::model::CallToolResult {
    if flowpilot_tool_result_is_error(&result) {
        rmcp::model::CallToolResult::error(vec![rmcp::model::Content::text(
            result
                .error
                .unwrap_or_else(|| result.text_result_for_llm.clone()),
        )])
    } else {
        let mut contents = vec![rmcp::model::Content::text(result.text_result_for_llm)];
        if let Some(images) = result.binary_results_for_llm {
            contents.extend(images.into_iter().filter_map(|image| {
                image
                    .mime_type
                    .starts_with("image/")
                    .then(|| rmcp::model::Content::image(image.data, image.mime_type))
            }));
        }
        rmcp::model::CallToolResult::success(contents)
    }
}

struct FlowPilotMcpBridge {
    url: String,
    cancellation_token: rmcp::transport::streamable_http_server::StreamableHttpServerConfig,
    server_task: Option<tokio::task::JoinHandle<()>>,
    workflow_state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
    tool_activity: Arc<StdMutex<McpToolActivityState>>,
    handler_quiescence: Arc<tokio::sync::Notify>,
}

const FLOWPILOT_MCP_SSE_KEEP_ALIVE: Duration = Duration::from_secs(15);

fn flowpilot_mcp_server_config()
-> rmcp::transport::streamable_http_server::StreamableHttpServerConfig {
    use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;

    let mut config = StreamableHttpServerConfig::default();
    config.stateful_mode = true;
    // Keep long-running POST/SSE tool calls active through proxies and Claude Code's HTTP client.
    // Setting this to `None` made a quiet flowpilot_board request look dead and its transport was
    // dropped while the frontend/nested agent continued mutating in the background.
    config.sse_keep_alive = Some(FLOWPILOT_MCP_SSE_KEEP_ALIVE);
    config
}

impl FlowPilotMcpBridge {
    async fn start(
        tools: Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)>,
        workflow_state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
        tool_activity: Arc<StdMutex<McpToolActivityState>>,
    ) -> Result<Self, String> {
        use rmcp::transport::streamable_http_server::{
            StreamableHttpService, session::local::LocalSessionManager,
        };

        let tools = Arc::new(
            tools
                .into_iter()
                .map(|(definition, handler)| {
                    (
                        definition.name.clone(),
                        FlowPilotMcpTool {
                            definition,
                            handler,
                        },
                    )
                })
                .collect::<HashMap<_, _>>(),
        );

        let config = flowpilot_mcp_server_config();
        let cancellation_token = config.clone();
        let service_tools = tools.clone();
        let service_workflow_state = workflow_state.clone();
        let service_tool_activity = tool_activity.clone();
        let handler_quiescence = Arc::new(tokio::sync::Notify::new());
        let service_handler_quiescence = handler_quiescence.clone();
        let workflow_operation_gate = Arc::new(tokio::sync::Mutex::new(()));
        let service_workflow_operation_gate = workflow_operation_gate.clone();
        let service: StreamableHttpService<FlowPilotMcpServer, LocalSessionManager> =
            StreamableHttpService::new(
                move || {
                    Ok(FlowPilotMcpServer::new(
                        service_tools.clone(),
                        service_workflow_state.clone(),
                        service_tool_activity.clone(),
                        service_handler_quiescence.clone(),
                        service_workflow_operation_gate.clone(),
                    ))
                },
                Default::default(),
                config,
            );
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind FlowPilot MCP server: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("Failed to read FlowPilot MCP address: {e}"))?;
        let shutdown_token = cancellation_token.cancellation_token.clone();
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { shutdown_token.cancelled_owned().await })
                .await;
        });

        Ok(Self {
            url: format!("http://{addr}/mcp"),
            cancellation_token,
            server_task: Some(server_task),
            workflow_state,
            tool_activity,
            handler_quiescence,
        })
    }

    fn cancel_active_handlers(&self) -> Result<(), String> {
        let cancellations = self
            .tool_activity
            .lock()
            .map_err(|_| "FlowPilot MCP handler registry is unavailable".to_string())?
            .active_handlers
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for cancellation in cancellations {
            cancellation.cancel();
        }
        Ok(())
    }

    async fn wait_for_handler_quiescence(&self) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + EXTERNAL_AGENT_HANDLER_QUIESCENCE_TIMEOUT;
        loop {
            // Register the notification future before inspecting the count so a handler cannot
            // leave between the check and the await and strand us until the timeout.
            let notified = self.handler_quiescence.notified();
            let active = self
                .tool_activity
                .lock()
                .map_err(|_| "FlowPilot MCP handler registry is unavailable".to_string())?
                .active_handlers
                .len();
            if active == 0 {
                return Ok(());
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Err(format!(
                    "FlowPilot cancelled a provider phase, but {active} synchronous MCP handler(s) did not quiesce within {} seconds. The repair continuation was stopped to prevent stale commands from overlapping a newer phase.",
                    EXTERNAL_AGENT_HANDLER_QUIESCENCE_TIMEOUT.as_secs()
                ));
            }
        }
    }

    /// Close one phase-local MCP server and wait for every detached-capable handler before the
    /// next repair process is allowed to start. Each provider phase gets a fresh URL, so a late
    /// HTTP request from the old CLI cannot be mistaken for work belonging to the new phase.
    async fn finish_phase(mut self) -> Result<FlowPilotMcpPhaseOutcome, String> {
        self.cancellation_token.cancellation_token.cancel();
        let cancellation_result = self.cancel_active_handlers();
        let quiescence_result = self.wait_for_handler_quiescence().await;

        if let Some(mut server_task) = self.server_task.take()
            && tokio::time::timeout(EXTERNAL_AGENT_SHUTDOWN_TIMEOUT, &mut server_task)
                .await
                .is_err()
        {
            server_task.abort();
            let _ = server_task.await;
            flowpilot_debug_log!(
                "[flowpilot-mcp] graceful shutdown exceeded {:?}; server task aborted",
                EXTERNAL_AGENT_SHUTDOWN_TIMEOUT
            );
        }

        cancellation_result?;
        quiescence_result?;

        let workflow_snapshot = self.workflow_state.as_ref().and_then(|state| {
            state.lock().ok().map(|mut state| {
                // A handler that panicked or lost its HTTP future before recording a result can
                // still leave the logical owner set. Quiescence proves no old worker can race this
                // repair-state transition now.
                state.finish_interrupted_phase();
                state.snapshot()
            })
        });
        let last_successful_mutation = self
            .tool_activity
            .lock()
            .ok()
            .and_then(|activity| activity.last_successful_mutation.clone());
        Ok(FlowPilotMcpPhaseOutcome {
            workflow_snapshot,
            last_successful_mutation,
        })
    }
}

struct FlowPilotMcpPhaseOutcome {
    workflow_snapshot: Option<WorkflowToolLoopSnapshot>,
    last_successful_mutation: Option<McpToolCompletion>,
}

impl Drop for FlowPilotMcpBridge {
    fn drop(&mut self) {
        // `external_code_agent_chat_internal` can itself be cancelled by Tauri/the caller. Do not
        // leave the session-local listener alive merely because the async shutdown path was skipped.
        self.cancellation_token.cancellation_token.cancel();
        let _ = self.cancel_active_handlers();
        if let Some(server_task) = self.server_task.take() {
            server_task.abort();
        }
    }
}

struct ExternalAgentInvocation {
    backend: FlowPilotAgentBackendKind,
    executable: std::path::PathBuf,
    path_dirs: Vec<PathBuf>,
    args: Vec<String>,
    prompt: String,
    final_output_path: Option<std::path::PathBuf>,
    envs: Vec<(String, String)>,
}

/// Normalize the optional UI override. An omitted/blank value, or the explicit
/// `default` sentinel, lets the selected backend use its own configured model default.
fn explicit_reasoning_effort(reasoning_effort: Option<&str>) -> Option<&str> {
    reasoning_effort
        .map(str::trim)
        .filter(|effort| !effort.is_empty() && !effort.eq_ignore_ascii_case("default"))
}

impl ExternalAgentInvocation {
    fn new(
        backend: FlowPilotAgentBackendKind,
        cli: CliResolution,
        model_id: &str,
        reasoning_effort: Option<&str>,
        mcp_url: &str,
        prompt: String,
        tool_names: Vec<String>,
        images: &[ChatImage],
    ) -> Result<Self, String> {
        match backend {
            FlowPilotAgentBackendKind::Codex => Self::codex(
                backend,
                cli,
                model_id,
                reasoning_effort,
                mcp_url,
                prompt,
                images,
            ),
            FlowPilotAgentBackendKind::ClaudeCode => Self::claude(
                backend,
                cli,
                model_id,
                reasoning_effort,
                mcp_url,
                prompt,
                tool_names,
                images,
            ),
            FlowPilotAgentBackendKind::GithubCopilot => Err(
                "GitHub Copilot uses the direct SDK backend, not the external runner.".to_string(),
            ),
        }
    }

    fn codex(
        backend: FlowPilotAgentBackendKind,
        cli: CliResolution,
        model_id: &str,
        reasoning_effort: Option<&str>,
        mcp_url: &str,
        prompt: String,
        images: &[ChatImage],
    ) -> Result<Self, String> {
        // Mirrors @openai/codex-sdk's stdio protocol: spawn
        // `codex exec --experimental-json`, pass config overrides as repeated
        // --config entries, and stream JSONL events from stdout.
        let mut args = vec![
            "exec".to_string(),
            "--experimental-json".to_string(),
            // Keep authentication in CODEX_HOME, but do not inherit user-configured MCP servers,
            // browser tools, or web-search settings. FlowPilot must expose exactly its scoped MCP
            // surface: the global orchestrator gets the reviewed public-web tools, while Data
            // Studio and every other specialist get none.
            "--ignore-user-config".to_string(),
            "--sandbox".to_string(),
            "read-only".to_string(),
            "--cd".to_string(),
            // FlowPilot supplies its own scoped context and tools. A neutral cwd
            // prevents project discovery and macOS Desktop/Documents permission
            // prompts when the desktop app happened to inherit a protected cwd.
            std::env::temp_dir().display().to_string(),
            "--skip-git-repo-check".to_string(),
            "--config".to_string(),
            format!("mcp_servers.flowpilot.url={:?}", mcp_url),
            "--config".to_string(),
            "mcp_servers.flowpilot.startup_timeout_sec=10".to_string(),
            "--config".to_string(),
            // Outer bound for every FlowPilot MCP tool call. Must be >= the longest per-tool
            // `timeout_secs` in the shared platform tool specs (call_app_chat = 1800), or Codex
            // would abort interactive app chats at the MCP layer before their dialogs are answered.
            // Other tools return their own shorter bridge-timeout result well before this fires.
            "mcp_servers.flowpilot.tool_timeout_sec=1800".to_string(),
            "--config".to_string(),
            "mcp_servers.flowpilot.default_tools_approval_mode=\"approve\"".to_string(),
            "--config".to_string(),
            "features.use_rmcp_client=true".to_string(),
            "--config".to_string(),
            "approval_policy=\"never\"".to_string(),
            "--config".to_string(),
            // Keep this explicit even with --ignore-user-config: it prevents Codex defaults or
            // future profile layers from enabling native Responses web search independently of the
            // scoped MCP surface. Global research must use FlowPilot's reviewed tools, while nested
            // specialists must remain unable to reach the public web at all.
            "web_search=\"disabled\"".to_string(),
        ];
        // Model ids reach this point straight from Codex's own auth-aware catalog
        // (discovered via `codex app-server`'s `model/list`), so an explicit
        // selection is safe to forward. "default" defers to Codex's configured
        // runtime model by omitting `--model` entirely.
        if !model_id.trim().is_empty() && model_id != "default" {
            args.extend(["--model".to_string(), model_id.to_string()]);
        }
        if let Some(effort) = explicit_reasoning_effort(reasoning_effort) {
            // `codex exec` exposes model effort through its regular TOML config
            // override surface rather than a dedicated command-line flag.
            args.extend([
                "--config".to_string(),
                format!("model_reasoning_effort={effort:?}"),
            ]);
        }
        // `codex exec` attaches images to the initial prompt via repeated
        // `--image` flags; the prompt itself stays on stdin. The `=` form is
        // required: bare `--image <file>` parses greedily (num_args=1..) and
        // would swallow any argument appended after it.
        if !images.is_empty() {
            for path in write_chat_image_temp_files(images)? {
                args.push(format!("--image={}", path.display()));
            }
        }

        Ok(Self {
            backend,
            executable: cli.executable,
            path_dirs: cli.path_dirs,
            args,
            prompt,
            final_output_path: None,
            envs: Vec::new(),
        })
    }

    fn claude(
        backend: FlowPilotAgentBackendKind,
        cli: CliResolution,
        model_id: &str,
        reasoning_effort: Option<&str>,
        mcp_url: &str,
        prompt: String,
        tool_names: Vec<String>,
        images: &[ChatImage],
    ) -> Result<Self, String> {
        let mcp_config_path = std::env::temp_dir().join(format!(
            "flowpilot-claude-mcp-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "flowpilot": {
                    "type": "http",
                    "url": mcp_url,
                    // This is a small, session-local reviewed toolset. Preload it so Claude does
                    // not spend turns repeatedly invoking its built-in ToolSearch just to reveal
                    // get_current_flowscript/get_declarations and the retained FlowScript source
                    // lifecycle schemas.
                    "alwaysLoad": true
                }
            }
        });
        std::fs::write(
            &mcp_config_path,
            serde_json::to_vec_pretty(&mcp_config)
                .map_err(|e| format!("Failed to serialize Claude MCP config: {e}"))?,
        )
        .map_err(|e| format!("Failed to write Claude MCP config: {e}"))?;

        let mut args = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            // Stream assistant tokens as content_block_delta frames so FlowPilot
            // can render the reply live instead of only at the final result.
            "--include-partial-messages".to_string(),
            "--strict-mcp-config".to_string(),
            "--mcp-config".to_string(),
            mcp_config_path.display().to_string(),
        ];
        if !tool_names.is_empty() {
            let allowed_mcp_tools = tool_names
                .iter()
                .map(|name| format!("mcp__flowpilot__{name}"))
                .collect::<Vec<_>>()
                .join(",");
            // Do NOT pass `--tools` here: it controls which tools are visible in
            // context and only understands built-in tool names, so listing MCP
            // tools there hides the whole toolset and the agent degrades to
            // text-only answers. Allow the FlowPilot MCP tools, auto-deny
            // everything else via `dontAsk`, and strip the built-in file/shell
            // tools from context entirely so headless runs cannot stall on them.
            args.extend([
                "--allowedTools".to_string(),
                allowed_mcp_tools,
                "--disallowedTools".to_string(),
                "Task,Bash,Glob,Grep,Read,Edit,Write,NotebookEdit,WebFetch,WebSearch".to_string(),
                "--permission-mode".to_string(),
                "dontAsk".to_string(),
            ]);
        }
        if !model_id.trim().is_empty() && model_id != "default" {
            args.extend(["--model".to_string(), model_id.to_string()]);
        }
        if let Some(effort) = explicit_reasoning_effort(reasoning_effort) {
            args.extend(["--effort".to_string(), effort.to_string()]);
        }

        // Text-only turns deliver the prompt via stdin as plain text (`-p` reads
        // stdin when no positional prompt is given): the prompt embeds the whole
        // board as FlowScript and can exceed OS argv length limits, so it must
        // never be passed positionally. Image turns switch to stream-json stdin
        // input so the user message can carry Anthropic image content blocks
        // (requires --output-format stream-json, already set above). Either way
        // the stdin writer thread sends the payload and closes the pipe, which
        // ends the turn.
        let stdin_prompt = if images.is_empty() {
            prompt
        } else {
            args.extend(["--input-format".to_string(), "stream-json".to_string()]);
            let mut content = vec![serde_json::json!({ "type": "text", "text": prompt })];
            for image in images {
                content.push(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": image.media_type,
                        "data": image.data,
                    }
                }));
            }
            let message = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": content }
            });
            let mut line = serde_json::to_string(&message)
                .map_err(|e| format!("Failed to serialize Claude user message: {e}"))?;
            line.push('\n');
            line
        };

        Ok(Self {
            backend,
            executable: cli.executable,
            path_dirs: cli.path_dirs,
            args,
            // Delivered via stdin (`-p` reads it when no positional prompt is
            // given). Text turns send the plain prompt; image turns send a
            // stream-json user message. Either way it can embed the whole board
            // as FlowScript and exceed OS argv length limits, so it stays off argv.
            prompt: stdin_prompt,
            // Claude Code applies MCP_TOOL_TIMEOUT as the overall MCP-call bound.
            // FlowPilot's frontend-bridge tools (e.g. UI generation via
            // flowpilot_widget) can legitimately run longer, so raise it to match
            // Codex's 1800s bound and avoid premature aborts.
            envs: vec![
                ("MCP_TOOL_TIMEOUT".to_string(), "1800000".to_string()),
                // Claude Code also has an independent no-progress watchdog for MCP calls. A
                // nested FlowPilot board run can legitimately stay silent while the delegated
                // model reasons or waits for a frontend operation, so the 300s default would
                // cancel a healthy request even though MCP_TOOL_TIMEOUT permits it. Disable this
                // session-local idle watchdog; FlowPilot still owns explicit cancellation and
                // its individual frontend/tool lifecycle bounds.
                (
                    "CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT".to_string(),
                    "0".to_string(),
                ),
                // Auto preloads tool definitions when they fit comfortably in context and falls
                // back to ToolSearch only for genuinely large surfaces. `alwaysLoad` above keeps
                // this session-local FlowPilot server on the preload path.
                ("ENABLE_TOOL_SEARCH".to_string(), "auto".to_string()),
            ],
            final_output_path: Some(mcp_config_path),
        })
    }
}

fn build_external_agent_prompt(
    system_content: &str,
    user_prompt: &str,
    workflow_edit_request: bool,
) -> String {
    let workflow_loop = if workflow_edit_request {
        r#"
THIS IS A WORKFLOW MUTATION RUN. Follow this bounded loop exactly:
1. FlowScript is the ONE model-authored representation for executable workflow behavior. Direct commands are reserved for visual/layout and non-FlowScript changes; never author workflow logic as command JSON.
2. Read get_current_flowscript once. Plan the whole request, then make ONE bounded, focused get_declarations batch for only the highest-leverage catalog calls needed to establish the end-to-end shape. Never enumerate every utility or guess a declaration or pin. Use at most six ancillary database/UI/storage inspections before the first write.
3. After any usable declaration result, call write_flowscript IMMEDIATELY with a stable draft id and a full-shape checkpoint that preserves the complete requested scope. It may retain compiler diagnostics; that is recoverable progress, not success. Do not chase omitted/unmatched declaration queries first. For an existing board, edit the exact returned document and preserve every kept //@n anchor. For a new board, author real functions and Event entries with concrete catalog calls.
4. If compilation fails, repair the SAME retained source with patch_flowscript. A coherent whole-document rewrite may use write_flowscript with the same draft id and `replace_existing: true`; then use the newly returned revision. Call check_flowscript next. Structured line/column, declaration, pin, type and execution diagnostics are authoritative. A newly named missing declaration permits one bounded deduplicated lookup; never restart broad discovery.
5. Call commit_flowscript at the latest checked revision. Only commit may create the exact review claim. Preserve every requested capability, helper, variable and Event across retries; a tiny smoke test, empty Event, or reduced workflow never counts as success.
6. When commit_flowscript returns `queued`/`already_queued`, stop workflow tools. If the user also requested UI, finish it with emit_ui; otherwise summarize briefly.

Helper rule: every helper declaration requires the literal keyword `function`, for example `function fetchMail(...) { ... }`. A bare `fetchMail(...) { ... }` block is not a helper. Keep each helper declaration in the same full document as its calls; never invent helper calls and expect them to resolve as catalog nodes. If a helper returns a value, declare a named return signature such as `function classify(...): (isSupport: bool) { ...; return result.value }`.

Entry-node rule: cron/schedules are app Event setup on an `eventsSimple()` entry, never catalog nodes. Use `eventsGeneric(payload: Struct, fieldName: string, ...)` for request/form payloads with typed field pins; parameters after payload create those pins on a new Generic entry. Use `eventsChat(...)` for chat context. This board run creates the compatible entry and logic; the outer platform assistant configures the Event record/sink afterwards.
"#
    } else {
        ""
    };
    format!(
        r#"SYSTEM INSTRUCTIONS
{system_content}

You are running through an external code-agent CLI connected to FlowPilot's shared MCP tools. Do not use shell/file-edit tools for workflow or UI edits. Use the FlowPilot MCP tools. Author all executable workflow behavior as FlowScript source; the host compiler owns the typed AST, validation, exact command claim, and application. Preserve all kept `//@n:<id>` anchors. If the user asks for explanation or no edit is needed, answer in normal text.
{workflow_loop}

USER REQUEST
{user_prompt}"#
    )
}

fn build_external_workflow_continuation_prompt(
    original_user_prompt: &str,
    snapshot: Option<&WorkflowToolLoopSnapshot>,
    attempt: u8,
) -> String {
    let status = snapshot
        .and_then(|state| state.last_status.as_deref())
        .or_else(|| {
            snapshot
                .filter(|state| {
                    state.last_declarations.is_some()
                        && state.flowscript_operation_attempts == 0
                        && state.typed_operation_attempts == 0
                })
                .map(|_| "declarations_ready_no_source")
        })
        .unwrap_or("no_edit_submitted");
    let errors = snapshot
        .filter(|state| !state.last_errors.is_empty())
        .map(|state| {
            let total = state.last_errors.len();
            let mut listed = state
                .last_errors
                .iter()
                .take(MAX_TERMINAL_REPORT_DIAGNOSTICS)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n- ");
            if total > MAX_TERMINAL_REPORT_DIAGNOSTICS {
                listed.push_str(&format!(
                    "\n- (+{} more diagnostics omitted here; check_flowscript returns the full list)",
                    total - MAX_TERMINAL_REPORT_DIAGNOSTICS
                ));
            }
            format!("\nValidation diagnostics ({total} total):\n- {listed}\n")
        })
        .unwrap_or_default();
    let structured_diagnostics = snapshot
        .filter(|state| !state.last_structured_diagnostics.is_empty())
        .and_then(|state| {
            serde_json::to_string_pretty(&state.last_structured_diagnostics)
                .ok()
                .map(|diagnostics| (state.last_structured_diagnostics.len(), diagnostics))
        })
        .map(|(count, diagnostics)| {
            format!(
                "\nSTRUCTURED ROOT DIAGNOSTICS ({count} retained; a `truncated` entry marks host-side omissions) (preserve spans, pins, expected/actual values, and exact fixes):\n```json\n{diagnostics}\n```\n"
            )
        })
        .unwrap_or_default();
    let typed_mode =
        snapshot.is_some_and(|state| state.mutation_path == Some(WorkflowMutationPath::TypedIr));
    let retained_source_mode = snapshot
        .is_some_and(|state| state.flowscript_draft_retained && state.last_flowscript.is_some());
    let draft = if typed_mode {
        snapshot
            .map(|state| {
                if state.typed_draft_retained {
                    format!(
                        "\nRETAINED TYPED DRAFT: draft_id={}, latest revision={}. Continue the exact typed draft with upsert/validate/commit tools; do not edit generated FlowScript text or start another draft. Missing modules: [{}].\n",
                        state.typed_draft_id.as_deref().unwrap_or("<unknown>"),
                        state
                            .typed_revision
                            .map(|revision| revision.to_string())
                            .unwrap_or_else(|| "<unknown>".to_string()),
                        state.typed_missing_modules.join(", ")
                    )
                } else {
                    format!(
                        "\nTYPED DRAFT WAS NOT STARTED: attempted draft_id={}, no retained revision exists. Repair the capability plan or begin arguments before retrying; do not claim this attempted id is resumable and do not edit generated FlowScript text.\n",
                        state.typed_draft_id.as_deref().unwrap_or("<unknown>")
                    )
                }
            })
            .unwrap_or_default()
    } else {
        snapshot
            .and_then(|state| {
                state
                    .last_flowscript
                    .as_deref()
                    .map(|source| (state.flowscript_draft_retained, source))
            })
            .map(|(retained, source)| {
                if retained {
                    format!(
                        "\nLATEST FLOWSCRIPT DRAFT TO REVISE (keep the complete source and repair it in place):\n```flowscript\n{source}\n```\n"
                    )
                } else {
                    format!(
                        "\nUNCLAIMED FLOWSCRIPT SOURCE REFERENCE (preserve its requested behavior, but write it under a fresh draft id before patch/check/commit):\n```flowscript\n{source}\n```\n"
                    )
                }
            })
            .unwrap_or_else(|| {
                "\nNo FlowScript draft was submitted. Reuse any retained current source and declarations, then call write_flowscript immediately with a full-shape implementation checkpoint. Do not postpone the first retained source for exhaustive discovery.\n".to_string()
            })
    };
    let declarations = snapshot
        .and_then(|state| state.last_declarations.as_deref())
        .map(|result| {
            format!(
                "\nDECLARATIONS ALREADY FETCHED BY THE PREVIOUS PROCESS (reuse these; do not search again):\n{result}\n"
            )
        })
        .unwrap_or_default();
    let unresolved_declarations = snapshot
        .filter(|state| {
            !state.declaration_lookup_complete
                && state.last_declarations.is_none()
                && !state.unresolved_declaration_queries.is_empty()
        })
        .map(|state| {
            format!(
                "\nUNRESOLVED DECLARATION COVERAGE (query only these missing capabilities; do not guess):\n- {}\n",
                state.unresolved_declaration_queries.join("\n- ")
            )
        })
        .unwrap_or_default();
    let repair_declarations = snapshot
        .filter(|state| !state.last_repair_declarations.is_empty())
        .map(|state| {
            format!(
                "\nEXACT LIVE-CATALOG REPAIR DECLARATIONS INJECTED BY THE LATEST VALIDATION (use these signatures directly; if several candidates are shown, choose by intended semantics instead of guessing):\n{}\n",
                state.last_repair_declarations.join("\n")
            )
        })
        .unwrap_or_default();
    let prior_attempts = snapshot
        .map(|state| state.edit_attempts)
        .unwrap_or_default();
    let source_operations = snapshot
        .map(|state| state.flowscript_operation_attempts)
        .unwrap_or_default();
    let retained_revision = snapshot
        .filter(|state| state.flowscript_draft_retained)
        .map(|state| {
            format!(
                "\nRETAINED SOURCE SESSION: draft_id={}, revision={}. Continue this exact draft id and expected_revision; patch or check it instead of starting another source session.\n",
                state.flowscript_draft_id.as_deref().unwrap_or("<unknown>"),
                state
                    .flowscript_revision
                    .map(|revision| revision.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string())
            )
        })
        .unwrap_or_default();

    let continuation_action = if typed_mode {
        "Continue only the typed-IR lifecycle selected by the retained state. Repair the same module/draft, validate it, and call commit_flow_ir_draft at the latest revision. Do not switch to FlowScript text or another mutation representation."
    } else if retained_source_mode {
        "Continue the SAME retained FlowScript draft. Repair it through write_flowscript/patch_flowscript, run check_flowscript, and call commit_flowscript at the latest revision. Do not repeat broad searches or restart with a smaller candidate."
    } else {
        "No source draft is retained yet. Continue the bounded pre-draft lifecycle: reuse any retained declarations and call write_flowscript immediately with a full-shape checkpoint. Do not resolve every omitted or unmatched query first; use compiler diagnostics for narrow follow-ups, then check and commit."
    };

    format!(
        r#"INTERNAL FLOWPILOT EXTERNAL CONTINUATION #{attempt}
The previous CLI turn ended without queueing workflow changes (last status: {status}, prior checks: {prior_attempts}, source operations: {source_operations}/{MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS}). Nothing has been applied.
{errors}{structured_diagnostics}{draft}{retained_revision}{declarations}{unresolved_declarations}{repair_declarations}
{continuation_action} The turn is complete only when commit returns `queued`/`already_queued` or the bounded repair budget reports its final compiler diagnostics.

Original user request:
{original_user_prompt}"#
    )
}

const MAX_TERMINAL_REPORT_DIAGNOSTICS: usize = 20;

fn nested_wall_clock_exhausted(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|deadline| Instant::now() >= deadline)
}

/// Terminal report for a nested run stopped at its wall-clock budget. It reuses the shared
/// incomplete-error path so the waiting outer agent receives the retained draft id/revision and
/// every retained diagnostic, plus an honest statement that the budget — not the work — ended
/// the run.
fn nested_wall_clock_incomplete_error(
    snapshot: Option<&WorkflowToolLoopSnapshot>,
    provider_continuations: u8,
) -> String {
    format!(
        "NESTED_RUN_WALL_CLOCK_BUDGET_EXHAUSTED: this nested FlowPilot run reached its {}-minute wall-clock budget and was stopped gracefully; this result is terminal for this run. {}",
        NESTED_RUN_WALL_CLOCK_BUDGET.as_secs() / 60,
        external_workflow_incomplete_error_with_fallback(
            snapshot,
            provider_continuations,
            "nested wall-clock budget",
        )
    )
}

fn external_workflow_incomplete_error(
    snapshot: Option<&WorkflowToolLoopSnapshot>,
    provider_continuations: u8,
) -> String {
    external_workflow_incomplete_error_with_fallback(
        snapshot,
        provider_continuations,
        "provider continuation budget",
    )
}

fn external_workflow_incomplete_error_with_fallback(
    snapshot: Option<&WorkflowToolLoopSnapshot>,
    provider_continuations: u8,
    fallback_exhausted: &str,
) -> String {
    let status = snapshot
        .and_then(|state| state.last_status.as_deref())
        .or_else(|| {
            snapshot
                .filter(|state| {
                    state.last_declarations.is_some()
                        && state.flowscript_operation_attempts == 0
                        && state.typed_operation_attempts == 0
                })
                .map(|_| "declarations_ready_no_source")
        })
        .unwrap_or("no_edit_submitted");
    let exhausted = snapshot
        .and_then(|state| state.exhausted_budget.as_deref())
        .unwrap_or(fallback_exhausted);
    let budgets = snapshot
        .map(|state| {
            format!(
                "provider continuations {provider_continuations}/{MAX_EXTERNAL_WORKFLOW_CONTINUATIONS}, checks {}/{MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS}, source operations {}/{MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS}, stalled repeats {}/{MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS}, commit attempts {}/{MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS}",
                state.edit_attempts,
                state.flowscript_operation_attempts,
                state.stalled_edit_attempts,
                state.flowscript_commit_attempts,
            )
        })
        .unwrap_or_else(|| {
            format!(
                "provider continuations {provider_continuations}/{MAX_EXTERNAL_WORKFLOW_CONTINUATIONS}"
            )
        });
    let source_state = snapshot
        .filter(|state| state.flowscript_draft_retained)
        .map(|state| {
            format!(
                " Retained FlowScript draft: draft_id={}, revision={}. A follow-up repair run can resume this exact draft only when it originates from the same user request.",
                state.flowscript_draft_id.as_deref().unwrap_or("unknown"),
                state
                    .flowscript_revision
                    .map(|revision| revision.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            )
        })
        .unwrap_or_default();
    let typed_state = snapshot
        .filter(|state| state.typed_operation_attempts > 0)
        .map(|state| {
            let retention = if state.typed_draft_retained {
                "Retained typed draft"
            } else {
                "Typed draft was not retained"
            };
            format!(
                " {retention}: draft_id={}, revision={}, operations={}/{}, missing_modules=[{}].",
                state.typed_draft_id.as_deref().unwrap_or("unknown"),
                state
                    .typed_revision
                    .map(|revision| revision.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                state.typed_operation_attempts,
                state.typed_operation_budget,
                state.typed_missing_modules.join(", ")
            )
        })
        .unwrap_or_default();
    let diagnostics = snapshot
        .filter(|state| !state.last_errors.is_empty())
        .map(|state| {
            let total = state.last_errors.len();
            let mut rendered = state
                .last_errors
                .iter()
                .take(MAX_TERMINAL_REPORT_DIAGNOSTICS)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("; ");
            if total > MAX_TERMINAL_REPORT_DIAGNOSTICS {
                rendered.push_str(&format!(
                    " (+{} more)",
                    total - MAX_TERMINAL_REPORT_DIAGNOSTICS
                ));
            }
            format!(" Remaining diagnostics ({total} total): {rendered}.")
        })
        .unwrap_or_default();
    let structured = snapshot
        .filter(|state| !state.last_structured_diagnostics.is_empty())
        .map(|state| {
            let total = state.last_structured_diagnostics.len();
            let mut rendered = state
                .last_structured_diagnostics
                .iter()
                .take(MAX_TERMINAL_REPORT_DIAGNOSTICS)
                .map(|entry| serde_json::to_string(entry).unwrap_or_else(|_| entry.to_string()))
                .collect::<Vec<_>>()
                .join(" ");
            if total > MAX_TERMINAL_REPORT_DIAGNOSTICS {
                rendered.push_str(&format!(
                    " (+{} more)",
                    total - MAX_TERMINAL_REPORT_DIAGNOSTICS
                ));
            }
            format!(" Structured diagnostics ({total} retained): {rendered}")
        })
        .unwrap_or_default();
    format!(
        "The external agent exhausted its {exhausted} without queueing changes (last status: {status}; budgets: {budgets}).{source_state}{typed_state}{diagnostics}{structured}"
    )
}

async fn run_external_agent_invocation(
    invocation: ExternalAgentInvocation,
    channel: Channel<String>,
    parent_request_id: Option<String>,
    cancellation: CancellationToken,
) -> Result<ExternalAgentRunOutput, String> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

    struct TemporaryOutputCleanup(Option<PathBuf>);
    impl Drop for TemporaryOutputCleanup {
        fn drop(&mut self) {
            if let Some(path) = self.0.as_ref() {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    let _temporary_output_cleanup = TemporaryOutputCleanup(invocation.final_output_path.clone());
    let mut command = tokio::process::Command::new(&invocation.executable);
    command
        .args(&invocation.args)
        // Claude inherits the process cwd, while Codex also receives the matching
        // --cd above. Neither should inspect an incidental Finder/Dock launch path.
        .current_dir(std::env::temp_dir())
        .env("PATH", augmented_path_with_dirs(&invocation.path_dirs))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in &invocation.envs {
        command.env(key, value);
    }

    let mut child = command.spawn().map_err(|e| {
        format!(
            "Failed to start {} CLI at {}: {e}",
            invocation.backend.label(),
            invocation.executable.display()
        )
    })?;

    // Write the prompt concurrently with stdout/stderr draining. A full stdin pipe must not block
    // the runtime or prevent the watchdog from killing an unresponsive CLI.
    let stdin_handle = match child.stdin.take() {
        Some(mut stdin) if !invocation.prompt.is_empty() => {
            let prompt = invocation.prompt.clone();
            let backend_label = invocation.backend.label();
            Some(tokio::spawn(async move {
                stdin
                    .write_all(prompt.as_bytes())
                    .await
                    .map_err(|e| format!("Failed to send prompt to {backend_label}: {e}"))?;
                stdin
                    .flush()
                    .await
                    .map_err(|e| format!("Failed to flush prompt to {backend_label}: {e}"))
            }))
        }
        _ => None,
    };

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{} did not expose stdout", invocation.backend.label()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{} did not expose stderr", invocation.backend.label()))?;

    let stderr_handle = tokio::spawn(async move {
        let mut stderr = tokio::io::BufReader::new(stderr);
        let mut retained = String::new();
        let mut buffer = [0u8; 8 * 1024];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => {
                    let chunk = String::from_utf8_lossy(&buffer[..read]);
                    append_bounded_tail(&mut retained, &chunk, EXTERNAL_AGENT_STDERR_MAX_BYTES);
                }
                Err(error) => {
                    append_bounded_tail(
                        &mut retained,
                        &format!("\n[failed reading stderr: {error}]"),
                        EXTERNAL_AGENT_STDERR_MAX_BYTES,
                    );
                    break;
                }
            }
        }
        retained
    });

    let mut final_text = String::new();
    let mut streamed_text = String::new();
    let mut fatal_error: Option<String> = None;
    let mut stream_state = ExternalAgentStreamState::default();
    let stream_result = {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        let drain_stdout = async {
            while let Some(line) = lines.next_line().await.map_err(|error| {
                format!(
                    "Failed to read {} output: {error}",
                    invocation.backend.label()
                )
            })? {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                    if let Some(error) = external_agent_error_text(&value) {
                        // Keep draining the stream so partial/final text is preserved; the error is
                        // surfaced after the process exits instead of aborting the run mid-stream.
                        send_external_progress_event(
                            &channel,
                            EXTERNAL_AGENT_TOOL_CALL_ID,
                            &format!("{} reported an error: {error}", invocation.backend.label()),
                            parent_request_id.as_deref(),
                        );
                        fatal_error.get_or_insert(error);
                    }

                    // A failed FlowPilot MCP connection leaves the agent tool-less: it will answer
                    // in plain text and "succeed" without editing. Treat that as a terminal failure.
                    if let Some(error) = external_agent_mcp_connect_failure(&value) {
                        send_external_progress_event(
                            &channel,
                            EXTERNAL_AGENT_TOOL_CALL_ID,
                            &error,
                            parent_request_id.as_deref(),
                        );
                        fatal_error.get_or_insert(error);
                    }

                    // Codex exposes MCP arguments on item.updated/item.completed. Publish the
                    // source as soon as it is present so the user can inspect the program before
                    // the compiler result arrives.
                    if invocation.backend == FlowPilotAgentBackendKind::Codex
                        && let Some(frame) = external_agent_flowscript_workspace_event(&value)
                    {
                        let frame = correlate_stream_frame(&frame, parent_request_id.as_deref());
                        let _ = channel.send(frame);
                    }

                    let tool_events = if invocation.backend == FlowPilotAgentBackendKind::ClaudeCode
                    {
                        claude_agent_tool_events(&value, &mut stream_state)
                    } else {
                        external_agent_process_event(&value).into_iter().collect()
                    };
                    if tool_events.is_empty() {
                        if let Some(label) = external_agent_progress_label(&value) {
                            send_external_progress_event(
                                &channel,
                                EXTERNAL_AGENT_TOOL_CALL_ID,
                                &label,
                                parent_request_id.as_deref(),
                            );
                        }
                    } else {
                        for event in tool_events {
                            let event =
                                correlate_stream_frame(&event, parent_request_id.as_deref());
                            let _ = channel.send(event);
                        }
                    }

                    if let Some(delta) =
                        external_agent_stream_delta(invocation.backend, &value, &mut stream_state)
                        && !delta.is_empty()
                    {
                        append_bounded_text(
                            &mut streamed_text,
                            &delta,
                            EXTERNAL_AGENT_TEXT_MAX_BYTES,
                        );
                        let _ = channel.send(delta);
                    }
                    if let Some(result) = external_agent_result_text(invocation.backend, &value) {
                        final_text.clear();
                        append_bounded_text(
                            &mut final_text,
                            &result,
                            EXTERNAL_AGENT_TEXT_MAX_BYTES,
                        );
                    }
                } else {
                    send_external_progress_event(
                        &channel,
                        EXTERNAL_AGENT_TOOL_CALL_ID,
                        &flow_like::flow::copilot::stream::safe_text_preview(&line, 1_200),
                        parent_request_id.as_deref(),
                    );
                }
            }
            Ok::<(), String>(())
        };
        tokio::pin!(drain_stdout);
        tokio::select! {
            result = &mut drain_stdout => result,
            _ = cancellation.cancelled() => Err("FlowPilot external agent run was cancelled".to_string()),
        }
    };

    let mut forced_stop = stream_result.as_ref().err().cloned();
    let status = if forced_stop.is_none() {
        tokio::select! {
            result = child.wait() => Some(result.map_err(|error| {
                format!("Failed to wait for {}: {error}", invocation.backend.label())
            })?),
            _ = cancellation.cancelled() => {
                forced_stop = Some("FlowPilot external agent run was cancelled".to_string());
                None
            }
        }
    } else {
        None
    };

    let status = match status {
        Some(status) => Some(status),
        None => {
            let _ = child.start_kill();
            tokio::time::timeout(EXTERNAL_AGENT_SHUTDOWN_TIMEOUT, child.wait())
                .await
                .ok()
                .and_then(Result::ok)
        }
    };

    let stderr_text = tokio::time::timeout(EXTERNAL_AGENT_SHUTDOWN_TIMEOUT, stderr_handle)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let stdin_error = match stdin_handle {
        Some(handle) if handle.is_finished() => handle.await.ok().and_then(Result::err),
        Some(handle) => {
            handle.abort();
            None
        }
        None => None,
    };

    if let Some(path) = &invocation.final_output_path {
        if invocation.backend == FlowPilotAgentBackendKind::Codex
            && let Ok(text) = std::fs::read_to_string(path)
            && !text.trim().is_empty()
        {
            final_text.clear();
            append_bounded_text(&mut final_text, &text, EXTERNAL_AGENT_TEXT_MAX_BYTES);
        }
    }

    if final_text.trim().is_empty() {
        final_text = streamed_text;
    }
    let text = final_text.trim().to_string();

    let mut error = fatal_error;
    if let Some(stop_error) = forced_stop {
        error = Some(match error {
            Some(existing) => format!("{existing}\n{stop_error}"),
            None => stop_error,
        });
    } else if let Some(status) = status.filter(|status| !status.success()) {
        let exit_error = format!(
            "{} exited with status {}{}",
            invocation.backend.label(),
            status,
            if stderr_text.is_empty() {
                String::new()
            } else {
                format!(":\n{stderr_text}")
            }
        );
        error = Some(match error {
            Some(existing) => format!("{existing}\n{exit_error}"),
            None => exit_error,
        });
    } else if error.is_none() {
        error = stdin_error;
    }

    match (text.is_empty(), error) {
        (true, Some(error)) => Err(error),
        (_, error) => Ok(ExternalAgentRunOutput { text, error }),
    }
}

#[derive(Default)]
struct ExternalAgentStreamState {
    agent_message_text_by_id: HashMap<String, String>,
    last_agent_message_id: Option<String>,
    has_streamed_assistant_text: bool,
    // Claude reports a tool's name only on the `tool_use` block; the matching
    // `tool_result` carries just the id, so remember id -> display name here.
    claude_tool_names: HashMap<String, String>,
    // Claude streams tool JSON by content-block index before it emits the complete assistant
    // message. Keep that transient index -> call-id mapping so FlowScript can appear while the
    // model is still writing the `source` JSON string.
    claude_tool_call_ids_by_index: HashMap<u64, String>,
    claude_flowscript_preview: flow_like::flow::copilot::stream::FlowScriptToolCallPreviewTracker,
}

impl ExternalAgentStreamState {
    fn decorate_agent_delta(&mut self, item_id: &str, delta: &str) -> String {
        if delta.is_empty() {
            return String::new();
        }

        let mut out = String::new();
        if self.has_streamed_assistant_text
            && self.last_agent_message_id.as_deref() != Some(item_id)
            && !delta.starts_with('\n')
        {
            out.push_str("\n\n");
        }
        self.last_agent_message_id = Some(item_id.to_string());
        self.has_streamed_assistant_text = true;
        out.push_str(delta);
        out
    }
}

fn external_debug_value_preview(value: &serde_json::Value, max_chars: usize) -> String {
    match value {
        serde_json::Value::String(text) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(parsed) => flow_like::flow::copilot::stream::safe_json_preview(&parsed, max_chars),
            Err(_) => flow_like::flow::copilot::stream::safe_text_preview(text, max_chars),
        },
        value => flow_like::flow::copilot::stream::safe_json_preview(value, max_chars),
    }
}

fn external_tool_result_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("text").and_then(serde_json::Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(content) = value.get("content") {
        if let Some(text) = content.as_str() {
            return Some(text.to_string());
        }
        if let Some(entries) = content.as_array() {
            let text = entries
                .iter()
                .filter_map(|entry| {
                    entry
                        .get("text")
                        .or_else(|| entry.get("content"))
                        .and_then(serde_json::Value::as_str)
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn external_result_details(
    result: Option<&serde_json::Value>,
    fallback_status: Option<&str>,
    error: Option<&str>,
) -> (String, String, String, Option<String>) {
    let result_text = result.and_then(external_tool_result_text);
    let terminal_status = result_text
        .as_deref()
        .and_then(extract_json_status)
        .or_else(|| fallback_status.map(str::to_string))
        .unwrap_or_else(|| {
            if error.is_some() {
                "error"
            } else {
                "completed"
            }
            .to_string()
        });
    let status = if error.is_some()
        || !matches!(
            terminal_status.trim().to_ascii_lowercase().as_str(),
            "ok" | "done"
                | "success"
                | "draft_started"
                | "draft_updated"
                | "valid"
                | "queued"
                | "already_queued"
                | "applied"
                | "completed"
                | "rendered"
        ) {
        "error"
    } else {
        "done"
    }
    .to_string();
    let result_summary = error
        .map(|error| flow_like::flow::copilot::stream::safe_text_preview(error, 600))
        .unwrap_or_else(|| {
            result_text
                .as_deref()
                .filter(|text| serde_json::from_str::<serde_json::Value>(text).is_ok())
                .map(flow_like::flow::copilot::stream::tool_result_summary)
                .unwrap_or_else(|| terminal_status.replace('_', " "))
        });
    let result_preview = result_text
        .as_deref()
        .map(|text| {
            flow_like::flow::copilot::stream::safe_tool_result_preview(
                text,
                flow_like::flow::copilot::stream::TOOL_RESULT_PREVIEW_CHARS,
            )
        })
        .or_else(|| {
            result.map(|value| {
                external_debug_value_preview(
                    value,
                    flow_like::flow::copilot::stream::TOOL_RESULT_PREVIEW_CHARS,
                )
            })
        });
    (status, terminal_status, result_summary, result_preview)
}

fn external_agent_process_event(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if !matches!(event_type, "item.started" | "item.completed") {
        return None;
    }

    let item = value.get("item")?;
    let item_type = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if item_type != "mcp_tool_call" {
        return None;
    }

    let tool_name = item
        .get("tool")
        .or_else(|| item.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("tool");
    let server_name = item
        .get("server")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("flowpilot");
    let tool_call_id = item
        .get("id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("external-{server_name}-{tool_name}"));

    if event_type == "item.started" {
        let arguments_preview =
            item.get("arguments")
                .or_else(|| item.get("input"))
                .map(|arguments| {
                    external_debug_value_preview(
                        arguments,
                        flow_like::flow::copilot::stream::TOOL_ARGUMENT_PREVIEW_CHARS,
                    )
                });
        return Some(flowpilot_stream_tag(
            "tool_start",
            &serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool": tool_name,
                "status": "running",
                "summary": format!("{server_name}/{tool_name}"),
                "arguments_preview": arguments_preview,
            }),
        ));
    }

    let error = item
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let fallback_status = item.get("status").and_then(serde_json::Value::as_str);
    let (status, terminal_status, result_summary, result_preview) = external_result_details(
        item.get("result").or_else(|| item.get("output")),
        fallback_status,
        error.as_deref(),
    );
    Some(flowpilot_stream_tag(
        "tool_end",
        &serde_json::json!({
            "tool_call_id": tool_call_id,
            "tool": tool_name,
            "status": status,
            "terminal_status": terminal_status,
            "result_summary": result_summary,
            "result_preview": result_preview,
            "error": error.map(|error| flow_like::flow::copilot::stream::safe_text_preview(&error, 600)),
        }),
    ))
}

/// Detect a failed FlowPilot MCP server connection in Claude Code's `system`/`init` frame
/// (`{"type":"system","subtype":"init","mcp_servers":[{"name":…,"status":…}]}`).
fn external_agent_mcp_connect_failure(value: &serde_json::Value) -> Option<String> {
    if value.get("type").and_then(serde_json::Value::as_str) != Some("system")
        || value.get("subtype").and_then(serde_json::Value::as_str) != Some("init")
    {
        return None;
    }
    let servers = value.get("mcp_servers")?.as_array()?;
    let failed: Vec<String> = servers
        .iter()
        .filter_map(|server| {
            let name = server.get("name").and_then(serde_json::Value::as_str)?;
            let status = server
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            matches!(status, "failed" | "error" | "disconnected")
                .then(|| format!("`{name}` (status: {status})"))
        })
        .collect();
    if failed.is_empty() {
        return None;
    }
    Some(format!(
        "MCP server connection failed: {} — the FlowPilot tools are unavailable, so this run cannot edit the board or UI",
        failed.join(", ")
    ))
}

/// Translate a Codex full-source authoring `mcp_tool_call` item into a workspace preview frame.
/// The frontend treats this `submitted` preview as non-authoritative; only the later `queued`
/// commit workspace owns application and command suppression.
#[cfg_attr(not(test), allow(dead_code))]
fn external_agent_flowscript_workspace_event(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) {
        return None;
    }

    let item = value.get("item")?;
    let item_type = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if item_type != "mcp_tool_call" {
        return None;
    }

    let tool_name = item
        .get("tool")
        .or_else(|| item.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let display_tool_name = claude_display_tool_name(tool_name);

    // Completion is authoritative: source lifecycle results contain the exact retained document,
    // revision and compiler status. Prefer it over the repeated call arguments on item.completed.
    if event_type == "item.completed" && is_flowscript_draft_operation_tool(display_tool_name) {
        let result = item.get("result").or_else(|| item.get("output"));
        if let Some(result_text) = result.and_then(external_tool_result_text)
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result_text)
            && let Some(mut payload) =
                flowscript_workspace_result_payload(display_tool_name, &parsed, None)
        {
            if let Some(id) = item.get("id").and_then(serde_json::Value::as_str)
                && let Some(object) = payload.as_object_mut()
            {
                object.insert(
                    "tool_call_id".to_string(),
                    serde_json::Value::String(id.to_string()),
                );
            }
            return Some(flowpilot_stream_tag("flowscript_workspace", &payload));
        }
    }

    if !is_flowscript_authoring_tool_name(display_tool_name) {
        return None;
    }

    let arguments = external_agent_tool_arguments(item)?;
    let flowscript = extract_flowscript_source_from_tool_arguments(&arguments)?;
    if flowscript.trim().is_empty() {
        return None;
    }

    Some(flowpilot_stream_tag(
        "flowscript_workspace",
        &serde_json::json!({
            "source": flowscript,
            "status": "submitted",
        }),
    ))
}

fn is_flowscript_authoring_tool_name(tool_name: &str) -> bool {
    flow_like::flow::copilot::stream::is_flowscript_authoring_tool(tool_name)
}

fn external_agent_tool_arguments(item: &serde_json::Value) -> Option<serde_json::Value> {
    for key in ["arguments", "args", "input", "params", "parameters"] {
        if let Some(value) = item.get(key)
            && let Some(arguments) = normalize_external_tool_arguments(value)
        {
            return Some(arguments);
        }
    }

    for pointer in [
        "/call/arguments",
        "/function/arguments",
        "/request/arguments",
        "/tool_call/arguments",
    ] {
        if let Some(value) = item.pointer(pointer)
            && let Some(arguments) = normalize_external_tool_arguments(value)
        {
            return Some(arguments);
        }
    }

    None
}

fn normalize_external_tool_arguments(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<serde_json::Value>(trimmed)
                .ok()
                .or_else(|| Some(serde_json::Value::String(text.clone())))
        }
        _ => Some(value.clone()),
    }
}

fn extract_flowscript_source_from_tool_arguments(value: &serde_json::Value) -> Option<String> {
    extract_flowscript_source_from_tool_arguments_inner(value, 0)
}

fn extract_flowscript_source_from_tool_arguments_inner(
    value: &serde_json::Value,
    depth: u8,
) -> Option<String> {
    if depth > 4 {
        return None;
    }

    match value {
        serde_json::Value::Object(map) => {
            for key in ["flowscript", "script", "source", "content"] {
                if let Some(source) = map.get(key).and_then(serde_json::Value::as_str)
                    && !source.trim().is_empty()
                {
                    return Some(source.to_string());
                }
            }

            for key in ["arguments", "args", "input", "params", "parameters"] {
                if let Some(nested) = map.get(key)
                    && let Some(source) =
                        extract_flowscript_source_from_tool_arguments_inner(nested, depth + 1)
                {
                    return Some(source);
                }
            }

            None
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                return extract_flowscript_source_from_tool_arguments_inner(&parsed, depth + 1);
            }
            Some(text.clone())
        }
        _ => None,
    }
}

fn external_agent_stream_delta(
    backend: FlowPilotAgentBackendKind,
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Option<String> {
    match backend {
        FlowPilotAgentBackendKind::Codex => codex_agent_message_delta(value, state),
        FlowPilotAgentBackendKind::ClaudeCode => claude_agent_message_delta(value, state),
        FlowPilotAgentBackendKind::GithubCopilot => None,
    }
}

fn codex_agent_message_delta(
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if matches!(
        event_type,
        "agent_message_delta" | "assistant_message_delta"
    ) {
        let item_id = value
            .get("item_id")
            .or_else(|| value.get("itemId"))
            .or_else(|| value.get("id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("codex-agent-message");
        let delta = value
            .get("delta")
            .or_else(|| value.pointer("/item/delta"))
            .or_else(|| value.get("text"))
            .and_then(serde_json::Value::as_str)?;
        return Some(state.decorate_agent_delta(item_id, delta));
    }

    if !matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) {
        return None;
    }

    let item = value.get("item")?;
    let item_type = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !matches!(item_type, "agent_message" | "assistant_message") {
        return None;
    }

    let item_id = item
        .get("id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("codex-agent-message");

    if let Some(delta) = item.get("delta").and_then(serde_json::Value::as_str) {
        if state.agent_message_text_by_id.len() >= EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES
            && !state.agent_message_text_by_id.contains_key(item_id)
        {
            state.agent_message_text_by_id.clear();
        }
        {
            let previous = state
                .agent_message_text_by_id
                .entry(item_id.to_string())
                .or_default();
            append_bounded_text(previous, delta, EXTERNAL_AGENT_TEXT_MAX_BYTES);
        }
        return Some(state.decorate_agent_delta(item_id, delta));
    }

    let full_text = item.get("text").and_then(serde_json::Value::as_str)?;
    if state.agent_message_text_by_id.len() >= EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES
        && !state.agent_message_text_by_id.contains_key(item_id)
    {
        state.agent_message_text_by_id.clear();
    }
    let delta = {
        let previous = state
            .agent_message_text_by_id
            .entry(item_id.to_string())
            .or_default();
        let delta = if full_text.starts_with(previous.as_str()) {
            full_text[previous.len()..].to_string()
        } else if previous.is_empty() {
            full_text.to_string()
        } else {
            String::new()
        };
        previous.clear();
        append_bounded_text(previous, full_text, EXTERNAL_AGENT_TEXT_MAX_BYTES);
        delta
    };

    if event_type == "item.completed" {
        state.agent_message_text_by_id.remove(item_id);
    }

    Some(state.decorate_agent_delta(item_id, &delta))
}

fn claude_agent_message_delta(
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Option<String> {
    // Claude Code (with --include-partial-messages) streams assistant tokens as
    // `stream_event` frames wrapping a content_block_delta / text_delta. The full
    // `assistant` message and the final `result` event are handled elsewhere, so
    // only the incremental text deltas are emitted here to avoid duplication.
    if value.get("type").and_then(serde_json::Value::as_str) != Some("stream_event") {
        return None;
    }
    let event = value.get("event")?;
    if event.get("type").and_then(serde_json::Value::as_str) != Some("content_block_delta") {
        return None;
    }
    let delta = event.get("delta")?;
    if delta.get("type").and_then(serde_json::Value::as_str) != Some("text_delta") {
        return None;
    }
    let text = delta.get("text").and_then(serde_json::Value::as_str)?;
    if text.is_empty() {
        return None;
    }

    Some(state.decorate_agent_delta("claude-agent-message", text))
}

/// Strip the `mcp__<server>__` prefix Claude uses for MCP tools so the frontend
/// tool labeller recognizes the bare FlowPilot tool name (e.g. `edit_flowscript`).
fn claude_display_tool_name(name: &str) -> &str {
    name.strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .map(|(_, tool)| tool)
        .unwrap_or(name)
}

/// Decode Claude's streamed `input_json_delta` fragments into live FlowScript workspace prefixes.
/// The later complete `assistant/tool_use` block remains authoritative and emits `submitted`.
fn claude_partial_tool_input_events(
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Vec<String> {
    let Some(event) = value.get("event") else {
        return Vec::new();
    };
    let event_type = event
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let index = event.get("index").and_then(serde_json::Value::as_u64);

    match event_type {
        "content_block_start" => {
            let Some(block) = event.get("content_block") else {
                return Vec::new();
            };
            if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_use") {
                return Vec::new();
            }
            let Some(id) = block.get("id").and_then(serde_json::Value::as_str) else {
                return Vec::new();
            };
            let name = block
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool");
            if let Some(index) = index {
                if state.claude_tool_call_ids_by_index.len()
                    >= EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES
                    && !state.claude_tool_call_ids_by_index.contains_key(&index)
                {
                    state.claude_tool_call_ids_by_index.clear();
                }
                state
                    .claude_tool_call_ids_by_index
                    .insert(index, id.to_string());
            }
            state.claude_flowscript_preview.observe_name(id, name);
            Vec::new()
        }
        "content_block_delta" => {
            let Some(index) = index else {
                return Vec::new();
            };
            let Some(id) = state.claude_tool_call_ids_by_index.get(&index).cloned() else {
                return Vec::new();
            };
            let Some(delta) = event.get("delta") else {
                return Vec::new();
            };
            if delta.get("type").and_then(serde_json::Value::as_str) != Some("input_json_delta") {
                return Vec::new();
            }
            delta
                .get("partial_json")
                .and_then(serde_json::Value::as_str)
                .and_then(|partial| {
                    state
                        .claude_flowscript_preview
                        .observe_arguments_delta(&id, partial)
                })
                .into_iter()
                .collect()
        }
        "content_block_stop" => {
            if let Some(index) = index {
                state.claude_tool_call_ids_by_index.remove(&index);
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Surface Claude Code's tool activity as FlowPilot `tool_start`/`tool_end`
/// frames. Claude reports tool calls as `tool_use` blocks inside an `assistant`
/// message and their outcomes as `tool_result` blocks in the following `user`
/// message — unlike Codex's `item.*` events, so it needs its own extractor.
fn claude_agent_tool_events(
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Vec<String> {
    let event_type = value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if event_type == "stream_event" {
        return claude_partial_tool_input_events(value, state);
    }
    let Some(blocks) = value
        .pointer("/message/content")
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };

    let mut events = Vec::new();
    match event_type {
        "assistant" => {
            for block in blocks {
                if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_use") {
                    continue;
                }
                let Some(id) = block.get("id").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let name = claude_display_tool_name(
                    block
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("tool"),
                )
                .to_string();
                if state.claude_tool_names.len() >= EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES
                    && !state.claude_tool_names.contains_key(id)
                {
                    state.claude_tool_names.clear();
                }
                state.claude_tool_names.insert(id.to_string(), name.clone());
                if let Some(arguments) = block.get("input").or_else(|| block.get("arguments"))
                    && let Some(frame) = state
                        .claude_flowscript_preview
                        .complete(id, &name, arguments)
                {
                    events.push(frame);
                }
                let arguments_preview =
                    block
                        .get("input")
                        .or_else(|| block.get("arguments"))
                        .map(|arguments| {
                            external_debug_value_preview(
                                arguments,
                                flow_like::flow::copilot::stream::TOOL_ARGUMENT_PREVIEW_CHARS,
                            )
                        });
                events.push(flowpilot_stream_tag(
                    "tool_start",
                    &serde_json::json!({
                        "tool_call_id": id,
                        "tool": name,
                        "status": "running",
                        "summary": format!("flowpilot/{name}"),
                        "arguments_preview": arguments_preview,
                    }),
                ));
            }
        }
        "user" => {
            for block in blocks {
                if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_result") {
                    continue;
                }
                let Some(id) = block.get("tool_use_id").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let is_error = block
                    .get("is_error")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let name = state
                    .claude_tool_names
                    .remove(id)
                    .unwrap_or_else(|| "tool".to_string());
                if is_flowscript_draft_operation_tool(&name)
                    && let Some(result_text) =
                        block.get("content").and_then(external_tool_result_text)
                    && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result_text)
                    && let Some(mut payload) =
                        flowscript_workspace_result_payload(&name, &parsed, None)
                {
                    if let Some(object) = payload.as_object_mut() {
                        object.insert(
                            "tool_call_id".to_string(),
                            serde_json::Value::String(id.to_string()),
                        );
                    }
                    events.push(flowpilot_stream_tag("flowscript_workspace", &payload));
                }
                let fallback_status = if is_error { "error" } else { "completed" };
                let error = is_error.then_some("Claude tool result reported an error");
                let (status, terminal_status, result_summary, result_preview) =
                    external_result_details(block.get("content"), Some(fallback_status), error);
                events.push(flowpilot_stream_tag(
                    "tool_end",
                    &serde_json::json!({
                        "tool_call_id": id,
                        "tool": name,
                        "status": status,
                        "terminal_status": terminal_status,
                        "result_summary": result_summary,
                        "result_preview": result_preview,
                        "error": error,
                    }),
                ));
            }
        }
        _ => {}
    }
    events
}

fn send_external_progress_event(
    channel: &Channel<String>,
    event_id: &str,
    message: &str,
    parent_request_id: Option<&str>,
) {
    let message = flow_like::flow::copilot::stream::safe_text_preview(message, 1_200);
    send_correlated_stream_json_event(
        channel,
        "tool_progress",
        &serde_json::json!({
            "tool_call_id": event_id,
            "message": message,
        }),
        parent_request_id,
    );
}

fn flowpilot_stream_tag(tag: &str, value: &serde_json::Value) -> String {
    format!(
        "<{tag}>{}</{tag}>",
        serde_json::to_string(value).unwrap_or_default()
    )
}

fn external_agent_progress_label(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)?;

    if matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) && let Some(item) = value.get("item")
    {
        let item_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let status = item
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();

        match item_type {
            "mcp_tool_call" => {
                let tool = item
                    .get("tool")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tool");
                if event_type == "item.completed" || status == "completed" {
                    return Some(format!("Completed {tool}"));
                }
                return Some(format!("Using {tool}..."));
            }
            "command_execution" => {
                let command = item
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("command");
                if event_type == "item.completed" || status == "completed" {
                    return Some(format!("Command completed: {command}"));
                }
                return Some(format!("Running command: {command}"));
            }
            "file_change" => {
                if event_type == "item.completed" || status == "completed" {
                    return Some("File changes completed".to_string());
                }
                return Some("Applying file changes...".to_string());
            }
            "web_search" => {
                let query = item
                    .get("query")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("web");
                return Some(format!("Searching {query}..."));
            }
            "error" => {
                if let Some(message) = item.get("message").and_then(serde_json::Value::as_str) {
                    return Some(format!("Error: {message}"));
                }
            }
            _ => {}
        }
    }

    if event_type.contains("tool") {
        let name = value
            .get("name")
            .or_else(|| value.pointer("/tool/name"))
            .or_else(|| value.pointer("/item/name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("tool");
        return Some(format!("Using {name}..."));
    }

    if event_type.contains("error") {
        return Some(format!("{}...", event_type.replace('_', " ")));
    }

    None
}

fn external_agent_error_text(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if event_type == "turn.failed" {
        return value
            .pointer("/error/message")
            .or_else(|| value.get("message"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
    }

    if event_type == "error" {
        return value
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
    }

    // Note: mcp_tool_call items with an error are deliberately NOT fatal — a single failed tool
    // call is surfaced as a tool_end error frame (external_agent_process_event) and the agent can
    // recover and continue the turn.
    if matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) && let Some(item) = value.get("item")
    {
        let item_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if item_type == "error" {
            return item
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
    }

    None
}

fn external_agent_result_text(
    _backend: FlowPilotAgentBackendKind,
    value: &serde_json::Value,
) -> Option<String> {
    // Codex item.completed agent messages match first; every backend then falls back to the
    // generic result/final extraction (`{"type":"result","message":…}` frames). mcp_tool_call
    // outputs never reach the fallback — their event type carries neither "result" nor "final".
    if let Some(text) = codex_agent_result_text(value) {
        return Some(text);
    }

    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if !event_type.contains("result") && !event_type.contains("final") {
        return None;
    }

    let text = extract_external_agent_text(value);
    (!text.trim().is_empty()).then_some(text)
}

fn codex_agent_result_text(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if event_type == "item.completed" {
        let item = value.get("item")?;
        let item_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if matches!(item_type, "agent_message" | "assistant_message") {
            return item
                .get("text")
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_string);
        }
    }

    None
}

fn extract_external_agent_text(value: &serde_json::Value) -> String {
    fn collect(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    if matches!(
                        key.as_str(),
                        "text" | "content" | "delta" | "message" | "result" | "summary"
                    ) {
                        match child {
                            serde_json::Value::String(text) => {
                                if !looks_like_machine_status(text) {
                                    out.push(text.clone());
                                }
                            }
                            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                                collect(child, out);
                            }
                            _ => {}
                        }
                    } else if matches!(
                        child,
                        serde_json::Value::Array(_) | serde_json::Value::Object(_)
                    ) {
                        collect(child, out);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect(item, out);
                }
            }
            _ => {}
        }
    }

    let mut parts = Vec::new();
    collect(value, &mut parts);
    parts.join("")
}

fn looks_like_machine_status(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty()
        || trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed == "started"
        || trimmed == "completed"
}

fn send_commands_event(channel: &Channel<String>, commands: &[BoardCommand]) {
    if commands.is_empty() {
        return;
    }

    let cmd_event = format!(
        "<commands>{}</commands>",
        serde_json::to_string(commands).unwrap_or_default()
    );
    let _ = channel.send(cmd_event);
}

fn workflow_edit_continuation_prompt(
    original_user_prompt: &str,
    latest_workspace: Option<&str>,
    attempt: u8,
    validation_failure: Option<&(String, Vec<String>)>,
) -> String {
    let failure_note = match validation_failure {
        Some((tool, errors)) if !errors.is_empty() => format!(
            "\nYour last `{tool}` call FAILED validation and nothing was applied. Fix exactly these errors and resubmit the corrected full document/batch:\n- {}\n",
            errors.join("\n- ")
        ),
        Some((tool, _)) => format!(
            "\nYour last `{tool}` call FAILED validation and nothing was applied. Fix the reported problems and resubmit.\n"
        ),
        None => String::new(),
    };
    let workspace_note = if latest_workspace.is_some() {
        "You already submitted a FlowScript draft, but it did not create a review claim. Use the compiler diagnostics and repair that same retained source revision."
    } else {
        "You did not finish the requested change yet."
    };

    format!(
        r#"INTERNAL FLOWPILOT CONTINUATION #{attempt}
{workspace_note}
{failure_note}
Do not ask the user to confirm. Do not say "Create draft", "go ahead", "tell me if", or similar.
Use placeholders for unknown credentials/data. Your next assistant turn must call tools: workflow behavior must proceed through write_flowscript/patch_flowscript, check_flowscript, and end with commit_flowscript creating the exact review claim; UI work must end with emit_ui rendering. The turn is not complete until that succeeds or blocking compiler diagnostics identify an actual unavailable capability.

Original user request:
{original_user_prompt}"#
    )
}

// =============================================================================
// GitHub Copilot SDK Direct Integration
// =============================================================================

use copilot_sdk::{AttachmentType, Client, LogLevel, MessageOptions, UserMessageAttachment};
use once_cell::sync::Lazy;
use tokio::sync::Mutex;

/// Global Copilot client instance. The mutex protects only slot replacement; callers clone the
/// `Arc` before awaiting RPCs so a wedged CLI cannot block status/stop/start on this mutex.
static COPILOT_CLIENT: Lazy<Mutex<Option<Arc<Client>>>> = Lazy::new(|| Mutex::new(None));

static COPILOT_START_GATE: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(1));

/// Options the main Copilot client was started with, reused to start nested pool clients.
/// Cleared on backend stop so a checkout that begins entirely after a stop fails fast instead of
/// spawning a pooled CLI process from stale configuration.
static COPILOT_START_OPTIONS: Lazy<Mutex<Option<FlowPilotBackendStartOptions>>> =
    Lazy::new(|| Mutex::new(None));

/// Retained draft stores are board-scoped and their base-fingerprint integrity requires that two
/// nested runs mutating the same board never interleave. Runs targeting DIFFERENT boards are
/// independent and may proceed concurrently, so nested runs are serialized per gate key instead
/// of process-wide. All four agent backends (Bits/rig, GitHub Copilot SDK, Codex CLI, Claude Code
/// CLI) acquire the same per-board gate for nested runs.
static NESTED_COPILOT_RUN_GATES: Lazy<StdMutex<HashMap<String, Arc<Semaphore>>>> =
    Lazy::new(|| StdMutex::new(HashMap::new()));

/// Gate key for a nested run: the targeted board id (board runs), the widget/page target board id
/// carried by the frontend tool context (widget runs), or a shared global key when no target
/// exists at all.
fn nested_copilot_run_gate_key(
    board: Option<&Board>,
    tool_context: Option<&FrontendToolContext>,
) -> String {
    board
        .map(|board| board.id.clone())
        .filter(|id| !id.trim().is_empty())
        .or_else(|| {
            tool_context
                .and_then(|context| context.board_id.clone())
                .filter(|id| !id.trim().is_empty())
        })
        .map(|id| format!("board:{id}"))
        .unwrap_or_else(|| "global".to_string())
}

/// Resolve the serialization gate for one gate key. Gates whose only owner is the map itself
/// (no permit holder, no queued waiter — both hold an `Arc` clone) are pruned on every lookup so
/// the map stays bounded by the number of concurrently active/queued nested runs.
fn nested_copilot_run_gate(key: &str) -> Arc<Semaphore> {
    let mut gates = NESTED_COPILOT_RUN_GATES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    gates.retain(|_, gate| Arc::strong_count(gate) > 1);
    gates
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(Semaphore::new(1)))
        .clone()
}

async fn acquire_nested_copilot_run_permit(
    gate: Arc<Semaphore>,
    cancellation: CancellationToken,
) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    tokio::select! {
        permit = gate.acquire_owned() => {
            permit.map_err(|_| "The nested Copilot run gate was closed".to_string())
        }
        _ = cancellation.cancelled() => {
            Err("FlowPilot Copilot run was cancelled before it started".to_string())
        }
    }
}

/// Dedicated CLI processes for NESTED FlowPilot runs (flowpilot_board / flowpilot_widget
/// sub-agents spawned while a parent Copilot session is mid-turn). The copilot CLI serializes
/// requests within one process: a `session.create` sent while the parent session has a pending
/// tool call is never answered, deadlocking the sub-run until the tool bridge times out. Separate
/// processes isolate nested sessions completely. This is a PER-PROCESS constraint, so nested runs
/// use a small pool: clients start lazily with the same options as the main client (up to
/// `NESTED_COPILOT_POOL_SIZE`) and idle processes are reused. A checked-out client is exclusively
/// owned by one run, preserving one-session-at-a-time per process by construction.
const NESTED_COPILOT_POOL_SIZE: usize = 3;

struct NestedCopilotPool {
    slots: Arc<Semaphore>,
    idle: StdMutex<Vec<Arc<Client>>>,
    /// Every live pooled process, idle or checked out. Quarantine removes an entry so the owning
    /// lease drops the process instead of returning it to `idle`; backend stop drains everything.
    registered: StdMutex<Vec<Arc<Client>>>,
    /// Bumped by every `drain` (under the `registered` lock). A client whose startup began before
    /// a drain must not register into the drained pool, or backend stop would leave a live CLI
    /// process behind that the stop path never saw. Lock order is always `registered` → `idle`.
    drain_epoch: AtomicU64,
}

impl NestedCopilotPool {
    fn new(size: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(size)),
            idle: StdMutex::new(Vec::new()),
            registered: StdMutex::new(Vec::new()),
            drain_epoch: AtomicU64::new(0),
        }
    }

    fn take_idle(&self) -> Option<Arc<Client>> {
        self.idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop()
    }

    fn epoch(&self) -> u64 {
        self.drain_epoch.load(AtomicOrdering::SeqCst)
    }

    /// Register a freshly started client, but only if no drain happened since `observed_epoch`
    /// was captured. Returns whether the client joined the pool; a rejected client must be
    /// stopped by the caller because no pool teardown path will ever see it.
    fn register_started(&self, client: Arc<Client>, observed_epoch: u64) -> bool {
        let mut registered = self
            .registered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.drain_epoch.load(AtomicOrdering::SeqCst) != observed_epoch {
            return false;
        }
        registered.push(client);
        true
    }

    fn deregister(&self, client: &Arc<Client>) -> bool {
        let mut registered = self
            .registered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = registered.len();
        registered.retain(|entry| !Arc::ptr_eq(entry, client));
        let removed = registered.len() != before;
        self.idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|entry| !Arc::ptr_eq(entry, client));
        removed
    }

    fn is_registered(&self, client: &Arc<Client>) -> bool {
        self.registered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|entry| Arc::ptr_eq(entry, client))
    }

    fn return_to_idle(&self, client: Arc<Client>) {
        // Hold the `registered` lock across the membership check AND the idle push: a concurrent
        // drain would otherwise interleave between them and leave a stopped client in `idle`.
        let registered = self
            .registered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if registered.iter().any(|entry| Arc::ptr_eq(entry, &client)) {
            self.idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(client);
        }
    }

    fn drain(&self) -> Vec<Arc<Client>> {
        let mut registered = self
            .registered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.drain_epoch.fetch_add(1, AtomicOrdering::SeqCst);
        self.idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        registered.drain(..).collect()
    }
}

static NESTED_COPILOT_POOL: Lazy<NestedCopilotPool> =
    Lazy::new(|| NestedCopilotPool::new(NESTED_COPILOT_POOL_SIZE));

/// Exclusive checkout of one pooled nested CLI process. Dropping the lease returns the client to
/// the idle pool unless it was quarantined/deregistered first; the pool slot frees either way so
/// a replacement process can start lazily on the next checkout.
struct NestedCopilotClientLease {
    pool: &'static NestedCopilotPool,
    client: Arc<Client>,
    _slot: tokio::sync::OwnedSemaphorePermit,
}

impl NestedCopilotClientLease {
    fn client(&self) -> Arc<Client> {
        self.client.clone()
    }

    /// Remove the leased client from the pool without stopping it, for drop paths that cannot
    /// await session cleanup: leaking one process is safe, re-pooling a client whose previous
    /// session may still be pending is not.
    fn deregister(&self) {
        self.pool.deregister(&self.client);
    }
}

impl Drop for NestedCopilotClientLease {
    fn drop(&mut self) {
        self.pool.return_to_idle(self.client.clone());
    }
}

async fn checkout_nested_copilot_client_from<F, Fut>(
    pool: &'static NestedCopilotPool,
    cancellation: CancellationToken,
    start_client: F,
) -> Result<NestedCopilotClientLease, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Arc<Client>, String>>,
{
    let slot = tokio::select! {
        permit = pool.slots.clone().acquire_owned() => {
            permit.map_err(|_| "The nested Copilot process pool was closed".to_string())?
        }
        _ = cancellation.cancelled() => {
            return Err("FlowPilot Copilot run was cancelled before it started".to_string());
        }
    };
    if let Some(client) = pool.take_idle() {
        return Ok(NestedCopilotClientLease {
            pool,
            client,
            _slot: slot,
        });
    }
    let observed_epoch = pool.epoch();
    let client = start_client().await?;
    if !pool.register_started(client.clone(), observed_epoch) {
        let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, client.force_stop()).await;
        return Err(
            "The nested Copilot process pool was drained while a replacement client was starting"
                .to_string(),
        );
    }
    Ok(NestedCopilotClientLease {
        pool,
        client,
        _slot: slot,
    })
}

async fn nested_copilot_start_options() -> Result<FlowPilotBackendStartOptions, String> {
    COPILOT_START_OPTIONS
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Copilot client not started".to_string())
}

async fn checkout_nested_copilot_client(
    cancellation: CancellationToken,
) -> Result<NestedCopilotClientLease, String> {
    checkout_nested_copilot_client_from(&NESTED_COPILOT_POOL, cancellation, || async {
        let options = nested_copilot_start_options().await?;
        flowpilot_debug_log!("[copilot_sdk_chat] starting dedicated CLI process for a nested run");
        Ok(Arc::new(build_and_start_copilot_client(&options).await?))
    })
    .await
}

async fn quarantine_nested_copilot_client(client: &Arc<Client>) {
    if NESTED_COPILOT_POOL.deregister(client) {
        let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, client.force_stop()).await;
    }
}

async fn build_and_start_copilot_client(
    options: &FlowPilotBackendStartOptions,
) -> Result<Client, String> {
    let mut builder = Client::builder()
        .use_stdio(options.use_stdio)
        .log_level(LogLevel::Error);

    if let Some(url) = options.cli_url.clone() {
        builder = builder.cli_url(url);
    } else if let Some(cli_path) = find_copilot_cli_path() {
        builder = builder.cli_path(cli_path);
    }

    // In production builds the app inherits a minimal PATH that often does
    // not include directories where `node` lives. The copilot CLI is a
    // Node.js script (#!/usr/bin/env node), so the spawned process needs
    // node on its PATH. Augment PATH with common Node/tool directories.
    builder = builder.env("PATH", augmented_path());

    let client = builder
        .build()
        .map_err(|e| format!("Failed to build Copilot client: {}", e))?;
    match tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, client.start()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, client.force_stop()).await;
            return Err(format!("Failed to start Copilot client: {error}"));
        }
        Err(_) => {
            let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, client.force_stop()).await;
            return Err(format!(
                "Copilot client startup exceeded {} seconds",
                SDK_CONTROL_RPC_TIMEOUT.as_secs()
            ));
        }
    }
    Ok(client)
}
static EXTERNAL_AGENT_BACKENDS: Lazy<Mutex<std::collections::HashSet<FlowPilotAgentBackendKind>>> =
    Lazy::new(|| Mutex::new(std::collections::HashSet::new()));

/// One model-specific reasoning-effort choice discovered from the backend runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningEffortOption {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Model info returned by any local FlowPilot agent backend. Reasoning capabilities
/// stay model-specific because accounts, policies, and installed runtimes can expose
/// different choices even within the same provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopilotModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub supported_reasoning_efforts: Vec<ReasoningEffortOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

impl CopilotModelInfo {
    fn basic(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            supported_reasoning_efforts: Vec::new(),
            default_reasoning_effort: None,
            is_default: false,
        }
    }
}

/// Auth status returned from GitHub Copilot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotAuthStatus {
    pub authenticated: bool,
    pub login: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowPilotBackendStatus {
    pub backend: FlowPilotAgentBackendKind,
    pub label: String,
    pub available: bool,
    pub running: bool,
    pub executable: Option<String>,
    pub message: Option<String>,
    pub transport: FlowPilotAgentTransportKind,
    pub capabilities: FlowPilotAgentCapabilitySet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlowPilotAgentTransportKind {
    DirectSdkTools,
    Mcp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowPilotAgentCapabilitySet {
    pub prompt_source: String,
    pub tool_protocol: FlowPilotAgentTransportKind,
    pub tool_names: Vec<String>,
}

impl FlowPilotAgentCapabilitySet {
    fn shared_for(scope: CopilotScope, has_board: bool, has_graph_context: bool) -> Self {
        let mut tool_names: Vec<&'static str> = Vec::new();

        if matches!(scope, CopilotScope::Board | CopilotScope::Both) {
            tool_names.extend(["catalog_search", "emit_commands", "get_declarations"]);
            if has_board {
                tool_names.extend([
                    "get_current_flowscript",
                    "write_flowscript",
                    "patch_flowscript",
                    "check_flowscript",
                    "commit_flowscript",
                ]);
            }
            if has_graph_context {
                tool_names.extend([
                    "get_node_details",
                    "get_unconfigured_nodes",
                    "list_board_nodes",
                ]);
            }
        }

        if matches!(scope, CopilotScope::Frontend | CopilotScope::Both) {
            tool_names.extend(["emit_ui", "get_component_schema"]);
        }

        tool_names.extend([
            "database_tool",
            "storage_tool",
            "execute_event",
            "execute_node",
            "query_execution_logs",
            "ask_user",
        ]);

        tool_names.sort_unstable();
        tool_names.dedup();

        Self {
            prompt_source: "flow_like::copilot::prompts".to_string(),
            tool_protocol: FlowPilotAgentTransportKind::DirectSdkTools,
            tool_names: tool_names.into_iter().map(str::to_string).collect(),
        }
    }

    fn add_global_orchestrator_tools(&mut self) {
        self.tool_names.extend([
            "internet_search".to_string(),
            "open_url".to_string(),
            "archive_lookup".to_string(),
        ]);
        self.tool_names.sort_unstable();
        self.tool_names.dedup();
    }

    fn for_surface(
        scope: CopilotScope,
        has_board: bool,
        has_graph_context: bool,
        global_orchestrator: bool,
    ) -> Self {
        let mut capabilities = Self::shared_for(scope, has_board, has_graph_context);
        if global_orchestrator {
            capabilities.add_global_orchestrator_tools();
        }
        capabilities
    }

    fn for_status(transport: FlowPilotAgentTransportKind) -> Self {
        let mut capabilities = Self::shared_for(CopilotScope::Both, true, true);
        capabilities.tool_protocol = transport;
        capabilities
    }
}

struct FlowPilotAgentSurface {
    graph_context: Option<Arc<GraphContext>>,
    board_arc: Option<Arc<Board>>,
    /// Registry-backed current board. Retained FlowScript source operations lock this at execution
    /// time so the commit fingerprint and host queue boundary cannot rely on a captured clone.
    live_board: Option<Arc<flow_like_types::sync::Mutex<Board>>>,
    /// Original host-owned workflow request used to derive a deterministic scope-coverage
    /// contract before the model can author its own capability plan. Bound to the immutable
    /// end-user request (not the per-run composed specialist instruction), so nested repair runs
    /// spawned from the same user turn share draft/acceptance identity.
    request_acceptance_prompt: Option<String>,
    catalog_provider: Option<Arc<dyn CatalogProvider>>,
    side_effect_commands: Arc<StdMutex<SideEffectCommandQueue>>,
    /// Last FlowScript submission that reconciled successfully. Nested external agents return it
    /// to the global bridge so detached boards can apply the validated document.
    queued_flowscript: Arc<StdMutex<Option<String>>>,
    /// UI trees captured from successful `emit_ui` calls, for transports that cannot parse tool
    /// results (external-agent MCP bridge).
    emitted_surfaces: Arc<StdMutex<Vec<super::copilot_sdk_tools::EmittedSurface>>>,
    /// Host-authorized source recovery for this exact immutable request. The prompt explains it,
    /// while the loop state separately enforces the draft id/revision without trusting the model
    /// to reconstruct those coordinates from prose.
    flowscript_recovery: Option<flow_like::flow::copilot::FlowScriptDraftRecovery>,
    system_content: String,
    workflow_edit_request: bool,
    capabilities: FlowPilotAgentCapabilitySet,
}

/// Resolve the current in-process board once per tool surface. The incoming Tauri `Board` value is
/// a request snapshot; the registry handle continues to reflect edits made while an agent is
/// planning. Detached boards are intentionally left on the captured snapshot fallback.
fn live_board_handle(
    app_handle: &AppHandle,
    board: Option<&Board>,
) -> Option<Arc<flow_like_types::sync::Mutex<Board>>> {
    let board_id = board
        .map(|board| board.id.trim())
        .filter(|board_id| !board_id.is_empty())?;
    app_handle
        .try_state::<TauriFlowLikeState>()
        .and_then(|state| state.0.get_board(board_id, None).ok())
}

/// Recover an exact pending source review before starting another model run. The temporary
/// acceptance binding is derived solely from the host's immutable raw request and is released
/// immediately after the read. The pending delivery itself is not consumed or re-claimed, so a
/// disconnected client can retry until it receives the same Apply/Dismiss token.
async fn pending_flowscript_redelivery_for_request(
    app_handle: &AppHandle,
    captured_board: &Board,
    request_identity_prompt: &str,
) -> Option<FlowScriptPendingDelivery> {
    let current_board = match live_board_handle(app_handle, Some(captured_board)) {
        Some(live_board) => live_board.lock().await.clone(),
        None => captured_board.clone(),
    };
    let store = retained_flow_ir_draft_store_for_board(&current_board).ok()?;
    let binding =
        store.bind_request_acceptance_contract(&current_board.id, request_identity_prompt);
    let delivery = store.pending_flowscript_delivery_for_binding(&current_board, &binding);
    let _ = store.release_request_acceptance_contract(&binding);
    delivery
}

fn pending_flowscript_redelivery_response(
    scope: CopilotScope,
    delivery: FlowScriptPendingDelivery,
) -> UnifiedCopilotResponse {
    let (message, workspace_status) = if delivery.stale_board {
        (
            "Recovered an exact pending FlowScript review, but the board has advanced. The old commands cannot be applied; dismiss this stale review before generating from the current board.",
            "stale",
        )
    } else {
        (
            "Redelivered the already-queued exact FlowScript revision for review. No model generation or duplicate command queueing occurred.",
            "queued",
        )
    };
    UnifiedCopilotResponse {
        message: message.to_string(),
        commands: delivery.commands,
        suggestions: Vec::new(),
        components: Vec::new(),
        canvas_settings: None,
        root_component_id: None,
        flowscript_workspace: Some(flowscript_workspace_envelope(
            &delivery.source,
            workspace_status,
        )),
        flow_ir_commit: Some(delivery.token),
        active_scope: scope,
    }
}

fn append_typed_ir_recovery_context(
    system_content: &mut String,
    recovery: &flow_like::flow::copilot::FlowIrDraftRecovery,
) {
    match recovery.status {
        flow_like::flow::copilot::FlowIrDraftRecoveryStatus::ExactMatch => {
            if let Ok(recovery_json) = serde_json::to_string_pretty(recovery) {
                system_content.push_str(&format!(
                    "\n\n## EXACT TYPED-DRAFT RECOVERY\nThe host matched this retained typed draft to the normalized immutable raw user request. Auto-resume this exact draft at its retained revision. Do not call begin_flow_ir_draft, switch mutation representations, or reconstruct it from the unchanged board/FlowScript.\n```json\n{recovery_json}\n```"
                ));
            }
        }
        flow_like::flow::copilot::FlowIrDraftRecoveryStatus::RequestMismatch => {
            // Deliberately omit the conflicting draft id/revision from model context. The host
            // owns that recovery decision; resumable coordinates would invite an unrelated
            // request to update or commit the old acceptance contract.
            let conflict = serde_json::json!({
                "status": "request_mismatch",
                "auto_resume": false,
                "conflicting_draft_present": recovery.conflicting_draft.is_some(),
                "next_actions": &recovery.next_actions,
                "message": &recovery.message,
            });
            if let Ok(conflict_json) = serde_json::to_string_pretty(&conflict) {
                system_content.push_str(&format!(
                    "\n\n## TYPED-DRAFT REQUEST MISMATCH\nThe host found retained typed work for this board, but it belongs to another immutable raw user request. It is non-authoritative for this run: do not update, validate, or commit it. Use only the host-owned recover/abandon choices below, or begin a separate draft id for the current request.\n```json\n{conflict_json}\n```"
                ));
            }
        }
        flow_like::flow::copilot::FlowIrDraftRecoveryStatus::None => {}
    }
}

/// Recover retained model-authored source across SDK/external requests using the same immutable
/// raw-request identity and stale-board rules as the built-in Rig path. The core renderer is the
/// authority for what may enter model context: exact matches include source, stale exact matches
/// include it only as a reference for a fresh draft, and request mismatches hide it completely.
#[cfg(test)]
fn append_flowscript_recovery_context(
    system_content: &mut String,
    board: &Board,
    raw_user_prompt: &str,
) {
    let Ok(store) = retained_flow_ir_draft_store_for_board(board) else {
        return;
    };
    let recovery = store.editable_flowscript_draft_recovery(board, raw_user_prompt);
    append_flowscript_recovery_payload(system_content, &recovery);
}

fn append_flowscript_recovery_payload(
    system_content: &mut String,
    recovery: &flow_like::flow::copilot::FlowScriptDraftRecovery,
) {
    let Some(instruction) =
        flow_like::flow::copilot::flowscript_recovery_system_instruction(recovery)
    else {
        return;
    };
    system_content.push_str("\n\n");
    system_content.push_str(&instruction);
}

#[allow(clippy::too_many_arguments)]
fn build_flowpilot_agent_surface(
    scope: CopilotScope,
    board: Option<&Board>,
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: &[String],
    current_surface: Option<&Vec<SurfaceComponent>>,
    history: &[UnifiedChatMessage],
    original_user_prompt: &str,
    // Immutable end-user request that owns retained drafts and the acceptance contract. For a
    // nested specialist run this differs from `original_user_prompt` (the per-run composed
    // instruction), so every identity bind below must use this value.
    request_identity_prompt: &str,
    host_context_guidance: Option<&str>,
    global: Option<&str>,
    // Read-only sub-run (flowpilot_board explain): keep the board copilot out of workflow-edit mode
    // so it streams and returns its answer instead of being coerced to emit an edit and, failing
    // that, returning a canned "could not produce board commands" message.
    read_only: bool,
) -> FlowPilotAgentSurface {
    use flow_like::flow::copilot::prepare_context;

    let graph_context = match scope {
        CopilotScope::Board | CopilotScope::Both => board
            .and_then(|board| prepare_context(board, selected_node_ids).ok())
            .map(Arc::new),
        CopilotScope::Frontend | CopilotScope::DataStudio => None,
    };

    let board_arc: Option<Arc<Board>> = match scope {
        CopilotScope::Board | CopilotScope::Both => board.map(|b| Arc::new(b.clone())),
        CopilotScope::Frontend | CopilotScope::DataStudio => None,
    };

    let desktop_catalog_provider = match scope {
        CopilotScope::Board | CopilotScope::Both => {
            Some(Arc::new(DesktopCatalogProvider::new(catalog_nodes)))
        }
        CopilotScope::Frontend | CopilotScope::DataStudio => None,
    };

    let catalog_provider: Option<Arc<dyn CatalogProvider>> = match scope {
        CopilotScope::Board | CopilotScope::Both => desktop_catalog_provider
            .as_ref()
            .map(|provider| provider.clone() as Arc<dyn CatalogProvider>),
        CopilotScope::Frontend | CopilotScope::DataStudio => None,
    };

    let board_flowscript = board_arc.as_ref().map(|board| {
        flow_like::flow::ast::board_to_flowscript(
            board,
            &flow_like::flow::ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
    });

    let catalog_node_count = desktop_catalog_provider
        .as_ref()
        .map(|provider| provider.len())
        .unwrap_or_else(|| flow_like_catalog::get_catalog().len());

    let workflow_edit_request = !read_only
        && matches!(scope, CopilotScope::Board | CopilotScope::Both)
        && board_arc.is_some()
        && is_workflow_edit_request(original_user_prompt);

    let mut system_content = if global.is_some() {
        global_assistant_system_prompt()
    } else {
        match scope {
            CopilotScope::Board => match board_flowscript.as_deref() {
                Some(flowscript) => {
                    flow_like::copilot::prompts::board_sdk_flowscript_system_prompt(
                        flowscript,
                        catalog_node_count,
                    )
                }
                None => flow_like::copilot::prompts::board_sdk_system_prompt(),
            },
            CopilotScope::Frontend => flow_like::copilot::prompts::frontend_sdk_system_prompt(),
            CopilotScope::DataStudio => flow_like::copilot::prompts::data_studio_system_prompt(""),
            CopilotScope::Both => match board_flowscript.as_deref() {
                // flowscript_board_context embeds the shared guidance blocks itself; the lean
                // header avoids duplicating them (~3.5k tokens).
                Some(flowscript) => {
                    let mut prompt = flow_like::copilot::prompts::general_system_prompt_lean();
                    prompt.push_str("\n\n");
                    prompt.push_str(&flow_like::copilot::prompts::flowscript_board_context(
                        flowscript,
                        catalog_node_count,
                    ));
                    prompt
                }
                None => flow_like::copilot::prompts::general_system_prompt(),
            },
        }
    };

    if let Some(context) = global
        && !context.is_empty()
    {
        system_content.push_str("\n\n");
        system_content.push_str(context);
    }

    if let Some(guidance) = host_context_guidance.filter(|guidance| !guidance.trim().is_empty()) {
        system_content.push_str("\n\n");
        system_content.push_str(guidance);
    }

    let flowscript_recovery = workflow_edit_request
        .then(|| board_arc.as_deref())
        .flatten()
        .and_then(|board| {
            retained_flow_ir_draft_store_for_board(board)
                .ok()
                .map(|store| {
                    store.editable_flowscript_draft_recovery(board, request_identity_prompt)
                })
        });
    if let Some(recovery) = flowscript_recovery.as_ref() {
        append_flowscript_recovery_payload(&mut system_content, recovery);
    }

    if matches!(scope, CopilotScope::Frontend | CopilotScope::Both)
        && let Some(components) = current_surface
        && !components.is_empty()
    {
        let components_json =
            serde_json::to_string_pretty(components).unwrap_or_else(|_| "[]".to_string());
        system_content.push_str(&format!(
            "\n\n## CURRENT UI COMPONENTS\nThe user has the following existing UI. You can modify or extend it:\n```json\n{}\n```",
            components_json
        ));
    }

    let mut context_parts = vec![];
    for msg in history {
        let role = match msg.role {
            flow_like::flow::copilot::ChatRole::User => "User",
            flow_like::flow::copilot::ChatRole::Assistant => "Assistant",
        };
        context_parts.push(format!("{}: {}", role, msg.content));
    }
    if !context_parts.is_empty() {
        system_content.push_str(&format!(
            "\n\nConversation history:\n{}",
            context_parts.join("\n\n")
        ));
    }

    let capabilities = FlowPilotAgentCapabilitySet::for_surface(
        scope,
        board_arc.is_some(),
        graph_context.is_some(),
        global.is_some(),
    );

    FlowPilotAgentSurface {
        graph_context,
        board_arc,
        live_board: None,
        request_acceptance_prompt: workflow_edit_request
            .then(|| request_identity_prompt.to_string()),
        catalog_provider,
        side_effect_commands: Arc::new(StdMutex::new(SideEffectCommandQueue::default())),
        queued_flowscript: Arc::new(StdMutex::new(None)),
        emitted_surfaces: Arc::new(StdMutex::new(Vec::new())),
        flowscript_recovery,
        system_content,
        workflow_edit_request,
        capabilities,
    }
}

#[derive(Debug, Clone)]
struct FlowPilotBackendStartOptions {
    use_stdio: bool,
    cli_url: Option<String>,
    app_handle: Option<AppHandle>,
}

#[async_trait]
trait FlowPilotAgentBackend: Send + Sync {
    fn kind(&self) -> FlowPilotAgentBackendKind;
    async fn start(&self, options: FlowPilotBackendStartOptions) -> Result<(), String>;
    async fn stop(&self) -> Result<(), String>;
    async fn is_running(&self) -> Result<bool, String>;
    async fn list_models(&self) -> Result<Vec<CopilotModelInfo>, String>;
    async fn get_auth_status(
        &self,
        app_handle: Option<&AppHandle>,
    ) -> Result<CopilotAuthStatus, String>;
    async fn status(&self, app_handle: Option<&AppHandle>) -> FlowPilotBackendStatus {
        let kind = self.kind();
        let executable = find_cli_resolution(kind, app_handle)
            .map(|resolution| resolution.executable.display().to_string());
        let available = executable.is_some();
        let running = self.is_running().await.unwrap_or(false);

        FlowPilotBackendStatus {
            backend: kind,
            label: kind.label().to_string(),
            available,
            running,
            executable,
            message: None,
            transport: FlowPilotAgentTransportKind::DirectSdkTools,
            capabilities: FlowPilotAgentCapabilitySet::for_status(
                FlowPilotAgentTransportKind::DirectSdkTools,
            ),
        }
    }
}

struct GithubCopilotBackend;

struct ExternalCodeAgentBackend {
    kind: FlowPilotAgentBackendKind,
}

fn agent_backend(kind: FlowPilotAgentBackendKind) -> Box<dyn FlowPilotAgentBackend> {
    match kind {
        FlowPilotAgentBackendKind::GithubCopilot => Box::new(GithubCopilotBackend),
        FlowPilotAgentBackendKind::Codex | FlowPilotAgentBackendKind::ClaudeCode => {
            Box::new(ExternalCodeAgentBackend { kind })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliResolutionSource {
    EnvOverride,
    BundledResource,
    CodexStandalone,
    CodexNpmPackage,
    Path,
    IdeExtensionFallback,
}

#[derive(Debug, Clone)]
struct CliResolution {
    executable: PathBuf,
    path_dirs: Vec<PathBuf>,
    source: CliResolutionSource,
}

impl CliResolution {
    fn new(executable: PathBuf, source: CliResolutionSource) -> Self {
        Self {
            executable,
            path_dirs: Vec::new(),
            source,
        }
    }

    fn with_path_dirs(
        executable: PathBuf,
        source: CliResolutionSource,
        path_dirs: Vec<PathBuf>,
    ) -> Self {
        Self {
            executable,
            path_dirs,
            source,
        }
    }
}

fn codex_binary_name() -> &'static str {
    if cfg!(windows) { "codex.exe" } else { "codex" }
}

fn claude_binary_name() -> &'static str {
    if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    }
}

fn codex_target() -> Option<(&'static str, &'static str)> {
    let target = if cfg!(target_os = "linux") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-unknown-linux-musl"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-unknown-linux-musl"
        } else {
            return None;
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-apple-darwin"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            return None;
        }
    } else if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-pc-windows-msvc"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-pc-windows-msvc"
        } else {
            return None;
        }
    } else {
        return None;
    };

    let package = match target {
        "x86_64-unknown-linux-musl" => "@openai/codex-linux-x64",
        "aarch64-unknown-linux-musl" => "@openai/codex-linux-arm64",
        "x86_64-apple-darwin" => "@openai/codex-darwin-x64",
        "aarch64-apple-darwin" => "@openai/codex-darwin-arm64",
        "x86_64-pc-windows-msvc" => "@openai/codex-win32-x64",
        "aarch64-pc-windows-msvc" => "@openai/codex-win32-arm64",
        _ => return None,
    };

    Some((target, package))
}

/// Collect extra bin directories that are typically absent from a bundled-app
/// PATH (Homebrew, nvm, volta, fnm, mise, pnpm, bun, npm-global, …).
fn extra_bin_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let Some(home) = dirs_next::home_dir() else {
        return vec![];
    };

    let mut dirs: Vec<PathBuf> = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".volta/bin"),
        home.join(".bun/bin"),
        home.join(".local/share/pnpm"),
        home.join(".local/bin"),
        home.join(".asdf/shims"),
    ];

    // Homebrew's documented Linux installation uses the linuxbrew home rather
    // than either macOS prefix above. A per-user Linuxbrew install is also common.
    #[cfg(target_os = "linux")]
    dirs.extend([
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        home.join(".linuxbrew/bin"),
    ]);

    // GUI apps on Windows may not see newly added user PATH entries until the
    // next login. Probe the standard npm and WinGet links directly, plus the
    // GitHub Copilot CLI location documented by GitHub's SDK guide.
    #[cfg(windows)]
    {
        if let Some(data_dir) = dirs_next::data_dir() {
            dirs.push(data_dir.join("npm"));
        }
        if let Some(local_data_dir) = dirs_next::data_local_dir() {
            dirs.push(local_data_dir.join("Microsoft/WinGet/Links"));
            dirs.push(local_data_dir.join("pnpm"));
        }
        for variable in ["ProgramFiles", "ProgramW6432"] {
            if let Ok(program_files) = std::env::var(variable) {
                let trimmed = program_files.trim();
                if !trimmed.is_empty() {
                    dirs.push(PathBuf::from(trimmed).join("GitHub"));
                }
            }
        }
    }

    // nvm – scan all installed node versions
    let nvm_dir = std::env::var("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".nvm"));
    if let Ok(entries) = std::fs::read_dir(nvm_dir.join("versions/node")) {
        for entry in entries.flatten() {
            dirs.push(entry.path().join("bin"));
        }
    }

    // fnm
    if let Ok(entries) = std::fs::read_dir(home.join(".local/share/fnm/node-versions")) {
        for entry in entries.flatten() {
            dirs.push(entry.path().join("installation/bin"));
        }
    }

    // mise / rtx node shims
    dirs.push(home.join(".local/share/mise/shims"));

    // npm global prefix variants
    dirs.push(home.join(".npm-global/bin"));
    dirs.push(home.join(".npm-packages/bin"));
    dirs.push(home.join(".npm/bin"));

    // Claude Code local install (e.g. after `claude migrate-installer`)
    let claude_home = std::env::var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".claude"));
    dirs.push(claude_home.join("local"));

    dirs.extend(codex_standalone_visible_dirs(&home));

    dirs.sort();
    dirs.dedup();
    dirs
}

fn codex_standalone_visible_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(install_dir) = std::env::var("CODEX_INSTALL_DIR") {
        let trimmed = install_dir.trim();
        if !trimmed.is_empty() {
            dirs.push(PathBuf::from(trimmed));
        }
    }

    let codex_home = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".codex"));
    dirs.push(codex_home.join("packages/standalone/current/bin"));
    dirs.push(codex_home.join("packages/standalone/current"));

    #[cfg(not(windows))]
    dirs.push(home.join(".local/bin"));

    #[cfg(windows)]
    if let Some(local_app_data) = dirs_next::data_local_dir() {
        dirs.push(local_app_data.join("Programs/OpenAI/Codex/bin"));
    }

    dirs
}

fn codex_ide_extension_candidate_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for root in [
        home.join(".vscode/extensions"),
        home.join(".vscode-insiders/extensions"),
        home.join(".vscode-oss/extensions"),
        home.join(".cursor/extensions"),
        home.join(".windsurf/extensions"),
    ] {
        collect_codex_cli_dirs(&root, 5, &mut dirs);
    }

    dirs.sort_by(|a, b| b.cmp(a));
    dirs.dedup();
    dirs
}

fn collect_codex_cli_dirs(root: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
    if depth == 0 || !root.is_dir() {
        return;
    }

    let codex_executable = root.join(codex_binary_name());
    if is_executable_file(&codex_executable) {
        out.push(root.to_path_buf());
        let helper_path = root.join("codex-path");
        if helper_path.is_dir() {
            out.push(helper_path);
        }
        return;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_codex_cli_dirs(&path, depth - 1, out);
        }
    }
}

/// Numeric version key from an `anthropic.claude-code-<version>-<platform>`
/// extension directory name (e.g. `[2, 1, 204]`), for newest-first ordering.
fn claude_extension_version_key(path: &Path) -> Vec<u64> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("anthropic.claude-code-"))
        .map(|rest| {
            rest.split('-')
                .next()
                .unwrap_or_default()
                .split('.')
                .map(|part| part.parse::<u64>().unwrap_or(0))
                .collect()
        })
        .unwrap_or_default()
}

/// Locate `claude` executables bundled inside IDE extensions, newest first.
///
/// The Claude Code editor extension ships the native CLI at
/// `<editor>/extensions/anthropic.claude-code-<version>-<platform>/resources/native-binary/claude`.
/// Desktop apps launched from Finder/Dock don't inherit `claude` on PATH, so this
/// lets FlowPilot reuse the CLI the user already installed via that extension.
fn claude_ide_extension_binaries(home: &Path) -> Vec<PathBuf> {
    let mut binaries = Vec::new();
    for root in [
        home.join(".vscode/extensions"),
        home.join(".vscode-insiders/extensions"),
        home.join(".vscode-oss/extensions"),
        home.join(".cursor/extensions"),
        home.join(".windsurf/extensions"),
    ] {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut extension_dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("anthropic.claude-code-"))
            })
            .collect();
        // Sort by parsed version (numeric, newest first) — a lexical sort would
        // rank "2.1.9" above "2.1.204".
        extension_dirs
            .sort_by(|a, b| claude_extension_version_key(b).cmp(&claude_extension_version_key(a)));
        for dir in extension_dirs {
            let candidate = dir
                .join("resources")
                .join("native-binary")
                .join(claude_binary_name());
            if is_executable_file(&candidate) {
                binaries.push(candidate);
            }
        }
    }
    binaries
}

/// Resolve the Copilot CLI path, searching beyond the (possibly limited) bundled-app PATH.
///
/// On macOS/Linux, apps launched from Finder/Dock inherit a minimal PATH that
/// excludes npm-global, nvm, volta, mise, and Homebrew directories. This
/// function probes those common locations so that prod builds can find the CLI.
fn find_cli_path(kind: FlowPilotAgentBackendKind) -> Option<std::path::PathBuf> {
    find_cli_resolution(kind, None).map(|resolution| resolution.executable)
}

fn find_cli_resolution(
    kind: FlowPilotAgentBackendKind,
    app_handle: Option<&AppHandle>,
) -> Option<CliResolution> {
    if let Ok(p) = std::env::var(kind.env_path_var()) {
        let trimmed = p.trim();
        if trimmed.is_empty() {
            return None;
        }

        let path = PathBuf::from(trimmed);
        if is_executable_file(&path) {
            return Some(CliResolution::new(path, CliResolutionSource::EnvOverride));
        }

        if path.is_dir() {
            let candidate = path.join(kind.cli_name());
            if is_executable_file(&candidate) {
                return Some(CliResolution::new(
                    candidate,
                    CliResolutionSource::EnvOverride,
                ));
            }
        }

        if path.components().count() == 1
            && let Some(found) = find_executable_in_path(trimmed, &augmented_path())
        {
            return Some(CliResolution::new(found, CliResolutionSource::EnvOverride));
        }
    }

    if kind == FlowPilotAgentBackendKind::Codex {
        if let Some(resolution) = find_bundled_codex_cli(app_handle) {
            return Some(resolution);
        }
        if let Some(resolution) = find_codex_standalone_cli() {
            return Some(resolution);
        }
        if let Some(resolution) = find_codex_npm_package_cli(app_handle) {
            return Some(resolution);
        }
    }

    if let Some(found) = find_executable_in_path(kind.cli_name(), &augmented_path()) {
        return Some(CliResolution::new(found, CliResolutionSource::Path));
    }

    if kind == FlowPilotAgentBackendKind::Codex
        && let Some(home) = dirs_next::home_dir()
    {
        for dir in codex_ide_extension_candidate_dirs(&home) {
            if let Some(candidate) = find_codex_executable_in_dir(&dir) {
                let mut path_dirs = Vec::new();
                let helper_path = dir.join("codex-path");
                if helper_path.is_dir() {
                    path_dirs.push(helper_path);
                }
                return Some(CliResolution::with_path_dirs(
                    candidate,
                    CliResolutionSource::IdeExtensionFallback,
                    path_dirs,
                ));
            }
        }
    }

    if kind == FlowPilotAgentBackendKind::ClaudeCode
        && let Some(home) = dirs_next::home_dir()
        && let Some(candidate) = claude_ide_extension_binaries(&home).into_iter().next()
    {
        return Some(CliResolution::new(
            candidate,
            CliResolutionSource::IdeExtensionFallback,
        ));
    }

    None
}

fn find_bundled_codex_cli(app_handle: Option<&AppHandle>) -> Option<CliResolution> {
    let mut roots = Vec::new();
    if let Some(app_handle) = app_handle
        && let Ok(resource_dir) = app_handle.path().resource_dir()
    {
        roots.extend([
            resource_dir.clone(),
            resource_dir.join("codex"),
            resource_dir.join("binaries"),
            resource_dir.join("bin"),
            resource_dir.join("node_modules"),
        ]);
    }

    for root in roots {
        if let Some(resolution) =
            find_codex_packaged_cli_under_root(&root, CliResolutionSource::BundledResource)
        {
            return Some(resolution);
        }
        if let Some(candidate) = find_codex_executable_in_dir(&root) {
            return Some(CliResolution::new(
                candidate,
                CliResolutionSource::BundledResource,
            ));
        }
    }

    None
}

fn find_codex_standalone_cli() -> Option<CliResolution> {
    let home = dirs_next::home_dir()?;
    for dir in codex_standalone_visible_dirs(&home) {
        if let Some(candidate) = find_codex_executable_in_dir(&dir) {
            return Some(CliResolution::new(
                candidate,
                CliResolutionSource::CodexStandalone,
            ));
        }
    }
    None
}

fn find_codex_npm_package_cli(app_handle: Option<&AppHandle>) -> Option<CliResolution> {
    for root in codex_npm_search_roots(app_handle) {
        if let Some(resolution) =
            find_codex_packaged_cli_under_root(&root, CliResolutionSource::CodexNpmPackage)
        {
            return Some(resolution);
        }
    }
    None
}

fn codex_npm_search_roots(app_handle: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(app_handle) = app_handle
        && let Ok(resource_dir) = app_handle.path().resource_dir()
    {
        roots.extend([
            resource_dir.join("node_modules"),
            resource_dir.join("codex/node_modules"),
        ]);
    }

    if let Some(home) = dirs_next::home_dir() {
        roots.extend([
            home.join(".npm-global/lib/node_modules"),
            home.join(".npm-packages/lib/node_modules"),
            home.join(".bun/install/global/node_modules"),
        ]);

        let nvm_dir = std::env::var("NVM_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".nvm"));
        if let Ok(entries) = std::fs::read_dir(nvm_dir.join("versions/node")) {
            for entry in entries.flatten() {
                roots.push(entry.path().join("lib/node_modules"));
            }
        }

        if let Ok(entries) = std::fs::read_dir(home.join(".local/share/fnm/node-versions")) {
            for entry in entries.flatten() {
                roots.push(entry.path().join("installation/lib/node_modules"));
            }
        }
    }

    #[cfg(windows)]
    if let Some(data_dir) = dirs_next::data_dir() {
        roots.push(data_dir.join("npm/node_modules"));
    }

    roots.sort();
    roots.dedup();
    roots
}

fn find_codex_packaged_cli_under_root(
    root: &Path,
    source: CliResolutionSource,
) -> Option<CliResolution> {
    let (target, platform_package) = codex_target()?;
    let package_leaf = platform_package
        .rsplit('/')
        .next()
        .unwrap_or(platform_package);
    let package_roots = [
        root.join(platform_package),
        root.join("@openai").join(package_leaf),
        root.join("@openai/codex/node_modules")
            .join(platform_package),
        root.join("@openai/codex/node_modules/@openai")
            .join(package_leaf),
        root.join("@openai/codex"),
        root.to_path_buf(),
    ];

    for package_root in package_roots {
        if let Some(resolution) = resolve_codex_native_package(&package_root, target, source) {
            return Some(resolution);
        }
    }

    None
}

fn resolve_codex_native_package(
    package_root: &Path,
    target: &str,
    source: CliResolutionSource,
) -> Option<CliResolution> {
    let target_root = package_root.join("vendor").join(target);
    let package_binary = target_root.join("bin").join(codex_binary_name());
    if is_executable_file(&package_binary) && target_root.join("codex-package.json").is_file() {
        let path_dirs = [target_root.join("codex-path")]
            .into_iter()
            .filter(|dir| dir.is_dir())
            .collect();
        return Some(CliResolution::with_path_dirs(
            package_binary,
            source,
            path_dirs,
        ));
    }

    let legacy_binary = target_root.join("codex").join(codex_binary_name());
    if is_executable_file(&legacy_binary) {
        let path_dirs = [target_root.join("path")]
            .into_iter()
            .filter(|dir| dir.is_dir())
            .collect();
        return Some(CliResolution::with_path_dirs(
            legacy_binary,
            source,
            path_dirs,
        ));
    }

    None
}

fn find_codex_executable_in_dir(dir: &Path) -> Option<PathBuf> {
    for file_name in codex_executable_file_names() {
        let candidate = dir.join(file_name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn codex_executable_file_names() -> Vec<String> {
    let mut names = vec![codex_binary_name().to_string()];
    if let Some((target, _)) = codex_target() {
        names.push(if cfg!(windows) {
            format!("codex-{target}.exe")
        } else {
            format!("codex-{target}")
        });
    }
    names
}

fn find_copilot_cli_path() -> Option<std::path::PathBuf> {
    find_cli_path(FlowPilotAgentBackendKind::GithubCopilot)
}

/// Build an augmented PATH that prepends the extra bin directories to the
/// current PATH so that the spawned copilot CLI process (a Node.js script)
/// can locate `node` and other tools even in production builds.
fn augmented_path() -> String {
    augmented_path_with_dirs(&[])
}

fn augmented_path_with_dirs(prefix_dirs: &[PathBuf]) -> String {
    let mut entries: Vec<PathBuf> = prefix_dirs
        .iter()
        .cloned()
        .chain(extra_bin_dirs())
        .filter(|d| d.exists())
        .collect();

    let current = std::env::var("PATH").unwrap_or_default();
    entries.extend(std::env::split_paths(&current));

    std::env::join_paths(entries)
        .unwrap_or_else(|_| current.into())
        .to_string_lossy()
        .into_owned()
}

fn find_executable_in_path(name: &str, path_value: &str) -> Option<std::path::PathBuf> {
    for dir in std::env::split_paths(path_value) {
        for file_name in executable_file_names(name) {
            let candidate = dir.join(file_name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

fn executable_file_names(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let path = std::path::Path::new(name);
        if path.extension().is_some() {
            return vec![name.to_string()];
        }

        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let mut names = vec![name.to_string()];
        for ext in pathext.split(';').filter(|ext| !ext.trim().is_empty()) {
            names.push(format!("{name}{}", ext.to_ascii_lowercase()));
            names.push(format!("{name}{}", ext.to_ascii_uppercase()));
        }
        names.sort();
        names.dedup();
        names
    }

    #[cfg(not(windows))]
    {
        vec![name.to_string()]
    }
}

fn is_executable_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

async fn probe_external_agent_cli(
    kind: FlowPilotAgentBackendKind,
    executable: &std::path::Path,
    path_dirs: &[PathBuf],
) -> Result<String, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(executable)
            .arg("--version")
            .env("PATH", augmented_path_with_dirs(path_dirs))
            .output(),
    )
    .await
    .map_err(|_| format!("{} CLI probe timed out after 5s", kind.label()))?
    .map_err(|e| {
        format!(
            "Failed to run {} CLI at {}: {e}",
            kind.label(),
            executable.display()
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(format!(
            "{} --version exited with status {}{}",
            kind.label(),
            output.status,
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        Ok(format!("{} CLI responded to --version", kind.label()))
    } else {
        Ok(stdout)
    }
}

/// Discover the Codex models available for the current authentication mode by
/// driving the installed `codex` CLI's `app-server` JSON-RPC protocol.
///
/// Codex model availability is auth-, policy-, and version-dependent, so the set
/// is read from Codex itself rather than hard-coded. Any failure (missing
/// `app-server` subcommand, unauthenticated session, timeout) is returned as an
/// error and the caller falls back to Codex's configured default.
async fn list_codex_models_via_app_server(
    cli: &CliResolution,
) -> Result<Vec<CopilotModelInfo>, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let mut child = tokio::process::Command::new(&cli.executable)
        .arg("app-server")
        .env("PATH", augmented_path_with_dirs(&cli.path_dirs))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start codex app-server: {e}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "codex app-server did not expose stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "codex app-server did not expose stdout".to_string())?;

    // Newline-delimited JSON-RPC 2.0 (without the "jsonrpc" field), matching the
    // codex app-server framing: initialize -> initialized -> model/list.
    const MODEL_LIST_ID: i64 = 1;
    let messages = [
        serde_json::json!({
            "method": "initialize",
            "id": 0,
            "params": {
                "clientInfo": {
                    "name": "flow-like",
                    "title": "Flow-Like FlowPilot",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }
        }),
        serde_json::json!({ "method": "initialized", "params": {} }),
        serde_json::json!({
            "method": "model/list",
            "id": MODEL_LIST_ID,
            "params": { "limit": 100, "includeHidden": false }
        }),
    ];
    let mut payload = String::new();
    for message in &messages {
        payload.push_str(&message.to_string());
        payload.push('\n');
    }
    stdin
        .write_all(payload.as_bytes())
        .await
        .map_err(|e| format!("Failed to send codex app-server request: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush codex app-server request: {e}"))?;

    let read_models = async {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| format!("Failed to read codex app-server output: {e}"))?
        {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                continue;
            };
            if value.get("id").and_then(serde_json::Value::as_i64) != Some(MODEL_LIST_ID) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(format!("codex app-server model/list failed: {error}"));
            }
            let entries = value
                .get("result")
                .and_then(|result| result.get("data"))
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            return Ok(parse_codex_model_catalog(&entries));
        }
        Err("codex app-server closed before returning models".to_string())
    };

    let outcome = tokio::time::timeout(Duration::from_secs(8), read_models).await;
    let _ = child.start_kill();
    match outcome {
        Ok(result) => result,
        Err(_) => Err("codex app-server model listing timed out".to_string()),
    }
}

fn reasoning_effort_display_name(id: &str) -> String {
    match id.trim().to_ascii_lowercase().as_str() {
        "xhigh" => "Extra high".to_string(),
        value => value
            .split(['-', '_'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn parse_reasoning_effort_options(value: Option<&serde_json::Value>) -> Vec<ReasoningEffortOption> {
    let Some(entries) = value.and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };

    let mut options = Vec::new();
    for entry in entries {
        let (id, name, description) = match entry {
            serde_json::Value::String(id) => {
                let id = id.trim();
                if id.is_empty() {
                    continue;
                }
                (id.to_string(), reasoning_effort_display_name(id), None)
            }
            serde_json::Value::Object(object) => {
                let Some(id) = object
                    .get("reasoningEffort")
                    .or_else(|| object.get("id"))
                    .or_else(|| object.get("value"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                let name = object
                    .get("name")
                    .or_else(|| object.get("displayName"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| reasoning_effort_display_name(id));
                let description = object
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|description| !description.is_empty())
                    .map(str::to_string);
                (id.to_string(), name, description)
            }
            _ => continue,
        };

        if options
            .iter()
            .any(|existing: &ReasoningEffortOption| existing.id == id)
        {
            continue;
        }
        options.push(ReasoningEffortOption {
            id,
            name,
            description,
        });
    }
    options
}

fn optional_non_empty_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Convert a `model/list` `data` array into FlowPilot model options, skipping
/// hidden entries and preserving Codex's ordering (recommended model first).
fn parse_codex_model_catalog(entries: &[serde_json::Value]) -> Vec<CopilotModelInfo> {
    let mut models = Vec::new();
    for entry in entries {
        if entry
            .get("hidden")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(id) = entry
            .get("id")
            .or_else(|| entry.get("model"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let name = entry
            .get("displayName")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id.as_str())
            .to_string();
        models.push(CopilotModelInfo {
            id,
            name,
            supported_reasoning_efforts: parse_reasoning_effort_options(
                entry.get("supportedReasoningEfforts"),
            ),
            default_reasoning_effort: optional_non_empty_string(
                entry.get("defaultReasoningEffort"),
            ),
            is_default: entry
                .get("isDefault")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        });
    }
    models
}

fn codex_models_with_configured_default(
    discovered: Vec<CopilotModelInfo>,
) -> Vec<CopilotModelInfo> {
    let mut configured_default = CopilotModelInfo::basic("default", "Codex configured default");
    configured_default.is_default = true;
    if let Some(runtime_default) = discovered.iter().find(|model| model.is_default) {
        configured_default.supported_reasoning_efforts =
            runtime_default.supported_reasoning_efforts.clone();
        configured_default.default_reasoning_effort =
            runtime_default.default_reasoning_effort.clone();
    }

    let mut models = vec![configured_default];
    for model in discovered {
        if model.id != "default" && !models.iter().any(|existing| existing.id == model.id) {
            models.push(model);
        }
    }
    models
}

/// Discover the Claude Code models available for the current authentication by
/// driving the CLI's stream-json control protocol — the same `initialize`
/// handshake the Agent SDK's `supportedModels()` reads. Claude Code has no
/// model-listing subcommand, so this is the only auth-aware, version-current
/// source; nothing about the model set is hard-coded. Any failure (CLI missing,
/// unauthenticated, protocol change, timeout) surfaces as an error and the
/// caller falls back to the CLI's configured default.
async fn list_claude_models_via_control_protocol(
    cli: &CliResolution,
) -> Result<Vec<CopilotModelInfo>, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    // A neutral cwd keeps the handshake from triggering workspace-trust or
    // CLAUDE.md discovery for the user's project; the model set only depends on
    // account auth (read from the keychain), not the working directory.
    let mut child = tokio::process::Command::new(&cli.executable)
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
        ])
        .current_dir(std::env::temp_dir())
        .env("PATH", augmented_path_with_dirs(&cli.path_dirs))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start claude control session: {e}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "claude control session did not expose stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "claude control session did not expose stdout".to_string())?;

    // Newline-delimited control protocol: send one `initialize` control_request;
    // the success control_response carries the model catalog at
    // `response.response.models`.
    let request = serde_json::json!({
        "request_id": "flowpilot-model-list",
        "type": "control_request",
        "request": { "subtype": "initialize" }
    });
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(|e| format!("Failed to send claude initialize request: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed to flush claude initialize request: {e}"))?;

    let read_models = async {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| format!("Failed to read claude control output: {e}"))?
        {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                continue;
            };
            if value.get("type").and_then(serde_json::Value::as_str) != Some("control_response") {
                continue;
            }
            let response = value.get("response");
            if response
                .and_then(|response| response.get("subtype"))
                .and_then(serde_json::Value::as_str)
                == Some("error")
            {
                let message = response
                    .and_then(|response| response.get("error"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown error");
                return Err(format!("claude initialize failed: {message}"));
            }
            let entries = response
                .and_then(|response| response.get("response"))
                .and_then(|inner| inner.get("models"))
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            if entries.is_empty() {
                continue;
            }
            return Ok(parse_claude_model_catalog(&entries));
        }
        Err("claude control session closed before returning models".to_string())
    };

    let outcome = tokio::time::timeout(Duration::from_secs(12), read_models).await;
    let _ = child.start_kill();
    match outcome {
        Ok(result) => result,
        Err(_) => Err("claude model listing timed out".to_string()),
    }
}

/// Convert the Claude Code `initialize` handshake's `models` array into FlowPilot
/// model options. `value` is the id passed to `--model`; `displayName` is shown
/// to the user (falling back to the value), preserving the CLI's ordering.
fn parse_claude_model_catalog(entries: &[serde_json::Value]) -> Vec<CopilotModelInfo> {
    let mut models = Vec::new();
    for entry in entries {
        let Some(id) = entry
            .get("value")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        if models
            .iter()
            .any(|existing: &CopilotModelInfo| existing.id == id)
        {
            continue;
        }
        let name = entry
            .get("displayName")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id.as_str())
            .to_string();
        let supports_effort = entry
            .get("supportsEffort")
            .and_then(serde_json::Value::as_bool);
        let supported_reasoning_efforts = if supports_effort == Some(false) {
            Vec::new()
        } else {
            parse_reasoning_effort_options(entry.get("supportedEffortLevels"))
        };
        models.push(CopilotModelInfo {
            is_default: entry
                .get("isDefault")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(id == "default"),
            id,
            name,
            supported_reasoning_efforts,
            default_reasoning_effort: optional_non_empty_string(
                entry
                    .get("defaultReasoningEffort")
                    .or_else(|| entry.get("defaultEffortLevel")),
            ),
        });
    }
    models
}

#[async_trait]
impl FlowPilotAgentBackend for GithubCopilotBackend {
    fn kind(&self) -> FlowPilotAgentBackendKind {
        FlowPilotAgentBackendKind::GithubCopilot
    }

    async fn start(&self, options: FlowPilotBackendStartOptions) -> Result<(), String> {
        let _start_permit =
            tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, COPILOT_START_GATE.acquire())
                .await
                .map_err(|_| {
                    "Timed out waiting for Copilot startup already in progress".to_string()
                })?
                .map_err(|_| "Copilot startup gate was closed".to_string())?;
        if COPILOT_CLIENT.lock().await.is_some() {
            return Ok(());
        }
        let client = Arc::new(build_and_start_copilot_client(&options).await?);

        {
            let mut opts = COPILOT_START_OPTIONS.lock().await;
            *opts = Some(options);
        }
        COPILOT_CLIENT.lock().await.replace(client);

        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        let client = {
            let mut guard = COPILOT_CLIENT.lock().await;
            guard.take()
        };
        // Clear before draining: a checkout that reads options after this point fails fast, and
        // one that read them earlier is rejected by the pool's drain epoch when it registers.
        COPILOT_START_OPTIONS.lock().await.take();
        let nested_clients = NESTED_COPILOT_POOL.drain();

        let mut errors: Vec<String> = Vec::new();
        if let Some(client) = client {
            let stop_errors = match tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, client.stop())
                .await
            {
                Ok(errors) => errors,
                Err(_) => {
                    let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, client.force_stop()).await;
                    errors.push("main: graceful stop timed out; client force-stopped".to_string());
                    Vec::new()
                }
            };
            if !stop_errors.is_empty() {
                errors.push(format!("{:?}", stop_errors));
            }
        }
        for client in nested_clients {
            let stop_errors = match tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, client.stop())
                .await
            {
                Ok(errors) => errors,
                Err(_) => {
                    let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, client.force_stop()).await;
                    errors
                        .push("nested: graceful stop timed out; client force-stopped".to_string());
                    Vec::new()
                }
            };
            if !stop_errors.is_empty() {
                errors.push(format!("nested: {:?}", stop_errors));
            }
        }
        if !errors.is_empty() {
            return Err(format!(
                "Failed to stop Copilot client: {}",
                errors.join("; ")
            ));
        }

        Ok(())
    }

    async fn is_running(&self) -> Result<bool, String> {
        let guard = COPILOT_CLIENT.lock().await;
        Ok(guard.is_some())
    }

    async fn list_models(&self) -> Result<Vec<CopilotModelInfo>, String> {
        let client = COPILOT_CLIENT
            .lock()
            .await
            .clone()
            .ok_or("Copilot client not started")?;
        let models = tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, client.list_models())
            .await
            .map_err(|_| "Timed out listing Copilot models".to_string())?
            .map_err(|e| format!("Failed to list models: {}", e))?;

        Ok(models
            .iter()
            .map(|m| CopilotModelInfo {
                id: m.id.clone(),
                name: m.name.clone(),
                supported_reasoning_efforts: if m.capabilities.supports.reasoning_effort {
                    m.supported_reasoning_efforts
                        .clone()
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|id| {
                            let id = id.trim();
                            (!id.is_empty()).then(|| ReasoningEffortOption {
                                id: id.to_string(),
                                name: reasoning_effort_display_name(id),
                                description: None,
                            })
                        })
                        .collect()
                } else {
                    Vec::new()
                },
                default_reasoning_effort: m
                    .capabilities
                    .supports
                    .reasoning_effort
                    .then(|| m.default_reasoning_effort.clone())
                    .flatten(),
                is_default: false,
            })
            .collect())
    }

    async fn get_auth_status(
        &self,
        _app_handle: Option<&AppHandle>,
    ) -> Result<CopilotAuthStatus, String> {
        let client = COPILOT_CLIENT
            .lock()
            .await
            .clone()
            .ok_or("Copilot client not started")?;
        let status = tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, client.get_auth_status())
            .await
            .map_err(|_| "Timed out checking Copilot authentication".to_string())?
            .map_err(|e| format!("Failed to get auth status: {}", e))?;

        Ok(CopilotAuthStatus {
            authenticated: status.is_authenticated,
            login: status.login.clone(),
            message: None,
        })
    }
}

#[async_trait]
impl FlowPilotAgentBackend for ExternalCodeAgentBackend {
    fn kind(&self) -> FlowPilotAgentBackendKind {
        self.kind
    }

    async fn start(&self, options: FlowPilotBackendStartOptions) -> Result<(), String> {
        let cli = find_cli_resolution(self.kind, options.app_handle.as_ref()).ok_or_else(|| {
            format!(
                "{} CLI was not found. Install it or set {} to its executable path.",
                self.kind.label(),
                self.kind.env_path_var()
            )
        })?;
        let version = probe_external_agent_cli(self.kind, &cli.executable, &cli.path_dirs).await?;
        let mut guard = EXTERNAL_AGENT_BACKENDS.lock().await;
        guard.insert(self.kind);
        tracing::info!(
            backend = self.kind.label(),
            executable = %cli.executable.display(),
            source = ?cli.source,
            version = %version,
            "enabled external FlowPilot backend"
        );
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        let mut guard = EXTERNAL_AGENT_BACKENDS.lock().await;
        guard.remove(&self.kind);
        Ok(())
    }

    async fn is_running(&self) -> Result<bool, String> {
        let guard = EXTERNAL_AGENT_BACKENDS.lock().await;
        Ok(guard.contains(&self.kind))
    }

    async fn list_models(&self) -> Result<Vec<CopilotModelInfo>, String> {
        let mut models = Vec::new();
        match self.kind {
            FlowPilotAgentBackendKind::Codex => {
                // Codex model availability depends on whether the user is
                // authenticated with a ChatGPT account, API key, enterprise
                // policy, and the installed Codex runtime version, so the options
                // are discovered from Codex itself (its `app-server` `model/list`)
                // rather than hard-coded. "default" is always offered first so the
                // user can defer to Codex's own configured/runtime model.
                let mut discovered_models = Vec::new();
                if let Some(cli) = find_cli_resolution(self.kind, None) {
                    match list_codex_models_via_app_server(&cli).await {
                        Ok(discovered) => {
                            discovered_models = discovered;
                        }
                        Err(_error) => {
                            flowpilot_debug_trace!(
                                backend = self.kind.label(),
                                error = %_error,
                                "codex model discovery unavailable; offering configured default only"
                            );
                        }
                    }
                }
                models = codex_models_with_configured_default(discovered_models);
            }
            FlowPilotAgentBackendKind::ClaudeCode => {
                // The Claude Code CLI exposes no model-listing subcommand, so the
                // options are discovered from its own auth-aware `initialize`
                // handshake (the same list the Agent SDK's `supportedModels()`
                // returns) rather than hard-coded. That catalog already includes
                // a "default (recommended)" entry, so nothing is prepended.
                if let Some(cli) = find_cli_resolution(self.kind, None) {
                    match list_claude_models_via_control_protocol(&cli).await {
                        Ok(discovered) => {
                            for model in discovered {
                                if !models.iter().any(|existing| existing.id == model.id) {
                                    models.push(model);
                                }
                            }
                        }
                        Err(_error) => {
                            flowpilot_debug_trace!(
                                backend = self.kind.label(),
                                error = %_error,
                                "claude model discovery unavailable; offering configured default only"
                            );
                        }
                    }
                }
                if models.is_empty() {
                    let mut configured_default =
                        CopilotModelInfo::basic("default", "Claude Code configured default");
                    configured_default.is_default = true;
                    models.push(configured_default);
                }
            }
            FlowPilotAgentBackendKind::GithubCopilot => {
                let mut configured_default =
                    CopilotModelInfo::basic("default", "GitHub Copilot configured default");
                configured_default.is_default = true;
                models.push(configured_default);
            }
        }
        Ok(models)
    }

    async fn get_auth_status(
        &self,
        app_handle: Option<&AppHandle>,
    ) -> Result<CopilotAuthStatus, String> {
        let resolution = find_cli_resolution(self.kind, app_handle);
        let executable = resolution
            .as_ref()
            .map(|resolution| resolution.executable.display().to_string());
        Ok(CopilotAuthStatus {
            authenticated: executable.is_some(),
            login: None,
            message: Some(match executable {
                Some(path) => format!(
                    "{} CLI found at {path} ({:?}). Authentication is delegated to that CLI.",
                    self.kind.label(),
                    resolution.map(|resolution| resolution.source)
                ),
                None => format!(
                    "{} CLI was not found. Set {} to its executable path.",
                    self.kind.label(),
                    self.kind.env_path_var()
                ),
            }),
        })
    }

    async fn status(&self, app_handle: Option<&AppHandle>) -> FlowPilotBackendStatus {
        let resolution = find_cli_resolution(self.kind, app_handle);
        let source = resolution.as_ref().map(|resolution| resolution.source);
        let executable = resolution
            .as_ref()
            .map(|resolution| resolution.executable.display().to_string());
        let available = executable.is_some();
        let running = self.is_running().await.unwrap_or(false);
        FlowPilotBackendStatus {
            backend: self.kind,
            label: self.kind.label().to_string(),
            available,
            running,
            executable,
            message: Some(if available {
                format!(
                    "{} uses FlowPilot's shared prompt/tool surface through a session-local MCP bridge ({source:?}).",
                    self.kind.label(),
                )
            } else {
                format!(
                    "{} CLI was not found. Install it or set {}.",
                    self.kind.label(),
                    self.kind.env_path_var()
                )
            }),
            transport: FlowPilotAgentTransportKind::Mcp,
            capabilities: FlowPilotAgentCapabilitySet::for_status(FlowPilotAgentTransportKind::Mcp),
        }
    }
}

fn parse_agent_backend(backend: String) -> Result<FlowPilotAgentBackendKind, String> {
    FlowPilotAgentBackendKind::parse(&backend)
        .ok_or_else(|| format!("Unsupported FlowPilot backend: {backend}"))
}

#[tauri::command]
pub async fn flowpilot_agent_backend_start(
    app_handle: AppHandle,
    backend: String,
    use_stdio: Option<bool>,
    cli_url: Option<String>,
) -> Result<(), String> {
    let backend = agent_backend(parse_agent_backend(backend)?);
    backend
        .start(FlowPilotBackendStartOptions {
            use_stdio: use_stdio.unwrap_or(true),
            cli_url,
            app_handle: Some(app_handle),
        })
        .await
}

#[tauri::command]
pub async fn flowpilot_agent_backend_stop(backend: String) -> Result<(), String> {
    agent_backend(parse_agent_backend(backend)?).stop().await
}

#[tauri::command]
pub async fn flowpilot_agent_backend_is_running(backend: String) -> Result<bool, String> {
    agent_backend(parse_agent_backend(backend)?)
        .is_running()
        .await
}

#[tauri::command]
pub async fn flowpilot_agent_backend_list_models(
    backend: String,
) -> Result<Vec<CopilotModelInfo>, String> {
    agent_backend(parse_agent_backend(backend)?)
        .list_models()
        .await
}

#[tauri::command]
pub async fn flowpilot_agent_backend_get_auth_status(
    app_handle: AppHandle,
    backend: String,
) -> Result<CopilotAuthStatus, String> {
    agent_backend(parse_agent_backend(backend)?)
        .get_auth_status(Some(&app_handle))
        .await
}

#[tauri::command]
pub async fn flowpilot_agent_backend_status(
    app_handle: AppHandle,
    backend: String,
) -> Result<FlowPilotBackendStatus, String> {
    Ok(agent_backend(parse_agent_backend(backend)?)
        .status(Some(&app_handle))
        .await)
}

#[tauri::command]
pub async fn flowpilot_agent_backend_list(
    app_handle: AppHandle,
) -> Result<Vec<FlowPilotBackendStatus>, String> {
    let mut statuses = Vec::new();
    for backend in [
        FlowPilotAgentBackendKind::GithubCopilot,
        FlowPilotAgentBackendKind::Codex,
        FlowPilotAgentBackendKind::ClaudeCode,
    ] {
        statuses.push(agent_backend(backend).status(Some(&app_handle)).await);
    }
    Ok(statuses)
}

/// Start the GitHub Copilot SDK client
#[tauri::command]
pub async fn copilot_sdk_start(
    app_handle: AppHandle,
    use_stdio: Option<bool>,
    cli_url: Option<String>,
) -> Result<(), String> {
    flowpilot_agent_backend_start(app_handle, "github-copilot".to_string(), use_stdio, cli_url)
        .await
}

/// Stop the GitHub Copilot SDK client
#[tauri::command]
pub async fn copilot_sdk_stop() -> Result<(), String> {
    flowpilot_agent_backend_stop("github-copilot".to_string()).await
}

/// Check if the Copilot SDK client is running
#[tauri::command]
pub async fn copilot_sdk_is_running() -> Result<bool, String> {
    flowpilot_agent_backend_is_running("github-copilot".to_string()).await
}

/// List available GitHub Copilot models
#[tauri::command]
pub async fn copilot_sdk_list_models() -> Result<Vec<CopilotModelInfo>, String> {
    flowpilot_agent_backend_list_models("github-copilot".to_string()).await
}

/// Get GitHub Copilot authentication status
#[tauri::command]
pub async fn copilot_sdk_get_auth_status(
    app_handle: AppHandle,
) -> Result<CopilotAuthStatus, String> {
    flowpilot_agent_backend_get_auth_status(app_handle, "github-copilot".to_string()).await
}

// =============================================================================
// Specialized Agents Configuration
// =============================================================================

/// Specialized agent type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpecializedAgentType {
    General,
    Frontend,
    Backend,
}

/// System prompts for specialized agents — delegate to the shared prompts module
/// in `flow_like::copilot::prompts` for consistency between bits and SDK paths.
fn frontend_agent_prompt() -> String {
    flow_like::copilot::prompts::frontend_sdk_system_prompt()
}

fn backend_agent_prompt() -> String {
    flow_like::copilot::prompts::board_sdk_system_prompt()
}

fn general_agent_prompt() -> String {
    flow_like::copilot::prompts::general_system_prompt()
}

/// Get the system prompt for a specialized agent
fn get_agent_prompt(agent_type: &SpecializedAgentType) -> String {
    match agent_type {
        SpecializedAgentType::General => general_agent_prompt(),
        SpecializedAgentType::Frontend => frontend_agent_prompt(),
        SpecializedAgentType::Backend => backend_agent_prompt(),
    }
}

/// Create a session with a specialized agent using Copilot SDK
#[tauri::command]
pub async fn copilot_sdk_create_agent_session(
    agent_type: SpecializedAgentType,
    model_id: Option<String>,
) -> Result<String, String> {
    let client = COPILOT_CLIENT
        .lock()
        .await
        .clone()
        .ok_or("Copilot client not started")?;

    let system_prompt = get_agent_prompt(&agent_type);

    let config = copilot_sdk::SessionConfig {
        model: model_id,
        streaming: true,
        system_message: Some(copilot_sdk::SystemMessageConfig {
            content: Some(system_prompt),
            mode: Some(copilot_sdk::SystemMessageMode::Append),
        }),
        infinite_sessions: Some(copilot_sdk::InfiniteSessionConfig::enabled()),
        ..Default::default()
    };

    let session = tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, client.create_session(config))
        .await
        .map_err(|_| "Timed out creating specialized Copilot session".to_string())?
        .map_err(|e| format!("Failed to create session: {}", e))?;

    Ok(session.session_id().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_matching_copilot_app_contexts() {
        assert_eq!(
            resolve_copilot_app_id(Some(" app-1 "), Some("app-1"), None).unwrap(),
            Some("app-1".to_string())
        );
    }

    #[test]
    fn rejects_conflicting_copilot_app_contexts() {
        assert!(resolve_copilot_app_id(Some("app-1"), None, Some("app-2")).is_err());
    }

    fn flowscript_recovery_test_board() -> Board {
        Board {
            id: format!("flowscript-recovery-{}", uuid::Uuid::new_v4()),
            name: "Recovery".to_string(),
            description: String::new(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: flow_like::flow::board::ExecutionStage::Dev,
            log_level: flow_like::flow::execution::LogLevel::Info,
            execution_mode: flow_like::flow::board::ExecutionMode::Hybrid,
            refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: std::time::SystemTime::now(),
            updated_at: std::time::SystemTime::now(),
            parent: None,
            board_dir: flow_like::flow_like_storage::Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
        }
    }

    fn retained_recovery_context(
        draft_id: &str,
        revision: u64,
    ) -> flow_like::flow::copilot::FlowIrEditableDraftContext {
        flow_like::flow::copilot::FlowIrEditableDraftContext {
            board_id: "board".to_string(),
            draft_id: draft_id.to_string(),
            revision,
            status: "editing".to_string(),
            base_fingerprint: "base".to_string(),
            missing_modules: vec!["send_reply".to_string()],
            remaining_capabilities: vec!["smtp_send".to_string()],
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn nested_runs_in_one_conversation_share_request_identity() {
        let outer_prompt = "yes, build it";
        let context = FrontendToolContext {
            conversation_id: Some("conversation-1".to_string()),
            source_user_prompt: Some(outer_prompt.to_string()),
            ..Default::default()
        };

        let first_nested = request_identity_prompt_for(
            Some(&context),
            "Execute the change NOW: build the intake workflow.",
        );
        let repair_nested = request_identity_prompt_for(
            Some(&context),
            "Repair the retained draft draft-1 at revision 3.",
        );

        assert_eq!(first_nested, repair_nested);
        assert_eq!(first_nested, format!("conversation-1\n{outer_prompt}"));
    }

    #[test]
    fn identical_prompts_in_different_conversations_never_share_identity() {
        let outer_prompt = "yes, build it";
        let conversation = |id: &str| FrontendToolContext {
            conversation_id: Some(id.to_string()),
            source_user_prompt: Some(outer_prompt.to_string()),
            ..Default::default()
        };

        assert_ne!(
            request_identity_prompt_for(Some(&conversation("conversation-a")), outer_prompt),
            request_identity_prompt_for(Some(&conversation("conversation-b")), outer_prompt),
        );
    }

    #[test]
    fn request_identity_falls_back_to_raw_prompt_without_conversation_scope() {
        assert_eq!(
            request_identity_prompt_for(None, "add a logging node"),
            "add a logging node"
        );
        assert_eq!(
            request_identity_prompt_for(
                Some(&FrontendToolContext::default()),
                "add a logging node"
            ),
            "add a logging node"
        );
        let conversation_only = FrontendToolContext {
            conversation_id: Some("conversation-1".to_string()),
            ..Default::default()
        };
        assert_eq!(
            request_identity_prompt_for(Some(&conversation_only), "add a logging node"),
            "conversation-1\nadd a logging node"
        );
        let blank_scope = FrontendToolContext {
            conversation_id: Some("   ".to_string()),
            source_user_prompt: Some("  ".to_string()),
            ..Default::default()
        };
        assert_eq!(
            request_identity_prompt_for(Some(&blank_scope), "add a logging node"),
            "add a logging node"
        );
    }

    #[test]
    fn desktop_recovery_injects_only_exact_request_coordinates() {
        let exact = flow_like::flow::copilot::FlowIrDraftRecovery {
            status: flow_like::flow::copilot::FlowIrDraftRecoveryStatus::ExactMatch,
            auto_resume: true,
            exact_match: Some(retained_recovery_context("exact-draft", 7)),
            conflicting_draft: None,
            next_actions: vec!["resume_exact_draft".to_string()],
            message: "exact request".to_string(),
        };
        let mut exact_prompt = String::new();
        append_typed_ir_recovery_context(&mut exact_prompt, &exact);
        assert!(exact_prompt.contains("EXACT TYPED-DRAFT RECOVERY"));
        assert!(exact_prompt.contains("exact-draft"));
        assert!(exact_prompt.contains("\"revision\": 7"));

        let mismatch = flow_like::flow::copilot::FlowIrDraftRecovery {
            status: flow_like::flow::copilot::FlowIrDraftRecoveryStatus::RequestMismatch,
            auto_resume: false,
            exact_match: None,
            conflicting_draft: Some(retained_recovery_context("secret-old-draft", 11)),
            next_actions: vec![
                "recover_with_original_request".to_string(),
                "abandon_retained_draft_via_host".to_string(),
            ],
            message: "different immutable request".to_string(),
        };
        let mut mismatch_prompt = String::new();
        append_typed_ir_recovery_context(&mut mismatch_prompt, &mismatch);
        assert!(mismatch_prompt.contains("TYPED-DRAFT REQUEST MISMATCH"));
        assert!(mismatch_prompt.contains("abandon_retained_draft_via_host"));
        assert!(mismatch_prompt.contains("\"auto_resume\": false"));
        assert!(!mismatch_prompt.contains("secret-old-draft"));
        assert!(!mismatch_prompt.contains("\"revision\""));
    }

    #[test]
    fn desktop_source_recovery_resumes_exact_request_and_hides_mismatches() {
        let board = flowscript_recovery_test_board();
        let request = "Build a durable customer-support logging workflow.";
        let draft_id = format!("source-draft-{}", uuid::Uuid::new_v4());
        let source = "function retainedRecoveryMarker() {\n    missingCatalogCall()\n}\n";
        let store = retained_flow_ir_draft_store_for_board(&board)
            .expect("desktop source recovery should acquire the board-scoped store");
        let binding = store.bind_request_acceptance_contract(&board.id, request);
        let written = store.write_flowscript_with_acceptance_binding(
            &board,
            &[],
            flow_like::flow::copilot::WriteFlowScriptArgs {
                draft_id: draft_id.clone(),
                replace_existing: false,
                mode: flow_like::flow::copilot::FlowIrDraftMode::Additive,
                source: source.to_string(),
                allow_scope_reduction: false,
            },
            &binding,
        );
        assert_eq!(written.revision, Some(0));

        let mut exact_prompt = String::new();
        append_flowscript_recovery_context(&mut exact_prompt, &board, request);
        assert!(exact_prompt.contains("EXACT RETAINED FLOWSCRIPT RECOVERY"));
        assert!(exact_prompt.contains(&draft_id));
        assert!(exact_prompt.contains("retainedRecoveryMarker"));
        assert!(exact_prompt.contains("revision: `0`"));

        let mut mismatch_prompt = String::new();
        append_flowscript_recovery_context(
            &mut mismatch_prompt,
            &board,
            "Build an unrelated invoice workflow.",
        );
        assert!(mismatch_prompt.contains("FLOWSCRIPT REQUEST MISMATCH"));
        assert!(mismatch_prompt.contains("source is intentionally hidden"));
        assert!(!mismatch_prompt.contains(&draft_id));
        assert!(!mismatch_prompt.contains("retainedRecoveryMarker"));

        let mut advanced_board = board.clone();
        let variable = flow_like::flow::variable::Variable::new(
            "board_changed_after_timeout",
            flow_like::flow::variable::VariableType::String,
            flow_like::flow::pin::ValueType::Normal,
        );
        advanced_board
            .variables
            .insert(variable.id.clone(), variable);
        let mut stale_prompt = String::new();
        append_flowscript_recovery_context(&mut stale_prompt, &advanced_board, request);
        assert!(stale_prompt.contains("STALE RETAINED FLOWSCRIPT"));
        assert!(stale_prompt.contains("retainedRecoveryMarker"));
        assert!(stale_prompt.contains("fresh draft_id"));
    }

    #[test]
    fn desktop_pending_source_redelivery_preserves_the_exact_review_payload() {
        let commands = vec![BoardCommand::RemoveNode {
            node_id: "exact-redelivery-node".to_string(),
            summary: None,
        }];
        let token = FlowIrCommitToken {
            board_id: "redelivery-board".to_string(),
            draft_id: "redelivery-draft".to_string(),
            revision: 4,
            base_fingerprint: "redelivery-base".to_string(),
            claim_id: "redelivery-claim".to_string(),
            requires_destructive_approval: true,
        };
        let response = pending_flowscript_redelivery_response(
            CopilotScope::Board,
            FlowScriptPendingDelivery {
                source: "eventsSimple() {\n    logInfo({ message: \"hello\" })\n}\n".to_string(),
                token: token.clone(),
                stale_board: false,
                commands: commands.clone(),
            },
        );

        assert_eq!(
            serde_json::to_value(&response.commands).unwrap(),
            serde_json::to_value(&commands).unwrap()
        );
        assert_eq!(response.flow_ir_commit, Some(token));
        assert_eq!(response.active_scope, CopilotScope::Board);
        let workspace: serde_json::Value = serde_json::from_str(
            response
                .flowscript_workspace
                .as_deref()
                .expect("redelivery includes the exact source workspace"),
        )
        .expect("workspace is valid JSON");
        assert_eq!(workspace["status"], "queued");
        assert!(workspace["source"].as_str().unwrap().contains("logInfo"));
        assert!(response.message.contains("No model generation"));

        let stale = pending_flowscript_redelivery_response(
            CopilotScope::Board,
            FlowScriptPendingDelivery {
                source: "eventsSimple() {}\n".to_string(),
                token: response.flow_ir_commit.clone().unwrap(),
                stale_board: true,
                commands: Vec::new(),
            },
        );
        assert!(stale.commands.is_empty());
        let stale_workspace: serde_json::Value = serde_json::from_str(
            stale
                .flowscript_workspace
                .as_deref()
                .expect("stale redelivery includes retained source"),
        )
        .expect("stale workspace is valid JSON");
        assert_eq!(stale_workspace["status"], "stale");
        assert!(stale.message.contains("dismiss this stale review"));
    }

    fn workflow_call_result_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
        let text = result
            .content
            .iter()
            .find_map(|content| match &content.raw {
                rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .expect("workflow guard result contains JSON text");
        serde_json::from_str(text).expect("workflow guard result is valid JSON")
    }

    #[test]
    fn atomic_typed_apply_errors_keep_the_full_apply_result_contract() {
        let response = ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_REVIEW_STALE",
            "Nothing was applied.",
        );
        let value = serde_json::to_value(response).expect("serialize atomic apply response");

        assert_eq!(value["status"], "stale");
        assert_eq!(value["code"], "IR_COMMIT_REVIEW_STALE");
        assert_eq!(value["commands"], serde_json::json!([]));
        assert_eq!(value["board_commands"], serde_json::json!([]));
        assert_eq!(value["diagnostics"], serde_json::json!([]));
    }

    #[test]
    fn typed_replacement_and_deletions_are_both_destructive_review_gated() {
        assert_eq!(
            typed_commit_destructive_review_items(true, &[]),
            vec!["The draft uses full-board replacement semantics."]
        );
        assert!(typed_commit_destructive_review_items(false, &[]).is_empty());
        assert_eq!(
            typed_commit_destructive_review_items(
                false,
                &[BoardCommand::RemoveNode {
                    node_id: "existing-node".to_string(),
                    summary: None,
                }],
            ),
            vec!["node `existing-node`"]
        );
    }

    #[test]
    fn native_destructive_dialog_window_revalidates_the_exact_batch() {
        let reviewed = vec![BoardCommand::RemoveNode {
            node_id: "reviewed-node".to_string(),
            summary: None,
        }];
        assert!(exact_board_command_batch_matches(&reviewed, &reviewed));

        let changed = vec![BoardCommand::RemoveNode {
            node_id: "different-node".to_string(),
            summary: None,
        }];
        assert!(!exact_board_command_batch_matches(&reviewed, &changed));
        assert!(!exact_board_command_batch_matches(&reviewed, &[]));
    }

    #[test]
    fn atomic_typed_apply_receipt_replays_exact_success_after_lost_response() {
        let token = FlowIrCommitToken {
            board_id: "receipt-board".to_string(),
            draft_id: "receipt-draft".to_string(),
            revision: 7,
            base_fingerprint: "base".to_string(),
            claim_id: uuid::Uuid::new_v4().to_string(),
            requires_destructive_approval: false,
        };
        let result = ApplyFlowIrCommitResult {
            status: "applied".to_string(),
            code: None,
            message: "Applied exact typed batch.".to_string(),
            commands: Vec::new(),
            board_commands: Vec::new(),
            diagnostics: Vec::new(),
            final_board_node_count: Some(3),
        };

        retain_flow_ir_applied_receipt("receipt-app", &token, &result);
        let replay = replay_flow_ir_applied_receipt("receipt-app", &token)
            .expect("exact token replays its applied receipt");
        assert_eq!(replay.status, "applied");
        assert_eq!(replay.final_board_node_count, Some(3));
        assert!(replay.message.contains("idempotent replay"));

        let mut wrong_claim = token.clone();
        wrong_claim.claim_id = uuid::Uuid::new_v4().to_string();
        assert!(replay_flow_ir_applied_receipt("receipt-app", &wrong_claim).is_none());

        FLOW_IR_APPLIED_RECEIPTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&flow_ir_applied_receipt_key("receipt-app", &token));
    }

    #[test]
    fn flowpilot_mcp_keeps_long_running_sse_calls_alive() {
        let config = flowpilot_mcp_server_config();
        assert_eq!(config.sse_keep_alive, Some(Duration::from_secs(15)));
        assert!(config.stateful_mode);
    }

    #[test]
    fn mcp_tool_heartbeat_uses_the_callers_progress_token_and_no_fake_total() {
        let token = rmcp::model::ProgressToken(rmcp::model::NumberOrString::String(
            Arc::<str>::from("flowpilot-request"),
        ));
        let first = mcp_progress_heartbeat_notification(
            token.clone(),
            1.0,
            "FlowPilot flowpilot_board is still running",
        );
        let second = mcp_progress_heartbeat_notification(
            token.clone(),
            2.0,
            "FlowPilot flowpilot_board is still running",
        );

        assert_eq!(first.progress_token, token);
        assert!(second.progress > first.progress);
        assert_eq!(first.total, None, "heartbeat ticks are not percentages");
        assert_eq!(
            first.message.as_deref(),
            Some("FlowPilot flowpilot_board is still running")
        );
        assert!(MCP_TOOL_PROGRESS_HEARTBEAT_INTERVAL < Duration::from_secs(300));
        assert_eq!(
            serde_json::to_value(first).expect("serialize progress notification")["progressToken"],
            "flowpilot-request"
        );
    }

    #[test]
    fn dropped_mcp_tool_request_cancels_its_blocking_handler() {
        let cancellation = CancellationToken::new();
        {
            let _guard = McpToolCancellationGuard::new(cancellation.clone());
            assert!(!cancellation.is_cancelled());
        }
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn completed_mcp_tool_request_does_not_emit_cancellation() {
        let cancellation = CancellationToken::new();
        {
            let mut guard = McpToolCancellationGuard::new(cancellation.clone());
            guard.disarm();
        }
        assert!(!cancellation.is_cancelled());
    }

    #[test]
    fn mcp_handler_registry_tracks_workers_until_their_guard_drops() {
        let activity = Arc::new(StdMutex::new(McpToolActivityState::default()));
        let quiescence = Arc::new(tokio::sync::Notify::new());
        let cancellation = CancellationToken::new();
        let guard = register_mcp_active_handler(&activity, &quiescence, cancellation.clone())
            .expect("handler registration");

        {
            let state = activity.lock().expect("handler registry");
            assert_eq!(state.active_handlers.len(), 1);
            state
                .active_handlers
                .values()
                .for_each(CancellationToken::cancel);
        }
        assert!(cancellation.is_cancelled());

        drop(guard);
        assert!(
            activity
                .lock()
                .expect("handler registry")
                .active_handlers
                .is_empty(),
            "a provider continuation may start only after the worker leaves the registry"
        );
    }

    #[test]
    fn dropping_copilot_run_guard_cancels_owned_tool_scope() {
        let (cancellation, guard) = register_copilot_run(None);
        assert!(!cancellation.is_cancelled());
        drop(guard);
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn nested_copilot_gate_waits_for_owner_without_a_queue_deadline() {
        let gate = Arc::new(Semaphore::new(1));
        let owner = gate
            .clone()
            .acquire_owned()
            .await
            .expect("initial nested owner");
        let cancellation = CancellationToken::new();
        let waiter = tokio::spawn(acquire_nested_copilot_run_permit(gate, cancellation));

        tokio::task::yield_now().await;
        assert!(
            !waiter.is_finished(),
            "the queued specialist must remain pending while another run owns the CLI"
        );
        drop(owner);

        let permit = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("queued specialist should acquire promptly after release")
            .expect("waiter task")
            .expect("nested permit");
        drop(permit);
    }

    #[tokio::test]
    async fn nested_copilot_gate_wait_is_explicitly_cancellable() {
        let gate = Arc::new(Semaphore::new(1));
        let _owner = gate
            .clone()
            .acquire_owned()
            .await
            .expect("initial nested owner");
        let cancellation = CancellationToken::new();
        let waiter = tokio::spawn(acquire_nested_copilot_run_permit(
            gate,
            cancellation.clone(),
        ));

        cancellation.cancel();
        let error = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("cancelled waiter should return promptly")
            .expect("waiter task")
            .expect_err("cancelled nested permit");
        assert!(error.contains("cancelled"));
    }

    #[test]
    fn nested_gate_key_prefers_board_then_widget_target_then_global() {
        let board = flowscript_recovery_test_board();
        let context = FrontendToolContext {
            board_id: Some("widget-target-board".to_string()),
            ..Default::default()
        };
        assert_eq!(
            nested_copilot_run_gate_key(Some(&board), Some(&context)),
            format!("board:{}", board.id)
        );
        assert_eq!(
            nested_copilot_run_gate_key(None, Some(&context)),
            "board:widget-target-board"
        );
        let empty_context = FrontendToolContext {
            board_id: Some("   ".to_string()),
            ..Default::default()
        };
        assert_eq!(
            nested_copilot_run_gate_key(None, Some(&empty_context)),
            "global"
        );
        assert_eq!(nested_copilot_run_gate_key(None, None), "global");
    }

    #[tokio::test]
    async fn nested_gate_serializes_same_board_but_not_different_boards() {
        let gate_a = nested_copilot_run_gate("board:gate-test-a");
        let gate_a_again = nested_copilot_run_gate("board:gate-test-a");
        let gate_b = nested_copilot_run_gate("board:gate-test-b");
        assert!(Arc::ptr_eq(&gate_a, &gate_a_again));

        let owner = gate_a
            .clone()
            .acquire_owned()
            .await
            .expect("first same-board run");
        let cancellation = CancellationToken::new();
        let same_board_waiter = tokio::spawn(acquire_nested_copilot_run_permit(
            gate_a_again,
            cancellation.clone(),
        ));
        let other_board_run = tokio::spawn(acquire_nested_copilot_run_permit(
            gate_b,
            cancellation.clone(),
        ));

        // A different board's run must complete while the same-board run is still queued.
        let other_permit = tokio::time::timeout(Duration::from_secs(1), other_board_run)
            .await
            .expect("a different board must not queue behind this board's run")
            .expect("other-board task")
            .expect("other-board permit");
        tokio::task::yield_now().await;
        assert!(
            !same_board_waiter.is_finished(),
            "a second run on the SAME board must stay queued while the first one runs"
        );

        drop(owner);
        let same_permit = tokio::time::timeout(Duration::from_secs(1), same_board_waiter)
            .await
            .expect("same-board run should acquire promptly after release")
            .expect("same-board task")
            .expect("same-board permit");
        drop(same_permit);
        drop(other_permit);
    }

    #[tokio::test]
    async fn nested_gate_map_prunes_gates_without_holders_or_waiters() {
        let key = "board:gate-prune-test";
        let gate = nested_copilot_run_gate(key);
        let permit = gate
            .clone()
            .acquire_owned()
            .await
            .expect("prune-test permit");
        drop(gate);

        // The held permit keeps an Arc clone alive, so lookups of other keys must not prune it.
        let _other = nested_copilot_run_gate("board:gate-prune-test-other");
        assert!(
            NESTED_COPILOT_RUN_GATES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains_key(key),
            "a gate with an active permit holder must survive pruning"
        );

        drop(permit);
        let _other = nested_copilot_run_gate("board:gate-prune-test-other");
        assert!(
            !NESTED_COPILOT_RUN_GATES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains_key(key),
            "a gate with no holders and no waiters must be pruned on the next lookup"
        );
    }

    #[tokio::test]
    async fn nested_pool_checkout_checkin_and_quarantine_replacement() {
        fn unstarted_client() -> Arc<Client> {
            Arc::new(Client::builder().build().expect("unstarted pool client"))
        }
        // Seed idle clients so checkout never spawns a real CLI process in tests.
        for _ in 0..NESTED_COPILOT_POOL_SIZE {
            let client = unstarted_client();
            assert!(
                NESTED_COPILOT_POOL.register_started(client.clone(), NESTED_COPILOT_POOL.epoch()),
                "seeding an undrained pool must register"
            );
            NESTED_COPILOT_POOL.return_to_idle(client);
        }

        let cancellation = CancellationToken::new();
        let lease_one = checkout_nested_copilot_client(cancellation.clone())
            .await
            .expect("first checkout");
        let lease_two = checkout_nested_copilot_client(cancellation.clone())
            .await
            .expect("second checkout");
        let lease_three = checkout_nested_copilot_client(cancellation.clone())
            .await
            .expect("third checkout");
        assert!(
            !Arc::ptr_eq(&lease_one.client, &lease_two.client)
                && !Arc::ptr_eq(&lease_two.client, &lease_three.client)
                && !Arc::ptr_eq(&lease_one.client, &lease_three.client),
            "each checked-out lease must exclusively own its own process"
        );

        // All slots busy: a fourth checkout queues, and a cancelled one returns promptly.
        let cancelled_token = CancellationToken::new();
        let cancelled_waiter =
            tokio::spawn(checkout_nested_copilot_client(cancelled_token.clone()));
        cancelled_token.cancel();
        let cancelled_result = tokio::time::timeout(Duration::from_secs(1), cancelled_waiter)
            .await
            .expect("cancelled checkout should return promptly")
            .expect("cancelled checkout task");
        match cancelled_result {
            Ok(_) => panic!("cancelled checkout must not receive a client"),
            Err(error) => assert!(error.contains("cancelled")),
        }

        let waiter = tokio::spawn(checkout_nested_copilot_client(cancellation.clone()));
        tokio::task::yield_now().await;
        assert!(
            !waiter.is_finished(),
            "a checkout past the pool cap must wait for a checkin"
        );

        // Checkin: dropping a lease returns its exact process to the idle pool.
        let released = lease_one.client();
        drop(lease_one);
        let lease_four = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("queued checkout should acquire promptly after a checkin")
            .expect("queued checkout task")
            .expect("queued checkout lease");
        assert!(
            Arc::ptr_eq(&lease_four.client, &released),
            "the queued checkout must reuse the checked-in idle process"
        );

        // Quarantine: the client leaves the pool, its lease drop must not re-pool it, and the
        // freed slot allows a lazy replacement.
        let quarantined = lease_two.client();
        quarantine_nested_copilot_client(&quarantined).await;
        drop(lease_two);
        assert!(
            !NESTED_COPILOT_POOL.is_registered(&quarantined),
            "a quarantined client must leave the pool registry"
        );
        assert_eq!(
            NESTED_COPILOT_POOL.slots.available_permits(),
            1,
            "the quarantined client's slot must free up for a lazy replacement"
        );

        drop(lease_three);
        drop(lease_four);
        let drained = NESTED_COPILOT_POOL.drain();
        assert_eq!(
            drained.len(),
            NESTED_COPILOT_POOL_SIZE - 1,
            "drain must return every live pooled client except the quarantined one"
        );
        assert!(
            drained
                .iter()
                .all(|client| !Arc::ptr_eq(client, &quarantined)),
            "the quarantined client must not reappear in the drained pool"
        );
    }

    fn unstarted_pool_client() -> Arc<Client> {
        Arc::new(Client::builder().build().expect("unstarted pool client"))
    }

    fn leaked_test_pool(size: usize) -> &'static NestedCopilotPool {
        Box::leak(Box::new(NestedCopilotPool::new(size)))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn nested_gate_stress_serializes_same_board_and_overlaps_across_boards() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        const BOARDS: usize = 4;
        const TASKS_PER_BOARD: usize = 8;
        const KEY_PREFIX: &str = "board:stress-gate-";

        let per_board_in_flight: Arc<Vec<AtomicUsize>> =
            Arc::new((0..BOARDS).map(|_| AtomicUsize::new(0)).collect());
        let global_in_flight = Arc::new(AtomicUsize::new(0));
        let max_global_in_flight = Arc::new(AtomicUsize::new(0));
        // One run per board rendezvouses INSIDE its critical section. Per-board gates make this
        // trivially deadlock-free; a global gate would deadlock here and trip the timeout.
        let cross_board_rendezvous = Arc::new(tokio::sync::Barrier::new(BOARDS));

        let mut handles = Vec::new();
        for board in 0..BOARDS {
            for task in 0..TASKS_PER_BOARD {
                let per_board_in_flight = per_board_in_flight.clone();
                let global_in_flight = global_in_flight.clone();
                let max_global_in_flight = max_global_in_flight.clone();
                let cross_board_rendezvous = cross_board_rendezvous.clone();
                handles.push(tokio::spawn(async move {
                    let gate = nested_copilot_run_gate(&format!("{KEY_PREFIX}{board}"));
                    let permit = acquire_nested_copilot_run_permit(gate, CancellationToken::new())
                        .await
                        .expect("stress gate permit");
                    let overlapping = per_board_in_flight[board].fetch_add(1, Ordering::SeqCst);
                    assert_eq!(
                        overlapping, 0,
                        "two nested runs overlapped on board {board}"
                    );
                    let concurrent = global_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_global_in_flight.fetch_max(concurrent, Ordering::SeqCst);
                    if task == 0 {
                        cross_board_rendezvous.wait().await;
                    }
                    tokio::time::sleep(Duration::from_millis(((board + task) % 3) as u64)).await;
                    global_in_flight.fetch_sub(1, Ordering::SeqCst);
                    per_board_in_flight[board].fetch_sub(1, Ordering::SeqCst);
                    drop(permit);
                }));
            }
        }
        tokio::time::timeout(Duration::from_secs(8), async {
            for handle in handles {
                handle.await.expect("gate stress task");
            }
        })
        .await
        .expect("per-board gates must never deadlock");

        assert!(
            max_global_in_flight.load(Ordering::SeqCst) >= BOARDS,
            "runs on different boards must overlap; the rendezvous held {BOARDS} boards' gates at once"
        );

        // The map must not leak finished stress gates: any lookup prunes ownerless entries.
        let _probe = nested_copilot_run_gate("board:stress-probe");
        let gates = NESTED_COPILOT_RUN_GATES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            gates.keys().all(|key| !key.starts_with(KEY_PREFIX)),
            "finished stress gates must be pruned from the gate map"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nested_gate_stress_cancelled_waiters_release_and_survivors_proceed() {
        const KEY: &str = "board:stress-gate-cancel";
        let gate = nested_copilot_run_gate(KEY);
        let owner = gate
            .clone()
            .acquire_owned()
            .await
            .expect("initial stress owner");

        let mut cancelled = Vec::new();
        let mut survivors = Vec::new();
        for index in 0..6 {
            let token = CancellationToken::new();
            let waiter_gate = gate.clone();
            let waiter_token = token.clone();
            // Survivors drop their permits inside the task: the semaphore queue order is not the
            // spawn order, so holding permits in unawaited JoinHandles would self-deadlock.
            let handle = tokio::spawn(async move {
                acquire_nested_copilot_run_permit(waiter_gate, waiter_token)
                    .await
                    .map(drop)
            });
            if index % 2 == 0 {
                cancelled.push((token, handle));
            } else {
                survivors.push(handle);
            }
        }

        // Cancelling queued waiters while the owner still holds the gate must fail them promptly
        // without consuming the permit.
        for (token, handle) in cancelled {
            token.cancel();
            let error = tokio::time::timeout(Duration::from_secs(2), handle)
                .await
                .expect("cancelled gate waiter must return promptly")
                .expect("cancelled waiter task")
                .expect_err("cancelled waiter must not acquire");
            assert!(error.contains("cancelled"));
        }

        drop(owner);
        // Every surviving waiter must acquire in turn once earlier permits are released.
        tokio::time::timeout(Duration::from_secs(8), async {
            for handle in survivors {
                handle
                    .await
                    .expect("surviving waiter task")
                    .expect("surviving waiter must acquire after cancellations");
            }
        })
        .await
        .expect("cancelled waiters must not strand surviving waiters");

        drop(gate);
        let _probe = nested_copilot_run_gate("board:stress-probe");
        assert!(
            !NESTED_COPILOT_RUN_GATES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains_key(KEY),
            "a fully drained gate must be pruned"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn nested_pool_stress_never_double_leases_and_replaces_quarantined_clients() {
        use std::collections::HashSet;
        use std::sync::atomic::{AtomicUsize, Ordering};

        const POOL_SIZE: usize = 3;
        const TASKS: usize = 24;
        let pool = leaked_test_pool(POOL_SIZE);
        let spawned = Arc::new(AtomicUsize::new(0));
        let quarantined = Arc::new(AtomicUsize::new(0));
        let leased: Arc<StdMutex<HashSet<usize>>> = Arc::new(StdMutex::new(HashSet::new()));

        let mut handles = Vec::new();
        for task in 0..TASKS {
            let spawned = spawned.clone();
            let quarantined = quarantined.clone();
            let leased = leased.clone();
            handles.push(tokio::spawn(async move {
                let spawn_counter = spawned.clone();
                let lease = checkout_nested_copilot_client_from(
                    pool,
                    CancellationToken::new(),
                    move || async move {
                        spawn_counter.fetch_add(1, Ordering::SeqCst);
                        Ok(unstarted_pool_client())
                    },
                )
                .await
                .expect("stress checkout");
                let key = Arc::as_ptr(&lease.client) as usize;
                assert!(
                    leased.lock().expect("lease tracker").insert(key),
                    "one pooled client was leased to two concurrent runs"
                );
                tokio::time::sleep(Duration::from_millis((task % 3) as u64)).await;
                if task % 5 == 0 {
                    // Quarantine without force_stop: these clients were never started.
                    assert!(
                        pool.deregister(&lease.client),
                        "a quarantine target must still be registered"
                    );
                    quarantined.fetch_add(1, Ordering::SeqCst);
                }
                assert!(leased.lock().expect("lease tracker").remove(&key));
                drop(lease);
            }));
        }
        tokio::time::timeout(Duration::from_secs(8), async {
            for handle in handles {
                handle.await.expect("pool stress task");
            }
        })
        .await
        .expect("more waiters than pool slots must drain without deadlock");

        assert_eq!(
            pool.slots.available_permits(),
            POOL_SIZE,
            "every pool slot must be released after its lease drops"
        );
        let survivors = pool.drain();
        assert!(
            survivors.len() <= POOL_SIZE,
            "the pool must never hold more live clients than slots"
        );
        assert_eq!(
            spawned.load(Ordering::SeqCst),
            survivors.len() + quarantined.load(Ordering::SeqCst),
            "every spawned replacement must end up pooled or quarantined, never lost or duplicated"
        );
    }

    #[tokio::test]
    async fn nested_pool_cancelled_waiters_release_their_queue_positions() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let pool = leaked_test_pool(1);
        let seeded = unstarted_pool_client();
        assert!(pool.register_started(seeded.clone(), pool.epoch()));
        pool.return_to_idle(seeded.clone());

        let spawned = Arc::new(AtomicUsize::new(0));
        let factory = |spawned: Arc<AtomicUsize>| {
            move || async move {
                spawned.fetch_add(1, Ordering::SeqCst);
                Ok(unstarted_pool_client())
            }
        };

        let lease = checkout_nested_copilot_client_from(
            pool,
            CancellationToken::new(),
            factory(spawned.clone()),
        )
        .await
        .expect("initial checkout");

        let mut cancelled = Vec::new();
        let mut survivors = Vec::new();
        for index in 0..4 {
            let token = CancellationToken::new();
            let waiter_token = token.clone();
            let waiter_factory = factory(spawned.clone());
            let expected_client = seeded.clone();
            // Survivors drop their leases inside the task: the slot queue order is not the spawn
            // order, so holding leases in unawaited JoinHandles would self-deadlock the pool.
            let handle = tokio::spawn(async move {
                let lease =
                    checkout_nested_copilot_client_from(pool, waiter_token, waiter_factory).await?;
                assert!(
                    Arc::ptr_eq(&lease.client, &expected_client),
                    "a freed idle client must be reused before any new process spawns"
                );
                drop(lease);
                Ok::<(), String>(())
            });
            if index % 2 == 0 {
                cancelled.push((token, handle));
            } else {
                survivors.push(handle);
            }
        }

        // With the single slot still leased, cancelled waiters must fail promptly and must not
        // consume the slot.
        for (token, handle) in cancelled {
            token.cancel();
            let error = tokio::time::timeout(Duration::from_secs(2), handle)
                .await
                .expect("cancelled pool waiter must return promptly")
                .expect("cancelled waiter task")
                .expect_err("cancelled waiter must not receive a lease");
            assert!(error.contains("cancelled"));
        }

        drop(lease);
        tokio::time::timeout(Duration::from_secs(8), async {
            for handle in survivors {
                handle
                    .await
                    .expect("surviving waiter task")
                    .expect("surviving waiter must check out after cancellations");
            }
        })
        .await
        .expect("cancelled waiters must not strand surviving checkouts");

        assert_eq!(
            spawned.load(Ordering::SeqCst),
            0,
            "the idle client is always returned before the slot frees, so no waiter may spawn"
        );
        assert_eq!(pool.slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn nested_pool_drain_during_client_start_rejects_the_stale_client() {
        let pool = leaked_test_pool(1);
        let result = checkout_nested_copilot_client_from(
            pool,
            CancellationToken::new(),
            move || async move {
                // A backend stop lands while the replacement CLI process is still starting.
                assert!(pool.drain().is_empty());
                Ok(unstarted_pool_client())
            },
        )
        .await;
        let error = match result {
            Ok(_) => panic!("a client started across a drain must not join the pool"),
            Err(error) => error,
        };
        assert!(error.contains("drained"), "unexpected error: {error}");
        assert!(
            pool.drain().is_empty(),
            "the stale client must not be registered into the drained pool"
        );
        assert_eq!(
            pool.slots.available_permits(),
            1,
            "the rejected checkout must release its slot"
        );
    }

    #[tokio::test]
    async fn nested_checkout_fails_fast_after_backend_stop_clears_start_options() {
        {
            let mut opts = COPILOT_START_OPTIONS.lock().await;
            *opts = Some(FlowPilotBackendStartOptions {
                use_stdio: true,
                cli_url: None,
                app_handle: None,
            });
        }
        assert!(nested_copilot_start_options().await.is_ok());

        // Backend stop clears the stored options alongside draining the nested pool.
        COPILOT_START_OPTIONS.lock().await.take();

        let pool = leaked_test_pool(1);
        let result =
            checkout_nested_copilot_client_from(pool, CancellationToken::new(), || async {
                nested_copilot_start_options().await?;
                Err::<Arc<Client>, String>(
                    "a post-stop checkout must not reach client startup".to_string(),
                )
            })
            .await;
        let error = match result {
            Ok(_) => panic!("checkout after backend stop must fail fast"),
            Err(error) => error,
        };
        assert!(error.contains("not started"), "unexpected error: {error}");
        assert_eq!(
            pool.slots.available_permits(),
            1,
            "the failed checkout must release its slot"
        );
    }

    #[tokio::test]
    async fn nested_pool_drained_lease_is_not_returned_to_idle() {
        let pool = leaked_test_pool(1);
        let seeded = unstarted_pool_client();
        assert!(pool.register_started(seeded.clone(), pool.epoch()));
        pool.return_to_idle(seeded.clone());

        let lease = checkout_nested_copilot_client_from(pool, CancellationToken::new(), || async {
            Err::<Arc<Client>, String>("factory must not run for an idle checkout".to_string())
        })
        .await
        .expect("seeded checkout");

        let stopped = pool.drain();
        assert_eq!(stopped.len(), 1);
        assert!(Arc::ptr_eq(&stopped[0], &seeded));

        drop(lease);
        assert!(
            pool.take_idle().is_none(),
            "a lease drained mid-run was force-stopped by the backend stop path and must not rejoin idle"
        );
        assert_eq!(pool.slots.available_permits(), 1);
    }

    #[test]
    fn direct_sdk_handlers_inherit_cancellation_without_an_overall_deadline() {
        let observed_no_deadline = Arc::new(StdMutex::new(false));
        let observed = observed_no_deadline.clone();
        let handler: copilot_sdk::ToolHandler = Arc::new(move |_name, _args| {
            let (cancellation, deadline) =
                crate::functions::ai::frontend_tool_bridge::current_tool_execution_for_test()
                    .expect("SDK handler must inherit the frontend execution scope");
            *observed.lock().expect("observation lock") = deadline.is_none();
            cancellation.cancel();
            copilot_sdk::ToolResultObject::text("ok")
        });
        let cancellation = CancellationToken::new();
        let mut tools = scope_sdk_tool_handlers(
            vec![(copilot_sdk::Tool::new("runtime_test"), handler)],
            cancellation.clone(),
        );
        let (_, handler) = tools.pop().expect("scoped SDK handler");

        let result = handler("runtime_test", &serde_json::json!({}));

        assert_eq!(result.text_result_for_llm, "ok");
        assert!(
            *observed_no_deadline.lock().expect("observation lock"),
            "an active SDK provider run must not inherit an arbitrary wall-clock deadline"
        );
        assert!(
            cancellation.is_cancelled(),
            "the scoped handler must receive the owning run token, not a detached token"
        );
        assert!(
            crate::functions::ai::frontend_tool_bridge::current_tool_execution_for_test().is_none(),
            "the synchronous SDK scope must be restored after the handler returns"
        );
    }

    #[test]
    fn cancelled_direct_sdk_handler_never_starts() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let calls = Arc::new(AtomicUsize::new(0));
        let handler_calls = calls.clone();
        let handler: copilot_sdk::ToolHandler = Arc::new(move |_name, _args| {
            handler_calls.fetch_add(1, Ordering::SeqCst);
            copilot_sdk::ToolResultObject::text("unexpected")
        });
        let cancellation = CancellationToken::new();
        let mut tools = scope_sdk_tool_handlers(
            vec![(copilot_sdk::Tool::new("runtime_test"), handler)],
            cancellation.clone(),
        );
        cancellation.cancel();
        let (_, handler) = tools.pop().expect("scoped SDK handler");

        let result = handler("runtime_test", &serde_json::json!({}));

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or(&result.text_result_for_llm)
                .contains("cancelled")
        );
    }

    #[test]
    fn board_prompt_submits_flowscript_before_database_setup() {
        let prompt = flow_like::copilot::prompts::board_sdk_flowscript_system_prompt("", 0);
        assert!(prompt.contains("database setup is\nnever a prerequisite"));
        assert!(
            prompt.contains("submit the full-shape board through `write_flowscript` immediately")
        );
        assert!(prompt.contains("ONE bounded, focused `get_declarations`"));
        assert!(prompt.contains("One such result proves the capability mismatch"));
        assert!(prompt.contains(
            "Record any remaining requested schemas as pending and finish/apply the board"
        ));
    }

    #[test]
    fn external_workflow_prompt_requires_an_early_retained_checkpoint() {
        let prompt = build_external_agent_prompt("system", "build it", true);
        assert!(prompt.contains("ONE bounded, focused get_declarations batch"));
        assert!(prompt.contains("call write_flowscript IMMEDIATELY"));
        assert!(prompt.contains("at most six ancillary"));
        assert!(prompt.contains("It may retain compiler diagnostics"));
        assert!(!prompt.contains("every required catalog-signature search"));
    }

    #[test]
    fn predraft_checkpoint_watchdog_arms_until_a_source_operation_starts() {
        let mut state = WorkflowToolLoopState::default();
        assert!(!workflow_waiting_for_initial_source_checkpoint(&state));
        state.initial_declaration_lookup_usable = true;
        assert!(workflow_waiting_for_initial_source_checkpoint(&state));
        // An unrelated position/comment operation must not permanently disarm the source
        // checkpoint. Source/typed operation counters are the authoritative transition.
        state.edit_in_flight = true;
        assert!(workflow_waiting_for_initial_source_checkpoint(&state));
        state.flowscript_operation_attempts = 1;
        assert!(!workflow_waiting_for_initial_source_checkpoint(&state));
    }

    #[test]
    fn continuation_writes_after_partial_but_usable_declaration_coverage() {
        let snapshot = WorkflowToolLoopSnapshot {
            last_declarations: Some(
                "declare function emailImapConnect({ host: string }): (connection: Struct);"
                    .to_string(),
            ),
            declaration_lookup_complete: false,
            unresolved_declaration_queries: vec!["smtp send".to_string()],
            ..Default::default()
        };
        let prompt =
            build_external_workflow_continuation_prompt("build support mail", Some(&snapshot), 1);
        assert!(prompt.contains("DECLARATIONS ALREADY FETCHED"));
        assert!(prompt.contains("call write_flowscript immediately"));
        assert!(prompt.contains("last status: declarations_ready_no_source"));
        assert!(!prompt.contains("UNRESOLVED DECLARATION COVERAGE"));
        let error = external_workflow_incomplete_error(Some(&snapshot), 0);
        assert!(error.contains("last status: declarations_ready_no_source"));
    }

    #[test]
    fn retained_agent_text_is_bounded_and_utf8_safe() {
        let mut retained = String::new();
        assert!(append_bounded_text(&mut retained, "hello", 32));
        assert!(!append_bounded_text(&mut retained, &"🦀".repeat(32), 32));
        assert!(retained.len() <= 32);
        assert!(std::str::from_utf8(retained.as_bytes()).is_ok());
        let capped = retained.clone();
        assert!(!append_bounded_text(&mut retained, "ignored", 32));
        assert_eq!(retained, capped);
    }

    #[test]
    fn resumable_global_chat_buffer_has_hard_bounds() {
        let mut buffer = GlobalChatRunBuffer::default();
        for _ in 0..(GLOBAL_CHAT_RUN_MAX_CHUNKS + 10) {
            buffer.push("x");
        }
        assert!(buffer.truncated);
        assert!(buffer.chunks.len() <= GLOBAL_CHAT_RUN_MAX_CHUNKS);
        assert!(buffer.bytes <= GLOBAL_CHAT_RUN_MAX_BUFFER_BYTES);
    }

    #[test]
    fn registered_agent_run_can_be_cancelled_and_is_removed_by_guard() {
        let request_id = format!("native-cancel-test-{}", uuid::Uuid::new_v4());
        let (token, guard) = register_copilot_run(Some(&request_id));
        assert!(!token.is_cancelled());
        assert_eq!(cancel_copilot_chat(request_id.clone()), Ok(true));
        assert!(token.is_cancelled());
        drop(guard);
        assert!(!ACTIVE_COPILOT_RUNS.contains_key(&request_id));
        assert_eq!(cancel_copilot_chat(request_id), Ok(false));
    }

    fn build_test_client() -> Option<Client> {
        let cli_path = find_copilot_cli_path();
        if cli_path.is_none() {
            eprintln!("SKIP: copilot CLI not found");
            return None;
        }

        let mut builder = Client::builder().use_stdio(true).log_level(LogLevel::Error);

        if let Some(path) = cli_path {
            builder = builder.cli_path(path);
        }
        builder = builder.env("PATH", augmented_path());

        Some(builder.build().expect("Client::builder().build() failed"))
    }

    async fn start_test_client() -> Option<Client> {
        let client = build_test_client()?;
        match client.start().await {
            Ok(()) => Some(client),
            Err(e) => {
                let err_str = format!("{:?}", e);
                if err_str.contains("ProtocolMismatch") {
                    eprintln!(
                        "SKIP: protocol mismatch — SDK expects v{}, CLI reports v3. \
                         Update copilot-sdk dependency.",
                        copilot_sdk::SDK_PROTOCOL_VERSION
                    );
                } else {
                    eprintln!("SKIP: client.start() failed: {}", err_str);
                }
                None
            }
        }
    }

    #[test]
    fn workflow_edit_classifier_allows_read_only_text_answers() {
        for prompt in [
            "explain why this node is not connected to the API Call",
            "what does this FlowScript do?",
            "check if the workflow execution wiring is correct",
            "debug why the For Each loop is not working",
        ] {
            assert!(
                !is_workflow_edit_request(prompt),
                "prompt should stay read-only: {prompt}"
            );
        }
    }

    #[test]
    fn workflow_edit_classifier_still_detects_mutations() {
        for prompt in [
            "generate a workflow that fetches the Rust RSS feed",
            "connect the API Call success output to To Text",
            "fix the workflow execution wiring",
            "update this flow to store rows in the database",
            "Bau mir eine App mit IMAP und SMTP für Support-Emails",
            "Erstelle einen Cron-Ablauf und speichere die Ergebnisse in der Datenbank",
            "create a cron event",
            "configure a scheduled trigger",
            "Konfiguriere einen Zeitplan-Auslöser",
        ] {
            assert!(
                is_workflow_edit_request(prompt),
                "prompt should be treated as workflow edit: {prompt}"
            );
        }
    }

    #[test]
    fn workflow_edit_classifier_keeps_german_explanations_read_only() {
        for prompt in [
            "Erkläre mir diesen Workflow",
            "Warum ist dieser Knoten nicht verbunden?",
            "Prüfe, warum der Email-Ablauf einen Fehler zeigt",
        ] {
            assert!(
                !is_workflow_edit_request(prompt),
                "German explanation should stay read-only: {prompt}"
            );
        }
    }

    #[test]
    fn host_unified_wrapper_does_not_turn_raw_ui_request_into_workflow_edit() {
        let raw = "Create a settings page with a form and explain the current button labels";
        let wrapped =
            format!("[UNIFIED MODE - generate workflow nodes and UI components.]\n\n{raw}");

        assert!(is_workflow_edit_request(&wrapped));
        assert!(!is_workflow_edit_request(raw));
    }

    #[test]
    fn external_workflow_tool_budget_forces_edit_and_retry() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));

        assert!(workflow_tool_preflight(&state, "get_current_flowscript").is_none());
        assert_eq!(
            workflow_tool_preflight(&state, "get_current_flowscript")
                .and_then(|result| result.is_error),
            Some(true),
            "the live document may only be fetched once"
        );
        assert!(
            workflow_tool_preflight_with_args(
                &state,
                "get_declarations",
                &serde_json::json!({ "queries": ["log information"] }),
            )
            .is_none()
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &serde_json::json!({ "queries": ["log information"] }),
            "declare function logInfo({ message: string }): void;",
        );
        assert_eq!(
            workflow_tool_preflight(&state, "get_declarations").and_then(|result| result.is_error),
            Some(false),
            "a second declaration call is a successful redirect to editing, not a provider error"
        );
        assert_eq!(
            workflow_tool_preflight(&state, "catalog_search").and_then(|result| result.is_error),
            Some(true),
            "legacy catalog discovery is never part of a FlowScript mutation loop"
        );

        assert!(workflow_tool_preflight(&state, "edit_flowscript").is_none());
        assert_eq!(
            workflow_tool_preflight(&state, "edit_flowscript").and_then(|result| result.is_error),
            Some(true),
            "parallel FlowScript drafts must be serialized"
        );
        workflow_tool_record(
            &state,
            "edit_flowscript",
            &serde_json::json!({ "flowscript": "eventsSimple() { brokenCall() }" }),
            &serde_json::json!({
                "status": "validation_errors",
                "errors": ["brokenCall does not match a catalog declaration"]
            })
            .to_string(),
        );
        let repair_lookup = serde_json::json!({ "queries": ["brokenCall"] });
        assert!(
            workflow_tool_preflight_with_args(&state, "get_declarations", &repair_lookup).is_none(),
            "one targeted declaration lookup is available for a repair"
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &repair_lookup,
            "declare function brokenCall({}): void;",
        );
        assert!(workflow_tool_preflight(&state, "edit_flowscript").is_none());
        workflow_tool_record(
            &state,
            "edit_flowscript",
            &serde_json::json!({ "flowscript": "eventsSimple() { log({ text: \"ok\" }) }" }),
            &serde_json::json!({ "status": "queued", "queued_count": 2 }).to_string(),
        );
        let terminal = workflow_tool_preflight(&state, "get_declarations")
            .expect("queued state returns an explicit terminal result");
        assert_eq!(terminal.is_error, Some(false));
        assert!(
            workflow_tool_preflight(&state, "emit_ui").is_none(),
            "a combined board + UI request may finish its UI after the workflow queues"
        );
        assert!(state.lock().expect("state lock").queued);
    }

    #[test]
    fn first_flowscript_write_requires_one_live_declaration_batch() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let write_args = serde_json::json!({
            "draft_id": "declaration-gated-source",
            "source": "eventsSimple() { logInfo({ message: \"hello\" }) }"
        });

        let redirected = workflow_tool_preflight_with_args(&state, "write_flowscript", &write_args)
            .expect("the first write must be redirected to live declaration discovery");
        assert_eq!(redirected.is_error, Some(false));
        let redirected = redirected
            .content
            .iter()
            .filter_map(|content| match &content.raw {
                rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(redirected.contains("declaration_lookup_required"));
        assert!(redirected.contains("repeated-input"));
        assert!(!state.lock().expect("state lock").edit_in_flight);

        let empty_lookup = workflow_tool_preflight_with_args(
            &state,
            "get_declarations",
            &serde_json::json!({ "queries": ["   "] }),
        )
        .expect("an empty initial declaration batch must be rejected");
        let empty_lookup = empty_lookup
            .content
            .iter()
            .filter_map(|content| match &content.raw {
                rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(empty_lookup.contains("declaration_batch_required"));

        assert!(
            workflow_tool_preflight_with_args(
                &state,
                "get_declarations",
                &serde_json::json!({ "queries": ["log information"] }),
            )
            .is_none()
        );
        assert!(
            workflow_tool_preflight_with_args(&state, "write_flowscript", &write_args).is_some(),
            "dispatching a lookup is not enough; its usable result must be retained"
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &serde_json::json!({ "queries": ["log information"] }),
            "No FlowScript declarations matched this query.",
        );
        let guard = state.lock().expect("state lock");
        assert_eq!(guard.declaration_calls, 1);
        assert_eq!(guard.initial_declaration_attempts, 1);
        assert!(!guard.initial_declaration_lookup_complete);
        drop(guard);
        assert!(
            workflow_tool_preflight_with_args(
                &state,
                "get_declarations",
                &serde_json::json!({ "queries": ["log information"] }),
            )
            .is_none(),
            "a no-match initial lookup leaves the focused initial lookup available"
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &serde_json::json!({ "queries": ["log information"] }),
            "declare function logInfo({ message: string }): void;  // impure",
        );
        assert!(
            workflow_tool_preflight_with_args(&state, "write_flowscript", &write_args).is_none(),
            "the exact same source write is dispatched after a usable declaration result"
        );
    }

    #[test]
    fn partial_declaration_coverage_retains_matches_and_unlocks_first_source_checkpoint() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let initial = serde_json::json!({ "queries": ["imap receive", "smtp send"] });
        assert!(workflow_tool_preflight_with_args(&state, "get_declarations", &initial).is_none());
        workflow_tool_record(
            &state,
            "get_declarations",
            &initial,
            concat!(
                "// flowpilot.declaration-batch/v1 {\"processed_count\":2,\"matched_count\":1,\"matched_queries\":[\"imap receive\"],\"unmatched_count\":1,\"unmatched_queries\":[\"smtp send\"],\"complete\":false,\"omitted_count\":0,\"omitted_queries\":[],\"truncated_query_count\":0}\n",
                "declare function emailImapConnect({ host: string }): (connection: Struct);"
            ),
        );
        {
            let guard = state.lock().expect("state lock");
            assert!(guard.initial_declaration_lookup_usable);
            assert!(!guard.initial_declaration_lookup_complete);
            assert_eq!(guard.unresolved_declaration_queries, ["smtp send"]);
            assert!(
                guard
                    .last_declarations
                    .as_deref()
                    .is_some_and(|declarations| { declarations.contains("emailImapConnect") })
            );
        }
        let write_args = serde_json::json!({
            "draft_id": "coverage-gated",
            "source": "eventsSimple() {}"
        });
        let unrelated = serde_json::json!({ "queries": ["string replace"] });
        let rejected = workflow_tool_preflight_with_args(&state, "get_declarations", &unrelated)
            .expect("a second pre-draft lookup must redirect to the retained source checkpoint");
        let rejected = workflow_call_result_json(&rejected);
        assert_eq!(rejected["status"], "discovery_budget_exhausted");
        assert_eq!(rejected["next_action"], "write_or_patch_flowscript");
        assert!(
            rejected["message"]
                .as_str()
                .is_some_and(|message| message.contains("Do not chase omitted or unmatched"))
        );
        {
            let guard = state.lock().expect("state lock");
            assert!(!guard.initial_declaration_lookup_complete);
            assert_eq!(guard.unresolved_declaration_queries, ["smtp send"]);
            assert_eq!(guard.initial_declaration_attempts, 1);
        }
        assert!(
            workflow_tool_preflight_with_args(&state, "write_flowscript", &write_args).is_none(),
            "one usable live signature must unlock a recoverable full-shape source checkpoint"
        );
    }

    #[test]
    fn focused_rephrasing_can_resolve_an_unmatched_declaration_query() {
        assert!(declaration_queries_are_related(
            "smtp send approval response",
            "smtp send email"
        ));
        assert!(!declaration_queries_are_related(
            "smtp send email",
            "smtp receive email"
        ));
        assert!(!declaration_queries_are_related(
            "smtp send email",
            "imap receive email"
        ));

        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let initial = serde_json::json!({ "queries": ["smtp send approval response"] });
        assert!(workflow_tool_preflight_with_args(&state, "get_declarations", &initial).is_none());
        workflow_tool_record(
            &state,
            "get_declarations",
            &initial,
            "// flowpilot.declaration-batch/v1 {\"processed_count\":1,\"matched_count\":0,\"matched_queries\":[],\"unmatched_count\":1,\"unmatched_queries\":[\"smtp send approval response\"],\"complete\":false,\"omitted_count\":0,\"omitted_queries\":[],\"truncated_query_count\":0}\nNo declaration matched.",
        );

        let rephrased = serde_json::json!({ "queries": ["smtp send email"] });
        assert!(
            workflow_tool_preflight_with_args(&state, "get_declarations", &rephrased).is_none(),
            "a rephrasing that retains the distinctive smtp capability may repair the miss"
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &rephrased,
            concat!(
                "// flowpilot.declaration-batch/v1 {\"processed_count\":1,\"matched_count\":1,\"matched_queries\":[\"smtp send email\"],\"unmatched_count\":0,\"unmatched_queries\":[],\"complete\":true,\"omitted_count\":0,\"omitted_queries\":[],\"truncated_query_count\":0}\n",
                "declare function emailSmtpSend({ to: string, bodyText: string }): void;"
            ),
        );
        let guard = state.lock().expect("state lock");
        assert!(guard.initial_declaration_lookup_complete);
        assert!(guard.unresolved_declaration_queries.is_empty());
    }

    #[test]
    fn source_lifecycle_errors_cannot_bypass_initial_declaration_coverage() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let patch_args = serde_json::json!({
            "draft_id": "not-retained",
            "expected_revision": 0,
            "old_text": "before",
            "new_text": "after"
        });
        let rejected = workflow_tool_preflight_with_args(&state, "patch_flowscript", &patch_args)
            .expect("patch cannot create an unretained draft");
        let payload = workflow_call_result_json(&rejected);
        assert_eq!(payload["code"], "FLOWSCRIPT_DRAFT_REQUIRED");
        assert_eq!(payload["next_action"], "get_declarations");

        // Even a stale/legacy worker result cannot turn an arbitrary error status into declaration
        // authorization. This protects old in-flight calls across an app upgrade as well.
        workflow_tool_record(
            &state,
            "patch_flowscript",
            &patch_args,
            &serde_json::json!({
                "status": "error",
                "code": "FLOWSCRIPT_DRAFT_NOT_FOUND",
                "message": "draft missing"
            })
            .to_string(),
        );
        let write = workflow_tool_preflight_with_args(
            &state,
            "write_flowscript",
            &serde_json::json!({
                "draft_id": "still-gated",
                "source": "eventsSimple() {}"
            }),
        )
        .expect("an incidental lifecycle error must not unlock source generation");
        let payload = workflow_call_result_json(&write);
        assert_eq!(payload["status"], "declaration_lookup_required");
    }

    #[test]
    fn request_identity_mismatch_does_not_adopt_rejected_source_coordinates() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
            initial_declaration_lookup_complete: true,
            ..Default::default()
        }));
        let rejected_args = serde_json::json!({
            "draft_id": "foreign-draft",
            "source": "eventsSimple() { logInfo({ message: \"must not retain\" }) }"
        });
        assert!(
            workflow_tool_preflight_with_args(&state, "write_flowscript", &rejected_args).is_none()
        );
        let mismatch = serde_json::json!({
            "status": "request_identity_mismatch",
            "code": "FLOWSCRIPT_DRAFT_REQUEST_IDENTITY_MISMATCH",
            "message": "This FlowScript draft belongs to a different immutable user request."
        })
        .to_string();
        workflow_tool_record(&state, "write_flowscript", &rejected_args, &mismatch);

        {
            let state = state.lock().expect("state lock");
            assert!(!state.flowscript_draft_retained);
            assert!(state.flowscript_draft_id.is_none());
            assert!(state.flowscript_revision.is_none());
            assert!(state.last_flowscript.is_none());
            assert_eq!(
                state.last_status.as_deref(),
                Some("request_identity_mismatch")
            );
        }
        assert!(flowpilot_tool_result_is_error(
            &copilot_sdk::ToolResultObject::text(mismatch)
        ));
        assert!(
            workflow_tool_preflight_with_args(
                &state,
                "write_flowscript",
                &serde_json::json!({
                    "draft_id": "current-request-draft",
                    "source": "eventsSimple() {}"
                }),
            )
            .is_none(),
            "a distinct draft id for the current request remains available"
        );
    }

    #[test]
    fn typed_request_mismatch_does_not_adopt_rejected_coordinates() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
            typed_draft_id: Some("current-draft".to_string()),
            typed_draft_retained: true,
            typed_revision: Some(4),
            ..Default::default()
        }));
        let rejected_args = serde_json::json!({
            "draft_id": "foreign-draft",
            "expected_revision": 99,
            "modules": []
        });
        // Include coordinates defensively: the core now omits them, but a stale/custom provider
        // response must not be able to poison the host-owned loop state either.
        let mismatch = serde_json::json!({
            "status": "request_identity_mismatch",
            "code": "IR_DRAFT_REQUEST_IDENTITY_MISMATCH",
            "draft_id": "foreign-draft",
            "revision": 99,
            "message": "This typed draft belongs to a different immutable user request."
        })
        .to_string();
        workflow_tool_record(&state, "validate_flow_ir_draft", &rejected_args, &mismatch);

        let state = state.lock().expect("state lock");
        assert_eq!(state.typed_draft_id.as_deref(), Some("current-draft"));
        assert_eq!(state.typed_revision, Some(4));
        assert!(state.typed_draft_retained);
        assert_eq!(
            state.last_status.as_deref(),
            Some("request_identity_mismatch")
        );
    }

    #[test]
    fn missing_or_stale_base_releases_unusable_draft_coordinates_for_restart() {
        for (code, include_source) in [
            ("FLOWSCRIPT_DRAFT_MISSING", false),
            ("FLOWSCRIPT_BASE_REVISION_CONFLICT", true),
        ] {
            let old_source = "eventsSimple() { logInfo({ message: \"preserve me\" }) }";
            let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
                initial_declaration_lookup_complete: true,
                mutation_path: Some(WorkflowMutationPath::FlowScript),
                flowscript_draft_id: Some("expired-draft".to_string()),
                flowscript_draft_retained: true,
                flowscript_revision: Some(4),
                last_flowscript: Some(old_source.to_string()),
                last_status: Some("valid".to_string()),
                ..Default::default()
            }));
            let check_args = serde_json::json!({
                "draft_id": "expired-draft",
                "expected_revision": 4
            });
            assert!(
                workflow_tool_preflight_with_args(&state, "check_flowscript", &check_args)
                    .is_none()
            );
            let mut result = serde_json::json!({
                "status": "error",
                "code": code,
                "message": "the retained coordinates can no longer be continued"
            });
            if include_source {
                result["source"] = serde_json::json!(old_source);
            }
            workflow_tool_record(&state, "check_flowscript", &check_args, &result.to_string());

            {
                let state = state.lock().expect("state lock");
                assert!(!state.flowscript_draft_retained, "{code}");
                assert!(state.flowscript_draft_id.is_none(), "{code}");
                assert!(state.flowscript_revision.is_none(), "{code}");
                assert_eq!(state.last_flowscript.as_deref(), Some(old_source), "{code}");
            }
            assert!(
                workflow_tool_preflight_with_args(
                    &state,
                    "write_flowscript",
                    &serde_json::json!({
                        "draft_id": format!("fresh-{code}"),
                        "source": old_source
                    }),
                )
                .is_none(),
                "{code} must allow a fresh host-authorized draft id"
            );
        }
    }

    #[test]
    fn incomplete_declaration_coverage_stops_after_bounded_attempts() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let args = serde_json::json!({ "queries": ["unavailable capability"] });
        let no_match = "// flowpilot.declaration-batch/v1 {\"processed_count\":1,\"matched_count\":0,\"matched_queries\":[],\"unmatched_count\":1,\"unmatched_queries\":[\"unavailable capability\"],\"complete\":false,\"omitted_count\":0,\"omitted_queries\":[],\"truncated_query_count\":0}\nNo FlowScript declarations matched this query.";
        for _ in 0..MAX_INITIAL_DECLARATION_ATTEMPTS {
            assert!(workflow_tool_preflight_with_args(&state, "get_declarations", &args).is_none());
            workflow_tool_record(&state, "get_declarations", &args, no_match);
        }
        let rejected = workflow_tool_preflight_with_args(
            &state,
            "write_flowscript",
            &serde_json::json!({
                "draft_id": "must-not-start",
                "source": "eventsSimple() {}"
            }),
        )
        .expect("guessing source after incomplete coverage must stop locally");
        let payload = workflow_call_result_json(&rejected);
        assert_eq!(payload["code"], "DECLARATION_COVERAGE_EXHAUSTED");
        assert_eq!(payload["attempts"], MAX_INITIAL_DECLARATION_ATTEMPTS);
        assert_eq!(payload["unresolved_queries"][0], "unavailable capability");
    }

    #[test]
    fn aborted_initial_declaration_lookup_releases_the_required_slot() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let args = serde_json::json!({ "queries": ["imap inbox messages"] });

        assert!(workflow_tool_preflight_with_args(&state, "get_declarations", &args).is_none());
        assert_eq!(state.lock().expect("state lock").declaration_calls, 1);

        workflow_tool_abort(&state, "get_declarations", "worker disconnected");
        let guard = state.lock().expect("state lock");
        assert_eq!(guard.declaration_calls, 0);
        assert_eq!(guard.declarations_since_edit, 0);
        drop(guard);

        assert!(
            workflow_tool_preflight_with_args(&state, "get_declarations", &args).is_none(),
            "the focused initial lookup can be retried after a worker abort"
        );
    }

    #[test]
    fn declaration_lookup_lease_blocks_parallel_lookup_and_source_dispatch() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let args = serde_json::json!({ "queries": ["imap inbox messages"] });
        assert!(workflow_tool_preflight_with_args(&state, "get_declarations", &args).is_none());

        let duplicate = workflow_tool_preflight_with_args(&state, "get_declarations", &args)
            .expect("a second lookup must not dispatch while the first result is pending");
        assert_eq!(
            workflow_call_result_json(&duplicate)["code"],
            "DECLARATION_LOOKUP_IN_FLIGHT"
        );
        let write = workflow_tool_preflight_with_args(
            &state,
            "write_flowscript",
            &serde_json::json!({
                "draft_id": "must-wait",
                "source": "eventsSimple() {}"
            }),
        )
        .expect("source authoring must wait for declaration authority");
        assert_eq!(
            workflow_call_result_json(&write)["code"],
            "DECLARATION_LOOKUP_IN_FLIGHT"
        );
        assert_eq!(state.lock().expect("state lock").declaration_calls, 1);

        workflow_tool_abort(&state, "get_declarations", "worker disconnected");
        assert!(workflow_tool_preflight_with_args(&state, "get_declarations", &args).is_none());
    }

    #[test]
    fn poisoned_workflow_state_fails_closed_before_tool_dispatch() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let poisoned = state.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = poisoned.lock().expect("initial lock");
            panic!("poison workflow loop state for regression coverage");
        });

        let rejected = workflow_tool_preflight(&state, "get_current_flowscript")
            .expect("a poisoned lifecycle state must return a terminal host error");
        assert_eq!(rejected.is_error, Some(true));
        assert_eq!(
            workflow_call_result_json(&rejected)["code"],
            "WORKFLOW_LOOP_STATE_UNAVAILABLE"
        );
        assert!(workflow_state_has_retained_candidate(Some(&state)));
    }

    #[test]
    fn declaration_headers_cannot_claim_complete_without_exact_bodies() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let args = serde_json::json!({ "queries": ["smtp send"] });
        assert!(workflow_tool_preflight_with_args(&state, "get_declarations", &args).is_none());
        workflow_tool_record(
            &state,
            "get_declarations",
            &args,
            "// flowpilot.declaration-batch/v1 {\"processed_count\":1,\"matched_count\":1,\"matched_queries\":[\"smtp send\"],\"unmatched_count\":0,\"unmatched_queries\":[],\"output_omitted_count\":0,\"output_omitted_queries\":[],\"complete\":true,\"omitted_count\":0,\"omitted_queries\":[],\"truncated_query_count\":0}\n// declaration body was lost",
        );
        {
            let state = state.lock().expect("state lock");
            assert!(!state.initial_declaration_lookup_complete);
            assert_eq!(state.unresolved_declaration_queries, ["smtp send"]);
        }
        assert!(
            workflow_tool_preflight_with_args(&state, "get_declarations", &args).is_none(),
            "the exact omitted capability gets one bounded focused retry"
        );
    }

    #[test]
    fn metadata_less_multi_query_results_do_not_unlock_source() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let args = serde_json::json!({ "queries": ["imap receive", "smtp send"] });
        assert!(workflow_tool_preflight_with_args(&state, "get_declarations", &args).is_none());
        workflow_tool_record(
            &state,
            "get_declarations",
            &args,
            "declare function emailImapConnect({ host: string }): Struct;",
        );
        let state = state.lock().expect("state lock");
        assert!(!state.initial_declaration_lookup_complete);
        assert_eq!(
            state.unresolved_declaration_queries,
            ["imap receive", "smtp send"]
        );
    }

    #[test]
    fn mixed_unnamed_declaration_omissions_are_counted_per_category() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let output_queries = (0..10)
            .map(|index| format!("output omitted {index}"))
            .collect::<Vec<_>>();
        let mut all_queries = output_queries.clone();
        all_queries.extend((0..5).map(|index| format!("input omitted {index}")));
        let args = serde_json::json!({ "queries": all_queries });
        assert!(workflow_tool_preflight_with_args(&state, "get_declarations", &args).is_none());
        let header = format!(
            "// flowpilot.declaration-batch/v1 {}\nNo exact declaration body fit.",
            serde_json::json!({
                "processed_count": 10,
                "matched_count": 0,
                "matched_queries": [],
                "unmatched_count": 0,
                "unmatched_queries": [],
                "output_omitted_count": 10,
                "output_omitted_queries": output_queries,
                "complete": false,
                "omitted_count": 5,
                "omitted_queries": [],
                "query_names_omitted_for_size": true,
                "truncated_query_count": 0
            })
        );
        workflow_tool_record(&state, "get_declarations", &args, &header);

        let state = state.lock().expect("state lock");
        assert_eq!(state.unresolved_declaration_queries.len(), 11);
        assert!(
            state
                .unresolved_declaration_queries
                .iter()
                .any(|query| { query.contains("5 additional omitted declaration query") })
        );
    }

    #[test]
    fn declaration_retention_prioritizes_complete_new_signatures_without_partial_lines() {
        let older = format!(
            "declare function oldCapability({{ input: string }}): string;\n{}",
            "old documentation ".repeat(2_000)
        );
        let newest = concat!(
            "// flowpilot.declaration-batch/v1 {}\n",
            "declare function emailSmtpConnect({ host: string, port: int }): Struct;\n",
            "declare function emailSmtpSend({ connection: Struct, to: string, bodyText: string }): void;\n",
            "// Usage: connect first, then pass the returned connection into the send call."
        );
        let retained = retain_declaration_result(Some(&older), newest);
        assert!(retained.contains("oldCapability"));
        assert!(retained.contains("emailSmtpConnect"));
        assert!(retained.contains("emailSmtpSend"));
        assert!(retained.contains("connect first"));
        assert!(!retained.contains("old documentation"));
        assert!(!retained.contains("…"));
    }

    #[test]
    fn typed_workflow_path_retains_revision_and_cannot_mix_raw_edits() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));

        assert!(workflow_tool_preflight(&state, "plan_flow_ir").is_none());
        assert!(workflow_tool_preflight(&state, "begin_flow_ir_draft").is_none());
        workflow_tool_record(
            &state,
            "begin_flow_ir_draft",
            &serde_json::json!({ "draft_id": "support-agent" }),
            &serde_json::json!({
                "status": "draft_started",
                "draft_id": "support-agent",
                "revision": 0,
                "diagnostics": []
            })
            .to_string(),
        );
        assert!(workflow_tool_preflight(&state, "upsert_flow_ir_module").is_none());
        workflow_tool_record(
            &state,
            "upsert_flow_ir_module",
            &serde_json::json!({ "draft_id": "support-agent", "expected_revision": 0 }),
            &serde_json::json!({
                "status": "module_needs_repair",
                "draft_id": "support-agent",
                "revision": 1,
                "diagnostics": [{
                    "code": "IR_INPUT_TYPE",
                    "message": "Generic output requires an explicit conversion"
                }]
            })
            .to_string(),
        );

        let guard = state.lock().expect("state lock");
        assert_eq!(guard.typed_draft_id.as_deref(), Some("support-agent"));
        assert_eq!(guard.typed_revision, Some(1));
        assert_eq!(guard.mutation_path, Some(WorkflowMutationPath::TypedIr));
        assert!(guard.last_errors[0].contains("IR_INPUT_TYPE"));
        drop(guard);

        let conflict = workflow_tool_preflight(&state, "edit_flowscript")
            .expect("raw mutation must be blocked after a typed draft starts");
        assert_eq!(conflict.is_error, Some(true));

        let snapshot = state.lock().expect("state lock").snapshot();
        let prompt = build_external_workflow_continuation_prompt(
            "build the support workflow",
            Some(&snapshot),
            1,
        );
        assert!(prompt.contains("draft_id=support-agent"));
        assert!(prompt.contains("latest revision=1"));
        assert!(prompt.contains("do not edit generated FlowScript text"));
    }

    #[test]
    fn typed_ir_budget_counts_every_dispatched_phase() {
        assert_eq!(
            typed_ir_operation_budget(0),
            MIN_EXTERNAL_TYPED_IR_OPERATION_BUDGET
        );
        assert_eq!(
            typed_ir_operation_budget(usize::MAX),
            MAX_EXTERNAL_TYPED_IR_OPERATION_BUDGET
        );
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let calls = [
            (
                "plan_flow_ir",
                serde_json::json!({ "modules": [{ "name": "classify" }, { "name": "eventsSimple" }] }),
                serde_json::json!({ "feasible": true, "requirements": [] }),
            ),
            (
                "begin_flow_ir_draft",
                serde_json::json!({
                    "draft_id": "typed-counts",
                    "expected_modules": ["classify", "eventsSimple"]
                }),
                serde_json::json!({
                    "status": "draft_started",
                    "draft_id": "typed-counts",
                    "revision": 0,
                    "missing_modules": ["classify", "eventsSimple"]
                }),
            ),
            (
                "update_flow_ir_draft",
                serde_json::json!({ "draft_id": "typed-counts", "expected_revision": 0 }),
                serde_json::json!({
                    "status": "draft_updated",
                    "draft_id": "typed-counts",
                    "revision": 1
                }),
            ),
            (
                "upsert_flow_ir_module",
                serde_json::json!({
                    "draft_id": "typed-counts",
                    "expected_revision": 1,
                    "module": { "kind": "function", "name": "classify", "steps": [] }
                }),
                serde_json::json!({
                    "status": "module_validated",
                    "draft_id": "typed-counts",
                    "revision": 2,
                    "missing_modules": ["eventsSimple"]
                }),
            ),
            (
                "validate_flow_ir_draft",
                serde_json::json!({ "draft_id": "typed-counts" }),
                serde_json::json!({
                    "status": "draft_valid",
                    "draft_id": "typed-counts",
                    "revision": 2
                }),
            ),
            (
                "commit_flow_ir_draft",
                serde_json::json!({ "draft_id": "typed-counts", "expected_revision": 2 }),
                serde_json::json!({
                    "status": "queued",
                    "draft_id": "typed-counts",
                    "revision": 2
                }),
            ),
        ];

        for (tool, args, result) in calls {
            assert!(
                workflow_tool_preflight_with_args(&state, tool, &args).is_none(),
                "{tool} should be dispatched"
            );
            workflow_tool_record(&state, tool, &args, &result.to_string());
        }

        let state = state.lock().expect("state lock");
        assert_eq!(state.typed_operation_attempts, 6);
        assert_eq!(state.typed_expected_modules, 2);
        assert!(state.queued);
    }

    #[test]
    fn typed_ir_module_scaled_budget_returns_recoverable_draft_state() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let begin_args = serde_json::json!({
            "draft_id": "support-timeout-recovery",
            "expected_modules": (0..10)
                .map(|index| format!("module_{index}"))
                .collect::<Vec<_>>()
        });
        assert!(
            workflow_tool_preflight_with_args(&state, "begin_flow_ir_draft", &begin_args).is_none()
        );
        workflow_tool_record(
            &state,
            "begin_flow_ir_draft",
            &begin_args,
            &serde_json::json!({
                "status": "draft_started",
                "draft_id": "support-timeout-recovery",
                "revision": 0
            })
            .to_string(),
        );

        let budget = typed_ir_operation_budget(10);
        for attempt in 1..budget {
            let args = serde_json::json!({ "draft_id": "support-timeout-recovery" });
            assert!(
                workflow_tool_preflight_with_args(&state, "validate_flow_ir_draft", &args)
                    .is_none(),
                "operation {attempt} should fit the module-scaled budget"
            );
            workflow_tool_record(
                &state,
                "validate_flow_ir_draft",
                &args,
                &serde_json::json!({
                    "status": "validation_errors",
                    "draft_id": "support-timeout-recovery",
                    "revision": attempt,
                    "diagnostics": [{
                        "code": "IR_REMAINING",
                        "message": format!("remaining repair {attempt}")
                    }],
                    "missing_modules": ["send_reply"]
                })
                .to_string(),
            );
        }

        let rejected = workflow_tool_preflight_with_args(
            &state,
            "commit_flow_ir_draft",
            &serde_json::json!({
                "draft_id": "support-timeout-recovery",
                "expected_revision": budget - 1
            }),
        )
        .expect("the operation after the hard cap must stop locally");
        assert_eq!(rejected.is_error, Some(true));
        let payload = workflow_call_result_json(&rejected);
        assert_eq!(payload["status"], "typed_repair_budget_exhausted");
        assert_eq!(payload["code"], "TYPED_IR_OPERATION_BUDGET_EXHAUSTED");
        assert_eq!(payload["draft_retained"], true);
        assert_eq!(payload["draft_id"], "support-timeout-recovery");
        assert_eq!(payload["revision"], u64::from(budget - 1));
        assert_eq!(payload["operation_attempts"], u64::from(budget));
        assert_eq!(payload["operation_budget"], u64::from(budget));
        assert_eq!(
            payload["missing_modules"],
            serde_json::json!(["send_reply"])
        );
        assert!(
            payload["remaining_diagnostics"][0]
                .as_str()
                .is_some_and(|diagnostic| diagnostic.contains("IR_REMAINING"))
        );
        assert_eq!(
            state.lock().expect("state lock").typed_operation_attempts,
            budget
        );
        let redirected = workflow_tool_preflight(&state, "get_current_flowscript")
            .expect("all workflow-loop tools stay terminal after typed budget exhaustion");
        assert_eq!(
            workflow_call_result_json(&redirected)["status"],
            "typed_repair_budget_exhausted"
        );
    }

    #[test]
    fn typed_ir_loop_stops_repeated_identical_module_diagnostics() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let begin_args = serde_json::json!({
            "draft_id": "stalled-module",
            "expected_modules": ["classify"]
        });
        assert!(
            workflow_tool_preflight_with_args(&state, "begin_flow_ir_draft", &begin_args).is_none()
        );
        workflow_tool_record(
            &state,
            "begin_flow_ir_draft",
            &begin_args,
            &serde_json::json!({
                "status": "draft_started",
                "draft_id": "stalled-module",
                "revision": 0
            })
            .to_string(),
        );

        for revision in 1..=u64::from(MAX_EXTERNAL_TYPED_IR_STALLED_ATTEMPTS) + 1 {
            let args = serde_json::json!({
                "draft_id": "stalled-module",
                "expected_revision": revision - 1,
                "module": { "kind": "function", "name": "classify", "steps": [] }
            });
            assert!(
                workflow_tool_preflight_with_args(&state, "upsert_flow_ir_module", &args).is_none()
            );
            workflow_tool_record(
                &state,
                "upsert_flow_ir_module",
                &args,
                &serde_json::json!({
                    "status": "module_needs_repair",
                    "draft_id": "stalled-module",
                    "revision": revision,
                    "diagnostics": [{
                        "code": "IR_INPUT_TYPE",
                        "message": "the exact same conversion is still missing"
                    }]
                })
                .to_string(),
            );
        }

        let attempts_before_rejection = state.lock().expect("state lock").typed_operation_attempts;
        let rejected = workflow_tool_preflight_with_args(
            &state,
            "upsert_flow_ir_module",
            &serde_json::json!({
                "draft_id": "stalled-module",
                "expected_revision": 4,
                "module": { "kind": "function", "name": "classify", "steps": [] }
            }),
        )
        .expect("the next repeated module repair must stop locally");
        let payload = workflow_call_result_json(&rejected);
        assert_eq!(payload["status"], "typed_repair_progress_stalled");
        assert_eq!(payload["draft_id"], "stalled-module");
        assert_eq!(payload["revision"], 4);
        assert_eq!(payload["stalled_attempts"], 3);
        assert_eq!(
            state.lock().expect("state lock").typed_operation_attempts,
            attempts_before_rejection,
            "the rejected preflight is not a dispatched operation or progress"
        );
    }

    #[test]
    fn typed_ir_schema_failures_do_not_claim_an_unstarted_draft_is_retained() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let args = serde_json::json!({
            "draft_id": "never-started",
            "expected_modules": ["eventsSimple"],
            "program": { "interfaces": [{ "type": "string" }] }
        });
        for _ in 0..=MAX_EXTERNAL_TYPED_IR_STALLED_ATTEMPTS {
            assert!(
                workflow_tool_preflight_with_args(&state, "begin_flow_ir_draft", &args).is_none()
            );
            workflow_tool_record(
                &state,
                "begin_flow_ir_draft",
                &args,
                "invalid params: FlowIrType must use the canonical object shape",
            );
        }

        let rejected = workflow_tool_preflight_with_args(&state, "begin_flow_ir_draft", &args)
            .expect("repeated invalid begin calls must stop locally");
        let payload = workflow_call_result_json(&rejected);
        assert_eq!(payload["status"], "typed_repair_progress_stalled");
        assert_eq!(payload["draft_retained"], false);
        assert_eq!(payload["draft_id"], "never-started");
        assert_eq!(payload["revision"], serde_json::Value::Null);
        assert_eq!(payload["next_action"], "stop_and_report_begin_failure");
    }

    #[test]
    fn typed_ir_infeasible_begin_with_draft_id_is_not_resumable() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let args = serde_json::json!({
            "draft_id": "infeasible-not-stored",
            "expected_modules": ["eventsSimple"]
        });
        assert!(workflow_tool_preflight_with_args(&state, "begin_flow_ir_draft", &args).is_none());
        workflow_tool_record(
            &state,
            "begin_flow_ir_draft",
            &args,
            &serde_json::json!({
                "status": "infeasible",
                "code": "IR_CAPABILITY_PLAN_INFEASIBLE",
                "draft_id": "infeasible-not-stored",
                "revision": null,
                "message": "The draft was not started",
                "diagnostics": [{
                    "code": "IR_CAPABILITY_UNAVAILABLE",
                    "message": "SMTP is unavailable"
                }]
            })
            .to_string(),
        );

        let state = state.lock().expect("state lock");
        assert_eq!(
            state.typed_draft_id.as_deref(),
            Some("infeasible-not-stored")
        );
        assert!(!state.typed_draft_retained);
        let snapshot = state.snapshot();
        drop(state);
        let continuation =
            build_external_workflow_continuation_prompt("build support mail", Some(&snapshot), 1);
        assert!(continuation.contains("TYPED DRAFT WAS NOT STARTED"));
        assert!(!continuation.contains("RETAINED TYPED DRAFT"));
    }

    #[test]
    fn raw_workflow_path_cannot_switch_to_typed_mid_repair() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        assert!(workflow_tool_preflight(&state, "edit_flowscript").is_none());
        workflow_tool_record(
            &state,
            "edit_flowscript",
            &serde_json::json!({ "flowscript": "eventsSimple() { brokenCall() }" }),
            &serde_json::json!({
                "status": "validation_errors",
                "errors": ["brokenCall does not match a catalog declaration"]
            })
            .to_string(),
        );
        let conflict = workflow_tool_preflight(&state, "begin_flow_ir_draft")
            .expect("typed mutation must be blocked after raw repair starts");
        assert_eq!(conflict.is_error, Some(true));
        assert_eq!(
            state.lock().expect("state lock").mutation_path,
            Some(WorkflowMutationPath::FlowScript)
        );
        assert_eq!(
            state.lock().expect("state lock").typed_operation_attempts,
            0,
            "a mutation-path preflight rejection is neither a dispatch nor progress"
        );
    }

    #[test]
    fn representation_rejected_emit_does_not_block_the_flowscript_path() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let forbidden = serde_json::json!({
            "commands": [{
                "command_type": "AddNode",
                "node_type": "log_info",
                "ref_id": "$0",
                "position": { "x": 0, "y": 0 },
                "summary": "Add log"
            }],
            "explanation": "Build behavior"
        });

        assert!(workflow_tool_preflight_with_args(&state, "emit_commands", &forbidden).is_none());
        assert_eq!(state.lock().expect("state lock").mutation_path, None);

        assert!(
            workflow_tool_preflight_with_args(
                &state,
                "get_declarations",
                &serde_json::json!({ "queries": ["log information"] }),
            )
            .is_none()
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &serde_json::json!({ "queries": ["log information"] }),
            "declare function logInfo({ message: string }): void;  // impure",
        );

        assert!(
            workflow_tool_preflight_with_args(
                &state,
                "write_flowscript",
                &serde_json::json!({
                    "draft_id": "redirected-source",
                    "source": "eventsSimple() { logInfo({ message: \"hello\" }) }"
                }),
            )
            .is_none()
        );
        assert_eq!(
            state.lock().expect("state lock").mutation_path,
            Some(WorkflowMutationPath::FlowScript)
        );

        let visual_state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        assert!(
            workflow_tool_preflight_with_args(
                &visual_state,
                "emit_commands",
                &serde_json::json!({
                    "commands": [{
                        "command_type": "MoveNode",
                        "node_id": "node-1",
                        "position": { "x": 20, "y": 40 },
                        "summary": "Move node"
                    }],
                    "explanation": "Align nodes"
                }),
            )
            .is_none()
        );
        assert_eq!(
            visual_state.lock().expect("state lock").mutation_path,
            Some(WorkflowMutationPath::DirectCommands)
        );
    }

    #[test]
    fn declaration_repairs_allow_new_diagnostic_signatures_but_deduplicate_old_ones() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let initial_lookup = serde_json::json!({ "queries": ["imap receive", "smtp send"] });
        assert!(
            workflow_tool_preflight_with_args(&state, "get_declarations", &initial_lookup)
                .is_none()
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &initial_lookup,
            concat!(
                "// flowpilot.declaration-batch/v1 {\"processed_count\":2,\"matched_count\":2,\"matched_queries\":[\"imap receive\",\"smtp send\"],\"unmatched_count\":0,\"unmatched_queries\":[],\"output_omitted_count\":0,\"output_omitted_queries\":[],\"complete\":true,\"omitted_count\":0,\"omitted_queries\":[],\"truncated_query_count\":0}\n",
                "declare function mailImapList({ inbox: Struct }): Struct[];\n",
                "declare function emailSmtpSend({ to: string }): void;"
            ),
        );

        let failed_edit = |signature: &str| {
            assert!(workflow_tool_preflight(&state, "edit_flowscript").is_none());
            workflow_tool_record(
                &state,
                "edit_flowscript",
                &serde_json::json!({
                    "flowscript": format!("eventsSimple() {{ {signature}() }}")
                }),
                &serde_json::json!({
                    "status": "validation_errors",
                    "errors": [format!("FlowScript call `{signature}` does not match a catalog declaration; call `get_declarations` and use the exact function name")]
                })
                .to_string(),
            );
        };

        failed_edit("firstMissingCall");
        let first_repair = serde_json::json!({ "queries": ["exact firstMissingCall signature"] });
        assert!(
            workflow_tool_preflight_with_args(&state, "get_declarations", &first_repair).is_none(),
            "the first diagnostic-targeted repair should be dispatched"
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &first_repair,
            "declare function firstMissingCall({}): void;",
        );

        failed_edit("firstMissingCall");
        let duplicate = workflow_tool_preflight_with_args(
            &state,
            "get_declarations",
            &serde_json::json!({ "queries": ["firstMissingCall"] }),
        )
        .expect("a duplicate targeted signature must be redirected locally");
        assert_eq!(duplicate.is_error, Some(false));
        let duplicate = duplicate
            .content
            .iter()
            .filter_map(|content| match &content.raw {
                rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            duplicate.contains("duplicate_declaration_lookup"),
            "{duplicate}"
        );

        failed_edit("secondMissingCall");
        assert!(
            workflow_tool_preflight_with_args(
                &state,
                "get_declarations",
                &serde_json::json!({ "queries": ["secondMissingCall"] }),
            )
            .is_none(),
            "a distinct signature from a new diagnostic must remain available after the old two-call cap"
        );
        assert_eq!(state.lock().expect("state lock").declaration_calls, 3);
    }

    #[test]
    fn declaration_repair_hints_cover_real_pin_and_type_diagnostics() {
        let hints = diagnostic_declaration_repair_hints(&[
            "binary comparison `==` has incompatible operand types `Generic` and `String`"
                .to_string(),
            "argument `condition` on `controlBranch` is not a literal or resolvable node output; skipped connection"
                .to_string(),
            "node `stringContains` has no input pin named `value`; skipped that argument"
                .to_string(),
            "binary comparison `==` has ambiguous operand type; candidates are equal_string, bool_equal, int_equal"
                .to_string(),
        ]);

        assert!(hints.exact_symbols.contains("controlbranch"));
        assert!(hints.exact_symbols.contains("stringcontains"));
        assert!(hints.exact_symbols.contains("equal_string"));
        assert!(hints.exact_symbols.contains("bool_equal"));
        assert!(hints.exact_symbols.contains("int_equal"));
        assert!(!hints.exact_symbols.contains("condition"));
        assert!(!hints.exact_symbols.contains("generic"));
        assert!(hints.topics.contains("comparison"));
        assert!(hints.topics.contains("type_conversion"));
        assert!(hints.topics.contains("string_operations"));
    }

    #[test]
    fn declaration_repairs_allow_bounded_type_and_pin_lookup_batches() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let initial_lookup = serde_json::json!({ "queries": ["boolean comparison"] });
        assert!(
            workflow_tool_preflight_with_args(&state, "get_declarations", &initial_lookup)
                .is_none()
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &initial_lookup,
            "declare function boolEqual({ boolean: boolean, boolean: boolean }): boolean;",
        );
        assert!(workflow_tool_preflight(&state, "edit_flowscript").is_none());
        workflow_tool_record(
            &state,
            "edit_flowscript",
            &serde_json::json!({ "flowscript": "eventsSimple() { brokenCall() }" }),
            &serde_json::json!({
                "status": "validation_errors",
                "errors": [
                    "binary comparison `==` has incompatible operand types `Generic` and `String`",
                    "node `stringContains` has no input pin named `value`; skipped that argument"
                ]
            })
            .to_string(),
        );

        let repair_lookup = serde_json::json!({
            "queries": [
                "equalString notEqualString exact signatures",
                "stringContains exact signature",
                "stringTrim exact signature",
                "stringStartsWith exact signature",
                "stringReplace exact signature",
                "integer add increment exact signature",
                "convert Generic any to string",
                "try catch error boundary invoke function per item"
            ]
        });
        assert!(
            workflow_tool_preflight_with_args(&state, "get_declarations", &repair_lookup).is_none(),
            "pin and type diagnostics should authorize one bounded corrective batch"
        );
        let repair_result = format!(
            "// flowpilot.declaration-batch/v1 {}\n{}",
            serde_json::json!({
                "processed_count": 8,
                "matched_count": 8,
                "matched_queries": repair_lookup["queries"],
                "unmatched_count": 0,
                "unmatched_queries": [],
                "output_omitted_count": 0,
                "output_omitted_queries": [],
                "complete": true,
                "omitted_count": 0,
                "omitted_queries": [],
                "truncated_query_count": 0
            }),
            concat!(
                "declare function equalString({ left: string, right: string }): boolean;\n",
                "declare function stringContains({ text: string, pattern: string }): boolean;\n",
                "declare function stringTrim({ text: string }): string;\n",
                "declare function stringStartsWith({ text: string, prefix: string }): boolean;\n",
                "declare function stringReplace({ text: string, pattern: string, replacement: string, isRegex: boolean }): string;\n",
                "declare function integerAdd({ integer: integer, integer: integer }): integer;\n",
                "declare function genericToString({ value: any }): string;\n",
                "declare function tryCatch({ invoke: Struct }): Struct;"
            )
        );
        workflow_tool_record(&state, "get_declarations", &repair_lookup, &repair_result);
        let state = state.lock().expect("state lock");
        let completed = &state.completed_repair_lookup_keys;
        assert!(completed.contains("topic:comparison"));
        assert!(completed.contains("topic:type_conversion"));
        assert!(completed.contains("topic:string_operations"));
        assert!(completed.contains("symbol:stringcontains"));
    }

    #[test]
    fn declaration_repairs_reject_broad_queries_even_when_they_name_a_target() {
        let hints = diagnostic_declaration_repair_hints(&[
            "node `stringContains` has no input pin named `value`".to_string(),
        ]);
        assert!(!declaration_repair_query_is_bounded(
            "stringContains and every node in the entire catalog"
        ));
        assert!(
            declaration_repair_query_keys("stringContains exact signature", &hints)
                .contains("symbol:stringcontains")
        );
    }

    #[test]
    fn authoritative_repair_no_match_is_deduplicated_but_abort_is_retryable() {
        let make_state = || {
            Arc::new(StdMutex::new(WorkflowToolLoopState {
                declaration_calls: 1,
                initial_declaration_lookup_complete: true,
                last_errors: vec!["FlowScript call `missingCall` does not match a catalog declaration; call `get_declarations` and use the exact function name".to_string()],
                ..Default::default()
            }))
        };
        let args = serde_json::json!({ "queries": ["missingCall exact signature"] });

        let no_match_state = make_state();
        assert!(
            workflow_tool_preflight_with_args(&no_match_state, "get_declarations", &args).is_none()
        );
        workflow_tool_record(
            &no_match_state,
            "get_declarations",
            &args,
            "// flowpilot.declaration-batch/v1 {\"processed_count\":1,\"matched_count\":0,\"matched_queries\":[],\"unmatched_count\":1,\"unmatched_queries\":[\"missingCall exact signature\"],\"output_omitted_count\":0,\"output_omitted_queries\":[],\"complete\":false,\"omitted_count\":0,\"omitted_queries\":[],\"truncated_query_count\":0}\nNo declaration matched.",
        );
        {
            let mut state = no_match_state.lock().expect("state lock");
            assert!(
                state
                    .completed_repair_lookup_keys
                    .contains("symbol:missingcall")
            );
            // A normal edit/check result opens the next diagnostic phase. Reproduce that boundary
            // directly so this assertion exercises persistent key deduplication, not merely the
            // one-discovery-per-edit guard.
            state.declarations_since_edit = 0;
        }
        let duplicate =
            workflow_tool_preflight_with_args(&no_match_state, "get_declarations", &args)
                .expect("a definitive catalog miss must not reopen the same repair forever");
        assert_eq!(
            workflow_call_result_json(&duplicate)["status"],
            "duplicate_declaration_lookup"
        );

        let aborted_state = make_state();
        assert!(
            workflow_tool_preflight_with_args(&aborted_state, "get_declarations", &args).is_none()
        );
        workflow_tool_abort(&aborted_state, "get_declarations", "worker disconnected");
        assert!(
            workflow_tool_preflight_with_args(&aborted_state, "get_declarations", &args).is_none(),
            "a transport abort consumes neither the diagnostic key nor its retry allowance"
        );
    }

    #[test]
    fn output_omitted_repair_gets_one_focused_retry_then_stops() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
            declaration_calls: 1,
            initial_declaration_lookup_complete: true,
            last_errors: vec!["FlowScript call `missingCall` does not match a catalog declaration; call `get_declarations` and use the exact function name".to_string()],
            ..Default::default()
        }));
        let args = serde_json::json!({ "queries": ["missingCall exact signature"] });
        let omitted = "// flowpilot.declaration-batch/v1 {\"processed_count\":1,\"matched_count\":0,\"matched_queries\":[],\"unmatched_count\":0,\"unmatched_queries\":[],\"output_omitted_count\":1,\"output_omitted_queries\":[\"missingCall exact signature\"],\"complete\":false,\"omitted_count\":0,\"omitted_queries\":[],\"truncated_query_count\":0}\nThe exact signature did not fit.";

        for attempt in 0..MAX_REPAIR_DECLARATION_ATTEMPTS_PER_KEY {
            assert!(
                workflow_tool_preflight_with_args(&state, "get_declarations", &args).is_none(),
                "omitted-signature attempt {attempt} should dispatch"
            );
            workflow_tool_record(&state, "get_declarations", &args, omitted);
        }
        let exhausted = workflow_tool_preflight_with_args(&state, "get_declarations", &args)
            .expect("the per-key omission cap must terminate exact repeats");
        assert_eq!(
            workflow_call_result_json(&exhausted)["status"],
            "duplicate_declaration_lookup"
        );
    }

    #[test]
    fn metadata_less_multi_query_repair_does_not_claim_every_target_resolved() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
            declaration_calls: 1,
            initial_declaration_lookup_complete: true,
            last_errors: vec!["FlowScript call `missingCall` does not match a catalog declaration; call `get_declarations` and use the exact function name".to_string()],
            ..Default::default()
        }));
        let broad = serde_json::json!({
            "queries": ["missingCall exact signature", "missingCall input pins"]
        });
        assert!(workflow_tool_preflight_with_args(&state, "get_declarations", &broad).is_none());
        workflow_tool_record(
            &state,
            "get_declarations",
            &broad,
            "declare function missingCall({ input: string }): void;",
        );
        assert!(
            state
                .lock()
                .expect("state lock")
                .completed_repair_lookup_keys
                .is_empty()
        );
        assert!(
            workflow_tool_preflight_with_args(
                &state,
                "get_declarations",
                &serde_json::json!({ "queries": ["missingCall exact signature"] }),
            )
            .is_none(),
            "one focused retry remains available when a legacy multi-result cannot prove coverage"
        );
    }

    #[test]
    fn declaration_repair_rejects_unrelated_broad_searches() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let initial_lookup = serde_json::json!({ "queries": ["missing call"] });
        assert!(
            workflow_tool_preflight_with_args(&state, "get_declarations", &initial_lookup)
                .is_none()
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &initial_lookup,
            "declare function missingCall({}): void;",
        );
        assert!(workflow_tool_preflight(&state, "edit_flowscript").is_none());
        workflow_tool_record(
            &state,
            "edit_flowscript",
            &serde_json::json!({ "flowscript": "eventsSimple() { missingCall() }" }),
            &serde_json::json!({
                "status": "validation_errors",
                "errors": ["FlowScript call `missingCall` does not match a catalog declaration; call `get_declarations` and use the exact function name"]
            })
            .to_string(),
        );

        let redirected = workflow_tool_preflight_with_args(
            &state,
            "get_declarations",
            &serde_json::json!({ "queries": ["search the entire mail catalog"] }),
        )
        .expect("unrelated repair discovery must be redirected");
        assert_eq!(redirected.is_error, Some(false));
        let text = redirected
            .content
            .iter()
            .filter_map(|content| match &content.raw {
                rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("diagnostic_lookup_required"), "{text}");
        assert_eq!(state.lock().expect("state lock").declaration_calls, 1);
    }

    #[test]
    fn workflow_edit_session_defers_runtime_verification_until_persisted() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));

        for tool in ["execute_event", "execute_node", "query_execution_logs"] {
            let result = workflow_tool_preflight(&state, tool)
                .expect("mutation sessions must return an explicit runtime deferral");
            assert_eq!(result.is_error, Some(true));
            let text = result
                .content
                .iter()
                .filter_map(|content| match &content.raw {
                    rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("runtime_verification_deferred"), "{text}");
            assert!(text.contains("later turn"), "{text}");
        }
    }

    #[test]
    fn workflow_edit_session_defers_table_creation_until_board_draft_is_queued() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let create_table = serde_json::json!({
            "operation": "create_table",
            "table_name": "support_tickets",
            "fields": [{ "name": "ticket_id", "type": "string" }]
        });

        let result = workflow_database_setup_preflight(&state, "database_tool", &create_table)
            .expect("schema creation before the first board draft must be rejected locally");
        assert_eq!(result.is_error, Some(false));
        let text = result
            .content
            .iter()
            .filter_map(|content| match &content.raw {
                rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("board_draft_required_before_database_setup"));
        assert!(text.contains("no approval was opened"));

        assert!(
            workflow_database_setup_preflight(
                &state,
                "database_tool",
                &serde_json::json!({ "operation": "list_tables" }),
            )
            .is_none(),
            "read-only database inspection remains available before the edit"
        );

        state.lock().expect("state lock").queued = true;
        assert!(
            workflow_database_setup_preflight(&state, "database_tool", &create_table).is_none(),
            "schema setup may proceed after a complete board draft has queued"
        );
    }

    #[test]
    fn ancillary_predraft_inspection_is_bounded_until_source_is_retained() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let tools = ["database_tool", "ui_inspect", "storage_tool"];
        for call in 0..MAX_EXTERNAL_PREDRAFT_CONTEXT_READS {
            assert!(
                workflow_predraft_context_preflight(&state, tools[usize::from(call) % tools.len()])
                    .is_none(),
                "ancillary context call {call} should fit the pre-draft budget"
            );
        }

        state
            .lock()
            .expect("state lock")
            .initial_declaration_lookup_usable = true;
        let blocked = workflow_predraft_context_preflight(&state, "database_tool")
            .expect("the next exhaustive inspection must redirect to source retention");
        let blocked = workflow_call_result_json(&blocked);
        assert_eq!(blocked["status"], "predraft_inspection_budget_exhausted");
        assert_eq!(blocked["next_action"], "write_flowscript");
        assert_eq!(
            blocked["inspection_budget"],
            u64::from(MAX_EXTERNAL_PREDRAFT_CONTEXT_READS)
        );

        state.lock().expect("state lock").flowscript_draft_retained = true;
        assert!(
            workflow_predraft_context_preflight(&state, "database_tool").is_none(),
            "focused inspection is available again after a recoverable source exists"
        );
    }

    #[test]
    fn failed_empty_event_retry_preserves_last_actionable_flowscript() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let actionable = r#"eventsSimple() {
    logInfo({ message: "poll support inbox" })
}"#;

        workflow_tool_record(
            &state,
            "edit_flowscript",
            &serde_json::json!({ "flowscript": actionable }),
            &serde_json::json!({
                "status": "validation_errors",
                "errors": ["fix one connection"]
            })
            .to_string(),
        );
        workflow_tool_record(
            &state,
            "edit_flowscript",
            &serde_json::json!({ "flowscript": "eventsSimple() {\n}" }),
            &serde_json::json!({
                "status": "validation_errors",
                "errors": ["empty event entries do not implement a workflow"]
            })
            .to_string(),
        );

        let snapshot = state.lock().expect("state lock").snapshot();
        assert_eq!(snapshot.last_flowscript.as_deref(), Some(actionable));
        assert_eq!(snapshot.last_status.as_deref(), Some("validation_errors"));
        assert_eq!(snapshot.last_errors, vec!["fix one connection"]);
    }

    fn rich_support_flowscript() -> &'static str {
        r#"@secret
const IMAP_HOST: string = ""

function pollSupportInbox() {
    const connection = emailImapConnect({ host: IMAP_HOST })
    const inbox = mailImapInbox({ connection: connection.connection })
    const refs = mailImapList({ inbox: inbox.inbox })
    const first = arrayGet({ array: refs.refs, index: 0 })
    const mail = emailImapInboxFetchMail({ emailRef: first.value })
    logInfo({ message: mail.subject })
}

function requestApproval() {
    smtpSendEmail({ to: "example@example.com", subject: "Review" })
}

eventsSimple() {
    pollSupportInbox()
    requestApproval()
}

eventsGeneric(payload: Struct) {
    structGet({ struct: payload, field: "ticket_id" })
}
"#
    }

    fn seed_failed_rich_candidate(state: &Arc<StdMutex<WorkflowToolLoopState>>) {
        workflow_tool_record(
            state,
            "edit_flowscript",
            &serde_json::json!({ "flowscript": rich_support_flowscript() }),
            &serde_json::json!({
                "status": "validation_errors",
                "errors": ["one connection still needs repair"]
            })
            .to_string(),
        );
    }

    #[test]
    fn sdk_guard_blocks_tiny_valid_candidate_before_it_can_queue() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        seed_failed_rich_candidate(&state);
        let handler_calls = Arc::new(AtomicUsize::new(0));
        let calls = handler_calls.clone();
        let handler: copilot_sdk::ToolHandler = Arc::new(move |_name, _args| {
            calls.fetch_add(1, Ordering::SeqCst);
            copilot_sdk::ToolResultObject::text(
                serde_json::json!({ "status": "queued", "queued_count": 1 }).to_string(),
            )
        });
        let mut guarded = guard_sdk_workflow_tools(
            vec![(copilot_sdk::Tool::new("edit_flowscript"), handler)],
            state.clone(),
        );
        let (_, guarded_handler) = guarded.pop().expect("guarded edit tool");
        let tiny = serde_json::json!({
            "flowscript": "eventsSimple() {\n    logInfo({ message: \"works\" })\n}",
            "allow_deletions": true
        });

        let result = guarded_handler("edit_flowscript", &tiny);
        let result_text = result
            .error
            .as_deref()
            .unwrap_or(&result.text_result_for_llm);
        assert_eq!(handler_calls.load(Ordering::SeqCst), 0);
        assert!(result_text.contains("candidate_regression"));
        assert!(result_text.contains("retained_flowscript"));

        let snapshot = state.lock().expect("state lock").snapshot();
        assert_eq!(
            snapshot.last_flowscript.as_deref(),
            Some(rich_support_flowscript())
        );
        assert_eq!(snapshot.edit_attempts, 1);
        assert!(
            snapshot
                .last_errors
                .iter()
                .any(|error| error.contains("one connection still needs repair"))
        );
        assert!(
            snapshot
                .last_errors
                .iter()
                .any(|error| error.contains("severe completeness regression"))
        );
        assert!(!state.lock().expect("state lock").edit_in_flight);
    }

    #[test]
    fn panicking_sdk_workflow_handler_aborts_state_and_recovers_the_operation_gate() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        let panicking: copilot_sdk::ToolHandler =
            Arc::new(|_name, _args| panic!("simulated tool handler crash"));
        let follow_up_calls = Arc::new(AtomicUsize::new(0));
        let calls = follow_up_calls.clone();
        let benign: copilot_sdk::ToolHandler = Arc::new(move |_name, _args| {
            calls.fetch_add(1, Ordering::SeqCst);
            copilot_sdk::ToolResultObject::text(
                serde_json::json!({
                    "status": "validation_errors",
                    "errors": ["stub diagnostic"]
                })
                .to_string(),
            )
        });
        // Both guarded handlers share one operation gate, like one live SDK session.
        let mut guarded = guard_sdk_workflow_tools(
            vec![
                (copilot_sdk::Tool::new("edit_flowscript"), panicking),
                (copilot_sdk::Tool::new("edit_flowscript"), benign),
            ],
            state.clone(),
        );
        let (_, benign_handler) = guarded.pop().expect("benign guarded tool");
        let (_, panicking_handler) = guarded.pop().expect("panicking guarded tool");
        let args = serde_json::json!({
            "flowscript": "eventsSimple() {\n    logInfo({ message: \"works\" })\n}"
        });

        let crashed = panicking_handler("edit_flowscript", &args);
        let crashed_text = crashed
            .error
            .as_deref()
            .unwrap_or(&crashed.text_result_for_llm);
        assert!(
            crashed_text.contains("simulated tool handler crash"),
            "{crashed_text}"
        );
        {
            let state = state
                .lock()
                .expect("a handler panic must not poison the workflow loop state");
            assert!(!state.edit_in_flight);
            assert_eq!(state.last_status.as_deref(), Some("error"));
        }

        // The gate must be usable again: the follow-up mutation reaches its handler instead of
        // the permanent retryable "wait" refusal a poisoned gate produced.
        let retried = benign_handler("edit_flowscript", &args);
        let retried_text = retried
            .error
            .as_deref()
            .unwrap_or(&retried.text_result_for_llm);
        assert!(
            !retried_text.contains("Another order-sensitive workflow operation"),
            "{retried_text}"
        );
        assert_eq!(follow_up_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn wrapped_log_helper_is_still_a_candidate_regression_but_domain_helper_can_pass() {
        let smoke_state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        seed_failed_rich_candidate(&smoke_state);
        assert!(workflow_tool_preflight(&smoke_state, "edit_flowscript").is_none());
        let wrapped_smoke = serde_json::json!({
            "flowscript": r#"function smoke() {
    logInfo({ message: "works" })
}
eventsSimple() {
    smoke()
}"#
        });
        let rejected =
            workflow_candidate_preflight(&smoke_state, "edit_flowscript", &wrapped_smoke)
                .expect("wrapping a log smoke test must not bypass completeness retention");
        assert_eq!(rejected.is_error, Some(true));
        assert!(!smoke_state.lock().expect("state lock").edit_in_flight);

        let domain_state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        seed_failed_rich_candidate(&domain_state);
        assert!(workflow_tool_preflight(&domain_state, "edit_flowscript").is_none());
        let modular_domain = serde_json::json!({
            "flowscript": r#"function pollInbox() {
    emailImapConnect({ host: "imap.example.com" })
}
eventsSimple() {
    pollInbox()
}"#
        });
        assert!(
            workflow_candidate_preflight(&domain_state, "edit_flowscript", &modular_domain,)
                .is_none(),
            "a non-empty domain helper invoked by a separate Event is a valid modular partial"
        );
        workflow_tool_record(
            &domain_state,
            "edit_flowscript",
            &modular_domain,
            &serde_json::json!({
                "status": "validation_errors",
                "errors": ["domain helper still needs one pin fix"]
            })
            .to_string(),
        );
        let failed_snapshot = domain_state.lock().expect("state lock").snapshot();
        assert!(!failed_snapshot.queued);
        assert!(failed_snapshot.modular_fallback.is_none());

        assert!(workflow_tool_preflight(&domain_state, "edit_flowscript").is_none());
        assert!(
            workflow_candidate_preflight(&domain_state, "edit_flowscript", &modular_domain,)
                .is_none()
        );
        let mut queued_result = copilot_sdk::ToolResultObject::text(
            serde_json::json!({ "status": "queued", "queued_count": 2 }).to_string(),
        );
        workflow_tool_record(
            &domain_state,
            "edit_flowscript",
            &modular_domain,
            &queued_result.text_result_for_llm,
        );
        annotate_modular_fallback_result(&domain_state, "edit_flowscript", &mut queued_result);
        assert!(
            queued_result
                .text_result_for_llm
                .contains("partial_working_slice")
        );

        let snapshot = domain_state.lock().expect("state lock").snapshot();
        assert!(snapshot.queued);
        assert!(snapshot.modular_fallback.is_some());
        assert_eq!(
            snapshot.retained_full_source.as_deref(),
            Some(rich_support_flowscript())
        );
        let envelope = flowscript_response_workspace_envelope(
            submitted_flowscript(&modular_domain).unwrap(),
            "queued",
            Some(&snapshot),
        );
        let envelope: serde_json::Value = serde_json::from_str(&envelope).unwrap();
        assert_eq!(envelope["completion"], "partial_working_slice");
        assert_eq!(envelope["retained_full_source"], rich_support_flowscript());
    }

    #[test]
    fn best_failed_candidate_survives_smaller_actionable_retries() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        seed_failed_rich_candidate(&state);
        workflow_tool_record(
            &state,
            "edit_flowscript",
            &serde_json::json!({
                "flowscript": "eventsSimple() {\n    emailImapConnect({ host: \"imap.example.com\" })\n    mailImapList({ inbox: inbox })\n    logInfo({ message: \"partial\" })\n}"
            }),
            &serde_json::json!({ "status": "no_changes" }).to_string(),
        );

        let snapshot = state.lock().expect("state lock").snapshot();
        assert_eq!(
            snapshot.last_flowscript.as_deref(),
            Some(rich_support_flowscript())
        );
        assert_eq!(snapshot.last_status.as_deref(), Some("validation_errors"));
        assert_eq!(
            snapshot.last_errors,
            vec!["one connection still needs repair"]
        );
    }

    #[test]
    fn best_failed_candidate_advances_when_a_same_scope_draft_has_fewer_diagnostics() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        workflow_tool_record(
            &state,
            "edit_flowscript",
            &serde_json::json!({ "flowscript": rich_support_flowscript() }),
            &serde_json::json!({
                "status": "validation_errors",
                "errors": ["first unresolved edge", "second unresolved edge"]
            })
            .to_string(),
        );
        let improved = rich_support_flowscript().replace(
            "    structGet({ struct: payload, field: \"ticket_id\" })\n",
            "",
        );
        assert!(
            profile_flowscript_candidate(&improved).completeness_score()
                < profile_flowscript_candidate(rich_support_flowscript()).completeness_score()
        );
        workflow_tool_record(
            &state,
            "edit_flowscript",
            &serde_json::json!({ "flowscript": improved.clone() }),
            &serde_json::json!({
                "status": "validation_errors",
                "errors": ["second unresolved edge"]
            })
            .to_string(),
        );

        let snapshot = state.lock().expect("state lock").snapshot();
        assert_eq!(snapshot.last_flowscript.as_deref(), Some(improved.as_str()));
        assert_eq!(snapshot.last_errors, vec!["second unresolved edge"]);
    }

    #[test]
    fn workflow_edit_budget_keeps_a_higher_hard_safety_cap() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        for attempt in 0..MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS {
            assert!(
                workflow_tool_preflight(&state, "edit_flowscript").is_none(),
                "attempt {attempt} should be available"
            );
            workflow_tool_abort(&state, "edit_flowscript", "retry");
        }
        let exhausted = workflow_tool_preflight(&state, "edit_flowscript")
            .expect("the attempt after the hard safety cap must be rejected");
        assert_eq!(exhausted.is_error, Some(true));
    }

    #[test]
    fn workflow_edit_loop_allows_more_than_five_repairs_when_diagnostics_progress() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        for attempt in 0..8 {
            assert!(
                workflow_tool_preflight(&state, "edit_flowscript").is_none(),
                "genuine diagnostic progress must keep attempt {attempt} available"
            );
            workflow_tool_record(
                &state,
                "edit_flowscript",
                &serde_json::json!({
                    "flowscript": format!("eventsSimple() {{ repair{attempt}() }}")
                }),
                &serde_json::json!({
                    "status": "validation_errors",
                    "errors": [format!("remaining diagnostic {attempt}")]
                })
                .to_string(),
            );
        }
        assert_eq!(state.lock().expect("state lock").edit_attempts, 8);
        assert_eq!(state.lock().expect("state lock").stalled_edit_attempts, 0);
        assert!(workflow_tool_preflight(&state, "edit_flowscript").is_none());
    }

    #[test]
    fn workflow_edit_loop_stops_after_repeated_identical_diagnostics() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        for attempt in 0..=MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS {
            assert!(
                workflow_tool_preflight(&state, "edit_flowscript").is_none(),
                "initial failure plus bounded stalled attempt {attempt} should run"
            );
            workflow_tool_record(
                &state,
                "edit_flowscript",
                &serde_json::json!({
                    "flowscript": format!("eventsSimple() {{ stillBroken{attempt}() }}")
                }),
                &serde_json::json!({
                    "status": "validation_errors",
                    "errors": ["the exact same unresolved diagnostic"]
                })
                .to_string(),
            );
        }

        let stalled = workflow_tool_preflight(&state, "edit_flowscript")
            .expect("the next identical-diagnostic repair must stop locally");
        assert_eq!(stalled.is_error, Some(true));
        let text = stalled
            .content
            .iter()
            .filter_map(|content| match &content.raw {
                rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("edit_progress_stalled"), "{text}");
    }

    #[test]
    fn final_workspace_envelope_keeps_source_and_status_atomic() {
        let queued = flowscript_workspace_envelope("eventsSimple() {}", "queued");
        let parsed: serde_json::Value = serde_json::from_str(&queued).unwrap();
        assert_eq!(parsed["source"], "eventsSimple() {}");
        assert_eq!(parsed["status"], "queued");

        let failed = flowscript_workspace_envelope(rich_support_flowscript(), "validation_errors");
        let parsed: serde_json::Value = serde_json::from_str(&failed).unwrap();
        assert_eq!(parsed["source"], rich_support_flowscript());
        assert_eq!(parsed["status"], "validation_errors");
    }

    #[test]
    fn flowscript_lifecycle_workspace_frame_carries_source_revision_and_status() {
        let submitted = "eventsSimple() {\n    logInfo({ message: \"ok\" })\n}";

        let checked = flowscript_workspace_result_payload(
            "check_flowscript",
            &serde_json::json!({
                "status": "valid",
                "draft_id": "support-flow",
                "revision": 4,
                "base_fingerprint": "board-v1",
                "source": submitted,
            }),
            None,
        )
        .expect("a retained source result should update the workspace");
        assert_eq!(checked["source"], submitted);
        assert_eq!(checked["status"], "valid");
        assert_eq!(checked["draft_id"], "support-flow");
        assert_eq!(checked["revision"], 4);
        assert_eq!(checked["base_fingerprint"], "board-v1");

        let queued = flowscript_workspace_result_payload(
            "commit_flowscript",
            &serde_json::json!({
                "status": "queued",
                "queued_count": 2,
                "draft_id": "support-flow",
                "revision": 4,
                "source": submitted,
            }),
            None,
        )
        .expect("queued source commit should produce a workspace status frame");
        assert_eq!(
            queued.get("source").and_then(|v| v.as_str()),
            Some(submitted)
        );
        assert_eq!(
            queued.get("status").and_then(|v| v.as_str()),
            Some("queued")
        );

        let rejected = flowscript_workspace_result_payload(
            "edit_flowscript",
            &serde_json::json!({ "status": "validation_errors" }),
            Some(submitted),
        )
        .expect("failed edit should still close the submitted workspace preview");
        assert_eq!(
            rejected.get("status").and_then(|v| v.as_str()),
            Some("validation_errors")
        );

        // Compatibility-only typed sessions can still be rendered while old runs drain.
        let typed_source = "eventsSimple() {\n    logInfo({ message: \"typed\" })\n}";
        let typed = flowscript_workspace_result_payload(
            "validate_flow_ir_draft",
            &serde_json::json!({
                "status": "draft_needs_repair",
                "flowscript": typed_source,
            }),
            None,
        )
        .expect("typed draft result should carry its compiled FlowScript workspace");
        assert_eq!(
            typed.get("source").and_then(|v| v.as_str()),
            Some(typed_source)
        );
        assert_eq!(
            typed.get("status").and_then(|v| v.as_str()),
            Some("draft_needs_repair")
        );

        assert!(
            flowscript_workspace_result_payload(
                "get_declarations",
                &serde_json::json!({ "status": "done" }),
                Some(submitted),
            )
            .is_none(),
            "unrelated tool results must not mutate workspace status"
        );
    }

    #[test]
    fn typed_structured_diagnostics_drive_direct_sdk_status() {
        let rejected = serde_json::json!({
            "status": "draft_started",
            "structured_diagnostics": [{
                "code": "FS_PIN_TYPE_MISMATCH",
                "message": "expected String, got Generic",
                "path": "/modules/0/steps/1/args/0"
            }],
            "missing_modules": ["deliver_message"]
        });
        let diagnostics = workflow_result_diagnostics(Some(&rejected));
        assert_eq!(
            diagnostics,
            vec![
                "[FS_PIN_TYPE_MISMATCH] expected String, got Generic".to_string(),
                "Missing required module: deliver_message".to_string(),
            ]
        );
        assert!(workflow_result_requires_repair(&rejected, &diagnostics));
        assert_eq!(
            direct_sdk_tool_result_stream_status(&rejected.to_string()),
            "error"
        );

        let valid = serde_json::json!({ "status": "draft_valid", "diagnostics": [] });
        assert_eq!(
            direct_sdk_tool_result_stream_status(&valid.to_string()),
            "done"
        );

        let staged = serde_json::json!({
            "status": "module_validated",
            "diagnostics": [],
            "missing_modules": ["final_event"]
        });
        assert_eq!(
            direct_sdk_tool_result_stream_status(&staged.to_string()),
            "done",
            "missing future modules are expected staged progress, not a failed upsert"
        );
    }

    #[test]
    fn external_continuation_carries_declarations_and_fails_honestly() {
        let snapshot = WorkflowToolLoopSnapshot {
            last_declarations: Some("declare function log(...): void".to_string()),
            last_repair_declarations: vec![
                "declare function stringReplace(string: string, pattern: string, replacement: string, isRegex: bool): (string: string)"
                    .to_string(),
            ],
            last_status: Some("validation_errors".to_string()),
            last_errors: vec!["missing pin".to_string()],
            edit_attempts: 2,
            flowscript_draft_id: Some("support-flow".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(3),
            ..Default::default()
        };
        let prompt = build_external_workflow_continuation_prompt("build it", Some(&snapshot), 1);
        assert!(prompt.contains("declare function log"));
        assert!(prompt.contains("EXACT LIVE-CATALOG REPAIR DECLARATIONS"));
        assert!(prompt.contains("isRegex: bool"));
        assert!(prompt.contains("missing pin"));
        assert!(prompt.contains("Validation diagnostics (1 total)"));
        assert!(prompt.contains("draft_id=support-flow, revision=3"));

        let error = external_workflow_incomplete_error(
            Some(&snapshot),
            MAX_EXTERNAL_WORKFLOW_CONTINUATIONS,
        );
        assert!(error.contains("without queueing changes"));
        assert!(error.contains("missing pin"));
        assert!(error.contains("draft_id=support-flow, revision=3"));
    }

    #[test]
    fn incomplete_error_reports_every_budget_and_all_retained_diagnostics() {
        let snapshot = WorkflowToolLoopSnapshot {
            last_status: Some("validation_errors".to_string()),
            last_errors: (0..25)
                .map(|index| format!("diagnostic number {index}"))
                .collect(),
            last_structured_diagnostics: vec![serde_json::json!({
                "code": "FS_UNKNOWN_INPUT_PIN",
                "phase": "type_check",
                "message": "unknown pin `mail_subject`"
            })],
            edit_attempts: 2,
            flowscript_operation_attempts: MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
            stalled_edit_attempts: 1,
            flowscript_commit_attempts: 0,
            exhausted_budget: Some(format!(
                "FlowScript source operation budget ({}/{})",
                MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
                MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS
            )),
            flowscript_draft_id: Some("mail-agent".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(16),
            ..Default::default()
        };

        let error = external_workflow_incomplete_error(Some(&snapshot), 2);
        assert!(error.contains("FlowScript source operation budget (24/24)"));
        assert!(error.contains("provider continuations 2/2"));
        assert!(error.contains("checks 2/12"));
        assert!(error.contains("source operations 24/24"));
        assert!(error.contains("stalled repeats 1/3"));
        assert!(error.contains("commit attempts 0/3"));
        assert!(error.contains("Remaining diagnostics (25 total)"));
        assert!(error.contains("diagnostic number 0"));
        assert!(error.contains("diagnostic number 19"));
        assert!(!error.contains("diagnostic number 20"));
        assert!(error.contains("(+5 more)"));
        assert!(error.contains("Structured diagnostics (1 retained)"));
        assert!(error.contains("FS_UNKNOWN_INPUT_PIN"));
        assert!(error.contains("draft_id=mail-agent, revision=16"));
        assert!(error.contains("same user request"));
    }

    #[test]
    fn nested_wall_clock_budget_terminates_gracefully_through_the_incomplete_path() {
        assert!(!nested_wall_clock_exhausted(None));
        assert!(!nested_wall_clock_exhausted(Some(
            Instant::now() + Duration::from_secs(60)
        )));
        assert!(nested_wall_clock_exhausted(Some(
            Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("test instant supports subtraction")
        )));

        let snapshot = WorkflowToolLoopSnapshot {
            last_status: Some("validation_errors".to_string()),
            last_errors: vec!["missing pin".to_string()],
            edit_attempts: 3,
            flowscript_draft_id: Some("uptime-monitor".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(11),
            ..Default::default()
        };
        let error = nested_wall_clock_incomplete_error(Some(&snapshot), 1);
        assert!(error.contains("NESTED_RUN_WALL_CLOCK_BUDGET_EXHAUSTED"));
        assert!(error.contains("wall-clock budget"));
        assert!(!error.contains("provider continuation budget"));
        assert!(error.contains("stopped gracefully"));
        assert!(error.contains("terminal for this run"));
        // The shared incomplete path keeps the retained draft coordinates and diagnostics so the
        // outer agent can resume the exact candidate instead of rebuilding.
        assert!(error.contains("draft_id=uptime-monitor, revision=11"));
        assert!(error.contains("missing pin"));
        assert!(error.contains("checks 3/12"));
    }

    #[test]
    fn nested_wall_clock_budget_stays_below_the_outer_bridge_dispatch_bound() {
        use flow_like::flow::copilot::tool_spec::find_global_tool_spec;
        // flowpilot_board is the long-build delegation whose 30-minute bridge bound previously
        // outlived a hung nested run for a whole outer turn. flowpilot_widget's own 10-minute
        // bridge bound is tighter than this budget, and its frontend deadline path already
        // cancels the nested run explicitly at that bound.
        let spec = find_global_tool_spec("flowpilot_board").expect("flowpilot_board spec");
        assert!(
            NESTED_RUN_WALL_CLOCK_BUDGET < Duration::from_secs(spec.timeout_secs),
            "the nested wall-clock budget must expire before the outer flowpilot_board bridge deadline so the waiting agent receives a terminal result instead of a channel loss"
        );
    }

    #[test]
    fn delegated_heartbeats_carry_nested_tool_and_budget_progress() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
            edit_attempts: 3,
            flowscript_operation_attempts: 9,
            flowscript_commit_attempts: 1,
            ..Default::default()
        }));
        record_delegated_run_tool_progress("patch_flowscript", 23, Some(&state));

        let base = "FlowPilot flowpilot_board is still running";
        let message = delegated_run_heartbeat_message(base);
        assert!(message.starts_with(base));
        assert!(message.contains("patch_flowscript"));
        assert!(message.contains("tool call 23"));
        assert!(message.contains(&format!("checks 3/{MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS}")));
        assert!(message.contains(&format!(
            "source operations 9/{MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS}"
        )));
        assert!(message.contains(&format!(
            "commit attempts 1/{MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS}"
        )));

        // The delegation tools themselves are the wait, not the nested progress.
        record_delegated_run_tool_progress("flowpilot_board", 99, None);
        let unchanged = delegated_run_heartbeat_message(base);
        assert!(unchanged.contains("patch_flowscript"));
        assert!(!unchanged.contains("tool call 99"));

        // Stale progress must not narrate a hung wait as movement.
        if let Ok(mut latest) = LATEST_DELEGATED_RUN_TOOL_PROGRESS.lock()
            && let Some((recorded_at, _)) = latest.as_mut()
        {
            *recorded_at = Instant::now()
                .checked_sub(DELEGATED_RUN_PROGRESS_FRESHNESS + Duration::from_secs(1))
                .expect("test instant supports freshness subtraction");
        }
        assert_eq!(delegated_run_heartbeat_message(base), base);
    }

    #[test]
    fn ack_race_diagnostic_states_the_persisted_board_was_kept() {
        let released = flow_ir_ack_race_diagnostic(true);
        assert!(released.contains("applied and persisted"));
        assert!(released.contains("released"));
        let kept = flow_ir_ack_race_diagnostic(false);
        assert!(kept.contains("applied and persisted board was kept"));
        assert!(kept.contains("lost response channel"));
    }

    #[test]
    fn continuation_prompt_marks_omitted_diagnostics() {
        let snapshot = WorkflowToolLoopSnapshot {
            last_status: Some("validation_errors".to_string()),
            last_errors: (0..23)
                .map(|index| format!("diagnostic number {index}"))
                .collect(),
            ..Default::default()
        };
        let prompt = build_external_workflow_continuation_prompt("build it", Some(&snapshot), 1);
        assert!(prompt.contains("Validation diagnostics (23 total)"));
        assert!(prompt.contains("diagnostic number 19"));
        assert!(!prompt.contains("diagnostic number 20"));
        assert!(prompt.contains("+3 more diagnostics omitted"));
    }

    #[test]
    fn workflow_snapshot_carries_terminal_budget_state() {
        let stalled = WorkflowToolLoopState {
            stalled_edit_attempts: MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS,
            flowscript_commit_attempts: 1,
            ..Default::default()
        };
        let snapshot = stalled.snapshot();
        assert_eq!(
            snapshot.stalled_edit_attempts,
            MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS
        );
        assert_eq!(snapshot.flowscript_commit_attempts, 1);
        assert!(
            snapshot
                .exhausted_budget
                .as_deref()
                .is_some_and(|budget| budget.contains("stalled repair progress"))
        );

        let ops_exhausted = WorkflowToolLoopState {
            flowscript_operation_attempts: MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
            ..Default::default()
        };
        assert!(
            ops_exhausted
                .snapshot()
                .exhausted_budget
                .as_deref()
                .is_some_and(|budget| budget.contains("source operation budget"))
        );

        let queued = WorkflowToolLoopState {
            queued: true,
            flowscript_operation_attempts: MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
            ..Default::default()
        };
        assert_eq!(queued.snapshot().exhausted_budget, None);

        let healthy = WorkflowToolLoopState::default();
        assert_eq!(healthy.snapshot().exhausted_budget, None);
    }

    #[test]
    fn run_summary_payload_is_built_from_a_populated_snapshot() {
        let state = WorkflowToolLoopState {
            queued: true,
            edit_attempts: 5,
            flowscript_operation_attempts: 9,
            stalled_edit_attempts: 1,
            flowscript_commit_attempts: 2,
            flowscript_draft_id: Some("draft-1".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(7),
            last_flowscript: Some("events {}".to_string()),
            last_review_notes: 3,
            last_structured_diagnostics: vec![
                serde_json::json!({ "code": "FS_TYPE_MISMATCH", "occurrences": 2 }),
                serde_json::json!({ "code": "FS_TYPE_MISMATCH" }),
                serde_json::json!({ "code": "FS_UNKNOWN_DECLARATION" }),
                serde_json::json!({ "message": "entry without a code is skipped" }),
            ],
            ..Default::default()
        };
        let snapshot = state.snapshot();
        assert_eq!(snapshot.last_review_notes, 3);

        let payload = workflow_run_summary_payload(
            "committed",
            "codex",
            "gpt-5",
            1_234,
            2,
            1,
            u32::from(MAX_EXTERNAL_WORKFLOW_CONTINUATIONS),
            Some(&snapshot),
            6,
        );
        assert_eq!(payload["kind"], "run_summary");
        assert_eq!(payload["status"], "done");
        assert_eq!(payload["outcome"], "committed");
        assert_eq!(payload["provider"], "codex");
        assert_eq!(payload["model"], "gpt-5");
        assert_eq!(payload["duration_ms"], 1_234);
        assert_eq!(payload["phases"], 2);
        assert_eq!(payload["budget"]["checks"]["used"], 5);
        assert_eq!(
            payload["budget"]["checks"]["limit"],
            u64::from(MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS)
        );
        assert_eq!(payload["budget"]["source_ops"]["used"], 9);
        assert_eq!(
            payload["budget"]["source_ops"]["limit"],
            u64::from(MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS)
        );
        assert_eq!(payload["budget"]["commits"]["used"], 2);
        assert_eq!(payload["budget"]["stalled"]["used"], 1);
        assert_eq!(payload["budget"]["continuations"]["used"], 1);
        assert_eq!(
            payload["budget"]["continuations"]["limit"],
            u64::from(MAX_EXTERNAL_WORKFLOW_CONTINUATIONS)
        );
        assert_eq!(payload["diagnostics_by_code"]["FS_TYPE_MISMATCH"], 3);
        assert_eq!(payload["diagnostics_by_code"]["FS_UNKNOWN_DECLARATION"], 1);
        assert_eq!(
            payload["diagnostics_by_code"]
                .as_object()
                .map(serde_json::Map::len),
            Some(2)
        );
        assert_eq!(payload["retained_draft"]["id"], "draft-1");
        assert_eq!(payload["retained_draft"]["revision"], 7);
        assert_eq!(payload["review_notes"], 3);
        assert_eq!(payload["applied_commands"], 6);
        // The frame must stay invisible to the process-step UIs, which key on tool_call_id.
        assert!(payload.get("tool_call_id").is_none());

        let bare =
            workflow_run_summary_payload("provider_failure", "bits", "o4", 10, 1, 0, 2, None, 0);
        assert_eq!(bare["status"], "error");
        assert!(bare["retained_draft"].is_null());
        assert_eq!(bare["budget"]["checks"]["used"], 0);
        assert_eq!(bare["review_notes"], 0);
        assert_eq!(
            bare["diagnostics_by_code"]
                .as_object()
                .map(serde_json::Map::len),
            Some(0)
        );
    }

    #[test]
    fn run_summary_review_notes_follow_the_latest_lifecycle_result() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        workflow_tool_record(
            &state,
            "commit_flowscript",
            &serde_json::json!({ "draft_id": "draft-1" }),
            &serde_json::json!({
                "status": "queued",
                "review_notes": [
                    { "code": "REVIEW_SECRET_PLACEHOLDER", "message": "fill the secret" },
                    { "code": "REVIEW_DESTRUCTIVE", "message": "removes two nodes" },
                ],
            })
            .to_string(),
        );
        let snapshot = state.lock().expect("state lock").snapshot();
        assert_eq!(snapshot.last_review_notes, 2);
        assert!(snapshot.queued);

        // A follow-up result without the field keeps the last known count.
        workflow_tool_record(
            &state,
            "check_flowscript",
            &serde_json::json!({}),
            &serde_json::json!({ "status": "valid" }).to_string(),
        );
        let snapshot = state.lock().expect("state lock").snapshot();
        assert_eq!(snapshot.last_review_notes, 2);
    }

    #[test]
    fn continuation_slice_makes_exhausted_budgets_executable_again() {
        let mut state = WorkflowToolLoopState {
            initial_declaration_lookup_complete: true,
            mutation_path: Some(WorkflowMutationPath::FlowScript),
            flowscript_draft_id: Some("mail-agent".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(4),
            stalled_edit_attempts: MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS,
            flowscript_operation_attempts: MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
            edit_attempts: MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS,
            flowscript_commit_attempts: MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS,
            ..Default::default()
        };
        state
            .flowscript_seen_repair_signatures
            .insert("stale".to_string());
        assert!(state.exhausted_budget().is_some());

        state.grant_continuation_slice();
        assert_eq!(state.exhausted_budget(), None);
        assert_eq!(state.stalled_edit_attempts, 0);
        assert!(state.flowscript_seen_repair_signatures.is_empty());
        assert_eq!(
            state.flowscript_operation_attempts,
            MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS - EXTERNAL_CONTINUATION_OPERATION_HEADROOM
        );
        assert_eq!(
            state.edit_attempts,
            MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS - EXTERNAL_CONTINUATION_CHECK_HEADROOM
        );
        assert_eq!(
            state.flowscript_commit_attempts,
            MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS - 1
        );

        // A patch on the granted slice must be dispatchable, not refused-on-arrival.
        let shared = Arc::new(StdMutex::new(state));
        assert!(
            workflow_tool_preflight_with_args(
                &shared,
                "patch_flowscript",
                &serde_json::json!({ "draft_id": "mail-agent", "expected_revision": 4 }),
            )
            .is_none()
        );

        // Queued work never hands out another slice.
        let mut queued = WorkflowToolLoopState {
            queued: true,
            stalled_edit_attempts: MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS,
            ..Default::default()
        };
        queued.grant_continuation_slice();
        assert_eq!(
            queued.stalled_edit_attempts,
            MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS
        );
    }

    #[test]
    fn sdk_idle_continuation_grants_one_slice_then_stops_the_same_budget_honestly() {
        // UI-only sessions have no workflow loop state; continuations stay executable.
        assert_eq!(
            prepare_sdk_idle_continuation_budget(None, None),
            IdleContinuationBudget::Executable
        );

        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        assert_eq!(
            prepare_sdk_idle_continuation_budget(Some(&state), None),
            IdleContinuationBudget::Executable
        );

        state.lock().expect("state lock").edit_attempts = MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS;
        let granted = prepare_sdk_idle_continuation_budget(Some(&state), None);
        let IdleContinuationBudget::SliceGranted(budget) = granted else {
            panic!("expected a granted continuation slice, got {granted:?}");
        };
        assert!(budget.contains("check budget"), "{budget}");
        // The granted slice makes the continuation instructions executable on arrival.
        assert_eq!(state.lock().expect("state lock").exhausted_budget(), None);

        // The continuation burned its slice on the same budget: no second grant, stop honestly.
        state.lock().expect("state lock").edit_attempts = MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS;
        let terminal = prepare_sdk_idle_continuation_budget(Some(&state), Some(budget.as_str()));
        let IdleContinuationBudget::Terminal(reason) = terminal else {
            panic!("expected an honest terminal outcome, got {terminal:?}");
        };
        assert!(reason.contains(&budget), "{reason}");
        assert!(reason.contains("exhausted again"), "{reason}");
    }

    #[test]
    fn parallel_flowscript_operations_are_refused_without_consuming_budget() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
            initial_declaration_lookup_complete: true,
            mutation_path: Some(WorkflowMutationPath::FlowScript),
            flowscript_draft_id: Some("mail-agent".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(2),
            flowscript_operation_attempts: 5,
            edit_attempts: 1,
            edit_in_flight: true,
            last_status: Some("valid".to_string()),
            ..Default::default()
        }));
        let args = serde_json::json!({
            "draft_id": "mail-agent",
            "expected_revision": 2,
            "source": "eventsSimple() { logInfo({ message: \"parallel\" }) }"
        });

        for tool in [
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
        ] {
            let refusal = workflow_tool_preflight_with_args(&state, tool, &args)
                .unwrap_or_else(|| panic!("{tool} must be refused while an edit is in flight"));
            assert_eq!(
                workflow_call_result_json(&refusal)["status"],
                "edit_in_flight",
                "{tool} refusal must be the cheap in-flight response"
            );
            let guard = state.lock().expect("state lock");
            assert_eq!(
                guard.flowscript_operation_attempts, 5,
                "{tool} refusal must not consume the source operation budget"
            );
            assert_eq!(
                guard.edit_attempts, 1,
                "{tool} refusal must not consume the check budget"
            );
            assert_eq!(guard.flowscript_commit_attempts, 0);
        }
    }

    #[test]
    fn structured_diagnostic_retention_ranks_root_causes_and_marks_truncation() {
        let mut diagnostics = Vec::new();
        // Emission order intentionally lists validation noise first and the parse root cause last.
        for index in 0..MAX_RETAINED_STRUCTURED_DIAGNOSTICS {
            diagnostics.push(serde_json::json!({
                "code": "FS_EXECUTION_ENTRY_UNCONNECTED",
                "phase": "validation",
                "message": format!("validation cascade {index}")
            }));
        }
        diagnostics.push(serde_json::json!({
            "code": "FS_UNKNOWN_INPUT_PIN",
            "phase": "type_check",
            "message": "unknown pin `mail_subject`"
        }));
        diagnostics.push(serde_json::json!({
            "code": "FS_PARSE_ERROR",
            "phase": "parse",
            "message": "return value 1 is not a resolvable FlowScript value"
        }));
        let payload = serde_json::json!({ "diagnostics": diagnostics });

        let retained = workflow_result_structured_diagnostics(Some(&payload));
        assert_eq!(retained.len(), MAX_RETAINED_STRUCTURED_DIAGNOSTICS + 1);
        assert_eq!(retained[0]["phase"], "parse");
        assert_eq!(retained[1]["phase"], "type_check");
        assert_eq!(retained[2]["phase"], "validation");
        let sentinel = retained
            .last()
            .expect("truncated retention appends a sentinel");
        assert_eq!(sentinel["truncated"], true);
        assert_eq!(sentinel["omitted_count"], 2);

        // One oversized item is skipped instead of ending retention for smaller later items.
        let oversized = serde_json::json!({
            "diagnostics": [
                {
                    "code": "FS_TYPE_MISMATCH",
                    "phase": "type_check",
                    "message": "x".repeat(MAX_RETAINED_STRUCTURED_DIAGNOSTIC_BYTES + 1)
                },
                {
                    "code": "FS_PARSE_ERROR",
                    "phase": "validation",
                    "message": "small trailing diagnostic"
                }
            ]
        });
        let retained = workflow_result_structured_diagnostics(Some(&oversized));
        assert!(
            retained
                .iter()
                .any(|entry| entry["message"] == "small trailing diagnostic")
        );
        assert!(
            retained
                .iter()
                .any(|entry| entry["truncated"] == true && entry["omitted_count"] == 1)
        );
    }

    #[test]
    fn unchanged_source_echo_is_replaced_with_a_retention_summary() {
        let source = "eventsSimple() {\n    logInfo({ message: \"hello\" })\n}\n";

        // write_flowscript: byte-identical response source is not re-echoed.
        let mut write_result = copilot_sdk::ToolResultObject::text(
            serde_json::json!({
                "status": "draft_written",
                "draft_id": "mail-agent",
                "revision": 3,
                "source": source
            })
            .to_string(),
        );
        suppress_unchanged_flowscript_source_echo(
            "write_flowscript",
            &serde_json::json!({ "draft_id": "mail-agent", "source": source }),
            &mut write_result,
        );
        let payload: serde_json::Value =
            serde_json::from_str(&write_result.text_result_for_llm).expect("payload stays JSON");
        assert!(payload.get("source").is_none());
        let echo = payload["source_echo"].as_str().expect("summary line");
        assert!(echo.contains("revision 3"));
        assert!(echo.contains("3 lines"));
        assert!(echo.contains(&flowscript_source_fingerprint(source)));

        // Host-normalized writes keep the full echo.
        let mut normalized_result = copilot_sdk::ToolResultObject::text(
            serde_json::json!({
                "status": "draft_written",
                "revision": 0,
                "source": format!("{source}// host-added anchor\n")
            })
            .to_string(),
        );
        suppress_unchanged_flowscript_source_echo(
            "write_flowscript",
            &serde_json::json!({ "source": source }),
            &mut normalized_result,
        );
        let payload: serde_json::Value =
            serde_json::from_str(&normalized_result.text_result_for_llm).expect("payload is JSON");
        assert!(payload["source"].as_str().is_some());

        // check_flowscript on the expected revision cannot change the source.
        let mut check_result = copilot_sdk::ToolResultObject::text(
            serde_json::json!({
                "status": "valid",
                "draft_id": "mail-agent",
                "revision": 3,
                "source": source
            })
            .to_string(),
        );
        suppress_unchanged_flowscript_source_echo(
            "check_flowscript",
            &serde_json::json!({ "draft_id": "mail-agent", "expected_revision": 3 }),
            &mut check_result,
        );
        let payload: serde_json::Value =
            serde_json::from_str(&check_result.text_result_for_llm).expect("payload is JSON");
        assert!(payload.get("source").is_none());
        assert_eq!(payload["status"], "valid");

        // A revision conflict returns a source the model has not seen; keep it.
        let mut conflict_result = copilot_sdk::ToolResultObject::text(
            serde_json::json!({
                "status": "error",
                "code": "FLOWSCRIPT_REVISION_CONFLICT",
                "revision": 4,
                "source": source
            })
            .to_string(),
        );
        suppress_unchanged_flowscript_source_echo(
            "check_flowscript",
            &serde_json::json!({ "expected_revision": 3 }),
            &mut conflict_result,
        );
        let payload: serde_json::Value =
            serde_json::from_str(&conflict_result.text_result_for_llm).expect("payload is JSON");
        assert!(payload["source"].as_str().is_some());

        // patch_flowscript results are host-computed merges and always keep the echo.
        let mut patch_result = copilot_sdk::ToolResultObject::text(
            serde_json::json!({
                "status": "draft_patched",
                "revision": 4,
                "source": source
            })
            .to_string(),
        );
        suppress_unchanged_flowscript_source_echo(
            "patch_flowscript",
            &serde_json::json!({ "expected_revision": 4 }),
            &mut patch_result,
        );
        let payload: serde_json::Value =
            serde_json::from_str(&patch_result.text_result_for_llm).expect("payload is JSON");
        assert!(payload["source"].as_str().is_some());
    }

    #[test]
    fn external_failure_recovery_is_bounded_to_transient_unqueued_work() {
        let retained = WorkflowToolLoopSnapshot {
            last_flowscript: Some("eventsSimple() { logInfo({ message: \"resume\" }) }".into()),
            flowscript_draft_id: Some("resume-source".into()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(2),
            ..Default::default()
        };
        assert_eq!(
            classify_external_agent_failure("transport stream closed unexpectedly", false),
            ExternalAgentExitKind::TransientInfrastructure
        );
        assert!(can_resume_external_workflow_after_failure(
            Some(&retained),
            "transport stream closed unexpectedly",
            false,
        ));
        assert!(!can_resume_external_workflow_after_failure(
            Some(&retained),
            "transport stream closed unexpectedly",
            true,
        ));
        assert!(!can_resume_external_workflow_after_failure(
            Some(&retained),
            "authentication failed: invalid API key",
            false,
        ));

        let declaration_only = WorkflowToolLoopSnapshot {
            last_declarations: Some("declare function logInfo({ message: string }): void;".into()),
            declaration_lookup_complete: true,
            ..Default::default()
        };
        assert!(can_resume_external_workflow_after_failure(
            Some(&declaration_only),
            "connection reset",
            false,
        ));
        assert!(can_resume_external_workflow_after_failure(
            Some(&WorkflowToolLoopSnapshot::default()),
            "connection reset before the first tool call",
            false,
        ));

        let queued = WorkflowToolLoopSnapshot {
            queued: true,
            ..retained.clone()
        };
        assert!(!can_resume_external_workflow_after_failure(
            Some(&queued),
            "connection reset",
            false,
        ));
        assert!(!can_resume_external_workflow_after_failure(
            None,
            "connection reset",
            false,
        ));
    }

    #[test]
    fn exact_source_recovery_seeds_authoritative_loop_coordinates_only() {
        use flow_like::flow::copilot::{
            FlowIrDraftRecoveryStatus, FlowScriptDraftRecovery, FlowScriptEditableDraftContext,
        };

        let context = FlowScriptEditableDraftContext {
            board_id: "recovery-board".into(),
            draft_id: "exact-source".into(),
            revision: 7,
            status: "validation_errors".into(),
            base_fingerprint: "base-1".into(),
            source: Some("eventsSimple() { brokenCall() }".into()),
            diagnostics: Vec::new(),
            checked: false,
            stale_board: false,
        };
        let exact = FlowScriptDraftRecovery {
            status: FlowIrDraftRecoveryStatus::ExactMatch,
            auto_resume: true,
            exact_match: Some(context.clone()),
            conflicting_draft: None,
            next_actions: vec!["resume_exact_flowscript_draft".into()],
            message: "resume".into(),
        };
        let state = Arc::new(StdMutex::new(
            WorkflowToolLoopState::from_flowscript_recovery(Some(&exact)),
        ));
        {
            let guard = state.lock().expect("state lock");
            assert_eq!(guard.mutation_path, Some(WorkflowMutationPath::FlowScript));
            assert_eq!(guard.flowscript_draft_id.as_deref(), Some("exact-source"));
            assert_eq!(guard.flowscript_revision, Some(7));
            assert_eq!(
                guard.last_flowscript.as_deref(),
                Some("eventsSimple() { brokenCall() }")
            );
        }

        let wrong = workflow_tool_preflight_with_args(
            &state,
            "patch_flowscript",
            &serde_json::json!({
                "draft_id": "different-source",
                "expected_revision": 7,
                "old_text": "brokenCall",
                "new_text": "logInfo"
            }),
        )
        .expect("a different source session must be rejected before dispatch");
        let payload = workflow_call_result_json(&wrong);
        assert_eq!(payload["code"], "FLOWSCRIPT_RETAINED_REVISION_REQUIRED");
        assert_eq!(payload["draft_id"], "exact-source");
        assert_eq!(payload["expected_revision"], 7);

        let stale = FlowScriptDraftRecovery {
            auto_resume: false,
            exact_match: Some(FlowScriptEditableDraftContext {
                stale_board: true,
                ..context.clone()
            }),
            ..exact.clone()
        };
        let stale_state = WorkflowToolLoopState::from_flowscript_recovery(Some(&stale));
        assert!(!stale_state.flowscript_draft_retained);
        assert!(stale_state.flowscript_draft_id.is_none());

        let mismatch = FlowScriptDraftRecovery {
            status: FlowIrDraftRecoveryStatus::RequestMismatch,
            auto_resume: false,
            exact_match: None,
            conflicting_draft: Some(FlowScriptEditableDraftContext {
                source: None,
                ..context
            }),
            next_actions: vec!["begin_separate_draft_for_current_request".into()],
            message: "belongs to another request".into(),
        };
        let mismatch_state = WorkflowToolLoopState::from_flowscript_recovery(Some(&mismatch));
        assert!(!mismatch_state.flowscript_draft_retained);
        assert!(mismatch_state.flowscript_draft_id.is_none());
        assert!(mismatch_state.last_flowscript.is_none());
    }

    #[test]
    fn source_operation_budget_counts_writes_patches_and_checks() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
            initial_declaration_lookup_complete: true,
            mutation_path: Some(WorkflowMutationPath::FlowScript),
            flowscript_draft_id: Some("bounded-source".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(0),
            last_status: Some("validation_errors".to_string()),
            ..Default::default()
        }));
        for attempt in 0..MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS {
            assert!(
                workflow_tool_preflight_with_args(
                    &state,
                    "patch_flowscript",
                    &serde_json::json!({
                        "draft_id": "bounded-source",
                        "expected_revision": 0,
                        "edits": []
                    }),
                )
                .is_none(),
                "dispatched source operation {attempt} should remain inside the hard budget"
            );
            workflow_tool_abort(&state, "patch_flowscript", "transient worker failure");
        }
        assert_eq!(
            state
                .lock()
                .expect("state lock")
                .flowscript_operation_attempts,
            MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS
        );
        let exhausted = workflow_tool_preflight_with_args(
            &state,
            "write_flowscript",
            &serde_json::json!({
                "draft_id": "bounded-source",
                "source": "eventsSimple() {}"
            }),
        )
        .expect("the operation after the hard source budget must be rejected");
        let text = exhausted
            .content
            .iter()
            .filter_map(|content| match &content.raw {
                rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("FLOWSCRIPT_OPERATION_BUDGET_EXHAUSTED"));
    }

    #[test]
    fn flowscript_repair_history_detects_a_b_a_cycles() {
        let mut state = WorkflowToolLoopState::default();
        for diagnostic in ["diagnostic A", "diagnostic B", "diagnostic A"] {
            state.last_structured_diagnostics.clear();
            state.record_flowscript_repair_progress(
                Some("validation_errors"),
                &[diagnostic.to_string()],
                true,
            );
        }
        assert_eq!(
            state.stalled_edit_attempts, 1,
            "returning to an older compiler state is not fresh progress"
        );
    }

    #[test]
    fn decreasing_grouped_diagnostic_occurrences_count_as_repair_progress() {
        let mut state = WorkflowToolLoopState::default();
        for occurrences in [8, 6, 4] {
            state.last_structured_diagnostics = vec![serde_json::json!({
                "code": "FS_INPUT_PIN_NOT_FOUND",
                "message": "boolOr has no input pin named a",
                "pin": "a",
                "occurrences": occurrences
            })];
            state.record_flowscript_repair_progress(
                Some("validation_errors"),
                &["[FS_INPUT_PIN_NOT_FOUND] boolOr has no input pin named a".to_string()],
                true,
            );
            assert_eq!(state.stalled_edit_attempts, 0);
        }

        state.record_flowscript_repair_progress(
            Some("validation_errors"),
            &["[FS_INPUT_PIN_NOT_FOUND] boolOr has no input pin named a".to_string()],
            true,
        );
        assert_eq!(state.stalled_edit_attempts, 1);
    }

    #[test]
    fn continuation_diagnostics_keep_root_repair_fields_and_drop_cascades() {
        let payload = serde_json::json!({
            "status": "validation_errors",
            "diagnostics": [
                {
                    "id": "root-1",
                    "code": "FS_PIN_TYPE",
                    "message": "pin has the wrong type",
                    "source_span": { "line": 4, "column": 9 },
                    "pin": "message",
                    "expected": "string",
                    "actual": "Struct",
                    "caused_by": [],
                    "fix": { "summary": "Use the declared text output." }
                },
                {
                    "id": "cascade-1",
                    "code": "FS_EXECUTION_TAIL",
                    "message": "execution continuation is missing",
                    "caused_by": "root-1"
                }
            ]
        });
        assert_eq!(
            workflow_result_diagnostics(Some(&payload)),
            ["[FS_PIN_TYPE] pin has the wrong type"]
        );
        let structured = workflow_result_structured_diagnostics(Some(&payload));
        assert_eq!(structured.len(), 1);
        assert_eq!(structured[0]["pin"], "message");
        assert_eq!(structured[0]["expected"], "string");
        assert_eq!(structured[0]["actual"], "Struct");
        assert_eq!(structured[0]["source_span"]["line"], 4);
        assert_eq!(
            structured[0]["fix"]["summary"],
            "Use the declared text output."
        );
    }

    #[test]
    fn repair_declarations_survive_transient_source_failures_and_clear_on_valid() {
        let exact =
            "declare function emailSmtpSend({ connection: Struct, from: string, to: string, bodyText: string }): void;"
                .to_string();
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
            last_repair_declarations: vec![exact.clone()],
            ..Default::default()
        }));

        workflow_tool_record(
            &state,
            "patch_flowscript",
            &serde_json::json!({ "draft_id": "support-flow", "expected_revision": 3 }),
            &serde_json::json!({
                "status": "revision_conflict",
                "message": "The retained revision advanced before this patch."
            })
            .to_string(),
        );
        assert_eq!(
            state.lock().expect("state lock").last_repair_declarations,
            [exact],
            "a transient response without replacement fixes must retain exact repair signatures"
        );

        workflow_tool_record(
            &state,
            "check_flowscript",
            &serde_json::json!({ "draft_id": "support-flow", "expected_revision": 4 }),
            &serde_json::json!({ "status": "valid", "diagnostics": [] }).to_string(),
        );
        assert!(
            state
                .lock()
                .expect("state lock")
                .last_repair_declarations
                .is_empty(),
            "a valid source no longer needs stale repair signatures"
        );
    }

    #[test]
    fn source_lifecycle_results_retain_exact_revision_for_repair() {
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
        assert!(
            workflow_tool_preflight_with_args(
                &state,
                "get_declarations",
                &serde_json::json!({ "queries": ["support email workflow"] }),
            )
            .is_none()
        );
        workflow_tool_record(
            &state,
            "get_declarations",
            &serde_json::json!({ "queries": ["support email workflow"] }),
            "declare function emailImapConnect({ host: string }): Struct;",
        );
        assert!(
            workflow_tool_preflight_with_args(
                &state,
                "write_flowscript",
                &serde_json::json!({
                    "draft_id": "support-flow",
                    "source": rich_support_flowscript(),
                }),
            )
            .is_none()
        );
        workflow_tool_record(
            &state,
            "write_flowscript",
            &serde_json::json!({
                "draft_id": "support-flow",
                "source": rich_support_flowscript(),
            }),
            &serde_json::json!({
                "status": "validation_errors",
                "draft_id": "support-flow",
                "revision": 0,
                "source": rich_support_flowscript(),
                "diagnostics": [{
                    "code": "FS_UNKNOWN_DECLARATION",
                    "message": "unknown declaration sendReply",
                    "fix": {
                        "summary": "Use the live SMTP declaration.",
                        "catalog_declarations": [
                            "declare function emailSmtpSend(connection: Struct, to: string, bodyText: string): void"
                        ],
                        "companion_declarations": [
                            "declare function emailSmtpConnect(host: string, port: int): (connection: Struct)"
                        ]
                    }
                }]
            })
            .to_string(),
        );

        let snapshot = state.lock().expect("state lock").snapshot();
        assert_eq!(
            snapshot.mutation_path,
            Some(WorkflowMutationPath::FlowScript)
        );
        assert!(snapshot.flowscript_draft_retained);
        assert_eq!(
            snapshot.flowscript_draft_id.as_deref(),
            Some("support-flow")
        );
        assert_eq!(snapshot.flowscript_revision, Some(0));
        assert_eq!(
            snapshot.last_flowscript.as_deref(),
            Some(rich_support_flowscript())
        );
        assert!(snapshot.last_errors[0].contains("FS_UNKNOWN_DECLARATION"));
        assert_eq!(
            snapshot.last_repair_declarations,
            [
                "declare function emailSmtpSend(connection: Struct, to: string, bodyText: string): void",
                "declare function emailSmtpConnect(host: string, port: int): (connection: Struct)",
            ]
        );
    }

    #[test]
    fn checked_valid_source_remains_committable_at_repair_budget_ceiling() {
        let valid = Arc::new(StdMutex::new(WorkflowToolLoopState {
            initial_declaration_lookup_complete: true,
            mutation_path: Some(WorkflowMutationPath::FlowScript),
            edit_attempts: MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS,
            flowscript_draft_id: Some("valid-source".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(7),
            last_flowscript: Some("eventsSimple() {}".to_string()),
            last_status: Some("valid".to_string()),
            ..Default::default()
        }));
        assert!(
            workflow_tool_preflight_with_args(
                &valid,
                "commit_flowscript",
                &serde_json::json!({
                    "draft_id": "valid-source",
                    "expected_revision": 7
                }),
            )
            .is_none()
        );

        let invalid = Arc::new(StdMutex::new(WorkflowToolLoopState {
            initial_declaration_lookup_complete: true,
            mutation_path: Some(WorkflowMutationPath::FlowScript),
            edit_attempts: MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS,
            flowscript_draft_id: Some("invalid-source".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(2),
            last_status: Some("validation_errors".to_string()),
            last_errors: vec!["missing execution edge".to_string()],
            ..Default::default()
        }));
        assert!(workflow_tool_preflight(&invalid, "check_flowscript").is_some());
        assert!(workflow_tool_preflight(&invalid, "commit_flowscript").is_some());
    }

    #[test]
    fn transient_commit_store_failures_preserve_valid_revision_for_bounded_retry() {
        let args = serde_json::json!({
            "draft_id": "valid-source",
            "expected_revision": 7
        });
        let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
            initial_declaration_lookup_complete: true,
            mutation_path: Some(WorkflowMutationPath::FlowScript),
            edit_attempts: MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS,
            flowscript_operation_attempts: MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
            flowscript_draft_id: Some("valid-source".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(7),
            last_flowscript: Some("eventsSimple() {}".to_string()),
            last_status: Some("valid".to_string()),
            ..Default::default()
        }));

        for attempt in 0..MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS {
            assert!(
                workflow_tool_preflight_with_args(&state, "commit_flowscript", &args).is_none(),
                "valid commit retry {attempt} should bypass the exhausted repair budget"
            );
            workflow_tool_record(
                &state,
                "commit_flowscript",
                &args,
                &serde_json::json!({
                    "status": "error",
                    "code": "FLOWSCRIPT_DRAFT_STORE_UNAVAILABLE",
                    "message": "FlowScript draft store lock is unavailable"
                })
                .to_string(),
            );
            assert_eq!(
                state.lock().expect("state lock").last_status.as_deref(),
                Some("valid")
            );
        }

        let exhausted = workflow_tool_preflight_with_args(&state, "commit_flowscript", &args)
            .expect("the bounded commit retry cap must terminate store failures");
        assert_eq!(
            workflow_call_result_json(&exhausted)["code"],
            "FLOWSCRIPT_COMMIT_RETRY_BUDGET_EXHAUSTED"
        );
    }

    #[test]
    fn retained_source_snapshot_prefers_latest_valid_revision_over_older_failure() {
        let mut state = WorkflowToolLoopState {
            mutation_path: Some(WorkflowMutationPath::FlowScript),
            flowscript_draft_id: Some("latest-source".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(2),
            last_flowscript: Some("eventsSimple() { logInfo({ message: \"valid\" }) }".to_string()),
            last_status: Some("valid".to_string()),
            ..Default::default()
        };
        state
            .repair_tracker
            .record_failed(rich_support_flowscript());

        let snapshot = state.snapshot();
        assert_eq!(snapshot.last_status.as_deref(), Some("valid"));
        assert_eq!(
            snapshot.last_flowscript.as_deref(),
            Some("eventsSimple() { logInfo({ message: \"valid\" }) }")
        );
    }

    #[test]
    fn workflow_snapshot_retains_a_draft_interrupted_during_validation() {
        let mut state = WorkflowToolLoopState {
            edit_attempts: 1,
            edit_in_flight: true,
            in_flight_flowscript: Some(rich_support_flowscript().to_string()),
            ..Default::default()
        };

        let snapshot = state.snapshot();
        assert_eq!(
            snapshot.last_flowscript.as_deref(),
            Some(rich_support_flowscript())
        );
        assert_eq!(snapshot.last_status.as_deref(), Some("edit_interrupted"));
        assert_eq!(
            snapshot.retained_full_source.as_deref(),
            Some(rich_support_flowscript())
        );

        state.finish_interrupted_phase();
        assert!(!state.edit_in_flight);
        assert!(
            state
                .repair_tracker
                .queued_candidate_regression(
                    "eventsSimple() {\n    logInfo({ message: \"test\" })\n}"
                )
                .is_some(),
            "a phase boundary must not allow a tiny valid draft to replace the retained workflow"
        );
        let snapshot = state.snapshot();
        assert_eq!(
            snapshot.last_flowscript.as_deref(),
            Some(rich_support_flowscript())
        );
        assert_eq!(snapshot.last_status.as_deref(), Some("validation_errors"));
        assert!(
            snapshot
                .last_errors
                .iter()
                .any(|error| error.contains("interrupted"))
        );
    }

    #[test]
    fn interrupted_typed_phase_reports_only_proven_recovery_state() {
        let mut retained = WorkflowToolLoopState {
            edit_in_flight: true,
            mutation_path: Some(WorkflowMutationPath::TypedIr),
            typed_draft_id: Some("resume-me".to_string()),
            typed_draft_retained: true,
            typed_revision: Some(7),
            ..Default::default()
        };
        retained.finish_interrupted_phase();
        assert!(retained.last_errors[0].contains("resume-me"));
        assert!(retained.last_errors[0].contains("revision 7"));

        let mut unconfirmed = WorkflowToolLoopState {
            edit_in_flight: true,
            mutation_path: Some(WorkflowMutationPath::TypedIr),
            typed_draft_id: Some("attempted-only".to_string()),
            ..Default::default()
        };
        unconfirmed.finish_interrupted_phase();
        assert!(unconfirmed.last_errors[0].contains("before draft retention could be confirmed"));
        assert!(!unconfirmed.last_errors[0].contains("remains resumable"));
    }

    #[test]
    fn interrupted_source_phase_reports_the_retained_revision() {
        let mut state = WorkflowToolLoopState {
            edit_in_flight: true,
            mutation_path: Some(WorkflowMutationPath::FlowScript),
            flowscript_draft_id: Some("resume-source".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(5),
            last_flowscript: Some(rich_support_flowscript().to_string()),
            ..Default::default()
        };
        state.finish_interrupted_phase();
        assert!(state.last_errors[0].contains("resume-source"));
        assert!(state.last_errors[0].contains("revision 5"));
    }

    #[test]
    fn mcp_marks_semantic_edit_failures_as_errors() {
        for status in ["validation_errors", "no_changes", "error"] {
            let result = flowpilot_tool_result_to_mcp(copilot_sdk::ToolResultObject::text(
                serde_json::json!({ "status": status, "next_action": "revise_and_resubmit" })
                    .to_string(),
            ));
            assert_eq!(
                result.is_error,
                Some(true),
                "semantic status {status} must be a red MCP tool result"
            );
        }

        let queued = flowpilot_tool_result_to_mcp(copilot_sdk::ToolResultObject::text(
            serde_json::json!({ "status": "queued", "next_action": "stop" }).to_string(),
        ));
        assert_eq!(queued.is_error, Some(false));
    }

    #[test]
    fn provider_exit_recovery_only_uses_successful_mutating_platform_tools() {
        assert!(is_recoverable_platform_mutation("flowpilot_board"));
        assert!(is_recoverable_platform_mutation("create_app"));
        assert!(!is_recoverable_platform_mutation("list_apps"));
        assert!(!is_recoverable_platform_mutation("get_declarations"));

        let success = copilot_sdk::ToolResultObject::text(
            serde_json::json!({ "status": "ok", "applied_commands": 65 }).to_string(),
        );
        assert!(!flowpilot_tool_result_is_error(&success));

        let failure = copilot_sdk::ToolResultObject::text(
            serde_json::json!({ "status": "validation_errors" }).to_string(),
        );
        assert!(flowpilot_tool_result_is_error(&failure));
    }

    #[test]
    fn recovered_mutation_message_preserves_result_and_redacts_secrets() {
        let completion = McpToolCompletion {
            tool_name: "flowpilot_board".to_string(),
            result_text: serde_json::json!({
                "status": "ok",
                "message": "Workflow persisted",
                "applied_commands": 65,
                "password": "must-not-leak",
            })
            .to_string(),
        };

        let message = render_recovered_mutation_message(&completion);
        assert!(message.contains("flowpilot_board"));
        assert!(message.contains("completed successfully"));
        assert!(message.contains("Workflow persisted"));
        assert!(message.contains("<redacted>"));
        assert!(!message.contains("must-not-leak"));
    }

    #[test]
    fn model_selection_routes_agent_backend_prefixes() {
        let github = FlowPilotModelSelection::parse(Some("github-copilot:gpt-5-mini".to_string()));
        assert_eq!(
            github.backend,
            FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::GithubCopilot)
        );
        assert_eq!(github.model_id.as_deref(), Some("gpt-5-mini"));

        let legacy = FlowPilotModelSelection::parse(Some("copilot:claude".to_string()));
        assert_eq!(
            legacy.backend,
            FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::GithubCopilot)
        );
        assert_eq!(legacy.model_id.as_deref(), Some("claude"));

        let codex = FlowPilotModelSelection::parse(Some("codex:default".to_string()));
        assert_eq!(
            codex.backend,
            FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::Codex)
        );

        let claude = FlowPilotModelSelection::parse(Some("claude-code:default".to_string()));
        assert_eq!(
            claude.backend,
            FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::ClaudeCode)
        );
    }

    #[test]
    fn persisted_tool_previews_redact_credentials_recursively() {
        let arguments = serde_json::json!({
            "operation": "connect",
            "password": "visible-password-must-not-leak",
            "nested": {
                "access_token": "visible-token-must-not-leak",
                "api_key": "visible-key-must-not-leak",
            },
        });
        let preview = preview_tool_arguments("future_tool", Some(&arguments));
        assert!(preview.contains("<redacted>"));
        assert!(!preview.contains("visible-password-must-not-leak"));
        assert!(!preview.contains("visible-token-must-not-leak"));
        assert!(!preview.contains("visible-key-must-not-leak"));

        let result_preview = preview_tool_result(&arguments.to_string());
        assert!(result_preview.contains("<redacted>"));
        assert!(!result_preview.contains("visible-password-must-not-leak"));
    }

    #[test]
    fn flowscript_and_non_json_previews_keep_safe_debug_content() {
        let flowscript = r#"function pollSupportInbox() {
    const password = "must-not-leak"
    logInfo({ message: "polling support inbox" })
}"#;
        let arguments = serde_json::json!({ "flowscript": flowscript });
        let arguments_preview = preview_tool_arguments("edit_flowscript", Some(&arguments));
        assert!(arguments_preview.contains("pollSupportInbox"));
        assert!(arguments_preview.contains("logInfo"));
        assert!(arguments_preview.contains("<redacted>"));
        assert!(!arguments_preview.contains("must-not-leak"));

        let result_preview = preview_tool_result(
            "validation failed: pollSupportInbox has no Done edge; token=must-not-leak",
        );
        assert!(result_preview.contains("validation failed"));
        assert!(result_preview.contains("pollSupportInbox"));
        assert!(result_preview.contains("token=<redacted>"));
        assert!(!result_preview.contains("must-not-leak"));
    }

    #[test]
    fn nested_stream_frames_carry_parent_request_id_without_touching_text() {
        let frame = flowpilot_stream_tag(
            "tool_start",
            &serde_json::json!({
                "tool_call_id": "child-tool-1",
                "tool": "edit_flowscript",
                "arguments_preview": "safe preview",
            }),
        );
        let correlated = correlate_stream_frame(&frame, Some("flowpilot-tool-parent-1"));
        assert!(correlated.contains("\"parent_request_id\":\"flowpilot-tool-parent-1\""));
        assert!(correlated.contains("\"tool_call_id\":\"child-tool-1\""));
        assert_eq!(
            correlate_stream_frame("assistant text", Some("flowpilot-tool-parent-1")),
            "assistant text"
        );

        let workspace = flowpilot_stream_tag(
            "flowscript_workspace",
            &serde_json::json!({
                "source": "eventsSimple() { logInfo({ message: \"live\" }) }",
                "status": "drafting",
                "tool_call_id": "child-tool-1",
            }),
        );
        let correlated_workspace =
            correlate_stream_frame(&workspace, Some("flowpilot-tool-parent-1"));
        assert!(correlated_workspace.contains("\"parent_request_id\":\"flowpilot-tool-parent-1\""));
        assert!(correlated_workspace.contains("\"status\":\"drafting\""));
        assert!(correlated_workspace.contains("logInfo"));
    }

    #[test]
    fn correlated_payload_preserves_an_existing_parent_id() {
        let payload = serde_json::json!({
            "tool_call_id": "child-tool-1",
            "parent_request_id": "authoritative-parent",
        });
        let correlated = correlated_stream_payload(&payload, Some("fallback-parent"));
        assert_eq!(
            correlated
                .get("parent_request_id")
                .and_then(serde_json::Value::as_str),
            Some("authoritative-parent")
        );
    }

    #[test]
    fn model_selection_keeps_bits_model_ids_unprefixed() {
        let selection = FlowPilotModelSelection::parse(Some("hub:model".to_string()));
        assert_eq!(selection.backend, FlowPilotChatBackend::Bits);
        assert_eq!(selection.model_id.as_deref(), Some("hub:model"));
    }

    #[test]
    fn board_runtime_bridge_uses_scoped_specs_before_context_injection() {
        use flow_like::flow::copilot::tool_spec::{
            ARCHIVE_LOOKUP_TOOL, INTERNET_SEARCH_TOOL, OPEN_URL_TOOL, ToolApprovalSpec,
            missing_required_args,
        };

        for global_only_tool in [INTERNET_SEARCH_TOOL, OPEN_URL_TOOL, ARCHIVE_LOOKUP_TOOL] {
            let scope_error = global_orchestrator_tool_scope_error(
                FrontendPlatformToolSet::BoardRuntime,
                global_only_tool,
            )
            .expect("board runtime must reject global-only tools before execution");
            assert!(scope_error.contains("global_orchestrator_tool_only"));
            assert!(
                global_orchestrator_tool_scope_error(
                    FrontendPlatformToolSet::Global,
                    global_only_tool
                )
                .is_none()
            );
            assert!(
                frontend_platform_tool_spec(
                    FrontendPlatformToolSet::BoardRuntime,
                    global_only_tool
                )
                .is_none(),
                "board runtime must not expose global-only tool {global_only_tool}"
            );
            assert!(
                frontend_platform_tool_spec(FrontendPlatformToolSet::Global, global_only_tool)
                    .is_some(),
                "global FlowPilot must expose {global_only_tool}"
            );
        }

        for name in ["execute_event", "execute_node", "query_execution_logs"] {
            assert!(
                frontend_platform_tool_spec(FrontendPlatformToolSet::BoardRuntime, name).is_some(),
                "Bits board bridge must expose the shared runtime spec for {name}"
            );
        }

        let execute_node =
            frontend_platform_tool_spec(FrontendPlatformToolSet::BoardRuntime, "execute_node")
                .expect("board-scoped execute_node spec");
        let node_args = serde_json::json!({ "board_id": "board", "node_id": "node" });
        assert!(
            missing_required_args(&execute_node, &node_args).is_none(),
            "app_id is supplied by FrontendToolContext after scoped validation"
        );
        assert!(matches!(
            execute_node.approval,
            ToolApprovalSpec::Execute { .. }
        ));

        let global_execute_node =
            frontend_platform_tool_spec(FrontendPlatformToolSet::Global, "execute_node")
                .expect("global execute_node spec");
        assert!(
            missing_required_args(&global_execute_node, &node_args)
                .is_some_and(|error| error.contains("app_id")),
            "the global schema must not accidentally validate board-scoped calls"
        );

        let query_logs = frontend_platform_tool_spec(
            FrontendPlatformToolSet::BoardRuntime,
            "query_execution_logs",
        )
        .expect("board-scoped log-query spec");
        assert!(
            missing_required_args(
                &query_logs,
                &serde_json::json!({ "board_id": "board", "run_id": "run" }),
            )
            .is_none()
        );
        assert!(matches!(query_logs.approval, ToolApprovalSpec::None));
    }

    #[test]
    fn specialist_agent_capability_set_covers_app_tools_without_global_web_tools() {
        let capabilities = FlowPilotAgentCapabilitySet::shared_for(CopilotScope::Both, true, true);
        for tool in [
            "get_declarations",
            "get_current_flowscript",
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
            "emit_commands",
            "emit_ui",
            "database_tool",
            "storage_tool",
            "execute_event",
            "execute_node",
            "query_execution_logs",
            "ask_user",
        ] {
            assert!(
                capabilities.tool_names.iter().any(|name| name == tool),
                "shared FlowPilot capability set must include {tool}; got {:?}",
                capabilities.tool_names
            );
        }
        for global_only_tool in ["internet_search", "open_url", "archive_lookup"] {
            assert!(
                !capabilities
                    .tool_names
                    .iter()
                    .any(|name| name == global_only_tool),
                "specialist capabilities must not advertise global-only tool {global_only_tool}"
            );
        }
        for legacy_typed_tool in [
            "plan_flow_ir",
            "begin_flow_ir_draft",
            "update_flow_ir_draft",
            "upsert_flow_ir_module",
            "validate_flow_ir_draft",
            "commit_flow_ir_draft",
        ] {
            assert!(
                !capabilities
                    .tool_names
                    .iter()
                    .any(|name| name == legacy_typed_tool),
                "model-facing capabilities must not advertise legacy typed JSON tool {legacy_typed_tool}"
            );
        }
        assert_eq!(
            capabilities.prompt_source, "flow_like::copilot::prompts",
            "all agent backends must use the shared prompt module"
        );
    }

    #[test]
    fn global_agent_capability_set_advertises_public_web_tools() {
        let capabilities =
            FlowPilotAgentCapabilitySet::for_surface(CopilotScope::Both, true, true, true);
        for tool in ["internet_search", "open_url", "archive_lookup"] {
            assert!(
                capabilities.tool_names.iter().any(|name| name == tool),
                "global FlowPilot capabilities must advertise {tool}"
            );
        }

        for scope in [
            CopilotScope::Board,
            CopilotScope::Frontend,
            CopilotScope::Both,
            CopilotScope::DataStudio,
        ] {
            let specialist = FlowPilotAgentCapabilitySet::for_surface(scope, true, true, false);
            for tool in ["internet_search", "open_url", "archive_lookup"] {
                assert!(
                    !specialist.tool_names.iter().any(|name| name == tool),
                    "specialist surface {scope:?} must not advertise global-only tool {tool}"
                );
            }
        }
    }

    #[test]
    fn scope_neutral_backend_status_does_not_claim_global_web_tools() {
        let capabilities =
            FlowPilotAgentCapabilitySet::for_status(FlowPilotAgentTransportKind::DirectSdkTools);
        for tool in ["internet_search", "open_url", "archive_lookup"] {
            assert!(
                !capabilities.tool_names.iter().any(|name| name == tool),
                "scope-neutral backend status must not advertise global-only tool {tool}"
            );
        }
    }

    #[test]
    fn read_only_tool_filter_covers_every_workflow_mutator() {
        for tool in [
            "emit_commands",
            "edit_flowscript",
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
            "begin_flow_ir_draft",
            "update_flow_ir_draft",
            "upsert_flow_ir_module",
            "validate_flow_ir_draft",
            "commit_flow_ir_draft",
            "emit_ui",
        ] {
            assert!(
                is_flowpilot_mutation_tool(tool),
                "read-only FlowPilot surfaces must hide {tool}"
            );
        }
        for tool in [
            "catalog_search",
            "get_declarations",
            "plan_flow_ir",
            "get_current_flowscript",
            "get_node_details",
            "list_board_nodes",
        ] {
            assert!(
                !is_flowpilot_mutation_tool(tool),
                "read-only inspection should retain {tool}"
            );
        }
    }

    #[test]
    fn source_lifecycle_classification_keeps_commit_boundary_explicit() {
        for tool in [
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
        ] {
            assert!(is_workflow_loop_tool(tool));
            assert!(is_flowscript_draft_operation_tool(tool));
            assert!(is_order_sensitive_workflow_tool(tool));
        }
        assert!(!is_workflow_commit_tool("write_flowscript"));
        assert!(!is_workflow_commit_tool("patch_flowscript"));
        assert!(!is_workflow_commit_tool("check_flowscript"));
        assert!(is_workflow_commit_tool("commit_flowscript"));
        assert!(is_workflow_commit_tool("edit_flowscript"));
    }

    #[test]
    fn codex_invocation_uses_streamable_http_mcp_server() {
        let invocation = ExternalAgentInvocation::new(
            FlowPilotAgentBackendKind::Codex,
            CliResolution::new(
                std::path::PathBuf::from("/usr/bin/codex"),
                CliResolutionSource::Path,
            ),
            "default",
            None,
            "http://127.0.0.1:12345/mcp",
            "hello".to_string(),
            vec!["edit_flowscript".to_string()],
            &[],
        )
        .expect("codex invocation should build");

        assert_eq!(invocation.backend, FlowPilotAgentBackendKind::Codex);
        assert!(invocation.args.contains(&"exec".to_string()));
        assert!(invocation.args.contains(&"--experimental-json".to_string()));
        assert!(
            invocation
                .args
                .contains(&"--ignore-user-config".to_string())
        );
        assert!(
            invocation
                .args
                .contains(&"--skip-git-repo-check".to_string())
        );
        let cd_index = invocation
            .args
            .iter()
            .position(|arg| arg == "--cd")
            .expect("Codex invocation should set a neutral working directory");
        assert_eq!(
            invocation.args.get(cd_index + 1),
            Some(&std::env::temp_dir().display().to_string()),
            "Codex must not inspect an incidental protected desktop working directory"
        );
        assert!(invocation.args.contains(&"--config".to_string()));
        assert!(
            !invocation.args.contains(&"--model".to_string()),
            "the \"default\" model selection must defer to Codex's configured runtime model by omitting --model: {:?}",
            invocation.args
        );
        assert!(
            !invocation
                .args
                .iter()
                .any(|arg| arg.starts_with("model_reasoning_effort=")),
            "an omitted effort must preserve Codex's configured default: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .args
                .windows(2)
                .any(|args| args == ["--sandbox", "read-only"]),
            "codex invocation should keep FlowPilot workspace edits in MCP tools, not shell writes: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .args
                .iter()
                .any(|arg| arg.contains("mcp_servers.flowpilot.url=")
                    && arg.contains("127.0.0.1:12345/mcp")),
            "codex args should contain MCP URL: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .args
                .iter()
                .any(|arg| arg == "mcp_servers.flowpilot.default_tools_approval_mode=\"approve\""),
            "codex exec must explicitly approve the session-local FlowPilot MCP tools in headless mode: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .args
                .iter()
                .any(|arg| arg == "approval_policy=\"never\""),
            "codex invocation should run non-interactively through FlowPilot approvals/tools"
        );
        assert!(invocation.prompt.contains("hello"));
    }

    #[test]
    fn codex_invocation_isolates_native_and_user_config_web_tools() {
        for tool_names in [
            // Nested specialist surface: no public-web MCP tools.
            vec!["edit_flowscript".to_string()],
            // Global orchestrator surface: public research is available only through these
            // reviewed FlowPilot MCP tools, never through Codex's native web-search tool.
            vec!["internet_search".to_string(), "open_url".to_string()],
        ] {
            let invocation = ExternalAgentInvocation::new(
                FlowPilotAgentBackendKind::Codex,
                CliResolution::new(
                    std::path::PathBuf::from("/usr/bin/codex"),
                    CliResolutionSource::Path,
                ),
                "default",
                None,
                "http://127.0.0.1:12345/mcp",
                "hello".to_string(),
                tool_names,
                &[],
            )
            .expect("codex invocation should build");

            let native_web_disable_overrides = invocation
                .args
                .windows(2)
                .filter(|args| *args == ["--config", "web_search=\"disabled\""])
                .count();
            assert_eq!(
                native_web_disable_overrides, 1,
                "every FlowPilot Codex invocation must override user config and force public-web access through the scoped MCP surface: {:?}",
                invocation.args
            );
            assert_eq!(
                invocation
                    .args
                    .iter()
                    .filter(|arg| arg.as_str() == "--ignore-user-config")
                    .count(),
                1,
                "every FlowPilot Codex invocation must exclude user-configured MCP/browser tools while retaining CODEX_HOME auth: {:?}",
                invocation.args
            );
        }
    }

    #[test]
    fn codex_invocation_forwards_selected_model() {
        let invocation = ExternalAgentInvocation::new(
            FlowPilotAgentBackendKind::Codex,
            CliResolution::new(
                std::path::PathBuf::from("/usr/bin/codex"),
                CliResolutionSource::Path,
            ),
            "gpt-5.5",
            Some("xhigh"),
            "http://127.0.0.1:12345/mcp",
            "hello".to_string(),
            vec!["edit_flowscript".to_string()],
            &[],
        )
        .expect("codex invocation should build");

        assert!(
            invocation
                .args
                .windows(2)
                .any(|args| args == ["--model", "gpt-5.5"]),
            "a discovered Codex model id must be forwarded via --model: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .args
                .windows(2)
                .any(|args| args == ["--config", "model_reasoning_effort=\"xhigh\""]),
            "the selected Codex reasoning effort must be forwarded as a config override: {:?}",
            invocation.args
        );
    }

    #[test]
    fn parse_codex_model_catalog_maps_and_filters() {
        let entries = vec![
            serde_json::json!({
                "id": "gpt-5.5",
                "displayName": "GPT-5.5",
                "hidden": false,
                "isDefault": true,
                "supportedReasoningEfforts": [
                    {
                        "reasoningEffort": "low",
                        "description": "Fast responses with lighter reasoning"
                    },
                    {
                        "reasoningEffort": "xhigh",
                        "description": "Extra high reasoning depth"
                    }
                ],
                "defaultReasoningEffort": "low"
            }),
            serde_json::json!({ "model": "gpt-5.4-mini", "hidden": false }),
            serde_json::json!({ "id": "internal", "displayName": "Internal", "hidden": true }),
            serde_json::json!({ "displayName": "No id here" }),
        ];

        let models = parse_codex_model_catalog(&entries);

        assert_eq!(models.len(), 2, "hidden and id-less entries are dropped");
        assert_eq!(models[0].id, "gpt-5.5");
        assert_eq!(models[0].name, "GPT-5.5");
        assert!(models[0].is_default);
        assert_eq!(models[0].default_reasoning_effort.as_deref(), Some("low"));
        assert_eq!(
            models[0].supported_reasoning_efforts,
            vec![
                ReasoningEffortOption {
                    id: "low".to_string(),
                    name: "Low".to_string(),
                    description: Some("Fast responses with lighter reasoning".to_string()),
                },
                ReasoningEffortOption {
                    id: "xhigh".to_string(),
                    name: "Extra high".to_string(),
                    description: Some("Extra high reasoning depth".to_string()),
                },
            ]
        );
        assert_eq!(models[1].id, "gpt-5.4-mini");
        assert_eq!(
            models[1].name, "gpt-5.4-mini",
            "displayName falls back to the model id"
        );

        let serialized = serde_json::to_value(&models[0]).expect("model DTO serializes");
        assert!(serialized.get("supportedReasoningEfforts").is_some());
        assert_eq!(serialized["defaultReasoningEffort"], "low");
        assert_eq!(serialized["isDefault"], true);
        assert!(serialized.get("supported_reasoning_efforts").is_none());

        let with_default = codex_models_with_configured_default(models);
        assert_eq!(with_default[0].id, "default");
        assert_eq!(
            with_default[0].default_reasoning_effort.as_deref(),
            Some("low")
        );
        assert_eq!(
            with_default[0].supported_reasoning_efforts,
            with_default[1].supported_reasoning_efforts,
            "the configured-default sentinel must inherit the runtime default's effort metadata"
        );
    }

    #[test]
    fn parse_claude_model_catalog_maps_and_dedupes() {
        let entries = vec![
            serde_json::json!({
                "value": "default",
                "resolvedModel": "claude-opus-4-8[1m]",
                "displayName": "Default (recommended)",
                "supportsEffort": true,
                "supportedEffortLevels": ["low", "medium", "high", "max"]
            }),
            serde_json::json!({
                "value": "sonnet",
                "displayName": "Sonnet",
                "supportsEffort": true,
                "supportedEffortLevels": ["low", "high"]
            }),
            serde_json::json!({ "value": "sonnet", "displayName": "Sonnet duplicate" }),
            serde_json::json!({
                "value": "claude-fable-5[1m]",
                "supportsEffort": false,
                "supportedEffortLevels": ["low"]
            }),
            serde_json::json!({ "displayName": "No value here" }),
        ];

        let models = parse_claude_model_catalog(&entries);

        assert_eq!(
            models.len(),
            3,
            "duplicate value and value-less entries drop"
        );
        assert_eq!(models[0].id, "default");
        assert_eq!(models[0].name, "Default (recommended)");
        assert!(models[0].is_default);
        assert_eq!(
            models[0]
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high", "max"]
        );
        assert_eq!(models[1].id, "sonnet");
        assert_eq!(
            models[1]
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["low", "high"]
        );
        assert_eq!(
            models[2].id, "claude-fable-5[1m]",
            "bracketed model ids are passed through verbatim for --model"
        );
        assert_eq!(
            models[2].name, "claude-fable-5[1m]",
            "displayName falls back to the value"
        );
        assert!(
            models[2].supported_reasoning_efforts.is_empty(),
            "supportsEffort=false must suppress stale level metadata"
        );
    }

    #[test]
    fn claude_invocation_uses_shared_mcp_config() {
        let invocation = ExternalAgentInvocation::new(
            FlowPilotAgentBackendKind::ClaudeCode,
            CliResolution::new(
                std::path::PathBuf::from("/usr/bin/claude"),
                CliResolutionSource::Path,
            ),
            "sonnet",
            Some("max"),
            "http://127.0.0.1:23456/mcp",
            "hello".to_string(),
            vec![
                "get_declarations".to_string(),
                "write_flowscript".to_string(),
                "patch_flowscript".to_string(),
                "check_flowscript".to_string(),
                "commit_flowscript".to_string(),
            ],
            &[],
        )
        .expect("claude invocation should build");

        assert_eq!(invocation.backend, FlowPilotAgentBackendKind::ClaudeCode);
        assert!(invocation.args.contains(&"--mcp-config".to_string()));
        assert!(invocation.args.contains(&"stream-json".to_string()));
        assert!(invocation.args.contains(&"--strict-mcp-config".to_string()));
        assert!(invocation.args.contains(&"--allowedTools".to_string()));
        assert!(
            !invocation.args.contains(&"--tools".to_string()),
            "--tools only understands built-in tool names; passing MCP names there hides the whole toolset"
        );
        assert!(invocation.args.contains(&"--disallowedTools".to_string()));
        assert!(invocation.args.contains(&"dontAsk".to_string()));
        assert_eq!(
            invocation.prompt, "hello",
            "prompt must be delivered via stdin"
        );
        assert!(
            !invocation.args.contains(&"hello".to_string()),
            "prompt must not be passed as argv (OS arg-length limits on large boards)"
        );
        assert!(
            invocation
                .args
                .iter()
                .any(|arg| arg.contains("mcp__flowpilot__get_declarations")
                    && arg.contains("mcp__flowpilot__write_flowscript")
                    && arg.contains("mcp__flowpilot__patch_flowscript")
                    && arg.contains("mcp__flowpilot__check_flowscript")
                    && arg.contains("mcp__flowpilot__commit_flowscript")),
            "claude invocation should allow only shared FlowPilot MCP tools: {:?}",
            invocation.args
        );
        assert!(invocation.args.contains(&"sonnet".to_string()));
        assert!(
            invocation
                .args
                .windows(2)
                .any(|args| args == ["--effort", "max"]),
            "Claude Code must receive the selected dynamic effort level: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .args
                .contains(&"--include-partial-messages".to_string()),
            "claude invocation must stream partial messages for live tokens: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .envs
                .iter()
                .any(|(key, value)| key == "MCP_TOOL_TIMEOUT" && value == "1800000"),
            "claude invocation must set the overall MCP tool timeout to 1800s: {:?}",
            invocation.envs
        );
        assert!(
            invocation
                .envs
                .iter()
                .any(|(key, value)| { key == "CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT" && value == "0" }),
            "claude invocation must disable Claude's independent 300s MCP idle watchdog for nested FlowPilot runs: {:?}",
            invocation.envs
        );
        assert!(
            invocation
                .envs
                .iter()
                .any(|(key, value)| key == "ENABLE_TOOL_SEARCH" && value == "auto"),
            "Claude ToolSearch should preload the small FlowPilot surface instead of discovering each schema turn-by-turn: {:?}",
            invocation.envs
        );
        assert!(
            !invocation.args.iter().any(|arg| arg == "--max-turns"),
            "Claude must use its normal turn lifecycle rather than an arbitrary hard cap: {:?}",
            invocation.args
        );

        let config_path = invocation
            .final_output_path
            .as_ref()
            .expect("claude invocation stores temp MCP config");
        let config = std::fs::read_to_string(config_path).expect("temp MCP config is readable");
        assert!(config.contains("flowpilot"));
        assert!(config.contains("127.0.0.1:23456/mcp"));
        assert!(config.contains("\"alwaysLoad\": true"));
        let _ = std::fs::remove_file(config_path);
    }

    // 1x1 transparent PNG — enough for arg/stdin plumbing assertions (real
    // snapshots must be larger for the model to perceive them).
    fn test_chat_image() -> ChatImage {
        ChatImage {
            data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string(),
            media_type: "image/png".to_string(),
        }
    }

    #[test]
    fn codex_invocation_attaches_images_via_image_flag() {
        let invocation = ExternalAgentInvocation::new(
            FlowPilotAgentBackendKind::Codex,
            CliResolution::new(
                std::path::PathBuf::from("/usr/bin/codex"),
                CliResolutionSource::Path,
            ),
            "default",
            None,
            "http://127.0.0.1:12345/mcp",
            "hello".to_string(),
            vec!["edit_flowscript".to_string()],
            &[test_chat_image()],
        )
        .expect("codex invocation should build");

        // `--image=<path>` single-arg form: the bare two-arg form parses
        // greedily and would swallow trailing arguments as image paths.
        assert!(
            invocation
                .args
                .iter()
                .any(|arg| arg.starts_with("--image=") && arg.ends_with(".png")),
            "codex invocation must attach images via --image=<path>: {:?}",
            invocation.args
        );
        assert!(
            invocation.prompt.contains("hello"),
            "codex prompt must stay on stdin"
        );
    }

    #[test]
    fn claude_invocation_sends_images_as_stream_json_stdin() {
        let invocation = ExternalAgentInvocation::new(
            FlowPilotAgentBackendKind::ClaudeCode,
            CliResolution::new(
                std::path::PathBuf::from("/usr/bin/claude"),
                CliResolutionSource::Path,
            ),
            "default",
            None,
            "http://127.0.0.1:23456/mcp",
            "hello".to_string(),
            vec![],
            &[test_chat_image()],
        )
        .expect("claude invocation should build");

        assert!(
            invocation
                .args
                .windows(2)
                .any(|args| args == ["--input-format", "stream-json"]),
            "image turns must switch Claude to stream-json stdin input: {:?}",
            invocation.args
        );
        assert!(
            !invocation.args.contains(&"hello".to_string()),
            "image turns must not also pass the prompt positionally: {:?}",
            invocation.args
        );

        let line: serde_json::Value = serde_json::from_str(invocation.prompt.trim())
            .expect("stdin prompt is a single JSON user message line");
        assert_eq!(line["type"], "user");
        let content = line["message"]["content"]
            .as_array()
            .expect("content blocks");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "hello");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");

        if let Some(config_path) = invocation.final_output_path.as_ref() {
            let _ = std::fs::remove_file(config_path);
        }
    }

    #[test]
    fn external_agent_text_extractor_handles_result_events() {
        let event = serde_json::json!({
            "type": "result",
            "message": {
                "content": [
                    { "type": "text", "text": "Created the FlowScript draft." }
                ]
            }
        });

        assert_eq!(
            external_agent_result_text(FlowPilotAgentBackendKind::Codex, &event).as_deref(),
            Some("Created the FlowScript draft.")
        );
    }

    #[test]
    fn codex_event_parser_uses_agent_message_completion() {
        let event = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item-1",
                "type": "agent_message",
                "text": "Created the FlowScript draft."
            }
        });

        assert_eq!(
            external_agent_result_text(FlowPilotAgentBackendKind::Codex, &event).as_deref(),
            Some("Created the FlowScript draft.")
        );
    }

    #[test]
    fn codex_stream_parser_ignores_mcp_tool_output_as_chat_text() {
        let event = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "tool-1",
                "type": "mcp_tool_call",
                "server": "flowpilot",
                "tool": "list_board_nodes",
                "status": "completed",
                "result": {
                    "content": [
                        { "type": "text", "text": "Board has 37 nodes and many variables." }
                    ]
                }
            }
        });

        let mut state = ExternalAgentStreamState::default();
        assert_eq!(codex_agent_message_delta(&event, &mut state), None);

        let process_event =
            external_agent_process_event(&event).expect("mcp tool call should be framed");
        assert!(process_event.starts_with("<tool_end>"));
        assert!(process_event.contains("list_board_nodes"));
        assert_eq!(
            external_agent_result_text(FlowPilotAgentBackendKind::Codex, &event),
            None
        );
        assert!(process_event.contains("result_preview"));
        assert!(process_event.contains("Board has 37 nodes"));
    }

    #[test]
    fn codex_tool_frames_include_bounded_redacted_input_and_output() {
        let started = serde_json::json!({
            "type": "item.started",
            "item": {
                "id": "tool-io-1",
                "type": "mcp_tool_call",
                "server": "flowpilot",
                "tool": "edit_flowscript",
                "arguments": {
                    "flowscript": "function pollSupportInbox() {\n const password = \"must-not-leak\"\n logInfo({ message: \"polling\" })\n}"
                }
            }
        });
        let start = external_agent_process_event(&started).expect("tool start frame");
        assert!(start.contains("arguments_preview"));
        assert!(start.contains("pollSupportInbox"));
        assert!(start.contains("logInfo"));
        assert!(start.contains("redacted"));
        assert!(!start.contains("must-not-leak"));

        let completed = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "tool-io-1",
                "type": "mcp_tool_call",
                "server": "flowpilot",
                "tool": "edit_flowscript",
                "status": "completed",
                "result": {
                    "content": [{
                        "type": "text",
                        "text": "{\"status\":\"validation_errors\",\"diagnostics\":[\"missing Done edge\"],\"token\":\"must-not-leak\"}"
                    }]
                }
            }
        });
        let end = external_agent_process_event(&completed).expect("tool end frame");
        assert!(end.contains("result_preview"));
        assert!(end.contains("validation_errors"));
        assert!(end.contains("missing Done edge"));
        assert!(end.contains("redacted"));
        assert!(!end.contains("must-not-leak"));
    }

    #[test]
    fn codex_stream_parser_emits_flowscript_workspace_from_write_tool_arguments() {
        let event = serde_json::json!({
            "type": "item.started",
            "item": {
                "id": "tool-1",
                "type": "mcp_tool_call",
                "server": "flowpilot",
                "tool": "write_flowscript",
                "arguments": {
                    "draft_id": "gmail-flow",
                    "source": "run() {\n    const db = openLocalDb({ name: \"gmail_vectors\" })\n}"
                }
            }
        });

        let workspace_event = external_agent_flowscript_workspace_event(&event)
            .expect("write_flowscript arguments should create a workspace stream event");
        assert!(workspace_event.starts_with("<flowscript_workspace>"));
        assert!(workspace_event.contains("openLocalDb"));
        assert!(workspace_event.contains("submitted"));
    }

    #[test]
    fn codex_stream_parser_accepts_json_string_write_tool_arguments() {
        let event = serde_json::json!({
            "type": "item.started",
            "item": {
                "id": "tool-1",
                "type": "mcp_tool_call",
                "server": "flowpilot",
                "tool": "mcp__flowpilot__write_flowscript",
                "arguments": "{\"draft_id\":\"hello-flow\",\"source\":\"run() {\\n    logInfo({ message: \\\"hello\\\" })\\n}\"}"
            }
        });

        let workspace_event = external_agent_flowscript_workspace_event(&event)
            .expect("json-string write_flowscript arguments should be parsed");
        assert!(workspace_event.starts_with("<flowscript_workspace>"));
        assert!(workspace_event.contains("logInfo"));
    }

    #[test]
    fn codex_stream_parser_emits_authoritative_patch_result_workspace() {
        let event = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "tool-patch-1",
                "type": "mcp_tool_call",
                "server": "flowpilot",
                "tool": "patch_flowscript",
                "status": "completed",
                "result": {
                    "content": [{
                        "type": "text",
                        "text": "{\"status\":\"draft_updated\",\"draft_id\":\"hello-flow\",\"revision\":2,\"source\":\"eventsSimple() { logInfo({ message: \\\"fixed\\\" }) }\"}"
                    }]
                }
            }
        });

        let workspace_event = external_agent_flowscript_workspace_event(&event)
            .expect("completed patch should publish its exact retained source");
        assert!(workspace_event.starts_with("<flowscript_workspace>"));
        assert!(workspace_event.contains("draft_updated"));
        assert!(workspace_event.contains("hello-flow"));
        assert!(workspace_event.contains("fixed"));
        assert!(workspace_event.contains("tool-patch-1"));
    }

    #[test]
    fn codex_stream_parser_emits_only_new_agent_message_suffixes() {
        let mut state = ExternalAgentStreamState::default();
        let first = serde_json::json!({
            "type": "item.updated",
            "item": {
                "id": "msg-1",
                "type": "agent_message",
                "text": "Hello"
            }
        });
        let second = serde_json::json!({
            "type": "item.updated",
            "item": {
                "id": "msg-1",
                "type": "agent_message",
                "text": "Hello world"
            }
        });
        let completed = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "msg-1",
                "type": "agent_message",
                "text": "Hello world"
            }
        });

        assert_eq!(
            codex_agent_message_delta(&first, &mut state).as_deref(),
            Some("Hello")
        );
        assert_eq!(
            codex_agent_message_delta(&second, &mut state).as_deref(),
            Some(" world")
        );
        assert_eq!(
            codex_agent_message_delta(&completed, &mut state).as_deref(),
            Some("")
        );
    }

    #[test]
    fn codex_stream_parser_separates_multiple_agent_messages() {
        let mut state = ExternalAgentStreamState::default();
        let first = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "msg-1",
                "type": "agent_message",
                "text": "First note."
            }
        });
        let second = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "msg-2",
                "type": "agent_message",
                "text": "Second note."
            }
        });

        assert_eq!(
            codex_agent_message_delta(&first, &mut state).as_deref(),
            Some("First note.")
        );
        assert_eq!(
            codex_agent_message_delta(&second, &mut state).as_deref(),
            Some("\n\nSecond note.")
        );
    }

    #[test]
    fn claude_stream_parser_emits_text_deltas_and_ignores_other_frames() {
        let mut state = ExternalAgentStreamState::default();
        let make_delta = |text: &str| {
            serde_json::json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "text_delta", "text": text }
                }
            })
        };

        assert_eq!(
            claude_agent_message_delta(&make_delta("hello "), &mut state).as_deref(),
            Some("hello ")
        );
        assert_eq!(
            claude_agent_message_delta(&make_delta("there"), &mut state).as_deref(),
            Some("there"),
            "consecutive deltas concatenate without inserting separators"
        );

        // Thinking deltas, full assistant messages, and results are handled
        // elsewhere and must not be double-emitted as streamed text.
        let thinking = serde_json::json!({
            "type": "stream_event",
            "event": { "type": "content_block_delta", "delta": { "type": "thinking_delta", "thinking": "hmm" } }
        });
        let assistant = serde_json::json!({
            "type": "assistant",
            "message": { "content": [{ "type": "text", "text": "hello there" }] }
        });
        let result =
            serde_json::json!({ "type": "result", "subtype": "success", "result": "hello there" });
        assert_eq!(claude_agent_message_delta(&thinking, &mut state), None);
        assert_eq!(claude_agent_message_delta(&assistant, &mut state), None);
        assert_eq!(claude_agent_message_delta(&result, &mut state), None);
    }

    #[test]
    fn claude_result_event_yields_final_text() {
        let event = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "result": "Here is what I found.",
            "usage": { "input_tokens": 10, "output_tokens": 5 }
        });
        assert_eq!(
            external_agent_result_text(FlowPilotAgentBackendKind::ClaudeCode, &event).as_deref(),
            Some("Here is what I found.")
        );
    }

    #[test]
    fn claude_pending_mcp_server_is_not_reported_as_a_connection_failure() {
        let event = serde_json::json!({
            "type": "system",
            "subtype": "init",
            "mcp_servers": [{ "name": "flowpilot", "status": "pending" }]
        });

        assert_eq!(external_agent_mcp_connect_failure(&event), None);
    }

    #[test]
    fn claude_failed_mcp_server_is_reported_as_a_connection_failure() {
        let event = serde_json::json!({
            "type": "system",
            "subtype": "init",
            "mcp_servers": [{ "name": "flowpilot", "status": "failed" }]
        });

        let error = external_agent_mcp_connect_failure(&event)
            .expect("a failed FlowPilot MCP server must be surfaced");
        assert!(error.contains("flowpilot"));
        assert!(error.contains("failed"));
    }

    #[test]
    fn claude_tool_events_frame_mcp_tool_use_and_result() {
        let mut state = ExternalAgentStreamState::default();
        let tool_use = serde_json::json!({
            "type": "assistant",
            "message": { "content": [
                { "type": "text", "text": "Let me check." },
                { "type": "tool_use", "id": "toolu_1", "name": "mcp__flowpilot__write_flowscript", "input": {
                    "draft_id": "widget-flow",
                    "source": "function buildWidget() {\n const api_token = \"must-not-leak\"\n logInfo({ message: \"ready\" })\n}"
                } }
            ] }
        });
        let tool_result = serde_json::json!({
            "type": "user",
            "message": { "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1", "content": "{\"status\":\"draft_started\",\"draft_id\":\"widget-flow\",\"revision\":0,\"source\":\"function buildWidget() { logInfo({ message: \\\"ready\\\" }) }\",\"message\":\"widget ready\",\"password\":\"must-not-leak\"}" }
            ] }
        });

        let starts = claude_agent_tool_events(&tool_use, &mut state);
        assert_eq!(
            starts.len(),
            2,
            "full-source authoring emits one inline workspace preview and one tool_start"
        );
        assert!(
            starts[0].contains("flowscript_workspace")
                && starts[0].contains("buildWidget")
                && starts[0].contains("submitted")
                && starts[0].contains("toolu_1"),
            "Claude source should be visible inline before execution: {}",
            starts[0]
        );
        assert!(
            starts[1].contains("tool_start")
                && starts[1].contains("\"tool\":\"write_flowscript\"")
                && starts[1].contains("toolu_1")
                && starts[1].contains("arguments_preview")
                && starts[1].contains("buildWidget")
                && starts[1].contains("redacted")
                && !starts[1].contains("must-not-leak"),
            "mcp__flowpilot__ prefix must be stripped: {}",
            starts[1]
        );

        let ends = claude_agent_tool_events(&tool_result, &mut state);
        assert_eq!(ends.len(), 2);
        assert!(
            ends[0].contains("flowscript_workspace")
                && ends[0].contains("draft_started")
                && ends[0].contains("widget-flow")
                && ends[0].contains("buildWidget"),
            "tool result should publish the authoritative retained source: {}",
            ends[0]
        );
        assert!(
            ends[1].contains("tool_end")
                && ends[1].contains("\"tool\":\"write_flowscript\"")
                && ends[1].contains("\"status\":\"done\"")
                && ends[1].contains("result_preview")
                && ends[1].contains("widget ready")
                && ends[1].contains("redacted")
                && !ends[1].contains("must-not-leak"),
            "tool_end reuses the remembered name and marks success: {}",
            ends[1]
        );
    }

    #[test]
    fn claude_partial_tool_json_streams_flowscript_while_model_is_writing() {
        let mut state = ExternalAgentStreamState::default();
        let start = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_start",
                "index": 2,
                "content_block": {
                    "type": "tool_use",
                    "id": "toolu_live",
                    "name": "mcp__flowpilot__write_flowscript",
                    "input": {}
                }
            }
        });
        assert!(claude_agent_tool_events(&start, &mut state).is_empty());

        let first_delta = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "index": 2,
                "delta": {
                    "type": "input_json_delta",
                    "partial_json": "{\"draft_id\":\"live-flow\",\"source\":\"function livePreview() {"
                }
            }
        });
        let first = claude_agent_tool_events(&first_delta, &mut state);
        assert_eq!(first.len(), 1);
        assert!(first[0].contains("flowscript_workspace"));
        assert!(first[0].contains("drafting"));
        assert!(first[0].contains("livePreview"));
        assert!(first[0].contains("toolu_live"));

        let newline_delta = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "index": 2,
                "delta": {
                    "type": "input_json_delta",
                    "partial_json": "\\n  logInfo({ message: \\\"still generating\\\" })"
                }
            }
        });
        let second = claude_agent_tool_events(&newline_delta, &mut state);
        assert_eq!(second.len(), 1);
        assert!(second[0].contains("still generating"));
        assert!(second[0].contains("\\n"));
    }

    #[test]
    fn claude_tool_events_ignore_plain_assistant_text() {
        let mut state = ExternalAgentStreamState::default();
        let text_only = serde_json::json!({
            "type": "assistant",
            "message": { "content": [{ "type": "text", "text": "just text" }] }
        });
        assert!(claude_agent_tool_events(&text_only, &mut state).is_empty());
    }

    #[test]
    fn codex_event_parser_surfaces_turn_failures() {
        let event = serde_json::json!({
            "type": "turn.failed",
            "error": {
                "message": "not authenticated"
            }
        });

        assert_eq!(
            external_agent_error_text(&event).as_deref(),
            Some("not authenticated")
        );
    }

    #[test]
    fn extra_bin_dirs_contains_common_locations() {
        let dirs = extra_bin_dirs();
        assert!(!dirs.is_empty(), "extra_bin_dirs should not be empty");

        let home = dirs_next::home_dir().expect("test requires a home directory");
        assert!(dirs.contains(&home.join(".local/bin")));
        assert!(dirs.contains(&home.join(".asdf/shims")));

        let paths_str: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        let has_homebrew = paths_str.iter().any(|p| p.contains("homebrew"));
        let has_usr_local = paths_str.iter().any(|p| p.contains("/usr/local/bin"));
        assert!(
            has_homebrew || has_usr_local,
            "Should include /opt/homebrew/bin or /usr/local/bin. Got: {:?}",
            paths_str
        );

        #[cfg(target_os = "linux")]
        assert!(dirs.contains(&PathBuf::from("/home/linuxbrew/.linuxbrew/bin")));
    }

    #[test]
    fn augmented_path_includes_existing_dirs() {
        let path = augmented_path();
        assert!(!path.is_empty(), "augmented_path should not be empty");
        // Must contain original PATH
        let current = std::env::var("PATH").unwrap_or_default();
        assert!(
            path.contains(&current),
            "augmented PATH should contain original PATH"
        );
    }

    #[test]
    fn executable_lookup_searches_supplied_path() -> std::io::Result<()> {
        let temp_dir = std::env::temp_dir().join(format!(
            "flowpilot-executable-lookup-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&temp_dir)?;
        let executable_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let executable = temp_dir.join(executable_name);
        std::fs::write(&executable, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
        }

        let path_value = std::env::join_paths([temp_dir.as_path()])
            .expect("test path should join")
            .to_string_lossy()
            .into_owned();

        assert_eq!(
            find_executable_in_path("codex", &path_value).as_deref(),
            Some(executable.as_path())
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
        Ok(())
    }

    #[test]
    fn codex_ide_extension_candidate_dirs_find_extension_bundled_binary() -> std::io::Result<()> {
        let temp_home = std::env::temp_dir().join(format!(
            "flowpilot-codex-extension-test-{}",
            uuid::Uuid::new_v4()
        ));
        let codex_dir = temp_home.join(".vscode/extensions/openai.chatgpt-test/bin/macos-aarch64");
        std::fs::create_dir_all(codex_dir.join("codex-path"))?;
        let executable_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let executable = codex_dir.join(executable_name);
        std::fs::write(&executable, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
        }

        let dirs = codex_ide_extension_candidate_dirs(&temp_home);
        assert!(
            dirs.iter().any(|dir| dir == &codex_dir),
            "expected extension binary directory in candidates: {:?}",
            dirs
        );
        assert!(
            dirs.iter().any(|dir| dir == &codex_dir.join("codex-path")),
            "expected bundled Codex PATH helper directory in candidates: {:?}",
            dirs
        );

        let _ = std::fs::remove_dir_all(&temp_home);
        Ok(())
    }

    #[test]
    fn claude_ide_extension_binaries_prefers_newest_version() -> std::io::Result<()> {
        let temp_home = std::env::temp_dir().join(format!(
            "flowpilot-claude-ext-test-{}",
            uuid::Uuid::new_v4()
        ));
        let binary_name = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        let make = |version: &str| -> std::io::Result<PathBuf> {
            let dir = temp_home
                .join(".vscode/extensions")
                .join(format!("anthropic.claude-code-{version}-darwin-arm64"))
                .join("resources/native-binary");
            std::fs::create_dir_all(&dir)?;
            let executable = dir.join(binary_name);
            std::fs::write(&executable, b"#!/bin/sh\nexit 0\n")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
            }
            Ok(executable)
        };
        let _older = make("2.1.9")?;
        let newest = make("2.1.204")?;
        // An unrelated extension must not be mistaken for the Claude Code CLI.
        std::fs::create_dir_all(temp_home.join(".vscode/extensions/some.other-ext"))?;

        let binaries = claude_ide_extension_binaries(&temp_home);
        assert_eq!(
            binaries.first(),
            Some(&newest),
            "newest extension version must resolve first: {:?}",
            binaries
        );

        let _ = std::fs::remove_dir_all(&temp_home);
        Ok(())
    }

    #[test]
    fn codex_npm_native_package_layout_resolves_like_official_sdk() -> std::io::Result<()> {
        let Some((target, platform_package)) = codex_target() else {
            return Ok(());
        };
        let temp_root = std::env::temp_dir().join(format!(
            "flowpilot-codex-npm-package-test-{}",
            uuid::Uuid::new_v4()
        ));
        let vendor_target = temp_root
            .join("node_modules")
            .join(platform_package)
            .join("vendor")
            .join(target);
        let bin_dir = vendor_target.join("bin");
        std::fs::create_dir_all(&bin_dir)?;
        std::fs::create_dir_all(vendor_target.join("codex-path"))?;
        std::fs::write(vendor_target.join("codex-package.json"), b"{}")?;
        let executable = bin_dir.join(codex_binary_name());
        std::fs::write(&executable, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
        }

        let resolution = find_codex_packaged_cli_under_root(
            &temp_root.join("node_modules"),
            CliResolutionSource::CodexNpmPackage,
        )
        .expect("official @openai/codex native package layout should resolve");
        assert_eq!(resolution.executable, executable);
        assert_eq!(resolution.source, CliResolutionSource::CodexNpmPackage);
        assert!(
            resolution
                .path_dirs
                .iter()
                .any(|dir| dir == &vendor_target.join("codex-path")),
            "expected codex-path helper dir in resolution: {:?}",
            resolution.path_dirs
        );

        let _ = std::fs::remove_dir_all(&temp_root);
        Ok(())
    }

    #[test]
    fn augmented_path_has_node_accessible() {
        let path = augmented_path();
        let found_node = path.split(':').any(|dir| {
            let candidate = std::path::Path::new(dir).join("node");
            candidate.exists()
        });
        assert!(
            found_node,
            "augmented PATH should include a directory containing `node`. PATH = {}",
            path
        );
    }

    #[test]
    fn find_copilot_cli_resolves() {
        let cli_path = find_copilot_cli_path();
        assert!(
            cli_path.is_some(),
            "find_copilot_cli_path() returned None — the `copilot` CLI binary is not installed or not on PATH. \
             Searched in: {:?}",
            extra_bin_dirs()
                .iter()
                .filter(|d| d.exists())
                .collect::<Vec<_>>()
        );
        if let Some(ref p) = cli_path {
            assert!(
                p.exists(),
                "resolved copilot CLI path does not exist: {:?}",
                p
            );
        }
    }

    #[tokio::test]
    async fn copilot_sdk_client_starts_and_stops() {
        let Some(client) = build_test_client() else {
            return;
        };

        let start_result = client.start().await;

        if let Err(ref e) = start_result {
            let err_str = format!("{:?}", e);
            if err_str.contains("ProtocolMismatch") {
                panic!(
                    "COPILOT SDK PROTOCOL MISMATCH: The copilot-sdk Rust crate (protocol v{}) \
                     is incompatible with the installed Copilot CLI (protocol v3). \
                     Update the copilot-sdk dependency in Cargo.toml to a version supporting \
                     protocol v3. Error: {}",
                    copilot_sdk::SDK_PROTOCOL_VERSION,
                    err_str
                );
            }
            panic!("client.start() failed: {:?}", e);
        }

        let stop_errors = client.stop().await;
        assert!(
            stop_errors.is_empty(),
            "client.stop() had errors: {:?}",
            stop_errors
        );
    }

    #[tokio::test]
    async fn copilot_sdk_auth_status() {
        let Some(client) = start_test_client().await else {
            return;
        };

        let auth = client.get_auth_status().await;
        assert!(auth.is_ok(), "get_auth_status() failed: {:?}", auth.err());

        let status = auth.unwrap();
        println!(
            "Auth status: authenticated={}, login={:?}",
            status.is_authenticated, status.login
        );
        assert!(
            status.is_authenticated,
            "Copilot is not authenticated. Run `copilot auth login` first."
        );

        let _ = client.stop().await;
    }

    #[tokio::test]
    async fn copilot_sdk_list_models() {
        let Some(client) = start_test_client().await else {
            return;
        };

        let models = client.list_models().await;
        assert!(models.is_ok(), "list_models() failed: {:?}", models.err());

        let models = models.unwrap();
        println!("Available models ({}):", models.len());
        for m in &models {
            println!("  - {} ({})", m.name, m.id);
        }
        assert!(
            !models.is_empty(),
            "No models returned from Copilot SDK — check subscription/auth"
        );

        let _ = client.stop().await;
    }

    #[tokio::test]
    async fn copilot_sdk_create_session_and_chat() {
        let Some(client) = start_test_client().await else {
            return;
        };

        let config = copilot_sdk::SessionConfig {
            streaming: true,
            ..Default::default()
        };

        let session = client.create_session(config).await;
        assert!(
            session.is_ok(),
            "create_session() failed: {:?}",
            session.err()
        );
        let session = session.unwrap();

        let mut events = session.subscribe();
        let send_result = session.send("Reply with only the word 'pong'").await;
        assert!(
            send_result.is_ok(),
            "session.send() failed: {:?}",
            send_result.err()
        );

        let mut got_response = false;
        let mut full_response = String::new();
        let timeout = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                match events.recv().await {
                    Ok(event) => match &event.data {
                        copilot_sdk::SessionEventData::AssistantMessageDelta(delta) => {
                            full_response.push_str(&delta.delta_content);
                        }
                        copilot_sdk::SessionEventData::AssistantMessage(msg) => {
                            if full_response.is_empty() {
                                full_response = msg.content.clone();
                            }
                            got_response = true;
                        }
                        copilot_sdk::SessionEventData::SessionIdle(_) => break,
                        copilot_sdk::SessionEventData::SessionError(err) => {
                            panic!("Session error: {:?}", err);
                        }
                        _ => {}
                    },
                    Err(e) => {
                        panic!("Event receive error: {}", e);
                    }
                }
            }
        })
        .await;

        assert!(timeout.is_ok(), "Chat timed out after 30s");
        assert!(
            !full_response.is_empty(),
            "Got empty response from Copilot session"
        );
        println!("Chat response: {}", full_response);

        let _ = client.stop().await;
    }
}
