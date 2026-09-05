use aws_config::{retry::RetryConfig, timeout::TimeoutConfig, SdkConfig};
use aws_lambda_events::sqs::SqsEvent;
use aws_sdk_dynamodb::Client as DynamoClient;
use aws_sdk_s3::Client as S3Client;
use flow_like_aws_data::dsql::{self, DsqlConfig, DsqlDatabase};
use flow_like_db::DbDialect;
use flow_like_secrets::{
    AwsParameterStoreProviderConfig, ExposeSecret, ProviderConfig, SecretRef, SecretStore,
    SecretStoreConfig,
};
use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
mod accounting;
mod event_handler;
use std::sync::Arc;
use std::time::Duration;

const APPLICATION_NAME: &str = "flow-like-aws-file-tracker";

/// The pool this process writes through, plus the DSQL token rotor when the
/// endpoint selected IAM connectivity.
#[derive(Clone)]
struct DatabaseHandle {
    connection: DatabaseConnection,
    dialect: DbDialect,
    dsql: Option<Arc<DsqlDatabase>>,
}

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
            let secrets = SecretStore::new(secret_config).expect("Failed to create secret store");
            let value = secrets
                .get_secret_string(&SecretRef::new("DATABASE_URL"))
                .await
                .expect("DATABASE_URL must be set via env or under SECRET_PREFIX");
            ExposeSecret::expose_secret(&*value).to_string()
        }
    }
}

/// A DSQL endpoint selects IAM-token connectivity; anything else keeps the
/// `DATABASE_URL` path.
async fn connect_database() -> DatabaseHandle {
    let dsql = DsqlConfig::from_env().expect("invalid Aurora DSQL configuration");
    match dsql {
        Some(config) => {
            let database = Arc::new(
                dsql::connect_as(&config, APPLICATION_NAME)
                    .await
                    .expect("failed to connect to Aurora DSQL"),
            );
            DatabaseHandle {
                connection: database.connection.clone(),
                dialect: DbDialect::Dsql,
                dsql: Some(database),
            }
        }
        None => {
            let db_url = resolve_database_url().await;
            let mut opt = ConnectOptions::new(db_url);
            opt.max_connections(100)
                .min_connections(1)
                .connect_timeout(Duration::from_secs(8));
            let connection = Database::connect(opt)
                .await
                .expect("Failed to connect to database");
            let dialect = DbDialect::resolve(None, &connection).await;
            DatabaseHandle {
                connection,
                dialect,
                dsql: None,
            }
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

    let db = connect_database().await;

    let config = aws_config::load_from_env().await;
    let dynamo = create_dynamo_client(&config);
    let s3 = S3Client::new(&config);
    let legacy = event_handler::LegacyBaseline::from_env()
        .expect("invalid legacy file accounting configuration");

    run(service_fn(|event: LambdaEvent<SqsEvent>| {
        let db = db.clone();
        let dynamo = dynamo.clone();
        let s3 = s3.clone();
        let legacy = legacy.clone();
        async move {
            // A frozen Lambda's timers do not tick, so the token is checked per
            // invocation; a failed mint surfaces on the query if it matters.
            if let Some(dsql) = &db.dsql {
                if let Err(error) = dsql.refresh_token_if_stale().await {
                    tracing::warn!(%error, "Aurora DSQL token refresh failed before invocation");
                }
            }
            event_handler::function_handler(event, dynamo, s3, legacy, db.connection, db.dialect)
                .await
        }
    }))
    .await
}
