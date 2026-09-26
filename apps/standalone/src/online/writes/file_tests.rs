use super::*;
use crate::{
    online::cache::{CacheControl, ReadCache},
    outbox::{BufferingConfig, Outbox},
};
use async_trait::async_trait;
use bytes::Bytes;
use flow_like_device_protocol::{
    INSTANCE_OFFLINE_LIMITS, OfflineExpected, OfflineMutation, OfflineReplayRequest, StoragePurpose,
};
use flow_like_offline_writes::{FileOverlay, FileOverlayParts};
use flow_like_storage::object_store::{
    self, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
    ObjectStoreExt, PutMode, PutMultipartOptions, PutOptions, PutPayload, PutResult, UpdateVersion,
    memory::InMemory, path::Path as ObjectPath,
};
use flow_like_types_contracts::authorization::AuthorizationError;
use futures_util::{StreamExt, TryStreamExt, stream::BoxStream};
use std::{
    fmt,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Mutex, Notify};

fn error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> object_store::Error {
    object_store::Error::Generic {
        store: "DeviceOfflineFiles",
        source: error.into(),
    }
}

#[derive(Debug, Default)]
struct Cloud {
    memory: InMemory,
    offline: AtomicBool,
}
impl fmt::Display for Cloud {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("file-overlay-test-cloud")
    }
}
impl Cloud {
    fn check(&self) -> object_store::Result<()> {
        if self.offline.load(Ordering::SeqCst) {
            Err(error(AuthorizationError::Unavailable))
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
        let version = options.version.clone();
        let mut result = self.memory.get_opts(path, options).await?;
        result.meta.version = version;
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

fn fixture(root: &Path, cloud: Arc<Cloud>, scheme: &str) -> (FileOverlay, Arc<CacheControl>) {
    let queue = Arc::new(
        Outbox::open(
            root,
            "placement",
            &"a".repeat(64),
            BufferingConfig::default(),
        )
        .unwrap(),
    );
    let cache = CacheControl::new(
        root,
        "file-tests",
        vec!["apps/project/upload/".into()],
        16 * 1024 * 1024,
    )
    .unwrap();
    let authorized_queue = queue.clone();
    let authorized_cache = cache.clone();
    let store = FileOverlay::for_tests(FileOverlayParts {
        inner: ReadCache::new(cloud, cache.clone()),
        queue,
        authorize: Arc::new(move || {
            if authorized_cache.is_revoked() {
                authorized_queue.quarantine("Revoked test authorization")?;
            }
            authorized_queue.check_authorized()
        }),
        gate: Arc::new(Mutex::new(())),
        wake: Arc::new(Notify::new()),
        replay_limits: INSTANCE_OFFLINE_LIMITS,
        options: FileOverlayOptions {
            routes: vec![FileRoute {
                purpose: StoragePurpose::Files,
                root: "apps/project/upload/".into(),
                prefix: "exports/".into(),
                scheme: scheme.into(),
            }],
            buffering: FileBuffering::Always,
            max_file_bytes: MAX_OFFLINE_OPERATION_BYTES,
            offline_error: offline_error(),
        },
    });
    (store, cache)
}
fn path(file: &str) -> ObjectPath {
    ObjectPath::from(format!("apps/project/upload/exports/{file}"))
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

#[tokio::test]
async fn outage_overwrites_and_deletes_are_durable_visible_and_coalesced() {
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let original = path("record.json");
    cloud
        .put(&original, Bytes::from_static(b"old").into())
        .await
        .unwrap();
    let (store, _) = fixture(root.path(), cloud.clone(), "az");
    assert_eq!(body(&store, &original).await, "old");
    let prefix = ObjectPath::from("apps/project/upload/exports");
    assert_eq!(
        store
            .list(Some(&prefix))
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .len(),
        1
    );
    cloud.offline.store(true, Ordering::SeqCst);
    store
        .put(&original, Bytes::from_static(b"first").into())
        .await
        .unwrap();
    store
        .put(&original, Bytes::from_static(b"latest").into())
        .await
        .unwrap();
    assert_eq!(store.queue.status().unwrap().pending_count, 1);
    assert_eq!(body(&store, &original).await, "latest");
    let (restarted, _) = fixture(root.path(), cloud.clone(), "az");
    assert_eq!(body(&restarted, &original).await, "latest");
    let listing = restarted
        .list(Some(&prefix))
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].size, 6);
    restarted.delete(&original).await.unwrap();
    assert!(matches!(
        restarted.get(&original).await,
        Err(object_store::Error::NotFound { .. })
    ));
    assert!(
        restarted
            .list(Some(&prefix))
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(restarted.queue.status().unwrap().pending_count, 1);
    assert_eq!(body(&cloud.memory, &original).await, "old");
}

#[tokio::test]
async fn conditional_versions_conflicts_and_attempted_writes_keep_queue_order() {
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let file = path("new.json");
    let (store, _) = fixture(root.path(), cloud.clone(), "az");
    cloud.offline.store(true, Ordering::SeqCst);
    let receipt = store
        .put(&file, Bytes::from_static(b"first").into())
        .await
        .unwrap();
    let mismatch = PutOptions {
        mode: PutMode::Update(UpdateVersion {
            e_tag: Some("stale".into()),
            version: None,
        }),
        ..Default::default()
    };
    assert!(matches!(
        store
            .put_opts(&file, Bytes::from_static(b"wrong").into(), mismatch)
            .await,
        Err(object_store::Error::Precondition { .. })
    ));
    let head = store.queue.head().unwrap().unwrap();
    store
        .queue
        .prepare_attempt(&head.operation_id, &head.payload)
        .unwrap();
    let update = PutOptions {
        mode: PutMode::Update(UpdateVersion {
            e_tag: receipt.e_tag,
            version: receipt.version,
        }),
        ..Default::default()
    };
    store
        .put_opts(&file, Bytes::from_static(b"second").into(), update)
        .await
        .unwrap();
    assert_eq!(store.queue.status().unwrap().pending_count, 2);
    store
        .queue
        .block(
            &head.operation_id,
            "conflict",
            "Cloud file was created elsewhere",
        )
        .unwrap();
    assert_eq!(body(&store, &file).await, "second");
    let status = store.queue.status().unwrap();
    assert_eq!(status.head.unwrap().state, "conflict");
    assert_eq!(status.pending_count, 2);
}

#[tokio::test]
async fn s3_rejects_existing_mutations_but_compacts_new_file_cancellation() {
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let existing = path("existing");
    cloud
        .put(&existing, Bytes::from_static(b"old").into())
        .await
        .unwrap();
    let (store, _) = fixture(root.path(), cloud.clone(), "s3");
    assert!(matches!(
        store
            .put(&existing, Bytes::from_static(b"new").into())
            .await,
        Err(object_store::Error::NotSupported { .. })
    ));
    assert!(matches!(
        store.delete(&existing).await,
        Err(object_store::Error::NotSupported { .. })
    ));
    let new = path("new");
    cloud.offline.store(true, Ordering::SeqCst);
    assert!(
        create(&store, &new, "new")
            .await
            .unwrap_err()
            .to_string()
            .contains("may already exist in the cloud")
    );
    store
        .put(&new, Bytes::from_static(b"new").into())
        .await
        .unwrap();
    store.delete(&new).await.unwrap();
    let head = store.queue.head().unwrap().unwrap();
    let request: OfflineReplayRequest = serde_json::from_value(head.payload).unwrap();
    assert!(matches!(request.expected, OfflineExpected::FileAbsent));
    assert!(matches!(request.mutation, OfflineMutation::FileDelete));
    assert_eq!(store.queue.status().unwrap().pending_count, 1);
    store
        .queue
        .prepare_attempt(&head.operation_id, &serde_json::to_value(request).unwrap())
        .unwrap();
    assert!(matches!(
        create(&store, &new, "later").await,
        Err(object_store::Error::NotSupported { .. })
    ));
}

#[tokio::test]
async fn pending_streams_stop_after_revocation_and_outside_scope_never_queues() {
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let (store, cache) = fixture(root.path(), cloud.clone(), "az");
    cloud.offline.store(true, Ordering::SeqCst);
    let outside = ObjectPath::from("apps/project/upload/exports-neighbor/private");
    assert!(create(&store, &outside, "outside").await.is_err());
    assert_eq!(store.queue.status().unwrap().pending_count, 0);
    assert!(
        store
            .delete(&path("unknown"))
            .await
            .unwrap_err()
            .to_string()
            .contains("offline")
    );
    let file = path("new");
    store
        .put(&file, Bytes::from_static(b"sensitive").into())
        .await
        .unwrap();
    let stream = store.get(&file).await.unwrap();
    cache.revoke().unwrap();
    assert!(stream.bytes().await.is_err());
    assert!(store.queue.status().unwrap().quarantined);
    assert!(store.get(&file).await.is_err());
}

#[tokio::test]
async fn acknowledged_bytes_replace_stale_cached_bytes_before_queue_release() {
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let file = path("acknowledged");
    cloud
        .put(&file, Bytes::from_static(b"old").into())
        .await
        .unwrap();
    let (store, cache) = fixture(root.path(), cloud.clone(), "az");
    assert_eq!(body(&store, &file).await, "old");
    let prefix = ObjectPath::from("apps/project/upload/exports");
    assert_eq!(
        store
            .list(Some(&prefix))
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .len(),
        1
    );
    store
        .put(&file, Bytes::from_static(b"new").into())
        .await
        .unwrap();
    let head = store.queue.head().unwrap().unwrap();
    store
        .queue
        .prepare_attempt(&head.operation_id, &head.payload)
        .unwrap();
    let receipt = cloud
        .put(&file, Bytes::from_static(b"new").into())
        .await
        .unwrap();
    let meta = cloud.head(&file).await.unwrap();
    cache.remember_file(&meta, b"new").unwrap();
    let revision = OfflineExpected::FileRevision {
        e_tag: receipt.e_tag,
        version: receipt.version,
    };
    store
        .queue
        .acknowledge(
            &head.operation_id,
            &serde_json::to_value(revision).unwrap(),
            &serde_json::json!({"applied":true}),
        )
        .unwrap();
    assert_eq!(store.queue.status().unwrap().pending_count, 0);
    cloud.offline.store(true, Ordering::SeqCst);
    let (restarted, _) = fixture(root.path(), cloud, "az");
    assert_eq!(body(&restarted, &file).await, "new");
    let listing = restarted
        .list(Some(&prefix))
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].e_tag, meta.e_tag);
    cache.remember_file_deleted(&file).unwrap();
    assert!(
        restarted
            .list(Some(&prefix))
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .is_empty()
    );
    let cold = ObjectPath::from("apps/project/upload/exports/cold");
    assert!(
        restarted
            .list(Some(&cold))
            .try_collect::<Vec<_>>()
            .await
            .is_err()
    );
}

#[tokio::test]
async fn multipart_is_bounded_and_lance_paths_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let (store, _) = fixture(root.path(), cloud, "az");
    assert!(matches!(
        create(&store, &path("table.lance/data/file"), "bad").await,
        Err(object_store::Error::NotSupported { .. })
    ));
    let mut upload = store.put_multipart(&path("large")).await.unwrap();
    upload
        .put_part(Bytes::from(vec![0; MAX_OFFLINE_OPERATION_BYTES]).into())
        .await
        .unwrap();
    assert!(
        upload
            .put_part(Bytes::from_static(b"overflow").into())
            .await
            .is_err()
    );
    assert!(upload.complete().await.is_err());
    assert_eq!(store.queue.status().unwrap().pending_count, 0);
}

#[tokio::test]
async fn pending_file_reads_enforce_ranges_conditions_and_delimiter_boundaries() {
    use flow_like_storage::object_store::GetRange;
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let (store, _) = fixture(root.path(), cloud, "az");
    let file = path("nested/report");
    let receipt = create(&store, &file, "contents").await.unwrap();
    let result = store
        .get_opts(
            &file,
            GetOptions {
                range: Some(GetRange::Bounded(2..100)),
                version: receipt.version.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result.range, 2..8);
    assert_eq!(result.bytes().await.unwrap(), "ntents");
    let result = store
        .get_opts(
            &file,
            GetOptions {
                range: Some(GetRange::Suffix(3)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result.bytes().await.unwrap(), "nts");
    assert!(
        store
            .get_opts(
                &file,
                GetOptions {
                    range: Some(GetRange::Offset(8)),
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
    assert!(matches!(
        store
            .get_opts(
                &file,
                GetOptions {
                    if_match: Some("stale".into()),
                    ..Default::default()
                }
            )
            .await,
        Err(object_store::Error::Precondition { .. })
    ));
    assert!(matches!(
        store
            .get_opts(
                &file,
                GetOptions {
                    if_none_match: receipt.e_tag,
                    ..Default::default()
                }
            )
            .await,
        Err(object_store::Error::NotModified { .. })
    ));
    let result = store
        .get_opts(
            &file,
            GetOptions {
                head: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result.meta.size, 8);
    assert!(result.bytes().await.unwrap().is_empty());
    let prefix = ObjectPath::from("apps/project/upload/exports");
    let listing = store.list_with_delimiter(Some(&prefix)).await.unwrap();
    assert!(listing.objects.is_empty());
    assert_eq!(listing.common_prefixes, [path("nested")]);
    let outside = ObjectPath::from("apps/project/upload/exports-neighbor");
    assert!(
        store
            .list(Some(&outside))
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn explicit_cloud_versions_remain_readable_through_pending_local_deletes() {
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let file = path("historical");
    cloud
        .put(&file, Bytes::from_static(b"cloud-version").into())
        .await
        .unwrap();
    let (store, _) = fixture(root.path(), cloud, "az");
    store
        .put(&file, Bytes::from_static(b"local").into())
        .await
        .unwrap();
    assert_eq!(body(&store, &file).await, "local");
    let historical = GetOptions {
        version: Some("cloud-generation-1".into()),
        ..Default::default()
    };
    assert_eq!(
        store
            .get_opts(&file, historical.clone())
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        "cloud-version"
    );
    store.delete(&file).await.unwrap();
    assert_eq!(
        store
            .get_opts(&file, historical)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        "cloud-version"
    );
    assert!(matches!(
        store.get(&file).await,
        Err(object_store::Error::NotFound { .. })
    ));
}

#[tokio::test]
async fn pending_file_listing_stops_between_items_after_revocation() {
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let (store, cache) = fixture(root.path(), cloud, "az");
    create(&store, &path("first"), "one").await.unwrap();
    create(&store, &path("second"), "two").await.unwrap();
    let prefix = ObjectPath::from("apps/project/upload/exports");
    let mut listing = store.list(Some(&prefix));
    assert!(listing.next().await.unwrap().is_ok());
    cache.revoke().unwrap();
    assert!(listing.next().await.unwrap().is_err());
    assert!(listing.next().await.is_none());
    assert!(store.queue.status().unwrap().quarantined);
}

#[tokio::test]
async fn explicit_create_of_unknown_path_fails_offline_with_e21() {
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let (store, _) = fixture(root.path(), cloud.clone(), "az");
    let file = path("unknown.json");
    cloud.offline.store(true, Ordering::SeqCst);
    let error = create(&store, &file, "created")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains(&format!(
        "'{file}' may already exist in the cloud, and this device cannot check while offline. Creating it only if absent needs a connection to the hub."
    )));
    assert_eq!(store.queue.status().unwrap().pending_count, 0);
    store
        .put(&file, Bytes::from_static(b"created").into())
        .await
        .unwrap();
    let request: OfflineReplayRequest =
        serde_json::from_value(store.queue.head().unwrap().unwrap().payload).unwrap();
    assert!(matches!(request.expected, OfflineExpected::FileAbsent));
    assert!(matches!(
        create(&store, &file, "again").await,
        Err(object_store::Error::AlreadyExists { .. })
    ));
}

#[tokio::test]
async fn table_format_commits_pass_through_online_and_fail_offline_with_e22() {
    let root = tempfile::tempdir().unwrap();
    let cloud = Arc::new(Cloud::default());
    let (store, _) = fixture(root.path(), cloud.clone(), "az");
    let commit = path("events/_delta_log/00000000000000000000.json");
    create(&store, &commit, "commit").await.unwrap();
    assert_eq!(body(&cloud.memory, &commit).await, "commit");
    assert_eq!(store.queue.status().unwrap().pending_count, 0);
    cloud.offline.store(true, Ordering::SeqCst);
    for commit in [
        path("events/_delta_log/00000000000000000001.json"),
        path("iceberg/metadata/v2.metadata.json"),
        path("iceberg/metadata/version-hint.text"),
        path("hudi/.hoodie/20260924.commit"),
    ] {
        let error = create(&store, &commit, "commit")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!(
            "'{commit}' belongs to a table format commit log (Delta, Iceberg or Hudi). These commits need a connection to the hub."
        )));
    }
    assert_eq!(store.queue.status().unwrap().pending_count, 0);
    store
        .put(
            &path("events/part-0001.parquet"),
            Bytes::from_static(b"data").into(),
        )
        .await
        .unwrap();
    assert_eq!(store.queue.status().unwrap().pending_count, 1);
}
