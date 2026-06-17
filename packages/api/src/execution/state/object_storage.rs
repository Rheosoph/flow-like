//! Object storage state store for large payload storage
//!
//! Uses S3-compatible storage for execution state. Good for large payloads.
//! TTL is implemented via lifecycle rules on the bucket.
//!
//! Prefers using the meta store from master_credentials when available.
//! Falls back to environment configuration for backwards compatibility.

use super::types::*;
use async_trait::async_trait;
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{self, ObjectStore, PutPayload, path::Path},
};
use futures::TryStreamExt;
use std::collections::HashMap;
use std::sync::Arc;

const RUNS_PREFIX: &str = "execution/runs";
const EVENTS_PREFIX: &str = "execution/events";
const INDEXES_PREFIX: &str = "execution/indexes";

pub struct ObjectStorageStateStore {
    store: Arc<dyn ObjectStore>,
}

impl std::fmt::Debug for ObjectStorageStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectStorageStateStore").finish()
    }
}

impl ObjectStorageStateStore {
    pub fn new(flow_store: Arc<FlowLikeStore>) -> Self {
        Self {
            store: flow_store.as_generic(),
        }
    }

    /// Create from environment configuration (fallback)
    pub async fn from_env() -> Result<Self, StateStoreError> {
        use aws_sdk_s3::config::Builder as S3ConfigBuilder;

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

    async fn delete(&self, path: &Path) -> Result<(), StateStoreError> {
        self.store
            .delete(path)
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;
        Ok(())
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
        let path = Self::run_path(run_id);
        self.get_json(&path).await
    }

    async fn get_run_for_app(
        &self,
        run_id: &str,
        app_id: &str,
    ) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        let record = self.get_run(run_id).await?;
        match record {
            Some(r) if r.app_id == app_id => Ok(Some(r)),
            _ => Ok(None),
        }
    }

    async fn update_run(
        &self,
        run_id: &str,
        input: UpdateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        let mut record = self
            .get_run(run_id)
            .await?
            .ok_or(StateStoreError::NotFound)?;

        record.updated_at = chrono::Utc::now().timestamp_millis();

        if let Some(progress) = input.progress {
            record.progress = progress;
        }
        if let Some(current_step) = input.current_step {
            record.current_step = Some(current_step);
        }
        if let Some(status) = input.status {
            record.status = status;
        }
        if let Some(output_payload_len) = input.output_payload_len {
            record.output_payload_len = output_payload_len;
        }
        if let Some(error_message) = input.error_message {
            record.error_message = Some(error_message);
        }
        if let Some(started_at) = input.started_at {
            record.started_at = Some(started_at);
        }
        if let Some(completed_at) = input.completed_at {
            record.completed_at = Some(completed_at);
        }

        let path = Self::run_path(run_id);
        self.put_json(&path, &record).await?;

        Ok(record)
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
            if let Some(run_id) = key.rsplit('_').next() {
                if let Some(record) = self.get_run(run_id).await? {
                    records.push(record);
                }
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
            if let Some(record) = self.get_json::<ExecutionRunRecord>(&obj.location).await? {
                if let Some(expires_at) = record.expires_at {
                    if expires_at < now {
                        self.delete(&obj.location).await?;
                        let index_path =
                            Self::app_index_path(&record.app_id, record.created_at, &record.id);
                        let _ = self.delete(&index_path).await;
                        deleted += 1;
                    }
                }
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
            self.put_json(&path, &record).await?;
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
            if let Some(after_seq) = query.after_sequence {
                if let Some(seq_str) = obj
                    .location
                    .filename()
                    .and_then(|s| s.strip_suffix(".json"))
                {
                    if let Ok(seq) = seq_str.parse::<i32>() {
                        if seq <= after_seq {
                            continue;
                        }
                    }
                }
            }

            if let Some(record) = self.get_json::<ExecutionEventRecord>(&obj.location).await? {
                if !query.only_undelivered || !record.delivered {
                    records.push(record);
                }
            }

            if let Some(limit) = query.limit {
                if records.len() >= limit as usize {
                    break;
                }
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
            {
                if let Ok(seq) = seq_str.parse::<i32>() {
                    max_seq = max_seq.max(seq);
                }
            }
        }

        Ok(max_seq)
    }

    async fn mark_events_delivered(&self, event_ids: &[String]) -> Result<(), StateStoreError> {
        for id in event_ids {
            let parts: Vec<&str> = id.split(':').collect();
            if parts.len() == 2 {
                let run_id = parts[0];
                if let Ok(sequence) = parts[1].parse::<i32>() {
                    let path = Self::event_path(run_id, sequence);
                    if let Some(mut record) = self.get_json::<ExecutionEventRecord>(&path).await? {
                        record.delivered = true;
                        self.put_json(&path, &record).await?;
                    }
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
            if let Some(record) = self.get_json::<ExecutionEventRecord>(&obj.location).await? {
                if record.expires_at < now {
                    self.delete(&obj.location).await?;
                    deleted += 1;
                }
            }
        }

        Ok(deleted)
    }
}
