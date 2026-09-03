//! Redis state store implementation with native TTL support
//!
//! Uses Redis EXPIRE for automatic TTL cleanup. Records are stored as JSON
//! in hash structures for efficient access patterns.

use super::{postgres::PostgresStateStore, types::*};
use async_trait::async_trait;
use futures::lock::Mutex;
use redis::{
    AsyncCommands, Client, ExistenceCheck, Script, SetExpiry, SetOptions,
    aio::MultiplexedConnection,
};
use sea_orm::DatabaseConnection;
use std::sync::Arc;

const RUN_PREFIX: &str = "exec:run:";
const EVENT_PREFIX: &str = "exec:event:";
const RUN_BY_APP_PREFIX: &str = "exec:app:runs:";
const EVENTS_BY_RUN_PREFIX: &str = "exec:run:events:";
const DEFAULT_TTL_SECS: i64 = 86400; // 24 hours
const UPDATE_MUTABLE_RUN_SCRIPT: &str = r#"
local current = redis.call('GET', KEYS[1])
if not current then
    return {0, ''}
end
local status = cjson.decode(current)['status']
if status == 'COMPLETED' or status == 'FAILED' or status == 'CANCELLED' or status == 'TIMEOUT' then
    return {2, current}
end
redis.call('SET', KEYS[1], ARGV[1], 'EX', ARGV[2])
return {1, ARGV[1]}
"#;
const CREATE_EVENT_SCRIPT: &str = r#"
local created = redis.call('SET', KEYS[1], ARGV[1], 'EX', ARGV[2], 'NX')
if not created then
    return 0
end
redis.call('ZADD', KEYS[2], ARGV[3], ARGV[4])
redis.call('EXPIRE', KEYS[2], ARGV[2])
return 1
"#;

#[derive(Debug)]
pub struct RedisStateStore {
    conn: Arc<Mutex<MultiplexedConnection>>,
    source_run_store: Option<PostgresStateStore>,
}

impl RedisStateStore {
    pub async fn new(url: &str) -> Result<Self, StateStoreError> {
        Self::new_with_source(url, None).await
    }

    pub async fn new_with_source(
        url: &str,
        source_db: Option<Arc<DatabaseConnection>>,
    ) -> Result<Self, StateStoreError> {
        let client = Client::open(url).map_err(|e| StateStoreError::Connection(e.to_string()))?;
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| StateStoreError::Connection(e.to_string()))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            source_run_store: postgres_source(source_db),
        })
    }

    pub async fn from_env() -> Result<Self, StateStoreError> {
        Self::from_env_with_source(None).await
    }

    pub async fn from_env_with_source(
        source_db: Option<Arc<DatabaseConnection>>,
    ) -> Result<Self, StateStoreError> {
        let url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());
        Self::new_with_source(&url, source_db).await
    }

    fn run_key(id: &str) -> String {
        format!("{RUN_PREFIX}{id}")
    }

    fn event_key(id: &str) -> String {
        format!("{EVENT_PREFIX}{id}")
    }

    fn runs_by_app_key(app_id: &str) -> String {
        format!("{RUN_BY_APP_PREFIX}{app_id}")
    }

    fn events_by_run_key(run_id: &str) -> String {
        format!("{EVENTS_BY_RUN_PREFIX}{run_id}")
    }

    fn calc_ttl(expires_at: Option<i64>) -> i64 {
        let now_ms = chrono::Utc::now().timestamp_millis();
        expires_at
            .map(|e| ((e - now_ms) / 1000).max(1))
            .unwrap_or(DEFAULT_TTL_SECS)
    }

    async fn read_run(&self, run_id: &str) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        let key = Self::run_key(run_id);
        let mut conn = self.conn.lock().await;
        let json: Option<String> = conn
            .get(&key)
            .await
            .map_err(|error: redis::RedisError| StateStoreError::Database(error.to_string()))?;

        json.map(|json| {
            serde_json::from_str(&json)
                .map_err(|error| StateStoreError::Serialization(error.to_string()))
        })
        .transpose()
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
        if source_run_expired(&record, chrono::Utc::now().timestamp_millis()) {
            // An expired row would get the 1-second minimum TTL below and the
            // 500ms poll loop would re-import it forever. Treat it as gone.
            return Ok(None);
        }

        let json = serde_json::to_string(&record)
            .map_err(|error| StateStoreError::Serialization(error.to_string()))?;
        let ttl = Self::calc_ttl(record.expires_at);
        let run_key = Self::run_key(run_id);
        let app_key = Self::runs_by_app_key(&record.app_id);
        let options = SetOptions::default()
            .conditional_set(ExistenceCheck::NX)
            .with_expiration(SetExpiry::EX(ttl as u64));

        let mut conn = self.conn.lock().await;
        let inserted: Option<String> = conn
            .set_options(&run_key, &json, options)
            .await
            .map_err(|error: redis::RedisError| StateStoreError::Database(error.to_string()))?;
        if inserted.is_some() {
            redis::pipe()
                .zadd(&app_key, run_id, record.created_at as f64)
                .expire(&app_key, ttl)
                .query_async::<()>(&mut *conn)
                .await
                .map_err(|error| StateStoreError::Database(error.to_string()))?;
        }
        drop(conn);

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
                "Redis source-run import for '{run_id}' did not persist a run"
            ))),
        }
    }
}

#[async_trait]
impl ExecutionStateStore for RedisStateStore {
    fn backend_name(&self) -> &'static str {
        "redis"
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
            expires_at: input.expires_at,
            user_id: input.user_id,
            technical_user_id: input.technical_user_id,
            app_id: input.app_id.clone(),
            created_at: now,
            updated_at: now,
        };

        let json = serde_json::to_string(&record)
            .map_err(|e| StateStoreError::Serialization(e.to_string()))?;

        let ttl = Self::calc_ttl(record.expires_at);
        let key = Self::run_key(&input.id);
        let app_key = Self::runs_by_app_key(&input.app_id);

        let mut conn = self.conn.lock().await;
        redis::pipe()
            .set_ex(&key, &json, ttl as u64)
            .zadd(&app_key, &input.id, now as f64)
            .expire(&app_key, ttl)
            .query_async::<()>(&mut *conn)
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
        let mut record = self
            .get_run(run_id)
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

        let json = serde_json::to_string(&record)
            .map_err(|e| StateStoreError::Serialization(e.to_string()))?;

        let ttl = Self::calc_ttl(record.expires_at);
        let key = Self::run_key(run_id);

        let mut conn = self.conn.lock().await;
        let (outcome, persisted): (i32, String) = Script::new(UPDATE_MUTABLE_RUN_SCRIPT)
            .key(&key)
            .arg(&json)
            .arg(ttl)
            .invoke_async(&mut *conn)
            .await
            .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?;

        match outcome {
            1 => Ok(record),
            2 => serde_json::from_str(&persisted)
                .map_err(|error| StateStoreError::Serialization(error.to_string())),
            _ => Err(StateStoreError::NotFound),
        }
    }

    async fn list_runs_for_app(
        &self,
        app_id: &str,
        limit: i32,
        cursor: Option<&str>,
    ) -> Result<Vec<ExecutionRunRecord>, StateStoreError> {
        let app_key = Self::runs_by_app_key(app_id);
        let mut conn = self.conn.lock().await;

        // Get run IDs from sorted set (newest first)
        let ids: Vec<String> = if let Some(cursor_id) = cursor {
            let cursor_score: Option<f64> = conn
                .zscore(&app_key, cursor_id)
                .await
                .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?;

            if let Some(score) = cursor_score {
                conn.zrevrangebyscore_limit(
                    &app_key,
                    format!("({}", score),
                    "-inf",
                    0,
                    limit as isize,
                )
                .await
                .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?
            } else {
                Vec::new()
            }
        } else {
            conn.zrevrange(&app_key, 0, (limit - 1) as isize)
                .await
                .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?
        };
        drop(conn);

        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(record) = self.get_run(&id).await? {
                records.push(record);
            }
        }

        Ok(records)
    }

    async fn delete_expired_runs(&self) -> Result<i64, StateStoreError> {
        // Redis TTL handles expiration automatically
        Ok(0)
    }

    async fn push_events(&self, events: Vec<CreateEventInput>) -> Result<i32, StateStoreError> {
        if events.is_empty() {
            return Ok(0);
        }

        let now = chrono::Utc::now().timestamp_millis();

        // SET NX and the run index update execute as one Redis script per
        // event, so a retry is a successful no-op and cannot reset delivered
        // state. Pipelining the EVALs sends the whole batch in one round trip
        // instead of one serialized EVAL per event under the connection lock.
        let mut pipe = redis::pipe();
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

            let json = serde_json::to_string(&record)
                .map_err(|e| StateStoreError::Serialization(e.to_string()))?;

            let ttl = Self::calc_ttl(Some(event.expires_at));
            let key = Self::event_key(&event.id);
            let run_events_key = Self::events_by_run_key(&event.run_id);

            pipe.cmd("EVAL")
                .arg(CREATE_EVENT_SCRIPT)
                .arg(2)
                .arg(&key)
                .arg(&run_events_key)
                .arg(&json)
                .arg(ttl)
                .arg(event.sequence)
                .arg(&event.id);
        }

        let mut conn = self.conn.lock().await;
        pipe.query_async::<Vec<i32>>(&mut *conn)
            .await
            .map_err(|error| StateStoreError::Database(error.to_string()))?;

        Ok(events.len() as i32)
    }

    async fn get_events(
        &self,
        query: EventQuery,
    ) -> Result<Vec<ExecutionEventRecord>, StateStoreError> {
        let run_events_key = Self::events_by_run_key(&query.run_id);
        let mut conn = self.conn.lock().await;

        let min_score = query.after_sequence.map(|s| s + 1).unwrap_or(0);
        let ids: Vec<String> = if let Some(limit) = query.limit {
            conn.zrangebyscore_limit(&run_events_key, min_score, "+inf", 0, limit as isize)
                .await
                .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?
        } else {
            conn.zrangebyscore(&run_events_key, min_score, "+inf")
                .await
                .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?
        };

        let mut records = Vec::with_capacity(ids.len());
        for id in &ids {
            let key = Self::event_key(id);
            let json: Option<String> = conn
                .get(&key)
                .await
                .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?;

            if let Some(j) = json {
                let record: ExecutionEventRecord = serde_json::from_str(&j)
                    .map_err(|e| StateStoreError::Serialization(e.to_string()))?;
                if !query.only_undelivered || !record.delivered {
                    records.push(record);
                }
            }
        }

        Ok(records)
    }

    async fn get_max_sequence(&self, run_id: &str) -> Result<i32, StateStoreError> {
        let run_events_key = Self::events_by_run_key(run_id);
        let mut conn = self.conn.lock().await;

        let result: Vec<(String, f64)> = conn
            .zrevrange_withscores(&run_events_key, 0, 0)
            .await
            .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?;

        Ok(result.first().map(|(_, score)| *score as i32).unwrap_or(0))
    }

    async fn mark_events_delivered(
        &self,
        run_id: &str,
        event_ids: &[String],
    ) -> Result<(), StateStoreError> {
        if event_ids.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock().await;

        for id in event_ids {
            let key = Self::event_key(id);
            let json: Option<String> = conn
                .get(&key)
                .await
                .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?;

            if let Some(j) = json {
                let mut record: ExecutionEventRecord = serde_json::from_str(&j)
                    .map_err(|e| StateStoreError::Serialization(e.to_string()))?;
                if record.run_id != run_id {
                    continue;
                }
                record.delivered = true;

                let new_json = serde_json::to_string(&record)
                    .map_err(|e| StateStoreError::Serialization(e.to_string()))?;

                let ttl = Self::calc_ttl(Some(record.expires_at));
                conn.set_ex::<&str, &str, ()>(&key, &new_json, ttl as u64)
                    .await
                    .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?;
            }
        }

        Ok(())
    }

    async fn delete_expired_events(&self) -> Result<i64, StateStoreError> {
        // Redis TTL handles expiration automatically
        Ok(0)
    }
}

fn postgres_source(source_db: Option<Arc<DatabaseConnection>>) -> Option<PostgresStateStore> {
    source_db.map(PostgresStateStore::new)
}

#[cfg(test)]
mod stateless_import_tests {
    use super::*;

    fn queue_run() -> ExecutionRunRecord {
        ExecutionRunRecord {
            id: "run-1".into(),
            board_id: "board-1".into(),
            version: Some("7".into()),
            event_id: Some("event-1".into()),
            status: RunStatus::Pending,
            mode: RunMode::Queue,
            run_variant: RunVariant::Primary,
            variant_name: None,
            shadow_of_run_id: None,
            regression_run_id: None,
            input_payload_len: 12,
            output_payload_len: 0,
            error_message: None,
            progress: 0,
            current_step: None,
            started_at: None,
            completed_at: None,
            expires_at: Some(1_900_000_000_000),
            user_id: Some("user-1".into()),
            technical_user_id: None,
            app_id: "app-1".into(),
            created_at: 1_800_000_000_000,
            updated_at: 1_800_000_000_001,
        }
    }

    #[test]
    fn stateless_lambda_redis_constructor_accepts_sql_source() {
        assert!(postgres_source(Some(Arc::new(DatabaseConnection::Disconnected))).is_some());
    }

    #[test]
    fn stateless_lambda_redis_import_payload_round_trips_queue_run() {
        let expected = queue_run();
        let encoded = serde_json::to_string(&expected).expect("queue run should serialize");
        let decoded: ExecutionRunRecord =
            serde_json::from_str(&encoded).expect("queue run should deserialize");

        assert_eq!(decoded.id, expected.id);
        assert_eq!(decoded.app_id, expected.app_id);
        assert_eq!(decoded.mode, RunMode::Queue);
        assert_eq!(decoded.status, RunStatus::Pending);
        assert_eq!(decoded.updated_at, expected.updated_at);
    }

    #[test]
    fn stateless_lambda_redis_update_script_guards_every_terminal_status() {
        for status in ["COMPLETED", "FAILED", "CANCELLED", "TIMEOUT"] {
            assert!(UPDATE_MUTABLE_RUN_SCRIPT.contains(status));
        }
    }

    #[test]
    fn stateless_lambda_redis_event_create_is_first_write_wins() {
        assert!(CREATE_EVENT_SCRIPT.contains("'NX'"));
        let guard = CREATE_EVENT_SCRIPT
            .find("if not created")
            .expect("duplicate guard should exist");
        let index_write = CREATE_EVENT_SCRIPT
            .find("'ZADD'")
            .expect("event index write should exist");
        assert!(guard < index_write);
    }
}
