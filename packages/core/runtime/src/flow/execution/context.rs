use super::{
    EventTrigger, ExecutionEnvironment, ExecutionMode, InternalNode, LogLevel, Run, RunPayload,
    byte_store::{self, BYTE_STORE_RESOURCE, ByteScope, ByteStore, INLINE_BYTES_MAX},
    function_lookup::{FUNCTION_LOOKUP_RESOURCE, RunFunctionLookup},
    internal_pin::InternalPin,
    log::LogMessage,
    trace::Trace,
};
use crate::a2ui::ElementCache;
use crate::models::llm::ModelUsageContext;
use crate::{
    credentials::SharedCredentials,
    flow::{
        board::{Board, ExecutionStage},
        node::{Node, NodeState},
        oauth::OAuthToken,
        pin::{PinType, ValueType},
        utils::evaluate_pin_value,
        variable::{Variable, VariableType},
    },
    profile::Profile,
    state::{FlowLikeState, FlowLikeStores, ProgressEvent, ToastEvent, ToastLevel},
};
use ahash::{AHashMap, AHashSet};
use flow_like_model_provider::provider::ModelProviderConfiguration;
use flow_like_storage::{files::store::FlowLikeStore, object_store::path::Path};
use flow_like_types::channel::{Channel, ChannelOutcome};
use flow_like_types::intercom::{InterComCallback, InterComEvent};
use flow_like_types::json::Map;
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{Bytes, Value};
use flow_like_types::{
    Cacheable,
    json::from_value,
    sync::{Mutex, RwLock},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const A2UI_UPDATE_LOG_KEY: &str = "__a2ui_update_log";
/// Backstop against high-frequency streaming loops (e.g. sprite/chart updates)
/// retaining every payload for the whole run. Chat flows stay far below this.
const A2UI_UPDATE_LOG_CAP: usize = 1024;

fn validate_live_resource_url(
    url: &flow_like_types::reqwest::Url,
) -> Result<(), flow_like_types::authorization::AuthorizationError> {
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    let path = url.path().to_ascii_lowercase();
    if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path().contains('\\')
        || ["%2f", "%5c", "%25"]
            .iter()
            .any(|escape| path.contains(escape))
    {
        return Err(flow_like_types::authorization::AuthorizationError::InvalidRequest);
    }
    Ok(())
}

pub(super) fn fresh_local_variable_scope(
    function_variables: &std::collections::HashMap<String, Variable>,
) -> Arc<Mutex<AHashMap<String, Variable>>> {
    let mut local_vars = AHashMap::with_capacity(function_variables.len());
    for (var_id, var) in function_variables {
        let value = match &var.default_value {
            Some(bytes) => flow_like_types::json::from_slice::<Value>(bytes).unwrap_or(Value::Null),
            None => Value::Null,
        };
        let mut cloned_var = var.clone();
        cloned_var.value = Arc::new(Mutex::new(value));
        local_vars.insert(var_id.clone(), cloned_var);
    }
    Arc::new(Mutex::new(local_vars))
}

/// Run-scoped, ordered log of surface-mutating a2ui messages. Shared across
/// nodes via the execution cache so snapshot consumers (e.g. Push Widget) can
/// replay updates emitted earlier in the run. The lock is never held across an
/// await, so a sync mutex suffices.
#[derive(Clone, Default)]
pub struct A2UIUpdateLog {
    pub entries: Arc<std::sync::Mutex<Vec<crate::a2ui::A2UIServerMessage>>>,
    pub truncated: Arc<std::sync::atomic::AtomicBool>,
}

impl A2UIUpdateLog {
    pub fn is_truncated(&self) -> bool {
        self.truncated.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Cacheable for A2UIUpdateLog {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

const A2UI_DETACHED_KEY: &str = "__a2ui_detached_elements";

/// Element ids this run detached children from. Unlike [`A2UIUpdateLog`] it is never capped:
/// a missed id would leave that container's dropped children on the page for good.
#[derive(Clone, Default)]
struct A2UIDetachedElements {
    ids: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
}

impl Cacheable for A2UIDetachedElements {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn detaching_element_id(message: &crate::a2ui::A2UIServerMessage) -> Option<&str> {
    use crate::a2ui::A2UIServerMessage as Msg;
    match message {
        Msg::UpsertElement { element_id, value } => matches!(
            value.get("type").and_then(Value::as_str),
            Some("clearChildren" | "removeChildAt")
        )
        .then_some(element_id.as_str()),
        Msg::RemoveElement { element_id, .. } => Some(element_id.as_str()),
        _ => None,
    }
}

/// The run's closing `pruneDetached`, sent only when the run detached children. Delivery is
/// best effort: a page that misses it keeps the detached components, as before.
pub(super) async fn stream_prune_detached(
    cache: &RwLock<AHashMap<String, Arc<dyn Cacheable>>>,
    callback: &InterComCallback,
) {
    let element_ids: Vec<String> = {
        let cache = cache.read().await;
        let Some(detached) = cache
            .get(A2UI_DETACHED_KEY)
            .and_then(|c| c.as_any().downcast_ref::<A2UIDetachedElements>().cloned())
        else {
            return;
        };
        detached
            .ids
            .lock()
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    };
    if element_ids.is_empty() {
        return;
    }
    let message = crate::a2ui::A2UIServerMessage::prune_detached(element_ids);
    if let Err(err) = InterComEvent::with_type("a2ui", message)
        .call(callback)
        .await
    {
        tracing::warn!(error = %err, "Failed to stream the run's pruneDetached message");
    }
}

#[derive(Clone)]
pub struct ExecutionContextCache {
    pub stores: FlowLikeStores,
    pub app_id: String,
    pub model_usage_app_id: Option<String>,
    pub board_dir: Path,
    pub board_id: String,
    pub node_id: Arc<str>,
    pub sub: String,
    /// Shadow/replay isolation: `stores` already carries read-only wrappers for
    /// app storage, user store and app meta store; consumers that resolve their
    /// own store (credential bypasses) must check this flag and wrap too.
    pub shadow: bool,
}

/// The stores a shadow run may not write. A write through any of them fails
/// loudly instead of silently no-oping — a fabricated success would make a
/// shadow-run diff a lie. The temporary store stays writable (scratch) and the
/// log store keeps recording the run itself.
fn shadow_isolated_stores(mut stores: FlowLikeStores) -> FlowLikeStores {
    stores.app_storage_store = stores.app_storage_store.map(|store| store.read_only());
    stores.user_store = stores.user_store.map(|store| store.read_only());
    stores.app_meta_store = stores.app_meta_store.map(|store| store.read_only());
    stores
}

impl ExecutionContextCache {
    pub async fn new(
        run: &Weak<Mutex<Run>>,
        state: &Arc<FlowLikeState>,
        node_id: Arc<str>,
    ) -> Option<Self> {
        let (app_id, model_usage_app_id, board_dir, board_id, sub, shadow) = match run.upgrade() {
            Some(run) => {
                let run = run.lock().await;
                let app_id = run.app_id.clone();
                let model_usage_app_id = run.model_usage_app_id.clone();
                let board = &run.board;
                let sub = run.sub.clone();
                (
                    app_id,
                    model_usage_app_id,
                    board.board_dir.clone(),
                    board.id.clone(),
                    sub,
                    run.shadow,
                )
            }
            None => return None,
        };

        let mut stores = state.config.read().await.stores.clone();
        if shadow {
            stores = shadow_isolated_stores(stores);
        }

        Some(ExecutionContextCache {
            stores,
            app_id,
            model_usage_app_id,
            board_dir,
            board_id,
            node_id,
            sub,
            shadow,
        })
    }

    /// Create ExecutionContextCache from cached RunMeta to avoid locking
    pub async fn from_meta(
        meta: &super::RunMeta,
        state: &Arc<FlowLikeState>,
        node_id: Arc<str>,
    ) -> Self {
        let mut stores = state.config.read().await.stores.clone();
        if meta.shadow {
            stores = shadow_isolated_stores(stores);
        }

        ExecutionContextCache {
            stores,
            app_id: meta.app_id.clone(),
            model_usage_app_id: meta.model_usage_app_id.clone(),
            board_dir: meta.board_dir.clone(),
            board_id: meta.board_id.clone(),
            node_id,
            sub: meta.sub.clone(),
            shadow: meta.shadow,
        }
    }

    fn for_node(&self, node_id: Arc<str>) -> Self {
        let mut cache = self.clone();
        cache.node_id = node_id;
        cache
    }

    pub fn get_user_dir(&self, node: bool) -> flow_like_types::Result<Path> {
        let base = Path::from("users")
            .join(self.sub.clone())
            .join("apps")
            .join(self.app_id.clone());
        if !node {
            return Ok(base);
        }

        Ok(base.join(self.node_id.as_ref()))
    }

    pub fn get_cache(&self, node: bool, user: bool) -> flow_like_types::Result<Path> {
        let base = if matches!(self.stores.temporary_store, Some(FlowLikeStore::Local(_))) {
            let base = Path::from("tmp");
            let base = if user {
                base.join("user").join(self.sub.clone())
            } else {
                base.join("global")
            };
            base.join("apps").join(self.app_id.clone())
        } else {
            let (user_prefix, global_prefix) =
                flow_like_types::storage_paths::temporary_prefixes(&self.sub, &self.app_id);
            Path::from(if user { user_prefix } else { global_prefix })
        };

        if !node {
            return Ok(base);
        }

        Ok(base.join(self.node_id.as_ref()))
    }

    pub fn get_storage(&self, node: bool) -> flow_like_types::Result<Path> {
        let base = self.board_dir.clone().join("storage");

        if !node {
            return Ok(base);
        }

        Ok(base.join(self.node_id.as_ref()))
    }

    pub fn get_upload_dir(&self) -> flow_like_types::Result<Path> {
        let base = self.board_dir.clone().join("upload");
        Ok(base)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
enum RunUpdateEventMethod {
    Add,
    Remove,
    Update,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct RunUpdateEvent {
    run_id: String,
    node_ids: Vec<String>,
    method: RunUpdateEventMethod,
}

/// How long a run waits for the page to answer an automatic element read.
const ELEMENT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct ExecutionContext {
    pub executor_payment_auth: Option<super::ExecutorPaymentAuth>,
    pub executor_api_auth: Option<super::ExecutorApiAuth>,
    pub id: Arc<str>,
    pub run: Weak<Mutex<Run>>,
    pub nodes: Arc<AHashMap<String, Arc<InternalNode>>>,
    pub profile: Arc<Profile>,
    pub node: Arc<InternalNode>,
    pub sub_traces: Vec<Trace>,
    pub app_state: Arc<FlowLikeState>,
    pub variables: Arc<Mutex<AHashMap<String, Variable>>>,
    /// Function-scoped local variables. Checked before global `variables` during resolution.
    pub local_variables: Option<Arc<Mutex<AHashMap<String, Variable>>>>,
    pub started_by: Option<Vec<Arc<InternalPin>>>,
    pub cache: Arc<RwLock<AHashMap<String, Arc<dyn Cacheable>>>>,
    pub stage: ExecutionStage,
    pub log_level: LogLevel,
    pub trace: Trace,
    pub execution_cache: Option<ExecutionContextCache>,
    pub completion_callbacks: Arc<RwLock<Vec<EventTrigger>>>,
    pub stream_state: bool,
    pub token: Option<String>,
    pub credentials: Option<Arc<SharedCredentials>>,
    pub delegated: bool,
    pub context_state: BTreeMap<String, Value>,
    /// Values are shared, not owned: `create_sub_context` clones this map once per node
    /// execution and `push_sub_context` drops a colliding entry per merged key. Holding
    /// the JSON behind an `Arc` makes both costs proportional to the key count instead of
    /// the bytes of JSON in scope, which is what a loop-heavy function scope accumulates.
    pub context_pin_overrides: Option<BTreeMap<String, Arc<Value>>>,
    pub result: Option<Value>,
    pub oauth_tokens: Arc<AHashMap<String, OAuthToken>>,
    /// Reply conduit to the run's client; `None` only for contexts built without a run.
    pub channel: Option<Arc<dyn Channel>>,
    /// The run's elements: seeded from `payload._elements`, grown by element requests.
    pub elements: Arc<RwLock<ElementCache>>,
    /// Package runtimes and host resources shared only with contexts of this run.
    pub resources: Arc<super::resources::RunResources>,
    /// User context containing information about who triggered the execution
    pub user_context: Option<super::UserExecutionContext>,
    /// Set while a handler invocation runs in its own byte scope; sub contexts inherit it.
    byte_scope: Option<Arc<ByteStore>>,
    nodes_executed: Arc<AtomicU64>,
    represented_trace_nodes: AHashSet<Arc<str>>,
    trace_taken: bool,
    log_spill_threshold: usize,
    log_flush_interval: Duration,
    last_log_spill: Instant,
    cancellation_token: Option<CancellationToken>,
    run_id: String,
    execution_environment: ExecutionEnvironment,
    execution_mode: ExecutionMode,
    state: NodeState,
    callback: InterComCallback,
    function_lookup: Option<Arc<RunFunctionLookup>>,
}

impl ExecutionContext {
    /// Descendant contexts share the provider through their execution state.
    pub fn request_authorizer(
        &self,
    ) -> Option<&Arc<dyn flow_like_types::authorization::RequestAuthorizer>> {
        self.app_state.request_authorizer.as_ref()
    }

    pub async fn authorize_request(
        &self,
        mut request: flow_like_types::reqwest::Request,
        audience: flow_like_types::authorization::ResourceAudience,
    ) -> flow_like_types::Result<flow_like_types::reqwest::Request> {
        use flow_like_types::{
            authorization::AuthorizationRequest,
            reqwest::header::{AUTHORIZATION, HeaderValue},
        };
        if let Some(provider) = self.request_authorizer() {
            validate_live_resource_url(request.url())?;
            request.headers_mut().remove(AUTHORIZATION);
            request.headers_mut().remove("dpop");
            let authorization = provider
                .authorize(AuthorizationRequest {
                    audience,
                    method: request.method().as_str(),
                    url: request.url().as_str(),
                })
                .await?;
            authorization.validate()?;
            let mut value = HeaderValue::from_str(authorization.authorization())?;
            value.set_sensitive(true);
            request.headers_mut().insert(AUTHORIZATION, value);
            if let Some(proof) = authorization.dpop() {
                let mut value = HeaderValue::from_str(proof)?;
                value.set_sensitive(true);
                request.headers_mut().insert("dpop", value);
            }
        }
        Ok(request)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        nodes: Arc<AHashMap<String, Arc<InternalNode>>>,
        run: &Weak<Mutex<Run>>,
        state: &Arc<FlowLikeState>,
        node: &Arc<InternalNode>,
        variables: &Arc<Mutex<AHashMap<String, Variable>>>,
        cache: &Arc<RwLock<AHashMap<String, Arc<dyn Cacheable>>>>,
        log_level: LogLevel,
        stage: ExecutionStage,
        profile: Arc<Profile>,
        callback: InterComCallback,
        completion_callbacks: Arc<RwLock<Vec<EventTrigger>>>,
        credentials: Option<Arc<SharedCredentials>>,
        token: Option<String>,
        oauth_tokens: Arc<AHashMap<String, OAuthToken>>,
        channel: Option<Arc<dyn Channel>>,
    ) -> Self {
        // Use cached node_id instead of locking
        let id = node.shared_node_id();
        let execution_cache = ExecutionContextCache::new(run, state, id.clone()).await;

        let mut trace = Trace::new_shared(id.clone());
        if log_level == LogLevel::Debug {
            trace.snapshot_variables(variables).await;
        }

        let (
            executor_payment_auth,
            executor_api_auth,
            run_id,
            stream_state,
            log_spill_threshold,
            log_flush_interval,
            nodes_executed,
            elements,
            resources,
        ) = match run.upgrade() {
            Some(run) => {
                let run = run.lock().await;
                (
                    run.executor_payment_auth.clone(),
                    run.executor_api_auth.clone(),
                    run.id.clone(),
                    run.stream_state,
                    run.log_spill_threshold,
                    run.log_flush_interval,
                    run.nodes_executed.clone(),
                    run.elements.clone(),
                    run.resources.clone(),
                )
            }
            None => (
                None,
                None,
                "".to_string(),
                false,
                super::DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD,
                super::DEFAULT_RUN_LOG_FLUSH_INTERVAL,
                Arc::new(AtomicU64::new(0)),
                Arc::new(RwLock::new(ElementCache::default())),
                {
                    // A detached context has no owner to close live resources.
                    let resources = Arc::new(super::resources::RunResources::default());
                    resources.abort();
                    resources
                },
            ),
        };
        ExecutionContext {
            executor_payment_auth,
            executor_api_auth,
            id,
            run_id,
            elements,
            function_lookup: resources.get(FUNCTION_LOOKUP_RESOURCE),
            resources,
            execution_environment: state.execution_environment,
            execution_mode: ExecutionMode::Sync,
            started_by: None,
            run: run.clone(),
            app_state: state.clone(),
            node: node.clone(),
            variables: variables.clone(),
            local_variables: None,
            cache: cache.clone(),
            log_level,
            stage,
            sub_traces: vec![],
            trace,
            profile,
            callback,
            token,
            execution_cache,
            stream_state,
            state: NodeState::Idle,
            context_state: BTreeMap::new(),
            nodes,
            completion_callbacks,
            credentials,
            context_pin_overrides: None,
            result: None,
            delegated: false,
            oauth_tokens,
            channel,
            cancellation_token: None,
            user_context: None,
            byte_scope: None,
            nodes_executed,
            represented_trace_nodes: AHashSet::new(),
            trace_taken: false,
            log_spill_threshold,
            log_flush_interval,
            last_log_spill: Instant::now(),
        }
    }
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub(crate) fn increment_nodes_executed(&self) {
        self.nodes_executed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn execution_environment(&self) -> ExecutionEnvironment {
        self.execution_environment
    }

    pub fn execution_mode(&self) -> ExecutionMode {
        self.execution_mode
    }

    pub fn model_usage_context(&self) -> Option<ModelUsageContext> {
        let cache = self.execution_cache.as_ref()?;
        Some(ModelUsageContext {
            app_id: cache.model_usage_app_id.clone(),
            run_id: Some(self.run_id.clone()),
            api_base_url: crate::hub::hub_origin(&self.profile.hub, self.profile.secure),
        })
    }

    pub fn callback(&self) -> &InterComCallback {
        &self.callback
    }

    /// Isolate a request's event delivery from a reusable execution context.
    /// Descendants inherit the callback; dropping this clone leaves the source unchanged.
    pub fn clone_with_callback(&self, callback: InterComCallback) -> Self {
        let mut context = self.clone();
        context.callback = callback;
        context
    }

    pub async fn event_id(&self) -> Option<String> {
        let run = self.run.upgrade()?;
        let run = run.lock().await;
        run.event_id.clone()
    }

    /// Create ExecutionContext using cached RunMeta to avoid locking Run
    #[allow(clippy::too_many_arguments)]
    pub async fn with_meta(
        nodes: Arc<AHashMap<String, Arc<InternalNode>>>,
        run: &Weak<Mutex<Run>>,
        run_meta: &super::RunMeta,
        state: &Arc<FlowLikeState>,
        node: &Arc<InternalNode>,
        variables: &Arc<Mutex<AHashMap<String, Variable>>>,
        cache: &Arc<RwLock<AHashMap<String, Arc<dyn Cacheable>>>>,
        log_level: LogLevel,
        stage: ExecutionStage,
        profile: Arc<Profile>,
        callback: InterComCallback,
        completion_callbacks: Arc<RwLock<Vec<EventTrigger>>>,
        credentials: Option<Arc<SharedCredentials>>,
        token: Option<String>,
        oauth_tokens: Arc<AHashMap<String, OAuthToken>>,
        channel: Option<Arc<dyn Channel>>,
    ) -> Self {
        // Use cached node_id instead of locking
        let id = node.shared_node_id();
        // Use RunMeta directly instead of locking Run
        let execution_cache = ExecutionContextCache::from_meta(run_meta, state, id.clone()).await;

        let mut trace = Trace::new_shared(id.clone());
        if log_level == LogLevel::Debug {
            trace.snapshot_variables(variables).await;
        }
        ExecutionContext {
            executor_payment_auth: run_meta.executor_payment_auth.clone(),
            executor_api_auth: run_meta.executor_api_auth.clone(),
            id,
            run_id: run_meta.run_id.clone(),
            elements: run_meta.elements.clone(),
            resources: run_meta.resources.clone(),
            function_lookup: run_meta.resources.get(FUNCTION_LOOKUP_RESOURCE),
            execution_environment: run_meta.environment,
            execution_mode: run_meta.execution_mode,
            started_by: None,
            run: run.clone(),
            app_state: state.clone(),
            node: node.clone(),
            variables: variables.clone(),
            local_variables: None,
            cache: cache.clone(),
            log_level,
            stage,
            sub_traces: vec![],
            trace,
            profile,
            callback,
            token,
            execution_cache: Some(execution_cache),
            stream_state: run_meta.stream_state,
            state: NodeState::Idle,
            context_state: BTreeMap::new(),
            nodes,
            completion_callbacks,
            credentials,
            context_pin_overrides: None,
            result: None,
            delegated: false,
            oauth_tokens,
            channel,
            cancellation_token: None,
            user_context: None,
            byte_scope: None,
            nodes_executed: run_meta.nodes_executed.clone(),
            represented_trace_nodes: AHashSet::new(),
            trace_taken: false,
            log_spill_threshold: run_meta.log_spill_threshold,
            log_flush_interval: run_meta.log_flush_interval,
            last_log_spill: Instant::now(),
        }
    }

    #[inline]
    pub fn started_by_first(&self) -> Option<Arc<InternalPin>> {
        self.started_by.as_ref().and_then(|v| v.first().cloned())
    }

    /// Resolve node pins against shared compiled topology while reading this
    /// run's live pin instances. Detached contexts retain node-scan semantics.
    pub fn runtime_pin_by_id(&self, id: &str) -> Option<Arc<InternalPin>> {
        if let Some(lookup) = &self.function_lookup
            && lookup.matches_nodes(&self.nodes)
        {
            match lookup.topology.runtime_pins.get(id) {
                Some(Some(index)) => return lookup.pins.get(*index as usize).cloned(),
                None => return None,
                // Duplicate IDs at different arena positions must preserve the
                // first matching node in this run's map iteration order.
                Some(None) => {}
            }
        }
        self.nodes
            .values()
            .find_map(|node| node.get_pin_by_id(id).ok())
    }

    /// Find the definition owning a function pin, preferring layer-local nodes.
    pub fn function_pin_owner(
        &self,
        board: &Board,
        layer_id: &str,
        pin_id: &str,
    ) -> Option<String> {
        if let Some(lookup) = &self.function_lookup
            && std::ptr::eq(board, lookup.topology.board.as_ref())
        {
            return lookup
                .topology
                .definition_owners
                .get(layer_id)?
                .get(pin_id)
                .map(|owner| owner.to_string());
        }
        let layer = board.layers.get(layer_id)?;
        layer
            .nodes
            .values()
            .find(|node| node.pins.contains_key(pin_id))
            .or_else(|| {
                board.nodes.values().find(|node| {
                    node.layer.as_deref() == Some(layer_id) && node.pins.contains_key(pin_id)
                })
            })
            .map(|node| node.id.clone())
    }

    pub fn set_result(&mut self, value: Value) {
        self.result = Some(value);
    }

    pub fn override_pin_value(&mut self, pin_id: &str, value: Value) {
        self.override_pin_value_shared(pin_id, Arc::new(value));
    }

    pub fn override_pin_value_shared(&mut self, pin_id: &str, value: Arc<Value>) {
        self.context_pin_overrides
            .get_or_insert_with(BTreeMap::new)
            .insert(pin_id.to_string(), value);
    }

    /// Mirror a pin value into an already active isolation scope.
    ///
    /// Sequential loops publish their iteration pins straight onto the shared pin cell,
    /// which is correct on a linear exec path but leaks across siblings once the loop
    /// runs inside a parallel body or a function invocation. Mirroring keeps those reads
    /// branch-private. No scope is created here: without one, the surrounding path
    /// already expects shared-pin semantics.
    pub fn override_pin_value_if_active(&mut self, pin_id: &str, value: &Value) {
        if let Some(overrides) = &mut self.context_pin_overrides {
            overrides.insert(pin_id.to_string(), Arc::new(value.clone()));
        }
    }

    pub fn clear_pin_override(&mut self, pin_id: &str) {
        if let Some(overrides) = &mut self.context_pin_overrides {
            overrides.remove(pin_id);
        }
    }

    pub fn clear_all_pin_overrides(&mut self) {
        if let Some(overrides) = &mut self.context_pin_overrides {
            overrides.clear();
        }
    }

    /// Set the user execution context
    pub fn set_user_context(&mut self, user_context: super::UserExecutionContext) {
        self.user_context = Some(user_context);
    }

    /// Get the user execution context, returning an error if not set
    pub fn require_user_context(&self) -> flow_like_types::Result<&super::UserExecutionContext> {
        self.user_context
            .as_ref()
            .ok_or_else(|| flow_like_types::anyhow!("User context not available - this execution was triggered by a sink that does not support user context"))
    }

    /// Get the user execution context if available
    pub fn user_context(&self) -> Option<&super::UserExecutionContext> {
        self.user_context.as_ref()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token
            .as_ref()
            .is_some_and(|t| t.is_cancelled())
    }

    pub fn check_cancelled(&self) -> flow_like_types::Result<()> {
        if self.is_cancelled() {
            return Err(flow_like_types::anyhow!("Execution was cancelled"));
        }
        Ok(())
    }

    /// Run a long-running async operation that can be cancelled.
    /// If the context's cancellation token is triggered, the operation will be aborted.
    /// Use this for expensive operations like file parsing, API calls, etc.
    pub async fn run_cancellable<F, T>(&self, future: F) -> flow_like_types::Result<T>
    where
        F: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        use flow_like_types::tokio;

        if let Some(token) = &self.cancellation_token {
            // Spawn the future so we can abort it when cancelled
            let handle = tokio::spawn(future);
            let abort_handle = handle.abort_handle();

            tokio::select! {
                biased;
                _ = token.cancelled() => {
                    abort_handle.abort();
                    Err(flow_like_types::anyhow!("Execution was cancelled"))
                }
                result = handle => {
                    result.map_err(|e| {
                        if e.is_cancelled() {
                            flow_like_types::anyhow!("Execution was cancelled")
                        } else {
                            flow_like_types::anyhow!("Task failed: {}", e)
                        }
                    })
                }
            }
        } else {
            // No cancellation token - just run directly without spawning
            Ok(future.await)
        }
    }

    pub fn get_cancellation_token(&self) -> Option<CancellationToken> {
        self.cancellation_token.clone()
    }

    pub fn set_cancellation_token(&mut self, token: CancellationToken) {
        self.cancellation_token = Some(token);
    }

    pub async fn create_sub_context(&self, node: &Arc<InternalNode>) -> ExecutionContext {
        self.create_sub_context_with_pin_overrides(node, self.context_pin_overrides.clone())
            .await
    }

    async fn create_sub_context_with_pin_overrides(
        &self,
        node: &Arc<InternalNode>,
        context_pin_overrides: Option<BTreeMap<String, Arc<Value>>>,
    ) -> ExecutionContext {
        let id = node.shared_node_id();
        let execution_cache = self
            .execution_cache
            .as_ref()
            .map(|cache| cache.for_node(id.clone()));
        let mut trace = Trace::new_shared(id.clone());
        if self.log_level == LogLevel::Debug {
            trace.snapshot_variables(&self.variables).await;
        }
        ExecutionContext {
            executor_payment_auth: self.executor_payment_auth.clone(),
            executor_api_auth: self.executor_api_auth.clone(),
            id,
            run: self.run.clone(),
            elements: self.elements.clone(),
            resources: self.resources.clone(),
            function_lookup: self.function_lookup.clone(),
            nodes: self.nodes.clone(),
            profile: self.profile.clone(),
            node: node.clone(),
            sub_traces: Vec::new(),
            app_state: self.app_state.clone(),
            variables: self.variables.clone(),
            local_variables: self.local_variables.clone(),
            started_by: None,
            cache: self.cache.clone(),
            stage: self.stage.clone(),
            log_level: self.log_level,
            trace,
            execution_cache,
            completion_callbacks: self.completion_callbacks.clone(),
            stream_state: self.stream_state,
            token: self.token.clone(),
            credentials: self.credentials.clone(),
            delegated: false,
            context_state: BTreeMap::new(),
            context_pin_overrides,
            result: None,
            oauth_tokens: self.oauth_tokens.clone(),
            channel: self.channel.clone(),
            user_context: self.user_context.clone(),
            byte_scope: self.byte_scope.clone(),
            nodes_executed: self.nodes_executed.clone(),
            represented_trace_nodes: AHashSet::new(),
            trace_taken: false,
            log_spill_threshold: self.log_spill_threshold,
            log_flush_interval: self.log_flush_interval,
            last_log_spill: Instant::now(),
            cancellation_token: self.cancellation_token.clone(),
            run_id: self.run_id.clone(),
            execution_environment: self.execution_environment,
            execution_mode: self.execution_mode,
            state: NodeState::Idle,
            callback: self.callback.clone(),
        }
    }

    /// Create a sub-context for function execution with isolated local variables.
    /// The function's local variables are cloned with fresh values so parallel
    /// invocations of the same function don't interfere with each other.
    pub async fn create_function_context(
        &self,
        node: &Arc<InternalNode>,
        function_variables: &std::collections::HashMap<String, Variable>,
    ) -> ExecutionContext {
        let mut context = self
            .create_sub_context_with_pin_overrides(node, Some(BTreeMap::new()))
            .await;

        context.local_variables = Some(fresh_local_variable_scope(function_variables));

        context
    }

    pub async fn get_variable(&self, variable_id: &str) -> flow_like_types::Result<Variable> {
        // Check local (function-scoped) variables first
        if let Some(local) = &self.local_variables
            && let Some(variable) = local.lock().await.get(variable_id).cloned()
        {
            return Ok(variable);
        }

        if let Some(variable) = self.variables.lock().await.get(variable_id).cloned() {
            return Ok(variable);
        }

        Err(flow_like_types::anyhow!("Variable not found"))
    }

    /// Resolve only the runtime fields needed to read a variable value.
    ///
    /// This avoids cloning the variable's descriptive metadata on hot read paths.
    /// The boolean marks values that must be omitted from diagnostic logs.
    pub async fn get_variable_value_ref(
        &self,
        variable_id: &str,
    ) -> flow_like_types::Result<(Arc<Mutex<Value>>, bool)> {
        if let Some(local) = &self.local_variables {
            let local = local.lock().await;
            if let Some(variable) = local.get(variable_id) {
                return Ok((
                    variable.value.clone(),
                    variable.secret || variable.runtime_configured,
                ));
            }
        }

        let variables = self.variables.lock().await;
        let variable = variables
            .get(variable_id)
            .ok_or_else(|| flow_like_types::anyhow!("Variable not found"))?;
        Ok((
            variable.value.clone(),
            variable.secret || variable.runtime_configured,
        ))
    }

    pub async fn get_payload(&self) -> flow_like_types::Result<Arc<RunPayload>> {
        let payload = self
            .run
            .upgrade()
            .ok_or_else(|| flow_like_types::anyhow!("Run not found"))?
            .lock()
            .await
            .payload
            .clone();

        if payload.id.as_str() == self.id.as_ref() {
            return Ok(payload);
        }
        Err(flow_like_types::anyhow!("Payload not found"))
    }

    pub async fn get_board(&self) -> flow_like_types::Result<Arc<super::super::board::Board>> {
        let board = self
            .run
            .upgrade()
            .ok_or_else(|| flow_like_types::anyhow!("Run not found"))?
            .lock()
            .await
            .board
            .clone();
        Ok(board)
    }

    /// Returns the run's payload without checking if this node is the entry point.
    /// Use this for nodes that need to access payload data (like _elements) regardless
    /// of where they are in the execution flow.
    pub async fn get_run_payload(&self) -> flow_like_types::Result<Arc<RunPayload>> {
        let payload = self
            .run
            .upgrade()
            .ok_or_else(|| flow_like_types::anyhow!("Run not found"))?
            .lock()
            .await
            .payload
            .clone();

        Ok(payload)
    }

    /// One element the run holds, resolved by the `_elements` key rules (exact, suffix, page
    /// retarget) or as a widget child inside its host's inline definition. A miss asks the live
    /// page once per id when the client declared `_elements_mode: "demand"`. Returns the
    /// resolved key alongside the element.
    pub async fn read_element(
        &mut self,
        element_id: &str,
    ) -> flow_like_types::Result<Option<(String, Value)>> {
        if let Some(hit) = self.elements.read().await.resolve_present(element_id) {
            return Ok(Some(hit));
        }
        if self.ensure_elements(&[element_id.to_string()]).await?
            && let Some(hit) = self.elements.read().await.resolve_present(element_id)
        {
            return Ok(Some(hit));
        }
        Ok(self.elements.read().await.resolve(element_id))
    }

    /// Borrow every element the run holds, for type/pattern queries and parent lookups.
    pub async fn with_elements<R>(
        &self,
        f: impl FnOnce(&Map<String, Value>) -> R,
    ) -> flow_like_types::Result<R> {
        let cache = self.elements.read().await;
        Ok(f(cache.elements()))
    }

    /// Ask the live page for the selectors this run has not requested yet. Returns whether a
    /// request was made; a no-op unless the client declared `_elements_mode: "demand"`.
    pub async fn ensure_elements(&mut self, selectors: &[String]) -> flow_like_types::Result<bool> {
        let pending = {
            let mut cache = self.elements.write().await;
            if !cache.on_demand() {
                return Ok(false);
            }
            cache.take_unrequested(selectors)
        };
        if pending.is_empty() {
            return Ok(false);
        }
        if let Err(err) = self
            .request_elements(pending.clone(), ELEMENT_REQUEST_TIMEOUT)
            .await
        {
            self.elements.write().await.forget_requested(&pending);
            return Err(err);
        }
        Ok(true)
    }

    /// Returns the current route from the run payload.
    pub async fn get_frontend_route(&self) -> flow_like_types::Result<Option<String>> {
        let payload = self.get_run_payload().await?;
        let route = payload
            .payload
            .as_ref()
            .and_then(|p| p.get("_route"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok(route)
    }

    /// Returns the route parameters from the run payload.
    pub async fn get_frontend_route_params(&self) -> flow_like_types::Result<Option<Value>> {
        let payload = self.get_run_payload().await?;
        let params = payload
            .payload
            .as_ref()
            .and_then(|p| p.get("_route_params"))
            .cloned();
        Ok(params)
    }

    /// Returns the query parameters from the run payload.
    pub async fn get_frontend_query_params(&self) -> flow_like_types::Result<Option<Value>> {
        let payload = self.get_run_payload().await?;
        let params = payload
            .payload
            .as_ref()
            .and_then(|p| p.get("_query_params"))
            .cloned();
        Ok(params)
    }

    pub async fn hook_completion_event(&mut self, cb: EventTrigger) {
        let mut callbacks = self.completion_callbacks.write().await;
        callbacks.push(cb);
    }

    pub async fn set_variable(&self, variable: Variable) {
        // If local variables exist and contain this variable, update there
        if let Some(local) = &self.local_variables {
            let mut local_vars = local.lock().await;
            if local_vars.contains_key(&variable.id) {
                local_vars.insert(variable.id.clone(), variable);
                return;
            }
        }

        let mut variables = self.variables.lock().await;
        variables.insert(variable.id.clone(), variable);
    }

    pub async fn set_variable_value(
        &self,
        variable_id: &str,
        value: Value,
    ) -> flow_like_types::Result<()> {
        let variable = self.get_variable(variable_id).await?;
        if variable.data_type == VariableType::Geometry {
            let board = self.get_board().await?;
            let schema = variable
                .schema
                .as_deref()
                .map(|schema| crate::flow::pin::resolve_schema(schema, &board.refs))
                .transpose()?;
            crate::flow::variable::validate_typed_value(
                &variable.data_type,
                &variable.value_type,
                schema,
                &value,
            )?;
        } else {
            variable.validate_value(&value).map_err(|error| {
                flow_like_types::anyhow!("Cannot assign to variable '{}': {error}", variable.name)
            })?;
        }
        if let Some(scope) = &self.byte_scope {
            scope.promote(&value).await?;
        }
        *variable.value.lock().await = value;
        Ok(())
    }

    pub async fn get_cache(&self, key: &str) -> Option<Arc<dyn Cacheable>> {
        let cache = self.cache.read().await;
        if let Some(value) = cache.get(key) {
            return Some(value.clone());
        }

        None
    }

    pub async fn has_cache(&self, key: &str) -> bool {
        let cache = self.cache.read().await;
        cache.contains_key(key)
    }

    pub async fn set_cache(&self, key: &str, value: Arc<dyn Cacheable>) {
        let mut cache = self.cache.write().await;
        cache.insert(key.to_string(), value);
    }

    /// Get an OAuth token for a specific provider.
    /// Returns the token if found and not expired.
    pub fn get_oauth_token(&self, provider_id: &str) -> Option<&OAuthToken> {
        self.oauth_tokens
            .get(provider_id)
            .filter(|token| !token.is_expired())
    }

    /// Get an OAuth access token string for a specific provider.
    /// Returns None if the token is not found or expired.
    pub fn get_oauth_access_token(&self, provider_id: &str) -> Option<&str> {
        self.get_oauth_token(provider_id)
            .map(|token| token.access_token.as_str())
    }

    /// Check if a valid OAuth token exists for a specific provider.
    pub fn has_oauth_token(&self, provider_id: &str) -> bool {
        self.get_oauth_token(provider_id).is_some()
    }

    pub fn log(&mut self, log: LogMessage) {
        if log.log_level < self.log_level {
            return;
        }

        let mut log = log;
        log.node_id = Some(self.trace.node_id.to_string());
        self.trace.logs.push(log);
        self.trace_taken = false;
        self.spill_trace_if_needed();
    }

    pub fn log_message(&mut self, message: &str, log_level: LogLevel) {
        if log_level < self.log_level {
            return;
        }

        let mut log = LogMessage::new(message, log_level, None);
        log.node_id = Some(self.trace.node_id.to_string());
        self.trace.logs.push(log);
        self.trace_taken = false;
        self.spill_trace_if_needed();
    }

    fn spill_trace_if_needed(&mut self) {
        let policy = super::log_flush_policy::LogFlushPolicy {
            interval: self.log_flush_interval,
            spill_threshold: self.log_spill_threshold,
        };
        if !policy.should_spill(self.trace.logs.len(), self.last_log_spill.elapsed()) {
            return;
        }

        let Some(run) = self.run.upgrade() else {
            return;
        };

        // A busy run lock is transient backpressure, not permission to discard
        // diagnostics. Keep the trace local and retry on the next log or merge.
        let Ok(mut run) = run.try_lock() else {
            return;
        };

        let mut trace = std::mem::replace(&mut self.trace, Trace::new_shared(self.id.clone()));
        trace.finish();
        run.push_trace(trace);
        self.trace_taken = false;
        self.last_log_spill = Instant::now();
    }

    pub async fn set_state(&mut self, state: NodeState) {
        self.state = state;

        let method = match self.state {
            NodeState::Running => RunUpdateEventMethod::Add,
            _ => RunUpdateEventMethod::Remove,
        };

        if !self.stream_state {
            return;
        }

        let update_event = RunUpdateEvent {
            run_id: self.run_id.clone(),
            node_ids: vec![self.id.to_string()],
            method,
        };

        let event = InterComEvent::with_type(format!("run:{}", self.run_id), update_event);

        if let Err(err) = event.call(&self.callback).await {
            self.log_message(
                &format!("Failed to send run update event: {}", err),
                LogLevel::Error,
            );
        }
    }

    pub fn get_state(&self) -> NodeState {
        self.state.clone()
    }

    pub async fn get_pin_by_name(&self, name: &str) -> flow_like_types::Result<Arc<InternalPin>> {
        let pin = self.node.get_pin_by_name(name).await?;
        Ok(pin)
    }

    pub async fn get_model_config(
        &self,
    ) -> flow_like_types::Result<Arc<ModelProviderConfiguration>> {
        let config = self.app_state.model_provider_config.clone();
        Ok(config)
    }

    pub async fn evaluate_pin<T: DeserializeOwned>(
        &self,
        name: &str,
    ) -> flow_like_types::Result<T> {
        let value = self.evaluate_pin_raw(name).await?;
        self.deserialize_pin_value(value).await
    }

    /// The pin's value with byte handles expanded back into arrays.
    pub async fn evaluate_pin_to_ref(&self, name: &str) -> flow_like_types::Result<Value> {
        let value = self.evaluate_pin_raw(name).await?;
        self.materialize_bytes(value).await
    }

    /// The pin's value as stored. Byte handles stay handles, so a node that only moves a
    /// value (into a variable, a struct field, another pin) never expands its bytes.
    pub async fn evaluate_pin_raw(&self, name: &str) -> flow_like_types::Result<Value> {
        let pin = self.get_pin_by_name(name).await?;
        evaluate_pin_value(pin, &self.context_pin_overrides).await
    }

    pub async fn evaluate_pin_ref<T: DeserializeOwned>(
        &self,
        reference: Arc<InternalPin>,
    ) -> flow_like_types::Result<T> {
        let value = evaluate_pin_value(reference, &self.context_pin_overrides).await?;
        self.deserialize_pin_value(value).await
    }

    /// A byte pin's buffer, shared with the run's store instead of copied.
    pub async fn evaluate_pin_bytes(&self, name: &str) -> flow_like_types::Result<Bytes> {
        let value = self.evaluate_pin_raw(name).await?;
        if let Some(hash) = byte_store::handle_hash(&value)
            && let Some(store) = self.byte_store()
        {
            return store.get(hash).await;
        }
        let bytes: Vec<u8> = self.deserialize_pin_value(value).await?;
        Ok(Bytes::from(bytes))
    }

    pub async fn set_pin_bytes(
        &mut self,
        pin: &str,
        bytes: impl Into<Bytes>,
    ) -> flow_like_types::Result<()> {
        let pin = self.get_pin_by_name(pin).await?;
        let value = self.intern_bytes(bytes.into(), pin.sensitive).await;
        self.set_pin_ref_value(&pin, value).await
    }

    /// The value a pin or struct field carries for `bytes`: a handle into the run's store,
    /// or a plain array when the buffer is small or the run has no store.
    pub async fn bytes_value(&self, bytes: impl Into<Bytes>) -> Value {
        self.intern_bytes(bytes.into(), false).await
    }

    async fn intern_bytes(&self, bytes: Bytes, sensitive: bool) -> Value {
        let store = (bytes.len() > INLINE_BYTES_MAX)
            .then(|| self.byte_store_or_create())
            .flatten();
        byte_store::intern(store.as_ref(), bytes, sensitive).await
    }

    /// The scope this context writes bytes to. A long-lived server keeps it and takes a
    /// `child()` per request without locking any handler context.
    pub fn byte_scope(&self) -> ByteScope {
        ByteScope(self.byte_store_or_create())
    }

    /// A byte scope for one handler invocation in this run (a REST request, a socket
    /// message). Enter it on the handler's context; buffers it creates go away with it.
    pub fn new_byte_scope(&self) -> ByteScope {
        self.byte_scope().child()
    }

    /// Run this context inside `scope`; returns the scope to restore afterwards.
    pub fn enter_byte_scope(&mut self, scope: &ByteScope) -> ByteScope {
        ByteScope(std::mem::replace(&mut self.byte_scope, scope.0.clone()))
    }

    pub fn restore_byte_scope(&mut self, previous: ByteScope) {
        self.byte_scope = previous.0;
    }

    /// `value` with byte handles expanded, for anything that leaves the run.
    pub async fn materialize_bytes(&self, value: Value) -> flow_like_types::Result<Value> {
        match self.byte_store() {
            Some(store) => store.materialize(value).await,
            None => Ok(value),
        }
    }

    async fn deserialize_pin_value<T: DeserializeOwned>(
        &self,
        value: Value,
    ) -> flow_like_types::Result<T> {
        match self.byte_store() {
            Some(store) => store.deserialize(value).await,
            None => Ok(from_value(value)?),
        }
    }

    fn byte_store(&self) -> Option<Arc<ByteStore>> {
        self.byte_scope
            .clone()
            .or_else(|| self.resources.get::<ByteStore>(BYTE_STORE_RESOURCE))
    }

    fn byte_store_or_create(&self) -> Option<Arc<ByteStore>> {
        if let Some(scope) = &self.byte_scope {
            return Some(scope.clone());
        }
        self.resources
            .get_or_insert_with(BYTE_STORE_RESOURCE, || Arc::new(ByteStore::default()))
            .ok()
    }

    /// Producers that still write `json!(bytes)` into a byte pin are moved into the store.
    async fn intern_byte_array(&self, pin: &InternalPin, value: Value) -> Value {
        if pin.data_type != VariableType::Byte
            || pin.value_type != ValueType::Array
            || value
                .as_array()
                .is_none_or(|items| items.len() <= INLINE_BYTES_MAX)
        {
            return value;
        }
        let Some(bytes) = byte_store::array_to_bytes(&value) else {
            return value;
        };
        drop(value);
        self.intern_bytes(Bytes::from(bytes), pin.sensitive).await
    }

    pub async fn get_pins_by_name(
        &self,
        name: &str,
    ) -> flow_like_types::Result<Vec<Arc<InternalPin>>> {
        let pins = self.node.get_pins_by_name(name).await?;
        Ok(pins)
    }

    pub async fn get_pin_by_id(&self, id: &str) -> flow_like_types::Result<Arc<InternalPin>> {
        let pin = self.node.get_pin_by_id(id)?;
        Ok(pin)
    }

    pub async fn set_pin_ref_value(
        &mut self,
        pin: &Arc<InternalPin>,
        value: Value,
    ) -> flow_like_types::Result<()> {
        pin.validate_value(&value)?;
        let value = self.intern_byte_array(pin, value).await;
        let pin_id = pin.id();

        // When in an override context, write to BOTH the override map AND the
        // shared pin. The override map provides per-invocation isolation so
        // read_outputs can read the correct value for each parallel call.
        // The shared pin write keeps bridge pin chains, get_raw_value() checks,
        // and other non-override-aware code paths working.
        if self.context_pin_overrides.is_some() {
            let shared = Arc::new(value);
            self.override_pin_value_shared(pin_id, shared.clone());
            pin.set_value(shared.as_ref().clone()).await;
            return Ok(());
        }

        pin.set_value(value).await;
        Ok(())
    }

    pub async fn set_pin_value(&mut self, pin: &str, value: Value) -> flow_like_types::Result<()> {
        let pin = self.get_pin_by_name(pin).await?;
        self.set_pin_ref_value(&pin, value).await
    }

    pub async fn activate_exec_pin(&self, pin: &str) -> flow_like_types::Result<()> {
        let pin = self.get_pin_by_name(pin).await?;
        self.activate_exec_pin_ref(&pin).await
    }

    pub async fn activate_exec_pin_ref(
        &self,
        pin: &Arc<InternalPin>,
    ) -> flow_like_types::Result<()> {
        // Direct access - no lock needed for type checks
        if pin.data_type != VariableType::Execution {
            return Err(flow_like_types::anyhow!("Pin is not of type Execution"));
        }

        if pin.pin_type != PinType::Output {
            return Err(flow_like_types::anyhow!("Pin is not of type Output"));
        }

        // Only value access needs locking
        pin.set_value(flow_like_types::json::json!(true)).await;

        Ok(())
    }

    pub async fn deactivate_exec_pin(&self, pin: &str) -> flow_like_types::Result<()> {
        let pin = self.get_pin_by_name(pin).await?;
        self.deactivate_exec_pin_ref(&pin).await
    }

    pub async fn deactivate_exec_pin_ref(
        &self,
        pin: &Arc<InternalPin>,
    ) -> flow_like_types::Result<()> {
        // Direct access - no lock needed for type checks
        if pin.data_type != VariableType::Execution {
            return Err(flow_like_types::anyhow!("Pin is not of type Execution"));
        }

        if pin.pin_type != PinType::Output {
            return Err(flow_like_types::anyhow!("Pin is not of type Output"));
        }

        // Only value access needs locking
        pin.set_value(flow_like_types::json::json!(false)).await;

        Ok(())
    }

    pub fn push_sub_context(&mut self, context: &mut ExecutionContext) {
        let sub_traces = context.take_traces();
        for trace in sub_traces {
            append_trace_deduplicating_empty(
                &mut self.sub_traces,
                &mut self.represented_trace_nodes,
                trace,
            );
        }
        if let Some(result) = &context.result {
            self.result = Some(result.clone());
        }
        // Propagate pin overrides back so function contexts accumulate
        // all pin values written during the exec chain, ensuring parallel
        // invocations each read their own isolated output values.
        if let Some(child_overrides) = context.context_pin_overrides.take()
            && !child_overrides.is_empty()
        {
            self.context_pin_overrides
                .get_or_insert_with(BTreeMap::new)
                .extend(child_overrides);
        }
    }

    pub fn end_trace(&mut self) {
        self.trace.finish();
    }

    pub fn take_traces(&mut self) -> Vec<Trace> {
        let mut traces = std::mem::take(&mut self.sub_traces);
        let mut represented = std::mem::take(&mut self.represented_trace_nodes);
        if !self.trace_taken {
            let trace = self.trace.take();
            if traces.is_empty() && represented.is_empty() {
                // The overwhelmingly common leaf-context path needs no
                // allocation or hash just to return its sole trace.
                traces.push(trace);
            } else {
                append_trace_deduplicating_empty(&mut traces, &mut represented, trace);
            }
            self.trace_taken = true;
        }
        traces.sort_by_key(|a| a.start);
        traces
    }

    pub fn try_get_run(&self) -> flow_like_types::Result<Arc<Mutex<Run>>> {
        if let Some(run) = self.run.upgrade() {
            return Ok(run);
        }

        Err(flow_like_types::anyhow!("Run not found"))
    }

    /// Flush logs to the database during long-running operations.
    /// This pushes the current trace's logs to the Run and triggers a flush.
    /// Call this periodically during long-running node operations to ensure
    /// logs are visible to users in real-time.
    pub async fn flush_logs(&mut self) -> flow_like_types::Result<()> {
        let run = self.try_get_run()?;
        let prepared: Option<super::PreparedFlush> = {
            let mut run = super::lock_with_timeout(run.as_ref(), "execution_context_run").await?;

            // Move all buffered traces into the run. This includes the current
            // tail trace, even when it never reached the spill threshold.
            self.trace.finish();
            let traces = self.take_traces();
            run.extend_traces(traces);
            self.last_log_spill = Instant::now();

            run.prepare_flush(false)?
        };

        if let Some(prepared) = prepared {
            let result = prepared.write().await?;
            if result.created_table {
                let mut run =
                    super::lock_with_timeout(run.as_ref(), "execution_context_mark").await?;
                run.log_initialized = true;
            }
        }

        Ok(())
    }

    pub async fn read_node(&self) -> Node {
        let node = self.node.node.lock().await;

        node.clone()
    }

    /// Get all referenced functions for this node.
    /// Returns an error if the node doesn't support function references.
    pub async fn get_referenced_functions(
        &self,
    ) -> flow_like_types::Result<Vec<Arc<InternalNode>>> {
        let node = self.node.node.lock().await;

        let fn_refs = node
            .fn_refs
            .as_ref()
            .ok_or_else(|| flow_like_types::anyhow!("Node does not support function references"))?;

        if !fn_refs.can_reference_fns {
            return Err(flow_like_types::anyhow!(
                "Node is not configured to reference functions"
            ));
        }

        let mut referenced_nodes = Vec::with_capacity(fn_refs.fn_refs.len());

        for fn_ref in &fn_refs.fn_refs {
            let referenced_node = self
                .nodes
                .get(fn_ref)
                .ok_or_else(|| {
                    flow_like_types::anyhow!("Referenced function '{}' not found", fn_ref)
                })?
                .clone();
            referenced_nodes.push(referenced_node);
        }

        Ok(referenced_nodes)
    }

    pub async fn toast_message(
        &mut self,
        message: &str,
        level: ToastLevel,
    ) -> flow_like_types::Result<()> {
        let event = InterComEvent::with_type("toast", ToastEvent::new(message, level));
        if let Err(err) = event.call(&self.callback).await {
            self.log_message(
                &format!("Failed to send toast event: {}", err),
                LogLevel::Error,
            );
        }
        Ok(())
    }

    pub async fn progress_message(
        &mut self,
        id: &str,
        message: &str,
        progress: Option<u8>,
    ) -> flow_like_types::Result<()> {
        let event = InterComEvent::with_type("progress", ProgressEvent::new(id, message, progress));
        if let Err(err) = event.call(&self.callback).await {
            self.log_message(
                &format!("Failed to send progress event: {}", err),
                LogLevel::Error,
            );
        }
        Ok(())
    }

    pub async fn progress_done(
        &mut self,
        id: &str,
        message: &str,
        success: bool,
    ) -> flow_like_types::Result<()> {
        let event = InterComEvent::with_type("progress", ProgressEvent::done(id, message, success));
        if let Err(err) = event.call(&self.callback).await {
            self.log_message(
                &format!("Failed to send progress done event: {}", err),
                LogLevel::Error,
            );
        }
        Ok(())
    }

    pub async fn stream_response<T>(
        &mut self,
        event_type: &str,
        event: T,
    ) -> flow_like_types::Result<()>
    where
        T: Serialize + DeserializeOwned,
    {
        tracing::debug!(event_type = %event_type, "Streaming response event");
        let mut event = InterComEvent::with_type(event_type, event);
        event.payload = self
            .materialize_bytes(std::mem::take(&mut event.payload))
            .await?;
        if let Err(err) = event.call(&self.callback).await {
            self.log_message(&format!("Failed to send event: {}", err), LogLevel::Error);
            tracing::error!(error = %err, "Failed to send stream event");
        } else {
            tracing::debug!(event_type = %event_type, "Successfully sent stream event");
        }
        Ok(())
    }

    pub async fn stream_a2ui_update(
        &mut self,
        message: crate::a2ui::A2UIServerMessage,
    ) -> flow_like_types::Result<()> {
        tracing::debug!("Streaming A2UI update");
        self.record_a2ui_update(&message).await;
        self.record_detached(&message).await;
        self.stream_response("a2ui", message).await
    }

    /// Get-or-insert under a single write lock: parallel branches emitting the run's first
    /// update must not race two entries into existence.
    async fn shared_cache_entry<T: Cacheable + Clone + Default + 'static>(&self, key: &str) -> T {
        let mut cache = self.cache.write().await;
        if let Some(entry) = cache
            .get(key)
            .and_then(|c| c.as_any().downcast_ref::<T>().cloned())
        {
            return entry;
        }
        let entry = T::default();
        cache.insert(
            key.to_string(),
            Arc::new(entry.clone()) as Arc<dyn Cacheable>,
        );
        entry
    }

    async fn record_detached(&self, message: &crate::a2ui::A2UIServerMessage) {
        let Some(element_id) = detaching_element_id(message) else {
            return;
        };
        let detached = self
            .shared_cache_entry::<A2UIDetachedElements>(A2UI_DETACHED_KEY)
            .await;
        if let Ok(mut ids) = detached.ids.lock() {
            ids.insert(element_id.to_string());
        }
    }

    /// Records surface-mutating a2ui messages in a run-scoped log so nodes that
    /// snapshot UI state later in the same run (e.g. Push Widget embedding a
    /// widget into a chat message) can replay updates that were streamed before
    /// the snapshot was taken.
    async fn record_a2ui_update(&self, message: &crate::a2ui::A2UIServerMessage) {
        use crate::a2ui::A2UIServerMessage as Msg;
        if !matches!(
            message,
            Msg::UpsertElement { .. }
                | Msg::DataModelUpdate { .. }
                | Msg::CreateElement { .. }
                | Msg::RemoveElement { .. }
        ) {
            return;
        }

        let log = self
            .shared_cache_entry::<A2UIUpdateLog>(A2UI_UPDATE_LOG_KEY)
            .await;

        if let Ok(mut entries) = log.entries.lock() {
            if entries.len() >= A2UI_UPDATE_LOG_CAP {
                log.truncated
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                return;
            }
            entries.push(message.clone());
        }
    }

    /// Returns all surface-mutating a2ui messages streamed so far in this run,
    /// in emission order, plus whether the log hit its cap and dropped entries.
    pub async fn get_a2ui_update_log(&self) -> (Vec<crate::a2ui::A2UIServerMessage>, bool) {
        match self.get_cache(A2UI_UPDATE_LOG_KEY).await {
            Some(cached) => match cached.as_any().downcast_ref::<A2UIUpdateLog>() {
                Some(log) => (
                    log.entries
                        .lock()
                        .map(|entries| entries.clone())
                        .unwrap_or_default(),
                    log.is_truncated(),
                ),
                None => (Vec::new(), false),
            },
            None => (Vec::new(), false),
        }
    }

    pub async fn stream_a2ui_begin_rendering(
        &mut self,
        surface: &crate::a2ui::Surface,
        data_model: &crate::a2ui::DataModel,
    ) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::begin_rendering(surface, data_model);
        self.stream_a2ui_update(message).await
    }

    pub async fn stream_a2ui_surface_update(
        &mut self,
        surface_id: &str,
        components: Vec<crate::a2ui::SurfaceComponent>,
    ) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::surface_update(surface_id, components);
        self.stream_a2ui_update(message).await
    }

    pub async fn stream_a2ui_set_canvas_settings(
        &mut self,
        surface_id: &str,
        canvas_settings: crate::a2ui::CanvasSettings,
    ) -> flow_like_types::Result<()> {
        let message =
            crate::a2ui::A2UIServerMessage::set_canvas_settings(surface_id, canvas_settings);
        self.stream_a2ui_update(message).await
    }

    pub async fn stream_a2ui_data_update(
        &mut self,
        surface_id: &str,
        path: Option<String>,
        value: Value,
    ) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::data_update(surface_id, path, value);
        self.stream_a2ui_update(message).await
    }

    pub async fn stream_a2ui_delete_surface(
        &mut self,
        surface_id: &str,
    ) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::delete_surface(surface_id);
        self.stream_a2ui_update(message).await
    }

    /// Live round-trip to the rendered page: stream a `requestElements` message carrying a
    /// channel handle, then merge the page's answer (`{ ok, elements }`) into the run's elements.
    pub async fn request_elements(
        &mut self,
        selectors: Vec<String>,
        timeout: Duration,
    ) -> flow_like_types::Result<Map<String, Value>> {
        let channel = self.channel()?;
        let ticket = channel.open(timeout).await?;
        let message = crate::a2ui::A2UIServerMessage::request_elements(
            &ticket.request_id,
            selectors.clone(),
            timeout.as_millis() as u64,
            Some(ticket.handle.clone()),
        );
        if let Err(err) = self.stream_a2ui_update(message).await {
            channel.abandon(&ticket).await;
            return Err(err);
        }

        let described = selectors.join(", ");
        let reply = match channel
            .wait(&ticket, self.cancellation_token.clone())
            .await?
        {
            ChannelOutcome::Responded(reply) => reply,
            ChannelOutcome::Expired => {
                return Err(flow_like_types::anyhow!(
                    "Element request [{}] timed out after {}ms: the app page did not answer. It may be reloading or closed. Open Use App, wait for the page to finish loading, then try the action again.",
                    described,
                    timeout.as_millis()
                ));
            }
            ChannelOutcome::Cancelled => {
                return Err(flow_like_types::anyhow!(
                    "Element request [{}] was cancelled",
                    described
                ));
            }
            ChannelOutcome::Closed => {
                return Err(flow_like_types::anyhow!(
                    "Element request [{}] could not complete: the connection closed before the app page answered. It may be reloading or closed. Open Use App, wait for the page to finish loading, then try the action again.",
                    described
                ));
            }
        };

        if reply.get("ok").and_then(Value::as_bool) == Some(false) {
            let error = reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(flow_like_types::anyhow!(
                "Element request [{}] failed on the page: {}",
                described,
                error
            ));
        }
        let elements = reply
            .get("elements")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Element request [{}] returned an invalid response: expected an elements object",
                    described
                )
            })?;
        {
            let mut cache = self.elements.write().await;
            cache.take_unrequested(&selectors);
            cache.merge_response(&selectors, elements.clone());
        }
        Ok(elements)
    }

    pub fn cancellation_token(&self) -> Option<CancellationToken> {
        self.cancellation_token.clone()
    }

    /// The run's reply conduit. Every run built through `InternalRun` has one; contexts
    /// assembled by hand (tests, tooling) may not.
    pub fn channel(&self) -> flow_like_types::Result<Arc<dyn Channel>> {
        self.channel.clone().ok_or_else(|| {
            flow_like_types::anyhow!(
                "No channel attached to run '{}': the client cannot be asked for input here",
                self.run_id
            )
        })
    }

    /// Live round-trip to the rendered frontend: run a contract query against
    /// a micro widget instance and await its `query:result`. The request rides
    /// the a2ui stream carrying a channel handle; the surface answers through
    /// the run's channel. Returns the raw response envelope
    /// `{ "ok": bool, "value"?: …, "error"?: … }`.
    pub async fn query_widget(
        &mut self,
        instance_id: &str,
        query: &str,
        args: Option<Value>,
        timeout: std::time::Duration,
    ) -> flow_like_types::Result<Value> {
        let channel = self.channel()?;
        let ticket = channel.open(timeout).await?;
        let message = crate::a2ui::A2UIServerMessage::widget_query(
            &ticket.request_id,
            instance_id,
            query,
            args,
            timeout.as_millis() as u64,
            Some(ticket.handle.clone()),
        );
        if let Err(err) = self.stream_a2ui_update(message).await {
            channel.abandon(&ticket).await;
            return Err(err);
        }

        match channel
            .wait(&ticket, self.cancellation_token.clone())
            .await?
        {
            ChannelOutcome::Responded(response) => Ok(response),
            ChannelOutcome::Expired => Err(flow_like_types::anyhow!(
                "Widget query '{}' on instance '{}' timed out after {}ms — no live surface answered",
                query,
                instance_id,
                timeout.as_millis()
            )),
            ChannelOutcome::Cancelled => Err(flow_like_types::anyhow!(
                "Widget query '{}' on instance '{}' was cancelled",
                query,
                instance_id
            )),
            ChannelOutcome::Closed => Err(flow_like_types::anyhow!(
                "Widget query '{}' on instance '{}' was dropped without a response",
                query,
                instance_id
            )),
        }
    }

    pub async fn show_screen(&mut self) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::show_screen();
        self.stream_a2ui_update(message).await
    }

    pub async fn upsert_element(
        &mut self,
        element_id: &str,
        value: Value,
    ) -> flow_like_types::Result<()> {
        // A ref pinned to another page's id retargets to the run's own surface here, so
        // every setter node emits a key the triggering page can apply. Bare ids and exact
        // keys pass through untouched; an unresolvable ref keeps its original id (it may
        // address a widget surface or create a new element).
        let retargeted = if element_id.contains('/') {
            self.with_elements(|elements| {
                if elements.contains_key(element_id) {
                    None
                } else {
                    crate::a2ui::resolve_element_key(elements, element_id).cloned()
                }
            })
            .await
            .ok()
            .flatten()
        } else {
            None
        };
        let element_id = retargeted.as_deref().unwrap_or(element_id);
        self.elements.write().await.clear_requested();
        tracing::info!(element_id = %element_id, "[A2UI] upsert_element called");
        self.log_message(
            &format!("[A2UI] upsert_element: {}", element_id),
            LogLevel::Debug,
        );
        let message = crate::a2ui::A2UIServerMessage::upsert_element(element_id, value);
        self.stream_a2ui_update(message).await
    }

    pub async fn navigate_to(&mut self, route: &str, replace: bool) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::navigate_to(route, replace);
        self.stream_a2ui_update(message).await
    }

    pub async fn create_element(
        &mut self,
        surface_id: &str,
        parent_id: &str,
        component: crate::a2ui::SurfaceComponent,
        index: Option<usize>,
    ) -> flow_like_types::Result<()> {
        self.elements.write().await.clear_requested();
        let message =
            crate::a2ui::A2UIServerMessage::create_element(surface_id, parent_id, component, index);
        self.stream_a2ui_update(message).await
    }

    pub async fn remove_element(
        &mut self,
        surface_id: &str,
        element_id: &str,
    ) -> flow_like_types::Result<()> {
        self.elements.write().await.clear_requested();
        let message = crate::a2ui::A2UIServerMessage::remove_element(surface_id, element_id);
        self.stream_a2ui_update(message).await
    }

    pub async fn set_global_state(
        &mut self,
        key: &str,
        value: flow_like_types::Value,
    ) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::set_global_state(key, value);
        self.stream_a2ui_update(message).await
    }

    pub async fn set_page_state(
        &mut self,
        page_id: &str,
        key: &str,
        value: flow_like_types::Value,
    ) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::set_page_state(page_id, key, value);
        self.stream_a2ui_update(message).await
    }

    pub async fn clear_page_state(&mut self, page_id: &str) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::clear_page_state(page_id);
        self.stream_a2ui_update(message).await
    }

    pub async fn set_query_param(
        &mut self,
        key: &str,
        value: Option<String>,
        replace: bool,
    ) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::set_query_param(key, value, replace);
        self.stream_a2ui_update(message).await
    }

    pub async fn open_dialog(
        &mut self,
        route: &str,
        title: Option<String>,
        query_params: Option<std::collections::HashMap<String, String>>,
        dialog_id: Option<String>,
    ) -> flow_like_types::Result<()> {
        let message =
            crate::a2ui::A2UIServerMessage::open_dialog(route, title, query_params, dialog_id);
        self.stream_a2ui_update(message).await
    }

    pub async fn close_dialog(&mut self, dialog_id: Option<String>) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::close_dialog(dialog_id);
        self.stream_a2ui_update(message).await
    }
}

fn append_trace_deduplicating_empty(
    traces: &mut Vec<Trace>,
    represented_nodes: &mut AHashSet<Arc<str>>,
    trace: Trace,
) {
    let first_for_node = !represented_nodes.contains(trace.node_id.as_ref());
    if first_for_node {
        represented_nodes.insert(trace.node_id.clone());
    }
    if !trace.logs.is_empty() || first_for_node {
        traces.push(trace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_user_paths_match_credentials_and_local_scratch_paths_stay_unchanged() {
        let mut cache = ExecutionContextCache {
            stores: FlowLikeStores::default(),
            app_id: "project".into(),
            model_usage_app_id: None,
            board_dir: Path::from("apps/project"),
            board_id: "board".into(),
            node_id: Arc::from("node"),
            sub: "auth0|delegating-user".into(),
            shadow: false,
        };
        let (user, global) =
            flow_like_types::storage_paths::temporary_prefixes(&cache.sub, &cache.app_id);
        assert_eq!(cache.get_cache(false, true).unwrap().as_ref(), user);
        assert_eq!(
            cache.get_cache(true, true).unwrap().as_ref(),
            format!("{user}/node")
        );
        assert_eq!(cache.get_cache(false, false).unwrap().as_ref(), global);
        assert_eq!(
            cache.get_user_dir(false).unwrap().join("db").as_ref(),
            "users/auth0%7Cdelegating-user/apps/project/db"
        );
        let directory = tempfile::tempdir().unwrap();
        cache.stores.temporary_store = Some(FlowLikeStore::Local(Arc::new(
            flow_like_storage::files::store::local_store::LocalObjectStore::new(
                directory.path().to_path_buf(),
            )
            .unwrap(),
        )));
        assert_eq!(
            cache.get_cache(false, true).unwrap().as_ref(),
            "tmp/user/auth0%7Cdelegating-user/apps/project"
        );
    }

    #[test]
    fn live_project_requests_require_tls_or_explicit_loopback() {
        for url in [
            "https://api.example.test/api/v1/apps/project/connections/target/token",
            "http://localhost:3000/api/v1",
            "http://127.0.0.1:3000/api/v1",
            "http://[::1]:3000/api/v1",
        ] {
            assert!(
                validate_live_resource_url(&flow_like_types::reqwest::Url::parse(url).unwrap())
                    .is_ok()
            );
        }
        for url in [
            "http://api.example.test/api/v1",
            "https://user:password@api.example.test/api/v1",
            "https://api.example.test/api/v1/..%2fadmin",
        ] {
            assert!(
                validate_live_resource_url(&flow_like_types::reqwest::Url::parse(url).unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn empty_traces_are_retained_once_per_node() {
        let mut traces = Vec::new();
        let mut represented_nodes = AHashSet::new();

        append_trace_deduplicating_empty(&mut traces, &mut represented_nodes, Trace::new("node-a"));
        append_trace_deduplicating_empty(&mut traces, &mut represented_nodes, Trace::new("node-a"));
        append_trace_deduplicating_empty(&mut traces, &mut represented_nodes, Trace::new("node-b"));

        assert_eq!(traces.len(), 2);
        assert!(
            traces
                .iter()
                .any(|trace| trace.node_id.as_ref() == "node-a")
        );
        assert!(
            traces
                .iter()
                .any(|trace| trace.node_id.as_ref() == "node-b")
        );
    }

    #[test]
    fn non_empty_traces_are_never_deduplicated() {
        let mut traces = Vec::new();
        let mut represented_nodes = AHashSet::new();
        let mut first = Trace::new("node");
        first
            .logs
            .push(LogMessage::new("first", LogLevel::Info, None));
        let mut second = Trace::new("node");
        second
            .logs
            .push(LogMessage::new("second", LogLevel::Info, None));

        append_trace_deduplicating_empty(&mut traces, &mut represented_nodes, first);
        append_trace_deduplicating_empty(&mut traces, &mut represented_nodes, second);

        assert_eq!(traces.len(), 2);
    }

    use crate::a2ui::A2UIServerMessage;
    use flow_like_types::json::json;

    fn update(element_id: &str, kind: &str) -> A2UIServerMessage {
        A2UIServerMessage::upsert_element(element_id, json!({ "type": kind, "index": 0 }))
    }

    fn capturing_callback() -> (InterComCallback, Arc<std::sync::Mutex<Vec<InterComEvent>>>) {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = events.clone();
        let callback: InterComCallback = Some(Arc::new(move |event| {
            sink.lock().unwrap().push(event);
            Box::pin(async { Ok(()) })
        }));
        (callback, events)
    }

    #[test]
    fn only_child_removals_detach() {
        assert_eq!(
            detaching_element_id(&update("page/list", "clearChildren")),
            Some("page/list")
        );
        assert_eq!(
            detaching_element_id(&update("inst/list", "removeChildAt")),
            Some("inst/list")
        );
        assert_eq!(
            detaching_element_id(&A2UIServerMessage::remove_element("page", "card")),
            Some("card")
        );
        assert_eq!(detaching_element_id(&update("list", "pushChild")), None);
        assert_eq!(
            detaching_element_id(&update("list", "createComponent")),
            None
        );
    }

    #[tokio::test]
    async fn run_end_names_every_detaching_element_once() {
        let detached = A2UIDetachedElements::default();
        detached
            .ids
            .lock()
            .unwrap()
            .extend(["list", "card", "list"].map(String::from));
        let cache = RwLock::new(AHashMap::new());
        cache.write().await.insert(
            A2UI_DETACHED_KEY.to_string(),
            Arc::new(detached) as Arc<dyn Cacheable>,
        );
        let (callback, events) = capturing_callback();

        stream_prune_detached(&cache, &callback).await;

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "a2ui");
        assert_eq!(
            events[0].payload,
            json!({ "type": "pruneDetached", "element_ids": ["card", "list"] })
        );
    }

    #[tokio::test]
    async fn a_run_that_detached_nothing_sends_nothing() {
        let (callback, events) = capturing_callback();
        stream_prune_detached(&RwLock::new(AHashMap::new()), &callback).await;
        assert!(events.lock().unwrap().is_empty());
    }

    struct NoopLogic;

    #[flow_like_types::async_trait]
    impl crate::flow::node::NodeLogic for NoopLogic {
        fn get_node(&self) -> Node {
            Node::new("noop", "Noop", "Noop", "Tests")
        }

        async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            Ok(())
        }
    }

    async fn byte_context(callback: InterComCallback) -> ExecutionContext {
        let mut node = Node::new("bytes", "Bytes", "Byte pins", "Tests");
        node.add_output_pin("bytes", "Bytes", "", VariableType::Byte)
            .set_value_type(ValueType::Array);
        let mut sensitive = crate::flow::pin::PinOptions::new();
        sensitive.set_sensitive(true);
        node.add_output_pin("secret", "Secret", "", VariableType::Byte)
            .set_value_type(ValueType::Array)
            .set_options(sensitive);
        node.add_output_pin("generic", "Generic", "", VariableType::Generic);

        let mut pins = AHashMap::new();
        let mut names: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();
        for pin in node.pins.values() {
            let pin = Arc::new(InternalPin::new(pin, false));
            names
                .entry(pin.name.to_string())
                .or_default()
                .push(pin.clone());
            pins.insert(pin.id().to_string(), pin);
        }
        let internal = Arc::new(InternalNode::new(node, pins, Arc::new(NoopLogic), names));
        for pin in internal.pins.iter() {
            pin.init_node(Arc::downgrade(&internal));
            pin.init_connected_to(Vec::new());
            pin.init_depends_on(Vec::new());
        }
        let nodes = Arc::new(AHashMap::from_iter([(
            internal.node_id().to_string(),
            internal.clone(),
        )]));
        let state = Arc::new(FlowLikeState::new(
            crate::state::FlowLikeConfig::new(),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let mut context = ExecutionContext::new(
            nodes,
            &Weak::new(),
            &state,
            &internal,
            &Arc::new(Mutex::new(AHashMap::new())),
            &Arc::new(RwLock::new(AHashMap::new())),
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            callback,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await;
        context.resources = Arc::new(super::super::resources::RunResources::default());
        context
    }

    fn photo(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index % 251) as u8).collect()
    }

    #[tokio::test]
    async fn function_contexts_isolate_overrides_and_reset_local_variables() {
        let mut parent = byte_context(None).await;
        parent.override_pin_value("outer", json!("parent"));
        let mut variable = Variable::new("counter", VariableType::Integer, ValueType::Normal);
        variable.default_value = Some(flow_like_types::json::to_vec(&json!(7)).unwrap());
        *variable.value.lock().await = json!(99);
        let variable_id = variable.id.clone();
        let definition_value = variable.value.clone();
        parent.local_variables = Some(Arc::new(Mutex::new(AHashMap::from_iter([(
            variable_id.clone(),
            variable.clone(),
        )]))));
        let function_variables = std::collections::HashMap::from([(variable_id.clone(), variable)]);

        let ordinary_child = parent.create_sub_context(&parent.node).await;
        assert_eq!(
            ordinary_child.context_pin_overrides,
            parent.context_pin_overrides
        );
        let (mut first, second) = tokio::join!(
            parent.create_function_context(&parent.node, &function_variables),
            parent.create_function_context(&parent.node, &function_variables),
        );
        assert_eq!(first.context_pin_overrides, Some(BTreeMap::new()));
        assert_eq!(second.context_pin_overrides, Some(BTreeMap::new()));

        let first_value = first.get_variable(&variable_id).await.unwrap().value;
        let second_value = second.get_variable(&variable_id).await.unwrap().value;
        assert_eq!(*first_value.lock().await, json!(7));
        *first_value.lock().await = json!(8);
        assert_eq!(*second_value.lock().await, json!(7));
        assert_eq!(*definition_value.lock().await, json!(99));
        assert_eq!(
            *parent
                .get_variable(&variable_id)
                .await
                .unwrap()
                .value
                .lock()
                .await,
            json!(99)
        );

        first.override_pin_value("outer", json!("first"));
        let mut nested = first.create_sub_context(&first.node).await;
        assert_eq!(nested.context_pin_overrides, first.context_pin_overrides);
        nested.override_pin_value("result", json!(42));
        first.push_sub_context(&mut nested);
        assert_eq!(
            first.context_pin_overrides.as_ref().unwrap()["result"].as_ref(),
            &json!(42)
        );
        assert_eq!(second.context_pin_overrides, Some(BTreeMap::new()));
        assert_eq!(
            parent.context_pin_overrides,
            Some(BTreeMap::from([(
                "outer".to_string(),
                Arc::new(json!("parent"))
            )]))
        );
    }

    #[tokio::test]
    async fn string_assignment_rejects_generic_numeric_fields_before_mutating_either_scope() {
        for local in [false, true] {
            let mut context = byte_context(None).await;
            let variable = Variable::new("size_text", VariableType::String, ValueType::Normal);
            let variable_id = variable.id.clone();
            let stored_value = variable.value.clone();
            *stored_value.lock().await = json!("previous");

            if local {
                let mut shadowed = variable.clone();
                shadowed.data_type = VariableType::Generic;
                shadowed.value = Arc::new(Mutex::new(json!("outer")));
                context
                    .variables
                    .lock()
                    .await
                    .insert(variable_id.clone(), shadowed);
                context.local_variables = Some(Arc::new(Mutex::new(AHashMap::from_iter([(
                    variable_id.clone(),
                    variable,
                )]))));
            } else {
                context
                    .variables
                    .lock()
                    .await
                    .insert(variable_id.clone(), variable);
            }

            let size_bytes = json!({"size_bytes": 4096})["size_bytes"].clone();
            context.set_pin_value("generic", size_bytes).await.unwrap();
            let raw = context.evaluate_pin_raw("generic").await.unwrap();
            let error = context
                .set_variable_value(&variable_id, raw)
                .await
                .unwrap_err()
                .to_string();
            assert_eq!(
                error,
                "Cannot assign to variable 'size_text': Type mismatch: expected String, got Integer"
            );
            assert_eq!(*stored_value.lock().await, json!("previous"));

            context
                .set_variable_value(&variable_id, json!("4096"))
                .await
                .unwrap();
            assert_eq!(*stored_value.lock().await, json!("4096"));
            if local {
                let outer = context.variables.lock().await[&variable_id].value.clone();
                assert_eq!(*outer.lock().await, json!("outer"));
            }

            let generic = Variable::new("raw_size", VariableType::Generic, ValueType::Normal);
            let generic_id = generic.id.clone();
            context.set_variable(generic).await;
            context
                .set_variable_value(&generic_id, json!(4096))
                .await
                .unwrap();
            let generic = context.get_variable(&generic_id).await.unwrap();
            assert_eq!(*generic.value.lock().await, json!(4096));
        }
    }

    #[tokio::test]
    async fn byte_pins_carry_handles_but_read_like_arrays() {
        let (callback, events) = capturing_callback();
        let mut context = byte_context(callback).await;
        let bytes = photo(300 * 1024);

        context.set_pin_bytes("bytes", bytes.clone()).await.unwrap();
        let raw = context.evaluate_pin_raw("bytes").await.unwrap();
        assert_eq!(byte_store::handle_len(&raw), Some(bytes.len() as u64));

        let typed: Vec<u8> = context.evaluate_pin("bytes").await.unwrap();
        assert_eq!(typed, bytes);
        let shared = context.evaluate_pin_bytes("bytes").await.unwrap();
        assert_eq!(&shared[..], &bytes[..]);
        let generic = context.evaluate_pin_to_ref("bytes").await.unwrap();
        assert_eq!(generic, byte_store::bytes_to_array(&bytes));

        context
            .stream_response("generic_result", json!({ "file": raw }))
            .await
            .unwrap();
        let events = events.lock().unwrap();
        assert_eq!(
            events[0].payload,
            json!({ "file": byte_store::bytes_to_array(&bytes) })
        );
    }

    #[tokio::test]
    async fn json_arrays_on_byte_pins_are_interned_and_small_ones_stay_inline() {
        let mut context = byte_context(None).await;
        let bytes = photo(64 * 1024);

        context.set_pin_value("bytes", json!(bytes)).await.unwrap();
        let raw = context.evaluate_pin_raw("bytes").await.unwrap();
        assert!(byte_store::handle_hash(&raw).is_some());

        context
            .set_pin_value("generic", json!(bytes))
            .await
            .unwrap();
        let generic = context.evaluate_pin_raw("generic").await.unwrap();
        assert!(generic.is_array());

        context
            .set_pin_bytes("bytes", vec![1u8, 2, 3])
            .await
            .unwrap();
        assert_eq!(
            context.evaluate_pin_raw("bytes").await.unwrap(),
            json!([1, 2, 3])
        );
    }

    #[tokio::test]
    async fn a_closed_run_keeps_bytes_inline() {
        let mut context = byte_context(None).await;
        context.resources.abort();
        let bytes = photo(64 * 1024);
        context.set_pin_bytes("bytes", bytes.clone()).await.unwrap();
        let raw = context.evaluate_pin_raw("bytes").await.unwrap();
        assert!(raw.is_array());
        let typed: Vec<u8> = context.evaluate_pin("bytes").await.unwrap();
        assert_eq!(typed, bytes);
    }

    #[tokio::test]
    async fn a_handler_scope_frees_its_bytes_but_keeps_what_reached_a_run_variable() {
        let mut context = byte_context(None).await;
        let variable = Variable::new("upload", VariableType::Byte, ValueType::Array);
        let variable_id = variable.id.clone();
        context
            .variables
            .lock()
            .await
            .insert(variable_id.clone(), variable);

        let scope = context.new_byte_scope();
        let previous = context.enter_byte_scope(&scope);
        context
            .set_pin_bytes("bytes", photo(64 * 1024))
            .await
            .unwrap();
        let kept = context.bytes_value(photo(80 * 1024)).await;
        context
            .set_variable_value(&variable_id, kept.clone())
            .await
            .unwrap();
        context.restore_byte_scope(previous);
        drop(scope);

        let stale = context.evaluate_pin::<Vec<u8>>("bytes").await.unwrap_err();
        assert!(stale.to_string().contains("already finished"), "{stale}");
        let from_variable: Vec<u8> = context.deserialize_pin_value(kept).await.unwrap();
        assert_eq!(from_variable, photo(80 * 1024));
    }

    #[tokio::test]
    async fn sensitive_byte_pins_stay_in_memory() {
        let mut context = byte_context(None).await;
        context
            .set_pin_bytes("secret", photo(64 * 1024))
            .await
            .unwrap();
        let raw = context.evaluate_pin_raw("secret").await.unwrap();
        let hash = byte_store::handle_hash(&raw).unwrap();
        assert!(
            context.byte_store().unwrap().is_resident(hash),
            "a sensitive buffer must never be spillable"
        );
        let secret = context.evaluate_pin_bytes("secret").await.unwrap();
        assert_eq!(secret.len(), 64 * 1024);
    }
}
