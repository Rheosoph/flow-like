use super::{
    EventTrigger, ExecutionEnvironment, ExecutionMode, InternalNode, LogLevel, Run, RunPayload,
    internal_pin::InternalPin, log::LogMessage, trace::Trace,
};
use crate::models::llm::ModelUsageContext;
use crate::{
    credentials::SharedCredentials,
    flow::{
        board::ExecutionStage,
        node::{Node, NodeState},
        oauth::OAuthToken,
        pin::PinType,
        utils::evaluate_pin_value,
        variable::{Variable, VariableType},
    },
    profile::Profile,
    state::{FlowLikeState, FlowLikeStores, ProgressEvent, ToastEvent, ToastLevel},
};
use ahash::{AHashMap, AHashSet};
use flow_like_model_provider::provider::ModelProviderConfiguration;
use flow_like_storage::object_store::path::Path;
use flow_like_types::Value;
use flow_like_types::intercom::{InterComCallback, InterComEvent};
use flow_like_types::tokio_util::sync::CancellationToken;
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

#[derive(Clone)]
pub struct ExecutionContextCache {
    pub stores: FlowLikeStores,
    pub app_id: String,
    pub model_usage_app_id: Option<String>,
    pub board_dir: Path,
    pub board_id: String,
    pub node_id: Arc<str>,
    pub sub: String,
}

impl ExecutionContextCache {
    pub async fn new(
        run: &Weak<Mutex<Run>>,
        state: &Arc<FlowLikeState>,
        node_id: Arc<str>,
    ) -> Option<Self> {
        let (app_id, model_usage_app_id, board_dir, board_id, sub) = match run.upgrade() {
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
                )
            }
            None => return None,
        };

        let stores = state.config.read().await.stores.clone();

        Some(ExecutionContextCache {
            stores,
            app_id,
            model_usage_app_id,
            board_dir,
            board_id,
            node_id,
            sub,
        })
    }

    /// Create ExecutionContextCache from cached RunMeta to avoid locking
    pub async fn from_meta(
        meta: &super::RunMeta,
        state: &Arc<FlowLikeState>,
        node_id: Arc<str>,
    ) -> Self {
        let stores = state.config.read().await.stores.clone();

        ExecutionContextCache {
            stores,
            app_id: meta.app_id.clone(),
            model_usage_app_id: meta.model_usage_app_id.clone(),
            board_dir: meta.board_dir.clone(),
            board_id: meta.board_id.clone(),
            node_id,
            sub: meta.sub.clone(),
        }
    }

    fn for_node(&self, node_id: Arc<str>) -> Self {
        let mut cache = self.clone();
        cache.node_id = node_id;
        cache
    }

    pub fn get_user_dir(&self, node: bool) -> flow_like_types::Result<Path> {
        let base = Path::from("users")
            .child(self.sub.clone())
            .child("apps")
            .child(self.app_id.clone());
        if !node {
            return Ok(base);
        }

        Ok(base.child(self.node_id.as_ref()))
    }

    pub fn get_cache(&self, node: bool, user: bool) -> flow_like_types::Result<Path> {
        let mut base = Path::from("tmp");

        if user {
            base = base.child("user").child(self.sub.clone());
        } else {
            base = base.child("global");
        }

        base = base.child("apps").child(self.app_id.clone());

        if !node {
            return Ok(base);
        }

        Ok(base.child(self.node_id.as_ref()))
    }

    pub fn get_storage(&self, node: bool) -> flow_like_types::Result<Path> {
        let base = self.board_dir.child("storage");

        if !node {
            return Ok(base);
        }

        Ok(base.child(self.node_id.as_ref()))
    }

    pub fn get_upload_dir(&self) -> flow_like_types::Result<Path> {
        let base = self.board_dir.child("upload");
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

#[derive(Clone)]
pub struct ExecutionContext {
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
    pub context_pin_overrides: Option<BTreeMap<String, Value>>,
    pub result: Option<Value>,
    pub oauth_tokens: Arc<AHashMap<String, OAuthToken>>,
    /// User context containing information about who triggered the execution
    pub user_context: Option<super::UserExecutionContext>,
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
}

impl ExecutionContext {
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
    ) -> Self {
        // Use cached node_id instead of locking
        let id = node.shared_node_id();
        let execution_cache = ExecutionContextCache::new(run, state, id.clone()).await;

        let mut trace = Trace::new_shared(id.clone());
        if log_level == LogLevel::Debug {
            trace.snapshot_variables(variables).await;
        }

        let (run_id, stream_state, log_spill_threshold, log_flush_interval, nodes_executed) =
            match run.upgrade() {
                Some(run) => {
                    let run = run.lock().await;
                    (
                        run.id.clone(),
                        run.stream_state,
                        run.log_spill_threshold,
                        super::DEFAULT_RUN_LOG_FLUSH_INTERVAL,
                        run.nodes_executed.clone(),
                    )
                }
                None => (
                    "".to_string(),
                    false,
                    super::DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD,
                    super::DEFAULT_RUN_LOG_FLUSH_INTERVAL,
                    Arc::new(AtomicU64::new(0)),
                ),
            };
        ExecutionContext {
            id,
            run_id,
            execution_environment: ExecutionEnvironment::Local,
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
            cancellation_token: None,
            user_context: None,
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
        })
    }

    pub fn callback(&self) -> &InterComCallback {
        &self.callback
    }

    pub async fn event_id(&self) -> Option<String> {
        let run = self.run.upgrade()?;
        let run = run.lock().await;
        run.event_id.clone()
    }

    /// Create ExecutionContext using cached RunMeta to avoid locking Run
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
            id,
            run_id: run_meta.run_id.clone(),
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
            cancellation_token: None,
            user_context: None,
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

    pub fn set_result(&mut self, value: Value) {
        self.result = Some(value);
    }

    pub fn override_pin_value(&mut self, pin_id: &str, value: Value) {
        if self.context_pin_overrides.is_none() {
            self.context_pin_overrides = Some(BTreeMap::new());
        }

        if let Some(overrides) = &mut self.context_pin_overrides {
            overrides.insert(pin_id.to_string(), value);
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
            id,
            run: self.run.clone(),
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
            context_pin_overrides: self.context_pin_overrides.clone(),
            result: None,
            oauth_tokens: self.oauth_tokens.clone(),
            user_context: self.user_context.clone(),
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
        let mut context = self.create_sub_context(node).await;

        // Clone the function's local variables with fresh value handles
        let mut local_vars = AHashMap::with_capacity(function_variables.len());
        for (var_id, var) in function_variables {
            let value = match &var.default_value {
                Some(bytes) => {
                    flow_like_types::json::from_slice::<Value>(bytes).unwrap_or(Value::Null)
                }
                None => Value::Null,
            };
            let mut cloned_var = var.clone();
            cloned_var.value = Arc::new(Mutex::new(value));
            local_vars.insert(var_id.clone(), cloned_var);
        }

        context.local_variables = Some(Arc::new(Mutex::new(local_vars)));
        context.context_pin_overrides = Some(BTreeMap::new());

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
    pub async fn get_variable_value_ref(
        &self,
        variable_id: &str,
    ) -> flow_like_types::Result<(Arc<Mutex<Value>>, bool)> {
        if let Some(local) = &self.local_variables {
            let local = local.lock().await;
            if let Some(variable) = local.get(variable_id) {
                return Ok((variable.value.clone(), variable.secret));
            }
        }

        let variables = self.variables.lock().await;
        let variable = variables
            .get(variable_id)
            .ok_or_else(|| flow_like_types::anyhow!("Variable not found"))?;
        Ok((variable.value.clone(), variable.secret))
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

    /// Returns the frontend elements map from the run payload.
    /// This is used by A2UI nodes to access element data passed from the frontend.
    /// Returns None if no elements are available.
    pub async fn get_frontend_elements(
        &self,
    ) -> flow_like_types::Result<Option<flow_like_types::json::Map<String, Value>>> {
        let payload = self.get_run_payload().await?;
        let elements = payload
            .payload
            .as_ref()
            .and_then(|p| p.get("_elements"))
            .and_then(|e| e.as_object())
            .cloned();
        Ok(elements)
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
        // Check local variables first
        if let Some(local) = &self.local_variables
            && let Some(var) = local.lock().await.get(variable_id)
        {
            let value_ref = var.value.clone();
            let mut guard = value_ref.lock().await;
            *guard = value;
            return Ok(());
        }

        let value_ref = self
            .variables
            .lock()
            .await
            .get(variable_id)
            .ok_or_else(|| flow_like_types::anyhow!("Variable not found"))?
            .value
            .clone();
        let mut guard = value_ref.lock().await;
        *guard = value;
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
        if self.trace.logs.is_empty() {
            return;
        }

        let spill_threshold = self.log_spill_threshold.max(1);
        let spill_by_size = self.trace.logs.len() >= spill_threshold;
        let spill_by_time = self.last_log_spill.elapsed() >= self.log_flush_interval;
        if !spill_by_size && !spill_by_time {
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
        let pin = self.get_pin_by_name(name).await?;
        let value = evaluate_pin_value(pin, &self.context_pin_overrides).await?;
        let value = from_value(value)?;
        Ok(value)
    }

    pub async fn evaluate_pin_to_ref(&self, name: &str) -> flow_like_types::Result<Value> {
        let pin = self.get_pin_by_name(name).await?;
        let value = evaluate_pin_value(pin, &self.context_pin_overrides).await?;
        Ok(value)
    }

    pub async fn evaluate_pin_ref<T: DeserializeOwned>(
        &self,
        reference: Arc<InternalPin>,
    ) -> flow_like_types::Result<T> {
        let value = evaluate_pin_value(reference, &self.context_pin_overrides).await?;
        let value = from_value(value)?;
        Ok(value)
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
        let pin_id = pin.id();

        // When in an override context, write to BOTH the override map AND the
        // shared pin. The override map provides per-invocation isolation so
        // read_outputs can read the correct value for each parallel call.
        // The shared pin write keeps bridge pin chains, get_raw_value() checks,
        // and other non-override-aware code paths working.
        if self.context_pin_overrides.is_some() {
            self.override_pin_value(pin_id, value.clone());
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
        traces.sort_by(|a, b| a.start.cmp(&b.start));
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
        let event = InterComEvent::with_type(event_type, event);
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
        tracing::debug!(message_type = ?message, "Streaming A2UI update");
        self.record_a2ui_update(&message).await;
        self.stream_response("a2ui", message).await
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

        // Get-or-insert under a single write lock: parallel branches emitting
        // the run's first update must not race two logs into existence.
        let log = {
            let mut cache = self.cache.write().await;
            match cache
                .get(A2UI_UPDATE_LOG_KEY)
                .and_then(|c| c.as_any().downcast_ref::<A2UIUpdateLog>().cloned())
            {
                Some(log) => log,
                None => {
                    let log = A2UIUpdateLog::default();
                    cache.insert(
                        A2UI_UPDATE_LOG_KEY.to_string(),
                        Arc::new(log.clone()) as Arc<dyn Cacheable>,
                    );
                    log
                }
            }
        };

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

    pub async fn request_elements(
        &mut self,
        element_ids: Vec<String>,
    ) -> flow_like_types::Result<()> {
        let message = crate::a2ui::A2UIServerMessage::request_elements(element_ids);
        self.stream_a2ui_update(message).await
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
        tracing::info!(element_id = %element_id, value = ?value, "[A2UI] upsert_element called");
        self.log_message(
            &format!("[A2UI] upsert_element: {} -> {:?}", element_id, value),
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
        let message =
            crate::a2ui::A2UIServerMessage::create_element(surface_id, parent_id, component, index);
        self.stream_a2ui_update(message).await
    }

    pub async fn remove_element(
        &mut self,
        surface_id: &str,
        element_id: &str,
    ) -> flow_like_types::Result<()> {
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
}
