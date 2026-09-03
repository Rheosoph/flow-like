//! DynamoDB state store implementation with native TTL support
//!
//! Uses DynamoDB Time To Live (TTL) for automatic expiration.
//! Large payloads (>100KB) are stored via FlowLikeStore under "polling/{run_id}/{event_id}"
//! Tables: ExecutionRuns, ExecutionEvents with GSIs for app/run lookups.

use super::{postgres::PostgresStateStore, types::*};
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_dynamodb::{
    Client,
    types::{AttributeValue, KeyType, ReturnValue, ScalarAttributeType, WriteRequest},
};
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{ObjectStore, path::Path},
};
use futures::{StreamExt, TryStreamExt, stream};
use sea_orm::DatabaseConnection;
use std::{collections::HashMap, sync::Arc, time::Duration};

const RUNS_TABLE: &str = "ExecutionRuns";
const EVENTS_TABLE: &str = "ExecutionEvents";
const APP_INDEX: &str = "AppIdIndex";
const RUN_INDEX: &str = "RunIdIndex";
const DEFAULT_TTL_SECS: i64 = 86400;
const PAYLOAD_SIZE_THRESHOLD: usize = 100 * 1024; // 100KB - offload to object store above this
const POLLING_PREFIX: &str = "polling";
const EVENT_WRITE_MAX_ATTEMPTS: usize = 6;
const EVENT_WRITE_BASE_DELAY_MS: u64 = 25;
const EVENT_WRITE_CONCURRENCY: usize = 16;
const LEASE_UPDATE_ATTEMPTS: usize = 3;
const SOURCE_IMPORT_CONDITION: &str = "attribute_not_exists(id)";
const UNLEASED_UPDATE_CONDITION: &str = "#status IN (:pending, :running) AND #updated_at = :expected_updated_at AND attribute_not_exists(#bound_job_id)";
const CLAIM_LEASE_CONDITION: &str = "#app_id = :app_id AND #status IN (:pending, :running) AND #updated_at = :expected_updated_at AND (attribute_not_exists(#expires_at) OR #expires_at > :now) AND (attribute_not_exists(#bound_job_id) OR #bound_job_id = :job_id) AND (attribute_not_exists(#lease_token) OR #lease_token = :lease_token OR #lease_expires_at <= :now)";
const TERMINAL_LEASE_CONDITION: &str = "#app_id = :app_id AND #status IN (:pending, :running) AND #updated_at = :expected_updated_at AND #bound_job_id = :job_id AND #lease_token = :lease_token AND #lease_expires_at > :now";

fn event_write_retry_delay(attempt: usize) -> Duration {
    let shift = attempt.min(5) as u32;
    Duration::from_millis(EVENT_WRITE_BASE_DELAY_MS.saturating_mul(1_u64 << shift))
}

fn canonical_execution_event_id(run_id: &str, sequence: i32) -> String {
    let digest = blake3::hash(format!("{run_id}:{sequence}").as_bytes());
    format!("evt-{}", digest.to_hex())
}

fn has_canonical_identity(event: &CreateEventInput) -> bool {
    event.id == canonical_execution_event_id(&event.run_id, event.sequence)
}

pub struct DynamoDbStateStore {
    client: Client,
    content_store: Arc<FlowLikeStore>,
    source_run_store: Option<PostgresStateStore>,
    runs_table: String,
    events_table: String,
}

impl std::fmt::Debug for DynamoDbStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamoDbStateStore")
            .field("runs_table", &self.runs_table)
            .field("events_table", &self.events_table)
            .finish()
    }
}

impl DynamoDbStateStore {
    /// Create from AWS SDK config and an existing FlowLikeStore (preferred)
    pub fn new(aws_config: &SdkConfig, content_store: Arc<FlowLikeStore>) -> Self {
        Self::new_with_source(aws_config, content_store, None)
    }

    /// Create with the canonical SQL run store used by API dispatch.
    pub fn new_with_source(
        aws_config: &SdkConfig,
        content_store: Arc<FlowLikeStore>,
        source_db: Option<Arc<DatabaseConnection>>,
    ) -> Self {
        let prefix = std::env::var("DYNAMODB_TABLE_PREFIX").unwrap_or_default();
        Self {
            client: Client::new(aws_config),
            content_store,
            source_run_store: postgres_source(source_db),
            runs_table: format!("{prefix}{RUNS_TABLE}"),
            events_table: format!("{prefix}{EVENTS_TABLE}"),
        }
    }

    /// Fallback constructor when AppState is not available (e.g., standalone Lambda)
    /// Requires AWS environment credentials and CDN_BUCKET_NAME env var
    pub async fn from_env() -> Result<Self, StateStoreError> {
        use flow_like_storage::object_store::aws::AmazonS3Builder;

        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

        let bucket_name = std::env::var("CDN_BUCKET_NAME")
            .or_else(|_| std::env::var("CONTENT_BUCKET"))
            .map_err(|_| StateStoreError::Configuration("CDN_BUCKET_NAME not set".into()))?;

        let mut builder = AmazonS3Builder::from_env().with_bucket_name(&bucket_name);

        if let Ok(endpoint) = std::env::var("CDN_BUCKET_ENDPOINT") {
            builder = builder.with_endpoint(endpoint);
        }

        let store = builder.build().map_err(|e| {
            StateStoreError::Configuration(format!("Failed to create S3 store: {}", e))
        })?;

        Ok(Self::new(
            &config,
            Arc::new(FlowLikeStore::AWS(Arc::new(store))),
        ))
    }

    /// `consistent` is required only where a read feeds OCC or lease decisions
    /// (lease claim/validate, conditional-update retry loops). Plain reads —
    /// the 500ms user poll loop above all — stay eventually consistent at half
    /// the read cost: the conditional-write guards (`attribute_not_exists` on
    /// import/create, the lease condition expressions) keep correctness even
    /// when such a read is stale.
    async fn read_run_item(
        &self,
        run_id: &str,
        consistent: bool,
    ) -> Result<Option<HashMap<String, AttributeValue>>, StateStoreError> {
        let result = self
            .client
            .get_item()
            .table_name(&self.runs_table)
            .key("id", AttributeValue::S(run_id.to_string()))
            .consistent_read(consistent)
            .send()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(result.item)
    }

    async fn read_run(
        &self,
        run_id: &str,
        consistent: bool,
    ) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        self.read_run_item(run_id, consistent)
            .await?
            .as_ref()
            .map(item_to_run)
            .transpose()
    }

    /// Async dispatch first writes the canonical audit row to SQL. A callback
    /// can arrive on a different Lambda instance, so import that row when the
    /// selected live-state backend has not seen the run yet. The conditional
    /// put prevents two cold instances from overwriting each other.
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
            // Re-importing would resurrect a TTL-deleted item with a past
            // `ttl` on every delete/import cycle. Treat the run as gone.
            return Ok(None);
        }

        let write = self
            .client
            .put_item()
            .table_name(&self.runs_table)
            .set_item(Some(run_to_item(&record)))
            .condition_expression(SOURCE_IMPORT_CONDITION)
            .send()
            .await;

        let scoped = |record: ExecutionRunRecord| {
            app_id
                .map(|expected| record.app_id == expected)
                .unwrap_or(true)
                .then_some(record)
        };
        match write {
            Ok(_) => Ok(self.read_run(run_id, true).await?.and_then(scoped)),
            Err(write_error) => match self.read_run(run_id, true).await {
                Ok(Some(existing)) => Ok(scoped(existing)),
                Ok(None) => Err(StateStoreError::Database(format!(
                    "DynamoDB source-run import failed: {write_error}"
                ))),
                Err(read_error) => Err(StateStoreError::Database(format!(
                    "DynamoDB source-run import failed: {write_error}; consistent re-read failed: {read_error}"
                ))),
            },
        }
    }

    async fn store_large_payload(
        &self,
        run_id: &str,
        event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<String, StateStoreError> {
        let body = serde_json::to_vec(payload)
            .map_err(|e| StateStoreError::Serialization(e.to_string()))?;
        let run_hash = blake3::hash(run_id.as_bytes()).to_hex();
        let event_hash = blake3::hash(event_id.as_bytes()).to_hex();
        let payload_hash = blake3::hash(&body).to_hex();
        let path = Path::from(format!(
            "{POLLING_PREFIX}/{run_hash}/{event_hash}-{payload_hash}.json"
        ));

        self.content_store
            .as_generic()
            .put(&path, body.into())
            .await
            .map_err(|e| StateStoreError::Database(format!("Object store put failed: {}", e)))?;

        Ok(format!("store://{}", path))
    }

    async fn event_exists(&self, event_id: &str) -> Result<bool, StateStoreError> {
        let result = self
            .client
            .get_item()
            .table_name(&self.events_table)
            .key("id", AttributeValue::S(event_id.to_string()))
            .projection_expression("id")
            .send()
            .await
            .map_err(|error| StateStoreError::Database(error.to_string()))?;
        Ok(result.item.is_some())
    }

    async fn put_event_if_absent(
        &self,
        item: HashMap<String, AttributeValue>,
    ) -> Result<(), StateStoreError> {
        for attempt in 0..EVENT_WRITE_MAX_ATTEMPTS {
            let write = self
                .client
                .put_item()
                .table_name(&self.events_table)
                .set_item(Some(item.clone()))
                .condition_expression("attribute_not_exists(id)")
                .send()
                .await;
            match write {
                Ok(_) => return Ok(()),
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|error| error.is_conditional_check_failed_exception()) =>
                {
                    // A canonical retry is a successful no-op. The original
                    // payload and delivered state remain authoritative.
                    return Ok(());
                }
                Err(error) if attempt + 1 == EVENT_WRITE_MAX_ATTEMPTS => {
                    return Err(StateStoreError::Database(format!(
                        "DynamoDB execution event create failed after {} attempts: {error}",
                        EVENT_WRITE_MAX_ATTEMPTS
                    )));
                }
                Err(_) => tokio::time::sleep(event_write_retry_delay(attempt)).await,
            }
        }
        unreachable!("event write retry loop has at least one attempt")
    }

    async fn batch_write_events(
        &self,
        mut pending: Vec<WriteRequest>,
    ) -> Result<(), StateStoreError> {
        for attempt in 0..EVENT_WRITE_MAX_ATTEMPTS {
            let result = self
                .client
                .batch_write_item()
                .request_items(&self.events_table, pending)
                .send()
                .await
                .map_err(|e| StateStoreError::Database(e.to_string()))?;

            pending = result
                .unprocessed_items
                .and_then(|mut items| items.remove(&self.events_table))
                .unwrap_or_default();
            if pending.is_empty() {
                return Ok(());
            }
            if attempt + 1 == EVENT_WRITE_MAX_ATTEMPTS {
                return Err(StateStoreError::Database(format!(
                    "DynamoDB left {} execution event writes unprocessed after {} attempts",
                    pending.len(),
                    EVENT_WRITE_MAX_ATTEMPTS
                )));
            }

            tokio::time::sleep(event_write_retry_delay(attempt)).await;
        }
        unreachable!("event write retry loop has at least one attempt")
    }

    async fn fetch_large_payload(
        &self,
        store_ref: &str,
    ) -> Result<serde_json::Value, StateStoreError> {
        let path_str = store_ref
            .strip_prefix("store://")
            .or_else(|| {
                store_ref
                    .strip_prefix("s3://")
                    .and_then(|s| s.split_once('/').map(|x| x.1))
            })
            .unwrap_or(store_ref);
        let path = Path::from(path_str);

        let result = self
            .content_store
            .as_generic()
            .get(&path)
            .await
            .map_err(|e| StateStoreError::Database(format!("Object store get failed: {}", e)))?;

        let bytes = result
            .bytes()
            .await
            .map_err(|e| StateStoreError::Database(format!("Object store read failed: {}", e)))?;

        serde_json::from_slice(&bytes).map_err(|e| StateStoreError::Serialization(e.to_string()))
    }

    pub async fn create_tables_if_not_exist(&self) -> Result<(), StateStoreError> {
        // Create runs table with TTL
        let tables = self
            .client
            .list_tables()
            .send()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        let existing: Vec<_> = tables.table_names().iter().collect();

        if !existing.contains(&&self.runs_table) {
            self.client
                .create_table()
                .table_name(&self.runs_table)
                .attribute_definitions(
                    aws_sdk_dynamodb::types::AttributeDefinition::builder()
                        .attribute_name("id")
                        .attribute_type(ScalarAttributeType::S)
                        .build()
                        .unwrap(),
                )
                .attribute_definitions(
                    aws_sdk_dynamodb::types::AttributeDefinition::builder()
                        .attribute_name("appId")
                        .attribute_type(ScalarAttributeType::S)
                        .build()
                        .unwrap(),
                )
                .attribute_definitions(
                    aws_sdk_dynamodb::types::AttributeDefinition::builder()
                        .attribute_name("createdAt")
                        .attribute_type(ScalarAttributeType::N)
                        .build()
                        .unwrap(),
                )
                .key_schema(
                    aws_sdk_dynamodb::types::KeySchemaElement::builder()
                        .attribute_name("id")
                        .key_type(KeyType::Hash)
                        .build()
                        .unwrap(),
                )
                .global_secondary_indexes(
                    aws_sdk_dynamodb::types::GlobalSecondaryIndex::builder()
                        .index_name(APP_INDEX)
                        .key_schema(
                            aws_sdk_dynamodb::types::KeySchemaElement::builder()
                                .attribute_name("appId")
                                .key_type(KeyType::Hash)
                                .build()
                                .unwrap(),
                        )
                        .key_schema(
                            aws_sdk_dynamodb::types::KeySchemaElement::builder()
                                .attribute_name("createdAt")
                                .key_type(KeyType::Range)
                                .build()
                                .unwrap(),
                        )
                        .projection(
                            aws_sdk_dynamodb::types::Projection::builder()
                                .projection_type(aws_sdk_dynamodb::types::ProjectionType::All)
                                .build(),
                        )
                        .build()
                        .unwrap(),
                )
                .billing_mode(aws_sdk_dynamodb::types::BillingMode::PayPerRequest)
                .send()
                .await
                .map_err(|e| StateStoreError::Database(e.to_string()))?;

            // Enable TTL on runs table
            self.client
                .update_time_to_live()
                .table_name(&self.runs_table)
                .time_to_live_specification(
                    aws_sdk_dynamodb::types::TimeToLiveSpecification::builder()
                        .attribute_name("ttl")
                        .enabled(true)
                        .build()
                        .unwrap(),
                )
                .send()
                .await
                .ok(); // Ignore error if TTL already enabled
        }

        if !existing.contains(&&self.events_table) {
            self.client
                .create_table()
                .table_name(&self.events_table)
                .attribute_definitions(
                    aws_sdk_dynamodb::types::AttributeDefinition::builder()
                        .attribute_name("id")
                        .attribute_type(ScalarAttributeType::S)
                        .build()
                        .unwrap(),
                )
                .attribute_definitions(
                    aws_sdk_dynamodb::types::AttributeDefinition::builder()
                        .attribute_name("runId")
                        .attribute_type(ScalarAttributeType::S)
                        .build()
                        .unwrap(),
                )
                .attribute_definitions(
                    aws_sdk_dynamodb::types::AttributeDefinition::builder()
                        .attribute_name("sequence")
                        .attribute_type(ScalarAttributeType::N)
                        .build()
                        .unwrap(),
                )
                .key_schema(
                    aws_sdk_dynamodb::types::KeySchemaElement::builder()
                        .attribute_name("id")
                        .key_type(KeyType::Hash)
                        .build()
                        .unwrap(),
                )
                .global_secondary_indexes(
                    aws_sdk_dynamodb::types::GlobalSecondaryIndex::builder()
                        .index_name(RUN_INDEX)
                        .key_schema(
                            aws_sdk_dynamodb::types::KeySchemaElement::builder()
                                .attribute_name("runId")
                                .key_type(KeyType::Hash)
                                .build()
                                .unwrap(),
                        )
                        .key_schema(
                            aws_sdk_dynamodb::types::KeySchemaElement::builder()
                                .attribute_name("sequence")
                                .key_type(KeyType::Range)
                                .build()
                                .unwrap(),
                        )
                        .projection(
                            aws_sdk_dynamodb::types::Projection::builder()
                                .projection_type(aws_sdk_dynamodb::types::ProjectionType::All)
                                .build(),
                        )
                        .build()
                        .unwrap(),
                )
                .billing_mode(aws_sdk_dynamodb::types::BillingMode::PayPerRequest)
                .send()
                .await
                .map_err(|e| StateStoreError::Database(e.to_string()))?;

            // Enable TTL on events table
            self.client
                .update_time_to_live()
                .table_name(&self.events_table)
                .time_to_live_specification(
                    aws_sdk_dynamodb::types::TimeToLiveSpecification::builder()
                        .attribute_name("ttl")
                        .enabled(true)
                        .build()
                        .unwrap(),
                )
                .send()
                .await
                .ok();
        }

        Ok(())
    }
}

fn postgres_source(source_db: Option<Arc<DatabaseConnection>>) -> Option<PostgresStateStore> {
    source_db.map(PostgresStateStore::new)
}

fn run_to_item(r: &ExecutionRunRecord) -> HashMap<String, AttributeValue> {
    let mut item = HashMap::new();
    item.insert("id".into(), AttributeValue::S(r.id.clone()));
    item.insert("boardId".into(), AttributeValue::S(r.board_id.clone()));
    item.insert(
        "status".into(),
        AttributeValue::S(format!("{:?}", r.status).to_uppercase()),
    );
    item.insert(
        "mode".into(),
        AttributeValue::S(format!("{:?}", r.mode).to_uppercase()),
    );
    item.insert(
        "runVariant".into(),
        AttributeValue::S(format!("{:?}", r.run_variant).to_uppercase()),
    );
    item.insert(
        "inputPayloadLen".into(),
        AttributeValue::N(r.input_payload_len.to_string()),
    );
    item.insert(
        "outputPayloadLen".into(),
        AttributeValue::N(r.output_payload_len.to_string()),
    );
    item.insert("progress".into(), AttributeValue::N(r.progress.to_string()));
    item.insert("appId".into(), AttributeValue::S(r.app_id.clone()));
    item.insert(
        "createdAt".into(),
        AttributeValue::N(r.created_at.to_string()),
    );
    item.insert(
        "updatedAt".into(),
        AttributeValue::N(r.updated_at.to_string()),
    );

    if let Some(v) = &r.version {
        item.insert("version".into(), AttributeValue::S(v.clone()));
    }
    if let Some(e) = &r.event_id {
        item.insert("eventId".into(), AttributeValue::S(e.clone()));
    }
    if let Some(v) = &r.variant_name {
        item.insert("variantName".into(), AttributeValue::S(v.clone()));
    }
    if let Some(s) = &r.shadow_of_run_id {
        item.insert("shadowOfRunId".into(), AttributeValue::S(s.clone()));
    }
    if let Some(s) = &r.regression_run_id {
        item.insert("regressionRunId".into(), AttributeValue::S(s.clone()));
    }
    if let Some(e) = &r.error_message {
        item.insert("errorMessage".into(), AttributeValue::S(e.clone()));
    }
    if let Some(s) = &r.current_step {
        item.insert("currentStep".into(), AttributeValue::S(s.clone()));
    }
    if let Some(t) = r.started_at {
        item.insert("startedAt".into(), AttributeValue::N(t.to_string()));
    }
    if let Some(t) = r.completed_at {
        item.insert("completedAt".into(), AttributeValue::N(t.to_string()));
    }
    if let Some(t) = r.expires_at {
        item.insert("expiresAt".into(), AttributeValue::N(t.to_string()));
        item.insert("ttl".into(), AttributeValue::N((t / 1000).to_string()));
    }
    if let Some(u) = &r.user_id {
        item.insert("userId".into(), AttributeValue::S(u.clone()));
    }
    if let Some(u) = &r.technical_user_id {
        item.insert("technicalUserId".into(), AttributeValue::S(u.clone()));
    }

    item
}

fn item_optional_string(item: &HashMap<String, AttributeValue>, key: &str) -> Option<String> {
    item.get(key).and_then(|value| value.as_s().ok()).cloned()
}

fn item_optional_number(item: &HashMap<String, AttributeValue>, key: &str) -> Option<i64> {
    item.get(key)
        .and_then(|value| value.as_n().ok())
        .and_then(|value| value.parse().ok())
}

fn classify_lease_claim(
    item: &HashMap<String, AttributeValue>,
    app_id: &str,
    job_id: &str,
    lease_token: &str,
    now: i64,
) -> Result<Option<RunLeaseClaim>, StateStoreError> {
    let record = item_to_run(item)?;
    if record.app_id != app_id
        || record
            .expires_at
            .is_some_and(|expires_at| expires_at <= now)
    {
        return Err(StateStoreError::NotFound);
    }
    if record.status.is_terminal() {
        return Ok(Some(RunLeaseClaim::Terminal { run: record }));
    }
    if item_optional_string(item, "boundJobId")
        .as_deref()
        .is_some_and(|bound| bound != job_id)
    {
        return Err(StateStoreError::LeaseConflict(
            "run is bound to a different broker job".to_string(),
        ));
    }
    if item_optional_string(item, "leaseToken")
        .as_deref()
        .is_some_and(|token| token != lease_token)
    {
        match item_optional_number(item, "leaseExpiresAt") {
            Some(expires_at) if expires_at > now => {
                return Ok(Some(RunLeaseClaim::Busy {
                    run: record,
                    expires_at,
                }));
            }
            Some(_) => {}
            None => {
                return Err(StateStoreError::LeaseConflict(
                    "run has an invalid execution lease".to_string(),
                ));
            }
        }
    }
    Ok(None)
}

fn active_lease_record(
    item: &HashMap<String, AttributeValue>,
    app_id: &str,
    job_id: &str,
    lease_token: &str,
    now: i64,
) -> Result<ExecutionRunRecord, StateStoreError> {
    let record = item_to_run(item)?;
    if record.app_id != app_id
        || record
            .expires_at
            .is_some_and(|expires_at| expires_at <= now)
    {
        return Err(StateStoreError::NotFound);
    }
    let owned = item_optional_string(item, "boundJobId").as_deref() == Some(job_id)
        && item_optional_string(item, "leaseToken").as_deref() == Some(lease_token)
        && item_optional_number(item, "leaseExpiresAt").is_some_and(|expires_at| expires_at > now);
    if owned && !record.status.is_terminal() {
        Ok(record)
    } else {
        Err(StateStoreError::LeaseConflict(
            "callback is not from the current unexpired delivery owner".to_string(),
        ))
    }
}

fn apply_run_update(record: &mut ExecutionRunRecord, input: &UpdateRunInput, now: i64) {
    record.updated_at = now.max(record.updated_at.saturating_add(1));
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

fn item_to_run(
    item: &HashMap<String, AttributeValue>,
) -> Result<ExecutionRunRecord, StateStoreError> {
    let get_s = |k: &str| -> Result<String, StateStoreError> {
        item.get(k)
            .and_then(|v| v.as_s().ok())
            .cloned()
            .ok_or_else(|| StateStoreError::Serialization(format!("Missing {k}")))
    };
    let get_n = |k: &str| -> Result<i64, StateStoreError> {
        item.get(k)
            .and_then(|v| v.as_n().ok())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| StateStoreError::Serialization(format!("Missing {k}")))
    };
    let get_opt_s =
        |k: &str| -> Option<String> { item.get(k).and_then(|v| v.as_s().ok()).cloned() };
    let get_opt_n = |k: &str| -> Option<i64> {
        item.get(k)
            .and_then(|v| v.as_n().ok())
            .and_then(|s| s.parse().ok())
    };

    let status_str = get_s("status")?;
    let status = match status_str.as_str() {
        "PENDING" => RunStatus::Pending,
        "RUNNING" => RunStatus::Running,
        "COMPLETED" => RunStatus::Completed,
        "FAILED" => RunStatus::Failed,
        "CANCELLED" => RunStatus::Cancelled,
        "TIMEOUT" => RunStatus::Timeout,
        _ => {
            return Err(StateStoreError::Serialization(format!(
                "Invalid status: {status_str}"
            )));
        }
    };

    let mode_str = get_s("mode")?;
    let mode = match mode_str.as_str() {
        "LOCAL" => RunMode::Local,
        "HTTP" => RunMode::Http,
        "LAMBDA" => RunMode::Lambda,
        "KUBERNETES_ISOLATED" | "KUBERNETESISOLATED" => RunMode::KubernetesIsolated,
        "KUBERNETES_POOL" | "KUBERNETESPOOL" => RunMode::KubernetesPool,
        "FUNCTION" => RunMode::Function,
        "QUEUE" => RunMode::Queue,
        _ => {
            return Err(StateStoreError::Serialization(format!(
                "Invalid mode: {mode_str}"
            )));
        }
    };

    // Items written before the variant column existed have no attribute;
    // they are PRIMARY, matching the serde default of the JSON backends.
    let run_variant = match get_opt_s("runVariant").as_deref() {
        None | Some("PRIMARY") => RunVariant::Primary,
        Some("CANARY") => RunVariant::Canary,
        Some("SHADOW") => RunVariant::Shadow,
        Some("REGRESSION") => RunVariant::Regression,
        Some(other) => {
            return Err(StateStoreError::Serialization(format!(
                "Invalid runVariant: {other}"
            )));
        }
    };

    Ok(ExecutionRunRecord {
        id: get_s("id")?,
        board_id: get_s("boardId")?,
        version: get_opt_s("version"),
        event_id: get_opt_s("eventId"),
        status,
        mode,
        run_variant,
        variant_name: get_opt_s("variantName"),
        shadow_of_run_id: get_opt_s("shadowOfRunId"),
        regression_run_id: get_opt_s("regressionRunId"),
        input_payload_len: get_n("inputPayloadLen")?,
        output_payload_len: get_n("outputPayloadLen")?,
        error_message: get_opt_s("errorMessage"),
        progress: get_n("progress")? as i32,
        current_step: get_opt_s("currentStep"),
        started_at: get_opt_n("startedAt"),
        completed_at: get_opt_n("completedAt"),
        expires_at: get_opt_n("expiresAt"),
        user_id: get_opt_s("userId"),
        technical_user_id: get_opt_s("technicalUserId"),
        app_id: get_s("appId")?,
        created_at: get_n("createdAt")?,
        updated_at: get_n("updatedAt")?,
    })
}

fn event_to_item(
    e: &ExecutionEventRecord,
    payload_ref: Option<&str>,
) -> HashMap<String, AttributeValue> {
    let mut item = HashMap::new();
    item.insert("id".into(), AttributeValue::S(e.id.clone()));
    item.insert("runId".into(), AttributeValue::S(e.run_id.clone()));
    item.insert("sequence".into(), AttributeValue::N(e.sequence.to_string()));
    item.insert("eventType".into(), AttributeValue::S(e.event_type.clone()));

    // If payload is in S3, store reference; otherwise store inline
    if let Some(s3_ref) = payload_ref {
        item.insert("payloadRef".into(), AttributeValue::S(s3_ref.to_string()));
        item.insert("payload".into(), AttributeValue::S("{}".to_string())); // placeholder
    } else {
        item.insert("payload".into(), AttributeValue::S(e.payload.to_string()));
    }

    item.insert("delivered".into(), AttributeValue::Bool(e.delivered));
    item.insert(
        "expiresAt".into(),
        AttributeValue::N(e.expires_at.to_string()),
    );
    item.insert(
        "ttl".into(),
        AttributeValue::N((e.expires_at / 1000).to_string()),
    );
    item.insert(
        "createdAt".into(),
        AttributeValue::N(e.created_at.to_string()),
    );
    item
}

/// Returns (event_record, optional_s3_ref)
fn item_to_event(
    item: &HashMap<String, AttributeValue>,
) -> Result<(ExecutionEventRecord, Option<String>), StateStoreError> {
    let get_s = |k: &str| -> Result<String, StateStoreError> {
        item.get(k)
            .and_then(|v| v.as_s().ok())
            .cloned()
            .ok_or_else(|| StateStoreError::Serialization(format!("Missing {k}")))
    };
    let get_n = |k: &str| -> Result<i64, StateStoreError> {
        item.get(k)
            .and_then(|v| v.as_n().ok())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| StateStoreError::Serialization(format!("Missing {k}")))
    };
    let get_opt_s =
        |k: &str| -> Option<String> { item.get(k).and_then(|v| v.as_s().ok()).cloned() };

    // Check if payload is stored in S3
    let payload_ref = get_opt_s("payloadRef");

    let payload = if payload_ref.is_some() {
        // Placeholder - will be fetched from S3 later
        serde_json::Value::Null
    } else {
        let payload_str = get_s("payload")?;
        serde_json::from_str(&payload_str)
            .map_err(|e| StateStoreError::Serialization(e.to_string()))?
    };

    let delivered = item
        .get("delivered")
        .and_then(|v| v.as_bool().ok())
        .copied()
        .unwrap_or(false);

    Ok((
        ExecutionEventRecord {
            id: get_s("id")?,
            run_id: get_s("runId")?,
            sequence: get_n("sequence")? as i32,
            event_type: get_s("eventType")?,
            payload,
            delivered,
            expires_at: get_n("expiresAt")?,
            created_at: get_n("createdAt")?,
        },
        payload_ref,
    ))
}

#[async_trait]
impl ExecutionStateStore for DynamoDbStateStore {
    fn backend_name(&self) -> &'static str {
        "dynamodb"
    }

    async fn create_run(
        &self,
        input: CreateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        let now = chrono::Utc::now().timestamp_millis();
        let expires_at = input.expires_at.unwrap_or(now + DEFAULT_TTL_SECS * 1000);

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

        let item = run_to_item(&record);

        self.client
            .put_item()
            .table_name(&self.runs_table)
            .set_item(Some(item))
            .send()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        Ok(record)
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        match self.read_run(run_id, false).await? {
            Some(record) => Ok(Some(record)),
            None => self.import_source_run(run_id, None).await,
        }
    }

    async fn get_run_for_app(
        &self,
        run_id: &str,
        app_id: &str,
    ) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
        match self.read_run(run_id, false).await? {
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
        // This read feeds the `#updated_at = :expected_updated_at` condition,
        // so it must be consistent to avoid spurious OCC conflicts.
        let mut record = match self.read_run(run_id, true).await? {
            Some(record) => record,
            None => self
                .import_source_run(run_id, None)
                .await?
                .ok_or(StateStoreError::NotFound)?,
        };

        if record.status.is_terminal() {
            return Ok(record);
        }

        let expected_updated_at = record.updated_at;
        apply_run_update(&mut record, &input, chrono::Utc::now().timestamp_millis());

        let item = run_to_item(&record);

        let write = self
            .client
            .put_item()
            .table_name(&self.runs_table)
            .set_item(Some(item))
            .condition_expression(UNLEASED_UPDATE_CONDITION)
            .expression_attribute_names("#status", "status")
            .expression_attribute_names("#updated_at", "updatedAt")
            .expression_attribute_names("#bound_job_id", "boundJobId")
            .expression_attribute_values(":pending", AttributeValue::S("PENDING".into()))
            .expression_attribute_values(":running", AttributeValue::S("RUNNING".into()))
            .expression_attribute_values(
                ":expected_updated_at",
                AttributeValue::N(expected_updated_at.to_string()),
            )
            .send()
            .await;

        if let Err(error) = write {
            if error
                .as_service_error()
                .is_some_and(|error| error.is_conditional_check_failed_exception())
            {
                if let Some(current) = self.read_run(run_id, true).await?
                    && current.status.is_terminal()
                {
                    return Ok(current);
                }
                return Err(StateStoreError::LeaseConflict(format!(
                    "execution run '{run_id}' changed while applying progress"
                )));
            }
            return Err(StateStoreError::Database(error.to_string()));
        }

        Ok(record)
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
        self.get_run_for_app(run_id, app_id)
            .await?
            .ok_or(StateStoreError::NotFound)?;

        for _ in 0..LEASE_UPDATE_ATTEMPTS {
            let item = self
                .read_run_item(run_id, true)
                .await?
                .ok_or(StateStoreError::NotFound)?;
            let now = chrono::Utc::now().timestamp_millis();
            if let Some(claim) = classify_lease_claim(&item, app_id, job_id, lease_token, now)? {
                return Ok(claim);
            }
            let record = item_to_run(&item)?;
            let expected_updated_at = record.updated_at;
            let updated_at = now.max(expected_updated_at.saturating_add(1));
            let expires_at = now.saturating_add(lease_duration_ms);

            let write = self
                .client
                .update_item()
                .table_name(&self.runs_table)
                .key("id", AttributeValue::S(run_id.to_string()))
                .condition_expression(CLAIM_LEASE_CONDITION)
                .update_expression("SET #bound_job_id = :job_id, #lease_token = :lease_token, #lease_expires_at = :lease_expires_at, #status = :running, #started_at = if_not_exists(#started_at, :now), #updated_at = :updated_at")
                .expression_attribute_names("#app_id", "appId")
                .expression_attribute_names("#status", "status")
                .expression_attribute_names("#updated_at", "updatedAt")
                .expression_attribute_names("#expires_at", "expiresAt")
                .expression_attribute_names("#bound_job_id", "boundJobId")
                .expression_attribute_names("#lease_token", "leaseToken")
                .expression_attribute_names("#lease_expires_at", "leaseExpiresAt")
                .expression_attribute_names("#started_at", "startedAt")
                .expression_attribute_values(":app_id", AttributeValue::S(app_id.to_string()))
                .expression_attribute_values(":pending", AttributeValue::S("PENDING".into()))
                .expression_attribute_values(":running", AttributeValue::S("RUNNING".into()))
                .expression_attribute_values(
                    ":expected_updated_at",
                    AttributeValue::N(expected_updated_at.to_string()),
                )
                .expression_attribute_values(":now", AttributeValue::N(now.to_string()))
                .expression_attribute_values(":job_id", AttributeValue::S(job_id.to_string()))
                .expression_attribute_values(
                    ":lease_token",
                    AttributeValue::S(lease_token.to_string()),
                )
                .expression_attribute_values(
                    ":lease_expires_at",
                    AttributeValue::N(expires_at.to_string()),
                )
                .expression_attribute_values(
                    ":updated_at",
                    AttributeValue::N(updated_at.to_string()),
                )
                .return_values(ReturnValue::AllNew)
                .send()
                .await;

            match write {
                Ok(result) => {
                    let run = result
                        .attributes
                        .as_ref()
                        .ok_or_else(|| {
                            StateStoreError::Database(
                                "DynamoDB lease claim returned no run attributes".to_string(),
                            )
                        })
                        .and_then(item_to_run)?;
                    return Ok(RunLeaseClaim::Acquired { run, expires_at });
                }
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|error| error.is_conditional_check_failed_exception()) =>
                {
                    continue;
                }
                Err(error) => return Err(StateStoreError::Database(error.to_string())),
            }
        }

        let item = self
            .read_run_item(run_id, true)
            .await?
            .ok_or(StateStoreError::NotFound)?;
        let now = chrono::Utc::now().timestamp_millis();
        if let Some(claim) = classify_lease_claim(&item, app_id, job_id, lease_token, now)? {
            return Ok(claim);
        }
        Err(StateStoreError::LeaseConflict(format!(
            "run '{run_id}' lease changed concurrently {LEASE_UPDATE_ATTEMPTS} times"
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
        self.get_run_for_app(run_id, app_id)
            .await?
            .ok_or(StateStoreError::NotFound)?;

        for _ in 0..LEASE_UPDATE_ATTEMPTS {
            let current = self
                .read_run_item(run_id, true)
                .await?
                .ok_or(StateStoreError::NotFound)?;
            let mut record = item_to_run(&current)?;
            if record.app_id != app_id {
                return Err(StateStoreError::NotFound);
            }
            if record.status.is_terminal() {
                return Ok(record);
            }
            let now = chrono::Utc::now().timestamp_millis();
            active_lease_record(&current, app_id, job_id, lease_token, now)?;
            let expected_updated_at = record.updated_at;
            apply_run_update(&mut record, &input, now);
            let mut item = run_to_item(&record);
            // The broker job remains permanently bound to the run, while the
            // active token is removed by replacing the item without it.
            item.insert("boundJobId".into(), AttributeValue::S(job_id.to_string()));

            let write = self
                .client
                .put_item()
                .table_name(&self.runs_table)
                .set_item(Some(item))
                .condition_expression(TERMINAL_LEASE_CONDITION)
                .expression_attribute_names("#app_id", "appId")
                .expression_attribute_names("#status", "status")
                .expression_attribute_names("#updated_at", "updatedAt")
                .expression_attribute_names("#bound_job_id", "boundJobId")
                .expression_attribute_names("#lease_token", "leaseToken")
                .expression_attribute_names("#lease_expires_at", "leaseExpiresAt")
                .expression_attribute_values(":app_id", AttributeValue::S(app_id.to_string()))
                .expression_attribute_values(":pending", AttributeValue::S("PENDING".into()))
                .expression_attribute_values(":running", AttributeValue::S("RUNNING".into()))
                .expression_attribute_values(
                    ":expected_updated_at",
                    AttributeValue::N(expected_updated_at.to_string()),
                )
                .expression_attribute_values(":now", AttributeValue::N(now.to_string()))
                .expression_attribute_values(":job_id", AttributeValue::S(job_id.to_string()))
                .expression_attribute_values(
                    ":lease_token",
                    AttributeValue::S(lease_token.to_string()),
                )
                .send()
                .await;

            match write {
                Ok(_) => return Ok(record),
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|error| error.is_conditional_check_failed_exception()) =>
                {
                    continue;
                }
                Err(error) => return Err(StateStoreError::Database(error.to_string())),
            }
        }

        if let Some(record) = self.read_run(run_id, true).await?
            && record.app_id == app_id
            && record.status.is_terminal()
        {
            return Ok(record);
        }
        Err(StateStoreError::LeaseConflict(format!(
            "run '{run_id}' terminal update changed concurrently {LEASE_UPDATE_ATTEMPTS} times"
        )))
    }

    async fn validate_run_lease(
        &self,
        run_id: &str,
        app_id: &str,
        job_id: &str,
        lease_token: &str,
    ) -> Result<(), StateStoreError> {
        self.get_run_for_app(run_id, app_id)
            .await?
            .ok_or(StateStoreError::NotFound)?;
        let item = self
            .read_run_item(run_id, true)
            .await?
            .ok_or(StateStoreError::NotFound)?;
        active_lease_record(
            &item,
            app_id,
            job_id,
            lease_token,
            chrono::Utc::now().timestamp_millis(),
        )?;
        Ok(())
    }

    async fn list_runs_for_app(
        &self,
        app_id: &str,
        limit: i32,
        cursor: Option<&str>,
    ) -> Result<Vec<ExecutionRunRecord>, StateStoreError> {
        let mut query = self
            .client
            .query()
            .table_name(&self.runs_table)
            .index_name(APP_INDEX)
            .key_condition_expression("appId = :app_id")
            .expression_attribute_values(":app_id", AttributeValue::S(app_id.to_string()))
            .scan_index_forward(false)
            .limit(limit);

        if let Some(cursor) = cursor
            && let Some(record) = self.get_run(cursor).await?
        {
            let mut key = HashMap::new();
            key.insert("id".into(), AttributeValue::S(cursor.to_string()));
            key.insert("appId".into(), AttributeValue::S(app_id.to_string()));
            key.insert(
                "createdAt".into(),
                AttributeValue::N(record.created_at.to_string()),
            );
            query = query.set_exclusive_start_key(Some(key));
        }

        let result = query
            .send()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        let mut records = Vec::new();
        if let Some(items) = result.items {
            for item in items {
                records.push(item_to_run(&item)?);
            }
        }

        Ok(records)
    }

    async fn delete_expired_runs(&self) -> Result<i64, StateStoreError> {
        // DynamoDB TTL handles expiration automatically
        Ok(0)
    }

    async fn push_events(&self, events: Vec<CreateEventInput>) -> Result<i32, StateStoreError> {
        if events.is_empty() {
            return Ok(0);
        }

        let now = chrono::Utc::now().timestamp_millis();

        // Process events - offload large payloads to S3
        let mut processed_events = Vec::new();
        for event in &events {
            let payload_json = event.payload.to_string();
            let payload_ref = if payload_json.len() > PAYLOAD_SIZE_THRESHOLD {
                // Avoid touching object storage for the ordinary HTTP retry
                // of an already-accepted canonical event. A simultaneous
                // first write can still create an unreferenced content-hash
                // object, but it cannot replace the winner's payload.
                if has_canonical_identity(event) && self.event_exists(&event.id).await? {
                    continue;
                }
                Some(
                    self.store_large_payload(&event.run_id, &event.id, &event.payload)
                        .await?,
                )
            } else {
                None
            };
            processed_events.push((event, payload_ref));
        }

        let mut canonical_items = Vec::new();
        let mut legacy_requests = Vec::new();
        for (event, payload_ref) in processed_events {
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
            let item = event_to_item(&record, payload_ref.as_deref());

            if has_canonical_identity(event) {
                canonical_items.push(item);
                continue;
            }

            legacy_requests.push(
                WriteRequest::builder()
                    .put_request(
                        aws_sdk_dynamodb::types::PutRequest::builder()
                            .set_item(Some(item))
                            .build()
                            .unwrap(),
                    )
                    .build(),
            );
            if legacy_requests.len() == 25 {
                self.batch_write_events(std::mem::take(&mut legacy_requests))
                    .await?;
            }
        }
        if !legacy_requests.is_empty() {
            self.batch_write_events(legacy_requests).await?;
        }

        // Each conditional put is first-write-wins on its own deterministic
        // id, so ordering across distinct events is irrelevant and the batch
        // can fan out instead of paying one serial round trip per event.
        stream::iter(
            canonical_items
                .into_iter()
                .map(|item| self.put_event_if_absent(item)),
        )
        .buffer_unordered(EVENT_WRITE_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;

        Ok(events.len() as i32)
    }

    async fn get_events(
        &self,
        query: EventQuery,
    ) -> Result<Vec<ExecutionEventRecord>, StateStoreError> {
        let min_seq = query.after_sequence.map(|s| s + 1).unwrap_or(0);

        let mut q = self
            .client
            .query()
            .table_name(&self.events_table)
            .index_name(RUN_INDEX)
            .key_condition_expression("runId = :run_id AND #seq >= :min_seq")
            .expression_attribute_names("#seq", "sequence")
            .expression_attribute_values(":run_id", AttributeValue::S(query.run_id.clone()))
            .expression_attribute_values(":min_seq", AttributeValue::N(min_seq.to_string()))
            .scan_index_forward(true);

        if query.only_undelivered {
            q = q
                .filter_expression("delivered = :delivered")
                .expression_attribute_values(":delivered", AttributeValue::Bool(false));
        }

        if let Some(limit) = query.limit {
            q = q.limit(limit);
        }

        let result = q
            .send()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        let mut records = Vec::new();
        if let Some(items) = result.items {
            for item in items {
                let (mut record, payload_ref) = item_to_event(&item)?;

                // Fetch large payload from S3 if needed
                if let Some(s3_ref) = payload_ref {
                    record.payload = self.fetch_large_payload(&s3_ref).await?;
                }

                records.push(record);
            }
        }

        Ok(records)
    }

    async fn get_max_sequence(&self, run_id: &str) -> Result<i32, StateStoreError> {
        let result = self
            .client
            .query()
            .table_name(&self.events_table)
            .index_name(RUN_INDEX)
            .key_condition_expression("runId = :run_id")
            .expression_attribute_values(":run_id", AttributeValue::S(run_id.to_string()))
            .scan_index_forward(false)
            .limit(1)
            .send()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        if let Some(items) = result.items
            && let Some(item) = items.first()
        {
            let (event, _) = item_to_event(item)?;
            return Ok(event.sequence);
        }

        Ok(0)
    }

    async fn mark_events_delivered(
        &self,
        run_id: &str,
        event_ids: &[String],
    ) -> Result<(), StateStoreError> {
        for id in event_ids {
            // The run condition also stops the update-as-upsert from minting
            // a phantom item for an unknown or foreign id; that condition
            // failing (expired or mismatched event) is a safe no-op.
            let write = self
                .client
                .update_item()
                .table_name(&self.events_table)
                .key("id", AttributeValue::S(id.clone()))
                .update_expression("SET delivered = :delivered")
                .condition_expression("runId = :run_id")
                .expression_attribute_values(":delivered", AttributeValue::Bool(true))
                .expression_attribute_values(":run_id", AttributeValue::S(run_id.to_string()))
                .send()
                .await;
            match write {
                Ok(_) => {}
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|error| error.is_conditional_check_failed_exception()) => {}
                Err(error) => return Err(StateStoreError::Database(error.to_string())),
            }
        }

        Ok(())
    }

    async fn delete_expired_events(&self) -> Result<i64, StateStoreError> {
        // DynamoDB TTL handles expiration automatically
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_queue_run() -> ExecutionRunRecord {
        ExecutionRunRecord {
            id: "run-1".into(),
            board_id: "board-1".into(),
            version: Some("1_2_3".into()),
            event_id: Some("event-1".into()),
            status: RunStatus::Pending,
            mode: RunMode::Queue,
            run_variant: RunVariant::Primary,
            variant_name: None,
            shadow_of_run_id: None,
            regression_run_id: None,
            input_payload_len: 17,
            output_payload_len: 0,
            error_message: None,
            progress: 0,
            current_step: None,
            started_at: None,
            completed_at: None,
            expires_at: Some(1_900_000_000_000),
            user_id: Some("user-1".into()),
            technical_user_id: Some("technical-user-1".into()),
            app_id: "app-1".into(),
            created_at: 1_800_000_000_000,
            updated_at: 1_800_000_000_001,
        }
    }

    #[test]
    fn stateless_lambda_queue_run_survives_dynamodb_import_encoding() {
        let expected = canonical_queue_run();
        let actual = item_to_run(&run_to_item(&expected)).expect("queue run should decode");

        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.app_id, expected.app_id);
        assert_eq!(actual.board_id, expected.board_id);
        assert_eq!(actual.version, expected.version);
        assert_eq!(actual.event_id, expected.event_id);
        assert_eq!(actual.mode, RunMode::Queue);
        assert_eq!(actual.status, RunStatus::Pending);
        assert_eq!(actual.expires_at, expected.expires_at);
        assert_eq!(actual.user_id, expected.user_id);
        assert_eq!(actual.technical_user_id, expected.technical_user_id);
        assert_eq!(actual.run_variant, RunVariant::Primary);
    }

    #[test]
    fn pre_variant_items_without_the_attribute_decode_as_primary() {
        let mut item = run_to_item(&canonical_queue_run());
        item.remove("runVariant");
        let decoded = item_to_run(&item).expect("legacy item should decode");
        assert_eq!(decoded.run_variant, RunVariant::Primary);
        assert_eq!(decoded.variant_name, None);
    }

    #[test]
    fn stateless_lambda_constructor_retains_source_store_for_cold_import() {
        let source_db = Arc::new(DatabaseConnection::default());
        assert!(postgres_source(Some(source_db)).is_some());
        assert!(postgres_source(None).is_none());
    }

    #[test]
    fn stateless_lambda_batch_write_retries_are_bounded() {
        assert_eq!(EVENT_WRITE_MAX_ATTEMPTS, 6);
        assert_eq!(event_write_retry_delay(0), Duration::from_millis(25));
        assert_eq!(event_write_retry_delay(1), Duration::from_millis(50));
        assert_eq!(event_write_retry_delay(5), Duration::from_millis(800));
        assert_eq!(
            event_write_retry_delay(usize::MAX),
            Duration::from_millis(800)
        );
    }

    #[test]
    fn stateless_lambda_canonical_event_identity_selects_atomic_create() {
        let mut event = CreateEventInput {
            id: canonical_execution_event_id("run-1", 7),
            run_id: "run-1".into(),
            sequence: 7,
            event_type: "chunk".into(),
            payload: serde_json::Value::Null,
            expires_at: 1_900_000_000_000,
        };
        assert!(has_canonical_identity(&event));
        event.id = "legacy-random-id".into();
        assert!(!has_canonical_identity(&event));
    }

    fn lease_item(
        status: RunStatus,
        bound_job_id: Option<&str>,
        token: Option<&str>,
        expires_at: Option<i64>,
    ) -> HashMap<String, AttributeValue> {
        let mut run = canonical_queue_run();
        run.status = status;
        let mut item = run_to_item(&run);
        if let Some(bound_job_id) = bound_job_id {
            item.insert("boundJobId".into(), AttributeValue::S(bound_job_id.into()));
        }
        if let Some(token) = token {
            item.insert("leaseToken".into(), AttributeValue::S(token.into()));
        }
        if let Some(expires_at) = expires_at {
            item.insert(
                "leaseExpiresAt".into(),
                AttributeValue::N(expires_at.to_string()),
            );
        }
        item
    }

    #[test]
    fn stateless_lambda_dynamodb_lease_claim_handles_race_renewal_and_takeover() {
        let now = 1_800_000_000_100;
        let unleased = lease_item(RunStatus::Pending, None, None, None);
        assert!(
            classify_lease_claim(&unleased, "app-1", "job-1", "token-a", now)
                .unwrap()
                .is_none()
        );

        let active = lease_item(
            RunStatus::Running,
            Some("job-1"),
            Some("token-a"),
            Some(now + 30_000),
        );
        assert!(
            classify_lease_claim(&active, "app-1", "job-1", "token-a", now)
                .unwrap()
                .is_none(),
            "the current token may renew"
        );
        match classify_lease_claim(&active, "app-1", "job-1", "token-b", now).unwrap() {
            Some(RunLeaseClaim::Busy { expires_at, .. }) => {
                assert_eq!(expires_at, now + 30_000)
            }
            other => panic!("competing token should be busy, got {other:?}"),
        }

        let expired = lease_item(
            RunStatus::Running,
            Some("job-1"),
            Some("token-a"),
            Some(now),
        );
        assert!(
            classify_lease_claim(&expired, "app-1", "job-1", "token-b", now)
                .unwrap()
                .is_none(),
            "an expired token may be replaced for the same broker job"
        );
    }

    #[test]
    fn stateless_lambda_dynamodb_lease_rejects_other_jobs_old_tokens_and_expiry() {
        let now = 1_800_000_000_100;
        let active = lease_item(
            RunStatus::Running,
            Some("job-1"),
            Some("token-a"),
            Some(now + 30_000),
        );
        assert!(classify_lease_claim(&active, "app-1", "job-2", "token-b", now).is_err());
        assert!(active_lease_record(&active, "app-1", "job-1", "token-a", now).is_ok());
        assert!(active_lease_record(&active, "app-1", "job-1", "token-b", now).is_err());
        assert!(active_lease_record(&active, "app-1", "job-1", "token-a", now + 30_000).is_err());
    }

    #[test]
    fn stateless_lambda_dynamodb_terminal_state_wins_over_redelivery() {
        let terminal = lease_item(RunStatus::Completed, Some("job-1"), None, None);
        assert!(matches!(
            classify_lease_claim(
                &terminal,
                "app-1",
                "job-1",
                "replacement-token",
                1_800_000_000_100
            )
            .unwrap(),
            Some(RunLeaseClaim::Terminal { .. })
        ));
    }

    #[test]
    fn stateless_lambda_dynamodb_lease_conditions_are_atomic_and_fail_closed() {
        for clause in [
            "#updated_at = :expected_updated_at",
            "attribute_not_exists(#bound_job_id) OR #bound_job_id = :job_id",
            "#lease_token = :lease_token OR #lease_expires_at <= :now",
        ] {
            assert!(CLAIM_LEASE_CONDITION.contains(clause));
        }
        for clause in [
            "#bound_job_id = :job_id",
            "#lease_token = :lease_token",
            "#lease_expires_at > :now",
        ] {
            assert!(TERMINAL_LEASE_CONDITION.contains(clause));
        }
        assert!(UNLEASED_UPDATE_CONDITION.contains("attribute_not_exists(#bound_job_id)"));
        assert_eq!(SOURCE_IMPORT_CONDITION, "attribute_not_exists(id)");
    }
}
