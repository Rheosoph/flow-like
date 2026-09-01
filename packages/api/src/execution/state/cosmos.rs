//! Azure Cosmos DB for NoSQL execution-state backend.
//!
//! Provision `execution-runs` with partition key `/app_id` and `execution-events` with
//! partition key `/run_id`. Both containers must enable TTL with default `-1`; individual
//! documents carry a relative `ttl`. Event bodies over 100 KiB are offloaded to the
//! existing Azure-backed `FlowLikeStore`, keeping Cosmos request units predictable.

use super::{PostgresStateStore, types::*};
use crate::cosmos::{
    CosmosClient, CosmosError, MutationOutcome, QueryParameter, ttl_seconds, validate_container_id,
};
use async_trait::async_trait;
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{ObjectStore, path::Path},
};
use futures::{StreamExt, stream};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

const DEFAULT_RUNS_CONTAINER: &str = "execution-runs";
const DEFAULT_EVENTS_CONTAINER: &str = "execution-events";
const DEFAULT_TTL_SECONDS: i64 = 86_400;
const PAYLOAD_OFFLOAD_BYTES: usize = 100 * 1024;
const PAYLOAD_PREFIX: &str = "polling";
const QUERY_PAGE_SIZE: i32 = 1_000;
const WRITE_CONCURRENCY: usize = 16;
const UPDATE_ATTEMPTS: usize = 3;
const MARK_QUERY_CHUNK: usize = 50;

fn has_canonical_event_identity(event: &CreateEventInput) -> bool {
    let digest = blake3::hash(format!("{}:{}", event.run_id, event.sequence).as_bytes());
    event.id == format!("evt-{}", digest.to_hex())
}

#[derive(Clone)]
pub struct CosmosStateStore {
    client: CosmosClient,
    content_store: Option<Arc<FlowLikeStore>>,
    runs_container: String,
    events_container: String,
    default_ttl_seconds: i64,
    source_run_store: Option<PostgresStateStore>,
}

impl std::fmt::Debug for CosmosStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CosmosStateStore")
            .field("client", &self.client)
            .field("runs_container", &self.runs_container)
            .field("events_container", &self.events_container)
            .field("has_content_store", &self.content_store.is_some())
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RunDocument {
    #[serde(flatten)]
    record: ExecutionRunRecord,
    #[serde(rename = "ttl", skip_serializing_if = "Option::is_none")]
    ttl: Option<i64>,
    #[serde(rename = "_etag", default, skip_serializing)]
    etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bound_job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lease: Option<RunLease>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RunLease {
    token: String,
    expires_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EventDocument {
    #[serde(flatten)]
    record: ExecutionEventRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload_ref: Option<String>,
    #[serde(rename = "ttl", skip_serializing_if = "Option::is_none")]
    ttl: Option<i64>,
    #[serde(rename = "_etag", default, skip_serializing)]
    etag: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct SequenceProjection {
    sequence: i32,
}

impl CosmosStateStore {
    pub fn from_env(
        content_store: Option<Arc<FlowLikeStore>>,
        source_db: Option<Arc<DatabaseConnection>>,
    ) -> Result<Self, StateStoreError> {
        let client = CosmosClient::from_env().map_err(map_error)?;
        let runs_container = optional_env("COSMOS_RUNS_CONTAINER", DEFAULT_RUNS_CONTAINER);
        let events_container = optional_env("COSMOS_EVENTS_CONTAINER", DEFAULT_EVENTS_CONTAINER);
        validate_container_id(&runs_container).map_err(map_error)?;
        validate_container_id(&events_container).map_err(map_error)?;
        let default_ttl_seconds = match std::env::var("EXECUTION_STATE_TTL_SECONDS") {
            Ok(value) => value.trim().parse::<i64>().map_err(|_| {
                StateStoreError::Configuration(
                    "EXECUTION_STATE_TTL_SECONDS must be a positive integer".to_string(),
                )
            })?,
            Err(std::env::VarError::NotPresent) => DEFAULT_TTL_SECONDS,
            Err(error) => {
                return Err(StateStoreError::Configuration(format!(
                    "EXECUTION_STATE_TTL_SECONDS could not be read: {error}"
                )));
            }
        };
        if default_ttl_seconds <= 0 {
            return Err(StateStoreError::Configuration(
                "EXECUTION_STATE_TTL_SECONDS must be a positive integer".to_string(),
            ));
        }
        Ok(Self {
            client,
            content_store,
            runs_container,
            events_container,
            default_ttl_seconds,
            source_run_store: source_db.map(PostgresStateStore::new),
        })
    }

    fn run_document(&self, record: ExecutionRunRecord, now_ms: i64) -> RunDocument {
        RunDocument {
            ttl: ttl_seconds(record.expires_at, now_ms),
            record,
            etag: None,
            bound_job_id: None,
            lease: None,
        }
    }

    fn event_document(
        record: ExecutionEventRecord,
        payload_ref: Option<String>,
        now_ms: i64,
    ) -> EventDocument {
        EventDocument {
            ttl: ttl_seconds(Some(record.expires_at), now_ms),
            record,
            payload_ref,
            etag: None,
        }
    }

    fn is_expired(expires_at: Option<i64>, now_ms: i64) -> bool {
        expires_at.is_some_and(|expires_at| expires_at <= now_ms)
    }

    async fn find_run_document(
        &self,
        run_id: &str,
    ) -> Result<Option<RunDocument>, StateStoreError> {
        // A cross-partition query can return empty pages that only carry a continuation
        // token, so the run must be considered missing only once the query is exhausted.
        let documents = self
            .query_all::<RunDocument>(
                &self.runs_container,
                "SELECT TOP 2 * FROM c WHERE c.id = @id",
                &[QueryParameter::new("@id", run_id)],
                None,
                Some(2),
            )
            .await?;
        if documents.len() > 1 {
            return Err(StateStoreError::Database(format!(
                "run ID '{run_id}' is present in more than one Cosmos app partition"
            )));
        }
        Ok(documents.into_iter().next())
    }

    async fn read_run_for_app_document(
        &self,
        run_id: &str,
        app_id: &str,
    ) -> Result<Option<RunDocument>, StateStoreError> {
        self.client
            .read_document(&self.runs_container, run_id, app_id)
            .await
            .map_err(map_error)
    }

    /// Async invocation creates the canonical audit row in PostgreSQL before
    /// dispatch. Import that row exactly once when Cosmos is selected for live
    /// execution state. A create conflict means another API replica won the
    /// same import race and is safe to re-read.
    async fn import_source_run(
        &self,
        run_id: &str,
        app_id: Option<&str>,
    ) -> Result<Option<RunDocument>, StateStoreError> {
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
        let now = chrono::Utc::now().timestamp_millis();
        if source_run_expired(&record, now) {
            // Reads already mask expired documents; skipping the import also
            // avoids re-creating one that native TTL would delete again.
            return Ok(None);
        }
        let document = self.run_document(record.clone(), now);
        match self
            .client
            .create_document(&self.runs_container, &record.app_id, &document)
            .await
            .map_err(map_error)?
        {
            MutationOutcome::Applied | MutationOutcome::Conflict => {}
            outcome => {
                return Err(StateStoreError::Database(format!(
                    "unexpected Cosmos source-run import result: {outcome:?}"
                )));
            }
        }
        self.read_run_for_app_document(run_id, &record.app_id).await
    }

    async fn ensure_run_for_app_document(
        &self,
        run_id: &str,
        app_id: &str,
    ) -> Result<Option<RunDocument>, StateStoreError> {
        match self.read_run_for_app_document(run_id, app_id).await? {
            Some(document) => Ok(Some(document)),
            None => self.import_source_run(run_id, Some(app_id)).await,
        }
    }

    async fn store_large_payload(
        &self,
        run_id: &str,
        event_id: &str,
        body: &[u8],
    ) -> Result<String, StateStoreError> {
        let store = self.content_store.as_ref().ok_or_else(|| {
            StateStoreError::Configuration(
                "Cosmos event payload exceeds 100 KiB but no content FlowLikeStore was supplied"
                    .to_string(),
            )
        })?;
        let run_hash = blake3::hash(run_id.as_bytes()).to_hex();
        let event_hash = blake3::hash(event_id.as_bytes()).to_hex();
        // Include the content hash in the immutable reference. Otherwise a
        // conflicting retry could overwrite the Blob for an already-created
        // event before Cosmos detects that its stable ID was reused.
        let payload_hash = blake3::hash(body).to_hex();
        let path = Path::from(format!(
            "{PAYLOAD_PREFIX}/{run_hash}/{event_hash}-{payload_hash}.json"
        ));
        store
            .as_generic()
            .put(&path, body.to_vec().into())
            .await
            .map_err(|error| {
                StateStoreError::Database(format!("Azure Blob payload write failed: {error}"))
            })?;
        Ok(format!("store://{path}"))
    }

    async fn fetch_large_payload(&self, reference: &str) -> Result<Value, StateStoreError> {
        let store = self.content_store.as_ref().ok_or_else(|| {
            StateStoreError::Configuration(
                "Cosmos event references Blob payload storage but no content FlowLikeStore was supplied"
                    .to_string(),
            )
        })?;
        let raw_path = reference.strip_prefix("store://").ok_or_else(|| {
            StateStoreError::Serialization("invalid Cosmos event payload reference".to_string())
        })?;
        let result = store
            .as_generic()
            .get(&Path::from(raw_path))
            .await
            .map_err(|error| {
                StateStoreError::Database(format!("Azure Blob payload read failed: {error}"))
            })?;
        let bytes = result.bytes().await.map_err(|error| {
            StateStoreError::Database(format!("Azure Blob payload stream failed: {error}"))
        })?;
        serde_json::from_slice(&bytes)
            .map_err(|error| StateStoreError::Serialization(error.to_string()))
    }

    async fn hydrate_event(
        &self,
        mut document: EventDocument,
    ) -> Result<ExecutionEventRecord, StateStoreError> {
        if let Some(reference) = document.payload_ref.as_deref() {
            document.record.payload = self.fetch_large_payload(reference).await?;
        }
        Ok(document.record)
    }

    async fn event_exists(&self, event: &CreateEventInput) -> Result<bool, StateStoreError> {
        self.client
            .read_document::<EventDocument>(&self.events_container, &event.id, &event.run_id)
            .await
            .map(|document| document.is_some())
            .map_err(map_error)
    }

    async fn query_all<T: serde::de::DeserializeOwned>(
        &self,
        container: &str,
        query: &str,
        parameters: &[QueryParameter],
        partition_key: Option<&str>,
        max_items: Option<usize>,
    ) -> Result<Vec<T>, StateStoreError> {
        let mut documents = Vec::new();
        let mut continuation = None;
        loop {
            let remaining = max_items
                .map(|limit| limit.saturating_sub(documents.len()))
                .unwrap_or(QUERY_PAGE_SIZE as usize);
            if remaining == 0 {
                break;
            }
            let page = self
                .client
                .query_page::<T>(
                    container,
                    query,
                    parameters,
                    partition_key,
                    remaining.min(QUERY_PAGE_SIZE as usize) as i32,
                    continuation.as_deref(),
                )
                .await
                .map_err(map_error)?;
            documents.extend(page.documents);
            continuation = page.continuation;
            if continuation.is_none() {
                break;
            }
        }
        Ok(documents)
    }
}

fn optional_env(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn map_error(error: CosmosError) -> StateStoreError {
    match error {
        CosmosError::Configuration(message) => StateStoreError::Configuration(message),
        CosmosError::Authentication(message) | CosmosError::Transport(message) => {
            StateStoreError::Connection(message)
        }
        CosmosError::Serialization(message) => StateStoreError::Serialization(message),
        error @ CosmosError::Service { .. } => StateStoreError::Database(error.to_string()),
    }
}

fn accept_event_create_outcome(outcome: MutationOutcome) -> Result<(), StateStoreError> {
    match outcome {
        MutationOutcome::Applied | MutationOutcome::Conflict => Ok(()),
        other => Err(StateStoreError::Database(format!(
            "unexpected Cosmos event create result: {other:?}"
        ))),
    }
}

fn apply_update(record: &mut ExecutionRunRecord, input: &UpdateRunInput, now_ms: i64) {
    record.updated_at = now_ms.max(record.updated_at.saturating_add(1));
    if let Some(progress) = input.progress {
        record.progress = progress;
    }
    if let Some(current_step) = input.current_step.as_ref() {
        record.current_step = Some(current_step.clone());
    }
    if let Some(status) = input.status.as_ref() {
        record.status = status.clone();
    }
    if let Some(output_payload_len) = input.output_payload_len {
        record.output_payload_len = output_payload_len;
    }
    if let Some(error_message) = input.error_message.as_ref() {
        record.error_message = Some(error_message.clone());
    }
    if let Some(started_at) = input.started_at {
        record.started_at = Some(started_at);
    }
    if let Some(completed_at) = input.completed_at {
        record.completed_at = Some(completed_at);
    }
}

#[async_trait]
impl ExecutionStateStore for CosmosStateStore {
    fn backend_name(&self) -> &'static str {
        "cosmos"
    }

    async fn create_run(
        &self,
        input: CreateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        let now = chrono::Utc::now().timestamp_millis();
        let expires_at = input
            .expires_at
            .unwrap_or_else(|| now.saturating_add(self.default_ttl_seconds.saturating_mul(1_000)));
        let record = ExecutionRunRecord {
            id: input.id,
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
            expires_at: Some(expires_at),
            user_id: input.user_id,
            technical_user_id: input.technical_user_id,
            app_id: input.app_id,
            created_at: now,
            updated_at: now,
        };
        let document = self.run_document(record.clone(), now);
        match self
            .client
            .create_document(&self.runs_container, &record.app_id, &document)
            .await
            .map_err(map_error)?
        {
            MutationOutcome::Applied => Ok(record),
            MutationOutcome::Conflict => Err(StateStoreError::Database(format!(
                "execution run '{}' already exists in app '{}'",
                record.id, record.app_id
            ))),
            outcome => Err(StateStoreError::Database(format!(
                "unexpected Cosmos run create result: {outcome:?}"
            ))),
        }
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        let document = match self.find_run_document(run_id).await? {
            Some(document) => Some(document),
            None => self.import_source_run(run_id, None).await?,
        };
        Ok(document.and_then(|document| {
            (!Self::is_expired(
                document.record.expires_at,
                chrono::Utc::now().timestamp_millis(),
            ))
            .then_some(document.record)
        }))
    }

    async fn get_run_for_app(
        &self,
        run_id: &str,
        app_id: &str,
    ) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        let document = self.ensure_run_for_app_document(run_id, app_id).await?;
        Ok(document.and_then(|document| {
            (!Self::is_expired(
                document.record.expires_at,
                chrono::Utc::now().timestamp_millis(),
            ))
            .then_some(document.record)
        }))
    }

    async fn update_run(
        &self,
        run_id: &str,
        input: UpdateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        for _ in 0..UPDATE_ATTEMPTS {
            let mut document = self
                .find_run_document(run_id)
                .await?
                .ok_or(StateStoreError::NotFound)?;
            let now = chrono::Utc::now().timestamp_millis();
            if Self::is_expired(document.record.expires_at, now) {
                return Err(StateStoreError::NotFound);
            }
            if document.record.status.is_terminal() {
                return Ok(document.record);
            }
            if document.bound_job_id.is_some() {
                return Err(StateStoreError::LeaseConflict(
                    "leased queue run requires ownership proof".to_string(),
                ));
            }
            apply_update(&mut document.record, &input, now);
            document.ttl = ttl_seconds(document.record.expires_at, now);

            match self
                .client
                .replace_document(
                    &self.runs_container,
                    run_id,
                    &document.record.app_id,
                    &document,
                    document.etag.as_deref(),
                )
                .await
                .map_err(map_error)?
            {
                MutationOutcome::Applied => return Ok(document.record),
                MutationOutcome::NotFound => return Err(StateStoreError::NotFound),
                MutationOutcome::PreconditionFailed | MutationOutcome::Conflict => continue,
            }
        }
        Err(StateStoreError::Database(format!(
            "execution run '{run_id}' changed concurrently {UPDATE_ATTEMPTS} times"
        )))
    }

    async fn claim_run_lease(
        &self,
        run_id: &str,
        app_id: &str,
        job_id: &str,
        lease_token: &str,
        lease_duration_ms: i64,
    ) -> Result<RunLeaseClaim, StateStoreError> {
        if job_id.is_empty() || lease_token.is_empty() || lease_duration_ms <= 0 {
            return Err(StateStoreError::LeaseConflict(
                "invalid execution lease claim".to_string(),
            ));
        }

        for _ in 0..UPDATE_ATTEMPTS {
            let mut document = self
                .ensure_run_for_app_document(run_id, app_id)
                .await?
                .ok_or(StateStoreError::NotFound)?;
            let now = chrono::Utc::now().timestamp_millis();
            if Self::is_expired(document.record.expires_at, now) {
                return Err(StateStoreError::NotFound);
            }
            if document.record.status.is_terminal() {
                return Ok(RunLeaseClaim::Terminal {
                    run: document.record,
                });
            }
            if document
                .bound_job_id
                .as_deref()
                .is_some_and(|bound| bound != job_id)
            {
                return Err(StateStoreError::LeaseConflict(
                    "run is bound to a different broker job".to_string(),
                ));
            }
            if let Some(lease) = document.lease.as_ref()
                && lease.token != lease_token
                && lease.expires_at > now
            {
                return Ok(RunLeaseClaim::Busy {
                    run: document.record,
                    expires_at: lease.expires_at,
                });
            }

            let expires_at = now.saturating_add(lease_duration_ms);
            document.bound_job_id = Some(job_id.to_string());
            document.lease = Some(RunLease {
                token: lease_token.to_string(),
                expires_at,
            });
            document.record.status = RunStatus::Running;
            document.record.started_at.get_or_insert(now);
            document.record.updated_at = now.max(document.record.updated_at.saturating_add(1));
            document.ttl = ttl_seconds(document.record.expires_at, now);

            match self
                .client
                .replace_document(
                    &self.runs_container,
                    run_id,
                    app_id,
                    &document,
                    document.etag.as_deref(),
                )
                .await
                .map_err(map_error)?
            {
                MutationOutcome::Applied => {
                    return Ok(RunLeaseClaim::Acquired {
                        run: document.record,
                        expires_at,
                    });
                }
                MutationOutcome::NotFound => return Err(StateStoreError::NotFound),
                MutationOutcome::PreconditionFailed | MutationOutcome::Conflict => continue,
            }
        }
        Err(StateStoreError::LeaseConflict(format!(
            "run '{run_id}' lease changed concurrently {UPDATE_ATTEMPTS} times"
        )))
    }

    async fn update_run_with_lease(
        &self,
        run_id: &str,
        app_id: &str,
        job_id: &str,
        lease_token: &str,
        input: UpdateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        if !input.status.as_ref().is_some_and(RunStatus::is_terminal) {
            return Err(StateStoreError::LeaseConflict(
                "lease-protected update must be terminal".to_string(),
            ));
        }

        for _ in 0..UPDATE_ATTEMPTS {
            let mut document = self
                .ensure_run_for_app_document(run_id, app_id)
                .await?
                .ok_or(StateStoreError::NotFound)?;
            if document.record.status.is_terminal() {
                return Ok(document.record);
            }
            let now = chrono::Utc::now().timestamp_millis();
            let owns_lease = document.bound_job_id.as_deref() == Some(job_id)
                && document
                    .lease
                    .as_ref()
                    .is_some_and(|lease| lease.token == lease_token && lease.expires_at > now);
            if !owns_lease {
                return Err(StateStoreError::LeaseConflict(
                    "terminal callback is not from the current delivery owner".to_string(),
                ));
            }
            apply_update(&mut document.record, &input, now);
            document.lease = None;
            document.ttl = ttl_seconds(document.record.expires_at, now);

            match self
                .client
                .replace_document(
                    &self.runs_container,
                    run_id,
                    app_id,
                    &document,
                    document.etag.as_deref(),
                )
                .await
                .map_err(map_error)?
            {
                MutationOutcome::Applied => return Ok(document.record),
                MutationOutcome::NotFound => return Err(StateStoreError::NotFound),
                MutationOutcome::PreconditionFailed | MutationOutcome::Conflict => continue,
            }
        }
        Err(StateStoreError::LeaseConflict(format!(
            "run '{run_id}' terminal update changed concurrently {UPDATE_ATTEMPTS} times"
        )))
    }

    async fn validate_run_lease(
        &self,
        run_id: &str,
        app_id: &str,
        job_id: &str,
        lease_token: &str,
    ) -> Result<(), StateStoreError> {
        let document = self
            .ensure_run_for_app_document(run_id, app_id)
            .await?
            .ok_or(StateStoreError::NotFound)?;
        let now = chrono::Utc::now().timestamp_millis();
        let owned = document.bound_job_id.as_deref() == Some(job_id)
            && document
                .lease
                .as_ref()
                .is_some_and(|lease| lease.token == lease_token && lease.expires_at > now);
        if owned && !document.record.status.is_terminal() {
            Ok(())
        } else {
            Err(StateStoreError::LeaseConflict(
                "event callback is not from the current unexpired delivery owner".to_string(),
            ))
        }
    }

    async fn list_runs_for_app(
        &self,
        app_id: &str,
        limit: i32,
        cursor: Option<&str>,
    ) -> Result<Vec<ExecutionRunRecord>, StateStoreError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 1_000) as usize;
        let now = chrono::Utc::now().timestamp_millis();
        let mut parameters = vec![QueryParameter::new("@now", now)];
        let query = if let Some(cursor) = cursor {
            let Some(cursor_record) = self.read_run_for_app_document(cursor, app_id).await? else {
                return Ok(Vec::new());
            };
            parameters.push(QueryParameter::new(
                "@cursor_created",
                cursor_record.record.created_at,
            ));
            parameters.push(QueryParameter::new("@cursor_id", cursor));
            "SELECT * FROM c WHERE (NOT IS_DEFINED(c.expires_at) OR c.expires_at > @now) \
             AND (c.created_at < @cursor_created OR \
                  (c.created_at = @cursor_created AND c.id < @cursor_id)) \
             ORDER BY c.created_at DESC, c.id DESC"
        } else {
            "SELECT * FROM c WHERE (NOT IS_DEFINED(c.expires_at) OR c.expires_at > @now) \
             ORDER BY c.created_at DESC, c.id DESC"
        };
        let documents = self
            .query_all::<RunDocument>(
                &self.runs_container,
                query,
                &parameters,
                Some(app_id),
                Some(limit),
            )
            .await?;
        Ok(documents
            .into_iter()
            .map(|document| document.record)
            .collect())
    }

    async fn delete_expired_runs(&self) -> Result<i64, StateStoreError> {
        Ok(0)
    }

    async fn push_events(&self, events: Vec<CreateEventInput>) -> Result<i32, StateStoreError> {
        if events.is_empty() {
            return Ok(0);
        }
        let accepted_count = events.len() as i32;
        let now = chrono::Utc::now().timestamp_millis();
        let mut documents = Vec::with_capacity(events.len());
        for event in events {
            let encoded = serde_json::to_vec(&event.payload)
                .map_err(|error| StateStoreError::Serialization(error.to_string()))?;
            let (payload, payload_ref) = if encoded.len() > PAYLOAD_OFFLOAD_BYTES {
                if has_canonical_event_identity(&event) && self.event_exists(&event).await? {
                    continue;
                }
                let reference = self
                    .store_large_payload(&event.run_id, &event.id, &encoded)
                    .await?;
                (Value::Null, Some(reference))
            } else {
                (event.payload, None)
            };
            let record = ExecutionEventRecord {
                id: event.id,
                run_id: event.run_id,
                sequence: event.sequence,
                event_type: event.event_type,
                payload,
                delivered: false,
                expires_at: event.expires_at,
                created_at: now,
            };
            documents.push(Self::event_document(record, payload_ref, now));
        }

        // Preserve prefix order and stop on the first failed item. Each item is
        // itself idempotent, so retrying this batch safely fills only the
        // missing suffix; concurrent unordered writes could otherwise expose
        // holes to a poller that advances by sequence.
        for document in documents {
            match self
                .client
                .create_document(&self.events_container, &document.record.run_id, &document)
                .await
            {
                // `create_document` is atomic. A conflict means another
                // request already committed this canonical ID, so the retry is
                // successful without replacing payload or delivery state.
                Ok(outcome) => accept_event_create_outcome(outcome)?,
                Err(error) => return Err(map_error(error)),
            }
        }
        Ok(accepted_count)
    }

    async fn get_events(
        &self,
        query: EventQuery,
    ) -> Result<Vec<ExecutionEventRecord>, StateStoreError> {
        if query.limit.is_some_and(|limit| limit <= 0) {
            return Ok(Vec::new());
        }
        let mut sql =
            String::from("SELECT * FROM c WHERE c.expires_at > @now AND c.sequence > @after");
        if query.only_undelivered {
            sql.push_str(" AND c.delivered = false");
        }
        sql.push_str(" ORDER BY c.sequence ASC");
        let parameters = [
            QueryParameter::new("@now", chrono::Utc::now().timestamp_millis()),
            QueryParameter::new("@after", query.after_sequence.unwrap_or(-1)),
        ];
        let documents = self
            .query_all::<EventDocument>(
                &self.events_container,
                &sql,
                &parameters,
                Some(&query.run_id),
                query.limit.map(|limit| limit.clamp(1, 1_000) as usize),
            )
            .await?;

        let mut records = Vec::with_capacity(documents.len());
        for document in documents {
            records.push(self.hydrate_event(document).await?);
        }
        Ok(records)
    }

    async fn get_max_sequence(&self, run_id: &str) -> Result<i32, StateStoreError> {
        let page = self
            .client
            .query_page::<SequenceProjection>(
                &self.events_container,
                "SELECT TOP 1 c.sequence FROM c WHERE c.expires_at > @now \
                 ORDER BY c.sequence DESC",
                &[QueryParameter::new(
                    "@now",
                    chrono::Utc::now().timestamp_millis(),
                )],
                Some(run_id),
                1,
                None,
            )
            .await
            .map_err(map_error)?;
        Ok(page
            .documents
            .first()
            .map(|document| document.sequence)
            .unwrap_or(0))
    }

    async fn mark_events_delivered(
        &self,
        run_id: &str,
        event_ids: &[String],
    ) -> Result<(), StateStoreError> {
        if event_ids.is_empty() {
            return Ok(());
        }
        for ids in event_ids.chunks(MARK_QUERY_CHUNK) {
            // Events are partitioned by run, so the run scope turns this into
            // a single-partition query instead of a cross-partition fan-out.
            let documents = self
                .query_all::<EventDocument>(
                    &self.events_container,
                    "SELECT * FROM c WHERE ARRAY_CONTAINS(@ids, c.id) AND c.expires_at > @now",
                    &[
                        QueryParameter::new("@ids", json!(ids)),
                        QueryParameter::new("@now", chrono::Utc::now().timestamp_millis()),
                    ],
                    Some(run_id),
                    Some(ids.len()),
                )
                .await?;
            let client = &self.client;
            let container = &self.events_container;
            let outcomes = stream::iter(documents.into_iter().map(|mut document| async move {
                document.record.delivered = true;
                document.ttl = ttl_seconds(
                    Some(document.record.expires_at),
                    chrono::Utc::now().timestamp_millis(),
                );
                client
                    .replace_document(
                        container,
                        &document.record.id,
                        &document.record.run_id,
                        &document,
                        document.etag.as_deref(),
                    )
                    .await
            }))
            .buffer_unordered(WRITE_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
            for outcome in outcomes {
                match outcome.map_err(map_error)? {
                    MutationOutcome::Applied
                    | MutationOutcome::NotFound
                    | MutationOutcome::PreconditionFailed => {}
                    MutationOutcome::Conflict => {
                        return Err(StateStoreError::Database(
                            "Cosmos event delivery update conflicted".to_string(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    async fn delete_expired_events(&self) -> Result<i64, StateStoreError> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_document_serializes_partition_key_and_native_ttl() {
        let store_ttl = ttl_seconds(Some(2_001), 1_000);
        let document = RunDocument {
            record: ExecutionRunRecord {
                id: "run".to_string(),
                board_id: "board".to_string(),
                version: None,
                event_id: None,
                status: RunStatus::Pending,
                mode: RunMode::Queue,
                input_payload_len: 1,
                output_payload_len: 0,
                error_message: None,
                progress: 0,
                current_step: None,
                started_at: None,
                completed_at: None,
                expires_at: Some(2_001),
                user_id: None,
                technical_user_id: None,
                app_id: "app".to_string(),
                created_at: 1_000,
                updated_at: 1_000,
            },
            ttl: store_ttl,
            etag: None,
            bound_job_id: None,
            lease: None,
        };
        let value = serde_json::to_value(document).unwrap();
        assert_eq!(value["app_id"], "app");
        assert_eq!(value["ttl"], 2);
    }

    #[test]
    fn run_updates_only_change_explicit_fields() {
        let mut record = ExecutionRunRecord {
            id: "run".to_string(),
            board_id: "board".to_string(),
            version: None,
            event_id: None,
            status: RunStatus::Pending,
            mode: RunMode::Queue,
            input_payload_len: 1,
            output_payload_len: 0,
            error_message: None,
            progress: 0,
            current_step: None,
            started_at: None,
            completed_at: None,
            expires_at: Some(10_000),
            user_id: None,
            technical_user_id: None,
            app_id: "app".to_string(),
            created_at: 1,
            updated_at: 1,
        };
        apply_update(
            &mut record,
            &UpdateRunInput {
                progress: Some(50),
                status: Some(RunStatus::Running),
                ..Default::default()
            },
            2,
        );
        assert_eq!(record.progress, 50);
        assert_eq!(record.status, RunStatus::Running);
        assert_eq!(record.updated_at, 2);
        assert_eq!(record.output_payload_len, 0);
    }

    #[test]
    fn stateless_lambda_canonical_event_create_conflict_is_a_first_write_wins_retry() {
        assert!(accept_event_create_outcome(MutationOutcome::Applied).is_ok());
        assert!(accept_event_create_outcome(MutationOutcome::Conflict).is_ok());
        assert!(accept_event_create_outcome(MutationOutcome::PreconditionFailed).is_err());
    }

    #[test]
    fn stateless_lambda_cosmos_recognizes_canonical_event_identity() {
        let mut event = CreateEventInput {
            id: String::new(),
            run_id: "run-1".into(),
            sequence: 4,
            event_type: "chunk".into(),
            payload: Value::Null,
            expires_at: 1_900_000_000_000,
        };
        let digest = blake3::hash(b"run-1:4");
        event.id = format!("evt-{}", digest.to_hex());
        assert!(has_canonical_event_identity(&event));
        event.id = "legacy".into();
        assert!(!has_canonical_event_identity(&event));
    }
}
