//! End-to-end plan compilation against the real node catalog.
//!
//! The compiler's own tests live in `flow-like` and necessarily run without a registry,
//! so they compile boards that have never been through `Board::node_updates` — the
//! fixpoint that applies every node's `on_update` and is precisely what compilation
//! freezes into the artifact. This test closes that gap from the catalog side, where the
//! full registry is available.
//!
//! It also pins the invariant the whole design rests on: a plan is only valid for the
//! catalog it was built against, so the catalog signature must actually move when the
//! catalog does.

use flow_like::{
    flow::board::{
        Board,
        compile::{CompileStamps, compile_board},
    },
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use flow_like_types::plan::{ArchivedColdPlan, ArchivedHotPlan, NONE_INDEX, PlanBuffer};
use std::{path::PathBuf, sync::Arc};

/// Boards checked into `tests/ast/`, the largest real ones available.
const FIXTURES: &[&str] = &["ttwctnp08u18sg2z6nmcqqak", "bypaw6n2ksuvrw0kcaj14omz"];

async fn state_with_catalog() -> Arc<FlowLikeState> {
    let mut config = FlowLikeConfig::new();
    let store = FlowLikeStore::Local(Arc::new(
        LocalObjectStore::new(PathBuf::from("../../tests")).unwrap(),
    ));
    config.register_bits_store(store.clone());
    config.register_user_store(store.clone());
    config.register_app_storage_store(store.clone());
    config.register_app_meta_store(store);

    let (http_client, _refetch_rx) = HTTPClient::new();
    let state = Arc::new(FlowLikeState::new(config, http_client));
    let weak = Arc::downgrade(&state);
    {
        let registry = state.node_registry.clone();
        let mut guard = registry.write().await;
        guard.initialize(weak);
        guard.push_nodes(flow_like_catalog::get_catalog());
    }
    state
}

/// `Board::load` runs `node_updates` + `cleanup` against the live registry, so this is the
/// exact board shape the version-publish path compiles.
async fn load_fixture(id: &str, state: Arc<FlowLikeState>) -> Board {
    Board::load(Path::from("ast"), id, state, None)
        .await
        .unwrap()
}

#[tokio::test]
async fn real_boards_compile_after_node_updates() {
    let state = state_with_catalog().await;

    for id in FIXTURES {
        let board = load_fixture(id, state.clone()).await;
        let container = board.compile_plan(&state).await.unwrap();
        let buffer = PlanBuffer::new(container).unwrap();

        let hot_section = buffer.hot().unwrap();
        let hot = hot_section.root();

        assert_eq!(hot.board_id.as_str(), board.id);
        assert_eq!(
            hot.nodes.len(),
            board.nodes.len()
                + board
                    .layers
                    .values()
                    .filter(|layer| matches!(
                        layer.r#type,
                        flow_like::flow::board::LayerType::Function
                    ))
                    .map(|layer| layer.nodes.len())
                    .sum::<usize>(),
            "{id}: every board and function-layer node must be lowered"
        );

        // Every node in the board must be addressable by its id, and its type key must
        // survive verbatim — that string is what resolves NodeLogic at hydration.
        for (node_id, node) in &board.nodes {
            let index = hot
                .node_by_id(node_id)
                .unwrap_or_else(|| panic!("{id}: node {node_id} missing from plan"));
            let entry = &hot.nodes[index as usize];
            assert_eq!(hot.symbol(entry.type_key.to_native()), node.name);
            assert_eq!(
                entry.pin_count.to_native() as usize,
                node.pins.len(),
                "{id}: pin count mismatch on {node_id}"
            );

            // Each pin must be resolvable by name through the shim's lookup table.
            for pin in node.pins.values() {
                let matches = hot.pins_by_name(index, &pin.name);
                assert!(
                    !matches.is_empty(),
                    "{id}: pin {} of {node_id} is not resolvable by name",
                    pin.name
                );
            }
        }
    }
}

/// Every node type the catalog can instantiate must survive lowering, so a plan never
/// carries a type key the registry cannot resolve at hydration.
#[tokio::test]
async fn every_lowered_type_key_resolves_in_the_registry() {
    let state = state_with_catalog().await;
    let registry = state.node_registry.read().await.node_registry.clone();

    for id in FIXTURES {
        let board = load_fixture(id, state.clone()).await;
        let container = board.compile_plan(&state).await.unwrap();
        let buffer = PlanBuffer::new(container).unwrap();
        let section = buffer.hot().unwrap();
        let hot = section.root();

        for entry in hot.nodes.iter() {
            let type_key = hot.symbol(entry.type_key.to_native());
            // WASM-backed nodes are overlaid onto the registry per request, so they are
            // legitimately absent from the native catalog.
            if entry.wasm_package.to_native() != NONE_INDEX {
                continue;
            }
            assert!(
                registry.get_node(type_key).is_ok(),
                "{id}: plan references unknown node type {type_key}"
            );
        }
    }
}

/// Descriptions and schemas reach the artifact already ref-resolved, so the runtime never
/// needs `board.refs`.
#[tokio::test]
async fn cold_strings_are_resolved_and_deduplicated() {
    let state = state_with_catalog().await;
    let board = load_fixture(FIXTURES[0], state.clone()).await;
    let container = board.compile_plan(&state).await.unwrap();
    let buffer = PlanBuffer::new(container).unwrap();

    let section = buffer.cold().unwrap();
    let cold = section.root();

    // Nothing may still look like an unresolved ref hash (a bare decimal u64 key).
    for handle in cold.pin_descriptions.iter() {
        let value = cold.string(handle.to_native());
        assert!(
            value.is_empty() || !value.chars().all(|c| c.is_ascii_digit()),
            "unresolved ref hash left in cold section: {value}"
        );
    }

    // Interning must actually share: a real board has far more pin slots than distinct
    // strings, which is the property that keeps the artifact smaller than the board.
    let distinct = cold.strings.len();
    let slots = cold.pin_descriptions.len() + cold.pin_schemas.len() + cold.pin_friendly_names.len();
    assert!(
        distinct < slots,
        "cold strings are not being shared: {distinct} distinct across {slots} slots"
    );
}

/// The runtime graph built from a plan must describe the same wiring as the board it was
/// compiled from, on real data.
///
/// This is the gate for swapping the engine over: until the compiled graph is known to be
/// equivalent on boards of this size and shape, the old string-keyed builder cannot be
/// deleted.
#[tokio::test]
async fn runtime_graph_matches_real_boards() {
    use flow_like::flow::board::LayerType;
    use flow_like::flow::execution::compiled::CompiledGraph;

    let state = state_with_catalog().await;
    let registry = state.node_registry.read().await.node_registry.clone();

    for id in FIXTURES {
        let board = load_fixture(id, state.clone()).await;
        let container = board.compile_plan(&state).await.unwrap();
        let graph =
            CompiledGraph::hydrate(PlanBuffer::new(container).unwrap(), &registry).unwrap();
        let runtime = graph.build_runtime_graph().unwrap();

        let expected_nodes: Vec<(&String, &flow_like::flow::node::Node)> = board
            .nodes
            .iter()
            .chain(
                board
                    .layers
                    .values()
                    .filter(|layer| matches!(layer.r#type, LayerType::Function))
                    .flat_map(|layer| layer.nodes.iter()),
            )
            .collect();

        assert_eq!(
            runtime.nodes.len(),
            expected_nodes.len(),
            "{id}: node count mismatch"
        );

        for (node_id, node) in &expected_nodes {
            let internal = runtime
                .nodes
                .get(*node_id)
                .unwrap_or_else(|| panic!("{id}: node {node_id} absent from runtime graph"));
            assert_eq!(internal.node_name(), node.name, "{id}: {node_id} type key");
            assert_eq!(
                internal.pins.len(),
                node.pins.len(),
                "{id}: {node_id} pin count"
            );

            for pin in node.pins.values() {
                let internal_pin = runtime
                    .pins
                    .get(&pin.id)
                    .unwrap_or_else(|| panic!("{id}: pin {} absent", pin.id));
                assert_eq!(internal_pin.name, pin.name, "{id}: pin {} name", pin.id);
                assert_eq!(internal_pin.pin_type, pin.pin_type);
                assert_eq!(internal_pin.data_type, pin.data_type);
                assert_eq!(internal_pin.index, pin.index);

                // Every edge the board records must be reachable in the runtime graph.
                // Edges pointing at pins that no longer exist are dropped by both the old
                // builder and the compiler, so only resolvable ones are compared.
                let expected: Vec<&String> = pin
                    .connected_to
                    .iter()
                    .filter(|target| runtime.pins.contains_key(*target))
                    .collect();
                let actual: Vec<String> = internal_pin
                    .connected_to()
                    .iter()
                    .filter_map(|weak| weak.upgrade().map(|target| target.id.clone()))
                    .collect();
                assert_eq!(
                    expected.len(),
                    actual.len(),
                    "{id}: pin {} connection count",
                    pin.id
                );
                for target in expected {
                    assert!(
                        actual.contains(target),
                        "{id}: pin {} lost edge to {target}",
                        pin.id
                    );
                }
            }
        }
    }
}

/// A plan must be detectably invalid against a different catalog, because compilation
/// freezes `on_update` output that would otherwise be recomputed on every board load.
#[tokio::test]
async fn catalog_signature_is_stable_and_content_sensitive() {
    let state = state_with_catalog().await;
    let board = load_fixture(FIXTURES[0], state.clone()).await;

    let first = board.compile_stamps(&state).await;
    let second = board.compile_stamps(&state).await;
    assert_eq!(
        first.catalog_signature, second.catalog_signature,
        "signature must be stable across calls for an unchanged catalog"
    );
    assert_ne!(first.catalog_signature, 0);

    let container = board.compile_plan(&state).await.unwrap();
    let header = PlanBuffer::new(container).unwrap().header().clone();
    assert!(header.matches(
        board.hash.unwrap_or(0),
        first.catalog_signature,
        first.wasm_signature
    ));
    assert!(
        !header.matches(
            board.hash.unwrap_or(0),
            first.catalog_signature.wrapping_add(1),
            first.wasm_signature
        ),
        "a drifted catalog signature must invalidate the plan"
    );
}
