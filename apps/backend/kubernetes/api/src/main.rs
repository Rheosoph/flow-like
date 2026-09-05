#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::Router;
use flow_like_api::cache::sweeper::{CacheSweeperConfig, spawn_cache_sweeper_for};
use flow_like_api::channel::{ChannelSweeperConfig, spawn_channel_sweeper};
use flow_like_api::execution::{
    RunSweeperConfig, spawn_regression_suites_worker, spawn_run_sweeper,
};
use flow_like_api::telemetry::{
    TelemetryAlertConfig, TelemetryRollupConfig, TelemetrySweeperConfig,
    spawn_telemetry_alert_evaluator, spawn_telemetry_rollup, spawn_telemetry_sweeper,
};
use flow_like_api::{construct_router, state::State};
use flow_like_catalog::get_catalog;
use hardening::{cors_from_env, shutdown};
use std::sync::Arc;

mod config;
#[path = "../../../shared/api_hardening.rs"]
mod hardening;
mod health;
mod metrics;
#[path = "../../../docker-compose/api/src/secrets.rs"]
mod secrets;
mod storage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    secrets::load()?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve())
}

async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    metrics::init_telemetry();

    tracing::info!("Starting Flow-Like Kubernetes API Service");

    let config = config::Config::from_env()?;
    tracing::info!(
        "Loaded storage configuration: provider={}",
        config.storage_provider()
    );

    for name in ["SINK_TOKEN_ENCRYPTION_KEY", "MAINTENANCE_TOKEN"] {
        if std::env::var(name)
            .ok()
            .is_none_or(|value| value.trim().len() < 32)
        {
            return Err(format!("{name} must be configured with at least 32 bytes").into());
        }
    }
    let cors = cors_from_env()?;

    let catalog = get_catalog();

    let cdn_bucket = storage::create_cdn_store(&config)?;

    let state = Arc::new(State::new(Arc::new(catalog), Arc::new(cdn_bucket), None).await);

    if !flow_like_api::execution::is_jwt_configured() {
        return Err("backend JWT signing keys were not initialized".into());
    }
    // Decode success alone does not establish that these keys form a pair.
    let token_type = flow_like_api::backend_jwt::TokenType::User;
    let time = flow_like_api::backend_jwt::make_time_claims(token_type, Some(60));
    let probe = serde_json::json!({
        "sub": "startup-key-check", "typ": token_type,
        "iss": flow_like_api::backend_jwt::issuer(), "aud": token_type.audience(),
        "iat": time.iat, "nbf": time.nbf, "exp": time.exp,
    });
    let signed = flow_like_api::backend_jwt::sign(&probe)?;
    flow_like_api::backend_jwt::verify::<serde_json::Value>(&signed, token_type)?;

    let _sweeper_handle = spawn_run_sweeper(
        flow_like_api::audit::ExecutionAuditContext::from(&state),
        RunSweeperConfig::from_env(),
    );
    let _regression_suites_handle = spawn_regression_suites_worker(state.clone());
    let _deletion_worker = flow_like_api::deletion::spawn_deletion_worker(
        state.clone(),
        flow_like_api::deletion::DeletionWorkerConfig::from_env(),
    );
    let _channel_sweeper_handle = spawn_channel_sweeper(
        Arc::new(state.db.clone()),
        state.db_dialect,
        ChannelSweeperConfig::from_env(),
    );

    // Only spawns for backends without native expiry; the others no-op and log why.
    let _cache_sweeper_handle =
        spawn_cache_sweeper_for(&state.cache, CacheSweeperConfig::from_env()).await;

    // Spawned before the sweeper so the aggregates lead the deletions. The
    // ordering guarantee itself lives in the sweeper, which clamps every raw
    // retention cutoff to the last fully rolled-up day.
    let _telemetry_rollup_handle = spawn_telemetry_rollup(
        Arc::new(state.db.clone()),
        TelemetryRollupConfig::from_env(),
    );

    let _telemetry_sweeper_handle = spawn_telemetry_sweeper(
        Arc::new(state.db.clone()),
        state.db_dialect,
        TelemetrySweeperConfig::from_env(),
    );

    let _telemetry_alert_handle =
        spawn_telemetry_alert_evaluator(state.clone(), TelemetryAlertConfig::from_env());

    let execution_store = flow_like_api::execution::state::create_state_store(
        flow_like_api::execution::state::StateStoreConfig::default()
            .with_db(Arc::new(state.db.clone())),
    )
    .await?;
    let app = Router::new()
        .merge(construct_router(state.clone()))
        .nest("/health", health::routes(state.db.clone(), execution_store))
        .layer(cors);

    let metrics_port = std::env::var("METRICS_PORT").unwrap_or_else(|_| "9090".to_string());
    let metrics_app = Router::new().route("/metrics", axum::routing::get(metrics::handler));

    let addr = format!("0.0.0.0:{}", config.port);
    let metrics_addr = format!("0.0.0.0:{}", metrics_port);

    tracing::info!("API listening on {}", addr);
    tracing::info!("Metrics listening on {}", metrics_addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let metrics_listener = tokio::net::TcpListener::bind(&metrics_addr).await?;

    tokio::try_join!(
        async {
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown())
                .await
        },
        async {
            axum::serve(metrics_listener, metrics_app)
                .with_graceful_shutdown(shutdown())
                .await
        },
    )?;

    Ok(())
}
