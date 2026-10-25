extern crate flow_like_runtime as flow_like;

use ahash::AHashMap;
use flow_like::{
    flow::{
        board::{Board, ExecutionStage, commands::pins::connect_pins::connect_pins},
        execution::{
            InternalRun, LogLevel, Run, RunPayload, RunStatus, context::ExecutionContext,
            internal_node::InternalNode, internal_pin::InternalPin,
        },
        node::{Node, NodeLogic, NodeState},
        variable::VariableType,
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_catalog_std_runtime::control::for_each::LoopNode;
use flow_like_types::{
    Value, async_trait,
    json::json,
    sync::{Mutex, RwLock},
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicUsize, Ordering},
};

struct Body {
    visits: Arc<Mutex<Vec<(Value, usize)>>>,
    fail_at: Option<usize>,
}

#[async_trait]
impl NodeLogic for Body {
    fn get_node(&self) -> Node {
        let mut node = Node::new("test_body", "Body", "Record each item", "Tests");
        node.add_input_pin("exec_in", "Exec", "Run", VariableType::Execution);
        node.add_input_pin("value", "Value", "Item", VariableType::Generic);
        node.add_input_pin("index", "Index", "Index", VariableType::Integer);
        node.add_output_pin(
            "auto_handle_error",
            "On Error",
            "Handle errors",
            VariableType::Execution,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("auto_handle_error").await?;
        let value = context.evaluate_pin("value").await?;
        let index = context.evaluate_pin("index").await?;
        self.visits.lock().await.push((value, index));
        context.log_message(&format!("Visited item {index}"), LogLevel::Info);
        if self.fail_at == Some(index) {
            return Err(flow_like_types::anyhow!("document lookup failed"));
        }
        Ok(())
    }
}

struct Counter(Arc<AtomicUsize>);

#[async_trait]
impl NodeLogic for Counter {
    fn get_node(&self) -> Node {
        let mut node = Node::new("test_counter", "Counter", "Count executions", "Tests");
        node.add_input_pin("exec_in", "Exec", "Run", VariableType::Execution);
        node
    }

    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn internal(logic: Arc<dyn NodeLogic>) -> Arc<InternalNode> {
    let node = logic.get_node();
    let pins = node
        .pins
        .values()
        .map(|pin| (pin.id.clone(), Arc::new(InternalPin::new(pin, false))))
        .collect();
    let node = Arc::new(InternalNode::new(node, pins, logic, AHashMap::new()));
    for pin in node.pins.iter() {
        pin.init_node(Arc::downgrade(&node));
    }
    node
}

async fn connect(source: &Arc<InternalNode>, output: &str, targets: &[(&Arc<InternalNode>, &str)]) {
    let output = source.get_pin_by_name(output).await.unwrap();
    let mut inputs = Vec::new();
    for (node, name) in targets {
        let input = node.get_pin_by_name(name).await.unwrap();
        input.init_depends_on(vec![Arc::downgrade(&output)]);
        inputs.push(Arc::downgrade(&input));
    }
    output.init_connected_to(inputs);
}

struct Fixture {
    context: ExecutionContext,
    visits: Arc<Mutex<Vec<(Value, usize)>>>,
    sibling_calls: Arc<AtomicUsize>,
    done_calls: Arc<AtomicUsize>,
    handled_errors: Arc<AtomicUsize>,
    body_id: String,
}

impl Fixture {
    async fn new(array: Value, fail_at: Option<usize>, handle_errors: bool) -> Self {
        let parent = internal(Arc::new(LoopNode::new()));
        let visits = Arc::new(Mutex::new(Vec::new()));
        let body = internal(Arc::new(Body {
            visits: visits.clone(),
            fail_at,
        }));
        let sibling_calls = Arc::new(AtomicUsize::new(0));
        let sibling = internal(Arc::new(Counter(sibling_calls.clone())));
        let done_calls = Arc::new(AtomicUsize::new(0));
        let done = internal(Arc::new(Counter(done_calls.clone())));
        let handled_errors = Arc::new(AtomicUsize::new(0));
        let handler = internal(Arc::new(Counter(handled_errors.clone())));

        // Connection traversal visits the last input first.
        connect(
            &parent,
            "exec_out",
            &[(&sibling, "exec_in"), (&body, "exec_in")],
        )
        .await;
        connect(&parent, "value", &[(&body, "value")]).await;
        connect(&parent, "index", &[(&body, "index")]).await;
        connect(&parent, "done", &[(&done, "exec_in")]).await;
        if handle_errors {
            connect(&body, "auto_handle_error", &[(&handler, "exec_in")]).await;
        }
        parent
            .get_pin_by_name("array")
            .await
            .unwrap()
            .set_value(array)
            .await;

        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let variables = Arc::new(Mutex::new(AHashMap::new()));
        let cache = Arc::new(RwLock::new(AHashMap::new()));
        let run: Weak<Mutex<Run>> = Weak::new();
        let nodes = Arc::new(
            [parent.clone(), body.clone(), sibling, done, handler]
                .into_iter()
                .map(|node| (node.node_id().to_string(), node))
                .collect(),
        );
        let context = ExecutionContext::new(
            nodes,
            &run,
            &state,
            &parent,
            &variables,
            &cache,
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(vec![])),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await;
        Self {
            context,
            visits,
            sibling_calls,
            done_calls,
            handled_errors,
            body_id: body.node_id().to_string(),
        }
    }

    async fn output(&self, name: &str) -> Value {
        self.context
            .node
            .get_pin_by_name(name)
            .await
            .unwrap()
            .get_raw_value()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn unhandled_iteration_errors_stop_the_loop_and_propagate() {
    for fail_at in [0, 1] {
        let mut fixture =
            Fixture::new(json!(["first", "second", "third"]), Some(fail_at), false).await;
        let error = InternalNode::trigger(&mut fixture.context, &mut None, true)
            .await
            .unwrap_err();
        let message = format!("{error:?}");
        assert!(message.contains("document lookup failed"), "{message}");
        assert!(
            message.contains(&format!("iteration {fail_at}")),
            "{message}"
        );
        assert_eq!(fixture.visits.lock().await.len(), fail_at + 1);
        assert_eq!(fixture.sibling_calls.load(Ordering::SeqCst), fail_at);
        assert_eq!(fixture.done_calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.output("exec_out").await, json!(false));
        assert_eq!(fixture.output("done").await, json!(false));
        assert_eq!(fixture.context.get_state(), NodeState::Error);
        assert!(fixture.context.sub_traces.iter().any(|trace| {
            trace.node_id.as_ref() == fixture.body_id
                && trace
                    .logs
                    .iter()
                    .any(|log| log.message.contains("document lookup failed"))
                && trace.end >= trace.start
        }));
    }
}

#[tokio::test]
async fn successful_iterations_publish_each_item_and_execute_done_once() {
    let mut fixture = Fixture::new(json!(["first", "second"]), None, false).await;
    InternalNode::trigger(&mut fixture.context, &mut None, true)
        .await
        .unwrap();
    assert_eq!(
        *fixture.visits.lock().await,
        vec![(json!("first"), 0), (json!("second"), 1)]
    );
    assert_eq!(fixture.sibling_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.done_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.output("exec_out").await, json!(false));
    assert_eq!(fixture.output("done").await, json!(true));
}

#[tokio::test]
async fn empty_arrays_execute_done_once_without_running_the_body() {
    let mut fixture = Fixture::new(json!([]), None, false).await;
    InternalNode::trigger(&mut fixture.context, &mut None, true)
        .await
        .unwrap();
    assert!(fixture.visits.lock().await.is_empty());
    assert_eq!(fixture.sibling_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.done_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.output("exec_out").await, json!(false));
    assert_eq!(fixture.output("done").await, json!(true));
}

#[tokio::test]
async fn handled_iteration_errors_allow_the_loop_to_finish() {
    let mut fixture = Fixture::new(json!(["first", "second"]), Some(0), true).await;
    InternalNode::trigger(&mut fixture.context, &mut None, true)
        .await
        .unwrap();
    assert_eq!(
        *fixture.visits.lock().await,
        vec![(json!("first"), 0), (json!("second"), 1)]
    );
    assert_eq!(fixture.handled_errors.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.sibling_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.done_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.output("exec_out").await, json!(false));
    assert_eq!(fixture.output("done").await, json!(true));
}

#[tokio::test]
async fn unhandled_iteration_errors_mark_the_run_failed() {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let visits = Arc::new(Mutex::new(Vec::new()));
    let loop_logic = Arc::new(LoopNode::new());
    let body_logic = Arc::new(Body {
        visits: visits.clone(),
        fail_at: Some(1),
    });
    state
        .node_registry
        .write()
        .await
        .push_nodes(vec![loop_logic.clone(), body_logic.clone()]);

    let mut loop_node = loop_logic.get_node();
    let array_pin = loop_node.get_pin_by_name("array").unwrap().id.clone();
    loop_node
        .pins
        .get_mut(&array_pin)
        .unwrap()
        .set_default_value(Some(json!(["first", "second", "third"])));
    let body_node = body_logic.get_node();
    let loop_id = loop_node.id.clone();
    let body_id = body_node.id.clone();
    let mut board = Board::new_detached(None, Default::default());
    board.nodes.insert(loop_id.clone(), loop_node);
    board.nodes.insert(body_id.clone(), body_node);
    for (output, input) in [
        ("exec_out", "exec_in"),
        ("value", "value"),
        ("index", "index"),
    ] {
        let output = board.nodes[&loop_id]
            .get_pin_by_name(output)
            .unwrap()
            .id
            .clone();
        let input = board.nodes[&body_id]
            .get_pin_by_name(input)
            .unwrap()
            .id
            .clone();
        connect_pins(&mut board, &loop_id, &output, &body_id, &input).unwrap();
    }

    let mut run = InternalRun::new(
        "for-each-errors",
        Arc::new(board),
        None,
        &state,
        &Profile::default(),
        &RunPayload {
            id: loop_id,
            payload: None,
            runtime_variables: None,
            filter_secrets: Some(true),
        },
        false,
        None,
        None,
        None,
        Default::default(),
    )
    .await
    .unwrap();
    run.execute(state).await;

    assert!(matches!(run.get_status().await, RunStatus::Failed));
    assert_eq!(
        *visits.lock().await,
        vec![(json!("first"), 0), (json!("second"), 1)]
    );
}
