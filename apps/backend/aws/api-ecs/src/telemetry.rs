//! Structured logs, plus opt-in trace export for a long-running task.
//!
//! Sampling matches the Lambda API: every span is recorded, and a trace is
//! kept when the ratio selects its id, any of its spans failed, or it ran
//! slow. Without an invocation boundary to flush at, spans wait in memory
//! until their local root closes; the root decides for its whole subtree and a
//! background task hands kept spans to the collector sidecar. Requests that
//! share a caller's trace id each decide for their own subtree.

use opentelemetry::trace::{
    SpanId, SpanKind, Status, TraceContextExt, TraceId, TracerProvider as _,
};
use opentelemetry::{Context as OtelContext, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{
    Sampler, SamplingDecision, SdkTracerProvider, ShouldSample, Span, SpanData, SpanExporter,
    SpanProcessor,
};
use span_export::{XrayIds, sanitize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinHandle;
use tracing_subscriber::prelude::*;

#[path = "../../shared/span_export.rs"]
mod span_export;

const TARGET: &str = "flow_like::observability";
const DEFAULT_SAMPLE_RATE: f64 = 0.05;
/// The Lambda API's threshold, so both targets keep the same slow requests.
const SLOW_TRACE: Duration = Duration::from_secs(1);
/// A root still open this long (a stream, a worker loop) releases its children.
const HOLD_LIMIT: Duration = Duration::from_secs(60);
const MAX_HELD_SPANS: usize = 16_384;
const MAX_READY_SPANS: usize = 16_384;
const REMEMBERED_ROOTS: usize = 4_096;
const MAX_BATCH: usize = 512;
const EXPORT_INTERVAL: Duration = Duration::from_secs(5);
const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);
/// Shutdown drain plus this budget must fit the task's `stopTimeout`.
const SHUTDOWN_EXPORT_BUDGET: Duration = Duration::from_secs(4);

#[derive(Debug)]
struct Held {
    trace_id: TraceId,
    since: Instant,
    spans: Vec<SpanData>,
}

#[derive(Debug, Default)]
struct Buffer {
    /// Open span → the local root whose decision it follows.
    roots: HashMap<SpanId, SpanId>,
    held: HashMap<SpanId, Held>,
    held_spans: usize,
    decided: HashMap<SpanId, bool>,
    decided_order: VecDeque<SpanId>,
    ready: Vec<SpanData>,
    dropped: u64,
}

impl Buffer {
    fn remember(&mut self, root: SpanId, keep: bool) {
        match self.decided.get_mut(&root) {
            Some(known) => *known |= keep,
            None => {
                self.decided.insert(root, keep);
                self.decided_order.push_back(root);
                while self.decided_order.len() > REMEMBERED_ROOTS {
                    if let Some(oldest) = self.decided_order.pop_front() {
                        self.decided.remove(&oldest);
                    }
                }
            }
        }
    }

    fn release(&mut self, spans: impl IntoIterator<Item = SpanData>) {
        for span in spans {
            if self.ready.len() < MAX_READY_SPANS {
                self.ready.push(span);
            } else {
                self.dropped += 1;
            }
        }
    }

    fn hold(&mut self, root: SpanId, span: SpanData) {
        if self.held_spans >= MAX_HELD_SPANS {
            self.dropped += 1;
            return;
        }
        self.held_spans += 1;
        self.held
            .entry(root)
            .or_insert_with(|| Held {
                trace_id: span.span_context.trace_id(),
                since: Instant::now(),
                spans: Vec::new(),
            })
            .spans
            .push(span);
    }

    fn take_held(&mut self, root: SpanId) -> Vec<SpanData> {
        let spans = self
            .held
            .remove(&root)
            .map(|held| held.spans)
            .unwrap_or_default();
        self.held_spans -= spans.len();
        spans
    }
}

#[derive(Debug)]
struct TraceTail {
    ratio: Sampler,
    buffer: Mutex<Buffer>,
    wake: Notify,
}

impl TraceTail {
    fn new(rate: f64) -> Self {
        Self {
            ratio: Sampler::TraceIdRatioBased(rate),
            buffer: Mutex::default(),
            wake: Notify::new(),
        }
    }

    fn ratio_keeps(&self, trace_id: TraceId) -> bool {
        self.ratio
            .should_sample(None, trace_id, "http.request", &SpanKind::Server, &[], &[])
            .decision
            == SamplingDecision::RecordAndSample
    }

    /// Decide for roots held longer than `limit`, or for all of them at shutdown.
    fn release_held(&self, limit: Duration) {
        let Ok(mut buffer) = self.buffer.lock() else {
            return;
        };
        let expired: Vec<SpanId> = buffer
            .held
            .iter()
            .filter(|(_, held)| held.since.elapsed() >= limit)
            .map(|(root, _)| *root)
            .collect();
        for root in expired {
            let trace_id = buffer.held[&root].trace_id;
            let spans = buffer.take_held(root);
            let keep = self.ratio_keeps(trace_id) || spans.iter().any(failed);
            buffer.remember(root, keep);
            if keep {
                buffer.release(spans);
            }
        }
    }

    fn take_batch(&self) -> (Vec<SpanData>, u64) {
        let Ok(mut buffer) = self.buffer.lock() else {
            return (Vec::new(), 0);
        };
        let count = buffer.ready.len().min(MAX_BATCH);
        let batch = buffer.ready.drain(..count).collect();
        (batch, std::mem::take(&mut buffer.dropped))
    }
}

fn failed(span: &SpanData) -> bool {
    matches!(span.status, Status::Error { .. })
}

fn notable(root: &SpanData) -> bool {
    failed(root)
        || root
            .end_time
            .duration_since(root.start_time)
            .is_ok_and(|duration| duration >= SLOW_TRACE)
}

#[derive(Clone, Debug)]
struct TailProcessor(Arc<TraceTail>);

impl SpanProcessor for TailProcessor {
    fn on_start(&self, span: &mut Span, cx: &OtelContext) {
        use opentelemetry::trace::Span as _;
        let id = span.span_context().span_id();
        let parent = cx.span().span_context().clone();
        let Ok(mut buffer) = self.0.buffer.lock() else {
            return;
        };
        // A parent that already closed is only known if it was a root; work
        // spawned by a finished request still follows that request's decision.
        let root = (parent.is_valid() && !parent.is_remote())
            .then(|| {
                let parent = parent.span_id();
                buffer
                    .roots
                    .get(&parent)
                    .copied()
                    .or_else(|| buffer.decided.contains_key(&parent).then_some(parent))
            })
            .flatten()
            .unwrap_or(id);
        buffer.roots.insert(id, root);
    }

    fn on_end(&self, mut span: SpanData) {
        sanitize(&mut span);
        let tail = &self.0;
        let Ok(mut buffer) = tail.buffer.lock() else {
            return;
        };
        let id = span.span_context.span_id();
        let root = buffer.roots.remove(&id).unwrap_or(id);
        if root == id {
            let mut spans = buffer.take_held(root);
            let keep = tail.ratio_keeps(span.span_context.trace_id())
                || notable(&span)
                || spans.iter().any(failed);
            buffer.remember(root, keep);
            if buffer.decided[&root] {
                spans.push(span);
                buffer.release(spans);
            }
        } else if let Some(keep) = buffer.decided.get(&root).copied() {
            if keep {
                buffer.release([span]);
            }
        } else {
            buffer.hold(root, span);
        }
        let full = buffer.ready.len() >= MAX_BATCH;
        drop(buffer);
        if full {
            tail.wake.notify_one();
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.wake.notify_one();
        Ok(())
    }

    fn shutdown_with_timeout(&self, _: Duration) -> OTelSdkResult {
        Ok(())
    }
}

async fn export_loop<E: SpanExporter>(
    tail: Arc<TraceTail>,
    exporter: E,
    mut stop: oneshot::Receiver<()>,
) {
    let mut interval = tokio::time::interval(EXPORT_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let stopping = tokio::select! {
            _ = interval.tick() => false,
            () = tail.wake.notified() => false,
            _ = &mut stop => true,
        };
        tail.release_held(if stopping { Duration::ZERO } else { HOLD_LIMIT });
        loop {
            let (batch, dropped) = tail.take_batch();
            if dropped > 0 {
                tracing::warn!(dropped, "OpenTelemetry span buffer capacity exceeded");
            }
            if batch.is_empty() {
                break;
            }
            let full = batch.len() == MAX_BATCH;
            let spans = batch.len();
            match tokio::time::timeout(EXPORT_TIMEOUT, exporter.export(batch)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, spans, "OpenTelemetry span export failed")
                }
                Err(_) => tracing::warn!(spans, "OpenTelemetry span export timed out"),
            }
            if !full {
                break;
            }
        }
        if stopping {
            break;
        }
    }
}

struct Enabled {
    provider: SdkTracerProvider,
    stop: oneshot::Sender<()>,
    exporter: JoinHandle<()>,
}

#[derive(Default)]
pub struct Telemetry {
    enabled: Option<Enabled>,
}

pub fn init() -> Result<Telemetry, String> {
    let fmt = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_filter(flow_like_api::warn_env_filter());
    if !std::env::var("FLOW_LIKE_OTEL_ENABLED").is_ok_and(|value| value == "true") {
        tracing_subscriber::registry().with(fmt).init();
        return Ok(Telemetry::default());
    }

    let sample_rate = std::env::var("FLOW_LIKE_OTEL_SAMPLE_RATE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|rate| rate.is_finite() && (0.0..=1.0).contains(rate))
        .unwrap_or(DEFAULT_SAMPLE_RATE);
    let setting = |name: &str, fallback: &str| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| fallback.to_owned())
    };
    // Explicit resources avoid forwarding arbitrary environment attributes.
    // Task and cluster identity come from the collector's ECS resource detector.
    let resource = Resource::builder_empty()
        .with_attributes([
            KeyValue::new(
                "service.name",
                setting("OTEL_SERVICE_NAME", "flow-like-api"),
            ),
            KeyValue::new(
                "deployment.environment.name",
                setting("FLOW_LIKE_ENVIRONMENT", "unknown"),
            ),
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_ecs"),
            KeyValue::new(
                "cloud.region",
                setting("AWS_REGION", &setting("AWS_DEFAULT_REGION", "unknown")),
            ),
        ])
        .build();
    let mut exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(setting(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://127.0.0.1:4317",
        ))
        .with_timeout(EXPORT_TIMEOUT)
        .build()
        .map_err(|error| format!("failed to build the OTLP span exporter: {error}"))?;
    exporter.set_resource(&resource);

    let tail = Arc::new(TraceTail::new(sample_rate));
    let provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_id_generator(XrayIds::default())
        .with_sampler(Sampler::AlwaysOn)
        .with_span_processor(TailProcessor(tail.clone()))
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

    let (stop, stopped) = oneshot::channel();
    let exporter = tokio::spawn(export_loop(tail, exporter, stopped));
    Ok(Telemetry {
        enabled: Some(Enabled {
            provider,
            stop,
            exporter,
        }),
    })
}

impl Telemetry {
    /// Export what is buffered, including traces whose roots are still open.
    pub async fn shutdown(self) {
        let Some(enabled) = self.enabled else {
            return;
        };
        let _ = enabled.stop.send(());
        if tokio::time::timeout(SHUTDOWN_EXPORT_BUDGET, enabled.exporter)
            .await
            .is_err()
        {
            tracing::warn!("OpenTelemetry export did not finish before shutdown");
        }
        drop(enabled.provider);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{Span as _, SpanContext, TraceFlags, TraceState, Tracer as _};
    use std::time::SystemTime;

    const TRACE: u128 = 0x69abcdef_00000000_ffffffff_ffffffff;

    fn provider(
        rate: f64,
    ) -> (
        Arc<TraceTail>,
        opentelemetry_sdk::trace::Tracer,
        SdkTracerProvider,
    ) {
        let tail = Arc::new(TraceTail::new(rate));
        let provider = SdkTracerProvider::builder()
            .with_span_processor(TailProcessor(tail.clone()))
            .build();
        (tail, provider.tracer("test"), provider)
    }

    fn caller(trace: u128, span: u8) -> OtelContext {
        OtelContext::new().with_remote_span_context(SpanContext::new(
            TraceId::from_bytes(trace.to_be_bytes()),
            SpanId::from_bytes([span; 8]),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ))
    }

    fn ready(tail: &TraceTail) -> Vec<String> {
        let mut names: Vec<String> = tail
            .buffer
            .lock()
            .unwrap()
            .ready
            .iter()
            .map(|span| span.name.to_string())
            .collect();
        names.sort();
        names
    }

    fn start(
        tracer: &opentelemetry_sdk::trace::Tracer,
        name: &'static str,
        parent: &OtelContext,
    ) -> OtelContext {
        parent.with_span(tracer.start_with_context(name, parent))
    }

    #[test]
    fn an_ordinary_fast_successful_trace_is_dropped() {
        let (tail, tracer, _provider) = provider(0.0);
        let root = start(&tracer, "http.request", &caller(TRACE, 1));
        start(&tracer, "db.query", &root).span().end();
        root.span().end();
        assert!(ready(&tail).is_empty());
        assert!(tail.buffer.lock().unwrap().roots.is_empty());
    }

    #[test]
    fn a_child_error_keeps_the_whole_subtree() {
        let (tail, tracer, _provider) = provider(0.0);
        let root = start(&tracer, "http.request", &OtelContext::new());
        let child = start(&tracer, "db.query", &root);
        child.span().set_status(Status::error("secret detail"));
        child.span().end();
        assert!(ready(&tail).is_empty(), "held until the root decides");
        root.span().end();
        assert_eq!(ready(&tail), ["db.query", "http.request"]);
        let buffer = tail.buffer.lock().unwrap();
        let exported = buffer.ready.iter().find(|span| span.name == "db.query");
        assert_eq!(exported.unwrap().status, Status::error(""));
    }

    #[test]
    fn slow_and_ratio_selected_roots_are_kept() {
        let (tail, tracer, _provider) = provider(0.0);
        let started = SystemTime::now();
        let mut slow = tracer
            .span_builder("http.request")
            .with_start_time(started)
            .start_with_context(&tracer, &OtelContext::new());
        slow.end_with_timestamp(started + SLOW_TRACE);
        assert_eq!(ready(&tail), ["http.request"]);

        let (tail, tracer, _provider) = provider(1.0);
        let root = start(&tracer, "http.request", &OtelContext::new());
        start(&tracer, "db.query", &root).span().end();
        root.span().end();
        assert_eq!(ready(&tail), ["db.query", "http.request"]);
    }

    #[test]
    fn late_children_follow_their_roots_decision() {
        let (tail, tracer, _provider) = provider(0.0);
        let kept = start(&tracer, "http.request", &OtelContext::new());
        let kept_late = start(&tracer, "background.task", &kept);
        kept.span().set_status(Status::error(""));
        kept.span().end();

        let dropped = start(&tracer, "http.request", &OtelContext::new());
        let dropped_late = start(&tracer, "background.late", &dropped);
        dropped.span().end();

        kept_late.span().end();
        dropped_late.span().end();
        assert_eq!(ready(&tail), ["background.task", "http.request"]);
        assert_eq!(tail.buffer.lock().unwrap().held_spans, 0);
    }

    #[test]
    fn work_started_after_its_root_closed_follows_the_root() {
        let (tail, tracer, _provider) = provider(0.0);
        let kept = start(&tracer, "http.request", &OtelContext::new());
        kept.span().set_status(Status::error(""));
        kept.span().end();
        let dropped = start(&tracer, "http.request", &OtelContext::new());
        dropped.span().end();

        start(&tracer, "spawned.kept", &kept).span().end();
        start(&tracer, "spawned.dropped", &dropped).span().end();
        assert_eq!(ready(&tail), ["http.request", "spawned.kept"]);
    }

    #[test]
    fn requests_sharing_a_caller_trace_decide_independently() {
        let (tail, tracer, _provider) = provider(0.0);
        let failing = start(&tracer, "http.request", &caller(TRACE, 1));
        let healthy = start(&tracer, "http.request", &caller(TRACE, 2));
        start(&tracer, "healthy.query", &healthy).span().end();
        let error = start(&tracer, "failing.query", &failing);
        error.span().set_status(Status::error(""));
        error.span().end();

        healthy.span().end();
        assert!(ready(&tail).is_empty(), "the healthy request took nothing");
        failing.span().end();
        assert_eq!(ready(&tail), ["failing.query", "http.request"]);
    }

    #[test]
    fn open_roots_release_their_children_after_the_hold_limit() {
        let (tail, tracer, _provider) = provider(0.0);
        let stream = start(&tracer, "http.request", &OtelContext::new());
        let chunk = start(&tracer, "stream.chunk", &stream);
        chunk.span().set_status(Status::error(""));
        chunk.span().end();
        tail.release_held(HOLD_LIMIT);
        assert!(ready(&tail).is_empty());
        tail.release_held(Duration::ZERO);
        assert_eq!(ready(&tail), ["stream.chunk"]);
        stream.span().end();
        assert_eq!(ready(&tail), ["http.request", "stream.chunk"]);
    }

    #[test]
    fn held_spans_are_bounded() {
        let (tail, tracer, _provider) = provider(0.0);
        let root = start(&tracer, "http.request", &OtelContext::new());
        for _ in 0..MAX_HELD_SPANS + 1 {
            start(&tracer, "db.query", &root).span().end();
        }
        let buffer = tail.buffer.lock().unwrap();
        assert_eq!(buffer.held_spans, MAX_HELD_SPANS);
        assert_eq!(buffer.dropped, 1);
    }

    #[derive(Debug, Default)]
    struct Recorder(Arc<Mutex<Vec<String>>>);

    impl SpanExporter for Recorder {
        async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
            self.0
                .lock()
                .unwrap()
                .extend(batch.into_iter().map(|span| span.name.to_string()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn shutdown_exports_ready_and_still_open_traces() {
        let (tail, tracer, _provider) = provider(1.0);
        start(&tracer, "finished", &OtelContext::new()).span().end();
        let open = start(&tracer, "http.request", &OtelContext::new());
        start(&tracer, "stream.chunk", &open).span().end();

        let exported = Arc::new(Mutex::new(Vec::new()));
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(export_loop(
            tail.clone(),
            Recorder(exported.clone()),
            stopped,
        ));
        stop.send(()).unwrap();
        task.await.unwrap();

        let mut names = exported.lock().unwrap().clone();
        names.sort();
        assert_eq!(names, ["finished", "stream.chunk"]);
    }
}
