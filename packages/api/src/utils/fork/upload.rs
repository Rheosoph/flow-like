//! Offline → online uploads: the server mints every id of the uploaded app.
//!
//! The desktop lists the ids its local app holds when it begins the upload.
//! While the upload job waits in its `upload` step, the app-edit endpoints
//! re-key whatever the desktop pushes into the job's id space (the job id is
//! the seed), so no id a client sends is stored as-is, and references between
//! boards, nodes, pages, events and widgets keep resolving because every
//! request derives the same ids.

use super::ids::derive_id;
use super::job::{self, ForkJobKind, ForkJobSpec, ForkJobStep};
use super::transfer::{
    MAX_TRANSFER_BATCH, is_uploadable_content_path, rekeyed_content_path, relative_key,
    upload_staging_prefix,
};
use crate::{entity::fork_job, error::ApiError, state::AppState};
use flow_like::app::remap::ForkIdMap;
use flow_like_storage::{
    Path as StorePath, join_object_path,
    object_store::{ObjectStore, ObjectStoreExt},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use utoipa::ToSchema;

const ADOPT_CONCURRENCY: usize = 16;
const MAX_UPLOAD_IDS: usize = 2_000_000;
const MAX_UPLOAD_ID_LEN: usize = 256;
const MAX_FORK_ID_SEEDS: usize = 32;
const MAX_FORK_ID_SEED_LEN: usize = 64;

/// Every id of the local app. The shared remap only rewrites a value it
/// finds in its map, so the server has to know all of them before the first
/// board arrives, including nodes, pins and layers other boards point at.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct UploadIdInventory {
    /// The local app's own id, which pages and widgets may name.
    pub app_id: String,
    #[serde(default)]
    pub boards: Vec<String>,
    #[serde(default)]
    pub nodes: Vec<String>,
    #[serde(default)]
    pub pins: Vec<String>,
    #[serde(default)]
    pub layers: Vec<String>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub pages: Vec<String>,
    #[serde(default)]
    pub widgets: Vec<String>,
    #[serde(default)]
    pub templates: Vec<String>,
}

impl UploadIdInventory {
    fn lists(&self) -> [(&str, &[String]); 8] {
        [
            ("boards", &self.boards),
            ("nodes", &self.nodes),
            ("pins", &self.pins),
            ("layers", &self.layers),
            ("events", &self.events),
            ("pages", &self.pages),
            ("widgets", &self.widgets),
            ("templates", &self.templates),
        ]
    }

    pub fn validate(&self) -> Result<(), ApiError> {
        if !is_listed_id(&self.app_id) {
            return Err(ApiError::bad_request(format!(
                "id_inventory.app_id must be 1-{MAX_UPLOAD_ID_LEN} bytes"
            )));
        }
        let total: usize = self.lists().iter().map(|(_, ids)| ids.len()).sum();
        if total > MAX_UPLOAD_IDS {
            return Err(ApiError::bad_request(format!(
                "id_inventory lists {total} ids, at most {MAX_UPLOAD_IDS} are allowed"
            )));
        }
        for (kind, ids) in self.lists() {
            if let Some(bad) = ids.iter().find(|id| !is_listed_id(id)) {
                return Err(ApiError::bad_request(format!(
                    "id_inventory.{kind} has an entry of {} bytes, ids must be 1-{MAX_UPLOAD_ID_LEN} bytes",
                    bad.len()
                )));
            }
        }
        Ok(())
    }

    /// The upload's id space: every listed id mapped to `derive_id(seed, id)`.
    pub fn id_map(&self, seed: &str, dest_app_id: &str) -> ForkIdMap {
        let derive = |ids: &[String]| -> HashMap<String, String> {
            ids.iter()
                .map(|id| (id.clone(), derive_id(seed, id)))
                .collect()
        };
        ForkIdMap {
            source_app_id: self.app_id.clone(),
            app_id: dest_app_id.to_string(),
            boards: derive(&self.boards),
            nodes: derive(&self.nodes),
            pins: derive(&self.pins),
            events: derive(&self.events),
            pages: derive(&self.pages),
            layers: derive(&self.layers),
            widgets: derive(&self.widgets),
            templates: derive(&self.templates),
            seed: seed.to_string(),
            ..Default::default()
        }
    }
}

fn is_listed_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_UPLOAD_ID_LEN
}

/// The local app's own `fork_id_seeds` as the desktop reports them: empty
/// for an app that is no fork, absent when the local fork never recorded them.
pub fn validate_fork_id_seeds(seeds: &[String]) -> Result<(), ApiError> {
    if seeds.len() > MAX_FORK_ID_SEEDS {
        return Err(ApiError::bad_request(format!(
            "fork_id_seeds has {} entries, at most {MAX_FORK_ID_SEEDS} are allowed",
            seeds.len()
        )));
    }
    if let Some(bad) = seeds.iter().find(|seed| {
        seed.is_empty()
            || seed.len() > MAX_FORK_ID_SEED_LEN
            || !seed
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    }) {
        return Err(ApiError::bad_request(format!(
            "fork_id_seeds entry {bad:?} is not an id (1-{MAX_FORK_ID_SEED_LEN} letters, digits, '_' or '-')"
        )));
    }
    Ok(())
}

/// The uploaded app's seed chain: the local chain followed by the upload's
/// own seed. Unknown while the local chain is unknown.
pub fn uploaded_fork_id_seeds(local: Option<Vec<String>>, seed: &str) -> Option<Vec<String>> {
    local.map(|mut chain| {
        chain.push(seed.to_string());
        chain
    })
}

/// The upload job still waiting for the desktop to push into `app_id`, with
/// its spec. `None` for every other app.
async fn waiting_upload(
    state: &AppState,
    app_id: &str,
) -> Result<Option<(fork_job::Model, ForkJobSpec)>, ApiError> {
    let Some(job) = job::find_live_by_dest(state, app_id).await? else {
        return Ok(None);
    };
    if ForkJobStep::parse(&job.step) != Some(ForkJobStep::Upload) {
        return Ok(None);
    }
    let spec = ForkJobSpec::parse(&job)?;
    if spec.kind != ForkJobKind::OfflineUpload {
        return Ok(None);
    }
    Ok(Some((job, spec)))
}

/// The id space of an upload still waiting for the desktop to push into
/// `app_id`. `None` for every other app, and for uploads begun by a desktop
/// that sent no inventory.
pub async fn upload_id_map(state: &AppState, app_id: &str) -> Result<Option<ForkIdMap>, ApiError> {
    Ok(waiting_upload(state, app_id)
        .await?
        .and_then(|(job, spec)| spec.upload_ids.map(|ids| ids.id_map(&job.id, app_id))))
}

/// The upload into `app_id` that `sub` began and has not finalized. Only its
/// uploader may stage files for it.
pub async fn own_waiting_upload(
    state: &AppState,
    app_id: &str,
    sub: &str,
) -> Result<fork_job::Model, ApiError> {
    let Some((job, _)) = waiting_upload(state, app_id).await? else {
        return Err(ApiError::conflict(format!(
            "app {app_id} has no upload waiting for files; begin one with POST /apps/fork/online/begin"
        )));
    };
    if job.user_id != sub {
        return Err(ApiError::FORBIDDEN);
    }
    Ok(job)
}

/// Move up to one page of the upload's staged files into the app and drop
/// each staged copy. Widget, template and page metadata moves to the ids the
/// upload minted. Returns how many files moved and whether any are left.
pub async fn adopt_staged_page(
    state: &AppState,
    app_id: &str,
    job: &fork_job::Model,
) -> Result<(u64, bool), ApiError> {
    use futures::{StreamExt, TryStreamExt};

    let store = state
        .master_credentials()
        .await?
        .to_store(false)
        .await?
        .as_generic();
    let target = AdoptTarget {
        staging: upload_staging_prefix(&job.user_id, app_id, &job.id),
        app_root: StorePath::from("apps").join(app_id),
        app_id,
        seed: ForkJobSpec::parse(job)?
            .upload_ids
            .is_some()
            .then_some(job.id.as_str()),
    };
    let (page, remaining) = first_staged_page(&store, &target).await?;
    let moved = futures::stream::iter(page)
        .map(|staged| adopt_one(&store, &target, staged))
        .buffer_unordered(ADOPT_CONCURRENCY)
        .try_fold(0u64, |total, moved| async move { Ok(total + moved) })
        .await?;
    Ok((moved, remaining))
}

/// Where an upload's staged files go: the app root, re-keyed with `seed`
/// when the upload was re-keyed.
struct AdoptTarget<'a> {
    staging: StorePath,
    app_root: StorePath,
    app_id: &'a str,
    seed: Option<&'a str>,
}

/// Up to one page of staged keys, and whether more wait behind it.
async fn first_staged_page(
    store: &Arc<dyn ObjectStore>,
    target: &AdoptTarget<'_>,
) -> Result<(Vec<StorePath>, bool), ApiError> {
    use futures::TryStreamExt;

    let mut listing = store.list(Some(&target.staging));
    let mut page = Vec::with_capacity(MAX_TRANSFER_BATCH);
    while let Some(object) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal(format!("list staged upload of {}: {e}", target.app_id)))?
    {
        if page.len() == MAX_TRANSFER_BATCH {
            return Ok((page, true));
        }
        page.push(object.location);
    }
    Ok((page, false))
}

/// Copy one staged file into the app (unless it lies outside the content
/// roots) and drop the staged copy. Returns 1 for a handled file.
async fn adopt_one(
    store: &Arc<dyn ObjectStore>,
    target: &AdoptTarget<'_>,
    staged: StorePath,
) -> Result<u64, ApiError> {
    let Some(relative) = relative_key(&staged, &target.staging) else {
        return Ok(0);
    };
    if is_uploadable_content_path(&relative) {
        let relative = match target.seed {
            Some(seed) => rekeyed_content_path(&relative, seed),
            None => relative,
        };
        store
            .copy(&staged, &join_object_path(&target.app_root, &relative))
            .await
            .map_err(|e| {
                ApiError::internal(format!(
                    "move staged upload file {relative} into app {}: {e}",
                    target.app_id
                ))
            })?;
    } else {
        tracing::warn!(app_id = %target.app_id, path = %relative, "dropped a staged upload file outside the content roots");
    }
    store
        .delete(&staged)
        .await
        .map_err(|e| ApiError::internal(format!("drop staged upload file {staged}: {e}")))?;
    Ok(1)
}

/// Move every staged file of the upload into the app.
pub async fn adopt_all_staged(
    state: &AppState,
    app_id: &str,
    job: &fork_job::Model,
) -> Result<u64, ApiError> {
    let mut total = 0;
    loop {
        let (moved, remaining) = adopt_staged_page(state, app_id, job).await?;
        total += moved;
        if !remaining {
            return Ok(total);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::app::remap::{derive_lineage_id, remap_board, remap_event, remap_page};
    use flow_like_types::proto;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn inventory() -> UploadIdInventory {
        UploadIdInventory {
            app_id: "local_app".into(),
            boards: ids(&["board_a", "board_b"]),
            nodes: ids(&["node_a", "node_b"]),
            pins: ids(&["pin_a"]),
            events: ids(&["event_a"]),
            pages: ids(&["page_a"]),
            ..Default::default()
        }
    }

    fn board(id: &str, node: &str, pins: &[(&str, &[u8])]) -> proto::Board {
        proto::Board {
            id: id.into(),
            nodes: [(
                node.to_string(),
                proto::Node {
                    id: node.into(),
                    name: "events_simple".into(),
                    pins: pins
                        .iter()
                        .map(|(pin, default)| {
                            (
                                pin.to_string(),
                                proto::Pin {
                                    id: pin.to_string(),
                                    name: pin.to_string(),
                                    default_value: default.to_vec(),
                                    ..Default::default()
                                },
                            )
                        })
                        .collect(),
                    ..Default::default()
                },
            )]
            .into(),
            page_ids: ids(&["page_a"]),
            ..Default::default()
        }
    }

    /// Each request re-derives the map, so ids only agree across requests
    /// because every one of them comes from the seed.
    #[test]
    fn separately_remapped_artifacts_still_point_at_each_other() {
        let inventory = inventory();
        let first_request = inventory.id_map("job", "dest");
        let second_request = inventory.id_map("job", "dest");

        let pointer = br#""node_b""#;
        let a = remap_board(
            board("board_a", "node_a", &[("pin_a", pointer)]),
            &mut first_request.clone(),
        )
        .unwrap();
        let b = remap_board(board("board_b", "node_b", &[]), &mut second_request.clone()).unwrap();
        let b_node = b.nodes.keys().next().unwrap().clone();
        let a_pin = a
            .nodes
            .values()
            .next()
            .unwrap()
            .pins
            .values()
            .next()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<String>(&a_pin.default_value).unwrap(),
            b_node,
            "a pin naming another board's node follows it"
        );
        assert_eq!(a.page_ids, vec![derive_id("job", "page_a")]);

        let mut event = proto::Event {
            id: "event_a".into(),
            board_id: "board_b".into(),
            node_id: "node_b".into(),
            ..Default::default()
        };
        remap_event(&mut event, &second_request);
        assert_eq!(event.board_id, b.id);
        assert_eq!(event.node_id, b_node);

        let mut page = proto::Page {
            id: "page_a".into(),
            board_id: Some("board_a".into()),
            on_load_event_id: Some("node_a".into()),
            ..Default::default()
        };
        remap_page(&mut page, &first_request.mint("page_a"), &first_request);
        assert_eq!(page.board_id.as_deref(), Some(a.id.as_str()));
        assert_eq!(
            page.on_load_event_id.as_deref(),
            a.nodes.keys().next().map(String::as_str)
        );
    }

    #[test]
    fn upload_ids_follow_the_lineage_chain() {
        let map = inventory().id_map("job", "dest");
        let seeds = uploaded_fork_id_seeds(Some(vec!["fork".into()]), "job").unwrap();
        assert_eq!(seeds, ids(&["fork", "job"]));
        assert_eq!(
            derive_lineage_id(&seeds, "board_a"),
            derive_id("job", &derive_id("fork", "board_a"))
        );
        assert_eq!(
            map.boards["board_a"],
            derive_lineage_id(&ids(&["job"]), "board_a")
        );
        assert_eq!(
            uploaded_fork_id_seeds(Some(vec![]), "job"),
            Some(ids(&["job"]))
        );
        assert_eq!(uploaded_fork_id_seeds(None, "job"), None);
    }

    #[test]
    fn inventories_and_seeds_reject_values_that_are_not_ids() {
        assert!(inventory().validate().is_ok());
        let mut empty_app = inventory();
        empty_app.app_id.clear();
        assert!(empty_app.validate().is_err());
        let mut long_node = inventory();
        long_node.nodes.push("x".repeat(MAX_UPLOAD_ID_LEN + 1));
        assert!(long_node.validate().is_err());

        assert!(validate_fork_id_seeds(&ids(&["job1", "copy-2"])).is_ok());
        for seed in ["", "a b", "a/b", &"x".repeat(MAX_FORK_ID_SEED_LEN + 1)] {
            assert!(validate_fork_id_seeds(&[seed.to_string()]).is_err());
        }
        assert!(validate_fork_id_seeds(&vec!["s".to_string(); MAX_FORK_ID_SEEDS + 1]).is_err());
    }
}
