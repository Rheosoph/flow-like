#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::{
    body::Body,
    http::Request,
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use flow_like_api::execution::{QueueConfig, QueueWorker, QueuedJob};
use flow_like_catalog::initialize as initialize_catalog;
use flow_like_executor::{
    execute, executor_router, ExecutionRequest, ExecutorConfig, ExecutorState,
};
use std::time::Instant;

mod config;
mod metrics;

async fn metrics_middleware(request: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let method = request.method().to_string();
    let path = request.uri().path().to_string();

    // Track active execution jobs
    let is_execute = path.starts_with("/execute");
    if is_execute {
        metrics::increment_active_jobs();
    }

    let response = next.run(request).await;

    let duration = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();

    // Record HTTP metrics
    metrics::record_http_request(&method, &path, status, duration);

    // Record execution metrics for execute endpoints
    if is_execute {
        metrics::decrement_active_jobs();
        let exec_status = if (200..300).contains(&status) {
            "success"
        } else {
            "error"
        };
        metrics::record_execution(exec_status, duration);
    }

    response
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    metrics::init_telemetry();

    tracing::info!("Starting Flow-Like Docker Compose Runtime");

    // Initialize catalog runtime (ONNX execution providers, etc.)
    initialize_catalog();

    let config = config::Config::from_env()?;
    tracing::info!(
        "Loaded configuration: max_concurrent={}, queue_worker={}",
        config.max_concurrent_executions,
        config.queue_worker_enabled
    );

    let executor_config = ExecutorConfig::from_env();

    // Start queue worker if enabled
    if config.queue_worker_enabled {
        let queue_config = QueueConfig {
            redis_url: config
                .redis_url
                .clone()
                .unwrap_or_else(|| "redis://localhost:6379".into()),
            queue_name: config.redis_queue_name.clone(),
            concurrency: config.max_concurrent_executions,
            poll_timeout_secs: 30,
        };

        let worker_executor_config = executor_config.clone();

        tokio::spawn(async move {
            match QueueWorker::new(queue_config).await {
                Ok(worker) => {
                    tracing::info!("Queue worker started");
                    let _ = worker
                        .run(move |job: QueuedJob| {
                            let executor_config = worker_executor_config.clone();
                            async move { process_queued_job(job, executor_config).await }
                        })
                        .await;
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to start queue worker");
                }
            }
        });
    }

    // Create executor state with metrics middleware
    let state = ExecutorState::new(executor_config);
    let app = executor_router(state)
        .route("/metrics", get(metrics::handler))
        .layer(middleware::from_fn(metrics_middleware));

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("Runtime listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Process a job from the Redis queue
async fn process_queued_job(job: QueuedJob, executor_config: ExecutorConfig) -> Result<(), String> {
    let job_id = job.job_id.clone();
    tracing::info!(job_id = %job_id, run_id = %job.run_id, "Processing queued job");

    let start_time = std::time::Instant::now();

    let exec_request = ExecutionRequest::try_from(job)
        .map_err(|e| format!("Failed to convert dispatch payload: {}", e))?;

    let result = execute(exec_request, executor_config).await;
    let duration_secs = start_time.elapsed().as_secs_f64();

    match &result {
        Ok(exec_result) => {
            tracing::info!(
                job_id = %job_id,
                run_id = %exec_result.run_id,
                status = ?exec_result.status,
                "Job completed"
            );
            metrics::record_execution("success", duration_secs);
        }
        Err(e) => {
            tracing::error!(job_id = %job_id, error = %e, "Job failed");
            metrics::record_execution("error", duration_secs);
        }
    }

    result.map(|_| ()).map_err(|e| e.to_string())
}
