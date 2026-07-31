//! End-to-end execution coverage for a non-trivial catalog board.
//!
//! Most catalog tests exercise individual node logic. This test deliberately goes through
//! `InternalRun::new` and `InternalRun::execute` so entry-node dispatch, exec-pin ordering, and
//! pure-node dependency pulls are covered together.

use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use flow_like::{
    flow::{
        ast::apply_flowscript_to_board,
        board::Board,
        execution::{InternalRun, LogLevel, RunPayload, RunStatus},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::Path;
use flow_like_types::intercom::BufferedInterComHandler;

const EXECUTION_FIXTURE: &str = include_str!("fixtures/execution_chain.flowscript");
const EXPECTED_LOGS: usize = 32;

async fn executable_board() -> (Arc<FlowLikeState>, Board, String) {
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

#[flow_like_types::tokio::test]
async fn executes_the_complete_catalog_board() {
    let (state, board, entry_id) = executable_board().await;
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
    let mut execution = InternalRun::new(
        "execution-test",
        Arc::new(board),
        None,
        &state,
        &Profile::default(),
        &payload,
        false,
        intercom.into_callback(),
        None,
        None,
        HashMap::new(),
    )
    .await
    .expect("build internal run");
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
