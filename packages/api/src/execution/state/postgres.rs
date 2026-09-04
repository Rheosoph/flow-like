//! PostgreSQL state store implementation using SeaORM
//!
//! This backend uses the existing Prisma-generated schema via SeaORM entities.
//! TTL cleanup is manual - call `delete_expired_runs/events` periodically.

use super::types::*;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseConnection, DbErr, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Select, Set, sea_query::OnConflict,
};
use std::sync::Arc;

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

#[derive(Debug, Clone)]
pub struct PostgresStateStore {
    db: Arc<DatabaseConnection>,
    dialect: DbDialect,
}

impl PostgresStateStore {
    /// A store for single-row work such as [`Self::mirror_run_update`]; bulk
    /// cleanup callers pass the resolved dialect through [`Self::with_dialect`].
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self::with_dialect(db, DbDialect::default())
    }

    pub fn with_dialect(db: Arc<DatabaseConnection>, dialect: DbDialect) -> Self {
        Self { db, dialect }
    }

    /// Drain the children of `run_ids` (events, caller-app rows) leaf first so
    /// the run delete that follows cascades onto nothing.
    async fn delete_run_children(&self, run_ids: &[String]) -> Result<(), DbErr> {
        delete_in_batches::<execution_event::Entity>(
            self.db.as_ref(),
            self.dialect,
            Condition::all().add(execution_event::Column::RunId.is_in(run_ids.to_vec())),
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
        Ok(())
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
    existing_updated_at: sea_orm::prelude::DateTime,
    accepted: &ExecutionRunRecord,
    accepted_updated_at: sea_orm::prelude::DateTime,
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

fn expired_runs(now: sea_orm::prelude::DateTime) -> Condition {
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

fn event_bytes(event: &execution_event::ActiveModel) -> usize {
    let text = [&event.id, &event.run_id, &event.event_type]
        .into_iter()
        .map(|value| value.try_as_ref().map(String::len).unwrap_or(0))
        .sum::<usize>();
    let payload = event.payload.try_as_ref().map(json_bytes).unwrap_or(0);
    text + payload + 64
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

fn ts_to_datetime(ts: i64) -> sea_orm::prelude::DateTime {
    Utc.timestamp_millis_opt(ts).unwrap().naive_utc()
}

fn datetime_to_ts(dt: sea_orm::prelude::DateTime) -> i64 {
    dt.and_utc().timestamp_millis()
}

fn opt_datetime_to_ts(dt: Option<sea_orm::prelude::DateTime>) -> Option<i64> {
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

fn event_model_to_record(m: execution_event::Model) -> ExecutionEventRecord {
    ExecutionEventRecord {
        id: m.id,
        run_id: m.run_id,
        sequence: m.sequence,
        event_type: m.event_type,
        payload: m.payload,
        delivered: m.delivered,
        expires_at: datetime_to_ts(m.expires_at),
        created_at: datetime_to_ts(m.created_at),
    }
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
        let now = chrono::Utc::now().naive_utc();
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
        model.updated_at = Set(chrono::Utc::now().naive_utc());

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
        let expired = expired_runs(chrono::Utc::now().naive_utc());
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
            self.delete_run_children(&run_ids)
                .await
                .map_err(|e| StateStoreError::Database(e.to_string()))?;
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

        let now = chrono::Utc::now().naive_utc();
        let models: Vec<execution_event::ActiveModel> = events
            .iter()
            .map(|e| execution_event::ActiveModel {
                id: Set(e.id.clone()),
                run_id: Set(e.run_id.clone()),
                sequence: Set(e.sequence),
                event_type: Set(e.event_type.clone()),
                payload: Set(e.payload.clone()),
                delivered: Set(false),
                expires_at: Set(ts_to_datetime(e.expires_at)),
                created_at: Set(now),
            })
            .collect();

        let count = models.len() as i32;
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

        Ok(results.into_iter().map(event_model_to_record).collect())
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
        let now = chrono::Utc::now().naive_utc();
        let outcome = delete_in_batches::<execution_event::Entity>(
            self.db.as_ref(),
            self.dialect,
            Condition::all().add(execution_event::Column::ExpiresAt.lt(now)),
            EXECUTION_WRITE_CHUNK,
            Some(MAX_CHUNKS_PER_CLEANUP),
        )
        .await
        .map_err(|e| StateStoreError::Database(e.to_string()))?;
        if outcome.stopped_early {
            tracing::warn!(
                deleted = outcome.rows,
                max_chunks = MAX_CHUNKS_PER_CLEANUP,
                "Expired event cleanup hit its budget; the rest is removed next call"
            );
        }

        Ok(outcome.rows as i64)
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
                >= large_payload.to_string().len() - small_payload.to_string().len()
                    - JSON_NUMBER_BYTES
        );
    }
}
