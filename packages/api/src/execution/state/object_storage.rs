//! Object storage state store for large payload storage
//!
//! Uses S3-compatible storage for execution state. Good for large payloads.
//! TTL is implemented via lifecycle rules on the bucket.
//!
//! Prefers using the meta store from master_credentials when available.
//! Falls back to environment configuration for backwards compatibility.

use super::{postgres::PostgresStateStore, types::*};
use async_trait::async_trait;
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{self, ObjectStore, PutMode, PutOptions, PutPayload, UpdateVersion, path::Path},
};
use futures::TryStreamExt;
use sea_orm::DatabaseConnection;
use std::{collections::HashSet, sync::Arc};

const RUNS_PREFIX: &str = "execution/runs";
const EVENTS_PREFIX: &str = "execution/events";
const INDEXES_PREFIX: &str = "execution/indexes";

pub struct ObjectStorageStateStore {
    store: Arc<dyn ObjectStore>,
    source_run_store: Option<PostgresStateStore>,
}

impl std::fmt::Debug for ObjectStorageStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectStorageStateStore").finish()
    }
}

impl ObjectStorageStateStore {
    pub fn new(flow_store: Arc<FlowLikeStore>) -> Self {
        Self::new_with_source(flow_store, None)
    }

    pub fn new_with_source(
        flow_store: Arc<FlowLikeStore>,
        source_db: Option<Arc<DatabaseConnection>>,
    ) -> Self {
        Self {
            store: flow_store.as_generic(),
            source_run_store: source_db.map(PostgresStateStore::new),
        }
    }

    /// Create from environment configuration (fallback)
    pub async fn from_env() -> Result<Self, StateStoreError> {
        let bucket = std::env::var("META_BUCKET")
            .or_else(|_| std::env::var("META_BUCKET_NAME"))
            .or_else(|_| std::env::var("S3_STATE_BUCKET"))
            .map_err(|_| {
                StateStoreError::Configuration(
                    "Neither META_BUCKET_NAME nor S3_STATE_BUCKET is set".into(),
                )
            })?;

        let is_express = std::env::var("META_BUCKET_EXPRESS_ZONE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());

        let mut builder = object_store::aws::AmazonS3Builder::from_env()
            .with_bucket_name(&bucket)
            .with_region(&region);

        if is_express {
            builder = builder.with_virtual_hosted_style_request(true);
        }

        let store = builder
            .build()
            .map_err(|e| StateStoreError::Configuration(e.to_string()))?;

        Ok(Self {
            store: Arc::new(store),
            source_run_store: None,
        })
    }

    fn run_path(id: &str) -> Path {
        Path::from(format!("{RUNS_PREFIX}/{id}.json"))
    }

    fn event_path(run_id: &str, sequence: i32) -> Path {
        Path::from(format!("{EVENTS_PREFIX}/{run_id}/{sequence:08}.json"))
    }

    fn app_index_path(app_id: &str, created_at: i64, run_id: &str) -> Path {
        Path::from(format!(
            "{INDEXES_PREFIX}/by-app/{app_id}/{created_at:020}_{run_id}"
        ))
    }

    fn events_prefix(run_id: &str) -> Path {
        Path::from(format!("{EVENTS_PREFIX}/{run_id}/"))
    }

    async fn put_json<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StateStoreError> {
        let json =
            serde_json::to_vec(value).map_err(|e| StateStoreError::Serialization(e.to_string()))?;

        self.store
            .put(path, PutPayload::from(json))
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(())
    }

    async fn put_json_if_absent<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StateStoreError> {
        let json =
            serde_json::to_vec(value).map_err(|e| StateStoreError::Serialization(e.to_string()))?;
        match self
            .store
            .put_opts(
                path,
                PutPayload::from(json),
                PutOptions {
                    mode: PutMode::Create,
                    ..Default::default()
                },
            )
            .await
        {
            Ok(_) | Err(object_store::Error::AlreadyExists { .. }) => Ok(()),
            Err(error) => Err(StateStoreError::Database(error.to_string())),
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
    ) -> Result<Option<T>, StateStoreError> {
        match self.store.get(path).await {
            Ok(result) => {
                let bytes = result
                    .bytes()
                    .await
                    .map_err(|e| StateStoreError::Database(e.to_string()))?;
                let value: T = serde_json::from_slice(&bytes)
                    .map_err(|e| StateStoreError::Serialization(e.to_string()))?;
                Ok(Some(value))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(StateStoreError::Database(e.to_string())),
        }
    }

    async fn get_json_with_version<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
    ) -> Result<Option<(T, UpdateVersion)>, StateStoreError> {
        match self.store.get(path).await {
            Ok(result) => {
                let version = UpdateVersion {
                    e_tag: result.meta.e_tag.clone(),
                    version: result.meta.version.clone(),
                };
                let bytes = result
                    .bytes()
                    .await
                    .map_err(|error| StateStoreError::Database(error.to_string()))?;
                let value = serde_json::from_slice(&bytes)
                    .map_err(|error| StateStoreError::Serialization(error.to_string()))?;
                Ok(Some((value, version)))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(StateStoreError::Database(error.to_string())),
        }
    }

    async fn delete(&self, path: &Path) -> Result<(), StateStoreError> {
        self.store
            .delete(path)
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn read_run(&self, run_id: &str) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        self.get_json(&Self::run_path(run_id)).await
    }

    async fn import_source_run(
        &self,
        run_id: &str,
        app_id: Option<&str>,
    ) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        let Some(source) = self.source_run_store.as_ref() else {
            return Ok(None);
        };
        let record = match app_id {
            Some(app_id) => source.get_run_for_app(run_id, app_id).await?,
            None => source.get_run(run_id).await?,
        };
        let Some(record) = record else {
            return Ok(None);
        };
        self.import_fetched_source_run(record, run_id, app_id).await
    }

    async fn import_fetched_source_run(
        &self,
        record: ExecutionRunRecord,
        run_id: &str,
        app_id: Option<&str>,
    ) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        if source_run_expired(&record, chrono::Utc::now().timestamp_millis()) {
            return Ok(None);
        }

        let path = Self::run_path(run_id);
        let json = serde_json::to_vec(&record)
            .map_err(|error| StateStoreError::Serialization(error.to_string()))?;
        let write = self
            .store
            .put_opts(
                &path,
                PutPayload::from(json),
                PutOptions {
                    mode: PutMode::Create,
                    ..Default::default()
                },
            )
            .await;

        match write {
            Ok(_) => {
                let index_path = Self::app_index_path(&record.app_id, record.created_at, run_id);
                self.store
                    .put(&index_path, PutPayload::from(run_id.as_bytes().to_vec()))
                    .await
                    .map_err(|error| StateStoreError::Database(error.to_string()))?;
            }
            Err(object_store::Error::AlreadyExists { .. }) => {}
            Err(write_error) => {
                if self.read_run(run_id).await?.is_none() {
                    return Err(StateStoreError::Database(format!(
                        "object-store source-run import failed: {write_error}"
                    )));
                }
            }
        }

        match self.read_run(run_id).await? {
            Some(existing)
                if app_id
                    .map(|expected| existing.app_id == expected)
                    .unwrap_or(true) =>
            {
                Ok(Some(existing))
            }
            Some(_) => Ok(None),
            None => Err(StateStoreError::Database(format!(
                "object-store source-run import for '{run_id}' did not persist a run"
            ))),
        }
    }
}

#[async_trait]
impl ExecutionStateStore for ObjectStorageStateStore {
    fn backend_name(&self) -> &'static str {
        "s3"
    }

    async fn create_run(
        &self,
        input: CreateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        let now = chrono::Utc::now().timestamp_millis();

        let record = ExecutionRunRecord {
            id: input.id.clone(),
            board_id: input.board_id,
            version: input.version,
            event_id: input.event_id,
            status: RunStatus::Pending,
            mode: input.mode,
            input_payload_len: input.input_payload_len,
            output_payload_len: 0,
            error_message: None,
            progress: 0,
            current_step: None,
            started_at: None,
            completed_at: None,
            expires_at: input.expires_at,
            user_id: input.user_id,
            technical_user_id: input.technical_user_id,
            app_id: input.app_id.clone(),
            created_at: now,
            updated_at: now,
        };

        let path = Self::run_path(&input.id);
        self.put_json(&path, &record).await?;

        let index_path = Self::app_index_path(&input.app_id, now, &input.id);
        self.store
            .put(&index_path, PutPayload::from(input.id.as_bytes().to_vec()))
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(record)
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        match self.read_run(run_id).await? {
            Some(record) => Ok(Some(record)),
            None => self.import_source_run(run_id, None).await,
        }
    }

    async fn get_run_for_app(
        &self,
        run_id: &str,
        app_id: &str,
    ) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        match self.read_run(run_id).await? {
            Some(record) if record.app_id == app_id => Ok(Some(record)),
            Some(_) => Ok(None),
            None => self.import_source_run(run_id, Some(app_id)).await,
        }
    }

    async fn update_run(
        &self,
        run_id: &str,
        input: UpdateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        let path = Self::run_path(run_id);
        for _ in 0..4 {
            let (mut record, version) = self
                .get_json_with_version::<ExecutionRunRecord>(&path)
                .await?
                .ok_or(StateStoreError::NotFound)?;

            if record.status.is_terminal() {
                return Ok(record);
            }

            record.updated_at = chrono::Utc::now()
                .timestamp_millis()
                .max(record.updated_at.saturating_add(1));

            if let Some(progress) = input.progress {
                record.progress = progress;
            }
            if let Some(current_step) = input.current_step.clone() {
                record.current_step = Some(current_step);
            }
            if let Some(status) = input.status.clone() {
                record.status = status;
            }
            if let Some(output_payload_len) = input.output_payload_len {
                record.output_payload_len = output_payload_len;
            }
            if let Some(error_message) = input.error_message.clone() {
                record.error_message = Some(error_message);
            }
            if let Some(started_at) = input.started_at {
                record.started_at = Some(started_at);
            }
            if let Some(completed_at) = input.completed_at {
                record.completed_at = Some(completed_at);
            }

            let json = serde_json::to_vec(&record)
                .map_err(|error| StateStoreError::Serialization(error.to_string()))?;
            match self
                .store
                .put_opts(
                    &path,
                    PutPayload::from(json),
                    PutOptions {
                        mode: PutMode::Update(version),
                        ..Default::default()
                    },
                )
                .await
            {
                Ok(_) => return Ok(record),
                Err(object_store::Error::Precondition { .. })
                | Err(object_store::Error::NotModified { .. }) => continue,
                Err(error) => return Err(StateStoreError::Database(error.to_string())),
            }
        }

        Err(StateStoreError::LeaseConflict(format!(
            "execution run '{run_id}' changed while applying progress"
        )))
    }

    async fn list_runs_for_app(
        &self,
        app_id: &str,
        limit: i32,
        cursor: Option<&str>,
    ) -> Result<Vec<ExecutionRunRecord>, StateStoreError> {
        let prefix = Path::from(format!("{INDEXES_PREFIX}/by-app/{app_id}/"));

        let offset = if let Some(cursor) = cursor {
            if let Some(record) = self.get_run(cursor).await? {
                Some(Self::app_index_path(app_id, record.created_at, cursor))
            } else {
                None
            }
        } else {
            None
        };

        let list_result = self
            .store
            .list_with_offset(Some(&prefix), &offset.unwrap_or_else(|| Path::from("")))
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        let mut keys: Vec<_> = list_result.iter().map(|o| o.location.to_string()).collect();
        keys.sort_by(|a, b| b.cmp(a));

        let mut records = Vec::new();
        for key in keys.iter().take(limit as usize) {
            if let Some(run_id) = key.rsplit('_').next()
                && let Some(record) = self.get_run(run_id).await?
            {
                records.push(record);
            }
        }

        Ok(records)
    }

    async fn delete_expired_runs(&self) -> Result<i64, StateStoreError> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut deleted = 0i64;

        let prefix = Path::from(RUNS_PREFIX);
        let list_result = self
            .store
            .list(Some(&prefix))
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        for obj in list_result {
            if let Some(record) = self.get_json::<ExecutionRunRecord>(&obj.location).await?
                && let Some(expires_at) = record.expires_at
                && expires_at < now
            {
                self.delete(&obj.location).await?;
                let index_path =
                    Self::app_index_path(&record.app_id, record.created_at, &record.id);
                let _ = self.delete(&index_path).await;
                deleted += 1;
            }
        }

        Ok(deleted)
    }

    async fn push_events(&self, events: Vec<CreateEventInput>) -> Result<i32, StateStoreError> {
        if events.is_empty() {
            return Ok(0);
        }

        let now = chrono::Utc::now().timestamp_millis();

        for event in &events {
            let record = ExecutionEventRecord {
                id: event.id.clone(),
                run_id: event.run_id.clone(),
                sequence: event.sequence,
                event_type: event.event_type.clone(),
                payload: event.payload.clone(),
                delivered: false,
                expires_at: event.expires_at,
                created_at: now,
            };

            let path = Self::event_path(&event.run_id, event.sequence);
            // Canonical event retries are first-write-wins. Conditional create
            // keeps the original payload and any delivered state intact.
            self.put_json_if_absent(&path, &record).await?;
        }

        Ok(events.len() as i32)
    }

    async fn get_events(
        &self,
        query: EventQuery,
    ) -> Result<Vec<ExecutionEventRecord>, StateStoreError> {
        let prefix = Self::events_prefix(&query.run_id);

        let list_result = self
            .store
            .list(Some(&prefix))
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        let mut records = Vec::new();
        for obj in list_result {
            if let Some(after_seq) = query.after_sequence
                && let Some(seq_str) = obj
                    .location
                    .filename()
                    .and_then(|s| s.strip_suffix(".json"))
                && let Ok(seq) = seq_str.parse::<i32>()
                && seq <= after_seq
            {
                continue;
            }

            if let Some(record) = self.get_json::<ExecutionEventRecord>(&obj.location).await?
                && (!query.only_undelivered || !record.delivered)
            {
                records.push(record);
            }

            if let Some(limit) = query.limit
                && records.len() >= limit as usize
            {
                break;
            }
        }

        records.sort_by_key(|e| e.sequence);

        Ok(records)
    }

    async fn get_max_sequence(&self, run_id: &str) -> Result<i32, StateStoreError> {
        let prefix = Self::events_prefix(run_id);

        let list_result = self
            .store
            .list(Some(&prefix))
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        let mut max_seq = 0;
        for obj in list_result {
            if let Some(seq_str) = obj
                .location
                .filename()
                .and_then(|s| s.strip_suffix(".json"))
                && let Ok(seq) = seq_str.parse::<i32>()
            {
                max_seq = max_seq.max(seq);
            }
        }

        Ok(max_seq)
    }

    async fn mark_events_delivered(
        &self,
        run_id: &str,
        event_ids: &[String],
    ) -> Result<(), StateStoreError> {
        if event_ids.is_empty() {
            return Ok(());
        }

        // The trait supplies opaque IDs, while this backend keys objects by
        // run and sequence. List only this run's prefix to resolve them; the
        // old `run:sequence` parser never matched executor-generated IDs.
        let mut remaining = event_ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let objects = self
            .store
            .list(Some(&Self::events_prefix(run_id)))
            .try_collect::<Vec<_>>()
            .await
            .map_err(|error| StateStoreError::Database(error.to_string()))?;
        for object in objects {
            let Some(mut record) = self
                .get_json::<ExecutionEventRecord>(&object.location)
                .await?
            else {
                continue;
            };
            if remaining.remove(record.id.as_str()) {
                record.delivered = true;
                self.put_json(&object.location, &record).await?;
                if remaining.is_empty() {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn delete_expired_events(&self) -> Result<i64, StateStoreError> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut deleted = 0i64;

        let prefix = Path::from(EVENTS_PREFIX);
        let list_result = self
            .store
            .list(Some(&prefix))
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        for obj in list_result {
            if let Some(record) = self.get_json::<ExecutionEventRecord>(&obj.location).await?
                && record.expires_at < now
            {
                self.delete(&obj.location).await?;
                deleted += 1;
            }
        }

        Ok(deleted)
    }
}

#[cfg(test)]
mod stateless_import_tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;

    fn memory_store(source_db: Option<Arc<DatabaseConnection>>) -> ObjectStorageStateStore {
        ObjectStorageStateStore::new_with_source(
            Arc::new(FlowLikeStore::Memory(Arc::new(InMemory::new()))),
            source_db,
        )
    }

    fn create_input() -> CreateRunInput {
        CreateRunInput {
            id: "run-1".into(),
            board_id: "board-1".into(),
            version: Some("7".into()),
            event_id: Some("event-1".into()),
            mode: RunMode::Queue,
            input_payload_len: 12,
            user_id: Some("user-1".into()),
            technical_user_id: None,
            app_id: "app-1".into(),
            expires_at: Some(1_900_000_000_000),
        }
    }

    #[test]
    fn stateless_lambda_object_store_constructor_retains_sql_source() {
        let store = memory_store(Some(Arc::new(DatabaseConnection::Disconnected)));
        assert!(store.source_run_store.is_some());
    }

    #[tokio::test]
    async fn stateless_lambda_object_store_round_trips_queue_runs() {
        let store = memory_store(None);
        let created = store
            .create_run(create_input())
            .await
            .expect("queue run should be stored");
        let loaded = store
            .get_run_for_app("run-1", "app-1")
            .await
            .expect("queue run should be readable")
            .expect("queue run should exist");

        assert_eq!(loaded.id, created.id);
        assert_eq!(loaded.mode, RunMode::Queue);
        assert_eq!(loaded.status, RunStatus::Pending);
        assert_eq!(loaded.app_id, "app-1");
        assert!(
            store
                .get_run_for_app("run-1", "other-app")
                .await
                .expect("wrong-app lookup should not fail")
                .is_none()
        );
    }

    #[tokio::test]
    async fn stateless_lambda_object_store_never_regresses_terminal_state() {
        let store = Arc::new(memory_store(None));
        store
            .create_run(create_input())
            .await
            .expect("queue run should be stored");

        let completed_store = store.clone();
        let running_store = store.clone();
        let (completed, running) = tokio::join!(
            completed_store.update_run(
                "run-1",
                UpdateRunInput {
                    status: Some(RunStatus::Completed),
                    completed_at: Some(1_800_000_010_000),
                    progress: Some(100),
                    ..Default::default()
                }
            ),
            running_store.update_run(
                "run-1",
                UpdateRunInput {
                    status: Some(RunStatus::Running),
                    progress: Some(50),
                    ..Default::default()
                }
            )
        );
        completed.expect("terminal update should succeed");
        running.expect("racing progress should resolve safely");

        let final_run = store
            .get_run("run-1")
            .await
            .expect("final run should be readable")
            .expect("final run should exist");
        assert_eq!(final_run.status, RunStatus::Completed);
        assert_eq!(final_run.progress, 100);
    }

    #[tokio::test]
    async fn stateless_lambda_object_store_event_retry_is_first_write_wins() {
        let store = memory_store(None);
        let base = CreateEventInput {
            id: "evt-1".into(),
            run_id: "run-1".into(),
            sequence: 0,
            event_type: "chunk".into(),
            payload: serde_json::json!({"attempt": 1}),
            expires_at: 1_900_000_000_000,
        };
        store
            .push_events(vec![base.clone()])
            .await
            .expect("first event write should succeed");
        store
            .mark_events_delivered("run-1", &[base.id.clone()])
            .await
            .expect("event should be marked delivered");

        let mut retry = base;
        retry.payload = serde_json::json!({"attempt": 2});
        store
            .push_events(vec![retry])
            .await
            .expect("canonical retry should be a no-op");
        let events = store
            .get_events(EventQuery {
                run_id: "run-1".into(),
                after_sequence: Some(-1),
                only_undelivered: false,
                limit: None,
            })
            .await
            .expect("stored event should be readable");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload, serde_json::json!({"attempt": 1}));
        assert!(events[0].delivered);
    }

    fn source_record(expires_at: Option<i64>, status: RunStatus) -> ExecutionRunRecord {
        ExecutionRunRecord {
            id: "run-1".into(),
            board_id: "board-1".into(),
            version: Some("7".into()),
            event_id: Some("event-1".into()),
            status,
            mode: RunMode::Queue,
            input_payload_len: 12,
            output_payload_len: 0,
            error_message: None,
            progress: 0,
            current_step: None,
            started_at: None,
            completed_at: None,
            expires_at,
            user_id: Some("user-1".into()),
            technical_user_id: None,
            app_id: "app-1".into(),
            created_at: 1_800_000_000_000,
            updated_at: 1_800_000_000_001,
        }
    }

    #[tokio::test]
    async fn stateless_lambda_object_store_never_imports_an_expired_source_run() {
        let store = memory_store(None);
        let expired = source_record(Some(1_000), RunStatus::Completed);

        let imported = store
            .import_fetched_source_run(expired, "run-1", Some("app-1"))
            .await
            .expect("expired source row should be a clean miss");
        assert!(imported.is_none(), "expired run must not be resurrected");
        assert!(
            store
                .read_run("run-1")
                .await
                .expect("store should stay readable")
                .is_none(),
            "no run object may be written for an expired source row"
        );
    }

    #[tokio::test]
    async fn stateless_lambda_object_store_still_imports_terminal_unexpired_runs() {
        let store = memory_store(None);
        let terminal = source_record(Some(1_900_000_000_000), RunStatus::Completed);

        let imported = store
            .import_fetched_source_run(terminal, "run-1", Some("app-1"))
            .await
            .expect("terminal-but-unexpired source row should import")
            .expect("queue redelivery needs the terminal state visible");
        assert_eq!(imported.status, RunStatus::Completed);
        assert_eq!(imported.app_id, "app-1");
    }

    #[test]
    fn source_run_expiry_guard_only_blocks_past_expiry() {
        let now = 1_800_000_000_000;
        assert!(source_run_expired(
            &source_record(Some(now), RunStatus::Completed),
            now
        ));
        assert!(source_run_expired(
            &source_record(Some(now - 1), RunStatus::Pending),
            now
        ));
        assert!(!source_run_expired(
            &source_record(Some(now + 1), RunStatus::Completed),
            now
        ));
        assert!(!source_run_expired(
            &source_record(None, RunStatus::Pending),
            now
        ));
    }

    #[tokio::test]
    async fn mark_events_delivered_only_touches_the_named_run() {
        let store = memory_store(None);
        let run_one = CreateEventInput {
            id: "evt-run-1".into(),
            run_id: "run-1".into(),
            sequence: 0,
            event_type: "chunk".into(),
            payload: serde_json::json!({"run": 1}),
            expires_at: 1_900_000_000_000,
        };
        let mut run_two = run_one.clone();
        run_two.id = "evt-run-2".into();
        run_two.run_id = "run-2".into();
        store
            .push_events(vec![run_one.clone(), run_two.clone()])
            .await
            .expect("events should be stored");

        store
            .mark_events_delivered("run-1", &[run_one.id.clone(), run_two.id.clone()])
            .await
            .expect("run-scoped delivery marking should succeed");

        let query = |run_id: &str| EventQuery {
            run_id: run_id.into(),
            after_sequence: Some(-1),
            only_undelivered: false,
            limit: None,
        };
        let first = store
            .get_events(query("run-1"))
            .await
            .expect("run-1 events should be readable");
        let second = store
            .get_events(query("run-2"))
            .await
            .expect("run-2 events should be readable");
        assert!(first[0].delivered, "run-1 event should be delivered");
        assert!(
            !second[0].delivered,
            "an id outside the named run must stay untouched"
        );
    }
}
