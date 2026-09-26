extern crate flow_like_runtime as flow_like;

use ahash::AHashMap;
use flow_like::{
    flow::{
        board::ExecutionStage,
        execution::{
            LogLevel, Run, context::ExecutionContext, internal_node::InternalNode,
            internal_pin::InternalPin,
        },
        node::{Node, NodeLogic},
        variable::VariableType,
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_catalog_automation::rpa::{
    error_handler::TryCatchNode, retry::RetryLoopNode, timeout::WithTimeoutNode,
};
use flow_like_types::{
    async_trait,
    json::json,
    sync::{Mutex, RwLock},
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicUsize, Ordering},
};

struct Action {
    calls: Arc<AtomicUsize>,
    failures: usize,
    delay: u64,
}
#[async_trait]
impl NodeLogic for Action {
    fn get_node(&self) -> Node {
        let mut node = Node::new("test_action", "Action", "Test action", "Tests");
        node.add_input_pin("exec_in", "Exec", "Run", VariableType::Execution);
        node
    }
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        tokio::time::sleep(std::time::Duration::from_millis(self.delay)).await;
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < self.failures {
            return Err(flow_like_types::anyhow!("test action failed"));
        }
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

async fn context(logic: Arc<dyn NodeLogic>, branch: &str, action: Action) -> ExecutionContext {
    let parent = internal(logic);
    let child = internal(Arc::new(action));
    let output = parent.get_pin_by_name(branch).await.unwrap();
    let input = child.get_pin_by_name("exec_in").await.unwrap();
    for pin in parent.pins.iter() {
        pin.init_connected_to(if pin.id == output.id {
            vec![Arc::downgrade(&input)]
        } else {
            vec![]
        });
        pin.init_depends_on(vec![]);
    }
    for pin in child.pins.iter() {
        pin.init_connected_to(vec![]);
        pin.init_depends_on(if pin.id == input.id {
            vec![Arc::downgrade(&output)]
        } else {
            vec![]
        });
    }
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let variables = Arc::new(Mutex::new(AHashMap::new()));
    let cache = Arc::new(RwLock::new(AHashMap::new()));
    let run: Weak<Mutex<Run>> = Weak::new();
    let nodes = Arc::new(AHashMap::from_iter([
        (parent.node_id().to_string(), parent.clone()),
        (child.node_id().to_string(), child),
    ]));
    ExecutionContext::new(
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
    .await
}
async fn value(context: &ExecutionContext, name: &str) -> flow_like_types::Value {
    context
        .node
        .get_pin_by_name(name)
        .await
        .unwrap()
        .get_raw_value()
        .await
        .unwrap_or_default()
}

#[tokio::test]
async fn retries_execute_the_action_and_clear_the_branch() {
    let logic = Arc::new(RetryLoopNode::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut context = context(
        logic.clone(),
        "exec_attempt",
        Action {
            calls: calls.clone(),
            failures: 2,
            delay: 0,
        },
    )
    .await;
    context
        .set_pin_value("initial_delay_ms", json!(0))
        .await
        .unwrap();
    logic.run(&mut context).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(value(&context, "exec_success").await, json!(true));
    assert_eq!(value(&context, "exec_attempt").await, json!(false));
    assert_eq!(value(&context, "total_attempts").await, json!(3));
}

#[tokio::test]
async fn try_catch_routes_real_action_errors() {
    let logic = Arc::new(TryCatchNode::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut context = context(
        logic.clone(),
        "exec_try",
        Action {
            calls: calls.clone(),
            failures: 1,
            delay: 0,
        },
    )
    .await;
    logic.run(&mut context).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(value(&context, "exec_catch").await, json!(true));
    assert_eq!(value(&context, "exec_success").await, json!(false));
    assert_eq!(value(&context, "exec_try").await, json!(false));
    assert!(
        value(&context, "message")
            .await
            .as_str()
            .unwrap()
            .contains("test action failed")
    );
}

#[tokio::test]
async fn timeout_stops_pending_actions_and_does_not_fire_success() {
    let logic = Arc::new(WithTimeoutNode::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut context = context(
        logic.clone(),
        "exec_action",
        Action {
            calls: calls.clone(),
            failures: 0,
            delay: 100,
        },
    )
    .await;
    context.set_pin_value("timeout_ms", json!(5)).await.unwrap();
    logic.run(&mut context).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(value(&context, "exec_timeout").await, json!(true));
    assert_eq!(value(&context, "exec_success").await, json!(false));
    assert_eq!(value(&context, "exec_action").await, json!(false));
}
