#[cfg(not(any(all(target_os = "macos", target_arch = "aarch64"), target_os = "ios")))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use dotenv::dotenv;
use flow_like_api::execution::{QueueConfig, QueueWorker, QueuedJob};
use flow_like_catalog::initialize as initialize_catalog;
use flow_like_executor::{
    execute, executor_router, ExecutionRequest, ExecutorConfig, ExecutorState,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("Starting Flow-Like Local Development Runtime");

    // Initialize catalog runtime (ONNX execution providers, etc.)
    initialize_catalog();

    let config = config::Config::from_env()?;
    tracing::info!(
        "Loaded configuration: port={}, max_concurrent={}, queue_worker={}",
        config.port,
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

    // Use the executor's router
    let state = ExecutorState::new(executor_config);
    let app = executor_router(state);

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

    let exec_request = ExecutionRequest::try_from(job)
        .map_err(|e| format!("Failed to convert dispatch payload: {}", e))?;

    let result = execute(exec_request, executor_config).await;

    match &result {
        Ok(exec_result) => {
            tracing::info!(
                job_id = %job_id,
                run_id = %exec_result.run_id,
                status = ?exec_result.status,
                "Job completed"
            );
        }
        Err(e) => {
            tracing::error!(job_id = %job_id, error = %e, "Job failed");
        }
    }

    result.map(|_| ()).map_err(|e| e.to_string())
}
