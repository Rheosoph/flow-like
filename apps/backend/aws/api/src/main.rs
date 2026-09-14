#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod compute_attempt;
mod telemetry;

use flow_like_api::construct_router;
use flow_like_api::state::{DbDialect, State};
use flow_like_aws_data::lambda::TokenRefreshLayer;
use flow_like_catalog::get_catalog;
use flow_like_secrets::{
    AwsParameterStoreProviderConfig, EnvProviderConfig, ProviderConfig, SecretStoreConfig,
};
use flow_like_storage::object_store::aws::AmazonS3Builder;
use flow_like_types::tokio;
use lambda_http::{Error, run_with_streaming_response};
use std::sync::Arc;
use tower::{Layer, util::BoxCloneService};
use tracing::Instrument;

#[flow_like_types::tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let telemetry = telemetry::init()?;
    let app = initialize()
        .instrument(
            tracing::info_span!(target: "flow_like::observability", parent: None, "api.initialize"),
        )
        .await;
    telemetry.flush().await;
    run_with_streaming_response(telemetry.wrap(app?)).await
}

async fn initialize() -> Result<
    BoxCloneService<lambda_http::Request, axum::response::Response, std::convert::Infallible>,
    Error,
> {
    // Initialize once per Lambda runtime process. The request loop below reuses
    // this secret store, the router and API state for warm invocations.
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
        flow_like_secrets::SecretStore::new(secret_config).expect("Failed to create secret store"),
    );
    secrets
        .warmup()
        .instrument(tracing::info_span!(target: "flow_like::observability", "secrets.prefetch"))
        .await;

    // CDN bucket (e.g. Cloudflare R2) — separate endpoint + credentials
    let cdn_bucket_name = std::env::var("CDN_BUCKET_NAME").expect("CDN_BUCKET_NAME must be set");
    let cdn_endpoint = std::env::var("CDN_BUCKET_ENDPOINT").ok();
    let cdn_access_key = std::env::var("CDN_BUCKET_ACCESS_KEY_ID").ok();
    let cdn_secret_key = secrets
        .get_secret_string(&flow_like_secrets::SecretRef::new(
            "CDN_BUCKET_SECRET_ACCESS_KEY",
        ))
        .await
        .ok()
        .map(|s| flow_like_secrets::ExposeSecret::expose_secret(&*s).to_string());

    let mut cdn_builder = AmazonS3Builder::new().with_bucket_name(cdn_bucket_name);
    if let Some(ep) = &cdn_endpoint
        && !ep.is_empty()
    {
        cdn_builder = cdn_builder.with_endpoint(ep);
    }
    if let (Some(ak), Some(sk)) = (&cdn_access_key, &cdn_secret_key)
        && !ak.is_empty()
        && !sk.is_empty()
    {
        cdn_builder = cdn_builder
            .with_access_key_id(ak)
            .with_secret_access_key(sk);
    }
    let cdn_bucket =
        flow_like_storage::files::store::FlowLikeStore::AWS(Arc::new(cdn_builder.build().unwrap()));

    let catalog = Arc::new(get_catalog());
    let cdn_bucket = Arc::new(cdn_bucket);
    // A DSQL endpoint selects IAM-token connectivity; anything else keeps the
    // `DATABASE_URL` secret path of every other deployment target.
    let dsql = flow_like_aws_data::dsql::DsqlConfig::from_env()
        .expect("invalid Aurora DSQL configuration");
    match dsql {
        Some(config) => {
            let database = Arc::new(
                flow_like_aws_data::dsql::connect(&config)
                    .await
                    .expect("failed to connect to Aurora DSQL"),
            );
            let state = Arc::new(
                State::new_with_secrets(
                    catalog,
                    cdn_bucket,
                    secrets,
                    Some(database.connection.clone()),
                    Some(DbDialect::Dsql),
                )
                .instrument(
                    tracing::info_span!(target: "flow_like::observability", "api.state.initialize"),
                )
                .await,
            );
            let app =
                TokenRefreshLayer::new(database).layer(construct_router(state.clone()).layer(
                    axum::middleware::from_fn_with_state(state, compute_attempt::record_attempt),
                ));
            Ok(BoxCloneService::new(app))
        }
        None => {
            let state = Arc::new(
                State::new_with_secrets(catalog, cdn_bucket, secrets, None, None)
                    .instrument(
                        tracing::info_span!(target: "flow_like::observability", "api.state.initialize"),
                    )
                    .await,
            );
            Ok(BoxCloneService::new(construct_router(state.clone()).layer(
                axum::middleware::from_fn_with_state(state, compute_attempt::record_attempt),
            )))
        }
    }
}
