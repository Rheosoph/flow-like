#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_api::construct_router;
use flow_like_catalog::get_catalog;
use flow_like_secrets::{
    AwsParameterStoreProviderConfig, EnvProviderConfig, ProviderConfig, SecretStoreConfig,
};
use flow_like_storage::object_store::aws::AmazonS3Builder;
use flow_like_types::tokio;
use lambda_http::{Error, run_with_streaming_response};
use std::sync::Arc;
use tracing_subscriber::prelude::*;

#[flow_like_types::tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let sentry_endpoint = std::env::var("SENTRY_ENDPOINT").unwrap_or_default();

    let env_filter = flow_like_api::warn_env_filter();

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

    // Build secret store (AWS Parameter Store + env fallback)
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
    let secrets = flow_like_secrets::SecretStore::new(secret_config.clone())
        .expect("Failed to create secret store");

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
    let state = Arc::new(
        flow_like_api::state::State::new(catalog, Arc::new(cdn_bucket), Some(secret_config)).await,
    );
    let app = construct_router(state);

    run_with_streaming_response(app).await
}
