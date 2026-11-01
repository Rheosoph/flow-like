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
use flow_like::utils::compression::ConditionalRead;
use flow_like_secrets::{
    EnvProviderConfig, ExposeSecret, ProviderConfig, SecretRef, SecretStore, SecretStoreConfig,
};
use flow_like_types::bail;
use flow_like_types::dispatch::WasmPackageRef;
use flow_like_types::{Result, Value, anyhow};
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
use sea_orm::{ConnectOptions, Database, DatabaseConnection, DatabaseTransaction, IsolationLevel};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tracing::Instrument;

pub use crate::db::DbDialect;

use crate::channel::ChannelIssuer;
use crate::compilation::{CompilationDispatchConfig, CompilationDispatcher};
use crate::credentials::{CredentialsAccess, RuntimeCredentials};
use crate::db::lease::MutationLease;
use crate::entity::role;
use crate::error::ApiError;
use crate::execution::{DispatchConfig, Dispatcher};
use crate::mail::{DynMailClient, create_mail_client};
use crate::permission::wasm_package_permission::WasmPackagePermission;
use crate::realtime_ice::RealtimeIceService;
use crate::routes::registry::ServerRegistry;
use crate::runtime_config::{ConfigSource, OAuthProviderConfig, OpenIdValidationOverrides};

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

/// Cache key for one board revision. Versioned snapshots and the floating draft are different
/// objects with different ETags, so they never share an entry.
fn board_cache_key(app_id: &str, board_id: &str, version: Option<(u32, u32, u32)>) -> String {
    match version {
        Some((maj, min, pat)) => format!(
            "{}\u{1f}{}\u{1f}{maj}.{min}.{pat}",
            app_id.trim(),
            board_id.trim()
        ),
        None => format!("{}\u{1f}{}\u{1f}", app_id.trim(), board_id.trim()),
    }
}

/// Boards are large (a few MB hydrated); bound the count rather than trying to weigh them.
const BOARD_CACHE_MAX_ENTRIES: u64 = 64;
/// Total nodes retained across all indexed segment bases (see `State::board_segment_bases`).
const BOARD_SEGMENT_BASE_MAX_NODES: u64 = 200_000;

/// Account for JSON allocations and keys, including empty search responses.
fn response_cache_weight(key: &String, value: &Value) -> u32 {
    fn heap_bytes(value: &Value) -> usize {
        match value {
            Value::String(text) => text.capacity(),
            Value::Array(items) => items.iter().fold(
                items
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Value>()),
                |bytes, item| bytes.saturating_add(heap_bytes(item)),
            ),
            Value::Object(fields) => fields.iter().fold(0usize, |bytes, (key, value)| {
                bytes
                    .saturating_add(std::mem::size_of::<(String, Value)>())
                    .saturating_add(3 * std::mem::size_of::<usize>())
                    .saturating_add(key.capacity())
                    .saturating_add(heap_bytes(value))
            }),
            _ => 0,
        }
    }
    let bytes = key
        .capacity()
        .saturating_add(std::mem::size_of::<(String, Value)>())
        .saturating_add(128) // Cache bookkeeping and allocation overhead.
        .saturating_add(heap_bytes(value));
    u32::try_from(bytes).unwrap_or(u32::MAX)
}

/// Key of a segment in `State::board_segment_bases`.
pub(crate) fn segment_base_key(app_id: &str, board_id: &str, token: &str) -> String {
    format!("{}\u{1f}{}\u{1f}{token}", app_id.trim(), board_id.trim())
}

/// A hydrated board together with the storage identity it is known to reproduce.
///
/// `board` is what `Board::load` produced for the object that carried `e_tag`: `node_updates`
/// and `cleanup` already applied, per-app WASM metadata and secret filtering **not** applied. It
/// is shared read-only; every consumer clones before mutating.
pub struct CachedBoard {
    pub e_tag: String,
    pub board: Arc<Board>,
    /// When storage last confirmed `e_tag`.
    verified_at: parking_lot::Mutex<Instant>,
    /// Start of the storage read that confirmed this entry. Waiting readers can
    /// share it only if it started after their own request.
    verification_started_at: parking_lot::Mutex<Option<Instant>>,
    /// Pages of a pinned version, as immutable as the version that owns them.
    version_pages: parking_lot::Mutex<HashMap<String, Arc<flow_like::a2ui::Page>>>,
    prerun_derivation: parking_lot::Mutex<Option<CachedPrerunDerivation>>,
    /// Exact-ETag `.board` snapshot of a draft (`ensure_draft_board_snapshot`).
    pub draft_snapshot: TrustedUntil,
    /// Board-only prerun authority of a draft (`ensure_draft_prerun_manifest`).
    pub draft_authority: TrustedUntil,
}

struct CachedPrerunDerivation {
    source: flow_like::flow::compiled::prerun::PageExecutionRevisionSource,
    pages: HashMap<
        String,
        (
            flow_like::flow::compiled::prerun::PrerunPageExecution,
            String,
        ),
    >,
}

const CACHED_PAGE_REVISIONS_PER_BOARD: usize = 64;

/// A pinned version's content never changes; revalidating it only bounds how long a deleted
/// board keeps resolving on this instance.
const PINNED_BOARD_REVALIDATE_AFTER: Duration = Duration::from_secs(5 * 60);

impl CachedBoard {
    pub(crate) fn new(e_tag: String, board: Board) -> Self {
        Self {
            e_tag,
            board: Arc::new(board),
            verified_at: parking_lot::Mutex::new(Instant::now()),
            verification_started_at: parking_lot::Mutex::new(None),
            version_pages: parking_lot::Mutex::new(HashMap::new()),
            prerun_derivation: parking_lot::Mutex::new(None),
            draft_snapshot: TrustedUntil::default(),
            draft_authority: TrustedUntil::default(),
        }
    }

    fn verified_recently(&self) -> bool {
        self.verified_at.lock().elapsed() < PINNED_BOARD_REVALIDATE_AFTER
    }

    fn mark_verified(&self, started_at: Instant) {
        *self.verified_at.lock() = Instant::now();
        *self.verification_started_at.lock() = Some(started_at);
    }

    fn verified_since(&self, requested_at: Instant) -> bool {
        self.verification_started_at
            .lock()
            .is_some_and(|started_at| started_at > requested_at)
    }

    /// A page of this pinned version read earlier. Never use it for a draft entry: draft pages
    /// are saved independently of the board's ETag.
    pub fn version_page(&self, page_id: &str) -> Option<flow_like::a2ui::Page> {
        self.version_pages
            .lock()
            .get(page_id)
            .map(|page| (**page).clone())
    }

    pub fn remember_version_page(&self, page: &flow_like::a2ui::Page) {
        self.version_pages
            .lock()
            .insert(page.id.clone(), Arc::new(page.clone()));
    }

    fn with_prerun_derivation<T>(&self, apply: impl FnOnce(&mut CachedPrerunDerivation) -> T) -> T {
        let mut cached = self.prerun_derivation.lock();
        if cached
            .as_ref()
            .is_none_or(|cached| !cached.source.matches(&self.board))
        {
            *cached = Some(CachedPrerunDerivation {
                source: flow_like::flow::compiled::prerun::PageExecutionRevisionSource::new(
                    &self.board,
                ),
                pages: HashMap::new(),
            });
        }
        apply(cached.as_mut().expect("prerun derivation was initialized"))
    }

    pub(crate) fn prerun_manifest(&self) -> Arc<flow_like::flow::compiled::PrerunManifest> {
        self.with_prerun_derivation(|cached| cached.source.manifest())
    }

    /// Memoize only derived data. Callers still revalidate storage, Event
    /// permissions and the separately saved draft Page before reaching here.
    pub(crate) fn page_execution_revision(
        &self,
        execution: &flow_like::flow::compiled::prerun::PrerunPageExecution,
    ) -> flow_like_types::Result<String> {
        self.with_prerun_derivation(|cached| {
            if let Some((previous, revision)) = cached.pages.get(&execution.page_id)
                && previous == execution
            {
                return Ok(revision.clone());
            }
            let revision = cached.source.revision(&self.board, execution)?;
            if cached.pages.len() >= CACHED_PAGE_REVISIONS_PER_BOARD
                && !cached.pages.contains_key(&execution.page_id)
            {
                cached.pages.clear();
            }
            cached.pages.insert(
                execution.page_id.clone(),
                (execution.clone(), revision.clone()),
            );
            Ok(revision)
        })
    }
}

fn recently_verified_pinned(
    board_cache: &moka::sync::Cache<String, Arc<CachedBoard>>,
    cache_key: &str,
    version: Option<(u32, u32, u32)>,
) -> Option<Arc<CachedBoard>> {
    version?;
    board_cache
        .get(cache_key)
        .filter(|entry| entry.verified_recently())
}

fn verified_board_after_wait(
    board_cache: &moka::sync::Cache<String, Arc<CachedBoard>>,
    cache_key: &str,
    version: Option<(u32, u32, u32)>,
    requested_at: Instant,
) -> Option<Arc<CachedBoard>> {
    board_cache.get(cache_key).filter(|entry| {
        (version.is_some() && entry.verified_recently()) || entry.verified_since(requested_at)
    })
}

/// Until when an object an entry depends on is known to exist without asking storage.
#[derive(Default)]
pub struct TrustedUntil(parking_lot::Mutex<Option<std::time::SystemTime>>);

impl TrustedUntil {
    pub fn holds(&self) -> bool {
        self.0
            .lock()
            .is_some_and(|until| std::time::SystemTime::now() < until)
    }

    pub fn extend(&self, until: std::time::SystemTime) {
        let mut current = self.0.lock();
        *current = Some(current.map_or(until, |current| current.max(until)));
    }
}

/// A registry of per-key process-local mutexes that forget themselves once nobody holds them.
fn keyed_local_lock(
    locks: &parking_lot::Mutex<HashMap<String, Weak<flow_like_types::tokio::sync::Mutex<()>>>>,
    key: String,
) -> Arc<flow_like_types::tokio::sync::Mutex<()>> {
    let mut locks = locks.lock();
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    // Weak entries make cleanup safe: a mutex can disappear only when no holder or waiter has
    // an Arc, so eviction can never create a second live lock for the same key.
    if locks.len() >= 4_096 {
        locks.retain(|_, lock| lock.strong_count() > 0);
    }
    let lock = Arc::new(flow_like_types::tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

fn scoped_credential_minimum_lifetime(
    mode: &CredentialsAccess,
) -> flow_like_types::Result<chrono::Duration> {
    let minimum_lifetime = native_async_credential_lifetime(
        mode,
        std::env::var("ASYNC_EXECUTION_BACKEND").ok().as_deref(),
        std::env::var("LAMBDA_ASYNC_EXECUTOR_FUNCTION")
            .ok()
            .as_deref(),
    );
    if std::env::var("EXECUTION_ISOLATION_MODE").as_deref() != Ok("per_run")
        || !matches!(
            mode,
            CredentialsAccess::ServerExecute | CredentialsAccess::ShadowExecute
        )
    {
        return Ok(minimum_lifetime);
    }
    let timeout = std::env::var("EXECUTION_TIMEOUT_SECONDS")
        .or_else(|_| std::env::var("EXECUTOR_TIMEOUT_SECS"))
        .unwrap_or_else(|_| "3600".into());
    let queue_wait =
        std::env::var("EXECUTION_QUEUE_MAX_WAIT_SECONDS").unwrap_or_else(|_| "300".into());
    let margin =
        std::env::var("EXECUTION_CREDENTIAL_MARGIN_SECONDS").unwrap_or_else(|_| "120".into());
    let isolated_lifetime = execution_credential_lifetime(
        &timeout,
        &queue_wait,
        &margin,
        crate::execution::queue::supervision_grace_seconds()?,
    )?;
    Ok(minimum_lifetime.max(isolated_lifetime))
}

fn native_async_credential_lifetime(
    mode: &CredentialsAccess,
    async_backend: Option<&str>,
    native_function: Option<&str>,
) -> chrono::Duration {
    let native_dispatch = async_backend.is_some_and(|backend| {
        backend.eq_ignore_ascii_case("lambda_invoke") || backend.eq_ignore_ascii_case("lambda_sdk")
    }) && native_function.is_some_and(|function| !function.trim().is_empty());
    if native_dispatch
        && matches!(
            mode,
            CredentialsAccess::ServerExecute
                | CredentialsAccess::ShadowExecute
                | CredentialsAccess::InvokeNone
                | CredentialsAccess::InvokeRead
                | CredentialsAccess::InvokeWrite
        )
    {
        // Native async deployment caps event age and hard execution at 900s
        // each. Keep another 60s for dispatch preparation and clock skew.
        // Both cached grants and freshly minted sessions must cover this window.
        chrono::Duration::seconds(900 + 900 + 60)
    } else {
        chrono::Duration::seconds(120)
    }
}

fn execution_credential_lifetime(
    timeout: &str,
    queue_wait: &str,
    margin: &str,
    supervision_grace: u64,
) -> flow_like_types::Result<chrono::Duration> {
    let bounded = |value: &str, name: &str, maximum: i64| -> flow_like_types::Result<i64> {
        value
            .parse::<i64>()
            .ok()
            .filter(|value| (1..=maximum).contains(value))
            .ok_or_else(|| flow_like_types::anyhow!("{name} must be 1..{maximum}"))
    };
    Ok(chrono::Duration::seconds(
        bounded(timeout, "EXECUTION_TIMEOUT_SECONDS", 86400)?
            + bounded(queue_wait, "EXECUTION_QUEUE_MAX_WAIT_SECONDS", 86400)?
            + bounded(margin, "EXECUTION_CREDENTIAL_MARGIN_SECONDS", 3600)?
            + i64::try_from(supervision_grace)
                .map_err(|_| flow_like_types::anyhow!("Invalid supervision grace"))?,
    ))
}

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

/// Holds both serialization layers for a canonical board mutation: the process-local mutex and
/// the committed `MutationLock` lease every API replica competes for.
///
/// No database transaction stays open while the guard is held; callers do their row writes
/// through [`State::transaction`] after the storage work. Dropping the guard hands the lease
/// back in the background; [`Self::release`] waits for it so the next writer does not spin.
pub(crate) struct BoardMutationGuard {
    lease: MutationLease,
}

impl BoardMutationGuard {
    /// Add another canonical board to this guard under the same lease owner.
    ///
    /// Page mutations first lock the globally unique page id, then add its owning board, so
    /// concurrent cross-board page-id claims serialize on the page id row.
    pub(crate) async fn acquire_additional_board(
        &mut self,
        state: &State,
        app_id: &str,
        board_id: &str,
    ) -> std::result::Result<(), ApiError> {
        let local = state
            .board_mutation_lock(app_id, board_id)
            .lock_owned()
            .await;
        self.lease
            .claim_additional(state, board_mutation_lock_id(app_id, board_id), local)
            .await
    }

    /// Fail the request when the lease is no longer provably ours.
    ///
    /// Call immediately before every canonical board or app write made under this guard. The
    /// heartbeat can only report a lapsed lease, never renew one retroactively: without this
    /// check a writer whose lease expired mid-request goes on to a full-object PUT that a second
    /// replica is already making, and the lost update is recorded as nothing but a log line.
    pub(crate) fn ensure_held(&self) -> std::result::Result<(), ApiError> {
        self.lease.ensure_held()
    }

    pub(crate) async fn release(self) {
        self.lease.release().await
    }
}

const JWKS_REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(30);
const JWKS_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const JWKS_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const JWKS_MAX_KEYS: usize = 64;

/// Cached JWT or API-key authentication. Revocable PATs always use the database.
#[derive(Clone, Debug)]
pub enum CachedAuth {
    /// OpenID user with sub and the token's own expiration. Cache hits must
    /// never extend an access token beyond this timestamp.
    OpenID { sub: String, exp: i64 },
    /// API key with key_id, app_id, and the creator user that owns tier/billing.
    ApiKey {
        key_id: String,
        app_id: String,
        creator_user_id: Option<String>,
    },
    /// Executor JWT: the principal exactly as the fresh path verified it, with
    /// every signed claim it carries, and the token's expiry.
    Executor {
        executor: crate::middleware::jwt::ExecutorUser,
        exp: i64,
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
    require_access_token_use: bool,
}

/// Statement pinning a pooled Postgres session to UTC.
///
/// Every timestamp column is `timestamptz`, so `date_trunc`/`to_char` and any
/// offset-less literal resolve against the session `TimeZone`. The Rust half of
/// the analytics endpoints is UTC by construction, so a non-UTC default would
/// silently bucket rows onto the wrong calendar day rather than fail. The SQL
/// in `utils::stats_period` and the telemetry timeseries pins UTC per
/// expression; this pins it per connection so nothing new can drift.
const PIN_SESSION_TIME_ZONE_SQL: &str = "SET TIME ZONE 'UTC'";

/// Runs [`PIN_SESSION_TIME_ZONE_SQL`] on every newly established pooled
/// connection.
///
/// sea-orm's own `ConnectOptions::after_connect` fires once against the whole
/// pool, which would leave later connections unpinned; sqlx's pool-level hook
/// is the per-connection one. Non-Postgres backends ignore this entirely.
fn pin_session_time_zone_to_utc(opt: &mut ConnectOptions) {
    opt.map_sqlx_postgres_pool_opts(|pool_opts| {
        pool_opts.after_connect(|conn, _meta| {
            Box::pin(async move {
                sea_orm::sqlx::Executor::execute(
                    conn,
                    sea_orm::sqlx::AssertSqlSafe(PIN_SESSION_TIME_ZONE_SQL.to_owned()),
                )
                .await
                .map(|_| ())
            })
        })
    });
}

#[derive(Debug)]
pub(crate) struct ValidatedOpenIdToken {
    pub(crate) claims: HashMap<String, Value>,
    pub(crate) expires_at: i64,
}

pub struct State {
    pub platform_config: Hub,
    pub(crate) oauth_providers: HashMap<String, OAuthProviderConfig>,
    openid_validation_overrides: OpenIdValidationOverrides,
    pub db: DatabaseConnection,
    pub db_dialect: DbDialect,
    jwks: flow_like_types::tokio::sync::RwLock<JwkSet>,
    jwks_refresh: flow_like_types::tokio::sync::Mutex<JwksRefreshState>,
    pub client: Client<HttpConnector, Body>,
    pub(crate) oidc_jwks_client: reqwest::Client,
    pub(crate) oidc_user_info_client: reqwest::Client,
    pub stripe_client: Option<stripe::Client>,
    pub payments_gateway:
        flow_like_types::tokio::sync::OnceCell<Arc<crate::stripe_connect::HttpStripeGateway>>,
    pub legacy_payments_gateway:
        flow_like_types::tokio::sync::OnceCell<Arc<crate::stripe_connect::HttpStripeGateway>>,
    pub mail_client: Option<DynMailClient>,
    pub(crate) mail_automation: crate::runtime_config::mail::MailAutomationConfig,
    pub(crate) automation_mail_client: Option<DynMailClient>,
    #[cfg(feature = "aws")]
    pub aws_client: Arc<SdkConfig>,
    #[cfg(feature = "aws")]
    pub(crate) scoped_sts_client: std::sync::OnceLock<aws_sdk_sts::Client>,
    pub catalog: Arc<Vec<Arc<dyn NodeLogic>>>,
    pub registry: Arc<FlowNodeRegistryInner>,
    pub provider: Arc<ModelProviderConfiguration>,
    pub dispatcher: Arc<Dispatcher>,
    pub compilation_dispatcher: Arc<CompilationDispatcher>,
    /// Mints run ⇄ client channel credentials (`CHANNEL_TRANSPORT`).
    pub channels: Arc<ChannelIssuer>,
    /// Mints short-lived STUN and TURN credentials for realtime board clients.
    pub realtime_ice: RealtimeIceService,
    pub permission_cache: moka::sync::Cache<String, Arc<role::Model>>,
    pub credentials_cache: moka::sync::Cache<String, Arc<RuntimeCredentials>>,
    /// Collapse simultaneous credential refreshes for one subject, app and grant.
    credential_refresh_locks:
        parking_lot::Mutex<HashMap<String, Weak<flow_like_types::tokio::sync::Mutex<()>>>>,
    pub state_cache: moka::sync::Cache<String, Arc<FlowLikeState>>,
    /// User+app+board-scoped typed workflow drafts retained across stateless chat HTTP requests.
    /// Each store is internally bounded; the outer TTL/cap keeps abandoned board sessions finite.
    pub flow_ir_draft_stores:
        moka::sync::Cache<String, Arc<flow_like::flow::copilot::FlowIrDraftStore>>,
    /// Process-local half of canonical app+board serialization. `board_mutation_guard` pairs each
    /// mutex with a leased database lock row so API replicas enter the same mutation lane.
    board_mutation_locks:
        parking_lot::Mutex<HashMap<String, Weak<flow_like_types::tokio::sync::Mutex<()>>>>,
    /// Hydrated boards pinned to the object identity they were loaded from. See
    /// [`State::master_board`] for the validation contract.
    board_cache: moka::sync::Cache<String, Arc<CachedBoard>>,
    /// Tokenised board parts per (board revision, WASM catalog); answers sync polls without
    /// re-serialising. Keyed by the ETag, so it never needs explicit invalidation.
    pub board_sync_cache:
        moka::sync::Cache<String, Arc<flow_like::flow::board::sync::BoardSyncSnapshot>>,
    /// The most recent snapshot built for a board on this instance, keyed like `board_cache`.
    /// Only ever the *starting point* of the next build (`from_board_incremental` reuses tokens
    /// by payload comparison), so a stale or foreign entry costs hashing time, never correctness.
    pub board_snapshot_heads:
        moka::sync::Cache<String, Arc<flow_like::flow::board::sync::BoardSyncSnapshot>>,
    /// Segments of recently built snapshots by `(app, board, token)`, so a client's segment token
    /// can be resolved to the payload it denotes and answered with a node-level patch. Fed by every
    /// snapshot build on this instance; a miss ships the whole segment. Scoped by app and board so
    /// a token can never address another tenant's data even in theory.
    pub board_segment_bases:
        moka::sync::Cache<String, Arc<flow_like::flow::board::sync::SyncSegment>>,
    /// Per-app WASM node catalog; see `app_wasm_nodes_cached`.
    pub app_wasm_nodes_cache:
        moka::sync::Cache<String, Arc<crate::routes::app::wasm_catalog::AppWasmNodes>>,
    /// Serialises concurrent cold loads of one board on this instance so the second reader waits
    /// for the first hydration instead of repeating it. Distinct from `board_mutation_locks`,
    /// which a writer already holds while it calls `master_board`.
    board_load_locks:
        parking_lot::Mutex<HashMap<String, Weak<flow_like_types::tokio::sync::Mutex<()>>>>,
    pub content_bucket: Arc<FlowLikeStore>,
    pub cdn_bucket: Arc<FlowLikeStore>,
    pub meta_bucket: Arc<FlowLikeStore>,
    /// Which concrete bucket, region and account each of the three stores above points
    /// at. The stores themselves cannot answer that, and every provider metrics API asks.
    pub storage_identity: crate::storage_identity::StorageIdentity,
    /// The write-once bucket, when `LOCKED_BUCKET` (or a provider variant) is set.
    pub locked_storage: Option<Arc<crate::locked_storage::LockedStorage>>,
    /// Positive cache for the pre-dispatch compiled-artifact check, keyed by
    /// (app, board, version|etag, registry fingerprint). Entries are
    /// content-addressed. The value is how long a draft artifact under
    /// lifecycle-managed `tmp/` is trusted to exist; `None` for immutable
    /// version artifacts.
    pub compiled_artifact_cache: moka::sync::Cache<String, Option<std::time::SystemTime>>,
    pub(crate) compiled_widget_artifacts: crate::execution::widget_artifacts::WidgetArtifactCache,
    /// Registries compiled artifacts are fingerprinted against, one per
    /// resolved WASM package set and current node metadata: the built-in
    /// catalog extended with the packages' manifest `Node`s. See
    /// [`State::artifact_registry`].
    pub artifact_registries: moka::sync::Cache<String, Arc<FlowNodeRegistryInner>>,
    /// Prerun manifests keyed by (app, board, version|etag). Content-addressed
    /// like `compiled_artifact_cache`, so entries never go stale. Keys start with
    /// `{app_id}:{board_id}:` so deleting a board can drop its entries by prefix.
    pub prerun_manifest_cache:
        moka::sync::Cache<String, Arc<flow_like::flow::compiled::PrerunManifest>>,
    pub response_cache: moka::sync::Cache<String, Value>,
    pub(crate) execution_usage_cache: crate::routes::usage::cache::UsageCache,
    /// WASM package permission cache: "wasm:{user_id}:{package_id}" -> the
    /// caller's grant. "No access" is never stored.
    pub wasm_permission_cache: moka::sync::Cache<String, WasmPackagePermission>,
    /// Auth token cache: token_hash -> CachedAuth
    /// Short TTL (240s) to balance security vs performance
    pub auth_cache: moka::sync::Cache<String, CachedAuth>,
    /// WASM package registry (optional)
    pub wasm_registry: Option<Arc<ServerRegistry>>,
    /// Sink scheduler for cron events (AWS EventBridge, K8s CronJobs, or in-memory)
    pub sink_scheduler: Option<Arc<dyn flow_like_sinks::SchedulerBackend>>,
    /// Key/value cache backend (`CACHE_BACKEND`) behind the app-facing cache routes and
    /// the platform partition (`cache.platform()`) the API uses for cross-replica
    /// coordination such as trigger idempotency.
    ///
    /// Initialized on first use and then held for the life of the process: cache reads
    /// are far more frequent than execution-state reads, and rebuilding a Redis
    /// connection on every call would dominate the latency the cache exists to avoid.
    pub cache: crate::cache::CacheBackendHandle,
    /// Execution state backend (`EXECUTION_STATE_BACKEND`), built on first use and shared
    /// by every executor callback and poll. A failed build is not memoized.
    pub(crate) execution_state_store: flow_like_types::tokio::sync::OnceCell<
        Arc<dyn crate::execution::state::ExecutionStateStore>,
    >,
    /// Secret store for accessing secrets from various providers (env, AWS Parameter Store, etc.)
    pub secrets: Arc<SecretStore>,
    /// Encryption key for token encryption (derived from SINK_TOKEN_ENCRYPTION_KEY)
    pub encryption_key: [u8; 32],
    /// `AUDIT_KID`, the key id the worker signs under.
    pub audit_kid: Option<String>,
    /// HMAC secret for signing/verifying sink trigger JWTs
    pub sink_secret: Option<String>,
    /// Dedicated bearer token accepted only by the internal maintenance API.
    ///
    /// This is intentionally separate from user auth, sink auth, and
    /// `BACKEND_KEY`, so a maintenance runner cannot mint broader credentials.
    pub maintenance_token: Option<String>,
    /// Per-replica fast path for sink trigger idempotency, keyed by the
    /// `Idempotency-Key` header. The cross-replica reservation lives in the platform
    /// cache; this only spares a repeat on the same replica the round trip.
    pub trigger_idempotency:
        moka::sync::Cache<String, crate::routes::sink::trigger::ServiceTriggerResponse>,
}

macro_rules! init_span {
    ($name:literal) => {
        tracing::info_span!(target: "flow_like::observability", $name)
    };
    (parent: $parent:expr, $name:literal) => {
        tracing::info_span!(target: "flow_like::observability", parent: $parent, $name)
    };
}

type CatalogNodes = Arc<Vec<Arc<dyn NodeLogic>>>;

/// The node catalog and its registry, built on a blocking thread. Both walk every node
/// definition: CPU work that [`State::new_with_secrets`] overlaps with its network round trips.
pub struct NodeCatalog(
    flow_like_types::tokio::task::JoinHandle<(CatalogNodes, FlowNodeRegistryInner)>,
);

impl NodeCatalog {
    pub fn spawn(build: impl FnOnce() -> Vec<Arc<dyn NodeLogic>> + Send + 'static) -> Self {
        Self::spawn_with(|parent| {
            Arc::new(init_span!(parent: parent, "api.init.catalog").in_scope(build))
        })
    }

    fn spawn_with(catalog: impl FnOnce(&tracing::Span) -> CatalogNodes + Send + 'static) -> Self {
        let parent = tracing::Span::current();
        Self(flow_like_types::tokio::task::spawn_blocking(move || {
            let catalog = catalog(&parent);
            let registry = init_span!(parent: &parent, "api.init.registry")
                .in_scope(|| FlowNodeRegistryInner::prepare(&catalog));
            (catalog, registry)
        }))
    }

    async fn ready(self) -> (CatalogNodes, FlowNodeRegistryInner) {
        match self.0.await {
            Ok(built) => built,
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Err(error) => panic!("building the node catalog was cancelled: {error}"),
        }
    }
}

impl From<CatalogNodes> for NodeCatalog {
    fn from(catalog: CatalogNodes) -> Self {
        Self::spawn_with(move |_| catalog)
    }
}

struct RuntimeConfig {
    platform: Hub,
    mail_automation: crate::runtime_config::mail::MailAutomationConfig,
    oauth_providers: HashMap<String, OAuthProviderConfig>,
    openid: OpenIdValidationOverrides,
}

/// Selects a single complete document before any service initializes. Loader errors carry no
/// document, filesystem path, or provider details.
async fn load_runtime_config(secrets: &SecretStore) -> RuntimeConfig {
    let effective_config = ConfigSource::from_env()
        .unwrap_or_else(|error| panic!("{error}"))
        .load(secrets)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let mail_automation = crate::runtime_config::mail::MailAutomationConfig::load_or_disabled(
        effective_config.mail_automation,
        secrets,
    )
    .await;
    let mut platform = effective_config.hub;
    crate::runtime_config::configure_mail_capability(&mut platform, mail_automation.enabled);
    if platform.payments.creation_enabled() {
        platform
            .payments
            .validate()
            .unwrap_or_else(|error| panic!("Payment creation configuration is invalid: {error}"));
    }
    if platform
        .authentication
        .as_ref()
        .is_some_and(|authentication| authentication.variant.eq_ignore_ascii_case("openid"))
    {
        openid_validation_settings_for_hub(&platform, &effective_config.openid).unwrap_or_else(
            |_| panic!("OpenID validation configuration must be complete and exact"),
        );
    }
    RuntimeConfig {
        platform,
        mail_automation,
        oauth_providers: effective_config.oauth_providers,
        openid: effective_config.openid,
    }
}

async fn optional_secret(secrets: &SecretStore, name: &str) -> Option<String> {
    secrets
        .get_secret_string(&SecretRef::new(name))
        .await
        .ok()
        .map(|secret| secret.expose_secret().to_string())
}

struct ServiceSecrets {
    sink_secret: Option<String>,
    maintenance_token: Option<String>,
    encryption_key: [u8; 32],
}

async fn load_service_secrets(secrets: &SecretStore) -> ServiceSecrets {
    let sink_secret = optional_secret(secrets, "SINK_SECRET").await;

    if sink_secret.is_none() {
        tracing::warn!("SINK_SECRET not configured — sink trigger endpoints will be unavailable");
    }

    let maintenance_token = optional_secret(secrets, "MAINTENANCE_TOKEN")
        .await
        .map(|token| token.trim().to_string())
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

    ServiceSecrets {
        sink_secret,
        maintenance_token,
        encryption_key,
    }
}

/// Installs the process-wide backend JWT and audit entry keys; returns `AUDIT_KID`.
async fn install_signing_keys(secrets: &SecretStore, platform_config: &Hub) -> Option<String> {
    let backend_key = secrets
        .get_secret_string(&SecretRef::new("BACKEND_KEY"))
        .await
        .inspect_err(|e| tracing::error!("Failed to fetch BACKEND_KEY from secret store: {e}"))
        .ok()
        .map(|s| s.expose_secret().to_string());
    tracing::info!(
        "BACKEND_KEY resolved: {}",
        if backend_key.is_some() { "yes" } else { "no" }
    );

    crate::backend_jwt::init(
        backend_key.as_deref(),
        optional_secret(secrets, "BACKEND_PUB").await.as_deref(),
        optional_secret(secrets, "BACKEND_KID").await,
    );
    let entry_key_source = crate::audit::keys::init_entry_key(
        optional_secret(secrets, "AUDIT_ENTRY_KEY").await.as_deref(),
        backend_key.as_deref(),
    )
    .expect("AUDIT_ENTRY_KEY must be base64 of 32 bytes");
    crate::audit::keys::init_previous_entry_key(
        optional_secret(secrets, "AUDIT_ENTRY_KEY_PREVIOUS")
            .await
            .as_deref(),
        optional_secret(secrets, "BACKEND_KEY_PREVIOUS")
            .await
            .as_deref(),
    )
    .expect("AUDIT_ENTRY_KEY_PREVIOUS must be base64 of 32 bytes");
    if platform_config.audit.enabled
        && platform_config.audit.require_signing
        && entry_key_source == crate::audit::keys::EntryKeySource::Ephemeral
    {
        panic!(
            "AUDIT SIGNING REQUIRED: set AUDIT_ENTRY_KEY (base64 of 32 bytes) or BACKEND_KEY \
             so the audit worker can verify pending records"
        );
    }
    if let Some(keys) = optional_secret(secrets, "AUDIT_VERIFYING_KEYS")
        .await
        .filter(|value| !value.trim().is_empty())
    {
        crate::audit::signer::register_verifying_keys_json(&keys)
            .expect("AUDIT_VERIFYING_KEYS must map key ids to P-256 SPKI PEM public keys");
    }
    optional_secret(secrets, "AUDIT_KID")
        .await
        .filter(|value| !value.trim().is_empty())
}

struct MasterStores {
    content: Arc<FlowLikeStore>,
    meta: Arc<FlowLikeStore>,
    identity: crate::storage_identity::StorageIdentity,
}

/// Content and meta buckets from master credentials, the same mechanism board storage uses,
/// so IAM roles and STS work unchanged.
async fn open_master_stores() -> MasterStores {
    let master_creds = RuntimeCredentials::master_credentials()
        .await
        .expect("Failed to load master credentials");
    let (content, meta) =
        flow_like_types::tokio::join!(master_creds.to_store(false), master_creds.to_store(true));
    MasterStores {
        content: Arc::new(content.expect("Failed to create content store from master credentials")),
        meta: Arc::new(meta.expect("Failed to create meta store from master credentials")),
        identity: crate::storage_identity::from_credentials(&master_creds),
    }
}

async fn load_locked_storage(
    secrets: &SecretStore,
) -> Option<Arc<crate::locked_storage::LockedStorage>> {
    crate::locked_storage::LockedStorage::load(secrets)
        .await
        .expect("Invalid locked storage configuration")
        .map(Arc::new)
}

/// A supplied connection keeps the caller's dialect; otherwise `DATABASE_URL` is opened lazily.
async fn connect_database(
    database: Option<DatabaseConnection>,
    dialect: Option<DbDialect>,
    secrets: &SecretStore,
    sqlx_logging: bool,
) -> (DatabaseConnection, DbDialect) {
    let db = match database {
        Some(database) => database,
        None => {
            let db_url = secrets
                .get_secret_string(&SecretRef::new("DATABASE_URL"))
                .await
                .expect("DATABASE_URL must be set");
            let mut opt = ConnectOptions::new(db_url.expose_secret().to_owned());
            let pool = flow_like_db::pool::PoolConfig::from_env()
                .expect("Invalid database connection pool configuration");
            opt.max_connections(pool.max_connections)
                .min_connections(pool.min_connections)
                .connect_timeout(Duration::from_secs(8))
                .acquire_timeout(Duration::from_secs(8))
                .connect_lazy(true)
                .sqlx_logging(sqlx_logging);
            pin_session_time_zone_to_utc(&mut opt);

            Database::connect(opt)
                .await
                .expect("Failed to connect to database")
        }
    };
    let db_dialect = DbDialect::resolve(dialect, &db).await;
    tracing::info!(dialect = %db_dialect, "database dialect resolved");
    (db, db_dialect)
}

async fn stripe_client(secrets: &SecretStore, premium: bool) -> Option<stripe::Client> {
    if !premium {
        return None;
    }
    let stripe_key = secrets
        .get_secret_string(&SecretRef::new("STRIPE_SECRET_KEY"))
        .await
        .expect("STRIPE_SECRET_KEY must be set");
    tracing::info!("Stripe client initialized");
    Some(stripe::Client::new(stripe_key.expose_secret()))
}

struct MailClients {
    transactional: Option<DynMailClient>,
    automation: Option<DynMailClient>,
}

async fn mail_clients(
    secrets: &SecretStore,
    mail_config: Option<&flow_like::hub::MailConfig>,
    mail_automation: &mut crate::runtime_config::mail::MailAutomationConfig,
) -> MailClients {
    let transactional = match mail_config {
        Some(mail_config) => match create_mail_client(mail_config).await {
            Ok(client) => Some(client),
            Err(e) => {
                tracing::warn!("Failed to initialize mail client: {}", e);
                None
            }
        },
        None => None,
    };
    let automation = mail_automation
        .create_client_or_disable(secrets, mail_config, transactional.clone())
        .await;
    MailClients {
        transactional,
        automation,
    }
}

struct CloudClients {
    #[cfg(feature = "aws")]
    aws: Arc<SdkConfig>,
    channels: Arc<ChannelIssuer>,
}

async fn cloud_clients(secrets: &SecretStore) -> CloudClients {
    #[cfg(feature = "aws")]
    {
        let aws = Arc::new(aws_config::load_from_env().await);
        let channels = Arc::new(ChannelIssuer::from_env(secrets, aws.clone()).await);
        CloudClients { aws, channels }
    }
    #[cfg(not(feature = "aws"))]
    {
        CloudClients {
            channels: Arc::new(ChannelIssuer::from_env(secrets).await),
        }
    }
}

/// Priority: AWS EventBridge > Kubernetes > None (sink-service polls /schedules).
async fn sink_scheduler_from_env() -> Option<Arc<dyn flow_like_sinks::SchedulerBackend>> {
    use flow_like_sinks::scheduler::{self, SchedulerProvider};
    let scheduler_provider = std::env::var("SINK_SCHEDULER_PROVIDER")
        .ok()
        .map(|s| SchedulerProvider::from_env_value(&s));

    match scheduler_provider {
        Some(SchedulerProvider::Aws) => {
            #[cfg(feature = "aws")]
            {
                let scheduler = scheduler::AwsEventBridgeScheduler::from_env().await;
                tracing::info!("Initialized AWS EventBridge sink scheduler");
                Some(Arc::new(scheduler))
            }
            #[cfg(not(feature = "aws"))]
            {
                tracing::warn!("AWS scheduler requested but aws feature not enabled");
                None
            }
        }
        Some(SchedulerProvider::Kubernetes) => {
            #[cfg(feature = "kubernetes")]
            {
                match scheduler::KubernetesScheduler::from_env().await {
                    Ok(scheduler) => {
                        tracing::info!("Initialized Kubernetes CronJob sink scheduler");
                        Some(Arc::new(scheduler))
                    }
                    Err(e) => {
                        tracing::warn!("Failed to initialize K8s scheduler: {}", e);
                        None
                    }
                }
            }
            #[cfg(not(feature = "kubernetes"))]
            {
                tracing::warn!("Kubernetes scheduler requested but kubernetes feature not enabled");
                None
            }
        }
        Some(SchedulerProvider::Memory) => {
            tracing::info!("Using in-memory sink scheduler");
            Some(Arc::new(scheduler::InMemoryScheduler::new()))
        }
        None => {
            tracing::debug!("No sink scheduler configured (SINK_SCHEDULER_PROVIDER not set)");
            None
        }
    }
}

impl State {
    /// The registry a compiled artifact is fingerprinted against.
    ///
    /// The built-in catalog alone when the run brings no WASM packages;
    /// otherwise the catalog extended with the `Node` definitions of exactly
    /// the resolved packages, the same set the executor overlays from the
    /// loaded modules, so both sides compute one fingerprint and the artifact
    /// the API writes is the one the executor accepts. Built from the node
    /// definitions the compiler workload reported at upload time
    /// (`wasm_package_version.nodes`): no module bytes are read here, and the
    /// stand-in logic refuses to run, so user WASM never executes in this
    /// process.
    ///
    /// Read the definitions on every call before reusing an overlay. Metadata
    /// repairs can change them while the package version and binary stay the same.
    pub async fn artifact_registry(
        &self,
        wasm_packages: Option<&HashMap<String, WasmPackageRef>>,
    ) -> Result<Arc<FlowNodeRegistryInner>> {
        let diagnostics = std::env::var("FLOW_REGISTRY_DIAGNOSTICS").as_deref() == Ok("1");
        if diagnostics {
            tracing::warn!(
                native_registry_fingerprint = %blake3::Hash::from_bytes(self.registry.fingerprint()).to_hex(),
                native_node_count = self.registry.registry.len(),
                "Compiled artifact registry diagnostics: native catalog"
            );
        }
        let Some(packages) = wasm_packages.filter(|packages| !packages.is_empty()) else {
            return Ok(self.registry.clone());
        };
        let pins: Vec<(String, String)> = packages
            .iter()
            .map(|(package_id, package)| (package_id.clone(), package.version.clone()))
            .collect();
        if diagnostics {
            let mut diagnostic_pins = pins.clone();
            diagnostic_pins.sort();
            tracing::warn!(
                packages = ?diagnostic_pins,
                "Compiled artifact registry diagnostics: resolved package versions"
            );
        }
        let nodes = crate::routes::app::wasm_catalog::wasm_nodes_for_pins(&self.db, &pins)
            .await
            .map_err(|e| anyhow!("failed to load WASM node manifests for compilation: {e}"))?;
        if diagnostics {
            for node in &nodes {
                tracing::warn!(
                    node_name = %node.name,
                    node_version = ?node.version,
                    semantic_hash = %format_args!("{:016x}", node.semantic_hash()),
                    "Compiled artifact registry diagnostics: WASM node"
                );
            }
        }

        let package_revision = flow_like_types::dispatch::wasm_package_set_revision(Some(packages));
        let registry = crate::execution::wasm_node_stubs::cached_artifact_registry(
            &self.registry,
            &self.artifact_registries,
            &package_revision,
            nodes,
        )?;
        if diagnostics {
            tracing::warn!(
                registry_fingerprint = %blake3::Hash::from_bytes(registry.fingerprint()).to_hex(),
                node_count = registry.registry.len(),
                "Compiled artifact registry diagnostics: resulting catalog"
            );
        }
        Ok(registry)
    }

    /// Run `body` in a transaction and retry it on a lost commit race.
    ///
    /// See [`crate::db::retry_transaction`] for what a body may and may not do.
    pub async fn transaction<F, T, E>(&self, body: F) -> std::result::Result<T, E>
    where
        F: for<'c> Fn(
                &'c DatabaseTransaction,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = std::result::Result<T, E>> + Send + 'c>,
            > + Send
            + Sync,
        T: Send,
        E: From<sea_orm::DbErr>
            + crate::db::AsDbConflict
            + std::fmt::Display
            + std::fmt::Debug
            + Send,
    {
        crate::db::retry_transaction(
            &self.db,
            self.db_dialect,
            None,
            &crate::db::RetryPolicy::default(),
            body,
        )
        .await
    }

    /// [`Self::transaction`] with an isolation level, honoured where the engine has one.
    pub async fn transaction_with<F, T, E>(
        &self,
        isolation: IsolationLevel,
        body: F,
    ) -> std::result::Result<T, E>
    where
        F: for<'c> Fn(
                &'c DatabaseTransaction,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = std::result::Result<T, E>> + Send + 'c>,
            > + Send
            + Sync,
        T: Send,
        E: From<sea_orm::DbErr>
            + crate::db::AsDbConflict
            + std::fmt::Display
            + std::fmt::Debug
            + Send,
    {
        crate::db::retry_transaction(
            &self.db,
            self.db_dialect,
            Some(isolation),
            &crate::db::RetryPolicy::default(),
            body,
        )
        .await
    }

    fn board_mutation_lock(
        &self,
        app_id: &str,
        board_id: &str,
    ) -> Arc<flow_like_types::tokio::sync::Mutex<()>> {
        keyed_local_lock(
            &self.board_mutation_locks,
            board_mutation_lock_key(app_id, board_id),
        )
    }

    /// Serialize one board writer both within this process and across API replicas.
    ///
    /// The local mutex is acquired first so same-process waiters never touch the database. The
    /// lease row is then claimed in a short retried transaction and kept alive by a heartbeat;
    /// canonical board bytes continue to be read and written through storage while it is held.
    /// A lease that stays busy for [`crate::db::lease::LEASE_WAIT_BUDGET`] fails the request.
    pub(crate) async fn board_mutation_guard(
        &self,
        app_id: &str,
        board_id: &str,
    ) -> std::result::Result<BoardMutationGuard, ApiError> {
        let local = self
            .board_mutation_lock(app_id, board_id)
            .lock_owned()
            .await;
        let lease =
            MutationLease::claim(self, board_mutation_lock_id(app_id, board_id), local).await?;
        Ok(BoardMutationGuard { lease })
    }

    pub async fn new(
        catalog: Arc<Vec<Arc<dyn NodeLogic>>>,
        cdn_bucket: Arc<FlowLikeStore>,
        secret_store_config: Option<SecretStoreConfig>,
    ) -> Self {
        let catalog = NodeCatalog::from(catalog);
        let secrets = Self::initialize_secret_store(secret_store_config).await;
        Self::new_with_secrets(catalog, cdn_bucket, secrets, None, None).await
    }

    /// Construct API state around a caller-managed database connection.
    ///
    /// Cloud targets that use short-lived, identity-backed database credentials
    /// must create the pool themselves so the access token never has to be stored
    /// in `DATABASE_URL`. The standard constructor intentionally retains its
    /// existing `DATABASE_URL` behavior for all other deployment targets.
    ///
    /// A caller that already knows which engine it connected to passes the
    /// `dialect`; `None` falls back to `FLOW_LIKE_DB_DIALECT` and a probe.
    pub async fn new_with_database(
        catalog: Arc<Vec<Arc<dyn NodeLogic>>>,
        cdn_bucket: Arc<FlowLikeStore>,
        secret_store_config: Option<SecretStoreConfig>,
        database: DatabaseConnection,
        dialect: Option<DbDialect>,
    ) -> Self {
        let catalog = NodeCatalog::from(catalog);
        let secrets = Self::initialize_secret_store(secret_store_config).await;
        Self::new_with_secrets(catalog, cdn_bucket, secrets, Some(database), dialect).await
    }

    async fn initialize_secret_store(
        secret_store_config: Option<SecretStoreConfig>,
    ) -> Arc<SecretStore> {
        let config = secret_store_config.unwrap_or_else(|| {
            let prefix = std::env::var("SECRET_PREFIX").ok();
            SecretStoreConfig::default()
                .with_provider(ProviderConfig::Env(EnvProviderConfig { prefix }))
        });
        let secrets = Arc::new(SecretStore::new(config).expect("Failed to create secret store"));
        secrets.warmup().await;
        secrets
    }

    /// Reuse a caller-owned secret store and its cache during API initialization.
    ///
    /// Warm the store before building services that need secrets. This constructor
    /// preserves that cache without repeating provider prefetches. A supplied
    /// database uses the same connection and dialect rules as `new_with_database`;
    /// otherwise the connection comes from `DATABASE_URL`.
    ///
    /// Steps that only share the loaded configuration run concurrently, and the node
    /// catalog finishes on its blocking thread meanwhile.
    pub async fn new_with_secrets(
        catalog: impl Into<NodeCatalog>,
        cdn_bucket: Arc<FlowLikeStore>,
        secrets: Arc<SecretStore>,
        database: Option<DatabaseConnection>,
        dialect: Option<DbDialect>,
    ) -> Self {
        let catalog = catalog.into();
        let RuntimeConfig {
            platform: mut platform_config,
            mut mail_automation,
            oauth_providers,
            openid: openid_validation_overrides,
        } = load_runtime_config(&secrets)
            .instrument(init_span!("api.init.config"))
            .await;

        let oidc_clients = crate::oidc_http::clients_from_env(
            platform_config
                .authentication
                .as_ref()
                .and_then(|authentication| authentication.openid.as_ref())
                .map(|openid| openid.jwks_url.as_str()),
        )
        .unwrap_or_else(|error| panic!("Invalid OIDC HTTPS configuration: {error}"));

        let (
            service_secrets,
            audit_kid,
            stores,
            locked_storage,
            (db, db_dialect),
            stripe_client,
            mail,
            cloud,
            sink_scheduler,
        ) = flow_like_types::tokio::join!(
            load_service_secrets(&secrets).instrument(init_span!("api.init.service_secrets")),
            install_signing_keys(&secrets, &platform_config)
                .instrument(init_span!("api.init.signing_keys")),
            open_master_stores().instrument(init_span!("api.init.storage")),
            load_locked_storage(&secrets).instrument(init_span!("api.init.locked_storage")),
            connect_database(
                database,
                dialect,
                &secrets,
                platform_config.environment == Environment::Development,
            )
            .instrument(init_span!("api.init.database")),
            stripe_client(&secrets, platform_config.features.premium)
                .instrument(init_span!("api.init.stripe")),
            mail_clients(
                &secrets,
                platform_config.mail.as_ref(),
                &mut mail_automation
            )
            .instrument(init_span!("api.init.mail")),
            cloud_clients(&secrets).instrument(init_span!("api.init.cloud_clients")),
            sink_scheduler_from_env().instrument(init_span!("api.init.sink_scheduler")),
        );
        let ServiceSecrets {
            sink_secret,
            maintenance_token,
            encryption_key,
        } = service_secrets;
        let MasterStores {
            content: content_bucket,
            meta: meta_bucket,
            identity: storage_identity,
        } = stores;
        #[cfg(feature = "aws")]
        let aws_client = cloud.aws;
        let channels = cloud.channels;

        let realtime_ice =
            RealtimeIceService::from_config(&platform_config.realtime, Arc::clone(&secrets))
                .unwrap_or_else(|error| {
                    panic!("Realtime ICE provider configuration is invalid: {error}")
                });

        // JWKS used to be downloaded by build.rs, making clean and cross builds depend on a
        // live identity-provider endpoint. Start with a fail-closed empty cache instead; the
        // existing bounded HTTPS refresh in `configured_jwk` fills it on first use.
        let jwks = JwkSet { keys: Vec::new() };

        init_span!("api.init.widget_policy").in_scope(|| {
            crate::routes::registry::widget_policy::init_widget_policy(
                &mut platform_config,
                &storage_identity.content,
            )
        });

        let client: Client<HttpConnector, Body> =
            hyper_util::client::legacy::Client::<(), ()>::builder(TokioExecutor::new())
                .build(HttpConnector::new());

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

        let cache = moka::sync::Cache::builder()
            .max_capacity(32 * 1024 * 1024) // 32 MB
            .time_to_live(Duration::from_secs(20 * 60)) // Each cache hit also checks the provider expiration.
            .build();

        let response_cache = moka::sync::Cache::builder()
            .weigher(response_cache_weight)
            .max_capacity(64 * 1024 * 1024)
            .time_to_live(Duration::from_secs(60))
            .build();

        // Both dispatchers build their clients once from env config.
        let (dispatcher, compilation_dispatcher) = flow_like_types::tokio::join!(
            async {
                Dispatcher::new(DispatchConfig::from_env(), Some(meta_bucket.clone()))
                    .await
                    .with_channel_issuer(channels.clone())
            }
            .instrument(init_span!("api.init.dispatcher")),
            async {
                let compilation_config = CompilationDispatchConfig::from_env();
                tracing::info!(backend = ?compilation_config.backend, "Compilation dispatch backend");
                Arc::new(
                    CompilationDispatcher::new(
                        compilation_config,
                        content_bucket.clone(),
                        meta_bucket.clone(),
                    )
                    .await,
                )
            }
            .instrument(init_span!("api.init.compilation_dispatcher")),
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

        // A cache that cannot come up is surfaced as a 503 from the cache endpoints at
        // first use rather than as a failed boot for every other feature.
        let cache_backend = crate::cache::CacheBackendHandle::new(Arc::new(db.clone()));
        #[cfg(feature = "aws")]
        let cache_backend = cache_backend.with_aws_config(aws_client.clone());

        let (catalog, registry) = catalog
            .ready()
            .instrument(init_span!("api.init.catalog_wait"))
            .await;

        Self {
            platform_config,
            oauth_providers,
            openid_validation_overrides,
            db,
            db_dialect,
            client,
            oidc_jwks_client: oidc_clients.jwks,
            oidc_user_info_client: oidc_clients.user_info,
            jwks: flow_like_types::tokio::sync::RwLock::new(jwks),
            jwks_refresh: flow_like_types::tokio::sync::Mutex::new(JwksRefreshState::default()),
            stripe_client,
            payments_gateway: flow_like_types::tokio::sync::OnceCell::new(),
            legacy_payments_gateway: flow_like_types::tokio::sync::OnceCell::new(),
            mail_client: mail.transactional,
            mail_automation,
            automation_mail_client: mail.automation,
            #[cfg(feature = "aws")]
            aws_client,
            #[cfg(feature = "aws")]
            scoped_sts_client: std::sync::OnceLock::new(),
            catalog,
            provider: Arc::new(provider),
            registry: Arc::new(registry),
            dispatcher: Arc::new(dispatcher),
            compilation_dispatcher,
            channels,
            realtime_ice,
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
            board_cache: moka::sync::Cache::builder()
                .max_capacity(BOARD_CACHE_MAX_ENTRIES)
                .time_to_idle(Duration::from_secs(30 * 60))
                .build(),
            compiled_artifact_cache: moka::sync::Cache::builder()
                .max_capacity(16_384)
                .time_to_idle(Duration::from_secs(12 * 60 * 60))
                .build(),
            compiled_widget_artifacts: Default::default(),
            artifact_registries: moka::sync::Cache::builder()
                .max_capacity(256)
                .time_to_idle(Duration::from_secs(60 * 60))
                .build(),
            prerun_manifest_cache: moka::sync::Cache::builder()
                .max_capacity(4_096)
                .time_to_idle(Duration::from_secs(24 * 60 * 60))
                .support_invalidation_closures()
                .build(),
            board_sync_cache: moka::sync::Cache::builder()
                .max_capacity(BOARD_CACHE_MAX_ENTRIES)
                .time_to_idle(Duration::from_secs(30 * 60))
                .build(),
            board_snapshot_heads: moka::sync::Cache::builder()
                .max_capacity(BOARD_CACHE_MAX_ENTRIES)
                .time_to_idle(Duration::from_secs(30 * 60))
                .build(),
            board_segment_bases: moka::sync::Cache::builder()
                // Weighed by node count: a handful of 1000-node boards' worth of recent
                // revisions, not a handful of entries.
                .weigher(
                    |_key, segment: &Arc<flow_like::flow::board::sync::SyncSegment>| {
                        u32::try_from(segment.nodes.len().max(1)).unwrap_or(u32::MAX)
                    },
                )
                .max_capacity(BOARD_SEGMENT_BASE_MAX_NODES)
                .time_to_idle(Duration::from_secs(30 * 60))
                .build(),
            // Installs, updates and removals are caught immediately by the package-pin digest in
            // the key. The TTL only backstops the one change that digest cannot see — a package
            // republished under the same version — and reclaims retired epochs.
            app_wasm_nodes_cache: moka::sync::Cache::builder()
                .max_capacity(1_000)
                .time_to_live(Duration::from_secs(10 * 60))
                .build(),
            board_load_locks: parking_lot::Mutex::new(HashMap::new()),
            credentials_cache: cache,
            credential_refresh_locks: parking_lot::Mutex::new(HashMap::new()),
            content_bucket,
            cdn_bucket,
            meta_bucket,
            storage_identity,
            locked_storage,
            response_cache,
            execution_usage_cache: Default::default(),
            wasm_permission_cache: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(120))
                .build(),
            // Auth cache: max 10k entries. JWT expiry is checked on every hit.
            // Entries are keyed by token hash to avoid storing raw tokens
            auth_cache: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(240))
                .build(),
            wasm_registry,
            sink_scheduler,
            cache: cache_backend,
            execution_state_store: flow_like_types::tokio::sync::OnceCell::new(),
            secrets,
            encryption_key,
            audit_kid,
            sink_secret,
            maintenance_token,
            trigger_idempotency: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(15 * 60))
                .build(),
        }
    }

    fn openid_validation_settings(&self) -> Result<OpenIdValidationSettings> {
        openid_validation_settings_for_hub(&self.platform_config, &self.openid_validation_overrides)
    }

    async fn configured_jwk(&self, kid: &str) -> Result<Jwk> {
        {
            let jwks = self.jwks.read().await;
            if let Some(jwk) = find_unique_jwk(&jwks, kid)? {
                return Ok(jwk);
            }
        }

        // A public JWKS endpoint needs no client secret. Refresh only the fixed,
        // reviewed URL from the embedded deployment config, serialize refreshes,
        // and rate-limit failed/attacker-triggered unknown-kid requests.
        let mut refresh = self.jwks_refresh.lock().await;
        {
            let jwks = self.jwks.read().await;
            if let Some(jwk) = find_unique_jwk(&jwks, kid)? {
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
        let refreshed = fetch_jwks(&self.oidc_jwks_client, &settings.jwks_url).await?;
        // Cache every successfully validated set even when this particular `kid` is unknown.
        // Otherwise a bogus first request after startup discards the valid key set and the
        // refresh throttle blocks legitimate tokens until the next interval.
        let jwk = find_unique_jwk(&refreshed, kid)?;
        *self.jwks.write().await = refreshed;
        jwk.ok_or_else(|| flow_like_types::anyhow!("OpenID signing key is not published"))
    }

    pub(crate) async fn validate_token(&self, token: &str) -> Result<ValidatedOpenIdToken> {
        let settings = self.openid_validation_settings()?;
        let claims = self.decode_openid_token(token, &settings).await?;
        let expires_at = validate_openid_claims(
            &claims,
            &settings.issuer,
            &settings.client_ids,
            &settings.audience,
            settings.tenant_id,
            chrono::Utc::now().timestamp(),
            settings.leeway,
        )?;

        Ok(ValidatedOpenIdToken { claims, expires_at })
    }

    pub(crate) async fn validate_mcp_token(
        &self,
        token: &str,
        audience: &str,
    ) -> Result<ValidatedOpenIdToken> {
        exact_nonempty_str("MCP audience", audience)?;
        let settings = self.openid_validation_settings()?;
        let claims = self.decode_openid_token(token, &settings).await?;
        let expires_at = validate_mcp_openid_claims(
            &claims,
            &settings,
            audience,
            chrono::Utc::now().timestamp(),
        )?;

        Ok(ValidatedOpenIdToken { claims, expires_at })
    }

    async fn decode_openid_token(
        &self,
        token: &str,
        settings: &OpenIdValidationSettings,
    ) -> Result<HashMap<String, Value>> {
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

        Ok(decode::<HashMap<String, Value>>(token, &decoding_key, &validation)?.claims)
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
        let key = format!("{}:{}:{}:{}:{}", sub.len(), sub, app_id.len(), app_id, mode);
        let minimum_lifetime = scoped_credential_minimum_lifetime(&mode)?;
        if let Some(credentials) = self.credentials_cache.get(&key) {
            if !credentials.expires_soon(minimum_lifetime) {
                return Ok(credentials);
            }
        }
        let refresh_lock = keyed_local_lock(&self.credential_refresh_locks, key.clone());
        let _refresh = refresh_lock.lock().await;
        // The first concurrent caller may already have renewed this grant.
        if let Some(credentials) = self.credentials_cache.get(&key) {
            if !credentials.expires_soon(minimum_lifetime) {
                return Ok(credentials);
            }
            self.credentials_cache.invalidate(&key);
        }
        let credentials = Arc::new(RuntimeCredentials::scoped(sub, app_id, self, mode).await?);
        if minimum_lifetime > chrono::Duration::seconds(120)
            && credentials.expires_soon(minimum_lifetime)
        {
            bail!(
                "Scoped execution credentials have insufficient actual lifetime for execution, queue wait and cleanup; increase the provider session duration (STS_SESSION_TTL_SECONDS for S3 STS)"
            );
        }
        self.credentials_cache.insert(key, credentials.clone());
        Ok(credentials)
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
        let app_state = self.master_state(state).await?;
        let app = App::load(app_id.to_string(), app_state).await?;
        Ok(app)
    }

    /// Replace the manifest's `visibility` with the database value. The manifest
    /// is written as `Offline` at creation and never follows visibility changes,
    /// so every server-side decision that reads `App::visibility` needs this.
    pub async fn hydrate_app_visibility(&self, app: &mut App) -> flow_like_types::Result<()> {
        use sea_orm::{EntityTrait, QuerySelect};
        let visibility = crate::entity::app::Entity::find_by_id(&app.id)
            .select_only()
            .column(crate::entity::app::Column::Visibility)
            .into_tuple::<crate::entity::sea_orm_active_enums::Visibility>()
            .one(&self.db)
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "App '{}' has no database row to read visibility from",
                    app.id
                )
            })?;
        app.visibility = visibility.into();
        Ok(())
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
        let storage_root = Path::from("apps").join(app_id.to_string());
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
        let app_state = self.master_state(state).await?;
        let cached = self
            .master_board_shared_with(app_id, board_id, app_state.clone(), version)
            .await?;
        let mut board = (*cached.board).clone();
        // The cached hydration may predate a master-state rotation; hand out the live one so
        // `save(None)` and command execution never run on retired credentials.
        board.app_state = Some(app_state);
        Ok(board)
    }

    /// The process-wide master `FlowLikeState`, built lazily and rotated by `state_cache`'s TTL.
    pub(crate) async fn master_state(
        &self,
        state: &AppState,
    ) -> flow_like_types::Result<Arc<FlowLikeState>> {
        if let Some(app_state) = self.state_cache.get("master") {
            return Ok(app_state);
        }
        let credentials = self.master_credentials().await?;
        let app_state = Arc::new(credentials.to_state(state.clone()).await?);
        self.state_cache
            .insert("master".to_string(), app_state.clone());
        Ok(app_state)
    }

    /// Load a board on master credentials, validated against storage but served from memory.
    ///
    /// Draft reads use an `If-None-Match` GET carrying the cached ETag. Concurrent callers
    /// share a check only when it started after their request. Storage answering `NotModified`
    /// proves the cached hydration is still what a fresh load would produce, regardless of which replica,
    /// fork job, CLI, or publish path wrote the object. A changed object arrives in that same
    /// round trip, so a miss costs exactly what the old unconditional load did. Pinned versions
    /// are immutable snapshots, so a confirmed one is served from memory and reconfirmed at most
    /// once per [`PINNED_BOARD_REVALIDATE_AFTER`].
    ///
    /// The returned board is shared. Callers that mutate (WASM hydration, secret filtering,
    /// commands) must clone first — [`State::master_board`] does that for them.
    #[tracing::instrument(
        name = "master_board_shared",
        skip(self, state),
        level = "debug",
        fields(app_id, board_id, version)
    )]
    pub async fn master_board_shared(
        &self,
        app_id: &str,
        board_id: &str,
        state: &AppState,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Arc<CachedBoard>> {
        let app_state = self.master_state(state).await?;
        self.master_board_shared_with(app_id, board_id, app_state, version)
            .await
    }

    async fn master_board_shared_with(
        &self,
        app_id: &str,
        board_id: &str,
        app_state: Arc<FlowLikeState>,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Arc<CachedBoard>> {
        let requested_at = Instant::now();
        let cache_key = board_cache_key(app_id, board_id, version);
        if let Some(pinned) = self.recently_verified_pinned(&cache_key, version) {
            return pinned;
        }
        let load_lock = keyed_local_lock(&self.board_load_locks, cache_key.clone());
        let _single_flight = load_lock.lock().await;
        // A queued burst shares a fresh check instead of serializing one storage
        // round trip per action. Later requests still revalidate floating drafts.
        if let Some(cached) =
            verified_board_after_wait(&self.board_cache, &cache_key, version, requested_at)
        {
            cached.board.ensure_supported_format()?;
            return Ok(cached);
        }

        let cached = self.board_cache.get(&cache_key);
        let store = Board::meta_store(&app_state).await?;
        let storage_root = Path::from("apps").join(app_id.to_string());
        let verification_started_at = Instant::now();
        let read = Board::load_proto_if_changed(
            store,
            &storage_root,
            board_id,
            version,
            cached.as_ref().map(|entry| entry.e_tag.as_str()),
        )
        .await?;

        let (proto, meta) = match read {
            ConditionalRead::NotModified => {
                let cached = cached.ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "storage reported NotModified for {app_id}/{board_id} without a cached ETag"
                    )
                })?;
                cached.board.ensure_supported_format()?;
                cached.mark_verified(verification_started_at);
                return Ok(cached);
            }
            ConditionalRead::Fresh(proto, meta) => (proto, meta),
        };

        let board =
            Board::from_loaded_proto_for_version(proto, storage_root, app_state, version).await?;
        let entry = Arc::new(CachedBoard::new(
            meta.e_tag.clone().unwrap_or_default(),
            board,
        ));
        entry.mark_verified(verification_started_at);
        // An object without an ETag can never be validated, so it is never cached.
        if meta.e_tag.is_some() {
            self.board_cache.insert(cache_key, entry.clone());
        }
        Ok(entry)
    }

    /// Pin an in-memory board to the object identity its writer just persisted.
    ///
    /// The writer's board is post-`execute_commands`, which ends with the same `node_updates` +
    /// `cleanup` normalisation `Board::load` applies, so it is what the next load of `put`
    /// would produce. Seeding makes the read that follows every edit a `NotModified` memory hit
    /// instead of a decode + hydration. Skipping this is always safe (the next read reloads);
    /// seeding a board that does **not** match `put` is not, so only call it with the exact
    /// board whose `save` returned `put`.
    ///
    /// Returns the seeded entry so the writer can build the revision's sync snapshot from the
    /// very board it just persisted (see `routes::app::board::sync_board::seed_board_revision`).
    /// `None` when the object carried no ETag and therefore cannot be validated later.
    pub fn seed_board_cache(
        &self,
        app_id: &str,
        board_id: &str,
        board: Board,
        put: &flow_like::flow_like_storage::object_store::PutResult,
    ) -> Option<Arc<CachedBoard>> {
        let e_tag = put.e_tag.clone()?;
        let entry = Arc::new(CachedBoard::new(e_tag, board));
        self.board_cache
            .insert(board_cache_key(app_id, board_id, None), entry.clone());
        Some(entry)
    }

    fn recently_verified_pinned(
        &self,
        cache_key: &str,
        version: Option<(u32, u32, u32)>,
    ) -> Option<flow_like_types::Result<Arc<CachedBoard>>> {
        let pinned = recently_verified_pinned(&self.board_cache, cache_key, version)?;
        Some(pinned.board.ensure_supported_format().map(|()| pinned))
    }

    /// The draft entry this instance holds, without asking storage whether it is current.
    /// Callers must compare `e_tag` against an identity they obtained from storage.
    pub fn cached_draft_board(&self, app_id: &str, board_id: &str) -> Option<Arc<CachedBoard>> {
        self.board_cache
            .get(&board_cache_key(app_id, board_id, None))
    }

    /// Key under which `board_snapshot_heads` remembers the last snapshot built for a board.
    pub fn board_snapshot_head_key(
        app_id: &str,
        board_id: &str,
        version: Option<(u32, u32, u32)>,
    ) -> String {
        board_cache_key(app_id, board_id, version)
    }

    /// Drop the cached floating draft. Only needed by writers that produce a board the seed
    /// contract cannot vouch for; ordinary readers self-heal through the ETag check.
    pub fn invalidate_board_cache(&self, app_id: &str, board_id: &str) {
        self.board_cache
            .invalidate(&board_cache_key(app_id, board_id, None));
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
        let app_state = self.master_state(state).await?;
        let storage_root = Path::from("apps").join(app_id.to_string());
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

        let storage_root = Path::from("apps").join(app_id.to_string());

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
        // Only grants are cached. Access is granted by other API processes
        // too (purchase webhook, approval, invitation), and their invalidation
        // never reaches this one, so a cached "no access" would outlive it.
        // A lookup that finds none still drops a grant cached before it.
        let key = format!("wasm:{}:{}", user_id, package_id);
        if perm.is_empty() {
            self.wasm_permission_cache.invalidate(&key);
            return;
        }
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

fn openid_validation_settings_for_hub(
    hub: &Hub,
    overrides: &OpenIdValidationOverrides,
) -> Result<OpenIdValidationSettings> {
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
        require_access_token_use: config.cognito.is_some(),
    })
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

#[tracing::instrument(
    target = "flow_like::observability",
    name = "auth.jwks.fetch",
    skip_all,
    fields(rpc.service = "openid", rpc.method = "GetJwks")
)]
async fn fetch_jwks(client: &reqwest::Client, raw_url: &str) -> Result<JwkSet> {
    let url = validate_jwks_url(raw_url)?;
    let mut response = client
        .get(url)
        .timeout(JWKS_REQUEST_TIMEOUT)
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

fn validate_mcp_openid_claims(
    claims: &HashMap<String, Value>,
    settings: &OpenIdValidationSettings,
    audience: &str,
    now: i64,
) -> Result<i64> {
    exact_nonempty_str("MCP audience", audience)?;
    // Resource-bound access tokens must identify both this MCP endpoint and
    // an OAuth client trusted by the deployment.
    if !claims.contains_key("aud") {
        bail!("MCP access token has no audience");
    }
    if !["client_id", "azp", "appid"]
        .iter()
        .any(|claim| claims.contains_key(*claim))
    {
        bail!("MCP access token has no OAuth client identifier");
    }
    match claims.get("token_use") {
        Some(Value::String(purpose)) if purpose == "access" => {}
        None if !settings.require_access_token_use => {}
        _ => bail!("MCP requires an access token"),
    }

    validate_openid_claims(
        claims,
        &settings.issuer,
        &settings.client_ids,
        audience,
        settings.tenant_id,
        now,
        settings.leeway,
    )
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
        OpenIdValidationSettings, board_mutation_lock_id, board_mutation_lock_key,
        cached_openid_is_current, course_attempt_lock_id, entra_tenant_from_issuer,
        execution_credential_lifetime, flow_ir_draft_store_key, native_async_credential_lifetime,
        validate_jwk_for_header, validate_jwks_set, validate_mcp_openid_claims,
        validate_openid_claims,
    };
    use flow_like_types::Value;
    use jsonwebtoken::{
        Algorithm,
        jwk::{Jwk, JwkSet},
    };
    use std::collections::{BTreeSet, HashMap};

    fn cached_board() -> std::sync::Arc<super::CachedBoard> {
        std::sync::Arc::new(super::CachedBoard::new(
            "\"etag-1\"".to_string(),
            flow_like::flow::board::Board::new_detached(
                Some("board-1".into()),
                flow_like::flow_like_storage::Path::from("apps").join("app-1"),
            ),
        ))
    }

    #[test]
    fn cached_page_revisions_reuse_board_metadata_and_follow_page_changes() {
        use flow_like::flow::compiled::prerun::{
            PrerunPageEventTarget, PrerunPageExecution, page_execution_revision,
        };
        use std::sync::Arc;

        let mut cached = Arc::try_unwrap(cached_board()).ok().unwrap();
        let board = Arc::get_mut(&mut cached.board).unwrap();
        let mut node = flow_like::flow::node::Node::new("events_simple", "Entry", "", "Events");
        node.id = "entry-1".into();
        node.set_start(true);
        board.nodes.insert(node.id.clone(), node);
        let mut execution = PrerunPageExecution {
            page_id: "page-1".into(),
            ..Default::default()
        };
        let manifest = cached.prerun_manifest();
        let initial = cached.page_execution_revision(&execution).unwrap();
        assert_eq!(
            initial,
            page_execution_revision(&cached.board, &execution).unwrap()
        );
        assert_eq!(initial, cached.page_execution_revision(&execution).unwrap());

        execution.special_events.load = Some(PrerunPageEventTarget {
            node_id: "entry-1".into(),
        });
        let changed = cached.page_execution_revision(&execution).unwrap();
        assert_ne!(initial, changed);
        assert_eq!(
            changed,
            page_execution_revision(&cached.board, &execution).unwrap()
        );
        assert!(Arc::ptr_eq(&manifest, &cached.prerun_manifest()));
        assert_eq!(
            cached
                .prerun_derivation
                .lock()
                .as_ref()
                .unwrap()
                .pages
                .len(),
            1
        );

        execution.special_events.load.as_mut().unwrap().node_id = "missing".into();
        assert!(cached.page_execution_revision(&execution).is_err());
    }

    #[test]
    fn cached_page_revisions_invalidate_after_board_mutation_or_replacement() {
        use flow_like::flow::compiled::prerun::{PrerunPageExecution, page_execution_revision};
        use std::sync::Arc;

        let mut cached = Arc::try_unwrap(cached_board()).ok().unwrap();
        let execution = PrerunPageExecution {
            page_id: "page-1".into(),
            ..Default::default()
        };
        let first = cached.page_execution_revision(&execution).unwrap();
        Arc::make_mut(&mut cached.board).name.push_str(" changed");
        let second = cached.page_execution_revision(&execution).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            second,
            page_execution_revision(&cached.board, &execution).unwrap()
        );

        let mut replacement = (*cached.board).clone();
        replacement.version = (2, 3, 4);
        cached.board = Arc::new(replacement);
        let third = cached.page_execution_revision(&execution).unwrap();
        assert_ne!(second, third);
        assert_eq!(
            third,
            page_execution_revision(&cached.board, &execution).unwrap()
        );
    }

    #[test]
    fn cached_page_revisions_are_bounded_and_shared_by_concurrent_readers() {
        use flow_like::flow::compiled::prerun::PrerunPageExecution;
        use std::sync::Arc;

        let cached = cached_board();
        for index in 0..=super::CACHED_PAGE_REVISIONS_PER_BOARD {
            cached
                .page_execution_revision(&PrerunPageExecution {
                    page_id: format!("page-{index}"),
                    ..Default::default()
                })
                .unwrap();
        }
        assert!(
            cached
                .prerun_derivation
                .lock()
                .as_ref()
                .unwrap()
                .pages
                .len()
                <= super::CACHED_PAGE_REVISIONS_PER_BOARD
        );
        let readers = (0..8)
            .map(|_| {
                let cached = cached.clone();
                std::thread::spawn(move || {
                    let revision = cached
                        .page_execution_revision(&PrerunPageExecution {
                            page_id: "page-current".into(),
                            ..Default::default()
                        })
                        .unwrap();
                    (revision, cached.prerun_manifest())
                })
            })
            .collect::<Vec<_>>();
        let expected = cached
            .page_execution_revision(&PrerunPageExecution {
                page_id: "page-current".into(),
                ..Default::default()
            })
            .unwrap();
        let manifest = cached.prerun_manifest();
        for reader in readers {
            let (revision, shared_manifest) = reader.join().unwrap();
            assert_eq!(revision, expected);
            assert!(Arc::ptr_eq(&manifest, &shared_manifest));
        }
    }

    #[test]
    fn pinned_boards_skip_revalidation_only_inside_the_window() {
        let cache = moka::sync::Cache::builder().max_capacity(4).build();
        let pinned_key = super::board_cache_key("app-1", "board-1", Some((1, 2, 3)));
        let draft_key = super::board_cache_key("app-1", "board-1", None);
        let entry = cached_board();
        cache.insert(pinned_key.clone(), entry.clone());
        cache.insert(draft_key.clone(), entry.clone());

        assert!(super::recently_verified_pinned(&cache, &pinned_key, Some((1, 2, 3))).is_some());
        assert!(
            super::recently_verified_pinned(&cache, &draft_key, None).is_none(),
            "a floating draft is revalidated on every read"
        );

        *entry.verified_at.lock() = std::time::Instant::now()
            - super::PINNED_BOARD_REVALIDATE_AFTER
            - std::time::Duration::from_secs(1);
        assert!(super::recently_verified_pinned(&cache, &pinned_key, Some((1, 2, 3))).is_none());
        entry.mark_verified(std::time::Instant::now());
        assert!(super::recently_verified_pinned(&cache, &pinned_key, Some((1, 2, 3))).is_some());
    }

    #[test]
    fn queued_draft_reads_share_only_checks_started_after_the_request() {
        let cache = moka::sync::Cache::builder().max_capacity(4).build();
        let key = super::board_cache_key("app-1", "board-1", None);
        let entry = cached_board();
        cache.insert(key.clone(), entry.clone());
        let now = std::time::Instant::now();
        let requested_at = now - std::time::Duration::from_millis(2);
        let reusable = || super::verified_board_after_wait(&cache, &key, None, requested_at);

        // Seeding after a write is not a fresh read of storage: another replica
        // may already have written a newer revision.
        assert!(reusable().is_none());

        // A slow read can finish after this request but still carry an older
        // snapshot. Completion time alone must never allow that read to be reused.
        entry.mark_verified(requested_at - std::time::Duration::from_millis(1));
        assert!(reusable().is_none());
        entry.mark_verified(requested_at);
        assert!(reusable().is_none());

        entry.mark_verified(now - std::time::Duration::from_millis(1));
        assert!(std::sync::Arc::ptr_eq(&reusable().unwrap(), &entry));
        assert!(super::verified_board_after_wait(&cache, &key, None, now).is_none());

        let other_app = super::board_cache_key("app-2", "board-1", None);
        let pinned = super::board_cache_key("app-1", "board-1", Some((1, 0, 0)));
        assert!(super::verified_board_after_wait(&cache, &other_app, None, requested_at).is_none());
        assert!(
            super::verified_board_after_wait(&cache, &pinned, Some((1, 0, 0)), requested_at)
                .is_none()
        );
        cache.insert(key.clone(), cached_board());
        assert!(
            reusable().is_none(),
            "a seeded replacement needs its own check"
        );
        cache.invalidate(&key);
        assert!(reusable().is_none());
    }

    #[tokio::test]
    async fn queued_draft_burst_reuses_one_revalidation_and_later_read_checks_again() {
        let cache = moka::sync::Cache::builder().max_capacity(4).build();
        let key = super::board_cache_key("app-1", "board-1", None);
        let entry = cached_board();
        cache.insert(key.clone(), entry.clone());
        let locks = parking_lot::Mutex::new(HashMap::new());
        let load_lock = super::keyed_local_lock(&locks, key.clone());
        let held = load_lock.lock().await;
        let (queued, mut waiting) = tokio::sync::mpsc::unbounded_channel();
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..32 {
            let (cache, key, entry, reads, queued) = (
                cache.clone(),
                key.clone(),
                entry.clone(),
                reads.clone(),
                queued.clone(),
            );
            let load_lock = super::keyed_local_lock(&locks, key.clone());
            tasks.push(tokio::spawn(async move {
                let requested_at = std::time::Instant::now();
                queued.send(()).unwrap();
                let _guard = load_lock.lock().await;
                if let Some(hit) =
                    super::verified_board_after_wait(&cache, &key, None, requested_at)
                {
                    return hit;
                }
                let started_at = std::time::Instant::now();
                reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::task::yield_now().await;
                entry.mark_verified(started_at);
                entry
            }));
        }
        for _ in 0..32 {
            waiting.recv().await.unwrap();
        }
        drop(held);
        for task in tasks {
            assert!(std::sync::Arc::ptr_eq(&task.await.unwrap(), &entry));
        }
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(
            super::verified_board_after_wait(&cache, &key, None, std::time::Instant::now())
                .is_none()
        );
    }

    #[test]
    fn pinned_pages_are_kept_with_their_version() {
        let entry = cached_board();
        assert!(entry.version_page("page-1").is_none());
        let page = flow_like::a2ui::Page::new("page-1", "Page", "/");
        entry.remember_version_page(&page);
        assert_eq!(entry.version_page("page-1").unwrap().id, page.id);
        assert!(entry.version_page("page-2").is_none());
    }

    #[test]
    fn trust_only_extends_and_lapses_at_its_deadline() {
        let trust = super::TrustedUntil::default();
        assert!(!trust.holds());
        let now = std::time::SystemTime::now();
        trust.extend(now + std::time::Duration::from_secs(60));
        assert!(trust.holds());
        trust.extend(now - std::time::Duration::from_secs(60));
        assert!(trust.holds(), "an older proof never shortens a newer one");

        let lapsed = super::TrustedUntil::default();
        lapsed.extend(now - std::time::Duration::from_secs(1));
        assert!(!lapsed.holds());
    }

    #[test]
    fn response_cache_accounts_for_large_keys_and_nested_values() {
        let small_key = "search".to_string();
        let large_key = "x".repeat(4096);
        let empty = serde_json::json!([]);
        let nested = serde_json::json!({"results": [{"name": "x".repeat(4096)}]});
        assert!(super::response_cache_weight(&large_key, &empty) >= 4096);
        assert!(super::response_cache_weight(&small_key, &nested) >= 4096);
        assert!(super::response_cache_weight(&small_key, &empty) > 0);

        let cache = moka::sync::Cache::builder()
            .max_capacity(2048)
            .weigher(super::response_cache_weight)
            .build();
        cache.insert(small_key.clone(), empty.clone());
        cache.run_pending_tasks();
        assert_eq!(cache.get(&small_key), Some(empty.clone()));
        cache.insert(large_key.clone(), empty);
        cache.run_pending_tasks();
        assert!(cache.get(&large_key).is_none());
        cache.insert("nested".to_string(), nested);
        cache.run_pending_tasks();
        assert!(cache.get("nested").is_none());
    }

    #[test]
    fn runtime_document_controls_openid_clients_and_clock_skew() {
        let mut document: Value = serde_json::from_str(include_str!(
            "../../../apps/backend/kubernetes/flow-like.config.example.json"
        ))
        .unwrap();
        document["authentication"]["openid"]["issuer"] =
            serde_json::json!("https://runtime-issuer.example.test");
        document["authentication"]["openid"]["client_id"] = serde_json::json!("runtime-client");
        document["authentication"]["openid"]["audience"] = serde_json::json!("runtime-audience");
        document["authentication"]["openid"]["jwks_url"] =
            serde_json::json!("https://runtime-issuer.example.test/keys");
        document["authentication"]["openid"]["additional_client_ids"] =
            serde_json::json!(["second-runtime-client"]);
        document["authentication"]["openid"]["leeway_seconds"] = serde_json::json!(3);
        document["authentication"]["openid"]["cognito"] =
            serde_json::json!({"user_pool_id": "eu-west-1_example"});
        let config = crate::runtime_config::EffectiveConfig::parse(&document.to_string()).unwrap();
        let settings =
            super::openid_validation_settings_for_hub(&config.hub, &config.openid).unwrap();
        assert_eq!(settings.issuer, "https://runtime-issuer.example.test");
        assert_eq!(settings.audience, "runtime-audience");
        assert_eq!(
            settings.jwks_url,
            "https://runtime-issuer.example.test/keys"
        );
        assert_eq!(
            settings.client_ids,
            client_ids(&["runtime-client", "second-runtime-client"])
        );
        assert_eq!(settings.leeway, 3);
        assert!(settings.require_access_token_use);

        document["authentication"]["openid"]
            .as_object_mut()
            .unwrap()
            .remove("additional_client_ids");
        document["authentication"]["openid"]
            .as_object_mut()
            .unwrap()
            .remove("leeway_seconds");
        document["authentication"]["openid"]
            .as_object_mut()
            .unwrap()
            .remove("cognito");
        let config = crate::runtime_config::EffectiveConfig::parse(&document.to_string()).unwrap();
        let settings =
            super::openid_validation_settings_for_hub(&config.hub, &config.openid).unwrap();
        assert_eq!(settings.client_ids, client_ids(&["runtime-client"]));
        assert_eq!(settings.leeway, 60);
        assert!(!settings.require_access_token_use);

        document["authentication"]["openid"]["additional_client_ids"] =
            serde_json::json!([" private-invalid-client-marker "]);
        let config = crate::runtime_config::EffectiveConfig::parse(&document.to_string()).unwrap();
        let error =
            super::openid_validation_settings_for_hub(&config.hub, &config.openid).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private-invalid-client-marker"));
    }

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

    #[cfg(feature = "aws")]
    #[test]
    fn cached_forty_minute_grant_cannot_back_a_new_hour_execution() {
        let required = execution_credential_lifetime("3600", "300", "120", 210).unwrap();
        let mut grant = crate::credentials::aws_credentials::AwsRuntimeCredentials::new(
            "meta",
            "content",
            "logs",
            "us-east-1",
        );
        grant.expiration = Some(chrono::Utc::now() + chrono::Duration::minutes(40));
        assert!(crate::credentials::RuntimeCredentials::Aws(grant.clone()).expires_soon(required));
        grant.expiration = Some(chrono::Utc::now() + chrono::Duration::hours(2));
        assert!(!crate::credentials::RuntimeCredentials::Aws(grant).expires_soon(required));
    }

    #[test]
    fn native_async_grants_cover_queue_age_and_execution_for_every_invoke_mode() {
        use crate::credentials::CredentialsAccess;
        for mode in [
            CredentialsAccess::ServerExecute,
            CredentialsAccess::ShadowExecute,
            CredentialsAccess::InvokeNone,
            CredentialsAccess::InvokeRead,
            CredentialsAccess::InvokeWrite,
        ] {
            for backend in ["lambda_invoke", "LAMBDA_INVOKE", "lambda_sdk"] {
                assert_eq!(
                    native_async_credential_lifetime(&mode, Some(backend), Some("worker"))
                        .num_seconds(),
                    1860
                );
            }
        }
        for (backend, function) in [
            (None, Some("worker")),
            (Some("lambda_stream"), Some("worker")),
            (Some("sqs_event_bridge"), Some("worker")),
            (Some("lambda_invoke"), None),
            (Some("lambda_invoke"), Some("  ")),
        ] {
            assert_eq!(
                native_async_credential_lifetime(
                    &CredentialsAccess::ServerExecute,
                    backend,
                    function,
                )
                .num_seconds(),
                120
            );
        }
        assert_eq!(
            native_async_credential_lifetime(
                &CredentialsAccess::ReadApp,
                Some("lambda_invoke"),
                Some("worker"),
            )
            .num_seconds(),
            120
        );
    }

    #[cfg(feature = "aws")]
    #[test]
    fn native_async_rejects_near_expiry_cached_and_short_fresh_grants() {
        use crate::credentials::{CredentialsAccess, RuntimeCredentials};
        let required = native_async_credential_lifetime(
            &CredentialsAccess::InvokeRead,
            Some("lambda_invoke"),
            Some("worker"),
        );
        let mut grant = crate::credentials::aws_credentials::AwsRuntimeCredentials::new(
            "meta",
            "content",
            "logs",
            "us-east-1",
        );
        // Unknown expiration also fails the native post-mint lifetime check.
        assert!(RuntimeCredentials::Aws(grant.clone()).expires_soon(required));
        for remaining_seconds in [121, 900, 1800] {
            grant.expiration =
                Some(chrono::Utc::now() + chrono::Duration::seconds(remaining_seconds));
            assert!(RuntimeCredentials::Aws(grant.clone()).expires_soon(required));
        }
        grant.expiration = Some(chrono::Utc::now() + chrono::Duration::seconds(3600));
        assert!(!RuntimeCredentials::Aws(grant).expires_soon(required));
        // The existing post-mint check applies whenever the required lifetime
        // exceeds the ordinary cache's 120-second refresh margin.
        assert!(required > chrono::Duration::seconds(120));
    }

    #[test]
    fn hour_run_credentials_cover_queue_wait_and_cleanup() {
        assert_eq!(
            execution_credential_lifetime("3600", "300", "120", 210)
                .unwrap()
                .num_seconds(),
            4230
        );
        assert_eq!(
            execution_credential_lifetime("30", "10", "120", 210)
                .unwrap()
                .num_seconds(),
            370
        );
        for (timeout, queue, margin) in [
            ("0", "300", "120"),
            ("3600", "-1", "120"),
            ("3600", "300", "0"),
            ("3600", "300", "3601"),
            ("overflow", "300", "120"),
        ] {
            assert!(execution_credential_lifetime(timeout, queue, margin, 210).is_err());
        }
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

    const MCP_AUDIENCE: &str = "https://api.example.test/m/server";

    fn mcp_openid_settings() -> OpenIdValidationSettings {
        OpenIdValidationSettings {
            issuer: "https://cognito-idp.eu-west-1.amazonaws.com/eu-west-1_example".into(),
            client_ids: client_ids(&["platform-client", "claude-client"]),
            audience: "platform-client".into(),
            jwks_url: "https://cognito-idp.eu-west-1.amazonaws.com/eu-west-1_example/.well-known/jwks.json".into(),
            tenant_id: None,
            leeway: 0,
            require_access_token_use: true,
        }
    }

    fn mcp_access_claims() -> HashMap<String, Value> {
        claims(&[
            ("iss", Value::String(mcp_openid_settings().issuer)),
            ("sub", Value::String("cognito-user".into())),
            ("aud", Value::String(MCP_AUDIENCE.into())),
            ("client_id", Value::String("claude-client".into())),
            ("token_use", Value::String("access".into())),
            ("exp", Value::from(2_000_i64)),
        ])
    }

    #[test]
    fn mcp_accepts_resource_bound_cognito_access_from_additional_client() {
        let settings = mcp_openid_settings();
        let claims = mcp_access_claims();
        assert_eq!(
            validate_mcp_openid_claims(&claims, &settings, MCP_AUDIENCE, 1_000).unwrap(),
            2_000
        );
        assert_eq!(
            claims.get("sub").and_then(Value::as_str),
            Some("cognito-user")
        );
        // Accepting the resource audience here must not extend platform login validation.
        assert!(
            validate_openid_claims(
                &claims,
                &settings.issuer,
                &settings.client_ids,
                &settings.audience,
                settings.tenant_id,
                1_000,
                settings.leeway,
            )
            .is_err()
        );
    }

    #[test]
    fn mcp_rejects_wrong_issuer_resource_client_subject_and_token_purpose() {
        let settings = mcp_openid_settings();
        for (claim, value) in [
            (
                "iss",
                Value::String("https://untrusted.example/pool".into()),
            ),
            ("aud", Value::String("platform-client".into())),
            (
                "aud",
                Value::String("https://api.example.test/m/other".into()),
            ),
            ("aud", serde_json::json!([MCP_AUDIENCE, 123])),
            ("client_id", Value::String("untrusted-client".into())),
            ("client_id", Value::Null),
            ("sub", Value::String(String::new())),
            ("sub", Value::from(123)),
            ("token_use", Value::String("id".into())),
            ("token_use", Value::Null),
            ("exp", Value::from(1_000_i64)),
            ("nbf", Value::from(1_001_i64)),
        ] {
            let mut claims = mcp_access_claims();
            claims.insert(claim.into(), value.clone());
            assert!(
                validate_mcp_openid_claims(&claims, &settings, MCP_AUDIENCE, 1_000).is_err(),
                "accepted invalid {claim}: {value}",
            );
        }
        for claim in ["iss", "aud", "client_id", "sub", "token_use", "exp"] {
            let mut claims = mcp_access_claims();
            claims.remove(claim);
            assert!(
                validate_mcp_openid_claims(&claims, &settings, MCP_AUDIENCE, 1_000).is_err(),
                "accepted missing {claim}",
            );
        }
    }

    #[test]
    fn mcp_accepts_oidc_authorized_party_and_rejects_conflicting_client_claims() {
        let mut settings = mcp_openid_settings();
        settings.require_access_token_use = false;
        for client_claim in ["client_id", "azp", "appid"] {
            let mut claims = mcp_access_claims();
            claims.remove("token_use");
            claims.remove("client_id");
            claims.insert(client_claim.into(), Value::String("claude-client".into()));
            assert!(validate_mcp_openid_claims(&claims, &settings, MCP_AUDIENCE, 1_000).is_ok());
            for conflicting_claim in ["client_id", "azp", "appid"] {
                let mut conflicting = claims.clone();
                conflicting.insert(
                    conflicting_claim.into(),
                    Value::String("untrusted-client".into()),
                );
                assert!(
                    validate_mcp_openid_claims(&conflicting, &settings, MCP_AUDIENCE, 1_000)
                        .is_err()
                );
            }
            claims.insert("token_use".into(), Value::String("id".into()));
            assert!(validate_mcp_openid_claims(&claims, &settings, MCP_AUDIENCE, 1_000).is_err());
        }
        let claims = mcp_access_claims();
        for audience in ["", " https://api.example.test/m/server"] {
            assert!(validate_mcp_openid_claims(&claims, &settings, audience, 1_000).is_err());
        }
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

    #[tokio::test]
    async fn node_catalog_registers_every_node_of_a_built_catalog() {
        let node = flow_like::flow::node::Node::new("catalog_probe", "Probe", "Probe", "Test");
        let logic: std::sync::Arc<dyn flow_like::flow::node::NodeLogic> =
            std::sync::Arc::new(crate::execution::wasm_node_stubs::WasmNodeStub::new(node));
        let catalog = std::sync::Arc::new(vec![logic]);

        let (nodes, registry) = super::NodeCatalog::from(catalog.clone()).ready().await;

        assert!(std::sync::Arc::ptr_eq(&nodes, &catalog));
        assert!(registry.get_node("catalog_probe").is_ok());
    }

    #[tokio::test]
    #[should_panic(expected = "catalog build failed")]
    async fn node_catalog_resumes_a_build_panic_at_the_await() {
        super::NodeCatalog::spawn(|| panic!("catalog build failed"))
            .ready()
            .await;
    }
}
