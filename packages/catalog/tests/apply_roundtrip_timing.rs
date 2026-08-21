//! Apply round trip on a large real board: wall clock, and that a scoped sweep agrees with a full
//! one against the whole node catalog.
//!
//! Ignored by default because both need a board fixture under `tmp/`. Run with:
//!   cargo test -p flow-like-catalog --test apply_roundtrip_timing -- --ignored --nocapture

use flow_like::{
    flow::board::{
        Board,
        commands::{GenericCommand, nodes::move_node::MoveNodeCommand},
        dirty::Touched,
    },
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use flow_like_types::tokio;
use std::{path::PathBuf, sync::Arc, time::Instant};

const BOARD_ID: &str = "ie6j0ph9szad636m0kz9xeft-mopo-monitor-main-v1";
const BOARD_DIR: &str = "tmp";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

async fn state_with_catalog() -> Arc<FlowLikeState> {
    let mut config = FlowLikeConfig::new();
    let store = LocalObjectStore::new(repo_root()).unwrap();
    let store = FlowLikeStore::Local(Arc::new(store));
    config.register_bits_store(store.clone());
    config.register_user_store(store.clone());
    config.register_app_storage_store(store.clone());
    config.register_app_meta_store(store);
    let (http_client, _refetch_rx) = HTTPClient::new();
    let state = Arc::new(FlowLikeState::new(config, http_client));
    let weak = Arc::downgrade(&state);
    {
        let registry_guard = state.node_registry.clone();
        let mut registry = registry_guard.write().await;
        registry.initialize(weak);
        registry.push_nodes(flow_like_catalog::get_catalog());
    }
    state
}

/// A scoped sweep has to leave the board a full sweep would.
///
/// `execute_commands` narrows the sweep to what the batch reached; `redo` runs the identical
/// commands and then re-derives every node. Applying the same batch both ways isolates exactly the
/// sweep, with no persistence or convergence differences mixed in. The fixture is copied into an
/// in-memory store first so nothing is ever written back to it.
#[tokio::test]
#[ignore]
async fn scoped_sweep_matches_full_sweep() {
    use flow_like_storage::object_store::{ObjectStore, memory::InMemory};

    let fixture = repo_root()
        .join(BOARD_DIR)
        .join(format!("{BOARD_ID}.board"));
    let Ok(bytes) = std::fs::read(&fixture) else {
        eprintln!("fixture missing: {}", fixture.display());
        return;
    };

    let memory = Arc::new(InMemory::new());
    memory
        .put(
            &Path::from(format!("{BOARD_DIR}/{BOARD_ID}.board")),
            bytes.into(),
        )
        .await
        .expect("seed in-memory fixture");

    let mut config = FlowLikeConfig::new();
    let store = FlowLikeStore::Other(memory);
    config.register_app_meta_store(store.clone());
    config.register_app_storage_store(store.clone());
    config.register_user_store(store.clone());
    config.register_bits_store(store);
    let (http_client, _refetch_rx) = HTTPClient::new();
    let state = Arc::new(FlowLikeState::new(config, http_client));
    let weak = Arc::downgrade(&state);
    {
        let registry_guard = state.node_registry.clone();
        let mut registry = registry_guard.write().await;
        registry.initialize(weak);
        registry.push_nodes(flow_like_catalog::get_catalog());
    }

    let mut board = Board::load(Path::from(BOARD_DIR), BOARD_ID, state.clone(), None)
        .await
        .unwrap();

    // A move seeds one node; re-submitting a node unchanged seeds it and its wired neighbours.
    // Between them they exercise both the direct and the propagated half of the scope.
    let movable: Vec<String> = board
        .nodes
        .values()
        .filter(|node| node.coordinates.is_some())
        .take(8)
        .map(|node| node.id.clone())
        .collect();
    let mut commands: Vec<GenericCommand> = movable
        .iter()
        .map(|node_id| {
            GenericCommand::MoveNode(MoveNodeCommand::new(
                node_id.clone(),
                (17.0, 23.0, 0.0),
                None,
            ))
        })
        .collect();
    let wired: Vec<String> = board
        .nodes
        .values()
        .filter(|node| {
            node.pins
                .values()
                .any(|pin| !pin.connected_to.is_empty() || !pin.depends_on.is_empty())
        })
        .take(8)
        .map(|node| node.id.clone())
        .collect();
    for node_id in &wired {
        let node = board.nodes.get(node_id).cloned().expect("wired node");
        commands.push(GenericCommand::UpdateNode(
            flow_like::flow::board::commands::nodes::update_node::UpdateNodeCommand {
                old_node: Some(node.clone()),
                node,
            },
        ));
    }

    let mut touched = Touched::default();
    for command in &commands {
        command.touched(&mut touched);
    }
    println!(
        "batch of {} commands seeds {} nodes / {} layers / {} variables",
        commands.len(),
        touched.nodes.len(),
        touched.layers.len(),
        touched.variables.len(),
    );

    let mut full = board.clone();
    full.redo(commands.clone(), state.clone())
        .await
        .expect("apply batch with a full sweep");
    board
        .execute_commands(commands, state.clone())
        .await
        .expect("apply batch with a scoped sweep");

    // Baseline: a second full sweep that applies no commands at all. Anything it still changes is
    // a node whose `on_update` is not idempotent, which no scoping decision can account for.
    let mut full_twice = full.clone();
    full_twice
        .redo(Vec::new(), state.clone())
        .await
        .expect("second full sweep");
    let churn: Vec<&str> = full
        .nodes
        .iter()
        .filter(|(node_id, node)| {
            full_twice
                .nodes
                .get(*node_id)
                .is_some_and(|second| second.hash != node.hash)
        })
        .map(|(_, node)| node.name.as_str())
        .collect();
    println!(
        "nodes a second, command-free full sweep still changes: {} ({:?})",
        churn.len(),
        {
            let mut kinds: Vec<&str> = churn.clone();
            kinds.sort_unstable();
            kinds.dedup();
            kinds
        }
    );

    let mut diverged = Vec::new();
    for (node_id, expected) in &full.nodes {
        let actual = board.nodes.get(node_id).expect("node present after sweep");
        if expected.hash != actual.hash || expected.error != actual.error {
            diverged.push(format!("{node_id} ({})", expected.name));
        }
    }
    assert_eq!(full.nodes.len(), board.nodes.len());
    assert!(
        diverged.is_empty(),
        "{} node(s) settled differently under the scoped sweep: {}",
        diverged.len(),
        diverged.join(", ")
    );
    println!(
        "scoped sweep agrees with the full sweep across {} nodes",
        board.nodes.len()
    );
}

#[tokio::test]
#[ignore]
async fn apply_roundtrip_timing() {
    let fixture = repo_root()
        .join(BOARD_DIR)
        .join(format!("{BOARD_ID}.board"));
    if !fixture.exists() {
        eprintln!("fixture missing: {}", fixture.display());
        return;
    }

    let state = state_with_catalog().await;

    let started = Instant::now();
    let mut board = Board::load(Path::from(BOARD_DIR), BOARD_ID, state.clone(), None)
        .await
        .unwrap();
    let load = started.elapsed();

    let pins: usize = board.nodes.values().map(|node| node.pins.len()).sum();
    println!(
        "board: {} nodes, {} pins, {} layers, {} refs",
        board.nodes.len(),
        pins,
        board.layers.len(),
        board.refs.len()
    );
    println!("load (includes one full node_updates sweep): {load:?}");

    let node_id = board
        .nodes
        .values()
        .find(|node| node.coordinates.is_some())
        .map(|node| node.id.clone())
        .expect("board should have a placed node");

    for round in 0..5 {
        let command = GenericCommand::MoveNode(MoveNodeCommand::new(
            node_id.clone(),
            (round as f32 * 10.0, 0.0, 0.0),
            None,
        ));
        let started = Instant::now();
        board
            .execute_commands(vec![command], state.clone())
            .await
            .unwrap();
        println!("apply MoveNode #{round}: {:?}", started.elapsed());
    }
}
