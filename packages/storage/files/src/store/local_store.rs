use super::secure_local::{DiskNames, SecureLocalStore};
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
        Self::open(prefix, DiskNames::Encoded)
    }

    /// Retained for callers selecting Android behavior. The descriptor backend
    /// enforces the same create-only contract on every platform.
    pub fn new_with_android_safe(prefix: PathBuf, _android_safe: bool) -> Result<Self> {
        Self::open(prefix, DiskNames::Encoded)
    }

    /// A store over a folder a person owns, such as a picked or mounted share.
    /// Keys stay percent-encoded like every other store, but each segment addresses the
    /// decoded name on disk: `F%C3%BCgen.pdf` opens `Fügen.pdf`, and listings encode the
    /// real names. App storage keeps [`Self::new`], whose files are named by the encoded key.
    pub fn host_folder(prefix: PathBuf) -> Result<Self> {
        Self::open(prefix, DiskNames::Decoded)
    }

    fn open(prefix: PathBuf, names: DiskNames) -> Result<Self> {
        if !prefix.exists() {
            fs::create_dir_all(&prefix).map_err(|source| object_store::Error::Generic {
                store: "LocalObjectStore",
                source: source.into(),
            })?;
        }
        let secure = SecureLocalStore::new(&prefix, names)?;
        let store = LocalFileSystem::new_with_prefix(prefix)?;
        Ok(Self { store, secure })
    }

    pub fn path_to_filesystem(&self, location: &Path) -> Result<PathBuf> {
        let path = self.store.path_to_filesystem(location)?;
        match self.secure.names() {
            DiskNames::Encoded => Ok(path),
            DiskNames::Decoded => self.decoded_path(location),
        }
    }

    pub fn metadata(&self, location: &Path) -> Result<Option<std::fs::Metadata>> {
        self.secure.metadata(location)
    }

    /// Resolve a directory prefix, including the empty prefix for the store root.
    pub fn directory_to_filesystem(&self, location: &Path) -> Result<PathBuf> {
        match self.secure.names() {
            DiskNames::Encoded => self.encoded_directory(location),
            DiskNames::Decoded => self.decoded_path(location),
        }
    }

    fn encoded_directory(&self, location: &Path) -> Result<PathBuf> {
        // object_store validates the final component as a filename, rejecting empty
        // paths and names ending in #<digits>. A directory prefix has neither restriction.
        // Resolve a placeholder child without creating it, then remove that component.
        let mut path = self
            .store
            .path_to_filesystem(&location.clone().join(".flow-like-directory-path"))?;
        path.pop();
        Ok(path)
    }

    fn decoded_path(&self, location: &Path) -> Result<PathBuf> {
        let mut path = self.encoded_directory(&Path::default())?;
        path.extend(self.secure.segments(location)?.iter().map(|name| &**name));
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
    async fn a_host_folder_addresses_real_names_through_encoded_keys() {
        use crate::object_path::normalize_object_path;
        use futures::TryStreamExt;

        const RAW_NAME: &str = "Übersicht (2)#1.pdf";
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("Fügen")).unwrap();
        fs::write(root.path().join("Fügen").join(RAW_NAME), b"share").unwrap();
        let store = LocalObjectStore::host_folder(root.path().into()).unwrap();
        let key = Path::from("Fügen").join(RAW_NAME);

        let listed: Vec<_> = store
            .list(None)
            .map_ok(|meta| meta.location)
            .try_collect()
            .await
            .unwrap();
        assert_eq!(listed, [key.clone()]);
        assert_eq!(
            store
                .list_with_delimiter(None)
                .await
                .unwrap()
                .common_prefixes,
            [Path::from("Fügen")]
        );
        let nested = store
            .list_with_delimiter(Some(&Path::from("Fügen")))
            .await
            .unwrap();
        assert_eq!(nested.objects[0].location, key);

        for location in [
            key.clone(),
            normalize_object_path(&format!("Fügen/{RAW_NAME}")),
        ] {
            assert_eq!(
                store.get(&location).await.unwrap().bytes().await.unwrap(),
                Bytes::from_static(b"share")
            );
        }
        let canonical = root.path().canonicalize().unwrap();
        assert_eq!(
            store.path_to_filesystem(&key).unwrap(),
            canonical.join("Fügen").join(RAW_NAME)
        );
        assert_eq!(
            store.directory_to_filesystem(&Path::from("Fügen")).unwrap(),
            canonical.join("Fügen")
        );

        let written = Path::from("Fügen").join("Ärger 100%.txt");
        store
            .put(&written, PutPayload::from_static(b"new"))
            .await
            .unwrap();
        assert_eq!(
            fs::read(root.path().join("Fügen").join("Ärger 100%.txt")).unwrap(),
            b"new"
        );
        store.delete(&written).await.unwrap();
        assert!(!root.path().join("Fügen").join("Ärger 100%.txt").exists());

        for escape in [
            Path::from(".."),
            Path::from("Fügen").join(".."),
            normalize_object_path("Fügen%2F..%2F..%2Fsecret"),
        ] {
            assert!(
                matches!(
                    store.get(&escape).await,
                    Err(object_store::Error::PermissionDenied { .. })
                ),
                "{escape}"
            );
        }
    }

    #[tokio::test]
    async fn app_storage_names_files_by_the_encoded_key() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::new(root.path().into()).unwrap();
        let key = Path::from("Fügen.txt");
        store
            .put(&key, PutPayload::from_static(b"data"))
            .await
            .unwrap();
        assert!(root.path().join(key.as_ref()).is_file());
        assert!(!root.path().join("Fügen.txt").exists());
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
    async fn a_delete_removes_the_folders_it_empties_but_never_the_root() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::new(root.path().into()).unwrap();
        for key in [
            "app/kept.txt",
            "app/deep/nested/file.txt",
            "app/deep/sibling/file.txt",
            "top.txt",
        ] {
            store
                .put(&Path::from(key), PutPayload::from_static(b"data"))
                .await
                .unwrap();
        }

        store
            .delete(&Path::from("app/deep/nested/file.txt"))
            .await
            .unwrap();
        assert!(!root.path().join("app/deep/nested").exists());
        assert!(root.path().join("app/deep/sibling/file.txt").is_file());

        store
            .delete(&Path::from("app/deep/sibling/file.txt"))
            .await
            .unwrap();
        assert!(!root.path().join("app/deep").exists());
        assert!(
            root.path().join("app/kept.txt").is_file(),
            "a folder that still holds a file stays"
        );

        for key in ["app/kept.txt", "top.txt"] {
            store.delete(&Path::from(key)).await.unwrap();
        }
        assert!(root.path().is_dir(), "the root stays when it is empty");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        assert!(
            store
                .list_with_delimiter(None)
                .await
                .unwrap()
                .common_prefixes
                .is_empty()
        );
        assert!(matches!(
            store.delete(&Path::from("app/kept.txt")).await,
            Err(object_store::Error::NotFound { .. })
        ));

        store
            .put(
                &Path::from("app/deep/again.txt"),
                PutPayload::from_static(b"data"),
            )
            .await
            .unwrap();
        assert!(root.path().join("app/deep/again.txt").is_file());
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
