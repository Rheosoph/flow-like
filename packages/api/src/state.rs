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
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::compilation::{CompilationDispatchConfig, CompilationDispatcher};
use crate::credentials::{CredentialsAccess, RuntimeCredentials};
use crate::entity::role;
use crate::execution::{DispatchConfig, Dispatcher};
use crate::mail::{DynMailClient, create_mail_client};
use crate::permission::wasm_package_permission::WasmPackagePermission;
use crate::routes::registry::ServerRegistry;

pub type AppState = Arc<State>;

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
        skip(self, state),
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
        skip(self, state),
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
        use crate::entity::membership;
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
