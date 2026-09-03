#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::{Router, routing::get};
use flow_like_api::cache::sweeper::{CacheSweeperConfig, spawn_cache_sweeper};
use flow_like_api::channel::{ChannelSweeperConfig, spawn_channel_sweeper};
use flow_like_api::execution::{
    RunSweeperConfig, spawn_regression_suites_worker, spawn_run_sweeper,
};
use flow_like_api::telemetry::{
    TelemetryAlertConfig, TelemetryRollupConfig, TelemetrySweeperConfig,
    spawn_telemetry_alert_evaluator, spawn_telemetry_rollup, spawn_telemetry_sweeper,
};
use flow_like_api::{construct_router_with_cors, state::State};
use flow_like_catalog::get_catalog;
use flow_like_secrets::{ExposeSecret, SecretRef};
use std::{future::IntoFuture, sync::Arc};

mod config;
mod health;
use flow_like_azure_data::postgres;
mod storage;

const REQUIRED_SECRETS: &[(&str, usize)] = &[
    ("BACKEND_KEY", 64),
    ("BACKEND_PUB", 64),
    ("BACKEND_KID", 8),
    ("SINK_SECRET", 32),
    ("SINK_TOKEN_ENCRYPTION_KEY", 32),
    ("MAINTENANCE_TOKEN", 32),
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    metrics_endpoint::init_telemetry();

    let config = config::Config::from_env()?;
    let postgres_config = postgres::ManagedIdentityPostgresConfig::from_env(
        config.managed_identity_client_id.as_deref(),
    )?;
    tracing::info!(
        storage_account = %config.storage_account_name,
        postgres_host = %postgres_config.host,
        postgres_database = %postgres_config.database,
        content_container = %config.content_container,
        meta_container = %config.meta_container,
        cdn_container = %config.cdn_container,
        log_container = %config.log_container,
        cors_origin_count = config.cors_origin_count(),
        user_assigned_identity = config.managed_identity_client_id.is_some(),
        "starting Flow-Like Azure API"
    );

    let managed_database = postgres::connect(&postgres_config).await?;
    tracing::info!(
        token_lifetime_seconds = managed_database.lifecycle.remaining_seconds(),
        tls_mode = "verify-full",
        "connected to Azure PostgreSQL with managed identity"
    );

    let catalog = get_catalog();
    let cdn_store = storage::create_cdn_store(&config)?;
    let state = Arc::new(
        State::new_with_database(
            Arc::new(catalog),
            Arc::new(cdn_store),
            Some(config.secret_store_config()),
            managed_database.connection.clone(),
        )
        .await,
    );

    validate_security_prerequisites(&state).await?;

    let _run_sweeper = spawn_run_sweeper(Arc::new(state.db.clone()), RunSweeperConfig::from_env());
    let _regression_suites = spawn_regression_suites_worker(state.clone());
    let _channel_sweeper =
        spawn_channel_sweeper(Arc::new(state.db.clone()), ChannelSweeperConfig::from_env());
    let _cache_sweeper = state
        .cache_store
        .clone()
        .and_then(|store| spawn_cache_sweeper(store, CacheSweeperConfig::from_env()));
    let _telemetry_rollup = spawn_telemetry_rollup(
        Arc::new(state.db.clone()),
        TelemetryRollupConfig::from_env(),
    );
    let _telemetry_sweeper = spawn_telemetry_sweeper(
        Arc::new(state.db.clone()),
        TelemetrySweeperConfig::from_env(),
    );
    let _telemetry_alerts =
        spawn_telemetry_alert_evaluator(state.clone(), TelemetryAlertConfig::from_env());

    let health = health::HealthState::new(state.db.clone());
    let app = Router::new()
        .merge(construct_router_with_cors(state, config.cors_layer()))
        .merge(health::routes(health.clone()));
    let metrics_app = Router::new().route("/metrics", get(metrics_endpoint::handler));

    let api_address = format!("0.0.0.0:{}", config.port);
    let metrics_address = format!("0.0.0.0:{}", config.metrics_port);
    let api_listener = tokio::net::TcpListener::bind(&api_address).await?;
    let metrics_listener = tokio::net::TcpListener::bind(&metrics_address).await?;

    tracing::info!(address = %api_address, "Azure API is listening");
    tracing::info!(address = %metrics_address, "Prometheus metrics are listening");

    let (shutdown_sender, mut shutdown_receiver) = tokio::sync::watch::channel(false);
    let lifecycle_controller = managed_database.lifecycle.clone();
    let health_controller = health.clone();
    tokio::spawn(async move {
        lifecycle_controller.wait_until_drain().await;
        health_controller.begin_database_token_drain();
        tracing::warn!(
            token_lifetime_seconds = lifecycle_controller.remaining_seconds(),
            drain_seconds = postgres::READINESS_DRAIN_SECONDS,
            "database access token entered its safety window; readiness is now closed"
        );
        tokio::time::sleep(std::time::Duration::from_secs(
            postgres::READINESS_DRAIN_SECONDS,
        ))
        .await;
        let _ = shutdown_sender.send(true);
    });

    let api_server = axum::serve(api_listener, app)
        .with_graceful_shutdown(async move {
            while !*shutdown_receiver.borrow() {
                if shutdown_receiver.changed().await.is_err() {
                    break;
                }
            }
        })
        .into_future();
    let metrics_server = axum::serve(metrics_listener, metrics_app).into_future();
    let hard_stop = managed_database.lifecycle.wait_until_hard_stop();
    tokio::pin!(api_server, metrics_server, hard_stop);

    tokio::select! {
        result = &mut api_server => result?,
        result = &mut metrics_server => result?,
        _ = &mut hard_stop => {
            tracing::error!(
                graceful_shutdown_seconds = postgres::GRACEFUL_SHUTDOWN_SECONDS,
                "forcing workload rotation before the database access token expires"
            );
        }
    }

    Ok(())
}

async fn validate_security_prerequisites(state: &State) -> Result<(), StartupError> {
    for (name, minimum_length) in REQUIRED_SECRETS {
        let secret = state
            .secrets
            .get_secret_string(&SecretRef::new(*name))
            .await
            .map_err(|error| {
                StartupError(format!(
                    "required Key Vault secret {name} could not be resolved: {error}"
                ))
            })?;
        let value = secret.expose_secret();

        if value.trim().is_empty() || value.len() < *minimum_length {
            return Err(StartupError(format!(
                "required Key Vault secret {name} must contain at least {minimum_length} bytes"
            )));
        }
    }

    if !flow_like_api::execution::is_jwt_configured() {
        return Err(StartupError(
            "BACKEND_KEY and BACKEND_PUB did not initialize a backend JWT keypair".to_string(),
        ));
    }

    validate_channel_transport(state)?;

    Ok(())
}

/// The issuer degrades to HTTP when a transport's settings or secrets are unusable; a revision
/// that asked for Web PubSub must not come up silently on the fallback.
fn validate_channel_transport(state: &State) -> Result<(), StartupError> {
    let requested = flow_like_api::channel::ChannelBackend::parse(
        std::env::var("CHANNEL_TRANSPORT").ok().as_deref(),
    )
    .map_err(StartupError)?;
    if state.channels.backend() != &requested {
        return Err(StartupError(format!(
            "CHANNEL_TRANSPORT={} could not be initialized; check CHANNEL_WEBPUBSUB_ENDPOINT, \
             CHANNEL_WEBPUBSUB_HUB and the Key Vault secret CHANNEL_WEBPUBSUB_ACCESS_KEY",
            requested.transport()
        )));
    }
    Ok(())
}

#[derive(Debug)]
struct StartupError(String);

impl std::fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StartupError {}

mod metrics_endpoint {
    use axum::response::IntoResponse;
    use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use std::sync::OnceLock;
    use std::time::Duration;
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

    const OTLP_ENDPOINT_VARS: [&str; 2] = [
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        "OTEL_EXPORTER_OTLP_ENDPOINT",
    ];
    const OTLP_TIMEOUT_VARS: [&str; 2] = [
        "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
        "OTEL_EXPORTER_OTLP_TIMEOUT",
    ];
    const DEFAULT_OTLP_EXPORT_TIMEOUT: Duration = Duration::from_millis(10_000);

    enum TracingSetup {
        Enabled {
            tracer: opentelemetry_sdk::trace::Tracer,
            endpoint: String,
            endpoint_var: &'static str,
            timeout: Duration,
        },
        Disabled(String),
    }

    pub fn init_telemetry() {
        let format_layer = tracing_subscriber::fmt::layer();
        let env_filter = flow_like_api::warn_env_filter();

        match init_tracing() {
            (
                TracingSetup::Enabled {
                    tracer,
                    endpoint,
                    endpoint_var,
                    timeout,
                },
                timeout_warning,
            ) => {
                tracing_subscriber::registry()
                    .with(format_layer)
                    .with(env_filter)
                    .with(tracing_opentelemetry::layer().with_tracer(tracer))
                    .init();
                if let Some(warning) = timeout_warning {
                    tracing::warn!("{warning}");
                }
                tracing::info!(
                    "OpenTelemetry tracing enabled (endpoint={endpoint} from {endpoint_var}, export timeout={}ms)",
                    timeout.as_millis()
                );
            }
            (TracingSetup::Disabled(reason), timeout_warning) => {
                tracing_subscriber::registry()
                    .with(format_layer)
                    .with(env_filter)
                    .init();
                if let Some(warning) = timeout_warning {
                    tracing::warn!("{warning}");
                }
                tracing::info!("OpenTelemetry tracing disabled ({reason})");
            }
        }

        init_metrics();
    }

    /// `AZURE_REQUIRE_OTEL=true` is the only configuration under which a
    /// missing or unbuildable exporter is fatal. Without it, an endpoint the
    /// exporter rejects leaves tracing disabled with the reason logged, rather
    /// than taking the process down over a telemetry variable.
    fn init_tracing() -> (TracingSetup, Option<String>) {
        let require_otel = std::env::var("AZURE_REQUIRE_OTEL")
            .is_ok_and(|value| value.eq_ignore_ascii_case("true"));
        let (timeout, timeout_warning) = resolve_otlp_export_timeout();
        let Some((endpoint_var, endpoint)) = resolve_otlp_endpoint() else {
            if require_otel {
                panic!(
                    "AZURE_REQUIRE_OTEL=true requires {} or {}",
                    OTLP_ENDPOINT_VARS[0], OTLP_ENDPOINT_VARS[1]
                )
            }
            return (
                TracingSetup::Disabled(format!(
                    "neither {} nor {} is set",
                    OTLP_ENDPOINT_VARS[0], OTLP_ENDPOINT_VARS[1]
                )),
                timeout_warning,
            );
        };
        let exporter = match opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .with_timeout(timeout)
            .build()
        {
            Ok(exporter) => exporter,
            Err(error) if require_otel => {
                panic!("required OTLP exporter could not initialize: {error}")
            }
            Err(error) => {
                return (
                    TracingSetup::Disabled(format!(
                        "the OTLP exporter for {endpoint_var}={endpoint} could not be built: {error}"
                    )),
                    timeout_warning,
                );
            }
        };
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .build();
        let tracer = provider.tracer("flow-like-azure-api");
        opentelemetry::global::set_tracer_provider(provider);
        (
            TracingSetup::Enabled {
                tracer,
                endpoint,
                endpoint_var,
                timeout,
            },
            timeout_warning,
        )
    }

    fn resolve_otlp_endpoint() -> Option<(&'static str, String)> {
        OTLP_ENDPOINT_VARS.into_iter().find_map(|name| {
            let value = std::env::var(name).ok()?;
            let value = value.trim();
            (!value.is_empty()).then(|| (name, value.to_string()))
        })
    }

    /// The OTel specification, and opentelemetry-otlp since 0.28, read these as
    /// milliseconds; 0.27 read them as seconds. Resolved here and passed to the
    /// builder explicitly so the value cannot change meaning under the process.
    fn resolve_otlp_export_timeout() -> (Duration, Option<String>) {
        for name in OTLP_TIMEOUT_VARS {
            let Ok(raw) = std::env::var(name) else {
                continue;
            };
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            return match raw.parse::<u64>() {
                Ok(millis) => (Duration::from_millis(millis), None),
                Err(_) => (
                    DEFAULT_OTLP_EXPORT_TIMEOUT,
                    Some(format!(
                        "{name}={raw:?} is not a whole number of milliseconds; using {}ms",
                        DEFAULT_OTLP_EXPORT_TIMEOUT.as_millis()
                    )),
                ),
            };
        }
        (DEFAULT_OTLP_EXPORT_TIMEOUT, None)
    }

    fn init_metrics() {
        let handle = PrometheusBuilder::new()
            .set_buckets_for_metric(
                Matcher::Full("http_request_duration_seconds".to_string()),
                &[
                    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
                ],
            )
            .expect("Prometheus histogram buckets must be valid")
            .install_recorder()
            .expect("Prometheus recorder must initialize once");

        PROMETHEUS_HANDLE
            .set(handle)
            .expect("Prometheus recorder must initialize once");
        metrics::describe_counter!("http_requests_total", "Total number of HTTP requests");
        metrics::describe_histogram!(
            "http_request_duration_seconds",
            "HTTP request duration in seconds"
        );
        metrics::describe_gauge!("api_active_connections", "Number of active connections");
    }

    pub async fn handler() -> impl IntoResponse {
        PROMETHEUS_HANDLE
            .get()
            .expect("Prometheus recorder must be initialized")
            .render()
    }
}
