//! AWS Lambda Executor with Streaming Support
//!
//! This Lambda function executes flows and streams results back
//! using Lambda's streaming response capability.
//!
//! ## Endpoints
//!
//! - `POST /execute` - Execute with callback (async)
//! - `POST /execute/stream` - Execute with NDJSON streaming
//! - `POST /execute/sse` - Execute with Server-Sent Events
//! - `GET /health` - Health check

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_catalog::initialize as initialize_catalog;
use flow_like_executor::{ExecutorState, executor_router};
use flow_like_types::tokio;
use lambda_http::{Error, run_with_streaming_response, tracing};
use tracing_subscriber::{EnvFilter, prelude::*};

#[flow_like_types::tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    // Initialize Sentry if configured
    let sentry_endpoint = std::env::var("SENTRY_ENDPOINT").unwrap_or_default();
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

    tracing::info!("Starting Flow-Like AWS Executor Lambda");

    // Initialize catalog runtime (ONNX execution providers, etc.)
    initialize_catalog();

    // Create executor state from environment
    let state = ExecutorState::from_env();

    // Build router with all execution endpoints
    let app = executor_router(state);

    // Run with streaming response support
    run_with_streaming_response(app).await
}
