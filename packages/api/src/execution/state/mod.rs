//! Execution state store abstraction
//!
//! This module provides different backends for storing execution state and events:
//! - **PostgreSQL**: Via Prisma/SeaORM - reliable, supports complex queries
//! - **Redis**: Fast, with native TTL support - good for high-throughput
//! - **DynamoDB**: Serverless, with TTL + FlowLikeStore for large payloads - good for AWS
//! - **Cosmos DB**: Serverless, native TTL + Blob offload - good for Azure
//! - **Firestore**: Serverless, native TTL + Cloud Storage offload - good for GCP
//! - **Object Storage**: S3/R2/GCS - for large payloads and archival
//!
//! ## Backend Selection
//!
//! | Backend | Latency | Scalability | TTL | Best For |
//! |---------|---------|-------------|-----|----------|
//! | PostgreSQL | Medium | Vertical | Manual | Full-featured, complex queries |
//! | Redis | Low | Horizontal | Native | High-throughput, real-time |
//! | DynamoDB | Low | Horizontal | Native | Serverless, AWS-native |
//! | Cosmos DB | Low | Horizontal | Native | Serverless, Azure-native |
//! | Firestore | Low | Horizontal | Native | Serverless, GCP-native |
//! | Object Storage | High | Infinite | Lifecycle | Large payloads, archival |
//!
//! ## Recommended Configuration
//!
//! | Deployment | Backend | Reason |
//! |------------|---------|--------|
//! | AWS Lambda/ECS | `dynamodb` | Native TTL, serverless, auto-scaling, FlowLikeStore for large payloads |
//! | Azure Container Apps | `cosmos` | Native TTL, serverless, Entra-only auth, Blob offload for large payloads |
//! | GCP Cloud Run | `firestore` | Native TTL, serverless, metadata-server auth, GCS offload for large payloads |
//! | Kubernetes | `redis` | Fast, native TTL, already deployed in cluster |
//! | Docker Compose | `redis` | Simple setup, native TTL |
//!
//! ## Configuration
//!
//! ```bash
//! # Select backend
//! EXECUTION_STATE_BACKEND=dynamodb  # postgres, redis, dynamodb, cosmos, firestore, s3
//!
//! # PostgreSQL (default, requires manual TTL cleanup)
//! DATABASE_URL=postgres://...
//!
//! # Redis (Kubernetes/Docker)
//! REDIS_URL=redis://...
//! EXECUTION_STATE_TTL_SECONDS=86400  # 24 hours
//!
//! # DynamoDB (AWS - recommended, reuses cdn_bucket from AppState)
//! DYNAMODB_TABLE_PREFIX=flowlike-  # optional
//! # Reuses cdn_bucket (FlowLikeStore) from AppState for large payloads
//! # Fallback: CDN_BUCKET_NAME env var when AppState not available
//!
//! # Azure Cosmos DB for NoSQL (Entra ID only; no account keys)
//! COSMOS_ENDPOINT=https://<account>.documents.azure.com
//! COSMOS_DATABASE=flowlike
//! COSMOS_RUNS_CONTAINER=execution-runs
//! COSMOS_EVENTS_CONTAINER=execution-events
//! COSMOS_AUTH_MODE=managed_identity
//!
//! # Google Cloud Firestore, Native mode (metadata-server tokens only; no key files)
//! GCP_PROJECT_ID=<project>
//! FIRESTORE_DATABASE=(default)
//! FIRESTORE_RUNS_COLLECTION=execution-runs
//! FIRESTORE_EVENTS_COLLECTION=execution-events
//! FIRESTORE_COLLECTION_PREFIX=          # optional, empty unless a database is shared
//!
//! # Object Storage (for large payloads)
//! EXECUTION_PAYLOAD_BUCKET=flow-like-execution-payloads
//! ```
//!
//! ## Large Payload Handling (DynamoDB)
//!
//! DynamoDB has a 400KB item limit. Payloads larger than 100KB are automatically
//! stored via FlowLikeStore under `polling/{run_id}/{event_id}.json` and referenced in DynamoDB.
//! Uses the cdn_bucket from AppState when available, avoiding duplicate client construction.
//!
//! Cosmos DB (2 MB documents) and Firestore (1 MiB documents) offload at the same 100KB
//! threshold under the same `polling/` prefix, so the boundary between an inline payload and
//! a stored one does not move when a deployment changes cloud.

mod postgres;
mod types;

#[cfg(feature = "redis")]
mod redis;

#[cfg(feature = "dynamodb")]
mod dynamodb;

#[cfg(feature = "cosmos")]
mod cosmos;

#[cfg(feature = "firestore")]
mod firestore;

#[cfg(feature = "s3")]
mod object_storage;

pub use postgres::PostgresStateStore;
pub use types::*;

#[cfg(feature = "redis")]
pub use redis::RedisStateStore;

#[cfg(feature = "dynamodb")]
pub use dynamodb::DynamoDbStateStore;

#[cfg(feature = "cosmos")]
pub use cosmos::CosmosStateStore;

#[cfg(feature = "firestore")]
pub use firestore::FirestoreStateStore;

#[cfg(feature = "s3")]
pub use object_storage::ObjectStorageStateStore;

use std::sync::Arc;

#[cfg(feature = "aws")]
use aws_config::SdkConfig;

#[cfg(any(
    feature = "dynamodb",
    feature = "cosmos",
    feature = "firestore",
    feature = "s3"
))]
use flow_like_storage::files::store::FlowLikeStore;

/// Backend type for execution state storage
#[derive(Clone, Debug, Default)]
pub enum StateBackend {
    #[default]
    Postgres,
    #[cfg(feature = "redis")]
    Redis,
    #[cfg(feature = "dynamodb")]
    DynamoDB,
    #[cfg(feature = "cosmos")]
    Cosmos,
    #[cfg(feature = "firestore")]
    Firestore,
    #[cfg(feature = "s3")]
    ObjectStorage,
}

impl StateBackend {
    pub fn from_env() -> Self {
        match std::env::var("EXECUTION_STATE_BACKEND")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            #[cfg(feature = "redis")]
            "redis" => Self::Redis,
            #[cfg(feature = "dynamodb")]
            "dynamodb" | "dynamo" => Self::DynamoDB,
            #[cfg(feature = "cosmos")]
            "cosmos" | "cosmosdb" => Self::Cosmos,
            #[cfg(feature = "firestore")]
            "firestore" | "gcp" => Self::Firestore,
            #[cfg(feature = "s3")]
            "s3" | "object_storage" | "objectstorage" => Self::ObjectStorage,
            _ => Self::Postgres,
        }
    }
}

/// Configuration for creating state stores
#[derive(Default)]
pub struct StateStoreConfig {
    pub db: Option<Arc<sea_orm::DatabaseConnection>>,
    /// The engine behind `db`; missing means the default dialect, which only
    /// changes how a lost commit race is logged.
    pub dialect: Option<crate::db::DbDialect>,
    #[cfg(feature = "aws")]
    pub aws_config: Option<Arc<SdkConfig>>,
    #[cfg(any(feature = "dynamodb", feature = "cosmos", feature = "firestore"))]
    pub content_store: Option<Arc<FlowLikeStore>>,
    #[cfg(feature = "s3")]
    pub meta_store: Option<Arc<FlowLikeStore>>,
}

impl StateStoreConfig {
    pub fn with_db(mut self, db: Arc<sea_orm::DatabaseConnection>) -> Self {
        self.db = Some(db);
        self
    }

    pub fn with_dialect(mut self, dialect: crate::db::DbDialect) -> Self {
        self.dialect = Some(dialect);
        self
    }

    #[cfg(feature = "aws")]
    pub fn with_aws_config(mut self, config: Arc<SdkConfig>) -> Self {
        self.aws_config = Some(config);
        self
    }

    #[cfg(any(feature = "dynamodb", feature = "cosmos", feature = "firestore"))]
    pub fn with_content_store(mut self, store: Arc<FlowLikeStore>) -> Self {
        self.content_store = Some(store);
        self
    }

    #[cfg(feature = "s3")]
    pub fn with_meta_store(mut self, store: Arc<FlowLikeStore>) -> Self {
        self.meta_store = Some(store);
        self
    }
}

/// Create a state store based on environment configuration
pub async fn create_state_store(
    config: StateStoreConfig,
) -> Result<Arc<dyn ExecutionStateStore>, types::StateStoreError> {
    let backend = StateBackend::from_env();

    match backend {
        StateBackend::Postgres => {
            let db = config.db.ok_or_else(|| {
                types::StateStoreError::Configuration(
                    "Database connection required for Postgres backend".into(),
                )
            })?;
            Ok(Arc::new(PostgresStateStore::with_dialect(
                db,
                config.dialect.unwrap_or_default(),
            )))
        }

        #[cfg(feature = "redis")]
        StateBackend::Redis => {
            let store = RedisStateStore::from_env_with_source(config.db).await?;
            Ok(Arc::new(store))
        }

        #[cfg(feature = "dynamodb")]
        StateBackend::DynamoDB => {
            // Prefer using provided AWS config and content store from AppState
            match (config.aws_config, config.content_store) {
                (Some(aws_cfg), Some(store)) => Ok(Arc::new(DynamoDbStateStore::new_with_source(
                    &aws_cfg, store, config.db,
                ))),
                _ => {
                    // Fallback to environment configuration
                    let store = DynamoDbStateStore::from_env().await?;
                    Ok(Arc::new(store))
                }
            }
        }

        #[cfg(feature = "cosmos")]
        StateBackend::Cosmos => Ok(Arc::new(CosmosStateStore::from_env(
            config.content_store,
            config.db,
        )?)),

        // The content store is optional here for the same reason it is on Cosmos: only an
        // event body over the offload threshold needs it, and a deployment that never
        // produces one should not fail to start. The database is the source of the
        // canonical run row an async dispatch already wrote, which Firestore imports once.
        #[cfg(feature = "firestore")]
        StateBackend::Firestore => Ok(Arc::new(FirestoreStateStore::from_env(
            config.content_store,
            config.db,
        )?)),

        #[cfg(feature = "s3")]
        StateBackend::ObjectStorage => match config.meta_store {
            Some(store) => Ok(Arc::new(ObjectStorageStateStore::new_with_source(
                store, config.db,
            ))),
            None => {
                let store = ObjectStorageStateStore::from_env().await?;
                Ok(Arc::new(store))
            }
        },
    }
}
