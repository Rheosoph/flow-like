//! Lambda owns the export boundary: hand completed spans to the local collector
//! before response EOF, while the runtime can still poll asynchronous IO.

use axum::body::Body;
use flow_like_types::tokio;
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use lambda_http::{Request, RequestExt, Response};
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState, TracerProvider,
};
use opentelemetry::{Context as OtelContext, KeyValue, Value};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{
    IdGenerator, RandomIdGenerator, Sampler, SdkTracerProvider, SpanData, SpanExporter,
    SpanProcessor,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tower::Service;
use tracing::{Instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::prelude::*;

const TARGET: &str = "flow_like::observability";
const MAX_QUEUED_SPANS: usize = 2048;
const EXPORT_BUDGET: Duration = Duration::from_millis(200);
type FlushFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

#[derive(Clone, Debug, Default)]
struct InvocationSpans {
    queue: Arc<Mutex<Vec<SpanData>>>,
    dropped: Arc<AtomicU64>,
}

impl SpanProcessor for InvocationSpans {
    fn on_start(&self, _: &mut opentelemetry_sdk::trace::Span, _: &OtelContext) {}

    fn on_end(&self, mut span: SpanData) {
        sanitize(&mut span);
        if let Ok(mut queue) = self.queue.lock() {
            if queue.len() < MAX_QUEUED_SPANS {
                queue.push(span);
                return;
            }
        }
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    fn force_flush(&self) -> OTelSdkResult {
        // A synchronous SDK flush would deadlock the current-thread runtime.
        // The invocation wrapper below performs the asynchronous handoff.
        if self.queue.lock().is_ok_and(|queue| queue.is_empty()) {
            Ok(())
        } else {
            Err(OTelSdkError::InternalFailure(
                "use the asynchronous invocation flush".into(),
            ))
        }
    }

    fn shutdown_with_timeout(&self, _: Duration) -> OTelSdkResult {
        self.force_flush()
    }
}

fn sanitize(span: &mut SpanData) {
    // Only our explicit target is collected. This second boundary prevents a
    // future instrument(skip_all) omission from exporting handler arguments.
    span.attributes
        .retain(|attribute| safe_attribute(attribute));
    span.events = Default::default();
    span.links = Default::default();
    if matches!(span.status, opentelemetry::trace::Status::Error { .. })
        || span
            .attributes
            .iter()
            .any(|attribute| attribute.key.as_str() == "error.type")
    {
        span.status = opentelemetry::trace::Status::error("");
    }
    if span.name.len() > 96
        || !span
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        span.name = "operation".into();
    }
}

fn safe_attribute(attribute: &KeyValue) -> bool {
    let key = attribute.key.as_str();
    match (&attribute.value, key) {
        (Value::I64(_), "http.status_code" | "http.response.status_code" | "retry_count") => true,
        (Value::Bool(_), "faas.coldstart" | "http.cancelled") => true,
        (
            Value::F64(value),
            "http.response_ready_ms" | "http.first_byte_ms" | "http.duration_ms",
        ) => value.is_finite() && *value >= 0.0,
        (Value::String(value), "http.route") => {
            let value = value.as_str();
            value.len() <= 256
                && (value == "unmatched" || value.starts_with('/'))
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/_-.{}:*".contains(&b))
        }
        (
            Value::String(value),
            "http.method"
            | "http.request.method"
            | "db.operation"
            | "db.system.name"
            | "db.table"
            | "rpc.service"
            | "rpc.method"
            | "cloud.service"
            | "error.type",
        ) => {
            value.as_str().len() <= 64
                && value
                    .as_str()
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        }
        (Value::String(value), "faas.invocation_id") => {
            let value = value.as_str();
            value.len() == 36
                && value.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit()
                    }
                })
        }
        _ => false,
    }
}

#[derive(Debug, Default)]
struct XrayIds(RandomIdGenerator);

impl IdGenerator for XrayIds {
    fn new_trace_id(&self) -> TraceId {
        let mut bytes = self.0.new_trace_id().to_bytes();
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        bytes[..4].copy_from_slice(&seconds.to_be_bytes());
        TraceId::from_bytes(bytes)
    }

    fn new_span_id(&self) -> SpanId {
        self.0.new_span_id()
    }
}

trait LocalExporter: Send + Sync {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> Pin<Box<dyn Future<Output = OTelSdkResult> + Send + '_>>;
}

impl LocalExporter for opentelemetry_otlp::SpanExporter {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> Pin<Box<dyn Future<Output = OTelSdkResult> + Send + '_>> {
        Box::pin(SpanExporter::export(self, batch))
    }
}

struct Enabled {
    _provider: Option<SdkTracerProvider>,
    spans: InvocationSpans,
    exporter: Box<dyn LocalExporter>,
    export_lock: tokio::sync::Mutex<()>,
}

#[derive(Clone, Default)]
pub struct Telemetry {
    inner: Option<Arc<Enabled>>,
    cold: Arc<AtomicBool>,
}

pub fn init() -> Result<Telemetry, lambda_http::Error> {
    let enabled = std::env::var("FLOW_LIKE_OTEL_ENABLED").is_ok_and(|value| value == "true");
    let fmt = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_filter(flow_like_api::warn_env_filter());
    if !enabled {
        tracing_subscriber::registry().with(fmt).init();
        return Ok(Telemetry::default());
    }

    let sample_rate = std::env::var("FLOW_LIKE_OTEL_SAMPLE_RATE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|rate| rate.is_finite() && (0.0..=1.0).contains(rate))
        .unwrap_or(0.05);
    // Explicit resources avoid forwarding arbitrary environment attributes.
    let resource = Resource::builder_empty()
        .with_attributes([
            KeyValue::new(
                "service.name",
                std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "flow-like-api".into()),
            ),
            KeyValue::new(
                "deployment.environment.name",
                std::env::var("FLOW_LIKE_ENVIRONMENT").unwrap_or_else(|_| "unknown".into()),
            ),
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_lambda"),
            KeyValue::new(
                "faas.name",
                std::env::var("AWS_LAMBDA_FUNCTION_NAME")
                    .unwrap_or_else(|_| "flow-like-api".into()),
            ),
            KeyValue::new(
                "cloud.region",
                std::env::var("AWS_REGION").unwrap_or_else(|_| "unknown".into()),
            ),
        ])
        .build();
    let mut exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(
            std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .unwrap_or_else(|_| "http://127.0.0.1:4317".into()),
        )
        .with_timeout(EXPORT_BUDGET)
        .build()?;
    exporter.set_resource(&resource);
    let spans = InvocationSpans::default();
    let provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_id_generator(XrayIds::default())
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            sample_rate,
        ))))
        .with_span_processor(spans.clone())
        .build();
    let layer = tracing_opentelemetry::layer()
        .with_tracer(provider.tracer("flow-like-api"))
        .with_location(false)
        .with_threads(false)
        .with_level(false)
        .with_target(false)
        .with_error_events_to_exceptions(false)
        .with_error_records_to_exceptions(false)
        .with_error_fields_to_exceptions(false)
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.is_span() && metadata.target() == TARGET
        }));
    tracing_subscriber::registry().with(fmt).with(layer).init();
    Ok(Telemetry {
        inner: Some(Arc::new(Enabled {
            _provider: Some(provider),
            spans,
            exporter: Box::new(exporter),
            export_lock: tokio::sync::Mutex::new(()),
        })),
        cold: Arc::new(AtomicBool::new(true)),
    })
}

impl Telemetry {
    fn enabled(&self) -> bool {
        self.inner.is_some()
    }

    #[cfg(test)]
    fn for_test(
        spans: InvocationSpans,
        callback: Arc<dyn Fn(Vec<SpanData>) -> FlushFuture + Send + Sync>,
    ) -> Self {
        struct TestExporter(Arc<dyn Fn(Vec<SpanData>) -> FlushFuture + Send + Sync>);
        impl LocalExporter for TestExporter {
            fn export(
                &self,
                batch: Vec<SpanData>,
            ) -> Pin<Box<dyn Future<Output = OTelSdkResult> + Send + '_>> {
                let future = self.0(batch);
                Box::pin(async move {
                    future.await;
                    Ok(())
                })
            }
        }
        Self {
            inner: Some(Arc::new(Enabled {
                _provider: None,
                spans,
                exporter: Box::new(TestExporter(callback)),
                export_lock: tokio::sync::Mutex::new(()),
            })),
            cold: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn wrap<S>(&self, inner: S) -> Invocation<S> {
        Invocation {
            inner,
            telemetry: self.clone(),
        }
    }

    pub async fn flush(&self) {
        let Some(inner) = &self.inner else {
            return;
        };
        let work = async {
            let _guard = inner.export_lock.lock().await;
            let batch = inner
                .spans
                .queue
                .lock()
                .map(|mut queue| std::mem::take(&mut *queue))
                .unwrap_or_default();
            if batch.is_empty() {
                return Ok(());
            }
            inner.exporter.export(batch).await
        };
        if !matches!(tokio::time::timeout(EXPORT_BUDGET, work).await, Ok(Ok(()))) {
            tracing::warn!("OpenTelemetry local export failed or exceeded its time budget");
        }
        let dropped = inner.spans.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            tracing::warn!(dropped, "OpenTelemetry span queue capacity exceeded");
        }
    }
}

/// Lambda provides this value per invocation. Never cache it at startup or
/// trust a viewer-controlled X-Amzn-Trace-Id header as the platform parent.
fn xray_parent(header: &str) -> Option<OtelContext> {
    let mut root = None;
    let mut parent = None;
    let mut sampled = None;
    for field in header.split(';') {
        let (key, value) = field.trim().split_once('=')?;
        match key {
            "Root" if root.is_none() => root = Some(value),
            "Parent" if parent.is_none() => parent = Some(value),
            "Sampled" if sampled.is_none() => {
                sampled = Some(match value {
                    "1" => true,
                    "0" => false,
                    _ => return None,
                })
            }
            "Root" | "Parent" | "Sampled" => return None,
            _ => {}
        }
    }
    let root = root?;
    let parts = root.split('-').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "1" || parts[1].len() != 8 || parts[2].len() != 24 {
        return None;
    }
    let trace_id = TraceId::from_hex(&format!("{}{}", parts[1], parts[2])).ok()?;
    let parent = parent?;
    if parent.len() != 16 {
        return None;
    }
    let span_id = SpanId::from_hex(parent).ok()?;
    let context = SpanContext::new(
        trace_id,
        span_id,
        if sampled? {
            TraceFlags::SAMPLED
        } else {
            TraceFlags::NOT_SAMPLED
        },
        true,
        TraceState::default(),
    );
    context
        .is_valid()
        .then(|| OtelContext::new().with_remote_span_context(context))
}

#[derive(Clone)]
pub struct Invocation<S> {
    inner: S,
    telemetry: Telemetry,
}

impl<S> Service<Request> for Invocation<S>
where
    S: Service<Request, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let replacement = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, replacement);
        let telemetry = self.telemetry.clone();
        if !telemetry.enabled() {
            return Box::pin(inner.call(request));
        }
        let cold = telemetry.cold.swap(false, Ordering::Relaxed);
        let span = tracing::info_span!(target: "flow_like::observability", parent: None, "lambda.invocation", otel.kind = "server", faas.coldstart = cold, faas.invocation_id = tracing::field::Empty, otel.status_code = tracing::field::Empty);
        if let Some(context) = request.lambda_context_ref() {
            span.record("faas.invocation_id", context.request_id.as_str());
            if let Some(parent) = context.xray_trace_id.as_deref().and_then(xray_parent) {
                let _ = span.set_parent(parent);
            }
        }
        Box::pin(async move {
            match inner.call(request).instrument(span.clone()).await {
                Ok(response) => {
                    let (parts, body) = response.into_parts();
                    if parts.status.is_server_error() {
                        span.record("otel.status_code", "ERROR");
                    }
                    Ok(Response::from_parts(
                        parts,
                        Body::new(FlushBody::new(body, span, telemetry)),
                    ))
                }
                Err(error) => {
                    span.record("otel.status_code", "ERROR");
                    drop(span);
                    telemetry.flush().await;
                    Err(error)
                }
            }
        })
    }
}

struct FlushBody {
    inner: Body,
    span: Option<Span>,
    telemetry: Telemetry,
    flushing: Option<FlushFuture>,
    trailers: Option<Frame<hyper::body::Bytes>>,
    error: Option<axum::Error>,
    done: bool,
}

impl FlushBody {
    fn new(inner: Body, span: Span, telemetry: Telemetry) -> Self {
        Self {
            inner,
            span: Some(span),
            telemetry,
            flushing: None,
            trailers: None,
            error: None,
            done: false,
        }
    }

    fn begin_flush(&mut self) {
        // The inner route body has already closed its span at EOF/error. Close
        // the invocation before draining, so this batch includes both roots.
        self.span.take();
        let telemetry = self.telemetry.clone();
        self.flushing = Some(Box::pin(async move { telemetry.flush().await }));
    }
}

impl HttpBody for FlushBody {
    type Data = hyper::body::Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if self.done {
            return Poll::Ready(None);
        }
        if self.flushing.is_none() {
            let polled = {
                let span = self.span.clone().unwrap_or_else(Span::none);
                let _entered = span.enter();
                Pin::new(&mut self.inner).poll_frame(cx)
            };
            match polled {
                Poll::Ready(None) => self.begin_flush(),
                Poll::Ready(Some(Ok(frame))) if frame.is_trailers() => {
                    // lambda_http treats trailers as EOF and will not poll us
                    // again, so complete the handoff before forwarding them.
                    self.trailers = Some(frame);
                    self.begin_flush();
                }
                Poll::Ready(Some(Err(error))) => {
                    if let Some(span) = &self.span {
                        span.record("otel.status_code", "ERROR");
                    }
                    self.error = Some(error);
                    self.begin_flush();
                }
                other => return other,
            }
        }
        if self
            .flushing
            .as_mut()
            .unwrap()
            .as_mut()
            .poll(cx)
            .is_pending()
        {
            return Poll::Pending;
        }
        self.flushing.take();
        self.done = true;
        if let Some(trailers) = self.trailers.take() {
            return Poll::Ready(Some(Ok(trailers)));
        }
        Poll::Ready(self.error.take().map(Err))
    }

    fn is_end_stream(&self) -> bool {
        self.done
    }
    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

#[cfg(test)]
#[path = "telemetry/tests.rs"]
mod lifecycle_tests;

impl Drop for FlushBody {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // Cancellation may race runtime shutdown. Close the route body first;
        // flush best-effort while the runtime is still alive. Hard resets cannot
        // guarantee delivery, which is why request metrics do not use sampling.
        self.inner = Body::empty();
        if let Some(span) = self.span.take() {
            span.record("otel.status_code", "ERROR");
        }
        let telemetry = self.telemetry.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { telemetry.flush().await });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::Tracer;

    #[test]
    fn platform_context_preserves_ids_and_sampling() {
        for flag in ["0", "1"] {
            let context = xray_parent(&format!(
                "Root=1-69abcdef-0123456789abcdef01234567;Parent=0123456789abcdef;Sampled={flag}"
            ))
            .unwrap();
            let span = context.span();
            assert_eq!(
                span.span_context().trace_id().to_string(),
                "69abcdef0123456789abcdef01234567"
            );
            assert_eq!(
                span.span_context().span_id().to_string(),
                "0123456789abcdef"
            );
            assert_eq!(span.span_context().is_sampled(), flag == "1");
            assert!(span.span_context().is_remote());
        }
    }

    #[test]
    fn rejects_invalid_or_ambiguous_platform_context() {
        for value in [
            "",
            "Root=bad;Parent=bad;Sampled=1",
            "Root=1-00000000-000000000000000000000000;Parent=0123456789abcdef;Sampled=1",
            "Root=1-69abcdef-0123456789abcdef01234567;Parent=0000000000000000;Sampled=1",
            "Root=1-69abcdef-0123456789abcdef01234567;Parent=0123456789abcdef;Sampled=?",
            "Root=1-69abcdef-0123456789abcdef01234567;Parent=0123456789abcdef;Sampled=1;Sampled=0",
        ] {
            assert!(xray_parent(value).is_none(), "{value}");
        }
    }

    #[test]
    fn exporter_boundary_drops_payloads_events_and_error_messages() {
        let queue = InvocationSpans::default();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(queue.clone())
            .build();
        let tracer = provider.tracer("test");
        let mut span = tracer.start("auth.authenticate");
        use opentelemetry::trace::Span as _;
        span.set_attribute(KeyValue::new("jwt", "secret"));
        span.set_attribute(KeyValue::new("db.statement", "SELECT secret FROM users"));
        span.set_attribute(KeyValue::new("http.route", "/user/{sub}"));
        span.set_attribute(KeyValue::new("db.operation", "SELECT"));
        span.add_event("secret payload", vec![]);
        span.set_status(opentelemetry::trace::Status::error("private user data"));
        span.end();
        let spans = queue.queue.lock().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].attributes.len(), 2);
        assert!(spans[0].events.is_empty());
        assert_eq!(spans[0].status, opentelemetry::trace::Status::error(""));
    }

    #[test]
    fn queue_is_bounded() {
        let queue = InvocationSpans::default();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(queue.clone())
            .build();
        let tracer = provider.tracer("test");
        for _ in 0..MAX_QUEUED_SPANS + 1 {
            drop(tracer.start("db.query"));
        }
        assert_eq!(queue.queue.lock().unwrap().len(), MAX_QUEUED_SPANS);
        assert_eq!(queue.dropped.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn empty_and_streaming_bodies_reach_flush_and_preserve_bytes() {
        for content in ["", "stream body"] {
            let body = Body::new(FlushBody::new(
                Body::from(content),
                Span::none(),
                Telemetry::default(),
            ));
            assert!(!body.is_end_stream());
            let bytes = axum::body::to_bytes(body, 1024).await.unwrap();
            assert_eq!(bytes.as_ref(), content.as_bytes());
        }
    }
}
