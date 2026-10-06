use crate::{Error, Result, client::DeviceSession, http::TunnelConnector, random_bytes};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use flow_like_device_protocol::TunnelOpen;
use http::{
    HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Uri, Version,
    header::{
        AUTHORIZATION, CONNECTION, EXPECT, HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE,
        TRAILER, TRANSFER_ENCODING, UPGRADE, WWW_AUTHENTICATE,
    },
};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::{
    client::legacy::Client,
    rt::{TokioIo, TokioTimer},
};
use std::{
    convert::Infallible,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, time::Instant};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

type ProxyBody = BoxBody<Bytes, hyper::Error>;

/// What a proxy serves: requests in flight, a streamed answer until its body ends, and when
/// a request last started, sent a part of its answer or ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProxyActivity {
    pub in_flight: usize,
    pub last_request: Option<Instant>,
}

#[derive(Default)]
struct Activity {
    in_flight: AtomicUsize,
    last: Mutex<Option<Instant>>,
}

impl Activity {
    fn begin(self: &Arc<Self>) -> InFlight {
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        self.stamp();
        InFlight(self.clone())
    }

    fn stamp(&self) {
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }

    fn read(&self) -> ProxyActivity {
        ProxyActivity {
            in_flight: self.in_flight.load(Ordering::Acquire),
            last_request: *self.last.lock().unwrap_or_else(PoisonError::into_inner),
        }
    }
}

/// One request, from its start until its answer's body ends or is dropped.
struct InFlight(Arc<Activity>);

impl InFlight {
    fn stamp(&self) {
        self.0.stamp();
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.stamp();
        self.0.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

const HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const HOP_BY_HOP: [HeaderName; 8] = [
    CONNECTION,
    HeaderName::from_static("keep-alive"),
    HeaderName::from_static("proxy-connection"),
    PROXY_AUTHENTICATE,
    TE,
    TRAILER,
    TRANSFER_ENCODING,
    UPGRADE,
];

struct ProxyState {
    client: Client<TunnelConnector, Incoming>,
    bearer: Zeroizing<String>,
    activity: Arc<Activity>,
}

/// The proxy a port forwards through now, if any.
struct Serving {
    authority: HeaderValue,
    current: Mutex<Option<Arc<ProxyState>>>,
}

impl Serving {
    fn slot(&self) -> MutexGuard<'_, Option<Arc<ProxyState>>> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stops `state`, unless a later proxy already took its place.
    fn stop(&self, state: &Arc<ProxyState>) {
        let mut current = self.slot();
        if current
            .as_ref()
            .is_some_and(|serving| Arc::ptr_eq(serving, state))
        {
            *current = None;
        }
    }
}

/// A port on 127.0.0.1 for one tunnel target, such as a device's model gateway, bound while
/// it lives. It forwards through the [`LoopbackProxy`] it serves now; before the first,
/// between two and after one closed, every request answers 503. A client that kept the
/// address of a closed proxy so reaches neither another process, which cannot take the port,
/// nor a later session, whose proxy has a bearer of its own.
pub struct LoopbackPort {
    address: SocketAddr,
    serving: Arc<Serving>,
    cancel: CancellationToken,
}

impl LoopbackPort {
    /// `authority` becomes the forwarded `Host`, for example `localhost`.
    pub async fn bind(authority: &str) -> Result<Arc<Self>> {
        let host = authority
            .parse::<http::uri::Authority>()
            .ok()
            .filter(|parsed| parsed.as_str() == authority)
            .and_then(|_| HeaderValue::from_str(authority).ok())
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "forwarded host {authority} is not a plain host[:port]"
                ))
            })?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| Error::Invalid(format!("Could not listen on 127.0.0.1: {error}")))?;
        let address = listener.local_addr().map_err(|error| {
            Error::Invalid(format!("Could not read the proxy address: {error}"))
        })?;
        let serving = Arc::new(Serving {
            authority: host,
            current: Mutex::default(),
        });
        let cancel = CancellationToken::new();
        tokio::spawn(serve(listener, serving.clone(), cancel.clone()));
        Ok(Arc::new(Self {
            address,
            serving,
            cancel,
        }))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }

    /// Forwards to `open` through `session` under a fresh bearer until the returned proxy
    /// closes. The proxy this port served before stops.
    pub fn serve(
        self: &Arc<Self>,
        session: DeviceSession,
        open: TunnelOpen,
    ) -> Result<LoopbackProxy> {
        let state = Arc::new(ProxyState {
            client: TunnelConnector::new(session, open)?.into_client(),
            bearer: Zeroizing::new(URL_SAFE_NO_PAD.encode(random_bytes::<32>())),
            activity: Arc::default(),
        });
        *self.serving.slot() = Some(state.clone());
        Ok(LoopbackProxy {
            port: self.clone(),
            state,
        })
    }
}

impl Drop for LoopbackPort {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// An HTTP/1.1 endpoint for one session on a [`LoopbackPort`]. Each request must carry the
/// proxy's bearer, which is removed before the request travels; bodies stream both ways,
/// including server-sent events. Closing or dropping it stops forwarding, and the port stays
/// bound while anything holds it.
pub struct LoopbackProxy {
    port: Arc<LoopbackPort>,
    state: Arc<ProxyState>,
}

impl LoopbackProxy {
    /// Authorized requests only; a streamed answer counts until its body ends.
    pub fn activity(&self) -> ProxyActivity {
        self.state.activity.read()
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.port.address
    }

    /// `http://127.0.0.1:<port>`; OpenAI-compatible clients append `/v1/...`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.port.address)
    }

    pub fn bearer(&self) -> &str {
        &self.state.bearer
    }

    /// New requests answer 503; answers already streaming end with their tunnel.
    pub fn close(&self) {
        self.port.serving.stop(&self.state);
    }
}

impl Drop for LoopbackProxy {
    fn drop(&mut self) {
        self.close();
    }
}

async fn serve(listener: TcpListener, serving: Arc<Serving>, cancel: CancellationToken) {
    loop {
        let accepted = tokio::select! {
            _ = cancel.cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        let socket = match accepted {
            Ok((socket, _)) => socket,
            Err(error) => {
                tracing::warn!("Device proxy accept failed: {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let _ = socket.set_nodelay(true);
        let serving = serving.clone();
        let cancel = cancel.child_token();
        tokio::spawn(async move {
            let service = service_fn(move |request| forward(serving.clone(), request));
            let connection = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .keep_alive(true)
                .serve_connection(TokioIo::new(socket), service);
            tokio::select! {
                _ = cancel.cancelled() => {}
                result = connection => if let Err(error) = result {
                    tracing::debug!("Device proxy connection ended: {error}");
                },
            }
        });
    }
}

async fn forward(
    serving: Arc<Serving>,
    request: Request<Incoming>,
) -> Result<Response<ProxyBody>, Infallible> {
    let current = serving.slot().clone();
    let Some(state) = current else {
        return Ok(Refusal::Closed.response());
    };
    let request = match outbound(&state, &serving.authority, request) {
        Ok(request) => request,
        Err(refusal) => return Ok(refusal.response()),
    };
    let in_flight = state.activity.begin();
    Ok(match state.client.request(request).await {
        Ok(response) => {
            let (mut parts, body) = response.into_parts();
            strip_hop_by_hop(&mut parts.headers);
            let body = body.map_frame(move |frame| {
                in_flight.stamp();
                frame
            });
            Response::from_parts(parts, body.boxed())
        }
        Err(error) => plain(
            StatusCode::BAD_GATEWAY,
            format!("The device did not answer: {}", describe(&error)),
        ),
    })
}

enum Refusal {
    Closed,
    Unauthorized,
    NotOriginForm,
    InvalidTarget,
}

impl Refusal {
    fn response(self) -> Response<ProxyBody> {
        let (status, message) = match self {
            Self::Closed => (
                StatusCode::SERVICE_UNAVAILABLE,
                "This device endpoint is closed: the device was locked or its connection ended.",
            ),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "This device endpoint needs its bearer token.",
            ),
            Self::NotOriginForm => (
                StatusCode::BAD_REQUEST,
                "Use an origin-form request target such as /v1/models.",
            ),
            Self::InvalidTarget => (StatusCode::BAD_REQUEST, "The request target is invalid."),
        };
        let mut response = plain(status, message);
        if status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

/// The bearer, the host of the client and every hop-by-hop header stay on this side.
fn outbound(
    state: &ProxyState,
    host: &HeaderValue,
    request: Request<Incoming>,
) -> Result<Request<Incoming>, Refusal> {
    if !authorized(request.headers(), &state.bearer) {
        return Err(Refusal::Unauthorized);
    }
    let target = origin_form(request.uri()).ok_or(Refusal::NotOriginForm)?;
    let authority = host.to_str().unwrap_or("localhost");
    let uri = format!("http://{authority}{target}")
        .parse::<Uri>()
        .map_err(|_| Refusal::InvalidTarget)?;
    let (mut parts, body) = request.into_parts();
    strip_hop_by_hop(&mut parts.headers);
    for name in [AUTHORIZATION, PROXY_AUTHORIZATION, EXPECT, HOST] {
        parts.headers.remove(name);
    }
    parts.headers.insert(HOST, host.clone());
    parts.uri = uri;
    parts.version = Version::HTTP_11;
    Ok(Request::from_parts(parts, body))
}

/// Errors of the hyper client name only their category; the cause chain says why.
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// `Bearer` is matched case-insensitively; the token in constant time.
fn authorized(headers: &HeaderMap, bearer: &str) -> bool {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let (Some(value), None) = (values.next(), values.next()) else {
        return false;
    };
    value
        .to_str()
        .ok()
        .and_then(|value| value.split_once(' '))
        .is_some_and(|(scheme, token)| {
            scheme.eq_ignore_ascii_case("bearer")
                && constant_time_eq::constant_time_eq(token.as_bytes(), bearer.as_bytes())
        })
}

pub(crate) fn origin_form(uri: &Uri) -> Option<&str> {
    if uri.scheme().is_some() || uri.authority().is_some() {
        return None;
    }
    uri.path_and_query()
        .map(|target| target.as_str())
        .filter(|target| target.starts_with('/'))
}

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let listed: Vec<HeaderName> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in listed.into_iter().chain(HOP_BY_HOP) {
        headers.remove(name);
    }
}

fn plain(status: StatusCode, message: impl Into<String>) -> Response<ProxyBody> {
    let mut response = Response::new(
        Full::new(Bytes::from(message.into()))
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = status;
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}
