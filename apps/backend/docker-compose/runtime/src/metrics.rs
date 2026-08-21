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

    let tracer = provider.tracer("flow-like-runtime");
    opentelemetry::global::set_tracer_provider(provider);

    Some(tracer)
}

fn init_metrics() {
    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("flow_execution_duration_seconds".to_string()),
            &[0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0],
        )
        .unwrap()
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    PROMETHEUS_HANDLE
        .set(handle)
        .expect("metrics already initialized");

    metrics::describe_counter!("flow_executions_total", "Total number of flow executions");
    metrics::describe_histogram!(
        "flow_execution_duration_seconds",
        "Flow execution duration in seconds"
    );
    metrics::describe_gauge!("executor_active_jobs", "Number of currently executing jobs");
    metrics::describe_counter!("http_requests_total", "Total HTTP requests");
    metrics::describe_histogram!(
        "http_request_duration_seconds",
        "HTTP request duration in seconds"
    );

    tracing::info!("Prometheus metrics initialized");
}

pub async fn handler() -> impl IntoResponse {
    let handle = PROMETHEUS_HANDLE.get().expect("metrics not initialized");
    handle.render()
}

pub fn record_execution(status: &str, duration_secs: f64) {
    metrics::counter!("flow_executions_total", "status" => status.to_string()).increment(1);
    metrics::histogram!("flow_execution_duration_seconds", "status" => status.to_string())
        .record(duration_secs);
}

pub fn increment_active_jobs() {
    metrics::gauge!("executor_active_jobs").increment(1.0);
}

pub fn decrement_active_jobs() {
    metrics::gauge!("executor_active_jobs").decrement(1.0);
}

pub fn record_http_request(method: &str, path: &str, status: u16, duration_secs: f64) {
    metrics::counter!("http_requests_total",
        "method" => method.to_string(),
        "path" => path.to_string(),
        "status" => status.to_string()
    )
    .increment(1);
    metrics::histogram!("http_request_duration_seconds",
        "method" => method.to_string(),
        "path" => path.to_string()
    )
    .record(duration_secs);
}
