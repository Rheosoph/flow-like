#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_compiler::{process_job, CompilationJob, CompilerConfig};
use flow_like_types::dispatch::CompilationJobRef;
use flow_like_types::reqwest;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer as _};

const DEFAULT_TASK_TIMEOUT_SECS: u64 = 3600; // 1 hour

/// ECS RunTask compiler.
///
/// EventBridge Pipe triggers this as an ECS task with SQS message(s)
/// injected via container overrides:
///   containerOverrides.environment = [{ name: "COMPILATION_JOB", value: <json> }]
///
/// `COMPILATION_JOB` may contain:
/// - A single `CompilationJob` (inline JSON)
/// - A `CompilationJobRef::Remote` (`{ "remote_url": "..." }`)
/// - A JSON array of `CompilationJob` objects
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

    // Dump all env var names (not values) for debugging the EventBridge Pipe handover
    let env_names: Vec<String> = std::env::vars().map(|(k, _)| k).collect();
    tracing::info!(env_vars = ?env_names, "Environment variables present");

    let job_json = match std::env::var("COMPILATION_JOB") {
        Ok(v) if !v.is_empty() => {
            tracing::info!(
                len = v.len(),
                preview = %v.char_indices().nth(200).map(|(i, _)| &v[..i]).unwrap_or(&v),
                "COMPILATION_JOB received"
            );
            v
        }
        Ok(v) => {
            tracing::error!(len = v.len(), "COMPILATION_JOB is present but empty");
            // Give CloudWatch time to flush before exit
            tokio::time::sleep(Duration::from_secs(5)).await;
            return std::process::ExitCode::FAILURE;
        }
        Err(_) => {
            tracing::error!(
                "COMPILATION_JOB env var is not set. \
                 EventBridge Pipe may not be substituting <$.body> correctly. \
                 Check pipe configuration and SQS message format."
            );
            // Give CloudWatch time to flush before exit
            tokio::time::sleep(Duration::from_secs(5)).await;
            return std::process::ExitCode::FAILURE;
        }
    };

    let jobs: Vec<CompilationJob> = match resolve_compilation_jobs(&job_json).await {
        Ok(jobs) => jobs,
        Err(e) => {
            tracing::error!(
                error = %e,
                raw_len = job_json.len(),
                raw_preview = %job_json.char_indices().nth(500).map(|(i, _)| &job_json[..i]).unwrap_or(&job_json),
                "Failed to resolve COMPILATION_JOB"
            );
            tokio::time::sleep(Duration::from_secs(5)).await;
            return std::process::ExitCode::FAILURE;
        }
    };

    if jobs.is_empty() {
        tracing::error!("No valid compilation jobs found after resolution");
        tokio::time::sleep(Duration::from_secs(5)).await;
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

/// Resolve `COMPILATION_JOB` env var into concrete `CompilationJob` values.
///
/// Accepts:
/// - `CompilationJobRef::Remote { remote_url }` → fetches full job from presigned URL
/// - `CompilationJobRef::Inline(job)` / raw `CompilationJob` → used directly
/// - `Vec<CompilationJob>` → batch mode
async fn resolve_compilation_jobs(json: &str) -> Result<Vec<CompilationJob>, String> {
    // Try as a remote reference first (compact `{ "remote_url": "..." }`)
    if let Ok(job_ref) = serde_json::from_str::<CompilationJobRef>(json) {
        match job_ref {
            CompilationJobRef::Remote { remote_url } => {
                tracing::info!(url = %remote_url, "Fetching remote compilation job");
                let response = tokio::time::timeout(
                    Duration::from_secs(30),
                    reqwest::get(&remote_url),
                )
                .await
                .map_err(|_| "Remote job fetch timed out".to_string())?
                .map_err(|e| format!("Failed to fetch remote job: {e}"))?;
                if !response.status().is_success() {
                    return Err(format!("HTTP {} from job URL", response.status()));
                }
                let body = response
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read response: {e}"))?;
                return parse_inline_jobs(&body);
            }
            CompilationJobRef::Inline(job) => return Ok(vec![job]),
        }
    }

    // Fall back to parsing as Vec<CompilationJob> or single CompilationJob
    parse_inline_jobs(json)
}

fn parse_inline_jobs(json: &str) -> Result<Vec<CompilationJob>, String> {
    if let Ok(jobs) = serde_json::from_str::<Vec<CompilationJob>>(json) {
        if !jobs.is_empty() {
            return Ok(jobs);
        }
    }
    match serde_json::from_str::<CompilationJob>(json) {
        Ok(job) => Ok(vec![job]),
        Err(e) => Err(format!("Failed to parse compilation job(s): {e}")),
    }
}
