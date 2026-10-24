//! Process-wide API state shared by the Lambda and ECS entry points: secret
//! store, CDN store, catalog and database. Each entry point decides how the
//! resulting state is served and how a DSQL token stays fresh.

use flow_like_api::state::{DbDialect, NodeCatalog, State};
use flow_like_aws_data::dsql::{DsqlConfig, DsqlDatabase};
use flow_like_catalog::get_catalog;
use flow_like_secrets::{
    AwsParameterStoreProviderConfig, EnvProviderConfig, ProviderConfig, SecretStore,
    SecretStoreConfig,
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
///
/// The catalog builds on a blocking thread from the start, while the secret
/// prefetch and the database connection share the network wait.
pub async fn initialize(
    application_name: &str,
    dsql: Option<DsqlConfig>,
) -> Result<Api, BootstrapError> {
    let catalog = NodeCatalog::spawn(get_catalog);
    let secrets = Arc::new(secret_store()?);
    let ((), dsql) = flow_like_types::tokio::join!(
        secrets.warmup().instrument(tracing::info_span!(
            target: "flow_like::observability",
            "api.init.secrets"
        )),
        connect_dsql(dsql, application_name),
    );
    let dsql = dsql?;

    let cdn_bucket = Arc::new(
        cdn_store(&secrets)
            .instrument(tracing::info_span!(
                target: "flow_like::observability",
                "api.init.cdn_store"
            ))
            .await?,
    );
    let state = Arc::new(
        State::new_with_secrets(
            catalog,
            cdn_bucket,
            secrets,
            dsql.as_ref().map(|database| database.connection.clone()),
            dsql.as_ref().map(|_| DbDialect::Dsql),
        )
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "api.init.state"
        ))
        .await,
    );
    Ok(Api { state, dsql })
}

fn secret_store() -> Result<SecretStore, BootstrapError> {
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
    SecretStore::new(secret_config)
        .map_err(|error| format!("failed to create the secret store: {error}").into())
}

async fn connect_dsql(
    config: Option<DsqlConfig>,
    application_name: &str,
) -> Result<Option<Arc<DsqlDatabase>>, BootstrapError> {
    let Some(config) = config else {
        return Ok(None);
    };
    let database = flow_like_aws_data::dsql::connect_as(&config, application_name).await?;
    Ok(Some(Arc::new(database)))
}

/// CDN bucket (e.g. Cloudflare R2), with its own endpoint and credentials.
async fn cdn_store(
    secrets: &SecretStore,
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
