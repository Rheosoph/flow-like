#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_catalog::initialize as initialize_catalog;
use flow_like_executor::{ExecutionRequest, ExecutorConfig, resolve_payload_from_str};
use flow_like_types::dispatch::DispatchPayload;
use std::time::Duration;
use tracing_subscriber::{EnvFilter, Layer as _, layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_TASK_TIMEOUT_SECS: u64 = 3600;

/// ECS RunTask executor for long-running flow executions.
///
/// EventBridge Pipe triggers this as an ECS task with SQS message(s)
/// injected via container overrides:
///   containerOverrides.environment = [{ name: "EXECUTION_JOB", value: <json> }]
///
/// `EXECUTION_JOB` may contain:
/// - A single `DispatchPayload` (inline JSON)
/// - A single `DispatchPayloadRef::Remote` (`{ "remote_url": "..." }`)
/// - A JSON array of either of the above (batch mode)
///
/// Remote payloads are transparently resolved via presigned URL fetch.
///
/// Exit code 0 = all succeeded, non-zero = at least one failure.
#[tokio::main]
async fn main() -> std::process::ExitCode {
    let sentry_endpoint = std::env::var("SENTRY_ENDPOINT").unwrap_or_default();
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info")
            .add_directive("hyper=warn".parse().unwrap())
            .add_directive("hyper_util=warn".parse().unwrap())
            .add_directive("rustls=warn".parse().unwrap())
            .add_directive("tokio=warn".parse().unwrap())
            .add_directive("h2=warn".parse().unwrap())
    });

    let _sentry_guard = if sentry_endpoint.is_empty() {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_filter(env_filter))
            .init();
        None
    } else {
        let guard = sentry::init((
            sentry_endpoint,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                traces_sample_rate: 0.3,
                ..Default::default()
            },
        ));
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_filter(env_filter))
            .with(sentry_tracing::layer())
            .init();
        Some(guard)
    };

    tracing::info!("Starting Flow-Like ECS Executor Task");

    initialize_catalog();

    let job_json = match std::env::var("EXECUTION_JOB") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            tracing::error!("EXECUTION_JOB env var missing or empty");
            return std::process::ExitCode::FAILURE;
        }
    };

    let items: Vec<String> = match serde_json::from_str::<Vec<serde_json::Value>>(&job_json) {
        Ok(arr) => arr.into_iter().map(|v| v.to_string()).collect(),
        Err(_) => vec![job_json],
    };

    if items.is_empty() {
        tracing::error!("No valid execution jobs found");
        return std::process::ExitCode::FAILURE;
    }

    tracing::info!(count = items.len(), "Processing execution jobs");

    let task_timeout_secs: u64 = std::env::var("ECS_TASK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TASK_TIMEOUT_SECS);
    let task_timeout = Duration::from_secs(task_timeout_secs);

    tracing::info!(timeout_secs = task_timeout_secs, "Task hard timeout");

    let config = ExecutorConfig::from_env();
    let mut had_failure = false;

    for (idx, item_json) in items.iter().enumerate() {
        tracing::info!(index = idx, "Resolving execution payload");

        let payload: DispatchPayload = match resolve_payload_from_str(item_json).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(index = idx, error = %e, "Failed to resolve payload");
                had_failure = true;
                continue;
            }
        };

        tracing::info!(
            job_id = %payload.job_id,
            run_id = %payload.run_id,
            app_id = %payload.app_id,
            board_id = %payload.board_id,
            "Processing execution job"
        );

        let request = match ExecutionRequest::try_from(payload) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(index = idx, error = %e, "Failed to convert payload to request");
                had_failure = true;
                continue;
            }
        };

        let result =
            match tokio::time::timeout(task_timeout, flow_like_executor::execute(request, config.clone())).await {
                Ok(Ok(result)) => result,
                Ok(Err(e)) => {
                    tracing::error!(index = idx, error = %e, "Execution failed");
                    had_failure = true;
                    continue;
                }
                Err(_) => {
                    tracing::error!(index = idx, "Execution timed out after {task_timeout_secs}s");
                    had_failure = true;
                    continue;
                }
            };

        tracing::info!(
            run_id = %result.run_id,
            status = ?result.status,
            duration_ms = result.duration_ms,
            "Execution completed"
        );
    }

    if had_failure {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
