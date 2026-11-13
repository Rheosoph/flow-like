use super::*;
use crate::{bridge::ToolEnvBridge, gate::ApprovalGate};
use flow_like::flow::copilot::{
    HostTools,
    platform::PlatformToolBridge,
    tool_grants::{GrantKey, GrantStore},
};
use std::sync::atomic::{AtomicBool, Ordering};

struct NoGrants;

#[async_trait]
impl GrantStore for NoGrants {
    async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>> {
        Ok(Vec::new())
    }
    async fn put(&self, _: &str, _: &str) -> flow_like_types::Result<()> {
        Ok(())
    }
    async fn delete(&self, _: &str, _: &str) -> flow_like_types::Result<bool> {
        Ok(false)
    }
}

fn bridge(hub: Option<Arc<DeviceHub>>, mutating: &Arc<AtomicBool>) -> ToolEnvBridge {
    let bridge = ToolEnvBridge::new(
        Arc::new(|_| panic!("a hub app tool builds no local tool environment")),
        HostTools::none(),
        ApprovalGate::new(Arc::new(NoGrants), "turn_1"),
        mutating.clone(),
        Arc::new(AtomicBool::new(false)),
    );
    match hub {
        Some(hub) => bridge.with_hub_apps(hub, "bot_1", "turn_1"),
        None => bridge,
    }
}

#[tokio::test]
async fn a_read_only_app_tool_runs_on_the_hub_with_the_turns_ids() {
    let hub = Arc::new(DeviceHub::default());
    let mutating = Arc::new(AtomicBool::new(false));
    let output = bridge(Some(hub.clone()), &mutating)
        .call("list_apps", json!({ "query": "crm" }))
        .await;
    assert_eq!(
        serde_json::from_str::<Value>(&output).unwrap()["status"],
        "ok"
    );
    assert_eq!(
        *hub.app_tool_calls.lock().unwrap(),
        [(
            "bot_1".into(),
            "turn_1".into(),
            "list_apps".into(),
            json!({ "query": "crm" })
        )]
    );
    assert!(!mutating.load(Ordering::SeqCst));
}

#[tokio::test]
async fn an_app_tool_without_a_hub_is_still_refused_headless() {
    let mutating = Arc::new(AtomicBool::new(false));
    let output = bridge(None, &mutating).call("list_apps", json!({})).await;
    assert_eq!(
        serde_json::from_str::<Value>(&output).unwrap()["code"],
        "bot_tool_unavailable_headless"
    );
}

#[tokio::test]
async fn a_hub_failure_reads_as_a_final_tool_error() {
    let hub = Arc::new(DeviceHub {
        app_tool_failure: true,
        ..Default::default()
    });
    let mutating = Arc::new(AtomicBool::new(false));
    let output = bridge(Some(hub), &mutating)
        .call("list_apps", json!({}))
        .await;
    let value = serde_json::from_str::<Value>(&output).unwrap();
    assert_eq!(value["code"], "bot_tool_hub_failed");
    assert_eq!(value["retryable"], false);
}

#[tokio::test]
async fn a_tool_the_hub_does_not_host_stays_refused_even_with_a_hub() {
    let hub = Arc::new(DeviceHub::default());
    let mutating = Arc::new(AtomicBool::new(false));
    let output = bridge(Some(hub.clone()), &mutating)
        .call("some_unhosted_tool", json!({}))
        .await;
    assert_eq!(
        serde_json::from_str::<Value>(&output).unwrap()["code"],
        "bot_tool_unavailable_headless"
    );
    assert!(hub.app_tool_calls.lock().unwrap().is_empty());
}
