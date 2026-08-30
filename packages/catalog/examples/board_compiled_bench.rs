//! Granular timing for the compiled-board pipeline on a large board.
//!
//! Stage the board file at `{root}/flow/{APP_ID}/{BOARD_ID}.board`, then:
//!   cargo run -p flow-like-catalog --example board_compiled_bench --release -- <root>

use flow_like::flow::board::Board;
use flow_like::flow::compiled::{
    CompiledRunTemplate, compile_board, compile_board_with_catalog, decode_artifact,
    encode_artifact,
};
use flow_like::flow::execution::{InternalRun, RunPayload, context::ExecutionContext};
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::profile::Profile;
use flow_like::state::{FlowLikeConfig, FlowLikeState};
use flow_like::utils::http::HTTPClient;
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use flow_like_types::{async_trait, intercom::BufferedInterComHandler, tokio};
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
    let state = default_state(root).await;
    let path = Path::from("flow").child(APP_ID);

    let t = Instant::now();
    let board = Board::load(path.clone(), BOARD_ID, state.clone(), None)
        .await
        .unwrap();
    println!(
        "Board::load (GET+lz4+prost+node_updates+cleanup+hash): {:?}",
        t.elapsed()
    );
    let board = Arc::new(board);

    register_stubs(&state, &board).await;
    let registry = state.node_registry.read().await.node_registry.clone();
    let fingerprint = registry.fingerprint();

    let t = Instant::now();
    let plain = compile_board(&board).expect("compile");
    let plain_elapsed = t.elapsed();
    let plain_raw = rkyv::to_bytes::<rkyv::rancor::Error>(&plain).expect("rkyv");
    let plain_bytes = encode_artifact(&plain, &fingerprint).expect("encode");
    println!(
        "compile_board (no interning): {:?} -> {} bytes compressed ({} uncompressed rkyv)",
        plain_elapsed,
        plain_bytes.len(),
        plain_raw.len()
    );
    drop(plain);
    drop(plain_raw);
    drop(plain_bytes);

    let t = Instant::now();
    let compiled = compile_board_with_catalog(&board, registry.as_ref()).expect("compile");
    println!(
        "compile_board_with_catalog: {:?} ({} nodes, {} pins, {} layers)",
        t.elapsed(),
        compiled.nodes.len(),
        compiled.pins.len(),
        compiled.layers.len()
    );

    let t = Instant::now();
    let bytes = encode_artifact(&compiled, &fingerprint).expect("encode");
    let interned_raw = rkyv::to_bytes::<rkyv::rancor::Error>(&compiled).expect("rkyv");
    println!(
        "encode_artifact (interned): {:?} ({} bytes compressed, {} uncompressed rkyv)",
        t.elapsed(),
        bytes.len(),
        interned_raw.len()
    );
    drop(interned_raw);

    let mut ids = 0usize;
    let mut names = 0usize;
    let mut friendly = 0usize;
    let mut friendly_count = 0usize;
    let mut desc = 0usize;
    let mut desc_count = 0usize;
    let mut schema = 0usize;
    let mut defaults = 0usize;
    let mut options_vals = 0usize;
    let mut edges = 0usize;
    for p in &compiled.pins {
        ids += p.id.len();
        names += p.name.len();
        if let Some(f) = &p.friendly_name {
            friendly += f.len();
            friendly_count += 1;
        }
        if let Some(d) = &p.description {
            desc += d.len();
            desc_count += 1;
        }
        schema += p.schema.as_ref().map(String::len).unwrap_or(0);
        defaults += p.default_value.as_ref().map(Vec::len).unwrap_or(0);
        options_vals += p
            .options
            .as_ref()
            .and_then(|o| o.valid_values.as_ref())
            .map(|v| v.iter().map(String::len).sum::<usize>())
            .unwrap_or(0);
        edges += (p.depends_on.len() + p.connected_to.len()) * 4;
    }
    let mut node_ids = 0usize;
    let mut node_names = 0usize;
    let mut node_friendly = 0usize;
    let mut nf_count = 0usize;
    let mut node_desc = 0usize;
    let mut nd_count = 0usize;
    for n in &compiled.nodes {
        node_ids += n.id.len();
        node_names += n.name.len();
        if let Some(f) = &n.friendly_name {
            node_friendly += f.len();
            nf_count += 1;
        }
        if let Some(d) = &n.description {
            node_desc += d.len();
            nd_count += 1;
        }
    }
    let refs_bytes: usize = compiled.refs.iter().map(|(k, v)| k.len() + v.len()).sum();
    println!("--- interned artifact content breakdown (string bytes, uncompressed) ---");
    println!(
        "pins:  ids {} | names {} | friendly {} (in {}/{}) | desc {} (in {}/{}) | schema {} | defaults {} | options {} | edges {}",
        ids,
        names,
        friendly,
        friendly_count,
        compiled.pins.len(),
        desc,
        desc_count,
        compiled.pins.len(),
        schema,
        defaults,
        options_vals,
        edges
    );
    println!(
        "nodes: ids {} | type-names {} | friendly {} (in {}/{}) | desc {} (in {}/{})",
        node_ids,
        node_names,
        node_friendly,
        nf_count,
        compiled.nodes.len(),
        node_desc,
        nd_count,
        compiled.nodes.len()
    );
    println!(
        "refs table: {} bytes in {} entries",
        refs_bytes,
        compiled.refs.len()
    );

    for i in 0..3 {
        let t = Instant::now();
        let decoded = decode_artifact(&bytes, Some(&fingerprint)).expect("decode");
        let decode_elapsed = t.elapsed();
        let t = Instant::now();
        let template =
            CompiledRunTemplate::from_compiled(&decoded, registry.as_ref(), path.clone())
                .expect("template");
        println!(
            "iter {}: decode_artifact {:?} + template build (artifact path) {:?}",
            i,
            decode_elapsed,
            t.elapsed()
        );
        drop(template);
    }

    let t = Instant::now();
    let template = Arc::new(
        CompiledRunTemplate::from_compiled(&compiled, registry.as_ref(), path.clone())
            .expect("template"),
    );
    println!("template build (reconstructed view): {:?}", t.elapsed());

    let start_id = board
        .nodes
        .values()
        .find(|n| n.start.unwrap_or(false))
        .map(|n| n.id.clone())
        .unwrap_or_else(|| board.nodes.keys().next().unwrap().clone());

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

    for i in 0..5 {
        let t = Instant::now();
        let run = InternalRun::from_template(
            "bench",
            template.clone(),
            None,
            &state,
            &profile,
            &payload,
            false,
            callback.clone(),
            None,
            None,
            HashMap::new(),
            None,
            None,
        )
        .await
        .unwrap();
        println!(
            "iter {}: InternalRun::from_template (warm per-run cost): {:?}",
            i,
            t.elapsed()
        );
        drop(run);
    }
}
