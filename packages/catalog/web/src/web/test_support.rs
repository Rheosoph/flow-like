use ahash::AHashMap;
use flow_like::{
    flow::{
        board::ExecutionStage,
        execution::{
            LogLevel, Run, context::ExecutionContext, internal_node::InternalNode,
            internal_pin::InternalPin,
        },
        node::{Node, NodeLogic},
        variable::VariableType,
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_types::{
    Value, async_trait,
    sync::{Mutex, RwLock},
};
use std::{
    net::{TcpListener, UdpSocket},
    sync::{Arc, Weak},
};

#[derive(Default)]
struct NoopLogic;

#[async_trait]
impl NodeLogic for NoopLogic {
    fn get_node(&self) -> Node {
        Node::new("test_noop", "Test Noop", "No-op test node", "Tests")
    }

    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Ok(())
    }
}

pub(crate) fn internal_node(node: Node) -> Arc<InternalNode> {
    internal_node_with_logic(node, Arc::new(NoopLogic))
}

pub(crate) fn internal_node_with_logic(node: Node, logic: Arc<dyn NodeLogic>) -> Arc<InternalNode> {
    let mut pins = AHashMap::new();
    let mut name_cache: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();

    for pin in node.pins.values() {
        let internal_pin = Arc::new(InternalPin::new(pin, false));
        name_cache
            .entry(pin.name.clone())
            .or_default()
            .push(internal_pin.clone());
        pins.insert(pin.id.clone(), internal_pin);
    }

    let internal = Arc::new(InternalNode::new(node, pins, logic, name_cache));

    for pin in internal.pins.values() {
        pin.init_node(Arc::downgrade(&internal));
        pin.init_connected_to(Vec::new());
        pin.init_depends_on(Vec::new());
    }

    internal
}

pub(crate) fn node_with_outputs(outputs: &[(&str, VariableType)]) -> Arc<InternalNode> {
    let mut node = Node::new("test_handler", "Test Handler", "Handler test node", "Tests");
    node.add_output_pin("exec_out", "Exec", "Execute", VariableType::Execution);
    for (name, data_type) in outputs {
        node.add_output_pin(name, name, name, data_type.clone());
    }
    internal_node(node)
}

pub(crate) async fn test_context(
    current: Arc<InternalNode>,
    nodes: Vec<Arc<InternalNode>>,
) -> ExecutionContext {
    let mut node_map = AHashMap::new();
    for node in nodes {
        node_map.insert(node.node_id().to_string(), node);
    }
    node_map
        .entry(current.node_id().to_string())
        .or_insert_with(|| current.clone());

    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let variables = Arc::new(Mutex::new(AHashMap::new()));
    let cache = Arc::new(RwLock::new(AHashMap::new()));
    let run: Weak<Mutex<Run>> = Weak::new();

    ExecutionContext::new(
        Arc::new(node_map),
        &run,
        &state,
        &current,
        &variables,
        &cache,
        LogLevel::Debug,
        ExecutionStage::Dev,
        Arc::new(Profile::default()),
        None,
        Arc::new(RwLock::new(Vec::new())),
        None,
        None,
        Arc::new(AHashMap::new()),
    )
    .await
}

pub(crate) async fn output_value(context: &ExecutionContext, name: &str) -> Option<Value> {
    context
        .node
        .get_pin_by_name(name)
        .await
        .unwrap()
        .get_raw_value()
        .await
}

pub(crate) fn free_tcp_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub(crate) fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
