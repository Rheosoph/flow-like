use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::{
    board::{Board, commands::GenericCommand},
    copilot::{BoardCommand, FlowIrCommitToken, FlowIrDraftStore, board_fingerprint},
};
use serde::{Deserialize, Serialize};

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::wasm_catalog::{app_wasm_nodes, hydrate_board_wasm_metadata},
    state::{AppState, flow_ir_draft_store_key},
};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowIrCommitDisposition {
    Preflight,
    Applied,
    Dismissed,
}

#[derive(Clone, Deserialize)]
pub struct FlowIrCommitDispositionBody {
    pub token: FlowIrCommitToken,
    pub disposition: FlowIrCommitDisposition,
}

#[derive(Clone, Deserialize)]
pub struct ApplyFlowIrCommitBody {
    pub token: FlowIrCommitToken,
    #[serde(default)]
    pub approve_destructive: bool,
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
    /// True when this invocation performed no mutation and replayed the exact success receipt
    /// embedded in the canonical board. Renderers can use this to reload/invalidate history
    /// instead of appending an older batch a second time.
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
}

const FLOW_IR_DURABLE_RECEIPT_REF_PREFIX: &str =
    "__flow_like_internal_v1/flowpilot-api-apply-receipt/";
const FLOW_IR_DURABLE_RECEIPT_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const FLOW_IR_DURABLE_RECEIPT_MAX_ENTRIES: usize = 512;
const FLOW_IR_DURABLE_RECEIPT_MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;
const FLOW_IR_DURABLE_RECEIPT_MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const FLOW_IR_DURABLE_RECEIPT_CLOCK_SKEW_MS: u64 = 5 * 60 * 1_000;
const FLOW_IR_DURABLE_PENDING_REF_PREFIX: &str =
    "__flow_like_internal_v1/flowpilot-api-pending-claim/";
const FLOW_IR_DURABLE_PENDING_TTL_MS: u64 = 2 * 60 * 60 * 1_000;
const FLOW_IR_DURABLE_PENDING_MAX_ENTRIES: usize = 128;
const FLOW_IR_DURABLE_PENDING_MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;
const FLOW_IR_DURABLE_PENDING_MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
struct DurableFlowIrAppliedReceipt {
    version: u8,
    created_at_ms: u64,
    /// Domain-separated digest of the authenticated principal/app/board scope and every token
    /// authority field. It is retained inside the value as well as its ref key so a malformed or
    /// colliding entry never grants replay authority.
    identity_digest: String,
    result: ApplyFlowIrCommitResult,
}

#[derive(Clone, Deserialize, Serialize)]
struct DurableFlowIrPendingClaim {
    version: u8,
    created_at_ms: u64,
    identity_digest: String,
    payload_digest: String,
    replacement_mode: bool,
    board_commands: Vec<BoardCommand>,
}

impl FlowIrCommitDispositionResult {
    fn success(status: &str, message: &str) -> Self {
        Self {
            status: status.to_string(),
            code: None,
            message: message.to_string(),
        }
    }

    fn error(code: &str, message: impl Into<String>) -> Self {
        Self {
            status: "error".to_string(),
            code: Some(code.to_string()),
            message: message.into(),
        }
    }
}

impl ApplyFlowIrCommitResult {
    fn empty(status: &str, code: &str, message: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            replayed: false,
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
            replayed: false,
            code: Some(code.to_string()),
            message: message.into(),
            commands: Vec::new(),
            board_commands,
            diagnostics,
            final_board_node_count: None,
        }
    }
}

fn commit_token_is_valid(token: &FlowIrCommitToken, board_id: &str) -> bool {
    !board_id.trim().is_empty()
        && token.board_id == board_id
        && !token.board_id.trim().is_empty()
        && !token.draft_id.trim().is_empty()
        && !token.base_fingerprint.trim().is_empty()
        && !token.claim_id.trim().is_empty()
}

fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn flow_ir_token_identity_digest(
    domain: &[u8],
    scope_key: &str,
    token: &FlowIrCommitToken,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&[0]);
    for value in [
        scope_key.as_bytes(),
        token.board_id.as_bytes(),
        token.draft_id.as_bytes(),
        token.base_fingerprint.as_bytes(),
        token.claim_id.as_bytes(),
    ] {
        hasher.update(&(value.len() as u64).to_le_bytes());
        hasher.update(value);
    }
    hasher.update(&token.revision.to_le_bytes());
    hasher.finalize().to_hex().to_string()
}

fn applied_receipt_identity_digest(scope_key: &str, token: &FlowIrCommitToken) -> String {
    flow_ir_token_identity_digest(b"flow-like.flow-ir-applied-receipt/v1", scope_key, token)
}

fn pending_claim_identity_digest(scope_key: &str, token: &FlowIrCommitToken) -> String {
    flow_ir_token_identity_digest(b"flow-like.flow-ir-pending-claim/v1", scope_key, token)
}

fn applied_receipt_ref_key(scope_key: &str, token: &FlowIrCommitToken) -> String {
    format!(
        "{FLOW_IR_DURABLE_RECEIPT_REF_PREFIX}{}",
        applied_receipt_identity_digest(scope_key, token)
    )
}

fn pending_claim_ref_key(scope_key: &str, token: &FlowIrCommitToken) -> String {
    format!(
        "{FLOW_IR_DURABLE_PENDING_REF_PREFIX}{}",
        pending_claim_identity_digest(scope_key, token)
    )
}

fn identity_digest_is_canonical(identity_digest: &str) -> bool {
    identity_digest.len() == 64
        && identity_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonicalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonicalize_json).collect())
        }
        serde_json::Value::Object(values) => {
            let sorted = values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        value => value,
    }
}

fn pending_payload_digest(
    commands: &[BoardCommand],
    replacement_mode: bool,
) -> Result<String, String> {
    let canonical = canonicalize_json(
        serde_json::to_value(commands)
            .map_err(|error| format!("FlowScript command digest encoding failed: {error}"))?,
    );
    let encoded = serde_json::to_vec(&canonical)
        .map_err(|error| format!("FlowScript command digest serialization failed: {error}"))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.flow-ir-pending-payload/v1\0");
    hasher.update(&[u8::from(replacement_mode)]);
    hasher.update(&encoded);
    Ok(hasher.finalize().to_hex().to_string())
}

fn decode_durable_applied_receipt(
    key: &str,
    encoded: &str,
    now_ms: u64,
) -> Option<DurableFlowIrAppliedReceipt> {
    if key.len().saturating_add(encoded.len()) > FLOW_IR_DURABLE_RECEIPT_MAX_ENTRY_BYTES {
        return None;
    }
    let receipt = serde_json::from_str::<DurableFlowIrAppliedReceipt>(encoded).ok()?;
    let digest_is_canonical = identity_digest_is_canonical(&receipt.identity_digest);
    let expected_key = format!(
        "{FLOW_IR_DURABLE_RECEIPT_REF_PREFIX}{}",
        receipt.identity_digest
    );
    let created_too_far_in_future =
        receipt.created_at_ms > now_ms.saturating_add(FLOW_IR_DURABLE_RECEIPT_CLOCK_SKEW_MS);
    if receipt.version != 1
        || !digest_is_canonical
        || key != expected_key
        || created_too_far_in_future
        || now_ms.saturating_sub(receipt.created_at_ms) > FLOW_IR_DURABLE_RECEIPT_TTL_MS
        || receipt.result.status != "applied"
    {
        return None;
    }
    Some(receipt)
}

fn decode_durable_pending_claim(
    key: &str,
    encoded: &str,
    now_ms: u64,
) -> Option<DurableFlowIrPendingClaim> {
    if key.len().saturating_add(encoded.len()) > FLOW_IR_DURABLE_PENDING_MAX_ENTRY_BYTES {
        return None;
    }
    let claim = serde_json::from_str::<DurableFlowIrPendingClaim>(encoded).ok()?;
    let expected_key = format!(
        "{FLOW_IR_DURABLE_PENDING_REF_PREFIX}{}",
        claim.identity_digest
    );
    let created_too_far_in_future =
        claim.created_at_ms > now_ms.saturating_add(FLOW_IR_DURABLE_RECEIPT_CLOCK_SKEW_MS);
    if claim.version != 1
        || !identity_digest_is_canonical(&claim.identity_digest)
        || !identity_digest_is_canonical(&claim.payload_digest)
        || key != expected_key
        || created_too_far_in_future
        || now_ms.saturating_sub(claim.created_at_ms) > FLOW_IR_DURABLE_PENDING_TTL_MS
        || claim.board_commands.is_empty()
        || pending_payload_digest(&claim.board_commands, claim.replacement_mode)
            .ok()
            .as_deref()
            != Some(claim.payload_digest.as_str())
    {
        return None;
    }
    Some(claim)
}

fn pending_claim_from_board(
    board: &Board,
    scope_key: &str,
    token: &FlowIrCommitToken,
    now_ms: u64,
) -> Option<DurableFlowIrPendingClaim> {
    let identity_digest = pending_claim_identity_digest(scope_key, token);
    let key = pending_claim_ref_key(scope_key, token);
    let encoded = board.internal_ref(&key)?;
    let claim = decode_durable_pending_claim(&key, encoded, now_ms)?;
    (claim.identity_digest == identity_digest).then_some(claim)
}

fn prune_durable_pending_claims(
    board: &mut Board,
    now_ms: u64,
    max_entries: usize,
    max_total_bytes: usize,
) -> Result<bool, String> {
    let before = board
        .internal_refs_with_prefix(FLOW_IR_DURABLE_PENDING_REF_PREFIX)
        .map(|(key, _)| key.to_string())
        .collect::<HashSet<_>>();
    let mut candidates = board
        .internal_refs_with_prefix(FLOW_IR_DURABLE_PENDING_REF_PREFIX)
        .filter_map(|(key, value)| {
            decode_durable_pending_claim(key, value, now_ms).map(|claim| {
                (
                    key.to_string(),
                    claim.created_at_ms,
                    key.len().saturating_add(value.len()),
                )
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut retained = HashSet::new();
    let mut retained_bytes = 0usize;
    for (key, _, entry_bytes) in candidates {
        if retained.len() >= max_entries
            || retained_bytes.saturating_add(entry_bytes) > max_total_bytes
        {
            continue;
        }
        retained_bytes = retained_bytes.saturating_add(entry_bytes);
        retained.insert(key);
    }
    board
        .retain_internal_refs_with_prefix(FLOW_IR_DURABLE_PENDING_REF_PREFIX, |key, _| {
            retained.contains(key)
        })
        .map_err(|error| error.to_string())?;
    Ok(before != retained)
}

fn retain_pending_claim_on_board(
    board: &mut Board,
    scope_key: &str,
    token: &FlowIrCommitToken,
    replacement_mode: bool,
    board_commands: Vec<BoardCommand>,
) -> Result<(), String> {
    if board_commands.is_empty() {
        return Err("an empty FlowScript command batch cannot be retained".to_string());
    }
    let identity_digest = pending_claim_identity_digest(scope_key, token);
    let payload_digest = pending_payload_digest(&board_commands, replacement_mode)?;
    let key = pending_claim_ref_key(scope_key, token);
    let claim = DurableFlowIrPendingClaim {
        version: 1,
        created_at_ms: wall_clock_ms(),
        identity_digest,
        payload_digest,
        replacement_mode,
        board_commands,
    };
    let encoded = serde_json::to_string(&claim)
        .map_err(|error| format!("FlowScript pending claim serialization failed: {error}"))?;
    let entry_bytes = key.len().saturating_add(encoded.len());
    if entry_bytes > FLOW_IR_DURABLE_PENDING_MAX_ENTRY_BYTES
        || entry_bytes > FLOW_IR_DURABLE_PENDING_MAX_TOTAL_BYTES
    {
        return Err(format!(
            "The exact FlowScript pending claim is {entry_bytes} bytes, above the durable claim limit."
        ));
    }

    board.remove_internal_ref(&key);
    prune_durable_pending_claims(
        board,
        claim.created_at_ms,
        FLOW_IR_DURABLE_PENDING_MAX_ENTRIES.saturating_sub(1),
        FLOW_IR_DURABLE_PENDING_MAX_TOTAL_BYTES.saturating_sub(entry_bytes),
    )?;
    board
        .insert_internal_ref(key, encoded)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn replay_applied_receipt_from_board(
    board: &Board,
    scope_key: &str,
    token: &FlowIrCommitToken,
    now_ms: u64,
) -> Option<ApplyFlowIrCommitResult> {
    let identity_digest = applied_receipt_identity_digest(scope_key, token);
    let key = applied_receipt_ref_key(scope_key, token);
    let encoded = board.internal_ref(&key)?;
    let receipt = decode_durable_applied_receipt(&key, encoded, now_ms)?;
    if receipt.identity_digest != identity_digest {
        return None;
    }
    let mut replay = receipt.result;
    replay.replayed = true;
    replay.message = format!("{} (durable idempotent replay)", replay.message);
    Some(replay)
}

fn prune_durable_applied_receipts(
    board: &mut Board,
    now_ms: u64,
    max_entries: usize,
    max_total_bytes: usize,
) -> Result<bool, String> {
    let before = board
        .internal_refs_with_prefix(FLOW_IR_DURABLE_RECEIPT_REF_PREFIX)
        .map(|(key, _)| key.to_string())
        .collect::<HashSet<_>>();
    let mut candidates = board
        .internal_refs_with_prefix(FLOW_IR_DURABLE_RECEIPT_REF_PREFIX)
        .filter_map(|(key, value)| {
            decode_durable_applied_receipt(key, value, now_ms).map(|receipt| {
                (
                    key.to_string(),
                    receipt.created_at_ms,
                    key.len().saturating_add(value.len()),
                )
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut retained = HashSet::new();
    let mut retained_bytes = 0usize;
    for (key, _, entry_bytes) in candidates {
        if retained.len() >= max_entries
            || retained_bytes.saturating_add(entry_bytes) > max_total_bytes
        {
            continue;
        }
        retained_bytes = retained_bytes.saturating_add(entry_bytes);
        retained.insert(key);
    }
    board
        .retain_internal_refs_with_prefix(FLOW_IR_DURABLE_RECEIPT_REF_PREFIX, |key, _| {
            retained.contains(key)
        })
        .map_err(|error| error.to_string())?;
    Ok(before != retained)
}

fn retain_applied_receipt_on_board(
    board: &mut Board,
    scope_key: &str,
    token: &FlowIrCommitToken,
    result: &ApplyFlowIrCommitResult,
) -> Result<(), String> {
    if result.status != "applied" {
        return Err("only a successful applied result may be retained".to_string());
    }
    let identity_digest = applied_receipt_identity_digest(scope_key, token);
    let key = applied_receipt_ref_key(scope_key, token);
    let mut persisted_result = result.clone();
    persisted_result.replayed = false;
    let receipt = DurableFlowIrAppliedReceipt {
        version: 1,
        created_at_ms: wall_clock_ms(),
        identity_digest,
        result: persisted_result,
    };
    let encoded = serde_json::to_string(&receipt)
        .map_err(|error| format!("FlowScript apply receipt serialization failed: {error}"))?;
    let entry_bytes = key.len().saturating_add(encoded.len());
    if entry_bytes > FLOW_IR_DURABLE_RECEIPT_MAX_ENTRY_BYTES
        || entry_bytes > FLOW_IR_DURABLE_RECEIPT_MAX_TOTAL_BYTES
    {
        return Err(format!(
            "The exact FlowScript apply receipt is {entry_bytes} bytes, above the durable receipt limit."
        ));
    }

    // The exact key should already have replayed before draft lookup. Remove it defensively so the
    // reservation below accounts for the new entry exactly once.
    board.remove_internal_ref(&key);
    prune_durable_applied_receipts(
        board,
        receipt.created_at_ms,
        FLOW_IR_DURABLE_RECEIPT_MAX_ENTRIES.saturating_sub(1),
        FLOW_IR_DURABLE_RECEIPT_MAX_TOTAL_BYTES.saturating_sub(entry_bytes),
    )?;
    board
        .insert_internal_ref(key, encoded)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn destructive_review_items(replacement_mode: bool, commands: &[BoardCommand]) -> Vec<String> {
    let mut items = flow_like::flow::ast::destructive_flowscript_command_summaries(commands);
    if replacement_mode && items.is_empty() {
        items.push("The draft uses full-board replacement semantics.".to_string());
    }
    items
}

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

async fn restore_persisted_snapshot(board: &Board) -> Option<String> {
    board.save(None).await.err().map(|error| error.to_string())
}

/// Persist the exact host-retained review before its token is exposed by a copilot response.
/// The board mutation guard makes this idempotent across concurrent response attempts, while the
/// embedded claim lets Apply/Dismiss run on a different API replica without sharing Moka memory.
pub(crate) async fn persist_pending_flow_ir_commit(
    state: &AppState,
    sub: &str,
    app_id: &str,
    token: &FlowIrCommitToken,
    store: &FlowIrDraftStore,
) -> Result<(), ApiError> {
    if app_id.trim().is_empty() || !commit_token_is_valid(token, &token.board_id) {
        return Err(ApiError::bad_request(
            "The FlowScript review token or app id is incomplete.",
        ));
    }
    let scope_key = flow_ir_draft_store_key(sub, app_id, &token.board_id);
    let mutation_guard = state.board_mutation_guard(app_id, &token.board_id).await?;
    let mut board = state
        .master_board(sub, app_id, &token.board_id, state, None)
        .await?;
    let now_ms = wall_clock_ms();

    let receipt_key = applied_receipt_ref_key(&scope_key, token);
    if board.internal_ref(&receipt_key).is_some() {
        return Err(ApiError::conflict(
            "This exact FlowScript review already has an applied receipt.",
        ));
    }

    let local_payload = store.pending_commit_payload_if_current(
        &board,
        &token.draft_id,
        token.revision,
        &token.base_fingerprint,
        &token.claim_id,
    );
    let pending_key = pending_claim_ref_key(&scope_key, token);
    if board.internal_ref(&pending_key).is_some() {
        if let Some(claim) = pending_claim_from_board(&board, &scope_key, token, now_ms) {
            if let Some((commands, replacement_mode)) = local_payload.as_ref() {
                let digest = pending_payload_digest(commands, *replacement_mode)
                    .map_err(ApiError::internal)?;
                if claim.payload_digest == digest && claim.replacement_mode == *replacement_mode {
                    return Ok(());
                }
                return Err(ApiError::conflict(
                    "The durable FlowScript claim does not match the current retained batch.",
                ));
            }
            // A previously persisted review may be redelivered after its board became stale. Its
            // exact durable envelope remains authoritative for Preflight/Dismiss even though the
            // process-local store can no longer call it current.
            if board_fingerprint(&board) != token.base_fingerprint {
                return Ok(());
            }
        }
        return Err(ApiError::conflict(
            "An invalid or incompatible durable pending claim already uses this FlowScript review token.",
        ));
    }

    let (board_commands, replacement_mode) = local_payload.ok_or_else(|| {
        ApiError::conflict(
            "The canonical board changed before its FlowScript review could be retained.",
        )
    })?;

    retain_pending_claim_on_board(
        &mut board,
        &scope_key,
        token,
        replacement_mode,
        board_commands,
    )
    .map_err(|error| ApiError::internal(format!("durable FlowScript claim failed: {error}")))?;
    mutation_guard.ensure_held()?;
    board.save(None).await?;
    Ok(())
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/flow-ir-commit/disposition",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = Object,
    responses(
        (status = 200, description = "Retained FlowScript review disposition", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 423, description = "Another writer holds this board's mutation lease (code BOARD_LOCKED). Nothing was written; retry the identical request shortly.")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/flow-ir-commit/disposition",
    skip(state, user, params)
)]
pub async fn flow_ir_commit_disposition(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(params): Json<FlowIrCommitDispositionBody>,
) -> Result<Json<FlowIrCommitDispositionResult>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    if !commit_token_is_valid(&params.token, &board_id) {
        return Ok(Json(FlowIrCommitDispositionResult::error(
            "IR_COMMIT_TOKEN_INVALID",
            "The compiled workflow review token is incomplete or belongs to another board.",
        )));
    }

    let scope_key = flow_ir_draft_store_key(&sub, &app_id, &board_id);
    let mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;
    if matches!(params.disposition, FlowIrCommitDisposition::Applied) {
        return Ok(Json(FlowIrCommitDispositionResult::error(
            "IR_COMMIT_ATOMIC_APPLY_REQUIRED",
            "Compiled workflow changes must be applied through the atomic Apply endpoint; a separate applied acknowledgement is not accepted.",
        )));
    }

    let mut board = match state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await
    {
        Ok(board) => board,
        Err(error) => {
            return Ok(Json(FlowIrCommitDispositionResult::error(
                "IR_COMMIT_BOARD_UNAVAILABLE",
                format!(
                    "The canonical review board could not be loaded; the review was not resolved: {error}"
                ),
            )));
        }
    };
    let now_ms = wall_clock_ms();
    let pending_key = pending_claim_ref_key(&scope_key, &params.token);
    let durable_pending_present = board.internal_ref(&pending_key).is_some();
    let durable_pending = pending_claim_from_board(&board, &scope_key, &params.token, now_ms);
    let store = state.flow_ir_draft_stores.get(&scope_key);

    match params.disposition {
        FlowIrCommitDisposition::Applied => unreachable!("handled above"),
        FlowIrCommitDisposition::Dismissed => {
            if durable_pending_present {
                // Dismiss changes host-only bookkeeping, never workflow semantics, so ReadBoards
                // remains intentional: a user allowed to generate/review must also be able to
                // release that review without WriteBoards. Possession of the fully scoped token
                // safely authorizes cleanup even when the exact value is expired or malformed;
                // malformed claims are never eligible for Apply.
                board.remove_internal_ref(&pending_key);
                prune_durable_pending_claims(
                    &mut board,
                    now_ms,
                    FLOW_IR_DURABLE_PENDING_MAX_ENTRIES,
                    FLOW_IR_DURABLE_PENDING_MAX_TOTAL_BYTES,
                )
                .map_err(|error| {
                    ApiError::internal(format!("durable FlowScript claim pruning failed: {error}"))
                })?;
                mutation_guard.ensure_held()?;
                board.save(None).await?;
            }
            let released_local = store.as_ref().is_some_and(|store| {
                store.release_commit_if_matches(
                    &params.token.draft_id,
                    params.token.revision,
                    &params.token.base_fingerprint,
                    &params.token.claim_id,
                )
            });
            if durable_pending_present || released_local {
                Ok(Json(FlowIrCommitDispositionResult::success(
                    "dismissed",
                    "The compiled workflow review was dismissed and its exact durable claim was released.",
                )))
            } else {
                Ok(Json(FlowIrCommitDispositionResult::error(
                    "IR_COMMIT_TOKEN_INVALID",
                    "The compiled workflow review token no longer identifies a durable or process-local pending revision.",
                )))
            }
        }
        FlowIrCommitDisposition::Preflight => {
            let durable_current = durable_pending.is_some()
                && board_fingerprint(&board) == params.token.base_fingerprint;
            let local_current = store.as_ref().is_some_and(|store| {
                store.pending_commit_is_current(
                    &board,
                    &params.token.draft_id,
                    params.token.revision,
                    &params.token.base_fingerprint,
                    &params.token.claim_id,
                )
            });
            if durable_current || (!durable_pending_present && local_current) {
                Ok(Json(FlowIrCommitDispositionResult::success(
                    "current",
                    "The compiled workflow review still matches the canonical board and may be applied.",
                )))
            } else {
                Ok(Json(FlowIrCommitDispositionResult::error(
                    "IR_COMMIT_REVIEW_STALE",
                    "The canonical board or retained compiled revision changed after this review was generated. Dismiss it and regenerate against the current board.",
                )))
            }
        }
    }
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/flow-ir-commit/apply",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = Object,
    responses(
        (status = 200, description = "Exact retained FlowScript command batch apply result", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 423, description = "Another writer holds this board's mutation lease (code BOARD_LOCKED). Nothing was written; retry the identical request shortly.")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/flow-ir-commit/apply",
    skip(state, user, params)
)]
pub async fn apply_flow_ir_commit(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(params): Json<ApplyFlowIrCommitBody>,
) -> Result<Json<ApplyFlowIrCommitResult>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;

    if !commit_token_is_valid(&params.token, &board_id) || app_id.trim().is_empty() {
        return Ok(Json(ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_TOKEN_INVALID",
            "The compiled workflow review token or app id is incomplete or mismatched.",
        )));
    }

    let scope_key = flow_ir_draft_store_key(&sub, &app_id, &board_id);
    let mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;

    let mut board = match state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await
    {
        Ok(board) => board,
        Err(error) => {
            return Ok(Json(ApplyFlowIrCommitResult::empty(
                "error",
                "IR_COMMIT_BOARD_UNAVAILABLE",
                format!(
                    "The canonical review board could not be loaded; nothing was applied: {error}"
                ),
            )));
        }
    };
    let now_ms = wall_clock_ms();
    let requested_receipt_key = applied_receipt_ref_key(&scope_key, &params.token);
    let requested_pending_key = pending_claim_ref_key(&scope_key, &params.token);
    let requested_receipt_was_present = board.internal_ref(&requested_receipt_key).is_some();
    let receipts_pruned = match prune_durable_applied_receipts(
        &mut board,
        now_ms,
        FLOW_IR_DURABLE_RECEIPT_MAX_ENTRIES,
        FLOW_IR_DURABLE_RECEIPT_MAX_TOTAL_BYTES,
    ) {
        Ok(pruned) => pruned,
        Err(error) => {
            return Ok(Json(ApplyFlowIrCommitResult::empty(
                "error",
                "IR_COMMIT_RECEIPT_PERSISTENCE_FAILED",
                format!(
                    "The canonical board's durable apply receipts could not be validated; nothing was applied: {error}"
                ),
            )));
        }
    };

    // This canonical-board lookup intentionally precedes the process-local draft lookup. A caller
    // that lost the first success response can therefore recover after an API restart or on a
    // different replica without retaining the pre-apply draft in shared memory.
    if let Some(receipt) =
        replay_applied_receipt_from_board(&board, &scope_key, &params.token, now_ms)
    {
        let obsolete_pending_removed = board.remove_internal_ref(&requested_pending_key).is_some();
        // Bookkeeping-only, but still a full-object PUT of a board loaded before the lease
        // lapsed: skipping it costs one more prune later, writing it would erase another
        // replica's graph mutation.
        if (receipts_pruned || obsolete_pending_removed)
            && mutation_guard.ensure_held().is_ok()
            && let Err(error) = board.save(None).await
        {
            tracing::warn!(
                app_id,
                board_id,
                error = %error,
                "Replayed a durable FlowScript apply receipt, but receipt pruning could not be persisted"
            );
        }
        return Ok(Json(receipt));
    }
    if requested_receipt_was_present {
        // Do not persist pruning of this exact invalid key. Leaving it in canonical storage makes
        // every retry fail closed instead of falling through to a still-live pending claim and
        // potentially applying a batch whose prior success marker was corrupted.
        return Ok(Json(ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_RECEIPT_INVALID",
            "An exact receipt key exists for this review, but its version, identity, age, or success payload is invalid. The batch was not applied again; regenerate against the canonical board.",
        )));
    }

    let requested_pending_was_present = board.internal_ref(&requested_pending_key).is_some();
    let durable_pending = pending_claim_from_board(&board, &scope_key, &params.token, now_ms);
    let pending_claims_pruned = prune_durable_pending_claims(
        &mut board,
        now_ms,
        FLOW_IR_DURABLE_PENDING_MAX_ENTRIES,
        FLOW_IR_DURABLE_PENDING_MAX_TOTAL_BYTES,
    )
    .map_err(|error| {
        ApiError::internal(format!("durable FlowScript claim pruning failed: {error}"))
    })?;
    if requested_pending_was_present && durable_pending.is_none() {
        // Keep the invalid exact key durable until an explicit Dismiss removes it. Persisting its
        // automatic prune would let the next retry fall back to process-local state and bypass the
        // fail-closed decision made here.
        return Ok(Json(ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_PENDING_CLAIM_INVALID",
            "An exact pending claim exists for this review, but its version, identity, age, or batch is invalid. Nothing was applied; dismiss or regenerate against the canonical board.",
        )));
    }

    let store = state.flow_ir_draft_stores.get(&scope_key);
    let (board_commands, replacement_mode, selected_payload_digest, uses_durable_pending) =
        if let Some(claim) = durable_pending {
            if board_fingerprint(&board) != params.token.base_fingerprint {
                return Ok(Json(ApplyFlowIrCommitResult::empty(
                    "stale",
                    "IR_COMMIT_REVIEW_STALE",
                    "The canonical board changed after this durable FlowScript review was generated. Nothing was applied.",
                )));
            }
            (
                claim.board_commands,
                claim.replacement_mode,
                claim.payload_digest,
                true,
            )
        } else if let Some(store) = store.as_ref() {
            let Some((board_commands, replacement_mode)) = store.pending_commit_payload_if_current(
                &board,
                &params.token.draft_id,
                params.token.revision,
                &params.token.base_fingerprint,
                &params.token.claim_id,
            ) else {
                return Ok(Json(ApplyFlowIrCommitResult::empty(
                    "stale",
                    "IR_COMMIT_REVIEW_STALE",
                    "The canonical board or retained compiled revision changed after this review was generated. Nothing was applied.",
                )));
            };
            let payload_digest = pending_payload_digest(&board_commands, replacement_mode)
                .map_err(ApiError::internal)?;
            (board_commands, replacement_mode, payload_digest, false)
        } else {
            // Same reasoning as the replay path above: bookkeeping never outranks the lease.
            if (receipts_pruned || pending_claims_pruned)
                && mutation_guard.ensure_held().is_ok()
                && let Err(error) = board.save(None).await
            {
                tracing::warn!(
                    app_id,
                    board_id,
                    error = %error,
                    "FlowScript review was unavailable and durable bookkeeping pruning could not be persisted"
                );
            }
            return Ok(Json(ApplyFlowIrCommitResult::empty(
                "stale",
                "IR_COMMIT_TOKEN_INVALID",
                "The compiled workflow review is no longer retained by this API process and no matching durable pending claim or applied receipt exists.",
            )));
        };

    let destructive_items = destructive_review_items(replacement_mode, &board_commands);
    if !destructive_items.is_empty() && !params.approve_destructive {
        return Ok(Json(ApplyFlowIrCommitResult::apply_error(
            "IR_COMMIT_DESTRUCTIVE_APPROVAL_REQUIRED",
            "The exact compiled workflow batch removes or replaces existing board state. Explicit destructive approval is required; nothing was applied and the claim remains pending.",
            board_commands,
            destructive_items,
        )));
    }

    let flow_state = if let Some(flow_state) = &board.app_state {
        flow_state.clone()
    } else {
        match state
            .scoped_credentials(
                &sub,
                &app_id,
                crate::credentials::CredentialsAccess::EditApp,
            )
            .await
        {
            Ok(credentials) => match credentials.to_state(state.clone()).await {
                Ok(flow_state) => Arc::new(flow_state),
                Err(error) => {
                    return Ok(Json(ApplyFlowIrCommitResult::empty(
                        "error",
                        "IR_COMMIT_PERSISTENCE_UNAVAILABLE",
                        format!(
                            "The app-scoped board state is unavailable; nothing was applied: {error}"
                        ),
                    )));
                }
            },
            Err(error) => {
                return Ok(Json(ApplyFlowIrCommitResult::empty(
                    "error",
                    "IR_COMMIT_PERSISTENCE_UNAVAILABLE",
                    format!(
                        "The app-scoped board credentials are unavailable; nothing was applied: {error}"
                    ),
                )));
            }
        }
    };

    let persisted_original = board.clone();
    let wasm_nodes = match app_wasm_nodes(&state, &app_id).await {
        Ok(nodes) => nodes,
        Err(error) => {
            return Ok(Json(ApplyFlowIrCommitResult::empty(
                "error",
                "IR_COMMIT_CATALOG_UNAVAILABLE",
                format!("The app node catalog is unavailable; nothing was applied: {error}"),
            )));
        }
    };
    let builtin_nodes = state.registry.as_ref().get_nodes();
    hydrate_board_wasm_metadata(&mut board, &wasm_nodes, &builtin_nodes);
    let mut catalog_nodes = builtin_nodes;
    catalog_nodes.extend(wasm_nodes);

    let retained_commands = board_commands.clone();
    let apply_result = match flow_like::flow::ast::apply_board_commands_to_board(
        &mut board,
        board_commands,
        &catalog_nodes,
        flow_state,
        None,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            return Ok(Json(ApplyFlowIrCommitResult::apply_error(
                "IR_COMMIT_APPLY_FAILED",
                format!(
                    "The exact compiled workflow batch could not be applied and remains retryable: {error}"
                ),
                retained_commands,
                vec![error.to_string()],
            )));
        }
    };

    if apply_result.commands.is_empty() || !apply_result.diagnostics.is_empty() {
        let diagnostics = if apply_result.diagnostics.is_empty() {
            vec!["The exact compiled workflow batch produced no executed commands.".to_string()]
        } else {
            apply_result.diagnostics
        };
        return Ok(Json(ApplyFlowIrCommitResult::apply_error(
            "IR_COMMIT_APPLY_FAILED",
            "The exact compiled workflow batch did not complete; its claim remains available for retry or dismissal.",
            apply_result.board_commands,
            diagnostics,
        )));
    }

    // The distributed app+board guard excludes canonical API writers across replicas. Reload
    // immediately before save as an additional fail-closed check against an out-of-process writer
    // that does not participate in that database-lock protocol.
    let persisted_base = match state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await
    {
        Ok(board) => board,
        Err(error) => {
            return Ok(Json(ApplyFlowIrCommitResult::apply_error(
                "IR_COMMIT_BOARD_UNAVAILABLE",
                format!(
                    "The canonical board could not be revalidated before persistence; nothing was saved: {error}"
                ),
                apply_result.board_commands,
                vec![error.to_string()],
            )));
        }
    };
    let durable_still_current =
        pending_claim_from_board(&persisted_base, &scope_key, &params.token, wall_clock_ms())
            .is_some_and(|claim| {
                claim.payload_digest == selected_payload_digest
                    && claim.replacement_mode == replacement_mode
            })
            && board_fingerprint(&persisted_base) == params.token.base_fingerprint;
    let local_still_current = store.as_ref().is_some_and(|store| {
        store
            .pending_commit_payload_if_current(
                &persisted_base,
                &params.token.draft_id,
                params.token.revision,
                &params.token.base_fingerprint,
                &params.token.claim_id,
            )
            .is_some_and(|(commands, retained_replacement_mode)| {
                retained_replacement_mode == replacement_mode
                    && pending_payload_digest(&commands, retained_replacement_mode)
                        .ok()
                        .as_deref()
                        == Some(selected_payload_digest.as_str())
            })
    });
    if (uses_durable_pending && !durable_still_current)
        || (!uses_durable_pending && !local_still_current)
    {
        return Ok(Json(ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_REVIEW_STALE",
            "The canonical board changed while the exact batch was being prepared. Nothing was saved.",
        )));
    }

    let result = ApplyFlowIrCommitResult {
        status: "applied".to_string(),
        replayed: false,
        code: None,
        message: format!(
            "Applied and persisted {} exact compiled workflow command(s).",
            apply_result.commands.len()
        ),
        commands: apply_result.commands.clone(),
        board_commands: apply_result.board_commands.clone(),
        diagnostics: apply_result.diagnostics.clone(),
        final_board_node_count: Some(board_total_node_count(&board)),
    };
    // Pending -> applied is one canonical state transition: no replica can observe the graph
    // mutation without its success receipt, or retain an applicable claim after that mutation.
    board.remove_internal_ref(&requested_pending_key);
    if let Err(error) =
        retain_applied_receipt_on_board(&mut board, &scope_key, &params.token, &result)
    {
        return Ok(Json(ApplyFlowIrCommitResult::apply_error(
            "IR_COMMIT_RECEIPT_PERSISTENCE_FAILED",
            "The compiled workflow was not persisted because its bounded crash-recovery receipt could not be prepared. The claim remains retryable.",
            apply_result.board_commands,
            vec![error],
        )));
    }

    // The mutation and its exact success receipt share one compressed board write. A retry can
    // therefore observe either neither or both, including after this process exits immediately
    // after persistence.
    mutation_guard.ensure_held()?;
    let saved = super::scoring::save_board_and_refresh_summary(&state, &app_id, &board).await;
    let put = match saved {
        Ok(put) => put,
        Err(error) => {
            let restore_error = restore_persisted_snapshot(&persisted_original).await;
            let mut diagnostics = vec![format!("Board persistence failed: {error}")];
            if let Some(error) = restore_error {
                diagnostics.push(format!(
                    "Restoring the persisted board snapshot also failed: {error}"
                ));
            }
            return Ok(Json(ApplyFlowIrCommitResult::apply_error(
                "IR_COMMIT_SAVE_FAILED",
                "The compiled workflow batch could not be persisted. The claim remains retryable.",
                apply_result.board_commands,
                diagnostics,
            )));
        }
    };

    if let Some(store) = store
        && !store.acknowledge_applied_commit(
            &board,
            &params.token.draft_id,
            params.token.revision,
            &params.token.base_fingerprint,
            &params.token.claim_id,
        )
    {
        let released = store.release_commit_if_matches(
            &params.token.draft_id,
            params.token.revision,
            &params.token.base_fingerprint,
            &params.token.claim_id,
        );
        tracing::warn!(
            app_id,
            board_id,
            draft_id = %params.token.draft_id,
            revision = params.token.revision,
            released,
            "The FlowScript claim acknowledgement raced after its mutation and durable receipt were persisted; keeping the canonical applied board"
        );
    }
    super::sync_board::seed_board_revision(&state, &app_id, &board_id, board, &put).await;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use flow_like::flow::copilot::{BoardCommand, FlowIrCommitToken};

    use super::*;

    fn token() -> FlowIrCommitToken {
        FlowIrCommitToken {
            board_id: "board".to_string(),
            draft_id: "draft".to_string(),
            revision: 3,
            base_fingerprint: "fingerprint".to_string(),
            claim_id: "claim".to_string(),
            requires_destructive_approval: false,
        }
    }

    #[test]
    fn commit_token_must_match_route_board_and_be_complete() {
        assert!(commit_token_is_valid(&token(), "board"));
        assert!(!commit_token_is_valid(&token(), "other"));

        let mut incomplete = token();
        incomplete.claim_id.clear();
        assert!(!commit_token_is_valid(&incomplete, "board"));
    }

    #[test]
    fn destructive_policy_is_derived_from_retained_batch_and_mode() {
        let removal = BoardCommand::RemoveNode {
            node_id: "old-node".to_string(),
            summary: None,
        };
        assert_eq!(
            destructive_review_items(false, &[removal]),
            vec!["node `old-node`".to_string()]
        );
        assert_eq!(
            destructive_review_items(true, &[]),
            vec!["The draft uses full-board replacement semantics.".to_string()]
        );
    }

    fn applied_result() -> ApplyFlowIrCommitResult {
        ApplyFlowIrCommitResult {
            status: "applied".to_string(),
            replayed: false,
            code: None,
            message: "Applied exact typed batch.".to_string(),
            commands: Vec::new(),
            board_commands: Vec::new(),
            diagnostics: Vec::new(),
            final_board_node_count: Some(3),
        }
    }

    fn detached_board() -> Board {
        Board::new_detached(
            Some("board".to_string()),
            flow_like::flow_like_storage::Path::from("/test"),
        )
    }

    fn insert_test_receipt(
        board: &mut Board,
        scope_key: &str,
        token: &FlowIrCommitToken,
        version: u8,
        created_at_ms: u64,
        identity_digest: Option<String>,
    ) -> String {
        let key = applied_receipt_ref_key(scope_key, token);
        let receipt = DurableFlowIrAppliedReceipt {
            version,
            created_at_ms,
            identity_digest: identity_digest
                .unwrap_or_else(|| applied_receipt_identity_digest(scope_key, token)),
            result: applied_result(),
        };
        board
            .insert_internal_ref(
                key.clone(),
                serde_json::to_string(&receipt).expect("receipt serialization"),
            )
            .expect("internal receipt");
        key
    }

    fn pending_commands(node_id: &str) -> Vec<BoardCommand> {
        vec![BoardCommand::RemoveNode {
            node_id: node_id.to_string(),
            summary: None,
        }]
    }

    fn insert_test_pending_claim(
        board: &mut Board,
        scope_key: &str,
        token: &FlowIrCommitToken,
        version: u8,
        created_at_ms: u64,
        replacement_mode: bool,
    ) -> String {
        let key = pending_claim_ref_key(scope_key, token);
        let board_commands = pending_commands(&token.claim_id);
        let claim = DurableFlowIrPendingClaim {
            version,
            created_at_ms,
            identity_digest: pending_claim_identity_digest(scope_key, token),
            payload_digest: pending_payload_digest(&board_commands, replacement_mode)
                .expect("payload digest"),
            replacement_mode,
            board_commands,
        };
        board
            .insert_internal_ref(
                key.clone(),
                serde_json::to_string(&claim).expect("pending claim serialization"),
            )
            .expect("internal pending claim");
        key
    }

    #[test]
    fn durable_receipt_identity_is_stable_and_fully_scoped() {
        let token = token();
        let first = applied_receipt_identity_digest("principal-a/app-a/board", &token);
        let replay = applied_receipt_identity_digest("principal-a/app-a/board", &token);
        let other_principal = applied_receipt_identity_digest("principal-b/app-a/board", &token);

        assert_eq!(first, replay);
        assert_ne!(first, other_principal);
        for altered in [
            FlowIrCommitToken {
                board_id: "other-board".to_string(),
                ..token.clone()
            },
            FlowIrCommitToken {
                draft_id: "other-draft".to_string(),
                ..token.clone()
            },
            FlowIrCommitToken {
                revision: token.revision + 1,
                ..token.clone()
            },
            FlowIrCommitToken {
                base_fingerprint: "other-fingerprint".to_string(),
                ..token.clone()
            },
            FlowIrCommitToken {
                claim_id: "other-claim".to_string(),
                ..token.clone()
            },
        ] {
            assert_ne!(
                first,
                applied_receipt_identity_digest("principal-a/app-a/board", &altered),
                "every authoritative token field must bind the durable identity"
            );
        }
        assert!(
            applied_receipt_ref_key("principal-a/app-a/board", &token)
                .starts_with(FLOW_IR_DURABLE_RECEIPT_REF_PREFIX)
        );
        assert_ne!(
            applied_receipt_identity_digest("principal-a/app-a/board", &token),
            pending_claim_identity_digest("principal-a/app-a/board", &token),
            "pending and applied authority domains must not alias"
        );
    }

    #[test]
    fn durable_pending_claim_roundtrips_exact_batch_policy_and_scope() {
        let mut board = detached_board();
        let token = token();
        let scope = "principal-a/app-a/board";
        let commands = pending_commands("reviewed-node");
        let expected_digest = pending_payload_digest(&commands, true).expect("payload digest");
        assert_ne!(
            expected_digest,
            pending_payload_digest(&commands, false).expect("additive payload digest"),
            "the authoritative replacement policy is covered by the payload digest"
        );
        retain_pending_claim_on_board(&mut board, scope, &token, true, commands)
            .expect("retain durable pending claim");

        let claim = pending_claim_from_board(&board, scope, &token, wall_clock_ms())
            .expect("exact pending claim");
        assert!(claim.replacement_mode);
        assert_eq!(claim.payload_digest, expected_digest);
        assert_eq!(claim.board_commands.len(), 1);
        assert!(
            pending_claim_from_board(&board, "principal-b/app-a/board", &token, wall_clock_ms())
                .is_none()
        );

        let key = pending_claim_ref_key(scope, &token);
        let mut tampered = serde_json::from_str::<DurableFlowIrPendingClaim>(
            board.internal_ref(&key).expect("encoded pending claim"),
        )
        .expect("decode pending claim for tampering");
        tampered.replacement_mode = false;
        board.remove_internal_ref(&key);
        board
            .insert_internal_ref(
                key,
                serde_json::to_string(&tampered).expect("tampered claim serialization"),
            )
            .expect("tampered internal pending claim");
        assert!(
            pending_claim_from_board(&board, scope, &token, wall_clock_ms()).is_none(),
            "commands and host-derived replacement policy are covered by the payload digest"
        );
    }

    #[test]
    fn pending_to_applied_transition_removes_claim_and_keeps_replay() {
        let mut board = detached_board();
        let token = token();
        let scope = "principal/app/board";
        retain_pending_claim_on_board(
            &mut board,
            scope,
            &token,
            false,
            pending_commands("reviewed-node"),
        )
        .expect("pending claim");

        board.remove_internal_ref(&pending_claim_ref_key(scope, &token));
        retain_applied_receipt_on_board(&mut board, scope, &token, &applied_result())
            .expect("applied receipt");

        assert!(pending_claim_from_board(&board, scope, &token, wall_clock_ms()).is_none());
        assert!(
            replay_applied_receipt_from_board(&board, scope, &token, wall_clock_ms()).is_some()
        );
    }

    #[test]
    fn pending_claim_pruning_rejects_expired_versions_and_honors_bounds() {
        let now_ms = FLOW_IR_DURABLE_PENDING_TTL_MS.saturating_add(10_000);
        let scope = "principal/app/board";
        let mut board = detached_board();

        let mut expired = token();
        expired.claim_id = "expired".to_string();
        let expired_key = insert_test_pending_claim(&mut board, scope, &expired, 1, 1, false);
        let mut unknown = token();
        unknown.claim_id = "unknown-version".to_string();
        let unknown_key = insert_test_pending_claim(&mut board, scope, &unknown, 2, now_ms, false);
        let mut valid_keys = Vec::new();
        for (claim_id, age_ms) in [("old", 30), ("middle", 20), ("new", 10)] {
            let mut current = token();
            current.claim_id = claim_id.to_string();
            valid_keys.push(insert_test_pending_claim(
                &mut board,
                scope,
                &current,
                1,
                now_ms.saturating_sub(age_ms),
                claim_id == "new",
            ));
        }

        assert!(
            prune_durable_pending_claims(&mut board, now_ms, 2, usize::MAX)
                .expect("bounded pending prune")
        );
        assert!(board.internal_ref(&expired_key).is_none());
        assert!(board.internal_ref(&unknown_key).is_none());
        assert!(board.internal_ref(&valid_keys[0]).is_none());
        assert!(board.internal_ref(&valid_keys[1]).is_some());
        assert!(board.internal_ref(&valid_keys[2]).is_some());
        assert!(
            prune_durable_pending_claims(&mut board, now_ms, 2, 0)
                .expect("pending byte-bound prune")
        );
        assert_eq!(
            board
                .internal_refs_with_prefix(FLOW_IR_DURABLE_PENDING_REF_PREFIX)
                .count(),
            0
        );
    }

    #[test]
    fn durable_receipt_replays_without_a_draft_and_rejects_another_scope() {
        let mut board = detached_board();
        let token = token();
        let scope = "principal-a/app-a/board";
        retain_applied_receipt_on_board(&mut board, scope, &token, &applied_result())
            .expect("retain durable receipt");

        let replay = replay_applied_receipt_from_board(&board, scope, &token, wall_clock_ms())
            .expect("exact receipt replay");
        assert_eq!(replay.status, "applied");
        assert!(replay.replayed);
        assert_eq!(replay.final_board_node_count, Some(3));
        assert!(replay.message.contains("durable idempotent replay"));
        assert!(
            replay_applied_receipt_from_board(
                &board,
                "principal-b/app-a/board",
                &token,
                wall_clock_ms()
            )
            .is_none()
        );
    }

    #[test]
    fn malformed_identity_and_unknown_receipt_version_never_replay() {
        let now_ms = 50_000;
        let scope = "principal/app/board";
        let token = token();

        let mut malformed = detached_board();
        insert_test_receipt(
            &mut malformed,
            scope,
            &token,
            1,
            now_ms,
            Some("0".repeat(64)),
        );
        assert!(replay_applied_receipt_from_board(&malformed, scope, &token, now_ms).is_none());

        let mut future_version = detached_board();
        insert_test_receipt(&mut future_version, scope, &token, 2, now_ms, None);
        assert!(
            replay_applied_receipt_from_board(&future_version, scope, &token, now_ms).is_none()
        );
    }

    #[test]
    fn durable_receipt_pruning_enforces_ttl_entry_and_total_size_bounds() {
        let now_ms = FLOW_IR_DURABLE_RECEIPT_TTL_MS.saturating_add(10_000);
        let scope = "principal/app/board";
        let mut board = detached_board();

        let mut expired = token();
        expired.claim_id = "expired".to_string();
        let expired_key = insert_test_receipt(&mut board, scope, &expired, 1, 1, None);

        let mut valid_keys = Vec::new();
        for (claim_id, age_ms) in [("old", 30), ("middle", 20), ("new", 10)] {
            let mut current = token();
            current.claim_id = claim_id.to_string();
            valid_keys.push(insert_test_receipt(
                &mut board,
                scope,
                &current,
                1,
                now_ms.saturating_sub(age_ms),
                None,
            ));
        }

        assert!(
            prune_durable_applied_receipts(&mut board, now_ms, 2, usize::MAX)
                .expect("bounded prune")
        );
        assert!(board.internal_ref(&expired_key).is_none());
        assert!(board.internal_ref(&valid_keys[0]).is_none());
        assert!(board.internal_ref(&valid_keys[1]).is_some());
        assert!(board.internal_ref(&valid_keys[2]).is_some());

        assert!(
            prune_durable_applied_receipts(&mut board, now_ms, 2, 0).expect("byte-bound prune")
        );
        assert_eq!(
            board
                .internal_refs_with_prefix(FLOW_IR_DURABLE_RECEIPT_REF_PREFIX)
                .count(),
            0
        );
    }
}
