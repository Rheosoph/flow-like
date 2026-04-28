use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::lance::session::Session as LanceSession;
use flow_like_storage::lancedb::connection::ConnectBuilder;
use flow_like_storage::object_store::path::Path;
use flow_like_types::Ok;
use flow_like_types::sync::{DashMap, Mutex, RwLock};
#[cfg(feature = "flow-runtime")]
use flow_like_types::tokio_util::sync::CancellationToken;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Weak};
#[cfg(feature = "flow-runtime")]
use std::time::Instant;

use crate::flow::event::Event;
#[cfg(feature = "flow-runtime")]
use crate::flow::execution::{LogMeta, log::LogMessage};

#[cfg(feature = "flow-runtime")]
use crate::flow::board::Board;
use crate::flow::node::Node;
#[cfg(feature = "flow-runtime")]
use crate::flow::node::NodeLogic;

use crate::models::embedding_factory::EmbeddingFactory;
#[cfg(feature = "model")]
use crate::models::llm::ModelFactory;
#[cfg(feature = "bit")]
use crate::utils::download_manager::DownloadManager;
use crate::utils::http::HTTPClient;
#[cfg(feature = "model")]
use flow_like_model_provider::provider::ModelProviderConfiguration;

#[derive(Clone, Default)]
pub struct FlowLikeStores {
    pub bits_store: Option<FlowLikeStore>,
    pub user_store: Option<FlowLikeStore>,
    pub app_storage_store: Option<FlowLikeStore>,
    pub app_meta_store: Option<FlowLikeStore>,
    pub temporary_store: Option<FlowLikeStore>,
    pub log_store: Option<FlowLikeStore>,
}

#[derive(Clone)]
pub struct FlowLikeCallbacks {
    pub build_project_database: Option<Arc<dyn (Fn(Path) -> ConnectBuilder) + Send + Sync>>,
    pub build_user_database: Option<Arc<dyn (Fn(Path) -> ConnectBuilder) + Send + Sync>>,
    pub build_logs_database: Option<Arc<dyn (Fn(Path) -> ConnectBuilder) + Send + Sync>>,
    /// Default write options for LanceDB. Android overrides these to add its object store wrapper.
    pub lance_write_options: Option<flow_like_storage::lancedb::table::WriteOptions>,
}

impl Default for FlowLikeCallbacks {
    fn default() -> Self {
        Self {
            build_project_database: None,
            build_user_database: None,
            build_logs_database: None,
            lance_write_options: Some(
                flow_like_storage::lancedb_write_options::default_write_options(),
            ),
        }
    }
}

#[derive(Clone, Default)]
pub struct FlowLikeConfig {
    pub stores: FlowLikeStores,
    pub callbacks: FlowLikeCallbacks,
}

impl FlowLikeConfig {
    pub fn new() -> Self {
        FlowLikeConfig {
            callbacks: FlowLikeCallbacks::default(),
            stores: FlowLikeStores::default(),
        }
    }

    pub fn with_default_store(store: FlowLikeStore) -> Self {
        FlowLikeConfig {
            callbacks: FlowLikeCallbacks::default(),
            stores: FlowLikeStores {
                app_storage_store: Some(store.clone()),
                app_meta_store: Some(store.clone()),
                bits_store: Some(store.clone()),
                user_store: Some(store.clone()),
                temporary_store: Some(store.clone()),
                log_store: Some(store),
            },
        }
    }

    pub fn register_app_storage_store(&mut self, store: FlowLikeStore) {
        self.stores.app_storage_store = Some(store);
    }

    pub fn register_app_meta_store(&mut self, store: FlowLikeStore) {
        self.stores.app_meta_store = Some(store);
    }

    pub fn register_user_store(&mut self, store: FlowLikeStore) {
        self.stores.user_store = Some(store);
    }

    pub fn register_bits_store(&mut self, store: FlowLikeStore) {
        self.stores.bits_store = Some(store);
    }

    pub fn register_temporary_store(&mut self, store: FlowLikeStore) {
        self.stores.temporary_store = Some(store);
    }

    pub fn register_log_store(&mut self, store: FlowLikeStore) {
        self.stores.log_store = Some(store);
    }

    pub fn register_build_project_database(
        &mut self,
        callback: Arc<dyn (Fn(Path) -> ConnectBuilder) + Send + Sync>,
    ) {
        self.callbacks.build_project_database = Some(callback);
    }

    pub fn register_build_user_database(
        &mut self,
        callback: Arc<dyn (Fn(Path) -> ConnectBuilder) + Send + Sync>,
    ) {
        self.callbacks.build_user_database = Some(callback);
    }

    pub fn register_build_logs_database(
        &mut self,
        callback: Arc<dyn (Fn(Path) -> ConnectBuilder) + Send + Sync>,
    ) {
        self.callbacks.build_logs_database = Some(callback);
    }

    pub fn register_lance_write_options(
        &mut self,
        options: flow_like_storage::lancedb::table::WriteOptions,
    ) {
        self.callbacks.lance_write_options = Some(options);
    }
}

#[cfg(feature = "flow-runtime")]
#[derive(Default, Clone)]
pub struct FlowNodeRegistryInner {
    pub registry: HashMap<String, (Node, Arc<dyn NodeLogic>)>,
}

impl FlowNodeRegistryInner {
    pub fn new(size: usize) -> Self {
        FlowNodeRegistryInner {
            registry: HashMap::with_capacity(size),
        }
    }

    pub fn insert(&mut self, node: Node, logic: Arc<dyn NodeLogic>) {
        self.registry.insert(node.name.clone(), (node, logic));
    }

    pub fn get_nodes(&self) -> Vec<Node> {
        self.registry.values().map(|node| node.0.clone()).collect()
    }

    pub fn prepare(nodes: &Arc<Vec<Arc<dyn NodeLogic>>>) -> Self {
        let mut registry = FlowNodeRegistryInner {
            registry: HashMap::with_capacity(nodes.len()),
        };

        for logic in nodes.iter() {
            let node = logic.get_node();
            registry.insert(node, logic.clone());
        }

        registry
    }

    #[inline]
    pub fn get_node(&self, node_id: &str) -> flow_like_types::Result<Node> {
        let node = self.registry.get(node_id);
        match node {
            Some(node) => Ok(node.0.clone()),
            None => Err(flow_like_types::anyhow!(
                "Node not found - Get Node: '{}'",
                node_id
            )),
        }
    }

    #[inline]
    pub fn instantiate(&self, node: &Node) -> flow_like_types::Result<Arc<dyn NodeLogic>> {
        let result = self.registry.get(&node.name);
        match result {
            Some(entry) => Ok(entry.1.clone()),
            None => Err(flow_like_types::anyhow!(
                "Node not found - Instancing: '{}' (id: {}, category: {})",
                node.name,
                node.id,
                node.category
            )),
        }
    }
}

#[cfg(feature = "flow-runtime")]
pub struct FlowNodeRegistry {
    pub node_registry: Arc<FlowNodeRegistryInner>,
    pub parent: Option<Weak<FlowLikeState>>,
}

#[cfg(feature = "flow-runtime")]
impl Default for FlowNodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FlowNodeRegistry {
    pub fn new() -> Self {
        FlowNodeRegistry {
            node_registry: Arc::new(FlowNodeRegistryInner::new(0)),
            parent: None,
        }
    }

    pub fn initialize(&mut self, parent: Weak<FlowLikeState>) {
        self.parent = Some(parent);
    }

    pub fn get_nodes(&self) -> flow_like_types::Result<Vec<Node>> {
        let nodes = self.node_registry.get_nodes();
        Ok(nodes)
    }

    pub fn push_node(&mut self, logic: Arc<dyn NodeLogic>) {
        let mut registry = FlowNodeRegistryInner {
            registry: self.node_registry.registry.clone(),
        };
        let node = logic.get_node();
        registry.insert(node, logic);
        self.node_registry = Arc::new(registry);
    }

    pub fn push_nodes(&mut self, nodes: Vec<Arc<dyn NodeLogic>>) {
        let mut registry = FlowNodeRegistryInner {
            registry: self.node_registry.registry.clone(),
        };

        for logic in nodes {
            let node = logic.get_node();
            registry.insert(node, logic);
        }
        self.node_registry = Arc::new(registry);
    }

    pub fn get_node(&self, node_id: &str) -> flow_like_types::Result<Node> {
        let node = self.node_registry.get_node(node_id)?;
        Ok(node)
    }

    pub fn instantiate(&self, node: &Node) -> flow_like_types::Result<Arc<dyn NodeLogic>> {
        let node = self.node_registry.instantiate(node)?;
        Ok(node)
    }
}

use std::sync::atomic::AtomicU64;

#[derive(Clone)]
pub struct RunData {
    pub start_time: Instant,
    pub board_id: Arc<str>,
    pub node_id: Arc<str>,
    pub event_id: Option<Arc<str>>,
    pub cancellation_token: CancellationToken,
    pub board_name: Option<Arc<str>>,
    pub event_name: Option<Arc<str>>,
    pub event_type: Option<Arc<str>>,
    /// Timestamp (ms since epoch) of the last node update event
    last_node_update_ms: Arc<AtomicU64>,
}

impl RunData {
    pub fn new(
        board_id: &str,
        node_id: &str,
        event_id: Option<String>,
        cancellation_token: CancellationToken,
    ) -> Self {
        RunData {
            start_time: Instant::now(),
            board_id: Arc::from(board_id),
            node_id: Arc::from(node_id),
            event_id: event_id.map(|s| Arc::from(s.as_str())),
            cancellation_token,
            board_name: None,
            event_name: None,
            event_type: None,
            last_node_update_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_metadata(
        board_id: &str,
        node_id: &str,
        event_id: Option<String>,
        cancellation_token: CancellationToken,
        board_name: Option<String>,
        event_name: Option<String>,
        event_type: Option<String>,
    ) -> Self {
        RunData {
            start_time: Instant::now(),
            board_id: Arc::from(board_id),
            node_id: Arc::from(node_id),
            event_id: event_id.map(|s| Arc::from(s.as_str())),
            cancellation_token,
            board_name: board_name.map(|s| Arc::from(s.as_str())),
            event_name: event_name.map(|s| Arc::from(s.as_str())),
            event_type: event_type.map(|s| Arc::from(s.as_str())),
            last_node_update_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }

    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Update the last node update timestamp to now
    pub fn touch_last_node_update(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.last_node_update_ms
            .store(now, std::sync::atomic::Ordering::Relaxed);
    }

    /// Get the last node update timestamp in milliseconds since epoch
    pub fn get_last_node_update_ms(&self) -> u64 {
        self.last_node_update_ms
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn from_event(event: &Event, cancellation_token: CancellationToken) -> Self {
        RunData {
            start_time: Instant::now(),
            board_id: Arc::from(event.board_id.as_str()),
            node_id: Arc::from(event.node_id.as_str()),
            event_id: Some(Arc::from(event.id.as_str())),
            cancellation_token,
            board_name: None,
            event_name: Some(Arc::from(event.name.as_str())),
            event_type: Some(Arc::from(event.event_type.as_str())),
            last_node_update_ms: Arc::new(AtomicU64::new(0)),
        }
    }
}

// TODO: implement dashmap
#[derive(Clone)]
pub struct FlowLikeState {
    pub config: Arc<RwLock<FlowLikeConfig>>,
    pub http_client: Arc<HTTPClient>,
    pub lance_session: Arc<LanceSession>,

    #[cfg(feature = "bit")]
    pub download_manager: Arc<Mutex<DownloadManager>>,

    #[cfg(feature = "model")]
    pub model_provider_config: Arc<ModelProviderConfiguration>,

    #[cfg(feature = "model")]
    pub model_factory: Arc<Mutex<ModelFactory>>,
    #[cfg(feature = "model")]
    pub embedding_factory: Arc<Mutex<EmbeddingFactory>>,

    #[cfg(feature = "flow-runtime")]
    pub node_registry: Arc<RwLock<FlowNodeRegistry>>,
    #[cfg(feature = "flow-runtime")]
    pub board_registry: Arc<DashMap<String, Arc<Mutex<Board>>>>, // TODO: should board be wrapped in RWLock or Mutex?
    #[cfg(feature = "flow-runtime")]
    pub board_run_registry: Arc<DashMap<String, Arc<RunData>>>,
    /// Cache of precompiled board graph topologies. Keyed by board ID.
    /// Compiled boards cache the expensive graph construction (pin wiring,
    /// node instantiation, dependency resolution) so subsequent executions
    /// of the same board version skip all topology work.
    #[cfg(feature = "flow-runtime")]
    pub compiled_boards: Arc<DashMap<String, Arc<crate::flow::execution::compiled::CompiledBoard>>>,

    // A2UI registries for open widgets/pages
    #[cfg(feature = "flow-runtime")]
    pub widget_registry: Arc<DashMap<String, crate::a2ui::widget::Widget>>,
    #[cfg(feature = "flow-runtime")]
    pub page_registry: Arc<DashMap<String, crate::a2ui::widget::Page>>,
}

impl FlowLikeState {
    pub fn new(config: FlowLikeConfig, client: HTTPClient) -> Self {
        FlowLikeState {
            config: Arc::new(RwLock::new(config)),
            http_client: Arc::new(client),
            lance_session: Arc::new(LanceSession::default()),

            #[cfg(feature = "bit")]
            download_manager: Arc::new(Mutex::new(DownloadManager::new())),

            #[cfg(feature = "model")]
            model_provider_config: Arc::new(ModelProviderConfiguration::default()),
            #[cfg(feature = "model")]
            model_factory: Arc::new(Mutex::new(ModelFactory::new())),

            #[cfg(feature = "model")]
            embedding_factory: Arc::new(Mutex::new(EmbeddingFactory::new())),

            #[cfg(feature = "flow-runtime")]
            node_registry: Arc::new(RwLock::new(FlowNodeRegistry::new())),
            #[cfg(feature = "flow-runtime")]
            board_registry: Arc::new(DashMap::new()),
            #[cfg(feature = "flow-runtime")]
            board_run_registry: Arc::new(DashMap::new()),
            #[cfg(feature = "flow-runtime")]
            compiled_boards: Arc::new(DashMap::new()),

            #[cfg(feature = "flow-runtime")]
            widget_registry: Arc::new(DashMap::new()),
            #[cfg(feature = "flow-runtime")]
            page_registry: Arc::new(DashMap::new()),
        }
    }

    #[cfg(feature = "model")]
    pub fn new_with_model_config(
        config: FlowLikeConfig,
        client: HTTPClient,
        model_provider_config: ModelProviderConfiguration,
    ) -> Self {
        FlowLikeState {
            config: Arc::new(RwLock::new(config)),
            http_client: Arc::new(client),
            lance_session: Arc::new(LanceSession::default()),

            #[cfg(feature = "bit")]
            download_manager: Arc::new(Mutex::new(DownloadManager::new())),

            model_provider_config: Arc::new(model_provider_config),
            model_factory: Arc::new(Mutex::new(ModelFactory::new())),
            embedding_factory: Arc::new(Mutex::new(EmbeddingFactory::new())),

            #[cfg(feature = "flow-runtime")]
            node_registry: Arc::new(RwLock::new(FlowNodeRegistry::new())),
            #[cfg(feature = "flow-runtime")]
            board_registry: Arc::new(DashMap::new()),
            #[cfg(feature = "flow-runtime")]
            board_run_registry: Arc::new(DashMap::new()),
            #[cfg(feature = "flow-runtime")]
            compiled_boards: Arc::new(DashMap::new()),

            #[cfg(feature = "flow-runtime")]
            widget_registry: Arc::new(DashMap::new()),
            #[cfg(feature = "flow-runtime")]
            page_registry: Arc::new(DashMap::new()),
        }
    }

    #[cfg(feature = "bit")]
    pub fn download_manager(&self) -> Arc<Mutex<DownloadManager>> {
        self.download_manager.clone()
    }

    #[cfg(feature = "model")]
    pub fn model_factory(&self) -> Arc<Mutex<ModelFactory>> {
        self.model_factory.clone()
    }

    pub fn with_lance_session(&self, builder: ConnectBuilder) -> ConnectBuilder {
        builder.session(self.lance_session.clone())
    }

    pub fn for_execution_run(&self) -> Self {
        FlowLikeState {
            config: self.config.clone(),
            http_client: self.http_client.clone(),
            lance_session: Arc::new(LanceSession::default()),

            #[cfg(feature = "bit")]
            download_manager: self.download_manager.clone(),

            #[cfg(feature = "model")]
            model_provider_config: self.model_provider_config.clone(),
            #[cfg(feature = "model")]
            model_factory: self.model_factory.clone(),
            #[cfg(feature = "model")]
            embedding_factory: self.embedding_factory.clone(),

            #[cfg(feature = "flow-runtime")]
            node_registry: self.node_registry.clone(),
            #[cfg(feature = "flow-runtime")]
            board_registry: Arc::new(DashMap::new()),
            #[cfg(feature = "flow-runtime")]
            board_run_registry: Arc::new(DashMap::new()),
            #[cfg(feature = "flow-runtime")]
            compiled_boards: self.compiled_boards.clone(),

            #[cfg(feature = "flow-runtime")]
            widget_registry: Arc::new(DashMap::new()),
            #[cfg(feature = "flow-runtime")]
            page_registry: Arc::new(DashMap::new()),
        }
    }

    #[cfg(feature = "flow-runtime")]
    pub fn node_registry(&self) -> Arc<RwLock<FlowNodeRegistry>> {
        self.node_registry.clone()
    }

    #[cfg(feature = "flow-runtime")]
    pub fn board_registry(&self) -> Arc<DashMap<String, Arc<Mutex<Board>>>> {
        self.board_registry.clone()
    }

    #[cfg(feature = "flow-runtime")]
    pub fn get_board(
        &self,
        board_id: &str,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Arc<Mutex<Board>>> {
        let key = if let Some(version) = version {
            format!("{}-{}-{}-{}", board_id, version.0, version.1, version.2)
        } else {
            board_id.to_string()
        };

        let board = self.board_registry.try_get(&key);

        match board.try_unwrap() {
            Some(board) => Ok(board.clone()),
            None => Err(flow_like_types::anyhow!(
                "Board not found or could not be locked"
            )),
        }
    }

    #[cfg(feature = "flow-runtime")]
    pub fn get_template(
        &self,
        template_id: &str,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Arc<Mutex<Board>>> {
        let key = if let Some(version) = version {
            format!("{}-{}-{}-{}", template_id, version.0, version.1, version.2)
        } else {
            template_id.to_string()
        };

        let board = self.board_registry.try_get(&key);

        match board.try_unwrap() {
            Some(board) => Ok(board.clone()),
            None => Err(flow_like_types::anyhow!(
                "Board not found or could not be locked"
            )),
        }
    }

    #[cfg(feature = "flow-runtime")]
    pub fn remove_board(
        &self,
        board_id: &str,
    ) -> flow_like_types::Result<Option<Arc<Mutex<Board>>>> {
        let removed = self.board_registry.remove(board_id);

        match removed {
            Some((_id, board)) => Ok(Some(board)),
            None => Ok(None),
        }
    }

    #[cfg(feature = "flow-runtime")]
    pub fn register_board(
        &self,
        board_id: &str,
        board: Arc<Mutex<Board>>,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<()> {
        let key = if let Some(version) = version {
            format!("{}-{}-{}-{}", board_id, version.0, version.1, version.2)
        } else {
            board_id.to_string()
        };
        self.board_registry.insert(key, board.clone());
        Ok(())
    }

    #[cfg(feature = "flow-runtime")]
    pub fn board_run_registry(&self) -> Arc<DashMap<String, Arc<RunData>>> {
        self.board_run_registry.clone()
    }

    #[cfg(feature = "flow-runtime")]
    pub fn list_runs(&self) -> flow_like_types::Result<Vec<(String, Arc<RunData>)>> {
        let runs = self
            .board_run_registry
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect::<Vec<_>>();
        Ok(runs)
    }

    #[cfg(feature = "flow-runtime")]
    pub fn register_run(&self, run_id: &str, run: RunData) {
        self.board_run_registry
            .insert(run_id.to_string(), Arc::new(run));
    }

    #[cfg(feature = "flow-runtime")]
    pub fn remove_and_cancel_run(&self, run_id: &str) -> flow_like_types::Result<()> {
        let removed = self.board_run_registry.remove(run_id);
        if let Some((_id, run)) = removed
            && !run.is_cancelled()
        {
            run.cancel();
        }

        Ok(())
    }

    #[cfg(feature = "flow-runtime")]
    pub fn get_run(&self, run_id: &str) -> flow_like_types::Result<Arc<RunData>> {
        let run = self.board_run_registry.try_get(run_id);

        match run.try_unwrap() {
            Some(run) => Ok(run.clone()),
            None => Err(flow_like_types::anyhow!(
                "Run not found or could not be locked"
            )),
        }
    }

    #[cfg(feature = "flow-runtime")]
    pub async fn query_run(
        &self,
        meta: &LogMeta,
        query: &str,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> flow_like_types::Result<Vec<LogMessage>> {
        use flow_like_storage::{
            lancedb::query::{ExecutableQuery, QueryBase},
            serde_arrow,
        };
        use flow_like_types::anyhow;
        use futures::TryStreamExt;

        use crate::flow::execution::log::StoredLogMessage;

        let limit = limit.unwrap_or(100);
        let offset = offset.unwrap_or(0);

        let db = {
            let guard = self.config.read().await;

            guard.callbacks.build_logs_database.clone()
        };

        let db_fn = db
            .as_ref()
            .ok_or_else(|| anyhow!("No log database configured"))?;
        let base_path = Path::from("runs")
            .child(meta.app_id.clone())
            .child(meta.board_id.clone());
        let db = db_fn(base_path.clone()).execute().await?;

        let db = db.open_table(meta.run_id.clone()).execute().await?;
        let mut q = db.query();

        if !query.is_empty() {
            q = q.only_if(query);
        }

        let results = q.offset(offset).limit(limit).execute().await?;
        let results = results.try_collect::<Vec<_>>().await?;

        let mut log_messages = Vec::with_capacity(results.len() * 10);
        for result in results {
            let result = serde_arrow::from_record_batch::<Vec<StoredLogMessage>>(&result)
                .unwrap_or_default();
            let result = result
                .into_iter()
                .map(|log| {
                    let log: LogMessage = log.into();
                    log
                })
                .collect::<Vec<_>>();
            log_messages.extend(result);
        }

        Ok(log_messages)
    }

    #[inline]
    pub async fn stores(state: &Arc<FlowLikeState>) -> FlowLikeStores {
        state.config.read().await.stores.clone()
    }

    #[inline]
    pub async fn project_storage_store(
        state: &Arc<FlowLikeState>,
    ) -> flow_like_types::Result<FlowLikeStore> {
        state
            .config
            .read()
            .await
            .stores
            .app_storage_store
            .clone()
            .ok_or(flow_like_types::anyhow!("No project store"))
    }

    #[inline]
    pub async fn project_meta_store(
        state: &Arc<FlowLikeState>,
    ) -> flow_like_types::Result<FlowLikeStore> {
        state
            .config
            .read()
            .await
            .stores
            .app_meta_store
            .clone()
            .ok_or(flow_like_types::anyhow!("No project store"))
    }

    #[inline]
    pub async fn bit_store(state: &Arc<FlowLikeState>) -> flow_like_types::Result<FlowLikeStore> {
        state
            .config
            .read()
            .await
            .stores
            .bits_store
            .clone()
            .ok_or(flow_like_types::anyhow!("No bit store"))
    }

    #[inline]
    pub async fn user_store(state: &Arc<FlowLikeState>) -> flow_like_types::Result<FlowLikeStore> {
        state
            .config
            .read()
            .await
            .stores
            .user_store
            .clone()
            .ok_or(flow_like_types::anyhow!("No user store"))
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ToastLevel {
    Success,
    Info,
    Warning,
    Error,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ToastEvent {
    pub message: String,
    pub level: ToastLevel,
}

impl ToastEvent {
    pub fn new(message: &str, level: ToastLevel) -> Self {
        ToastEvent {
            message: message.to_string(),
            level,
        }
    }
}

impl Default for ToastEvent {
    fn default() -> Self {
        ToastEvent {
            message: "".to_string(),
            level: ToastLevel::Info,
        }
    }
}

/// Event sent via InterCom to show progress to the user
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProgressEvent {
    /// Unique identifier to update/dismiss this progress toast
    pub id: String,
    /// The message shown to the user
    pub message: String,
    /// Progress value between 0 and 100 (None to dismiss)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    /// Whether this is the final update (dismisses or completes the progress)
    #[serde(default)]
    pub done: bool,
    /// Whether the operation succeeded (only relevant when done=true)
    #[serde(default)]
    pub success: bool,
}

impl ProgressEvent {
    pub fn new(id: &str, message: &str, progress: Option<u8>) -> Self {
        ProgressEvent {
            id: id.to_string(),
            message: message.to_string(),
            progress,
            done: false,
            success: false,
        }
    }

    pub fn done(id: &str, message: &str, success: bool) -> Self {
        ProgressEvent {
            id: id.to_string(),
            message: message.to_string(),
            progress: if success { Some(100) } else { None },
            done: true,
            success,
        }
    }
}

/// Event sent via InterCom to notify the user
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NotificationEvent {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    pub show_desktop: bool,

    /// Event ID that triggered this workflow execution.
    /// If present, UIs can persist the notification via the backend API.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,

    /// Target user sub (for project-user notifications)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_user_sub: Option<String>,

    /// Optional tracking info
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_node_id: Option<String>,
}

impl NotificationEvent {
    pub fn new(title: &str) -> Self {
        NotificationEvent {
            title: title.to_string(),
            description: None,
            icon: None,
            link: None,
            show_desktop: true,
            event_id: None,
            target_user_sub: None,
            source_run_id: None,
            source_node_id: None,
        }
    }

    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    pub fn with_icon(mut self, icon: &str) -> Self {
        self.icon = Some(icon.to_string());
        self
    }

    pub fn with_link(mut self, link: &str) -> Self {
        self.link = Some(link.to_string());
        self
    }

    pub fn with_event_id(mut self, event_id: &str) -> Self {
        if !event_id.trim().is_empty() {
            self.event_id = Some(event_id.to_string());
        }
        self
    }

    pub fn with_target_user_sub(mut self, target_user_sub: &str) -> Self {
        if !target_user_sub.trim().is_empty() {
            self.target_user_sub = Some(target_user_sub.to_string());
        }
        self
    }

    pub fn with_source_run_id(mut self, run_id: &str) -> Self {
        if !run_id.trim().is_empty() {
            self.source_run_id = Some(run_id.to_string());
        }
        self
    }

    pub fn with_source_node_id(mut self, node_id: &str) -> Self {
        if !node_id.trim().is_empty() {
            self.source_node_id = Some(node_id.to_string());
        }
        self
    }

    pub fn with_desktop(mut self, show_desktop: bool) -> Self {
        self.show_desktop = show_desktop;
        self
    }
}

impl Default for NotificationEvent {
    fn default() -> Self {
        NotificationEvent {
            title: String::new(),
            description: None,
            icon: None,
            link: None,
            show_desktop: true,
            event_id: None,
            target_user_sub: None,
            source_run_id: None,
            source_node_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use flow_like_storage::object_store::{ObjectStore, PutPayload};
    use flow_like_types::{Bytes, Cacheable, tokio};

    use super::*;
    use std::path::PathBuf;

    #[test]
    fn object_store_path_serialization() {
        let path = Path::from("test").child("path").child("one");
        let event = PathBuf::from("random").join(path.to_string());
        assert_eq!(path.to_string(), "test/path/one".to_string());
        assert_eq!(event.to_str().unwrap(), "random/test/path/one");
    }

    #[tokio::test]
    async fn test_object_store_any_cast() {
        let memory_store = flow_like_storage::object_store::memory::InMemory::new();
        let test_string = b"Hi, I am Testing";
        let test_path = Path::from("test");
        memory_store
            .put(&test_path, PutPayload::from_static(test_string))
            .await
            .unwrap();
        let store: FlowLikeStore = FlowLikeStore::Other(Arc::new(memory_store));
        let store: Arc<dyn Cacheable> = Arc::new(store.clone());
        let down_casted: &FlowLikeStore = store.downcast_ref().unwrap();
        let read_bytes: Bytes = down_casted
            .as_generic()
            .get(&test_path)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let test_bytes = Bytes::from_static(test_string);
        assert_eq!(read_bytes, test_bytes);
    }
}
