//! Request timings measured through response-body completion, without a database sink.

use axum::{body::Body, http::StatusCode};
use bytes::Bytes;
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use serde_json::{Value, json};
use std::{
    io::Write,
    pin::Pin,
    sync::{Arc, OnceLock},
    task::{Context, Poll},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tracing::Span;

use super::spans::{FIELD_STATUS, truncate};

#[derive(Clone)]
struct MetricsConfig {
    service: String,
    environment: String,
}

impl MetricsConfig {
    fn from_env() -> Option<Arc<Self>> {
        if !std::env::var("FLOW_LIKE_OTEL_ENABLED").is_ok_and(|value| value == "true") {
            return None;
        }
        let label = |key, fallback: &str| {
            let value = std::env::var(key).unwrap_or_default();
            let value = value.trim();
            truncate(if value.is_empty() { fallback } else { value }, 256)
        };
        Some(Arc::new(Self {
            service: label("OTEL_SERVICE_NAME", "flow-like-api"),
            environment: label("FLOW_LIKE_ENVIRONMENT", "unknown"),
        }))
    }
}

fn metrics_config() -> Option<Arc<MetricsConfig>> {
    static CONFIG: OnceLock<Option<Arc<MetricsConfig>>> = OnceLock::new();
    CONFIG.get_or_init(MetricsConfig::from_env).clone()
}

pub(crate) fn method_label(method: &str) -> &str {
    match method {
        "GET" | "HEAD" | "POST" | "PUT" | "DELETE" | "CONNECT" | "OPTIONS" | "TRACE" | "PATCH" => {
            method
        }
        _ => "OTHER",
    }
}

#[derive(Clone, Copy)]
enum Outcome {
    Complete,
    Error,
    Cancelled,
}

/// Owns the span while the handler runs and while its response body is polled.
/// Dropping either future or body records cancellation exactly once.
pub(crate) struct RequestLifetime {
    span: Option<Span>,
    started: Instant,
    response_ready: Option<Duration>,
    first_byte: Option<Duration>,
    status: Option<StatusCode>,
    method: String,
    route: String,
    metrics: Option<Arc<MetricsConfig>>,
    trace_id: Option<String>,
}

impl RequestLifetime {
    pub(crate) fn new(span: Span, method: String, route: String, started: Instant) -> Self {
        #[cfg(feature = "otel")]
        let trace_id = {
            use opentelemetry::trace::TraceContextExt;
            use tracing_opentelemetry::OpenTelemetrySpanExt;
            let context = span.context();
            let current = context.span();
            let context = current.span_context();
            context.is_valid().then(|| context.trace_id().to_string())
        };
        #[cfg(not(feature = "otel"))]
        let trace_id = None;

        Self {
            span: Some(span),
            started,
            response_ready: None,
            first_byte: None,
            status: None,
            method,
            route,
            metrics: metrics_config(),
            trace_id,
        }
    }

    pub(crate) fn response_ready(&mut self, status: StatusCode) {
        let elapsed = self.started.elapsed();
        self.response_ready = Some(elapsed);
        self.status = Some(status);
        if let Some(span) = &self.span {
            span.record("http.status_code", u64::from(status.as_u16()));
            span.record("http.response_ready_ms", milliseconds(elapsed));
        }
    }

    fn first_byte(&mut self) {
        if self.first_byte.is_none() {
            let elapsed = self.started.elapsed();
            self.first_byte = Some(elapsed);
            if let Some(span) = &self.span {
                span.record("http.first_byte_ms", milliseconds(elapsed));
            }
        }
    }

    fn finish(&mut self, outcome: Outcome) {
        let Some(span) = self.span.take() else { return };
        let duration = self.started.elapsed();
        let error = matches!(outcome, Outcome::Error)
            || self.status.is_some_and(|status| status.is_server_error());
        let cancelled = matches!(outcome, Outcome::Cancelled);
        span.record("http.duration_ms", milliseconds(duration));
        span.record("http.cancelled", cancelled);
        if error || cancelled {
            span.record(FIELD_STATUS, "error");
            span.record("otel.status_code", "ERROR");
        }
        if let Some(config) = &self.metrics {
            let event = self.event(config, duration, error, cancelled);
            // EMF must be a standalone JSON log event, independent of trace sampling.
            // A logging failure must never fail the request or panic during Drop.
            let _ = writeln!(std::io::stdout().lock(), "{event}");
        }
        drop(span);
    }

    fn event(
        &self,
        config: &MetricsConfig,
        duration: Duration,
        error: bool,
        cancelled: bool,
    ) -> Value {
        let mut metrics = vec![
            json!({"Name": "DurationMs", "Unit": "Milliseconds"}),
            json!({"Name": "Requests", "Unit": "Count"}),
            json!({"Name": "Errors", "Unit": "Count"}),
            json!({"Name": "Cancelled", "Unit": "Count"}),
        ];
        // Route and method stay searchable in Logs Insights, but only service
        // and environment create CloudWatch metric series.
        let mut event = json!({
            "Service": config.service,
            "Environment": config.environment,
            "HttpMethod": self.method,
            "Route": self.route,
            "DurationMs": milliseconds(duration),
            "Requests": 1,
            "Errors": u8::from(error),
            "Cancelled": u8::from(cancelled),
        });
        if let Some(elapsed) = self.response_ready {
            metrics.push(json!({"Name": "ResponseReadyMs", "Unit": "Milliseconds"}));
            event["ResponseReadyMs"] = json!(milliseconds(elapsed));
        }
        if let Some(elapsed) = self.first_byte {
            metrics.push(json!({"Name": "FirstByteMs", "Unit": "Milliseconds"}));
            event["FirstByteMs"] = json!(milliseconds(elapsed));
        }
        if let Some(trace_id) = &self.trace_id {
            event["TraceId"] = json!(trace_id);
        }
        event["_aws"] = json!({
            "Timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
            "CloudWatchMetrics": [{
                "Namespace": "FlowLike/API",
                "Dimensions": [["Service", "Environment"]],
                "Metrics": metrics,
            }],
        });
        event
    }
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

impl Drop for RequestLifetime {
    fn drop(&mut self) {
        self.finish(Outcome::Cancelled);
    }
}

pub(crate) struct ObservedBody {
    body: Body,
    lifetime: RequestLifetime,
}

impl ObservedBody {
    pub(crate) fn new(body: Body, lifetime: RequestLifetime) -> Self {
        Self { body, lifetime }
    }
}

impl Drop for ObservedBody {
    fn drop(&mut self) {
        // HTTP consumers may stop after Content-Length bytes or skip a HEAD/
        // empty body entirely. An exhausted inner body is successful completion.
        if self.body.is_end_stream() {
            self.lifetime.finish(Outcome::Complete);
        }
    }
}

impl HttpBody for ObservedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if this.lifetime.span.is_none() {
            return Poll::Ready(None);
        }
        let result = {
            let _entered = this.lifetime.span.as_ref().map(Span::enter);
            Pin::new(&mut this.body).poll_frame(cx)
        };
        match &result {
            Poll::Ready(Some(Ok(frame))) => {
                if frame.data_ref().is_some_and(|data| !data.is_empty()) {
                    this.lifetime.first_byte();
                }
                // Lambda and HTTP consumers may treat trailers as terminal
                // without polling again. Close before handing that frame out.
                if frame.is_trailers() {
                    this.lifetime.finish(Outcome::Complete);
                }
            }
            Poll::Ready(Some(Err(_))) => this.lifetime.finish(Outcome::Error),
            Poll::Ready(None) => this.lifetime.finish(Outcome::Complete),
            Poll::Pending => {}
        }
        result
    }

    fn is_end_stream(&self) -> bool {
        // The consumer must poll EOF even for a body with a known content length.
        self.lifetime.span.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        if self.lifetime.span.is_none() {
            SizeHint::with_exact(0)
        } else {
            self.body.size_hint()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        middleware::trace_context::server_span,
        telemetry::spans::{SpanExportConfig, telemetry_span_layer},
    };
    use futures::{StreamExt, future::poll_fn};
    use tracing_subscriber::layer::SubscriberExt;

    fn lifetime() -> RequestLifetime {
        let mut lifetime = RequestLifetime::new(
            server_span(None, "GET", "/api/v1/apps/{app_id}"),
            "GET".into(),
            "/api/v1/apps/{app_id}".into(),
            Instant::now(),
        );
        lifetime.metrics = None;
        lifetime.response_ready(StatusCode::OK);
        lifetime
    }

    #[tokio::test]
    async fn the_span_stays_open_until_stream_eof() {
        let (layer, mut exporter) = telemetry_span_layer(SpanExportConfig {
            sample_rate: 1.0,
            ..SpanExportConfig::default()
        });
        let subscriber = tracing_subscriber::registry().with(layer);
        let _subscriber = tracing::subscriber::set_default(subscriber);
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        let mut body = ObservedBody::new(Body::from_stream(receiver), lifetime());
        assert!(!body.is_end_stream());
        assert!(exporter.drain().is_empty());
        sender
            .unbounded_send(Ok::<_, std::io::Error>(Bytes::from_static(b"one")))
            .unwrap();
        let frame = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.into_data().unwrap(), "one");
        assert!(body.lifetime.first_byte.is_some());
        assert!(exporter.drain().is_empty());
        drop(sender);
        assert!(
            poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
                .await
                .is_none()
        );
        assert!(body.is_end_stream());
        assert_eq!(exporter.drain().len(), 1);
        drop(body);
        assert!(
            exporter.drain().is_empty(),
            "completion must be emitted once"
        );
    }

    #[tokio::test]
    async fn body_errors_are_preserved_and_close_the_span() {
        let (layer, mut exporter) = telemetry_span_layer(SpanExportConfig {
            sample_rate: 1.0,
            ..SpanExportConfig::default()
        });
        let _subscriber =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        let stream = futures::stream::once(async {
            Err::<Bytes, _>(std::io::Error::other("sensitive upstream error"))
        });
        let body = Body::new(ObservedBody::new(Body::from_stream(stream), lifetime()));
        let error = body.into_data_stream().next().await.unwrap().unwrap_err();
        assert!(error.to_string().contains("sensitive upstream error"));
        let spans = exporter.drain();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].status, "error");
        assert!(!format!("{spans:?}").contains("sensitive upstream error"));
    }

    #[test]
    fn dropping_an_unfinished_body_records_cancellation_once() {
        let (layer, mut exporter) = telemetry_span_layer(SpanExportConfig {
            sample_rate: 1.0,
            ..SpanExportConfig::default()
        });
        let _subscriber =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        let pending = futures::stream::pending::<Result<Bytes, std::io::Error>>();
        drop(ObservedBody::new(Body::from_stream(pending), lifetime()));
        let spans = exporter.drain();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].status, "error");
    }

    #[tokio::test]
    async fn an_empty_body_completes_without_a_fabricated_first_byte() {
        let mut body = ObservedBody::new(Body::empty(), lifetime());
        assert!(!body.is_end_stream());
        assert!(
            poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
                .await
                .is_none()
        );
        assert!(body.lifetime.first_byte.is_none());
        assert!(body.is_end_stream());
    }

    #[test]
    fn dropping_an_already_empty_body_is_successful_completion() {
        let (layer, mut exporter) = telemetry_span_layer(SpanExportConfig {
            sample_rate: 1.0,
            ..SpanExportConfig::default()
        });
        let _subscriber =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        drop(ObservedBody::new(Body::empty(), lifetime()));
        let spans = exporter.drain();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].status, "ok");
    }

    #[tokio::test]
    async fn dropping_after_the_last_known_frame_is_successful_completion() {
        let (layer, mut exporter) = telemetry_span_layer(SpanExportConfig {
            sample_rate: 1.0,
            ..SpanExportConfig::default()
        });
        let _subscriber =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        let mut body = ObservedBody::new(Body::from("done"), lifetime());
        assert!(
            poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
                .await
                .is_some()
        );
        assert!(exporter.drain().is_empty());
        drop(body);
        let spans = exporter.drain();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].status, "ok");
    }

    #[tokio::test]
    async fn trailers_are_preserved_and_close_before_the_consumer_stops_polling() {
        struct TrailerBody;
        impl HttpBody for TrailerBody {
            type Data = Bytes;
            type Error = std::convert::Infallible;

            fn poll_frame(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
                let mut trailers = axum::http::HeaderMap::new();
                trailers.insert("x-check", "finished".parse().unwrap());
                Poll::Ready(Some(Ok(Frame::trailers(trailers))))
            }
        }

        let (layer, mut exporter) = telemetry_span_layer(SpanExportConfig {
            sample_rate: 1.0,
            ..SpanExportConfig::default()
        });
        let _subscriber =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        let mut body = ObservedBody::new(Body::new(TrailerBody), lifetime());
        let frame = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.trailers_ref().unwrap()["x-check"], "finished");
        assert!(body.is_end_stream());
        assert_eq!(exporter.drain().len(), 1);
        drop(body);
        assert!(exporter.drain().is_empty());
    }

    #[test]
    fn metric_dimensions_exclude_routes_methods_and_trace_identifiers() {
        let lifetime = lifetime();
        let event = lifetime.event(
            &MetricsConfig {
                service: "flow-like-api".into(),
                environment: "test".into(),
            },
            Duration::from_millis(23),
            true,
            false,
        );
        assert_eq!(
            event["_aws"]["CloudWatchMetrics"][0]["Dimensions"],
            json!([["Service", "Environment"]])
        );
        assert_eq!(event["Route"], "/api/v1/apps/{app_id}");
        assert_eq!(event["Requests"], 1);
        assert_eq!(event["Errors"], 1);
        assert_eq!(event["Cancelled"], 0);
        assert!(event.get("FirstByteMs").is_none());
        assert_eq!(method_label("user-chosen-method"), "OTHER");
    }
}
