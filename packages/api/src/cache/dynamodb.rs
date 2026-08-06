//! DynamoDB cache backend with native TTL.
//!
//! One table, `AppCache`, partitioned by app so an app teardown is a single query. The
//! `expires_at` attribute holds epoch **seconds** — DynamoDB's TTL feature requires that
//! unit and silently ignores milliseconds — and must be configured as the table's TTL
//! attribute for automatic eviction.
//!
//! ## Provisioning
//!
//! The table is **not** created automatically; nothing in this workspace grants the API
//! `dynamodb:CreateTable`, and racing cold-start Lambdas are a poor place to provision
//! infrastructure. Create it once per environment:
//!
//! ```bash
//! aws dynamodb create-table \
//!   --table-name "${DYNAMODB_TABLE_PREFIX}AppCache" \
//!   --attribute-definitions \
//!       AttributeName=app_id,AttributeType=S \
//!       AttributeName=entry_key,AttributeType=S \
//!   --key-schema \
//!       AttributeName=app_id,KeyType=HASH \
//!       AttributeName=entry_key,KeyType=RANGE \
//!   --billing-mode PAY_PER_REQUEST
//!
//! aws dynamodb update-time-to-live \
//!   --table-name "${DYNAMODB_TABLE_PREFIX}AppCache" \
//!   --time-to-live-specification "Enabled=true,AttributeName=expires_at"
//! ```
//!
//! No secondary index is needed: `app_id` is the partition key, so per-app teardown is a
//! plain `Query`.
//!
//! Required IAM actions on the table ARN: `GetItem`, `PutItem`, `DeleteItem`, `Query`.
//!
//! Forgetting the TTL specification is the failure mode to watch for — entries still read
//! as expired (the stored expiry is checked on read), but nothing is ever reclaimed, so
//! the table grows without bound.

use super::types::*;
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_dynamodb::{Client, types::AttributeValue};
use chrono::Utc;
use std::collections::HashMap;

const CACHE_TABLE: &str = "AppCache";
const PARTITION_KEY: &str = "app_id";
const SORT_KEY: &str = "entry_key";
const TTL_ATTRIBUTE: &str = "expires_at";

pub struct DynamoDbCacheStore {
    client: Client,
    table: String,
}

impl std::fmt::Debug for DynamoDbCacheStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamoDbCacheStore")
            .field("table", &self.table)
            .finish()
    }
}

impl DynamoDbCacheStore {
    pub fn new(aws_config: &SdkConfig) -> Self {
        let prefix = std::env::var("DYNAMODB_TABLE_PREFIX").unwrap_or_default();
        Self {
            client: Client::new(aws_config),
            table: format!("{prefix}{CACHE_TABLE}"),
        }
    }

    pub async fn from_env() -> Result<Self, CacheStoreError> {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        Ok(Self::new(&config))
    }

    /// Sort key. The app id is already the partition key, but keeping it inside the
    /// composite costs nothing and keeps the format identical across backends.
    fn sort_key(key: &CacheKey) -> String {
        key.composite()
    }

    fn primary_key(key: &CacheKey) -> HashMap<String, AttributeValue> {
        HashMap::from([
            (
                PARTITION_KEY.to_string(),
                AttributeValue::S(key.app_id.clone()),
            ),
            (SORT_KEY.to_string(), AttributeValue::S(Self::sort_key(key))),
        ])
    }

    fn build_item(
        &self,
        entry: &SetCacheEntry,
        now_ms: i64,
    ) -> Result<HashMap<String, AttributeValue>, CacheStoreError> {
        let encoded = serde_json::to_string(&entry.value)
            .map_err(|e| CacheStoreError::Serialization(e.to_string()))?;

        let mut item = Self::primary_key(&entry.key);
        item.insert("key".to_string(), AttributeValue::S(entry.key.key.clone()));
        item.insert(
            "scope".to_string(),
            AttributeValue::S(entry.key.scope.as_str().to_string()),
        );
        item.insert(
            "user_id".to_string(),
            AttributeValue::S(entry.key.user_id.clone()),
        );
        item.insert("value".to_string(), AttributeValue::S(encoded));
        item.insert(
            "updated_at".to_string(),
            AttributeValue::N(now_ms.to_string()),
        );

        if let Some(expires_at) = entry.expires_at {
            // Seconds, rounded up: a sub-second TTL must not floor to "already expired".
            // Expiries are epoch milliseconds and therefore positive, so the manual
            // round-up is exact.
            let seconds = (expires_at + 999) / 1_000;
            item.insert(
                TTL_ATTRIBUTE.to_string(),
                AttributeValue::N(seconds.to_string()),
            );
        }

        Ok(item)
    }

    fn item_to_entry(item: &HashMap<String, AttributeValue>) -> Result<CacheEntry, CacheStoreError> {
        let raw_value = item
            .get("value")
            .and_then(|v| v.as_s().ok())
            .ok_or_else(|| CacheStoreError::Serialization("Item is missing `value`".to_string()))?;

        let value = serde_json::from_str(raw_value)
            .map_err(|e| CacheStoreError::Serialization(e.to_string()))?;

        let key = item
            .get("key")
            .and_then(|v| v.as_s().ok())
            .cloned()
            .unwrap_or_default();

        let expires_at = item
            .get(TTL_ATTRIBUTE)
            .and_then(|v| v.as_n().ok())
            .and_then(|n| n.parse::<i64>().ok())
            .map(|seconds| seconds * 1_000);

        let updated_at = item
            .get("updated_at")
            .and_then(|v| v.as_n().ok())
            .and_then(|n| n.parse::<i64>().ok())
            .unwrap_or_default();

        Ok(CacheEntry {
            key,
            value,
            expires_at,
            updated_at,
        })
    }
}

#[async_trait]
impl CacheStore for DynamoDbCacheStore {
    fn backend_name(&self) -> &'static str {
        "dynamodb"
    }

    async fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, CacheStoreError> {
        let output = self
            .client
            .get_item()
            .table_name(&self.table)
            .set_key(Some(Self::primary_key(key)))
            .send()
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        let Some(item) = output.item else {
            return Ok(None);
        };

        let entry = Self::item_to_entry(&item)?;

        // DynamoDB TTL deletion can lag by up to 48 hours, so the stored expiry is the
        // authority for reads.
        if entry.is_expired_at(Utc::now().timestamp_millis()) {
            return Ok(None);
        }

        Ok(Some(entry))
    }

    async fn exists(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        // Project only the TTL attribute: enough to judge liveness, and the value stays
        // in DynamoDB instead of being billed and transferred.
        let output = self
            .client
            .get_item()
            .table_name(&self.table)
            .set_key(Some(Self::primary_key(key)))
            .expression_attribute_names("#ttl", TTL_ATTRIBUTE)
            .projection_expression("#ttl")
            .send()
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        let Some(item) = output.item else {
            return Ok(false);
        };

        let expires_at_ms = item
            .get(TTL_ATTRIBUTE)
            .and_then(|v| v.as_n().ok())
            .and_then(|n| n.parse::<i64>().ok())
            .map(|seconds| seconds * 1_000);

        Ok(!expires_at_ms.is_some_and(|expires| expires <= Utc::now().timestamp_millis()))
    }

    async fn set(&self, entry: SetCacheEntry) -> Result<CacheEntry, CacheStoreError> {
        let now_ms = Utc::now().timestamp_millis();
        let item = self.build_item(&entry, now_ms)?;

        self.client
            .put_item()
            .table_name(&self.table)
            .set_item(Some(item))
            .send()
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(CacheEntry {
            key: entry.key.key,
            value: entry.value,
            expires_at: entry.expires_at,
            updated_at: now_ms,
        })
    }

    async fn try_insert(
        &self,
        entry: SetCacheEntry,
    ) -> Result<Option<CacheEntry>, CacheStoreError> {
        let now_ms = Utc::now().timestamp_millis();
        let item = self.build_item(&entry, now_ms)?;
        let now_seconds = now_ms / 1_000;

        // DynamoDB's TTL reaper can lag by hours, so `attribute_not_exists` alone would
        // permanently refuse to refill a key whose value expired. The second clause lets
        // a lapsed item be overwritten. An item with no TTL attribute at all fails the
        // comparison, which is correct — it never expires.
        let result = self
            .client
            .put_item()
            .table_name(&self.table)
            .set_item(Some(item))
            .condition_expression("attribute_not_exists(#pk) OR #ttl <= :now")
            .expression_attribute_names("#pk", PARTITION_KEY)
            .expression_attribute_names("#ttl", TTL_ATTRIBUTE)
            .expression_attribute_values(":now", AttributeValue::N(now_seconds.to_string()))
            .send()
            .await;

        match result {
            Ok(_) => Ok(Some(CacheEntry {
                key: entry.key.key,
                value: entry.value,
                expires_at: entry.expires_at,
                updated_at: now_ms,
            })),
            Err(error) => {
                // `as_service_error` rather than `into_service_error`: the latter panics
                // on transport failures, which are exactly the errors that must surface
                // as errors instead of taking down the process.
                if error
                    .as_service_error()
                    .is_some_and(|e| e.is_conditional_check_failed_exception())
                {
                    return Ok(None);
                }
                Err(CacheStoreError::Database(error.to_string()))
            }
        }
    }

    async fn delete(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        let output = self
            .client
            .delete_item()
            .table_name(&self.table)
            .set_key(Some(Self::primary_key(key)))
            .return_values(aws_sdk_dynamodb::types::ReturnValue::AllOld)
            .send()
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(output.attributes.is_some_and(|attrs| !attrs.is_empty()))
    }

    async fn delete_app(&self, app_id: &str) -> Result<i64, CacheStoreError> {
        let mut deleted = 0i64;
        let mut last_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let output = self
                .client
                .query()
                .table_name(&self.table)
                .key_condition_expression("#pk = :app_id")
                .expression_attribute_names("#pk", PARTITION_KEY)
                .expression_attribute_values(":app_id", AttributeValue::S(app_id.to_string()))
                .projection_expression(format!("{PARTITION_KEY}, {SORT_KEY}"))
                .set_exclusive_start_key(last_key)
                .send()
                .await
                .map_err(|e| CacheStoreError::Database(e.to_string()))?;

            for item in output.items() {
                let (Some(pk), Some(sk)) = (item.get(PARTITION_KEY), item.get(SORT_KEY)) else {
                    continue;
                };
                self.client
                    .delete_item()
                    .table_name(&self.table)
                    .key(PARTITION_KEY, pk.clone())
                    .key(SORT_KEY, sk.clone())
                    .send()
                    .await
                    .map_err(|e| CacheStoreError::Database(e.to_string()))?;
                deleted += 1;
            }

            last_key = output.last_evaluated_key;
            if last_key.is_none() {
                break;
            }
        }

        Ok(deleted)
    }

    async fn delete_expired(&self) -> Result<i64, CacheStoreError> {
        // DynamoDB evicts via the TTL attribute. A manual scan would cost far more than
        // it reclaims, and reads already filter lapsed entries.
        Ok(0)
    }
}
