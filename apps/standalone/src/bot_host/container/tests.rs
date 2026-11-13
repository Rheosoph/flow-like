use super::*;
use flow_like_types::bot_turn::{BotTurn, TurnAudience, TurnHostKind, TurnStatus};

#[derive(Default)]
struct FakeApi {
    calls: Mutex<Vec<(Method, String, Option<Value>)>>,
    batched: bool,
    exhausted: AtomicBool,
    claim_gate: Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
    poll_status: std::sync::atomic::AtomicU16,
    held_status: std::sync::atomic::AtomicU16,
}

fn turn(id: &str, bot: &str) -> BotTurn {
    BotTurn {
        id: id.into(),
        bot_id: bot.into(),
        conversation_id: "conversation".into(),
        trigger_message_id: "message".into(),
        host_kind: TurnHostKind::Device,
        status: TurnStatus::Running,
        attempts: 1,
        host_id: Some("device:device".into()),
        lease_until: None,
        reply_message_id: None,
        error_code: None,
        hops: 0,
        owner_id: "owner".into(),
        requester_id: None,
        requester_name: None,
        audience: TurnAudience::Owner,
        created_at: "2026-11-13T00:00:00Z".into(),
        updated_at: "2026-11-13T00:00:00Z".into(),
        batched: Vec::new(),
    }
}

#[async_trait]
impl DeviceApi for FakeApi {
    fn base_url(&self) -> &str {
        "https://hub.test/api/v1"
    }
    fn device_id(&self) -> &str {
        "device"
    }
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Vec<u8>)> {
        lock(&self.calls).push((method, path.into(), body.cloned()));
        if path.ends_with("/poll") {
            let status = self.poll_status.load(Ordering::Acquire);
            if status != 0 {
                return Ok((status, serde_json::to_vec(&json!({"error":"signed_out"}))?));
            }
        }
        if path.ends_with("/heartbeat") || path.contains("/checkpoint") {
            let status = self.held_status.load(Ordering::Acquire);
            if status != 0 {
                return Ok((
                    status,
                    serde_json::to_vec(
                        &json!({"error":{"code":if status == 409 { "BOT_TURN_LEASE_LOST" } else { "TEMPORARY" }}}),
                    )?,
                ));
            }
        }
        let gate = lock(&self.claim_gate).clone();
        if path.ends_with("/claim")
            && let Some((started, finish)) = gate
        {
            started.notify_one();
            finish.notified().await;
        }
        let value = if path.ends_with("/claim") {
            json!({"turn":turn(if self.batched { "batched" } else { "turn" }, "bot"),"host_id":"device:device","conversation":{"id":"conversation","kind":"direct"},"bot":{"bot_id":"bot","name":"Bot","execution_mode":"docker"},"models":{"model_id":"model","completion":{}},"bot_token":{"token":"only-this-bot","expires_at":9999999999i64},"deadline":9999999999i64})
        } else if path.ends_with("/release") {
            json!({"requeued":!self.exhausted.load(Ordering::Acquire)})
        } else {
            json!({"status":"parked","checkpoint":null})
        };
        Ok((200, serde_json::to_vec(&value)?))
    }
}

fn fixture(
    batched: bool,
) -> (
    Arc<Coordinator>,
    Arc<FakeApi>,
    tempfile::TempDir,
    mpsc::Receiver<Frame>,
) {
    let api = Arc::new(FakeApi {
        batched,
        ..Default::default()
    });
    let state = tempfile::tempdir().unwrap();
    let host = Coordinator::new(state.path().into(), api.clone(), CancellationToken::new());
    lock(&host.bots).insert(
        "bot".into(),
        DeviceBotInfo {
            id: "bot".into(),
            name: Some("Bot".into()),
            execution_mode: BotExecutionMode::Docker,
            capabilities: Vec::new(),
            updated_at: "now".into(),
        },
    );
    lock(&host.offers).insert(
        "turn".into(),
        Offer {
            bot: "bot".into(),
            conversation: "conversation".into(),
        },
    );
    lock(&host.offers).insert(
        "other-turn".into(),
        Offer {
            bot: "other".into(),
            conversation: "other-conversation".into(),
        },
    );
    let (tx, rx) = mpsc::channel(8);
    lock(&host.workers).insert(
        "bot".into(),
        Worker {
            generation: "worker".into(),
            tx,
            views: Arc::default(),
            cancel: CancellationToken::new(),
            stopped: CancellationToken::new(),
            ready: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
        },
    );
    (host, api, state, rx)
}

fn route(tail: &str) -> String {
    format!("/devices/device/bots/{tail}")
}
fn body() -> Value {
    json!({"host_id":"device:device"})
}

#[test]
fn screen_routing_requires_an_explicit_live_bot_assignment() {
    let (host, _api, _state, _rx) = fixture(false);
    let mut bot = lock(&host.bots).remove("bot").unwrap();
    assert_eq!(host.cached_execution_mode("bot"), None);
    assert_eq!(host.cached_execution_mode("unknown"), None);

    lock(&host.bots).insert(bot.id.clone(), bot.clone());
    assert_eq!(
        host.cached_execution_mode("bot"),
        Some(BotExecutionMode::Docker)
    );
    bot.execution_mode = BotExecutionMode::Native;
    lock(&host.bots).insert(bot.id.clone(), bot.clone());
    assert_eq!(
        host.cached_execution_mode("bot"),
        Some(BotExecutionMode::Native)
    );

    lock(&host.bots).clear();
    assert_eq!(host.cached_execution_mode("bot"), None);
    lock(&host.bots).insert(bot.id.clone(), bot);
    host.cancel.cancel();
    assert_eq!(host.cached_execution_mode("bot"), None);
}

#[tokio::test(start_paused = true)]
async fn a_mode_flip_waits_for_the_previous_container_to_finish_cleanup() {
    let (host, _api, _state, _rx) = fixture(false);
    *host.probe.lock().await = Some((
        Instant::now(),
        DeviceDockerSupport {
            installed: true,
            available: true,
            reason: None,
        },
    ));
    let original = lock(&host.workers).get("bot").unwrap().clone();
    let docker_bot = lock(&host.bots).get("bot").unwrap().clone();
    let mut native_bot = docker_bot.clone();
    native_bot.execution_mode = BotExecutionMode::Native;
    let native = DevicePollResponse {
        bots: vec![native_bot],
        turns: Vec::new(),
        next_poll_ms: 500,
    };

    host.reconcile(&native).await;
    assert!(original.cancel.is_cancelled());
    assert!(!original.stopped.is_cancelled());
    assert_eq!(
        lock(&host.workers).get("bot").unwrap().generation,
        original.generation
    );

    host.reconcile(&DevicePollResponse {
        bots: vec![docker_bot],
        turns: Vec::new(),
        next_poll_ms: 500,
    })
    .await;
    assert_eq!(
        lock(&host.workers).get("bot").unwrap().generation,
        original.generation
    );

    original.stopped.cancel();
    host.reconcile(&native).await;
    assert!(!lock(&host.workers).contains_key("bot"));
}

#[tokio::test]
async fn an_untrusted_child_cannot_sign_for_another_bot_or_device() {
    let (host, api, _state, _rx) = fixture(false);
    for path in [
        route("turns/other-turn/claim"),
        route("turns/not-offered/claim"),
        "/devices/device/reboot".into(),
        "/devices/other/bots/poll".into(),
    ] {
        assert!(
            host.scoped_call("bot", "worker", Method::POST, &path, Some(&body()))
                .await
                .is_err(),
            "{path}"
        );
    }
    assert!(
        host.scoped_call(
            "bot",
            "old-worker",
            Method::POST,
            &route("turns/turn/claim"),
            Some(&body())
        )
        .await
        .is_err()
    );
    assert!(
        host.scoped_call(
            "bot",
            "worker",
            Method::POST,
            &route("inbox-keys"),
            Some(&json!({"principal_id":"other","key_id":"key"}))
        )
        .await
        .is_err()
    );
    assert!(lock(&api.calls).is_empty());
}

#[tokio::test]
async fn lifecycle_requests_are_bound_to_the_claim_and_attempt() {
    let (host, api, _state, _rx) = fixture(false);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/claim"),
        Some(&body()),
    )
    .await
    .unwrap();
    assert_eq!(
        lock(&api.calls)[0].2.as_ref().unwrap()["execution_modes"],
        json!(["docker"])
    );
    assert!(
        host.scoped_call(
            "bot",
            "worker",
            Method::POST,
            &route("turns/other-turn/heartbeat"),
            Some(&body())
        )
        .await
        .is_err()
    );
    assert!(
        host.scoped_call(
            "bot",
            "worker",
            Method::GET,
            &route("turns/turn/checkpoint?host_id=device%3Adevice&attempt=2"),
            None
        )
        .await
        .is_err()
    );
    host.scoped_call(
        "bot",
        "worker",
        Method::GET,
        &route("turns/turn/checkpoint?host_id=device%3Adevice&attempt=1"),
        None,
    )
    .await
    .unwrap();
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/park"),
        Some(&body()),
    )
    .await
    .unwrap();
    assert!(
        host.scoped_call(
            "bot",
            "worker",
            Method::POST,
            &route("turns/turn/heartbeat"),
            Some(&body())
        )
        .await
        .is_err()
    );
    assert_eq!(
        host.slots.available_permits(),
        super::super::CONCURRENT_TURNS
    );
}

#[tokio::test]
async fn a_batched_claim_tracks_the_actual_turn() {
    let (host, _api, _state, _rx) = fixture(true);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/claim"),
        Some(&body()),
    )
    .await
    .unwrap();
    assert!(
        host.scoped_call(
            "bot",
            "worker",
            Method::POST,
            &route("turns/turn/heartbeat"),
            Some(&body())
        )
        .await
        .is_err()
    );
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/batched/heartbeat"),
        Some(&body()),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_child_poll_sees_only_its_bot_and_cannot_remove_other_hosts() {
    let (host, api, _state, _rx) = fixture(false);
    let mut other = lock(&host.bots).get("bot").unwrap().clone();
    other.id = "other".into();
    *host.poll.lock().await = Some(Snapshot {
        at: Instant::now(),
        response: DevicePollResponse {
            bots: vec![lock(&host.bots).get("bot").unwrap().clone(), other],
            turns: vec![turn("turn", "bot"), turn("other-turn", "other")],
            next_poll_ms: 2_000,
        },
    });
    let (_, data) = host
        .scoped_call("bot", "worker", Method::POST, &route("poll"), Some(&body()))
        .await
        .unwrap();
    let response: DevicePollResponse = serde_json::from_slice(&data).unwrap();
    assert_eq!(response.bots.len(), 1);
    assert_eq!(response.turns.len(), 1);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("hosts/forget"),
        Some(&body()),
    )
    .await
    .unwrap();
    assert!(lock(&api.calls).is_empty());
}

#[tokio::test]
async fn disconnect_releases_only_that_workers_claims() {
    let (host, api, _state, _rx) = fixture(false);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/claim"),
        Some(&body()),
    )
    .await
    .unwrap();
    host.release_worker("bot", "old-worker").await;
    assert_eq!(host.leases.lock().await.len(), 1);
    host.release_worker("bot", "worker").await;
    assert!(host.leases.lock().await.is_empty());
    let calls = lock(&api.calls);
    let release = calls.last().unwrap();
    assert!(release.1.ends_with("turns/turn/release"));
    assert_eq!(release.2.as_ref().unwrap()["retry"], true);
}

#[tokio::test]
async fn a_container_stopping_during_claim_releases_the_new_lease() {
    let (host, api, _state, _rx) = fixture(false);
    let started = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    *lock(&api.claim_gate) = Some((started.clone(), finish.clone()));
    let caller = host.clone();
    let claim = tokio::spawn(async move {
        caller
            .scoped_call(
                "bot",
                "worker",
                Method::POST,
                &route("turns/turn/claim"),
                Some(&body()),
            )
            .await
    });
    started.notified().await;
    lock(&host.workers).get("bot").unwrap().cancel.cancel();
    finish.notify_one();
    assert!(claim.await.unwrap().is_err());
    assert!(host.leases.lock().await.is_empty());
    assert!(lock(&api.calls).last().unwrap().1.ends_with("/release"));
    assert_eq!(
        host.slots.available_permits(),
        super::super::CONCURRENT_TURNS
    );
}

#[tokio::test]
async fn exhausted_retries_still_allow_the_final_failure_reply() {
    let (host, api, _state, _rx) = fixture(false);
    api.exhausted.store(true, Ordering::Release);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/claim"),
        Some(&body()),
    )
    .await
    .unwrap();
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/release"),
        Some(&json!({"host_id":"device:device","retry":true})),
    )
    .await
    .unwrap();
    assert_eq!(host.leases.lock().await.len(), 1);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/reply"),
        Some(&body()),
    )
    .await
    .unwrap();
    assert!(host.leases.lock().await.is_empty());
}

#[tokio::test]
async fn dropping_a_view_closes_the_child_stream() {
    let (host, _api, _state, mut rx) = fixture(false);
    let worker = lock(&host.workers).get("bot").unwrap().clone();
    let (sender, _receiver) = mpsc::channel(1);
    lock(&worker.views).insert(9, sender);
    drop(ViewGuard {
        worker: worker.clone(),
        id: 9,
    });
    assert!(lock(&worker.views).is_empty());
    assert!(matches!(rx.recv().await, Some(Frame::CloseView { id: 9 })));
}

#[tokio::test]
async fn a_fake_container_pipe_cannot_escape_the_broker_scope() {
    let (host, api, _state, mut replies) = fixture(false);
    let worker = lock(&host.workers).get("bot").unwrap().clone();
    let (mut child, mut parent) = tokio::io::duplex(wire::PIPE_BYTES);
    let serving = host.clone();
    let task = tokio::spawn(async move { serving.read_worker("bot", &worker, &mut parent).await });
    wire::write(
        &mut child,
        &Frame::Call {
            id: 4,
            method: "POST".into(),
            path: route("turns/other-turn/claim"),
            body: Some(body()),
        },
    )
    .await
    .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(1), replies.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        reply,
        Frame::Reply {
            id: 4,
            status: 403,
            ..
        }
    ));
    assert!(lock(&api.calls).is_empty());
    child.shutdown().await.unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn losing_device_authorization_stops_containers_and_preserves_the_status() {
    let (host, api, _state, _rx) = fixture(false);
    api.poll_status.store(401, Ordering::Release);
    *host.probe.lock().await = Some((
        Instant::now(),
        DeviceDockerSupport {
            installed: true,
            available: true,
            reason: None,
        },
    ));
    let result = host
        .native_api()
        .call(Method::POST, &route("poll"), Some(&body()))
        .await
        .unwrap();
    assert_eq!(result.0, 401);
    assert!(
        lock(&host.workers)
            .get("bot")
            .unwrap()
            .cancel
            .is_cancelled()
    );
}

#[tokio::test]
async fn authoritative_lease_loss_releases_capacity_but_transient_errors_do_not() {
    let (host, api, _state, _rx) = fixture(false);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/claim"),
        Some(&body()),
    )
    .await
    .unwrap();
    api.held_status.store(503, Ordering::Release);
    let response = host
        .scoped_call(
            "bot",
            "worker",
            Method::POST,
            &route("turns/turn/heartbeat"),
            Some(&body()),
        )
        .await
        .unwrap();
    assert_eq!(response.0, 503);
    assert_eq!(host.leases.lock().await.len(), 1);
    api.held_status.store(409, Ordering::Release);
    let response = host
        .scoped_call(
            "bot",
            "worker",
            Method::GET,
            &route("turns/turn/checkpoint?host_id=device%3Adevice&attempt=1"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(response.0, 409);
    assert!(host.leases.lock().await.is_empty());
    assert_eq!(
        host.slots.available_permits(),
        super::super::CONCURRENT_TURNS
    );
}

#[tokio::test]
async fn revoked_device_credentials_on_heartbeat_stop_the_container() {
    let (host, api, _state, _rx) = fixture(false);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/claim"),
        Some(&body()),
    )
    .await
    .unwrap();
    api.held_status.store(403, Ordering::Release);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/heartbeat"),
        Some(&body()),
    )
    .await
    .unwrap();
    assert!(host.leases.lock().await.is_empty());
    assert!(
        lock(&host.workers)
            .get("bot")
            .unwrap()
            .cancel
            .is_cancelled()
    );
}

#[tokio::test]
async fn a_lapsed_disconnected_turn_cannot_hold_device_capacity_forever() {
    let (host, _api, _state, _rx) = fixture(false);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/claim"),
        Some(&body()),
    )
    .await
    .unwrap();
    host.leases.lock().await.get_mut("turn").unwrap().renewed =
        Instant::now() - Duration::from_secs(flow_like_types::bot_turn::LEASE_SECS as u64 + 6);
    host.scoped_call(
        "bot",
        "worker",
        Method::POST,
        &route("turns/turn/claim"),
        Some(&body()),
    )
    .await
    .unwrap();
    assert_eq!(host.leases.lock().await.len(), 1);
    assert_eq!(
        host.slots.available_permits(),
        super::super::CONCURRENT_TURNS - 1
    );
}

#[test]
fn containers_receive_only_their_named_volume_and_bounded_resources() {
    let name = resource_name("device", "bot");
    let args = run_args(
        &name,
        "ghcr.io/flow/agent@sha256:abc",
        Path::new("/private/profile.json"),
        "device",
        "bot",
    );
    assert_eq!(args[0], "create");
    assert!(args.contains(&"--log-driver=none".into()));
    assert!(args.contains(&"--init".into()));
    assert!(args.contains(&"--cap-drop=ALL".into()));
    assert!(args.contains(&"--security-opt=no-new-privileges".into()));
    assert!(args.contains(&"--read-only".into()));
    assert!(args.contains(&"--memory=4g".into()));
    assert!(args.contains(&"--pids-limit=512".into()));
    assert!(args.contains(&"--env=FLOW_LIKE_PLAYGROUND=virtual".into()));
    assert_eq!(
        args.iter().filter(|arg| arg.as_str() == "--mount").count(),
        1
    );
    assert!(
        args.iter()
            .any(|arg| arg == &format!("type=volume,source={name}-data,target=/var/lib/flow-like"))
    );
    assert!(!args.iter().any(|arg| arg.contains("docker.sock")
        || arg.contains("type=bind")
        || arg.contains("privileged")
        || arg.contains("network=host")));
    assert_eq!(args.last().unwrap(), "bot-container-worker");
    assert_ne!(
        resource_name("device", "bot"),
        resource_name("device", "other")
    );
    assert!(!resource_name("../device", "../../bot").contains('/'));
}
