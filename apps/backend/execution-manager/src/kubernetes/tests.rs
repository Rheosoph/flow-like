use super::*;

type KubeEvent = (Method, String, String, Option<Value>);

#[derive(Default)]
struct FakeKube {
    objects: Mutex<HashMap<(String, String), Value>>,
    events: Mutex<Vec<KubeEvent>>,
    refuse_termination: AtomicBool,
    fail_runner_create: AtomicBool,
}

#[async_trait]
impl KubeApi for FakeKube {
    async fn request(
        &self,
        method: Method,
        kind: &str,
        name: Option<&str>,
        body: Option<Value>,
        query: &[(&str, String)],
    ) -> transport::TransportResult<Value> {
        self.events.lock().unwrap().push((
            method.clone(),
            kind.into(),
            name.unwrap_or_default().into(),
            body.clone(),
        ));
        let mut objects = self.objects.lock().unwrap();
        if method == Method::GET {
            if let Some(name) = name {
                return objects
                    .get(&(kind.into(), name.into()))
                    .cloned()
                    .ok_or(TransportError::Status(404));
            }
            let selector = query
                .iter()
                .find(|(key, _)| *key == "labelSelector")
                .map(|(_, value)| value.as_str())
                .unwrap_or_default();
            let values = objects
                .iter()
                .filter(|((stored, _), value)| {
                    stored == kind
                        && selector.split(',').all(|part| {
                            if let Some((key, expected)) = part.split_once('=') {
                                value["metadata"]["labels"][key] == expected
                            } else {
                                value["metadata"]["labels"].get(part).is_some()
                            }
                        })
                })
                .map(|(_, value)| value.clone())
                .collect::<Vec<_>>();
            return Ok(json!({"items":values,"metadata":{}}));
        }
        if method == Method::POST {
            let mut value = body.unwrap();
            let name = value["metadata"]["name"].as_str().unwrap().to_owned();
            if self.fail_runner_create.load(Ordering::Relaxed)
                && kind == "pods"
                && name.ends_with("-runner")
            {
                return Err(TransportError::Status(500));
            }
            if objects.contains_key(&(kind.into(), name.clone())) {
                return Err(TransportError::Status(409));
            }
            value["metadata"]["uid"] = format!("{name}-uid").into();
            if kind == "pods" && value.get("status").is_none() {
                value["status"] = json!({"phase":"Running","podIP":"10.0.0.2","startTime":"2026-01-01T00:00:00Z","conditions":[{"type":"Ready","status":"True"}]});
            }
            objects.insert((kind.into(), name), value.clone());
            return Ok(value);
        }
        let key = (kind.into(), name.unwrap().into());
        if method == Method::DELETE {
            objects.remove(&key);
            return Ok(json!({}));
        }
        if method == Method::PATCH {
            let value = objects.get_mut(&key).ok_or(TransportError::Status(404))?;
            let body = body.unwrap();
            if let Some(metadata) = body["metadata"].as_object() {
                for (key, data) in metadata {
                    if !value["metadata"][key].is_object() {
                        value["metadata"][key] = json!({});
                    }
                    value["metadata"][key]
                        .as_object_mut()
                        .unwrap()
                        .extend(data.as_object().unwrap().clone());
                }
            }
            if body.get("spec").is_some()
                && body.get("metadata").is_none()
                && !self.refuse_termination.load(Ordering::Relaxed)
            {
                value["status"]["phase"] = "Failed".into();
            }
            return Ok(value.clone());
        }
        panic!("unexpected request");
    }
}

#[derive(Default)]
struct FakeClaims {
    values: Mutex<HashMap<String, String>>,
    ambiguous: AtomicBool,
}
#[async_trait]
impl ClaimStore for FakeClaims {
    async fn ping(&self) -> transport::TransportResult<()> {
        Ok(())
    }
    async fn claim(&self, run_id: &str, slot_id: &str) -> transport::TransportResult<bool> {
        let mut values = self.values.lock().unwrap();
        let added = if values.contains_key(run_id) {
            false
        } else {
            values.insert(run_id.into(), slot_id.into());
            true
        };
        if self.ambiguous.load(Ordering::Relaxed) {
            Err(TransportError::Unavailable)
        } else {
            Ok(added)
        }
    }
}

fn config() -> Config {
    Config {
        common: Arc::new(CommonConfig {
            token: "t".repeat(64),
            callback_url: "http://api:8080".into(),
            object_store_url: "http://objects:9000".into(),
            allowed_https_hosts: vec![],
            buckets: vec![
                "flow-like-meta".into(),
                "flow-like-content".into(),
                "flow-like-logs".into(),
            ],
            object_store_tls_gateway: false,
            backend_pub: "public-key".into(),
            capacity: 10,
            timeout: 3600,
            startup_grace: 30,
            terminal_grace: 60,
            cleanup_timeout: 30,
            warm_pool_size: 2,
            warm_create_concurrency: 2,
            warm_idle_seconds: 600,
            installation: "test".into(),
        }),
        image: format!("test/runner@sha256:{}", "a".repeat(64)),
        gateway_image: format!("test/gateway@sha256:{}", "b".repeat(64)),
        namespace: "test".into(),
        pod_name: "manager".into(),
        pod_uid: "manager-uid".into(),
        memory_mb: 1024,
        cpus: 1,
        tmp_mb: 256,
        runtime_class: "runsc".into(),
        key_id: "kid".into(),
        node_selector: json!({}),
        tolerations: json!([]),
        pull_secrets: json!([]),
        app_name: "flow-like".into(),
        kubernetes_host: "10.0.0.1".into(),
        kubernetes_port: 443,
    }
}

fn payload(run: &str) -> Dispatch {
    serde_json::from_value(json!({"job_id":"job","run_id":run,"app_id":"app","board_id":"board","node_id":"node","user_id":"user","credentials":{},"executor_jwt":"signed-run-capability","callback_url":"http://api:8080"})).unwrap()
}

fn controller() -> (Arc<Manager>, Arc<FakeKube>, Arc<FakeClaims>) {
    let kube = Arc::new(FakeKube::default());
    let claims = Arc::new(FakeClaims::default());
    let manager = Manager::new(config(), kube.clone(), claims.clone()).unwrap();
    manager.ready.store(true, Ordering::Release);
    (manager, kube, claims)
}

#[test]
fn runner_and_gateway_have_independent_capabilities_and_security_contexts() {
    let config = config();
    let mut slot = Slot::new();
    slot.gateway_ip = "fd00::1234".into();
    let runner = config.pod(&slot, false);
    let gateway = config.pod(&slot, true);
    assert_eq!(runner["spec"]["runtimeClassName"], "runsc");
    assert!(gateway["spec"].get("runtimeClassName").is_none());
    assert_eq!(runner["spec"]["automountServiceAccountToken"], false);
    assert_eq!(gateway["spec"]["automountServiceAccountToken"], false);
    assert_eq!(runner["spec"]["dnsPolicy"], "None");
    assert_eq!(
        runner["spec"]["containers"][0]["command"],
        json!(["/app/execution-slot"])
    );
    let env = runner["spec"]["containers"][0]["env"].as_array().unwrap();
    assert!(env.iter().any(|item|item["name"]=="HTTP_PROXY" && item["value"]=="http://[fd00::1234]:3128"));
    for secret in [
        "GATEWAY_TOKEN",
        "BACKEND_KEY",
        "REDIS_URL",
        "AWS_SECRET_ACCESS_KEY",
        "EXECUTION_MANAGER_TOKEN",
    ] {
        assert!(!env.iter().any(|item| item["name"] == secret));
    }
    assert_eq!(
        runner["spec"]["containers"][0]["securityContext"]["capabilities"]["drop"],
        json!(["ALL"])
    );
    assert!(runner["spec"]["activeDeadlineSeconds"].as_u64().unwrap() >= 3600 + 600);
}

#[test]
fn runner_policy_grants_only_its_paired_gateway() {
    let config = config();
    let slot = Slot::new();
    let policy = config.policy(&slot, false);
    assert_eq!(policy["spec"]["policyTypes"], json!(["Ingress", "Egress"]));
    assert_eq!(policy["spec"]["egress"].as_array().unwrap().len(), 1);
    assert_eq!(
        policy["spec"]["egress"][0]["to"][0]["podSelector"]["matchLabels"][SLOT],
        slot.name
    );
    assert_eq!(policy["spec"]["egress"][0]["ports"][0]["port"], 3128);
    let gateway = config.policy(&slot, true);
    assert_eq!(gateway["spec"]["ingress"][0]["ports"][0]["port"], 9001);
    assert_eq!(
        gateway["spec"]["ingress"][1]["from"][0]["podSelector"]["matchLabels"][SLOT],
        slot.name
    );
}

#[tokio::test]
async fn both_policies_precede_any_pod_creation_and_cleanup_preserves_them_until_termination() {
    let (manager, kube, _) = controller();
    let slot = manager.create_slot().await.unwrap();
    let events = kube.events.lock().unwrap().clone();
    assert_eq!(
        (&events[0].0, events[0].1.as_str()),
        (&Method::POST, "networkpolicies")
    );
    assert_eq!(
        (&events[1].0, events[1].1.as_str()),
        (&Method::POST, "networkpolicies")
    );
    manager.discard(&slot).await.unwrap();
    let count = {
        let events = kube.events.lock().unwrap();
        let deletes = events
            .iter()
            .filter(|(method, _, _, _)| *method == Method::DELETE)
            .collect::<Vec<_>>();
        assert!(deletes[0].2.ends_with("-runner"));
        assert!(deletes[1].2.ends_with("-gateway"));
        assert_eq!(deletes[2].1, "networkpolicies");
        events.len()
    };
    manager.discard(&slot).await.unwrap();
    assert_eq!(kube.events.lock().unwrap().len(), count);
}

#[tokio::test]
async fn partial_creation_removes_gateway_and_policies() {
    let (manager, kube, _) = controller();
    kube.fail_runner_create.store(true, Ordering::Release);
    assert!(manager.create_slot().await.is_err());
    assert!(kube.objects.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_force_deletion_or_policy_removal_without_kubelet_confirmation() {
    let (manager, kube, _) = controller();
    let slot = manager.create_slot().await.unwrap();
    kube.refuse_termination.store(true, Ordering::Release);
    let pod = kube
        .get("pods", &format!("{}-runner", slot.name))
        .await
        .unwrap()
        .unwrap();
    assert!(manager.finish_pod(pod, Instant::now()).await.is_err());
    assert!(
        !kube
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(method, _, _, _)| *method == Method::DELETE)
    );
    assert!(
        kube.get("networkpolicies", &slot.name)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn cancellation_marker_precedes_termination_and_survives_manager_ownership() {
    let (manager, kube, _) = controller();
    let slot = manager.create_slot().await.unwrap();
    for suffix in ["-runner", "-gateway"] {
        kube.request(
            Method::PATCH,
            "pods",
            Some(&format!("{}{suffix}", slot.name)),
            Some(manager.config.bind("run", now() + 3600.0, now())),
            &[],
        )
        .await
        .unwrap();
    }
    kube.events.lock().unwrap().clear();
    let answer = manager.cancel("run").await.unwrap();
    assert_eq!(answer["terminated"], true);
    {
        let events = kube.events.lock().unwrap();
        assert_eq!(events[0].0, Method::POST);
        assert_eq!(events[0].1, "configmaps");
    }
    let marker = kube
        .get("configmaps", &manager.config.marker_name("run"))
        .await
        .unwrap()
        .unwrap();
    assert!(marker["metadata"].get("ownerReferences").is_none());
    assert!(!expired_object(&marker));
}

#[tokio::test]
async fn concurrent_reservations_assign_each_slot_once_without_pod_mutations() {
    let (manager, kube, _) = controller();
    manager
        .state
        .lock()
        .unwrap()
        .warm
        .push_back(Arc::new(Slot::new()));
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..32 {
        let manager = manager.clone();
        tasks.spawn(async move { manager.reserve(&payload(&format!("run-{i}"))).await });
    }
    let mut reserved = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(reservation) = result.unwrap() {
            reserved.push(reservation);
        }
    }
    assert_eq!(reserved.len(), 1);
    assert!(kube.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn two_manager_replicas_cannot_assign_the_same_run() {
    let (first, kube, claims) = controller();
    let second = Manager::new(config(), kube, claims).unwrap();
    second.ready.store(true, Ordering::Release);
    for manager in [&first, &second] {
        manager
            .state
            .lock()
            .unwrap()
            .warm
            .push_back(Arc::new(Slot::new()));
    }
    let dispatch = payload("same-run");
    let (one, two) = tokio::join!(first.reserve(&dispatch), second.reserve(&dispatch));
    assert_ne!(one.is_ok(), two.is_ok());
    assert_eq!(
        first.state.lock().unwrap().active.len() + second.state.lock().unwrap().active.len(),
        1
    );
    assert_eq!(
        first.state.lock().unwrap().warm.len() + second.state.lock().unwrap().warm.len(),
        1
    );
}

#[tokio::test]
async fn ambiguous_claim_preserves_tombstone_and_only_returns_pristine_slot() {
    let (manager, _, claims) = controller();
    manager
        .state
        .lock()
        .unwrap()
        .warm
        .push_back(Arc::new(Slot::new()));
    claims.ambiguous.store(true, Ordering::Release);
    assert!(matches!(
        manager.reserve(&payload("run")).await,
        Err(Error::Internal(_))
    ));
    assert!(claims.values.lock().unwrap().contains_key("run"));
    assert_eq!(manager.state.lock().unwrap().warm.len(), 1);
    assert!(manager.state.lock().unwrap().active.is_empty());
    claims.ambiguous.store(false, Ordering::Release);
    assert!(matches!(
        manager.reserve(&payload("run")).await,
        Err(Error::Cancelled)
    ));
}

#[tokio::test]
async fn expired_slots_are_skipped_even_if_preparation_completed_out_of_order() {
    let (manager, _, _) = controller();
    let mut old = Slot::new();
    old.born -= 1000.0;
    let fresh = Arc::new(Slot::new());
    manager
        .state
        .lock()
        .unwrap()
        .warm
        .extend([Arc::new(old), fresh.clone()]);
    let _reservation = manager.reserve(&payload("run")).await.unwrap();
    assert!(
        manager
            .state
            .lock()
            .unwrap()
            .active
            .contains_key(&fresh.name)
    );
    assert!(matches!(
        manager.reserve(&payload("next-run")).await,
        Err(Error::NoCapacity)
    ));
}

#[tokio::test]
async fn startup_deadline_bounds_headers_without_shortening_execution_stream() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_http_head(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        socket.write_all(b"hello").await.unwrap();
    });
    let request = Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{address}/"));
    let response = send_with_header_deadline(request, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(response.bytes().await.unwrap(), "hello");
    server.await.unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_http_head(&mut socket).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
    });
    let request = Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{address}/"));
    assert!(
        matches!(send_with_header_deadline(request,Duration::from_millis(50)).await,Err(Error::Internal(message)) if message.contains("startup budget"))
    );
    server.await.unwrap();
}

async fn read_http_head(socket: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    tokio::time::timeout(Duration::from_secs(1), async {
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            assert!(head.len() < 8192, "HTTP fixture received oversized headers");
            head.push(socket.read_u8().await.unwrap());
        }
        assert!(head.starts_with(b"GET / HTTP/1.1\r\n"));
    })
    .await
    .unwrap();
}
