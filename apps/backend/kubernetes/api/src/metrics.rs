use axum::response::IntoResponse;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::sync::OnceLock;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

pub fn init_telemetry() {
    let fmt_layer = tracing_subscriber::fmt::layer();
    let env_filter = tracing_subscriber::EnvFilter::from_default_env();

    if let Some(tracer) = init_tracing() {
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        tracing_subscriber::registry()
            .with(fmt_layer)
            .with(env_filter)
            .with(otel_layer)
            .init();
        tracing::info!("OpenTelemetry tracing enabled");
    } else {
        tracing_subscriber::registry()
            .with(fmt_layer)
            .with(env_filter)
            .init();
        tracing::info!("OpenTelemetry tracing disabled (OTEL_EXPORTER_OTLP_ENDPOINT not set)");
    }

    init_metrics();
}

fn init_tracing() -> Option<opentelemetry_sdk::trace::Tracer> {
    let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok()?;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&otlp_endpoint)
        .build()
        .ok()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();

    let tracer = provider.tracer("flow-like-api");
    opentelemetry::global::set_tracer_provider(provider);

    Some(tracer)
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
