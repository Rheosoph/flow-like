use axum::response::IntoResponse;
use flow_like_gcp_data::metadata::{AccessToken, MetadataError, TokenSource};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{WithExportConfig, WithTonicConfig};
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::sync::{Arc, OnceLock, RwLock, mpsc};
use std::time::Duration;
use tonic::{
    Request, Status, metadata::AsciiMetadataValue, service::Interceptor, transport::ClientTlsConfig,
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// telemetry.googleapis.com accepts the Cloud Trace append scope, and nothing
/// this exporter does needs more. Minting the bearer narrow follows the rest of
/// `flow_like_gcp_data::metadata`: a leaked copy can append spans and nothing
/// else, whatever roles the service account holds.
const TELEMETRY_TRACE_SCOPE: &str = "https://www.googleapis.com/auth/trace.append";

/// How often the refresher thread consults the token source. The source caches
/// until roughly 3m45s before expiry, so almost every poll is a mutex and a
/// clock check; the interval only bounds how stale the slot can be once the
/// metadata server rotates the token.
const TOKEN_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Upper bound on the startup credential check under `GCP_REQUIRE_OTEL=true`.
/// The metadata provider retries three times with a ten-second request timeout,
/// so a minute covers the worst honest fetch and everything beyond it is the
/// outage the knob exists to surface.
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

pub fn init_telemetry() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info")
            .add_directive("hyper=warn".parse().expect("valid filter"))
            .add_directive("rustls=warn".parse().expect("valid filter"))
            .add_directive("tokio=warn".parse().expect("valid filter"))
    });
    let (otlp, timeout_warning) = init_tracing();
    let enabled = otlp
        .as_ref()
        .map(|otlp| (otlp.endpoint.clone(), otlp.endpoint_var, otlp.timeout));
    let otel_layer = otlp.map(|otlp| tracing_opentelemetry::layer().with_tracer(otlp.tracer));

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(env_filter)
        .with(otel_layer)
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
}

fn init_tracing() -> (Option<EnabledTracing>, Option<String>) {
    let require_otel =
        std::env::var("GCP_REQUIRE_OTEL").is_ok_and(|value| value.eq_ignore_ascii_case("true"));
    let (timeout, timeout_warning) = resolve_otlp_export_timeout();
    let Some((endpoint_var, endpoint)) = resolve_otlp_endpoint() else {
        // Traces carry the audit trail for the API -> executor hop. A deployment
        // that declared it requires them must not come up quietly without them,
        // so the missing endpoint is fatal rather than a warning — the same
        // knob, with the same meaning, as on the GCP API image.
        if require_otel {
            panic!(
                "GCP_REQUIRE_OTEL=true requires {} or {}",
                OTLP_ENDPOINT_VARS[0], OTLP_ENDPOINT_VARS[1]
            )
        }
        return (None, timeout_warning);
    };
    // telemetry.googleapis.com meters every export against the project named on
    // the RPC, so an exporter without a project would only ever be rejected.
    // Announced on stderr rather than silently skipped: an endpoint with no
    // project is a misconfiguration, not an opt-out, and tracing is not
    // installed yet at this point.
    let project_id = match std::env::var("GCP_PROJECT_ID") {
        Ok(project) if !project.trim().is_empty() => project,
        _ if require_otel => {
            panic!(
                "GCP_REQUIRE_OTEL=true requires GCP_PROJECT_ID for the x-goog-user-project export header"
            )
        }
        _ => {
            eprintln!("{endpoint_var} is set but GCP_PROJECT_ID is not; OTLP export stays disabled");
            return (None, timeout_warning);
        }
    };
    let authorizer = OtlpAuthorizer::start(&project_id, require_otel);
    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&endpoint)
        .with_timeout(timeout)
        // Explicit, not implied by the https:// scheme: the exporter builds its
        // channel through tonic's `Endpoint::from_shared`, which never applies
        // the default TLS configuration `Endpoint::new` would, so without this
        // call the first export fails with HttpsUriWithoutTlsSupport even with
        // the tls-roots feature compiled in. `with_enabled_roots` trusts the
        // image's ca-certificates bundle — the same store every other TLS
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
    let tracer = provider.tracer("flow-like-gcp-executor");
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

/// Per-RPC gRPC metadata for telemetry.googleapis.com: `authorization` carries
/// the workload service account's metadata-server bearer and
/// `x-goog-user-project` names the project whose Telemetry API quota the export
/// spends. A static `with_metadata` map cannot carry the bearer — the token
/// rotates within the hour — so the interceptor reads whatever token the
/// refresher thread last stored.
#[derive(Clone)]
struct OtlpAuthorizer {
    token_slot: Arc<RwLock<Option<AccessToken>>>,
    user_project: AsciiMetadataValue,
}

impl OtlpAuthorizer {
    /// Spawns the refresher and, under `GCP_REQUIRE_OTEL=true`, blocks until
    /// the first fetch settles, so a revision whose credential cannot exist
    /// fails at boot instead of dropping spans. The check stops at the
    /// credential on purpose: a full export probe at boot would write a
    /// synthetic span into the production trace store on every cold start, and
    /// an export that fails later is already loud — the batch exporter logs
    /// every rejected RPC. The one failure this cannot see, a missing
    /// roles/telemetry.tracesWriter grant, is Terraform's to guarantee and
    /// shows up in those same rejection logs.
    fn start(project_id: &str, require_otel: bool) -> Self {
        let user_project = AsciiMetadataValue::try_from(project_id).unwrap_or_else(|_| {
            panic!("GCP_PROJECT_ID {project_id:?} is not a valid gRPC metadata value")
        });
        let token_slot = Arc::new(RwLock::new(None));
        let refresher_slot = Arc::clone(&token_slot);
        let (ready_sender, ready_receiver) = mpsc::channel();
        // A dedicated thread with its own single-thread runtime rather than a
        // task on the serving runtime: this function runs while the subscriber
        // is still being assembled, before anything may block on the runtime,
        // and the thread parks in a timer for all but microseconds of its life.
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
                    panic!("GCP_REQUIRE_OTEL=true but no OTLP export token is obtainable: {error}")
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
        // would be rejected by the endpoint anyway, and this error names the
        // actual problem in the exporter's failure log line.
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
        let bearer =
            AsciiMetadataValue::try_from(format!("Bearer {}", token.secret())).map_err(|_| {
                Status::unauthenticated("the metadata access token is not a valid header value")
            })?;
        request.metadata_mut().insert("authorization", bearer);
        request
            .metadata_mut()
            .insert("x-goog-user-project", self.user_project.clone());
        Ok(request)
    }
}

/// Body of the refresher thread. The token source keeps its own cache with the
/// refresh margin `flow_like_gcp_data::metadata` documents, so the loop stores
/// whatever the source considers current and goes back to sleep. Only the first
/// result is reported through `ready`, and only a `GCP_REQUIRE_OTEL=true` boot
/// listens for it.
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

/// The recorder is process-global: anything in the executor's dependency tree
/// that records through the `metrics` facade lands on `/metrics`. Today the
/// only producer is the request middleware in main.rs, so only that series is
/// described — describing series nothing records, as the Kubernetes executor
/// does, only advertises metrics a dashboard can never draw.
fn init_metrics() {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("Prometheus recorder must initialize once");

    PROMETHEUS_HANDLE
        .set(handle)
        .expect("Prometheus recorder must initialize once");
    metrics::describe_counter!(
        "http_requests_total",
        "Total number of HTTP requests, by matched route, method and response status"
    );
}

/// One increment per response, keyed by the *matched* route rather than the
/// raw path so a probe of unmatched URLs cannot mint unbounded label values.
/// Only a counter is recorded on purpose: `/execute/sse` and `/execute/stream`
/// return their headers before the run finishes, so a duration measured at the
/// middleware would time the response start, not the execution, and reporting
/// it under `http_request_duration_seconds` would be a number that lies for the
/// two endpoints the API actually calls.
pub fn record_request(route: &str, method: &str, status: u16) {
    metrics::counter!(
        "http_requests_total",
        "route" => route.to_owned(),
        "method" => method.to_owned(),
        "status" => status.to_string()
    )
    .increment(1);
}

pub async fn metrics_handler() -> impl IntoResponse {
    PROMETHEUS_HANDLE
        .get()
        .expect("Prometheus recorder must be initialized")
        .render()
}
