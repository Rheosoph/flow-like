#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use aws_lambda_events::sqs::{BatchItemFailure, SqsBatchResponse, SqsEvent};
use flow_like_compiler::{process_job, CompilationJob, CompilerConfig};
use lambda_runtime::{run, service_fn, tracing, Error, LambdaEvent};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

static CONFIG: OnceLock<CompilerConfig> = OnceLock::new();

/// Buffer reserved for callback/cleanup after compilation finishes.
const DEADLINE_BUFFER: Duration = Duration::from_secs(30);

fn get_config() -> &'static CompilerConfig {
    CONFIG.get().expect("Config not initialized")
}

fn remaining_secs(deadline: SystemTime) -> Option<u64> {
    deadline
        .duration_since(SystemTime::now())
        .ok()
        .map(|d| d.as_secs().saturating_sub(DEADLINE_BUFFER.as_secs()))
        .filter(|&s| s > 0)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let sentry_endpoint = std::env::var("SENTRY_ENDPOINT").unwrap_or_default();
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("warn")
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

    tracing::info!("Starting Flow-Like AWS Compiler Lambda (SQS)");

    let _ = CONFIG.set(CompilerConfig::from_env());

    run(service_fn(sqs_handler)).await
}

async fn sqs_handler(event: LambdaEvent<SqsEvent>) -> Result<SqsBatchResponse, Error> {
    let mut failures = Vec::new();
    let base_config = get_config();
    let deadline = event.context.deadline();

    for record in event.payload.records.iter() {
        let Some(secs_left) = remaining_secs(deadline) else {
            tracing::warn!("Lambda deadline approaching \u{2014} skipping remaining records");
            failures.push(BatchItemFailure {
                item_identifier: record.message_id.as_ref().unwrap().clone(),
            });
            continue;
        };

        let config = base_config
            .clone()
            .with_timeout_secs(secs_left.min(base_config.compilation_timeout_secs));

        let body = record.body.as_deref().unwrap_or_default();

        let job: CompilationJob = match serde_json::from_str(body) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "Failed to parse SQS message as CompilationJob");
                failures.push(BatchItemFailure {
                    item_identifier: record.message_id.as_ref().unwrap().clone(),
                });
                continue;
            }
        };

        tracing::info!(
            job_id = %job.job_id,
            timeout_secs = secs_left,
            "Processing compilation job"
        );

        let result = process_job(job, &config).await;

        if result.error.is_some() {
            tracing::error!(
                job_id = %result.job_id,
                error = ?result.error,
                "Compilation job failed"
            );
            failures.push(BatchItemFailure {
                item_identifier: record.message_id.as_ref().unwrap().clone(),
            });
        }
    }

    Ok(SqsBatchResponse {
        batch_item_failures: failures,
    })
}
