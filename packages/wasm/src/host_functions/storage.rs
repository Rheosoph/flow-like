//! Storage host functions
//!
//! Provides storage access for WASM modules.

use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::normalize_object_path;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::object_store::{path::Path, PutPayload};
use flow_like_types::Bytes;
use std::collections::HashMap;

use super::StorageContext;

pub const MAX_STORAGE_FILE_SIZE: usize = 10 * 1024 * 1024;
pub const MAX_PENDING_WRITES: usize = 8;
pub const MAX_TOTAL_WRITE_SIZE: usize = 512 * 1024 * 1024;

/// A host-resolved store and the object paths supplied to this invocation.
#[derive(Clone)]
pub struct StorageStore {
    pub store: FlowLikeStore,
    pub roots: Vec<Path>,
}

#[derive(Debug)]
pub struct PendingWrite {
    pub flow_path: StorageFlowPath,
    pub buffer: Vec<u8>,
    pub total_size: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct StorageFlowPath {
    pub path: String,
    pub store_ref: String,
    pub cache_store_ref: Option<String>,
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

pub fn start_write(
    pending: &mut HashMap<String, PendingWrite>,
    flow_path: StorageFlowPath,
    total_size: u64,
) -> Option<String> {
    if pending.len() >= MAX_PENDING_WRITES {
        tracing::warn!("[wasm write-start] rejected: too many pending writes");
        return None;
    }
    if total_size as usize > MAX_TOTAL_WRITE_SIZE {
        tracing::warn!(
            "[wasm write-start] rejected: total_size {} exceeds max {}",
            total_size,
            MAX_TOTAL_WRITE_SIZE
        );
        return None;
    }
    let id = format!(
        "cw_{:x}_{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        pending.len()
    );
    pending.insert(
        id.clone(),
        PendingWrite {
            flow_path,
            buffer: Vec::with_capacity(total_size as usize),
            total_size,
        },
    );
    Some(id)
}

pub fn append_chunk(
    pending: &mut HashMap<String, PendingWrite>,
    write_id: &str,
    data: &[u8],
) -> bool {
    if data.len() > MAX_STORAGE_FILE_SIZE {
        tracing::warn!(
            "[wasm write-chunk] rejected: chunk size {} exceeds max {}",
            data.len(),
            MAX_STORAGE_FILE_SIZE
        );
        return false;
    }
    let Some(pw) = pending.get_mut(write_id) else {
        tracing::warn!("[wasm write-chunk] rejected: unknown write_id {write_id}");
        return false;
    };
    if pw.buffer.len() + data.len() > pw.total_size as usize {
        tracing::warn!("[wasm write-chunk] rejected: would exceed declared total_size");
        return false;
    }
    pw.buffer.extend_from_slice(data);
    true
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

pub async fn finish_write(
    pending: &mut HashMap<String, PendingWrite>,
    write_id: &str,
    storage_ctx: &StorageContext,
) -> bool {
    let Some(pw) = pending.remove(write_id) else {
        tracing::warn!("[wasm write-finish] rejected: unknown write_id {write_id}");
        return false;
    };
    put_flow_path(storage_ctx, &pw.flow_path, pw.buffer, "wasm write-finish").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::state::FlowLikeStores;
    use flow_like_storage::display_file_name;
    use flow_like_storage::object_store::memory::InMemory;
    use parking_lot::RwLock;
    use std::sync::Arc;

    const NAME: &str = "Übersicht (2)#1.pdf";

    fn flow_path(path: impl Into<String>) -> StorageFlowPath {
        StorageFlowPath {
            path: path.into(),
            store_ref: "store".into(),
            cache_store_ref: None,
        }
    }

    fn storage_context() -> (StorageContext, FlowLikeStore, FlowLikeStore) {
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

    async fn stored_bytes(store: &FlowLikeStore, path: &Path) -> Bytes {
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
        let mut pending = HashMap::new();

        for (path, allowed) in [
            ("apps/a/storage/other-node/file.txt", false),
            ("apps/b/storage/node/file.txt", false),
            ("apps/a/storage/node/file.txt", true),
        ] {
            let flow_path = StorageFlowPath {
                path: path.into(),
                ..directory.clone()
            };
            let write_id = start_write(&mut pending, flow_path, 7).expect("pending write");
            assert!(append_chunk(&mut pending, &write_id, b"payload"));
            assert_eq!(
                finish_write(&mut pending, &write_id, &context).await,
                allowed
            );
            assert!(pending.is_empty());

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
