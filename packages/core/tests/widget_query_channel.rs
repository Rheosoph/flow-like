//! End-to-end test of the live run-to-frontend widget-query channel:
//! `ExecutionContext::query_widget` streams an a2ui `widgetQuery` message
//! through the InterCom callback (playing the frontend), which answers through
//! the run's channel via `InProcessChannel::deliver` — exactly what the
//! desktop `channel_push` command does.

use ahash::AHashMap;
use flow_like::{
    flow::{
        board::ExecutionStage,
        execution::{
            LogLevel, Run, context::ExecutionContext, internal_node::InternalNode,
            internal_pin::InternalPin,
        },
        node::{Node, NodeLogic},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_types::{
    Value, async_trait,
    channel::{ChannelPush, ChannelPushKind, InProcessChannel, InProcessPushResult},
    intercom::InterComEvent,
    json::json,
    sync::{Mutex, RwLock},
};
use std::sync::{Arc, Weak};
use std::time::Duration;

#[derive(Default)]
struct NoopLogic;

#[async_trait]
impl NodeLogic for NoopLogic {
    fn get_node(&self) -> Node {
        Node::new("test_noop", "Test Noop", "No-op test node", "Tests")
    }

    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Ok(())
    }
}

fn internal_node(node: Node) -> Arc<InternalNode> {
    let mut pins = AHashMap::new();
    let mut name_cache: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();

    for pin in node.pins.values() {
        let internal_pin = Arc::new(InternalPin::new(pin, false));
        name_cache
            .entry(pin.name.clone())
            .or_default()
            .push(internal_pin.clone());
        pins.insert(pin.id.clone(), internal_pin);
    }

    let internal = Arc::new(InternalNode::new(
        node,
        pins,
        Arc::new(NoopLogic),
        name_cache,
    ));

    for pin in internal.pins.iter() {
        pin.init_node(Arc::downgrade(&internal));
        pin.init_connected_to(Vec::new());
        pin.init_depends_on(Vec::new());
    }

    internal
}

async fn context_with_callback(
    channel_id: &str,
    callback: flow_like_types::intercom::InterComCallback,
) -> (ExecutionContext, Arc<InProcessChannel>) {
    let current = internal_node(Node::new(
        "test_query",
        "Test Query",
        "Query channel test node",
        "Tests",
    ));
    let mut node_map = AHashMap::new();
    node_map.insert(current.node_id().to_string(), current.clone());

    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let variables = Arc::new(Mutex::new(AHashMap::new()));
    let cache = Arc::new(RwLock::new(AHashMap::new()));
    let run: Weak<Mutex<Run>> = Weak::new();
    let channel = InProcessChannel::register(channel_id, Duration::from_secs(60)).await;

    let context = ExecutionContext::new(
        Arc::new(node_map),
        &run,
        &state,
        &current,
        &variables,
        &cache,
        LogLevel::Debug,
        ExecutionStage::Dev,
        Arc::new(Profile::default()),
        callback,
        Arc::new(RwLock::new(Vec::new())),
        None,
        None,
        Arc::new(AHashMap::new()),
        Some(channel.clone()),
    )
    .await;
    (context, channel)
}

/// The "frontend": receives the a2ui widgetQuery message off the InterCom
/// stream and answers through the channel handle it carries.
fn frontend_answering(callback_value: Value) -> flow_like_types::intercom::InterComCallback {
    Some(Arc::new(move |event: InterComEvent| {
        let value = callback_value.clone();
        Box::pin(async move {
            if event.event_type != "a2ui" {
                return Ok(());
            }
            let payload = event.payload;
            if payload.get("type").and_then(Value::as_str) != Some("widgetQuery") {
                return Ok(());
            }
            let handle = payload
                .get("channel")
                .expect("widgetQuery message must carry a channel handle");
            let channel_id = handle["channel_id"].as_str().unwrap().to_string();
            let request_id = handle["request_id"].as_str().unwrap().to_string();
            assert_eq!(
                payload.get("request_id").and_then(Value::as_str),
                Some(request_id.as_str())
            );
            assert_eq!(handle["transport"]["type"], json!("in_process"));
            assert_eq!(
                payload.get("instance_id").and_then(Value::as_str),
                Some("inst-42")
            );
            assert_eq!(
                payload.get("query").and_then(Value::as_str),
                Some("getSelection")
            );
            assert_eq!(payload.get("args"), Some(&json!({ "limit": 5 })));

            flow_like_types::tokio::spawn(async move {
                let push = ChannelPush {
                    channel_id,
                    request_id: Some(request_id),
                    kind: ChannelPushKind::Reply,
                    value,
                };
                let accepted = InProcessChannel::deliver(push.clone()).await;
                assert_eq!(accepted, InProcessPushResult::Delivered);

                let late = InProcessChannel::deliver(ChannelPush {
                    value: json!({ "ok": false, "error": "late duplicate" }),
                    ..push
                })
                .await;
                assert_ne!(late, InProcessPushResult::Delivered);
            });
            Ok(())
        })
    }))
}

#[tokio::test]
async fn test_widget_query_live_roundtrip() {
    let response = json!({ "ok": true, "value": { "rows": [1, 2, 3] } });
    let (mut context, _channel) =
        context_with_callback("wq-run-1", frontend_answering(response)).await;

    let envelope = context
        .query_widget(
            "inst-42",
            "getSelection",
            Some(json!({ "limit": 5 })),
            Duration::from_secs(5),
        )
        .await
        .expect("live round-trip must resolve");

    assert_eq!(envelope["ok"], json!(true));
    assert_eq!(envelope["value"]["rows"], json!([1, 2, 3]));
}

#[tokio::test]
async fn test_widget_query_error_envelope_passthrough() {
    let response = json!({ "ok": false, "error": "widget exploded" });
    let (mut context, _channel) =
        context_with_callback("wq-run-2", frontend_answering(response)).await;

    let envelope = context
        .query_widget(
            "inst-42",
            "getSelection",
            Some(json!({ "limit": 5 })),
            Duration::from_secs(5),
        )
        .await
        .expect("transport succeeds; the error lives in the envelope");

    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["error"], json!("widget exploded"));
}

#[tokio::test]
async fn test_widget_query_times_out_without_responder() {
    let silent: flow_like_types::intercom::InterComCallback =
        Some(Arc::new(|_event| Box::pin(async { Ok(()) })));
    let (mut context, _channel) = context_with_callback("wq-run-3", silent).await;

    let result = context
        .query_widget("inst-42", "getValue", None, Duration::from_secs(1))
        .await;

    let err = result.expect_err("no responder must yield a timeout error");
    assert!(err.to_string().contains("timed out"), "got: {err}");
}

#[tokio::test]
async fn test_widget_query_without_channel_fails_clearly() {
    let silent: flow_like_types::intercom::InterComCallback =
        Some(Arc::new(|_event| Box::pin(async { Ok(()) })));
    let (mut context, _channel) = context_with_callback("wq-run-4", silent).await;
    context.channel = None;

    let err = context
        .query_widget("inst-42", "getValue", None, Duration::from_secs(1))
        .await
        .expect_err("a context without a channel cannot ask the client");
    assert!(err.to_string().contains("No channel"), "got: {err}");
}
