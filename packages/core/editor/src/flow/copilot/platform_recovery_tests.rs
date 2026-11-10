use super::*;
use crate::flow::copilot::run_recovery::{
    BotRunRecovery, PendingCall, RunSnapshot, current_recovery_call_id, finished_reply,
    record_unexecuted_approval,
};
use flow_like_types::bot_turn::BotRunCheckpoint;
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
async fn approval_reconciliation_keeps_completed_siblings_and_unknown_calls_distinct() {
    let store = Arc::new(Store::default());
    interrupted(store.clone(), false, 20).await;
    store.fail_before.store(0, Ordering::Relaxed);
    let mut checkpoint = store.saved.lock().unwrap().clone().unwrap();
    let mut snapshot: RunSnapshot = serde_json::from_value(checkpoint.state.clone()).unwrap();
    snapshot.pending_calls[0].output =
        Some(json!({"status":"ok", "observed":"existing apps"}).to_string());
    snapshot.pending_calls.push(PendingCall {
        id: "approval-2".into(),
        name: "shell_run".into(),
        arguments: json!({"command":"touch report.txt"}),
        output: None,
    });
    checkpoint.state = serde_json::to_value(snapshot).unwrap();
    assert!(
        record_unexecuted_approval(&checkpoint, "wrong", "shell_run")
            .unwrap()
            .is_none()
    );
    let reconciled = record_unexecuted_approval(&checkpoint, "approval-2", "shell_run")
        .unwrap()
        .unwrap();
    assert_eq!(reconciled.revision, checkpoint.revision + 1);
    assert_eq!(reconciled.state["phase"], "ready");
    let saved: RunSnapshot = serde_json::from_value(reconciled.state).unwrap();
    assert_eq!(saved.iteration, 1);
    let transcript = serde_json::to_string(&saved.prompt).unwrap();
    assert!(transcript.contains("existing apps"));
    assert!(transcript.contains("executed"));
    assert!(transcript.contains("bot_tool_waiting_for_approval"));

    let mut snapshot: RunSnapshot = serde_json::from_value(checkpoint.state.clone()).unwrap();
    snapshot.pending_calls.push(PendingCall {
        id: "unknown-3".into(),
        name: "computer_click".into(),
        arguments: json!({"x":10,"y":10}),
        output: None,
    });
    checkpoint.state = serde_json::to_value(snapshot).unwrap();
    let reconciled = record_unexecuted_approval(&checkpoint, "approval-2", "shell_run")
        .unwrap()
        .unwrap();
    assert_eq!(reconciled.state["phase"], "tools_in_flight");
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
    assert!(current_recovery_call_id().is_none());
    crate::flow::copilot::run_recovery::scope_call("parent".into(), async {
        assert_eq!(current_recovery_call_id().as_deref(), Some("parent"));
        crate::flow::copilot::run_recovery::scope_call("child".into(), async {
            assert_eq!(current_recovery_call_id().as_deref(), Some("child"));
        })
        .await;
        assert_eq!(current_recovery_call_id().as_deref(), Some("parent"));
    })
    .await;
    assert!(current_recovery_call_id().is_none());
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
    for index in 0..5 {
        let client = Client::new(vec![call(
            &format!("approval-{index}"),
            "list_apps",
            json!({}),
        )]);
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
        assert_eq!(checkpoint.state["phase"], "ready");
        assert_eq!(checkpoint.state["recoveries"], 0);
        assert_eq!(checkpoint.state["model_calls_reserved"], index + 1);
        let call_id = bridge.0.lock().unwrap().clone().unwrap();
        let next = record_unexecuted_approval(&checkpoint, &call_id, "list_apps")
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
struct FirstSiteParksBridge {
    started: Mutex<Vec<String>>,
}

#[async_trait]
impl PlatformToolBridge for FirstSiteParksBridge {
    async fn call(&self, name: &str, arguments: Value) -> String {
        if name == "list_apps" {
            tokio::time::sleep(Duration::from_millis(50)).await;
            self.started.lock().unwrap().push(name.into());
            return json!({"status": "ok", "apps": [], "observation": "slow sibling"}).to_string();
        }
        let url = arguments["url"].as_str().unwrap_or_default().to_string();
        self.started.lock().unwrap().push(url);
        json!({"status": "waiting", "code": "bot_tool_waiting_for_approval", "message": "Call no further tools and write nothing more now."}).to_string()
    }
}

#[tokio::test]
async fn a_parked_call_closes_its_batch_so_the_approved_turn_resumes() {
    let store = Arc::new(Store::default());
    let fetch = |id: &str, url: &str| call(id, "web_request", json!({"method": "GET", "url": url}));
    let client = Client::new(vec![
        [
            fetch("fetch-1", "https://a.example/feed"),
            call("inventory", "list_apps", json!({})),
            fetch("fetch-2", "https://b.example/feed"),
            fetch("fetch-3", "https://c.example/feed"),
        ]
        .concat(),
    ]);
    let bridge = Arc::new(FirstSiteParksBridge::default());
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let result = PlatformCopilot::new(state, None)
        .with_tool_specs(vec![
            find_global_tool_spec("list_apps").unwrap(),
            crate::flow::copilot::find_bot_tool_spec("web_request").unwrap(),
        ])
        .with_tool_rounds(20)
        .with_long_running()
        .with_recovery(store.clone())
        .chat_with_client(
            PlatformSurface::Orchestrator,
            "Help the owner.".into(),
            "Fetch every feed once.".into(),
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("waiting for owner approval")
    );
    let mut started = bridge.started.lock().unwrap().clone();
    started.sort();
    assert_eq!(started, vec!["https://a.example/feed", "list_apps"]);

    let checkpoint = store.saved.lock().unwrap().clone().unwrap();
    assert_eq!(checkpoint.state["phase"], "ready");
    let results = serde_json::to_string(&checkpoint.state["prompt"]).unwrap();
    assert!(results.contains("slow sibling"), "{results}");
    assert_eq!(
        results.matches("bot_tool_not_started").count(),
        2,
        "{results}"
    );
    assert!(
        results.contains("bot_tool_waiting_for_approval"),
        "{results}"
    );
    assert!(!results.contains("Call no further tools"), "{results}");

    let next = record_unexecuted_approval(&checkpoint, "fetch-1", "web_request")
        .unwrap()
        .unwrap();
    assert_eq!(next.state["approval_resume"], true);
    store.save(&next).await.unwrap();

    let client = Client::new(vec![text("All three feeds are queued again.")]);
    let (answer, bridge) = run_saved(
        &client,
        store.clone(),
        20,
        "Fetch every feed once.\nThe owner approved web_request.",
        None,
    )
    .await;
    assert_eq!(answer.unwrap(), "All three feeds are queued again.");
    assert_eq!(bridge.calls.load(Ordering::Relaxed), 0);
    let requests = client.0.requests.lock().unwrap();
    assert_complete_tool_pairs(&requests[0]);
    let restored = serde_json::to_string(&requests[0]).unwrap();
    assert!(restored.contains("bot_tool_not_started"));
    assert!(!restored.contains("outcomes are unknown"));
    assert_eq!(
        store.saved.lock().unwrap().as_ref().unwrap().state["recoveries"],
        0
    );
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
