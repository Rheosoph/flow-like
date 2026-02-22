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
    /// API key with key_id and app_id
    ApiKey { key_id: String, app_id: String },
    /// Executor JWT with sub, app_id, run_id
    Executor {
        sub: String,
        app_id: String,
        run_id: String,
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
}

impl State {
    pub async fn new(
        catalog: Arc<Vec<Arc<dyn NodeLogic>>>,
        content_bucket: Arc<FlowLikeStore>,
        cdn_bucket: Arc<FlowLikeStore>,
        meta_bucket: Arc<FlowLikeStore>,
    ) -> Self {
        let platform_config: Hub =
            serde_json::from_str(CONFIG).expect("Failed to parse config file");

        let jwks = flow_like_types::json::from_str::<JwkSet>(JWKS).expect("Failed to parse JWKS");

        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let mut opt = ConnectOptions::new(db_url.to_owned());
        let client: Client<HttpConnector, Body> =
            hyper_util::client::legacy::Client::<(), ()>::builder(TokioExecutor::new())
                .build(HttpConnector::new());
        opt.max_connections(10)
            .min_connections(1)
            .connect_timeout(Duration::from_secs(8))
            .sqlx_logging(platform_config.environment == Environment::Development);

        let db = Database::connect(opt)
            .await
            .expect("Failed to connect to database");

        let stripe_client = if platform_config.features.premium {
            let stripe_key =
                std::env::var("STRIPE_SECRET_KEY").expect("STRIPE_SECRET_KEY must be set");
            let stripe_client = stripe::Client::new(stripe_key);
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
            .time_to_live(Duration::from_secs(30 * 60)) // 30 minutes
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
            CompilationDispatcher::new(compilation_config, content_bucket.clone(), meta_bucket.clone()).await,
        );

        // Initialize WASM registry if enabled (uses PostgreSQL + CDN)
        let wasm_registry = if platform_config.features.wasm_registry {
            let cdn_base_url = platform_config.cdn.clone();
            let registry = ServerRegistry::new(
                db.clone(),
                content_bucket.clone(),
                meta_bucket.clone(),
                cdn_base_url,
            )
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
        }
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
