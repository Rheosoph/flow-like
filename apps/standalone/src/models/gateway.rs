//! The loopback model gateway: OpenAI-compatible routes in front of the engines. Every
//! request runs as a principal, under that principal's limits, with its usage recorded.
//!
//! Principals:
//! - a placement presents the bearer token the broker issued to it;
//! - the tunnel forwarder presents the per-boot agent secret and names the authenticated
//!   grant in [`PRINCIPAL_HEADER`] (`owner` or `grant:<id>`).
//!
//! Nothing a client sends besides the body and its content type reaches an engine, and of the
//! body's images only inline data (see [`images`]).
#![allow(clippy::result_large_err)] // Err is axum's Response, the currency type of this module

mod images;

use super::{
    db::RequestRecord,
    fetch::Fetcher,
    stats::{CLIENT_CLOSED, StatsRecorder},
    supervisor::{AcquireError, EngineLease, Supervisor},
};
use crate::enrollment::unix_time;
use anyhow::{Context, Result};
use axum::{
    Extension, Json, Router,
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    serve::{Listener, ListenerExt},
};
use flow_like_device_protocol::{ModelConsumer, ModelKind, validate_management_id};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub const AGENT_SECRET_HEADER: &str = "x-flowlike-agent-secret";
pub const PRINCIPAL_HEADER: &str = "x-flowlike-principal";
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENGINE_ANSWER_BYTES: usize = 64 * 1024 * 1024;
/// The longest event of an engine's stream the gateway holds while it waits for the event's end.
const MAX_EVENT_BYTES: usize = 1024 * 1024;
const STREAM_BUFFER: usize = 32;
/// Request bodies count against the in-flight budget in kibibytes.
const BODY_UNIT: usize = 1024;

#[derive(Clone, Debug)]
pub struct GatewayConfig {
    pub concurrency_per_principal: usize,
    pub queue_per_principal: usize,
    pub queue_wait: Duration,
    /// Connections served at once; further clients wait to be accepted.
    pub max_connections: usize,
    /// Bytes of request bodies the gateway holds at once, across all connections.
    pub max_body_bytes_in_flight: usize,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            concurrency_per_principal: 8,
            queue_per_principal: 32,
            queue_wait: Duration::from_secs(60),
            max_connections: 256,
            max_body_bytes_in_flight: 256 * 1024 * 1024,
        }
    }
}

/// Bearer tokens of placements, one per placement for this boot of the agent, and the hosted
/// models each placement's pinned Bits resolved to, the only models the placement may call.
#[derive(Default)]
pub struct PlacementTokens {
    by_placement: Mutex<HashMap<String, Zeroizing<String>>>,
    by_hash: Mutex<HashMap<[u8; 32], String>>,
    /// The hosted models each placement's pinned Bits resolved to, whichever revision of the
    /// placement asked, until its token is revoked.
    models: Mutex<HashMap<String, HashSet<String>>>,
}

impl PlacementTokens {
    pub fn issue(&self, placement_id: &str) -> Result<Zeroizing<String>> {
        validate_management_id(placement_id)?;
        let mut tokens = lock!(self.by_placement);
        if let Some(token) = tokens.get(placement_id) {
            return Ok(token.clone());
        }
        let token = Zeroizing::new(format!("flp_{}", super::engines::random_key().as_str()));
        lock!(self.by_hash).insert(
            *blake3::hash(token.as_bytes()).as_bytes(),
            placement_id.to_owned(),
        );
        tokens.insert(placement_id.to_owned(), token.clone());
        Ok(token)
    }

    /// The placement's token, which from now on may call the model one of its pinned Bits
    /// resolved to.
    pub fn issue_for(&self, placement_id: &str, model_id: &str) -> Result<Zeroizing<String>> {
        let token = self.issue(placement_id)?;
        lock!(self.models)
            .entry(placement_id.to_owned())
            .or_default()
            .insert(model_id.to_owned());
        Ok(token)
    }

    pub fn revoke(&self, placement_id: &str) {
        if let Some(token) = lock!(self.by_placement).remove(placement_id) {
            lock!(self.by_hash).remove(blake3::hash(token.as_bytes()).as_bytes());
        }
        lock!(self.models).remove(placement_id);
    }

    /// Revokes the token of every placement `running` refuses; one that starts again gets a
    /// new token.
    pub fn retain(&self, running: impl Fn(&str) -> bool) {
        let mut tokens = lock!(self.by_placement);
        let mut hashes = lock!(self.by_hash);
        tokens.retain(|placement_id, token| {
            let kept = running(placement_id);
            if !kept {
                hashes.remove(blake3::hash(token.as_bytes()).as_bytes());
            }
            kept
        });
        lock!(self.models).retain(|placement_id, _| tokens.contains_key(placement_id));
    }

    fn placement(&self, token: &str) -> Option<String> {
        lock!(self.by_hash)
            .get(blake3::hash(token.as_bytes()).as_bytes())
            .cloned()
    }

    /// Whether one of the placement's pinned Bits resolved to this model.
    fn allows(&self, placement_id: &str, model_id: &str) -> bool {
        lock!(self.models)
            .get(placement_id)
            .is_some_and(|models| models.contains(model_id))
    }
}

/// Accepts a connection only while fewer than its permits are open; each connection gives its
/// permit back when it closes.
struct Capped<L> {
    listener: L,
    permits: Arc<Semaphore>,
}

impl<L: Listener> Listener for Capped<L> {
    type Io = Counted<L::Io>;
    type Addr = L::Addr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let Ok(permit) = Arc::clone(&self.permits).acquire_owned().await else {
            return std::future::pending().await;
        };
        let (io, address) = self.listener.accept().await;
        let io = Counted {
            io,
            _permit: permit,
        };
        (io, address)
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

/// A connection holding one of the gateway's connection permits.
struct Counted<Io> {
    io: Io,
    _permit: OwnedSemaphorePermit,
}

impl<Io: AsyncRead + Unpin> AsyncRead for Counted<Io> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut std::task::Context,
        buffer: &mut ReadBuf,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(context, buffer)
    }
}

impl<Io: AsyncWrite + Unpin> AsyncWrite for Counted<Io> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut std::task::Context,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(context, bytes)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        context: &mut std::task::Context,
        buffers: &[std::io::IoSlice],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write_vectored(context, buffers)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        context: &mut std::task::Context,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(context)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        context: &mut std::task::Context,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(context)
    }
}

struct Limit {
    permits: Arc<Semaphore>,
    queued: AtomicUsize,
}

struct Shared {
    supervisor: Supervisor,
    stats: StatsRecorder,
    tokens: Arc<PlacementTokens>,
    secret: Zeroizing<String>,
    config: GatewayConfig,
    limits: Mutex<HashMap<ModelConsumer, Arc<Limit>>>,
    /// Kibibytes of request bodies the gateway may still hold.
    bodies: Arc<Semaphore>,
    client: reqwest::Client,
    /// Fetches the public images a chat request links, under the agent's address policy.
    fetcher: Fetcher,
}

pub struct Gateway {
    address: SocketAddr,
    secret: Zeroizing<String>,
    tokens: Arc<PlacementTokens>,
}

impl Gateway {
    pub async fn start(
        supervisor: Supervisor,
        stats: StatsRecorder,
        config: GatewayConfig,
        fetcher: Fetcher,
        cancel: CancellationToken,
    ) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .context("Bind the model gateway on loopback")?;
        let address = listener.local_addr()?;
        let secret = super::engines::random_key();
        let tokens = Arc::new(PlacementTokens::default());
        let connections = config.max_connections;
        let shared = Arc::new(Shared {
            supervisor,
            stats,
            tokens: Arc::clone(&tokens),
            secret: secret.clone(),
            bodies: Arc::new(Semaphore::new(config.max_body_bytes_in_flight / BODY_UNIT)),
            config,
            limits: Mutex::default(),
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .tcp_nodelay(true)
                .build()
                .context("Build the gateway's engine client")?,
            fetcher,
        });
        serve(listener, routes(shared), connections, cancel);
        Ok(Self {
            address,
            secret,
            tokens,
        })
    }

    /// `http://127.0.0.1:<port>/v1`, the API base engines' clients use.
    pub fn base_url(&self) -> String {
        format!("http://{}/v1", self.address)
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The per-boot secret the tunnel forwarder presents with [`AGENT_SECRET_HEADER`].
    pub fn agent_secret(&self) -> &str {
        &self.secret
    }

    pub fn tokens(&self) -> &Arc<PlacementTokens> {
        &self.tokens
    }
}

/// The OpenAI routes, each behind authentication from the request's headers.
fn routes(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/embeddings", post(embeddings))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&shared),
            authenticated,
        ))
        .with_state(shared)
}

/// Serves `router` until `cancel`, on at most `connections` connections at once.
fn serve(
    listener: tokio::net::TcpListener,
    router: Router,
    connections: usize,
    cancel: CancellationToken,
) {
    let listener = Capped {
        listener: listener.tap_io(|stream| {
            let _ = stream.set_nodelay(true);
        }),
        permits: Arc::new(Semaphore::new(connections.max(1))),
    };
    tokio::spawn(async move {
        let serve = axum::serve(listener, router)
            .with_graceful_shutdown(async move { cancel.cancelled().await });
        if let Err(error) = serve.await {
            tracing::warn!("The model gateway stopped: {error}");
        }
    });
}

fn error(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"error": {"message": message, "type": kind}})),
    )
        .into_response()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (left, right)| difference | (left ^ right))
            == 0
}

/// `owner` or `grant:<id>`, set by the tunnel forwarder alone.
pub fn principal_header(consumer: &ModelConsumer) -> Option<String> {
    match consumer {
        ModelConsumer::Owner => Some("owner".into()),
        ModelConsumer::Grant { grant_id } => Some(format!("grant:{grant_id}")),
        ModelConsumer::Placement { .. } => None,
    }
}

fn parse_principal(value: &str) -> Option<ModelConsumer> {
    if value == "owner" {
        return Some(ModelConsumer::Owner);
    }
    let grant_id = value.strip_prefix("grant:")?;
    validate_management_id(grant_id).ok()?;
    Some(ModelConsumer::Grant {
        grant_id: grant_id.to_owned(),
    })
}

fn authenticate(shared: &Shared, headers: &HeaderMap) -> Option<ModelConsumer> {
    let text = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    if let Some(secret) = headers.get(AGENT_SECRET_HEADER) {
        if !constant_time_eq(secret.as_bytes(), shared.secret.as_bytes()) {
            return None;
        }
        return parse_principal(text(PRINCIPAL_HEADER)?);
    }
    let token = text(header::AUTHORIZATION.as_str())?.strip_prefix("Bearer ")?;
    shared
        .tokens
        .placement(token)
        .map(|placement_id| ModelConsumer::Placement { placement_id })
}

fn unauthorized() -> Response {
    error(
        StatusCode::UNAUTHORIZED,
        "authentication_error",
        "This gateway needs a placement token or the tunnel's grant",
    )
}

/// Authenticates from the headers alone, so no handler reads the body of an unknown caller.
async fn authenticated(
    State(shared): State<Arc<Shared>>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(consumer) = authenticate(&shared, request.headers()) else {
        return unauthorized();
    };
    request.extensions_mut().insert(consumer);
    next.run(request).await
}

fn too_many(retry_after: Duration, message: &str) -> Response {
    let mut response = error(StatusCode::TOO_MANY_REQUESTS, "rate_limit_error", message);
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from(retry_after.as_secs().max(1)),
    );
    response
}

fn too_large() -> Response {
    error(
        StatusCode::PAYLOAD_TOO_LARGE,
        "invalid_request_error",
        "The request body is larger than this gateway accepts, or it broke off",
    )
}

/// A request body read after its principal was authenticated; it counts against the gateway's
/// budget for bodies in memory until the request has gone to the engine.
struct RequestBody {
    bytes: Bytes,
    held: OwnedSemaphorePermit,
}

/// Waits for room in the in-flight budget for the declared length, else for the longest body
/// the gateway accepts, then reads the body up to that length.
async fn read_body(
    shared: &Shared,
    headers: &HeaderMap,
    body: Body,
) -> std::result::Result<RequestBody, Response> {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok()?.parse::<usize>().ok());
    let limit = declared.unwrap_or(MAX_BODY_BYTES);
    if limit > MAX_BODY_BYTES.min(shared.config.max_body_bytes_in_flight) {
        return Err(too_large());
    }
    let units = u32::try_from(limit.div_ceil(BODY_UNIT).max(1)).unwrap_or(u32::MAX);
    let room = Arc::clone(&shared.bodies).acquire_many_owned(units);
    let Ok(Ok(held)) = tokio::time::timeout(shared.config.queue_wait, room).await else {
        let mut response = error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            "The gateway holds too many request bodies; retry shortly",
        );
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(1));
        return Err(response);
    };
    let bytes = axum::body::to_bytes(body, limit)
        .await
        .map_err(|_| too_large())?;
    Ok(RequestBody { bytes, held })
}

impl Shared {
    /// A placement calls only the models its pinned Bits resolved to; the owner and the grants
    /// holding Use models, which the tunnel checked, call every hosted model.
    fn may_call(&self, consumer: &ModelConsumer, model_id: &str) -> bool {
        match consumer {
            ModelConsumer::Placement { placement_id } => self.tokens.allows(placement_id, model_id),
            ModelConsumer::Owner | ModelConsumer::Grant { .. } => true,
        }
    }

    fn limit(&self, consumer: &ModelConsumer) -> Arc<Limit> {
        Arc::clone(
            lock!(self.limits)
                .entry(consumer.clone())
                .or_insert_with(|| {
                    Arc::new(Limit {
                        permits: Arc::new(Semaphore::new(self.config.concurrency_per_principal)),
                        queued: AtomicUsize::new(0),
                    })
                }),
        )
    }

    /// A concurrency slot of the principal; waits in its bounded queue, or answers 429.
    async fn admit(
        &self,
        consumer: &ModelConsumer,
    ) -> std::result::Result<OwnedSemaphorePermit, Response> {
        let limit = self.limit(consumer);
        if let Ok(permit) = Arc::clone(&limit.permits).try_acquire_owned() {
            return Ok(permit);
        }
        if limit.queued.fetch_add(1, Ordering::SeqCst) >= self.config.queue_per_principal {
            limit.queued.fetch_sub(1, Ordering::SeqCst);
            return Err(too_many(
                Duration::from_secs(1),
                "Too many requests are waiting for this consumer",
            ));
        }
        let waited = tokio::time::timeout(
            self.config.queue_wait,
            Arc::clone(&limit.permits).acquire_owned(),
        )
        .await;
        limit.queued.fetch_sub(1, Ordering::SeqCst);
        match waited {
            Ok(Ok(permit)) => Ok(permit),
            _ => Err(too_many(
                Duration::from_secs(1),
                "This consumer's requests waited too long for a free slot",
            )),
        }
    }
}

async fn models(
    State(shared): State<Arc<Shared>>,
    Extension(consumer): Extension<ModelConsumer>,
) -> Response {
    let data: Vec<Value> = shared
        .supervisor
        .models()
        .into_iter()
        .filter(|model| shared.may_call(&consumer, &model.id))
        .map(|model| {
            json!({"id": model.id, "object": "model", "owned_by": "device",
                   "kind": model.kind, "name": model.display_name})
        })
        .collect();
    Json(json!({"object": "list", "data": data})).into_response()
}

/// What one request records once it ends.
struct Usage {
    consumer: ModelConsumer,
    model_id: String,
    started: Instant,
    queue_ms: u32,
    request_bytes: u64,
    response_bytes: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    cached_tokens: u64,
    first_token: Option<Instant>,
    last_token: Option<Instant>,
    decode_ms: Option<u32>,
}

fn millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

impl Usage {
    /// `queue_ms` is the wait for one of the principal's slots; a model load, which some
    /// requests wait for next, counts neither as queue wait nor as time to the first token.
    fn new(consumer: ModelConsumer, model_id: String, queue_ms: u32, request_bytes: u64) -> Self {
        Self {
            consumer,
            model_id,
            started: Instant::now(),
            queue_ms,
            request_bytes,
            response_bytes: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            cached_tokens: 0,
            first_token: None,
            last_token: None,
            decode_ms: None,
        }
    }

    fn absorb(&mut self, usage: &Value) {
        let number = |value: &Value| value.as_u64().unwrap_or(0);
        if let Some(prompt) = usage.get("prompt_tokens") {
            self.prompt_tokens = number(prompt);
        }
        if let Some(completion) = usage.get("completion_tokens") {
            self.completion_tokens = number(completion);
        }
        if let Some(cached) = usage.pointer("/prompt_tokens_details/cached_tokens") {
            self.cached_tokens = number(cached);
        }
    }

    fn absorb_timings(&mut self, timings: &Value) {
        if let Some(predicted) = timings.get("predicted_ms").and_then(Value::as_f64) {
            self.decode_ms = Some(predicted.max(0.0).min(f64::from(u32::MAX)) as u32);
        }
    }

    /// Records a request the gateway refused and passes the refusal on.
    fn refused(self, stats: &StatsRecorder, response: Response) -> Response {
        self.record(stats, response.status().as_u16());
        response
    }

    /// Time spent generating: the engine's figure, else the span from the first to the last
    /// streamed token. An answer that measured neither yet generated tokens (a non-streamed
    /// MLX answer, or one that arrived as a single event) counts its engine time, so every
    /// generated token carries decode time and `completion_tokens / decode_ms` stays a speed.
    fn generation_ms(&self, ended: Instant) -> u32 {
        if let Some(reported) = self.decode_ms {
            return reported;
        }
        let streamed = self
            .first_token
            .zip(self.last_token)
            .map_or(0, |(first, last)| {
                millis(last.saturating_duration_since(first))
            });
        if streamed > 0 || self.completion_tokens == 0 {
            return streamed;
        }
        millis(
            self.last_token
                .unwrap_or(ended)
                .saturating_duration_since(self.started),
        )
    }

    fn record(self, stats: &StatsRecorder, status: u16) {
        let ended = Instant::now();
        let ttft = self
            .first_token
            .map(|first| millis(first.saturating_duration_since(self.started)));
        let decode = self.generation_ms(ended);
        stats.record(RequestRecord {
            at: unix_time().unwrap_or_default(),
            model_id: self.model_id,
            consumer: self.consumer,
            status,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            cached_tokens: self.cached_tokens,
            ttft_ms: ttft,
            duration_ms: millis(ended.saturating_duration_since(self.started)),
            decode_ms: decode,
            queue_ms: self.queue_ms,
            request_bytes: self.request_bytes,
            response_bytes: self.response_bytes,
        });
    }
}

/// The parsed body of an OpenAI request and its model.
fn request_body(
    body: &Bytes,
) -> std::result::Result<(serde_json::Map<String, Value>, String), Response> {
    let Ok(Value::Object(request)) = serde_json::from_slice::<Value>(body) else {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "The body must be a JSON object",
        ));
    };
    let Some(model) = request
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "The request names no model",
        ));
    };
    Ok((request, model))
}

fn acquire_failure(failure: AcquireError) -> Response {
    match failure {
        AcquireError::NotFound(model) => error(
            StatusCode::NOT_FOUND,
            "model_not_found",
            &format!("This device hosts no model called {model}"),
        ),
        AcquireError::Unavailable {
            message,
            retry_after,
        } => {
            let mut response = error(
                StatusCode::SERVICE_UNAVAILABLE,
                "model_unavailable",
                &message,
            );
            if let Some(retry_after) = retry_after {
                response.headers_mut().insert(
                    header::RETRY_AFTER,
                    HeaderValue::from(retry_after.as_secs().max(1)),
                );
            }
            response
        }
        AcquireError::Failed(failure) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "model_unavailable",
            &format!("{failure:#}"),
        ),
    }
}

/// A request past authentication, its model's kind, the principal's limit and a lease, and
/// its place in the budget for bodies in memory.
struct Admitted {
    request: serde_json::Map<String, Value>,
    permit: OwnedSemaphorePermit,
    lease: EngineLease,
    usage: Usage,
    body: OwnedSemaphorePermit,
}

/// Refuses models the device does not host, models the principal may not call, which look
/// the same, and models of a kind the route does not serve.
fn check_route(
    shared: &Shared,
    consumer: &ModelConsumer,
    model_id: &str,
    kinds: &[ModelKind],
) -> std::result::Result<(), Response> {
    let model = shared
        .supervisor
        .model(model_id)
        .filter(|_| shared.may_call(consumer, model_id))
        .ok_or_else(|| acquire_failure(AcquireError::NotFound(model_id.to_owned())))?;
    if kinds.contains(&model.kind) {
        return Ok(());
    }
    Err(error(
        StatusCode::BAD_REQUEST,
        "invalid_request_error",
        &format!(
            "Model {model_id} is a {:?} model and cannot serve this route",
            model.kind
        ),
    ))
}

/// Requests refused past authentication are recorded too, so the statistics show a
/// principal's 429s and a model that could not load.
async fn prepare(
    shared: &Shared,
    consumer: ModelConsumer,
    headers: &HeaderMap,
    body: Body,
    kinds: &[ModelKind],
) -> std::result::Result<Admitted, Response> {
    let body = read_body(shared, headers, body).await?;
    let (mut request, model_id) = request_body(&body.bytes)?;
    check_route(shared, &consumer, &model_id, kinds)?;
    let linked = images::linked(&mut request)?;
    let queued = Instant::now();
    let admitted = shared.admit(&consumer).await;
    let mut usage = Usage::new(
        consumer,
        model_id,
        millis(queued.elapsed()),
        body.bytes.len() as u64,
    );
    let permit = match admitted {
        Ok(permit) => permit,
        Err(response) => return Err(usage.refused(&shared.stats, response)),
    };
    let lease = match engine_for(shared, &mut request, linked, &usage.model_id).await {
        Ok(lease) => lease,
        Err(response) => return Err(usage.refused(&shared.stats, response)),
    };
    usage.started = Instant::now();
    Ok(Admitted {
        request,
        permit,
        lease,
        usage,
        body: body.held,
    })
}

/// Puts the request's linked images inline, then leases the model's engine, loading it first
/// when it is not running.
async fn engine_for(
    shared: &Shared,
    request: &mut serde_json::Map<String, Value>,
    linked: Vec<images::Linked>,
    model_id: &str,
) -> std::result::Result<EngineLease, Response> {
    images::inline(&shared.fetcher, request, linked).await?;
    shared
        .supervisor
        .acquire(model_id)
        .await
        .map_err(acquire_failure)
}

async fn send(
    shared: &Shared,
    lease: &EngineLease,
    path: &str,
    request: &serde_json::Map<String, Value>,
) -> std::result::Result<reqwest::Response, Response> {
    shared
        .client
        .post(format!("{}{path}", lease.base_url()))
        .bearer_auth(lease.key())
        .header(header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(request).unwrap_or_default())
        .send()
        .await
        .map_err(|failure| {
            shared.supervisor.engine_unreachable(lease.model_id());
            error(
                StatusCode::BAD_GATEWAY,
                "engine_error",
                &format!(
                    "The engine of {} did not answer: {failure}",
                    lease.model_id()
                ),
            )
        })
}

async fn chat_completions(
    State(shared): State<Arc<Shared>>,
    Extension(consumer): Extension<ModelConsumer>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let kinds = [ModelKind::Chat, ModelKind::Vision];
    let admitted = match prepare(&shared, consumer, &headers, body, &kinds).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    if admitted.request.get("stream") == Some(&Value::Bool(true)) {
        stream_chat(shared, admitted).await
    } else {
        forward_whole(shared, admitted, "/v1/chat/completions").await
    }
}

/// Makes the engine report usage on the stream; answers whether the client asked for it.
fn force_usage(request: &mut serde_json::Map<String, Value>) -> bool {
    let options = request.entry("stream_options").or_insert_with(|| json!({}));
    if !options.is_object() {
        *options = json!({});
    }
    let asked = options.get("include_usage") == Some(&Value::Bool(true));
    options["include_usage"] = Value::Bool(true);
    asked
}

async fn stream_chat(shared: Arc<Shared>, admitted: Admitted) -> Response {
    let Admitted {
        mut request,
        permit,
        lease,
        usage,
        body,
    } = admitted;
    let client_wants_usage = force_usage(&mut request);
    let sent = send(&shared, &lease, "/v1/chat/completions", &request).await;
    drop((request, body));
    let upstream = match sent {
        Ok(upstream) => upstream,
        Err(response) => {
            usage.record(&shared.stats, response.status().as_u16());
            return response;
        }
    };
    if !upstream.status().is_success() {
        return finish_whole(shared, upstream, usage).await;
    }
    let (sender, receiver) =
        mpsc::channel::<std::result::Result<Bytes, std::io::Error>>(STREAM_BUFFER);
    tokio::spawn(relay(
        Arc::clone(&shared),
        upstream,
        sender,
        usage,
        client_wants_usage,
        (permit, lease),
    ));
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn embeddings(
    State(shared): State<Arc<Shared>>,
    Extension(consumer): Extension<ModelConsumer>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    match prepare(&shared, consumer, &headers, body, &[ModelKind::Embedding]).await {
        Ok(admitted) => forward_whole(shared, admitted, "/v1/embeddings").await,
        Err(response) => response,
    }
}

/// The permit and lease stay held until the whole answer has been read; the request's body
/// leaves the budget once the engine has it.
async fn forward_whole(shared: Arc<Shared>, admitted: Admitted, path: &str) -> Response {
    let Admitted {
        request,
        permit,
        lease,
        usage,
        body,
    } = admitted;
    let sent = send(&shared, &lease, path, &request).await;
    drop((request, body));
    let response = match sent {
        Ok(upstream) => finish_whole(Arc::clone(&shared), upstream, usage).await,
        Err(response) => {
            usage.record(&shared.stats, response.status().as_u16());
            response
        }
    };
    drop((permit, lease));
    response
}

/// Reads a whole engine answer, records its usage and passes it on unchanged.
async fn finish_whole(
    shared: Arc<Shared>,
    mut upstream: reqwest::Response,
    mut usage: Usage,
) -> Response {
    let status = upstream.status();
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or(HeaderValue::from_static("application/json"));
    let mut bytes = Vec::new();
    loop {
        match upstream.chunk().await {
            Ok(Some(chunk)) if bytes.len() + chunk.len() <= MAX_ENGINE_ANSWER_BYTES => {
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            _ => {
                usage.record(&shared.stats, StatusCode::BAD_GATEWAY.as_u16());
                return error(
                    StatusCode::BAD_GATEWAY,
                    "engine_error",
                    "The engine's answer broke off or was too large",
                );
            }
        }
    }
    if let Ok(answer) = serde_json::from_slice::<Value>(&bytes) {
        if let Some(found) = answer.get("usage") {
            usage.absorb(found);
        }
        if let Some(timings) = answer.get("timings") {
            usage.absorb_timings(timings);
        }
    }
    usage.response_bytes = bytes.len() as u64;
    usage.record(&shared.stats, status.as_u16());
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() =
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response
}

/// One server-sent event of the engine, rewritten for the client.
enum Event {
    Pass(Bytes),
    Rewritten(Bytes),
    Drop,
}

/// Captures usage, timings and the first token, and hides the usage chunk the gateway
/// asked for when the client did not.
fn rewrite(event: &[u8], usage: &mut Usage, client_wants_usage: bool) -> Event {
    let pass = || Event::Pass(Bytes::copy_from_slice(event));
    let Some(mut chunk) = event_json(event) else {
        return pass();
    };
    if carries_tokens(&chunk) {
        let now = Instant::now();
        usage.first_token.get_or_insert(now);
        usage.last_token = Some(now);
    }
    if let Some(timings) = chunk.get("timings") {
        usage.absorb_timings(timings);
    }
    let Some(found) = chunk.get("usage").filter(|value| !value.is_null()).cloned() else {
        return pass();
    };
    usage.absorb(&found);
    if client_wants_usage {
        return pass();
    }
    let choices = chunk.get("choices").and_then(Value::as_array);
    if choices.is_none_or(Vec::is_empty) {
        return Event::Drop;
    }
    if let Some(object) = chunk.as_object_mut() {
        object.remove("usage");
    }
    Event::Rewritten(Bytes::from(format!("data: {chunk}\n\n")))
}

/// The JSON of a `data:` event; `None` for comments, `[DONE]` and anything unreadable.
fn event_json(event: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(event);
    let data: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect();
    if data.is_empty() || data == ["[DONE]"] {
        return None;
    }
    serde_json::from_str(&data.join("\n")).ok()
}

fn carries_tokens(chunk: &Value) -> bool {
    let delta_has = |choice: &Value| {
        ["content", "reasoning_content", "tool_calls"]
            .iter()
            .any(|field| {
                choice["delta"]
                    .get(*field)
                    .is_some_and(|value| !value.is_null())
            })
    };
    chunk
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|choices| choices.iter().any(delta_has))
}

type EventSender = mpsc::Sender<std::result::Result<Bytes, std::io::Error>>;

/// Where a stream stands after one chunk of the engine.
enum Relayed {
    More,
    ClientGone,
    /// The engine sent more than [`MAX_EVENT_BYTES`] without ending an event.
    Oversized,
}

/// Passes on every event the chunk completes and keeps the start of the next one.
async fn relay_chunk(
    buffer: &mut Vec<u8>,
    chunk: &[u8],
    usage: &mut Usage,
    client_wants_usage: bool,
    sender: &EventSender,
) -> Relayed {
    buffer.extend_from_slice(chunk);
    while let Some(end) = event_end(buffer) {
        let event: Vec<u8> = buffer.drain(..end).collect();
        let bytes = match rewrite(&event, usage, client_wants_usage) {
            Event::Pass(bytes) | Event::Rewritten(bytes) => bytes,
            Event::Drop => continue,
        };
        usage.response_bytes += bytes.len() as u64;
        if sender.send(Ok(bytes)).await.is_err() {
            return Relayed::ClientGone;
        }
    }
    if buffer.len() > MAX_EVENT_BYTES {
        return Relayed::Oversized;
    }
    Relayed::More
}

/// The error event that ends a stream whose engine sent an event too long to hold.
fn oversized_event() -> Bytes {
    let error = json!({"error": {
        "message": "The engine sent a stream event longer than 1 MiB",
        "type": "engine_error",
    }});
    Bytes::from(format!("data: {error}\n\n"))
}

/// Streams the engine's events to the client while the permit and lease stay held.
async fn relay(
    shared: Arc<Shared>,
    mut upstream: reqwest::Response,
    sender: EventSender,
    mut usage: Usage,
    client_wants_usage: bool,
    held: (OwnedSemaphorePermit, EngineLease),
) {
    let mut buffer: Vec<u8> = Vec::new();
    let mut status = StatusCode::OK.as_u16();
    loop {
        let next = tokio::select! {
            () = sender.closed() => {
                status = CLIENT_CLOSED;
                break;
            }
            next = upstream.chunk() => next,
        };
        let chunk = match next {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(_) => {
                status = StatusCode::BAD_GATEWAY.as_u16();
                break;
            }
        };
        match relay_chunk(&mut buffer, &chunk, &mut usage, client_wants_usage, &sender).await {
            Relayed::More => {}
            Relayed::ClientGone => {
                status = CLIENT_CLOSED;
                break;
            }
            Relayed::Oversized => {
                status = StatusCode::BAD_GATEWAY.as_u16();
                let _ = sender.send(Ok(oversized_event())).await;
                break;
            }
        }
    }
    if !buffer.is_empty() && status == StatusCode::OK.as_u16() {
        usage.response_bytes += buffer.len() as u64;
        let _ = sender.send(Ok(Bytes::from(buffer))).await;
    }
    drop(upstream);
    usage.record(&shared.stats, status);
    drop(held);
}

/// The end of the first complete event: a blank line, in LF or CRLF form.
fn event_end(buffer: &[u8]) -> Option<usize> {
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|at| at + 2);
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|at| at + 4);
    match (lf, crlf) {
        (Some(lf), Some(crlf)) => Some(lf.min(crlf)),
        (lf, crlf) => lf.or(crlf),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage() -> Usage {
        Usage::new(ModelConsumer::Owner, "m".into(), 0, 0)
    }

    #[test]
    fn the_usage_chunk_is_hidden_unless_asked_for() {
        let usage_only =
            br#"data: {"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":2}}}

"#;
        let mut captured = usage();
        assert!(matches!(
            rewrite(usage_only, &mut captured, false),
            Event::Drop
        ));
        assert_eq!(
            (
                captured.prompt_tokens,
                captured.completion_tokens,
                captured.cached_tokens
            ),
            (7, 3, 2)
        );
        assert!(matches!(
            rewrite(usage_only, &mut usage(), true),
            Event::Pass(_)
        ));

        let attached =
            br#"data: {"choices":[{"delta":{"content":"x"}}],"usage":{"completion_tokens":1}}

"#;
        let mut captured = usage();
        let Event::Rewritten(bytes) = rewrite(attached, &mut captured, false) else {
            panic!("the attached usage is removed");
        };
        assert!(!String::from_utf8_lossy(&bytes).contains("usage"));
        assert_eq!(captured.completion_tokens, 1);
        assert!(captured.first_token.is_some());

        let done = b"data: [DONE]\n\n";
        assert!(matches!(rewrite(done, &mut usage(), false), Event::Pass(_)));
    }

    #[test]
    fn every_generated_token_carries_decode_time() {
        let mut answer = usage();
        let at = |milliseconds: u64| answer.started + Duration::from_millis(milliseconds);
        let (ended, single, last) = (at(2_000), at(900), at(1_900));

        answer.completion_tokens = 100;
        assert_eq!(
            answer.generation_ms(ended),
            2_000,
            "not streamed, no timings"
        );

        answer.first_token = Some(single);
        answer.last_token = Some(single);
        assert_eq!(answer.generation_ms(ended), 900, "streamed as one event");

        answer.last_token = Some(last);
        assert_eq!(answer.generation_ms(ended), 1_000, "streamed");

        answer.decode_ms = Some(750);
        assert_eq!(answer.generation_ms(ended), 750, "the engine's figure");

        let embedding = usage();
        assert_eq!(embedding.generation_ms(ended), 0, "nothing generated");
    }

    #[test]
    fn events_end_at_blank_lines() {
        assert_eq!(event_end(b"data: a\n\ndata: b"), Some(9));
        assert_eq!(event_end(b"data: a\r\n\r\n"), Some(11));
        assert_eq!(event_end(b"data: a\n"), None);
    }

    #[test]
    fn principals_parse_strictly() {
        assert_eq!(parse_principal("owner"), Some(ModelConsumer::Owner));
        assert_eq!(
            parse_principal("grant:g-1"),
            Some(ModelConsumer::Grant {
                grant_id: "g-1".into()
            })
        );
        assert_eq!(parse_principal("placement:p"), None);
        assert_eq!(parse_principal("grant:a b"), None);
        assert_eq!(
            principal_header(&ModelConsumer::Placement {
                placement_id: "p".into()
            }),
            None
        );
    }

    #[test]
    fn placement_tokens_are_stable_per_boot_and_revocable() -> Result<()> {
        let tokens = PlacementTokens::default();
        let first = tokens.issue("api")?;
        assert_eq!(tokens.issue("api")?, first);
        assert_eq!(tokens.placement(&first).as_deref(), Some("api"));
        assert_ne!(tokens.issue("web")?, first);
        tokens.revoke("api");
        assert_eq!(tokens.placement(&first), None);
        assert!(tokens.issue("../x").is_err());
        Ok(())
    }

    #[test]
    fn tokens_of_placements_without_a_running_replica_are_revoked() -> Result<()> {
        let tokens = PlacementTokens::default();
        let stopped = tokens.issue("api")?;
        let running = tokens.issue("web")?;
        tokens.retain(|placement_id| placement_id == "web");
        assert_eq!(tokens.placement(&stopped), None);
        assert_eq!(tokens.placement(&running).as_deref(), Some("web"));
        assert_eq!(tokens.issue("web")?, running);
        let restarted = tokens.issue("api")?;
        assert_ne!(restarted, stopped);
        assert_eq!(tokens.placement(&restarted).as_deref(), Some("api"));
        Ok(())
    }

    #[test]
    fn a_placement_may_call_only_the_models_its_bits_resolved_to() -> Result<()> {
        let tokens = PlacementTokens::default();
        let token = tokens.issue("api")?;
        assert!(!tokens.allows("api", "qwen"), "a token alone calls nothing");
        assert_eq!(tokens.issue_for("api", "qwen")?, token);
        tokens.issue_for("api", "nomic")?;
        assert!(tokens.allows("api", "qwen") && tokens.allows("api", "nomic"));
        assert!(!tokens.allows("api", "private-70b") && !tokens.allows("web", "qwen"));
        tokens.retain(|placement_id| placement_id == "web");
        assert!(!tokens.allows("api", "nomic"));
        tokens.issue_for("web", "qwen")?;
        tokens.revoke("web");
        assert!(!tokens.allows("web", "qwen"));
        Ok(())
    }

    #[tokio::test]
    async fn an_event_longer_than_the_cap_ends_the_stream() {
        let (sender, mut receiver) = mpsc::channel(4);
        let (mut buffer, mut answer) = (Vec::new(), usage());
        let first = b"data: {\"choices\":[]}\n\ndata: ";
        let relayed = relay_chunk(&mut buffer, first, &mut answer, true, &sender).await;
        assert!(matches!(relayed, Relayed::More));
        assert_eq!(buffer, b"data: ");
        let endless = vec![b'x'; MAX_EVENT_BYTES];
        let relayed = relay_chunk(&mut buffer, &endless, &mut answer, true, &sender).await;
        assert!(matches!(relayed, Relayed::Oversized));
        let passed = receiver.recv().await.and_then(std::result::Result::ok);
        assert_eq!(passed.as_deref(), Some(&b"data: {\"choices\":[]}\n\n"[..]));
        assert!(String::from_utf8_lossy(&oversized_event()).contains("\"error\""));
    }
}
