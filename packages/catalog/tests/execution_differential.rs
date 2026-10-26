//! End-to-end execution coverage for catalog boards.
//!
//! Most catalog tests exercise individual node logic. These tests deliberately go through
//! `InternalRun::new` and `InternalRun::execute` so entry-node dispatch, exec-pin ordering, and
//! pure-node dependency pulls are covered together.

use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use flow_like::{
    flow::{
        ast::{RenderOptions, apply_flowscript_to_board, board_to_flowscript},
        board::Board,
        execution::{InternalRun, LogLevel, RunPayload, RunStatus},
        node::Node,
        pin::PinType,
        variable::VariableType,
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::Path;
use flow_like_types::intercom::BufferedInterComHandler;

const EXECUTION_FIXTURE: &str = include_str!("fixtures/execution_chain.flowscript");
const EXPECTED_LOGS: usize = 32;

async fn catalog_state() -> (Arc<FlowLikeState>, Vec<Node>) {
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
    (state, catalog_nodes)
}

async fn executable_board() -> (Arc<FlowLikeState>, Board, String) {
    let (state, catalog_nodes) = catalog_state().await;

    let mut board =
        Board::new_detached(Some("execution-differential".to_string()), Path::default());
    board.name = "Execution Differential".to_string();
    board.log_level = LogLevel::Info;

    let applied = apply_flowscript_to_board(
        &mut board,
        EXECUTION_FIXTURE,
        &catalog_nodes,
        state.clone(),
        None,
        false,
    )
    .await
    .expect("execution fixture applies");
    assert!(
        applied.diagnostics.is_empty(),
        "execution fixture diagnostics: {:#?}",
        applied.diagnostics
    );
    assert!(
        board.nodes.len() >= EXPECTED_LOGS + 2,
        "fixture must stay large enough to expose per-node execution regressions"
    );

    let mut entry_nodes = board.nodes.values().filter(|node| node.start == Some(true));
    let entry_id = entry_nodes
        .next()
        .expect("execution fixture has an entry node")
        .id
        .clone();
    assert!(
        entry_nodes.next().is_none(),
        "execution fixture has exactly one entry node"
    );

    (state, board, entry_id)
}

async fn internal_run(state: &Arc<FlowLikeState>, board: Board, entry_id: String) -> InternalRun {
    let intercom = BufferedInterComHandler::new(
        Arc::new(|_events| Box::pin(async { Ok(()) })),
        Some(100),
        Some(400),
        Some(false),
    );
    let payload = RunPayload {
        id: entry_id,
        payload: None,
        runtime_variables: None,
        filter_secrets: Some(true),
    };
    InternalRun::new(
        "execution-test",
        Arc::new(board),
        None,
        state,
        &Profile::default(),
        &payload,
        false,
        intercom.into_callback(),
        None,
        None,
        HashMap::new(),
    )
    .await
    .expect("build internal run")
}

#[flow_like_types::tokio::test]
async fn executes_the_complete_catalog_board() {
    let (state, board, entry_id) = executable_board().await;
    let mut execution = internal_run(&state, board, entry_id).await;
    // A long interval makes the old polling implementation hang for almost a minute after the
    // final node. Cancellation must interrupt that wait, so the public execute API stays bounded.
    execution
        .set_log_flush_policy(Duration::from_secs(60), 500)
        .await
        .expect("set test flush policy");

    flow_like_types::tokio::time::timeout(Duration::from_secs(5), execution.execute(state.clone()))
        .await
        .expect("initial execute must interrupt the background flush wait");

    assert!(
        matches!(execution.get_status().await, RunStatus::Success),
        "the execution-heavy fixture must finish successfully"
    );
    let messages = execution
        .get_traces()
        .await
        .into_iter()
        .flat_map(|trace| trace.logs)
        .map(|log| log.message)
        .filter(|message| message.starts_with("execution fixture"))
        .collect::<Vec<_>>();
    assert_eq!(
        messages.len(),
        EXPECTED_LOGS,
        "every exec-linked logging node must run exactly once"
    );
    assert_eq!(
        messages.first().map(String::as_str),
        Some("execution fixture"),
        "the first logging node must pull and evaluate its pure string dependency"
    );
    assert_eq!(
        messages.last().map(String::as_str),
        Some("execution fixture 32"),
        "execution must reach the tail of the chain"
    );

    let first_nodes_executed = execution.meta.get_nodes_executed();
    assert!(first_nodes_executed > 0);
    for node in execution.nodes.values() {
        node.exec_calls.store(128_000, Ordering::Relaxed);
    }

    execution.fork().await.expect("fork completed execution");
    assert_eq!(execution.meta.get_nodes_executed(), 0);
    flow_like_types::tokio::time::timeout(Duration::from_secs(5), execution.execute(state))
        .await
        .expect("forked execute must interrupt the background flush wait");

    assert!(
        matches!(execution.get_status().await, RunStatus::Success),
        "the forked execution must restart from the payload entry node"
    );
    let forked_messages = execution
        .get_traces()
        .await
        .into_iter()
        .flat_map(|trace| trace.logs)
        .map(|log| log.message)
        .filter(|message| message.starts_with("execution fixture"))
        .collect::<Vec<_>>();
    assert_eq!(
        forked_messages, messages,
        "fork must reset node counters and execute the complete board again"
    );
    assert_eq!(
        execution.meta.get_nodes_executed(),
        first_nodes_executed,
        "node execution metadata must restart from zero on fork"
    );
}

const IMPURE_FUNCTION: &str = r#"function foo(): (out: string, ok: bool) {
    let v: string = ""
    v = "bar"
    return v, 1 < 2
}

eventsSimple() {
    const value = foo()
    if (value.ok) {
        logInfo({ message: value.out })
    }
}
"#;

async fn apply(state: &Arc<FlowLikeState>, catalog: &[Node], board: &mut Board, source: &str) {
    let applied = apply_flowscript_to_board(board, source, catalog, state.clone(), None, true)
        .await
        .expect("FlowScript applies");
    assert!(
        applied.diagnostics.is_empty(),
        "{:#?}\n{source}",
        applied.diagnostics
    );
}

async fn function_board() -> (Arc<FlowLikeState>, Vec<Node>, Board) {
    let (state, catalog) = catalog_state().await;
    let mut board = Board::new_detached(Some("function-calls".to_string()), Path::default());
    board.log_level = LogLevel::Info;
    apply(&state, &catalog, &mut board, IMPURE_FUNCTION).await;
    (state, catalog, board)
}

fn function_exec_in(board: &mut Board) -> &mut flow_like::flow::pin::Pin {
    board
        .layers
        .values_mut()
        .find(|layer| layer.name == "foo")
        .and_then(|layer| {
            layer.pins.values_mut().find(|pin| {
                pin.pin_type == PinType::Input && pin.data_type == VariableType::Execution
            })
        })
        .expect("`foo` has an execution boundary")
}

async fn run_logs(state: &Arc<FlowLikeState>, board: Board) -> (RunStatus, Vec<String>) {
    let entry_id = board
        .nodes
        .values()
        .find(|node| node.start == Some(true))
        .expect("the board has an entry node")
        .id
        .clone();
    let mut execution = internal_run(state, board, entry_id).await;
    flow_like_types::tokio::time::timeout(
        Duration::from_secs(10),
        execution.execute(state.clone()),
    )
    .await
    .expect("the run finishes");
    let logs = execution
        .get_traces()
        .await
        .into_iter()
        .flat_map(|trace| trace.logs)
        .map(|log| log.message)
        .collect();
    (execution.get_status().await, logs)
}

/// Editing an impure function down to a bare `return` keeps its layer's execution boundary with
/// nothing wired to it, and every call failed with "Function 'foo' has no node connected to its
/// exec input".
#[flow_like_types::tokio::test]
async fn a_function_edited_down_to_a_bare_return_still_runs() {
    let (state, catalog, mut board) = function_board().await;
    let text = board_to_flowscript(
        &board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    let start = text.find("function foo").expect("`foo` renders");
    let header_end = start + text[start..].find('\n').expect("`foo` has a body");
    let body_end = start + text[start..].find("\n}\n").expect("`foo` closes");
    let edited = format!(
        "{}\n    return \"bar\", 1 < 2{}",
        &text[..header_end],
        &text[body_end..]
    );
    apply(&state, &catalog, &mut board, &edited).await;
    assert!(
        function_exec_in(&mut board).connected_to.is_empty(),
        "this covers a kept boundary with nothing wired to it"
    );

    let (status, logs) = run_logs(&state, board).await;
    assert!(
        matches!(status, RunStatus::Success),
        "{status:?}: {logs:#?}"
    );
    assert!(logs.iter().any(|log| log == "bar"), "{logs:#?}");
}

/// A body whose entry wire alone was cut still reaches `exec_out`, so it is a broken impure body,
/// not a pure function.
#[flow_like_types::tokio::test]
async fn a_function_body_cut_from_its_entry_still_fails() {
    let (state, _, mut board) = function_board().await;
    let exec_in = function_exec_in(&mut board);
    let exec_in_id = exec_in.id.clone();
    exec_in.connected_to.clear();
    for pin in board
        .nodes
        .values_mut()
        .flat_map(|node| node.pins.values_mut())
    {
        pin.depends_on.remove(&exec_in_id);
    }

    let (status, logs) = run_logs(&state, board).await;
    assert!(matches!(status, RunStatus::Failed), "{status:?}: {logs:#?}");
    assert!(
        logs.iter()
            .any(|log| log.contains("Function 'foo' has no node connected to its exec input")),
        "{logs:#?}"
    );
}
