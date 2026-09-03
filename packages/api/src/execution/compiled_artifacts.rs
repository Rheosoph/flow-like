//! Pre-dispatch compiled-artifact assurance.
//!
//! Executors are read-only on the meta store (AWS scopes them to GetObject),
//! so the API guarantees the compiled artifact exists before handing a run to
//! them. The check is designed to be near-free on the hot path:
//!
//! 1. In-memory positive cache keyed by (app, board, version|etag, registry
//!    fingerprint). ETag-bound Page runs still revalidate object existence
//!    because bucket lifecycle cleanup can remove an otherwise valid artifact.
//! 2. One ranged GET of the 40-byte artifact envelope — existence *and*
//!    fingerprint validity in a single round trip (on S3 Express One Zone a
//!    GET costs $0.03/M at single-digit-millisecond latency; see
//!    todo/compiled-board-execution.md for the S3-vs-DynamoDB numbers).
//!    Ordinary floating runs need one extra HEAD on the source `.board` for
//!    its ETag. Governed Page runs already carry the ETag they authorized.
//! 3. On miss: compile from the API's prepared-board cache and persist.
//!
//! Ordinary runs treat this as a warm-up optimization because the executor can
//! compile the current source in memory. An ETag-bound Page run fails dispatch
//! if its exact artifact cannot be assured; substituting a newer source would
//! violate the action contract.

use crate::state::AppState;
use flow_like::flow::board::Board;
use flow_like::flow::compiled;
use flow_like_storage::{Path, object_store::Error as StoreError};
use flow_like_types::anyhow;
use flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL;

fn require_selected_draft_etag(
    board_id: &str,
    selected_etag: Option<&str>,
    loaded_etag: &str,
) -> flow_like_types::Result<()> {
    if selected_etag.is_some_and(|selected| selected != loaded_etag) {
        return Err(anyhow!(
            "Latest board {board_id} changed before its ETag-bound compiled artifact was created"
        ));
    }
    Ok(())
}

/// Ensure the compiled artifact for (board, version|latest) exists on the
/// meta store and matches the API's registry fingerprint.
pub async fn ensure_compiled_artifact(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    version: Option<(u32, u32, u32)>,
    expected_etag: Option<&str>,
) -> flow_like_types::Result<()> {
    let fingerprint = state.registry.fingerprint();
    let fingerprint_hex = blake3::Hash::from_bytes(fingerprint).to_hex();
    let storage_root = Path::from("apps").child(app_id.to_string());
    let meta_store = state.meta_bucket.as_generic();

    if expected_etag.is_some() {
        let sentinel_path = Board::proto_path(
            &storage_root,
            board_id,
            Some(ETAG_BOUND_LATEST_VERSION_SENTINEL),
        );
        match meta_store.head(&sentinel_path).await {
            Err(StoreError::NotFound { .. }) => {}
            Ok(_) => {
                return Err(anyhow!(
                    "board {board_id} already has a snapshot at the version reserved for ETag-bound Latest dispatch"
                ));
            }
            Err(error) => {
                return Err(anyhow!(
                    "failed to verify the reserved-version guard for board {board_id}: {error}"
                ));
            }
        }
    }

    // Lookup 1 (drafts only): the current source etag keys the artifact.
    let (artifact_path, purge_prefix, cache_key, selected_draft_etag) = match version {
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
            None,
        ),
        None if expected_etag.is_some() => {
            let e_tag = expected_etag.expect("guarded by match");
            if e_tag.trim().is_empty() {
                return Err(anyhow!(
                    "Latest board {board_id} has an empty expected etag"
                ));
            }
            (
                compiled::draft_artifact_path(app_id, board_id, e_tag, &fingerprint),
                // Exact Page runs may overlap a later save. Keep their
                // content-addressed artifact until object-store lifecycle
                // cleanup instead of purging it under an in-flight executor.
                None,
                format!(
                    "{app_id}:{board_id}:{e_tag}:{}",
                    &fingerprint_hex.as_str()[..16]
                ),
                Some(e_tag.to_string()),
            )
        }
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
                compiled::draft_artifact_path(app_id, board_id, &e_tag, &fingerprint),
                // Draft artifacts are content-addressed. A queued Page run may
                // still need an older ETag after a later save, so lifecycle
                // cleanup owns reclamation instead of purge-on-write.
                None,
                format!(
                    "{app_id}:{board_id}:{e_tag}:{}",
                    &fingerprint_hex.as_str()[..16]
                ),
                Some(e_tag),
            )
        }
    };

    if expected_etag.is_none() && state.compiled_artifact_cache.get(&cache_key).is_some() {
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
    let board = if let Some(expected_etag) = expected_etag {
        let source_path = compiled::draft_source_path(app_id, board_id, expected_etag);
        let proto = flow_like::utils::compression::from_compressed(
            meta_store.clone(),
            source_path.clone(),
        )
        .await
        .map_err(|error| {
            anyhow!(
                "exact source snapshot for Latest board {board_id} is unavailable at {source_path}: {error}"
            )
        })?;
        let app_state = state.master_state(state).await?;
        let board = Board::from_loaded_proto(proto, storage_root.clone(), app_state).await;
        if board.id != board_id {
            return Err(anyhow!(
                "exact source snapshot for Latest board {board_id} contains board {}",
                board.id
            ));
        }
        std::sync::Arc::new(board)
    } else {
        let cached = state
            .master_board_shared(app_id, board_id, state, version)
            .await?;
        require_selected_draft_etag(board_id, selected_draft_etag.as_deref(), &cached.e_tag)?;
        cached.board.clone()
    };

    // Boards with WASM nodes are compiled by the executor: only its registry
    // carries the bundle's `on_update` behavior, so an API-built artifact
    // would be rejected there anyway. Cache the decision — it is keyed by
    // board content + fingerprint and flips on the next edit.
    let has_wasm = board.has_wasm_nodes();
    if has_wasm {
        state.compiled_artifact_cache.insert(cache_key, ());
        return Ok(());
    }

    let compiled_board = compiled::compile::compile_board_with_catalog(&board, &state.registry)?;
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

#[cfg(test)]
mod tests {
    use super::require_selected_draft_etag;

    #[test]
    fn a_draft_load_may_not_be_written_under_an_older_head_etag() {
        require_selected_draft_etag("board-1", Some("etag-a"), "etag-a")
            .expect("an unchanged conditional selection is valid");
        assert!(
            require_selected_draft_etag("board-1", Some("etag-a"), "etag-b").is_err(),
            "a concurrent save must stop before bytes for B can be written at A's path"
        );
        require_selected_draft_etag("board-1", None, "etag-b")
            .expect("pinned versions do not use a draft ETag selector");
    }
}
