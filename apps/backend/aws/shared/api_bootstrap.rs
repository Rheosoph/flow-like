//! Process-wide API state shared by the Lambda and ECS entry points: secret
//! store, CDN store, catalog and database. Each entry point decides how the
//! resulting state is served and how a DSQL token stays fresh.

use flow_like_api::state::{DbDialect, State};
use flow_like_aws_data::dsql::{DsqlConfig, DsqlDatabase};
use flow_like_catalog::get_catalog;
use flow_like_secrets::{
    AwsParameterStoreProviderConfig, EnvProviderConfig, ProviderConfig, SecretStoreConfig,
};
use flow_like_storage::object_store::aws::AmazonS3Builder;
use std::sync::Arc;
use tracing::Instrument;

pub type BootstrapError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub struct Api {
    pub state: Arc<State>,
    pub dsql: Option<Arc<DsqlDatabase>>,
}

/// `dsql` selects Aurora DSQL with IAM tokens; `None` keeps the `DATABASE_URL`
/// secret path of every other deployment target.
pub async fn initialize(
    application_name: &str,
    dsql: Option<DsqlConfig>,
) -> Result<Api, BootstrapError> {
    let secret_prefix = std::env::var("SECRET_PREFIX").ok();
    let secret_config = SecretStoreConfig::default()
        .with_provider(ProviderConfig::AwsParameterStore(
            AwsParameterStoreProviderConfig {
                prefix: secret_prefix.clone(),
                with_decryption: true,
                ..Default::default()
            },
        ))
        .with_provider(ProviderConfig::Env(EnvProviderConfig {
            prefix: secret_prefix,
        }));
    let secrets = Arc::new(
        flow_like_secrets::SecretStore::new(secret_config)
            .map_err(|error| format!("failed to create the secret store: {error}"))?,
    );
    secrets
        .warmup()
        .instrument(tracing::info_span!(target: "flow_like::observability", "secrets.prefetch"))
        .await;

    let cdn_bucket = Arc::new(cdn_store(&secrets).await?);
    let catalog = Arc::new(get_catalog());

    let (database, dialect, dsql) = match dsql {
        Some(config) => {
            let database =
                Arc::new(flow_like_aws_data::dsql::connect_as(&config, application_name).await?);
            (
                Some(database.connection.clone()),
                Some(DbDialect::Dsql),
                Some(database),
            )
        }
        None => (None, None, None),
    };
    let state = Arc::new(
        State::new_with_secrets(catalog, cdn_bucket, secrets, database, dialect)
            .instrument(
                tracing::info_span!(target: "flow_like::observability", "api.state.initialize"),
            )
            .await,
    );
    Ok(Api { state, dsql })
}

/// CDN bucket (e.g. Cloudflare R2), with its own endpoint and credentials.
async fn cdn_store(
    secrets: &flow_like_secrets::SecretStore,
) -> Result<flow_like_storage::files::store::FlowLikeStore, BootstrapError> {
    let bucket = std::env::var("CDN_BUCKET_NAME").map_err(|_| "CDN_BUCKET_NAME must be set")?;
    let endpoint = std::env::var("CDN_BUCKET_ENDPOINT").ok();
    let access_key = std::env::var("CDN_BUCKET_ACCESS_KEY_ID").ok();
    let secret_key = secrets
        .get_secret_string(&flow_like_secrets::SecretRef::new(
            "CDN_BUCKET_SECRET_ACCESS_KEY",
        ))
        .await
        .ok()
        .map(|s| flow_like_secrets::ExposeSecret::expose_secret(&*s).to_string());

    let mut builder = AmazonS3Builder::new().with_bucket_name(bucket);
    if let Some(endpoint) = &endpoint
        && !endpoint.is_empty()
    {
        builder = builder.with_endpoint(endpoint);
    }
    if let (Some(access_key), Some(secret_key)) = (&access_key, &secret_key)
        && !access_key.is_empty()
        && !secret_key.is_empty()
    {
        builder = builder
            .with_access_key_id(access_key)
            .with_secret_access_key(secret_key);
    }
    let store = builder
        .build()
        .map_err(|error| format!("failed to build the CDN bucket store: {error}"))?;
    Ok(flow_like_storage::files::store::FlowLikeStore::AWS(
        Arc::new(store),
    ))
}
