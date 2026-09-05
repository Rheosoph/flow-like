use super::*;

fn common() -> Arc<CommonConfig> {
    Arc::new(CommonConfig {
        token: "secret".repeat(8),
        callback_url: "http://callback".into(),
        object_store_url: "http://objects".into(),
        allowed_https_hosts: vec!["example.com".into()],
        buckets: vec![
            "flow-meta".into(),
            "flow-content".into(),
            "flow-logs".into(),
        ],
        object_store_tls_gateway: false,
        backend_pub: "public-key".into(),
        capacity: 1,
        timeout: 3600,
        startup_grace: 120,
        terminal_grace: 60,
        cleanup_timeout: 1,
        warm_pool_size: 2,
        warm_create_concurrency: 2,
        warm_idle_seconds: 300,
        installation: "test".into(),
    })
}

async fn manager() -> Arc<Manager> {
    let common = common();
    let registry = Registry::open(":memory:".into(), "test".into())
        .await
        .unwrap();
    let config = DockerConfig {
        image: format!("sha256:{}", "a".repeat(64)),
        gateway_image: format!("sha256:{}", "b".repeat(64)),
        network: "private-gateway".into(),
        memory_mb: 2048,
        cpus: 1,
        pids: 256,
        tmp_mb: 512,
    };
    Arc::new_cyclic(|this| Manager {
        this: this.clone(),
        common,
        config,
        engine: Engine::new("/nonexistent/execution-test.sock".into()),
        registry,
        owner: "owner".into(),
        inventory: Mutex::new(Inventory::default()),
        changed: Notify::new(),
        ready: AtomicBool::new(true),
        draining: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        created: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        assignments: AtomicU64::new(0),
        assignment_nanos: AtomicU64::new(0),
    })
}

fn payload(run: &str) -> Dispatch {
    serde_json::from_value(json!({
        "job_id":"job", "run_id":run, "app_id":"app", "board_id":"board", "node_id":"node", "user_id":"user",
        "credentials":{}, "executor_jwt":"execution-only", "callback_url":"http://callback"
    })).unwrap()
}

async fn add_slot(manager: &Arc<Manager>, name: &str) -> Arc<Slot> {
    let record = Record {
        name: name.into(),
        gateway: format!("{name}-gateway"),
        volume: format!("{name}-socket"),
    };
    manager
        .registry
        .add(record.clone(), "owner".into(), now() + 5000.0)
        .await
        .unwrap();
    manager.registry.ready(record.name.clone()).await.unwrap();
    let slot = Arc::new(Slot {
        record,
        ready_at: Instant::now(),
        transport: AsyncMutex::new(None),
        closed: AtomicBool::new(false),
        cleanup: AsyncMutex::new(()),
    });
    manager
        .inventory
        .lock()
        .unwrap()
        .available
        .push_back(slot.clone());
    slot
}

#[tokio::test]
async fn runner_specification_has_no_host_network_secrets_or_writable_mounts() {
    let manager = manager().await;
    let row = Record {
        name: "slot".into(),
        gateway: "gateway".into(),
        volume: "socket".into(),
    };
    let runner = manager.specification(&row, 100.0, 100, true);
    assert_eq!(runner["HostConfig"]["Runtime"], "runsc");
    assert_eq!(runner["HostConfig"]["NetworkMode"], "none");
    assert_eq!(runner["HostConfig"]["ReadonlyRootfs"], true);
    assert_eq!(runner["HostConfig"]["CapDrop"], json!(["ALL"]));
    assert_eq!(
        runner["HostConfig"]["SecurityOpt"],
        json!(["no-new-privileges:true"])
    );
    assert_eq!(
        runner["HostConfig"]["Mounts"],
        json!([{"Type":"volume","Source":"socket","Target":"/gateway","ReadOnly":true}])
    );
    assert_eq!(
        runner["HostConfig"]["Memory"],
        runner["HostConfig"]["MemorySwap"]
    );
    assert!(
        runner["HostConfig"]["Tmpfs"]["/tmp"]
            .as_str()
            .unwrap()
            .contains("noexec,nosuid,nodev")
    );
    assert_eq!(runner["Labels"][OWNER_LABEL], "test");
    assert_eq!(runner["User"], "1000:1000");
    let env = runner["Env"].as_array().unwrap();
    assert!(!env.iter().filter_map(Value::as_str).any(|value| {
        value.starts_with("EXECUTION_MANAGER_TOKEN=")
            || value.starts_with("AWS_")
            || value.starts_with("BACKEND_KEY=")
    }));
    assert_eq!(runner["Cmd"][2], "/app/runtime");
    let gateway = manager.specification(&row, 100.0, 100, false);
    assert_eq!(gateway["Cmd"][2], "/app/execution-gateway");
    assert_eq!(gateway["Cmd"][3], "--unix-warm");
    assert_eq!(gateway["HostConfig"]["NetworkMode"], "private-gateway");
    assert_eq!(
        gateway["HostConfig"]["CapAdd"],
        json!(["CHOWN", "SETUID", "SETGID", "KILL"])
    );
}

#[tokio::test]
async fn warm_assignment_needs_no_docker_connection_and_capacity_is_bounded() {
    let manager = manager().await;
    let slot = add_slot(&manager, "one").await;
    add_slot(&manager, "two").await;
    // The configured Engine socket does not exist. Successful reservation
    // proves admission only touches the local inventory and durable registry.
    let reservation = manager.reserve(&payload("run")).await.unwrap();
    assert!(matches!(
        manager.reserve(&payload("other")).await,
        Err(Error::NoCapacity)
    ));
    assert_eq!(manager.inventory.lock().unwrap().available.len(), 1);
    assert_eq!(
        manager
            .registry
            .cancel("run".into(), now() + 100.0)
            .await
            .unwrap()[0]
            .name,
        "one"
    );
    slot.closed.store(true, Ordering::Release);
    drop(reservation);
}

#[tokio::test]
async fn an_empty_warm_pool_does_not_attempt_cold_creation() {
    let manager = manager().await;
    assert!(matches!(
        manager.reserve(&payload("run")).await,
        Err(Error::NoCapacity)
    ));
    assert!(manager.ready());
    assert_eq!(manager.inventory.lock().unwrap().creating, 0);
}

#[tokio::test]
async fn retiring_slots_hold_the_warm_resource_budget_until_cleanup_finishes() {
    let manager = manager().await;
    manager.inventory.lock().unwrap().retiring = manager.common.warm_pool_size;
    let replenish = tokio::spawn(manager.clone().replenish());
    tokio::time::sleep(Duration::from_millis(20)).await;
    // A creation attempt would hit the deliberately missing Docker socket and
    // close admission. Retiring slots must continue to occupy the warm reserve.
    assert!(manager.ready());
    assert_eq!(manager.errors.load(Ordering::Relaxed), 0);
    assert_eq!(manager.inventory.lock().unwrap().creating, 0);
    manager.fail_closed();
    replenish.await.unwrap();
}

#[tokio::test]
async fn failed_cleanup_closes_admission_and_retains_reconciliation_record() {
    let manager = manager().await;
    let slot = add_slot(&manager, "one").await;
    assert!(manager.discard(&slot).await.is_err());
    assert!(!manager.ready());
    assert!(!slot.closed.load(Ordering::Acquire));
    let expired = manager.registry.expired("owner".into()).await.unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].name, "one");
}

#[tokio::test]
async fn cancellation_does_not_acknowledge_unconfirmed_removal() {
    let manager = manager().await;
    let slot = add_slot(&manager, "one").await;
    let reservation = manager.reserve(&payload("run")).await.unwrap();
    assert!(manager.cancel("run").await.is_err());
    assert!(!manager.ready());
    slot.closed.store(true, Ordering::Release);
    drop(reservation);
}

#[tokio::test]
async fn cancelling_before_assignment_cannot_admit_the_run() {
    let manager = manager().await;
    let slot = add_slot(&manager, "one").await;
    manager.cancel("run").await.unwrap();
    // Skip Engine cleanup in this admission-only test; the cancellation fence
    // must have been recorded before the rejected assignment.
    slot.closed.store(true, Ordering::Release);
    assert!(matches!(
        manager.reserve(&payload("run")).await,
        Err(Error::Cancelled)
    ));
}
