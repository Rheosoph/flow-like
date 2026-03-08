#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_compiler::{process_job, CompilationJob, CompilerConfig};
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer as _};

const DEFAULT_TASK_TIMEOUT_SECS: u64 = 3600; // 1 hour

/// ECS RunTask compiler.
///
/// EventBridge Pipe triggers this as an ECS task with SQS message(s)
/// injected via container overrides:
///   containerOverrides.environment = [{ name: "COMPILATION_JOB", value: <json> }]
///
/// `COMPILATION_JOB` may be a single `CompilationJob` or a `Vec<CompilationJob>`
/// (EventBridge Pipe can batch multiple SQS messages into one RunTask).
///
/// Exit code 0 = all succeeded, non-zero = at least one failure.
#[tokio::main]
async fn main() -> std::process::ExitCode {
    let sentry_endpoint = std::env::var("SENTRY_ENDPOINT").unwrap_or_default();
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info")
            .add_directive("hyper=warn".parse().unwrap())
            .add_directive("rustls=warn".parse().unwrap())
            .add_directive("tokio=warn".parse().unwrap())
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

    tracing::info!("Starting Flow-Like ECS Compiler Task");

    let job_json = match std::env::var("COMPILATION_JOB") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            tracing::error!("COMPILATION_JOB env var missing or empty");
            return std::process::ExitCode::FAILURE;
        }
    };

    let jobs: Vec<CompilationJob> = serde_json::from_str::<Vec<CompilationJob>>(&job_json)
        .unwrap_or_else(
            |_| match serde_json::from_str::<CompilationJob>(&job_json) {
                Ok(single) => vec![single],
                Err(e) => {
                    tracing::error!(error = %e, "Failed to parse COMPILATION_JOB");
                    Vec::new()
                }
            },
        );

    if jobs.is_empty() {
        tracing::error!("No valid compilation jobs found");
        return std::process::ExitCode::FAILURE;
    }

    tracing::info!(count = jobs.len(), "Processing compilation jobs");

    let task_timeout_secs: u64 = std::env::var("ECS_TASK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TASK_TIMEOUT_SECS);
    let task_timeout = Duration::from_secs(task_timeout_secs);

    tracing::info!(timeout_secs = task_timeout_secs, "Task hard timeout");

    let config = CompilerConfig::from_env();
    let mut had_failure = false;

    for job in jobs {
        tracing::info!(
            job_id = %job.job_id,
            package_id = %job.package_id,
            version = %job.version,
            targets = job.targets.len(),
            "Processing compilation job"
        );

        let result = match tokio::time::timeout(task_timeout, process_job(job.clone(), &config))
            .await
        {
            Ok(r) => r,
            Err(_) => {
                tracing::error!(job_id = %job.job_id, "Compilation timed out after {task_timeout_secs}s");
                had_failure = true;
                continue;
            }
        };

        if let Some(ref err) = result.error {
            tracing::error!(job_id = %result.job_id, error = %err, "Compilation failed");
            had_failure = true;
        } else {
            tracing::info!(job_id = %result.job_id, "Compilation completed successfully");
        }
    }

    if had_failure {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
