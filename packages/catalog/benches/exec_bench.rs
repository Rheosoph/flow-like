//! Execution timing harness: total wall-clock and cost per node execution.
//!
//! Not a criterion benchmark. The fixtures this targets include a board driving a
//! multi-million-iteration `while` loop, where a single run takes seconds — criterion's
//! sampling model (dozens to hundreds of runs, plus warm-up) simply cannot complete, and
//! its statistics add nothing once a single measurement is seconds long and dominated by
//! real work rather than timer noise.
//!
//! Reports total wall-clock per run. A per-node figure would be more comparable across
//! boards, but `RunMeta::increment_nodes_executed` has no callers anywhere in the tree, so
//! `get_nodes_executed()` is always 0 — see the note in todo/compile-graphs.md.
//!
//! Deliberately uses only long-standing APIs so the identical file can be dropped into an
//! older checkout for a before/after comparison.
//!
//! Run with: `cargo bench -p flow-like-catalog --bench exec_bench`
//! Env: `FL_EXEC_REPS` (default 3) — repetitions per board.

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
use flow_like_types::{intercom::BufferedInterComHandler, tokio};
use std::collections::HashMap;
use std::{path::PathBuf, sync::Arc, time::{Duration, Instant}};

const APP_ID: &str = "q99s8hb4z56mpwz8dscz7qmz";

async fn default_state() -> Arc<FlowLikeState> {
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

struct Entry {
    /// Store-relative directory, since fixtures live both directly in `tests/flow/` and
    /// inside per-app subdirectories.
    dir: Path,
    board_id: String,
    start_id: String,
    nodes: usize,
    pins: usize,
}

async fn discover_entries(state: &Arc<FlowLikeState>) -> Vec<Entry> {
    let mut candidates: Vec<(Path, PathBuf)> = Vec::new();

    for entry in std::fs::read_dir("../../tests/flow").unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            let app = path.file_name().and_then(|n| n.to_str()).unwrap_or(APP_ID);
            for nested in std::fs::read_dir(&path).unwrap().flatten() {
                candidates.push((Path::from("flow").child(app.to_string()), nested.path()));
            }
        } else {
            candidates.push((Path::from("flow"), path));
        }
    }

    let mut found = Vec::new();
    for (dir, path) in candidates {
        if path.extension().and_then(|e| e.to_str()) != Some("board") {
            continue;
        }
        let Some(board_id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(board) = Board::load(dir.clone(), board_id, state.clone(), None).await else {
            continue;
        };
        let pins: usize = board.nodes.values().map(|node| node.pins.len()).sum();
        for node in board.nodes.values() {
            if node.start.unwrap_or(false) {
                found.push(Entry {
                    dir: dir.clone(),
                    board_id: board_id.to_string(),
                    start_id: node.id.clone(),
                    nodes: board.nodes.len(),
                    pins,
                });
            }
        }
    }
    found.sort_by(|a, b| a.board_id.cmp(&b.board_id).then(a.start_id.cmp(&b.start_id)));
    found
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let state = rt.block_on(default_state());
    let profile = Profile::default();
    let reps: usize = std::env::var("FL_EXEC_REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    let entries = rt.block_on(discover_entries(&state));
    if entries.is_empty() {
        println!("exec_bench: no boards with a start node found");
        return;
    }

    // One handler for the whole harness, constructed inside the runtime because it spawns
    // an idle-flush task. Building one per run measures task accrual, not the engine.
    let callback = rt.block_on(async {
        Arc::new(BufferedInterComHandler::new(
            Arc::new(move |_event| Box::pin(async move { Ok(()) })),
            Some(100),
            Some(400),
            Some(true),
        ))
        .into_callback()
    });

    for entry in entries {
        let board = Arc::new(rt.block_on(async {
            Board::load(entry.dir.clone(), &entry.board_id, state.clone(), None)
                .await
                .unwrap()
        }));

        println!(
            "\n=== {} ({} nodes, {} pins) ===",
            entry.board_id, entry.nodes, entry.pins
        );

        let mut samples: Vec<f64> = Vec::with_capacity(reps);
        for rep in 0..reps {
            let board = board.clone();
            let state = state.clone();
            let profile = profile.clone();
            let callback = callback.clone();
            let start_id = entry.start_id.clone();

            let elapsed = rt.block_on(async move {
                let payload = RunPayload {
                    id: start_id,
                    payload: None,
                    runtime_variables: None,
                    filter_secrets: Some(true),
                };
                let mut run = InternalRun::new(
                    "bench",
                    board,
                    None,
                    &state,
                    &profile,
                    &payload,
                    false,
                    callback,
                    None,
                    None,
                    HashMap::new(),
                )
                .await
                .unwrap();

                // `execute()` ends by setting a cancel flag and awaiting the background
                // flush task, but that task only re-checks the flag after its next tick.
                // At the 5s default every measurement is therefore rounded up to a
                // multiple of 5s — 85.001s here is 17 ticks, not a real duration. Shrink
                // the interval so the trailing wait is negligible.
                run.set_log_flush_policy(Duration::from_millis(5), 500)
                    .await
                    .unwrap();

                // Timed region is execution only; construction is measured separately by
                // plan_bench's graph_build benchmarks.
                let started = Instant::now();
                run.execute(state).await;
                let elapsed = started.elapsed().as_secs_f64();

                // Functional check: dump end-state variables so runs can be compared
                // across revisions. Identical timing with a different end state would
                // be a fast wrong answer.
                if std::env::var("FL_EXEC_DUMP_VARS").is_ok() {
                    let variables = run.variables.lock().await;
                    let mut sorted: Vec<_> = variables.values().collect();
                    sorted.sort_by(|a, b| a.name.cmp(&b.name));
                    for variable in sorted {
                        let value = variable.value.lock().await;
                        println!("  var {} = {}", variable.name, value);
                    }
                }
                elapsed
            });

            println!("  run {}: {:>12.3} ms", rep + 1, elapsed * 1000.0);
            samples.push(elapsed);
        }

        let best = samples.iter().cloned().fold(f64::MAX, f64::min);
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        println!(
            "  BEST {:.3} ms   MEAN {:.3} ms",
            best * 1000.0,
            mean * 1000.0
        );
    }
}
