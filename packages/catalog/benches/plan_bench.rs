//! Compiled-plan load benchmark.
//!
//! Measures the cost of getting from "bytes in memory" to "ready to look things up",
//! which is what a cold execution pays before its first node can run.
//!
//! The two paths compared are:
//!   * today — lz4 + protobuf decode, then `node_updates` (a fixpoint of `on_update` over
//!     every node) and `cleanup`, then per-run graph construction;
//!   * compiled — bytecheck-validate the plan section and address it directly.
//!
//! Run with: `cargo bench -p flow-like-catalog --bench plan_bench`

use criterion::{Criterion, criterion_group, criterion_main};
use flow_like::{
    flow::{
        board::Board,
        execution::{InternalRun, RunPayload, compiled::CompiledGraph},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_types::intercom::BufferedInterComHandler;
use std::collections::HashMap;
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use flow_like_types::{
    plan::{ArchivedHotPlan, PlanBuffer},
    tokio,
};
use std::hint::black_box;
use std::{path::PathBuf, sync::Arc, time::Duration};

/// The largest real boards in the repo; small boards hide fixed costs.
const FIXTURES: &[(&str, &str)] = &[
    ("large_267kb", "ttwctnp08u18sg2z6nmcqqak"),
    ("medium_167kb", "bypaw6n2ksuvrw0kcaj14omz"),
];

async fn default_state() -> Arc<FlowLikeState> {
    let mut config = FlowLikeConfig::new();
    let store = LocalObjectStore::new(PathBuf::from("../../tests")).unwrap();
    let store = FlowLikeStore::Local(Arc::new(store));
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

async fn store_of(
    state: &Arc<FlowLikeState>,
) -> Arc<dyn flow_like_storage::object_store::ObjectStore> {
    state
        .config
        .read()
        .await
        .stores
        .app_meta_store
        .clone()
        .unwrap()
        .as_generic()
}

fn criterion_benchmark(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let state = rt.block_on(default_state());
    let board_dir = Path::from("ast");

    let mut group = c.benchmark_group("plan_load");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));

    for (label, board_id) in FIXTURES {
        let store = rt.block_on(store_of(&state));

        // Prepare once, outside the measured region.
        let board = rt.block_on(async {
            Board::load(board_dir.clone(), board_id, state.clone(), None)
                .await
                .unwrap()
        });
        let plan_bytes = rt.block_on(async { board.compile_plan(&state).await.unwrap() });
        let board_bytes = std::fs::read(format!("../../tests/ast/{board_id}.board")).unwrap();

        eprintln!(
            "{label}: board {} bytes (lz4 protobuf) -> plan {} bytes (lz4 rkyv sections), \
             {} nodes",
            board_bytes.len(),
            plan_bytes.len(),
            board.nodes.len()
        );

        // Today: decode the stored protobuf and run the load-time fixups.
        group.bench_function(format!("{label}/board_load_current"), |b| {
            b.to_async(&rt).iter(|| {
                let state = state.clone();
                let board_dir = board_dir.clone();
                let store = store.clone();
                async move {
                    let proto = Board::load_proto(store, &board_dir, board_id, None)
                        .await
                        .unwrap();
                    black_box(Board::from_loaded_proto(proto, board_dir, state).await)
                }
            });
        });

        // Compiled: validate the section and address it. No decode, no fixups.
        group.bench_function(format!("{label}/plan_load_compiled"), |b| {
            b.iter(|| {
                let buffer = PlanBuffer::new(black_box(plan_bytes.clone())).unwrap();
                let section = buffer.hot().unwrap();
                let archived = section.root();
                black_box(archived.nodes.len())
            });
        });

        // Compilation itself is paid once per version, not per run.
        group.bench_function(format!("{label}/compile_once"), |b| {
            b.to_async(&rt).iter(|| {
                let state = state.clone();
                let board = &board;
                async move { black_box(board.compile_plan(&state).await.unwrap().len()) }
            });
        });

        // ── Per-run graph construction ───────────────────────────────────────────
        // This is the cost paid on every run, even when the board is already cached:
        // the old builder rebuilds the whole pin/node graph from string-keyed maps,
        // while the compiled path walks CSR rows and reuses pre-parsed defaults.
        let registry = rt.block_on(async { state.node_registry.read().await.node_registry.clone() });
        let compiled = CompiledGraph::hydrate(
            PlanBuffer::new(plan_bytes.clone()).unwrap(),
            &registry,
        )
        .unwrap();
        let shared_board = Arc::new(
            rt.block_on(async {
                Board::load(board_dir.clone(), board_id, state.clone(), None)
                    .await
                    .unwrap()
            }),
        );
        let profile = Profile::default();

        group.bench_function(format!("{label}/graph_build_current"), |b| {
            b.to_async(&rt).iter(|| {
                let state = state.clone();
                let board = shared_board.clone();
                let profile = profile.clone();
                async move {
                    let sender = Arc::new(BufferedInterComHandler::new(
                        Arc::new(move |_event| Box::pin(async move { Ok(()) })),
                        Some(100),
                        Some(400),
                        Some(true),
                    ));
                    let payload = RunPayload {
                        id: String::new(),
                        payload: None,
                        runtime_variables: None,
                        filter_secrets: Some(true),
                    };
                    black_box(
                        InternalRun::new(
                            "bench",
                            board,
                            None,
                            &state,
                            &profile,
                            &payload,
                            false,
                            sender.into_callback(),
                            None,
                            None,
                            HashMap::new(),
                        )
                        .await
                        .unwrap(),
                    )
                }
            });
        });

        group.bench_function(format!("{label}/graph_build_compiled"), |b| {
            b.iter(|| black_box(compiled.build_runtime_graph().unwrap().nodes.len()));
        });

        // Representative lookups on the hot path: resolving a node id and a pin name.
        let buffer = PlanBuffer::new(plan_bytes.clone()).unwrap();
        let section = buffer.hot().unwrap();
        let archived = section.root();
        let sample_id = archived
            .symbol(archived.nodes[0].instance_id.to_native())
            .to_string();
        let sample_pin = archived
            .symbol(archived.pins[0].name.to_native())
            .to_string();

        group.bench_function(format!("{label}/lookup_node_by_id"), |b| {
            b.iter(|| black_box(archived.node_by_id(black_box(&sample_id))));
        });
        group.bench_function(format!("{label}/lookup_pin_by_name"), |b| {
            b.iter(|| black_box(archived.pins_by_name(0, black_box(&sample_pin))));
        });
    }

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
