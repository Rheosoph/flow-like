#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_catalog::initialize as initialize_catalog;
use flow_like_types::tokio;
use flow_like_types_contracts::dispatch::DispatchPayloadRef;
use lambda_runtime::{Error, LambdaEvent, run, service_fn};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};
mod execution;
mod guard;

#[flow_like_types::tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
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

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(env_filter))
        .init();

    // Initialize catalog runtime (ONNX execution providers, etc.)
    initialize_catalog();

    run(service_fn(function_handler)).await
}

async fn function_handler(event: LambdaEvent<DispatchPayloadRef>) -> Result<(), Error> {
    execution::execute(event.payload, &event.context).await
}
