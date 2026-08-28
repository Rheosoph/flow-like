#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::{
    Router,
    body::Body,
    extract::MatchedPath,
    http::Request,
    middleware::{self, Next},
    response::IntoResponse,
};
use flow_like_api::cache::sweeper::{CacheSweeperConfig, spawn_cache_sweeper};
use flow_like_api::channel::{ChannelSweeperConfig, spawn_channel_sweeper};
use flow_like_api::execution::{RunSweeperConfig, spawn_run_sweeper};
use flow_like_api::telemetry::{
    TelemetryAlertConfig, TelemetryRollupConfig, TelemetrySweeperConfig,
    spawn_telemetry_alert_evaluator, spawn_telemetry_rollup, spawn_telemetry_sweeper,
};
use flow_like_api::{construct_router, state::State};
use flow_like_catalog::get_catalog;
use std::{sync::Arc, time::Instant};
use tower_http::cors::CorsLayer;

mod config;
mod metrics;
mod storage;

async fn metrics_middleware(request: Request<Body>, next: Next) -> impl IntoResponse {
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let method = request.method().to_string();

    let start = Instant::now();
    let response = next.run(request).await;
    let duration = start.elapsed().as_secs_f64();

    let status = response.status().as_u16().to_string();

    metrics::counter!(
        "http_requests_total",
        "method" => method.clone(),
        "path" => path.clone(),
        "status" => status.clone()
    )
    .increment(1);

    metrics::histogram!("http_request_duration_seconds", "method" => method, "path" => path)
        .record(duration);

    response
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    metrics::init_telemetry();

    tracing::info!("Starting Flow-Like Docker Compose API Service");

    let config = config::Config::from_env()?;
    tracing::info!("Loaded configuration: provider={}", config.provider());

    if !flow_like_api::execution::is_jwt_configured() {
        tracing::warn!(
            "Execution JWT keys not configured. \
            Generate keys using tools/gen-execution-keys.sh and set \
            BACKEND_KEY, BACKEND_PUB environment variables."
        );
    }

    let catalog = get_catalog();

    let cdn_bucket = storage::create_cdn_store(&config)?;

    let state = Arc::new(State::new(Arc::new(catalog), Arc::new(cdn_bucket), None).await);

    let _sweeper_handle =
        spawn_run_sweeper(Arc::new(state.db.clone()), RunSweeperConfig::from_env());
    let _channel_sweeper_handle =
        spawn_channel_sweeper(Arc::new(state.db.clone()), ChannelSweeperConfig::from_env());

    // Only spawns for backends without native expiry; the others no-op and log why.
    let _cache_sweeper_handle = state
        .cache_store
        .clone()
        .and_then(|store| spawn_cache_sweeper(store, CacheSweeperConfig::from_env()));

    // Spawned before the sweeper so the aggregates lead the deletions. The
    // ordering guarantee itself lives in the sweeper, which clamps every raw
    // retention cutoff to the last fully rolled-up day.
    let _telemetry_rollup_handle = spawn_telemetry_rollup(
        Arc::new(state.db.clone()),
        TelemetryRollupConfig::from_env(),
    );

    let _telemetry_sweeper_handle = spawn_telemetry_sweeper(
        Arc::new(state.db.clone()),
        TelemetrySweeperConfig::from_env(),
    );

    let _telemetry_alert_handle =
        spawn_telemetry_alert_evaluator(state.clone(), TelemetryAlertConfig::from_env());

    let app = Router::new()
        .merge(construct_router(state.clone()))
        .layer(middleware::from_fn(metrics_middleware))
        .layer(CorsLayer::permissive());

    let metrics_port = std::env::var("METRICS_PORT").unwrap_or_else(|_| "9090".to_string());
    let metrics_app = Router::new().route("/metrics", axum::routing::get(metrics::handler));

    let addr = format!("0.0.0.0:{}", config.port);
    let metrics_addr = format!("0.0.0.0:{}", metrics_port);

    tracing::info!("API listening on {}", addr);
    tracing::info!("Metrics listening on {}", metrics_addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let metrics_listener = tokio::net::TcpListener::bind(&metrics_addr).await?;

    tokio::select! {
        res = axum::serve(listener, app) => res?,
        res = axum::serve(metrics_listener, metrics_app) => res?,
    }

    Ok(())
}
