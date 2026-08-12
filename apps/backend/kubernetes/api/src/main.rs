#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::Router;
use flow_like_api::cache::sweeper::{CacheSweeperConfig, spawn_cache_sweeper};
use flow_like_api::execution::{RunSweeperConfig, spawn_run_sweeper};
use flow_like_api::telemetry::{
    TelemetryAlertConfig, TelemetryRollupConfig, TelemetrySweeperConfig,
    spawn_telemetry_alert_evaluator, spawn_telemetry_rollup, spawn_telemetry_sweeper,
};
use flow_like_api::{construct_router, state::State};
use flow_like_catalog::get_catalog;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

mod config;
mod health;
mod metrics;
mod storage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    metrics::init_telemetry();

    tracing::info!("Starting Flow-Like Kubernetes API Service");

    let config = config::Config::from_env()?;
    tracing::info!(
        "Loaded storage configuration: provider={}",
        config.storage_provider()
    );

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
        .nest("/health", health::routes())
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
