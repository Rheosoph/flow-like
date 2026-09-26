use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{
    CopyMode, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
    ObjectStore, ObjectStoreExt, PutMode, PutMultipartOptions, PutOptions, PutPayload, PutResult,
    RenameOptions, RenameTargetMode, Result,
};
use std::fs;
use std::ops::Range;
use std::path::PathBuf;

#[derive(Debug)]
pub struct LocalObjectStore {
    store: LocalFileSystem,
    /// When true, use Android-safe implementations that avoid hard_link()
    android_safe: bool,
}

impl std::fmt::Display for LocalObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LocalFileSystem({})", self.store)
    }
}

impl LocalObjectStore {
    pub fn new(prefix: PathBuf) -> Result<Self> {
        if !prefix.exists() {
            fs::create_dir_all(&prefix).map(|_| ()).map_err(|_| {
                object_store::Error::NotImplemented {
                    operation: "create_dir_all".to_string(),
                    implementer: "LocalObjectStore".to_string(),
                }
            })?;
        }

        let store = LocalFileSystem::new_with_prefix(prefix)?.with_automatic_cleanup(true);
        Ok(Self {
            store,
            android_safe: cfg!(target_os = "android"),
        })
    }

    /// Create a new LocalObjectStore with explicit Android-safe mode setting
    pub fn new_with_android_safe(prefix: PathBuf, android_safe: bool) -> Result<Self> {
        if !prefix.exists() {
            fs::create_dir_all(&prefix).map(|_| ()).map_err(|_| {
                object_store::Error::NotImplemented {
                    operation: "create_dir_all".to_string(),
                    implementer: "LocalObjectStore".to_string(),
                }
            })?;
        }

        let store = LocalFileSystem::new_with_prefix(prefix)?.with_automatic_cleanup(true);
        Ok(Self {
            store,
            android_safe,
        })
    }

    pub fn path_to_filesystem(&self, location: &Path) -> Result<PathBuf> {
        let path = self.store.path_to_filesystem(location)?;
        Ok(path)
    }

    /// Resolve a directory prefix, including the empty prefix for the store root.
    pub fn directory_to_filesystem(&self, location: &Path) -> Result<PathBuf> {
        // object_store validates the final component as a filename, rejecting empty
        // paths and names ending in #<digits>. A directory prefix has neither restriction.
        // Resolve a placeholder child without creating it, then remove that component.
        let mut path = self
            .store
            .path_to_filesystem(&location.clone().join(".flow-like-directory-path"))?;
        path.pop();
        Ok(path)
    }
}

#[async_trait]
impl ObjectStore for LocalObjectStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> Result<PutResult> {
        let path = self.store.path_to_filesystem(location)?;
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent).map(|_| ()).map_err(|_| {
                object_store::Error::NotImplemented {
                    operation: "create_dir_all".to_string(),
                    implementer: "LocalObjectStore".to_string(),
                }
            })?;
        }

        // LocalFileSystem rejects PutMode::Update with NotImplemented. Emulate the
        // compare-and-swap with head + overwrite; the desktop store is effectively
        // single-process, so the head/put window is acceptable.
        if let PutMode::Update(expected) = &opts.mode {
            let current = self.store.head(location).await?;
            if expected.e_tag.is_some() && current.e_tag != expected.e_tag {
                return Err(object_store::Error::Precondition {
                    path: location.to_string(),
                    source: "Local object ETag did not match update precondition".into(),
                });
            }
            if expected.version.is_some() && current.version != expected.version {
                return Err(object_store::Error::Precondition {
                    path: location.to_string(),
                    source: "Local object version did not match update precondition".into(),
                });
            }
            return self
                .store
                .put_opts(
                    location,
                    payload,
                    PutOptions {
                        mode: PutMode::Overwrite,
                        ..opts
                    },
                )
                .await;
        }

        // On Android, PutMode::Create uses hard_link() which fails due to SELinux.
        // Use existence check + overwrite instead.
        if self.android_safe && matches!(opts.mode, PutMode::Create) {
            match self.store.head(location).await {
                Ok(_) => {
                    return Err(object_store::Error::AlreadyExists {
                        path: location.to_string(),
                        source: "File already exists (Android-safe check)".into(),
                    });
                }
                Err(object_store::Error::NotFound { .. }) => {
                    return self
                        .store
                        .put_opts(
                            location,
                            payload,
                            PutOptions {
                                mode: PutMode::Overwrite,
                                ..opts
                            },
                        )
                        .await;
                }
                Err(e) => return Err(e),
            }
        }

        self.store.put_opts(location, payload, opts).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>> {
        let path = self.store.path_to_filesystem(location)?;
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent).map(|_| ()).map_err(|_| {
                object_store::Error::NotImplemented {
                    operation: "create_dir_all".to_string(),
                    implementer: "LocalObjectStore".to_string(),
                }
            })?;
        }
        self.store.put_multipart_opts(location, opts).await
    }

    async fn get_opts(&self, location: &Path, opts: GetOptions) -> Result<GetResult> {
        if opts.head {
            let path = self.store.path_to_filesystem(location)?;
            if let Some(parent) = path.parent()
                && !parent.exists()
            {
                fs::create_dir_all(parent).map_err(|_| object_store::Error::NotImplemented {
                    operation: "create_dir_all".to_string(),
                    implementer: "LocalObjectStore".to_string(),
                })?;
            }
        }
        self.store.get_opts(location, opts).await
    }

    async fn get_ranges(&self, location: &Path, ranges: &[Range<u64>]) -> Result<Vec<Bytes>> {
        self.store.get_ranges(location, ranges).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, Result<Path>>,
    ) -> BoxStream<'static, Result<Path>> {
        self.store.delete_stream(locations)
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, Result<ObjectMeta>> {
        self.store.list(prefix)
    }

    fn list_with_offset(
        &self,
        prefix: Option<&Path>,
        offset: &Path,
    ) -> BoxStream<'static, Result<ObjectMeta>> {
        self.store.list_with_offset(prefix, offset)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> Result<ListResult> {
        self.store.list_with_delimiter(prefix).await
    }

    async fn copy_opts(&self, from: &Path, to: &Path, mut options: CopyOptions) -> Result<()> {
        // On Android, copy_if_not_exists uses hard_link() which fails due to SELinux.
        // Use existence check + copy instead.
        if self.android_safe && options.mode == CopyMode::Create {
            match self.store.head(to).await {
                Ok(_) => {
                    return Err(object_store::Error::AlreadyExists {
                        path: to.to_string(),
                        source: "File already exists (Android-safe check)".into(),
                    });
                }
                Err(object_store::Error::NotFound { .. }) => {
                    options.mode = CopyMode::Overwrite;
                }
                Err(e) => return Err(e),
            }
        }
        self.store.copy_opts(from, to, options).await
    }

    async fn rename_opts(&self, from: &Path, to: &Path, mut options: RenameOptions) -> Result<()> {
        // On Android, rename_if_not_exists uses hard_link() which fails due to SELinux.
        // Use existence check + rename instead.
        if self.android_safe && options.target_mode == RenameTargetMode::Create {
            match self.store.head(to).await {
                Ok(_) => {
                    return Err(object_store::Error::AlreadyExists {
                        path: to.to_string(),
                        source: "File already exists (Android-safe check)".into(),
                    });
                }
                Err(object_store::Error::NotFound { .. }) => {
                    options.target_mode = RenameTargetMode::Overwrite;
                }
                Err(e) => return Err(e),
            }
        }
        self.store.rename_opts(from, to, options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::UpdateVersion;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_store() -> (LocalObjectStore, PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "flow-like-local-store-cas-{}-{}",
            std::process::id(),
            nanos
        ));
        let store = LocalObjectStore::new(dir.clone()).unwrap();
        (store, dir)
    }

    #[test]
    fn directory_paths_resolve_root_and_missing_prefixes_without_creating_files() {
        let (_, parent) = temp_store();
        let dir = parent.join("store with spaces % and #123");
        let store = LocalObjectStore::new(dir.clone()).unwrap();
        let root = dir.canonicalize().unwrap();
        assert_eq!(
            store.directory_to_filesystem(&Path::from("")).unwrap(),
            root
        );
        for prefix in [
            "nested/repo",
            "build#123",
            "space and ünicode",
            "percent%20literal",
        ] {
            assert_eq!(
                store
                    .directory_to_filesystem(&Path::parse(prefix).unwrap())
                    .unwrap(),
                root.join(prefix),
            );
            let encoded = Path::from(prefix);
            assert_eq!(
                store.directory_to_filesystem(&encoded).unwrap(),
                root.join(encoded.as_ref())
            );
        }
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        // File operations must retain object_store's reserved filename checks.
        assert!(
            store
                .path_to_filesystem(&Path::parse("build#123").unwrap())
                .is_err()
        );
        fs::remove_dir_all(parent).unwrap();
    }

    #[tokio::test]
    async fn convenience_operations_preserve_create_only_and_android_behavior() {
        use futures::{StreamExt, TryStreamExt, stream};

        for android_safe in [false, true] {
            let (_, dir) = temp_store();
            let store = LocalObjectStore::new_with_android_safe(dir.clone(), android_safe).unwrap();
            let source = Path::from("nested/source.txt");
            let target = Path::from("nested/target.txt");
            store
                .put_opts(
                    &source,
                    PutPayload::from_static(b"source"),
                    PutOptions::from(PutMode::Create),
                )
                .await
                .unwrap();
            store
                .put(&target, PutPayload::from_static(b"target"))
                .await
                .unwrap();

            assert!(matches!(
                store.copy_if_not_exists(&source, &target).await,
                Err(object_store::Error::AlreadyExists { .. })
            ));
            assert!(matches!(
                store.rename_if_not_exists(&source, &target).await,
                Err(object_store::Error::AlreadyExists { .. })
            ));
            assert_eq!(
                store.get(&target).await.unwrap().bytes().await.unwrap(),
                Bytes::from_static(b"target")
            );
            assert_eq!(store.head(&source).await.unwrap().size, 6);

            store.copy(&source, &target).await.unwrap();
            assert_eq!(
                store.get_range(&target, 1..4).await.unwrap(),
                Bytes::from_static(b"our")
            );
            store.delete(&target).await.unwrap();
            store.copy_if_not_exists(&source, &target).await.unwrap();
            store.delete(&target).await.unwrap();
            store.rename_if_not_exists(&source, &target).await.unwrap();
            assert!(matches!(
                store.head(&source).await,
                Err(object_store::Error::NotFound { .. })
            ));
            let removed: Vec<_> = store
                .delete_stream(stream::iter([Ok(target.clone())]).boxed())
                .try_collect()
                .await
                .unwrap();
            assert_eq!(removed, [target]);
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[tokio::test]
    async fn put_update_swaps_on_matching_etag_and_rejects_stale() {
        let (store, dir) = temp_store();
        let location = Path::from("board.board");

        store
            .put(&location, PutPayload::from_static(b"v1"))
            .await
            .unwrap();
        let first = store.head(&location).await.unwrap();

        store
            .put_opts(
                &location,
                PutPayload::from_static(b"v2"),
                PutOptions {
                    mode: PutMode::Update(UpdateVersion {
                        e_tag: first.e_tag.clone(),
                        version: first.version.clone(),
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let bytes = store.get(&location).await.unwrap().bytes().await.unwrap();
        assert_eq!(bytes.as_ref(), b"v2");

        let stale = store
            .put_opts(
                &location,
                PutPayload::from_static(b"v3"),
                PutOptions {
                    mode: PutMode::Update(UpdateVersion {
                        e_tag: first.e_tag,
                        version: first.version,
                    }),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(
            stale,
            Err(object_store::Error::Precondition { .. })
        ));
        let bytes = store.get(&location).await.unwrap().bytes().await.unwrap();
        assert_eq!(bytes.as_ref(), b"v2");

        fs::remove_dir_all(dir).ok();
    }
}
