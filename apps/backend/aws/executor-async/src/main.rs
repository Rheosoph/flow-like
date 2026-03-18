#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use aws_lambda_events::sqs::{BatchItemFailure, SqsBatchResponse, SqsEvent};
use flow_like_catalog::initialize as initialize_catalog;
use flow_like_types::tokio;
use lambda_runtime::{Error, LambdaEvent, run, service_fn, tracing};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};
mod execution;

#[flow_like_types::tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let sentry_endpoint = std::env::var("SENTRY_ENDPOINT").unwrap_or_default();

    // Default to warn level for CloudWatch logs to reduce noise
    // Explicitly filter out verbose logs from dependencies
    // Can be overridden with RUST_LOG env var
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("warn")
            .add_directive("hyper=warn".parse().unwrap())
            .add_directive("hyper_util=warn".parse().unwrap())
            .add_directive("rustls=warn".parse().unwrap())
            .add_directive("tokio=warn".parse().unwrap())
            .add_directive("h2=warn".parse().unwrap())
            .add_directive("tower=warn".parse().unwrap())
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

    // Initialize catalog runtime (ONNX execution providers, etc.)
    initialize_catalog();

    run(service_fn(sqs_function_handler)).await
}

pub async fn sqs_function_handler(event: LambdaEvent<SqsEvent>) -> Result<SqsBatchResponse, Error> {
    let mut batch_item_failures = Vec::new();

    for record in event.payload.records.iter() {
        let body = record.body.as_deref().unwrap_or_default();

        // ... Process the message, if it fails, add to batch_item_failures
        match execution::execute(body).await {
            Ok(_) => {
                continue;
            }
            Err(e) => {
                tracing::error!("Failed to process message: {}", e);
                batch_item_failures.push(BatchItemFailure {
                    item_identifier: record.message_id.as_ref().unwrap().clone(),
                });
            }
        }
    }

    Ok(SqsBatchResponse {
        batch_item_failures,
    })
}
