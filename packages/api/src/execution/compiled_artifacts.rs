//! Pre-dispatch compiled-artifact assurance.
//!
//! Executors are read-only on the meta store (AWS scopes them to GetObject),
//! so the API guarantees the compiled artifact exists before handing a run to
//! them. The check is designed to be near-free on the hot path:
//!
//! 1. In-memory positive cache keyed by (app, board, version|etag, registry
//!    fingerprint) — entries are content-addressed and never go stale.
//! 2. One ranged GET of the 40-byte artifact envelope — existence *and*
//!    fingerprint validity in a single round trip (on S3 Express One Zone a
//!    GET costs $0.03/M at single-digit-millisecond latency; see
//!    todo/compiled-board-execution.md for the S3-vs-DynamoDB numbers).
//!    Floating drafts need one extra HEAD on the source `.board` for its etag.
//! 3. On miss: compile from the API's prepared-board cache and persist.
//!
//! Best-effort by design — the executor compiles in-memory when no artifact
//! is available, so a failure here costs warmth, never correctness.

use crate::state::AppState;
use flow_like::flow::board::Board;
use flow_like::flow::compiled;
use flow_like_storage::Path;
use flow_like_types::anyhow;

/// Ensure the compiled artifact for (board, version|latest) exists on the
/// meta store and matches the API's registry fingerprint.
pub async fn ensure_compiled_artifact(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    version: Option<(u32, u32, u32)>,
) -> flow_like_types::Result<()> {
    let fingerprint = state.registry.fingerprint();
    let fingerprint_hex = blake3::Hash::from_bytes(fingerprint).to_hex();
    let storage_root = Path::from("apps").child(app_id.to_string());
    let meta_store = state.meta_bucket.as_generic();

    // Lookup 1 (drafts only): the current source etag keys the artifact.
    let (artifact_path, purge_prefix, cache_key) = match version {
        Some(v) => (
            compiled::artifact_path(&storage_root, board_id, v),
            None,
            format!(
                "{app_id}:{board_id}:{}_{}_{}:{}",
                v.0,
                v.1,
                v.2,
                &fingerprint_hex.as_str()[..16]
            ),
        ),
        None => {
            let proto_path = Board::proto_path(&storage_root, board_id, None);
            let head = meta_store
                .head(&proto_path)
                .await
                .map_err(|e| anyhow!("board {board_id} not found for artifact check: {e}"))?;
            let e_tag = head
                .e_tag
                .ok_or_else(|| anyhow!("meta store returned no etag for board {board_id}"))?;
            (
                compiled::draft_artifact_path(app_id, board_id, &e_tag),
                Some(compiled::draft_artifact_dir(app_id, board_id)),
                format!(
                    "{app_id}:{board_id}:{e_tag}:{}",
                    &fingerprint_hex.as_str()[..16]
                ),
            )
        }
    };

    if state.compiled_artifact_cache.get(&cache_key).is_some() {
        return Ok(());
    }

    // Lookup 2: a ranged GET of the envelope answers existence + format
    // version + fingerprint in one round trip.
    if let Ok(header_bytes) = meta_store.get_range(&artifact_path, 0..40).await
        && let Ok(header) = compiled::peek_header(&header_bytes)
        && header.registry_fingerprint == fingerprint
    {
        state.compiled_artifact_cache.insert(cache_key, ());
        return Ok(());
    }

    // Miss: compile from the API's prepared-board cache and persist.
    let cached = state
        .master_board_shared(app_id, board_id, state, version)
        .await?;

    // Boards with WASM nodes are compiled by the executor: only its registry
    // carries the bundle's `on_update` behavior, so an API-built artifact
    // would be rejected there anyway. Cache the decision — it is keyed by
    // board content + fingerprint and flips on the next edit.
    let has_wasm = cached.board.nodes.values().any(|n| n.wasm.is_some())
        || cached
            .board
            .layers
            .values()
            .any(|l| l.nodes.values().any(|n| n.wasm.is_some()));
    if has_wasm {
        state.compiled_artifact_cache.insert(cache_key, ());
        return Ok(());
    }

    let compiled_board =
        compiled::compile::compile_board_with_catalog(&cached.board, &state.registry)?;
    let bytes = compiled::encode_artifact(&compiled_board, &fingerprint)?;
    compiled::persist_artifact(&meta_store, &artifact_path, purge_prefix, bytes).await?;
    state.compiled_artifact_cache.insert(cache_key, ());
    tracing::debug!(
        app_id = %app_id,
        board_id = %board_id,
        path = %artifact_path,
        "Compiled artifact ensured before dispatch"
    );
    Ok(())
}
