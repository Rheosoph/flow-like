//! Typed board commit approval, application, and durable receipts.

use super::board_jobs::{
    BOARD_EDIT_JOB_MAX_APPLY_RECEIPT_BYTES, BOARD_EDIT_JOBS, BoardEditJobPhase, BoardEditJobRecord,
    wall_clock_ms,
};
use crate::{
    functions::ai::copilot_sdk_tools::retained_flow_ir_draft_store, state::TauriFlowLikeState,
};
use flow_like::{
    app::{App, AppVisibility},
    copilot::FlowIrCommitToken,
    flow::{
        board::{Board, commands::GenericCommand},
        copilot::{BoardCommand, board_fingerprint},
    },
    flow_like_storage::object_store::path::Path,
};
use flow_like_types::{FromProto, ToProto};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex as StdMutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

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

#[derive(Clone, Deserialize, Serialize)]
pub struct ApplyFlowIrCommitResult {
    pub status: String,
    /// True when no mutation occurred in this invocation and the exact durable success receipt was
    /// replayed. Renderers must invalidate/reload history instead of appending this older batch.
    #[serde(default)]
    pub replayed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    pub commands: Vec<GenericCommand>,
    pub board_commands: Vec<BoardCommand>,
    pub diagnostics: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_board_node_count: Option<usize>,
    /// Exact persisted graph content after this batch, retained unchanged on receipt replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persisted_board_fingerprint: Option<String>,
}

#[derive(Serialize)]
pub struct FlowIrCommitReadback {
    pub app_id: String,
    pub board_id: String,
    pub graph_fingerprint: String,
    pub flowscript: String,
}

/// Hash the storage representation, including every node/pin identity and connection. FlowScript
/// can inline distinct getter nodes into identical expressions, so source text cannot prove the
/// compiled graph was persisted. Internal receipt refs are excluded by Board's JSON serializer.
pub(super) fn persisted_board_graph_fingerprint(board: &Board) -> Result<String, String> {
    let stored = Board::from_proto(board.to_proto());
    let mut value = serde_json::to_value(stored).map_err(|error| error.to_string())?;
    let fields = value
        .as_object_mut()
        .ok_or("Board did not serialize as an object")?;
    for key in ["created_at", "updated_at", "hash"] {
        fields.remove(key);
    }
    value.sort_all_objects();
    let bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    Ok(format!(
        "flowpilot-board-v1:{}",
        blake3::hash(&bytes).to_hex()
    ))
}

/// Read the saved object directly. Loading a registered board or running catalog migrations here
/// would audit editor memory or a derived graph instead of the native Apply's persistence boundary.
#[tauri::command(async)]
pub async fn flowpilot_read_flow_ir_commit_board(
    app_handle: AppHandle,
    app_id: String,
    board_id: String,
) -> Result<FlowIrCommitReadback, String> {
    if app_id.trim().is_empty() || board_id.trim().is_empty() {
        return Err("IR_READBACK_TARGET_INVALID: App and board ids are required.".to_string());
    }
    let state = TauriFlowLikeState::construct(&app_handle)
        .await
        .map_err(|error| error.to_string())?;
    let app = App::load(app_id.clone(), state)
        .await
        .map_err(|error| format!("IR_READBACK_APP_UNAVAILABLE: {error}"))?;
    if app.id != app_id || !app.boards.contains(&board_id) {
        return Err(
            "IR_READBACK_APP_BOARD_MISMATCH: The board is not owned by this app.".to_string(),
        );
    }
    let store = TauriFlowLikeState::get_project_meta_store(&app_handle)
        .await
        .map_err(|error| error.to_string())?;
    let proto = Board::load_proto(
        store,
        &Path::from("apps").join(app_id.clone()),
        &board_id,
        None,
    )
    .await
    .map_err(|error| format!("IR_READBACK_PERSISTENCE_UNAVAILABLE: {error}"))?;
    let board = Board::from_proto(proto);
    if board.id != board_id {
        return Err(
            "IR_READBACK_BOARD_ID_MISMATCH: The saved object contains another board.".to_string(),
        );
    }
    board
        .ensure_supported_format()
        .map_err(|error| error.to_string())?;
    Ok(FlowIrCommitReadback {
        app_id,
        board_id,
        graph_fingerprint: persisted_board_graph_fingerprint(&board)?,
        flowscript: flow_like::flow::ast::board_to_flowscript(
            &board,
            &flow_like::flow::ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        ),
    })
}

const FLOW_IR_APPLIED_RECEIPT_TTL: Duration = Duration::from_secs(2 * 60 * 60);

const FLOW_IR_APPLIED_RECEIPT_MAX_ENTRIES: usize = 512;

const FLOW_IR_DURABLE_RECEIPT_REF_PREFIX: &str = "__flow_like_internal_v1/flowpilot-apply-receipt/";

#[derive(Deserialize, Serialize)]
struct DurableFlowIrAppliedReceipt {
    version: u8,
    created_at_ms: u64,
    identity: String,
    result: ApplyFlowIrCommitResult,
}

pub(super) static FLOW_IR_APPLIED_RECEIPTS: LazyLock<
    StdMutex<HashMap<String, (Instant, ApplyFlowIrCommitResult)>>,
> = LazyLock::new(|| StdMutex::new(HashMap::new()));

pub(super) fn flow_ir_applied_receipt_key(app_id: &str, token: &FlowIrCommitToken) -> String {
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

pub(super) fn flow_ir_durable_receipt_ref_key(app_id: &str, token: &FlowIrCommitToken) -> String {
    let identity = flow_ir_applied_receipt_key(app_id, token);
    format!(
        "{FLOW_IR_DURABLE_RECEIPT_REF_PREFIX}{}",
        blake3::hash(identity.as_bytes()).to_hex()
    )
}

fn replay_flow_ir_applied_receipt_from_board(
    board: &Board,
    app_id: &str,
    token: &FlowIrCommitToken,
) -> Option<ApplyFlowIrCommitResult> {
    let identity = flow_ir_applied_receipt_key(app_id, token);
    let encoded = board.internal_ref(&flow_ir_durable_receipt_ref_key(app_id, token))?;
    let receipt = serde_json::from_str::<DurableFlowIrAppliedReceipt>(encoded).ok()?;
    if receipt.version != 1 || receipt.identity != identity {
        return None;
    }
    let mut result = receipt.result;
    result.replayed = true;
    result.message = format!("{} (durable idempotent replay)", result.message);
    Some(result)
}

pub(super) fn compact_durable_apply_receipt(
    result: &ApplyFlowIrCommitResult,
) -> ApplyFlowIrCommitResult {
    let mut compact = result.clone();
    // BoardCommands are the pre-apply compiler artifact and remain represented by the token. A
    // replay only needs executed GenericCommands for remote sync/history plus result metadata.
    compact.board_commands.clear();
    compact
}

pub(super) fn validate_board_edit_delivery_bounds(
    result: &ApplyFlowIrCommitResult,
    requires_remote_delivery: bool,
) -> Result<(), String> {
    if requires_remote_delivery {
        // The complete receipt is one board transaction. Measuring commands separately would
        // permit an aggregate payload that can only be delivered as independently persisted
        // prefixes (setup first, connections last), recreating a nodes-only Hub board on failure.
        crate::functions::flow::board::validate_remote_command_batch_size(&result.commands)
            .map_err(|error| {
                format!("The atomic executed command batch cannot be delivered: {error}")
            })?;
    }

    let durable_bytes = serde_json::to_vec(&compact_durable_apply_receipt(result))
        .map_err(|error| format!("Could not size the durable apply receipt: {error}"))?
        .len();
    if durable_bytes > BOARD_EDIT_JOB_MAX_APPLY_RECEIPT_BYTES {
        return Err(format!(
            "The executed receipt expands to {durable_bytes} bytes, above the {} MiB crash-recovery limit.",
            BOARD_EDIT_JOB_MAX_APPLY_RECEIPT_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
}

fn retain_flow_ir_applied_receipt_on_board(
    board: &mut Board,
    app_id: &str,
    token: &FlowIrCommitToken,
    result: &ApplyFlowIrCommitResult,
) -> Result<(), String> {
    let now_ms = wall_clock_ms();
    let mut receipts = board
        .internal_refs_with_prefix(FLOW_IR_DURABLE_RECEIPT_REF_PREFIX)
        .filter_map(|(key, value)| {
            serde_json::from_str::<DurableFlowIrAppliedReceipt>(value)
                .ok()
                .map(|receipt| (key.to_string(), receipt.created_at_ms))
        })
        .collect::<Vec<_>>();
    receipts.sort_by_key(|(_, created_at_ms)| *created_at_ms);
    let keep_from = receipts
        .len()
        .saturating_sub(FLOW_IR_APPLIED_RECEIPT_MAX_ENTRIES.saturating_sub(1));
    let retained = receipts
        .into_iter()
        .skip(keep_from)
        .map(|(key, _)| key)
        .collect::<HashSet<_>>();
    board
        .retain_internal_refs_with_prefix(FLOW_IR_DURABLE_RECEIPT_REF_PREFIX, |key, _| {
            retained.contains(key)
        })
        .map_err(|error| error.to_string())?;

    let identity = flow_ir_applied_receipt_key(app_id, token);
    let receipt = DurableFlowIrAppliedReceipt {
        version: 1,
        created_at_ms: now_ms,
        identity,
        result: compact_durable_apply_receipt(result),
    };
    let encoded = serde_json::to_string(&receipt)
        .map_err(|error| format!("Could not serialize the durable apply receipt: {error}"))?;
    board
        .insert_internal_ref(flow_ir_durable_receipt_ref_key(app_id, token), encoded)
        .map_err(|error| error.to_string())?;
    Ok(())
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

pub(super) fn replay_flow_ir_applied_receipt(
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
            replay.replayed = true;
            replay.message = format!("{} (idempotent replay)", replay.message);
            replay
        })
}

pub(super) fn retain_flow_ir_applied_receipt(
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
        (now, compact_durable_apply_receipt(result)),
    );
}

impl ApplyFlowIrCommitResult {
    pub(super) fn empty(status: &str, code: &str, message: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            replayed: false,
            code: Some(code.to_string()),
            message: message.into(),
            commands: Vec::new(),
            board_commands: Vec::new(),
            diagnostics: Vec::new(),
            final_board_node_count: None,
            persisted_board_fingerprint: None,
        }
    }

    pub(super) fn apply_error(
        code: &str,
        message: impl Into<String>,
        board_commands: Vec<BoardCommand>,
        diagnostics: Vec<String>,
    ) -> Self {
        Self {
            status: "error".to_string(),
            replayed: false,
            code: Some(code.to_string()),
            message: message.into(),
            commands: Vec::new(),
            board_commands,
            diagnostics,
            final_board_node_count: None,
            persisted_board_fingerprint: None,
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
            Some(live_board) => Some(live_board.write().await),
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
    let board = live_board.snapshot();
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
#[derive(Clone)]
pub(super) struct RecoveredBoardEditBatch {
    pub(super) board_commands: Vec<BoardCommand>,
    pub(super) replacement_mode: bool,
}

#[tauri::command]
pub async fn flowpilot_apply_flow_ir_commit(
    app_handle: AppHandle,
    app_id: String,
    token: FlowIrCommitToken,
) -> ApplyFlowIrCommitResult {
    flowpilot_apply_flow_ir_commit_with_recovery(app_handle, app_id, token, None, false).await
}

pub(super) fn board_edit_job_matches_terminal_delivery(
    record: &BoardEditJobRecord,
    app_id: &str,
    token: &FlowIrCommitToken,
) -> bool {
    record.job.phase == BoardEditJobPhase::Applied
        && flow_ir_applied_receipt_key(&record.job.app_id, &record.job.token)
            == flow_ir_applied_receipt_key(app_id, token)
}

fn board_edit_job_delivery_is_terminal(app_id: &str, token: &FlowIrCommitToken) -> bool {
    let jobs = BOARD_EDIT_JOBS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    jobs.values()
        .any(|record| board_edit_job_matches_terminal_delivery(record, app_id, token))
}

/// Host-only recovery variant. The renderer can never supply this batch: it comes exclusively
/// from the atomically persisted BoardEditJob that was created from the retained compiler claim.
pub(super) async fn flowpilot_apply_flow_ir_commit_with_recovery(
    app_handle: AppHandle,
    app_id: String,
    token: FlowIrCommitToken,
    recovered_batch: Option<RecoveredBoardEditBatch>,
    destructive_preapproved: bool,
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

    // The renderer has already durably synchronized and acknowledged this native job. Returning
    // its old commands through a stale/direct presenter would create a second remote delivery
    // attempt after the job boundary, so terminal job identity takes precedence over receipts.
    if board_edit_job_delivery_is_terminal(&app_id, &token) {
        return ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_DELIVERY_FINALIZED",
            "This exact compiled workflow review was already applied and fully delivered.",
        );
    }

    if let Some(receipt) = replay_flow_ir_applied_receipt(&app_id, &token) {
        return receipt;
    }

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
    {
        let board = live_board.write().await;
        if let Some(receipt) = replay_flow_ir_applied_receipt_from_board(&board, &app_id, &token) {
            retain_flow_ir_applied_receipt(&app_id, &token, &receipt);
            return receipt;
        }
    }
    // The in-memory retained store is preferred while the originating process is alive. A native
    // BoardEditJob additionally owns an exact persisted batch so restart recovery does not depend
    // on reconstructing transient compiler claim state.
    let store = retained_flow_ir_draft_store(&token.board_id);
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

    let resolve_exact_batch = |board: &Board| -> Option<(Vec<BoardCommand>, bool)> {
        if let Some(store) = store.as_ref()
            && let Some(commands) = store.pending_commands_if_current(
                board,
                &token.draft_id,
                token.revision,
                &token.base_fingerprint,
                &token.claim_id,
            )
            && let Some(replacement_mode) = store.pending_commit_requires_destructive_approval(
                &token.draft_id,
                token.revision,
                &token.base_fingerprint,
                &token.claim_id,
            )
        {
            return Some((commands, replacement_mode));
        }

        let recovered = recovered_batch.as_ref()?;
        if recovered.board_commands.is_empty() || board_fingerprint(board) != token.base_fingerprint
        {
            return None;
        }
        Some((recovered.board_commands.clone(), recovered.replacement_mode))
    };

    let mut board = live_board.write().await;
    let Some((mut board_commands, replacement_mode)) = resolve_exact_batch(&board) else {
        return ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_REVIEW_STALE",
            "The live board no longer matches the exact retained or crash-recovered review batch. Nothing was applied.",
        );
    };
    let destructive_review_items =
        typed_commit_destructive_review_items(replacement_mode, &board_commands);
    // The renderer already carries the user's decision when it approved the review card or when
    // they armed auto mode, so re-asking here is a duplicate prompt rather than a second boundary.
    // Everything that actually protects the batch — the exact claim, the base fingerprint, and the
    // batch-equality revalidation below — is enforced regardless of this flag.
    if !destructive_review_items.is_empty() && !destructive_preapproved {
        // Release the live-board lock while the operating-system dialog is open, then reacquire it
        // and repeat the exact claim/base/batch checks before applying. A compromised renderer can
        // request this dialog, but it cannot synthesize the native user's answer or race an
        // approved answer onto another revision.
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

        board = live_board.write().await;
        let Some((revalidated_commands, revalidated_replacement_mode)) =
            resolve_exact_batch(&board)
        else {
            return ApplyFlowIrCommitResult::empty(
                "stale",
                "IR_COMMIT_REVIEW_STALE",
                "The live board or exact recovered batch changed while native destructive confirmation was open. Nothing was applied.",
            );
        };
        let revalidated_review_items = typed_commit_destructive_review_items(
            revalidated_replacement_mode,
            &revalidated_commands,
        );
        let exact_batch_unchanged =
            exact_board_command_batch_matches(&board_commands, &revalidated_commands);
        if !exact_batch_unchanged
            || revalidated_replacement_mode != replacement_mode
            || revalidated_review_items != destructive_review_items
        {
            return ApplyFlowIrCommitResult::empty(
                "stale",
                "IR_COMMIT_REVIEW_STALE",
                "The exact destructive batch changed while native confirmation was open. Nothing was applied.",
            );
        }
        board_commands = revalidated_commands;
    }

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
            // Core rolls back every executed prefix. Discarding the draft is the final host guard
            // so an unexpected planner/rollback defect cannot leak a partial live mutation.
            board.discard();
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
        board.discard();
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

    let persisted_board_fingerprint = match persisted_board_graph_fingerprint(&board) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            board.discard();
            return ApplyFlowIrCommitResult::apply_error(
                "IR_COMMIT_FINGERPRINT_FAILED",
                "The compiled workflow was rolled back because its persisted graph could not be fingerprinted.",
                apply_result.board_commands,
                vec![error],
            );
        }
    };
    let mut result = ApplyFlowIrCommitResult {
        status: "applied".to_string(),
        replayed: false,
        code: None,
        message: format!(
            "Applied and persisted {} exact compiled workflow board command(s).",
            apply_result.commands.len()
        ),
        commands: apply_result.commands.clone(),
        board_commands: apply_result.board_commands.clone(),
        diagnostics: Vec::new(),
        final_board_node_count: Some(board_total_node_count(&board)),
        persisted_board_fingerprint: Some(persisted_board_fingerprint),
    };
    if let Err(error) = validate_board_edit_delivery_bounds(
        &result,
        !matches!(app.visibility, AppVisibility::Offline),
    ) {
        board.discard();
        return ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_DELIVERY_TOO_LARGE",
            format!(
                "The compiled workflow was not persisted because its actual undo/sync receipt cannot be delivered crash-safely: {error} Split the workflow change or reduce the size of the affected node."
            ),
        );
    }
    if let Err(error) =
        retain_flow_ir_applied_receipt_on_board(&mut board, &app_id, &token, &result)
    {
        board.discard();
        return ApplyFlowIrCommitResult::apply_error(
            "IR_COMMIT_RECEIPT_PERSISTENCE_FAILED",
            "The compiled workflow was rolled back because its crash-recovery receipt could not be prepared.",
            apply_result.board_commands,
            vec![error],
        );
    }

    if let Err(error) = board.save(Some(project_store.clone())).await {
        let rollback_error = board
            .undo(apply_result.commands.clone(), flow_like_state.clone())
            .await
            .err()
            .map(|error| error.to_string());
        board.discard();
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

    if let Some(store) = store {
        let acknowledged = store.acknowledge_applied_commit(
            &board,
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        );
        // The exact claim/base/batch was validated under this continuous board lock before
        // execution, so a failed acknowledgement only means claim bookkeeping was resolved
        // concurrently. The applied and persisted board remains authoritative.
        if !acknowledged {
            let released = store.release_commit_if_matches(
                &token.draft_id,
                token.revision,
                &token.base_fingerprint,
                &token.claim_id,
            );
            result.code = Some("IR_COMMIT_ACK_RACED".to_string());
            result
                .diagnostics
                .push(flow_ir_ack_race_diagnostic(released));
        }
    } else {
        result.diagnostics.push(
            "Applied from the host-persisted exact review batch after the transient compiler store was unavailable."
                .to_string(),
        );
    }
    retain_flow_ir_applied_receipt(&app_id, &token, &result);
    result
}

/// Human-readable trace for an apply whose claim acknowledgement raced a concurrent disposition.
/// The applied, persisted board is kept either way; this only records how the retained-store
/// bookkeeping was resolved.
pub(super) fn flow_ir_ack_race_diagnostic(released: bool) -> String {
    if released {
        "The exact claim was resolved concurrently while this apply was executing; the leftover pending review was released after the batch was applied and persisted."
            .to_string()
    } else {
        "The exact claim was resolved concurrently while this apply was executing (for example a dismissal after a lost response channel); the applied and persisted board was kept."
            .to_string()
    }
}

pub(super) fn typed_commit_destructive_review_items(
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
pub(super) fn exact_board_command_batch_matches(
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
