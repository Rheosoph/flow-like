use super::files::{Breaker, files_with, path as file_path, when_offline};
use crate::{
    BufferedTable, BufferingConfig,
    fs::unix_time,
    host::{OfflineHost, ReplayError, ReplayErrorKind},
    manager::{
        FastForward, RefreshOutcome, TableActivation, TableSetup, WriteManager, WriteManagerOptions,
    },
    outbox::QueueLanes,
    table::{self, TableOverlay, optional_table, replay_mutation},
};
use anyhow::{Context, Result};
use flow_like_device_protocol::{
    DESKTOP_OFFLINE_LIMITS, MAX_OFFLINE_OPERATION_BYTES, OfflineExpected, OfflineLimits,
    OfflineMutation, OfflineReplayRequest, OfflineReplayResponse, OfflineReplayStatus,
    OfflineResource, StoragePurpose, format_limit, request_wire_bytes,
};
use flow_like_storage::{
    databases::vector::{
        lancedb::{
            DatabaseSelector, LanceDBVectorStore, LogicalTableMutation, LogicalTableMutationAdapter,
        },
        offline_replay::{self, ReplayMarker, ReplayOutcome},
    },
    lancedb::{self, Connection, Table, query::Select},
    object_store::{ObjectStoreExt, path::Path as ObjectPath},
};
use flow_like_types::authorization::AuthorizationError;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

/// How the test hub answers file replays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileReplay {
    Apply,
    Reject,
    Unavailable,
}

#[derive(Clone)]
pub(super) struct ReplayServer {
    pub(super) cloud: Connection,
    pub(super) unavailable: Arc<AtomicBool>,
    pub(super) lose_ack: Arc<AtomicBool>,
    pub(super) not_claimed: Arc<AtomicBool>,
    pub(super) files: Arc<Mutex<FileReplay>>,
}

pub(super) struct TestHost {
    pub(super) remote: Connection,
    pub(super) server: Option<ReplayServer>,
    pub(super) revoked: AtomicBool,
    /// Held by a test to pause `remote_table`.
    pub(super) hold: tokio::sync::Mutex<()>,
    pub(super) queue_changes: AtomicUsize,
    pub(super) discarded: Mutex<Vec<(String, String)>>,
}

impl TestHost {
    pub(super) fn new(remote: Connection, server: Option<ReplayServer>) -> Arc<Self> {
        Arc::new(Self {
            remote,
            server,
            revoked: AtomicBool::new(false),
            hold: tokio::sync::Mutex::new(()),
            queue_changes: AtomicUsize::new(0),
            discarded: Mutex::new(Vec::new()),
        })
    }
}

fn unavailable(message: impl Into<String>) -> ReplayError {
    ReplayError {
        kind: ReplayErrorKind::Unavailable,
        code: None,
        message: message.into(),
    }
}

#[async_trait::async_trait]
impl OfflineHost for TestHost {
    fn authorization_current(&self) -> Result<(), AuthorizationError> {
        if self.revoked.load(Ordering::Acquire) {
            Err(AuthorizationError::Denied)
        } else {
            Ok(())
        }
    }
    async fn replay(
        &self,
        request: &OfflineReplayRequest,
    ) -> Result<OfflineReplayResponse, ReplayError> {
        let Some(state) = &self.server else {
            return Err(unavailable("Replay endpoint is unreachable"));
        };
        if state.unavailable.load(Ordering::Acquire) {
            return Err(unavailable("Replay endpoint is unavailable"));
        }
        if state.not_claimed.load(Ordering::Acquire) {
            return Err(ReplayError {
                kind: ReplayErrorKind::NotClaimed,
                code: Some("subject_mismatch".into()),
                message: "The signed-in account changed".into(),
            });
        }
        if let OfflineResource::File { .. } = &request.resource {
            return file_endpoint(*state.files.lock().unwrap(), request);
        }
        let response = replay_endpoint(state, request)
            .await
            .map_err(|error| unavailable(error.to_string()))?;
        if state.lose_ack.swap(false, Ordering::AcqRel) {
            return Err(unavailable("Replay acknowledgement was lost"));
        }
        Ok(response)
    }
    fn location_prefix(&self, purpose: StoragePurpose) -> Option<String> {
        match purpose {
            StoragePurpose::Storage => Some("apps/project/storage/".into()),
            StoragePurpose::Files => Some("apps/project/upload/".into()),
            _ => None,
        }
    }
    async fn remote_table(&self, table: &BufferedTable) -> Result<Option<Table>> {
        let _held = self.hold.lock().await;
        optional_table(&self.remote, &table.table).await
    }
    async fn remote_table_names(&self, _database: &ObjectPath) -> Result<Vec<String>> {
        Ok(self.remote.table_names().execute().await?)
    }
    fn file_acknowledged(
        &self,
        _path: &ObjectPath,
        _operation_id: &str,
        _bytes: &[u8],
        _revision: &OfflineExpected,
    ) -> Result<()> {
        Ok(())
    }
    fn file_deleted(&self, _path: &ObjectPath, _operation_id: &str) -> Result<()> {
        Ok(())
    }
    fn file_discarded(&self, path: &ObjectPath, operation_id: &str) -> Result<()> {
        self.discarded
            .lock()
            .unwrap()
            .push((path.to_string(), operation_id.into()));
        Ok(())
    }
    fn queue_changed(&self) {
        self.queue_changes.fetch_add(1, Ordering::AcqRel);
    }
}

fn file_endpoint(
    mode: FileReplay,
    request: &OfflineReplayRequest,
) -> Result<OfflineReplayResponse, ReplayError> {
    let result = match (mode, &request.mutation) {
        (FileReplay::Unavailable, _) => return Err(unavailable("File storage is unavailable")),
        (FileReplay::Reject, _) => {
            return Err(ReplayError {
                kind: ReplayErrorKind::Rejected,
                code: Some("forbidden".into()),
                message: "Writing this file is not allowed".into(),
            });
        }
        (FileReplay::Apply, OfflineMutation::FileDelete) => OfflineExpected::FileAbsent,
        (FileReplay::Apply, _) => OfflineExpected::FileRevision {
            e_tag: Some("cloud-etag".into()),
            version: None,
        },
    };
    Ok(OfflineReplayResponse {
        operation_id: request.operation_id.clone(),
        digest: request
            .digest()
            .map_err(|error| unavailable(error.to_string()))?,
        status: OfflineReplayStatus::Applied,
        result: Some(result),
        message: None,
    })
}

async fn replay_endpoint(
    state: &ReplayServer,
    request: &OfflineReplayRequest,
) -> Result<OfflineReplayResponse> {
    request.validate()?;
    let OfflineExpected::TableVersion {
        version,
        fingerprint,
    } = &request.expected
    else {
        anyhow::bail!("Expected table mutation")
    };
    let marker = ReplayMarker {
        operation_id: request.operation_id.clone(),
        digest: request.digest()?,
        expected_version: *version,
        expected_fingerprint: fingerprint.clone(),
    };
    let outcome = if *version == 0 {
        let items = match &request.mutation {
            OfflineMutation::TableInsert { rows } | OfflineMutation::TableUpsert { rows, .. } => {
                rows.clone()
            }
            _ => anyhow::bail!("Absent tables require a schema-bearing create"),
        };
        offline_replay::create(&state.cloud, "measurements", &marker, items).await?
    } else {
        let table = state.cloud.open_table("measurements").execute().await?;
        offline_replay::replay(&table, &marker, replay_mutation(request.mutation.clone())?).await?
    };
    let (status, result) = match outcome {
        ReplayOutcome::Applied {
            version,
            fingerprint,
        } => (
            OfflineReplayStatus::Applied,
            Some(OfflineExpected::TableVersion {
                version,
                fingerprint: Some(fingerprint),
            }),
        ),
        ReplayOutcome::Conflict { .. } => (OfflineReplayStatus::Conflict, None),
        _ => (OfflineReplayStatus::OutcomeUnknown, None),
    };
    Ok(OfflineReplayResponse {
        operation_id: request.operation_id.clone(),
        digest: request.digest()?,
        status,
        result,
        message: None,
    })
}

pub(super) fn selection() -> BufferedTable {
    BufferedTable {
        purpose: StoragePurpose::Storage,
        database: "db".into(),
        table: "measurements".into(),
        primary_key: "id".into(),
    }
}

pub(super) fn active() -> TableSetup {
    TableSetup {
        activation: TableActivation::Active,
        validate_key: false,
        prefetch: false,
    }
}

pub(super) fn options(root: &Path) -> WriteManagerOptions {
    WriteManagerOptions::standalone(
        root.to_path_buf(),
        "placement".into(),
        "a".repeat(64),
        BufferingConfig {
            max_mirror_bytes: 32 * 1024 * 1024,
            ..BufferingConfig::default()
        },
    )
}

/// The desktop's queue options: lanes, coalescing and no re-creation of deleted tables.
pub(super) fn desktop_options(root: &Path) -> WriteManagerOptions {
    WriteManagerOptions {
        lanes: QueueLanes::PerResource,
        coalesce_row_batches: true,
        recreate_dropped_tables: false,
        quarantine_other_scopes: false,
        ..options(root)
    }
}

pub(super) async fn open_table(
    options: WriteManagerOptions,
    host: Arc<TestHost>,
    setup: TableSetup,
) -> Result<(Arc<WriteManager>, Arc<TableOverlay>)> {
    let writer = WriteManager::open(options, host).await?;
    writer.add_table(selection(), setup).await?;
    let table = writer
        .overlay(&resource_key(&selection()))
        .context("Offline table was not registered")?;
    Ok((writer, table))
}

pub(super) fn resource_key(table: &BufferedTable) -> String {
    serde_json::to_string(&OfflineResource::Table {
        purpose: table.purpose,
        database: table.database.clone(),
        table: table.table.clone(),
    })
    .unwrap()
}

async fn manager(
    root: &Path,
    remote: Connection,
    server: Option<ReplayServer>,
) -> Result<(Arc<WriteManager>, Arc<TableOverlay>)> {
    manager_with_host(root, TestHost::new(remote, server)).await
}

async fn manager_with_host(
    root: &Path,
    host: Arc<TestHost>,
) -> Result<(Arc<WriteManager>, Arc<TableOverlay>)> {
    open_table(options(root), host, active()).await
}

async fn rows(table: &Table, filter: &str, select: Select) -> Result<Vec<Value>> {
    table::rows(table, filter, select, MAX_OFFLINE_OPERATION_BYTES).await
}

pub(super) async fn seed(root: &Path) -> Result<Connection> {
    let db = lancedb::connect(root.to_str().unwrap()).execute().await?;
    db.create_table(
        "measurements",
        flow_like_storage::arrow_utils::value_to_batch_reader(vec![
            json!({"id":1,"value":10}),
            json!({"id":2,"value":20}),
        ])?,
    )
    .execute()
    .await?;
    Ok(db)
}

pub(super) fn replay_server(cloud: Connection, unavailable: bool, lose_ack: bool) -> ReplayServer {
    ReplayServer {
        cloud,
        unavailable: Arc::new(AtomicBool::new(unavailable)),
        lose_ack: Arc::new(AtomicBool::new(lose_ack)),
        not_claimed: Arc::new(AtomicBool::new(false)),
        files: Arc::new(Mutex::new(FileReplay::Apply)),
    }
}

#[tokio::test]
async fn frozen_updates_preserve_numeric_keys_and_quoted_columns() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = lancedb::connect(cloud.path().to_str().unwrap())
        .execute()
        .await?;
    remote
        .create_table(
            "measurements",
            flow_like_storage::arrow_utils::value_to_batch_reader(vec![json!({
                "id": 1,
                "value": 10,
                "sensor label": "north",
                "select": 17,
                "counter": u64::MAX
            })])?,
        )
        .execute()
        .await?;
    let (writer, table) = manager(root.path(), remote, None).await?;
    table
        .apply(LogicalTableMutation::Update {
            filter: "id = 1".into(),
            updates: vec![("value".into(), "0".into())],
        })
        .await?;
    let request: OfflineReplayRequest =
        serde_json::from_value(writer.queue.head()?.unwrap().payload)?;
    let OfflineMutation::TableUpsert { rows: frozen, .. } = request.mutation else {
        anyhow::bail!("Update was not frozen to complete rows")
    };
    assert_eq!(
        frozen,
        vec![json!({
            "id": 1,
            "value": 0,
            "sensor label": "north",
            "select": 17,
            "counter": u64::MAX
        })]
    );
    assert_eq!(
        rows(&table.read_table().await?.unwrap(), "true", Select::All).await?,
        frozen
    );
    table
        .apply(LogicalTableMutation::Delete {
            filter: "id = 1".into(),
        })
        .await?;
    assert_eq!(
        table.read_table().await?.unwrap().count_rows(None).await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn offline_update_then_delete_compacts_and_reads_survive_restart() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let (writer, table) = manager(root.path(), remote.clone(), None).await?;
    table
        .apply(LogicalTableMutation::Update {
            filter: "id = 1".into(),
            updates: vec![("value".into(), "value + 5".into())],
        })
        .await?;
    assert_eq!(
        rows(&table.read_table().await?.unwrap(), "id = 1", Select::All).await?[0]["value"],
        15
    );
    table
        .apply(LogicalTableMutation::Delete {
            filter: "id = 1".into(),
        })
        .await?;
    assert_eq!(writer.queue.status()?.pending_count, 1);
    assert_eq!(
        table.read_table().await?.unwrap().count_rows(None).await?,
        1
    );
    assert_eq!(
        remote
            .open_table("measurements")
            .execute()
            .await?
            .count_rows(None)
            .await?,
        2
    );
    drop(table);
    drop(writer);
    let (writer, table) = manager(root.path(), remote, None).await?;
    assert_eq!(
        table.read_table().await?.unwrap().count_rows(None).await?,
        1
    );
    assert_eq!(writer.queue.status()?.pending_count, 1);
    Ok(())
}

#[tokio::test]
async fn skip_rebuilds_local_branch_without_the_blocked_update() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let (writer, table) = manager(root.path(), seed(cloud.path()).await?, None).await?;
    let first = table
        .apply(LogicalTableMutation::Upsert {
            id_field: "id".into(),
            items: vec![json!({"id":1,"value":99})],
        })
        .await?;
    table
        .apply(LogicalTableMutation::Upsert {
            id_field: "id".into(),
            items: vec![json!({"id":2,"value":42})],
        })
        .await?;
    writer.queue.block(
        &first.operation_id,
        "conflict",
        "External writer advanced the table",
    )?;
    writer.queue.request_skip(
        &first.operation_id,
        "Operator discarded the conflicted update",
        false,
    )?;
    table
        .skip(
            &writer,
            &first.operation_id,
            "Operator discarded the conflicted update",
        )
        .await?;
    let local = table.read_table().await?.unwrap();
    assert_eq!(rows(&local, "id = 1", Select::All).await?[0]["value"], 10);
    assert_eq!(rows(&local, "id = 2", Select::All).await?[0]["value"], 42);
    assert_eq!(writer.queue.status()?.pending_count, 1);
    Ok(())
}

#[tokio::test]
async fn empty_updates_are_not_queued_and_revocation_locks_the_overlay() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let host = TestHost::new(seed(cloud.path()).await?, None);
    let (writer, table) = manager_with_host(root.path(), host.clone()).await?;
    assert_eq!(
        table
            .apply(LogicalTableMutation::Update {
                filter: "id = 404".into(),
                updates: vec![("value".into(), "0".into())]
            })
            .await?
            .state,
        "unchanged"
    );
    assert_eq!(writer.queue.status()?.pending_count, 0);
    table
        .apply(LogicalTableMutation::Delete {
            filter: "id = 1".into(),
        })
        .await?;
    host.revoked.store(true, Ordering::Release);
    assert!(table.read_table().await.is_err());
    assert_eq!(writer.queue.status()?.pending_count, 1);
    assert!(writer.queue.status()?.quarantined);
    Ok(())
}

#[tokio::test]
async fn absent_table_creation_survives_a_following_upsert_or_delete() -> Result<()> {
    for delete in [false, true] {
        let root = tempfile::tempdir()?;
        let cloud = tempfile::tempdir()?;
        let remote = lancedb::connect(cloud.path().to_str().unwrap())
            .execute()
            .await?;
        let state = replay_server(remote.clone(), false, false);
        let (writer, table) = manager(root.path(), remote.clone(), Some(state)).await?;
        let first = table
            .apply(LogicalTableMutation::Upsert {
                items: vec![json!({"id":1,"value":10})],
                id_field: "id".into(),
            })
            .await?;
        let second = if delete {
            LogicalTableMutation::Delete {
                filter: "id = 1".into(),
            }
        } else {
            LogicalTableMutation::Upsert {
                items: vec![json!({"id":1,"value":20})],
                id_field: "id".into(),
            }
        };
        table.apply(second).await?;
        assert_eq!(writer.queue.status()?.pending_count, 2);
        assert_eq!(
            writer.queue.head()?.unwrap().operation_id,
            first.operation_id
        );
        assert!(writer.drain_one().await?);
        let created = remote.open_table("measurements").execute().await?;
        assert_eq!(rows(&created, "true", Select::All).await?[0]["value"], 10);
        assert!(writer.drain_one().await?);
        assert_eq!(writer.queue.status()?.pending_count, 0);
        let final_table = remote.open_table("measurements").execute().await?;
        let remaining = rows(&final_table, "true", Select::All).await?;
        if delete {
            assert!(remaining.is_empty());
        } else {
            assert_eq!(remaining[0]["value"], 20);
        }
    }
    Ok(())
}

#[tokio::test]
async fn offline_replay_recovers_lost_ack_after_restart_without_duplicate_effect() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let state = replay_server(remote.clone(), true, true);
    let (writer, table) = manager(root.path(), remote.clone(), Some(state.clone())).await?;
    table
        .apply(LogicalTableMutation::Insert {
            items: vec![json!({"id":3,"value":30})],
        })
        .await?;
    assert!(writer.drain_one().await.is_err());
    assert_eq!(
        table.read_table().await?.unwrap().count_rows(None).await?,
        3
    );
    state.unavailable.store(false, Ordering::Release);
    assert!(writer.drain_one().await.is_err());
    assert_eq!(
        remote
            .open_table("measurements")
            .execute()
            .await?
            .count_rows(None)
            .await?,
        3
    );
    drop(table);
    drop(writer);
    let (writer, table) = manager(root.path(), remote.clone(), Some(state)).await?;
    assert!(writer.drain_one().await?);
    assert_eq!(writer.queue.status()?.pending_count, 0);
    assert_eq!(
        remote
            .open_table("measurements")
            .execute()
            .await?
            .count_rows(None)
            .await?,
        3
    );
    assert_eq!(
        table.read_table().await?.unwrap().count_rows(None).await?,
        3
    );
    Ok(())
}

#[tokio::test]
async fn idle_snapshot_refresh_observes_cloud_updates_and_table_deletion() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let (writer, table) = manager(root.path(), remote.clone(), None).await?;
    let source = remote.open_table("measurements").execute().await?;
    source
        .update()
        .only_if("id = 1")
        .column("value", "50")
        .execute()
        .await?;
    source.delete("id = 2").await?;

    table.refresh_idle(&writer).await?;
    let local = table
        .read_table()
        .await?
        .context("refreshed snapshot is missing")?;
    assert_eq!(local.count_rows(None).await?, 1);
    assert_eq!(rows(&local, "id = 1", Select::All).await?[0]["value"], 50);
    assert_eq!(writer.queue.status()?.pending_count, 0);

    remote.drop_table("measurements", &[]).await?;
    table.last_refresh.store(0, Ordering::Release);
    table.refresh_idle(&writer).await?;
    assert!(table.read_table().await?.is_none());
    assert_eq!(
        writer.queue.resource_revision(&table.key)?,
        Some(serde_json::to_value(OfflineExpected::TableVersion {
            version: 0,
            fingerprint: None
        })?)
    );
    Ok(())
}

#[tokio::test]
async fn failed_local_schema_recovery_is_visible_without_disclosing_payload() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let (writer, table) = manager(root.path(), remote, None).await?;
    let request = OfflineReplayRequest {
        operation_id: uuid::Uuid::new_v4().to_string(),
        resource: table.resource.clone(),
        expected: serde_json::from_value(writer.queue.resource_revision(&table.key)?.unwrap())?,
        mutation: OfflineMutation::TableInsert {
            rows: vec![json!({"id":3,"value":{"private":"secret-offline-payload"}})],
        },
    };
    let payload = serde_json::to_value(&request)?;
    let operation = writer
        .queue
        .enqueue(&table.key, payload.clone(), None, unix_time()?)?;
    assert!(table.recover_local(&writer).await.is_err());
    let retained = writer.queue.head()?.unwrap();
    assert_eq!(retained.operation_id, operation.operation_id);
    assert_eq!(retained.state, "blocked");
    assert_eq!(retained.payload, payload);
    assert!(
        retained
            .error
            .as_deref()
            .is_some_and(|message| message.contains("disk budget and table schema"))
    );
    let status = writer.queue.status()?;
    assert!(status.head.as_ref().unwrap().payload.is_null());
    assert!(!serde_json::to_string(&status)?.contains("secret-offline-payload"));
    Ok(())
}

#[tokio::test]
async fn local_commit_before_sqlite_marker_is_recovered_without_reapplying() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let (writer, table) = manager(root.path(), remote.clone(), None).await?;
    let request = OfflineReplayRequest {
        operation_id: uuid::Uuid::new_v4().to_string(),
        resource: table.resource.clone(),
        expected: serde_json::from_value(writer.queue.resource_revision(&table.key)?.unwrap())?,
        mutation: OfflineMutation::TableInsert {
            rows: vec![json!({"id": 3, "value": 30})],
        },
    };
    let operation = writer.queue.enqueue(
        &table.key,
        serde_json::to_value(&request)?,
        None,
        unix_time()?,
    )?;
    let local = table.local_table(&writer).await?.unwrap();
    let (version, fingerprint) = offline_replay::revision(&local).await?;
    let expected = OfflineExpected::TableVersion {
        version,
        fingerprint: Some(fingerprint.clone()),
    };
    writer
        .queue
        .prepare_local(&operation.operation_id, &serde_json::to_value(expected)?)?;
    let marker = ReplayMarker {
        operation_id: operation.operation_id.clone(),
        digest: request.digest()?,
        expected_version: version,
        expected_fingerprint: Some(fingerprint),
    };
    let applied =
        offline_replay::replay(&local, &marker, replay_mutation(request.mutation)?).await?;
    let ReplayOutcome::Applied {
        version: applied_version,
        ..
    } = applied
    else {
        anyhow::bail!("local commit was not applied")
    };
    assert!(writer.queue.next_unmaterialized(&table.key)?.is_some());

    // Simulate process loss after the durable Lance commit and before mark_local.
    drop(local);
    drop(table);
    drop(writer);
    let (writer, table) = manager(root.path(), remote.clone(), None).await?;
    let recovered = table.read_table().await?.unwrap();
    assert_eq!(recovered.count_rows(None).await?, 3);
    assert_eq!(
        offline_replay::revision(&recovered).await?.0,
        applied_version
    );
    assert_eq!(
        writer.queue.head()?.unwrap().local_version,
        Some(applied_version)
    );
    assert!(writer.queue.next_unmaterialized(&table.key)?.is_none());
    assert_eq!(
        remote
            .open_table("measurements")
            .execute()
            .await?
            .count_rows(None)
            .await?,
        2
    );
    Ok(())
}

fn database_path() -> ObjectPath {
    ObjectPath::from("apps/project/storage/db")
}

async fn insert(table: &TableOverlay, id: i64, value: i64) -> Result<String> {
    Ok(table
        .apply(LogicalTableMutation::Insert {
            items: vec![json!({"id": id, "value": value})],
        })
        .await?
        .operation_id)
}

async fn cloud_rows(remote: &Connection) -> Result<Vec<Value>> {
    rows(
        &remote.open_table("measurements").execute().await?,
        "true",
        Select::All,
    )
    .await
}

fn store_for(connection: &Connection) -> Result<LanceDBVectorStore> {
    LanceDBVectorStore::from_connection_for_overlay(
        connection.clone(),
        "measurements".into(),
        DatabaseSelector::default(),
    )
}

fn head_request(writer: &WriteManager) -> Result<OfflineReplayRequest> {
    Ok(serde_json::from_value(
        writer.queue.head()?.context("queue is empty")?.payload,
    )?)
}

fn lance_tables(writer: &WriteManager) -> Result<Vec<String>> {
    Ok(std::fs::read_dir(writer.root().join("tables"))?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|name| name.ends_with(".lance"))
        .collect())
}

fn limits() -> BufferingConfig {
    BufferingConfig {
        max_mirror_bytes: 32 * 1024 * 1024,
        ..BufferingConfig::default()
    }
}

#[tokio::test]
async fn frozen_requests_use_validate_with_before_enqueue_and_dispatch() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let host = TestHost::new(seed(cloud.path()).await?, None);
    let (writer, table) = open_table(desktop_options(root.path()), host, active()).await?;
    insert(&table, 3, 30).await?;
    let wire = request_wire_bytes(&head_request(&writer)?)?;
    let at = |bytes| OfflineLimits {
        max_request_bytes: Some(bytes),
        ..DESKTOP_OFFLINE_LIMITS
    };
    writer.set_limits(limits(), at(wire)).await?;
    insert(&table, 4, 40).await?;
    writer.set_limits(limits(), at(wire - 1)).await?;
    assert_eq!(
        insert(&table, 5, 50).await.unwrap_err().to_string(),
        format!(
            "Offline change exceeds the hub's sync limit of {} once encoded; split the logical batch",
            format_limit(wire - 1)
        )
    );
    assert_eq!(writer.queue.status()?.pending_count, 2);
    assert_eq!(
        table.read_table().await?.unwrap().count_rows(None).await?,
        4
    );
    Ok(())
}

#[tokio::test]
async fn lowered_limits_block_the_head_as_unclaimed_hub_limit() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let host = TestHost::new(
        remote.clone(),
        Some(replay_server(remote.clone(), false, false)),
    );
    let (writer, table) = open_table(desktop_options(root.path()), host, active()).await?;
    let id = insert(&table, 3, 30).await?;
    let lowered = OfflineLimits {
        max_request_bytes: Some(100),
        ..DESKTOP_OFFLINE_LIMITS
    };
    writer.set_limits(limits(), lowered).await?;
    assert!(!writer.drain_once().await?);
    let head = writer.queue.head()?.unwrap();
    assert_eq!((head.state.as_str(), head.attempts), ("blocked", 0));
    let lookup = writer.queue.operation_state(&id)?.unwrap();
    assert_eq!(lookup.error_code.as_deref(), Some("hub_limit"));
    assert_eq!(
        lookup.error.unwrap(),
        format!(
            "Offline change exceeds the hub's sync limit of {} once encoded; split the logical batch",
            format_limit(100)
        )
    );
    writer.queue.retry(&id)?;
    writer.set_limits(limits(), DESKTOP_OFFLINE_LIMITS).await?;
    assert!(writer.drain_once().await?);
    assert_eq!(cloud_rows(&remote).await?.len(), 3);
    Ok(())
}

#[tokio::test]
async fn not_claimed_rejection_resets_attempts_and_skip_needs_no_acknowledgement() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let server = replay_server(remote.clone(), false, false);
    server.not_claimed.store(true, Ordering::Release);
    let host = TestHost::new(remote.clone(), Some(server));
    let (writer, table) = open_table(desktop_options(root.path()), host, active()).await?;
    let id = insert(&table, 3, 30).await?;
    assert!(writer.drain_once().await.is_err());
    let head = writer.queue.head()?.unwrap();
    assert_eq!((head.state.as_str(), head.attempts), ("blocked", 0));
    assert_eq!(
        writer
            .queue
            .operation_state(&id)?
            .unwrap()
            .error_code
            .as_deref(),
        Some("subject_mismatch")
    );
    writer
        .queue
        .request_skip(&id, "The account changed", false)?;
    assert!(!writer.drain_once().await?);
    assert_eq!(writer.queue.status()?.pending_count, 0);
    assert_eq!(
        table.read_table().await?.unwrap().count_rows(None).await?,
        2
    );
    Ok(())
}

#[tokio::test]
async fn blocked_and_unavailable_lanes_do_not_stop_other_lanes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let server = replay_server(remote.clone(), false, false);
    *server.files.lock().unwrap() = FileReplay::Unavailable;
    let host = TestHost::new(remote.clone(), Some(server.clone()));
    let files = files_with(root.path(), host, "az", |breaker| {
        when_offline(breaker, Duration::from_secs(1))
    })
    .await;
    let writer = files.manager.clone();
    writer.add_table(selection(), active()).await?;
    let table = writer.overlay(&resource_key(&selection())).unwrap();
    files.breaker.offline.store(true, Ordering::SeqCst);
    files
        .store
        .put(&file_path("a.txt"), bytes::Bytes::from_static(b"a").into())
        .await?;
    insert(&table, 3, 30).await?;
    assert!(writer.drain_once().await.is_err());
    assert!(writer.drain_once().await?);
    assert!(!writer.drain_once().await?);
    *server.files.lock().unwrap() = FileReplay::Reject;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(writer.drain_once().await.is_err());
    let blocked = writer.queue.blocked_heads(10)?;
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].error_code.as_deref(), Some("forbidden"));
    insert(&table, 4, 40).await?;
    assert!(writer.drain_once().await?);
    assert_eq!(cloud_rows(&remote).await?.len(), 4);
    Ok(())
}

#[tokio::test]
async fn adjacent_batches_coalesce_at_dispatch_and_report_superseded() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let host = TestHost::new(
        remote.clone(),
        Some(replay_server(remote.clone(), false, false)),
    );
    let (writer, table) = open_table(desktop_options(root.path()), host, active()).await?;
    let first = insert(&table, 3, 30).await?;
    let second = insert(&table, 4, 40).await?;
    let upsert = table
        .apply(LogicalTableMutation::Upsert {
            id_field: "id".into(),
            items: vec![json!({"id": 1, "value": 5}), json!({"id": 2, "value": 5})],
        })
        .await?
        .operation_id;
    let latest = table
        .apply(LogicalTableMutation::Upsert {
            id_field: "id".into(),
            items: vec![json!({"id": 1, "value": 6})],
        })
        .await?
        .operation_id;
    assert!(writer.drain_once().await?);
    assert_eq!(cloud_rows(&remote).await?.len(), 4);
    let merged = writer.queue.operation_state(&second)?.unwrap();
    assert_eq!(merged.state, "superseded");
    assert_eq!(merged.superseded_by.as_deref(), Some(first.as_str()));
    assert_eq!(
        writer.queue.operation_state(&first)?.unwrap().state,
        "applied"
    );
    assert!(writer.drain_once().await?);
    assert_eq!(
        writer
            .queue
            .operation_state(&latest)?
            .unwrap()
            .superseded_by,
        Some(upsert)
    );
    let cloud = remote.open_table("measurements").execute().await?;
    assert_eq!(rows(&cloud, "id = 1", Select::All).await?[0]["value"], 6);
    assert_eq!(rows(&cloud, "id = 2", Select::All).await?[0]["value"], 5);
    assert_eq!(writer.queue.status()?.pending_count, 0);
    assert!(!writer.drain_once().await?);
    Ok(())
}

#[tokio::test]
async fn coalesced_head_skip_rebuilds_without_merged_rows() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let host = TestHost::new(
        remote.clone(),
        Some(replay_server(remote.clone(), true, false)),
    );
    let (writer, table) = open_table(desktop_options(root.path()), host, active()).await?;
    let first = insert(&table, 3, 30).await?;
    let second = insert(&table, 4, 40).await?;
    assert!(writer.drain_once().await.is_err());
    assert_eq!(
        writer.queue.operation_state(&second)?.unwrap().state,
        "superseded"
    );
    writer
        .queue
        .block(&first, "conflict", "A foreign commit changed the table")?;
    writer
        .queue
        .request_skip(&first, "Discard the merged inserts", true)?;
    assert!(!writer.drain_once().await?);
    let local = table.read_table().await?.unwrap();
    assert_eq!(local.count_rows(None).await?, 2);
    assert!(rows(&local, "id = 4", Select::All).await?.is_empty());
    assert_eq!(writer.queue.status()?.pending_count, 0);
    Ok(())
}

#[tokio::test]
async fn fast_forward_refreshes_an_idle_table_before_freezing() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let breaker = Arc::new(Breaker::default());
    let options = WriteManagerOptions {
        fast_forward: Some(FastForward {
            probe_timeout: Duration::from_secs(10),
            min_interval: Duration::ZERO,
        }),
        connectivity: Some(breaker),
        ..desktop_options(root.path())
    };
    let (writer, table) =
        open_table(options, TestHost::new(remote.clone(), None), active()).await?;
    let source = remote.open_table("measurements").execute().await?;
    source
        .update()
        .only_if("id = 1")
        .column("value", "50")
        .execute()
        .await?;
    table
        .apply(LogicalTableMutation::Update {
            filter: "id = 1".into(),
            updates: vec![("value".into(), "value + 1".into())],
        })
        .await?;
    let request = head_request(&writer)?;
    let OfflineMutation::TableUpsert { rows: frozen, .. } = request.mutation else {
        anyhow::bail!("update was not frozen")
    };
    assert_eq!(frozen[0]["value"], 51);
    let source = remote.open_table("measurements").execute().await?;
    let (version, fingerprint) = offline_replay::revision(&source).await?;
    assert_eq!(
        request.expected,
        OfflineExpected::TableVersion {
            version,
            fingerprint: Some(fingerprint)
        }
    );
    Ok(())
}

#[tokio::test]
async fn held_tables_reject_until_activated_and_activation_refreshes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let held = TableSetup {
        activation: TableActivation::Held,
        ..active()
    };
    let (writer, table) = open_table(
        desktop_options(root.path()),
        TestHost::new(remote.clone(), None),
        held,
    )
    .await?;
    let expected = "Table 'measurements' is being prepared for offline use on this device. Try again when the flows of this project that were already running have finished and the hub is reachable.";
    assert!(writer.table_is_managed(&database_path(), "measurements"));
    assert!(writer.managed_table_names(&database_path()).is_empty());
    assert_eq!(
        writer
            .decorate(&database_path(), store_for(&remote)?)
            .await
            .map(|_| ())
            .unwrap_err()
            .to_string(),
        expected
    );
    assert_eq!(
        insert(&table, 3, 30).await.unwrap_err().to_string(),
        expected
    );
    let source = remote.open_table("measurements").execute().await?;
    source
        .update()
        .only_if("id = 1")
        .column("value", "50")
        .execute()
        .await?;
    let state = writer.activate_table(&selection()).await?;
    assert_eq!(state.activation, Some(TableActivation::Active));
    let local = table.read_table().await?.unwrap();
    assert_eq!(rows(&local, "id = 1", Select::All).await?[0]["value"], 50);
    assert!(
        writer
            .decorate(&database_path(), store_for(&remote)?)
            .await?
            .is_durably_managed()
    );
    assert_eq!(
        writer.managed_table_names(&database_path()),
        vec!["measurements".to_string()]
    );
    Ok(())
}

#[tokio::test]
async fn key_validation_runs_before_registration_so_hosted_runs_stay_unmanaged() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = lancedb::connect(cloud.path().to_str().unwrap())
        .execute()
        .await?;
    remote
        .create_table(
            "measurements",
            flow_like_storage::arrow_utils::value_to_batch_reader(vec![
                json!({"id": 1, "value": 1}),
                json!({"id": 1, "value": 2}),
                json!({"id": null, "value": 3}),
                json!({"id": 2, "value": 4}),
            ])?,
        )
        .execute()
        .await?;
    let host = TestHost::new(remote.clone(), None);
    let writer = WriteManager::open(desktop_options(root.path()), host.clone()).await?;
    let paused = host.hold.lock().await;
    let adding = {
        let writer = writer.clone();
        tokio::spawn(async move {
            writer
                .add_table(
                    selection(),
                    TableSetup {
                        activation: TableActivation::Held,
                        validate_key: true,
                        prefetch: false,
                    },
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!writer.table_is_managed(&database_path(), "measurements"));
    assert!(
        !writer
            .decorate(&database_path(), store_for(&remote)?)
            .await?
            .is_durably_managed()
    );
    drop(paused);
    assert_eq!(
        adding.await?.unwrap_err().to_string(),
        "Column 'id' is not a unique, non-empty key in 'measurements': 3 rows are empty or duplicated."
    );
    assert!(!writer.table_is_managed(&database_path(), "measurements"));
    assert!(
        writer
            .queue
            .resource_revision(&resource_key(&selection()))?
            .is_none()
    );
    assert_eq!(lance_tables(&writer)?, Vec::<String>::new());
    let cloud = tempfile::tempdir()?;
    let other = tempfile::tempdir()?;
    let valid = WriteManager::open(
        desktop_options(other.path()),
        TestHost::new(seed(cloud.path()).await?, None),
    )
    .await?;
    valid
        .add_table(
            selection(),
            TableSetup {
                activation: TableActivation::Held,
                validate_key: true,
                prefetch: false,
            },
        )
        .await?;
    assert!(valid.table_is_managed(&database_path(), "measurements"));
    Ok(())
}

#[tokio::test]
async fn key_validation_spills_instead_of_holding_every_key_in_memory() -> Result<()> {
    use flow_like_storage::{
        arrow_array::{Int64Array, RecordBatch, RecordBatchIterator, RecordBatchReader},
        arrow_schema::{DataType, Field, Schema},
    };
    let cloud = tempfile::tempdir()?;
    let spill = tempfile::tempdir()?;
    let remote = lancedb::connect(cloud.path().to_str().unwrap())
        .execute()
        .await?;
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
    let keys = (0..1_000_000i64).chain([7]).collect::<Vec<_>>();
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(keys))])?;
    remote
        .create_table(
            "big",
            Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema))
                as Box<dyn RecordBatchReader + Send>,
        )
        .execute()
        .await?;
    let table = remote.open_table("big").execute().await?;
    let version = table.version().await?;
    assert_eq!(
        table::validate_key(table, version, "big", "id", spill.path(), 8 * 1024 * 1024)
            .await
            .unwrap_err()
            .to_string(),
        "Column 'id' is not a unique, non-empty key in 'big': 2 rows are empty or duplicated."
    );
    assert!(std::fs::read_dir(spill.path())?.next().is_none());
    Ok(())
}

#[tokio::test]
async fn remove_table_refuses_pending_work_and_drops_the_mirror_when_idle() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let host = TestHost::new(
        remote.clone(),
        Some(replay_server(remote.clone(), false, false)),
    );
    let (writer, table) = open_table(desktop_options(root.path()), host, active()).await?;
    insert(&table, 3, 30).await?;
    assert_eq!(
        writer
            .remove_table(&selection())
            .await
            .unwrap_err()
            .to_string(),
        "Table 'measurements' still has queued changes"
    );
    assert!(writer.drain_once().await?);
    writer.remove_table(&selection()).await?;
    assert!(!writer.table_is_managed(&database_path(), "measurements"));
    assert!(writer.queue.resource_revision(&table.key)?.is_none());
    assert_eq!(lance_tables(&writer)?, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn removed_overlay_rejects_stale_handles() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let (writer, table) = open_table(
        desktop_options(root.path()),
        TestHost::new(remote, None),
        active(),
    )
    .await?;
    writer.remove_table(&selection()).await?;
    let expected = "Offline access for table 'measurements' was turned off while this run was using it. Start the run again.";
    assert_eq!(table.read_table().await.unwrap_err().to_string(), expected);
    assert_eq!(
        insert(&table, 3, 30).await.unwrap_err().to_string(),
        expected
    );
    Ok(())
}

#[tokio::test]
async fn remote_deletion_marks_the_table_missing_without_recreating_it() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let (writer, _table) = open_table(
        desktop_options(root.path()),
        TestHost::new(remote.clone(), None),
        active(),
    )
    .await?;
    let revision = writer
        .queue
        .resource_revision(&resource_key(&selection()))?;
    remote.drop_table("measurements", &[]).await?;
    assert_eq!(
        writer.refresh_table(&selection()).await?,
        RefreshOutcome::RemoteMissing
    );
    let expected = "Table 'measurements' was deleted in the cloud. Turn off offline access for it on this device; its queued changes cannot be applied.";
    let state = writer.table_states().await?.remove(0);
    assert!(state.remote_missing);
    assert_eq!(state.mirror_error.as_deref(), Some(expected));
    assert_eq!(
        writer
            .queue
            .resource_revision(&resource_key(&selection()))?,
        revision
    );
    assert_eq!(
        writer
            .managed_store(
                &database_path(),
                "measurements",
                DatabaseSelector::default()
            )
            .await
            .map(|_| ())
            .unwrap_err()
            .to_string(),
        expected
    );
    drop(state);
    drop(_table);
    drop(writer);
    let (writer, _) = open_table(
        desktop_options(root.path()),
        TestHost::new(remote, None),
        active(),
    )
    .await?;
    assert!(writer.table_states().await?[0].remote_missing);
    Ok(())
}

#[tokio::test]
async fn retired_snapshots_are_pruned_after_the_grace() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let options = WriteManagerOptions {
        retired_snapshot_grace: Some(Duration::ZERO),
        ..desktop_options(root.path())
    };
    let (writer, _table) =
        open_table(options, TestHost::new(remote.clone(), None), active()).await?;
    let key = resource_key(&selection());
    let old = writer.queue.local_view(&key)?.0.unwrap();
    let old_directory = writer.root().join("tables").join(format!("{old}.lance"));
    assert!(old_directory.exists());
    remote
        .open_table("measurements")
        .execute()
        .await?
        .delete("id = 2")
        .await?;
    assert!(matches!(
        writer.refresh_table(&selection()).await?,
        RefreshOutcome::Refreshed { .. }
    ));
    assert!(old_directory.exists());
    writer.idle_pass().await;
    assert!(!old_directory.exists());
    assert!(writer.queue.retired_tables()?.is_empty());
    Ok(())
}

#[tokio::test]
async fn set_limits_applies_without_restart() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let (writer, table) = open_table(
        desktop_options(root.path()),
        TestHost::new(seed(cloud.path()).await?, None),
        active(),
    )
    .await?;
    let small = OfflineLimits {
        max_operation_bytes: 64,
        ..DESKTOP_OFFLINE_LIMITS
    };
    writer.set_limits(limits(), small).await?;
    let large = LogicalTableMutation::Insert {
        items: vec![json!({"id": 3, "value": 30, "note": "x".repeat(100)})],
    };
    let error = table.apply(large).await.unwrap_err();
    assert!(format!("{error:#}").contains(&format!(
        "Offline mutation exceeds {}; split the logical batch",
        format_limit(64)
    )));
    writer
        .set_limits(
            BufferingConfig {
                max_operations: 1,
                ..limits()
            },
            DESKTOP_OFFLINE_LIMITS,
        )
        .await?;
    insert(&table, 3, 30).await?;
    assert_eq!(
        insert(&table, 4, 40).await.unwrap_err().to_string(),
        "Offline queue operation limit reached"
    );
    assert!(
        writer
            .set_limits(
                BufferingConfig {
                    max_operations: 0,
                    ..limits()
                },
                DESKTOP_OFFLINE_LIMITS,
            )
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn close_releases_files_and_stops_the_drain() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let (writer, table) = open_table(
        desktop_options(root.path()),
        TestHost::new(remote.clone(), None),
        active(),
    )
    .await?;
    insert(&table, 3, 30).await?;
    writer.spawn_drain();
    writer.close().await?;
    let closed = "Offline changes for this project were removed from this device";
    assert_eq!(writer.drain_once().await.unwrap_err().to_string(), closed);
    assert_eq!(writer.queue.status().unwrap_err().to_string(), closed);
    assert!(table.read_table().await.is_err());
    writer.close().await?;
    let (reopened, _) = open_table(
        desktop_options(root.path()),
        TestHost::new(remote, None),
        active(),
    )
    .await?;
    assert_eq!(reopened.queue.status()?.pending_count, 1);
    Ok(())
}

#[tokio::test]
async fn file_discarded_is_called_on_skip() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let server = replay_server(remote.clone(), false, false);
    *server.files.lock().unwrap() = FileReplay::Reject;
    let host = TestHost::new(remote, Some(server));
    let files = files_with(root.path(), host.clone(), "az", |breaker| {
        when_offline(breaker, Duration::from_secs(1))
    })
    .await;
    files.breaker.offline.store(true, Ordering::SeqCst);
    let path = file_path("discard.txt");
    files
        .store
        .put(&path, bytes::Bytes::from_static(b"bytes").into())
        .await?;
    let id = files.manager.queue.head()?.unwrap().operation_id;
    assert!(files.manager.drain_once().await.is_err());
    files
        .manager
        .queue
        .request_skip(&id, "Keep the cloud version", true)?;
    assert!(!files.manager.drain_once().await?);
    assert_eq!(
        *host.discarded.lock().unwrap(),
        vec![(path.to_string(), id.clone())]
    );
    assert_eq!(
        files.manager.queue.operation_state(&id)?.unwrap().state,
        "skipped"
    );
    Ok(())
}

#[tokio::test]
async fn open_without_quarantine_leaves_sibling_scopes_usable() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let scoped = |scope: char, quarantine: bool| WriteManagerOptions {
        scope: scope.to_string().repeat(64),
        quarantine_other_scopes: quarantine,
        ..desktop_options(root.path())
    };
    let first = WriteManager::open(scoped('a', false), TestHost::new(remote.clone(), None)).await?;
    let _second =
        WriteManager::open(scoped('b', false), TestHost::new(remote.clone(), None)).await?;
    assert!(!first.queue.status()?.quarantined);
    first
        .queue
        .enqueue("resource", json!(1), None, unix_time()?)?;
    let _third = WriteManager::open(scoped('c', true), TestHost::new(remote, None)).await?;
    assert!(first.queue.status()?.quarantined);
    Ok(())
}

#[tokio::test]
async fn refresh_interval_option_throttles_idle_refresh() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let options = WriteManagerOptions {
        refresh_interval: Duration::from_secs(3600),
        ..desktop_options(root.path())
    };
    let (writer, table) =
        open_table(options, TestHost::new(remote.clone(), None), active()).await?;
    let source = remote.open_table("measurements").execute().await?;
    let value = |table: Arc<TableOverlay>| async move {
        let local = table.read_table().await?.unwrap();
        Ok::<_, anyhow::Error>(rows(&local, "id = 1", Select::All).await?[0]["value"].clone())
    };
    source
        .update()
        .only_if("id = 1")
        .column("value", "50")
        .execute()
        .await?;
    table.refresh_idle(&writer).await?;
    assert_eq!(value(table.clone()).await?, 50);
    source
        .update()
        .only_if("id = 1")
        .column("value", "60")
        .execute()
        .await?;
    table.refresh_idle(&writer).await?;
    assert_eq!(value(table.clone()).await?, 50);
    writer.refresh_table(&selection()).await?;
    assert_eq!(value(table.clone()).await?, 60);
    assert!(writer.table_states().await?[0].refreshed_at.is_some());
    Ok(())
}

#[tokio::test]
async fn queue_changed_fires_on_enqueue_ack_block_and_skip() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let server = replay_server(remote.clone(), false, false);
    let host = TestHost::new(remote, Some(server.clone()));
    let (writer, table) = open_table(desktop_options(root.path()), host.clone(), active()).await?;
    let changes = || host.queue_changes.load(Ordering::Acquire);
    assert_eq!(changes(), 0);
    insert(&table, 3, 30).await?;
    assert_eq!(changes(), 1);
    assert!(writer.drain_once().await?);
    assert_eq!(changes(), 2);
    server.not_claimed.store(true, Ordering::Release);
    let id = insert(&table, 4, 40).await?;
    assert_eq!(changes(), 3);
    assert!(writer.drain_once().await.is_err());
    assert_eq!(changes(), 4);
    writer.queue.request_skip(&id, "Discard", false)?;
    assert!(!writer.drain_once().await?);
    assert_eq!(changes(), 5);
    Ok(())
}

#[tokio::test]
async fn add_table_does_not_hold_the_gate_while_snapshotting() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cloud = tempfile::tempdir()?;
    let remote = seed(cloud.path()).await?;
    let (writer, _table) = open_table(
        desktop_options(root.path()),
        TestHost::new(remote.clone(), None),
        active(),
    )
    .await?;
    remote
        .create_table(
            "other",
            flow_like_storage::arrow_utils::value_to_batch_reader(vec![json!({"id": 1})])?,
        )
        .execute()
        .await?;
    let _gate = writer.gate.lock().await;
    let other = BufferedTable {
        table: "other".into(),
        ..selection()
    };
    let state =
        tokio::time::timeout(Duration::from_secs(60), writer.add_table(other, active())).await??;
    assert_eq!(state.activation, Some(TableActivation::Active));
    assert!(state.local_bytes > 0);
    let mut names = writer.table_names(&database_path()).await?;
    names.sort();
    assert_eq!(names, vec!["measurements".to_string(), "other".into()]);
    Ok(())
}
