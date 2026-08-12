//! Basic flow execution benchmark - measures single and batch execution performance.
//!
//! This benchmark focuses on measuring the raw execution time of workflows,
//! useful for profiling and identifying bottlenecks in the execution engine.
//!
//! Run with the allocator used by shipping Flow-Like binaries:
//!   cargo bench -p flow-like-catalog --bench flow_bench --features mimalloc

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use criterion::{Criterion, criterion_group, criterion_main};
use flow_like::{
    flow::{
        board::Board,
        execution::{InternalRun, RunPayload},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use flow_like_types::{
    intercom::{BufferedInterComHandler, InterComCallback},
    tokio,
};
use std::collections::HashMap;
use std::{path::PathBuf, sync::Arc, time::Duration};

const BOARD_ID: &str = "o4wqrpzkx1cp4svxe91yordw";
const START_ID: &str = "ek4tee4s3nufw3drfnwd20hw";
const APP_ID: &str = "q99s8hb4z56mpwz8dscz7qmz";

async fn default_state() -> Arc<FlowLikeState> {
    let mut config = FlowLikeConfig::new();
    let store = LocalObjectStore::new(PathBuf::from("../../tests")).unwrap();
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

async fn open_board(id: &str, state: Arc<FlowLikeState>) -> Board {
    let path = Path::from("flow").child(APP_ID);
    Board::load(path, id, state, None).await.unwrap()
}

async fn run_once(
    board: Arc<Board>,
    state: Arc<FlowLikeState>,
    profile: &Profile,
    start_id: &str,
    callback: InterComCallback,
) {
    let payload = RunPayload {
        id: start_id.to_string(),
        payload: None,
        runtime_variables: None,
        filter_secrets: Some(true),
    };
    let mut run = InternalRun::new(
        "bench",
        board,
        None,
        &state,
        profile,
        &payload,
        false,
        callback,
        None,
        None,
        HashMap::new(),
    )
    .await
    .unwrap();
    run.execute(state).await;
}

fn criterion_benchmark(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let state = rt.block_on(default_state());
    let board = Arc::new(rt.block_on(open_board(BOARD_ID, state.clone())));
    let profile = Profile::default();
    // Construct the handler while the runtime is entered (it spawns an idle-flush task), then
    // retain it for the full benchmark so samples do not accumulate background tasks.
    let buffered_sender = rt.block_on(async {
        BufferedInterComHandler::new(
            Arc::new(move |_event| Box::pin(async move { Ok(()) })),
            Some(100),
            Some(400),
            Some(true),
        )
    });
    let callback = buffered_sender.into_callback();

    // Warmup
    rt.block_on(async {
        for _ in 0..50 {
            run_once(
                board.clone(),
                state.clone(),
                &profile,
                START_ID,
                callback.clone(),
            )
            .await;
        }
    });

    let mut group = c.benchmark_group("flow_execution");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(100);

    group.bench_function("single_execution", |b| {
        b.to_async(&rt).iter(|| {
            let board = board.clone();
            let state = state.clone();
            let profile = profile.clone();
            let callback = callback.clone();
            async move {
                run_once(board, state, &profile, START_ID, callback).await;
            }
        });
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
