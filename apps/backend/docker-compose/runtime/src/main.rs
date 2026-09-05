#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get,
    Router,
};
use flow_like_api::execution::queue::QueueDisposition;
use flow_like_api::execution::{QueueConfig, QueueWorker, QueuedJob};
use flow_like_executor::{
    execute, executor_router, ExecutionRequest, ExecutorConfig, ExecutorState,
};
use std::{
    future::{Future, IntoFuture},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

mod config;
mod metrics;
mod once;

async fn metrics_middleware(request: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let method = request.method().to_string();
    // Unknown paths share one label so attacker-selected URLs cannot create
    // an unbounded set of Prometheus time series.
    let path = match request.uri().path() {
        "/execute" => "/execute",
        "/execute/stream" => "/execute/stream",
        "/execute/sse" => "/execute/sse",
        "/health" => "/health",
        "/ready" => "/ready",
        "/metrics" => "/metrics",
        _ => "other",
    };
    let response = next.run(request).await;
    metrics::record_http_request(
        &method,
        path,
        response.status().as_u16(),
        start.elapsed().as_secs_f64(),
    );
    response
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--once") {
        // stdout is exclusively the NDJSON result transport.
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .init();
        return once::run(&std::env::args().nth(2).unwrap_or_default()).await;
    }
    metrics::init_telemetry();
    let config = config::Config::from_env()?;
    let executor_config =
        ExecutorConfig::from_env().with_completion_observer(metrics::record_execution);
    let capacity = Arc::new(Semaphore::new(config.max_concurrent_executions));
    let ready = Arc::new(AtomicBool::new(true));
    let mut worker_task = None;
    let mut shutdown_budget = executor_config.execution_timeout() + Duration::from_secs(60);

    if config.queue_worker_enabled {
        let queue_config = QueueConfig {
            redis_url: config.redis_url.clone().ok_or("REDIS_URL is required")?,
            queue_name: config.redis_queue_name.clone(),
            concurrency: config.queue_worker_concurrency,
            poll_timeout_secs: config.poll_timeout_secs,
        };
        let worker = QueueWorker::with_admission(queue_config, capacity.clone()).await?;
        // The manager request leaves a further 30 seconds for trusted status
        // verification and delivery settlement during a normal drain.
        shutdown_budget = worker.execution_request_timeout() + Duration::from_secs(30);
        let worker_config = config.clone();
        let worker_executor = executor_config.clone().with_required_terminal_status_ack();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(worker.execution_request_timeout())
            .build()?;
        let worker_ready = ready.clone();
        let worker_capacity = capacity.clone();
        worker_task = Some(tokio::spawn(async move {
            let result = worker
                .run_with_outcomes(move |job| {
                    let config = worker_config.clone();
                    let executor = worker_executor.clone();
                    let client = client.clone();
                    async move { process_queued_job(job, config, executor, client).await }
                })
                .await;
            worker_ready.store(false, Ordering::SeqCst);
            worker_capacity.close();
            if let Err(error) = &result {
                tracing::error!(%error, "Queue worker stopped");
            }
            result
        }));
    }

    let app = if config.isolation_mode == "trusted_shared" {
        flow_like_catalog::initialize();
        tracing::warn!("Shared execution enabled for trusted workflows only");
        executor_router(ExecutorState::with_admission(
            executor_config.clone(),
            capacity.clone(),
        ))
    } else {
        // This process handles trusted queue delivery only. It cannot execute
        // a workflow over HTTP or load workflow catalog code in per_run mode.
        Router::new().route("/health", get(|| async { "healthy" }))
    };
    let readiness = ready.clone();
    let metric_capacity = capacity.clone();
    let max_capacity = config.max_concurrent_executions;
    let app = app
        .route(
            "/ready",
            get(move || {
                let ready = readiness.clone();
                async move {
                    if ready.load(Ordering::SeqCst) {
                        StatusCode::OK
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                }
            }),
        )
        .route(
            "/metrics",
            get(move || {
                let capacity = metric_capacity.clone();
                async move {
                    metrics::set_active_jobs(
                        max_capacity.saturating_sub(capacity.available_permits()),
                    );
                    metrics::handler().await
                }
            }),
        )
        .layer(middleware::from_fn(metrics_middleware));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
    let shutdown_ready = ready.clone();
    let shutdown = async move {
        loop {
            tokio::select! {
                _ = shutdown_signal() => break,
                _ = tokio::time::sleep(Duration::from_millis(250)) => {
                    if !shutdown_ready.load(Ordering::SeqCst) { break; }
                }
            }
        }
    };
    let (stop_http, stopped) = tokio::sync::oneshot::channel();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .into_future();
    tokio::pin!(server);
    let finished = tokio::select! {
        result = &mut server => Some(result),
        _ = shutdown => None,
    };
    // One monotonic deadline starts when admission closes. HTTP and queue
    // draining consume the same allowance, including a stalled HTTP client.
    ready.store(false, Ordering::SeqCst);
    capacity.close();
    let deadline = tokio::time::Instant::now() + shutdown_budget;
    let _ = stop_http.send(());
    let result = drain_execution_work(
        async {
            match finished {
                Some(result) => result,
                None => server.await,
            }
        },
        capacity,
        max_capacity,
        deadline,
    )
    .await;
    if let Some(task) = worker_task {
        task.abort();
    }
    result?;
    Ok(())
}

async fn drain_execution_work<S>(
    server: S,
    capacity: Arc<Semaphore>,
    maximum: usize,
    deadline: tokio::time::Instant,
) -> std::io::Result<()>
where
    S: Future<Output = std::io::Result<()>>,
{
    let http_result = tokio::time::timeout_at(deadline, server).await;
    while capacity.available_permits() < maximum && tokio::time::Instant::now() < deadline {
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(100)).min(deadline),
        )
        .await;
    }
    if http_result.is_err() || capacity.available_permits() < maximum {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Execution drain deadline expired; unsettled queue deliveries remain retained",
        ));
    }
    http_result.expect("HTTP deadline was checked")
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn process_queued_job(
    job: QueuedJob,
    config: config::Config,
    executor: ExecutorConfig,
    client: reqwest::Client,
) -> Result<QueueDisposition, String> {
    let started = Instant::now();
    let result = if config.isolation_mode == "per_run" {
        let dispatch_url = format!(
            "{}/execute",
            config
                .manager_url
                .as_deref()
                .unwrap_or_default()
                .trim_end_matches('/')
        );
        let response = client
            .post(&dispatch_url)
            .header(
                "X-Execution-Manager-Token",
                config.manager_token.as_deref().unwrap_or_default(),
            )
            .header("X-Execution-Queued", "true")
            .json(&job)
            .send()
            .await;
        if response.as_ref().is_ok_and(|response| {
            explicitly_not_admitted(response.status().as_u16(), response.headers())
        }) {
            metrics::record_execution("not_admitted", started.elapsed().as_secs_f64());
            return Ok(QueueDisposition::NotAdmitted {
                retry_after: Duration::from_millis(500),
            });
        }
        let manager_succeeded = response
            .as_ref()
            .is_ok_and(|response| response.status().is_success());
        // Runner stdout is untrusted. On a manager error, only a confirmed
        // cancellation can settle delivery: the API commits it after teardown.
        let url = format!(
            "{}/api/v1/execution/result",
            config
                .api_url
                .as_deref()
                .unwrap_or_default()
                .trim_end_matches('/')
        );
        match client
            .get(url)
            .bearer_auth(&job.executor_jwt)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(status) if status.status().is_success() => {
                match status.json::<TerminalAcknowledgement>().await {
                    Ok(status) if status.matches_transport(&job.run_id, manager_succeeded) => {
                        Ok(QueueDisposition::Completed)
                    }
                    _ => Err("API did not confirm a safely settled terminal execution".into()),
                }
            }
            _ => Err("Could not verify durable terminal execution status".into()),
        }
    } else {
        match ExecutionRequest::try_from(job) {
            Ok(request) => execute(request, executor)
                .await
                .map(|_| QueueDisposition::Completed)
                .map_err(|error| error.to_string()),
            Err(error) => Err(format!("Invalid dispatch payload: {error}")),
        }
    };
    metrics::record_execution(
        if result.is_ok() { "settled" } else { "error" },
        started.elapsed().as_secs_f64(),
    );
    result
}

fn explicitly_not_admitted(status: u16, headers: &reqwest::header::HeaderMap) -> bool {
    matches!(status, 429 | 503)
        && headers
            .get("X-Execution-Admitted")
            .is_some_and(|value| value == "false")
}

#[derive(serde::Deserialize)]
struct TerminalAcknowledgement {
    run_id: String,
    status: String,
    terminal: bool,
}

impl TerminalAcknowledgement {
    fn matches_transport(&self, expected_run: &str, manager_succeeded: bool) -> bool {
        self.matches_run(expected_run) && (manager_succeeded || self.status == "cancelled")
    }

    fn matches_run(&self, expected_run: &str) -> bool {
        self.run_id == expected_run
            && self.terminal
            && matches!(self.status.as_str(), "completed" | "failed" | "cancelled")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_does_not_restart_its_budget_after_http_drains() {
        let capacity = Arc::new(Semaphore::new(1));
        let _unsettled = capacity.clone().acquire_owned().await.unwrap();
        let start = tokio::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_millis(90),
            drain_execution_work(
                async {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    Ok(())
                },
                capacity,
                1,
                start + Duration::from_millis(60),
            ),
        )
        .await
        .expect("drain must use its original deadline");
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn shutdown_bounds_stalled_http_and_waits_for_live_queue_work() {
        let capacity = Arc::new(Semaphore::new(1));
        let result = drain_execution_work(
            std::future::pending(),
            capacity.clone(),
            1,
            tokio::time::Instant::now() + Duration::from_millis(20),
        )
        .await;
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        let permit = capacity.clone().acquire_owned().await.unwrap();
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(permit);
        });
        drain_execution_work(
            async { Ok(()) },
            capacity.clone(),
            1,
            tokio::time::Instant::now() + Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(capacity.available_permits(), 1);
    }

    #[test]
    fn automatic_retry_requires_explicit_non_admission() {
        let mut headers = reqwest::header::HeaderMap::new();
        for status in [200, 400, 429, 500, 502, 503, 504] {
            assert!(!explicitly_not_admitted(status, &headers));
        }
        headers.insert("X-Execution-Admitted", "false".parse().unwrap());
        for status in [200, 400, 429, 500, 502, 503, 504] {
            assert_eq!(
                explicitly_not_admitted(status, &headers),
                matches!(status, 429 | 503)
            );
        }
        headers.insert("X-Execution-Admitted", "true".parse().unwrap());
        assert!(!explicitly_not_admitted(503, &headers));
    }

    #[test]
    fn manager_failure_only_settles_confirmed_hard_cancellation() {
        for status in ["completed", "failed", "cancelled"] {
            let acknowledgement = TerminalAcknowledgement {
                run_id: "run-1".into(),
                status: status.into(),
                terminal: true,
            };
            assert_eq!(
                acknowledgement.matches_transport("run-1", false),
                status == "cancelled"
            );
            assert!(acknowledgement.matches_transport("run-1", true));
        }
    }

    #[test]
    fn queue_settlement_requires_matching_durable_terminal_run() {
        for (run_id, status, terminal, expected) in [
            ("run-1", "completed", true, true),
            ("run-1", "failed", true, true),
            ("run-2", "completed", true, false),
            ("run-1", "running", false, false),
            ("run-1", "running", true, false),
            ("run-1", "completed", false, false),
        ] {
            assert_eq!(
                TerminalAcknowledgement {
                    run_id: run_id.into(),
                    status: status.into(),
                    terminal
                }
                .matches_run("run-1"),
                expected
            );
        }
    }
}
