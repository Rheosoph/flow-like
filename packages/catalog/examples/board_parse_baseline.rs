//! Baseline timing for the full board load + run-construction path on a large board.
//!
//! Stage the board file at `{root}/flow/{APP_ID}/{BOARD_ID}.board`, then:
//!   cargo run -p flow-like-catalog --example board_parse_baseline --release -- <root>

use flow_like::{
    flow::{
        board::Board,
        execution::{InternalRun, RunPayload, context::ExecutionContext},
        node::{Node, NodeLogic},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use flow_like_types::async_trait;
use flow_like_types::{intercom::BufferedInterComHandler, tokio};
use std::collections::HashMap;
use std::{path::PathBuf, sync::Arc, time::Instant};

const APP_ID: &str = "bench-app";
const BOARD_ID: &str = "mopo-monitor";

struct StubLogic {
    name: String,
}

#[async_trait]
impl NodeLogic for StubLogic {
    fn get_node(&self) -> Node {
        Node::new(&self.name, &self.name, "bench stub", "Bench")
    }

    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Ok(())
    }
}

/// Boards may reference node types outside the built-in catalog (installed
/// packages). Register no-op stubs so run construction can resolve them.
async fn register_stubs(state: &Arc<FlowLikeState>, board: &Board) {
    let mut missing: Vec<String> = vec![];
    {
        let registry = state.node_registry.read().await;
        let inner = registry.node_registry.clone();
        let mut check = |node: &Node| {
            if !inner.registry.contains_key(&node.name) && !missing.contains(&node.name) {
                missing.push(node.name.clone());
            }
        };
        for node in board.nodes.values() {
            check(node);
        }
        for layer in board.layers.values() {
            for node in layer.nodes.values() {
                check(node);
            }
        }
    }
    if missing.is_empty() {
        return;
    }
    println!(
        "registering {} stub node types: {:?}",
        missing.len(),
        missing
    );
    let stubs: Vec<Arc<dyn NodeLogic>> = missing
        .into_iter()
        .map(|name| Arc::new(StubLogic { name }) as Arc<dyn NodeLogic>)
        .collect();
    state.node_registry.write().await.push_nodes(stubs);
}

async fn default_state(root: PathBuf) -> Arc<FlowLikeState> {
    let mut config = FlowLikeConfig::new();
    let store = LocalObjectStore::new(root).unwrap();
    let store = FlowLikeStore::Local(Arc::new(store));
    config.register_bits_store(store.clone());
    config.register_user_store(store.clone());
    config.register_app_storage_store(store.clone());
    config.register_app_meta_store(store);
    let (http_client, _refetch_rx) = HTTPClient::new();
    let state = FlowLikeState::new(config, http_client);
    let state_ref = Arc::new(state);
    let weak_ref = Arc::downgrade(&state_ref);
    let catalog = flow_like_catalog::get_catalog();
    {
        let registry_guard = state_ref.node_registry.clone();
        let mut registry = registry_guard.write().await;
        registry.initialize(weak_ref);
        registry.push_nodes(catalog);
    }
    state_ref
}

#[tokio::main]
async fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("pass store root dir"));

    let t = Instant::now();
    let state = default_state(root).await;
    println!(
        "registry build (catalog get_node per type): {:?}",
        t.elapsed()
    );

    let path = Path::from("flow").child(APP_ID);

    let t = Instant::now();
    let board = Board::load(path.clone(), BOARD_ID, state.clone(), None)
        .await
        .unwrap();
    println!(
        "Board::load cold (GET+lz4+prost+from_proto+node_updates+cleanup+hash): {:?}",
        t.elapsed()
    );
    println!(
        "nodes: {}, layers: {}, variables: {}",
        board.nodes.len(),
        board.layers.len(),
        board.variables.len()
    );

    for i in 0..3 {
        let t = Instant::now();
        let _b = Board::load(path.clone(), BOARD_ID, state.clone(), None)
            .await
            .unwrap();
        println!("Board::load hot iter {}: {:?}", i, t.elapsed());
    }

    register_stubs(&state, &board).await;

    let start_id = board
        .nodes
        .values()
        .find(|n| n.start.unwrap_or(false))
        .map(|n| n.id.clone())
        .unwrap_or_else(|| board.nodes.keys().next().unwrap().clone());
    println!("start node: {}", start_id);

    let board = Arc::new(board);
    let profile = Profile::default();
    let buffered = BufferedInterComHandler::new(
        Arc::new(move |_event| Box::pin(async move { Ok(()) })),
        Some(100),
        Some(400),
        Some(true),
    );
    let callback = buffered.into_callback();
    let payload = RunPayload {
        id: start_id,
        payload: None,
        runtime_variables: None,
        filter_secrets: Some(true),
    };

    for i in 0..4 {
        let t = Instant::now();
        let clone_t = Instant::now();
        let board_clone = (*board).clone();
        let clone_elapsed = clone_t.elapsed();
        let run = InternalRun::new(
            "bench",
            Arc::new(board_clone),
            None,
            &state,
            &profile,
            &payload,
            false,
            callback.clone(),
            None,
            None,
            HashMap::new(),
        )
        .await
        .unwrap();
        println!(
            "iter {}: board deep clone {:?}, total incl. InternalRun::new: {:?}",
            i,
            clone_elapsed,
            t.elapsed()
        );
        drop(run);
    }
}
