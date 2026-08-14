#[cfg(feature = "aws")]
use aws_config::SdkConfig;
use axum::body::Body;
use flow_like::app::App;
use flow_like::flow::board::Board;
use flow_like::flow::node::NodeLogic;
use flow_like::flow_like_model_provider::provider::{ModelProviderConfiguration, OpenAIConfig};
use flow_like::flow_like_storage::Path;
use flow_like::flow_like_storage::files::store::FlowLikeStore;
use flow_like::hub::{Environment, Hub};
use flow_like::state::{FlowLikeState, FlowNodeRegistryInner};
use flow_like_secrets::{
    EnvProviderConfig, ExposeSecret, ProviderConfig, SecretRef, SecretStore, SecretStoreConfig,
};
use flow_like_types::bail;
use flow_like_types::{Result, Value};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode,
    jwk::{
        AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse,
    },
};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection,
    DatabaseTransaction, IsolationLevel, Statement, TransactionTrait,
};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use crate::compilation::{CompilationDispatchConfig, CompilationDispatcher};
use crate::credentials::{CredentialsAccess, RuntimeCredentials};
use crate::entity::role;
use crate::execution::{DispatchConfig, Dispatcher};
use crate::mail::{DynMailClient, create_mail_client};
use crate::permission::wasm_package_permission::WasmPackagePermission;
use crate::routes::registry::ServerRegistry;

pub type AppState = Arc<State>;

/// Stable ownership key for retained FlowPilot drafts and durable pending/applied review records.
///
/// Board ids are normally globally unique, but the review token is an authority boundary. Include
/// the authenticated principal and app explicitly so a same-id board in another app (or another
/// user's session) can never resolve the retained commands.
pub(crate) fn flow_ir_draft_store_key(sub: &str, app_id: &str, board_id: &str) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        sub.trim(),
        app_id.trim(),
        board_id.trim()
    )
}

/// Process-local serialization key for every canonical writer of one board.
///
/// Unlike the retained-draft authority key, this deliberately excludes the user: two authorized
/// collaborators still mutate the same canonical app board and must take the same mutex.
pub(crate) fn board_mutation_lock_key(app_id: &str, board_id: &str) -> String {
    format!("{}\u{1f}{}", app_id.trim(), board_id.trim())
}

const ENSURE_MUTATION_LOCK_SQL: &str =
    r#"INSERT INTO "MutationLock" ("id") VALUES ($1) ON CONFLICT ("id") DO NOTHING"#;
const ACQUIRE_MUTATION_LOCK_SQL: &str =
    r#"UPDATE "MutationLock" SET "updatedAt" = CURRENT_TIMESTAMP WHERE "id" = $1"#;

fn scoped_mutation_lock_id(domain: &[u8], parts: &[&str]) -> i64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.mutation-lock/v1\0");
    hasher.update(&(domain.len() as u64).to_be_bytes());
    hasher.update(domain);
    for part in parts {
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    i64::from_be_bytes(
        digest.as_bytes()[..8]
            .try_into()
            .expect("BLAKE3 digests are at least eight bytes"),
    )
}

/// Stable database-lock id for one canonical app board.
pub(crate) fn board_mutation_lock_id(app_id: &str, board_id: &str) -> i64 {
    scoped_mutation_lock_id(b"board", &[app_id.trim(), board_id.trim()])
}

/// Stable database-lock id for one learner's challenge scores.
///
/// The aggregate leaderboard total is per learner, so different challenges for the same learner
/// must share a lane as well as duplicate submissions for one challenge.
pub(crate) fn course_attempt_lock_id(user_id: &str) -> i64 {
    scoped_mutation_lock_id(b"course-attempt-user", &[user_id])
}

async fn ensure_mutation_lock<C: ConnectionTrait>(
    connection: &C,
    lock_id: i64,
) -> std::result::Result<(), sea_orm::DbErr> {
    connection
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            ENSURE_MUTATION_LOCK_SQL,
            [lock_id.into()],
        ))
        .await?;
    Ok(())
}

async fn acquire_mutation_lock<C: ConnectionTrait>(
    connection: &C,
    lock_id: i64,
) -> std::result::Result<(), sea_orm::DbErr> {
    let result = connection
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            ACQUIRE_MUTATION_LOCK_SQL,
            [lock_id.into()],
        ))
        .await?;
    if result.rows_affected() != 1 {
        return Err(sea_orm::DbErr::RecordNotFound(format!(
            "mutation lock row {lock_id} disappeared before acquisition"
        )));
    }
    Ok(())
}

/// Holds both serialization layers for a canonical board mutation.
///
/// Dropping the transaction releases the database write intent. Call [`Self::release`] when a
/// normal path wants to commit work performed through [`Self::connection`]; error and early-return
/// paths can safely rely on drop/rollback.
pub(crate) struct BoardMutationGuard {
    _locals: Vec<flow_like_types::tokio::sync::OwnedMutexGuard<()>>,
    transaction: Option<DatabaseTransaction>,
}

impl BoardMutationGuard {
    pub(crate) fn connection(&self) -> &DatabaseTransaction {
        self.transaction
            .as_ref()
            .expect("mutation guard connection is unavailable after release")
    }

    /// Add another canonical board to this guard without opening a second transaction.
    ///
    /// Page mutations first lock the globally unique page id, then add its owning board. Keeping
    /// both database lock rows on one transaction prevents concurrent cross-board page-id claims
    /// without consuming two pooled database connections per request.
    pub(crate) async fn acquire_additional_board(
        &mut self,
        state: &State,
        app_id: &str,
        board_id: &str,
    ) -> std::result::Result<(), sea_orm::DbErr> {
        let local = state
            .board_mutation_lock(app_id, board_id)
            .lock_owned()
            .await;
        let lock_id = board_mutation_lock_id(app_id, board_id);
        ensure_mutation_lock(self.connection(), lock_id).await?;
        acquire_mutation_lock(self.connection(), lock_id).await?;
        self._locals.push(local);
        Ok(())
    }

    pub(crate) async fn release(mut self) -> std::result::Result<(), sea_orm::DbErr> {
        if let Some(transaction) = self.transaction.take() {
            transaction.commit().await?;
        }
        Ok(())
    }
}

const CONFIG: &str = include_str!("../../../flow-like.config.json");
const JWKS: &str = include_str!(concat!(env!("OUT_DIR"), "/jwks.json"));
const JWKS_REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(30);
const JWKS_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const JWKS_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const JWKS_MAX_KEYS: usize = 64;
/// Clock-skew tolerance applied to `exp`/`nbf`. Matches the historical
/// `jsonwebtoken` default so IdPs and clients with slightly offset clocks keep
/// working; override with `authentication.openid.leeway_seconds`.
const DEFAULT_OPENID_LEEWAY_SECONDS: u64 = 60;

/// Cached auth result for JWT/PAT/API key
#[derive(Clone, Debug)]
pub enum CachedAuth {
    /// OpenID user with sub and the token's own expiration. Cache hits must
    /// never extend an access token beyond this timestamp.
    OpenID { sub: String, exp: i64 },
    /// PAT user with sub
    PAT { sub: String },
    /// API key with key_id, app_id, and the creator user that owns tier/billing.
    ApiKey {
        key_id: String,
        app_id: String,
        creator_user_id: Option<String>,
    },
    /// Executor JWT with sub, app_id, run_id, and optional originating technical user.
    Executor {
        sub: String,
        app_id: String,
        run_id: String,
        technical_user_id: Option<String>,
        app_chain: Option<Vec<String>>,
        correlation: Option<crate::correlation::CorrelationContext>,
    },
    /// App-connection JWT: one app calling another app it is connected to.
    /// `exp` is re-checked on cache hits so short-lived tokens cannot outlive
    /// their expiry through the auth cache.
    AppConnection {
        sub: Option<String>,
        origin_app_id: String,
        target_app_id: String,
        app_chain: Vec<String>,
        technical_user_id: Option<String>,
        run_id: Option<String>,
        correlation: Option<crate::correlation::CorrelationContext>,
        exp: i64,
    },
    /// Invalid/expired token
    Invalid,
}

#[derive(Debug, Default)]
struct JwksRefreshState {
    last_attempt: Option<Instant>,
}

#[derive(Debug)]
struct OpenIdValidationSettings {
    issuer: String,
    /// Every app client that may mint tokens for this deployment. Contains the
    /// configured `client_id` plus any `additional_client_ids`, so a user pool
    /// with more than one app client keeps working.
    client_ids: BTreeSet<String>,
    audience: String,
    jwks_url: String,
    tenant_id: Option<uuid::Uuid>,
    leeway: u64,
}

/// Additive OpenID validation settings read from the embedded deployment
/// config. Every field defaults to the historical behaviour, so configs that
/// do not declare them validate exactly as before.
#[derive(Debug, serde::Deserialize)]
struct OpenIdValidationOverrides {
    #[serde(default = "default_openid_leeway_seconds")]
    leeway_seconds: u64,
    #[serde(default)]
    additional_client_ids: Vec<String>,
}

impl Default for OpenIdValidationOverrides {
    fn default() -> Self {
        Self {
            leeway_seconds: DEFAULT_OPENID_LEEWAY_SECONDS,
            additional_client_ids: Vec::new(),
        }
    }
}

fn default_openid_leeway_seconds() -> u64 {
    DEFAULT_OPENID_LEEWAY_SECONDS
}

#[derive(Debug)]
pub(crate) struct ValidatedOpenIdToken {
    pub(crate) claims: HashMap<String, Value>,
    pub(crate) expires_at: i64,
}

pub struct State {
    pub platform_config: Hub,
    pub db: DatabaseConnection,
    jwks: flow_like_types::tokio::sync::RwLock<JwkSet>,
    jwks_refresh: flow_like_types::tokio::sync::Mutex<JwksRefreshState>,
    pub client: Client<HttpConnector, Body>,
    pub stripe_client: Option<stripe::Client>,
    pub mail_client: Option<DynMailClient>,
    #[cfg(feature = "aws")]
    pub aws_client: Arc<SdkConfig>,
    pub catalog: Arc<Vec<Arc<dyn NodeLogic>>>,
    pub registry: Arc<FlowNodeRegistryInner>,
    pub provider: Arc<ModelProviderConfiguration>,
    pub dispatcher: Arc<Dispatcher>,
    pub compilation_dispatcher: Arc<CompilationDispatcher>,
    pub permission_cache: moka::sync::Cache<String, Arc<role::Model>>,
    pub credentials_cache: moka::sync::Cache<String, Arc<RuntimeCredentials>>,
    pub state_cache: moka::sync::Cache<String, Arc<FlowLikeState>>,
    /// User+app+board-scoped typed workflow drafts retained across stateless chat HTTP requests.
    /// Each store is internally bounded; the outer TTL/cap keeps abandoned board sessions finite.
    pub flow_ir_draft_stores:
        moka::sync::Cache<String, Arc<flow_like::flow::copilot::FlowIrDraftStore>>,
    /// Process-local half of canonical app+board serialization. `board_mutation_guard` pairs each
    /// mutex with a database lock row so API replicas enter the same mutation lane.
    board_mutation_locks:
        parking_lot::Mutex<HashMap<String, Weak<flow_like_types::tokio::sync::Mutex<()>>>>,
    pub content_bucket: Arc<FlowLikeStore>,
    pub cdn_bucket: Arc<FlowLikeStore>,
    pub meta_bucket: Arc<FlowLikeStore>,
    pub response_cache: moka::sync::Cache<String, Value>,
    /// WASM package permission cache: "{user_id}:{package_id}" -> WasmPackagePermission
    pub wasm_permission_cache: moka::sync::Cache<String, WasmPackagePermission>,
    /// Auth token cache: token_hash -> CachedAuth
    /// Short TTL (240s) to balance security vs performance
    pub auth_cache: moka::sync::Cache<String, CachedAuth>,
    /// WASM package registry (optional)
    pub wasm_registry: Option<Arc<ServerRegistry>>,
    /// Sink scheduler for cron events (AWS EventBridge, K8s CronJobs, or in-memory)
    pub sink_scheduler: Option<Arc<dyn flow_like_sinks::SchedulerBackend>>,
    /// Key/value cache backend used by flows (`CACHE_BACKEND`).
    ///
    /// Built once at startup rather than per request: cache reads are far more frequent
    /// than execution-state reads, and rebuilding a Redis connection on every call would
    /// dominate the latency the cache exists to avoid.
    pub cache_store: Option<Arc<dyn crate::cache::CacheStore>>,
    /// Secret store for accessing secrets from various providers (env, AWS Parameter Store, etc.)
    pub secrets: Arc<SecretStore>,
    /// Encryption key for token encryption (derived from SINK_TOKEN_ENCRYPTION_KEY)
    pub encryption_key: [u8; 32],
    /// HMAC secret for signing/verifying sink trigger JWTs
    pub sink_secret: Option<String>,
    /// Dedicated bearer token accepted only by the internal maintenance API.
    ///
    /// This is intentionally separate from user auth, sink auth, and
    /// `BACKEND_KEY`, so a maintenance runner cannot mint broader credentials.
    pub maintenance_token: Option<String>,
    /// Idempotency cache for sink trigger requests. Keyed by the
    /// `Idempotency-Key` header; callers (Lambda, cron worker) use the
    /// invocation-unique key to collapse automatic retries into a single run.
    pub trigger_idempotency:
        moka::sync::Cache<String, crate::routes::sink::trigger::ServiceTriggerResponse>,
    /// Cached WASM package resolution — bundle of presigned URLs that the
    /// executor uses to download `.cwasm` artifacts. Keyed by `app_id`.
    /// Invalidated when an app's package list changes; the TTL is set well
    /// below the URL signing duration so signatures are always fresh.
    pub wasm_resolve_cache: moka::sync::Cache<
        String,
        Arc<std::collections::HashMap<String, flow_like_types::dispatch::WasmPackageRef>>,
    >,
}

impl State {
    fn board_mutation_lock(
        &self,
        app_id: &str,
        board_id: &str,
    ) -> Arc<flow_like_types::tokio::sync::Mutex<()>> {
        let key = board_mutation_lock_key(app_id, board_id);
        let mut locks = self.board_mutation_locks.lock();
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        // Weak entries make cleanup safe: a mutex can disappear only when no holder or waiter has
        // an Arc, so eviction can never create a second live lock for the same board.
        if locks.len() >= 4_096 {
            locks.retain(|_, lock| lock.strong_count() > 0);
        }
        let lock = Arc::new(flow_like_types::tokio::sync::Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    /// Open a transaction and acquire one durable, database-backed mutation lock.
    ///
    /// The lock row is created outside the transaction so rollback-only board mutations do not
    /// remove it. `READ COMMITTED` gives CockroachDB durable locking reads/write intents and avoids
    /// its retry-prone default `SERIALIZABLE` behavior for this mutex-only transaction; it also
    /// matches PostgreSQL's default isolation.
    pub(crate) async fn mutation_transaction(
        &self,
        lock_id: i64,
    ) -> std::result::Result<DatabaseTransaction, sea_orm::DbErr> {
        ensure_mutation_lock(&self.db, lock_id).await?;
        let transaction = self
            .db
            .begin_with_config(Some(IsolationLevel::ReadCommitted), None)
            .await?;
        acquire_mutation_lock(&transaction, lock_id).await?;
        Ok(transaction)
    }

    /// Serialize one board writer both within this process and across API replicas.
    ///
    /// The local mutex is acquired first to avoid spending a database connection on same-process
    /// waiters. The database transaction remains open solely to retain its lock-row write intent
    /// for the guard's lifetime; canonical board bytes continue to be read and written through
    /// storage.
    pub(crate) async fn board_mutation_guard(
        &self,
        app_id: &str,
        board_id: &str,
    ) -> std::result::Result<BoardMutationGuard, sea_orm::DbErr> {
        let local = self
            .board_mutation_lock(app_id, board_id)
            .lock_owned()
            .await;
        let transaction = self
            .mutation_transaction(board_mutation_lock_id(app_id, board_id))
            .await?;

        Ok(BoardMutationGuard {
            _locals: vec![local],
            transaction: Some(transaction),
        })
    }

    pub async fn new(
        catalog: Arc<Vec<Arc<dyn NodeLogic>>>,
        cdn_bucket: Arc<FlowLikeStore>,
        secret_store_config: Option<SecretStoreConfig>,
    ) -> Self {
        Self::new_inner(catalog, cdn_bucket, secret_store_config, None).await
    }

    /// Construct API state around a caller-managed database connection.
    ///
    /// Cloud targets that use short-lived, identity-backed database credentials
    /// must create the pool themselves so the access token never has to be stored
    /// in `DATABASE_URL`. The standard constructor intentionally retains its
    /// existing `DATABASE_URL` behavior for all other deployment targets.
    pub async fn new_with_database(
        catalog: Arc<Vec<Arc<dyn NodeLogic>>>,
        cdn_bucket: Arc<FlowLikeStore>,
        secret_store_config: Option<SecretStoreConfig>,
        database: DatabaseConnection,
    ) -> Self {
        Self::new_inner(catalog, cdn_bucket, secret_store_config, Some(database)).await
    }

    async fn new_inner(
        catalog: Arc<Vec<Arc<dyn NodeLogic>>>,
        cdn_bucket: Arc<FlowLikeStore>,
        secret_store_config: Option<SecretStoreConfig>,
        database: Option<DatabaseConnection>,
    ) -> Self {
        let secrets = {
            let config = secret_store_config.unwrap_or_else(|| {
                let prefix = std::env::var("SECRET_PREFIX").ok();
                SecretStoreConfig::default()
                    .with_provider(ProviderConfig::Env(EnvProviderConfig { prefix }))
            });
            Arc::new(SecretStore::new(config).expect("Failed to create secret store"))
        };

        // Batch-fetch all secrets under the prefix (e.g. SSM GetParametersByPath)
        // so individual get_secret() calls below hit the warm cache.
        secrets.warmup().await;

        let sink_secret = secrets
            .get_secret_string(&SecretRef::new("SINK_SECRET"))
            .await
            .ok()
            .map(|s| s.expose_secret().to_string());

        if sink_secret.is_none() {
            tracing::warn!(
                "SINK_SECRET not configured — sink trigger endpoints will be unavailable"
            );
        }

        let maintenance_token = secrets
            .get_secret_string(&SecretRef::new("MAINTENANCE_TOKEN"))
            .await
            .ok()
            .map(|secret| secret.expose_secret().trim().to_string())
            .filter(|token| !token.is_empty())
            .and_then(|token| {
                if token.len() < 32 {
                    tracing::error!(
                        "MAINTENANCE_TOKEN must contain at least 32 bytes — maintenance endpoint disabled"
                    );
                    None
                } else {
                    Some(token)
                }
            });

        if maintenance_token.is_none() {
            tracing::warn!(
                "MAINTENANCE_TOKEN not configured — scheduled maintenance endpoint will be unavailable"
            );
        }

        let encryption_key = {
            let key_material = secrets
                .get_secret_string(&SecretRef::new("SINK_TOKEN_ENCRYPTION_KEY"))
                .await
                .map(|s| s.expose_secret().to_string())
                .unwrap_or_else(|_| {
                    tracing::warn!(
                        "SINK_TOKEN_ENCRYPTION_KEY not set - using insecure development key. \
                        Set SINK_TOKEN_ENCRYPTION_KEY in production!"
                    );
                    "flow-like-dev-encryption-key-DO-NOT-USE-IN-PRODUCTION".to_string()
                });
            *blake3::hash(key_material.as_bytes()).as_bytes()
        };

        // Initialize backend JWT keys from the secret store
        {
            let backend_key = secrets
                .get_secret_string(&SecretRef::new("BACKEND_KEY"))
                .await;
            if let Err(ref e) = backend_key {
                tracing::error!("Failed to fetch BACKEND_KEY from secret store: {e}");
            }
            let backend_key = backend_key.ok().map(|s| s.expose_secret().to_string());
            tracing::info!(
                "BACKEND_KEY resolved: {}",
                if backend_key.is_some() { "yes" } else { "no" }
            );

            let backend_pub = secrets
                .get_secret_string(&SecretRef::new("BACKEND_PUB"))
                .await
                .ok()
                .map(|s| s.expose_secret().to_string());
            let backend_kid = secrets
                .get_secret_string(&SecretRef::new("BACKEND_KID"))
                .await
                .ok()
                .map(|s| s.expose_secret().to_string());

            crate::backend_jwt::init(
                backend_key.as_deref(),
                backend_pub.as_deref(),
                backend_kid.clone(),
            );
            crate::audit::sign::init(backend_key.as_deref(), backend_kid);
        }

        let platform_config: Hub =
            serde_json::from_str(CONFIG).expect("Failed to parse config file");
        if platform_config
            .authentication
            .as_ref()
            .is_some_and(|authentication| authentication.variant.eq_ignore_ascii_case("openid"))
        {
            openid_validation_settings_for_hub(&platform_config)
                .expect("OpenID validation configuration must be complete and exact");
        }

        let jwks = flow_like_types::json::from_str::<JwkSet>(JWKS).expect("Failed to parse JWKS");
        validate_jwks_set(&jwks).expect("Embedded OpenID JWKS must be safe and unambiguous");

        // Create content + meta buckets from master credentials (same mechanism
        // that board/storage already uses — works with IAM roles, STS, etc.)
        let master_creds = RuntimeCredentials::master_credentials()
            .await
            .expect("Failed to load master credentials");
        let content_bucket = Arc::new(
            master_creds
                .to_store(false)
                .await
                .expect("Failed to create content store from master credentials"),
        );
        let meta_bucket = Arc::new(
            master_creds
                .to_store(true)
                .await
                .expect("Failed to create meta store from master credentials"),
        );

        let client: Client<HttpConnector, Body> =
            hyper_util::client::legacy::Client::<(), ()>::builder(TokioExecutor::new())
                .build(HttpConnector::new());
        let db = match database {
            Some(database) => database,
            None => {
                let db_url = secrets
                    .get_secret_string(&SecretRef::new("DATABASE_URL"))
                    .await
                    .expect("DATABASE_URL must be set");
                let mut opt = ConnectOptions::new(db_url.expose_secret().to_owned());
                opt.max_connections(10)
                    .min_connections(1)
                    .connect_timeout(Duration::from_secs(8))
                    .connect_lazy(true)
                    .sqlx_logging(platform_config.environment == Environment::Development);

                Database::connect(opt)
                    .await
                    .expect("Failed to connect to database")
            }
        };

        if let Err(error) = crate::db_backfills::run_startup_backfills(&db).await {
            tracing::warn!("Failed to run startup database backfills: {error}");
        }

        let stripe_client = if platform_config.features.premium {
            let stripe_key = secrets
                .get_secret_string(&SecretRef::new("STRIPE_SECRET_KEY"))
                .await
                .expect("STRIPE_SECRET_KEY must be set");
            let exposed = stripe_key.expose_secret();
            let preview: String = exposed.chars().take(8).collect();
            tracing::info!("Stripe client initialized (key starts with: {preview}…)");
            let stripe_client = stripe::Client::new(exposed);
            Some(stripe_client)
        } else {
            None
        };

        let mut provider = ModelProviderConfiguration::default();

        let openai_endpoint = std::env::var("OPENAI_ENDPOINT").ok();
        let openai_key = std::env::var("OPENAI_API_KEY").ok();

        if let (Some(endpoint), Some(key)) = (openai_endpoint, openai_key) {
            provider.openai_config.push(OpenAIConfig {
                endpoint: Some(endpoint),
                api_key: Some(key),
                organization: None,
                proxy: None,
            })
        }

        let registry = FlowNodeRegistryInner::prepare(&catalog);

        let cache = moka::sync::Cache::builder()
            .max_capacity(32 * 1024 * 1024) // 32 MB
            .time_to_live(Duration::from_secs(20 * 60)) // 20 minutes — credentials are valid for 1h, so cached ones always have ≥40min remaining
            .build();

        let response_cache = moka::sync::Cache::builder()
            .max_capacity(64 * 1024 * 1024) // 32 MB
            .time_to_live(Duration::from_secs(60)) // 30 minutes
            .build();

        let mail_client = if let Some(mail_config) = &platform_config.mail {
            match create_mail_client(mail_config).await {
                Ok(client) => Some(client),
                Err(e) => {
                    tracing::warn!("Failed to initialize mail client: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Initialize dispatcher once with env config (caches AWS/Redis clients)
        let dispatch_config = DispatchConfig::from_env();
        let dispatcher = Dispatcher::new(dispatch_config, Some(meta_bucket.clone())).await;

        // Initialize compilation dispatcher (mirrors execution dispatcher pattern)
        let compilation_config = CompilationDispatchConfig::from_env();
        tracing::info!(backend = ?compilation_config.backend, "Compilation dispatch backend");
        let compilation_dispatcher = Arc::new(
            CompilationDispatcher::new(
                compilation_config,
                content_bucket.clone(),
                meta_bucket.clone(),
            )
            .await,
        );

        // Initialize WASM registry if enabled (uses PostgreSQL)
        let wasm_registry = if platform_config.features.wasm_registry {
            let registry =
                ServerRegistry::new(db.clone(), content_bucket.clone(), meta_bucket.clone())
                    .with_compilation_dispatcher(compilation_dispatcher.clone());
            Some(Arc::new(registry))
        } else {
            None
        };

        // Initialize sink scheduler based on environment
        // Priority: AWS EventBridge > Kubernetes > None (sink-service polls /schedules)
        let sink_scheduler: Option<Arc<dyn flow_like_sinks::SchedulerBackend>> = {
            let scheduler_provider = std::env::var("SINK_SCHEDULER_PROVIDER")
                .ok()
                .map(|s| flow_like_sinks::scheduler::SchedulerProvider::from_str(&s));

            match scheduler_provider {
                Some(flow_like_sinks::scheduler::SchedulerProvider::Aws) => {
                    #[cfg(feature = "aws")]
                    {
                        let scheduler =
                            flow_like_sinks::scheduler::AwsEventBridgeScheduler::from_env().await;
                        tracing::info!("Initialized AWS EventBridge sink scheduler");
                        Some(Arc::new(scheduler) as Arc<dyn flow_like_sinks::SchedulerBackend>)
                    }
                    #[cfg(not(feature = "aws"))]
                    {
                        tracing::warn!("AWS scheduler requested but aws feature not enabled");
                        None
                    }
                }
                Some(flow_like_sinks::scheduler::SchedulerProvider::Kubernetes) => {
                    #[cfg(feature = "kubernetes")]
                    {
                        match flow_like_sinks::scheduler::KubernetesScheduler::from_env().await {
                            Ok(scheduler) => {
                                tracing::info!("Initialized Kubernetes CronJob sink scheduler");
                                Some(Arc::new(scheduler)
                                    as Arc<dyn flow_like_sinks::SchedulerBackend>)
                            }
                            Err(e) => {
                                tracing::warn!("Failed to initialize K8s scheduler: {}", e);
                                None
                            }
                        }
                    }
                    #[cfg(not(feature = "kubernetes"))]
                    {
                        tracing::warn!(
                            "Kubernetes scheduler requested but kubernetes feature not enabled"
                        );
                        None
                    }
                }
                Some(flow_like_sinks::scheduler::SchedulerProvider::Memory) => {
                    tracing::info!("Using in-memory sink scheduler");
                    Some(
                        Arc::new(flow_like_sinks::scheduler::InMemoryScheduler::new())
                            as Arc<dyn flow_like_sinks::SchedulerBackend>,
                    )
                }
                None => {
                    tracing::debug!(
                        "No sink scheduler configured (SINK_SCHEDULER_PROVIDER not set)"
                    );
                    None
                }
            }
        };

        // A cache the flows cannot reach is better surfaced as an explicit 503 from the
        // cache endpoints than as a failed boot for every other feature.
        let cache_store = {
            let config = crate::cache::CacheStoreConfig::default().with_db(Arc::new(db.clone()));
            match crate::cache::create_cache_store(config).await {
                Ok(store) => {
                    tracing::info!(backend = store.backend_name(), "Initialized cache backend");
                    Some(store)
                }
                Err(error) => {
                    tracing::error!(
                        error = %error,
                        "Failed to initialize the cache backend; cache endpoints will return 503"
                    );
                    None
                }
            }
        };

        Self {
            platform_config,
            db,
            client,
            jwks: flow_like_types::tokio::sync::RwLock::new(jwks),
            jwks_refresh: flow_like_types::tokio::sync::Mutex::new(JwksRefreshState::default()),
            stripe_client,
            mail_client,
            #[cfg(feature = "aws")]
            aws_client: Arc::new(aws_config::load_from_env().await),
            catalog,
            provider: Arc::new(provider),
            registry: Arc::new(registry),
            dispatcher: Arc::new(dispatcher),
            compilation_dispatcher,
            permission_cache: moka::sync::Cache::builder()
                .max_capacity(32 * 1024 * 1024)
                .time_to_live(Duration::from_secs(120))
                .build(),
            state_cache: moka::sync::Cache::builder()
                .max_capacity(32 * 1024 * 1024) // 32 MB
                .time_to_live(Duration::from_secs(30 * 60))
                .build(),
            flow_ir_draft_stores: moka::sync::Cache::builder()
                .max_capacity(2_048)
                .time_to_idle(Duration::from_secs(2 * 60 * 60))
                .build(),
            board_mutation_locks: parking_lot::Mutex::new(HashMap::new()),
            credentials_cache: cache,
            content_bucket,
            cdn_bucket,
            meta_bucket,
            response_cache,
            wasm_permission_cache: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(120))
                .build(),
            // Auth cache: max 10k entries, 60s TTL for security
            // Entries are keyed by token hash to avoid storing raw tokens
            auth_cache: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(240))
                .build(),
            wasm_registry,
            sink_scheduler,
            cache_store,
            secrets,
            encryption_key,
            sink_secret,
            maintenance_token,
            trigger_idempotency: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(15 * 60))
                .build(),
            wasm_resolve_cache: moka::sync::Cache::builder()
                .max_capacity(1_000)
                // Presigned URLs are signed for 1h. Cap our cache at half of
                // that so callers always receive a URL with ≥30 min remaining.
                .time_to_live(Duration::from_secs(30 * 60))
                .time_to_idle(Duration::from_secs(10 * 60))
                .build(),
        }
    }

    /// Invalidate the cached WASM package resolution for an app. Call this
    /// when the app's package list changes (add/update/delete).
    pub fn invalidate_wasm_resolve(&self, app_id: &str) {
        self.wasm_resolve_cache.invalidate(app_id);
    }

    fn openid_validation_settings(&self) -> Result<OpenIdValidationSettings> {
        openid_validation_settings_for_hub(&self.platform_config)
    }

    async fn configured_jwk(&self, kid: &str) -> Result<Jwk> {
        {
            let jwks = self.jwks.read().await;
            if let Some(jwk) = find_unique_jwk(&*jwks, kid)? {
                return Ok(jwk);
            }
        }

        // A public JWKS endpoint needs no client secret. Refresh only the fixed,
        // reviewed URL from the embedded deployment config, serialize refreshes,
        // and rate-limit failed/attacker-triggered unknown-kid requests.
        let mut refresh = self.jwks_refresh.lock().await;
        {
            let jwks = self.jwks.read().await;
            if let Some(jwk) = find_unique_jwk(&*jwks, kid)? {
                return Ok(jwk);
            }
        }
        if refresh
            .last_attempt
            .is_some_and(|last| last.elapsed() < JWKS_REFRESH_MIN_INTERVAL)
        {
            bail!("OpenID signing key is unknown and JWKS refresh is rate limited");
        }
        refresh.last_attempt = Some(Instant::now());

        let settings = self.openid_validation_settings()?;
        let refreshed = fetch_jwks(&settings.jwks_url).await?;
        let jwk = find_unique_jwk(&refreshed, kid)?
            .ok_or_else(|| flow_like_types::anyhow!("OpenID signing key is not published"))?;
        *self.jwks.write().await = refreshed;
        Ok(jwk)
    }

    pub(crate) async fn validate_token(&self, token: &str) -> Result<ValidatedOpenIdToken> {
        let settings = self.openid_validation_settings()?;
        let header = jsonwebtoken::decode_header(token)?;
        ensure_allowed_oidc_algorithm(header.alg)?;
        let kid = header
            .kid
            .as_deref()
            .filter(|kid| !kid.is_empty() && kid.len() <= 256)
            .ok_or_else(|| flow_like_types::anyhow!("OpenID token has no valid kid"))?;
        let jwk = self.configured_jwk(kid).await?;
        validate_jwk_for_header(&jwk, kid, header.alg)?;

        let decoding_key = decoding_key_for_algorithm(&jwk.algorithm)?;
        let mut validation = Validation::new(header.alg);
        validation.algorithms = vec![header.alg];
        validation.leeway = settings.leeway;
        validation.validate_exp = true;
        validation.validate_nbf = true;
        // Cognito access tokens use `client_id` rather than `aud`. Validate
        // both target-claim forms explicitly below without weakening either.
        validation.validate_aud = false;
        validation.set_issuer(&[&settings.issuer]);
        validation.set_required_spec_claims(&["exp", "iss", "sub"]);

        let decoded = decode::<HashMap<String, Value>>(token, &decoding_key, &validation)?;
        let expires_at = validate_openid_claims(
            &decoded.claims,
            &settings.issuer,
            &settings.client_ids,
            &settings.audience,
            settings.tenant_id,
            chrono::Utc::now().timestamp(),
            settings.leeway,
        )?;

        Ok(ValidatedOpenIdToken {
            claims: decoded.claims,
            expires_at,
        })
    }

    #[tracing::instrument(
        name = "scoped_credentials",
        skip(self),
        fields(sub, app_id, board_id, version)
    )]
    pub async fn scoped_credentials(
        &self,
        sub: &str,
        app_id: &str,
        mode: CredentialsAccess,
    ) -> flow_like_types::Result<Arc<RuntimeCredentials>> {
        let key = format!("{}:{}:{}", sub, app_id, mode);
        if let Some(credentials) = self.credentials_cache.get(&key) {
            return Ok(credentials);
        }
        let credentials = RuntimeCredentials::scoped(sub, app_id, self, mode).await?;
        self.credentials_cache
            .insert(key, Arc::new(credentials.clone()));
        Ok(Arc::new(credentials))
    }

    #[tracing::instrument(
        name = "scoped_app",
        skip(self, state),
        fields(sub, app_id, board_id, version)
    )]
    pub async fn scoped_app(
        &self,
        sub: &str,
        app_id: &str,
        state: &AppState,
        mode: CredentialsAccess,
    ) -> flow_like_types::Result<App> {
        let credentials = self.scoped_credentials(sub, app_id, mode).await?;
        let app_state = Arc::new(credentials.to_state(state.clone()).await?);

        let app = App::load(app_id.to_string(), app_state.clone()).await?;

        Ok(app)
    }

    #[tracing::instrument(
        name = "master_app",
        skip(self, state, _sub),
        fields(sub, app_id, board_id, version)
    )]
    pub async fn master_app(
        &self,
        _sub: &str,
        app_id: &str,
        state: &AppState,
    ) -> flow_like_types::Result<App> {
        let credentials = self.master_credentials().await?;

        let app_state = self.state_cache.get("master");

        let app_state = match app_state {
            Some(state) => state,
            None => {
                let state = Arc::new(credentials.to_state(state.clone()).await?);
                self.state_cache.insert("master".to_string(), state.clone());
                state
            }
        };

        let app = App::load(app_id.to_string(), app_state.clone()).await?;

        Ok(app)
    }

    #[tracing::instrument(
        name = "scoped_board",
        skip(self, state),
        level = "debug",
        fields(sub, app_id, board_id, version)
    )]
    pub async fn scoped_board(
        &self,
        sub: &str,
        app_id: &str,
        board_id: &str,
        state: &AppState,
        version: Option<(u32, u32, u32)>,
        mode: CredentialsAccess,
    ) -> flow_like_types::Result<Board> {
        let credentials = self.scoped_credentials(sub, app_id, mode).await?;
        let app_state = Arc::new(credentials.to_state(state.clone()).await?);
        let storage_root = Path::from("apps").child(app_id.to_string());
        let board = Board::load(storage_root, board_id, app_state, version).await?;
        Ok(board)
    }

    #[tracing::instrument(
        name = "master_board",
        skip(self, state, _sub),
        level = "debug",
        fields(sub, app_id, board_id, version)
    )]
    pub async fn master_board(
        &self,
        _sub: &str,
        app_id: &str,
        board_id: &str,
        state: &AppState,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Board> {
        let credentials = self.master_credentials().await?;

        let app_state = self.state_cache.get("master");

        let app_state = match app_state {
            Some(state) => state,
            None => {
                let state = Arc::new(credentials.to_state(state.clone()).await?);
                self.state_cache.insert("master".to_string(), state.clone());
                state
            }
        };

        let storage_root = Path::from("apps").child(app_id.to_string());
        let board = Board::load(storage_root, board_id, app_state, version).await?;

        Ok(board)
    }

    /// Load a template on master credentials, for callers that were authorized
    /// by something other than app membership — e.g. the public template
    /// preview, which is gated on the owning app's visibility.
    #[tracing::instrument(
        name = "master_template",
        skip(self, state),
        level = "debug",
        fields(app_id, template_id, version)
    )]
    pub async fn master_template(
        &self,
        app_id: &str,
        template_id: &str,
        state: &AppState,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Board> {
        let credentials = self.master_credentials().await?;

        let app_state = match self.state_cache.get("master") {
            Some(state) => state,
            None => {
                let state = Arc::new(credentials.to_state(state.clone()).await?);
                self.state_cache.insert("master".to_string(), state.clone());
                state
            }
        };

        let storage_root = Path::from("apps").child(app_id.to_string());
        let board = Board::load_template(storage_root, template_id, app_state, version).await?;

        Ok(board)
    }

    pub async fn scoped_template(
        &self,
        sub: &str,
        app_id: &str,
        template_id: &str,
        state: &AppState,
        version: Option<(u32, u32, u32)>,
        mode: CredentialsAccess,
    ) -> flow_like_types::Result<Board> {
        let credentials = self.scoped_credentials(sub, app_id, mode).await?;
        let app_state = Arc::new(credentials.to_state(state.clone()).await?);

        let storage_root = Path::from("apps").child(app_id.to_string());

        let board = Board::load_template(storage_root, template_id, app_state, version).await?;

        Ok(board)
    }

    pub async fn master_credentials(&self) -> flow_like_types::Result<Arc<RuntimeCredentials>> {
        let credentials = self.credentials_cache.get("master");
        if let Some(credentials) = credentials {
            return Ok(credentials);
        }
        let credentials = Arc::new(RuntimeCredentials::master_credentials().await?);
        self.credentials_cache
            .insert("master".to_string(), credentials.clone());
        Ok(credentials)
    }

    pub fn check_permission(&self, sub: &str, app_id: &str) -> Option<Arc<role::Model>> {
        let key = format!("{}:{}", sub, app_id);
        self.permission_cache.get(&key)
    }

    pub fn put_permission(&self, sub: &str, app_id: &str, role: Arc<role::Model>) {
        let key = format!("{}:{}", sub, app_id);
        self.permission_cache.insert(key, role);
    }

    pub fn invalidate_permission(&self, sub: &str, app_id: &str) {
        let key = format!("{}:{}", sub, app_id);
        self.permission_cache.invalidate(&key);
    }

    pub fn check_wasm_permission(
        &self,
        user_id: &str,
        package_id: &str,
    ) -> Option<WasmPackagePermission> {
        let key = format!("wasm:{}:{}", user_id, package_id);
        self.wasm_permission_cache.get(&key)
    }

    pub fn put_wasm_permission(
        &self,
        user_id: &str,
        package_id: &str,
        perm: WasmPackagePermission,
    ) {
        let key = format!("wasm:{}:{}", user_id, package_id);
        self.wasm_permission_cache.insert(key, perm);
    }

    pub fn invalidate_wasm_permission(&self, user_id: &str, package_id: &str) {
        let key = format!("wasm:{}:{}", user_id, package_id);
        self.wasm_permission_cache.invalidate(&key);
    }

    pub async fn invalidate_role_permissions(
        &self,
        role_id: &str,
        app_id: &str,
    ) -> flow_like_types::Result<()> {
        use crate::entity::{app_connection, membership};
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

        let user_ids: Vec<String> = membership::Entity::find()
            .filter(membership::Column::RoleId.eq(role_id))
            .filter(membership::Column::AppId.eq(app_id))
            .select_only()
            .column(membership::Column::UserId)
            .into_tuple()
            .all(&self.db)
            .await?;

        for user_id in &user_ids {
            self.invalidate_permission(user_id, app_id);
        }

        let source_app_ids: Vec<String> = app_connection::Entity::find()
            .filter(app_connection::Column::RoleId.eq(role_id))
            .filter(app_connection::Column::TargetAppId.eq(app_id))
            .select_only()
            .column(app_connection::Column::SourceAppId)
            .into_tuple()
            .all(&self.db)
            .await?;

        for source_app_id in &source_app_ids {
            self.invalidate_permission(
                &crate::middleware::jwt::app_connection_cache_sub(source_app_id),
                app_id,
            );
        }

        Ok(())
    }

    pub fn get_cache<T>(&self, key: &str) -> Option<T>
    where
        T: serde::de::DeserializeOwned,
    {
        self.response_cache
            .get(key)
            .and_then(|json_value| serde_json::from_value(json_value).ok())
    }

    pub fn set_cache<T>(&self, key: String, value: T)
    where
        T: serde::Serialize,
    {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.response_cache.insert(key, json_value);
        }
    }

    pub fn invalidate_cache(&self, key: &str) {
        self.response_cache.invalidate(key);
    }
}

fn openid_validation_settings_for_hub(hub: &Hub) -> Result<OpenIdValidationSettings> {
    let authentication = hub
        .authentication
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("OpenID authentication is not configured"))?;
    if !authentication.variant.eq_ignore_ascii_case("openid") {
        bail!("OpenID authentication is not enabled");
    }

    let config = authentication
        .openid
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("OpenID configuration is missing"))?;
    let issuer = match config.issuer.as_deref() {
        Some(issuer) => exact_nonempty_str("authentication.openid.issuer", issuer)?,
        None => exact_nonempty_setting(
            "authentication.openid.issuer or authentication.openid.authority",
            &config.authority,
        )?,
    };
    let client_id = exact_nonempty_setting("authentication.openid.client_id", &config.client_id)?;
    let audience = match config.audience.as_deref() {
        Some(audience) => exact_nonempty_str("authentication.openid.audience", audience)?,
        None => client_id,
    };
    let jwks_url = exact_nonempty_str("authentication.openid.jwks_url", &config.jwks_url)?;
    validate_jwks_url(jwks_url)?;

    let overrides = openid_validation_overrides();
    let mut client_ids = BTreeSet::new();
    client_ids.insert(client_id.to_string());
    for additional in &overrides.additional_client_ids {
        let additional =
            exact_nonempty_str("authentication.openid.additional_client_ids", additional)?;
        client_ids.insert(additional.to_string());
    }

    Ok(OpenIdValidationSettings {
        tenant_id: entra_tenant_from_issuer(issuer),
        issuer: issuer.to_string(),
        client_ids,
        audience: audience.to_string(),
        jwks_url: jwks_url.to_string(),
        leeway: overrides.leeway_seconds,
    })
}

/// `OpenIdConfig` ignores unknown keys, so the additive validation settings are
/// read from the same embedded config document that produced the `Hub`.
fn openid_validation_overrides() -> OpenIdValidationOverrides {
    serde_json::from_str::<Value>(CONFIG)
        .ok()
        .and_then(|config| {
            config
                .get("authentication")
                .and_then(|authentication| authentication.get("openid"))
                .cloned()
        })
        .and_then(|openid| serde_json::from_value(openid).ok())
        .unwrap_or_default()
}

fn exact_nonempty_setting<'a>(name: &str, value: &'a Option<String>) -> Result<&'a str> {
    let value = value
        .as_deref()
        .ok_or_else(|| flow_like_types::anyhow!("{name} must be configured"))?;
    exact_nonempty_str(name, value)
}

fn exact_nonempty_str<'a>(name: &str, value: &'a str) -> Result<&'a str> {
    if value.is_empty() || value.trim() != value {
        bail!("{name} must be a non-empty exact value without surrounding whitespace");
    }
    Ok(value)
}

fn validate_jwks_url(raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || url.fragment().is_some()
    {
        bail!("OpenID jwks_url must be an absolute HTTPS URL without credentials or fragment");
    }
    Ok(url)
}

fn entra_tenant_from_issuer(issuer: &str) -> Option<uuid::Uuid> {
    let url = reqwest::Url::parse(issuer).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let is_entra = matches!(
        host.as_str(),
        "login.microsoftonline.com"
            | "login.microsoftonline.us"
            | "login.microsoftonline.de"
            | "login.partner.microsoftonline.cn"
            | "sts.windows.net"
    ) || host.ends_with(".ciamlogin.com");
    if !is_entra {
        return None;
    }

    url.path_segments()?
        .find_map(|segment| uuid::Uuid::parse_str(segment).ok())
}

async fn fetch_jwks(raw_url: &str) -> Result<JwkSet> {
    let url = validate_jwks_url(raw_url)?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(JWKS_REQUEST_TIMEOUT)
        .build()?;
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?;
    if !response.status().is_success() {
        bail!("OpenID JWKS endpoint returned a non-success status");
    }
    if response
        .content_length()
        .is_some_and(|length| length > JWKS_MAX_RESPONSE_BYTES as u64)
    {
        bail!("OpenID JWKS response exceeds the configured size limit");
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > JWKS_MAX_RESPONSE_BYTES {
            bail!("OpenID JWKS response exceeds the configured size limit");
        }
        body.extend_from_slice(&chunk);
    }
    let jwks: JwkSet = serde_json::from_slice(&body)?;
    validate_jwks_set(&jwks)?;
    Ok(jwks)
}

fn validate_jwks_set(jwks: &JwkSet) -> Result<()> {
    if jwks.keys.is_empty() || jwks.keys.len() > JWKS_MAX_KEYS {
        bail!("OpenID JWKS contains an invalid number of keys");
    }
    let mut seen = std::collections::HashSet::with_capacity(jwks.keys.len());
    for key in &jwks.keys {
        let kid = key
            .common
            .key_id
            .as_deref()
            .filter(|kid| !kid.is_empty() && kid.len() <= 256)
            .ok_or_else(|| flow_like_types::anyhow!("OpenID JWK has no valid kid"))?;
        if !seen.insert(kid) {
            bail!("OpenID JWKS contains duplicate kid values");
        }
    }
    Ok(())
}

fn find_unique_jwk(jwks: &JwkSet, kid: &str) -> Result<Option<Jwk>> {
    let mut matches = jwks
        .keys
        .iter()
        .filter(|key| key.common.key_id.as_deref() == Some(kid));
    let result = matches.next().cloned();
    if matches.next().is_some() {
        bail!("OpenID JWKS contains duplicate kid values");
    }
    Ok(result)
}

fn ensure_allowed_oidc_algorithm(algorithm: Algorithm) -> Result<()> {
    match algorithm {
        Algorithm::RS256
        | Algorithm::RS384
        | Algorithm::RS512
        | Algorithm::PS256
        | Algorithm::PS384
        | Algorithm::PS512
        | Algorithm::ES256
        | Algorithm::ES384
        | Algorithm::EdDSA => Ok(()),
        _ => bail!("OpenID token uses a disallowed signing algorithm"),
    }
}

fn jwk_algorithm(algorithm: KeyAlgorithm) -> Result<Algorithm> {
    match algorithm {
        KeyAlgorithm::RS256 => Ok(Algorithm::RS256),
        KeyAlgorithm::RS384 => Ok(Algorithm::RS384),
        KeyAlgorithm::RS512 => Ok(Algorithm::RS512),
        KeyAlgorithm::PS256 => Ok(Algorithm::PS256),
        KeyAlgorithm::PS384 => Ok(Algorithm::PS384),
        KeyAlgorithm::PS512 => Ok(Algorithm::PS512),
        KeyAlgorithm::ES256 => Ok(Algorithm::ES256),
        KeyAlgorithm::ES384 => Ok(Algorithm::ES384),
        KeyAlgorithm::EdDSA => Ok(Algorithm::EdDSA),
        _ => bail!("OpenID JWK uses a disallowed or unsupported algorithm"),
    }
}

fn validate_jwk_for_header(jwk: &Jwk, kid: &str, header_algorithm: Algorithm) -> Result<()> {
    ensure_allowed_oidc_algorithm(header_algorithm)?;
    if jwk.common.key_id.as_deref() != Some(kid) {
        bail!("OpenID JWK kid does not match the token header");
    }
    if let Some(public_key_use) = &jwk.common.public_key_use
        && public_key_use != &PublicKeyUse::Signature
    {
        bail!("OpenID JWK is not designated for signatures");
    }
    if let Some(operations) = &jwk.common.key_operations
        && !operations.contains(&KeyOperations::Verify)
    {
        bail!("OpenID JWK is not designated for signature verification");
    }

    // Entra ID JWKS omit `alg`; the header algorithm is still bound by the
    // allowlist above and the key-type match below.
    if let Some(key_algorithm) = jwk.common.key_algorithm
        && jwk_algorithm(key_algorithm)? != header_algorithm
    {
        bail!("OpenID JWK alg does not match the token header");
    }
    let compatible_key_type = matches!(
        (&jwk.algorithm, header_algorithm),
        (
            AlgorithmParameters::RSA(_),
            Algorithm::RS256
                | Algorithm::RS384
                | Algorithm::RS512
                | Algorithm::PS256
                | Algorithm::PS384
                | Algorithm::PS512
        ) | (
            AlgorithmParameters::EllipticCurve(jsonwebtoken::jwk::EllipticCurveKeyParameters {
                curve: EllipticCurve::P256,
                ..
            }),
            Algorithm::ES256
        ) | (
            AlgorithmParameters::EllipticCurve(jsonwebtoken::jwk::EllipticCurveKeyParameters {
                curve: EllipticCurve::P384,
                ..
            }),
            Algorithm::ES384
        ) | (
            AlgorithmParameters::OctetKeyPair(jsonwebtoken::jwk::OctetKeyPairParameters {
                curve: EllipticCurve::Ed25519,
                ..
            }),
            Algorithm::EdDSA
        )
    );
    if !compatible_key_type {
        bail!("OpenID JWK key type does not match its signing algorithm");
    }
    Ok(())
}

fn numeric_date(claims: &HashMap<String, Value>, name: &str) -> Result<i64> {
    let value = claims
        .get(name)
        .ok_or_else(|| flow_like_types::anyhow!("OpenID token is missing {name}"))?;
    if let Some(value) = value.as_i64() {
        return Ok(value);
    }
    if let Some(value) = value.as_u64() {
        return i64::try_from(value)
            .map_err(|_| flow_like_types::anyhow!("OpenID token has an invalid {name}"));
    }
    bail!("OpenID token has an invalid {name}")
}

fn validate_openid_claims(
    claims: &HashMap<String, Value>,
    expected_issuer: &str,
    expected_client_ids: &BTreeSet<String>,
    expected_audience: &str,
    expected_tenant: Option<uuid::Uuid>,
    now: i64,
    leeway: u64,
) -> Result<i64> {
    let leeway = i64::try_from(leeway).unwrap_or(i64::MAX);
    let is_expected_client_id =
        |value: Option<&str>| value.is_some_and(|value| expected_client_ids.contains(value));
    let issuer = claims
        .get("iss")
        .and_then(Value::as_str)
        .ok_or_else(|| flow_like_types::anyhow!("OpenID token has no string issuer"))?;
    if issuer != expected_issuer {
        bail!("OpenID token issuer does not match the configured issuer");
    }
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|subject| !subject.is_empty())
        .ok_or_else(|| flow_like_types::anyhow!("OpenID token has no valid subject"))?;
    let _ = subject;

    let expires_at = numeric_date(claims, "exp")?;
    if expires_at.saturating_add(leeway) <= now {
        bail!("OpenID token is expired");
    }
    if let Some(not_before) = claims.get("nbf") {
        let not_before = not_before
            .as_i64()
            .or_else(|| {
                not_before
                    .as_u64()
                    .and_then(|value| i64::try_from(value).ok())
            })
            .ok_or_else(|| flow_like_types::anyhow!("OpenID token has an invalid nbf"))?;
        if not_before > now.saturating_add(leeway) {
            bail!("OpenID token is not valid yet");
        }
    }

    let mut has_target_claim = false;
    if let Some(audience) = claims.get("aud") {
        has_target_claim = true;
        let audience_matches = match audience {
            Value::String(value) => value == expected_audience,
            Value::Array(values) => {
                if values.is_empty() || values.iter().any(|value| !value.is_string()) {
                    bail!("OpenID token has an invalid audience");
                }
                let matches = values
                    .iter()
                    .any(|value| value.as_str() == Some(expected_audience));
                if values.len() > 1
                    && !is_expected_client_id(claims.get("azp").and_then(Value::as_str))
                {
                    bail!("OpenID token with multiple audiences has an invalid azp");
                }
                matches
            }
            _ => bail!("OpenID token has an invalid audience"),
        };
        if !audience_matches {
            bail!("OpenID token audience does not match the configured audience");
        }
    }
    if let Some(client_id) = claims.get("client_id") {
        has_target_claim = true;
        if !is_expected_client_id(client_id.as_str()) {
            bail!("OpenID token client_id does not match a configured client_id");
        }
    }
    for authorized_party_claim in ["azp", "appid"] {
        if let Some(authorized_party) = claims.get(authorized_party_claim)
            && !is_expected_client_id(authorized_party.as_str())
        {
            bail!("OpenID token {authorized_party_claim} does not match a configured client_id");
        }
    }
    if !has_target_claim {
        bail!("OpenID token has neither an audience nor a client_id");
    }

    if let Some(expected_tenant) = expected_tenant {
        let tenant = claims
            .get("tid")
            .and_then(Value::as_str)
            .and_then(|tenant| uuid::Uuid::parse_str(tenant).ok())
            .ok_or_else(|| flow_like_types::anyhow!("Entra token has no valid tid"))?;
        if tenant != expected_tenant {
            bail!("Entra token tid does not match the issuer tenant");
        }
    }

    Ok(expires_at)
}

pub(crate) fn cached_openid_is_current(exp: i64, now: i64) -> bool {
    exp > now
}

fn decoding_key_for_algorithm(alg: &AlgorithmParameters) -> flow_like_types::Result<DecodingKey> {
    let key = match alg {
        AlgorithmParameters::RSA(rsa) => DecodingKey::from_rsa_components(&rsa.n, &rsa.e),
        AlgorithmParameters::EllipticCurve(ec) => DecodingKey::from_ec_components(&ec.x, &ec.y),
        AlgorithmParameters::OctetKeyPair(octet) => DecodingKey::from_ed_components(&octet.x),
        _ => bail!("Unsupported algorithm"),
    }?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::{
        ACQUIRE_MUTATION_LOCK_SQL, ENSURE_MUTATION_LOCK_SQL, board_mutation_lock_id,
        board_mutation_lock_key, cached_openid_is_current, course_attempt_lock_id,
        entra_tenant_from_issuer, flow_ir_draft_store_key, validate_jwk_for_header,
        validate_jwks_set, validate_openid_claims,
    };
    use flow_like_types::Value;
    use jsonwebtoken::{
        Algorithm,
        jwk::{Jwk, JwkSet},
    };
    use std::collections::{BTreeSet, HashMap};

    fn claims(values: &[(&str, Value)]) -> HashMap<String, Value> {
        values
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect()
    }

    fn client_ids(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn rsa_jwk(algorithm: &str, kid: &str) -> Jwk {
        serde_json::from_value(serde_json::json!({
            "kty": "RSA",
            "use": "sig",
            "key_ops": ["verify"],
            "alg": algorithm,
            "kid": kid,
            "n": "sXchvX3L7MdCKMImnlUiVDXQ4x_8OmtkPL3MyT9c6nr8YjC-rf1W_gKVVdQVrWjQxw",
            "e": "AQAB"
        }))
        .expect("valid test JWK")
    }

    #[test]
    fn openid_claims_require_exact_issuer_and_client_target() {
        let valid = claims(&[
            ("iss", Value::String("https://issuer.example/tenant".into())),
            ("sub", Value::String("user".into())),
            ("aud", Value::String("client".into())),
            ("exp", Value::from(2_000_i64)),
            ("nbf", Value::from(900_i64)),
        ]);
        assert_eq!(
            validate_openid_claims(
                &valid,
                "https://issuer.example/tenant",
                &client_ids(&["client"]),
                "client",
                None,
                1_000,
                0,
            )
            .unwrap(),
            2_000
        );

        let mut wrong_issuer = valid.clone();
        wrong_issuer.insert(
            "iss".into(),
            Value::String("https://attacker.example".into()),
        );
        assert!(
            validate_openid_claims(
                &wrong_issuer,
                "https://issuer.example/tenant",
                &client_ids(&["client"]),
                "client",
                None,
                1_000,
                0,
            )
            .is_err()
        );

        let mut wrong_audience = valid.clone();
        wrong_audience.insert("aud".into(), Value::String("other-client".into()));
        assert!(
            validate_openid_claims(
                &wrong_audience,
                "https://issuer.example/tenant",
                &client_ids(&["client"]),
                "client",
                None,
                1_000,
                0,
            )
            .is_err()
        );
    }

    #[test]
    fn openid_claims_enforce_time_tenant_and_authorized_party() {
        let tenant = uuid::Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
        let issuer = format!("https://login.microsoftonline.com/{tenant}/v2.0");
        assert_eq!(entra_tenant_from_issuer(&issuer), Some(tenant));

        let base = claims(&[
            ("iss", Value::String(issuer.clone())),
            ("sub", Value::String("user".into())),
            ("aud", serde_json::json!(["client", "another-audience"])),
            ("azp", Value::String("client".into())),
            ("tid", Value::String(tenant.to_string())),
            ("exp", Value::from(2_000_i64)),
            ("nbf", Value::from(900_i64)),
        ]);
        let accepted = client_ids(&["client"]);
        assert!(
            validate_openid_claims(&base, &issuer, &accepted, "client", Some(tenant), 1_000, 0)
                .is_ok()
        );

        let mut future = base.clone();
        future.insert("nbf".into(), Value::from(1_001_i64));
        assert!(
            validate_openid_claims(
                &future,
                &issuer,
                &accepted,
                "client",
                Some(tenant),
                1_000,
                0
            )
            .is_err()
        );
        assert!(
            validate_openid_claims(
                &future,
                &issuer,
                &accepted,
                "client",
                Some(tenant),
                1_000,
                60
            )
            .is_ok()
        );

        let mut expired = base.clone();
        expired.insert("exp".into(), Value::from(1_000_i64));
        assert!(
            validate_openid_claims(
                &expired,
                &issuer,
                &accepted,
                "client",
                Some(tenant),
                1_000,
                0
            )
            .is_err()
        );
        assert!(
            validate_openid_claims(
                &expired,
                &issuer,
                &accepted,
                "client",
                Some(tenant),
                1_040,
                60
            )
            .is_ok()
        );
        assert!(
            validate_openid_claims(
                &expired,
                &issuer,
                &accepted,
                "client",
                Some(tenant),
                1_100,
                60
            )
            .is_err()
        );

        let mut wrong_azp = base.clone();
        wrong_azp.insert("azp".into(), Value::String("attacker-client".into()));
        assert!(
            validate_openid_claims(
                &wrong_azp,
                &issuer,
                &accepted,
                "client",
                Some(tenant),
                1_000,
                0,
            )
            .is_err()
        );

        let mut wrong_tenant = base;
        wrong_tenant.insert(
            "tid".into(),
            Value::String("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".into()),
        );
        assert!(
            validate_openid_claims(
                &wrong_tenant,
                &issuer,
                &accepted,
                "client",
                Some(tenant),
                1_000,
                0,
            )
            .is_err()
        );
    }

    #[test]
    fn openid_accepts_cognito_client_id_and_rejects_conflicts() {
        let mut access_token = claims(&[
            ("iss", Value::String("https://cognito.example/pool".into())),
            ("sub", Value::String("user".into())),
            ("client_id", Value::String("client".into())),
            ("exp", Value::from(2_000_i64)),
        ]);
        assert!(
            validate_openid_claims(
                &access_token,
                "https://cognito.example/pool",
                &client_ids(&["client"]),
                "client",
                None,
                1_000,
                0,
            )
            .is_ok()
        );

        access_token.insert("aud".into(), Value::String("different-client".into()));
        assert!(
            validate_openid_claims(
                &access_token,
                "https://cognito.example/pool",
                &client_ids(&["client"]),
                "client",
                None,
                1_000,
                0,
            )
            .is_err()
        );
    }

    #[test]
    fn openid_accepts_every_configured_client_id_and_rejects_unknown_ones() {
        let access_token = |client: &str| {
            claims(&[
                ("iss", Value::String("https://cognito.example/pool".into())),
                ("sub", Value::String("user".into())),
                ("client_id", Value::String(client.into())),
                ("exp", Value::from(2_000_i64)),
            ])
        };
        let accepted = client_ids(&["client", "second-app-client"]);

        for client in ["client", "second-app-client"] {
            assert!(
                validate_openid_claims(
                    &access_token(client),
                    "https://cognito.example/pool",
                    &accepted,
                    "client",
                    None,
                    1_000,
                    0,
                )
                .is_ok()
            );
        }

        assert!(
            validate_openid_claims(
                &access_token("attacker-client"),
                "https://cognito.example/pool",
                &accepted,
                "client",
                None,
                1_000,
                0,
            )
            .is_err()
        );

        let mut authorized_party = access_token("client");
        authorized_party.insert("azp".into(), Value::String("second-app-client".into()));
        assert!(
            validate_openid_claims(
                &authorized_party,
                "https://cognito.example/pool",
                &accepted,
                "client",
                None,
                1_000,
                0,
            )
            .is_ok()
        );

        authorized_party.insert("azp".into(), Value::String("attacker-client".into()));
        assert!(
            validate_openid_claims(
                &authorized_party,
                "https://cognito.example/pool",
                &accepted,
                "client",
                None,
                1_000,
                0,
            )
            .is_err()
        );
    }

    #[test]
    fn openid_jwk_must_match_kid_algorithm_and_signature_use() {
        let key = rsa_jwk("RS256", "key-1");
        assert!(validate_jwk_for_header(&key, "key-1", Algorithm::RS256).is_ok());
        assert!(validate_jwk_for_header(&key, "key-1", Algorithm::RS512).is_err());
        assert!(validate_jwk_for_header(&key, "other-key", Algorithm::RS256).is_err());
        assert!(validate_jwk_for_header(&key, "key-1", Algorithm::HS256).is_err());

        let encryption_key: Jwk = serde_json::from_value(serde_json::json!({
            "kty": "RSA",
            "use": "enc",
            "alg": "RS256",
            "kid": "key-1",
            "n": "sXchvX3L7MdCKMImnlUiVDXQ4x_8OmtkPL3MyT9c6nr8YjC-rf1W_gKVVdQVrWjQxw",
            "e": "AQAB"
        }))
        .unwrap();
        assert!(validate_jwk_for_header(&encryption_key, "key-1", Algorithm::RS256).is_err());
    }

    #[test]
    fn openid_jwk_without_alg_accepts_compatible_header_algorithms_only() {
        let entra_key: Jwk = serde_json::from_value(serde_json::json!({
            "kty": "RSA",
            "use": "sig",
            "kid": "entra-key",
            "n": "sXchvX3L7MdCKMImnlUiVDXQ4x_8OmtkPL3MyT9c6nr8YjC-rf1W_gKVVdQVrWjQxw",
            "e": "AQAB"
        }))
        .expect("valid test JWK");
        assert!(validate_jwk_for_header(&entra_key, "entra-key", Algorithm::RS256).is_ok());
        assert!(validate_jwk_for_header(&entra_key, "entra-key", Algorithm::ES256).is_err());
        assert!(validate_jwk_for_header(&entra_key, "entra-key", Algorithm::HS256).is_err());
    }

    #[test]
    fn jwks_rejects_duplicate_key_ids_and_cache_never_extends_expiry() {
        let duplicate = JwkSet {
            keys: vec![rsa_jwk("RS256", "same"), rsa_jwk("RS256", "same")],
        };
        assert!(validate_jwks_set(&duplicate).is_err());
        assert!(cached_openid_is_current(1_001, 1_000));
        assert!(!cached_openid_is_current(1_000, 1_000));
        assert!(!cached_openid_is_current(999, 1_000));
    }

    #[test]
    fn retained_flow_ir_key_is_scoped_by_user_app_and_board() {
        let key = flow_ir_draft_store_key("user", "app", "board");
        assert_ne!(key, flow_ir_draft_store_key("other", "app", "board"));
        assert_ne!(key, flow_ir_draft_store_key("user", "other", "board"));
        assert_ne!(key, flow_ir_draft_store_key("user", "app", "other"));
        assert_eq!(key, flow_ir_draft_store_key(" user ", " app ", " board "));
    }

    #[test]
    fn board_mutation_lock_is_shared_across_authorized_users() {
        assert_eq!(
            board_mutation_lock_key("app", "board"),
            board_mutation_lock_key(" app ", " board ")
        );
        assert_ne!(
            board_mutation_lock_key("app", "board"),
            board_mutation_lock_key("other", "board")
        );
        assert_ne!(
            board_mutation_lock_key("app", "board"),
            board_mutation_lock_key("app", "other")
        );

        assert_eq!(
            board_mutation_lock_id("app", "board"),
            board_mutation_lock_id(" app ", " board ")
        );
        assert_ne!(
            board_mutation_lock_id("app", "board"),
            board_mutation_lock_id("other", "board")
        );
        assert_ne!(
            board_mutation_lock_id("app", "board"),
            board_mutation_lock_id("app", "other")
        );
    }

    #[test]
    fn mutation_lock_namespaces_do_not_overlap() {
        assert_ne!(
            board_mutation_lock_id("user", "challenge"),
            course_attempt_lock_id("user")
        );
        assert_ne!(
            course_attempt_lock_id("user"),
            course_attempt_lock_id("other")
        );
    }

    #[test]
    fn mutation_lock_sql_uses_portable_row_writes() {
        assert!(ENSURE_MUTATION_LOCK_SQL.contains("ON CONFLICT"));
        assert!(ACQUIRE_MUTATION_LOCK_SQL.starts_with("UPDATE"));
        assert!(!ENSURE_MUTATION_LOCK_SQL.contains("pg_advisory"));
        assert!(!ACQUIRE_MUTATION_LOCK_SQL.contains("pg_advisory"));
    }
}
