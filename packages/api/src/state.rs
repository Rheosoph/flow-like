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
    DecodingKey, Validation, decode,
    jwk::{AlgorithmParameters, JwkSet},
};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection,
    DatabaseTransaction, IsolationLevel, Statement, TransactionTrait,
};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::Duration,
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

/// Cached auth result for JWT/PAT/API key
#[derive(Clone, Debug)]
pub enum CachedAuth {
    /// OpenID user with sub
    OpenID { sub: String },
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

pub struct State {
    pub platform_config: Hub,
    pub db: DatabaseConnection,
    pub jwks: JwkSet,
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

        let jwks = flow_like_types::json::from_str::<JwkSet>(JWKS).expect("Failed to parse JWKS");

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

        let db_url = secrets
            .get_secret_string(&SecretRef::new("DATABASE_URL"))
            .await
            .expect("DATABASE_URL must be set");
        let mut opt = ConnectOptions::new(db_url.expose_secret().to_owned());
        let client: Client<HttpConnector, Body> =
            hyper_util::client::legacy::Client::<(), ()>::builder(TokioExecutor::new())
                .build(HttpConnector::new());
        opt.max_connections(10)
            .min_connections(1)
            .connect_timeout(Duration::from_secs(8))
            .connect_lazy(true)
            .sqlx_logging(platform_config.environment == Environment::Development);

        let db = Database::connect(opt)
            .await
            .expect("Failed to connect to database");

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

        Self {
            platform_config,
            db,
            client,
            jwks,
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

    pub fn validate_token(&self, token: &str) -> Result<HashMap<String, Value>> {
        let header = jsonwebtoken::decode_header(token)?;
        let Some(kid) = header.kid else {
            return Err(flow_like_types::anyhow!("Missing kid in token header"));
        };
        let Some(jwk) = self.jwks.find(&kid) else {
            return Err(flow_like_types::anyhow!("JWK not found for kid: {}", kid));
        };
        let alg = decoding_key_for_algorithm(&jwk.algorithm)?;
        let mut validation = Validation::new(header.alg);
        validation.validate_aud = false;
        let decoded = decode::<HashMap<String, Value>>(token, &alg, &validation)?;
        let claims = decoded.claims;
        Ok(claims)
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
        board_mutation_lock_key, course_attempt_lock_id, flow_ir_draft_store_key,
    };

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
