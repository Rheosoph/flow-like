//! Storage host functions
//!
//! Provides storage access for WASM modules.

use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::normalize_object_path;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::object_store::{path::Path, ObjectMeta, PutPayload};
use flow_like_types::Bytes;
use futures::stream::{self, BoxStream};
use futures::StreamExt;
use std::sync::Arc;

use super::StorageContext;

pub const MAX_STORAGE_FILE_SIZE: usize = 10 * 1024 * 1024;
pub const MAX_PENDING_WRITES: usize = 8;
#[cfg(test)]
const COPY_BUFFER_SIZE: usize = 10 * 1024 * 1024;

#[path = "storage_writer.rs"]
mod writer;
pub use writer::{
    abort_write, append_chunk, cleanup_pending_writes, finish_write, start_write, PendingWrite,
};

/// A host-resolved store and the object paths supplied to this invocation.
#[derive(Clone)]
pub struct StorageStore {
    pub store: FlowLikeStore,
    pub roots: Vec<Path>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct StorageFlowPath {
    pub path: String,
    pub store_ref: String,
    pub cache_store_ref: Option<String>,
}

/// A listed file's byte length lets guests check their budget before reading it.
#[derive(Debug, serde::Serialize)]
pub(crate) struct StorageFileEntry {
    #[serde(flatten)]
    flow_path: StorageFlowPath,
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    e_tag: Option<String>,
}

impl StorageFileEntry {
    pub(crate) fn from_metadata(directory: &StorageFlowPath, metadata: ObjectMeta) -> Self {
        Self {
            flow_path: StorageFlowPath {
                path: metadata.location.as_ref().to_string(),
                store_ref: directory.store_ref.clone(),
                cache_store_ref: directory.cache_store_ref.clone(),
            },
            size: metadata.size,
            e_tag: metadata.e_tag,
        }
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct StorageListRequest {
    #[serde(flatten)]
    pub flow_path: StorageFlowPath,
    #[serde(default)]
    pub exclude_directories: Vec<String>,
}

pub(crate) fn include_listed_file(path: &Path, prefix: &Path, excluded: &[String]) -> bool {
    let Some(parts) = path.prefix_match(prefix) else {
        return false;
    };
    let mut parts = parts.peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_some() && excluded.iter().any(|name| name == part.as_ref()) {
            return false;
        }
    }
    true
}

/// Prune excluded subtrees before enumeration so dependency folders do not
/// consume directory handles or remote listing requests.
pub(crate) fn list_files_excluding(
    store: Arc<dyn flow_like_storage::object_store::ObjectStore>,
    prefix: Path,
    excluded: Vec<String>,
) -> BoxStream<'static, flow_like_storage::object_store::Result<ObjectMeta>> {
    if excluded.is_empty() {
        return store.list(Some(&prefix));
    }

    let directories = vec![prefix.clone()];
    let objects = Vec::new().into_iter();
    stream::try_unfold(
        (store, prefix, excluded, directories, objects),
        |(store, prefix, excluded, mut directories, mut objects)| async move {
            loop {
                if let Some(metadata) = objects.next() {
                    return Ok(Some((
                        metadata,
                        (store, prefix, excluded, directories, objects),
                    )));
                }
                let Some(directory) = directories.pop() else {
                    return Ok(None);
                };
                let listing = store.list_with_delimiter(Some(&directory)).await?;
                for child in listing.common_prefixes.into_iter().rev() {
                    if child == directory || !child.prefix_matches(&directory) {
                        continue;
                    }
                    let included = child.prefix_match(&prefix).is_some_and(|mut parts| {
                        !parts.any(|part| excluded.iter().any(|name| name == part.as_ref()))
                    });
                    if included {
                        directories.push(child);
                    }
                }
                objects = listing
                    .objects
                    .into_iter()
                    .filter(|meta| include_listed_file(&meta.location, &prefix, &excluded))
                    .collect::<Vec<_>>()
                    .into_iter();
            }
        },
    )
    .boxed()
}

impl StorageFlowPath {
    /// Canonical object key. Guests may send a raw name or a key that came out
    /// of a list response; both resolve to the same object.
    pub fn object_path(&self) -> Path {
        normalize_object_path(&self.path)
    }
}

pub fn validate_path(path: &str) -> bool {
    !path.contains("..") && !path.starts_with('/') && !path.is_empty()
}

pub async fn put_flow_path(
    storage_ctx: &StorageContext,
    flow_path: &StorageFlowPath,
    data: Vec<u8>,
    log_prefix: &str,
) -> bool {
    let path = flow_path.object_path();
    let Some(store) = storage_ctx.resolve_store(&flow_path.store_ref, &path) else {
        tracing::warn!(
            "[{log_prefix}] rejected: unresolved or out-of-scope store_ref={}",
            flow_path.store_ref
        );
        return false;
    };

    let cache_store = match flow_path.cache_store_ref.as_deref() {
        Some(cache_store_ref) if cache_store_ref != flow_path.store_ref => {
            let Some(cache_store) = storage_ctx.resolve_store(cache_store_ref, &path) else {
                tracing::warn!(
                    "[{log_prefix}] rejected: unresolved or out-of-scope cache_store_ref={cache_store_ref}"
                );
                return false;
            };
            Some(cache_store)
        }
        _ => None,
    };
    let payload = PutPayload::from_bytes(Bytes::from(data));

    if let Err(e) = store.as_generic().put(&path, payload.clone()).await {
        tracing::warn!(
            "[{log_prefix}] put failed for path={} store_ref={}: {e}",
            flow_path.path,
            flow_path.store_ref
        );
        return false;
    }

    if let Some(cache_store) = cache_store {
        if let Err(e) = cache_store.as_generic().put(&path, payload).await {
            tracing::warn!(
                "[{log_prefix}] cache put failed for path={} cache_store_ref={:?}: {e}",
                flow_path.path,
                flow_path.cache_store_ref
            );
        }
    }

    true
}

async fn copy_between_stores(
    source: &FlowLikeStore,
    source_path: &Path,
    destination: &FlowLikeStore,
    destination_path: &Path,
) -> super::storage_ops::StorageResult<()> {
    super::storage_ops::copy_between_stores(
        &source.as_generic(),
        source_path,
        &destination.as_generic(),
        destination_path,
        true,
    )
    .await
}

pub(crate) async fn copy_flow_path(
    context: &StorageContext,
    source: &StorageFlowPath,
    destination: &StorageFlowPath,
) -> bool {
    let source_path = source.object_path();
    let destination_path = destination.object_path();
    let Some(source_store) = context.resolve_store(&source.store_ref, &source_path) else {
        return false;
    };
    let Some(destination_store) = context.resolve_store(&destination.store_ref, &destination_path)
    else {
        return false;
    };
    // Resolve the cache grant before changing either destination.
    let cache_store = match destination.cache_store_ref.as_deref() {
        Some(reference) if reference != destination.store_ref => {
            let Some(store) = context.resolve_store(reference, &destination_path) else {
                return false;
            };
            Some(store)
        }
        _ => None,
    };
    if let Err(error) = copy_between_stores(
        &source_store,
        &source_path,
        &destination_store,
        &destination_path,
    )
    .await
    {
        tracing::warn!("[wasm copy-file] copy failed: {error}");
        return false;
    }
    if let Some(cache_store) = cache_store {
        if let Err(error) = copy_between_stores(
            &destination_store,
            &destination_path,
            &cache_store,
            &destination_path,
        )
        .await
        {
            tracing::warn!("[wasm copy-file] cache copy failed: {error}");
        }
    }
    true
}

pub(crate) async fn flow_path_url(
    context: &StorageContext,
    flow_path: &StorageFlowPath,
) -> Option<String> {
    let path = flow_path.object_path();
    let store = context.resolve_store(&flow_path.store_ref, &path)?;
    // An asset URL would reopen an ambient path after authorization. Local
    // resources must be read through the descriptor-backed storage bridge.
    if matches!(store, FlowLikeStore::Local(_)) {
        return None;
    }
    match store
        .sign_reference("GET", &path, std::time::Duration::from_secs(3600))
        .await
    {
        Ok(url) => url.map(|url| url.to_string()),
        Err(error) => {
            tracing::warn!("[wasm file-url] signing failed: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::state::FlowLikeStores;
    use flow_like_storage::display_file_name;
    use flow_like_storage::object_store::memory::InMemory;
    use parking_lot::RwLock;
    use std::collections::HashMap;
    use std::sync::Arc;

    const NAME: &str = "Übersicht (2)#1.pdf";

    fn flow_path(path: impl Into<String>) -> StorageFlowPath {
        StorageFlowPath {
            path: path.into(),
            store_ref: "store".into(),
            cache_store_ref: None,
        }
    }

    pub(super) fn storage_context() -> (StorageContext, FlowLikeStore, FlowLikeStore) {
        let primary = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let backing = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let context = StorageContext {
            stores: FlowLikeStores {
                app_storage_store: Some(backing.clone()),
                temporary_store: Some(backing.clone()),
                user_store: Some(backing.clone()),
                ..Default::default()
            },
            store_cache: RwLock::new(HashMap::new()),
            credentials_store: Some(primary.clone()),
            app_id: "a".into(),
            board_dir: Path::from("apps/a"),
            board_id: "board".into(),
            node_id: "node".into(),
            sub: "user".into(),
        };
        (context, primary, backing)
    }

    pub(super) async fn stored_bytes(store: &FlowLikeStore, path: &Path) -> Bytes {
        store
            .as_generic()
            .get(path)
            .await
            .expect("stored object")
            .bytes()
            .await
            .expect("stored bytes")
    }

    #[test]
    fn raw_and_listed_paths_resolve_to_the_same_key() {
        let expected = Path::from("apps/a/upload").join(NAME);
        let raw = flow_path(format!("apps/a/upload/{NAME}"));
        let listed = flow_path(expected.as_ref());

        assert_eq!(raw.object_path(), expected);
        assert_eq!(listed.object_path(), expected);
        assert_eq!(
            display_file_name(&listed.object_path()).as_deref(),
            Some(NAME)
        );
    }

    #[test]
    fn listed_files_include_sizes_and_remain_flowpaths() {
        let directory = StorageFlowPath {
            path: "apps/a/upload".into(),
            store_ref: "primary".into(),
            cache_store_ref: Some("cache".into()),
        };
        let location = directory.object_path().join(NAME);
        for (size, e_tag) in [(0, None), (u64::from(u32::MAX) + 1, Some("version-2"))] {
            let entry = StorageFileEntry::from_metadata(
                &directory,
                ObjectMeta {
                    location: location.clone(),
                    last_modified: Default::default(),
                    size,
                    e_tag: e_tag.map(str::to_string),
                    version: None,
                },
            );
            let value = serde_json::to_value(entry).expect("listed file JSON");
            let mut expected = serde_json::json!({
                "path": location.as_ref(),
                "store_ref": "primary",
                "cache_store_ref": "cache",
                "size": size,
            });
            if let Some(e_tag) = e_tag {
                expected["e_tag"] = serde_json::json!(e_tag);
            }
            assert_eq!(value, expected);
            let decoded: StorageFlowPath =
                serde_json::from_value(value).expect("existing FlowPath readers accept file sizes");
            assert_eq!(decoded.object_path(), location);
            assert_eq!(decoded.store_ref, directory.store_ref);
            assert_eq!(decoded.cache_store_ref, directory.cache_store_ref);
        }
    }

    #[test]
    fn listing_exclusions_only_match_nested_directory_names() {
        let request: StorageListRequest = serde_json::from_value(serde_json::json!({
            "path": "site",
            "store_ref": "store",
            "exclude_directories": ["node_modules", ".git", "dist"],
        }))
        .unwrap();
        let prefix = request.flow_path.object_path();
        for (path, included) in [
            ("site/src/pages/index.astro", true),
            ("site/cms/node_modules/pkg/index.js", false),
            ("site/.git/config", false),
            ("site/nested/dist/index.html", false),
            ("site/src/node_modules", true),
            ("site/src/distinct/file.txt", true),
            ("site-other/index.astro", false),
        ] {
            assert_eq!(
                include_listed_file(&Path::from(path), &prefix, &request.exclude_directories),
                included,
                "{path}",
            );
        }
        let legacy: StorageListRequest =
            serde_json::from_value(serde_json::to_value(request.flow_path).unwrap()).unwrap();
        assert!(legacy.exclude_directories.is_empty());
    }

    #[tokio::test]
    async fn storage_copy_preserves_primary_and_cache() {
        let (context, primary, backing) = storage_context();
        let root = context
            .dir_flow_path("storage", context.get_storage_dir(false))
            .unwrap();
        let mut source = root.clone();
        source.path = root.object_path().join("source.bin").to_string();
        let mut destination = root;
        destination.path = destination.object_path().join("output.bin").to_string();
        primary
            .put(&source.object_path(), Bytes::from_static(b"source bytes"))
            .await
            .unwrap();
        assert!(copy_flow_path(&context, &source, &destination).await);
        for store in [&primary, &backing] {
            assert_eq!(
                stored_bytes(store, &destination.object_path())
                    .await
                    .as_ref(),
                b"source bytes",
            );
        }
    }

    #[tokio::test]
    async fn storage_copy_checks_all_grants_before_mutating() {
        let (context, primary, _) = storage_context();
        let root = context
            .dir_flow_path("storage", context.get_storage_dir(false))
            .unwrap();
        let mut source = root.clone();
        source.path = root.object_path().join("source.bin").to_string();
        let mut destination = root;
        destination.path = destination.object_path().join("output.bin").to_string();
        primary
            .put(&source.object_path(), Bytes::from_static(b"new"))
            .await
            .unwrap();
        primary
            .put(&destination.object_path(), Bytes::from_static(b"original"))
            .await
            .unwrap();
        let mut denied = source.clone();
        denied.path = "apps/other/storage/private.bin".into();
        assert!(!copy_flow_path(&context, &denied, &destination).await);
        destination.cache_store_ref = Some("unknown cache".into());
        assert!(!copy_flow_path(&context, &source, &destination).await);
        assert_eq!(
            stored_bytes(&primary, &destination.object_path())
                .await
                .as_ref(),
            b"original"
        );
    }

    #[tokio::test]
    async fn storage_copy_streams_large_files_between_stores() {
        let source = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let destination = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let path = Path::from("large.bin");
        let data = Bytes::from(vec![0x8a; COPY_BUFFER_SIZE + 257]);
        source.put(&path, data.clone()).await.unwrap();
        copy_between_stores(&source, &path, &destination, &path)
            .await
            .unwrap();
        assert_eq!(stored_bytes(&destination, &path).await, data);
    }

    #[test]
    fn traversal_in_guest_path_stays_inert() {
        assert_eq!(
            flow_path("apps/a/../../etc/passwd").object_path().as_ref(),
            "apps/a/%2E%2E/%2E%2E/etc/passwd"
        );
    }

    #[tokio::test]
    async fn put_validates_primary_and_cache_before_mutating_either_store() {
        let (context, primary, backing) = storage_context();
        let mut valid = context
            .dir_flow_path("storage", context.get_storage_dir(false))
            .expect("storage directory");
        let path = valid.object_path().join("file.txt");
        valid.path = path.to_string();
        primary
            .put(&path, Bytes::from_static(b"original primary"))
            .await
            .unwrap();
        backing
            .put(&path, Bytes::from_static(b"original cache"))
            .await
            .unwrap();

        for (store_ref, cache_store_ref) in [
            (
                valid.store_ref.clone(),
                Some("cache_dirs__storage_apps/b/storage".into()),
            ),
            (valid.store_ref.clone(), Some("unknown_cache".into())),
            (
                "dirs__storage_apps/b/storage".into(),
                valid.cache_store_ref.clone(),
            ),
            ("unknown_primary".into(), valid.cache_store_ref.clone()),
        ] {
            let rejected = StorageFlowPath {
                path: path.to_string(),
                store_ref,
                cache_store_ref,
            };
            assert!(!put_flow_path(&context, &rejected, b"blocked".to_vec(), "test").await);
            assert_eq!(
                stored_bytes(&primary, &path).await.as_ref(),
                b"original primary"
            );
            assert_eq!(
                stored_bytes(&backing, &path).await.as_ref(),
                b"original cache"
            );
        }

        assert!(put_flow_path(&context, &valid, b"allowed".to_vec(), "test").await);
        assert_eq!(stored_bytes(&primary, &path).await.as_ref(), b"allowed");
        assert_eq!(stored_bytes(&backing, &path).await.as_ref(), b"allowed");
    }

    #[tokio::test]
    async fn finish_write_enforces_the_originating_directory_boundary() {
        let (context, primary, backing) = storage_context();
        let directory = context
            .dir_flow_path("storage", context.get_storage_dir(true))
            .expect("node storage directory");
        let pending = RwLock::new(HashMap::new());

        for (path, allowed) in [
            ("apps/a/storage/other-node/file.txt", false),
            ("apps/b/storage/node/file.txt", false),
            ("apps/a/storage/node/file.txt", true),
        ] {
            let flow_path = StorageFlowPath {
                path: path.into(),
                ..directory.clone()
            };
            let result = start_write(&context, &pending, flow_path, Some(7)).await;
            assert_eq!(result.is_ok(), allowed);
            if let Ok(write_id) = result {
                append_chunk(&pending, &write_id, b"payload".to_vec())
                    .await
                    .unwrap();
                finish_write(&context, &pending, &write_id).await.unwrap();
            }
            assert!(pending.read().is_empty());

            let path = Path::from(path);
            for store in [&primary, &backing] {
                if allowed {
                    assert_eq!(stored_bytes(store, &path).await.as_ref(), b"payload");
                } else {
                    assert!(matches!(
                        store.as_generic().head(&path).await,
                        Err(flow_like_storage::object_store::Error::NotFound { .. })
                    ));
                }
            }
        }
    }

    #[test]
    fn opaque_store_references_use_only_their_explicit_roots() {
        let (context, primary, _) = storage_context();
        context.store_cache.write().insert(
            "external".into(),
            StorageStore {
                store: primary.clone(),
                roots: vec![Path::from("shared/file.txt"), Path::from("shared/folder")],
            },
        );
        for path in [
            "shared/file.txt",
            "shared/file.txt/child",
            "shared/folder",
            "shared/folder/child.txt",
        ] {
            assert!(
                context
                    .resolve_store("external", &Path::from(path))
                    .is_some(),
                "{path}"
            );
        }
        for path in [
            "",
            "shared",
            "shared/file.txt-other",
            "shared/folder-other/file",
        ] {
            assert!(
                context
                    .resolve_store("external", &Path::from(path))
                    .is_none(),
                "{path}"
            );
        }
        assert!(context
            .resolve_store("unknown", &Path::from("shared/file.txt"))
            .is_none());

        context.store_cache.write().insert(
            "explicit_root".into(),
            StorageStore {
                store: primary.clone(),
                roots: vec![Path::default()],
            },
        );
        for path in [Path::default(), Path::from("arbitrary/path.txt")] {
            assert!(context.resolve_store("explicit_root", &path).is_some());
        }
        context.store_cache.write().insert(
            "no_roots".into(),
            StorageStore {
                store: primary,
                roots: vec![],
            },
        );
        assert!(context
            .resolve_store("no_roots", &Path::default())
            .is_none());
    }

    #[test]
    fn cached_directory_references_cannot_bypass_context_boundaries() {
        let (context, primary, _) = storage_context();
        for (store_ref, path) in [
            ("dirs__storage_apps/b/storage", "apps/b/storage/file.txt"),
            (
                "wasm_dirs__storage_apps/b/storage",
                "apps/b/storage/file.txt",
            ),
            (
                "cache_dirs__storage_apps/b/storage",
                "apps/b/storage/file.txt",
            ),
            ("dirs__storage_", "apps/a/storage/file.txt"),
            (
                "dirs__storage_apps/a/storage/node",
                "apps/a/storage/other-node/file.txt",
            ),
            (
                "dirs__user_users/other/apps/a",
                "users/other/apps/a/file.txt",
            ),
            ("dirs__unknown_apps/a/storage", "apps/a/storage/file.txt"),
        ] {
            context.store_cache.write().insert(
                store_ref.into(),
                StorageStore {
                    store: primary.clone(),
                    roots: vec![Path::default()],
                },
            );
            assert!(
                context
                    .resolve_store(store_ref, &Path::from(path))
                    .is_none(),
                "{store_ref}"
            );
        }
    }
}
