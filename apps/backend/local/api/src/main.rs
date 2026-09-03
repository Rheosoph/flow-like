#[cfg(not(any(all(target_os = "macos", target_arch = "aarch64"), target_os = "ios")))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use dotenv::dotenv;
use flow_like_api::axum;
use flow_like_api::cache::sweeper::{CacheSweeperConfig, spawn_cache_sweeper};
use flow_like_api::channel::{ChannelSweeperConfig, spawn_channel_sweeper};
use flow_like_api::construct_router;
use flow_like_api::execution::{
    RunSweeperConfig, spawn_regression_suites_worker, spawn_run_sweeper,
};
use flow_like_api::telemetry::{
    SpanExportConfig, TelemetryAlertConfig, TelemetryRollupConfig, TelemetrySweeperConfig,
    spawn_telemetry_alert_evaluator, spawn_telemetry_rollup, spawn_telemetry_sweeper,
    telemetry_span_layer,
};
use flow_like_catalog::get_catalog;
use flow_like_secrets::{
    EnvProviderConfig, ExposeSecret, ProviderConfig, SecretRef, SecretStore, SecretStoreConfig,
};
use flow_like_storage::object_store::aws::AmazonS3Builder;
use flow_like_types::tokio;
use sentry_tracing::{EventFilter, default_event_filter};
use socket2::{Domain, Socket, Type};
use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
    time::Duration,
};
use tracing_subscriber::prelude::*;

#[flow_like_types::tokio::main]
async fn main() {
    dotenv().ok();

    let sentry_endpoint = std::env::var("SENTRY_ENDPOINT").unwrap_or_default();

    // Converts closed spans into internal telemetry rows. Stays inert until the
    // exporter is spawned below, and disarms itself when telemetry is disabled.
    let (span_layer, span_exporter) = telemetry_span_layer(SpanExportConfig::from_env());

    let _sentry_guard = if sentry_endpoint.is_empty() {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(span_layer)
            .init();
        None
    } else {
        let sentry_layer =
            sentry_tracing::layer().event_filter(|metadata| match *metadata.level() {
                tracing::Level::ERROR => EventFilter::Breadcrumb,
                _ => default_event_filter(metadata),
            });
        let guard = sentry::init((
            sentry_endpoint,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                traces_sample_rate: 0.3,
                ..Default::default()
            },
        ));
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(sentry_layer)
            .with(span_layer)
            .init();
        Some(guard)
    };

    let secret_prefix = std::env::var("SECRET_PREFIX").ok();
    let secret_config =
        SecretStoreConfig::default().with_provider(ProviderConfig::Env(EnvProviderConfig {
            prefix: secret_prefix,
        }));
    let secrets =
        Arc::new(SecretStore::new(secret_config.clone()).expect("Failed to create secret store"));

    let cdn_bucket = std::env::var("CDN_BUCKET_NAME").unwrap();
    let cdn_bucket_endpoint = std::env::var("CDN_BUCKET_ENDPOINT").ok();
    let cdn_bucket_access_key = std::env::var("CDN_BUCKET_ACCESS_KEY_ID").ok();
    let cdn_bucket_secret_key = secrets
        .get_secret_string(&SecretRef::new("CDN_BUCKET_SECRET_ACCESS_KEY"))
        .await
        .ok()
        .map(|s| s.expose_secret().to_string());

    let mut cdn_bucket = AmazonS3Builder::new().with_bucket_name(cdn_bucket);
    if let Some(endpoint) = cdn_bucket_endpoint
        && !endpoint.is_empty()
    {
        cdn_bucket = cdn_bucket.with_endpoint(endpoint);
    }

    if let (Some(access_key), Some(secret_key)) = (cdn_bucket_access_key, cdn_bucket_secret_key)
        && !access_key.is_empty()
        && !secret_key.is_empty()
    {
        cdn_bucket = cdn_bucket.with_access_key_id(access_key);
        cdn_bucket = cdn_bucket.with_secret_access_key(secret_key);
    }

    let cdn_bucket =
        flow_like_storage::files::store::FlowLikeStore::AWS(Arc::new(cdn_bucket.build().unwrap()));

    let catalog = Arc::new(get_catalog());
    let state = Arc::new(
        flow_like_api::state::State::new(catalog, Arc::new(cdn_bucket), Some(secret_config)).await,
    );

    let _sweeper_handle =
        spawn_run_sweeper(Arc::new(state.db.clone()), RunSweeperConfig::from_env());
    let _regression_suites_handle = spawn_regression_suites_worker(state.clone());
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

    let _span_exporter_handle = span_exporter.spawn_for_state(&state);

    let app = construct_router(state);

    let port = std::env::var("API_PORT").unwrap_or_else(|_| "8080".to_string());
    let port: u16 = port.parse().expect("API_PORT must be a valid port number");
    let listener = match create_listener(format!("0.0.0.0:{}", port)) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("Failed to bind to port {}: {:?}", port, err);
            return;
        }
    };
    axum::serve(listener, app).await.unwrap();
}

fn create_listener<A: ToSocketAddrs>(
    addr: A,
) -> io::Result<flow_like_types::tokio::net::TcpListener> {
    let mut addrs = addr.to_socket_addrs()?;
    let addr = addrs.next().unwrap();
    let listener = match &addr {
        SocketAddr::V4(_) => Socket::new(Domain::IPV4, Type::STREAM, None)?,
        SocketAddr::V6(_) => Socket::new(Domain::IPV6, Type::STREAM, None)?,
    };

    listener.set_nonblocking(true)?;
    listener.set_reuse_address(true)?;
    listener.set_linger(Some(Duration::from_secs(0)))?;
    listener.bind(&addr.into())?;
    listener.listen(i32::MAX)?;

    let listener = std::net::TcpListener::from(listener);
    let listener = flow_like_types::tokio::net::TcpListener::from_std(listener)?;
    Ok(listener)
}
