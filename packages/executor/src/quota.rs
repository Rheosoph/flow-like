use crate::{error::ExecutorError, jwt::ExecutorClaims};
use flow_like_types::{tokio, tokio_util::sync::CancellationToken};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct ComputeContext {
    pub function_name: String,
    pub request_id: String,
    pub memory_mb: i32,
    pub architecture: String,
    pub region: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    started: Instant,
    started_recorded: Arc<AtomicBool>,
}

impl ComputeContext {
    pub fn new(function_name: String, request_id: String, memory_mb: i32) -> Self {
        Self {
            function_name,
            request_id,
            memory_mb,
            architecture: match std::env::consts::ARCH {
                "aarch64" => "arm64",
                other => other,
            }
            .into(),
            region: std::env::var("AWS_REGION").unwrap_or_else(|_| "unknown".into()),
            started_at: chrono::Utc::now(),
            started: Instant::now(),
            started_recorded: Arc::new(AtomicBool::new(false)),
        }
    }
}

tokio::task_local! { pub static CURRENT_COMPUTE: ComputeContext; }

/// Keeps the initial accounting request alive while setup proceeds, and joins
/// it before admission or terminal evidence. Cancellation aborts owned work.
#[derive(Default)]
pub(crate) struct ComputeStart {
    task: Option<tokio::task::JoinHandle<()>>,
}

impl ComputeStart {
    async fn spawn(report: impl std::future::Future<Output = ()> + Send + 'static) -> Self {
        let (polled, first_poll) = tokio::sync::oneshot::channel();
        let start = Self {
            task: Some(tokio::spawn(async move {
                tokio::pin!(report);
                let mut polled = Some(polled);
                futures_util::future::poll_fn(|cx| {
                    let result = report.as_mut().poll(cx);
                    if let Some(polled) = polled.take() {
                        let _ = polled.send(());
                    }
                    result
                })
                .await;
            })),
        };
        // Warm setup can finish without yielding on a current-thread runtime.
        // Start the request first, while keeping ownership if this wait is cancelled.
        let _ = first_poll.await;
        start
    }

    pub(crate) async fn complete<T>(&mut self, outcome: std::thread::Result<T>) -> T {
        self.wait().await;
        match outcome {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    pub(crate) async fn wait(&mut self) {
        // Keep ownership while awaiting so cancellation also reaches Drop.
        if let Some(task) = self.task.as_mut() {
            if let Err(error) = task.await {
                tracing::warn!(%error, "Initial compute evidence task failed");
            }
        }
        self.task = None;
    }
}

impl Drop for ComputeStart {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub(crate) async fn start_compute(claims: &ExecutorClaims, token: &str) -> ComputeStart {
    if claims.runtime_limit_ms.is_none() {
        return ComputeStart::default();
    }
    let Ok(context) = CURRENT_COMPUTE.try_with(Clone::clone) else {
        return ComputeStart::default();
    };
    let claims = claims.clone();
    let token = token.to_owned();
    // Tokio tasks do not inherit task-local values. Capture the physical
    // invocation before spawning so each report keeps its original identity.
    ComputeStart::spawn(async move {
        record_compute_status(&claims, &token, &context, "started").await;
    })
    .await
}

pub async fn record_compute(claims: &ExecutorClaims, token: &str, status: &str) {
    if claims.runtime_limit_ms.is_none() {
        return;
    }
    let Ok(context) = CURRENT_COMPUTE.try_with(Clone::clone) else {
        return;
    };
    // Retry a failed initial report before terminal evidence. Reusing its
    // original timestamp and revision keeps a lost acknowledgement idempotent.
    if status != "started" && !context.started_recorded.load(Ordering::Acquire) {
        record_compute_status(claims, token, &context, "started").await;
    }
    record_compute_status(claims, token, &context, status).await;
}

fn compute_report(context: &ComputeContext, status: &str) -> serde_json::Value {
    let duration = (status != "started").then(|| {
        context
            .started
            .elapsed()
            .as_nanos()
            .div_ceil(1_000_000)
            .min(i64::MAX as u128) as i64
    });
    json!({
        "functionName":context.function_name,"requestId":context.request_id,"memoryMb":context.memory_mb,
        "architecture":context.architecture,"region":context.region,"startedAt":context.started_at,
        "measuredDurationMs":duration,"status":status,"revision":if duration.is_some(){"executor-finished"}else{"started"},
        "operationId":null,"payerId":null,"role":"executor","costClass":"workflow_compute","billedDurationMs":null,
        "costMicroUsd":null,"evidence":"measured_estimate","rateVersion":"aws-lambda-reference-2026-09"
    })
}

async fn record_compute_status(
    claims: &ExecutorClaims,
    token: &str,
    context: &ComputeContext,
    status: &str,
) {
    let body = json!({"phase":"attempt","attempt_id":context.request_id,"compute":compute_report(context, status)});
    match callback(&claims.callback_url, token, body).await {
        Ok(true) if status == "started" => context.started_recorded.store(true, Ordering::Release),
        Ok(_) => (),
        Err(error) => {
            tracing::warn!(%error,run_id=%claims.run_id,"Compute attempt needs AWS report reconciliation");
        }
    }
}

pub struct RuntimeLease {
    token: String,
    callback_url: String,
    receipt_url: Option<String>,
    attempt_id: String,
    started: Instant,
    cancellation_watch: Option<tokio::task::JoinHandle<()>>,
    pub limit: Duration,
}

async fn callback(url: &str, token: &str, body: serde_json::Value) -> Result<bool, ExecutorError> {
    static CLIENT: tokio::sync::OnceCell<reqwest::Client> = tokio::sync::OnceCell::const_new();
    let client = CLIENT
        .get_or_try_init(|| async {
            reqwest::Client::builder()
                .user_agent(concat!("flow-like-executor/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| ExecutorError::Callback(e.to_string()))
        })
        .await?;
    let url = format!("{}/api/v1/execution/quota", url.trim_end_matches('/'));
    let mut last = String::new();
    for attempt in 0..3 {
        match client
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                match response.json::<serde_json::Value>().await {
                    Ok(value) => return Ok(value["accepted"].as_bool().unwrap_or(false)),
                    Err(error) => last = error.to_string(),
                }
            }
            Ok(response) if response.status().is_client_error() => {
                return Err(ExecutorError::Callback(format!(
                    "Quota callback rejected: {}",
                    response.status()
                )));
            }
            Ok(response) => last = format!("Quota callback returned {}", response.status()),
            Err(error) => last = error.to_string(),
        }
        if attempt < 2 {
            flow_like_types::tokio::time::sleep(Duration::from_millis(100 << attempt)).await;
        }
    }
    Err(ExecutorError::Callback(last))
}

pub async fn begin(
    claims: &ExecutorClaims,
    token: &str,
) -> Result<Option<RuntimeLease>, ExecutorError> {
    let Some(limit_ms) = claims.runtime_limit_ms else {
        return Ok(None);
    };
    if limit_ms == 0 {
        return Err(ExecutorError::InvalidRequest(
            "Cloud runtime allowance is empty".into(),
        ));
    }
    let attempt_id = flow_like_types::create_id();
    if !callback(
        &claims.callback_url,
        token,
        json!({"phase":"start","attempt_id":attempt_id}),
    )
    .await?
    {
        return Err(ExecutorError::InvalidRequest(
            "This cloud run is already claimed or completed".into(),
        ));
    }
    Ok(Some(RuntimeLease {
        token: token.into(),
        callback_url: claims.callback_url.clone(),
        receipt_url: claims.quota_receipt_url.clone(),
        attempt_id,
        started: Instant::now(),
        cancellation_watch: None,
        limit: Duration::from_millis(limit_ms),
    }))
}

impl RuntimeLease {
    /// Poll a fixed operation row. Connection loss cannot clear the reservation.
    pub fn watch_cancellation(&mut self, cancellation: CancellationToken) {
        let url = self.callback_url.clone();
        let token = self.token.clone();
        let attempt = self.attempt_id.clone();
        self.cancellation_watch = Some(tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
                match callback(&url, &token, json!({"phase":"poll","attempt_id":attempt})).await {
                    Ok(true) => {}
                    Ok(false) | Err(_) => {
                        cancellation.cancel();
                        break;
                    }
                }
            }
        }));
    }
    pub fn elapsed_ms(&self) -> u64 {
        self.started
            .elapsed()
            .as_nanos()
            .div_ceil(1_000_000)
            .min(u64::MAX as u128) as u64
    }

    pub async fn finish(&self, duration_ms: u64, status: &str) -> Result<(), ExecutorError> {
        let receipt = json!({"phase":"finish","attempt_id":self.attempt_id,"runtime_ms":duration_ms,"status":status});
        let mut stored = false;
        if let Some(url) = &self.receipt_url {
            static CLIENT: tokio::sync::OnceCell<reqwest::Client> =
                tokio::sync::OnceCell::const_new();
            let client = CLIENT
                .get_or_try_init(|| async {
                    reqwest::Client::builder()
                        .timeout(Duration::from_secs(5))
                        .build()
                        .map_err(|e| ExecutorError::Callback(e.to_string()))
                })
                .await?;
            for _ in 0..3 {
                if client
                    .put(url)
                    .header(reqwest::header::IF_NONE_MATCH, "*")
                    .json(&receipt)
                    .send()
                    .await
                    .is_ok_and(|r| {
                        r.status().is_success()
                            || r.status() == reqwest::StatusCode::PRECONDITION_FAILED
                    })
                {
                    stored = true;
                    break;
                }
            }
        }
        match callback(&self.callback_url, &self.token, receipt).await {
            Ok(_) => Ok(()),
            Err(error) if stored => {
                tracing::warn!(%error,"Runtime receipt saved; scheduled reconciliation will finish accounting");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for RuntimeLease {
    fn drop(&mut self) {
        if let Some(watch) = self.cancellation_watch.take() {
            watch.abort();
        }
    }
}

pub async fn reject(claims: &ExecutorClaims, token: &str) {
    if claims.runtime_limit_ms.is_some() {
        if let Err(error) = callback(
            &claims.callback_url,
            token,
            json!({"phase":"reject","attempt_id":flow_like_types::create_id()}),
        )
        .await
        {
            tracing::warn!(run_id=%claims.run_id,%error,"Failed to release unstarted runtime reservation");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        http::{HeaderMap, StatusCode},
        routing::{post, put},
        Json, Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn claims_for(base: &str) -> ExecutorClaims {
        serde_json::from_value(json!({"sub":"payer","run_id":"run","app_id":"app",
            "board_id":"board","callback_url":base,"runtime_limit_ms":30000,
            "typ":"executor","iss":"test","aud":"test","iat":0,"nbf":0,
            "exp":9999999999_i64,"jti":"id"}))
        .unwrap()
    }

    fn compute_context() -> ComputeContext {
        ComputeContext::new("executor".into(), "physical-attempt".into(), 2048)
    }

    async fn callback_server<F>(
        respond: F,
    ) -> (
        String,
        Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>,
        tokio::task::JoinHandle<()>,
    )
    where
        F: Fn(serde_json::Value, usize) -> axum::response::Response + Clone + Send + Sync + 'static,
    {
        let bodies = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let received = bodies.clone();
        let router = Router::new().route(
            "/api/v1/execution/quota",
            post(move |Json(body): Json<serde_json::Value>| {
                let received = received.clone();
                let respond = respond.clone();
                async move {
                    let mut bodies = received.lock().await;
                    bodies.push(body.clone());
                    respond(body, bodies.len())
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (base, bodies, server)
    }

    async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), future)
            .await
            .expect("compute callback test timed out")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn start_polls_accounting_before_returning_without_waiting_for_completion() {
        let polled = Arc::new(AtomicBool::new(false));
        let report_polled = polled.clone();
        let (release, released) = tokio::sync::oneshot::channel();
        let mut start = bounded(ComputeStart::spawn(async move {
            report_polled.store(true, Ordering::Release);
            released.await.unwrap();
        }))
        .await;
        assert!(polled.load(Ordering::Acquire));
        assert!(!start.task.as_ref().unwrap().is_finished());
        release.send(()).unwrap();
        bounded(start.wait()).await;
    }

    #[tokio::test]
    async fn early_compute_overlaps_setup_and_keeps_legacy_admission_order() {
        use axum::{
            body::{Body, Bytes},
            response::IntoResponse,
        };
        let received = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let callback_received = received.clone();
        let callback_release = release.clone();
        let (base, bodies, server) = callback_server(move |body, _| {
            if body["compute"]["status"] == "started" {
                callback_received.notify_one();
                let release = callback_release.clone();
                let stream = futures_util::stream::once(async move {
                    release.notified().await;
                    Ok::<Bytes, std::convert::Infallible>(Bytes::from_static(
                        b"{\"accepted\":true}",
                    ))
                });
                axum::response::Response::new(Body::from_stream(stream))
            } else {
                Json(json!({"accepted":true})).into_response()
            }
        })
        .await;
        let context = compute_context();
        let started_at = serde_json::to_value(context.started_at).unwrap();
        bounded(CURRENT_COMPUTE.scope(context, async {
            let claims = claims_for(&base);
            let mut start = start_compute(&claims, "token").await;
            // Setup can observe the in-flight request and finish before its reply.
            received.notified().await;
            assert!(!start.task.as_ref().unwrap().is_finished());
            assert_eq!(bodies.lock().await.len(), 1);
            release.notify_one();
            start.wait().await;
            let lease = begin(&claims, "token").await.unwrap().unwrap();
            assert_eq!(lease.limit, Duration::from_secs(30));
            record_compute(&claims, "token", "completed").await;
        }))
        .await;
        let bodies = bodies.lock().await;
        assert_eq!(
            bodies
                .iter()
                .map(|body| body["phase"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["attempt", "start", "attempt"]
        );
        assert_eq!(bodies[0]["compute"]["startedAt"], started_at);
        assert_eq!(bodies[0]["compute"]["requestId"], "physical-attempt");
        assert_eq!(bodies[0]["compute"]["revision"], "started");
        assert_eq!(bodies[2]["compute"]["status"], "completed");
        assert_eq!(bodies[2]["compute"]["startedAt"], started_at);
        server.abort();
    }

    #[tokio::test]
    async fn early_compute_captures_context_before_leaving_its_scope() {
        use axum::response::IntoResponse;
        let (base, bodies, server) =
            callback_server(|_, _| Json(json!({"accepted":true})).into_response()).await;
        let mut start = CURRENT_COMPUTE
            .scope(compute_context(), async {
                start_compute(&claims_for(&base), "token").await
            })
            .await;
        assert!(CURRENT_COMPUTE.try_with(Clone::clone).is_err());
        bounded(start.wait()).await;
        let bodies = bodies.lock().await;
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["compute"]["requestId"], "physical-attempt");
        assert_eq!(bodies[0]["compute"]["status"], "started");
        server.abort();
    }

    #[tokio::test]
    async fn lost_started_response_retries_the_identical_compute_revision() {
        use axum::{
            body::{Body, Bytes},
            response::IntoResponse,
        };
        let (base, bodies, server) = callback_server(|_, attempt| {
            if attempt == 1 {
                // The API performed its write, but its reply did not reach the executor.
                let stream = futures_util::stream::once(async {
                    Err::<Bytes, _>(std::io::Error::other("connection closed after commit"))
                });
                axum::response::Response::new(Body::from_stream(stream))
            } else {
                Json(json!({"accepted":true})).into_response()
            }
        })
        .await;
        bounded(CURRENT_COMPUTE.scope(compute_context(), async {
            let mut start = start_compute(&claims_for(&base), "token").await;
            start.wait().await;
        }))
        .await;
        let bodies = bodies.lock().await;
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0], bodies[1]);
        assert_eq!(bodies[0]["phase"], "attempt");
        assert_eq!(bodies[0]["compute"]["revision"], "started");
        server.abort();
    }

    #[tokio::test]
    async fn setup_failure_records_started_before_final_evidence_without_claiming() {
        use axum::response::IntoResponse;
        let (base, bodies, server) =
            callback_server(|_, _| Json(json!({"accepted":true})).into_response()).await;
        let context = compute_context();
        let started_at = serde_json::to_value(context.started_at).unwrap();
        bounded(CURRENT_COMPUTE.scope(context, async {
            let claims = claims_for(&base);
            let mut start = start_compute(&claims, "token").await;
            start.wait().await;
            record_compute(&claims, "token", "failed").await;
        }))
        .await;
        let bodies = bodies.lock().await;
        assert_eq!(bodies.len(), 2);
        assert!(bodies.iter().all(|body| body["phase"] == "attempt"));
        assert_eq!(bodies[0]["compute"]["status"], "started");
        assert_eq!(bodies[1]["compute"]["status"], "failed");
        assert!(bodies
            .iter()
            .all(|body| body["compute"]["startedAt"] == started_at));
        assert!(bodies[1]["compute"]["measuredDurationMs"].is_number());
        server.abort();
    }

    #[tokio::test]
    async fn failed_initial_evidence_is_retried_before_terminal_evidence() {
        use axum::response::IntoResponse;
        let (base, bodies, server) = callback_server(|_, attempt| {
            if attempt == 1 {
                StatusCode::FORBIDDEN.into_response()
            } else {
                Json(json!({"accepted":true})).into_response()
            }
        })
        .await;
        bounded(CURRENT_COMPUTE.scope(compute_context(), async {
            let claims = claims_for(&base);
            let mut start = start_compute(&claims, "token").await;
            start.wait().await;
            record_compute(&claims, "token", "failed").await;
        }))
        .await;
        let bodies = bodies.lock().await;
        assert_eq!(bodies.len(), 3);
        assert_eq!(bodies[0], bodies[1]);
        assert_eq!(bodies[1]["compute"]["status"], "started");
        assert_eq!(bodies[2]["compute"]["status"], "failed");
        server.abort();
    }

    #[tokio::test]
    async fn rejected_admission_keeps_its_physical_attempt() {
        use axum::response::IntoResponse;
        let (base, bodies, server) = callback_server(|body, _| {
            Json(json!({"accepted":body["phase"] != "start"})).into_response()
        })
        .await;
        bounded(CURRENT_COMPUTE.scope(compute_context(), async {
            let claims = claims_for(&base);
            let mut start = start_compute(&claims, "token").await;
            start.wait().await;
            assert!(begin(&claims, "token").await.is_err());
            record_compute(&claims, "token", "failed").await;
        }))
        .await;
        let bodies = bodies.lock().await;
        assert_eq!(bodies.len(), 3);
        assert_eq!(bodies[0]["compute"]["status"], "started");
        assert_eq!(bodies[1]["phase"], "start");
        assert_eq!(bodies[2]["compute"]["status"], "failed");
        server.abort();
    }

    #[tokio::test]
    async fn cancellation_during_wait_aborts_the_owned_accounting_task() {
        struct SignalDrop(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for SignalDrop {
            fn drop(&mut self) {
                let _ = self.0.take().unwrap().send(());
            }
        }
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _signal = SignalDrop(Some(dropped_tx));
            started_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        let mut start = ComputeStart { task: Some(task) };
        bounded(started_rx).await.unwrap();
        // Polling the join must retain ownership; dropping a JoinHandle detaches it.
        assert!(futures_util::poll!(Box::pin(start.wait())).is_pending());
        drop(start);
        bounded(dropped_rx).await.unwrap();
    }

    #[tokio::test]
    async fn setup_panic_waits_for_started_evidence_before_resuming_unwind() {
        use futures_util::FutureExt;
        let release = Arc::new(tokio::sync::Notify::new());
        let recorded = Arc::new(AtomicBool::new(false));
        let task_release = release.clone();
        let task_recorded = recorded.clone();
        let task = tokio::spawn(async move {
            task_release.notified().await;
            task_recorded.store(true, Ordering::Release);
        });
        let mut start = ComputeStart { task: Some(task) };
        let outcome = std::panic::AssertUnwindSafe(async { panic!("setup failed") })
            .catch_unwind()
            .await;
        let mut completion =
            Box::pin(std::panic::AssertUnwindSafe(start.complete(outcome)).catch_unwind());
        assert!(futures_util::poll!(&mut completion).is_pending());
        assert!(!recorded.load(Ordering::Acquire));
        release.notify_one();
        assert!(bounded(completion).await.is_err());
        assert!(recorded.load(Ordering::Acquire));
        assert!(start.task.is_none());
    }

    #[tokio::test]
    async fn non_lambda_admission_uses_the_legacy_start_without_compute_evidence() {
        use axum::response::IntoResponse;
        let (base, bodies, server) =
            callback_server(|_, _| Json(json!({"accepted":true})).into_response()).await;
        let claims = claims_for(&base);
        let mut start = start_compute(&claims, "token").await;
        assert!(start.task.is_none());
        bounded(async {
            start.wait().await;
            begin(&claims, "token").await.unwrap().unwrap();
        })
        .await;
        let bodies = bodies.lock().await;
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["phase"], "start");
        assert!(bodies[0].get("compute").is_none());
        server.abort();
    }

    #[tokio::test]
    async fn settlement_reuses_connections_without_reusing_credentials() {
        use axum::extract::ConnectInfo;
        use std::net::SocketAddr;

        let callbacks = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let receipts = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let callback_observations = callbacks.clone();
        let receipt_observations = receipts.clone();
        let router = Router::new()
            .route(
                "/receipt",
                put(
                    move |ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap| {
                        let observations = receipt_observations.clone();
                        async move {
                            assert_eq!(headers[reqwest::header::IF_NONE_MATCH], "*");
                            assert!(!headers.contains_key(reqwest::header::AUTHORIZATION));
                            observations.lock().await.push(peer);
                            StatusCode::NO_CONTENT
                        }
                    },
                ),
            )
            .route(
                "/api/v1/execution/quota",
                post(
                    move |ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap| {
                        let observations = callback_observations.clone();
                        async move {
                            assert_eq!(
                                headers[reqwest::header::USER_AGENT],
                                concat!("flow-like-executor/", env!("CARGO_PKG_VERSION"))
                            );
                            let token = headers[reqwest::header::AUTHORIZATION]
                                .to_str()
                                .unwrap()
                                .to_owned();
                            observations.lock().await.push((peer, token));
                            Json(json!({"accepted": true}))
                        }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        for token in ["first-run", "second-run", "third-run"] {
            let lease = RuntimeLease {
                token: token.to_string(),
                callback_url: base.clone(),
                receipt_url: Some(format!("{base}/receipt")),
                attempt_id: token.to_string(),
                started: Instant::now(),
                cancellation_watch: None,
                limit: Duration::from_secs(30),
            };
            lease.finish(10, "completed").await.unwrap();
        }

        let callbacks = callbacks.lock().await;
        let receipts = receipts.lock().await;
        assert_eq!(callbacks.len(), 3);
        assert_eq!(receipts.len(), 3);
        assert!(callbacks.iter().all(|(peer, _)| *peer == callbacks[0].0));
        assert!(receipts.iter().all(|peer| *peer == receipts[0]));
        assert_eq!(
            callbacks
                .iter()
                .map(|(_, token)| token.as_str())
                .collect::<Vec<_>>(),
            ["Bearer first-run", "Bearer second-run", "Bearer third-run"]
        );
        server.abort();
    }

    #[tokio::test]
    async fn durable_receipt_survives_unavailable_settlement_api() {
        let receipts = Arc::new(tokio::sync::Mutex::new(Vec::<serde_json::Value>::new()));
        let failures = Arc::new(AtomicUsize::new(0));
        let received = receipts.clone();
        let observed = receipts.clone();
        let counted = failures.clone();
        let router = Router::new()
            .route(
                "/receipt",
                put(
                    move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                        let received = received.clone();
                        async move {
                            assert_eq!(headers.get(reqwest::header::IF_NONE_MATCH).unwrap(), "*");
                            received.lock().await.push(body);
                            StatusCode::OK
                        }
                    },
                ),
            )
            .route(
                "/api/v1/execution/quota",
                post(move |Json(body): Json<serde_json::Value>| {
                    let observed = observed.clone();
                    let counted = counted.clone();
                    async move {
                        if body["phase"] == "finish" {
                            assert_eq!(
                                observed.lock().await.len(),
                                1,
                                "receipt precedes settlement"
                            );
                            counted.fetch_add(1, Ordering::SeqCst);
                            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})))
                        } else {
                            (StatusCode::OK, Json(json!({"accepted":true})))
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let claims:ExecutorClaims=serde_json::from_value(json!({"sub":"payer","run_id":"run","app_id":"app","board_id":"board","callback_url":base,"runtime_limit_ms":30000,"quota_receipt_url":format!("{base}/receipt"),"typ":"executor","iss":"test","aud":"test","iat":0,"nbf":0,"exp":9999999999_i64,"jti":"id"})).unwrap();
        let lease = begin(&claims, "test-token").await.unwrap().unwrap();
        lease
            .finish(37, "completed")
            .await
            .expect("durable receipt permits later reconciliation");
        assert_eq!(failures.load(Ordering::SeqCst), 3);
        let stored = receipts.lock().await;
        assert_eq!(stored[0]["runtime_ms"], 37);
        assert_eq!(stored[0]["attempt_id"], lease.attempt_id);
        server.abort();
    }
}
