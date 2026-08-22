use axum::{
    body::Body,
    extract::MatchedPath,
    http::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

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

struct EnabledTracing {
    tracer: opentelemetry_sdk::trace::Tracer,
    endpoint: String,
    endpoint_var: &'static str,
    timeout: Duration,
}

pub fn init_telemetry() -> Result<(), TelemetryError> {
    let format_layer = tracing_subscriber::fmt::layer();
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info")
            .add_directive("hyper=warn".parse().expect("valid filter"))
            .add_directive("rustls=warn".parse().expect("valid filter"))
            .add_directive("tokio=warn".parse().expect("valid filter"))
    });

    let (otlp, timeout_warning) = init_tracing()?;
    let enabled = otlp
        .as_ref()
        .map(|otlp| (otlp.endpoint.clone(), otlp.endpoint_var, otlp.timeout));

    if let Some(otlp) = otlp {
        tracing_subscriber::registry()
            .with(format_layer)
            .with(env_filter)
            .with(tracing_opentelemetry::layer().with_tracer(otlp.tracer))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(format_layer)
            .with(env_filter)
            .init();
    }

    if let Some(warning) = timeout_warning {
        tracing::warn!("{warning}");
    }
    match enabled {
        Some((endpoint, endpoint_var, timeout)) => tracing::info!(
            "OpenTelemetry tracing enabled (endpoint={endpoint} from {endpoint_var}, export timeout={}ms)",
            timeout.as_millis()
        ),
        None => tracing::info!(
            "OpenTelemetry tracing disabled (neither {} nor {} is set)",
            OTLP_ENDPOINT_VARS[0],
            OTLP_ENDPOINT_VARS[1]
        ),
    }

    init_metrics();
    Ok(())
}

/// Traces are exported only when `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` or
/// `OTEL_EXPORTER_OTLP_ENDPOINT` is set; the Azure root wires the collector to
/// the API alone today. Unlike the Kubernetes executor, an endpoint that is set
/// but cannot be dialled is a startup error rather than a silently disabled
/// exporter.
fn init_tracing() -> Result<(Option<EnabledTracing>, Option<String>), TelemetryError> {
    let (timeout, timeout_warning) = resolve_otlp_export_timeout();
    let Some((endpoint_var, endpoint)) = resolve_otlp_endpoint() else {
        return Ok((None, timeout_warning));
    };
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&endpoint)
        .with_timeout(timeout)
        .build()
        .map_err(|error| {
            TelemetryError(format!(
                "{endpoint_var} is set but the OTLP exporter could not initialize: {error}"
            ))
        })?;
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();
    let tracer = provider.tracer("flow-like-azure-executor");
    opentelemetry::global::set_tracer_provider(provider);
    Ok((
        Some(EnabledTracing {
            tracer,
            endpoint,
            endpoint_var,
            timeout,
        }),
        timeout_warning,
    ))
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
        .set_buckets_for_metric(
            Matcher::Full("flow_execution_duration_seconds".to_string()),
            &[0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0],
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
    metrics::describe_counter!("flow_executions_total", "Total number of flow executions");
    metrics::describe_histogram!(
        "flow_execution_duration_seconds",
        "Flow execution duration in seconds"
    );
    metrics::describe_gauge!("executor_active_jobs", "Number of currently executing jobs");
}

pub async fn handler() -> impl IntoResponse {
    PROMETHEUS_HANDLE
        .get()
        .expect("Prometheus recorder must be initialized")
        .render()
}

/// Same names as the docker-compose runtime so its dashboards and alert rules
/// apply unchanged. `path` is the matched route template, never the raw URI,
/// so an unauthenticated scanner cannot grow the label set. On `/execute` the
/// handler awaits the whole run, so the sample is the run; on the streaming
/// routes the handler returns at the first event and the sample ends there,
/// which is why `flow_execution_duration_seconds` on those routes reads as
/// time-to-first-event and the run's true duration lives in the API's run
/// record.
pub async fn http_middleware(
    matched_path: Option<MatchedPath>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let started = Instant::now();
    let method = request.method().to_string();
    let path = matched_path
        .map(|matched| matched.as_str().to_string())
        .unwrap_or_else(|| "unmatched".to_string());
    let is_execute = path.starts_with("/execute");

    if is_execute {
        metrics::gauge!("executor_active_jobs").increment(1.0);
    }

    let response = next.run(request).await;

    let duration = started.elapsed().as_secs_f64();
    let status = response.status().as_u16();
    metrics::counter!(
        "http_requests_total",
        "method" => method.clone(),
        "path" => path.clone(),
        "status" => status.to_string()
    )
    .increment(1);
    metrics::histogram!(
        "http_request_duration_seconds",
        "method" => method,
        "path" => path
    )
    .record(duration);

    if is_execute {
        metrics::gauge!("executor_active_jobs").decrement(1.0);
        let outcome = if response.status().is_success() {
            "success"
        } else {
            "error"
        };
        metrics::counter!("flow_executions_total", "status" => outcome).increment(1);
        metrics::histogram!("flow_execution_duration_seconds", "status" => outcome)
            .record(duration);
    }

    response
}

#[derive(Debug)]
pub struct TelemetryError(String);

impl std::fmt::Display for TelemetryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TelemetryError {}
