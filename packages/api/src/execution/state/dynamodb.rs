//! DynamoDB state store implementation with native TTL support
//!
//! Uses DynamoDB Time To Live (TTL) for automatic expiration.
//! Large payloads (>100KB) are stored via FlowLikeStore under "polling/{run_id}/{event_id}"
//! Tables: ExecutionRuns, ExecutionEvents with GSIs for app/run lookups.

use super::types::*;
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_dynamodb::{
    Client,
    types::{AttributeValue, KeyType, ScalarAttributeType},
};
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{ObjectStore, path::Path},
};
use std::{collections::HashMap, sync::Arc};

const RUNS_TABLE: &str = "ExecutionRuns";
const EVENTS_TABLE: &str = "ExecutionEvents";
const APP_INDEX: &str = "AppIdIndex";
const RUN_INDEX: &str = "RunIdIndex";
const DEFAULT_TTL_SECS: i64 = 86400;
const PAYLOAD_SIZE_THRESHOLD: usize = 100 * 1024; // 100KB - offload to object store above this
const POLLING_PREFIX: &str = "polling";

pub struct DynamoDbStateStore {
    client: Client,
    content_store: Arc<FlowLikeStore>,
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
        let prefix = std::env::var("DYNAMODB_TABLE_PREFIX").unwrap_or_default();
        Self {
            client: Client::new(aws_config),
            content_store,
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

    async fn store_large_payload(
        &self,
        run_id: &str,
        event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<String, StateStoreError> {
        let path = Path::from(format!("{}/{}/{}.json", POLLING_PREFIX, run_id, event_id));
        let body = serde_json::to_vec(payload)
            .map_err(|e| StateStoreError::Serialization(e.to_string()))?;

        self.content_store
            .as_generic()
            .put(&path, body.into())
            .await
            .map_err(|e| StateStoreError::Database(format!("Object store put failed: {}", e)))?;

        Ok(format!("store://{}", path))
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
        _ => {
            return Err(StateStoreError::Serialization(format!(
                "Invalid mode: {mode_str}"
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
        let result = self
            .client
            .get_item()
            .table_name(&self.runs_table)
            .key("id", AttributeValue::S(run_id.to_string()))
            .send()
            .await
            .map_err(|e| StateStoreError::Database(e.to_string()))?;

        match result.item {
            Some(item) => Ok(Some(item_to_run(&item)?)),
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
                Some(
                    self.store_large_payload(&event.run_id, &event.id, &event.payload)
                        .await?,
                )
            } else {
                None
            };
            processed_events.push((event, payload_ref));
        }

        // Batch write (max 25 items per batch)
        for chunk in processed_events.chunks(25) {
            let mut requests = Vec::new();
            for (event, payload_ref) in chunk {
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
                requests.push(
                    aws_sdk_dynamodb::types::WriteRequest::builder()
                        .put_request(
                            aws_sdk_dynamodb::types::PutRequest::builder()
                                .set_item(Some(item))
                                .build()
                                .unwrap(),
                        )
                        .build(),
                );
            }

            self.client
                .batch_write_item()
                .request_items(&self.events_table, requests)
                .send()
                .await
                .map_err(|e| StateStoreError::Database(e.to_string()))?;
        }

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

    async fn mark_events_delivered(&self, event_ids: &[String]) -> Result<(), StateStoreError> {
        for id in event_ids {
            self.client
                .update_item()
                .table_name(&self.events_table)
                .key("id", AttributeValue::S(id.clone()))
                .update_expression("SET delivered = :delivered")
                .expression_attribute_values(":delivered", AttributeValue::Bool(true))
                .send()
                .await
                .map_err(|e| StateStoreError::Database(e.to_string()))?;
        }

        Ok(())
    }

    async fn delete_expired_events(&self) -> Result<i64, StateStoreError> {
        // DynamoDB TTL handles expiration automatically
        Ok(0)
    }
}
