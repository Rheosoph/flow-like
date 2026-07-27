//! Execution-level differential: the compiled path must *run* like the board path.
//!
//! The structural gate in `compile_plan_with_catalog.rs` proves the compiled runtime graph
//! has the same nodes, pins and edges as the one built from a `Board`. That is necessary
//! but not sufficient: a graph can be structurally identical and still behave differently
//! — a pin cell shared where the old builder made a fresh one, a dropped `depends_on` edge
//! silently falling back to a default, a pure node cached across loop iterations instead of
//! re-evaluated.
//!
//! So these tests actually execute the same board both ways and compare observable end
//! state: every variable's final value, the run status, and the set of nodes that ran.
//! This is the gate that has to stay green before the old builder can be deleted.

use flow_like::{
    flow::{
        board::{Board, compile::compile_board},
        execution::{InternalRun, RunPayload, compiled::CompiledGraph},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use flow_like_types::{intercom::BufferedInterComHandler, plan::PlanBuffer, tokio};
use std::collections::{BTreeMap, HashMap};
use std::{path::PathBuf, sync::Arc};

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

/// Observable outcome of a run, for comparison across construction paths.
#[derive(Debug, PartialEq)]
struct Outcome {
    variables: BTreeMap<String, String>,
    status: String,
    visited: Vec<String>,
}

fn noop_callback() -> flow_like_types::intercom::InterComCallback {
    Arc::new(BufferedInterComHandler::new(
        Arc::new(move |_event| Box::pin(async move { Ok(()) })),
        Some(100),
        Some(400),
        Some(true),
    ))
    .into_callback()
}

async fn outcome_of(mut run: InternalRun, state: Arc<FlowLikeState>) -> Outcome {
    run.execute(state).await;

    let variables = {
        let guard = run.variables.lock().await;
        let mut out = BTreeMap::new();
        for variable in guard.values() {
            let value = variable.value.lock().await;
            // Keyed by id (names may repeat) and carrying the name in the value, so a
            // projected board that loses variable names fails rather than passing quietly.
            out.insert(
                variable.id.clone(),
                format!("{}={}", variable.name, value),
            );
        }
        out
    };

    let guard = run.run.lock().await;
    let mut visited: Vec<String> = guard.visited_nodes.keys().cloned().collect();
    visited.sort();

    Outcome {
        variables,
        status: format!("{:?}", guard.status),
        visited,
    }
}

/// Load a board, then run it once through each constructor and compare.
async fn assert_paths_agree(board_dir: Path, board_id: &str, start_id: &str) {
    let state = state_with_catalog().await;
    let board_dir_for_plan = board_dir.clone();
    let board = Arc::new(
        Board::load(board_dir, board_id, state.clone(), None)
            .await
            .unwrap(),
    );
    let profile = Profile::default();
    let payload = RunPayload {
        id: start_id.to_string(),
        payload: None,
        runtime_variables: None,
        filter_secrets: Some(true),
    };

    let from_board = InternalRun::new(
        "differential",
        board.clone(),
        None,
        &state,
        &profile,
        &payload,
        false,
        noop_callback(),
        None,
        None,
        HashMap::new(),
    )
    .await
    .unwrap();
    let board_outcome = outcome_of(from_board, state.clone()).await;

    let stamps = board.compile_stamps(&state).await;
    let container = compile_board(&board, stamps).unwrap().to_container().unwrap();
    let registry = state.node_registry.read().await.node_registry.clone();
    let graph =
        CompiledGraph::hydrate(PlanBuffer::new(container).unwrap(), &registry).unwrap();

    // Note: no `Board` is passed. The plan-built run projects its own board, so this
    // exercises the path the executor will use — plan object only, no `.board` load.
    let from_plan = InternalRun::from_compiled(
        &graph,
        "differential",
        &board_dir_for_plan,
        None,
        &state,
        &profile,
        &payload,
        false,
        noop_callback(),
        None,
        None,
        HashMap::new(),
        None,
    )
    .await
    .unwrap();
    let plan_outcome = outcome_of(from_plan, state.clone()).await;

    assert_eq!(
        board_outcome.variables, plan_outcome.variables,
        "{board_id}: final variable values differ between board-built and plan-built runs"
    );
    assert_eq!(
        board_outcome.visited, plan_outcome.visited,
        "{board_id}: the set of executed nodes differs"
    );
    assert_eq!(
        board_outcome.status, plan_outcome.status,
        "{board_id}: run status differs"
    );
}

/// The loop fixture: a `while` loop whose body reads a variable, adds to it through a pure
/// chain, and writes it back. Exercises exec-pin re-triggering, per-iteration pure
/// re-evaluation, and variable read/write — the semantics most likely to diverge.
#[tokio::test]
async fn loop_board_runs_identically_from_plan() {
    assert_paths_agree(
        Path::from("flow"),
        "xe70lo8chu02qwxatxbjvehm",
        "jnyfhvjsralsswemjszzjxyh",
    )
    .await;
}

/// A payload naming a node that does not exist must produce the same (empty) run on both
/// paths rather than one silently finding an entry the other does not.
#[tokio::test]
async fn unknown_entry_behaves_identically() {
    assert_paths_agree(
        Path::from("flow"),
        "xe70lo8chu02qwxatxbjvehm",
        "definitely-not-a-node-id",
    )
    .await;
}
