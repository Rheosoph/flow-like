//! A function-cache miss must not keep the function's execution successor from running while the
//! cache backend is still persisting the result.

extern crate flow_like_runtime as flow_like;

use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use flow_like::{
    app::AppVisibility,
    flow::{
        board::{Board, Layer, LayerCache, LayerCacheScope, LayerType},
        execution::{InternalRun, LogLevel, RunPayload, RunStatus, context::ExecutionContext},
        node::{Node, NodeLogic},
        variable::VariableType,
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_catalog_std::{control::call_function::CallFunctionNode, logging::info::InfoNode};
use flow_like_storage::{
    Path,
    files::store::FlowLikeStore,
    object_store::{
        GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
        PutMultipartOptions, PutOptions, PutPayload, PutResult, Result as ObjectStoreResult,
        memory::InMemory, path::Path as ObjectPath,
    },
};
use flow_like_types::{async_trait, json::json};
use futures::stream::BoxStream;
use tokio::sync::{Semaphore, oneshot};

const APP_ID: &str = "function-cache-continuation";
const CALL_ID: &str = "cached-call";
const FUNCTION_ID: &str = "cached-function";
const FUNCTION_NODE_ID: &str = "function-body";
const SUCCESSOR_ID: &str = "after-cached-call";

struct CountingBodyNode {
    executions: Arc<AtomicU64>,
}

impl CountingBodyNode {
    fn new(executions: Arc<AtomicU64>) -> Self {
        Self { executions }
    }
}

#[async_trait]
impl NodeLogic for CountingBodyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "test_count_cached_function_body",
            "Count Cached Function Body",
            "Counts executions of the cached function body",
            "Tests",
        );
        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "", VariableType::Execution);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        self.executions.fetch_add(1, Ordering::AcqRel);
        context.activate_exec_pin("exec_out").await
    }
}

#[derive(Debug)]
struct GatedStore {
    inner: InMemory,
    put_count: AtomicU64,
    write_started: Mutex<Option<oneshot::Sender<()>>>,
    write_release: Arc<Semaphore>,
}

impl GatedStore {
    fn new() -> (Arc<Self>, oneshot::Receiver<()>, Arc<Semaphore>) {
        let (write_started_tx, write_started) = oneshot::channel();
        let write_release = Arc::new(Semaphore::new(0));
        let store = Arc::new(Self {
            inner: InMemory::new(),
            put_count: AtomicU64::new(0),
            write_started: Mutex::new(Some(write_started_tx)),
            write_release: write_release.clone(),
        });
        (store, write_started, write_release)
    }

    fn put_count(&self) -> u64 {
        self.put_count.load(Ordering::Acquire)
    }
}

impl Display for GatedStore {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GatedStore")
    }
}

#[async_trait]
impl ObjectStore for GatedStore {
    async fn put_opts(
        &self,
        location: &ObjectPath,
        payload: PutPayload,
        options: PutOptions,
    ) -> ObjectStoreResult<PutResult> {
        self.put_count.fetch_add(1, Ordering::AcqRel);
        let write_started = self
            .write_started
            .lock()
            .expect("cache write-start lock")
            .take();
        if let Some(write_started) = write_started {
            let _ = write_started.send(());
            let permit = self
                .write_release
                .acquire()
                .await
                .expect("cache write gate remains open");
            permit.forget();
        }
        self.inner.put_opts(location, payload, options).await
    }

    async fn put_multipart_opts(
        &self,
        location: &ObjectPath,
        options: PutMultipartOptions,
    ) -> ObjectStoreResult<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, options).await
    }

    async fn get_opts(
        &self,
        location: &ObjectPath,
        options: GetOptions,
    ) -> ObjectStoreResult<GetResult> {
        self.inner.get_opts(location, options).await
    }

    async fn delete(&self, location: &ObjectPath) -> ObjectStoreResult<()> {
        self.inner.delete(location).await
    }

    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, ObjectStoreResult<ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> ObjectStoreResult<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy(&self, from: &ObjectPath, to: &ObjectPath) -> ObjectStoreResult<()> {
        self.inner.copy(from, to).await
    }

    async fn copy_if_not_exists(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
    ) -> ObjectStoreResult<()> {
        self.inner.copy_if_not_exists(from, to).await
    }
}

fn pin_id(node: &Node, name: &str) -> String {
    node.get_pin_by_name(name)
        .unwrap_or_else(|| panic!("node '{}' has pin '{name}'", node.name))
        .id
        .clone()
}

fn connect_exec_nodes(from: &mut Node, from_pin: &str, to: &mut Node, to_pin: &str) {
    let from_pin = pin_id(from, from_pin);
    let to_pin = pin_id(to, to_pin);
    from.pins
        .get_mut(&from_pin)
        .expect("source execution pin")
        .connected_to
        .insert(to_pin.clone());
    to.pins
        .get_mut(&to_pin)
        .expect("target execution pin")
        .depends_on
        .insert(from_pin);
}

async fn cached_function_board(body_logic: &CountingBodyNode) -> Board {
    let mut board = Board::new_detached(
        Some("cache-continuation-board".to_string()),
        Path::default(),
    );
    board.name = "Function Cache Continuation".to_string();
    board.log_level = LogLevel::Info;

    let mut layer = Layer::new(
        FUNCTION_ID.to_string(),
        "Cached Function".to_string(),
        LayerType::Function,
    );
    layer.cache = Some(LayerCache {
        enabled: true,
        prefix: "cache-continuation".to_string(),
        ttl_seconds: Some(60),
        scope: LayerCacheScope::App,
    });

    let mut boundary = Node::new("function-boundary", "Function Boundary", "", "Tests");
    let layer_exec_in = boundary
        .add_input_pin("exec_in", "Input", "", VariableType::Execution)
        .clone();
    let layer_exec_out = boundary
        .add_output_pin("exec_out", "Output", "", VariableType::Execution)
        .clone();
    let layer_exec_in_id = layer_exec_in.id.clone();
    let layer_exec_out_id = layer_exec_out.id.clone();
    layer.pins.insert(layer_exec_in.id.clone(), layer_exec_in);
    layer.pins.insert(layer_exec_out.id.clone(), layer_exec_out);

    let mut function_node = body_logic.get_node();
    function_node.id = FUNCTION_NODE_ID.to_string();
    function_node.layer = Some(FUNCTION_ID.to_string());
    let function_exec_in_id = pin_id(&function_node, "exec_in");
    let function_exec_out_id = pin_id(&function_node, "exec_out");

    layer
        .pins
        .get_mut(&layer_exec_in_id)
        .expect("function execution input")
        .connected_to
        .insert(function_exec_in_id.clone());
    function_node
        .pins
        .get_mut(&function_exec_in_id)
        .expect("function entry execution pin")
        .depends_on
        .insert(layer_exec_in_id);
    function_node
        .pins
        .get_mut(&function_exec_out_id)
        .expect("function exit execution pin")
        .connected_to
        .insert(layer_exec_out_id.clone());
    layer
        .pins
        .get_mut(&layer_exec_out_id)
        .expect("function execution output")
        .depends_on
        .insert(function_exec_out_id);
    layer.nodes.insert(function_node.id.clone(), function_node);
    board.layers.insert(layer.id.clone(), layer);

    let call_logic = CallFunctionNode::new();
    let mut call = call_logic.get_node();
    call.id = CALL_ID.to_string();
    call.get_pin_mut_by_name("function_layer_id")
        .expect("call function selector")
        .set_default_value(Some(json!(FUNCTION_ID)));
    call_logic.on_update(&mut call, &board).await;

    let mut successor = InfoNode::new().get_node();
    successor.id = SUCCESSOR_ID.to_string();
    connect_exec_nodes(&mut call, "exec_out", &mut successor, "exec_in");
    board.nodes.insert(call.id.clone(), call);
    board.nodes.insert(successor.id.clone(), successor);

    board
}

async fn wait_until_executed(exec_calls: &AtomicU64) {
    while exec_calls.load(Ordering::Acquire) == 0 {
        tokio::task::yield_now().await;
    }
}

async fn build_execution(
    state: &Arc<FlowLikeState>,
    board: Arc<Board>,
    payload: &RunPayload,
) -> InternalRun {
    let mut execution = InternalRun::new(
        APP_ID,
        board,
        None,
        state,
        &Profile::default(),
        payload,
        false,
        None,
        None,
        None,
        HashMap::new(),
    )
    .await
    .expect("build cached-function run");
    execution
        .set_usage_attribution_from_visibility(&AppVisibility::Offline)
        .await;
    execution
        .set_log_flush_policy(Duration::from_secs(60), 500)
        .await
        .expect("set test log policy");
    execution
}

#[flow_like_types::tokio::test]
async fn cache_miss_write_does_not_block_the_function_successor() {
    let (gated_store, write_started, write_release) = GatedStore::new();
    let function_body_executions = Arc::new(AtomicU64::new(0));
    let body_logic = Arc::new(CountingBodyNode::new(function_body_executions.clone()));
    let mut config = FlowLikeConfig::new();
    config.register_app_storage_store(FlowLikeStore::Other(gated_store.clone()));
    let state = Arc::new(FlowLikeState::new(
        config,
        HTTPClient::new_without_refetch(),
    ));
    let catalog: Vec<Arc<dyn NodeLogic>> = vec![
        Arc::new(CallFunctionNode::new()),
        Arc::new(InfoNode::new()),
        body_logic.clone(),
    ];
    state.node_registry.write().await.push_nodes(catalog);

    let payload = RunPayload {
        id: CALL_ID.to_string(),
        payload: None,
        runtime_variables: None,
        filter_secrets: Some(true),
    };
    let board = Arc::new(cached_function_board(&body_logic).await);
    let mut execution = build_execution(&state, board.clone(), &payload).await;
    let successor = execution
        .nodes
        .get(SUCCESSOR_ID)
        .expect("successor is in the execution graph")
        .clone();

    let execution_task = tokio::spawn({
        let state = state.clone();
        async move {
            execution.execute(state).await;
            execution.get_status().await
        }
    });

    let write_started_result = tokio::time::timeout(Duration::from_secs(5), write_started).await;
    let continuation_result = tokio::time::timeout(
        Duration::from_secs(2),
        wait_until_executed(&successor.exec_calls),
    )
    .await;

    write_release.add_permits(1);
    let status = tokio::time::timeout(Duration::from_secs(5), execution_task)
        .await
        .expect("execution finishes after releasing the cache write")
        .expect("execution task does not panic");
    write_started_result
        .expect("function cache miss reaches the cache write")
        .expect("cache write-start signal is sent");
    continuation_result.expect(
        "the function successor must execute while the cache write response is still gated",
    );
    assert!(
        matches!(status, RunStatus::Success),
        "the cached-function run must finish successfully"
    );
    assert_eq!(
        function_body_executions.load(Ordering::Acquire),
        1,
        "the cache miss must execute the function body once"
    );
    let writes_after_miss = gated_store.put_count();
    assert_eq!(writes_after_miss, 1, "the cache miss writes exactly once");

    let mut cached_execution = build_execution(&state, board, &payload).await;
    let cached_status = tokio::time::timeout(Duration::from_secs(5), async {
        cached_execution.execute(state).await;
        cached_execution.get_status().await
    })
    .await
    .expect("the cached run finishes");
    assert!(
        matches!(cached_status, RunStatus::Success),
        "the cached run must finish successfully"
    );
    assert_eq!(
        function_body_executions.load(Ordering::Acquire),
        1,
        "the cached run must bypass the function body"
    );
    assert_eq!(
        gated_store.put_count(),
        writes_after_miss,
        "the cached run must reuse the persisted result without another write"
    );
}
