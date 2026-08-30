use axum::response::IntoResponse;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::sync::OnceLock;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub use metrics::{counter, histogram};

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
    let fmt_layer = tracing_subscriber::fmt::layer();
    let env_filter = tracing_subscriber::EnvFilter::from_default_env();

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
            let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
            tracing_subscriber::registry()
                .with(fmt_layer)
                .with(env_filter)
                .with(otel_layer)
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
                .with(fmt_layer)
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

fn init_tracing() -> (TracingSetup, Option<String>) {
    let (timeout, timeout_warning) = resolve_otlp_export_timeout();
    let Some((endpoint_var, endpoint)) = resolve_otlp_endpoint() else {
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

    let tracer = provider.tracer("flow-like-api");
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
        .unwrap()
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    PROMETHEUS_HANDLE
        .set(handle)
        .expect("metrics already initialized");

    metrics::describe_counter!("http_requests_total", "Total number of HTTP requests");
    metrics::describe_histogram!(
        "http_request_duration_seconds",
        "HTTP request duration in seconds"
    );
    metrics::describe_gauge!("api_active_connections", "Number of active connections");

    tracing::info!("Prometheus metrics initialized");
}

pub async fn handler() -> impl IntoResponse {
    let handle = PROMETHEUS_HANDLE.get().expect("metrics not initialized");
    handle.render()
}
