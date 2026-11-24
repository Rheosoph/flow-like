//! What a start from the source board costs: `Board::load` with its full `node_updates` sweep
//! against the whole node catalog, then the compile. Companion to `flow-like-runtime`'s
//! `compiled_cold_start_timing`, which times the start from the artifact; same variables.
//!
//! Ignored by default because it reads a board from a local object-store root. Run with:
//!   FLOW_LIKE_BENCH_ROOT="$HOME/Library/Application Support/flow-like/projects" \
//!   FLOW_LIKE_BENCH_APP=<app id> FLOW_LIKE_BENCH_BOARD=<board id> \
//!   cargo test -p flow-like-catalog --profile release-fast \
//!     --test source_cold_start_timing -- --ignored --nocapture

use flow_like::{
    flow::{
        board::Board,
        compiled::{CompiledRunTemplate, compile_board_with_catalog, encode_artifact},
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
    },
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use flow_like_types::{async_trait, tokio};
use std::{collections::HashSet, path::PathBuf, sync::Arc, time::Instant};

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

/// Node types of packages this process has not installed still have to resolve for the compile.
async fn register_missing_types(state: &Arc<FlowLikeState>, board: &Board) -> usize {
    let known = state.node_registry.read().await.node_registry.clone();
    let missing: HashSet<&str> = board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        .map(|node| node.name.as_str())
        .filter(|name| !known.registry.contains_key(*name))
        .collect();
    let stubs: Vec<Arc<dyn NodeLogic>> = missing
        .iter()
        .map(|name| {
            Arc::new(StubLogic {
                name: name.to_string(),
            }) as Arc<dyn NodeLogic>
        })
        .collect();
    let count = stubs.len();
    if count > 0 {
        state.node_registry.write().await.push_nodes(stubs);
    }
    count
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn cold_start_from_the_source_board() {
    let (Ok(root), Ok(app_id), Ok(board_id)) = (
        std::env::var("FLOW_LIKE_BENCH_ROOT"),
        std::env::var("FLOW_LIKE_BENCH_APP"),
        std::env::var("FLOW_LIKE_BENCH_BOARD"),
    ) else {
        eprintln!("set FLOW_LIKE_BENCH_ROOT, FLOW_LIKE_BENCH_APP and FLOW_LIKE_BENCH_BOARD");
        return;
    };

    let store = FlowLikeStore::Local(Arc::new(
        LocalObjectStore::new(PathBuf::from(root)).unwrap(),
    ));
    let mut config = FlowLikeConfig::new();
    config.register_bits_store(store.clone());
    config.register_user_store(store.clone());
    config.register_app_storage_store(store.clone());
    config.register_app_meta_store(store);
    let (http_client, _refetch) = HTTPClient::new();
    let state = Arc::new(FlowLikeState::new(config, http_client));

    let started = Instant::now();
    let catalog = flow_like_catalog::get_catalog();
    let types = catalog.len();
    {
        let mut registry = state.node_registry.write().await;
        registry.initialize(Arc::downgrade(&state));
        registry.push_nodes(catalog);
    }
    println!(
        "registry build ({types} node types, get_node each): {:?}",
        started.elapsed()
    );
    let started = Instant::now();
    let fingerprint = state.node_registry.read().await.node_registry.fingerprint();
    println!("registry fingerprint, first call: {:?}", started.elapsed());

    let board_dir = Path::from("apps").join(app_id);
    let mut board = None;
    for round in 0..4 {
        let started = Instant::now();
        let loaded = Board::load(board_dir.clone(), &board_id, state.clone(), None)
            .await
            .unwrap();
        println!(
            "Board::load #{round} (read + decode + full node_updates sweep + cleanup + hash): {:?}",
            started.elapsed()
        );
        board = Some(loaded);
    }
    let board = Arc::new(board.expect("the board was loaded"));
    let pins: usize = board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        .map(|node| node.pins.len())
        .sum();
    println!(
        "nodes {} (+{} in layers) · pins {} · layers {} · pages {}",
        board.nodes.len(),
        board
            .layers
            .values()
            .map(|layer| layer.nodes.len())
            .sum::<usize>(),
        pins,
        board.layers.len(),
        board.page_ids.len()
    );

    let stubbed = register_missing_types(&state, &board).await;
    if stubbed > 0 {
        println!("{stubbed} node types are not in this catalog and run as stubs");
    }
    let registry = state.node_registry.read().await.node_registry.clone();
    for round in 0..3 {
        let started = Instant::now();
        let compiled = compile_board_with_catalog(&board, &registry).unwrap();
        let compile = started.elapsed();
        let started = Instant::now();
        let bytes = encode_artifact(&compiled, &fingerprint).unwrap();
        let encode = started.elapsed();
        let started = Instant::now();
        CompiledRunTemplate::from_board(board.clone(), &registry).unwrap();
        println!(
            "#{round} compile {compile:?} · encode {encode:?} ({} B) · from_board (compile + template) {:?}",
            bytes.len(),
            started.elapsed()
        );
    }
}
