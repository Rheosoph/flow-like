use axum::response::IntoResponse;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{runtime, trace::TracerProvider};
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
        tracing::info!("OpenTelemetry tracing disabled");
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

    let provider = TracerProvider::builder()
        .with_batch_exporter(exporter, runtime::Tokio)
        .build();

    let tracer = provider.tracer("flow-like-compiler");
    opentelemetry::global::set_tracer_provider(provider);

    Some(tracer)
}

fn init_metrics() {
    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("compilation_duration_seconds".to_string()),
            &[0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0],
        )
        .unwrap()
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    PROMETHEUS_HANDLE
        .set(handle)
        .expect("metrics already initialized");

    metrics::describe_counter!("compilations_total", "Total number of WASM compilations");
    metrics::describe_histogram!(
        "compilation_duration_seconds",
        "WASM compilation duration in seconds"
    );

    tracing::info!("Prometheus metrics initialized");
}

pub async fn handler() -> impl IntoResponse {
    let handle = PROMETHEUS_HANDLE.get().expect("metrics not initialized");
    handle.render()
}
