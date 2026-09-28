use super::super::super::git;
use super::{Workspace, same_version};
use flow_like_storage::{
    Path as StorePath,
    files::store::FlowLikeStore,
    object_store::{
        self, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
        ObjectStore, ObjectStoreExt, PutMultipartOptions, PutOptions, PutPayload, PutResult,
        memory::InMemory,
    },
};
use flow_like_types::{async_trait, tokio};
use futures::stream::BoxStream;
use std::{fs, path::Path, sync::Arc};

#[derive(Debug)]
struct FailingPutStore {
    inner: Arc<InMemory>,
}

impl std::fmt::Display for FailingPutStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "FailingPutStore")
    }
}

#[async_trait]
impl ObjectStore for FailingPutStore {
    async fn put_opts(
        &self,
        location: &StorePath,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        if location.filename() == Some("z-fail.txt") {
            return Err(object_store::Error::NotSupported {
                source: "injected upload failure".into(),
            });
        }
        self.inner.put_opts(location, payload, options).await
    }

    async fn put_multipart_opts(
        &self,
        location: &StorePath,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, options).await
    }

    async fn get_opts(
        &self,
        location: &StorePath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.inner.get_opts(location, options).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<StorePath>>,
    ) -> BoxStream<'static, object_store::Result<StorePath>> {
        self.inner.delete_stream(locations)
    }

    fn list(
        &self,
        prefix: Option<&StorePath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&StorePath>,
    ) -> object_store::Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &StorePath,
        to: &StorePath,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        self.inner.copy_opts(from, to, options).await
    }
}

fn memory_store() -> FlowLikeStore {
    FlowLikeStore::Memory(Arc::new(InMemory::new()))
}

fn initialize(path: &Path) {
    fs::create_dir_all(path).unwrap();
    git::run(path, &["init", "--initial-branch", "main"]).unwrap();
    git::run(path, &["config", "user.name", "Workspace Test"]).unwrap();
    git::run(path, &["config", "user.email", "workspace@example.invalid"]).unwrap();
}

fn commit_file(path: &Path, file: &str, content: &str, message: &str) {
    fs::write(path.join(file), content).unwrap();
    git::run(path, &["add", "--", file]).unwrap();
    git::run(path, &["commit", "-m", message]).unwrap();
}

#[tokio::test]
async fn cloud_listing_metadata_uses_shared_change_tokens_before_timestamps() {
    let objects = InMemory::new();
    let key = StorePath::from("repositories/versioned/file.txt");
    objects
        .put(&key, PutPayload::from_static(b"original content"))
        .await
        .unwrap();
    let mut downloaded = objects.head(&key).await.unwrap();
    assert!(downloaded.e_tag.is_some());
    downloaded.version = Some("object-version-1".into());
    let mut listed = downloaded.clone();
    listed.version = None;
    listed.last_modified = std::time::UNIX_EPOCH.into();
    assert!(same_version(&listed, &downloaded));
    assert!(same_version(&downloaded, &listed));

    let mut changed = listed.clone();
    changed.e_tag = Some("different-etag".into());
    assert!(!same_version(&changed, &downloaded));
    changed = listed.clone();
    changed.version = Some("object-version-2".into());
    assert!(!same_version(&changed, &downloaded));
    changed = listed.clone();
    changed.size += 1;
    assert!(!same_version(&changed, &downloaded));

    listed.e_tag = None;
    downloaded.e_tag = None;
    listed.version = downloaded.version.clone();
    assert!(same_version(&listed, &downloaded));
    listed.version = None;
    downloaded.version = None;
    assert!(!same_version(&listed, &downloaded));
    listed.last_modified = downloaded.last_modified;
    assert!(same_version(&listed, &downloaded));
}

#[tokio::test]
async fn initialized_repository_without_commits_survives_reopen() {
    let store = memory_store();
    let prefix = StorePath::from("repositories/empty");
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    initialize(workspace.path());
    workspace.persist().await.unwrap();

    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    git::ensure_repository(workspace.path()).unwrap();
    assert_eq!(
        git::run(workspace.path(), &["branch", "--show-current"])
            .unwrap()
            .trim(),
        "main"
    );
    assert!(
        git::run(workspace.path(), &["status", "--porcelain=v1"])
            .unwrap()
            .is_empty()
    );
    commit_file(workspace.path(), "first.txt", "first\n", "First commit");
    workspace.persist().await.unwrap();

    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    assert_eq!(
        git::run(workspace.path(), &["show", "HEAD:first.txt"]).unwrap(),
        "first\n"
    );
    workspace.persist().await.unwrap();
}

#[tokio::test]
async fn memory_workspace_preserves_index_worktree_and_commit_history() {
    let store = memory_store();
    let prefix = StorePath::from("repositories/project");
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    initialize(workspace.path());
    commit_file(
        workspace.path(),
        "tracked.txt",
        "original\n",
        "Initial commit",
    );
    workspace.persist().await.unwrap();

    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    git::ensure_repository(workspace.path()).unwrap();
    assert_eq!(
        git::run(workspace.path(), &["branch", "--show-current"])
            .unwrap()
            .trim(),
        "main"
    );
    fs::write(workspace.path().join("tracked.txt"), "staged\n").unwrap();
    git::run(workspace.path(), &["add", "--", "tracked.txt"]).unwrap();
    fs::write(workspace.path().join("tracked.txt"), "unstaged\n").unwrap();
    fs::write(workspace.path().join("notes.txt"), "untracked\n").unwrap();
    workspace.persist().await.unwrap();

    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    assert_eq!(
        git::run(workspace.path(), &["show", ":tracked.txt"]).unwrap(),
        "staged\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("tracked.txt")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("notes.txt")).unwrap(),
        "untracked\n"
    );
    let status = git::run(workspace.path(), &["status", "--porcelain=v1"]).unwrap();
    assert!(status.lines().any(|line| line == "MM tracked.txt"));
    assert!(status.lines().any(|line| line == "?? notes.txt"));
    let result = git::run(workspace.path(), &["commit", "-m", "Commit staged content"]);
    workspace.finish(result).await.unwrap();

    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    assert_eq!(
        git::run(workspace.path(), &["log", "--format=%s", "-2"]).unwrap(),
        "Commit staged content\nInitial commit\n"
    );
    assert_eq!(
        git::run(workspace.path(), &["show", "HEAD:tracked.txt"]).unwrap(),
        "staged\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("tracked.txt")).unwrap(),
        "unstaged\n"
    );
    workspace.persist().await.unwrap();
}

#[tokio::test]
async fn generic_object_store_deletes_removed_files_without_touching_neighbor_prefixes() {
    let objects = Arc::new(InMemory::new());
    let removed = StorePath::from("repositories/project/removed.txt");
    let neighbor = StorePath::from("repositories/project-other/keep.txt");
    objects
        .put(&removed, PutPayload::from_static(b"remove me"))
        .await
        .unwrap();
    objects
        .put(&neighbor, PutPayload::from_static(b"neighbor"))
        .await
        .unwrap();
    let store = FlowLikeStore::Other(objects.clone());
    let prefix = StorePath::from("repositories/project");
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    fs::remove_file(workspace.path().join("removed.txt")).unwrap();
    fs::write(workspace.path().join("added.txt"), "new content").unwrap();
    workspace.persist().await.unwrap();

    assert!(matches!(
        objects.get(&removed).await,
        Err(flow_like_storage::object_store::Error::NotFound { .. })
    ));
    assert_eq!(
        objects.get(&neighbor).await.unwrap().bytes().await.unwrap(),
        b"neighbor".as_slice()
    );
    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    assert!(!workspace.path().join("removed.txt").exists());
    assert_eq!(
        fs::read_to_string(workspace.path().join("added.txt")).unwrap(),
        "new content"
    );
    workspace.persist().await.unwrap();
}

#[tokio::test]
async fn external_edit_prevents_all_workspace_writes_and_releases_lock() {
    let objects = Arc::new(InMemory::new());
    let prefix = StorePath::from("repositories/externally-edited");
    let shared = prefix.clone().join("shared.txt");
    let unchanged = prefix.clone().join("unchanged.txt");
    let added = prefix.clone().join("added.txt");
    objects
        .put(&shared, PutPayload::from_static(b"original shared content"))
        .await
        .unwrap();
    objects
        .put(
            &unchanged,
            PutPayload::from_static(b"original other content"),
        )
        .await
        .unwrap();
    let store = FlowLikeStore::Memory(objects.clone());
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    fs::write(workspace.path().join("shared.txt"), "workspace change").unwrap();
    fs::write(workspace.path().join("unchanged.txt"), "another change").unwrap();
    fs::write(workspace.path().join("added.txt"), "new workspace file").unwrap();
    objects
        .put(&shared, PutPayload::from_static(b"external change"))
        .await
        .unwrap();

    let error = workspace.persist().await.unwrap_err();
    assert!(error.to_string().contains("storage changed"));
    assert_eq!(
        objects.get(&shared).await.unwrap().bytes().await.unwrap(),
        b"external change".as_slice()
    );
    assert_eq!(
        objects
            .get(&unchanged)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        b"original other content".as_slice()
    );
    assert!(matches!(
        objects.get(&added).await,
        Err(flow_like_storage::object_store::Error::NotFound { .. })
    ));

    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("shared.txt")).unwrap(),
        "external change"
    );
    workspace.persist().await.unwrap();
}

#[tokio::test]
async fn partial_save_retains_recovery_checkout_and_lock_until_manual_recovery() {
    let objects = Arc::new(InMemory::new());
    let store = FlowLikeStore::Other(Arc::new(FailingPutStore {
        inner: objects.clone(),
    }));
    let prefix = StorePath::from("repositories/interrupted-save");
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    fs::create_dir_all(workspace.path()).unwrap();
    fs::write(workspace.path().join("a-written.txt"), "first upload").unwrap();
    fs::write(workspace.path().join("z-fail.txt"), "recover this content").unwrap();
    let recovery_path = workspace.path().to_path_buf();
    let temporary = recovery_path.parent().unwrap().to_path_buf();
    let lock = prefix.clone().join(".git").join("flow-like-store.lock");

    let error = workspace.persist().await.unwrap_err().to_string();
    assert!(error.contains("partially updated"));
    assert!(error.contains(recovery_path.to_str().unwrap()));
    assert!(error.contains(lock.as_ref()));
    assert_eq!(
        objects
            .get(&prefix.clone().join("a-written.txt"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        b"first upload".as_slice()
    );
    assert!(matches!(
        objects.get(&prefix.clone().join("z-fail.txt")).await,
        Err(object_store::Error::NotFound { .. })
    ));

    drop(workspace);
    assert!(recovery_path.is_dir());
    assert!(objects.head(&lock).await.is_ok());
    assert!(
        Workspace::from_store(store.clone(), prefix.clone())
            .await
            .is_err()
    );
    let recovered = fs::read(recovery_path.join("z-fail.txt")).unwrap();
    assert_eq!(recovered, b"recover this content");
    objects
        .put(&prefix.clone().join("z-fail.txt"), recovered.into())
        .await
        .unwrap();
    objects.delete(&lock).await.unwrap();
    fs::remove_dir_all(temporary).unwrap();

    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("a-written.txt")).unwrap(),
        "first upload"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("z-fail.txt")).unwrap(),
        "recover this content"
    );
    workspace.persist().await.unwrap();
}

#[tokio::test]
async fn dropping_unsaved_workspace_removes_temporary_files_and_releases_lock() {
    let objects = Arc::new(InMemory::new());
    let store = FlowLikeStore::Memory(objects.clone());
    let prefix = StorePath::from("repositories/dropped");
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    fs::create_dir_all(workspace.path()).unwrap();
    fs::write(workspace.path().join("unsaved.txt"), "temporary change").unwrap();
    let temporary = workspace.path().parent().unwrap().to_path_buf();
    let lock = prefix.clone().join(".git").join("flow-like-store.lock");
    drop(workspace);

    assert!(!temporary.exists());
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if matches!(
                objects.head(&lock).await,
                Err(object_store::Error::NotFound { .. })
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping a workspace should release its storage lock");

    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    assert!(!workspace.path().join("unsaved.txt").exists());
    workspace.persist().await.unwrap();
}

#[tokio::test]
async fn literal_percent_and_hash_filenames_survive_reopen_without_collisions() {
    let store = memory_store();
    let prefix = StorePath::from("repositories/special-names");
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    initialize(workspace.path());
    let files = [
        ("%41", "literal percent"),
        ("A", "letter"),
        ("file#123", "hash"),
    ];
    for (name, content) in files {
        fs::write(workspace.path().join(name), content).unwrap();
        git::run(workspace.path(), &["add", "--", name]).unwrap();
    }
    git::run(workspace.path(), &["commit", "-m", "Special filenames"]).unwrap();
    workspace.persist().await.unwrap();

    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    for (name, content) in files {
        assert_eq!(
            fs::read_to_string(workspace.path().join(name)).unwrap(),
            content
        );
        assert_eq!(
            git::run(workspace.path(), &["show", &format!("HEAD:{name}")]).unwrap(),
            content
        );
    }
    assert!(
        git::run(workspace.path(), &["status", "--porcelain=v1"])
            .unwrap()
            .is_empty()
    );
    workspace.persist().await.unwrap();
}

#[tokio::test]
async fn encoded_traversal_is_rejected_before_materialization_and_releases_lock() {
    let objects = Arc::new(InMemory::new());
    let prefix = StorePath::from("repositories/unsafe");
    let unsafe_key = prefix.clone().join("..").join("escaped.txt");
    assert!(unsafe_key.as_ref().contains("%2E%2E"));
    objects
        .put(&unsafe_key, PutPayload::from_static(b"outside checkout"))
        .await
        .unwrap();
    let store = FlowLikeStore::Memory(objects.clone());
    let result = Workspace::from_store(store.clone(), prefix.clone()).await;
    let error = match result {
        Ok(_) => panic!("an encoded parent-directory component must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("unsafe path"));
    assert_eq!(
        objects
            .get(&unsafe_key)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        b"outside checkout".as_slice()
    );

    objects.delete(&unsafe_key).await.unwrap();
    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    workspace.persist().await.unwrap();
}

#[tokio::test]
async fn workspace_rejects_concurrent_open_and_releases_lock_after_operation_error() {
    let store = memory_store();
    let prefix = StorePath::from("repositories/locked");
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    assert!(
        Workspace::from_store(store.clone(), prefix.clone())
            .await
            .is_err()
    );
    fs::create_dir_all(workspace.path()).unwrap();
    fs::write(workspace.path().join("partial.txt"), "partial change").unwrap();
    let result = workspace
        .finish::<()>(Err(flow_like_types::anyhow!("Git operation failed")))
        .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Git operation failed")
    );

    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("partial.txt")).unwrap(),
        "partial change"
    );
    workspace.finish(Ok(())).await.unwrap();
}

#[tokio::test]
async fn failed_merge_preserves_conflicts_for_a_later_abort() {
    let store = memory_store();
    let prefix = StorePath::from("repositories/conflicts");
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    initialize(workspace.path());
    commit_file(workspace.path(), "conflict.txt", "base\n", "Base");
    git::run(workspace.path(), &["switch", "-c", "topic"]).unwrap();
    commit_file(workspace.path(), "conflict.txt", "topic\n", "Topic");
    git::run(workspace.path(), &["switch", "main"]).unwrap();
    commit_file(workspace.path(), "conflict.txt", "main\n", "Main");
    workspace.persist().await.unwrap();

    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    let result = git::run(workspace.path(), &["merge", "topic"]);
    assert!(result.is_err());
    assert!(workspace.finish(result).await.is_err());

    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    assert!(workspace.path().join(".git/MERGE_HEAD").is_file());
    assert!(
        git::run(workspace.path(), &["status", "--porcelain=v1"])
            .unwrap()
            .lines()
            .any(|line| line == "UU conflict.txt")
    );
    assert!(
        fs::read_to_string(workspace.path().join("conflict.txt"))
            .unwrap()
            .contains("<<<<<<< HEAD")
    );
    let result = git::run(workspace.path(), &["merge", "--abort"]);
    workspace.finish(result).await.unwrap();

    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    assert!(!workspace.path().join(".git/MERGE_HEAD").exists());
    assert!(
        git::run(workspace.path(), &["status", "--porcelain=v1"])
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("conflict.txt")).unwrap(),
        "main\n"
    );
    workspace.persist().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn memory_workspace_preserves_symlinks_and_executable_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let store = memory_store();
    let prefix = StorePath::from("repositories/modes");
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    initialize(workspace.path());
    let script = workspace.path().join("run.sh");
    fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    symlink("run.sh", workspace.path().join("tracked-link")).unwrap();
    git::run(workspace.path(), &["add", "--", "run.sh", "tracked-link"]).unwrap();
    git::run(workspace.path(), &["commit", "-m", "Executable and link"]).unwrap();
    symlink("run.sh", workspace.path().join("untracked-link")).unwrap();
    let untracked_script = workspace.path().join("untracked.sh");
    fs::write(&untracked_script, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&untracked_script, fs::Permissions::from_mode(0o755)).unwrap();
    workspace.persist().await.unwrap();

    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    for name in ["tracked-link", "untracked-link"] {
        assert!(
            fs::symlink_metadata(workspace.path().join(name))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(workspace.path().join(name)).unwrap(),
            Path::new("run.sh")
        );
    }
    for name in ["run.sh", "untracked.sh"] {
        assert_ne!(
            fs::metadata(workspace.path().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }
    assert!(
        git::run(workspace.path(), &["diff", "--name-only"])
            .unwrap()
            .is_empty()
    );
    workspace.persist().await.unwrap();
}

#[tokio::test]
async fn filtered_persistence_excludes_git_metadata_when_requested() {
    let store = memory_store();
    let prefix = StorePath::from("repositories/files-only");
    let workspace = Workspace::from_store(store.clone(), prefix.clone())
        .await
        .unwrap();
    initialize(workspace.path());
    commit_file(workspace.path(), "file.txt", "content\n", "Initial commit");
    workspace.persist_filtered(false).await.unwrap();

    let workspace = Workspace::from_store(store, prefix).await.unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("file.txt")).unwrap(),
        "content\n"
    );
    assert!(!workspace.path().join(".git/HEAD").exists());
    assert!(git::ensure_repository(workspace.path()).is_err());
    workspace.persist().await.unwrap();
}

#[tokio::test]
async fn directory_case_variants_are_preserved_or_rejected_without_changing_storage() {
    let probe =
        std::env::temp_dir().join(format!("flow-like-git-case-probe-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&probe).unwrap();
    fs::create_dir(probe.join("A")).unwrap();
    let directory_case_aliases = probe.join("a").exists();
    fs::remove_dir_all(probe).unwrap();

    let objects = Arc::new(InMemory::new());
    let prefix = StorePath::from("repositories/case-variants");
    let uppercase_file = prefix.clone().join("A").join("x.txt");
    let lowercase_file = prefix.clone().join("a").join("y.txt");
    objects
        .put(
            &uppercase_file,
            PutPayload::from_static(b"uppercase directory"),
        )
        .await
        .unwrap();
    objects
        .put(
            &lowercase_file,
            PutPayload::from_static(b"lowercase directory"),
        )
        .await
        .unwrap();
    let uppercase_original = objects.head(&uppercase_file).await.unwrap();
    let lowercase_original = objects.head(&lowercase_file).await.unwrap();
    let store = FlowLikeStore::Memory(objects.clone());
    let result = Workspace::from_store(store, prefix.clone()).await;
    if directory_case_aliases {
        assert!(
            result.is_err(),
            "directory aliases must be rejected before storage can be changed"
        );
    } else {
        let workspace = result.unwrap();
        assert_eq!(
            fs::read_to_string(workspace.path().join("A/x.txt")).unwrap(),
            "uppercase directory"
        );
        assert_eq!(
            fs::read_to_string(workspace.path().join("a/y.txt")).unwrap(),
            "lowercase directory"
        );
        workspace.persist().await.unwrap();
    }

    for (key, original, content) in [
        (uppercase_file, uppercase_original, "uppercase directory"),
        (lowercase_file, lowercase_original, "lowercase directory"),
    ] {
        let stored = objects.get(&key).await.unwrap();
        assert_eq!(stored.meta.e_tag, original.e_tag);
        assert_eq!(stored.meta.version, original.version);
        assert_eq!(stored.bytes().await.unwrap(), content.as_bytes());
    }
    for renamed in [
        prefix.clone().join("A").join("y.txt"),
        prefix.clone().join("a").join("x.txt"),
    ] {
        assert!(matches!(
            objects.get(&renamed).await,
            Err(object_store::Error::NotFound { .. })
        ));
    }
    let lock = prefix.clone().join(".git").join("flow-like-store.lock");
    assert!(matches!(
        objects.head(&lock).await,
        Err(object_store::Error::NotFound { .. })
    ));
}
