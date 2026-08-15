use aws_config::{retry::RetryConfig, timeout::TimeoutConfig, SdkConfig};
use aws_lambda_events::sqs::SqsEvent;
use aws_sdk_dynamodb::Client as DynamoClient;
use flow_like_secrets::{
    AwsParameterStoreProviderConfig, ExposeSecret, ProviderConfig, SecretRef, SecretStore,
    SecretStoreConfig,
};
use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use sea_orm::{ConnectOptions, Database};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
mod entity;
mod event_handler;
use std::time::Duration;

async fn resolve_database_url() -> String {
    match std::env::var("DATABASE_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            let secret_prefix = std::env::var("SECRET_PREFIX").ok();
            let secret_config = SecretStoreConfig::default().with_provider(
                ProviderConfig::AwsParameterStore(AwsParameterStoreProviderConfig {
                    prefix: secret_prefix,
                    with_decryption: true,
                    ..Default::default()
                }),
            );
            let secrets =
                SecretStore::new(secret_config).expect("Failed to create secret store");
            let value = secrets
                .get_secret_string(&SecretRef::new("DATABASE_URL"))
                .await
                .expect("DATABASE_URL must be set via env or under SECRET_PREFIX");
            ExposeSecret::expose_secret(&*value).to_string()
        }
    }
}

fn create_dynamo_client(config: &SdkConfig) -> DynamoClient {
    let retry_config = RetryConfig::standard()
        .with_max_attempts(5)
        .with_initial_backoff(Duration::from_millis(100));

    let timeout_config = TimeoutConfig::builder()
        .operation_timeout(Duration::from_secs(30))
        .build();

    let dynamo_config = aws_sdk_dynamodb::config::Builder::from(config)
        .retry_config(retry_config)
        .timeout_config(timeout_config)
        .build();

    DynamoClient::from_conf(dynamo_config)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(env_filter)
        .try_init();

    let db_url = resolve_database_url().await;
    let mut opt = ConnectOptions::new(db_url.to_owned());

    opt.max_connections(100)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(8));

    let db = Database::connect(opt)
        .await
        .expect("Failed to connect to database");

    let config = aws_config::load_from_env().await;
    let dynamo = create_dynamo_client(&config);

    run(service_fn(|event: LambdaEvent<SqsEvent>| {
        event_handler::function_handler(event, dynamo.clone(), db.clone())
    }))
    .await
}
