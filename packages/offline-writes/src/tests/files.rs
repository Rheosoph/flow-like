use super::manager::{TestHost, desktop_options, seed};
use crate::{
    files::{FileBuffering, FileOverlay, FileOverlayOptions, FileRoute},
    fs,
    host::{Connectivity, Observation},
    manager::WriteManager,
};
use async_trait::async_trait;
use bytes::Bytes;
use flow_like_device_protocol::{OfflineExpected, OfflineReplayRequest, StoragePurpose};
use flow_like_storage::object_store::{
    self, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    ObjectStoreExt, PutMode, PutMultipartOptions, PutOptions, PutPayload, PutResult,
    memory::InMemory, path::Path as ObjectPath,
};
use flow_like_types::authorization::AuthorizationError;
use futures_util::stream::BoxStream;
use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

/// An in-memory cloud that can go offline, stall puts and hide revisions.
#[derive(Debug, Default)]
pub(super) struct Cloud {
    pub(super) memory: InMemory,
    pub(super) offline: AtomicBool,
    pub(super) put_delay_ms: AtomicU64,
    pub(super) puts: AtomicUsize,
    pub(super) strip_revisions: AtomicBool,
}
impl fmt::Display for Cloud {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("offline-writes-test-cloud")
    }
}
impl Cloud {
    fn check(&self) -> object_store::Result<()> {
        if self.offline.load(Ordering::SeqCst) {
            Err(object_store::Error::Generic {
                store: "test-cloud",
                source: Box::new(AuthorizationError::Unavailable),
            })
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl ObjectStore for Cloud {
    async fn put_opts(
        &self,
        path: &ObjectPath,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        let delay = self.put_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        self.check()?;
        self.memory.put_opts(path, payload, options).await
    }
    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.check()?;
        self.memory.put_multipart_opts(path, options).await
    }
    async fn get_opts(
        &self,
        path: &ObjectPath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.check()?;
        let mut result = self.memory.get_opts(path, options).await?;
        if self.strip_revisions.load(Ordering::SeqCst) {
            result.meta.e_tag = None;
            result.meta.version = None;
        }
        Ok(result)
    }
    fn delete_stream(
        &self,
        paths: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        if let Err(error) = self.check() {
            return Box::pin(futures_util::stream::once(async { Err(error) }));
        }
        self.memory.delete_stream(paths)
    }
    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        if let Err(error) = self.check() {
            return Box::pin(futures_util::stream::once(async { Err(error) }));
        }
        self.memory.list(prefix)
    }
    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        self.check()?;
        self.memory.list_with_delimiter(prefix).await
    }
    async fn copy_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        self.check()?;
        self.memory.copy_opts(from, to, options).await
    }
}

#[derive(Default)]
pub(super) struct Breaker {
    pub(super) offline: AtomicBool,
    pub(super) observed: Mutex<Vec<Observation>>,
}
impl Connectivity for Breaker {
    fn is_offline(&self) -> bool {
        self.offline.load(Ordering::SeqCst)
    }
    fn observe(&self, observation: Observation) {
        self.observed.lock().unwrap().push(observation);
    }
}

pub(super) fn path(file: &str) -> ObjectPath {
    ObjectPath::from(format!("apps/project/upload/exports/{file}"))
}

pub(super) struct Files {
    pub(super) manager: Arc<WriteManager>,
    pub(super) store: Arc<FileOverlay>,
    pub(super) cloud: Arc<Cloud>,
    pub(super) breaker: Arc<Breaker>,
}

pub(super) fn when_offline(breaker: Arc<Breaker>, settle: Duration) -> FileBuffering {
    FileBuffering::WhenOffline {
        connectivity: breaker,
        direct_write_base: Duration::from_millis(200),
        direct_write_min_bytes_per_second: 1000,
        pending_settle_timeout: settle,
    }
}

pub(super) async fn files_with(
    root: &std::path::Path,
    host: Arc<TestHost>,
    scheme: &str,
    buffering: impl FnOnce(Arc<Breaker>) -> FileBuffering,
) -> Files {
    let manager = WriteManager::open(desktop_options(root), host)
        .await
        .unwrap();
    let cloud = Arc::new(Cloud::default());
    let breaker = Arc::new(Breaker::default());
    let store = manager
        .file_overlay(
            cloud.clone(),
            FileOverlayOptions {
                routes: vec![FileRoute {
                    purpose: StoragePurpose::Files,
                    root: "apps/project/upload/".into(),
                    prefix: "exports/".into(),
                    scheme: scheme.into(),
                }],
                buffering: buffering(breaker.clone()),
                max_file_bytes: 1024 * 1024,
                offline_error: Arc::new(|error| {
                    fs::is_offline_error(error, &|_| false).then_some(Observation::ConnectFailed)
                }),
            },
        )
        .unwrap();
    Files {
        manager,
        store,
        cloud,
        breaker,
    }
}

async fn files(root: &std::path::Path, scheme: &str, settle: Duration) -> Files {
    let cloud = tempfile::tempdir().unwrap();
    let remote = seed(cloud.path()).await.unwrap();
    files_with(root, TestHost::new(remote, None), scheme, |breaker| {
        when_offline(breaker, settle)
    })
    .await
}

async fn body(store: &dyn ObjectStore, path: &ObjectPath) -> Bytes {
    store.get(path).await.unwrap().bytes().await.unwrap()
}

async fn create(
    store: &dyn ObjectStore,
    path: &ObjectPath,
    data: &'static str,
) -> object_store::Result<PutResult> {
    store
        .put_opts(
            path,
            Bytes::from_static(data.as_bytes()).into(),
            PutOptions {
                mode: PutMode::Create,
                ..Default::default()
            },
        )
        .await
}

fn pending(files: &Files) -> u64 {
    files.manager.queue().status().unwrap().pending_count
}

#[tokio::test]
async fn when_offline_mode_writes_directly_while_online_and_queues_after_connect_errors() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path(), "az", Duration::from_secs(1)).await;
    let online = path("online.txt");
    files
        .store
        .put(&online, Bytes::from_static(b"direct").into())
        .await
        .unwrap();
    assert_eq!(body(&files.cloud.memory, &online).await, "direct");
    assert_eq!(pending(&files), 0);
    files.cloud.offline.store(true, Ordering::SeqCst);
    let failed = path("failed.txt");
    let receipt = files
        .store
        .put(&failed, Bytes::from_static(b"queued").into())
        .await
        .unwrap();
    assert!(receipt.e_tag.unwrap().starts_with("offline-"));
    assert_eq!(pending(&files), 1);
    assert_eq!(
        *files.breaker.observed.lock().unwrap(),
        vec![Observation::Succeeded, Observation::ConnectFailed]
    );
    let request: OfflineReplayRequest =
        serde_json::from_value(files.manager.queue().head().unwrap().unwrap().payload).unwrap();
    assert!(matches!(request.expected, OfflineExpected::FileAbsent));
    files.breaker.offline.store(true, Ordering::SeqCst);
    let puts = files.cloud.puts.load(Ordering::SeqCst);
    files
        .store
        .put(&path("breaker.txt"), Bytes::from_static(b"queued").into())
        .await
        .unwrap();
    assert_eq!(files.cloud.puts.load(Ordering::SeqCst), puts);
    assert_eq!(pending(&files), 2);
    assert_eq!(body(files.store.as_ref(), &failed).await, "queued");
}

#[tokio::test]
async fn when_offline_mode_timeouts_queue_and_scale_with_payload_size() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path(), "az", Duration::from_secs(1)).await;
    files.cloud.put_delay_ms.store(400, Ordering::SeqCst);
    let small = path("small.txt");
    files
        .store
        .put(&small, Bytes::from_static(b"tiny").into())
        .await
        .unwrap();
    assert_eq!(pending(&files), 1);
    assert_eq!(
        *files.breaker.observed.lock().unwrap(),
        vec![Observation::TimedOut]
    );
    let large = path("large.bin");
    files
        .store
        .put(&large, Bytes::from(vec![7u8; 1000]).into())
        .await
        .unwrap();
    assert_eq!(pending(&files), 1);
    assert_eq!(body(&files.cloud.memory, &large).await.len(), 1000);
}

#[tokio::test]
async fn when_offline_mode_routes_paths_with_pending_ops_through_the_queue_online() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path(), "az", Duration::from_secs(1)).await;
    let file = path("report.txt");
    files.breaker.offline.store(true, Ordering::SeqCst);
    files
        .store
        .put(&file, Bytes::from_static(b"first").into())
        .await
        .unwrap();
    files.breaker.offline.store(false, Ordering::SeqCst);
    files
        .store
        .put(&file, Bytes::from_static(b"second").into())
        .await
        .unwrap();
    assert_eq!(pending(&files), 1);
    assert!(files.cloud.memory.head(&file).await.is_err());
    assert_eq!(body(files.store.as_ref(), &file).await, "second");
    files.store.delete(&file).await.unwrap();
    assert_eq!(pending(&files), 1);
    assert!(matches!(
        files.store.get(&file).await,
        Err(object_store::Error::NotFound { .. })
    ));
    assert_eq!(files.cloud.puts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn in_flight_path_waits_for_settle_then_writes_directly() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path(), "s3", Duration::from_secs(5)).await;
    let file = path("upload.txt");
    files.breaker.offline.store(true, Ordering::SeqCst);
    files
        .store
        .put(&file, Bytes::from_static(b"offline").into())
        .await
        .unwrap();
    files.breaker.offline.store(false, Ordering::SeqCst);
    let head = files.manager.queue().head().unwrap().unwrap();
    files
        .manager
        .queue()
        .prepare_attempt(&head.operation_id, &head.payload)
        .unwrap();
    let manager = files.manager.clone();
    let id = head.operation_id.clone();
    let acknowledged = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        manager
            .queue()
            .acknowledge(
                &id,
                &serde_json::to_value(OfflineExpected::FileRevision {
                    e_tag: Some("cloud".into()),
                    version: None,
                })
                .unwrap(),
                &serde_json::json!({"applied": true}),
            )
            .unwrap();
    });
    files
        .store
        .put(&file, Bytes::from_static(b"online").into())
        .await
        .unwrap();
    acknowledged.await.unwrap();
    assert_eq!(body(&files.cloud.memory, &file).await, "online");
    assert_eq!(pending(&files), 0);

    let root = tempfile::tempdir().unwrap();
    let files = self::files(root.path(), "s3", Duration::from_millis(100)).await;
    files.breaker.offline.store(true, Ordering::SeqCst);
    files
        .store
        .put(&file, Bytes::from_static(b"offline").into())
        .await
        .unwrap();
    files.breaker.offline.store(false, Ordering::SeqCst);
    let head = files.manager.queue().head().unwrap().unwrap();
    files
        .manager
        .queue()
        .prepare_attempt(&head.operation_id, &head.payload)
        .unwrap();
    let error = files
        .store
        .put(&file, Bytes::from_static(b"online").into())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains(&format!(
        "'{file}' is still uploading from an earlier offline change. Try again in a moment."
    )));
}

#[tokio::test]
async fn when_offline_mode_allows_copy_rename_online_and_rejects_them_offline() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path(), "az", Duration::from_secs(1)).await;
    let source = path("source.txt");
    files
        .store
        .put(&source, Bytes::from_static(b"content").into())
        .await
        .unwrap();
    files.store.copy(&source, &path("copy.txt")).await.unwrap();
    files
        .store
        .rename(&path("copy.txt"), &path("renamed.txt"))
        .await
        .unwrap();
    assert_eq!(
        body(&files.cloud.memory, &path("renamed.txt")).await,
        "content"
    );
    files.breaker.offline.store(true, Ordering::SeqCst);
    assert!(
        files
            .store
            .copy(&source, &path("offline-copy.txt"))
            .await
            .unwrap_err()
            .to_string()
            .contains(
                "Copy involving buffered files requires an explicit read followed by a write"
            )
    );
    assert!(
        files
            .store
            .rename(&source, &path("offline-rename.txt"))
            .await
            .unwrap_err()
            .to_string()
            .contains("Rename involving buffered files is not an atomic offline operation")
    );
}

#[tokio::test]
async fn when_offline_mode_streams_multipart_online_and_buffers_it_offline() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path(), "az", Duration::from_secs(1)).await;
    let online = path("online.bin");
    let mut upload = files.store.put_multipart(&online).await.unwrap();
    upload
        .put_part(Bytes::from(vec![1u8; 32]).into())
        .await
        .unwrap();
    upload.complete().await.unwrap();
    assert_eq!(body(&files.cloud.memory, &online).await.len(), 32);
    assert_eq!(pending(&files), 0);
    files.breaker.offline.store(true, Ordering::SeqCst);
    let offline = path("offline.bin");
    let mut upload = files.store.put_multipart(&offline).await.unwrap();
    upload
        .put_part(Bytes::from(vec![2u8; 16]).into())
        .await
        .unwrap();
    upload.complete().await.unwrap();
    assert_eq!(pending(&files), 1);
    assert_eq!(body(files.store.as_ref(), &offline).await.len(), 16);
}

#[tokio::test]
async fn explicit_create_of_unknown_path_fails_offline_and_overwrite_queues_a_create() {
    for always in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let cloud = tempfile::tempdir().unwrap();
        let remote = seed(cloud.path()).await.unwrap();
        let files = files_with(root.path(), TestHost::new(remote, None), "az", |breaker| {
            if always {
                FileBuffering::Always
            } else {
                when_offline(breaker, Duration::from_secs(1))
            }
        })
        .await;
        files.breaker.offline.store(true, Ordering::SeqCst);
        files.cloud.offline.store(true, Ordering::SeqCst);
        let file = path("unknown.txt");
        let error = create(files.store.as_ref(), &file, "created")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!(
            "'{file}' may already exist in the cloud, and this device cannot check while offline. Creating it only if absent needs a connection to the hub."
        )));
        assert_eq!(pending(&files), 0);
        files
            .store
            .put(&file, Bytes::from_static(b"created").into())
            .await
            .unwrap();
        let request: OfflineReplayRequest =
            serde_json::from_value(files.manager.queue().head().unwrap().unwrap().payload).unwrap();
        assert!(matches!(request.expected, OfflineExpected::FileAbsent));
    }
}

#[tokio::test]
async fn table_format_commits_pass_through_online_and_fail_offline() {
    for always in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let cloud = tempfile::tempdir().unwrap();
        let remote = seed(cloud.path()).await.unwrap();
        let files = files_with(root.path(), TestHost::new(remote, None), "az", |breaker| {
            if always {
                FileBuffering::Always
            } else {
                when_offline(breaker, Duration::from_secs(1))
            }
        })
        .await;
        let online = path("events/_delta_log/00000000000000000000.json");
        create(files.store.as_ref(), &online, "commit")
            .await
            .unwrap();
        assert_eq!(body(&files.cloud.memory, &online).await, "commit");
        files.breaker.offline.store(true, Ordering::SeqCst);
        files.cloud.offline.store(true, Ordering::SeqCst);
        let puts = files.cloud.puts.load(Ordering::SeqCst);
        for commit in [
            path("events/_delta_log/00000000000000000001.json"),
            path("iceberg/metadata/v3.metadata.json"),
            path("iceberg/metadata/version-hint.text"),
            path("hudi/.hoodie/20260924.commit"),
        ] {
            let error = create(files.store.as_ref(), &commit, "commit")
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!(
                "'{commit}' belongs to a table format commit log (Delta, Iceberg or Hudi). These commits need a connection to the hub."
            )));
        }
        let expected_puts = if always { puts + 4 } else { puts };
        assert_eq!(files.cloud.puts.load(Ordering::SeqCst), expected_puts);
        assert_eq!(pending(&files), 0);
    }
}

#[tokio::test]
async fn unknown_cloud_revision_is_rejected_with_its_own_message() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path(), "az", Duration::from_secs(1)).await;
    let file = path("known.txt");
    files
        .store
        .put(&file, Bytes::from_static(b"cloud").into())
        .await
        .unwrap();
    files.cloud.strip_revisions.store(true, Ordering::SeqCst);
    files.breaker.offline.store(true, Ordering::SeqCst);
    let error = files
        .store
        .put(&file, Bytes::from_static(b"replacement").into())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains(&format!(
        "This device does not know the cloud revision of '{file}', so it cannot be replaced or deleted offline. Write a new file name instead."
    )));
    assert_eq!(pending(&files), 0);
}

#[tokio::test]
async fn pending_reports_queued_puts_deletes_and_digest_checked_bytes() {
    let root = tempfile::tempdir().unwrap();
    let files = files(root.path(), "az", Duration::from_secs(1)).await;
    let file = path("pending.txt");
    assert_eq!(
        files.store.pending(&file).unwrap(),
        crate::files::PendingFile::None
    );
    files.breaker.offline.store(true, Ordering::SeqCst);
    files
        .store
        .put(&file, Bytes::from_static(b"bytes").into())
        .await
        .unwrap();
    assert!(matches!(
        files.store.pending(&file).unwrap(),
        crate::files::PendingFile::Put { meta } if meta.size == 5
    ));
    assert_eq!(files.store.pending_bytes(&file).unwrap().unwrap(), "bytes");
    let id = files.manager.queue().head().unwrap().unwrap().operation_id;
    let (_, payload) = files.manager.file_payload(&id).unwrap().unwrap();
    assert_eq!(payload, "bytes");
    files.store.delete(&file).await.unwrap();
    assert_eq!(
        files.store.pending(&file).unwrap(),
        crate::files::PendingFile::Deleted
    );
    assert!(files.store.pending_bytes(&file).unwrap().is_none());
}
