//! AWS Lambda query service. Streams framed query responses through Lambda's
//! response streaming; only the API invokes it.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_query::server::{ServiceConfig, auth::TokenVerifier, query_router};
use lambda_http::{
    Error, Request, run_with_streaming_response, service_fn, tower::ServiceExt, tracing,
};
use std::sync::Arc;
use tracing_subscriber::{EnvFilter, prelude::*};

// DataFusion runs one task per partition; a multi-threaded runtime lets them use every vCPU.
#[flow_like_types::tokio::main]
async fn main() -> Result<(), Error> {
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

    tracing::info!("Starting Flow-Like AWS query Lambda");

    let verifier = TokenVerifier::from_env().map_err(|error| {
        tracing::error!(%error, "BACKEND_PUB is missing or invalid; the query service cannot verify API tokens");
        std::io::Error::other(error)
    })?;
    let app = query_router(Arc::new(ServiceConfig::from_env()), verifier);
    let handler = service_fn(move |request: Request| {
        let app = app.clone();
        async move { Ok::<_, Error>(app.oneshot(request).await.expect("Axum routers are infallible")) }
    });

    run_with_streaming_response(handler).await
}
