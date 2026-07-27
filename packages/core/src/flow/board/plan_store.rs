//! Persistence for compiled execution plans.
//!
//! Plans live beside the boards they derive from and are addressed by *format version* in
//! the object name. That is what lets a rolling deploy work: a binary that only speaks
//! format 1 keeps reading `…f1.plan` while a newer one writes `…f2.plan`, and neither ever
//! sees bytes it cannot interpret. Nothing is ever migrated — a plan that does not exist,
//! or whose stamps no longer match, is simply recompiled from the `.board`.
//!
//! ```text
//! versions/{board_id}/{maj}_{min}_{pat}.f{N}.plan   immutable, pinned to a board version
//! plans/{board_id}/{content_hash}.f{N}.plan         draft / "latest", content-addressed
//! plans/{board_id}/latest.f{N}.ptr                  which content hash is current
//! ```
//!
//! Everything goes through `object_store`, so the same code path serves S3, Azure Blob,
//! GCS and the local filesystem. Plans are stored uncompressed: the hot section is mostly
//! `u32` arrays, and keeping the bytes directly castable is worth more than shaving a few
//! kilobytes off a transfer.

use std::sync::Arc;

use flow_like_storage::Path;
use flow_like_storage::object_store::{
    Error as ObjectStoreError, ObjectStore, PutMode, PutOptions, PutPayload,
};
use flow_like_types::plan::{PLAN_FORMAT_VERSION, PlanBuffer};
use serde::{Deserialize, Serialize};

/// Names the current content hash for a board's draft plan.
///
/// A run of "latest" compares this against a cheap HEAD of the `.board`: if the recorded
/// e_tag still matches, the named plan is current and can be served without downloading
/// the board at all.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LatestPlanPointer {
    pub content_hash: u64,
    /// e_tag of the `.board` this plan was compiled from, when the store reported one.
    pub board_e_tag: Option<String>,
    /// Previous content hash, retained so garbage collection can leave one generation of
    /// grace for readers that are mid-fetch.
    #[serde(default)]
    pub previous_content_hash: Option<u64>,
}

pub fn plans_dir(board_dir: &Path, board_id: &str) -> Path {
    board_dir.child("plans").child(board_id.to_string())
}

/// Immutable plan for a published board version.
pub fn version_plan_path(board_dir: &Path, board_id: &str, version: (u32, u32, u32)) -> Path {
    board_dir
        .child("versions")
        .child(board_id.to_string())
        .child(format!(
            "{}_{}_{}.f{}.plan",
            version.0, version.1, version.2, PLAN_FORMAT_VERSION
        ))
}

/// Content-addressed plan for a draft board.
pub fn draft_plan_path(board_dir: &Path, board_id: &str, content_hash: u64) -> Path {
    plans_dir(board_dir, board_id).child(format!("{content_hash}.f{PLAN_FORMAT_VERSION}.plan"))
}

pub fn pointer_path(board_dir: &Path, board_id: &str) -> Path {
    plans_dir(board_dir, board_id).child(format!("latest.f{PLAN_FORMAT_VERSION}.ptr"))
}

/// Write a plan that must never be overwritten.
///
/// Plan bytes are deterministic for a given board, catalog and format version, so two
/// racing writers produce identical objects and an `AlreadyExists` is success, not a
/// conflict.
pub async fn write_plan_immutable(
    store: Arc<dyn ObjectStore>,
    path: Path,
    bytes: Vec<u8>,
) -> flow_like_types::Result<()> {
    let result = store
        .put_opts(
            &path,
            PutPayload::from(bytes),
            PutOptions {
                mode: PutMode::Create,
                ..Default::default()
            },
        )
        .await;

    match result {
        Ok(_) => Ok(()),
        Err(ObjectStoreError::AlreadyExists { .. }) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// Fetch a plan and parse its container header.
///
/// Returns `Ok(None)` when the object simply is not there, which is the common case for a
/// board that has not been compiled yet and must not be treated as an error.
pub async fn read_plan(
    store: Arc<dyn ObjectStore>,
    path: Path,
) -> flow_like_types::Result<Option<PlanBuffer>> {
    let response = match store.get(&path).await {
        Ok(response) => response,
        Err(ObjectStoreError::NotFound { .. }) => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    let bytes = response.bytes().await?;
    Ok(Some(PlanBuffer::new(bytes.to_vec())?))
}

pub async fn read_pointer(
    store: Arc<dyn ObjectStore>,
    path: Path,
) -> flow_like_types::Result<Option<LatestPlanPointer>> {
    let response = match store.get(&path).await {
        Ok(response) => response,
        Err(ObjectStoreError::NotFound { .. }) => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    let bytes = response.bytes().await?;
    match flow_like_types::json::from_slice(&bytes) {
        Ok(pointer) => Ok(Some(pointer)),
        // A pointer we cannot read is indistinguishable from no pointer: the caller
        // recompiles and overwrites it. Never fail a run over a cache breadcrumb.
        Err(err) => {
            tracing::warn!("discarding unreadable plan pointer at {}: {}", path, err);
            Ok(None)
        }
    }
}

pub async fn write_pointer(
    store: Arc<dyn ObjectStore>,
    path: Path,
    pointer: &LatestPlanPointer,
) -> flow_like_types::Result<()> {
    let bytes = flow_like_types::json::to_vec(pointer)?;
    store.put(&path, PutPayload::from(bytes)).await?;
    Ok(())
}

/// Delete draft plans that the pointer no longer references.
///
/// Keeps the current hash and one predecessor so a reader that fetched the pointer just
/// before it moved can still complete. Runs in-process rather than relying on bucket
/// lifecycle rules, because those do not exist uniformly across S3, Azure and GCS.
pub async fn collect_garbage(
    store: Arc<dyn ObjectStore>,
    board_dir: &Path,
    board_id: &str,
    pointer: &LatestPlanPointer,
) -> flow_like_types::Result<usize> {
    use futures::StreamExt;

    let keep: Vec<String> = [Some(pointer.content_hash), pointer.previous_content_hash]
        .into_iter()
        .flatten()
        .map(|hash| format!("{hash}.f{PLAN_FORMAT_VERSION}.plan"))
        .collect();

    let dir = plans_dir(board_dir, board_id);
    let mut listing = store.list(Some(&dir));
    let mut stale = Vec::new();

    while let Some(entry) = listing.next().await {
        let entry = entry?;
        let Some(name) = entry.location.filename() else {
            continue;
        };
        if !name.ends_with(".plan") || keep.iter().any(|keeper| keeper == name) {
            continue;
        }
        stale.push(entry.location);
    }

    let removed = stale.len();
    for location in stale {
        // A concurrent collector may have removed it already; that is not a failure.
        if let Err(err) = store.delete(&location).await {
            tracing::debug!("plan gc skipped {}: {}", location, err);
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board_dir() -> Path {
        Path::from("apps/app-1")
    }

    #[test]
    fn paths_carry_the_format_version() {
        let version = version_plan_path(&board_dir(), "board-1", (1, 2, 3));
        assert_eq!(
            version.to_string(),
            format!("apps/app-1/versions/board-1/1_2_3.f{PLAN_FORMAT_VERSION}.plan")
        );

        let draft = draft_plan_path(&board_dir(), "board-1", 42);
        assert_eq!(
            draft.to_string(),
            format!("apps/app-1/plans/board-1/42.f{PLAN_FORMAT_VERSION}.plan")
        );

        let pointer = pointer_path(&board_dir(), "board-1");
        assert_eq!(
            pointer.to_string(),
            format!("apps/app-1/plans/board-1/latest.f{PLAN_FORMAT_VERSION}.ptr")
        );
    }

    #[test]
    fn pointer_roundtrips_through_json() {
        let pointer = LatestPlanPointer {
            content_hash: 7,
            board_e_tag: Some("\"abc\"".into()),
            previous_content_hash: Some(6),
        };
        let bytes = flow_like_types::json::to_vec(&pointer).unwrap();
        let decoded: LatestPlanPointer = flow_like_types::json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, pointer);
    }

    /// Pointers written before `previous_content_hash` existed must still parse.
    #[test]
    fn pointer_tolerates_missing_optional_fields() {
        let decoded: LatestPlanPointer =
            flow_like_types::json::from_slice(br#"{"content_hash":9,"board_e_tag":null}"#).unwrap();
        assert_eq!(decoded.content_hash, 9);
        assert_eq!(decoded.previous_content_hash, None);
    }

    mod storage {
        use super::*;
        use crate::flow::board::Board;
        use crate::state::{FlowLikeConfig, FlowLikeState};
        use crate::utils::http::HTTPClient;
        use flow_like_storage::files::store::FlowLikeStore;
        use flow_like_storage::object_store::{self, ObjectStore};
        use flow_like_types::plan::PLAN_FORMAT_VERSION;
        use flow_like_types::tokio;
        use std::sync::Arc;

        async fn state() -> Arc<FlowLikeState> {
            let mut config = FlowLikeConfig::new();
            config.register_app_meta_store(FlowLikeStore::Other(Arc::new(
                object_store::memory::InMemory::new(),
            )));
            Arc::new(FlowLikeState::new(
                config,
                HTTPClient::new_without_refetch(),
            ))
        }

        async fn store_of(state: &Arc<FlowLikeState>) -> Arc<dyn ObjectStore> {
            state
                .config
                .read()
                .await
                .stores
                .app_meta_store
                .clone()
                .unwrap()
                .as_generic()
        }

        #[tokio::test]
        async fn publishing_a_version_writes_a_readable_plan() {
            let state = state().await;
            let store = store_of(&state).await;
            let mut board = Board::new(
                Some("board-e2e".into()),
                Path::from("apps/app-e2e"),
                state.clone(),
            );

            let mut node = crate::flow::node::Node::new("log", "Log", "", "Test");
            node.id = "node-e2e".into();
            node.add_input_pin(
                "exec_in",
                "In",
                "",
                crate::flow::variable::VariableType::Execution,
            );
            board.nodes.insert(node.id.clone(), node);
            // Publication compares the snapshot against the persisted draft, so the draft
            // has to be saved in its hashed form exactly as the editor would leave it.
            board.hash();
            board.save(Some(store.clone())).await.unwrap();

            // The publish protocol ends with a draft-consistency check that reloads the
            // board through `node_updates`, which needs a registry able to round-trip every
            // node type; this fixture deliberately registers no catalog, so that final step
            // reports a mismatch. What is under test here is the ordering guarantee: the
            // plan is emitted before the version marker, so a visible version can never be
            // missing one.
            let version = (0, 1, 0);
            let _ = board
                .snapshot_at_version(version, Some(store.clone()))
                .await;

            let plan = read_plan(
                store.clone(),
                version_plan_path(&board.board_dir, &board.id, version),
            )
            .await
            .unwrap()
            .expect("the plan must be written before the version marker");

            assert_eq!(plan.header().format_version, PLAN_FORMAT_VERSION);
            assert_eq!(plan.header().stamps.board_version, version);

            let section = plan.hot().unwrap();
            let archived = section.root();
            assert_eq!(archived.board_id.as_str(), "board-e2e");
            assert!(archived.node_by_id("node-e2e").is_some());
        }

        #[tokio::test]
        async fn reading_an_absent_plan_is_not_an_error() {
            let state = state().await;
            let store = store_of(&state).await;
            let missing = read_plan(
                store,
                version_plan_path(&Path::from("apps/x"), "b", (1, 0, 0)),
            )
            .await
            .unwrap();
            assert!(missing.is_none());
        }

        /// Plan bytes are deterministic, so a racing writer is success rather than a
        /// conflict — otherwise two executors compiling the same board would fail one run.
        #[tokio::test]
        async fn immutable_writes_tolerate_a_racing_identical_writer() {
            let state = state().await;
            let store = store_of(&state).await;
            let path = draft_plan_path(&Path::from("apps/app-1"), "board-1", 7);

            write_plan_immutable(store.clone(), path.clone(), vec![1, 2, 3])
                .await
                .unwrap();
            write_plan_immutable(store.clone(), path.clone(), vec![1, 2, 3])
                .await
                .expect("second write must be a no-op, not a conflict");
        }

        #[tokio::test]
        async fn garbage_collection_keeps_current_and_previous_only() {
            let state = state().await;
            let store = store_of(&state).await;
            let dir = Path::from("apps/app-gc");

            for hash in [1u64, 2, 3, 4] {
                write_plan_immutable(
                    store.clone(),
                    draft_plan_path(&dir, "board-1", hash),
                    vec![hash as u8],
                )
                .await
                .unwrap();
            }

            let pointer = LatestPlanPointer {
                content_hash: 4,
                board_e_tag: None,
                previous_content_hash: Some(3),
            };
            let removed = collect_garbage(store.clone(), &dir, "board-1", &pointer)
                .await
                .unwrap();
            assert_eq!(removed, 2);

            for stale in [1u64, 2] {
                let path = draft_plan_path(&dir, "board-1", stale);
                assert!(
                    store.head(&path).await.is_err(),
                    "hash {stale} should be gone"
                );
            }
            for kept in [3u64, 4] {
                let path = draft_plan_path(&dir, "board-1", kept);
                assert!(store.head(&path).await.is_ok(), "hash {kept} must survive");
            }
        }
    }
}
