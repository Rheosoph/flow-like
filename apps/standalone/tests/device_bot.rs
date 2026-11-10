mod support;

use async_trait::async_trait;
use flow_like_bot_tools::browser::{
    BrowserBackend, BrowserStep, Download, PageState, SnapshotRequest, SnapshotView,
};
use flow_like_standalone::bot_host::{
    self,
    env::{MachineBackends, Machines},
};
use flow_like_types::{
    bot::BotCapability,
    bot_turn::{reply_message_id, turn_id},
    device_bot::{DeviceParkRequest, DeviceReleaseRequest, DeviceReplyRequest},
};
use serde_json::json;
use std::{collections::VecDeque, sync::Arc, time::Duration};
use support::*;
use tokio_util::sync::CancellationToken;

const WAIT: Duration = Duration::from_secs(90);

fn scripted(steps: impl IntoIterator<Item = Step>) -> VecDeque<Step> {
    steps.into_iter().collect()
}

fn position(hub: &Hub, needle: &str) -> usize {
    hub.requests
        .iter()
        .position(|rec| format!("{} {}", rec.method, rec.path).ends_with(needle))
        .unwrap_or_else(|| panic!("no request ending in {needle:?}; saw {:#?}", hub.paths("")))
}

fn count(hub: &Hub, suffix: &str) -> usize {
    hub.calls(suffix).len()
}

struct NoBrowser;

#[async_trait]
impl BrowserBackend for NoBrowser {
    async fn open(&self, url: &str) -> flow_like_types::Result<PageState> {
        panic!("the turn must park before opening {url}")
    }
    async fn snapshot(&self, _: SnapshotRequest) -> flow_like_types::Result<SnapshotView> {
        unreachable!()
    }
    async fn act(&self, _: &BrowserStep) -> flow_like_types::Result<PageState> {
        unreachable!()
    }
    async fn screenshot(&self) -> flow_like_types::Result<Vec<u8>> {
        unreachable!()
    }
    async fn current(&self) -> flow_like_types::Result<Option<PageState>> {
        Ok(None)
    }
    async fn leave(&self) -> flow_like_types::Result<()> {
        Ok(())
    }
    async fn take_downloads(&self) -> flow_like_types::Result<Vec<Download>> {
        Ok(Vec::new())
    }
    async fn close(&self) -> flow_like_types::Result<()> {
        Ok(())
    }
}

struct FakeBrowserMachines;

#[test]
fn configuration_tools_are_not_offered_without_a_device_configuration_backend() {
    let tools = bot_host::runner::host_tools(&MachineBackends::default());
    for tool in [
        "bot_configuration",
        "bot_models",
        "bot_update_configuration",
    ] {
        assert!(!tools.runs(tool), "{tool}");
    }
}

impl Machines for FakeBrowserMachines {
    fn machine(&self, _: &str, _: Option<&str>, _: bool, _: bool) -> MachineBackends {
        MachineBackends {
            browser: Some(Arc::new(NoBrowser)),
            computer: None,
        }
    }
}

#[tokio::test]
async fn a_queued_turn_is_answered_through_the_real_entry_point() {
    let _serial = serial().await;
    let mock = MockHub::start(Scenario {
        model: scripted([Step::Text("Hello from the device".into())]),
        ..Scenario::default()
    })
    .await;
    let state_dir = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let host = tokio::spawn(bot_host::run(
        state_dir.path().to_owned(),
        mock.session.clone(),
        cancel.clone(),
    ));
    mock.wait("the reply", WAIT, |hub| hub.state == TurnState::Replied)
        .await;
    let polls = count(&*mock.hub.lock().await, "/bots/poll");
    mock.wait("an idle poll after the reply", WAIT, |hub| {
        count(hub, "/bots/poll") > polls
    })
    .await;
    cancel.cancel();
    host.await.unwrap().unwrap();

    let hub = mock.hub.lock().await;
    let device = hub.device_id();
    let turn = turn_id(BOT_ID, TRIGGER_ID);
    let bots = format!("/api/v1/devices/{device}/bots");
    let order = [
        format!("POST {bots}/poll"),
        format!("POST {bots}/turns/{turn}/claim"),
        format!("GET /api/v1/inbox/conversations/{CONVERSATION_ID}/entries"),
        format!("GET {bots}/turns/{turn}/checkpoint"),
        "POST /api/v1/chat/completions".to_owned(),
        format!("POST {bots}/turns/{turn}/reply"),
    ];
    let mut last = 0;
    for step in &order {
        let at = position(&hub, step);
        assert!(at >= last, "{step} came out of order: {:#?}", hub.paths(""));
        last = at;
    }
    let load = &hub.requests[position(&hub, &format!("GET {bots}/turns/{turn}/checkpoint"))];
    for fence in [format!("host_id=device%3A{device}"), "attempt=1".to_owned()] {
        assert!(load.query.contains(&fence), "{fence}: {load:?}");
    }
    let reply_at = position(&hub, &format!("/turns/{turn}/reply"));
    let next_poll = hub.requests[reply_at..]
        .iter()
        .position(|rec| rec.path.ends_with("/bots/poll"))
        .expect("the host polls again after replying");
    assert!(next_poll > 0);
    position(&hub, &format!("POST {bots}/hosts/forget"));

    let reply: DeviceReplyRequest =
        serde_json::from_value(hub.requests[reply_at].body.clone()).unwrap();
    assert_eq!(reply.message_id, reply_message_id(BOT_ID, TRIGGER_ID));
    assert_eq!(reply.host_id, format!("device:{device}"));
    assert_eq!(reply.envelope["enc"], "none");
    assert_eq!(reply.envelope["body"]["text"], "Hello from the device");
    assert_eq!(reply.failure_code, None);
    assert_eq!(reply.usage.unwrap().operation_id, OPERATION_ID);

    assert!(hub.dpop_verified >= 4);
    for rec in hub
        .requests
        .iter()
        .filter(|rec| rec.path.contains("/bots/poll") || rec.path.contains("/turns/"))
    {
        assert!(rec.authorization.starts_with("DPoP "), "{rec:?}");
    }
    for rec in hub
        .requests
        .iter()
        .filter(|rec| rec.path.contains("/inbox/") || rec.path.contains("tool-grants"))
    {
        assert_eq!(rec.authorization, format!("Bearer {BOT_TOKEN}"), "{rec:?}");
    }
    let model = &hub.model_requests[0];
    assert!(model.to_string().contains("Please say hello"), "{model}");
    assert_eq!(
        hub.calls("/silent").len() + hub.calls("/park").len() + hub.calls("/release").len(),
        0
    );
}

#[tokio::test]
async fn an_approval_gated_tool_without_a_grant_parks_the_turn() {
    let _serial = serial().await;
    let mock = MockHub::start(Scenario {
        capabilities: vec![BotCapability::BrowserUse],
        model: scripted([Step::Tool(
            "browser_open",
            json!({"url": "https://example.org/"}),
        )]),
        ..Scenario::default()
    })
    .await;
    let _host = start_host(mock.session.clone(), Some(Arc::new(FakeBrowserMachines)));
    mock.wait("the park", WAIT, |hub| hub.state == TurnState::Parked)
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let hub = mock.hub.lock().await;
    let park = hub.calls("/park");
    assert_eq!(park.len(), 1);
    let park: DeviceParkRequest = serde_json::from_value(park[0].body.clone()).unwrap();
    assert_eq!(park.approval.tool, "browser_open");
    assert_eq!(park.approval.turn_id, turn_id(BOT_ID, TRIGGER_ID));
    assert_eq!(park.approval.scope, "browser:example.org");
    assert_eq!(park.usage.unwrap().operation_id, OPERATION_ID);
    assert_eq!(count(&hub, "/reply"), 0);
    assert_eq!(count(&hub, "/release"), 0);
}

#[tokio::test]
async fn a_lost_lease_stops_the_run_and_posts_nothing() {
    let _serial = serial().await;
    let mock = MockHub::start(Scenario {
        lease_lost: true,
        model: scripted([Step::Slow(
            Duration::from_secs(8),
            Box::new(Step::Text("too late".into())),
        )]),
        ..Scenario::default()
    })
    .await;
    let _host = start_host(mock.session.clone(), None);
    mock.wait("the lost lease", WAIT, |hub| hub.state == TurnState::Lost)
        .await;
    mock.wait(
        "the model call to be dropped",
        Duration::from_secs(5),
        |hub| hub.model_in_flight == 0,
    )
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let hub = mock.hub.lock().await;
    assert_eq!(hub.model_dropped, hub.model_requests.len());
    assert_eq!(count(&hub, "/reply"), 0);
    assert_eq!(count(&hub, "/silent"), 0);
    assert_eq!(count(&hub, "/park"), 0);
    assert_eq!(count(&hub, "/claim"), 1);
}

#[tokio::test]
async fn a_sealed_conversation_is_refused_without_further_calls() {
    let _serial = serial().await;
    let mock = MockHub::start(Scenario {
        claim: Claim::Refuse(409, "BOT_TURN_ENCRYPTED"),
        ..Scenario::default()
    })
    .await;
    let _host = start_host(mock.session.clone(), None);
    mock.wait("the refused claim", WAIT, |hub| {
        hub.state == TurnState::Failed
    })
    .await;
    let polls = count(&*mock.hub.lock().await, "/bots/poll");
    mock.wait("two more polls", WAIT, |hub| {
        count(hub, "/bots/poll") >= polls + 2
    })
    .await;
    let hub = mock.hub.lock().await;
    assert_eq!(count(&hub, "/claim"), 1);
    for forbidden in [
        "/heartbeat",
        "/reply",
        "/silent",
        "/park",
        "/release",
        "/entries",
        "/chat/completions",
    ] {
        assert_eq!(
            count(&hub, forbidden),
            0,
            "{forbidden}: {:#?}",
            hub.paths("")
        );
    }
}

#[tokio::test]
async fn a_model_failure_before_any_side_effect_releases_the_turn_for_a_retry() {
    let _serial = serial().await;
    let mock = MockHub::start(Scenario {
        model: scripted([Step::Status(500), Step::Status(500), Step::Status(500)]),
        ..Scenario::default()
    })
    .await;
    let _host = start_host(mock.session.clone(), None);
    mock.wait("the release", WAIT, |hub| hub.state == TurnState::Released)
        .await;
    let hub = mock.hub.lock().await;
    let release = hub.calls("/release");
    assert_eq!(release.len(), 1);
    let release: DeviceReleaseRequest = serde_json::from_value(release[0].body.clone()).unwrap();
    assert!(release.retry, "{release:?}");
    assert_eq!(release.error_code, "BOT_TURN_FAILED");
    assert_eq!(release.usage.unwrap().operation_id, OPERATION_ID);
    assert_eq!(count(&hub, "/reply"), 0);
}

#[tokio::test]
#[ignore = "drives real Chrome; run with FLOW_LIKE_E2E_BROWSER=1 and --ignored"]
async fn a_browser_turn_reads_a_page_through_the_playground_worker() {
    if std::env::var("FLOW_LIKE_E2E_BROWSER").as_deref() != Ok("1") {
        eprintln!("FLOW_LIKE_E2E_BROWSER=1 is not set; skipping");
        return;
    }
    let _serial = serial().await;
    let page = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", page.local_addr().unwrap());
    let pages = axum::Router::new().route(
        "/",
        axum::routing::get(|| async {
            axum::response::Html(
                "<html><head><title>Vault</title></head><body><h1>The vault code is 7391</h1></body></html>",
            )
        }),
    );
    let page_server = tokio::spawn(async move { axum::serve(page, pages).await.unwrap() });
    let mock = MockHub::start(Scenario {
        capabilities: vec![BotCapability::BrowserUse],
        grants: vec![("browser_open".into(), "browser:127.0.0.1".into())],
        model: scripted([
            Step::Tool("browser_open", json!({ "url": url })),
            Step::Tool("browser_snapshot", json!({})),
            Step::EchoLastTool("The page says: "),
        ]),
        ..Scenario::default()
    })
    .await;
    let data = tempfile::tempdir().unwrap();
    let machines = flow_like_standalone::playground::MachineHost::new(
        data.path().to_owned(),
        flow_like_standalone::playground::Mode::Worker {
            program: env!("CARGO_BIN_EXE_flow-like-standalone").into(),
        },
    );
    let _host = start_host(mock.session.clone(), Some(Arc::new(machines.clone())));
    mock.wait("the reply", Duration::from_secs(150), |hub| {
        hub.state == TurnState::Replied
    })
    .await;
    machines.shutdown().await;
    let hub = mock.hub.lock().await;
    let reply: DeviceReplyRequest =
        serde_json::from_value(hub.calls("/reply")[0].body.clone()).unwrap();
    let text = reply.envelope["body"]["text"].as_str().unwrap();
    assert!(text.contains("7391"), "{text}");
    assert_eq!(hub.model_requests.len(), 3);
    assert_eq!(count(&hub, "/park"), 0);
    page_server.abort();
}
