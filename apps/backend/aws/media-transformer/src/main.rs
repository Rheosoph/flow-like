use aws_config::{retry::RetryConfig, timeout::TimeoutConfig, BehaviorVersion};
use lambda_runtime::{run, service_fn, Error};
use std::time::Duration;

mod event_handler;
use event_handler::function_handler;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Default to warn level to reduce log noise (errors, warnings, fatals only)
    // Can be overridden with RUST_LOG env var
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    let _tracing = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(env_filter)
        .try_init();

    let bucket_name = std::env::var("BUCKET_NAME").map_err(|e| {
        Error::from(format!(
            "Failed to get BUCKET_NAME environment variable: {}",
            e
        ))
    })?;

    let retry_config = RetryConfig::adaptive()
        .with_max_attempts(5)
        .with_initial_backoff(Duration::from_millis(100));

    let timeout_config = TimeoutConfig::builder()
        .operation_timeout(Duration::from_secs(300))
        .operation_attempt_timeout(Duration::from_secs(60))
        .build();

    let config = aws_config::defaults(BehaviorVersion::latest())
        .retry_config(retry_config)
        .timeout_config(timeout_config)
        .load()
        .await;
    let s3_client = aws_sdk_s3::Client::new(&config);

    run(service_fn(move |event| {
        function_handler(event, s3_client.clone(), bucket_name.clone())
    }))
    .await
}
