//! Lambda owns the export boundary: hand completed spans to the local collector
//! before response EOF, while the runtime can still poll asynchronous IO.

use axum::body::Body;
use flow_like_types::tokio;
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use lambda_http::{Request, RequestExt, Response};
use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, TraceContextExt, TraceFlags, TraceId, TraceState, TracerProvider,
};
use opentelemetry::{Context as OtelContext, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{
    Sampler, SamplingDecision, SdkTracerProvider, ShouldSample, SpanData, SpanExporter,
    SpanProcessor,
};
use span_export::{XrayIds, sanitize};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tower::Service;
use tracing::{Instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::prelude::*;

#[path = "../../shared/span_export.rs"]
mod span_export;

const TARGET: &str = "flow_like::observability";
const MAX_QUEUED_SPANS: usize = 2048;
const EXPORT_BUDGET: Duration = Duration::from_millis(200);
const INVOCATION: &str = "lambda.invocation";
/// Between dev p90 (338 ms) and p99 (2.4 s): keeps roughly the slowest 3%.
const SLOW_INVOCATION: Duration = Duration::from_secs(1);
const REMEMBERED_TRACES: usize = 64;
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

/// Every invocation is recorded; export is decided per trace id when the batch
/// is flushed. A trace whose invocation span is in the batch is kept when the
/// ratio selects it, any of its spans failed, it ran slow, it was a cold start,
/// or the platform sampled it. Late spans of other traces reuse a remembered
/// decision or fall back to the ratio, so they never flip the current trace.
#[derive(Debug)]
struct TailSampling {
    ratio: Sampler,
    remembered: Mutex<VecDeque<(TraceId, bool)>>,
}

impl TailSampling {
    fn new(rate: f64) -> Self {
        Self {
            ratio: Sampler::TraceIdRatioBased(rate),
            remembered: Mutex::default(),
        }
    }

    fn ratio_keeps(&self, trace_id: TraceId) -> bool {
        self.ratio
            .should_sample(None, trace_id, INVOCATION, &SpanKind::Server, &[], &[])
            .decision
            == SamplingDecision::RecordAndSample
    }

    fn retain(&self, mut batch: Vec<SpanData>) -> Vec<SpanData> {
        let mut decisions = HashMap::new();
        for span in batch.iter().filter(|span| span.name == INVOCATION) {
            let trace_id = span.span_context.trace_id();
            let keep = decisions.entry(trace_id).or_insert(false);
            *keep = *keep || notable_invocation(span) || self.ratio_keeps(trace_id);
        }
        for span in &batch {
            if let Some(keep) = decisions.get_mut(&span.span_context.trace_id()) {
                *keep = *keep || matches!(span.status, opentelemetry::trace::Status::Error { .. });
            }
        }
        let mut remembered = self.remembered.lock().ok();
        if let Some(remembered) = remembered.as_mut() {
            for (trace_id, keep) in &decisions {
                remembered.retain(|(known, _)| known != trace_id);
                remembered.push_back((*trace_id, *keep));
            }
            while remembered.len() > REMEMBERED_TRACES {
                remembered.pop_front();
            }
        }
        batch.retain(|span| {
            let trace_id = span.span_context.trace_id();
            *decisions.entry(trace_id).or_insert_with(|| {
                remembered
                    .as_ref()
                    .and_then(|remembered| {
                        remembered
                            .iter()
                            .find_map(|(known, keep)| (*known == trace_id).then_some(*keep))
                    })
                    .unwrap_or_else(|| self.ratio_keeps(trace_id))
            })
        });
        batch
    }
}

fn notable_invocation(span: &SpanData) -> bool {
    // The invocation span has a parent only when the platform header said Sampled=1.
    span.parent_span_id != SpanId::INVALID
        || span
            .attributes
            .contains(&KeyValue::new("faas.coldstart", true))
        || span
            .end_time
            .duration_since(span.start_time)
            .is_ok_and(|duration| duration >= SLOW_INVOCATION)
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
    sampling: TailSampling,
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
        .with_sampler(Sampler::AlwaysOn)
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
            sampling: TailSampling::new(sample_rate),
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
        sample_rate: f64,
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
                sampling: TailSampling::new(sample_rate),
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
            let batch = inner.sampling.retain(batch);
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
            // An unsampled platform parent is never exported, so start a new root.
            if let Some(parent) = context
                .xray_trace_id
                .as_deref()
                .and_then(xray_parent)
                .filter(|parent| parent.span().span_context().is_sampled())
            {
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

    const RATIO_KEEPS: u128 = 0x69abcdef_00000000_00000000_00000001;
    const RATIO_DROPS: u128 = 0x69abcdef_00000000_ffffffff_ffffffff;
    const OTHER_RATIO_DROPS: u128 = 0x69abcdf0_00000000_fffffffe_ffffffff;

    fn trace(value: u128) -> TraceId {
        TraceId::from_bytes(value.to_be_bytes())
    }

    fn recorded(name: &'static str, trace_id: u128) -> SpanData {
        let queue = InvocationSpans::default();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(queue.clone())
            .build();
        drop(provider.tracer("test").start(name));
        let mut span = queue.queue.lock().unwrap().pop().unwrap();
        span.span_context = SpanContext::new(
            trace(trace_id),
            span.span_context.span_id(),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        );
        span.parent_span_id = SpanId::INVALID;
        span.end_time = span.start_time + Duration::from_millis(40);
        span
    }

    fn invocation(trace_id: u128) -> SpanData {
        let mut span = recorded(INVOCATION, trace_id);
        span.attributes = vec![KeyValue::new("faas.coldstart", false)];
        span
    }

    fn names(batch: &[SpanData]) -> Vec<&str> {
        batch.iter().map(|span| span.name.as_ref()).collect()
    }

    #[test]
    fn ratio_decision_is_deterministic_per_trace_id() {
        let first = TailSampling::new(0.05);
        let second = TailSampling::new(0.05);
        assert!(first.ratio_keeps(trace(RATIO_KEEPS)));
        assert!(!first.ratio_keeps(trace(RATIO_DROPS)));
        let kept = (0..10_000u64)
            .map(|i| trace(RATIO_DROPS & !u128::from(u64::MAX) | u128::from(u64::MAX / 10_000 * i)))
            .filter(|trace_id| {
                let keep = first.ratio_keeps(*trace_id);
                assert_eq!(keep, first.ratio_keeps(*trace_id));
                assert_eq!(keep, second.ratio_keeps(*trace_id));
                keep
            })
            .count();
        assert!((450..=550).contains(&kept), "{kept}");
    }

    #[test]
    fn an_ordinary_fast_successful_trace_is_dropped() {
        let sampling = TailSampling::new(0.05);
        let batch = vec![
            recorded("http.request", RATIO_DROPS),
            invocation(RATIO_DROPS),
        ];
        assert!(sampling.retain(batch).is_empty());
    }

    #[test]
    fn each_keep_rule_exports_the_whole_invocation_trace() {
        type Apply = fn(&mut SpanData, &mut SpanData);
        let rules: [(&str, u128, Apply); 6] = [
            ("ratio", RATIO_KEEPS, |_, _| {}),
            ("child error", RATIO_DROPS, |child, _| {
                child.status = opentelemetry::trace::Status::error("")
            }),
            ("invocation error", RATIO_DROPS, |_, root| {
                root.status = opentelemetry::trace::Status::error("")
            }),
            ("slow", RATIO_DROPS, |_, root| {
                root.end_time = root.start_time + SLOW_INVOCATION
            }),
            ("cold start", RATIO_DROPS, |_, root| {
                root.attributes = vec![KeyValue::new("faas.coldstart", true)]
            }),
            ("platform sampled", RATIO_DROPS, |_, root| {
                root.parent_span_id = SpanId::from_bytes([1; 8]);
                root.parent_span_is_remote = true;
            }),
        ];
        for (rule, trace_id, apply) in rules {
            let mut child = recorded("db.query", trace_id);
            let mut root = invocation(trace_id);
            apply(&mut child, &mut root);
            let kept = TailSampling::new(0.05).retain(vec![child, root]);
            assert_eq!(names(&kept), ["db.query", INVOCATION], "{rule}");
        }
    }

    #[test]
    fn late_spans_follow_their_own_trace_decision() {
        let sampling = TailSampling::new(0.05);
        let mut late_error = recorded("background.task", OTHER_RATIO_DROPS);
        late_error.status = opentelemetry::trace::Status::error("");
        assert!(
            sampling
                .retain(vec![late_error, invocation(RATIO_DROPS)])
                .is_empty(),
            "an unknown late error neither keeps itself nor the current trace"
        );

        let late_unknown = recorded("background.task", RATIO_KEEPS);
        let kept = sampling.retain(vec![late_unknown, invocation(RATIO_DROPS)]);
        assert_eq!(
            names(&kept),
            ["background.task"],
            "unknown traces use the ratio"
        );

        let mut cold = invocation(OTHER_RATIO_DROPS);
        cold.attributes = vec![KeyValue::new("faas.coldstart", true)];
        assert_eq!(sampling.retain(vec![cold]).len(), 1);
        let late = recorded("background.task", OTHER_RATIO_DROPS);
        let kept = sampling.retain(vec![late, invocation(RATIO_DROPS)]);
        assert_eq!(
            names(&kept),
            ["background.task"],
            "a late span of a kept trace follows the remembered decision"
        );
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
