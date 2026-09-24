//! Per-route request deadlines.
//!
//! Every request gets a total budget chosen from its method and matched route
//! template, measured from the moment it enters this middleware. The budget
//! covers the handler future and the whole response body, so an SSE or relay
//! stream ends when its class runs out, not when the platform timeout kills the
//! container. Keep the default classes short; extend only routes with evidence.

use axum::{
    body::Body,
    extract::{MatchedPath, Request},
    http::Method,
    middleware::Next,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use flow_like_types::tokio::time::{Instant, Sleep, sleep_until, timeout_at};
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use pin_project_lite::pin_project;
use std::{
    future::Future,
    pin::Pin,
    sync::LazyLock,
    task::{Context, Poll},
    time::Duration,
};

use crate::{error::ApiError, telemetry::request_metrics::ResponseFault};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeadlineClass {
    Read,
    Write,
    Data,
    Job,
    Dispatch,
    Ai,
    HostedWorker,
    BitMirror,
}

const LAMBDA_DISPATCH_SECS: u64 = 870;
const DISPATCH_GRACE_SECS: u64 = 60;

/// Sync runs stream until the executor finishes, so self-hosted deployments follow
/// the executor's own limits; on Lambda the 900 s function timeout caps them.
static DISPATCH_SECS: LazyLock<u64> = LazyLock::new(|| {
    dispatch_secs(
        env_secs(
            &["EXECUTION_TIMEOUT_SECONDS", "EXECUTOR_TIMEOUT_SECS"],
            3600,
        ),
        env_secs(&["EXECUTION_QUEUE_MAX_WAIT_SECONDS"], 300),
        std::env::var_os("AWS_LAMBDA_FUNCTION_NAME").is_some(),
    )
});

fn env_secs(names: &[&str], default: u64) -> u64 {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| (1..=86_400).contains(value))
        .unwrap_or(default)
}

fn dispatch_secs(execution_timeout: u64, queue_wait: u64, on_lambda: bool) -> u64 {
    let executor_bound = execution_timeout + queue_wait + DISPATCH_GRACE_SECS;
    if on_lambda {
        executor_bound.min(LAMBDA_DISPATCH_SECS)
    } else {
        executor_bound
    }
}

impl DeadlineClass {
    pub(crate) fn budget(self) -> Duration {
        Duration::from_secs(match self {
            Self::Read => 10,
            Self::Write => 30,
            Self::Data => 120,
            Self::Job => 300,
            Self::Dispatch => *DISPATCH_SECS,
            // HostedRateSnapshot.max_request_ms caps at 840 s; readers wait 30 s more.
            Self::Ai | Self::HostedWorker | Self::BitMirror => 870,
        })
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Data => "data",
            Self::Job => "job",
            Self::Dispatch => "dispatch",
            Self::Ai => "ai",
            Self::HostedWorker => "hosted_worker",
            Self::BitMirror => "bit_mirror",
        }
    }
}

#[derive(Clone, Copy)]
struct Methods(u8);

impl Methods {
    const GET: Self = Self(1);
    const HEAD: Self = Self(1 << 1);
    const OPTIONS: Self = Self(1 << 2);
    const POST: Self = Self(1 << 3);
    const PUT: Self = Self(1 << 4);
    const PATCH: Self = Self(1 << 5);
    const DELETE: Self = Self(1 << 6);
    const ANY: Self = Self(u8::MAX);

    const fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn contains(self, method: &Method) -> bool {
        let bit = match method.as_str() {
            "GET" => Self::GET,
            "HEAD" => Self::HEAD,
            "OPTIONS" => Self::OPTIONS,
            "POST" => Self::POST,
            "PUT" => Self::PUT,
            "PATCH" => Self::PATCH,
            "DELETE" => Self::DELETE,
            _ => return self.0 == Self::ANY.0,
        };
        self.0 & bit.0 != 0
    }
}

const READS: Methods = Methods::GET.or(Methods::HEAD).or(Methods::OPTIONS);
const GET_HEAD: Methods = Methods::GET.or(Methods::HEAD);

struct Rule {
    methods: Methods,
    /// Route template. `{..}` matches any path parameter regardless of its
    /// name; a trailing `*` matches the prefix itself and anything below it.
    path: &'static str,
    class: DeadlineClass,
}

const fn rule(methods: Methods, path: &'static str, class: DeadlineClass) -> Rule {
    Rule {
        methods,
        path,
        class,
    }
}

use DeadlineClass::{Ai, BitMirror, Data, Dispatch, HostedWorker, Job, Read, Write};
use Methods as M;

/// First match wins. Observed maxima are from the API EMF request logs,
/// 2026-09-14 09:00Z to 2026-09-15 16:00Z.
const RULES: &[Rule] = &[
    // Observed max 826.7 s; runs one provider request to its reserved deadline, then settles.
    rule(M::POST, "/api/v1/maintenance/hosted-ai/{id}", HostedWorker),
    // Mirrors multi-GB model files behind an SSE progress stream; known up to 900 s.
    rule(M::PUT, "/api/v1/admin/bit/{bit_id}", BitMirror),
    // Observed max 140.9 s.
    rule(M::POST, "/api/v1/chat/completions", Ai),
    rule(M::POST, "/api/v1/responses", Ai),
    // Observed max 630.7 s.
    rule(M::POST, "/api/v1/ai/copilot/chat", Ai),
    // Observed max 231.2 s.
    rule(M::POST, "/api/v1/ai/global-chat", Ai),
    // Observed max 33.3 s.
    rule(M::POST, "/api/v1/embeddings/embed", Ai),
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/ai-act/assessment/suggest",
        Ai,
    ),
    // Same governance agent as the app-level suggest route.
    rule(M::POST, "/api/v1/admin/ai-act/assist/{app_id}", Ai),
    // Observed max 232.9 s.
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/events/{event_id}/invoke",
        Dispatch,
    ),
    // Observed max 64.0 s.
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/board/{board_id}/invoke",
        Dispatch,
    ),
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/graph/{overlay_id}/actions/{action_id}/invoke",
        Dispatch,
    ),
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/events/{event_id}/invoke/async",
        Dispatch,
    ),
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/board/{board_id}/invoke/async",
        Dispatch,
    ),
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/events/{event_id}/mcp-operation",
        Dispatch,
    ),
    rule(
        M::ANY,
        "/api/v1/apps/{app_id}/events/{event_id}/rest/*",
        Dispatch,
    ),
    // GET stays short so a server-to-client MCP SSE cannot pin a container again.
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/events/{event_id}/mcp/*",
        Dispatch,
    ),
    // Upsert, setup, restore and promote run executor setup synchronously (default 90 s).
    rule(M::PUT, "/api/v1/apps/{app_id}/events/{event_id}", Dispatch),
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/events/{event_id}/setup",
        Dispatch,
    ),
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/events/{event_id}/restore",
        Dispatch,
    ),
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/events/{event_id}/canary/promote",
        Dispatch,
    ),
    rule(M::ANY, "/r/*", Dispatch),
    // Observed max 7.3 s; tool calls collect results for up to 120 s.
    rule(M::POST, "/m/*", Dispatch),
    // Observed POST /sink/trigger/async max 3.0 s; HTTP sinks wait up to 120 s.
    rule(M::ANY, "/api/v1/sink/trigger/*", Dispatch),
    // Observed max 33.4 s; the deletion pass budget reaches 270 s.
    rule(M::POST, "/api/v1/maintenance/run", Job),
    rule(M::POST, "/api/v1/maintenance/payments", Job),
    rule(
        M::POST,
        "/api/v1/maintenance/compute-attempts/reconcile",
        Job,
    ),
    rule(M::POST, "/api/v1/admin/deletions/run", Job),
    rule(M::POST, "/api/v1/admin/governance/scores/recompute", Job),
    rule(M::POST, "/api/v1/admin/packages/ensure-wasm-artifacts", Job),
    rule(M::POST, "/api/v1/admin/telemetry/rollup", Job),
    rule(M::POST, "/api/v1/admin/telemetry/sweep", Job),
    rule(M::POST, "/api/v1/admin/telemetry/alerts/evaluate", Job),
    rule(M::POST, "/api/v1/admin/runs/sweep", Job),
    rule(M::POST, "/api/v1/admin/cache/sweep", Job),
    rule(M::POST, "/api/v1/admin/usage/reconcile", Job),
    rule(M::ANY, "/api/v1/admin/forks/orphans/*", Job),
    // Observed optimize max 35.6 s, table read 7.9 s, columns 4.8 s.
    rule(M::ANY, "/api/v1/apps/{app_id}/db/*", Data),
    rule(M::ANY, "/api/v1/apps/{app_id}/graph/*", Data),
    // Observed dashboard max 6.5 s.
    rule(GET_HEAD, "/api/v1/apps/{app_id}/analytics/*", Data),
    rule(GET_HEAD, "/api/v1/apps/{app_id}/sales/*", Data),
    // Observed max 5.2 s.
    rule(
        GET_HEAD,
        "/api/v1/apps/{app_id}/board/{board_id}/logs",
        Data,
    ),
    // Writes up to 10,000 uploaded log rows of a local run as a new Lance table.
    rule(
        M::POST,
        "/api/v1/apps/{app_id}/board/{board_id}/runs/{run_id}/logs",
        Data,
    ),
    // Observed max 4.5 s on a small app; inline forks copy up to 64 MiB.
    rule(M::POST, "/api/v1/apps/{app_id}/fork", Data),
    rule(M::POST, "/api/v1/apps/fork/online/begin", Data),
    rule(M::POST, "/api/v1/apps/{app_id}/fork/online/finalize", Data),
    rule(M::POST, "/api/v1/apps/{app_id}/fork/offline/begin", Data),
    rule(M::POST, "/api/v1/registry/publish", Data),
    rule(
        M::POST,
        "/api/v1/courses/{course_id}/assets/{asset_id}/optimize",
        Data,
    ),
    rule(M::POST, "/api/v1/admin/telemetry/sourcemaps", Data),
    rule(M::POST, "/api/v1/admin/models/sync", Data),
    rule(M::ANY, "/api/v1/admin/telemetry/*", Data),
    rule(GET_HEAD, "/api/v1/admin/usage/*", Data),
    rule(GET_HEAD, "/api/v1/admin/logs/*", Data),
    rule(M::ANY, "/api/v1/admin/ai-act/*", Data),
    // Long-polls for up to 30 s.
    rule(M::GET, "/api/v1/execution/poll", Data),
    // Upstream cancel call has a 60 s timeout.
    rule(M::DELETE, "/api/v1/execution/run/{run_id}", Data),
    // A first or full verification reads a whole chain or the whole epoch timeline.
    rule(GET_HEAD, "/api/v1/audit/verify", Data),
    rule(GET_HEAD, "/api/v1/audit/verify/epochs", Data),
    // Provider calls have their own 30 s timeout, which a write deadline would pre-empt.
    rule(M::POST, "/api/v1/oauth/*", Data),
    // A replay commits one Lance version; a cut handler leaves an unknown outcome to reconcile.
    rule(M::POST, "/api/v1/apps/{app_id}/invoke/offline/replay", Data),
    rule(M::POST, "/api/v1/instances/project/offline/replay", Data),
    // The upstream fetch has its own 10 s timeout, which a read deadline would pre-empt.
    rule(M::GET, "/api/v1/og", Write),
    // All other observed routes finish within 3 s.
    rule(READS, "/*", Read),
    // Other observed mutations peak at 6.0 s (POST /bit).
    rule(M::ANY, "/*", Write),
];

fn is_parameter(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}')
}

fn template_matches(pattern: &str, template: &str) -> bool {
    let mut actual = template.split('/').filter(|segment| !segment.is_empty());
    for expected in pattern.split('/').filter(|segment| !segment.is_empty()) {
        if expected == "*" {
            return true;
        }
        match actual.next() {
            Some(segment)
                if segment == expected || (is_parameter(expected) && is_parameter(segment)) => {}
            _ => return false,
        }
    }
    actual.next().is_none()
}

fn matching_rule(method: &Method, route: Option<&str>) -> Option<usize> {
    // Unmatched paths and fallbacks have no template and only reach the method defaults.
    let template = route.unwrap_or("/");
    RULES
        .iter()
        .position(|rule| rule.methods.contains(method) && template_matches(rule.path, template))
}

pub(crate) fn classify(method: &Method, route: Option<&str>) -> DeadlineClass {
    matching_rule(method, route).map_or(Write, |index| RULES[index].class)
}

#[derive(Clone, Copy)]
enum Phase {
    Handler,
    Body,
}

struct Expiry {
    method: Method,
    route: Option<MatchedPath>,
    class: DeadlineClass,
    fault: Option<ResponseFault>,
}

impl Expiry {
    fn exceeded(self, phase: Phase) {
        if let Some(fault) = &self.fault {
            fault.mark();
        }
        tracing::warn!(
            method = %self.method,
            route = self.route.as_ref().map_or("unmatched", MatchedPath::as_str),
            deadline_class = self.class.name(),
            deadline_secs = self.class.budget().as_secs(),
            phase = match phase {
                Phase::Handler => "handler",
                Phase::Body => "body",
            },
            "request deadline exceeded"
        );
    }
}

pub async fn deadline_middleware(req: Request, next: Next) -> Response {
    let started = Instant::now();
    let route = req.extensions().get::<MatchedPath>().cloned();
    let class = classify(req.method(), route.as_ref().map(MatchedPath::as_str));
    let expiry = Expiry {
        method: req.method().clone(),
        route,
        class,
        fault: req.extensions().get::<ResponseFault>().cloned(),
    };
    enforce(next.run(req), started + class.budget(), expiry).await
}

async fn enforce(
    handler: impl Future<Output = Response>,
    deadline: Instant,
    expiry: Expiry,
) -> Response {
    match timeout_at(deadline, handler).await {
        Ok(response) => {
            let (parts, body) = response.into_parts();
            Response::from_parts(parts, Body::new(DeadlineBody::new(body, deadline, expiry)))
        }
        Err(_) => {
            let budget = expiry.class.budget().as_secs();
            expiry.exceeded(Phase::Handler);
            ApiError::request_deadline_exceeded(format!(
                "The request exceeded its {budget} s deadline"
            ))
            .into_response()
        }
    }
}

pin_project! {
    /// Ends the response body at the request's absolute deadline. Headers are
    /// already sent by then, so the stream closes cleanly and the request is
    /// flagged as failed for the request metrics.
    struct DeadlineBody {
        body: Body,
        #[pin]
        sleep: Sleep,
        expiry: Option<Expiry>,
    }
}

impl DeadlineBody {
    fn new(body: Body, deadline: Instant, expiry: Expiry) -> Self {
        Self {
            body,
            sleep: sleep_until(deadline),
            expiry: Some(expiry),
        }
    }
}

impl HttpBody for DeadlineBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let mut this = self.project();
        if this.expiry.is_none() {
            return Poll::Ready(None);
        }
        if Instant::now() < this.sleep.deadline() {
            match Pin::new(&mut *this.body).poll_frame(cx) {
                Poll::Ready(None) => {
                    *this.expiry = None;
                    return Poll::Ready(None);
                }
                Poll::Ready(frame) => return Poll::Ready(frame),
                Poll::Pending if this.sleep.as_mut().poll(cx).is_pending() => {
                    return Poll::Pending;
                }
                Poll::Pending => {}
            }
        }
        if let Some(expiry) = this.expiry.take() {
            expiry.exceeded(Phase::Body);
        }
        Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        self.expiry.is_none() || self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        if self.expiry.is_none() {
            SizeHint::with_exact(0)
        } else {
            self.body.size_hint()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::to_bytes,
        http::{StatusCode, header},
        middleware::from_fn,
        routing::{get, post},
    };
    use futures::StreamExt;
    use std::collections::BTreeSet;
    use tower::ServiceExt;

    /// (method, full route template, expected class, observed max seconds).
    const CASES: &[(&str, &str, DeadlineClass, Option<f64>)] = &[
        (
            "POST",
            "/api/v1/maintenance/hosted-ai/{id}",
            HostedWorker,
            Some(826.707),
        ),
        ("PUT", "/api/v1/admin/bit/{bit_id}", BitMirror, None),
        ("DELETE", "/api/v1/admin/bit/{bit_id}", Write, None),
        ("POST", "/api/v1/chat/completions", Ai, Some(140.885)),
        ("GET", "/api/v1/chat/usage", Read, None),
        ("POST", "/api/v1/responses", Ai, None),
        ("POST", "/api/v1/ai/copilot/chat", Ai, Some(630.692)),
        ("POST", "/api/v1/ai/global-chat", Ai, Some(231.185)),
        ("PUT", "/api/v1/ai/global-chat/feedback", Write, None),
        ("POST", "/api/v1/embeddings/embed", Ai, Some(33.290)),
        (
            "POST",
            "/api/v1/apps/{app_id}/ai-act/assessment/suggest",
            Ai,
            None,
        ),
        (
            "GET",
            "/api/v1/apps/{app_id}/ai-act/questionnaire",
            Read,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/events/{event_id}/invoke",
            Dispatch,
            Some(232.871),
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/board/{board_id}/invoke",
            Dispatch,
            Some(63.999),
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/graph/{overlay_id}/actions/{action_id}/invoke",
            Dispatch,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/events/{event_id}/invoke/async",
            Dispatch,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/board/{board_id}/invoke/async",
            Dispatch,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/events/{event_id}/mcp-operation",
            Dispatch,
            None,
        ),
        (
            "GET",
            "/api/v1/apps/{app_id}/events/{event_id}/rest",
            Dispatch,
            None,
        ),
        (
            "DELETE",
            "/api/v1/apps/{app_id}/events/{event_id}/rest/{*path}",
            Dispatch,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/events/{event_id}/mcp",
            Dispatch,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/events/{event_id}/mcp/{*path}",
            Dispatch,
            None,
        ),
        (
            "GET",
            "/api/v1/apps/{app_id}/events/{event_id}/mcp",
            Read,
            None,
        ),
        (
            "DELETE",
            "/api/v1/apps/{app_id}/events/{event_id}/mcp/{*path}",
            Write,
            None,
        ),
        (
            "PUT",
            "/api/v1/apps/{app_id}/events/{event_id}",
            Dispatch,
            None,
        ),
        ("GET", "/api/v1/apps/{app_id}/events/{event_id}", Read, None),
        (
            "DELETE",
            "/api/v1/apps/{app_id}/events/{event_id}",
            Write,
            None,
        ),
        ("GET", "/api/v1/apps/{app_id}/events", Read, Some(3.0)),
        (
            "POST",
            "/api/v1/apps/{app_id}/events/{event_id}/setup",
            Dispatch,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/events/{event_id}/restore",
            Dispatch,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/events/{event_id}/canary/promote",
            Dispatch,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/events/{event_id}/canary/abort",
            Write,
            None,
        ),
        ("GET", "/r/{slug_or_id}", Dispatch, None),
        ("POST", "/r/{slug_or_id}/{*path}", Dispatch, None),
        ("POST", "/m/{slug_or_id}", Dispatch, Some(7.312)),
        ("POST", "/m/{slug_or_id}/{*path}", Dispatch, None),
        ("GET", "/m/{slug_or_id}", Read, None),
        ("DELETE", "/m/{slug_or_id}/{*path}", Write, None),
        ("POST", "/api/v1/sink/trigger/async", Dispatch, Some(3.038)),
        (
            "GET",
            "/api/v1/sink/trigger/http/{app_id}/{*path}",
            Dispatch,
            None,
        ),
        (
            "POST",
            "/api/v1/sink/trigger/telegram/{event_id}",
            Dispatch,
            None,
        ),
        ("GET", "/api/v1/sink/schedules", Read, None),
        ("GET", "/api/v1/execution/poll", Data, None),
        ("DELETE", "/api/v1/execution/run/{run_id}", Data, None),
        ("GET", "/api/v1/execution/run/{run_id}", Read, None),
        (
            "POST",
            "/api/v1/apps/{app_id}/invoke/offline/replay",
            Data,
            None,
        ),
        (
            "GET",
            "/api/v1/apps/{app_id}/invoke/offline/capabilities",
            Read,
            None,
        ),
        (
            "POST",
            "/api/v1/instances/project/offline/replay",
            Data,
            None,
        ),
        ("POST", "/api/v1/maintenance/run", Job, Some(33.359)),
        ("POST", "/api/v1/maintenance/payments", Job, None),
        (
            "POST",
            "/api/v1/maintenance/compute-attempts/reconcile",
            Job,
            None,
        ),
        (
            "GET",
            "/api/v1/maintenance/compute-attempts/pending",
            Read,
            None,
        ),
        ("POST", "/api/v1/admin/deletions/run", Job, None),
        (
            "POST",
            "/api/v1/admin/governance/scores/recompute",
            Job,
            None,
        ),
        (
            "POST",
            "/api/v1/admin/packages/ensure-wasm-artifacts",
            Job,
            None,
        ),
        ("POST", "/api/v1/admin/telemetry/rollup", Job, None),
        ("POST", "/api/v1/admin/telemetry/sweep", Job, None),
        ("POST", "/api/v1/admin/telemetry/alerts/evaluate", Job, None),
        ("POST", "/api/v1/admin/runs/sweep", Job, None),
        ("POST", "/api/v1/admin/cache/sweep", Job, None),
        ("POST", "/api/v1/admin/usage/reconcile", Job, None),
        ("GET", "/api/v1/admin/forks/orphans", Job, None),
        (
            "POST",
            "/api/v1/admin/forks/orphans/{app_id}/delete",
            Job,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/db/{table}/optimize",
            Data,
            Some(35.646),
        ),
        ("GET", "/api/v1/apps/{app_id}/db/{table}", Data, Some(7.911)),
        (
            "POST",
            "/api/v1/apps/{app_id}/db/{table}/columns",
            Data,
            Some(4.836),
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/db/queries/execute",
            Data,
            Some(3.348),
        ),
        ("GET", "/api/v1/apps/{app_id}/db", Data, None),
        (
            "POST",
            "/api/v1/apps/{app_id}/graph/{overlay_id}/cypher",
            Data,
            None,
        ),
        ("GET", "/api/v1/apps/{app_id}/graph", Data, None),
        (
            "GET",
            "/api/v1/apps/{app_id}/analytics/dashboard",
            Data,
            Some(6.456),
        ),
        ("GET", "/api/v1/apps/{app_id}/analytics", Data, None),
        ("GET", "/api/v1/apps/{app_id}/sales/dashboard", Data, None),
        ("PATCH", "/api/v1/apps/{app_id}/sales/price", Write, None),
        (
            "GET",
            "/api/v1/apps/{app_id}/board/{board_id}/logs",
            Data,
            Some(5.224),
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/board/{board_id}/runs/{run_id}/logs",
            Data,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/board/{board_id}",
            Write,
            Some(17.317),
        ),
        ("PUT", "/api/v1/apps/{app_id}/board/{board_id}", Write, None),
        (
            "PATCH",
            "/api/v1/apps/{app_id}/board/{board_id}",
            Write,
            None,
        ),
        ("GET", "/api/v1/apps/{app_id}/board/{board_id}", Read, None),
        (
            "DELETE",
            "/api/v1/apps/{app_id}/board/{board_id}",
            Write,
            None,
        ),
        ("POST", "/api/v1/apps/{app_id}/fork", Data, Some(4.451)),
        ("POST", "/api/v1/apps/fork/online/begin", Data, None),
        ("GET", "/api/v1/apps/fork/jobs/{job_id}", Read, None),
        (
            "POST",
            "/api/v1/apps/{app_id}/fork/online/finalize",
            Data,
            None,
        ),
        (
            "POST",
            "/api/v1/apps/{app_id}/fork/offline/begin",
            Data,
            None,
        ),
        ("GET", "/api/v1/apps/{app_id}/fork/preview", Read, None),
        ("POST", "/api/v1/registry/publish", Data, None),
        (
            "POST",
            "/api/v1/courses/{course_id}/assets/{asset_id}/optimize",
            Data,
            None,
        ),
        ("POST", "/api/v1/admin/telemetry/sourcemaps", Data, None),
        ("POST", "/api/v1/admin/models/sync", Data, None),
        ("POST", "/api/v1/admin/telemetry/query", Data, None),
        (
            "GET",
            "/api/v1/admin/telemetry/traces/{trace_id}",
            Data,
            None,
        ),
        ("GET", "/api/v1/admin/usage/overview", Data, None),
        (
            "PUT",
            "/api/v1/admin/usage/apps/{app_id}/limits",
            Write,
            None,
        ),
        ("GET", "/api/v1/admin/logs/timeseries", Data, None),
        ("GET", "/api/v1/admin/ai-act/inventory/export", Data, None),
        ("POST", "/api/v1/admin/ai-act/assist/{app_id}", Ai, None),
        (
            "POST",
            "/api/v1/admin/ai-act/inventory/{app_id}/reconcile-models",
            Data,
            None,
        ),
        ("GET", "/api/v1/audit/verify", Data, None),
        ("GET", "/api/v1/audit/verify/epochs", Data, None),
        ("GET", "/api/v1/audit/records", Read, None),
        ("POST", "/api/v1/oauth/token/{provider_id}", Data, None),
        (
            "POST",
            "/api/v1/oauth/device/poll/{provider_id}",
            Data,
            None,
        ),
        ("GET", "/api/v1/og", Write, None),
        ("GET", "/api/v1/og/", Write, None),
        ("GET", "/api-doc/openapi.json", Read, None),
        ("GET", "/swagger-ui/{*rest}", Read, None),
        ("HEAD", "/api/v1/health", Read, None),
        ("OPTIONS", "/api/v1/user/info", Read, None),
        ("GET", "/api/v1/apps/{app_id}/detail", Read, None),
        ("POST", "/api/v1/bit", Write, Some(5.968)),
        ("DELETE", "/api/v1/apps/{app_id}", Write, Some(5.614)),
        (
            "DELETE",
            "/api/v1/channels/{channel_id}",
            Write,
            Some(3.801),
        ),
        ("PUT", "/api/v1/user/info", Write, None),
        ("TRACE", "/api/v1/apps", Write, None),
    ];

    fn method(name: &str) -> Method {
        Method::from_bytes(name.as_bytes()).unwrap()
    }

    #[test]
    fn every_rule_is_reachable_and_classifies_its_routes() {
        let mut matched = BTreeSet::new();
        for &(name, template, expected, observed) in CASES {
            let parsed = method(name);
            let index = matching_rule(&parsed, Some(template)).expect("defaults match everything");
            matched.insert(index);
            let class = classify(&parsed, Some(template));
            assert_eq!(class, expected, "{name} {template}");
            if let Some(observed) = observed {
                assert!(
                    class.budget().as_secs_f64() > observed,
                    "{name} {template}: {} deadline {:?} is below the observed {observed} s",
                    class.name(),
                    class.budget()
                );
            }
        }
        let unreached: Vec<_> = (0..RULES.len())
            .filter(|index| !matched.contains(index))
            .map(|index| RULES[index].path)
            .collect();
        assert!(
            unreached.is_empty(),
            "rules without a case or shadowed: {unreached:?}"
        );
    }

    #[test]
    fn deadline_classifies_desktop_and_instance_replay_as_data() {
        for template in [
            "/api/v1/apps/{app_id}/invoke/offline/replay",
            "/api/v1/instances/project/offline/replay",
        ] {
            assert_eq!(classify(&Method::POST, Some(template)), Data, "{template}");
        }
        assert_eq!(
            classify(
                &Method::GET,
                Some("/api/v1/apps/{app_id}/invoke/offline/capabilities")
            ),
            Read
        );
    }

    #[test]
    fn unmatched_paths_use_the_method_default() {
        assert_eq!(classify(&Method::GET, None), Read);
        assert_eq!(classify(&Method::HEAD, None), Read);
        assert_eq!(classify(&Method::POST, None), Write);
        assert_eq!(classify(&method("PROPFIND"), None), Write);
    }

    #[test]
    fn dispatch_follows_executor_limits_and_is_capped_on_lambda() {
        assert_eq!(dispatch_secs(3600, 300, false), 3960);
        assert_eq!(dispatch_secs(3600, 300, true), LAMBDA_DISPATCH_SECS);
        assert_eq!(dispatch_secs(120, 60, true), 240);
    }

    #[test]
    fn parameters_match_by_position_not_name() {
        assert_eq!(
            classify(
                &Method::POST,
                Some("/api/v1/apps/{id}/events/{event}/invoke")
            ),
            Dispatch
        );
        assert_eq!(
            classify(
                &Method::POST,
                Some("/api/v1/apps/search/events/{event_id}/invoke")
            ),
            Write,
            "a literal segment never matches a parameter"
        );
        assert_eq!(
            classify(
                &Method::POST,
                Some("/api/v1/apps/{app_id}/events/{event_id}/invoke/x")
            ),
            Write,
            "exact rules do not match longer templates"
        );
    }

    fn surface(router: Router) -> Router {
        Router::new().nest("/api/v1", router.layer(from_fn(deadline_middleware)))
    }

    /// The paused clock advances to whole timer ticks, so allow sub-second slack.
    fn assert_at_deadline(started: Instant, class: DeadlineClass) {
        let elapsed = started.elapsed();
        assert!(
            elapsed >= class.budget() && elapsed < class.budget() + Duration::from_secs(1),
            "ended after {elapsed:?}, expected the {:?} deadline",
            class.budget()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_handler_gets_the_api_504_at_its_class_deadline() {
        let router = surface(Router::new().route(
            "/apps/{app_id}/events",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                "late"
            }),
        ));
        let started = Instant::now();
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/apps/a1/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_at_deadline(started, Read);
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], "GATEWAY_TIMEOUT");
        assert_eq!(
            body["error"]["message"],
            "The request exceeded its 10 s deadline"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_nested_route_is_classified_by_its_full_template() {
        let router = surface(Router::new().route(
            "/apps/{app_id}/board/{board_id}/invoke",
            post(|| async {
                tokio::time::sleep(Duration::from_secs(300)).await;
                "done"
            }),
        ));
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/apps/a1/board/b1/invoke")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(to_bytes(response.into_body(), 64).await.unwrap(), "done");
    }

    #[tokio::test(start_paused = true)]
    async fn a_fast_handler_is_untouched_and_still_compressed() {
        let payload = "x".repeat(4096);
        let router = surface(
            Router::new().route("/apps/{app_id}/events", get(move || async move { payload })),
        )
        .layer(tower_http::compression::CompressionLayer::new());
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/apps/a1/events")
                    .header(header::ACCEPT_ENCODING, "gzip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
        let compressed = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        assert!(compressed.starts_with(&[0x1f, 0x8b]), "gzip member header");
        assert!(compressed.len() < 4096);
    }

    fn ticking_stream(period: Duration) -> Body {
        Body::from_stream(futures::stream::unfold(0u32, move |tick| async move {
            tokio::time::sleep(period).await;
            Some((
                Ok::<_, std::io::Error>(Bytes::from(format!("tick {tick}\n"))),
                tick + 1,
            ))
        }))
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_ends_cleanly_at_the_deadline_measured_from_request_start() {
        let router = surface(Router::new().route(
            "/apps/{app_id}/events",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                ticking_stream(Duration::from_secs(4))
            }),
        ));
        let started = Instant::now();
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/apps/a1/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let frames: Vec<_> = response.into_body().into_data_stream().collect().await;
        assert_at_deadline(started, Read);
        let frames: Vec<_> = frames.into_iter().map(Result::unwrap).collect();
        assert_eq!(frames, ["tick 0\n", "tick 1\n"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_finished_stream_is_not_reported_as_cut() {
        let fault = ResponseFault::default();
        let expiry = Expiry {
            method: Method::GET,
            route: None,
            class: Read,
            fault: Some(fault.clone()),
        };
        let body = Body::from_stream(futures::stream::iter([Ok::<_, std::io::Error>(
            Bytes::from_static(b"all"),
        )]));
        let response = enforce(
            async move { Response::new(body) },
            Instant::now() + Read.budget(),
            expiry,
        )
        .await;
        let body = to_bytes(response.into_body(), 64).await.unwrap();
        tokio::time::sleep(Read.budget() * 2).await;
        assert_eq!(body, "all");
        assert!(!fault.is_marked());
    }

    #[tokio::test(start_paused = true)]
    async fn a_cut_stream_is_recorded_as_an_error_by_the_request_span() {
        use crate::{
            middleware::trace_context::trace_context_middleware,
            telemetry::spans::{SpanExportConfig, telemetry_span_layer},
        };
        use tracing_subscriber::layer::SubscriberExt;

        let (layer, mut exporter) = telemetry_span_layer(SpanExportConfig {
            sample_rate: 1.0,
            ..SpanExportConfig::default()
        });
        let _subscriber =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        let router = surface(Router::new().route(
            "/apps/{app_id}/events",
            get(|| async { ticking_stream(Duration::from_secs(4)) }),
        ))
        .layer(from_fn(trace_context_middleware));
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/apps/a1/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        let spans = exporter.drain();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].status, "error");
    }
}
