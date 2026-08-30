use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        board::{
            scoring::save_board_and_refresh_summary,
            sync_board::{board_sync_tail, seed_board_revision},
        },
        wasm_catalog::{
            app_wasm_nodes_cached, hydrate_board_wasm_metadata, sanitize_wasm_command_metadata,
        },
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::HeaderMap,
};
use flow_like::flow::board::{
    commands::{GenericCommand, canonical_commands_digest},
    sync::{BoardSyncRequest, BoardSyncResponse},
};
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use utoipa::ToSchema;

#[derive(Clone, Deserialize, ToSchema)]
pub struct ExecuteCommandsBody {
    #[schema(value_type = Vec<Object>)]
    pub commands: Vec<GenericCommand>,
    /// The client's board sync manifest. When present the response also carries the sync diff
    /// against the revision this batch produced, saving the follow-up `/sync` round trip. Absent
    /// for clients that predate it, which get the bare command list back as before.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub sync: Option<BoardSyncRequest>,
}

/// `Commands` when the request carried no `sync`, `WithSync` when it did. `sync: null` inside
/// `WithSync` means the tail could not be built (never an error — the write is committed) and the
/// client should sync separately.
#[allow(clippy::large_enum_variant)] // untagged response body, built once per request and moved straight into Json(..)
#[derive(Serialize)]
#[serde(untagged)]
pub enum ExecuteCommandsResponse {
    Commands(Vec<GenericCommand>),
    WithSync {
        commands: Vec<GenericCommand>,
        sync: Option<BoardSyncResponse>,
    },
}

impl ExecuteCommandsResponse {
    fn new(commands: Vec<GenericCommand>, sync: Option<Option<BoardSyncResponse>>) -> Self {
        match sync {
            Some(sync) => Self::WithSync { commands, sync },
            None => Self::Commands(commands),
        }
    }
}

const COMMAND_RECEIPT_REF_PREFIX: &str = "__flow_like_internal_v1/command-receipt/";
const COMMAND_RECEIPT_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const FLOWPILOT_COMMAND_RECEIPT_TTL_MS: u64 = 10 * 365 * 24 * 60 * 60 * 1_000;
const COMMAND_RECEIPT_FUTURE_TOLERANCE_MS: u64 = 5 * 60 * 1_000;
const COMMAND_RECEIPT_MAX_ENTRIES: usize = 8_192;

#[derive(Debug, Serialize, Deserialize)]
struct DurableCommandReceipt {
    version: u8,
    created_at_ms: u64,
    #[serde(default)]
    expires_at_ms: u64,
    request_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopedCommandReceiptKey {
    ref_key: String,
    retention_ms: u64,
}

fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn scoped_command_receipt_key(
    headers: &HeaderMap,
    header_name: &'static str,
    display_name: &'static str,
    sub: &str,
    app_id: &str,
    board_id: &str,
) -> Result<Option<ScopedCommandReceiptKey>, ApiError> {
    let Some(value) = headers.get(header_name) else {
        return Ok(None);
    };
    let key = value
        .to_str()
        .map_err(|_| ApiError::bad_request(format!("{display_name} must be valid ASCII.")))?
        .trim();
    if key.is_empty() || key.len() > 240 {
        return Err(ApiError::bad_request(format!(
            "{display_name} must contain between 1 and 240 characters."
        )));
    }
    let identity = format!("{sub}\u{1f}{app_id}\u{1f}{board_id}\u{1f}{key}");
    Ok(Some(ScopedCommandReceiptKey {
        ref_key: format!(
            "{COMMAND_RECEIPT_REF_PREFIX}{}",
            blake3::hash(identity.as_bytes()).to_hex()
        ),
        retention_ms: if key.starts_with("flowpilot-board-edit:") {
            FLOWPILOT_COMMAND_RECEIPT_TTL_MS
        } else {
            COMMAND_RECEIPT_TTL_MS
        },
    }))
}

fn command_idempotency_key(
    headers: &HeaderMap,
    sub: &str,
    app_id: &str,
    board_id: &str,
) -> Result<Option<ScopedCommandReceiptKey>, ApiError> {
    scoped_command_receipt_key(
        headers,
        "idempotency-key",
        "Idempotency-Key",
        sub,
        app_id,
        board_id,
    )
}

fn command_receipt_ack_key(
    headers: &HeaderMap,
    sub: &str,
    app_id: &str,
    board_id: &str,
) -> Result<Option<ScopedCommandReceiptKey>, ApiError> {
    scoped_command_receipt_key(
        headers,
        "flowlike-idempotency-ack",
        "FlowLike-Idempotency-Ack",
        sub,
        app_id,
        board_id,
    )
}

fn request_digest(commands: &[GenericCommand]) -> Result<String, ApiError> {
    canonical_commands_digest(commands)
        .map_err(|error| ApiError::internal(format!("command digest failed: {error}")))
}

fn prune_command_receipts(
    board: &mut flow_like::flow::board::Board,
    now_ms: u64,
    limit: usize,
) -> Result<bool, ApiError> {
    let before = board
        .internal_refs_with_prefix(COMMAND_RECEIPT_REF_PREFIX)
        .count();
    let mut retained = board
        .internal_refs_with_prefix(COMMAND_RECEIPT_REF_PREFIX)
        .filter_map(|(key, value)| {
            serde_json::from_str::<DurableCommandReceipt>(value)
                .ok()
                .filter(|receipt| {
                    let expires_at_ms = if receipt.expires_at_ms == 0 {
                        receipt.created_at_ms.saturating_add(COMMAND_RECEIPT_TTL_MS)
                    } else {
                        receipt.expires_at_ms
                    };
                    receipt.version == 1
                        && receipt.created_at_ms
                            <= now_ms.saturating_add(COMMAND_RECEIPT_FUTURE_TOLERANCE_MS)
                        && now_ms <= expires_at_ms
                })
                .map(|receipt| {
                    let expires_at_ms = if receipt.expires_at_ms == 0 {
                        receipt.created_at_ms.saturating_add(COMMAND_RECEIPT_TTL_MS)
                    } else {
                        receipt.expires_at_ms
                    };
                    let long_lived = expires_at_ms.saturating_sub(receipt.created_at_ms)
                        > COMMAND_RECEIPT_TTL_MS;
                    (key.to_string(), receipt.created_at_ms, long_lived)
                })
        })
        .collect::<Vec<_>>();
    // An ordinary week-long retry marker must never evict a pending FlowPilot delivery marker.
    retained.sort_by_key(|(_, created_at_ms, long_lived)| (*long_lived, *created_at_ms));
    let keep_from = retained.len().saturating_sub(limit);
    let retained = retained
        .into_iter()
        .skip(keep_from)
        .map(|(key, _, _)| key)
        .collect::<std::collections::HashSet<_>>();
    board
        .retain_internal_refs_with_prefix(COMMAND_RECEIPT_REF_PREFIX, |key, _| {
            retained.contains(key)
        })
        .map_err(|error| ApiError::internal(format!("command receipt pruning failed: {error}")))?;
    Ok(before != retained.len())
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = ExecuteCommandsBody,
    responses(
        (status = 200, description = "Commands executed. Returns the resulting commands, or `{commands, sync}` when the request carried a sync manifest", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}",
    skip(state, user, headers, params)
)]
pub async fn execute_commands(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(mut params): Json<ExecuteCommandsBody>,
) -> Result<Json<ExecuteCommandsResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);

    let sub = permission.sub()?;
    let sync_request = params.sync.take();
    let mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;
    let idempotency_key = command_idempotency_key(&headers, &sub, &app_id, &board_id)?;
    let receipt_ack_key = command_receipt_ack_key(&headers, &sub, &app_id, &board_id)?;
    // Folding an acknowledgement into a mutation is only meaningful for a *previous* batch.
    // Acknowledging the key this very request submits under would release the receipt that guards
    // it, letting a retry apply the same commands twice.
    if let (Some(mutation), Some(acknowledgement)) =
        (idempotency_key.as_ref(), receipt_ack_key.as_ref())
        && mutation.ref_key == acknowledgement.ref_key
    {
        return Err(ApiError::bad_request(
            "FlowLike-Idempotency-Ack must reference a previous batch, not this request's Idempotency-Key.",
        ));
    }
    let digest = idempotency_key
        .as_ref()
        .map(|_| request_digest(&params.commands))
        .transpose()?;
    // The idempotent sync contract is an acknowledgement of this exact submitted batch. Keep the
    // first response byte-semantically aligned with replay instead of returning server-mutated undo
    // state only on the first attempt.
    let idempotent_response = idempotency_key.as_ref().map(|_| params.commands.clone());

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;
    let now_ms = wall_clock_ms();
    // Capture exact-key presence before global pruning. An invalid/corrupt receipt is still
    // evidence that this delivery may already have mutated the board; it must fail closed instead
    // of disappearing and allowing the same commands to execute again.
    let requested_receipt_before_prune = idempotency_key
        .as_ref()
        .and_then(|key| board.internal_ref(&key.ref_key).map(ToString::to_string));
    let pruned_receipts = prune_command_receipts(&mut board, now_ms, COMMAND_RECEIPT_MAX_ENTRIES)?;
    // A delivery may acknowledge the previous batch while submitting the next one. Releasing the
    // receipt here lets it ride along with this mutation's single board write instead of costing a
    // second request with its own permission check, mutation lock, board load and full-board PUT.
    // A request that fails before saving keeps the receipt, so the release stays fail-closed.
    let released_receipt = match receipt_ack_key.as_ref() {
        Some(key) => board.remove_internal_ref(&key.ref_key).is_some(),
        None => false,
    };

    if receipt_ack_key.is_some() && idempotency_key.is_none() {
        if !params.commands.is_empty() {
            return Err(ApiError::bad_request(
                "A receipt ACK request must contain an empty command list.",
            ));
        }
        let mut cached = None;
        if released_receipt || pruned_receipts {
            let put = board.save(None).await?;
            // Re-pin the cache to what this request just wrote. Without it the next mutation's
            // `If-None-Match` misses and pays a full board load and normalisation sweep for an
            // object this instance is already holding.
            cached = state.seed_board_cache(&app_id, &board_id, board, &put);
        }
        drop(mutation_guard);
        let sync = match &sync_request {
            Some(request) => {
                Some(board_sync_tail(&state, &app_id, &board_id, cached, request).await)
            }
            None => None,
        };
        return Ok(Json(ExecuteCommandsResponse::new(Vec::new(), sync)));
    }
    if let (Some(key), Some(digest), Some(original_receipt)) = (
        idempotency_key.as_ref(),
        digest.as_ref(),
        requested_receipt_before_prune,
    ) {
        if let Some(encoded) = board.internal_ref(&key.ref_key)
            && let Ok(receipt) = serde_json::from_str::<DurableCommandReceipt>(encoded)
            && receipt.version == 1
        {
            if pruned_receipts || released_receipt {
                board.save(None).await?;
            }
            if receipt.request_digest != *digest {
                return Err(ApiError::conflict(
                    "The Idempotency-Key was already used for a different command payload.",
                ));
            }
            drop(mutation_guard);
            let sync = match &sync_request {
                Some(request) => {
                    Some(board_sync_tail(&state, &app_id, &board_id, None, request).await)
                }
                None => None,
            };
            return Ok(Json(ExecuteCommandsResponse::new(params.commands, sync)));
        }

        // Restore the exact opaque value removed by pruning. Only an explicit receipt ACK may
        // release this key; automatically forgetting uncertain apply evidence would permit a
        // duplicate board mutation on the next retry.
        if board.internal_ref(&key.ref_key).is_none() {
            board
                .insert_internal_ref(key.ref_key.clone(), original_receipt)
                .map_err(|error| {
                    ApiError::internal(format!(
                        "invalid command receipt could not be retained fail-closed: {error}"
                    ))
                })?;
        }
        if pruned_receipts || released_receipt {
            board.save(None).await?;
        }
        return Err(ApiError::conflict(
            "The Idempotency-Key has an invalid or expired prior receipt. The command batch was not executed again; explicit receipt recovery is required.",
        ));
    }

    let flow_state = {
        if let Some(flow_state) = &board.app_state {
            flow_state.clone()
        } else {
            let flow_state = state
                .scoped_credentials(
                    &sub,
                    &app_id,
                    crate::credentials::CredentialsAccess::EditApp,
                )
                .await?
                .to_state(state.clone())
                .await?;
            Arc::new(flow_state)
        }
    };
    let wasm = app_wasm_nodes_cached(&state, &app_id).await?;
    let builtin_nodes = state.registry.as_ref().get_nodes_shared();
    if hydrate_board_wasm_metadata(&mut board, &wasm.nodes, &builtin_nodes) {
        board.mark_changed();
    }
    sanitize_wasm_command_metadata(&mut params.commands, &wasm.nodes, &builtin_nodes)?;

    // A rejected batch is a property of the payload, not a server fault: the client will resubmit
    // the identical bytes forever. Falling through the generic `Error` conversion would answer 500
    // "Internal Server Error" with the real reason stripped, leaving the client unable to tell a
    // permanently unapplicable batch from an outage.
    let batch = params.commands.len();
    let commands = board
        .execute_commands(params.commands, flow_state)
        .instrument(tracing::debug_span!("execute_commands.apply", batch))
        .await
        .map_err(|error| {
            ApiError::unprocessable(format!(
                "The command batch could not be applied to this board: {error}"
            ))
        })?;

    if let Some((key, digest)) = idempotency_key.zip(digest) {
        // Reserve the new entry before inserting so the persisted set never exceeds the cap.
        prune_command_receipts(
            &mut board,
            now_ms,
            COMMAND_RECEIPT_MAX_ENTRIES.saturating_sub(1),
        )?;
        let receipt = DurableCommandReceipt {
            version: 1,
            created_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(key.retention_ms),
            request_digest: digest,
        };
        let encoded = serde_json::to_string(&receipt).map_err(|error| {
            ApiError::internal(format!("command receipt serialization failed: {error}"))
        })?;
        // The marker is written into the same compressed board object as the mutation. A replay
        // therefore observes either neither change or both, including after process restart.
        board
            .insert_internal_ref(key.ref_key, encoded)
            .map_err(|error| {
                ApiError::internal(format!("command receipt insert failed: {error}"))
            })?;
    }

    let put = save_board_and_refresh_summary(&state, &app_id, &board)
        .instrument(tracing::debug_span!("execute_commands.save"))
        .await?;
    // The write is committed; everything below is read-only bookkeeping, so concurrent writers
    // may proceed while the snapshot and the sync tail are built.
    drop(mutation_guard);
    // Pinning the cache to this write is what makes the next request's `If-None-Match` a memory
    // hit. Building the sync snapshot on top of it only pays off when a tail is actually requested:
    // the desktop drain never sends a manifest, and a Lambda freezes at the response, so there is no
    // window in which an unrequested snapshot could be built off the critical path anyway.
    let sync = match &sync_request {
        Some(request) => {
            let cached = seed_board_revision(&state, &app_id, &board_id, board, &put)
                .instrument(tracing::debug_span!("execute_commands.snapshot"))
                .await;
            Some(
                board_sync_tail(&state, &app_id, &board_id, cached, request)
                    .instrument(tracing::debug_span!("execute_commands.sync_tail"))
                    .await,
            )
        }
        None => {
            state.seed_board_cache(&app_id, &board_id, board, &put);
            None
        }
    };

    Ok(Json(ExecuteCommandsResponse::new(
        idempotent_response.unwrap_or(commands),
        sync,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn idempotency_key_is_optional_but_rejects_malformed_values() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            command_idempotency_key(&headers, "principal", "app", "board")
                .expect("missing header is valid"),
            None
        );

        headers.insert("idempotency-key", HeaderValue::from_static("   "));
        assert!(command_idempotency_key(&headers, "principal", "app", "board").is_err());

        let oversized = "x".repeat(241);
        headers.insert(
            "idempotency-key",
            HeaderValue::from_str(&oversized).expect("valid header characters"),
        );
        assert!(command_idempotency_key(&headers, "principal", "app", "board").is_err());
    }

    #[test]
    fn idempotency_receipt_identity_is_stable_and_principal_scoped() {
        let mut headers = HeaderMap::new();
        headers.insert("idempotency-key", HeaderValue::from_static("delivery:7"));

        let first = command_idempotency_key(&headers, "principal-a", "app-a", "board-a")
            .expect("valid header")
            .expect("receipt key");
        let replay = command_idempotency_key(&headers, "principal-a", "app-a", "board-a")
            .expect("valid header")
            .expect("receipt key");
        let other_principal = command_idempotency_key(&headers, "principal-b", "app-a", "board-a")
            .expect("valid header")
            .expect("receipt key");
        let other_board = command_idempotency_key(&headers, "principal-a", "app-a", "board-b")
            .expect("valid header")
            .expect("receipt key");

        assert_eq!(first, replay);
        assert_ne!(first, other_principal);
        assert_ne!(first, other_board);
        assert!(first.ref_key.starts_with(COMMAND_RECEIPT_REF_PREFIX));
    }

    #[test]
    fn flowpilot_receipts_outlive_the_generic_retry_window() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "idempotency-key",
            HeaderValue::from_static("flowpilot-board-edit:job-1:0"),
        );
        let flowpilot = command_idempotency_key(&headers, "principal", "app", "board")
            .expect("valid header")
            .expect("receipt key");
        headers.insert(
            "idempotency-key",
            HeaderValue::from_static("offline-sync:job-1:0"),
        );
        let ordinary = command_idempotency_key(&headers, "principal", "app", "board")
            .expect("valid header")
            .expect("receipt key");

        assert_eq!(ordinary.retention_ms, COMMAND_RECEIPT_TTL_MS);
        assert_eq!(flowpilot.retention_ms, FLOWPILOT_COMMAND_RECEIPT_TTL_MS);
    }

    #[test]
    fn receipt_ack_uses_the_same_principal_scoped_identity() {
        let mut headers = HeaderMap::new();
        headers.insert("idempotency-key", HeaderValue::from_static("delivery:7"));
        let mutation = command_idempotency_key(&headers, "principal", "app", "board")
            .expect("valid mutation header")
            .expect("mutation key");
        headers.remove("idempotency-key");
        headers.insert(
            "flowlike-idempotency-ack",
            HeaderValue::from_static("delivery:7"),
        );
        let acknowledgement = command_receipt_ack_key(&headers, "principal", "app", "board")
            .expect("valid ACK header")
            .expect("ACK key");

        assert_eq!(mutation.ref_key, acknowledgement.ref_key);
    }

    #[test]
    fn receipt_pruning_rejects_unknown_versions_and_honors_limit() {
        let mut board = flow_like::flow::board::Board::new_detached(
            Some("board".to_string()),
            flow_like::flow_like_storage::Path::from("/test"),
        );
        let now_ms = 10_000;
        for (suffix, version, created_at_ms) in [
            ("old", 1, 1),
            ("new-a", 1, 9_000),
            ("new-b", 1, 9_500),
            ("future", 2, 9_900),
        ] {
            let encoded = serde_json::to_string(&DurableCommandReceipt {
                version,
                created_at_ms,
                expires_at_ms: 0,
                request_digest: suffix.to_string(),
            })
            .expect("receipt");
            board
                .insert_internal_ref(format!("{COMMAND_RECEIPT_REF_PREFIX}{suffix}"), encoded)
                .expect("internal receipt");
        }

        assert!(prune_command_receipts(&mut board, now_ms, 2).expect("prune"));
        let retained = board
            .internal_refs_with_prefix(COMMAND_RECEIPT_REF_PREFIX)
            .map(|(key, _)| key.to_string())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(retained.len(), 2);
        assert!(retained.contains(&format!("{COMMAND_RECEIPT_REF_PREFIX}new-a")));
        assert!(retained.contains(&format!("{COMMAND_RECEIPT_REF_PREFIX}new-b")));
    }

    #[test]
    fn ordinary_receipts_cannot_evict_a_pending_flowpilot_delivery() {
        let mut board = flow_like::flow::board::Board::new_detached(
            Some("board".to_string()),
            flow_like::flow_like_storage::Path::from("/test"),
        );
        let now_ms = 20_000;
        for (suffix, created_at_ms, expires_at_ms) in [
            ("flowpilot", 1_000, 1_000 + FLOWPILOT_COMMAND_RECEIPT_TTL_MS),
            ("ordinary", 19_000, 19_000 + COMMAND_RECEIPT_TTL_MS),
        ] {
            let encoded = serde_json::to_string(&DurableCommandReceipt {
                version: 1,
                created_at_ms,
                expires_at_ms,
                request_digest: suffix.to_string(),
            })
            .expect("receipt");
            board
                .insert_internal_ref(format!("{COMMAND_RECEIPT_REF_PREFIX}{suffix}"), encoded)
                .expect("internal receipt");
        }

        assert!(prune_command_receipts(&mut board, now_ms, 1).expect("prune"));
        assert!(
            board
                .internal_ref(&format!("{COMMAND_RECEIPT_REF_PREFIX}flowpilot"))
                .is_some()
        );
        assert!(
            board
                .internal_ref(&format!("{COMMAND_RECEIPT_REF_PREFIX}ordinary"))
                .is_none()
        );
    }
}
