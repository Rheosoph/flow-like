use std::{
    collections::HashSet,
    sync::{Arc, LazyLock},
    time::Duration,
};

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::{
    board::{Board, commands::GenericCommand},
    copilot::{BoardCommand, FlowIrCommitToken},
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

// A successful response can be lost after persistence and acknowledgement. Keep a bounded receipt
// so retrying the exact token is idempotent for as long as its process-local draft could have been
// retained. A multi-replica deployment needs to move both drafts and receipts to the same shared
// durable store.
static FLOW_IR_APPLIED_RECEIPTS: LazyLock<moka::sync::Cache<String, Arc<ApplyFlowIrCommitResult>>> =
    LazyLock::new(|| {
        moka::sync::Cache::builder()
            .max_capacity(2_048)
            .time_to_idle(Duration::from_secs(2 * 60 * 60))
            .build()
    });

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

fn commit_token_is_valid(token: &FlowIrCommitToken, board_id: &str) -> bool {
    !board_id.trim().is_empty()
        && token.board_id == board_id
        && !token.board_id.trim().is_empty()
        && !token.draft_id.trim().is_empty()
        && !token.base_fingerprint.trim().is_empty()
        && !token.claim_id.trim().is_empty()
}

fn applied_receipt_key(scope_key: &str, token: &FlowIrCommitToken) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        scope_key,
        token.draft_id.trim(),
        token.revision,
        token.base_fingerprint.trim(),
        token.claim_id.trim()
    )
}

fn replay_applied_receipt(
    scope_key: &str,
    token: &FlowIrCommitToken,
) -> Option<ApplyFlowIrCommitResult> {
    FLOW_IR_APPLIED_RECEIPTS
        .get(&applied_receipt_key(scope_key, token))
        .map(|receipt| {
            let mut replay = receipt.as_ref().clone();
            replay.message = format!("{} (idempotent replay)", replay.message);
            replay
        })
}

fn retain_applied_receipt(
    scope_key: &str,
    token: &FlowIrCommitToken,
    result: &ApplyFlowIrCommitResult,
) {
    FLOW_IR_APPLIED_RECEIPTS.insert(
        applied_receipt_key(scope_key, token),
        Arc::new(result.clone()),
    );
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
        (status = 403, description = "Forbidden")
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
    let mutation_lock = state.board_mutation_lock(&app_id, &board_id);
    let _mutation_guard = mutation_lock.lock().await;
    let Some(store) = state.flow_ir_draft_stores.get(&scope_key) else {
        return Ok(Json(FlowIrCommitDispositionResult::error(
            "IR_COMMIT_TOKEN_INVALID",
            "The compiled workflow review is no longer retained by this API process.",
        )));
    };

    match params.disposition {
        FlowIrCommitDisposition::Applied => Ok(Json(FlowIrCommitDispositionResult::error(
            "IR_COMMIT_ATOMIC_APPLY_REQUIRED",
            "Compiled workflow changes must be applied through the atomic Apply endpoint; a separate applied acknowledgement is not accepted.",
        ))),
        FlowIrCommitDisposition::Dismissed => {
            if store.release_commit_if_matches(
                &params.token.draft_id,
                params.token.revision,
                &params.token.base_fingerprint,
                &params.token.claim_id,
            ) {
                Ok(Json(FlowIrCommitDispositionResult::success(
                    "dismissed",
                    "The compiled workflow review was dismissed and its exact revision was released.",
                )))
            } else {
                Ok(Json(FlowIrCommitDispositionResult::error(
                    "IR_COMMIT_TOKEN_INVALID",
                    "The compiled workflow review token no longer identifies a pending revision.",
                )))
            }
        }
        FlowIrCommitDisposition::Preflight => {
            let board = match state
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
            if store.pending_commit_is_current(
                &board,
                &params.token.draft_id,
                params.token.revision,
                &params.token.base_fingerprint,
                &params.token.claim_id,
            ) {
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
        (status = 403, description = "Forbidden")
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
    let mutation_lock = state.board_mutation_lock(&app_id, &board_id);
    let _mutation_guard = mutation_lock.lock().await;

    // Re-check after acquiring the board mutation lock so two simultaneous retries cannot both pass
    // the initial lookup before the first one records its successful receipt.
    if let Some(receipt) = replay_applied_receipt(&scope_key, &params.token) {
        return Ok(Json(receipt));
    }

    let Some(store) = state.flow_ir_draft_stores.get(&scope_key) else {
        return Ok(Json(ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_TOKEN_INVALID",
            "The compiled workflow review is no longer retained by this API process.",
        )));
    };

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

    let Some(board_commands) = store.pending_commands_if_current(
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

    let Some(replacement_mode) = store.pending_commit_requires_destructive_approval(
        &params.token.draft_id,
        params.token.revision,
        &params.token.base_fingerprint,
        &params.token.claim_id,
    ) else {
        return Ok(Json(ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_REVIEW_STALE",
            "The retained compiled revision changed while its review policy was checked. Nothing was applied.",
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

    // The process-local app+board mutex excludes every canonical API writer. Reload immediately
    // before save as a fail-closed check against a non-sticky replica or an out-of-process writer.
    // It cannot eliminate the final distributed load/save race without a shared lock or
    // conditional object-store write.
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
    if !store.pending_commit_is_current(
        &persisted_base,
        &params.token.draft_id,
        params.token.revision,
        &params.token.base_fingerprint,
        &params.token.claim_id,
    ) {
        return Ok(Json(ApplyFlowIrCommitResult::empty(
            "stale",
            "IR_COMMIT_REVIEW_STALE",
            "The canonical board changed while the exact batch was being prepared. Nothing was saved.",
        )));
    }

    if let Err(error) = board.save(None).await {
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

    if !store.acknowledge_applied_commit(
        &board,
        &params.token.draft_id,
        params.token.revision,
        &params.token.base_fingerprint,
        &params.token.claim_id,
    ) {
        let restore_error = restore_persisted_snapshot(&persisted_original).await;
        let mut diagnostics = vec![
            "The exact compiled workflow claim could not be acknowledged after persistence."
                .to_string(),
        ];
        if let Some(error) = restore_error {
            diagnostics.push(format!(
                "Restoring the persisted board snapshot also failed: {error}"
            ));
        }
        return Ok(Json(ApplyFlowIrCommitResult::apply_error(
            "IR_COMMIT_ACK_FAILED",
            "The compiled workflow batch could not be acknowledged, so the previous board snapshot was restored where possible.",
            apply_result.board_commands,
            diagnostics,
        )));
    }

    let result = ApplyFlowIrCommitResult {
        status: "applied".to_string(),
        code: None,
        message: format!(
            "Applied and persisted {} exact compiled workflow command(s).",
            apply_result.commands.len()
        ),
        commands: apply_result.commands,
        board_commands: apply_result.board_commands,
        diagnostics: apply_result.diagnostics,
        final_board_node_count: Some(board_total_node_count(&board)),
    };
    retain_applied_receipt(&scope_key, &params.token, &result);
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use flow_like::flow::copilot::{BoardCommand, FlowIrCommitToken};

    use super::{commit_token_is_valid, destructive_review_items};

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
}
