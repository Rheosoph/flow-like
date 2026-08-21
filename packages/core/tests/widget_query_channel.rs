//! End-to-end test of the live run→frontend widget-query channel:
//! `ExecutionContext::query_widget` streams an a2ui `widgetQuery` message
//! through the InterCom callback (playing the frontend), which answers via
//! `flow_like_types::frontend_request::resolve_frontend_request` — exactly
//! what the `respond_widget_query` Tauri command and the
//! `/widget-query/{id}/respond` API route do.

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
    callback: flow_like_types::intercom::InterComCallback,
) -> ExecutionContext {
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

    ExecutionContext::new(
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
    )
    .await
}

/// The "frontend": receives the a2ui widgetQuery message off the InterCom
/// stream and answers through the global frontend-request registry.
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
            let request_id = payload
                .get("request_id")
                .and_then(Value::as_str)
                .expect("widgetQuery message must carry request_id")
                .to_string();
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
                let accepted =
                    flow_like_types::frontend_request::resolve_frontend_request(&request_id, value)
                        .await;
                assert!(accepted, "first response must be accepted");

                let late = flow_like_types::frontend_request::resolve_frontend_request(
                    &request_id,
                    json!({ "ok": false, "error": "late duplicate" }),
                )
                .await;
                assert!(!late, "duplicate response must be rejected");
            });
            Ok(())
        })
    }))
}

#[tokio::test]
async fn test_widget_query_live_roundtrip() {
    let response = json!({ "ok": true, "value": { "rows": [1, 2, 3] } });
    let mut context = context_with_callback(frontend_answering(response)).await;

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
    let mut context = context_with_callback(frontend_answering(response)).await;

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
    let mut context = context_with_callback(silent).await;

    let result = context
        .query_widget("inst-42", "getValue", None, Duration::from_millis(150))
        .await;

    let err = result.expect_err("no responder must yield a timeout error");
    assert!(err.to_string().contains("timed out"), "got: {err}");
}
