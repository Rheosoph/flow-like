use super::*;
use crate::Reservation;
use async_trait::async_trait;
use std::sync::atomic::AtomicUsize;
use tower::ServiceExt;

struct MockBackend {
    failure: AtomicUsize,
    reservations: AtomicUsize,
    executed: Arc<AtomicUsize>,
    admit: Arc<Semaphore>,
    finish: Arc<Semaphore>,
    stopped: AtomicBool,
}

struct MockReservation {
    executed: Arc<AtomicUsize>,
    finish: Arc<Semaphore>,
}

#[async_trait]
impl Reservation for MockReservation {
    async fn execute(self: Box<Self>, _: Dispatch, _: Mode, events: EventSink) -> Result<Value> {
        self.finish.acquire().await.unwrap().forget();
        let result = json!({"success": true});
        events
            .send(event("completed\n\nevent: injected", result.clone()))
            .await;
        self.executed.fetch_add(1, Ordering::SeqCst);
        Ok(result)
    }
}

#[async_trait]
impl Backend for MockBackend {
    fn ready(&self) -> bool {
        !self.stopped.load(Ordering::SeqCst)
    }
    fn metrics(&self) -> String {
        String::new()
    }
    async fn prepare(self: Arc<Self>) -> Result<()> {
        Ok(())
    }
    async fn reserve(&self, _: &Dispatch) -> Result<Box<dyn Reservation>> {
        self.reservations.fetch_add(1, Ordering::SeqCst);
        self.admit.acquire().await.unwrap().forget();
        match self.failure.load(Ordering::SeqCst) {
            1 => Err(Error::NoCapacity),
            2 => Err(Error::internal("secret should not reach HTTP")),
            _ => Ok(Box::new(MockReservation {
                executed: self.executed.clone(),
                finish: self.finish.clone(),
            })),
        }
    }
    async fn cancel(&self, run_id: &str) -> Result<Value> {
        Ok(json!({"run_id":run_id,"terminated":true}))
    }
    async fn shutdown(&self) -> Result<()> {
        self.stopped.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn setup() -> (Arc<MockBackend>, ServerState) {
    let config = Arc::new(CommonConfig {
        token: "a".repeat(32),
        callback_url: "http://callback:8080".into(),
        object_store_url: "http://store:9000".into(),
        allowed_https_hosts: vec![],
        buckets: vec!["meta".into(), "content".into(), "logs".into()],
        object_store_tls_gateway: false,
        backend_pub: "public-key".into(),
        capacity: 1,
        timeout: 1,
        startup_grace: 1,
        terminal_grace: 1,
        cleanup_timeout: 1,
        warm_pool_size: 1,
        warm_create_concurrency: 1,
        warm_idle_seconds: 300,
        installation: "test".into(),
    });
    let backend = Arc::new(MockBackend {
        failure: AtomicUsize::new(0),
        reservations: AtomicUsize::new(0),
        executed: Arc::new(AtomicUsize::new(0)),
        admit: Arc::new(Semaphore::new(100)),
        finish: Arc::new(Semaphore::new(0)),
        stopped: AtomicBool::new(false),
    });
    (backend.clone(), ServerState::new(backend, config))
}

fn payload() -> Value {
    json!({"job_id":"job","run_id":"run","app_id":"app","board_id":"board","node_id":"node","user_id":"user", "credentials":{},"executor_jwt":"signed-token","callback_url":"http://callback:8080",
        "artifact":{"url":"http://store:9000/meta/artifact","path":"artifact","registry_fingerprint":"hash"}})
}

fn request(path: &str, value: Value) -> Request {
    let raw = serde_json::to_vec(&value).unwrap();
    Request::builder()
        .method("POST")
        .uri(path)
        .header("x-execution-manager-token", "a".repeat(32))
        .header(header::CONTENT_LENGTH, raw.len())
        .body(Body::from(raw))
        .unwrap()
}

async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn streaming_success_requires_confirmed_admission_and_ambiguous_errors_cannot_retry() {
    for (failure, status, marker) in [
        (1, StatusCode::TOO_MANY_REQUESTS, true),
        (2, StatusCode::BAD_GATEWAY, false),
    ] {
        let (backend, state) = setup();
        backend.failure.store(failure, Ordering::SeqCst);
        let response = router(state)
            .oneshot(request("/execute/sse", payload()))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(
            response.headers().get("x-execution-admitted").is_some(),
            marker
        );
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("secret"));
        assert_eq!(backend.executed.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn invalid_and_unauthenticated_dispatches_never_reach_reservation() {
    let (backend, state) = setup();
    for (field, value) in [
        ("image", json!("evil")),
        ("callback_url", json!("http://other")),
        ("token", json!("general-key")),
        ("artifact", Value::Null),
    ] {
        let mut body = payload();
        body[field] = value;
        assert_eq!(
            router(state.clone())
                .oneshot(request("/execute", body))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let mut bad = request("/execute", payload());
    bad.headers_mut()
        .append("x-execution-manager-token", "duplicate".parse().unwrap());
    assert_eq!(
        router(state).oneshot(bad).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(backend.reservations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn disconnected_client_cannot_interrupt_durable_admission_or_execution() {
    let (backend, state) = setup();
    backend.admit.forget_permits(100);
    let frontend = tokio::spawn(router(state.clone()).oneshot(request("/execute", payload())));
    until(|| backend.reservations.load(Ordering::SeqCst) == 1).await;
    frontend.abort();
    backend.admit.add_permits(1);
    backend.finish.add_permits(1);
    until(|| {
        backend.executed.load(Ordering::SeqCst) == 1 && state.capacity.available_permits() == 1
    })
    .await;
}

#[tokio::test]
async fn admission_lasts_until_execution_finishes_after_stream_disconnect() {
    let (backend, state) = setup();
    let first = router(state.clone())
        .oneshot(request("/execute/stream", payload()))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    drop(first);
    let second = router(state.clone())
        .oneshot(request("/execute", payload()))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(backend.reservations.load(Ordering::SeqCst), 1);
    backend.finish.add_permits(1);
    until(|| state.capacity.available_permits() == 1).await;
    assert_eq!(backend.executed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sse_keeps_tenant_strings_inside_json_data() {
    let (backend, state) = setup();
    backend.finish.add_permits(1);
    let response = router(state)
        .oneshot(request("/execute/sse", payload()))
        .await
        .unwrap();
    let data = to_bytes(response.into_body(), 4096).await.unwrap();
    let text = std::str::from_utf8(&data).unwrap();
    assert!(text.starts_with("data: {"));
    assert!(!text.contains("\nevent: injected"));
    assert!(text.ends_with("\n\n"));
}

#[tokio::test]
async fn server_shutdown_waits_for_admitted_work_and_native_health_route_works() {
    let (backend, state) = setup();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let supervisor = tokio::spawn(serve(listener, state, stop.clone()));
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert!(
        client
            .get(format!("http://{address}/ready"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let accepted = client
        .post(format!("http://{address}/execute/stream"))
        .header("x-execution-manager-token", "a".repeat(32))
        .json(&payload())
        .send()
        .await
        .unwrap();
    assert!(accepted.status().is_success());
    stop.cancel();
    until(|| backend.stopped.load(Ordering::SeqCst)).await;
    assert!(!supervisor.is_finished());
    backend.finish.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), supervisor)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(backend.executed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn complete_request_can_wait_for_a_run_longer_than_the_header_deadline() {
    let (backend, state) = setup();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let supervisor = tokio::spawn(serve_with_header_timeout(
        listener,
        state,
        stop.clone(),
        Duration::from_millis(50),
    ));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let response = tokio::spawn(async move {
        client
            .post(format!("http://{address}/execute"))
            .header("x-execution-manager-token", "a".repeat(32))
            .json(&payload())
            .send()
            .await
    });
    until(|| backend.reservations.load(Ordering::SeqCst) == 1).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !response.is_finished(),
        "Execution connections must survive an idle request side"
    );
    backend.finish.add_permits(1);
    assert_eq!(response.await.unwrap().unwrap().status(), StatusCode::OK);
    stop.cancel();
    supervisor.await.unwrap().unwrap();
}

#[tokio::test]
async fn half_closed_request_keeps_receiving_its_execution_result() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (backend, state) = setup();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let supervisor = tokio::spawn(serve(listener, state, stop.clone()));
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = serde_json::to_vec(&payload()).unwrap();
    let headers = format!(
        "POST /execute HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nX-Execution-Manager-Token: {}\r\nContent-Length: {}\r\n\r\n",
        "a".repeat(32),
        body.len()
    );
    client.write_all(headers.as_bytes()).await.unwrap();
    client.write_all(&body).await.unwrap();
    client.shutdown().await.unwrap();
    until(|| backend.reservations.load(Ordering::SeqCst) == 1).await;
    backend.finish.add_permits(1);
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(2), client.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("\"success\":true"));
    assert_eq!(backend.executed.load(Ordering::SeqCst), 1);
    stop.cancel();
    supervisor.await.unwrap().unwrap();
}
