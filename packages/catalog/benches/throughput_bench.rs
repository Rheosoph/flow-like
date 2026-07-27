//! Peak throughput benchmark - find maximum workflow executions per second.
//!
//! This benchmark sweeps through different concurrency levels to find
//! the optimal point for maximum throughput on your specific hardware.
//!
//! Environment variables:
//!   FL_BOARD_ID          - Board ID to benchmark (default: o4wqrpzkx1cp4svxe91yordw)
//!   FL_START_ID          - Start node ID (default: ek4tee4s3nufw3drfnwd20hw)
//!   FL_APP_ID            - App ID (default: q99s8hb4z56mpwz8dscz7qmz)
//!   FL_TESTS_DIR         - Tests directory path (default: ../../tests)
//!   FL_WORKER_THREADS    - Tokio worker threads (default: CPU count)
//!   FL_CONCURRENCY_LIST  - Comma-separated concurrency levels (e.g., "1,2,4,8,16")
//!   FL_MAX_CONCURRENCY   - Max concurrency for auto-sweep (default: CPU * 8)
//!   FL_MEASURE_SECS      - Measurement duration per level (default: 10)
//!
//! Run with the allocator used by shipping Flow-Like binaries:
//!   cargo bench -p flow-like-catalog --bench throughput_bench --features mimalloc

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use flow_like::{
    flow::{
        board::Board,
        execution::{InternalRun, RunPayload},
    },
    num_cpus,
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
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_or_string(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn concurrency_list() -> Vec<usize> {
    if let Ok(raw) = std::env::var("FL_CONCURRENCY_LIST") {
        return raw
            .split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .filter(|&n| n > 0)
            .collect();
    }
    let max = env_or("FL_MAX_CONCURRENCY", num_cpus::get() * 8);
    let mut v = vec![1];
    let mut n = 2;
    while n <= max {
        v.push(n);
        n *= 2;
    }
    // Add some intermediate points for better resolution
    let cpus = num_cpus::get();
    for factor in [1, 2, 3, 4, 6] {
        let val = cpus * factor;
        if val <= max && !v.contains(&val) {
            v.push(val);
        }
    }
    v.sort_unstable();
    v.dedup();
    v
}

fn board_id() -> String {
    env_or_string("FL_BOARD_ID", "o4wqrpzkx1cp4svxe91yordw")
}
fn start_id() -> String {
    env_or_string("FL_START_ID", "ek4tee4s3nufw3drfnwd20hw")
}
fn app_id() -> String {
    env_or_string("FL_APP_ID", "q99s8hb4z56mpwz8dscz7qmz")
}
fn tests_dir() -> PathBuf {
    PathBuf::from(env_or_string("FL_TESTS_DIR", "../../tests"))
}

async fn default_state() -> Arc<FlowLikeState> {
    let mut config: FlowLikeConfig = FlowLikeConfig::new();
    let store = LocalObjectStore::new(tests_dir()).expect("LocalObjectStore");
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

fn construct_profile() -> Profile {
    Profile::default()
}

async fn open_board(id: &str, state: Arc<FlowLikeState>) -> Board {
    let path = Path::from("flow").child(&*app_id());
    let mut board = Board::load(path, id, state, None)
        .await
        .expect("load board");
    // Suppress ALL internal logging during benchmarks for maximum performance
    board.log_level = flow_like::flow::execution::LogLevel::Fatal;
    board
}

fn create_intercom() -> Arc<BufferedInterComHandler> {
    BufferedInterComHandler::new(
        Arc::new(move |_event| Box::pin(async move { Ok(()) })),
        Some(100),
        Some(400),
        Some(true),
    )
}

async fn run_once(
    board: Arc<Board>,
    state: Arc<FlowLikeState>,
    profile: &Profile,
    start: &str,
    callback: InterComCallback,
) {
    let payload = RunPayload {
        id: start.to_string(),
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
    .expect("InternalRun::new");
    run.execute(state).await;
}

fn throughput_bench(c: &mut Criterion) {
    let worker_threads = env_or("FL_WORKER_THREADS", num_cpus::get());
    let max_blocking = env_or("FL_MAX_BLOCKING_THREADS", worker_threads * 4);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .max_blocking_threads(max_blocking)
        .enable_all()
        .build()
        .expect("tokio runtime");

    let state = rt.block_on(default_state());
    let board = Arc::new(rt.block_on(open_board(&board_id(), state.clone())));
    let profile = construct_profile();
    let start = start_id();
    // The handler starts an idle-flush task, so construct it inside the runtime and retain one
    // instance for this complete Criterion function instead of spawning a task per execution.
    let intercom = rt.block_on(async { create_intercom() });
    let callback = intercom.into_callback();

    // Warmup phase - important for JIT and cache warming
    eprintln!("[throughput] Warming up...");
    rt.block_on(async {
        for _ in 0..200 {
            run_once(
                board.clone(),
                state.clone(),
                &profile,
                &start,
                callback.clone(),
            )
            .await;
        }
    });

    let mut group = c.benchmark_group("peak_throughput");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(env_or("FL_MEASURE_SECS", 10)));
    group.sample_size(20);

    let concs = concurrency_list();
    eprintln!(
        "[throughput] Testing {} concurrency levels: {:?}",
        concs.len(),
        concs
    );

    for &concurrency in &concs {
        group.throughput(Throughput::Elements(concurrency as u64));
        group.bench_with_input(
            BenchmarkId::new("executions", concurrency),
            &concurrency,
            |b, &conc| {
                b.to_async(&rt).iter_custom(|iters| {
                    let state = state.clone();
                    let board = board.clone();
                    let profile = profile.clone();
                    let start = start.clone();
                    let callback = callback.clone();

                    async move {
                        let begin = Instant::now();
                        for _ in 0..iters {
                            let mut tasks = Vec::with_capacity(conc);
                            for _ in 0..conc {
                                let st = state.clone();
                                let brd = board.clone();
                                let pr = profile.clone();
                                let sid = start.clone();
                                let callback = callback.clone();
                                tasks.push(tokio::spawn(async move {
                                    run_once(brd, st, &pr, &sid, callback).await;
                                }));
                            }
                            for t in tasks {
                                let _ = t.await;
                            }
                        }
                        begin.elapsed()
                    }
                });
            },
        );
    }

    group.finish();
}

/// Raw throughput test - measures absolute maximum executions in a fixed time window.
fn raw_throughput_bench(c: &mut Criterion) {
    let worker_threads = env_or("FL_WORKER_THREADS", num_cpus::get());
    let max_blocking = env_or("FL_MAX_BLOCKING_THREADS", worker_threads * 4);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .max_blocking_threads(max_blocking)
        .enable_all()
        .build()
        .expect("tokio runtime");

    let state = rt.block_on(default_state());
    let board = Arc::new(rt.block_on(open_board(&board_id(), state.clone())));
    let profile = construct_profile();
    let start = start_id();
    let intercom = rt.block_on(async { create_intercom() });
    let callback = intercom.into_callback();

    // Warmup
    rt.block_on(async {
        for _ in 0..100 {
            run_once(
                board.clone(),
                state.clone(),
                &profile,
                &start,
                callback.clone(),
            )
            .await;
        }
    });

    let mut group = c.benchmark_group("raw_throughput");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(15));
    group.sample_size(10);

    let max_in_flight = env_or("FL_MAX_IN_FLIGHT", num_cpus::get() * 4);
    group.throughput(Throughput::Elements(1));

    group.bench_function("max_throughput", |b| {
        b.to_async(&rt).iter_custom(|iters| {
            let state = state.clone();
            let board = board.clone();
            let profile = profile.clone();
            let start = start.clone();
            let callback = callback.clone();

            async move {
                let counter = Arc::new(AtomicU64::new(0));
                let target = iters * (max_in_flight as u64);
                let begin = Instant::now();

                let mut handles = Vec::with_capacity(max_in_flight);
                while counter.load(Ordering::Relaxed) < target {
                    while handles.len() < max_in_flight && counter.load(Ordering::Relaxed) < target
                    {
                        let st = state.clone();
                        let brd = board.clone();
                        let pr = profile.clone();
                        let sid = start.clone();
                        let cnt = counter.clone();
                        let callback = callback.clone();
                        handles.push(tokio::spawn(async move {
                            run_once(brd, st, &pr, &sid, callback).await;
                            cnt.fetch_add(1, Ordering::Relaxed);
                        }));
                    }
                    if let Some(h) = handles.pop() {
                        let _ = h.await;
                    }
                }
                for h in handles {
                    let _ = h.await;
                }
                begin.elapsed()
            }
        });
    });

    group.finish();

    eprintln!(
        "\n[raw_throughput] Workers: {}, Max in-flight: {}",
        worker_threads, max_in_flight
    );
}

criterion_group!(benches, throughput_bench, raw_throughput_bench);
criterion_main!(benches);
