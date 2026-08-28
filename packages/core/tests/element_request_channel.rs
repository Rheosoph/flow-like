//! End-to-end test of the live run→frontend element read: a cache miss in
//! `ExecutionContext::read_element` streams an a2ui `requestElements` message
//! carrying a channel handle, and the page's answer — delivered through
//! `InProcessChannel::deliver`, exactly what the desktop `channel_push`
//! command does — is merged into the run's element cache.

mod support;

use flow_like_types::{
    Value,
    channel::{ChannelPush, ChannelPushKind, InProcessChannel, InProcessPushResult},
    intercom::{InterComCallback, InterComEvent},
    json::json,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use support::context_with_callback;

fn title_element() -> Value {
    json!({
        "page/title": {
            "id": "title",
            "component": { "type": "text", "content": { "literalString": "Hi" } }
        }
    })
}

/// The "page": answers every `requestElements` message with `reply` and counts them.
fn page_answering(reply: Value, requests: Arc<AtomicUsize>) -> InterComCallback {
    Some(Arc::new(move |event: InterComEvent| {
        let reply = reply.clone();
        let requests = requests.clone();
        Box::pin(async move {
            if event.event_type != "a2ui" {
                return Ok(());
            }
            let payload = event.payload;
            if payload.get("type").and_then(Value::as_str) != Some("requestElements") {
                return Ok(());
            }
            requests.fetch_add(1, Ordering::SeqCst);
            let handle = payload
                .get("channel")
                .expect("requestElements must carry a channel handle");
            let channel_id = handle["channel_id"].as_str().unwrap().to_string();
            let request_id = handle["request_id"].as_str().unwrap().to_string();
            assert_eq!(
                payload.get("request_id").and_then(Value::as_str),
                Some(request_id.as_str())
            );
            assert!(payload.get("timeout_ms").and_then(Value::as_u64).unwrap() > 0);
            assert!(!payload["selectors"].as_array().unwrap().is_empty());

            flow_like_types::tokio::spawn(async move {
                let accepted = InProcessChannel::deliver(ChannelPush {
                    channel_id,
                    request_id: Some(request_id),
                    kind: ChannelPushKind::Reply,
                    value: reply,
                })
                .await;
                assert_eq!(accepted, InProcessPushResult::Delivered);
            });
            Ok(())
        })
    }))
}

#[tokio::test]
async fn request_elements_merges_the_answer_into_the_run() {
    let requests = Arc::new(AtomicUsize::new(0));
    let reply = json!({ "ok": true, "elements": title_element() });
    let (mut context, _channel) =
        context_with_callback("er-run-1", page_answering(reply, requests.clone())).await;

    let fetched = context
        .request_elements(vec!["page/title".to_string()], Duration::from_secs(5))
        .await
        .expect("round trip must resolve");
    assert_eq!(fetched.len(), 1);

    let (key, element) = context
        .read_element("title")
        .await
        .unwrap()
        .expect("answered elements are cached for the run");
    assert_eq!(key, "page/title");
    assert_eq!(element["component"]["type"], json!("text"));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn read_element_never_asks_unless_the_client_declared_demand_mode() {
    let requests = Arc::new(AtomicUsize::new(0));
    let reply = json!({ "ok": true, "elements": title_element() });
    let (mut context, _channel) =
        context_with_callback("er-run-2", page_answering(reply, requests.clone())).await;

    assert!(context.read_element("page/title").await.unwrap().is_none());
    assert_eq!(requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn read_element_asks_once_per_id_in_demand_mode() {
    let requests = Arc::new(AtomicUsize::new(0));
    let reply = json!({ "ok": true, "elements": title_element() });
    let (mut context, _channel) =
        context_with_callback("er-run-3", page_answering(reply, requests.clone())).await;
    context.elements.write().await.set_on_demand(true);

    let (key, _) = context
        .read_element("page/title")
        .await
        .unwrap()
        .expect("fetched on demand");
    assert_eq!(key, "page/title");
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    assert!(
        context
            .read_element("page/missing")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        context
            .read_element("page/missing")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        requests.load(Ordering::SeqCst),
        2,
        "a repeated miss for the same id must not ask again"
    );
}

#[tokio::test]
async fn page_side_errors_and_silence_surface_as_errors() {
    let requests = Arc::new(AtomicUsize::new(0));
    let reply = json!({ "ok": false, "error": "no live surface for this run" });
    let (mut context, _channel) =
        context_with_callback("er-run-4", page_answering(reply, requests)).await;
    let err = context
        .request_elements(vec!["page/title".to_string()], Duration::from_secs(5))
        .await
        .expect_err("a page-side error must fail the request");
    assert!(err.to_string().contains("no live surface"), "got: {err}");

    let silent: InterComCallback = Some(Arc::new(|_event| Box::pin(async { Ok(()) })));
    let (mut context, _channel) = context_with_callback("er-run-5", silent).await;
    let err = context
        .request_elements(vec!["page/title".to_string()], Duration::from_secs(1))
        .await
        .expect_err("no responder must yield a timeout error");
    assert!(err.to_string().contains("timed out"), "got: {err}");
}
