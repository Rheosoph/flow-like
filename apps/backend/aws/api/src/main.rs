#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[path = "../../shared/api_bootstrap.rs"]
mod bootstrap;
mod compute_attempt;
mod telemetry;

use flow_like_api::construct_router;
use flow_like_aws_data::lambda::TokenRefreshLayer;
use flow_like_types::tokio;
use lambda_http::{Error, run_with_streaming_response};
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
    let dsql = flow_like_aws_data::dsql::DsqlConfig::from_env()?;
    let api =
        bootstrap::initialize(flow_like_aws_data::dsql::DEFAULT_APPLICATION_NAME, dsql).await?;
    let router = construct_router(api.state.clone()).layer(axum::middleware::from_fn_with_state(
        api.state,
        compute_attempt::record_attempt,
    ));
    // A frozen Lambda's timers do not tick, so the DSQL token is checked per request.
    Ok(match api.dsql {
        Some(database) => BoxCloneService::new(TokenRefreshLayer::new(database).layer(router)),
        None => BoxCloneService::new(router),
    })
}
