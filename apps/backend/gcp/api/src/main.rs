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
use flow_like_gcp_data::postgres;
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
    // Ordered before telemetry on purpose: the exporters open sockets, and the
    // Sentry transport honours proxy environment variables. See
    // `config::reject_forbidden_environment`.
    config::reject_forbidden_environment()?;
    let _sentry_guard = metrics_endpoint::init_telemetry();

    let config = config::Config::from_env()?;
    let postgres_config = postgres::IamPostgresConfig::from_env()?;
    // Cloud Run puts no instance identifier in the structured log payload, so
    // without a per-process id the staggered drain windows of several replicas
    // are indistinguishable in one log stream — which is exactly the situation
    // the jitter in the token lifecycle creates on purpose.
    let boot_id = uuid::Uuid::new_v4();
    tracing::info!(
        boot_id = %boot_id,
        project_id = %config.project_id,
        postgres_host = %postgres_config.host,
        postgres_database = %postgres_config.database,
        content_bucket = %config.content_bucket,
        meta_bucket = %config.meta_bucket,
        cdn_bucket = %config.cdn_bucket,
        log_bucket = %config.log_bucket,
        cors_origin_count = config.cors_origin_count(),
        "starting Flow-Like GCP API"
    );

    let managed_database = postgres::connect(&postgres_config).await?;
    tracing::info!(
        token_lifetime_seconds = managed_database.lifecycle.remaining_seconds(),
        tls_mode = "verify-full",
        "connected to Cloud SQL with IAM database authentication"
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
    // Cloud Run publishes exactly one container port per service, so the second
    // listener the Azure image binds on METRICS_PORT would be unreachable here.
    // Prometheus is mounted on the serving router instead. `/metrics` is
    // deliberately absent from the load balancer's URL map and Cloud Run ingress
    // is restricted to the load balancer, so the route stays reachable only from
    // inside the project — keep it out of the URL map.
    let app = Router::new()
        .merge(construct_router_with_cors(state, config.cors_layer()))
        .merge(health::routes(health.clone()))
        .route("/metrics", get(metrics_endpoint::handler));

    let api_address = format!("0.0.0.0:{}", config.port);
    let api_listener = tokio::net::TcpListener::bind(&api_address).await?;

    tracing::info!(address = %api_address, "GCP API is listening");

    let (shutdown_sender, mut shutdown_receiver) = tokio::sync::watch::channel(false);

    let lifecycle_controller = managed_database.lifecycle.clone();
    let token_health = health.clone();
    let token_shutdown = shutdown_sender.clone();
    tokio::spawn(async move {
        lifecycle_controller.wait_until_drain().await;
        token_health.begin_drain();
        tracing::warn!(
            boot_id = %boot_id,
            token_lifetime_seconds = lifecycle_controller.remaining_seconds(),
            drain_seconds = postgres::READINESS_DRAIN_SECONDS,
            "Cloud SQL IAM token entered its safety window; readiness is now closed"
        );
        // Long enough for the load balancer's health check to observe the closed
        // readiness endpoint and stop steering new requests at this instance
        // before the listener goes away.
        tokio::time::sleep(std::time::Duration::from_secs(
            postgres::READINESS_DRAIN_SECONDS,
        ))
        .await;
        let _ = token_shutdown.send(true);
    });

    let signal_health = health.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        signal_health.begin_drain();
        // No drain sleep on this path. Cloud Run sends SIGTERM roughly ten
        // seconds before SIGKILL and has already taken the instance out of
        // rotation by then, so waiting out READINESS_DRAIN_SECONDS would
        // guarantee the hard kill lands in the middle of a request instead of
        // preventing it.
        tracing::warn!(boot_id = %boot_id, "received a termination signal; draining");
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
    let hard_stop = managed_database.lifecycle.wait_until_hard_stop();
    tokio::pin!(api_server, hard_stop);

    tokio::select! {
        result = &mut api_server => result?,
        _ = &mut hard_stop => {
            tracing::error!(
                boot_id = %boot_id,
                graceful_shutdown_seconds = postgres::GRACEFUL_SHUTDOWN_SECONDS,
                "forcing instance rotation before the Cloud SQL IAM token expires"
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
                    "required Secret Manager secret {name} could not be resolved: {error}"
                ))
            })?;
        let value = secret.expose_secret();

        if value.trim().is_empty() || value.len() < *minimum_length {
            return Err(StartupError(format!(
                "required Secret Manager secret {name} must contain at least {minimum_length} bytes"
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
/// that asked for the Realtime Database transport must not come up silently on the fallback.
fn validate_channel_transport(state: &State) -> Result<(), StartupError> {
    let requested = flow_like_api::channel::ChannelBackend::parse(
        std::env::var("CHANNEL_TRANSPORT").ok().as_deref(),
    )
    .map_err(StartupError)?;
    if state.channels.backend() != &requested {
        return Err(StartupError(format!(
            "CHANNEL_TRANSPORT={} could not be initialized; check CHANNEL_FIREBASE_DATABASE_URL, \
             CHANNEL_FIREBASE_API_KEY and the Secret Manager secret CHANNEL_FIREBASE_SERVICE_ACCOUNT",
            requested.transport()
        )));
    }
    Ok(())
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let terminate = signal(SignalKind::terminate());
    match terminate {
        Ok(mut terminate) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
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
    use flow_like_gcp_data::metadata::{AccessToken, MetadataError, TokenSource};
    use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::{WithExportConfig, WithTonicConfig};
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use std::sync::{Arc, OnceLock, RwLock, mpsc};
    use std::time::Duration;
    use tonic::{
        Request, Status, metadata::AsciiMetadataValue, service::Interceptor,
        transport::ClientTlsConfig,
    };
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

    /// telemetry.googleapis.com accepts the Cloud Trace append scope, and
    /// nothing this exporter does needs more. Minting the bearer narrow follows
    /// the rest of `flow_like_gcp_data::metadata`: a leaked copy can append
    /// spans and nothing else, whatever roles the service account holds.
    const TELEMETRY_TRACE_SCOPE: &str = "https://www.googleapis.com/auth/trace.append";

    /// How often the refresher thread consults the token source. The source
    /// caches until roughly 3m45s before expiry, so almost every poll is a
    /// mutex and a clock check; the interval only bounds how stale the slot can
    /// be once the metadata server rotates the token.
    const TOKEN_POLL_INTERVAL: Duration = Duration::from_secs(30);

    /// Upper bound on the startup credential check under
    /// `GCP_REQUIRE_OTEL=true`. The metadata provider retries three times with
    /// a ten-second request timeout, so a minute covers the worst honest fetch
    /// and everything beyond it is the outage the knob exists to surface.
    const STARTUP_TOKEN_TIMEOUT: Duration = Duration::from_secs(60);

    const OTLP_ENDPOINT_VARS: [&str; 2] = [
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        "OTEL_EXPORTER_OTLP_ENDPOINT",
    ];
    const OTLP_TIMEOUT_VARS: [&str; 2] = [
        "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
        "OTEL_EXPORTER_OTLP_TIMEOUT",
    ];
    const DEFAULT_OTLP_EXPORT_TIMEOUT: Duration = Duration::from_millis(10_000);

    struct EnabledTracing {
        tracer: opentelemetry_sdk::trace::Tracer,
        endpoint: String,
        endpoint_var: &'static str,
        timeout: Duration,
    }

    pub fn init_telemetry() -> Option<sentry::ClientInitGuard> {
        // Sentry is bound before the subscriber is installed because
        // `sentry_tracing::layer()` forwards to whichever client is bound when
        // an event is recorded; a layer installed ahead of the client silently
        // drops everything that happens during startup — precisely the events
        // worth having.
        let sentry_guard = init_sentry();
        let format_layer = tracing_subscriber::fmt::layer();
        let env_filter = flow_like_api::warn_env_filter();
        let (otlp, timeout_warning) = init_tracing();
        let enabled = otlp
            .as_ref()
            .map(|otlp| (otlp.endpoint.clone(), otlp.endpoint_var, otlp.timeout));
        let otel_layer = otlp.map(|otlp| tracing_opentelemetry::layer().with_tracer(otlp.tracer));
        let sentry_layer = sentry_guard.is_some().then(sentry_tracing::layer);

        tracing_subscriber::registry()
            .with(format_layer)
            .with(env_filter)
            .with(otel_layer)
            .with(sentry_layer)
            .init();

        if let Some(warning) = timeout_warning {
            tracing::warn!("{warning}");
        }
        if let Some((endpoint, endpoint_var, timeout)) = enabled {
            tracing::info!(
                "OpenTelemetry tracing enabled (endpoint={endpoint} from {endpoint_var}, export timeout={}ms)",
                timeout.as_millis()
            );
        }

        init_metrics();
        sentry_guard
    }

    fn init_sentry() -> Option<sentry::ClientInitGuard> {
        let endpoint = std::env::var("SENTRY_ENDPOINT").ok()?;
        if endpoint.trim().is_empty() {
            return None;
        }

        Some(sentry::init((
            endpoint,
            sentry::ClientOptions {
                release: sentry::release_name!(),
                traces_sample_rate: 0.3,
                ..Default::default()
            },
        )))
    }

    fn init_tracing() -> (Option<EnabledTracing>, Option<String>) {
        let require_otel =
            std::env::var("GCP_REQUIRE_OTEL").is_ok_and(|value| value.eq_ignore_ascii_case("true"));
        let (timeout, timeout_warning) = resolve_otlp_export_timeout();
        let Some((endpoint_var, endpoint)) = resolve_otlp_endpoint() else {
            // Traces carry the audit trail for cross-service calls. A deployment
            // that declared it requires them must not come up quietly without
            // them, so the missing endpoint is fatal rather than a warning.
            if require_otel {
                panic!(
                    "GCP_REQUIRE_OTEL=true requires {} or {}",
                    OTLP_ENDPOINT_VARS[0], OTLP_ENDPOINT_VARS[1]
                )
            }
            return (None, timeout_warning);
        };
        // telemetry.googleapis.com meters every export against the project
        // named on the RPC, so an exporter without a project would only ever be
        // rejected. Read from the environment directly because this runs before
        // `config::Config::from_env`, which validates the same variable for the
        // rest of the process. Announced on stderr rather than silently
        // skipped: an endpoint with no project is a misconfiguration, not an
        // opt-out, and tracing is not installed yet at this point.
        let project_id = match std::env::var("GCP_PROJECT_ID") {
            Ok(project) if !project.trim().is_empty() => project,
            _ if require_otel => {
                panic!(
                    "GCP_REQUIRE_OTEL=true requires GCP_PROJECT_ID for the x-goog-user-project export header"
                )
            }
            _ => {
                eprintln!(
                    "{endpoint_var} is set but GCP_PROJECT_ID is not; OTLP export stays disabled"
                );
                return (None, timeout_warning);
            }
        };
        let authorizer = OtlpAuthorizer::start(&project_id, require_otel);
        let exporter = match opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .with_timeout(timeout)
            // Explicit, not implied by the https:// scheme: the exporter builds
            // its channel through tonic's `Endpoint::from_shared`, which never
            // applies the default TLS configuration `Endpoint::new` would, so
            // without this call the first export fails with
            // HttpsUriWithoutTlsSupport even with the tls-roots feature
            // compiled in. `with_enabled_roots` trusts the image's
            // ca-certificates bundle — the same store every other TLS
            // connection this process opens verifies against.
            .with_tls_config(ClientTlsConfig::new().with_enabled_roots())
            .with_interceptor(authorizer)
            .build()
        {
            Ok(exporter) => exporter,
            Err(error) if require_otel => {
                panic!("required OTLP exporter could not initialize: {error}")
            }
            Err(error) => {
                eprintln!(
                    "the OTLP exporter for {endpoint_var}={endpoint} could not be built: {error}; OTLP export stays disabled"
                );
                return (None, timeout_warning);
            }
        };
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .build();
        let tracer = provider.tracer("flow-like-gcp-api");
        opentelemetry::global::set_tracer_provider(provider);
        (
            Some(EnabledTracing {
                tracer,
                endpoint,
                endpoint_var,
                timeout,
            }),
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

    /// Per-RPC gRPC metadata for telemetry.googleapis.com: `authorization`
    /// carries the workload service account's metadata-server bearer and
    /// `x-goog-user-project` names the project whose Telemetry API quota the
    /// export spends. A static `with_metadata` map cannot carry the bearer —
    /// the token rotates within the hour — so the interceptor reads whatever
    /// token the refresher thread last stored.
    #[derive(Clone)]
    struct OtlpAuthorizer {
        token_slot: Arc<RwLock<Option<AccessToken>>>,
        user_project: AsciiMetadataValue,
    }

    impl OtlpAuthorizer {
        /// Spawns the refresher and, under `GCP_REQUIRE_OTEL=true`, blocks
        /// until the first fetch settles, so a revision whose credential cannot
        /// exist fails at boot instead of dropping spans. The check stops at
        /// the credential on purpose: a full export probe at boot would write a
        /// synthetic span into the production trace store on every cold start,
        /// and an export that fails later is already loud — the batch exporter
        /// logs every rejected RPC. The one failure this cannot see, a missing
        /// roles/telemetry.tracesWriter grant, is Terraform's to guarantee and
        /// shows up in those same rejection logs.
        fn start(project_id: &str, require_otel: bool) -> Self {
            let user_project = AsciiMetadataValue::try_from(project_id).unwrap_or_else(|_| {
                panic!("GCP_PROJECT_ID {project_id:?} is not a valid gRPC metadata value")
            });
            let token_slot = Arc::new(RwLock::new(None));
            let refresher_slot = Arc::clone(&token_slot);
            let (ready_sender, ready_receiver) = mpsc::channel();
            // A dedicated thread with its own single-thread runtime rather than
            // a task on the serving runtime: this function runs while the
            // subscriber is still being assembled, before anything may block on
            // the runtime, and the thread parks in a timer for all but
            // microseconds of its life.
            std::thread::Builder::new()
                .name("otlp-token-refresh".to_string())
                .spawn(move || refresh_tokens(refresher_slot, ready_sender))
                .unwrap_or_else(|error| {
                    panic!("the OTLP token refresher thread could not be spawned: {error}")
                });
            if require_otel {
                match ready_receiver.recv_timeout(STARTUP_TOKEN_TIMEOUT) {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        panic!(
                            "GCP_REQUIRE_OTEL=true but no OTLP export token is obtainable: {error}"
                        )
                    }
                    Err(_) => panic!(
                        "GCP_REQUIRE_OTEL=true but the metadata server did not deliver an OTLP export token within {}s",
                        STARTUP_TOKEN_TIMEOUT.as_secs()
                    ),
                }
            }
            Self {
                token_slot,
                user_project,
            }
        }
    }

    impl Interceptor for OtlpAuthorizer {
        fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
            let token = match self.token_slot.read() {
                Ok(slot) => slot.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            };
            // Failing the RPC beats sending it bare: an unauthenticated export
            // would be rejected by the endpoint anyway, and this error names
            // the actual problem in the exporter's failure log line.
            let Some(token) = token else {
                return Err(Status::unauthenticated(
                    "no metadata-server access token is available yet for the OTLP export",
                ));
            };
            if token.remaining_seconds() <= 0 {
                return Err(Status::unauthenticated(
                    "the cached OTLP export token has expired and its refresh has not succeeded",
                ));
            }
            let bearer = AsciiMetadataValue::try_from(format!("Bearer {}", token.secret()))
                .map_err(|_| {
                    Status::unauthenticated("the metadata access token is not a valid header value")
                })?;
            request.metadata_mut().insert("authorization", bearer);
            request
                .metadata_mut()
                .insert("x-goog-user-project", self.user_project.clone());
            Ok(request)
        }
    }

    /// Body of the refresher thread. The token source keeps its own cache with
    /// the refresh margin `flow_like_gcp_data::metadata` documents, so the loop
    /// stores whatever the source considers current and goes back to sleep.
    /// Only the first result is reported through `ready`, and only a
    /// `GCP_REQUIRE_OTEL=true` boot listens for it.
    fn refresh_tokens(
        slot: Arc<RwLock<Option<AccessToken>>>,
        ready: mpsc::Sender<Result<(), MetadataError>>,
    ) {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = ready.send(Err(MetadataError::Configuration(format!(
                    "the OTLP token refresher could not build its runtime: {error}"
                ))));
                return;
            }
        };
        runtime.block_on(async move {
            let source = match TokenSource::metadata(&[TELEMETRY_TRACE_SCOPE]) {
                Ok(source) => source,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            let mut ready = Some(ready);
            loop {
                match source.token().await {
                    Ok(token) => {
                        match slot.write() {
                            Ok(mut slot) => *slot = Some(token),
                            Err(poisoned) => *poisoned.into_inner() = Some(token),
                        }
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(Ok(()));
                        }
                    }
                    Err(error) => {
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(Err(error));
                        } else {
                            tracing::warn!(
                                error = %error,
                                "OTLP export token refresh failed; the exporter keeps the previous token until it expires"
                            );
                        }
                    }
                }
                tokio::time::sleep(TOKEN_POLL_INTERVAL).await;
            }
        });
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
