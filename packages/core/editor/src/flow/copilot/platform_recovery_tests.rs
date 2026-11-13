use super::*;
use crate::flow::copilot::run_recovery::{
    BotRunRecovery, PendingCall, RunSnapshot, current_recovery_call_id, finished_reply,
    record_held_batch, resumes_held_round,
};
use crate::flow::copilot::tool_grants::{
    GrantKey, GrantStore, HeadlessAnswer, ParkedBatch, expensive_documents_request, grant_scopes,
    record_batch_answer,
};
use crate::flow::copilot::{ReplyFiles, ToolTrail};
use flow_like_types::bot_turn::BotRunCheckpoint;
use flow_like_types::tool_grant::{ApprovalAnswer, ApprovalDecision};
use std::sync::atomic::{AtomicI64, AtomicU64};

#[derive(Debug, Default)]
struct Store {
    saved: Mutex<Option<BotRunCheckpoint>>,
    writes: Mutex<Vec<BotRunCheckpoint>>,
    fail_before: AtomicU64,
    fail_after: AtomicU64,
    deadline: AtomicI64,
}

#[async_trait]
impl BotRunRecovery for Store {
    async fn load(&self) -> Result<Option<BotRunCheckpoint>> {
        Ok(self.saved.lock().unwrap().clone())
    }
    async fn save(&self, checkpoint: &BotRunCheckpoint) -> Result<()> {
        checkpoint
            .validate()
            .map_err(|error| flow_like_types::anyhow!(error))?;
        if self.fail_before.load(Ordering::Relaxed) == checkpoint.revision {
            return Err(flow_like_types::anyhow!("injected failure before commit"));
        }
        let mut saved = self.saved.lock().unwrap();
        let expected = saved.as_ref().map_or(1, |saved| saved.revision + 1);
        if checkpoint.revision != expected {
            return Err(flow_like_types::anyhow!("stale checkpoint writer"));
        }
        if let Some(previous) = saved.as_ref() {
            assert_eq!(checkpoint.started_at_ms, previous.started_at_ms);
            assert_eq!(checkpoint.deadline_at_ms, previous.deadline_at_ms);
        }
        *saved = Some(checkpoint.clone());
        self.writes.lock().unwrap().push(checkpoint.clone());
        if self.fail_after.load(Ordering::Relaxed) == checkpoint.revision {
            return Err(flow_like_types::anyhow!(
                "injected process interruption after commit"
            ));
        }
        Ok(())
    }
    fn deadline_at_ms(&self) -> Option<i64> {
        let deadline = self.deadline.load(Ordering::Relaxed);
        (deadline != 0).then_some(deadline)
    }
}

async fn run_saved(
    client: &Client,
    store: Arc<Store>,
    rounds: usize,
    prompt: &str,
    images: Option<Vec<ChatImage>>,
) -> (Result<String>, Arc<Bridge>) {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let bridge = Arc::new(Bridge {
        calls: AtomicUsize::new(0),
        cancellation_checks: AtomicUsize::new(0),
        cancelled: client.0.cancelled.clone(),
        large_results: false,
        steering: Mutex::new(Vec::new()),
    });
    let result = PlatformCopilot::new(state, None)
        .with_tool_specs(vec![find_global_tool_spec("list_apps").unwrap()])
        .with_tool_rounds(rounds)
        .with_long_running()
        .with_recovery(store)
        .chat_with_client(
            PlatformSurface::Orchestrator,
            "Help the owner.".into(),
            prompt.into(),
            images,
            Vec::new(),
            "scripted".into(),
            client,
            None,
            bridge.clone(),
            None,
            None::<fn(String)>,
        )
        .await;
    (result, bridge)
}

async fn interrupted(store: Arc<Store>, after_commit: bool, rounds: usize) -> Arc<Bridge> {
    if after_commit {
        store.fail_after.store(3, Ordering::Relaxed);
    } else {
        store.fail_before.store(3, Ordering::Relaxed);
    }
    let client = Client::new(vec![call("inspection", "list_apps", json!({}))]);
    let (result, bridge) =
        run_saved(&client, store, rounds, "Inspect the app inventory.", None).await;
    assert!(result.is_err());
    assert_eq!(bridge.calls.load(Ordering::Relaxed), 1);
    bridge
}

#[tokio::test]
async fn a_committed_tool_result_resumes_without_repeating_the_action() {
    let store = Arc::new(Store::default());
    interrupted(store.clone(), true, 20).await;
    store.fail_after.store(0, Ordering::Relaxed);
    assert_eq!(
        store.saved.lock().unwrap().as_ref().unwrap().state["phase"],
        "ready"
    );
    let client = Client::new(vec![text("The inventory is verified.")]);
    let (answer, bridge) = run_saved(
        &client,
        store.clone(),
        128,
        "Inspect the app inventory.\nThe owner approved continuing.",
        None,
    )
    .await;
    assert_eq!(answer.unwrap(), "The inventory is verified.");
    assert_eq!(bridge.calls.load(Ordering::Relaxed), 0);
    let requests = client.0.requests.lock().unwrap();
    let restored = serde_json::to_string(&requests[0]).unwrap();
    assert!(restored.contains("resumed from a durable checkpoint"));
    assert!(restored.contains("owner approved continuing"));
    assert_complete_tool_pairs(&requests[0]);
    let writes = store.writes.lock().unwrap();
    for (index, checkpoint) in writes.iter().enumerate() {
        assert_eq!(checkpoint.revision, index as u64 + 1);
    }
    assert_eq!(writes.last().unwrap().state["max_tool_rounds"], 20);
    assert_eq!(writes.last().unwrap().state["recoveries"], 1);
    assert_eq!(writes.last().unwrap().state["model_calls_reserved"], 2);
}

#[tokio::test]
async fn an_unknown_tool_outcome_is_never_replayed() {
    let store = Arc::new(Store::default());
    interrupted(store.clone(), false, 20).await;
    store.fail_before.store(0, Ordering::Relaxed);
    let client = Client::new(Vec::new());
    let (answer, bridge) = run_saved(
        &client,
        store.clone(),
        20,
        "Inspect the app inventory.",
        None,
    )
    .await;
    let answer = answer.unwrap();
    assert!(answer.contains("outcomes are unknown"));
    assert!(answer.contains("list_apps (inspection)"));
    assert_eq!(bridge.calls.load(Ordering::Relaxed), 0);
    assert!(client.0.requests.lock().unwrap().is_empty());
    assert_eq!(
        finished_reply(store.saved.lock().unwrap().as_ref().unwrap()).unwrap(),
        Some(answer)
    );
}

#[tokio::test]
async fn checkpoint_failure_before_dispatch_prevents_the_tool_action() {
    let store = Arc::new(Store::default());
    store.fail_before.store(2, Ordering::Relaxed);
    let client = Client::new(vec![call("inspection", "list_apps", json!({}))]);
    let (answer, bridge) = run_saved(&client, store, 20, "Inspect the app inventory.", None).await;
    assert!(answer.is_err());
    assert_eq!(bridge.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn stored_completion_and_expired_runs_need_no_model() {
    let store = Arc::new(Store::default());
    let client = Client::new(vec![text("Already finished.")]);
    assert_eq!(
        run_saved(
            &client,
            store.clone(),
            20,
            "Inspect the app inventory.",
            None
        )
        .await
        .0
        .unwrap(),
        "Already finished."
    );
    store
        .deadline
        .store(chrono::Utc::now().timestamp_millis() - 1, Ordering::Relaxed);
    let client = Client::new(Vec::new());
    assert_eq!(
        run_saved(
            &client,
            store.clone(),
            20,
            "Inspect the app inventory.",
            None
        )
        .await
        .0
        .unwrap(),
        "Already finished."
    );
    assert!(client.0.requests.lock().unwrap().is_empty());

    let store = Arc::new(Store::default());
    interrupted(store.clone(), true, 20).await;
    store.fail_after.store(0, Ordering::Relaxed);
    store
        .deadline
        .store(chrono::Utc::now().timestamp_millis() - 1, Ordering::Relaxed);
    let client = Client::new(Vec::new());
    let answer = run_saved(
        &client,
        store.clone(),
        128,
        "Inspect the app inventory.",
        None,
    )
    .await
    .0
    .unwrap();
    assert!(answer.contains("execution deadline passed"));
    assert!(client.0.requests.lock().unwrap().is_empty());
    assert_eq!(
        store.saved.lock().unwrap().as_ref().unwrap().state["answer"]["stop_reason"],
        serde_json::to_value(WorkStopReason::Budget).unwrap()
    );
}

#[tokio::test]
async fn recovery_keeps_original_round_and_attempt_limits() {
    let store = Arc::new(Store::default());
    interrupted(store.clone(), true, 1).await;
    store.fail_after.store(0, Ordering::Relaxed);
    let client = Client::new(vec![text("Final evidence report.")]);
    let answer = run_saved(
        &client,
        store.clone(),
        128,
        "Inspect the app inventory.",
        None,
    )
    .await
    .0
    .unwrap();
    assert!(answer.contains("step limit"));
    assert!(client.0.requests.lock().unwrap()[0].tools.is_empty());

    let store = Arc::new(Store::default());
    interrupted(store.clone(), true, 20).await;
    store.fail_after.store(0, Ordering::Relaxed);
    store.saved.lock().unwrap().as_mut().unwrap().state["recoveries"] = json!(3);
    let client = Client::new(Vec::new());
    assert!(
        run_saved(&client, store, 20, "Inspect the app inventory.", None)
            .await
            .0
            .unwrap()
            .contains("recovery limit")
    );
    assert!(client.0.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_round_saved_before_held_calls_is_reconciled_as_before() {
    let store = Arc::new(Store::default());
    interrupted(store.clone(), false, 20).await;
    store.fail_before.store(0, Ordering::Relaxed);
    let (mut session, saved) = RecoverySession::load(store.clone()).await.unwrap();
    let saved = saved.unwrap();
    let paused =
        json!({"status": "error", "code": "bot_tool_waiting_for_approval", "executed": false});
    session
        .record_result(
            "inspection",
            "list_apps",
            paused.to_string(),
            saved.work_plan.clone(),
            saved.web.clone(),
            saved.delegated_web.clone(),
            saved.stats.clone(),
        )
        .await
        .unwrap();
    let promoted = store.saved.lock().unwrap().clone().unwrap();
    assert_eq!(promoted.state["phase"], "ready");
    let confirm = |id: &str| record_held_batch(&promoted, &[(id.into(), "list_apps".into())]);
    assert!(confirm("other").unwrap().is_none());
    let resumed = confirm("inspection").unwrap().unwrap();
    assert_eq!(resumed.revision, promoted.revision + 1);
    assert_eq!(resumed.state["approval_resume"], true);
    assert!(!resumes_held_round(&resumed));
    let again = record_held_batch(&resumed, &[("inspection".into(), "list_apps".into())]);
    assert!(again.unwrap().is_none());
}

#[tokio::test]
async fn durable_payload_omits_images_and_retains_reference_fingerprints() {
    let store = Arc::new(Store::default());
    store.fail_after.store(1, Ordering::Relaxed);
    let client = Client::new(Vec::new());
    let data = "c2VjcmV0LXZpc3VhbC1yZWZlcmVuY2U=";
    assert!(
        run_saved(
            &client,
            store.clone(),
            20,
            "Inspect the app inventory.",
            Some(vec![ChatImage::inline(data.into(), "image/png".into())])
        )
        .await
        .0
        .is_err()
    );
    let checkpoint = store.saved.lock().unwrap().clone().unwrap();
    assert!(!checkpoint.state.to_string().contains(data));
    assert_eq!(
        checkpoint.state["reference_fingerprints"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    store.fail_after.store(0, Ordering::Relaxed);
    assert!(
        run_saved(&client, store, 20, "Inspect the app inventory.", None)
            .await
            .0
            .unwrap()
            .contains("visual references are unavailable")
    );
}

#[tokio::test]
async fn tool_call_identity_is_scoped_and_restored() {
    use crate::flow::copilot::run_recovery::{CallScope, round_has_web_content, scope_call};
    let scope = |id: &str, round_web_content| CallScope {
        id: id.into(),
        round_web_content,
        held_replay: false,
    };
    assert!(current_recovery_call_id().is_none());
    scope_call(scope("parent", true), async {
        assert_eq!(current_recovery_call_id().as_deref(), Some("parent"));
        assert!(round_has_web_content());
        scope_call(scope("child", false), async {
            assert_eq!(current_recovery_call_id().as_deref(), Some("child"));
            assert!(!round_has_web_content());
        })
        .await;
        assert_eq!(current_recovery_call_id().as_deref(), Some("parent"));
    })
    .await;
    assert!(current_recovery_call_id().is_none());
    assert!(!round_has_web_content());
}

#[derive(Default)]
struct ApprovalBridge(Mutex<Option<String>>);

#[async_trait]
impl PlatformToolBridge for ApprovalBridge {
    async fn call(&self, _: &str, _: Value) -> String {
        *self.0.lock().unwrap() = current_recovery_call_id();
        json!({"status":"waiting", "code":"bot_tool_waiting_for_approval", "executed":false})
            .to_string()
    }
}

#[tokio::test]
async fn repeated_manual_approvals_do_not_spend_the_crash_recovery_allowance() {
    let store = Arc::new(Store::default());
    for _ in 0..5 {
        let client = Client::new(vec![call("approval", "list_apps", json!({}))]);
        let bridge = Arc::new(ApprovalBridge::default());
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let result = PlatformCopilot::new(state, None)
            .with_tool_specs(vec![find_global_tool_spec("list_apps").unwrap()])
            .with_tool_rounds(20)
            .with_long_running()
            .with_recovery(store.clone())
            .chat_with_client(
                PlatformSurface::Orchestrator,
                "Help the owner.".into(),
                "Inspect the app inventory.".into(),
                None,
                Vec::new(),
                "scripted".into(),
                &client,
                None,
                bridge.clone(),
                None,
                None::<fn(String)>,
            )
            .await;
        assert!(result.is_err());
        let checkpoint = store.saved.lock().unwrap().clone().unwrap();
        assert_eq!(checkpoint.state["phase"], "tools_in_flight");
        assert_eq!(checkpoint.state["recoveries"], 0);
        assert_eq!(checkpoint.state["model_calls_reserved"], 1);
        let call_id = bridge.0.lock().unwrap().clone().unwrap();
        let next = record_held_batch(&checkpoint, &[(call_id, "list_apps".into())])
            .unwrap()
            .unwrap();
        assert_eq!(next.state["approval_resume"], true);
        store.save(&next).await.unwrap();
    }
    let client = Client::new(vec![text("The approved work is finished.")]);
    assert_eq!(
        run_saved(
            &client,
            store.clone(),
            128,
            "Inspect the app inventory.",
            None
        )
        .await
        .0
        .unwrap(),
        "The approved work is finished."
    );
    assert_eq!(
        store.saved.lock().unwrap().as_ref().unwrap().state["recoveries"],
        0
    );
}

#[derive(Default)]
struct Grants(Mutex<Vec<GrantKey>>);

#[async_trait]
impl GrantStore for Grants {
    async fn list(&self) -> Result<Vec<GrantKey>> {
        Ok(self.0.lock().unwrap().clone())
    }
    async fn put(&self, tool: &str, scope: &str) -> Result<()> {
        let key = GrantKey::new(tool, scope);
        let mut keys = self.0.lock().unwrap();
        if !keys.contains(&key) {
            keys.push(key);
        }
        Ok(())
    }
    async fn delete(&self, tool: &str, scope: &str) -> Result<bool> {
        let mut keys = self.0.lock().unwrap();
        let before = keys.len();
        keys.retain(|key| !(key.tool == tool && key.scope == scope));
        Ok(keys.len() != before)
    }
}

/// A headless host with one approval batch per attempt: `web_request` asks the owner, and an
/// expensive `file_extract` parks for its cost after it ran.
struct GateHost {
    grants: Arc<Grants>,
    batch: ParkedBatch,
    ran: Mutex<Vec<String>>,
}

fn fetch_description(arguments: &Value) -> String {
    format!(
        "The bot wants to send a GET request to {}.",
        arguments["url"].as_str().unwrap_or_default()
    )
}

fn waiting_for(tool: &str) -> String {
    json!({"status": "waiting", "tool": tool, "code": "bot_tool_waiting_for_approval"}).to_string()
}

impl GateHost {
    fn new(grants: &Arc<Grants>) -> Arc<Self> {
        Arc::new(Self {
            grants: grants.clone(),
            batch: ParkedBatch::new("bt_1"),
            ran: Mutex::new(Vec::new()),
        })
    }

    fn ran(&self) -> Vec<String> {
        self.ran.lock().unwrap().clone()
    }
}

#[async_trait]
impl PlatformToolBridge for GateHost {
    async fn call(&self, name: &str, arguments: Value) -> String {
        if name == "web_request" {
            let description = fetch_description(&arguments);
            match self
                .batch
                .answer(
                    self.grants.as_ref(),
                    name,
                    &arguments,
                    None,
                    &description,
                    None,
                )
                .await
            {
                HeadlessAnswer::Run => {}
                HeadlessAnswer::Refuse(message) => {
                    return json!({"status": "error", "message": message}).to_string();
                }
                HeadlessAnswer::Park(_) => return waiting_for(name),
            }
        }
        let target = arguments["url"].as_str().unwrap_or(name).to_string();
        self.ran.lock().unwrap().push(target.clone());
        if name == "file_extract" {
            let card = expensive_documents_request("bt_1", name, &arguments, "Above the limit.");
            self.batch.park(card, false);
            return waiting_for(name);
        }
        if name == "list_apps" {
            tokio::time::sleep(Duration::from_millis(50)).await;
            return json!({"status": "ok", "apps": [], "observation": "slow sibling"}).to_string();
        }
        json!({"status": "ok", "http_status": 200, "body": format!("feed of {target}")}).to_string()
    }

    async fn clearance(&self, name: &str, arguments: &Value) -> Clearance {
        if name != "web_request" {
            return Clearance::Clear;
        }
        let description = fetch_description(arguments);
        match self
            .batch
            .clearance(
                self.grants.as_ref(),
                name,
                arguments,
                None,
                &description,
                None,
            )
            .await
        {
            HeadlessAnswer::Run => Clearance::Clear,
            HeadlessAnswer::Refuse(message) => Clearance::Refused(message),
            HeadlessAnswer::Park(_) => Clearance::Asks,
        }
    }
}

/// Answers every search with one page, so no test reaches the network.
struct OnePageSearch;

#[async_trait]
impl SearchProvider for OnePageSearch {
    async fn search(&self, _: &SearchRequest) -> Option<Value> {
        Some(json!({
            "status": "ok",
            "tool": INTERNET_SEARCH_TOOL,
            "results": [{"title": "KEV", "url": "https://cisa.gov/kev", "content": "Feed"}],
        }))
    }
}

fn fetch(id: &str, url: &str) -> Vec<RawStreamingChoice<()>> {
    call(id, "web_request", json!({"method": "GET", "url": url}))
}

fn web_search(id: &str) -> Vec<RawStreamingChoice<()>> {
    call(id, INTERNET_SEARCH_TOOL, json!({"query": "kev feed"}))
}

async fn attempt(
    store: &Arc<Store>,
    client: &Client,
    host: &Arc<GateHost>,
    tools: &[&str],
) -> Result<String> {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let specs = tools
        .iter()
        .map(|tool| {
            find_global_tool_spec(tool)
                .or_else(|| crate::flow::copilot::find_bot_tool_spec(tool))
                .unwrap()
        })
        .collect();
    PlatformCopilot::new(state, None)
        .with_tool_specs(specs)
        .with_tool_rounds(20)
        .with_long_running()
        .with_web_access(WebAccess {
            search: Some(Arc::new(OnePageSearch)),
            reader: None,
        })
        .with_recovery(store.clone())
        .chat_with_client(
            PlatformSurface::Orchestrator,
            "Help the owner.".into(),
            "Read every feed once.".into(),
            None,
            Vec::new(),
            "scripted".into(),
            client,
            None,
            host.clone(),
            None,
            None::<fn(String)>,
        )
        .await
}

fn saved(store: &Store) -> BotRunCheckpoint {
    store.saved.lock().unwrap().clone().unwrap()
}

/// Confirms what the attempt's gate parked, as a host does after the run.
async fn confirm(store: &Store, host: &GateHost) -> BotRunCheckpoint {
    let confirmed = record_held_batch(&saved(store), &host.batch.held_calls())
        .unwrap()
        .unwrap();
    store.save(&confirmed).await.unwrap();
    confirmed
}

async fn answer_all(grants: &Grants, host: &GateHost, decision: ApprovalDecision) {
    let card = host.batch.card().unwrap();
    let answer = ApprovalAnswer {
        approval_id: card.id.clone(),
        decision,
        items: Vec::new(),
    };
    record_batch_answer(grants, &card, &answer).await.unwrap();
}

fn waits(result: Result<String>) -> bool {
    result
        .unwrap_err()
        .to_string()
        .contains("waiting for owner approval")
}

const FETCH_TOOLS: [&str; 1] = ["web_request"];

async fn twelve_parked() -> (Arc<Store>, Arc<Grants>, Arc<GateHost>) {
    let store = Arc::new(Store::default());
    let grants = Arc::new(Grants::default());
    let round = (1..=12)
        .flat_map(|n| {
            fetch(
                &format!("fetch-{n}"),
                &format!("https://site{n}.example/feed"),
            )
        })
        .collect();
    let host = GateHost::new(&grants);
    assert!(waits(
        attempt(&store, &Client::new(vec![round]), &host, &FETCH_TOOLS).await
    ));
    (store, grants, host)
}

#[tokio::test]
async fn twelve_asking_calls_in_one_lane_park_once_with_twelve_items() {
    let (store, _, host) = twelve_parked().await;
    assert!(host.ran().is_empty());
    let card = host.batch.card().unwrap();
    assert_eq!(card.items.len(), 12);
    assert_eq!(host.batch.held_calls().len(), 12);
    let parked = saved(&store);
    assert_eq!(parked.state["phase"], "tools_in_flight");
    assert!(!resumes_held_round(&parked));
    let confirmed = confirm(&store, &host).await;
    assert!(resumes_held_round(&confirmed));
    assert_eq!(confirmed.state["approval_resume"], true);
}

#[tokio::test]
async fn a_resumed_held_round_runs_the_answered_calls_without_a_model_round() {
    let (store, grants, host) = twelve_parked().await;
    confirm(&store, &host).await;
    answer_all(&grants, &host, ApprovalDecision::Once).await;

    let client = Client::new(vec![text("Twelve feeds read.")]);
    let resumed = GateHost::new(&grants);
    let answer = attempt(&store, &client, &resumed, &FETCH_TOOLS).await;
    assert_eq!(answer.unwrap(), "Twelve feeds read.");
    assert_eq!(resumed.ran().len(), 12);
    assert!(resumed.batch.card().is_none());
    let requests = client.0.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_complete_tool_pairs(&requests[0]);
    let restored = serde_json::to_string(&requests[0]).unwrap();
    assert_eq!(restored.matches("feed of https://site").count(), 12);
    assert!(!restored.contains("outcomes are unknown"));
    assert!(grants.list().await.unwrap().is_empty());
    assert_eq!(saved(&store).state["recoveries"], 0);
}

#[tokio::test]
async fn other_lanes_finish_before_the_park() {
    let store = Arc::new(Store::default());
    let grants = Arc::new(Grants::default());
    let round = [
        fetch("fetch-1", "https://a.example/feed"),
        call("inventory", "list_apps", json!({})),
        fetch("fetch-2", "https://b.example/feed"),
        fetch("fetch-3", "https://c.example/feed"),
    ]
    .concat();
    let host = GateHost::new(&grants);
    let tools = ["list_apps", "web_request"];
    assert!(waits(
        attempt(&store, &Client::new(vec![round]), &host, &tools).await
    ));
    assert_eq!(host.ran(), vec!["list_apps"]);
    assert_eq!(host.batch.card().unwrap().items.len(), 3);
    let parked = saved(&store);
    let calls = parked.state["pending_calls"].as_array().unwrap();
    assert!(
        calls[1]["output"]
            .as_str()
            .unwrap()
            .contains("slow sibling")
    );
    assert!(calls[0]["output"].is_null() && calls[0].get("held").is_none());
    assert_eq!(
        (calls[2]["held"].clone(), calls[3]["held"].clone()),
        (json!(true), json!(true))
    );

    confirm(&store, &host).await;
    answer_all(&grants, &host, ApprovalDecision::Once).await;
    let client = Client::new(vec![text("All three feeds are read.")]);
    let resumed = GateHost::new(&grants);
    let answer = attempt(&store, &client, &resumed, &tools).await;
    assert_eq!(answer.unwrap(), "All three feeds are read.");
    assert_eq!(
        resumed.ran(),
        vec![
            "https://a.example/feed",
            "https://b.example/feed",
            "https://c.example/feed"
        ]
    );
    let restored = serde_json::to_string(&client.0.requests.lock().unwrap()[0]).unwrap();
    assert!(restored.contains("slow sibling"));
    assert!(!restored.contains("outcomes are unknown"));
}

#[tokio::test]
async fn a_late_cost_approval_joins_the_batch() {
    let store = Arc::new(Store::default());
    let grants = Arc::new(Grants::default());
    let round = [
        call("extract", "file_extract", json!({"file": "report.pdf"})),
        fetch("fetch-1", "https://a.example/feed"),
    ]
    .concat();
    let host = GateHost::new(&grants);
    let tools = ["file_extract", "web_request"];
    assert!(waits(
        attempt(&store, &Client::new(vec![round]), &host, &tools).await
    ));
    assert_eq!(host.ran(), vec!["file_extract"]);
    let card = host.batch.card().unwrap();
    let tools_on_card: Vec<&str> = card.items.iter().map(|item| item.tool.as_str()).collect();
    assert_eq!(tools_on_card, vec!["file_extract", "web_request"]);
    assert_eq!(card.items[0].scope, "documents.expensive");
    let confirmed = confirm(&store, &host).await;
    assert!(resumes_held_round(&confirmed));
}

#[derive(Default)]
struct ClearanceHost(Mutex<Vec<(String, Value)>>);

#[async_trait]
impl PlatformToolBridge for ClearanceHost {
    async fn call(&self, _: &str, _: Value) -> String {
        unreachable!("clearance never runs a tool")
    }

    async fn clearance(&self, tool_name: &str, arguments: &Value) -> Clearance {
        self.0
            .lock()
            .unwrap()
            .push((tool_name.to_string(), arguments.clone()));
        Clearance::Asks
    }
}

#[derive(Debug)]
struct NoRoomUpdates;

#[async_trait]
impl RoomUpdates for NoRoomUpdates {
    async fn drain(&self) -> Vec<String> {
        Vec::new()
    }
}

#[tokio::test]
async fn clearance_reaches_the_host_through_every_wrapper() {
    use crate::flow::copilot::{
        bot_reply_files::ReplyFilesBridge,
        bot_run::BotRunContext,
        bot_trail::TrailBridge,
        tool_gate::{GatedBridge, tool_gate},
        turn_label::LabelBridge,
    };
    let host = Arc::new(ClearanceHost::default());
    let label = TurnLabel::new(&BotRunContext::general(), None);
    let (opener, gate) = tool_gate();
    let replies: Arc<dyn PlatformToolBridge> = Arc::new(ReplyFilesBridge::new(
        ReplyFiles::new(),
        None,
        label.clone(),
        None,
        host.clone(),
    ));
    let trail = Arc::new(TrailBridge::new(ToolTrail::new(None), replies));
    let gated = Arc::new(GatedBridge::new(gate, trail));
    let labelled = Arc::new(LabelBridge {
        label: label.clone(),
        inner: gated,
    });
    let root = RoomUpdatesBridge::new(labelled, Arc::new(NoRoomUpdates));
    opener.open();

    let args = json!({"url": "https://a.example/feed"});
    assert_eq!(root.clearance("web_request", &args).await, Clearance::Asks);
    assert_eq!(
        root.clearance("file_attach", &json!({"file": "a.pdf"}))
            .await,
        Clearance::Clear
    );
    assert_eq!(
        *host.0.lock().unwrap(),
        vec![("web_request".to_string(), args.clone())]
    );
    label.raise(flow_like_types::data_classification::DataSensitivity {
        classification: Some(
            flow_like_types::data_classification::DataClassification::Confidential,
        ),
        pii: false,
    });
    assert!(matches!(
        root.clearance("web_request", &args).await,
        Clearance::Refused(refusal) if refusal.contains("turn_label_outbound_refused")
    ));
    assert_eq!(host.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_research_turn_searches_and_reads_without_a_card() {
    let store = Arc::new(Store::default());
    let host = GateHost::new(&Arc::default());
    let client = Client::new(vec![
        web_search("search-1"),
        web_search("search-2"),
        text("Here is what the feeds say."),
    ]);
    let tools = [INTERNET_SEARCH_TOOL, "web_request"];
    let answer = attempt(&store, &client, &host, &tools).await;
    assert_eq!(answer.unwrap(), "Here is what the feeds say.");
    assert!(host.batch.card().is_none());
    assert_eq!(saved(&store).state["web_content"], true);
}

async fn granted_site(grants: &Grants, url: &str) {
    let scope = grant_scopes("web_request", &json!({"method": "GET", "url": url}), None)
        .unwrap()
        .remove(0);
    grants.put("web_request", &scope).await.unwrap();
}

#[tokio::test]
async fn a_granted_web_request_written_after_a_search_round_parks() {
    let store = Arc::new(Store::default());
    let grants = Arc::new(Grants::default());
    granted_site(&grants, "https://cisa.gov/kev").await;
    let host = GateHost::new(&grants);
    let client = Client::new(vec![
        web_search("search"),
        fetch("fetch", "https://cisa.gov/kev"),
    ]);
    let tools = [INTERNET_SEARCH_TOOL, "web_request"];
    assert!(waits(attempt(&store, &client, &host, &tools).await));
    assert!(host.ran().is_empty());
    let card = host.batch.card().unwrap();
    assert_eq!(
        (card.tool.as_str(), card.scope.as_str()),
        ("web_request", "")
    );
    assert_eq!(card.items.len(), 1);
    assert!(card.items[0].after_web_content);
    assert_eq!(saved(&store).state["web_content"], true);
}

#[tokio::test]
async fn a_granted_web_request_in_the_search_round_runs() {
    let store = Arc::new(Store::default());
    let grants = Arc::new(Grants::default());
    granted_site(&grants, "https://cisa.gov/kev").await;
    let host = GateHost::new(&grants);
    let client = Client::new(vec![
        [web_search("search"), fetch("fetch", "https://cisa.gov/kev")].concat(),
        text("Read it."),
    ]);
    let tools = [INTERNET_SEARCH_TOOL, "web_request"];
    assert_eq!(
        attempt(&store, &client, &host, &tools).await.unwrap(),
        "Read it."
    );
    assert_eq!(host.ran(), vec!["https://cisa.gov/kev"]);
    assert!(host.batch.card().is_none());
}

#[tokio::test]
async fn the_mark_survives_a_park_and_does_not_reask_the_held_round() {
    let store = Arc::new(Store::default());
    let grants = Arc::new(Grants::default());
    granted_site(&grants, "https://cisa.gov/kev").await;
    granted_site(&grants, "https://nvd.nist.gov/feed").await;
    let host = GateHost::new(&grants);
    let client = Client::new(vec![
        web_search("search"),
        [
            fetch("fetch-1", "https://cisa.gov/kev"),
            fetch("fetch-2", "https://nvd.nist.gov/feed"),
        ]
        .concat(),
    ]);
    let tools = [INTERNET_SEARCH_TOOL, "web_request"];
    assert!(waits(attempt(&store, &client, &host, &tools).await));
    let card = host.batch.card().unwrap();
    assert_eq!(card.items.len(), 2);
    assert!(card.items.iter().all(|item| item.after_web_content));
    assert_eq!(card.scope, "");
    let confirmed = confirm(&store, &host).await;
    assert_eq!(confirmed.state["web_content"], true);
    answer_all(&grants, &host, ApprovalDecision::Once).await;

    let client = Client::new(vec![text("Both feeds read.")]);
    let resumed = GateHost::new(&grants);
    assert_eq!(
        attempt(&store, &client, &resumed, &tools).await.unwrap(),
        "Both feeds read."
    );
    assert_eq!(
        resumed.ran(),
        vec!["https://cisa.gov/kev", "https://nvd.nist.gov/feed"]
    );
    assert!(resumed.batch.card().is_none());
    assert_eq!(client.0.requests.lock().unwrap().len(), 1);
    assert_eq!(saved(&store).state["web_content"], true);
}

#[tokio::test]
async fn durable_payload_strips_hidden_model_reasoning() {
    let store = Arc::new(Store::default());
    interrupted(store.clone(), true, 20).await;
    store.fail_after.store(0, Ordering::Relaxed);
    let (mut session, snapshot) =
        crate::flow::copilot::run_recovery::RecoverySession::load(store.clone())
            .await
            .unwrap();
    let mut snapshot = snapshot.unwrap();
    snapshot.history.insert(
        0,
        rig::message::Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::Reasoning(rig::message::Reasoning::new(
                "hidden-reasoning-canary",
            ))),
        },
    );
    session.save(snapshot).await.unwrap();
    assert!(
        !store
            .saved
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .state
            .to_string()
            .contains("hidden-reasoning-canary")
    );
}

#[test]
fn web_provenance_and_spending_survive_snapshot_restore() {
    let web = WebResearchSession::new("Read https://example.com/reference");
    assert!(web.reserve_search_call());
    assert!(web.reserve_archive_call());
    web.prepare_open_url_call(json!({"url":"https://example.com/reference", "max_chars":4000}))
        .unwrap();
    web.adopt_opened_urls(&["https://example.com/reference".into()]);
    web.close_public_web_phase();
    let snapshot = web.snapshot();
    let restored = WebResearchSession::restore(snapshot.clone()).unwrap();
    assert_eq!(restored.snapshot(), snapshot);
    assert!(restored.public_web_phase_error("open_url").is_some());
    assert_eq!(
        restored.opened_urls(),
        vec!["https://example.com/reference".to_string()]
    );
    let mut invalid = snapshot;
    invalid["search_calls"] = json!(usize::MAX);
    assert!(WebResearchSession::restore(invalid).is_err());
}

#[test]
fn only_a_gate_shaped_output_pauses_a_run() {
    use crate::flow::copilot::run_recovery::gate_waiting_code;
    let gate = json!({"status": "waiting", "code": "bot_tool_waiting_for_approval"}).to_string();
    assert_eq!(
        gate_waiting_code(&gate),
        Some("bot_tool_waiting_for_approval")
    );
    let tool_error =
        json!({"status": "error", "code": "bot_tool_waiting_for_approval"}).to_string();
    assert_eq!(gate_waiting_code(&tool_error), None);
}

#[tokio::test]
async fn the_deadline_keeps_the_text_streamed_so_far() {
    let store = Arc::new(Store::default());
    interrupted(store.clone(), true, 20).await;
    store.fail_after.store(0, Ordering::Relaxed);
    store
        .deadline
        .store(chrono::Utc::now().timestamp_millis() - 1, Ordering::Relaxed);
    let answer = RecoverySession::terminal_before_model(store.clone(), Some("Half an answer"))
        .await
        .unwrap()
        .unwrap();
    assert!(answer.output.starts_with("Half an answer"));
    assert!(answer.output.contains("cut off at the execution deadline"));
    assert_eq!(answer.stop_reason, Some(WorkStopReason::Budget));
}

/// A saved round whose `list_apps` call (`inspection`) ran but never recorded its result, plus
/// open `web_request` calls with these ids.
async fn round_in_flight(fetches: &[&str]) -> (Arc<Store>, BotRunCheckpoint) {
    let store = Arc::new(Store::default());
    interrupted(store.clone(), false, 20).await;
    store.fail_before.store(0, Ordering::Relaxed);
    let (mut session, snapshot) = RecoverySession::load(store.clone()).await.unwrap();
    let mut snapshot = snapshot.unwrap();
    for id in fetches {
        snapshot.pending_calls.push(PendingCall {
            id: (*id).into(),
            name: "web_request".into(),
            arguments: json!({"url": format!("https://example.com/{id}")}),
            output: None,
            held: false,
        });
    }
    session.save(snapshot).await.unwrap();
    let checkpoint = store.saved.lock().unwrap().clone().unwrap();
    (store, checkpoint)
}

fn parked(calls: &[(&str, &str)]) -> Vec<(String, String)> {
    calls
        .iter()
        .map(|(id, tool)| ((*id).into(), (*tool).into()))
        .collect()
}

fn stored(checkpoint: &BotRunCheckpoint) -> Arc<Store> {
    Arc::new(Store {
        saved: Mutex::new(Some(checkpoint.clone())),
        ..Default::default()
    })
}

#[tokio::test]
async fn held_calls_never_end_unknown() {
    let (store, _) = round_in_flight(&["fetch-2"]).await;
    let (mut session, _) = RecoverySession::load(store.clone()).await.unwrap();
    session
        .record_held(&parked(&[("fetch-2", "web_request")]))
        .await
        .unwrap();
    let checkpoint = store.saved.lock().unwrap().clone().unwrap();
    assert!(!resumes_held_round(&checkpoint));
    let mixed = RecoverySession::terminal_before_model(stored(&checkpoint), None)
        .await
        .unwrap()
        .unwrap();
    assert!(
        mixed
            .output
            .contains("in flight: list_apps (inspection). Their outcomes are unknown"),
        "{}",
        mixed.output
    );
    assert!(
        mixed.output.contains("did not run: web_request (fetch-2)."),
        "{}",
        mixed.output
    );

    let confirmed = record_held_batch(&checkpoint, &parked(&[("inspection", "list_apps")]))
        .unwrap()
        .unwrap();
    assert!(resumes_held_round(&confirmed));
    store.save(&confirmed).await.unwrap();
    assert!(
        RecoverySession::terminal_before_model(store.clone(), None)
            .await
            .unwrap()
            .is_none()
    );
    store
        .deadline
        .store(chrono::Utc::now().timestamp_millis() - 1, Ordering::Relaxed);
    let expired = RecoverySession::terminal_before_model(store.clone(), None)
        .await
        .unwrap()
        .unwrap();
    assert!(
        expired
            .output
            .contains("did not run: list_apps (inspection), web_request (fetch-2)."),
        "{}",
        expired.output
    );
    assert!(!expired.output.contains("unknown"), "{}", expired.output);
}

#[tokio::test]
async fn an_unconfirmed_waiting_call_stays_unknown() {
    let (store, checkpoint) = round_in_flight(&[]).await;
    for confirmation in [
        parked(&[]),
        parked(&[("inspection", "web_request")]),
        parked(&[("other", "list_apps")]),
    ] {
        assert!(
            record_held_batch(&checkpoint, &confirmation)
                .unwrap()
                .is_none()
        );
    }
    assert!(!resumes_held_round(&checkpoint));
    let answer = RecoverySession::terminal_before_model(store, None)
        .await
        .unwrap()
        .unwrap();
    assert!(answer.output.contains("outcomes are unknown"));
    assert!(!answer.output.contains("did not run"));
}

#[tokio::test]
async fn approval_resume_is_set_only_when_every_open_call_is_held() {
    let (_, checkpoint) = round_in_flight(&["fetch-1", "fetch-2"]).await;
    let partial = record_held_batch(&checkpoint, &parked(&[("fetch-1", "web_request")]))
        .unwrap()
        .unwrap();
    assert_eq!(partial.revision, checkpoint.revision + 1);
    assert_eq!(partial.state["approval_resume"], false);
    assert_eq!(partial.state["pending_calls"][1]["held"], true);
    assert!(partial.state["pending_calls"][2].get("held").is_none());
    assert!(!resumes_held_round(&partial));

    let rest = parked(&[("inspection", "list_apps"), ("fetch-2", "web_request")]);
    let all = record_held_batch(&partial, &rest).unwrap().unwrap();
    assert_eq!(all.state["approval_resume"], true);
    assert!(resumes_held_round(&all));
    assert!(record_held_batch(&all, &rest).unwrap().is_none());
}

#[tokio::test]
async fn a_snapshot_without_the_new_fields_loads() {
    let (_, checkpoint) = round_in_flight(&[]).await;
    let mut state = checkpoint.state.clone();
    assert!(state.get("web_content").is_none());
    assert!(state["pending_calls"][0].get("held").is_none());
    let snapshot: RunSnapshot = serde_json::from_value(state.clone()).unwrap();
    assert!(!snapshot.web_content && !snapshot.pending_calls[0].held);
    assert_eq!(finished_reply(&checkpoint).unwrap(), None);

    let mut marked = snapshot;
    marked.web_content = true;
    marked.pending_calls[0].held = true;
    let written = serde_json::to_value(&marked).unwrap();
    assert_eq!(written["web_content"], true);
    assert_eq!(written["pending_calls"][0]["held"], true);

    let mut finished = checkpoint.clone();
    finished.state["pending_calls"][0]["held"] = json!(true);
    finished.state["pending_calls"][0]["output"] = json!("{}");
    let error = finished_reply(&finished).unwrap_err().to_string();
    assert!(error.contains("although its result is recorded"), "{error}");

    state["pending_calls"][0]["surprise"] = json!(1);
    assert!(serde_json::from_value::<RunSnapshot>(state).is_err());
}
