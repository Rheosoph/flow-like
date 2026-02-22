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
        std::env::var("CONTENT_BUCKET").or_else(|_| std::env::var("CDN_BUCKET_NAME")).unwrap();
    let cdn_bucket_name =
        std::env::var("CDN_BUCKET_NAME").unwrap_or_else(|_| content_bucket_name.clone());
    let meta_bucket_name =
        std::env::var("META_BUCKET").unwrap_or_else(|_| content_bucket_name.clone());

    let bucket_endpoint = std::env::var("CDN_BUCKET_ENDPOINT").ok();
    let bucket_access_key = std::env::var("CDN_BUCKET_ACCESS_KEY_ID").ok();
    let bucket_secret_key = std::env::var("CDN_BUCKET_SECRET_ACCESS_KEY").ok();

    let build_s3 = |name: String| -> flow_like_storage::files::store::FlowLikeStore {
        let mut builder = AmazonS3Builder::new().with_bucket_name(name);
        if let Some(endpoint) = &bucket_endpoint {
            if !endpoint.is_empty() {
                builder = builder.with_endpoint(endpoint);
            }
        }
        if let (Some(ak), Some(sk)) = (&bucket_access_key, &bucket_secret_key) {
            if !ak.is_empty() && !sk.is_empty() {
                builder = builder.with_access_key_id(ak).with_secret_access_key(sk);
            }
        }
        flow_like_storage::files::store::FlowLikeStore::AWS(Arc::new(builder.build().unwrap()))
    };

    let content_bucket = build_s3(content_bucket_name);
    let cdn_bucket = build_s3(cdn_bucket_name);
    let meta_bucket = build_s3(meta_bucket_name);

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
