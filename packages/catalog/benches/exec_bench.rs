//! Execution-engine benchmark over a source-controlled, non-trivial board.
//!
//! Run with the allocator used by shipping Flow-Like binaries:
//!   cargo bench -p flow-like-catalog --bench exec_bench --features mimalloc

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use criterion::{Criterion, criterion_group, criterion_main};
use flow_like::{
    flow::{
        ast::apply_flowscript_to_board,
        board::Board,
        execution::{InternalRun, LogLevel, RunPayload},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::Path;
use flow_like_types::{
    intercom::{BufferedInterComHandler, InterComCallback},
    tokio,
};

const EXECUTION_FIXTURE: &str = include_str!("../tests/fixtures/execution_chain.flowscript");

async fn benchmark_board() -> (Arc<FlowLikeState>, Arc<Board>, String) {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let catalog = flow_like_catalog::get_catalog();
    let catalog_nodes = catalog
        .iter()
        .map(|logic| logic.get_node())
        .collect::<Vec<_>>();
    state.node_registry.write().await.push_nodes(catalog);

    let mut board = Board::new_detached(Some("execution-benchmark".to_string()), Path::default());
    board.name = "Execution Benchmark".to_string();
    board.log_level = LogLevel::Fatal;

    let applied = apply_flowscript_to_board(
        &mut board,
        EXECUTION_FIXTURE,
        &catalog_nodes,
        state.clone(),
        None,
        false,
    )
    .await
    .expect("execution benchmark fixture applies");
    assert!(
        applied.diagnostics.is_empty(),
        "execution benchmark fixture diagnostics: {:#?}",
        applied.diagnostics
    );
    assert!(
        board.nodes.len() >= 34,
        "execution benchmark fixture must remain execution-heavy"
    );

    let mut entry_nodes = board.nodes.values().filter(|node| node.start == Some(true));
    let entry_id = entry_nodes
        .next()
        .expect("execution benchmark fixture has an entry node")
        .id
        .clone();
    assert!(
        entry_nodes.next().is_none(),
        "execution benchmark fixture has exactly one entry node"
    );

    (state, Arc::new(board), entry_id)
}

async fn build_run(
    board: Arc<Board>,
    state: &Arc<FlowLikeState>,
    profile: &Profile,
    entry_id: &str,
    callback: InterComCallback,
) -> InternalRun {
    let payload = RunPayload {
        id: entry_id.to_string(),
        payload: None,
        runtime_variables: None,
        filter_secrets: Some(true),
    };
    InternalRun::new(
        "bench",
        board,
        None,
        state,
        profile,
        &payload,
        false,
        callback,
        None,
        None,
        HashMap::new(),
    )
    .await
    .expect("build benchmark run")
}

fn criterion_benchmark(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (state, board, entry_id) = runtime.block_on(benchmark_board());
    // BufferedInterComHandler may spawn an idle-flush task, so it must be constructed while the
    // runtime is entered. Keep one handler alive for the entire benchmark instead of spawning a
    // background task per sample.
    let intercom = runtime.block_on(async {
        BufferedInterComHandler::new(
            Arc::new(|_events| Box::pin(async { Ok(()) })),
            Some(100),
            Some(400),
            Some(true),
        )
    });
    let callback = intercom.into_callback();
    let profile = Profile::default();

    runtime.block_on(async {
        for _ in 0..10 {
            let mut run =
                build_run(board.clone(), &state, &profile, &entry_id, callback.clone()).await;
            run.execute(state.clone()).await;
        }
    });

    let mut group = c.benchmark_group("execution_engine");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(20);
    group.bench_function("34_node_chain", |bencher| {
        bencher.to_async(&runtime).iter_custom(|iterations| {
            let state = state.clone();
            let board = board.clone();
            let profile = profile.clone();
            let entry_id = entry_id.clone();
            let callback = callback.clone();

            async move {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let mut run =
                        build_run(board.clone(), &state, &profile, &entry_id, callback.clone())
                            .await;
                    let started = Instant::now();
                    run.execute(state.clone()).await;
                    measured += started.elapsed();
                }
                measured
            }
        });
    });
    group.finish();

    drop(intercom);
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
