use super::secure_local::SecureLocalStore;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{
    CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions, Result,
};
use std::fs;
use std::ops::Range;
use std::path::PathBuf;

#[derive(Debug)]
pub struct LocalObjectStore {
    store: LocalFileSystem,
    secure: SecureLocalStore,
}

impl std::fmt::Display for LocalObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LocalFileSystem({})", self.store)
    }
}

impl LocalObjectStore {
    pub fn new(prefix: PathBuf) -> Result<Self> {
        Self::new_with_android_safe(prefix, cfg!(target_os = "android"))
    }

    /// Retained for callers selecting Android behavior. The descriptor backend
    /// enforces the same create-only contract on every platform.
    pub fn new_with_android_safe(prefix: PathBuf, _android_safe: bool) -> Result<Self> {
        if !prefix.exists() {
            fs::create_dir_all(&prefix).map_err(|source| object_store::Error::Generic {
                store: "LocalObjectStore",
                source: source.into(),
            })?;
        }
        let secure = SecureLocalStore::new(&prefix)?;
        let store = LocalFileSystem::new_with_prefix(prefix)?;
        Ok(Self { store, secure })
    }

    pub fn path_to_filesystem(&self, location: &Path) -> Result<PathBuf> {
        let path = self.store.path_to_filesystem(location)?;
        Ok(path)
    }

    pub fn metadata(&self, location: &Path) -> Result<Option<std::fs::Metadata>> {
        self.secure.metadata(location)
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
        self.path_to_filesystem(location)?;
        self.secure.put_opts(location, payload, opts).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>> {
        self.path_to_filesystem(location)?;
        self.secure.put_multipart_opts(location, opts).await
    }

    async fn get_opts(&self, location: &Path, opts: GetOptions) -> Result<GetResult> {
        self.path_to_filesystem(location)?;
        self.secure.get_opts(location, opts).await
    }

    async fn get_ranges(&self, location: &Path, ranges: &[Range<u64>]) -> Result<Vec<Bytes>> {
        self.path_to_filesystem(location)?;
        self.secure.get_ranges(location, ranges).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, Result<Path>>,
    ) -> BoxStream<'static, Result<Path>> {
        self.secure.delete_stream(locations)
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, Result<ObjectMeta>> {
        self.secure.list(prefix)
    }

    fn list_with_offset(
        &self,
        prefix: Option<&Path>,
        offset: &Path,
    ) -> BoxStream<'static, Result<ObjectMeta>> {
        self.secure.list_with_offset(prefix, offset)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> Result<ListResult> {
        self.secure.list_with_delimiter(prefix).await
    }

    async fn copy_opts(&self, from: &Path, to: &Path, options: CopyOptions) -> Result<()> {
        self.path_to_filesystem(from)?;
        self.path_to_filesystem(to)?;
        self.secure.copy_opts(from, to, options).await
    }

    async fn rename_opts(&self, from: &Path, to: &Path, options: RenameOptions) -> Result<()> {
        self.path_to_filesystem(from)?;
        self.path_to_filesystem(to)?;
        self.secure.rename_opts(from, to, options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::{ObjectStoreExt, PutMode, UpdateVersion};
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
    async fn missing_metadata_does_not_create_parent_directories() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::new(root.path().into()).unwrap();
        assert!(matches!(
            store.head(&Path::from("missing/nested/file.txt")).await,
            Err(object_store::Error::NotFound { .. })
        ));
        assert!(!root.path().join("missing").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_metadata_through_symlink_does_not_create_target_directories() {
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::new(root.path().into()).unwrap();
        std::os::unix::fs::symlink(target.path(), root.path().join("link")).unwrap();
        assert!(matches!(
            store
                .head(&Path::from("link/missing/nested/file.txt"))
                .await,
            Err(object_store::Error::NotFound { .. } | object_store::Error::PermissionDenied { .. })
        ));
        assert!(!target.path().join("missing").exists());
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
