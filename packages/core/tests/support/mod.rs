//! Shared harness for run→client channel tests: a minimal execution context wired to an
//! in-process channel, with the InterCom callback playing the frontend.

use ahash::AHashMap;
use flow_like::{
    flow::{
        board::ExecutionStage,
        execution::{
            LogLevel, Run, context::ExecutionContext, internal_node::InternalNode,
            internal_pin::InternalPin,
        },
        node::{Node, NodeLogic},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_types::{
    async_trait,
    channel::InProcessChannel,
    sync::{Mutex, RwLock},
};
use std::sync::{Arc, Weak};
use std::time::Duration;

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

pub fn internal_node(node: Node) -> Arc<InternalNode> {
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

    let internal = Arc::new(InternalNode::new(
        node,
        pins,
        Arc::new(NoopLogic),
        name_cache,
    ));

    for pin in internal.pins.iter() {
        pin.init_node(Arc::downgrade(&internal));
        pin.init_connected_to(Vec::new());
        pin.init_depends_on(Vec::new());
    }

    internal
}

pub async fn context_with_callback(
    channel_id: &str,
    callback: flow_like_types::intercom::InterComCallback,
) -> (ExecutionContext, Arc<InProcessChannel>) {
    let current = internal_node(Node::new(
        "test_query",
        "Test Query",
        "Query channel test node",
        "Tests",
    ));
    let mut node_map = AHashMap::new();
    node_map.insert(current.node_id().to_string(), current.clone());

    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let variables = Arc::new(Mutex::new(AHashMap::new()));
    let cache = Arc::new(RwLock::new(AHashMap::new()));
    let run: Weak<Mutex<Run>> = Weak::new();
    let channel = InProcessChannel::register(channel_id, Duration::from_secs(60)).await;

    let context = ExecutionContext::new(
        Arc::new(node_map),
        &run,
        &state,
        &current,
        &variables,
        &cache,
        LogLevel::Debug,
        ExecutionStage::Dev,
        Arc::new(Profile::default()),
        callback,
        Arc::new(RwLock::new(Vec::new())),
        None,
        None,
        Arc::new(AHashMap::new()),
        Some(channel.clone()),
    )
    .await;
    (context, channel)
}
