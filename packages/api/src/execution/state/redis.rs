//! Redis state store implementation with native TTL support
//!
//! Uses Redis EXPIRE for automatic TTL cleanup. Records are stored as JSON
//! in hash structures for efficient access patterns.

use super::types::*;
use async_trait::async_trait;
use futures::lock::Mutex;
use redis::{AsyncCommands, Client, aio::MultiplexedConnection};
use std::sync::Arc;

const RUN_PREFIX: &str = "exec:run:";
const EVENT_PREFIX: &str = "exec:event:";
const RUN_BY_APP_PREFIX: &str = "exec:app:runs:";
const EVENTS_BY_RUN_PREFIX: &str = "exec:run:events:";
const DEFAULT_TTL_SECS: i64 = 86400; // 24 hours

#[derive(Debug)]
pub struct RedisStateStore {
    conn: Arc<Mutex<MultiplexedConnection>>,
}

impl RedisStateStore {
    pub async fn new(url: &str) -> Result<Self, StateStoreError> {
        let client = Client::open(url).map_err(|e| StateStoreError::Connection(e.to_string()))?;
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| StateStoreError::Connection(e.to_string()))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn from_env() -> Result<Self, StateStoreError> {
        let url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());
        Self::new(&url).await
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
        let key = Self::run_key(run_id);
        let mut conn = self.conn.lock().await;

        let json: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?;

        match json {
            Some(j) => {
                let record: ExecutionRunRecord = serde_json::from_str(&j)
                    .map_err(|e| StateStoreError::Serialization(e.to_string()))?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
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

        let json = serde_json::to_string(&record)
            .map_err(|e| StateStoreError::Serialization(e.to_string()))?;

        let ttl = Self::calc_ttl(record.expires_at);
        let key = Self::run_key(run_id);

        let mut conn = self.conn.lock().await;
        conn.set_ex::<&str, &str, ()>(&key, &json, ttl as u64)
            .await
            .map_err(|e: redis::RedisError| StateStoreError::Database(e.to_string()))?;

        Ok(record)
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
        let mut conn = self.conn.lock().await;
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

            pipe.set_ex(&key, &json, ttl as u64);
            pipe.zadd(&run_events_key, &event.id, event.sequence as f64);
            pipe.expire(&run_events_key, ttl);
        }

        pipe.query_async::<()>(&mut *conn)
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

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

    async fn mark_events_delivered(&self, event_ids: &[String]) -> Result<(), StateStoreError> {
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
