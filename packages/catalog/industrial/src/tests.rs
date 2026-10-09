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
use flow_like_types::{
    async_trait,
    json::json,
    sync::{Mutex, RwLock},
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Probe {
    fail: bool,
    calls: Arc<AtomicUsize>,
    cancel_during_run: bool,
}
#[async_trait]
impl NodeLogic for Probe {
    fn get_node(&self) -> Node {
        Node::new("probe", "Probe", "Test handler", "Test")
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.cancel_during_run {
            context.get_cancellation_token().unwrap().cancel();
        }
        if self.fail {
            Err(flow_like_types::anyhow!("handler failure"))
        } else {
            Ok(())
        }
    }
}

fn internal(node: Node, logic: Arc<dyn NodeLogic>) -> Arc<InternalNode> {
    let mut pins = AHashMap::new();
    let mut names: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();
    for pin in node.pins.values() {
        let pin = Arc::new(InternalPin::new(pin, false));
        names
            .entry(pin.name.to_string())
            .or_default()
            .push(pin.clone());
        pins.insert(pin.id().to_string(), pin);
    }
    let node = Arc::new(InternalNode::new(node, pins, logic, names));
    for pin in node.pins.iter() {
        pin.init_node(Arc::downgrade(&node));
        pin.init_connected_to(Vec::new());
        pin.init_depends_on(Vec::new());
    }
    node
}

async fn context(fail: bool) -> (ExecutionContext, Arc<AtomicUsize>) {
    context_with_cancellation(fail, false).await
}

async fn context_with_cancellation(
    fail: bool,
    cancel_during_run: bool,
) -> (ExecutionContext, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let logic: Arc<dyn NodeLogic> = Arc::new(Probe {
        fail,
        calls: calls.clone(),
        cancel_during_run,
    });
    (context_with_logic(logic).await, calls)
}

pub(super) async fn context_with_logic(logic: Arc<dyn NodeLogic>) -> ExecutionContext {
    let mut handler = Node::new("probe", "Probe", "Test handler", "Test");
    handler
        .add_output_pin("event", "Event", "Incoming event", VariableType::Struct)
        .set_open_schema();
    handler.add_output_pin(
        "label",
        "Label",
        "Optional event field",
        VariableType::String,
    );
    handler.add_output_pin("exec_out", "Done", "Done", VariableType::Execution);
    let mut parent = Node::new("listener", "Listener", "Test listener", "Test");
    parent.set_can_reference_fns(true);
    parent
        .fn_refs
        .as_mut()
        .unwrap()
        .fn_refs
        .push(handler.id.clone());
    let handler = internal(handler, logic.clone());
    let parent = internal(parent, logic);
    let nodes = Arc::new(AHashMap::from_iter([
        (parent.node_id().to_string(), parent.clone()),
        (handler.node_id().to_string(), handler),
    ]));
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let run: Weak<Mutex<Run>> = Weak::new();
    let context = ExecutionContext::new(
        nodes,
        &run,
        &state,
        &parent,
        &Arc::new(Mutex::new(AHashMap::new())),
        &Arc::new(RwLock::new(AHashMap::new())),
        LogLevel::Debug,
        ExecutionStage::Dev,
        Arc::new(Profile::default()),
        None,
        Arc::new(RwLock::new(Vec::new())),
        None,
        None,
        Arc::new(AHashMap::new()),
        None,
    )
    .await;
    context
}

#[tokio::test]
async fn handler_success_and_failure_are_visible_to_consumer() {
    for fail in [false, true] {
        let (context, calls) = context(fail).await;
        let mut handler = crate::runtime::Handler::new(&context).await.unwrap();
        let result = handler.dispatch(json!({"payload":[1,2,3]})).await;
        assert_eq!(
            result.is_err(),
            fail,
            "consumer must not acknowledge a failed handler"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn session_cancellation_removes_resource_and_rejects_wrong_protocol() {
    struct Resource(Arc<AtomicBool>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let (mut context, _) = context(false).await;
    let token = tokio_util::sync::CancellationToken::new();
    context.set_cancellation_token(token.clone());
    let dropped = Arc::new(AtomicBool::new(false));
    let session = crate::runtime::store(&context, "test", Resource(dropped.clone())).await;
    assert!(
        crate::runtime::get::<Resource>(&context, &session, "another")
            .await
            .is_err()
    );
    assert!(
        crate::runtime::get::<Resource>(&context, &session, "test")
            .await
            .is_ok()
    );
    token.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!context.has_cache(&session.ref_id).await);
    assert!(
        crate::runtime::get::<Resource>(&context, &session, "test")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancelled_handler_never_reports_acknowledgement_success() {
    for cancel_before in [true, false] {
        let (mut context, calls) = context_with_cancellation(false, !cancel_before).await;
        let token = tokio_util::sync::CancellationToken::new();
        context.set_cancellation_token(token.clone());
        let mut handler = crate::runtime::Handler::new(&context).await.unwrap();
        if cancel_before {
            token.cancel();
        }
        assert!(handler.dispatch(json!({"payload":1})).await.is_err());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            if cancel_before { 0 } else { 1 }
        );
    }
}

#[tokio::test]
async fn later_deliveries_replace_event_and_clear_absent_fields() {
    let (context, calls) = context(false).await;
    let referenced = context.get_referenced_functions().await.unwrap();
    let event = referenced[0]
        .pins
        .iter()
        .find(|pin| pin.name.as_ref() == "event")
        .unwrap()
        .clone();
    let label = referenced[0]
        .pins
        .iter()
        .find(|pin| pin.name.as_ref() == "label")
        .unwrap()
        .clone();
    let mut handler = crate::runtime::Handler::new(&context).await.unwrap();
    handler
        .dispatch(json!({"payload":1,"label":"first"}))
        .await
        .unwrap();
    assert_eq!(
        event.get_raw_value().await.unwrap(),
        json!({"payload":1,"label":"first"})
    );
    assert_eq!(label.get_raw_value().await.unwrap(), json!("first"));
    handler.dispatch(json!({"payload":2})).await.unwrap();
    assert_eq!(event.get_raw_value().await.unwrap(), json!({"payload":2}));
    assert_eq!(label.get_raw_value().await.unwrap(), json!(null));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
