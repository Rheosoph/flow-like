#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_api::construct_router;
use flow_like_catalog::get_catalog;
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

    let content_bucket_name =
        std::env::var("CONTENT_BUCKET")
            .or_else(|_| std::env::var("CONTENT_BUCKET_NAME"))
            .expect("CONTENT_BUCKET or CONTENT_BUCKET_NAME must be set");
    let cdn_bucket_name =
        std::env::var("CDN_BUCKET_NAME").unwrap_or_else(|_| content_bucket_name.clone());
    let meta_bucket_name =
        std::env::var("META_BUCKET")
            .or_else(|_| std::env::var("META_BUCKET_NAME"))
            .unwrap_or_else(|_| content_bucket_name.clone());

    let cdn_endpoint = std::env::var("CDN_BUCKET_ENDPOINT").ok();
    let cdn_access_key = std::env::var("CDN_BUCKET_ACCESS_KEY_ID").ok();
    let cdn_secret_key = std::env::var("CDN_BUCKET_SECRET_ACCESS_KEY").ok();

    let content_endpoint = std::env::var("CONTENT_BUCKET_ENDPOINT").ok();
    let content_access_key = std::env::var("CONTENT_BUCKET_ACCESS_KEY_ID").ok();
    let content_secret_key = std::env::var("CONTENT_BUCKET_SECRET_ACCESS_KEY").ok();

    let meta_endpoint = std::env::var("META_BUCKET_ENDPOINT").ok();
    let meta_access_key = std::env::var("META_BUCKET_ACCESS_KEY_ID").ok();
    let meta_secret_key = std::env::var("META_BUCKET_SECRET_ACCESS_KEY").ok();

    let build_s3 = |name: String,
                    endpoint: &Option<String>,
                    access_key: &Option<String>,
                    secret_key: &Option<String>|
     -> flow_like_storage::files::store::FlowLikeStore {
        let mut builder = AmazonS3Builder::new().with_bucket_name(name);
        if let Some(ep) = endpoint {
            if !ep.is_empty() {
                builder = builder.with_endpoint(ep);
            }
        }
        if let (Some(ak), Some(sk)) = (access_key, secret_key) {
            if !ak.is_empty() && !sk.is_empty() {
                builder = builder.with_access_key_id(ak).with_secret_access_key(sk);
            }
        }
        flow_like_storage::files::store::FlowLikeStore::AWS(Arc::new(builder.build().unwrap()))
    };

    let content_bucket = build_s3(content_bucket_name, &content_endpoint, &content_access_key, &content_secret_key);
    let cdn_bucket = build_s3(cdn_bucket_name, &cdn_endpoint, &cdn_access_key, &cdn_secret_key);
    let meta_bucket = build_s3(meta_bucket_name, &meta_endpoint, &meta_access_key, &meta_secret_key);

    let catalog = Arc::new(get_catalog());
    let state = Arc::new(
        flow_like_api::state::State::new(
            catalog,
            Arc::new(content_bucket),
            Arc::new(cdn_bucket),
            Arc::new(meta_bucket),
        )
        .await,
    );
    let app = construct_router(state);

    run_with_streaming_response(app).await
}
