//! Inbound W3C `traceparent` propagation.
//!
//! Clients that sampled a trace send `traceparent: 00-<trace>-<span>-<flags>`
//! with their API calls. This middleware parses that header strictly, exposes
//! it to handlers as a [`TraceContext`] extension, and opens a server span
//! carrying the propagation fields that [`crate::telemetry::spans`] reads — so
//! the backend's spans land in the same trace waterfall as the client's.
//!
//! Malformed headers are ignored, never rejected: a broken tracer must not cost
//! a user their request.
//!
//! The server span records the matched route *template* (`/user/lookup/{sub}`),
//! never the concrete URI. Concrete paths carry the path parameters —
//! `/user/lookup/auth0|1234` is an auth subject, `/user/search/a@b.com` is free
//! text an admin typed — and `http.route` is persisted by
//! [`crate::telemetry::spans`] into a table documented as anonymous.

use axum::{
    body::Body,
    extract::{MatchedPath, Request},
    http::HeaderMap,
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tracing::Instrument;

use crate::telemetry::request_metrics::{ObservedBody, RequestLifetime, method_label};
use crate::telemetry::spans::truncate;

pub const TRACEPARENT_HEADER: &str = "traceparent";

const TRACEPARENT_VERSION: &str = "00";
const TRACE_ID_LEN: usize = 32;
const SPAN_ID_LEN: usize = 16;
const FLAGS_LEN: usize = 2;
const SAMPLED_FLAG: u8 = 0x01;

const MAX_ROUTE_LEN: usize = 256;
const MAX_STATIC_SEGMENT_LEN: usize = 32;
const MIN_OPAQUE_ID_LEN: usize = 12;

/// Trace context of the caller, continued by this process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceContext {
    /// 32 lowercase hex characters.
    pub trace_id: String,
    /// Caller's span id (16 lowercase hex characters); parent of our spans.
    pub parent_span_id: String,
    /// Upstream head-sampling decision, honored as-is.
    pub sampled: bool,
}

impl TraceContext {
    /// Render the context as a `traceparent` header value for outgoing calls.
    pub fn traceparent(&self) -> String {
        let flags = if self.sampled { SAMPLED_FLAG } else { 0 };
        format!(
            "{}-{}-{}-{:02x}",
            TRACEPARENT_VERSION, self.trace_id, self.parent_span_id, flags
        )
    }
}

/// Strict W3C `traceparent` parser: version `00`, a 32 hex trace id, a 16 hex
/// span id and 2 hex flags, all lowercase and neither id all-zero. Anything
/// else yields `None`.
pub fn parse_traceparent(raw: &str) -> Option<TraceContext> {
    let mut parts = raw.trim().split('-');
    let version = parts.next()?;
    let trace_id = parts.next()?;
    let parent_span_id = parts.next()?;
    let flags = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    if version != TRACEPARENT_VERSION
        || !is_lower_hex(trace_id, TRACE_ID_LEN)
        || !is_lower_hex(parent_span_id, SPAN_ID_LEN)
        || !is_lower_hex(flags, FLAGS_LEN)
        || is_all_zero(trace_id)
        || is_all_zero(parent_span_id)
    {
        return None;
    }

    let flags = u8::from_str_radix(flags, 16).ok()?;

    Some(TraceContext {
        trace_id: trace_id.to_string(),
        parent_span_id: parent_span_id.to_string(),
        sampled: flags & SAMPLED_FLAG != 0,
    })
}

pub fn trace_context_from_headers(headers: &HeaderMap) -> Option<TraceContext> {
    parse_traceparent(headers.get(TRACEPARENT_HEADER)?.to_str().ok()?)
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_all_zero(value: &str) -> bool {
    value.bytes().all(|byte| byte == b'0')
}

/// Only registered templates become route labels. Unmatched paths have one
/// fixed label, so arbitrary path text cannot leak or create metric dimensions.
fn route_label(matched: Option<&str>, _raw_path: &str) -> String {
    match matched {
        Some(template) => truncate(template, MAX_ROUTE_LEN),
        None => "unmatched".to_string(),
    }
}

/// Classifies labels accepted by the client performance ingest. Server request
/// labels use registered route templates or `unmatched` instead.
pub(crate) fn is_static_segment(segment: &str) -> bool {
    if segment.is_empty() {
        return true;
    }
    if segment.len() > MAX_STATIC_SEGMENT_LEN {
        return false;
    }
    if !segment
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
    {
        return false;
    }
    if !segment.chars().any(|c| c.is_ascii_lowercase()) {
        return false;
    }
    let opaque_id = segment.len() >= MIN_OPAQUE_ID_LEN
        && segment.chars().all(|c| c.is_ascii_alphanumeric())
        && segment.chars().any(|c| c.is_ascii_digit());
    !opaque_id
}

/// Server span for one request. The field names are the propagation contract
/// with the telemetry span layer; they must stay in sync with the `FIELD_*`
/// constants in [`crate::telemetry::spans`]. `route` is a registered template
/// or the fixed unmatched label, never a concrete URI.
pub(crate) fn server_span(
    context: Option<&TraceContext>,
    method: &str,
    route: &str,
) -> tracing::Span {
    let span = match context {
        Some(context) => {
            let trace_id = context.trace_id.as_str();
            let parent_span_id = context.parent_span_id.as_str();
            tracing::info_span!(
                target: "flow_like::observability",
                "http.request",
                otel.kind = "server",
                telemetry.trace_id = trace_id,
                telemetry.parent_span_id = parent_span_id,
                telemetry.sampled = context.sampled,
                http.method = method,
                http.route = route,
                http.status_code = tracing::field::Empty,
                http.response_ready_ms = tracing::field::Empty,
                http.first_byte_ms = tracing::field::Empty,
                http.duration_ms = tracing::field::Empty,
                http.cancelled = tracing::field::Empty,
                otel.status_code = tracing::field::Empty,
                telemetry.status = tracing::field::Empty
            )
        }
        None => tracing::info_span!(
            target: "flow_like::observability",
            "http.request",
            otel.kind = "server",
            http.method = method,
            http.route = route,
            http.status_code = tracing::field::Empty,
            http.response_ready_ms = tracing::field::Empty,
            http.first_byte_ms = tracing::field::Empty,
            http.duration_ms = tracing::field::Empty,
            http.cancelled = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
            telemetry.status = tracing::field::Empty
        ),
    };
    #[cfg(feature = "otel")]
    {
        use opentelemetry::trace::TraceContextExt;
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        // Lambda installs its invocation parent before routing. Outside Lambda,
        // bridge the W3C header only when there is no valid local parent.
        let local = tracing::Span::current().context();
        let local = if local.span().span_context().is_valid() {
            local
        } else {
            opentelemetry::Context::current()
        };
        if let Some(parent) = parent_context(local, context) {
            let _ = span.set_parent(parent);
        }
    }
    span
}

#[cfg(feature = "otel")]
fn parent_context(
    local: opentelemetry::Context,
    inbound: Option<&TraceContext>,
) -> Option<opentelemetry::Context> {
    use opentelemetry::trace::{
        SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
    };
    if local.span().span_context().is_valid() {
        return Some(local);
    }
    let inbound = inbound?;
    let parent = SpanContext::new(
        TraceId::from_hex(&inbound.trace_id).ok()?,
        SpanId::from_hex(&inbound.parent_span_id).ok()?,
        if inbound.sampled {
            TraceFlags::SAMPLED
        } else {
            TraceFlags::default()
        },
        true,
        TraceState::default(),
    );
    parent
        .is_valid()
        .then(|| opentelemetry::Context::new().with_remote_span_context(parent))
}

/// Continues an inbound client trace and records the request as a server span.
pub async fn trace_context_middleware(mut req: Request, next: Next) -> Response {
    let started = Instant::now();
    let context = trace_context_from_headers(req.headers());
    let method = method_label(req.method().as_str()).to_string();
    let route = route_label(
        req.extensions()
            .get::<MatchedPath>()
            .map(MatchedPath::as_str),
        req.uri().path(),
    );

    if let Some(context) = context.clone() {
        req.extensions_mut().insert(context);
    }

    let span = server_span(context.as_ref(), &method, &route);
    let mut lifetime = RequestLifetime::new(span.clone(), method, route, started);
    let response = next.run(req).instrument(span).await;
    lifetime.response_ready(response.status());
    let (parts, body) = response.into_parts();
    Response::from_parts(parts, Body::new(ObservedBody::new(body, lifetime)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const SPAN_ID: &str = "00f067aa0ba902b7";

    fn header_map(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(TRACEPARENT_HEADER, value.parse().unwrap());
        headers
    }

    #[test]
    fn parses_a_sampled_traceparent() {
        let context = parse_traceparent(&format!("00-{TRACE_ID}-{SPAN_ID}-01")).unwrap();
        assert_eq!(context.trace_id, TRACE_ID);
        assert_eq!(context.parent_span_id, SPAN_ID);
        assert!(context.sampled);
    }

    #[test]
    fn parses_an_unsampled_traceparent() {
        let context = parse_traceparent(&format!("00-{TRACE_ID}-{SPAN_ID}-00")).unwrap();
        assert!(!context.sampled);
    }

    #[test]
    fn reads_the_sampled_bit_out_of_arbitrary_flags() {
        assert!(
            parse_traceparent(&format!("00-{TRACE_ID}-{SPAN_ID}-ff"))
                .unwrap()
                .sampled
        );
        assert!(
            parse_traceparent(&format!("00-{TRACE_ID}-{SPAN_ID}-03"))
                .unwrap()
                .sampled
        );
        assert!(
            !parse_traceparent(&format!("00-{TRACE_ID}-{SPAN_ID}-fe"))
                .unwrap()
                .sampled
        );
    }

    #[test]
    fn rejects_unsupported_versions() {
        assert!(parse_traceparent(&format!("01-{TRACE_ID}-{SPAN_ID}-01")).is_none());
        assert!(parse_traceparent(&format!("ff-{TRACE_ID}-{SPAN_ID}-01")).is_none());
        assert!(parse_traceparent(&format!("0-{TRACE_ID}-{SPAN_ID}-01")).is_none());
    }

    #[test]
    fn rejects_malformed_ids() {
        let short_trace = "4bf92f3577b34da6a3ce929d0e0e473";
        let long_trace = format!("{TRACE_ID}a");
        let upper_trace = TRACE_ID.to_uppercase();
        for raw in [
            format!("00-{short_trace}-{SPAN_ID}-01"),
            format!("00-{long_trace}-{SPAN_ID}-01"),
            format!("00-{upper_trace}-{SPAN_ID}-01"),
            format!("00-{TRACE_ID}-{}-01", &SPAN_ID[..15]),
            format!("00-{TRACE_ID}-{}g-01", &SPAN_ID[..15]),
            format!("00-{TRACE_ID}-{SPAN_ID}-1"),
            format!("00-{TRACE_ID}-{SPAN_ID}-xy"),
        ] {
            assert!(
                parse_traceparent(&raw).is_none(),
                "expected '{raw}' to be rejected"
            );
        }
    }

    #[test]
    fn rejects_all_zero_ids() {
        assert!(parse_traceparent(&format!("00-{}-{SPAN_ID}-01", "0".repeat(32))).is_none());
        assert!(parse_traceparent(&format!("00-{TRACE_ID}-{}-01", "0".repeat(16))).is_none());
    }

    #[test]
    fn rejects_structurally_broken_headers() {
        for raw in [
            "",
            "00",
            &format!("00-{TRACE_ID}"),
            &format!("00-{TRACE_ID}-{SPAN_ID}"),
            &format!("00-{TRACE_ID}-{SPAN_ID}-01-extra"),
            &format!("00 {TRACE_ID} {SPAN_ID} 01"),
        ] {
            assert!(
                parse_traceparent(raw).is_none(),
                "expected '{raw}' to be rejected"
            );
        }
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        assert!(parse_traceparent(&format!("  00-{TRACE_ID}-{SPAN_ID}-01 ")).is_some());
    }

    #[test]
    fn reads_the_header_only_when_it_is_valid() {
        assert_eq!(
            trace_context_from_headers(&header_map(&format!("00-{TRACE_ID}-{SPAN_ID}-01")))
                .map(|context| context.trace_id),
            Some(TRACE_ID.to_string())
        );
        assert!(trace_context_from_headers(&header_map("garbage")).is_none());
        assert!(trace_context_from_headers(&HeaderMap::new()).is_none());
    }

    #[test]
    fn the_matched_route_template_is_recorded_verbatim() {
        assert_eq!(
            route_label(Some("/user/lookup/{sub}"), "/user/lookup/auth0|1234"),
            "/user/lookup/{sub}"
        );
        assert_eq!(
            route_label(
                Some("/user/search/{query}"),
                "/user/search/someone@example.com"
            ),
            "/user/search/{query}"
        );
        assert_eq!(
            route_label(Some(&"/a".repeat(400)), "/anything").len(),
            MAX_ROUTE_LEN
        );
    }

    #[test]
    fn the_fallback_never_uses_caller_controlled_path_text() {
        for raw in [
            "/user/lookup/auth0|1234",
            "/user/search/someone@example.com",
            "/user/lookup/auth0%7C1234",
            "/apps/0f1e2d3c4b5a69788796/board",
            "/apps/f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "/apps/12345/flows",
            "/user/lookup/QWxhZGRpbjpvcGVuIHNlc2FtZQ",
            "/alice/smith",
        ] {
            assert_eq!(route_label(None, raw), "unmatched", "raw path '{raw}'");
        }
    }

    #[test]
    fn unknown_paths_share_a_single_metric_dimension() {
        for path in [
            "/",
            "/api/v1/apps",
            "/telemetry/performance",
            "/admin/telemetry/traces",
            "/auth/openid",
            "/notifications/subscriptions",
        ] {
            assert_eq!(route_label(None, path), "unmatched");
        }
        assert_eq!(route_label(None, ""), "unmatched");
    }

    /// Runs `uri` through `router` with a probe layered exactly where
    /// [`trace_context_middleware`] sits, and reports the label it computed.
    async fn label_seen_by_middleware(router: axum::Router, uri: &str) -> Option<String> {
        use axum::body::Body;
        use axum::http::Request as HttpRequest;
        use axum::middleware::from_fn;
        use std::sync::{Arc, Mutex};
        use tower::ServiceExt;

        let observed: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let sink = observed.clone();

        let app = router.layer(from_fn(move |req: Request, next: Next| {
            let sink = sink.clone();
            async move {
                let label = route_label(
                    req.extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str),
                    req.uri().path(),
                );
                *sink.lock().unwrap() = Some(label);
                next.run(req).await
            }
        }));

        let request = HttpRequest::builder().uri(uri).body(Body::empty()).unwrap();
        app.oneshot(request).await.unwrap();

        observed.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn a_matched_route_reaches_the_middleware_as_its_template() {
        let router =
            axum::Router::new().route("/user/lookup/{sub}", axum::routing::get(|| async { "ok" }));
        assert_eq!(
            label_seen_by_middleware(router, "/user/lookup/auth0%7C1234").await,
            Some("/user/lookup/{sub}".to_string()),
            "the middleware must record the template, never the concrete path"
        );
    }

    #[tokio::test]
    async fn a_nested_route_reaches_the_middleware_as_its_full_template() {
        let router = axum::Router::new().nest(
            "/user",
            axum::Router::new().route("/lookup/{sub}", axum::routing::get(|| async { "ok" })),
        );
        assert_eq!(
            label_seen_by_middleware(router, "/user/lookup/auth0%7C1234").await,
            Some("/user/lookup/{sub}".to_string()),
            "nested routes are flattened, so the template carries the nest prefix"
        );
    }

    #[tokio::test]
    async fn an_unmatched_path_has_a_fixed_label() {
        let router =
            axum::Router::new().route("/user/lookup/{sub}", axum::routing::get(|| async { "ok" }));
        assert_eq!(
            label_seen_by_middleware(router, "/user/search/someone@example.com").await,
            Some("unmatched".to_string()),
            "a request that matches no route must never contribute path text"
        );
    }

    #[tokio::test]
    async fn the_outer_boundary_observes_each_surface_once_without_path_parameters() {
        use crate::telemetry::spans::{SpanExportConfig, telemetry_span_layer};
        use axum::{Router, body::to_bytes, middleware::from_fn, routing::get};
        use tower::ServiceExt;
        use tracing_subscriber::layer::SubscriberExt;

        let (layer, mut exporter) = telemetry_span_layer(SpanExportConfig {
            sample_rate: 1.0,
            ..SpanExportConfig::default()
        });
        let _subscriber =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        let surface = || Router::new().route("/items/{id}", get(|| async { "ok" }));
        let router = Router::new()
            .nest("/api/v1", surface())
            .nest("/r", surface())
            .nest("/m", surface())
            .route("/api-doc/openapi.json", get(|| async { "ok" }))
            .layer(from_fn(trace_context_middleware));

        for (path, expected) in [
            ("/api/v1/items/alice@example.com", "/api/v1/items/{id}"),
            ("/r/items/alice@example.com", "/r/items/{id}"),
            ("/m/items/alice@example.com", "/m/items/{id}"),
            ("/api-doc/openapi.json", "/api-doc/openapi.json"),
            ("/alice/smith", "unmatched"),
        ] {
            let response = router
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert!(
                exporter.drain().is_empty(),
                "response headers must not close the span"
            );
            to_bytes(response.into_body(), 1024).await.unwrap();
            let spans = exporter.drain();
            assert_eq!(spans.len(), 1, "exactly one server span for {path}");
            assert_eq!(spans[0].name, "http.request");
            assert_eq!(
                spans[0].attributes.as_ref().unwrap()["http.route"],
                expected
            );
            assert!(!format!("{spans:?}").contains("alice"));
        }
    }

    #[cfg(feature = "otel")]
    #[test]
    fn otel_inherits_the_local_invocation_instead_of_an_untrusted_header() {
        use opentelemetry::{Context, trace::TraceContextExt};
        let invocation = TraceContext {
            trace_id: "11111111111111111111111111111111".into(),
            parent_span_id: "2222222222222222".into(),
            sampled: false,
        };
        let inbound = parse_traceparent(&format!("00-{TRACE_ID}-{SPAN_ID}-01")).unwrap();
        let local = parent_context(Context::new(), Some(&invocation)).unwrap();
        let parent = parent_context(local, Some(&inbound)).unwrap();
        assert_eq!(
            parent.span().span_context().trace_id().to_string(),
            invocation.trace_id
        );
        assert!(!parent.span().span_context().is_sampled());
    }

    #[cfg(feature = "otel")]
    #[test]
    fn otel_bridges_w3c_context_and_its_sampling_decision_without_a_local_parent() {
        use opentelemetry::{Context, trace::TraceContextExt};
        for sampled in [false, true] {
            let inbound = TraceContext {
                trace_id: TRACE_ID.into(),
                parent_span_id: SPAN_ID.into(),
                sampled,
            };
            let parent = parent_context(Context::new(), Some(&inbound)).unwrap();
            let parent_span = parent.span();
            let context = parent_span.span_context();
            assert_eq!(context.trace_id().to_string(), TRACE_ID);
            assert_eq!(context.span_id().to_string(), SPAN_ID);
            assert_eq!(context.is_sampled(), sampled);
            assert!(context.is_remote());
        }
        assert!(parent_context(Context::new(), None).is_none());
    }

    #[test]
    fn round_trips_through_the_header_representation() {
        let context = TraceContext {
            trace_id: TRACE_ID.to_string(),
            parent_span_id: SPAN_ID.to_string(),
            sampled: true,
        };
        assert_eq!(context.traceparent(), format!("00-{TRACE_ID}-{SPAN_ID}-01"));
        assert_eq!(
            parse_traceparent(&context.traceparent()).as_ref(),
            Some(&context)
        );

        let unsampled = TraceContext {
            sampled: false,
            ..context
        };
        assert_eq!(
            unsampled.traceparent(),
            format!("00-{TRACE_ID}-{SPAN_ID}-00")
        );
        assert_eq!(
            parse_traceparent(&unsampled.traceparent()).as_ref(),
            Some(&unsampled)
        );
    }
}
