//! Google Cloud Firestore (Native mode) execution-state backend.
//!
//! Provision `execution-runs` and `execution-events` in the same database, each with a
//! TTL policy on `expires_at` and a composite index — `app_id ASC, created_at DESC` for
//! runs, `run_id ASC, sequence ASC` for events. Both collections need an index exemption
//! on `payload` and `payload_ref`; Firestore indexes every field it has never been told
//! not to, and an event payload is caller data with no bounded key set. Event bodies over
//! 100 KiB are offloaded to the GCS-backed `FlowLikeStore`, which keeps documents well
//! under Firestore's 1 MiB ceiling and puts a run's payloads in the same place they land
//! on DynamoDB and Cosmos.
//!
//! The stored documents differ from their Cosmos counterparts in exactly two places, both
//! forced by Firestore's type system rather than chosen: `expires_at` is written as the
//! `Timestamp` the TTL policy can collect instead of the epoch-millisecond integer the
//! records carry, and an event payload is stored as an opaque JSON string. Both are
//! reversed on read so the trait sees the same records every other backend returns.

use super::{PostgresStateStore, types::*};
use async_trait::async_trait;
use flow_like_gcp_data::firestore::{
    Document, EXPIRES_AT_FIELD, FilterOperator, FirestoreClient, FirestoreError, MutationOutcome,
    QueryFilter, QueryOrder, SortDirection, validate_collection_id,
};
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{ObjectStore, path::Path},
};
use flow_like_types::utils::constant_time_eq;
use futures::{StreamExt, stream};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::Arc;

const DEFAULT_RUNS_COLLECTION: &str = "execution-runs";
const DEFAULT_EVENTS_COLLECTION: &str = "execution-events";
const DEFAULT_TTL_SECONDS: i64 = 86_400;
const PAYLOAD_PREFIX: &str = "polling";
const PAYLOAD_FIELD: &str = "payload";
const QUERY_PAGE_SIZE: i32 = 1_000;
const WRITE_CONCURRENCY: usize = 16;
const UPDATE_ATTEMPTS: usize = 3;
/// Ceiling on the pages one listing may walk. Expiry and delivery are filtered after the
/// query rather than inside it (see `collect_filtered`), so a caller asking for one
/// surviving document could otherwise page a whole collection without ever reaching its
/// limit.
const MAX_SCAN_PAGES: usize = 10_000;

fn has_canonical_event_identity(event: &CreateEventInput) -> bool {
    let digest = blake3::hash(format!("{}:{}", event.run_id, event.sequence).as_bytes());
    event.id == format!("evt-{}", digest.to_hex())
}

#[derive(Clone)]
pub struct FirestoreStateStore {
    client: FirestoreClient,
    content_store: Option<Arc<FlowLikeStore>>,
    runs_collection: String,
    events_collection: String,
    default_ttl_seconds: i64,
    source_run_store: Option<PostgresStateStore>,
}

impl std::fmt::Debug for FirestoreStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirestoreStateStore")
            .field("client", &self.client)
            .field("runs_collection", &self.runs_collection)
            .field("events_collection", &self.events_collection)
            .field("has_content_store", &self.content_store.is_some())
            .finish()
    }
}

/// The stored form of a run.
///
/// Cosmos carries its concurrency token inside the body as `_etag`; Firestore keeps it in
/// the response envelope as `updateTime`, so it is attached here after the read and never
/// serialized. Every conditional write in this file passes it back — that precondition is
/// the only thing standing between two workers and the same run.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RunDocument {
    #[serde(flatten)]
    record: ExecutionRunRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bound_job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lease: Option<RunLease>,
    #[serde(skip)]
    update_time: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload_ref: Option<String>,
    #[serde(skip)]
    update_time: Option<String>,
}

impl FirestoreStateStore {
    pub fn from_env(
        content_store: Option<Arc<FlowLikeStore>>,
        source_db: Option<Arc<DatabaseConnection>>,
    ) -> Result<Self, StateStoreError> {
        let client = FirestoreClient::from_env().map_err(map_error)?;
        let runs_collection = optional_env("FIRESTORE_RUNS_COLLECTION", DEFAULT_RUNS_COLLECTION);
        let events_collection =
            optional_env("FIRESTORE_EVENTS_COLLECTION", DEFAULT_EVENTS_COLLECTION);
        // Validate here as well as inside the client: a collection name that only fails on
        // the first write turns a deployment typo into a runtime outage halfway through a
        // rollout, and the TTL policy and composite indexes are declared against these
        // exact names in Terraform.
        validate_collection_id(&runs_collection).map_err(map_error)?;
        validate_collection_id(&events_collection).map_err(map_error)?;
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
            runs_collection,
            events_collection,
            default_ttl_seconds,
            source_run_store: source_db.map(PostgresStateStore::new),
        })
    }

    fn run_document(record: ExecutionRunRecord) -> RunDocument {
        RunDocument {
            record,
            bound_job_id: None,
            lease: None,
            update_time: None,
        }
    }

    fn event_document(record: ExecutionEventRecord, payload_ref: Option<String>) -> EventDocument {
        EventDocument {
            record,
            payload_ref,
            update_time: None,
        }
    }

    fn is_expired(expires_at: Option<i64>, now_ms: i64) -> bool {
        expires_at.is_some_and(|expires_at| expires_at <= now_ms)
    }

    /// Firestore has no partition key, so the run ID alone addresses the document and the
    /// cross-partition query Cosmos needs here collapses into a point read. The
    /// "same ID in two partitions" failure it guards against cannot occur.
    async fn read_run_document(
        &self,
        run_id: &str,
    ) -> Result<Option<RunDocument>, StateStoreError> {
        let document = self
            .client
            .read_document::<Value>(&self.runs_collection, run_id)
            .await
            .map_err(map_error)?;
        document.map(run_from_document).transpose()
    }

    /// The app scoping Cosmos gets for free from the partition key has to be spelled out
    /// on Firestore: without this comparison any caller holding a run ID could read that
    /// run through any app it happens to have access to.
    async fn read_run_for_app_document(
        &self,
        run_id: &str,
        app_id: &str,
    ) -> Result<Option<RunDocument>, StateStoreError> {
        Ok(self
            .read_run_document(run_id)
            .await?
            .filter(|document| document.record.app_id == app_id))
    }

    /// Async invocation creates the canonical audit row in PostgreSQL before
    /// dispatch. Import that row exactly once when Firestore is selected for live
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
        if source_run_expired(&record, chrono::Utc::now().timestamp_millis()) {
            // Reads already mask expired documents; skipping the import also
            // avoids re-creating one that native TTL would delete again.
            return Ok(None);
        }
        let expires_at = record.expires_at;
        let document = Self::run_document(record.clone());
        match self
            .client
            .create_document(&self.runs_collection, &record.id, &document, expires_at)
            .await
            .map_err(map_error)?
        {
            MutationOutcome::Applied | MutationOutcome::Conflict => {}
            outcome => {
                return Err(StateStoreError::Database(format!(
                    "unexpected Firestore source-run import result: {outcome:?}"
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

    async fn read_event_document(
        &self,
        event_id: &str,
    ) -> Result<Option<EventDocument>, StateStoreError> {
        let document = self
            .client
            .read_document::<Value>(&self.events_collection, event_id)
            .await
            .map_err(map_error)?;
        document.map(event_from_document).transpose()
    }

    async fn store_large_payload(
        &self,
        run_id: &str,
        event_id: &str,
        body: &[u8],
    ) -> Result<String, StateStoreError> {
        let store = self.content_store.as_ref().ok_or_else(|| {
            StateStoreError::Configuration(
                "Firestore event payload exceeds 100 KiB but no content FlowLikeStore was supplied"
                    .to_string(),
            )
        })?;
        let run_hash = blake3::hash(run_id.as_bytes()).to_hex();
        let event_hash = blake3::hash(event_id.as_bytes()).to_hex();
        // Include the content hash in the immutable reference. Otherwise a
        // conflicting retry could overwrite the object for an already-created
        // event before Firestore detects that its stable ID was reused.
        let payload_hash = blake3::hash(body).to_hex();
        let path = Path::from(format!(
            "{PAYLOAD_PREFIX}/{run_hash}/{event_hash}-{payload_hash}.json"
        ));
        store
            .as_generic()
            .put(&path, body.to_vec().into())
            .await
            .map_err(|error| {
                StateStoreError::Database(format!("Cloud Storage payload write failed: {error}"))
            })?;
        Ok(format!("store://{path}"))
    }

    async fn fetch_large_payload(&self, reference: &str) -> Result<Value, StateStoreError> {
        let store = self.content_store.as_ref().ok_or_else(|| {
            StateStoreError::Configuration(
                "Firestore event references object payload storage but no content FlowLikeStore was supplied"
                    .to_string(),
            )
        })?;
        let raw_path = reference.strip_prefix("store://").ok_or_else(|| {
            StateStoreError::Serialization("invalid Firestore event payload reference".to_string())
        })?;
        let result = store
            .as_generic()
            .get(&Path::from(raw_path))
            .await
            .map_err(|error| {
                StateStoreError::Database(format!("Cloud Storage payload read failed: {error}"))
            })?;
        let bytes = result.bytes().await.map_err(|error| {
            StateStoreError::Database(format!("Cloud Storage payload stream failed: {error}"))
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

    /// Page a query until `limit` documents have survived `keep`, or the result set is
    /// exhausted.
    ///
    /// Expiry and delivery state are decided here rather than in the query on purpose.
    /// Firestore serves a query only from an index that covers it, so pushing either of
    /// them down would demand a composite index beyond the two Terraform declares — and
    /// an inequality on `expires_at` would additionally drop every run that has no expiry
    /// at all, because a document missing the field matches no range filter.
    async fn collect_filtered<T: Send>(
        &self,
        collection: &str,
        filters: &[QueryFilter],
        order_by: &[QueryOrder],
        limit: usize,
        decode: fn(Document<Value>) -> Result<T, StateStoreError>,
        keep: &(dyn Fn(&T) -> bool + Sync),
    ) -> Result<Vec<T>, StateStoreError> {
        let mut kept: Vec<T> = Vec::new();
        let mut continuation: Option<String> = None;
        for _ in 0..MAX_SCAN_PAGES {
            let remaining = limit.saturating_sub(kept.len());
            if remaining == 0 {
                break;
            }
            let page = self
                .client
                .query_page::<Value>(
                    collection,
                    filters,
                    order_by,
                    remaining.min(QUERY_PAGE_SIZE as usize) as i32,
                    continuation.as_deref(),
                )
                .await
                .map_err(map_error)?;
            continuation = page.continuation;
            for document in page.documents {
                let decoded = decode(document)?;
                if keep(&decoded) {
                    kept.push(decoded);
                    if kept.len() >= limit {
                        break;
                    }
                }
            }
            if continuation.is_none() {
                break;
            }
        }
        Ok(kept)
    }

    /// Flip one event to delivered under its `updateTime`. A lost race or a TTL collection
    /// between the read and the write is not an error: the flag is delivery bookkeeping,
    /// and re-delivering an event the poller already has is cheaper than failing the batch.
    async fn mark_delivered(
        &self,
        run_id: &str,
        event_id: &str,
        now_ms: i64,
    ) -> Result<(), StateStoreError> {
        let Some(mut document) = self.read_event_document(event_id).await? else {
            return Ok(());
        };
        if document.record.run_id != run_id || document.record.expires_at <= now_ms {
            return Ok(());
        }
        document.record.delivered = true;
        let stored = stored_event(&document)?;
        match self
            .client
            .replace_document(
                &self.events_collection,
                event_id,
                &stored,
                Some(document.record.expires_at),
                document.update_time.as_deref(),
            )
            .await
            .map_err(map_error)?
        {
            MutationOutcome::Applied
            | MutationOutcome::NotFound
            | MutationOutcome::PreconditionFailed => Ok(()),
            MutationOutcome::Conflict => Err(StateStoreError::Database(
                "Firestore event delivery update conflicted".to_string(),
            )),
        }
    }
}

fn optional_env(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn map_error(error: FirestoreError) -> StateStoreError {
    match error {
        FirestoreError::Configuration(message) => StateStoreError::Configuration(message),
        FirestoreError::Authentication(message) | FirestoreError::Transport(message) => {
            StateStoreError::Connection(message)
        }
        FirestoreError::Serialization(message) => StateStoreError::Serialization(message),
        error @ FirestoreError::Service { .. } => StateStoreError::Database(error.to_string()),
    }
}

fn accept_event_create_outcome(outcome: MutationOutcome) -> Result<(), StateStoreError> {
    match outcome {
        MutationOutcome::Applied | MutationOutcome::Conflict => Ok(()),
        other => Err(StateStoreError::Database(format!(
            "unexpected Firestore event create result: {other:?}"
        ))),
    }
}

/// Encode an event for storage.
///
/// The payload is stored as an opaque JSON string, which is the one deviation from the
/// Cosmos document shape and is forced twice over. Firestore refuses an array nested
/// directly inside another array, so a perfectly ordinary caller payload such as a matrix
/// or a GeoJSON ring would be rejected at write time; and a structured payload creates one
/// automatically indexed field path per key it happens to contain, which no fixed set of
/// Terraform index exemptions can cover. A string is one exemptable field and stores
/// anything `serde_json` can represent.
fn stored_event(document: &EventDocument) -> Result<Value, StateStoreError> {
    let mut value = serde_json::to_value(document)
        .map_err(|error| StateStoreError::Serialization(error.to_string()))?;
    let map = value.as_object_mut().ok_or_else(|| {
        StateStoreError::Serialization(
            "execution event document must serialize to an object".to_string(),
        )
    })?;
    let payload = map.remove(PAYLOAD_FIELD).unwrap_or(Value::Null);
    let encoded = serde_json::to_string(&payload)
        .map_err(|error| StateStoreError::Serialization(error.to_string()))?;
    map.insert(PAYLOAD_FIELD.to_string(), Value::String(encoded));
    Ok(value)
}

fn run_from_document(document: Document<Value>) -> Result<RunDocument, StateStoreError> {
    let mut value = document.value;
    restore_expires_at(object_mut(&mut value)?)?;
    let mut run: RunDocument = serde_json::from_value(value)
        .map_err(|error| StateStoreError::Serialization(error.to_string()))?;
    run.update_time = document.update_time;
    Ok(run)
}

fn event_from_document(document: Document<Value>) -> Result<EventDocument, StateStoreError> {
    let mut value = document.value;
    let map = object_mut(&mut value)?;
    restore_expires_at(map)?;
    match map.remove(PAYLOAD_FIELD) {
        Some(Value::String(encoded)) => {
            let payload: Value = serde_json::from_str(&encoded)
                .map_err(|error| StateStoreError::Serialization(error.to_string()))?;
            map.insert(PAYLOAD_FIELD.to_string(), payload);
        }
        // Anything else is a payload written before the string encoding existed; it is
        // already a usable value and putting it back untouched keeps such a document
        // readable instead of failing the whole poll.
        Some(payload) => {
            map.insert(PAYLOAD_FIELD.to_string(), payload);
        }
        None => {}
    }
    let mut event: EventDocument = serde_json::from_value(value)
        .map_err(|error| StateStoreError::Serialization(error.to_string()))?;
    event.update_time = document.update_time;
    Ok(event)
}

fn object_mut(value: &mut Value) -> Result<&mut Map<String, Value>, StateStoreError> {
    value.as_object_mut().ok_or_else(|| {
        StateStoreError::Serialization(
            "Firestore returned a document that was not an object".to_string(),
        )
    })
}

/// Turn the TTL field back into the epoch milliseconds the records are typed with.
///
/// Firestore's TTL policy only ever collects a `Timestamp`, so the client writes
/// `expires_at` as one and the integer the record serialized is overwritten on the way
/// out. Reading it back as a number is what keeps `expires_at` a single value rather than
/// a stored pair that can drift apart.
/// Drops the TTL sibling before the document is deserialized into a record.
///
/// [`EXPIRES_AT_FIELD`] (`expires_at_ts`) is infrastructure metadata, not part
/// of any record: it exists solely so the Firestore TTL policy configured in
/// `modules/gcp/state` has an absolute `Timestamp` to watch. The record keeps
/// its own epoch-milliseconds integer `expires_at`, which serializes and
/// deserializes untouched, so nothing has to be converted back here - removing
/// the sibling is the whole job.
///
/// An earlier revision had the writer OVERWRITE the integer with the timestamp
/// and this function convert it back. That coupling is what let the Terraform
/// policy and the Rust constant drift apart in opposite directions during an
/// audit pass without either side failing; the sibling design cannot express
/// that bug, because the record's field is never the TTL field.
fn restore_expires_at(map: &mut Map<String, Value>) -> Result<(), StateStoreError> {
    map.remove(EXPIRES_AT_FIELD);
    Ok(())
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
impl ExecutionStateStore for FirestoreStateStore {
    fn backend_name(&self) -> &'static str {
        "firestore"
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
            run_variant: input.run_variant,
            variant_name: input.variant_name,
            shadow_of_run_id: input.shadow_of_run_id,
            regression_run_id: input.regression_run_id,
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
        let document = Self::run_document(record.clone());
        match self
            .client
            .create_document(
                &self.runs_collection,
                &record.id,
                &document,
                Some(expires_at),
            )
            .await
            .map_err(map_error)?
        {
            MutationOutcome::Applied => Ok(record),
            MutationOutcome::Conflict => Err(StateStoreError::Database(format!(
                "execution run '{}' already exists in app '{}'",
                record.id, record.app_id
            ))),
            outcome => Err(StateStoreError::Database(format!(
                "unexpected Firestore run create result: {outcome:?}"
            ))),
        }
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        let document = match self.read_run_document(run_id).await? {
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
                .read_run_document(run_id)
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

            match self
                .client
                .replace_document(
                    &self.runs_collection,
                    run_id,
                    &document,
                    document.record.expires_at,
                    document.update_time.as_deref(),
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
                && !constant_time_eq(lease.token.as_bytes(), lease_token.as_bytes())
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

            // `updateTime` is what makes this a claim rather than an announcement: two
            // workers reading the same unleased run both write, and the one whose read
            // was stale is refused instead of overwriting the winner's lease.
            match self
                .client
                .replace_document(
                    &self.runs_collection,
                    run_id,
                    &document,
                    document.record.expires_at,
                    document.update_time.as_deref(),
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
                && document.lease.as_ref().is_some_and(|lease| {
                    constant_time_eq(lease.token.as_bytes(), lease_token.as_bytes())
                        && lease.expires_at > now
                });
            if !owns_lease {
                return Err(StateStoreError::LeaseConflict(
                    "terminal callback is not from the current delivery owner".to_string(),
                ));
            }
            apply_update(&mut document.record, &input, now);
            document.lease = None;

            match self
                .client
                .replace_document(
                    &self.runs_collection,
                    run_id,
                    &document,
                    document.record.expires_at,
                    document.update_time.as_deref(),
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
            && document.lease.as_ref().is_some_and(|lease| {
                constant_time_eq(lease.token.as_bytes(), lease_token.as_bytes())
                    && lease.expires_at > now
            });
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

        let mut filters = vec![QueryFilter::new("app_id", FilterOperator::Equal, app_id)];
        let mut boundary: Option<(i64, String)> = None;
        if let Some(cursor) = cursor {
            let Some(document) = self.read_run_for_app_document(cursor, app_id).await? else {
                return Ok(Vec::new());
            };
            // Firestore has one `startAt` cursor and it belongs to the paging loop, so the
            // caller's run-ID cursor is expressed the way Cosmos expresses it: narrow to
            // the cursor's `created_at` and drop the tie group up to and including the
            // cursor itself. The client appends `__name__` to the sort, and within one
            // collection that orders by document ID, so the tie-break matches the
            // `created_at DESC, id DESC` ordering the Cosmos query states outright.
            filters.push(QueryFilter::new(
                "created_at",
                FilterOperator::LessThanOrEqual,
                document.record.created_at,
            ));
            boundary = Some((document.record.created_at, cursor.to_string()));
        }
        let order_by = [QueryOrder::new("created_at", SortDirection::Descending)];
        let keep = |document: &RunDocument| {
            if Self::is_expired(document.record.expires_at, now) {
                return false;
            }
            match boundary.as_ref() {
                // Strictly after the cursor in `created_at DESC, id DESC` order is
                // strictly less than it as a tuple.
                Some((created_at, id)) => {
                    (document.record.created_at, &document.record.id) < (*created_at, id)
                }
                None => true,
            }
        };

        let documents = self
            .collect_filtered(
                &self.runs_collection,
                &filters,
                &order_by,
                limit,
                run_from_document,
                &keep,
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
                if has_canonical_event_identity(&event)
                    && self.read_event_document(&event.id).await?.is_some()
                {
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
            documents.push(Self::event_document(record, payload_ref));
        }

        // Preserve prefix order and stop on the first failed item. Each item is
        // itself idempotent, so retrying this batch safely fills only the
        // missing suffix; concurrent unordered writes could otherwise expose
        // holes to a poller that advances by sequence. A `:commit` batch would be
        // atomic but could not answer a conflict with "same content, already stored",
        // which is the check that makes a retried batch free.
        for document in documents {
            let stored = stored_event(&document)?;
            match self
                .client
                .create_document(
                    &self.events_collection,
                    &document.record.id,
                    &stored,
                    Some(document.record.expires_at),
                )
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
        let now = chrono::Utc::now().timestamp_millis();
        let limit = query
            .limit
            .map(|limit| limit.clamp(1, 1_000) as usize)
            .unwrap_or(usize::MAX);
        let filters = [
            QueryFilter::new("run_id", FilterOperator::Equal, query.run_id.as_str()),
            QueryFilter::new(
                "sequence",
                FilterOperator::GreaterThan,
                query.after_sequence.unwrap_or(-1),
            ),
        ];
        let order_by = [QueryOrder::new("sequence", SortDirection::Ascending)];
        let only_undelivered = query.only_undelivered;
        let keep = move |document: &EventDocument| {
            document.record.expires_at > now && !(only_undelivered && document.record.delivered)
        };

        let documents = self
            .collect_filtered(
                &self.events_collection,
                &filters,
                &order_by,
                limit,
                event_from_document,
                &keep,
            )
            .await?;

        let mut records = Vec::with_capacity(documents.len());
        for document in documents {
            records.push(self.hydrate_event(document).await?);
        }
        Ok(records)
    }

    async fn get_max_sequence(&self, run_id: &str) -> Result<i32, StateStoreError> {
        // Descending order over the `run_id ASC, sequence ASC` index: Firestore serves a
        // sort in the exact reverse of an index as readily as in its own direction, and
        // `run_id` is pinned by equality, so no second index is needed.
        //
        // Unlike Cosmos this does not exclude expired events. Firestore's TTL sweep runs
        // on its own schedule, so an expired-but-uncollected event would be invisible to a
        // filtered query while its document ID still exists — and the only use of this
        // value is to pick the next sequence number, where counting a doomed event is the
        // safe direction and skipping one hands out a number twice.
        let page = self
            .client
            .query_page::<Value>(
                &self.events_collection,
                &[QueryFilter::new("run_id", FilterOperator::Equal, run_id)],
                &[QueryOrder::new("sequence", SortDirection::Descending)],
                1,
                None,
            )
            .await
            .map_err(map_error)?;
        let Some(document) = page.documents.first() else {
            return Ok(0);
        };
        let sequence = document
            .value
            .get("sequence")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                StateStoreError::Serialization(format!(
                    "execution event '{}' has no numeric sequence",
                    document.id
                ))
            })?;
        i32::try_from(sequence).map_err(|_| {
            StateStoreError::Serialization(format!(
                "execution event '{}' has an out-of-range sequence {sequence}",
                document.id
            ))
        })
    }

    async fn mark_events_delivered(
        &self,
        run_id: &str,
        event_ids: &[String],
    ) -> Result<(), StateStoreError> {
        if event_ids.is_empty() {
            return Ok(());
        }
        // The event ID is the document ID, so this is a fan-out of point reads. The Cosmos
        // implementation chunks because it has to bind the IDs into one `ARRAY_CONTAINS`
        // query; here only the write concurrency needs a ceiling.
        let now = chrono::Utc::now().timestamp_millis();
        // Built eagerly rather than with `.map(|id| self.mark_delivered(id, now))`.
        // A closure returning a future that borrows both `self` and the iterator item
        // is higher-ranked over that item's lifetime, and `#[async_trait]` boxes this
        // method's future at one concrete lifetime — the two cannot be reconciled and
        // rustc rejects it with "implementation of `FnOnce` is not general enough".
        // Constructing the futures in a plain loop pins every lifetime up front.
        let mut writes = Vec::with_capacity(event_ids.len());
        for event_id in event_ids {
            writes.push(self.mark_delivered(run_id, event_id.as_str(), now));
        }
        let outcomes = stream::iter(writes)
            .buffer_unordered(WRITE_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        for outcome in outcomes {
            outcome?;
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
    use flow_like_gcp_data::firestore::timestamp_value;

    fn run_record() -> ExecutionRunRecord {
        ExecutionRunRecord {
            id: "run".to_string(),
            board_id: "board".to_string(),
            version: None,
            event_id: None,
            status: RunStatus::Pending,
            mode: RunMode::Queue,
            run_variant: RunVariant::Primary,
            variant_name: None,
            shadow_of_run_id: None,
            regression_run_id: None,
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
        }
    }

    fn event_document(payload: Value) -> EventDocument {
        FirestoreStateStore::event_document(
            ExecutionEventRecord {
                id: "event".to_string(),
                run_id: "run".to_string(),
                sequence: 1,
                event_type: "progress".to_string(),
                payload,
                delivered: false,
                expires_at: 2_001,
                created_at: 1_000,
            },
            None,
        )
    }

    fn document(value: Value) -> Document<Value> {
        Document {
            id: "event".to_string(),
            create_time: None,
            update_time: Some("2026-08-15T00:00:00.000000Z".to_string()),
            value,
        }
    }

    #[test]
    fn run_document_keeps_the_indexed_fields_at_the_top_level() {
        let value = serde_json::to_value(FirestoreStateStore::run_document(run_record())).unwrap();
        assert_eq!(value["app_id"], "app");
        assert_eq!(value["created_at"], 1_000);
        assert!(value.get("update_time").is_none());
    }

    #[test]
    fn nested_arrays_survive_the_stored_payload_encoding() {
        let payload = serde_json::json!({ "matrix": [[1, 2], [3, 4]] });
        let stored = stored_event(&event_document(payload.clone())).unwrap();
        assert!(stored[PAYLOAD_FIELD].is_string());

        let restored = event_from_document(document(stored)).unwrap();
        assert_eq!(restored.record.payload, payload);
        assert_eq!(restored.record.expires_at, 2_001);
        assert_eq!(
            restored.update_time.as_deref(),
            Some("2026-08-15T00:00:00.000000Z")
        );
    }

    #[test]
    fn the_ttl_sibling_is_dropped_and_the_record_integer_is_authoritative() {
        let mut stored = stored_event(&event_document(Value::Null)).unwrap();
        // Exactly what the client ADDS on write, next to - never over - the
        // record's own integer `expires_at`. The two deliberately disagree here
        // so a reader that still preferred the timestamp would be caught.
        let ttl = timestamp_value(1_764_547_200_123).unwrap();
        stored[EXPIRES_AT_FIELD] = ttl["timestampValue"].clone();
        assert_eq!(stored["expires_at"], Value::from(2_001));

        let restored = event_from_document(document(stored)).unwrap();
        assert_eq!(restored.record.expires_at, 2_001);
    }

    #[test]
    fn run_updates_only_change_explicit_fields() {
        let mut record = run_record();
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
        // A stale clock cannot move `updated_at` backwards; it bumps past the
        // stored value so OCC re-reads always observe a change.
        assert_eq!(record.updated_at, 1_001);
        assert_eq!(record.output_payload_len, 0);

        apply_update(&mut record, &UpdateRunInput::default(), 2_000);
        assert_eq!(record.updated_at, 2_000);
    }

    #[test]
    fn stateless_lambda_canonical_event_create_conflict_is_a_first_write_wins_retry() {
        assert!(accept_event_create_outcome(MutationOutcome::Applied).is_ok());
        assert!(accept_event_create_outcome(MutationOutcome::Conflict).is_ok());
        assert!(accept_event_create_outcome(MutationOutcome::PreconditionFailed).is_err());
    }

    #[test]
    fn stateless_lambda_firestore_recognizes_canonical_event_identity() {
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
