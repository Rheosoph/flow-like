//! PostgreSQL state store implementation using SeaORM
//!
//! This backend uses the existing Prisma-generated schema via SeaORM entities.
//! TTL cleanup is manual - call `delete_expired_runs/events` periodically.
//!
//! Event payloads over [`PAYLOAD_OFFLOAD_BYTES`] are staged on the content
//! store and referenced by `ExecutionEvent.payloadRef`, the same claim check
//! the DynamoDB and Cosmos backends run. Aurora DSQL rejects any text or jsonb
//! value over 1 MiB, and an execution payload has no upper bound.

use super::types::*;
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{Error as ObjectStoreError, ObjectStore, path::Path},
};
use futures::{StreamExt, TryStreamExt, stream};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseConnection, DbErr, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Select, Set, sea_query::OnConflict,
};
use std::{collections::HashSet, sync::Arc};

use crate::db::batch::{DEFAULT_WRITE_BYTES, chunk_by_rows_and_bytes_with, insert_chunks};
use crate::db::{
    DbDialect, RetryPolicy, delete_in_batches, delete_in_batches_by_tuple, retry_transaction,
};
use crate::entity::{
    execution_event, execution_run, execution_run_caller_app,
    sea_orm_active_enums::{
        RunMode as EntityRunMode, RunStatus as EntityRunStatus, RunVariant as EntityRunVariant,
    },
};

/// Rows per transaction for execution rows, whose JSONB payloads make them
/// heavier than the default chunk.
const EXECUTION_WRITE_CHUNK: usize = 500;
/// Expired runs removed per transaction once their children are gone.
const RUN_DELETE_PAGE: usize = 250;
/// Transactions one cleanup call may spend per table; the maintenance
/// schedule finishes a larger backlog over several calls.
const MAX_CHUNKS_PER_CLEANUP: usize = 200;
/// Staging prefix for offloaded event payloads. `tmp/` is this codebase's
/// convention for objects that belong to one run rather than to an app, and
/// `polling` keeps the DynamoDB/Cosmos name so the artifact is recognisable.
const STAGED_PAYLOAD_PREFIX: &str = "tmp/polling";
/// Object deletes in flight while a cleanup page drains.
const STAGED_DELETE_CONCURRENCY: usize = 16;
/// Objects one age sweep may walk before it hands the rest to the next call.
/// Every stale object it did reach is deleted, so the prefix shrinks and the
/// next call reaches further; a run that stops early says so in its report.
const STAGED_SWEEP_MAX_OBJECTS: u64 = 200_000;
/// Stale objects buffered before a delete flush, so neither the listing nor
/// the delete set is held whole in memory.
const STAGED_SWEEP_PAGE: usize = 1_000;

/// What one [`PostgresStateStore::drain_offloaded_events`] pass achieved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DrainOutcome {
    rows: u64,
    /// The page budget ran out while offloaded rows still matched; the rest
    /// still holds objects nothing else may delete.
    stopped_early: bool,
}

/// Why a staged payload could not be served.
///
/// `Gone` is a final answer about the object, so the read may degrade to a
/// placeholder. `Transient` says nothing about it: degrading it would let the
/// poll handler mark the event delivered and lose a payload that was there the
/// whole time.
#[derive(Debug)]
enum StagedReadError {
    Gone(String),
    Transient(String),
}

#[derive(Debug, Clone)]
pub struct PostgresStateStore {
    db: Arc<DatabaseConnection>,
    dialect: DbDialect,
    content_store: Option<Arc<FlowLikeStore>>,
}

impl PostgresStateStore {
    /// A store for single-row work such as [`Self::mirror_run_update`]; bulk
    /// cleanup callers pass the resolved dialect through [`Self::with_dialect`].
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self::with_dialect(db, DbDialect::default())
    }

    pub fn with_dialect(db: Arc<DatabaseConnection>, dialect: DbDialect) -> Self {
        Self {
            db,
            dialect,
            content_store: None,
        }
    }

    /// The store that carries event payloads too large for a row. Only a
    /// deployment that pushes events needs it, so it stays optional the way it
    /// is on Cosmos and Firestore.
    pub fn with_content_store(mut self, content_store: Arc<FlowLikeStore>) -> Self {
        self.content_store = Some(content_store);
        self
    }

    /// Stage one oversized payload and return its reference and stored size.
    /// The path is content addressed, so a retried push of the same event
    /// rewrites the same object rather than leaving a second one behind.
    async fn stage_payload(
        &self,
        run_id: &str,
        event_id: &str,
        body: Vec<u8>,
    ) -> Result<Option<(String, usize)>, StateStoreError> {
        let Some(store) = self.content_store.as_ref() else {
            tracing::warn!(
                run_id,
                event_id,
                bytes = body.len(),
                "No content store configured; the oversized event payload stays in the row"
            );
            return Ok(None);
        };

        let bytes = body.len();
        let path = staged_payload_path(run_id, event_id, &body);
        store
            .as_generic()
            .put(&path, body.into())
            .await
            .map_err(|error| {
                StateStoreError::Database(format!(
                    "Staging the payload of event '{event_id}' at '{path}' failed: {error}"
                ))
            })?;

        Ok(Some((format!("store://{path}"), bytes)))
    }

    /// The ids of `candidates` that are already stored. One query, so the
    /// probe costs the same whether a push carries one oversized event or
    /// five hundred; an empty candidate list asks nothing.
    async fn already_stored(
        &self,
        candidates: Vec<String>,
    ) -> Result<HashSet<String>, StateStoreError> {
        if candidates.is_empty() {
            return Ok(HashSet::new());
        }

        let present: Vec<String> = execution_event::Entity::find()
            .select_only()
            .column(execution_event::Column::Id)
            .filter(execution_event::Column::Id.is_in(candidates))
            .into_tuple()
            .all(self.db.as_ref())
            .await
            .map_err(|error| StateStoreError::Database(error.to_string()))?;

        Ok(present.into_iter().collect())
    }

    /// The rows to insert for `events`, skipping the ones a probe already
    /// found stored. `bodies` are the serialized payloads, in the same order.
    async fn event_models(
        &self,
        events: &[CreateEventInput],
        bodies: Vec<Vec<u8>>,
        already_stored: &HashSet<String>,
        now: sea_orm::prelude::DateTimeWithTimeZone,
    ) -> Result<Vec<execution_event::ActiveModel>, StateStoreError> {
        let mut models = Vec::with_capacity(events.len());
        for (event, body) in events.iter().zip(bodies) {
            if already_stored.contains(&event.id) {
                continue;
            }
            models.push(self.event_model(event, body, now).await?);
        }
        Ok(models)
    }

    async fn event_model(
        &self,
        event: &CreateEventInput,
        body: Vec<u8>,
        now: sea_orm::prelude::DateTimeWithTimeZone,
    ) -> Result<execution_event::ActiveModel, StateStoreError> {
        let staged = if body.len() > PAYLOAD_OFFLOAD_BYTES {
            self.stage_payload(&event.run_id, &event.id, body).await?
        } else {
            None
        };
        let (payload, payload_ref) = match staged {
            Some((reference, bytes)) => (offloaded_placeholder(bytes), Some(reference)),
            None => (event.payload.clone(), None),
        };

        Ok(execution_event::ActiveModel {
            id: Set(event.id.clone()),
            run_id: Set(event.run_id.clone()),
            sequence: Set(event.sequence),
            event_type: Set(event.event_type.clone()),
            payload: Set(payload),
            payload_ref: Set(payload_ref),
            delivered: Set(false),
            expires_at: Set(ts_to_datetime(event.expires_at)),
            created_at: Set(now),
        })
    }

    /// Read back a staged payload.
    ///
    /// An object the store reports as gone degrades to a marked payload: the
    /// event still carries the sequence a poller is waiting on, so dropping it
    /// would stall the stream instead. Every other failure propagates, because
    /// the caller marks what it returns as delivered — degrading a timeout or a
    /// throttle would retire an event whose payload was never missing.
    async fn resolve_payload(
        &self,
        event_id: &str,
        reference: &str,
    ) -> Result<serde_json::Value, StateStoreError> {
        let Some(store) = self.content_store.as_ref() else {
            tracing::error!(
                event_id,
                reference,
                "Staged event payload cannot be read without a content store"
            );
            return Err(StateStoreError::Configuration(format!(
                "event '{event_id}' has a staged payload but no content store is configured"
            )));
        };

        match fetch_staged_payload(store, reference).await {
            Ok(payload) => Ok(payload),
            Err(StagedReadError::Gone(reason)) => {
                tracing::error!(
                    event_id,
                    reference,
                    reason,
                    "Staged event payload is gone; the event is served without it"
                );
                Ok(unavailable_payload(reference, &reason))
            }
            Err(StagedReadError::Transient(reason)) => {
                tracing::warn!(
                    event_id,
                    reference,
                    reason,
                    "Staged event payload could not be read; the poll is retried"
                );
                Err(StateStoreError::Database(format!(
                    "reading the staged payload of event '{event_id}' failed: {reason}"
                )))
            }
        }
    }

    /// Delete the staged objects of the events matched by `condition`, then the
    /// rows that referenced them, one bounded page at a time.
    ///
    /// The object goes before its row: a row that outlives its object degrades
    /// on read and expires anyway, while an object that outlives its row has
    /// lost its only pointer and orphans forever.
    async fn drain_offloaded_events(
        &self,
        condition: &Condition,
        max_chunks: usize,
    ) -> Result<DrainOutcome, StateStoreError> {
        let Some(store) = self.content_store.clone() else {
            return Ok(DrainOutcome::default());
        };

        let mut outcome = DrainOutcome::default();
        for _ in 0..max_chunks {
            let page: Vec<(String, Option<String>)> = offloaded_event_page(condition)
                .into_tuple()
                .all(self.db.as_ref())
                .await
                .map_err(|error| StateStoreError::Database(error.to_string()))?;
            if page.is_empty() {
                return Ok(outcome);
            }
            let fetched = page.len();

            stream::iter(page.iter().filter_map(|(_, reference)| reference.clone()))
                .for_each_concurrent(STAGED_DELETE_CONCURRENCY, |reference| {
                    let store = store.clone();
                    async move {
                        delete_staged_payload(&store, &reference).await;
                    }
                })
                .await;

            let ids: Vec<String> = page.into_iter().map(|(id, _)| id).collect();
            outcome.rows += retry_transaction::<_, u64, DbErr>(
                self.db.as_ref(),
                self.dialect,
                None,
                &RetryPolicy::idempotent(),
                move |txn| {
                    let delete = execution_event::Entity::delete_many()
                        .filter(execution_event::Column::Id.is_in(ids.clone()));
                    Box::pin(
                        async move { delete.exec(txn).await.map(|result| result.rows_affected) },
                    )
                },
            )
            .await
            .map_err(|error| StateStoreError::Database(error.to_string()))?;

            if fetched < EXECUTION_WRITE_CHUNK {
                return Ok(outcome);
            }
        }

        outcome.stopped_early = true;
        Ok(outcome)
    }

    /// Drain the children of `run_ids` (events, caller-app rows) leaf first so
    /// the run delete that follows cascades onto nothing.
    ///
    /// Returns whether every staged object of those runs was removed. The
    /// caller must not delete the run rows when it was not: `ExecutionEvent`
    /// cascades on `runId`, and a cascaded row takes `payloadRef` — the only
    /// pointer to the object — with it.
    async fn delete_run_children(&self, run_ids: &[String]) -> Result<bool, DbErr> {
        let events_of_runs =
            Condition::all().add(execution_event::Column::RunId.is_in(run_ids.to_vec()));
        let drain = self
            .drain_offloaded_events(&events_of_runs, MAX_CHUNKS_PER_CLEANUP)
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))?;
        delete_in_batches::<execution_event::Entity>(
            self.db.as_ref(),
            self.dialect,
            rows_without_staged_payload(events_of_runs),
            EXECUTION_WRITE_CHUNK,
            None,
        )
        .await?;
        delete_in_batches_by_tuple::<execution_run_caller_app::Entity>(
            self.db.as_ref(),
            self.dialect,
            Condition::all().add(execution_run_caller_app::Column::RunId.is_in(run_ids.to_vec())),
            EXECUTION_WRITE_CHUNK,
            None,
        )
        .await?;
        Ok(!drain.stopped_early)
    }

    async fn delete_runs(&self, run_ids: Vec<String>, expired: &Condition) -> Result<u64, DbErr> {
        retry_transaction::<_, u64, DbErr>(
            self.db.as_ref(),
            self.dialect,
            None,
            &RetryPolicy::idempotent(),
            move |txn| {
                let delete = execution_run::Entity::delete_many()
                    .filter(execution_run::Column::Id.is_in(run_ids.clone()))
                    .filter(expired.clone());
                Box::pin(async move { delete.exec(txn).await.map(|result| result.rows_affected) })
            },
        )
        .await
    }

    /// Mirror an update accepted by a non-Postgres execution store into the
    /// canonical SQL run row. Non-terminal updates are ordered by the source
    /// timestamp. Terminal updates win over mutable SQL state, but never
    /// replace an existing terminal result.
    pub async fn mirror_run_update(&self, run: &ExecutionRunRecord) -> Result<(), StateStoreError> {
        let result = run_mirror_update(run)
            .exec(self.db.as_ref())
            .await
            .map_err(|error| StateStoreError::Database(error.to_string()))?;

        if result.rows_affected > 0 {
            return Ok(());
        }

        // A retried callback may find a terminal SQL row, while an out-of-order
        // non-terminal callback may find a newer mutable row. Both are safe
        // no-ops. Other zero-row results indicate a lost or inconsistent
        // canonical handoff and must remain retryable.
        let accepted_updated_at = ts_to_datetime(run.updated_at);
        match execution_run::Entity::find_by_id(&run.id)
            .filter(execution_run::Column::AppId.eq(&run.app_id))
            .one(self.db.as_ref())
            .await
            .map_err(|error| StateStoreError::Database(error.to_string()))?
        {
            Some(existing)
                if accepted_mirror_is_obsolete(
                    &existing.status,
                    existing.updated_at,
                    run,
                    accepted_updated_at,
                ) =>
            {
                Ok(())
            }
            Some(_) => Err(StateStoreError::Database(format!(
                "canonical SQL run '{}' rejected an accepted state-store update",
                run.id
            ))),
            None => Err(StateStoreError::NotFound),
        }
    }
}

fn accepted_mirror_is_obsolete(
    existing_status: &EntityRunStatus,
    existing_updated_at: sea_orm::prelude::DateTimeWithTimeZone,
    accepted: &ExecutionRunRecord,
    accepted_updated_at: sea_orm::prelude::DateTimeWithTimeZone,
) -> bool {
    matches!(
        existing_status,
        EntityRunStatus::Completed
            | EntityRunStatus::Failed
            | EntityRunStatus::Cancelled
            | EntityRunStatus::Timeout
    ) || (!accepted.status.is_terminal() && existing_updated_at > accepted_updated_at)
}

fn run_mirror_update(run: &ExecutionRunRecord) -> sea_orm::UpdateMany<execution_run::Entity> {
    let mut update = execution_run::Entity::update_many()
        .set(run_mirror_model(run))
        .filter(execution_run::Column::Id.eq(&run.id))
        .filter(execution_run::Column::AppId.eq(&run.app_id))
        .filter(
            execution_run::Column::Status
                .is_in([EntityRunStatus::Pending, EntityRunStatus::Running]),
        );

    if !run.status.is_terminal() {
        update =
            update.filter(execution_run::Column::UpdatedAt.lte(ts_to_datetime(run.updated_at)));
        if run.status == RunStatus::Pending {
            update = update.filter(execution_run::Column::Status.eq(EntityRunStatus::Pending));
        }
    }

    update
}

fn run_mirror_model(run: &ExecutionRunRecord) -> execution_run::ActiveModel {
    execution_run::ActiveModel {
        status: Set(type_run_status_to_entity(run.status.clone())),
        output_payload_len: Set(run.output_payload_len),
        error_message: Set(run.error_message.clone()),
        progress: Set(run.progress),
        current_step: Set(run.current_step.clone()),
        started_at: Set(run.started_at.map(ts_to_datetime)),
        completed_at: Set(run.completed_at.map(ts_to_datetime)),
        updated_at: Set(ts_to_datetime(run.updated_at)),
        ..Default::default()
    }
}

fn mutable_run_update(
    run_id: &str,
    model: execution_run::ActiveModel,
) -> sea_orm::UpdateMany<execution_run::Entity> {
    execution_run::Entity::update_many()
        .set(model)
        .filter(execution_run::Column::Id.eq(run_id))
        .filter(
            execution_run::Column::Status
                .is_in([EntityRunStatus::Pending, EntityRunStatus::Running]),
        )
}

fn event_first_write_wins() -> OnConflict {
    OnConflict::column(execution_event::Column::Id)
        .do_nothing()
        .to_owned()
}

fn expired_runs(now: sea_orm::prelude::DateTimeWithTimeZone) -> Condition {
    Condition::all()
        .add(execution_run::Column::ExpiresAt.is_not_null())
        .add(execution_run::Column::ExpiresAt.lt(now))
}

fn expired_run_page(expired: &Condition) -> Select<execution_run::Entity> {
    execution_run::Entity::find()
        .filter(expired.clone())
        .select_only()
        .column(execution_run::Column::Id)
        .order_by_asc(execution_run::Column::Id)
        .limit(RUN_DELETE_PAGE as u64)
}

fn offloaded_event_page(condition: &Condition) -> Select<execution_event::Entity> {
    execution_event::Entity::find()
        .filter(condition.clone())
        .filter(execution_event::Column::PayloadRef.is_not_null())
        .select_only()
        .column(execution_event::Column::Id)
        .column(execution_event::Column::PayloadRef)
        .order_by_asc(execution_event::Column::Id)
        .limit(EXECUTION_WRITE_CHUNK as u64)
}

fn staged_payload_path(run_id: &str, event_id: &str, body: &[u8]) -> Path {
    let run_hash = blake3::hash(run_id.as_bytes()).to_hex();
    let event_hash = blake3::hash(event_id.as_bytes()).to_hex();
    let payload_hash = blake3::hash(body).to_hex();
    Path::from(format!(
        "{STAGED_PAYLOAD_PREFIX}/{run_hash}/{event_hash}-{payload_hash}.json"
    ))
}

/// The row keeps a descriptor rather than `{}`, which a reader that does not
/// know about `payloadRef` would take for a legitimately empty payload.
fn offloaded_placeholder(bytes: usize) -> serde_json::Value {
    serde_json::json!({ "__payloadOffloaded": true, "bytes": bytes })
}

fn unavailable_payload(reference: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({
        "__payloadOffloaded": true,
        "__payloadUnavailable": true,
        "reference": reference,
        "reason": reason,
    })
}

fn staged_reference_path(reference: &str) -> Path {
    let path = reference
        .strip_prefix("store://")
        .or_else(|| {
            reference
                .strip_prefix("s3://")
                .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
        })
        .unwrap_or(reference);
    Path::from(path)
}

/// A store that reports the object as missing is answering about the object;
/// every other failure is about this attempt and may succeed on the next one.
fn classify_read_error(path: &Path, action: &str, error: ObjectStoreError) -> StagedReadError {
    match error {
        ObjectStoreError::NotFound { .. } => {
            StagedReadError::Gone(format!("object store {action} of '{path}' found no object"))
        }
        error => {
            StagedReadError::Transient(format!("object store {action} of '{path}' failed: {error}"))
        }
    }
}

async fn fetch_staged_payload(
    store: &FlowLikeStore,
    reference: &str,
) -> Result<serde_json::Value, StagedReadError> {
    let path = staged_reference_path(reference);
    let object = store
        .as_generic()
        .get(&path)
        .await
        .map_err(|error| classify_read_error(&path, "get", error))?;
    let bytes = object
        .bytes()
        .await
        .map_err(|error| classify_read_error(&path, "read", error))?;
    // A body that is not JSON is as final as a missing one: no retry turns it
    // back into a payload.
    serde_json::from_slice(&bytes).map_err(|error| {
        StagedReadError::Gone(format!("staged payload at '{path}' is not JSON: {error}"))
    })
}

/// The newest `last_modified` an age sweep may delete.
///
/// Floored at one event lifetime, so neither a misconfigured `min_age_secs` nor
/// clock skew between the API and the object store can put an object that a
/// live row still names — or that an insert in flight is about to name — in
/// range of a sweep that reads no rows at all.
fn staged_sweep_cutoff(min_age_secs: u64, now: DateTime<Utc>) -> DateTime<Utc> {
    let seconds = i64::try_from(min_age_secs.max(EVENT_TTL_SECS)).unwrap_or(i64::MAX);
    now - chrono::Duration::try_seconds(seconds)
        .unwrap_or_else(|| chrono::Duration::seconds(EVENT_TTL_SECS as i64))
}

/// The event rows a batch delete may remove. The drain owns every offloaded
/// row: deleting one here would take `payloadRef`, the only pointer to its
/// object, with it.
fn rows_without_staged_payload(condition: Condition) -> Condition {
    condition.add(execution_event::Column::PayloadRef.is_null())
}

/// The events worth an existence probe: their payload needs the content store,
/// and a retry would carry the same id.
///
/// Staging one of these twice leaks. The insert is a `DO NOTHING`, so the row
/// keeps naming the first object, and a retried payload is not byte identical —
/// page-action capabilities are re-signed on every push, minting a fresh `jti`
/// and `iat` — so the second object lands at a different content-addressed path
/// that nothing will ever reference or delete.
fn oversized_canonical_ids(events: &[CreateEventInput], bodies: &[Vec<u8>]) -> Vec<String> {
    events
        .iter()
        .zip(bodies)
        .filter(|(event, body)| body.len() > PAYLOAD_OFFLOAD_BYTES && has_canonical_identity(event))
        .map(|(event, _)| event.id.clone())
        .collect()
}

/// An object that is already gone is in the state the sweep wants, so only a
/// real failure is worth a line - and it must not stop the row delete, or the
/// sweep would never make progress past it.
async fn delete_staged_object(store: &Arc<dyn ObjectStore>, path: &Path) -> bool {
    match store.delete(path).await {
        Ok(()) | Err(ObjectStoreError::NotFound { .. }) => true,
        Err(error) => {
            tracing::warn!(
                %path,
                %error,
                "Staged event payload could not be deleted; the object is orphaned"
            );
            false
        }
    }
}

/// Delete the object a `payloadRef` names. Shared with app deletion, which has
/// to remove these objects before the rows that name them drain.
pub(crate) async fn delete_staged_payload(store: &FlowLikeStore, reference: &str) -> bool {
    let generic = store.as_generic();
    delete_staged_object(&generic, &staged_reference_path(reference)).await
}

/// Delete `paths` with bounded concurrency and report how many are gone. One
/// that will not delete stays listed, and its age keeps it in range next time.
async fn delete_staged_objects(store: &Arc<dyn ObjectStore>, paths: Vec<Path>) -> u64 {
    stream::iter(paths)
        .map(|path| async move { u64::from(delete_staged_object(store, &path).await) })
        .buffer_unordered(STAGED_DELETE_CONCURRENCY)
        .fold(0u64, |total, deleted| async move { total + deleted })
        .await
}

/// Widest decimal rendering of an `i64` or `f64`, charged for every number so
/// the estimate can only ever overshoot.
const JSON_NUMBER_BYTES: usize = 20;

/// An upper bound on the serialized size of a JSON value without serializing it.
fn json_bytes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 4,
        serde_json::Value::Bool(_) => 5,
        serde_json::Value::Number(_) => JSON_NUMBER_BYTES,
        serde_json::Value::String(text) => text.len() + 2,
        serde_json::Value::Array(items) => {
            2 + items.iter().map(|item| json_bytes(item) + 1).sum::<usize>()
        }
        serde_json::Value::Object(fields) => {
            2 + fields
                .iter()
                .map(|(key, item)| key.len() + 4 + json_bytes(item))
                .sum::<usize>()
        }
    }
}

/// The stored size of a row, which for an offloaded event is its reference and
/// placeholder rather than the payload that went to the content store.
fn event_bytes(event: &execution_event::ActiveModel) -> usize {
    let text = [&event.id, &event.run_id, &event.event_type]
        .into_iter()
        .map(|value| value.try_as_ref().map(String::len).unwrap_or(0))
        .sum::<usize>();
    let payload_ref = event
        .payload_ref
        .try_as_ref()
        .map(|value| value.as_deref().map_or(0, str::len))
        .unwrap_or(0);
    let payload = event.payload.try_as_ref().map(json_bytes).unwrap_or(0);
    text + payload_ref + payload + 64
}

// Conversion helpers
fn entity_run_status_to_type(s: EntityRunStatus) -> RunStatus {
    match s {
        EntityRunStatus::Pending => RunStatus::Pending,
        EntityRunStatus::Running => RunStatus::Running,
        EntityRunStatus::Completed => RunStatus::Completed,
        EntityRunStatus::Failed => RunStatus::Failed,
        EntityRunStatus::Cancelled => RunStatus::Cancelled,
        EntityRunStatus::Timeout => RunStatus::Timeout,
    }
}

fn type_run_status_to_entity(s: RunStatus) -> EntityRunStatus {
    match s {
        RunStatus::Pending => EntityRunStatus::Pending,
        RunStatus::Running => EntityRunStatus::Running,
        RunStatus::Completed => EntityRunStatus::Completed,
        RunStatus::Failed => EntityRunStatus::Failed,
        RunStatus::Cancelled => EntityRunStatus::Cancelled,
        RunStatus::Timeout => EntityRunStatus::Timeout,
    }
}

fn entity_run_mode_to_type(m: EntityRunMode) -> RunMode {
    match m {
        EntityRunMode::Local => RunMode::Local,
        EntityRunMode::Http => RunMode::Http,
        EntityRunMode::Lambda => RunMode::Lambda,
        EntityRunMode::KubernetesIsolated => RunMode::KubernetesIsolated,
        EntityRunMode::KubernetesPool => RunMode::KubernetesPool,
        EntityRunMode::Function => RunMode::Function,
        EntityRunMode::Queue => RunMode::Queue,
    }
}

fn type_run_mode_to_entity(m: RunMode) -> EntityRunMode {
    match m {
        RunMode::Local => EntityRunMode::Local,
        RunMode::Http => EntityRunMode::Http,
        RunMode::Lambda => EntityRunMode::Lambda,
        RunMode::KubernetesIsolated => EntityRunMode::KubernetesIsolated,
        RunMode::KubernetesPool => EntityRunMode::KubernetesPool,
        RunMode::Function => EntityRunMode::Function,
        RunMode::Queue => EntityRunMode::Queue,
    }
}

fn entity_run_variant_to_type(v: EntityRunVariant) -> RunVariant {
    match v {
        EntityRunVariant::Primary => RunVariant::Primary,
        EntityRunVariant::Canary => RunVariant::Canary,
        EntityRunVariant::Shadow => RunVariant::Shadow,
        EntityRunVariant::Regression => RunVariant::Regression,
    }
}

fn type_run_variant_to_entity(v: RunVariant) -> EntityRunVariant {
    match v {
        RunVariant::Primary => EntityRunVariant::Primary,
        RunVariant::Canary => EntityRunVariant::Canary,
        RunVariant::Shadow => EntityRunVariant::Shadow,
        RunVariant::Regression => EntityRunVariant::Regression,
    }
}

fn ts_to_datetime(ts: i64) -> sea_orm::prelude::DateTimeWithTimeZone {
    Utc.timestamp_millis_opt(ts).unwrap().fixed_offset()
}

fn datetime_to_ts(dt: sea_orm::prelude::DateTimeWithTimeZone) -> i64 {
    dt.timestamp_millis()
}

fn opt_datetime_to_ts(dt: Option<sea_orm::prelude::DateTimeWithTimeZone>) -> Option<i64> {
    dt.map(datetime_to_ts)
}

fn run_model_to_record(m: execution_run::Model) -> ExecutionRunRecord {
    ExecutionRunRecord {
        id: m.id,
        board_id: m.board_id,
        version: m.version,
        event_id: m.event_id,
        status: entity_run_status_to_type(m.status),
        mode: entity_run_mode_to_type(m.mode),
        run_variant: entity_run_variant_to_type(m.run_variant),
        variant_name: m.variant_name,
        shadow_of_run_id: m.shadow_of_run_id,
        regression_run_id: m.regression_run_id,
        input_payload_len: m.input_payload_len,
        output_payload_len: m.output_payload_len,
        error_message: m.error_message,
        progress: m.progress,
        current_step: m.current_step,
        started_at: opt_datetime_to_ts(m.started_at),
        completed_at: opt_datetime_to_ts(m.completed_at),
        expires_at: opt_datetime_to_ts(m.expires_at),
        user_id: m.user_id,
        technical_user_id: m.technical_user_id,
        app_id: m.app_id,
        created_at: datetime_to_ts(m.created_at),
        updated_at: datetime_to_ts(m.updated_at),
    }
}

/// Returns the record and, when the payload was offloaded, its reference.
fn event_model_to_record(m: execution_event::Model) -> (ExecutionEventRecord, Option<String>) {
    (
        ExecutionEventRecord {
            id: m.id,
            run_id: m.run_id,
            sequence: m.sequence,
            event_type: m.event_type,
            payload: m.payload,
            delivered: m.delivered,
            expires_at: datetime_to_ts(m.expires_at),
            created_at: datetime_to_ts(m.created_at),
        },
        m.payload_ref,
    )
}

#[async_trait]
impl ExecutionStateStore for PostgresStateStore {
    fn backend_name(&self) -> &'static str {
        "postgres"
    }

    async fn create_run(
        &self,
        input: CreateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        let now = chrono::Utc::now().fixed_offset();
        let model = execution_run::ActiveModel {
            id: Set(input.id.clone()),
            board_id: Set(input.board_id),
            version: Set(input.version),
            event_id: Set(input.event_id),
            node_id: Set(None),
            status: Set(EntityRunStatus::Pending),
            mode: Set(type_run_mode_to_entity(input.mode)),
            run_variant: Set(type_run_variant_to_entity(input.run_variant)),
            variant_name: Set(input.variant_name),
            shadow_of_run_id: Set(input.shadow_of_run_id),
            regression_run_id: Set(input.regression_run_id),
            input_payload_len: Set(input.input_payload_len),
            input_payload_key: Set(None),
            output_payload_len: Set(0),
            log_level: Set(0),
            error_message: Set(None),
            progress: Set(0),
            current_step: Set(None),
            started_at: Set(None),
            completed_at: Set(None),
            expires_at: Set(input.expires_at.map(ts_to_datetime)),
            user_id: Set(input.user_id),
            technical_user_id: Set(input.technical_user_id),
            caller_app_chain: Set(None),
            trace_id: Set(Some(input.id.clone())),
            parent_run_id: Set(None),
            correlation_keys: Set(None),
            app_id: Set(input.app_id),
            created_at: Set(now),
            updated_at: Set(now),
        };

        let result = model
            .insert(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(run_model_to_record(result))
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        let result = execution_run::Entity::find_by_id(run_id)
            .one(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(result.map(run_model_to_record))
    }

    async fn get_run_for_app(
        &self,
        run_id: &str,
        app_id: &str,
    ) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        let result = execution_run::Entity::find_by_id(run_id)
            .filter(execution_run::Column::AppId.eq(app_id))
            .one(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(result.map(run_model_to_record))
    }

    async fn update_run(
        &self,
        run_id: &str,
        input: UpdateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        let existing = execution_run::Entity::find_by_id(run_id)
            .one(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?
            .ok_or(StateStoreError::NotFound)?;

        if matches!(
            existing.status,
            EntityRunStatus::Completed
                | EntityRunStatus::Failed
                | EntityRunStatus::Cancelled
                | EntityRunStatus::Timeout
        ) {
            return Ok(run_model_to_record(existing));
        }

        let mut model: execution_run::ActiveModel = existing.into();
        model.updated_at = Set(chrono::Utc::now().fixed_offset());

        if let Some(progress) = input.progress {
            model.progress = Set(progress);
        }
        if let Some(current_step) = input.current_step {
            model.current_step = Set(Some(current_step));
        }
        if let Some(status) = input.status {
            model.status = Set(type_run_status_to_entity(status));
        }
        if let Some(output_payload_len) = input.output_payload_len {
            model.output_payload_len = Set(output_payload_len);
        }
        if let Some(error_message) = input.error_message {
            model.error_message = Set(Some(error_message));
        }
        if let Some(started_at) = input.started_at {
            model.started_at = Set(Some(ts_to_datetime(started_at)));
        }
        if let Some(completed_at) = input.completed_at {
            model.completed_at = Set(Some(ts_to_datetime(completed_at)));
        }

        let result = mutable_run_update(run_id, model)
            .exec(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        if result.rows_affected == 0 {
            let current = execution_run::Entity::find_by_id(run_id)
                .one(self.db.as_ref())
                .await
                .map_err(|e| StateStoreError::Database(e.to_string()))?
                .ok_or(StateStoreError::NotFound)?;
            if matches!(
                current.status,
                EntityRunStatus::Completed
                    | EntityRunStatus::Failed
                    | EntityRunStatus::Cancelled
                    | EntityRunStatus::Timeout
            ) {
                return Ok(run_model_to_record(current));
            }
            return Err(StateStoreError::Database(format!(
                "execution run '{run_id}' changed while applying progress"
            )));
        }

        self.get_run(run_id).await?.ok_or(StateStoreError::NotFound)
    }

    async fn list_runs_for_app(
        &self,
        app_id: &str,
        limit: i32,
        cursor: Option<&str>,
    ) -> Result<Vec<ExecutionRunRecord>, StateStoreError> {
        let mut query = execution_run::Entity::find()
            .filter(execution_run::Column::AppId.eq(app_id))
            .order_by_desc(execution_run::Column::CreatedAt)
            .limit(limit as u64);

        if let Some(cursor) = cursor {
            query = query.filter(execution_run::Column::Id.lt(cursor));
        }

        let results = query
            .all(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(results.into_iter().map(run_model_to_record).collect())
    }

    /// Expired runs go one page at a time: the page's events and caller-app
    /// rows are drained first, then the runs themselves, so every transaction
    /// touches a known number of rows and the cascade finds nothing left.
    async fn delete_expired_runs(&self) -> Result<i64, StateStoreError> {
        let expired = expired_runs(chrono::Utc::now().fixed_offset());
        let mut deleted = 0u64;
        let mut pages = 0usize;
        loop {
            if pages >= MAX_CHUNKS_PER_CLEANUP {
                tracing::warn!(
                    deleted,
                    max_pages = MAX_CHUNKS_PER_CLEANUP,
                    "Expired run cleanup hit its budget; the rest is removed next call"
                );
                break;
            }
            let run_ids: Vec<String> = expired_run_page(&expired)
                .into_tuple()
                .all(self.db.as_ref())
                .await
                .map_err(|e| StateStoreError::Database(e.to_string()))?;
            if run_ids.is_empty() {
                break;
            }
            pages += 1;
            let fetched = run_ids.len();
            let drained = self
                .delete_run_children(&run_ids)
                .await
                .map_err(|e| StateStoreError::Database(e.to_string()))?;
            if !drained {
                tracing::warn!(
                    runs = fetched,
                    deleted,
                    "Staged payloads of expired runs were not fully drained; their rows stay \
                     until the next call, because deleting the run would cascade the only \
                     pointer to those objects away"
                );
                break;
            }
            let removed = self
                .delete_runs(run_ids, &expired)
                .await
                .map_err(|e| StateStoreError::Database(e.to_string()))?;
            deleted += removed;
            if removed == 0 || fetched < RUN_DELETE_PAGE {
                break;
            }
        }

        Ok(deleted as i64)
    }

    async fn push_events(&self, events: Vec<CreateEventInput>) -> Result<i32, StateStoreError> {
        if events.is_empty() {
            return Ok(0);
        }

        let now = chrono::Utc::now().fixed_offset();
        // Serialized once: the size decides whether the payload is offloaded,
        // and these are the bytes that get staged.
        let mut bodies = Vec::with_capacity(events.len());
        for event in &events {
            bodies.push(
                serde_json::to_vec(&event.payload)
                    .map_err(|error| StateStoreError::Serialization(error.to_string()))?,
            );
        }

        // Ask once which oversized events are already stored, and write
        // neither their object nor their row. Their insert would be a
        // `DO NOTHING`, while staging them again would leave a second object
        // on the content store that no row will ever name.
        let already_stored = self
            .already_stored(oversized_canonical_ids(&events, &bodies))
            .await?;
        let models = self
            .event_models(&events, bodies, &already_stored, now)
            .await?;

        // Every event in the request is accepted, whether this call stored it
        // or an earlier one did.
        let count = events.len() as i32;
        if models.is_empty() {
            return Ok(count);
        }

        // Canonical IDs make HTTP retries the same logical event. Keep the
        // first accepted payload and never reset its delivery state; the same
        // rule makes a chunk that is replayed after an ambiguous commit a no-op.
        let chunks = chunk_by_rows_and_bytes_with(
            models,
            EXECUTION_WRITE_CHUNK,
            DEFAULT_WRITE_BYTES,
            event_bytes,
        );
        insert_chunks(
            self.db.as_ref(),
            self.dialect,
            chunks,
            Some(event_first_write_wins()),
        )
        .await
        .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(count)
    }

    async fn get_events(
        &self,
        query: EventQuery,
    ) -> Result<Vec<ExecutionEventRecord>, StateStoreError> {
        let mut q = execution_event::Entity::find()
            .filter(execution_event::Column::RunId.eq(&query.run_id))
            .order_by_asc(execution_event::Column::Sequence);

        if let Some(after) = query.after_sequence {
            q = q.filter(execution_event::Column::Sequence.gt(after));
        }

        if query.only_undelivered {
            q = q.filter(execution_event::Column::Delivered.eq(false));
        }

        if let Some(limit) = query.limit {
            q = q.limit(limit as u64);
        }

        let results = q
            .all(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        let mut records = Vec::with_capacity(results.len());
        for model in results {
            let (mut record, payload_ref) = event_model_to_record(model);
            if let Some(reference) = payload_ref {
                record.payload = self.resolve_payload(&record.id, &reference).await?;
            }
            records.push(record);
        }

        Ok(records)
    }

    async fn get_max_sequence(&self, run_id: &str) -> Result<i32, StateStoreError> {
        let result = execution_event::Entity::find()
            .filter(execution_event::Column::RunId.eq(run_id))
            .order_by_desc(execution_event::Column::Sequence)
            .limit(1)
            .one(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(result.map(|m| m.sequence).unwrap_or(0))
    }

    async fn mark_events_delivered(
        &self,
        run_id: &str,
        event_ids: &[String],
    ) -> Result<(), StateStoreError> {
        if event_ids.is_empty() {
            return Ok(());
        }

        execution_event::Entity::update_many()
            .col_expr(
                execution_event::Column::Delivered,
                sea_orm::sea_query::Expr::value(true),
            )
            .filter(execution_event::Column::RunId.eq(run_id))
            .filter(execution_event::Column::Id.is_in(event_ids.to_vec()))
            .exec(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(())
    }

    async fn delete_expired_events(&self) -> Result<i64, StateStoreError> {
        let now = chrono::Utc::now().fixed_offset();
        let expired = Condition::all().add(execution_event::Column::ExpiresAt.lt(now));
        let drain = self
            .drain_offloaded_events(&expired, MAX_CHUNKS_PER_CLEANUP)
            .await?;
        // The drain is the only deleter of offloaded rows. A row delete that
        // ran past a budget-capped drain would take `payloadRef` — the only
        // pointer to the object — with it.
        let outcome = delete_in_batches::<execution_event::Entity>(
            self.db.as_ref(),
            self.dialect,
            rows_without_staged_payload(expired),
            EXECUTION_WRITE_CHUNK,
            Some(MAX_CHUNKS_PER_CLEANUP),
        )
        .await
        .map_err(|e| StateStoreError::Database(e.to_string()))?;
        if outcome.stopped_early || drain.stopped_early {
            tracing::warn!(
                deleted = outcome.rows + drain.rows,
                offloaded = drain.rows,
                offloaded_stopped_early = drain.stopped_early,
                rows_stopped_early = outcome.stopped_early,
                max_chunks = MAX_CHUNKS_PER_CLEANUP,
                "Expired event cleanup hit its budget; the rest is removed next call"
            );
        }

        Ok((outcome.rows + drain.rows) as i64)
    }

    /// Delete staged objects older than `min_age_secs`, whatever any row says.
    ///
    /// Nothing else can reclaim an object whose row was never committed: the
    /// object is written before the insert, and `payloadRef` is its only
    /// pointer, so a failed or partially applied multi-chunk write leaves it
    /// unreachable. Age is the one property it still carries.
    ///
    /// Safe against live writes because `min_age_secs` is at least one event
    /// lifetime: an object young enough to belong to an in-flight insert, or to
    /// a row that has not expired yet, is never in range.
    async fn sweep_staged_payloads(
        &self,
        min_age_secs: u64,
    ) -> Result<StagedPayloadSweep, StateStoreError> {
        let Some(store) = self.content_store.clone() else {
            return Ok(StagedPayloadSweep::default());
        };
        let cutoff = staged_sweep_cutoff(min_age_secs, Utc::now());

        let prefix = Path::from(STAGED_PAYLOAD_PREFIX);
        let store = store.as_generic();
        let mut listing = store.list(Some(&prefix));
        let mut report = StagedPayloadSweep::default();
        let mut stale: Vec<Path> = Vec::new();

        while let Some(object) = listing.try_next().await.map_err(|error| {
            StateStoreError::Database(format!(
                "listing staged payloads at '{prefix}' failed: {error}"
            ))
        })? {
            report.scanned += 1;
            if object.last_modified < cutoff {
                stale.push(object.location);
            }
            if stale.len() >= STAGED_SWEEP_PAGE {
                report.deleted += delete_staged_objects(&store, std::mem::take(&mut stale)).await;
            }
            if report.scanned >= STAGED_SWEEP_MAX_OBJECTS {
                report.stopped_early = true;
                break;
            }
        }
        report.deleted += delete_staged_objects(&store, stale).await;

        Ok(report)
    }
}

#[cfg(test)]
mod terminal_mirror_tests {
    use super::*;
    use sea_orm::{DatabaseBackend, QueryTrait};

    fn terminal_run() -> ExecutionRunRecord {
        ExecutionRunRecord {
            id: "run-1".into(),
            board_id: "board-1".into(),
            version: Some("3".into()),
            event_id: Some("event-1".into()),
            status: RunStatus::Completed,
            mode: RunMode::Queue,
            run_variant: RunVariant::Primary,
            variant_name: None,
            shadow_of_run_id: None,
            regression_run_id: None,
            input_payload_len: 12,
            output_payload_len: 34,
            error_message: Some("accepted error field".into()),
            progress: 100,
            current_step: Some("complete".into()),
            started_at: Some(1_800_000_000_000),
            completed_at: Some(1_800_000_010_000),
            expires_at: Some(1_900_000_000_000),
            user_id: Some("user-1".into()),
            technical_user_id: None,
            app_id: "app-1".into(),
            created_at: 1_799_999_999_000,
            updated_at: 1_800_000_010_001,
        }
    }

    #[test]
    fn stateless_lambda_sql_mirror_copies_accepted_fields() {
        let run = terminal_run();
        let model = run_mirror_model(&run);

        assert_eq!(model.status, Set(EntityRunStatus::Completed));
        assert_eq!(model.output_payload_len, Set(34));
        assert_eq!(
            model.error_message,
            Set(Some("accepted error field".into()))
        );
        assert_eq!(model.progress, Set(100));
        assert_eq!(model.current_step, Set(Some("complete".into())));
        assert_eq!(
            model.started_at,
            Set(Some(ts_to_datetime(1_800_000_000_000)))
        );
        assert_eq!(
            model.completed_at,
            Set(Some(ts_to_datetime(1_800_000_010_000)))
        );
        assert_eq!(model.updated_at, Set(ts_to_datetime(1_800_000_010_001)));
    }

    #[test]
    fn stateless_lambda_terminal_mirror_is_app_scoped_and_monotonic() {
        let statement = run_mirror_update(&terminal_run())
            .build(DatabaseBackend::Postgres)
            .to_string();

        assert!(statement.contains("\"ExecutionRun\".\"id\" = 'run-1'"));
        assert!(statement.contains("\"ExecutionRun\".\"appId\" = 'app-1'"));
        assert!(statement.contains("\"ExecutionRun\".\"status\" IN"));
        assert!(statement.contains("'PENDING'"));
        assert!(statement.contains("'RUNNING'"));
        assert!(!statement.contains("\"ExecutionRun\".\"updatedAt\" <="));
    }

    #[test]
    fn stateless_lambda_nonterminal_mirror_rejects_stale_updates() {
        let mut run = terminal_run();
        run.status = RunStatus::Running;
        run.updated_at = 1_800_000_005_000;

        let statement = run_mirror_update(&run)
            .build(DatabaseBackend::Postgres)
            .to_string();
        assert!(statement.contains("\"ExecutionRun\".\"updatedAt\" <="));

        let accepted_at = ts_to_datetime(run.updated_at);
        assert!(accepted_mirror_is_obsolete(
            &EntityRunStatus::Running,
            accepted_at + chrono::Duration::milliseconds(1),
            &run,
            accepted_at,
        ));
        assert!(!accepted_mirror_is_obsolete(
            &EntityRunStatus::Running,
            accepted_at,
            &run,
            accepted_at,
        ));
    }

    #[test]
    fn stateless_lambda_terminal_mirror_never_overwrites_terminal_sql() {
        let run = terminal_run();
        let accepted_at = ts_to_datetime(run.updated_at);

        assert!(accepted_mirror_is_obsolete(
            &EntityRunStatus::Timeout,
            accepted_at - chrono::Duration::hours(1),
            &run,
            accepted_at,
        ));
        assert!(!accepted_mirror_is_obsolete(
            &EntityRunStatus::Running,
            accepted_at + chrono::Duration::hours(1),
            &run,
            accepted_at,
        ));
    }

    #[test]
    fn stateless_lambda_postgres_update_is_atomically_terminal_monotonic() {
        let statement = mutable_run_update(
            "run-1",
            execution_run::ActiveModel {
                status: Set(EntityRunStatus::Running),
                progress: Set(50),
                ..Default::default()
            },
        )
        .build(DatabaseBackend::Postgres)
        .to_string();

        assert!(statement.contains("\"ExecutionRun\".\"id\" = 'run-1'"));
        assert!(statement.contains("\"ExecutionRun\".\"status\" IN"));
        assert!(statement.contains("'PENDING'"));
        assert!(statement.contains("'RUNNING'"));
        assert!(!statement.contains("'COMPLETED'"));
        assert!(!statement.contains("'FAILED'"));
    }

    #[test]
    fn stateless_lambda_event_retries_are_first_write_wins() {
        let statement = execution_event::Entity::insert(execution_event::ActiveModel {
            id: Set("evt-1".into()),
            run_id: Set("run-1".into()),
            sequence: Set(0),
            event_type: Set("chunk".into()),
            payload: Set(serde_json::json!({"value": 1})),
            payload_ref: Set(None),
            delivered: Set(false),
            expires_at: Set(ts_to_datetime(1_900_000_000_000)),
            created_at: Set(ts_to_datetime(1_800_000_000_000)),
        })
        .on_conflict(event_first_write_wins())
        .build(DatabaseBackend::Postgres)
        .to_string();

        assert!(statement.contains("ON CONFLICT (\"id\") DO NOTHING"));
        assert!(!statement.contains("DO UPDATE"));
    }

    #[test]
    fn expired_runs_are_paged_by_id_before_their_children_are_drained() {
        let statement = expired_run_page(&expired_runs(ts_to_datetime(1_800_000_000_000)))
            .build(DatabaseBackend::Postgres)
            .to_string();

        assert!(statement.starts_with("SELECT \"ExecutionRun\".\"id\" FROM"));
        assert!(statement.contains("\"ExecutionRun\".\"expiresAt\" IS NOT NULL"));
        assert!(statement.contains("\"ExecutionRun\".\"expiresAt\" <"));
        assert!(statement.ends_with(&format!(
            "ORDER BY \"ExecutionRun\".\"id\" ASC LIMIT {RUN_DELETE_PAGE}"
        )));
    }

    /// The estimate feeds the byte budget that chunks writes, so it must never
    /// undercount the serialized row, and it must follow the payload instead of
    /// staying flat. It does not track byte-for-byte: a JSON number is charged a
    /// flat `JSON_NUMBER_BYTES` worst case, so replacing one with a 10 KB string
    /// grows the estimate by slightly less than the string itself.
    #[test]
    fn event_size_estimate_tracks_the_payload() {
        let event = |payload: serde_json::Value| execution_event::ActiveModel {
            id: Set("evt-1".into()),
            run_id: Set("run-1".into()),
            sequence: Set(0),
            event_type: Set("chunk".into()),
            payload: Set(payload),
            payload_ref: Set(None),
            delivered: Set(false),
            expires_at: Set(ts_to_datetime(1_900_000_000_000)),
            created_at: Set(ts_to_datetime(1_800_000_000_000)),
        };
        let small_payload = serde_json::json!({"value": 1});
        let large_payload = serde_json::json!({"value": "x".repeat(10_000)});
        let small = event_bytes(&event(small_payload.clone()));
        let large = event_bytes(&event(large_payload.clone()));

        assert!(small < 200);
        assert!(large > 10_000);
        assert!(small >= small_payload.to_string().len());
        assert!(large >= large_payload.to_string().len());
        assert!(
            large - small
                >= large_payload.to_string().len()
                    - small_payload.to_string().len()
                    - JSON_NUMBER_BYTES
        );
    }
}

#[cfg(test)]
mod payload_offload_tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;
    use sea_orm::QueryTrait;

    fn store_with_content() -> (PostgresStateStore, Arc<FlowLikeStore>) {
        let content = Arc::new(FlowLikeStore::Memory(Arc::new(InMemory::new())));
        let store = PostgresStateStore::new(Arc::new(DatabaseConnection::default()))
            .with_content_store(content.clone());
        (store, content)
    }

    async fn model_of(
        store: &PostgresStateStore,
        event: &CreateEventInput,
    ) -> execution_event::ActiveModel {
        let body = serde_json::to_vec(&event.payload).unwrap();
        store
            .event_model(event, body, ts_to_datetime(1_800_000_000_000))
            .await
            .unwrap()
    }

    async fn staged_object_count(content: &FlowLikeStore) -> usize {
        content
            .as_generic()
            .list(Some(&Path::from(STAGED_PAYLOAD_PREFIX)))
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .len()
    }

    fn event(payload: serde_json::Value) -> CreateEventInput {
        CreateEventInput {
            id: "evt-1".into(),
            run_id: "run-1".into(),
            sequence: 0,
            event_type: "chunk".into(),
            payload,
            expires_at: 1_900_000_000_000,
        }
    }

    /// `{"value":"x…"}` costs 12 bytes of framing around the string.
    fn payload_of_bytes(bytes: usize) -> serde_json::Value {
        serde_json::json!({ "value": "x".repeat(bytes - 12) })
    }

    #[tokio::test]
    async fn payload_at_the_threshold_stays_in_the_row() {
        let (store, _content) = store_with_content();
        let payload = payload_of_bytes(PAYLOAD_OFFLOAD_BYTES);
        assert_eq!(
            serde_json::to_vec(&payload).unwrap().len(),
            PAYLOAD_OFFLOAD_BYTES
        );

        let model = model_of(&store, &event(payload.clone())).await;

        assert_eq!(model.payload, Set(payload));
        assert_eq!(model.payload_ref, Set(None));
    }

    #[tokio::test]
    async fn payload_over_the_threshold_is_staged_and_read_back() {
        let (store, content) = store_with_content();
        let payload = payload_of_bytes(PAYLOAD_OFFLOAD_BYTES + 1);

        let model = model_of(&store, &event(payload.clone())).await;

        let reference = model.payload_ref.try_as_ref().unwrap().clone().unwrap();
        assert!(reference.starts_with(&format!("store://{STAGED_PAYLOAD_PREFIX}/")));
        assert!(reference.ends_with(".json"));
        assert_eq!(
            model.payload,
            Set(offloaded_placeholder(PAYLOAD_OFFLOAD_BYTES + 1))
        );

        assert_eq!(
            fetch_staged_payload(&content, &reference).await.unwrap(),
            payload
        );
        assert_eq!(
            store.resolve_payload("evt-1", &reference).await.unwrap(),
            payload
        );
    }

    #[tokio::test]
    async fn a_staged_payload_is_deleted_once_and_tolerates_a_second_pass() {
        let (store, content) = store_with_content();
        let model = model_of(&store, &event(payload_of_bytes(PAYLOAD_OFFLOAD_BYTES + 1))).await;
        let reference = model.payload_ref.try_as_ref().unwrap().clone().unwrap();

        delete_staged_payload(&content, &reference).await;
        assert!(fetch_staged_payload(&content, &reference).await.is_err());
        delete_staged_payload(&content, &reference).await;
    }

    /// A reference the content store cannot serve keeps the event, and its
    /// sequence, in the stream — with a payload no reader can mistake for data.
    #[tokio::test]
    async fn a_missing_object_degrades_to_a_marked_payload() {
        let (store, _content) = store_with_content();
        let reference = format!("store://{STAGED_PAYLOAD_PREFIX}/deadbeef/cafe-f00d.json");

        let payload = store.resolve_payload("evt-1", &reference).await.unwrap();

        assert_eq!(payload["__payloadOffloaded"], serde_json::json!(true));
        assert_eq!(payload["__payloadUnavailable"], serde_json::json!(true));
        assert_eq!(payload["reference"], serde_json::json!(reference));
        assert!(
            payload["reason"]
                .as_str()
                .unwrap()
                .contains("found no object")
        );
    }

    /// The byte budget chunks writes by what the row actually stores, so an
    /// offloaded event must be charged its reference and placeholder, not the
    /// payload that went to the content store.
    #[tokio::test]
    async fn an_offloaded_event_is_charged_its_stored_size() {
        let (store, _content) = store_with_content();
        let payload = payload_of_bytes(4 * PAYLOAD_OFFLOAD_BYTES);

        let inline = model_of(&store, &event(payload_of_bytes(1_000))).await;
        let offloaded = model_of(&store, &event(payload)).await;

        assert!(event_bytes(&offloaded) < 1_000);
        assert!(event_bytes(&offloaded) > event_bytes(&inline) - 1_000);
    }

    fn canonical_event(sequence: i32, bytes: usize) -> CreateEventInput {
        CreateEventInput {
            id: canonical_execution_event_id("run-1", sequence),
            run_id: "run-1".into(),
            sequence,
            event_type: "a2ui".into(),
            payload: payload_of_bytes(bytes),
            expires_at: 1_900_000_000_000,
        }
    }

    fn bodies_of(events: &[CreateEventInput]) -> Vec<Vec<u8>> {
        events
            .iter()
            .map(|event| serde_json::to_vec(&event.payload).unwrap())
            .collect()
    }

    /// The defect this guards: an executor retries a large a2ui event whose
    /// commit it never saw. The payload is not byte identical — page actions
    /// are re-signed on every push — so it hashes to a new path, while the
    /// insert stays a `DO NOTHING` and the row keeps naming the first object.
    /// Without the probe, every retry leaves one full payload behind forever.
    #[tokio::test]
    async fn a_retried_canonical_event_stages_no_second_object() {
        let (store, content) = store_with_content();
        let now = ts_to_datetime(1_800_000_000_000);
        let first = canonical_event(0, PAYLOAD_OFFLOAD_BYTES + 1);
        let retry = canonical_event(0, PAYLOAD_OFFLOAD_BYTES + 2);
        assert_eq!(first.id, retry.id);
        let nothing_stored = HashSet::new();

        let models = store
            .event_models(
                std::slice::from_ref(&first),
                bodies_of(std::slice::from_ref(&first)),
                &nothing_stored,
                now,
            )
            .await
            .unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(staged_object_count(&content).await, 1);

        let already_stored = HashSet::from([first.id.clone()]);
        let models = store
            .event_models(
                std::slice::from_ref(&retry),
                bodies_of(std::slice::from_ref(&retry)),
                &already_stored,
                now,
            )
            .await
            .unwrap();
        assert!(models.is_empty());
        assert_eq!(staged_object_count(&content).await, 1);

        // The same retry without the probe is what the leak looked like.
        store
            .event_models(
                std::slice::from_ref(&retry),
                bodies_of(std::slice::from_ref(&retry)),
                &nothing_stored,
                now,
            )
            .await
            .unwrap();
        assert_eq!(staged_object_count(&content).await, 2);
    }

    /// The probe costs a query, so it asks only about the events that could
    /// leak an object: oversized, and identified by a value a retry reproduces.
    #[test]
    fn only_oversized_canonical_events_are_probed() {
        let oversized = canonical_event(0, PAYLOAD_OFFLOAD_BYTES + 1);
        let small = canonical_event(1, 1_000);
        let mut legacy = canonical_event(2, PAYLOAD_OFFLOAD_BYTES + 1);
        legacy.id = "legacy-allocated-id".into();

        let events = vec![oversized.clone(), small, legacy];
        assert_eq!(
            oversized_canonical_ids(&events, &bodies_of(&events)),
            vec![oversized.id]
        );
    }

    /// A reference with no store behind it is a deployment fault, not a lost
    /// payload: it must fail the read so the event stays undelivered until the
    /// store is configured again.
    #[tokio::test]
    async fn a_reference_without_a_content_store_fails_the_read() {
        let store = PostgresStateStore::new(Arc::new(DatabaseConnection::default()));

        let error = store
            .resolve_payload("evt-1", "store://tmp/polling/run/event.json")
            .await
            .unwrap_err();

        assert!(matches!(error, StateStoreError::Configuration(_)));
    }

    /// A payload the store cannot serve right now is not a payload that is
    /// gone: degrading it would let the poll handler mark the event delivered
    /// and retire an event whose object was intact the whole time.
    #[test]
    fn only_a_missing_object_is_a_final_answer() {
        let path = Path::from("tmp/polling/run/event.json");
        assert!(matches!(
            classify_read_error(
                &path,
                "get",
                ObjectStoreError::NotFound {
                    path: path.to_string(),
                    source: "gone".into(),
                },
            ),
            StagedReadError::Gone(_)
        ));
        assert!(matches!(
            classify_read_error(
                &path,
                "get",
                ObjectStoreError::Generic {
                    store: "S3",
                    source: "throttled".into(),
                },
            ),
            StagedReadError::Transient(_)
        ));
    }

    /// Both TTL sweeps leave offloaded rows to the drain: a budget-capped drain
    /// followed by an unfiltered row delete orphans everything it did not reach.
    #[test]
    fn expired_row_deletes_never_touch_an_offloaded_row() {
        let statement = execution_event::Entity::find()
            .filter(rows_without_staged_payload(Condition::all().add(
                execution_event::Column::ExpiresAt.lt(ts_to_datetime(1_800_000_000_000)),
            )))
            .build(sea_orm::DatabaseBackend::Postgres)
            .to_string();

        assert!(statement.contains("\"ExecutionEvent\".\"expiresAt\" <"));
        assert!(statement.contains("\"ExecutionEvent\".\"payloadRef\" IS NULL"));
    }

    /// The age sweep reads no rows, so its only protection against deleting an
    /// object that a live row names — or that an insert in flight is about to —
    /// is the floor under its cutoff.
    #[test]
    fn the_age_cutoff_never_reaches_a_live_object() {
        let now = DateTime::from_timestamp(1_800_000_000, 0).unwrap();

        assert_eq!(
            staged_sweep_cutoff(0, now),
            now - chrono::Duration::seconds(EVENT_TTL_SECS as i64)
        );
        assert_eq!(
            staged_sweep_cutoff(STAGED_PAYLOAD_MIN_AGE_SECS, now),
            now - chrono::Duration::seconds(STAGED_PAYLOAD_MIN_AGE_SECS as i64)
        );
        assert_eq!(
            staged_sweep_cutoff(u64::MAX, now),
            now - chrono::Duration::seconds(EVENT_TTL_SECS as i64)
        );
        assert!(staged_payload_min_age_secs() >= EVENT_TTL_SECS);
    }

    /// The object a push just staged must survive a sweep running beside it.
    #[tokio::test]
    async fn the_age_sweep_keeps_objects_young_enough_to_have_a_row() {
        let (store, content) = store_with_content();
        model_of(&store, &canonical_event(0, PAYLOAD_OFFLOAD_BYTES + 1)).await;

        let report = store.sweep_staged_payloads(0).await.unwrap();

        assert_eq!(report.scanned, 1);
        assert_eq!(report.deleted, 0);
        assert!(!report.stopped_early);
        assert_eq!(staged_object_count(&content).await, 1);
    }

    /// A deployment without a content store has nothing staged to sweep, and
    /// must not fail the maintenance job over it.
    #[tokio::test]
    async fn the_age_sweep_is_a_no_op_without_a_content_store() {
        let store = PostgresStateStore::new(Arc::new(DatabaseConnection::default()));

        assert_eq!(
            store
                .sweep_staged_payloads(STAGED_PAYLOAD_MIN_AGE_SECS)
                .await
                .unwrap(),
            StagedPayloadSweep::default()
        );
    }

    #[test]
    fn a_reference_resolves_to_the_path_it_names() {
        assert_eq!(
            staged_reference_path("store://tmp/polling/run/event.json"),
            Path::from("tmp/polling/run/event.json")
        );
        assert_eq!(
            staged_reference_path("s3://bucket/tmp/polling/run/event.json"),
            Path::from("tmp/polling/run/event.json")
        );
    }
}
