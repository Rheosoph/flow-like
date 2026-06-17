//! PostgreSQL state store implementation using SeaORM
//!
//! This backend uses the existing Prisma-generated schema via SeaORM entities.
//! TTL cleanup is manual - call `delete_expired_runs/events` periodically.

use super::types::*;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use std::sync::Arc;

use crate::entity::{
    execution_event, execution_run,
    sea_orm_active_enums::{RunMode as EntityRunMode, RunStatus as EntityRunStatus},
};

#[derive(Debug, Clone)]
pub struct PostgresStateStore {
    db: Arc<DatabaseConnection>,
}

impl PostgresStateStore {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
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
            id: Set(input.id),
            board_id: Set(input.board_id),
            version: Set(input.version),
            event_id: Set(input.event_id),
            node_id: Set(None),
            status: Set(EntityRunStatus::Pending),
            mode: Set(type_run_mode_to_entity(input.mode)),
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

        let result = model
            .update(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(run_model_to_record(result))
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

    async fn delete_expired_runs(&self) -> Result<i64, StateStoreError> {
        let now = chrono::Utc::now().naive_utc();
        let result = execution_run::Entity::delete_many()
            .filter(
                Condition::all()
                    .add(execution_run::Column::ExpiresAt.is_not_null())
                    .add(execution_run::Column::ExpiresAt.lt(now)),
            )
            .exec(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(result.rows_affected as i64)
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
        execution_event::Entity::insert_many(models)
            .exec(self.db.as_ref())
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

    async fn mark_events_delivered(&self, event_ids: &[String]) -> Result<(), StateStoreError> {
        if event_ids.is_empty() {
            return Ok(());
        }

        execution_event::Entity::update_many()
            .col_expr(
                execution_event::Column::Delivered,
                sea_orm::sea_query::Expr::value(true),
            )
            .filter(execution_event::Column::Id.is_in(event_ids.to_vec()))
            .exec(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(())
    }

    async fn delete_expired_events(&self) -> Result<i64, StateStoreError> {
        let now = chrono::Utc::now().naive_utc();
        let result = execution_event::Entity::delete_many()
            .filter(execution_event::Column::ExpiresAt.lt(now))
            .exec(self.db.as_ref())
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(result.rows_affected as i64)
    }
}
