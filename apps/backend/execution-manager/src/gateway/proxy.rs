use std::{
    convert::Infallible,
    io,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use futures_util::stream;
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::{
    HeaderMap, Method, Request, Response, StatusCode,
    body::{Frame, Incoming},
    header::{CONNECTION, CONTENT_LENGTH, TRANSFER_ENCODING},
    server::conn::http1,
    service::service_fn,
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioIo, TokioTimer},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{sleep, timeout},
};
use tokio_io_timeout::TimeoutStream;
use tokio_util::sync::CancellationToken;

use super::{
    BoxError, MAX_BODY, MAX_POLICY,
    policy::{Policy, PolicyData, now},
};

type Body = UnsyncBoxBody<Bytes, BoxError>;
type HttpClient = Client<HttpsConnector<HttpConnector>, Body>;
const IDLE: Duration = Duration::from_secs(30);

pub struct Gateway {
    policy: OnceLock<Arc<Policy>>,
    assigned: AtomicBool,
    pub revoked: CancellationToken,
    client: Mutex<Option<HttpClient>>,
    pub connections: Arc<Semaphore>,
    tunnels: Arc<Semaphore>,
}

impl Gateway {
    pub fn new() -> Result<Arc<Self>, BoxError> {
        let mut http = HttpConnector::new();
        http.enforce_http(false);
        http.set_connect_timeout(Some(Duration::from_secs(15)));
        http.set_nodelay(true);
        let https = HttpsConnectorBuilder::new()
            .with_provider_and_webpki_roots(Arc::new(rustls::crypto::ring::default_provider()))?
            .https_or_http()
            .enable_http1()
            .wrap_connector(http);
        let client = Client::builder(TokioExecutor::new())
            .pool_timer(TokioTimer::new())
            .http1_max_headers(100)
            .http1_max_buf_size(32768)
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(8)
            // A failed mutation must remain ambiguous to the caller. Retrying
            // an apparently idle connection could duplicate a callback or PUT.
            .retry_canceled_requests(false)
            .build(https);
        Ok(Arc::new(Self {
            policy: OnceLock::new(),
            assigned: AtomicBool::new(false),
            revoked: CancellationToken::new(),
            client: Mutex::new(Some(client)),
            connections: Arc::new(Semaphore::new(64)),
            tunnels: Arc::new(Semaphore::new(64)),
        }))
    }

    pub fn assign(self: &Arc<Self>, policy: Policy) -> Result<(), &'static str> {
        if self
            .assigned
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("Policy already assigned");
        }
        let lifetime = Duration::from_secs_f64((policy.data.deadline - now()).max(0.0));
        self.policy
            .set(Arc::new(policy))
            .map_err(|_| "Policy already assigned")?;
        let revoked = self.revoked.clone();
        let gateway = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::select! { _ = revoked.cancelled() => {}, _ = sleep(lifetime) => {
                if let Some(gateway) = gateway.upgrade() { gateway.revoke(); }
            } }
        });
        Ok(())
    }

    pub fn revoke(&self) {
        self.assigned.store(true, Ordering::Release);
        self.revoked.cancel();
        self.client
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }

    fn active_policy(&self) -> Result<Arc<Policy>, &'static str> {
        if self.revoked.is_cancelled() {
            return Err("Execution capability revoked");
        }
        let policy = self.policy.get().ok_or("No execution assigned")?;
        if now() >= policy.data.deadline {
            return Err("Execution capability expired");
        }
        Ok(policy.clone())
    }

    async fn proxy(
        self: Arc<Self>,
        mut request: Request<Incoming>,
    ) -> Result<Response<Body>, Infallible> {
        let response = async {
            validate_headers(request.headers(), MAX_BODY)?;
            let policy = self.active_policy()?;
            if request.method() == Method::CONNECT {
                let tunnel_permit = self.tunnels.clone().try_acquire_owned().map_err(|_| "Tunnel limit reached")?;
                let target = request.uri().authority().ok_or("CONNECT authority required")?.as_str();
                if request.uri().scheme().is_some() || request.uri().path_and_query().is_some() { return Err("Invalid CONNECT target".into()); }
                let address = policy.connect_destination(target).await?;
                let upstream = timeout(Duration::from_secs(15), TcpStream::connect(address)).await??;
                upstream.set_nodelay(true)?;
                let upgraded = hyper::upgrade::on(&mut request);
                let revoked = self.revoked.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = revoked.cancelled() => {},
                        _ = async {
                            if let Ok(Ok(client)) = timeout(Duration::from_secs(10), upgraded).await {
                                let _ = tunnel(TokioIo::new(client), upstream).await;
                            }
                        } => {},
                    }
                    drop(tunnel_permit);
                });
                let mut response = empty(StatusCode::OK, false);
                response.headers_mut().remove(CONTENT_LENGTH);
                return Ok(response);
            }
            policy.authorize(request.method(), request.uri(), request.headers())?;
            let (mut parts, body) = request.into_parts();
            strip_hop_headers(&mut parts.headers);
            let request = Request::from_parts(parts, bounded(body, MAX_BODY));
            let client = self.client.lock().unwrap_or_else(|error| error.into_inner()).clone().ok_or("Execution capability revoked")?;
            let response = timeout(IDLE, client.request(request)).await??;
            let (mut parts, body) = response.into_parts();
            if declared_length(&parts.headers)?.is_some_and(|length| length > MAX_BODY) {
                return Err("Upstream response exceeds limit".into());
            }
            strip_hop_headers(&mut parts.headers);
            parts.headers.insert(CONNECTION, "close".parse().unwrap());
            Ok::<_, BoxError>(Response::from_parts(parts, bounded(body, MAX_BODY)))
        }.await;
        Ok(response.unwrap_or_else(|_| empty(StatusCode::FORBIDDEN, true)))
    }

    pub async fn serve_proxy<I>(self: Arc<Self>, io: I, permit: OwnedSemaphorePermit)
    where
        I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let mut io = TimeoutStream::new(io);
        io.set_read_timeout(Some(IDLE));
        io.set_write_timeout(Some(IDLE));
        let connection = http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(Duration::from_secs(10))
            .max_headers(100)
            .max_buf_size(32768)
            .half_close(true)
            .keep_alive(false)
            .serve_connection(
                TokioIo::new(Box::pin(io)),
                service_fn({
                    let gateway = self.clone();
                    move |request| gateway.clone().proxy(request)
                }),
            )
            .with_upgrades();
        tokio::select! { _ = self.revoked.cancelled() => {}, _ = connection => {} }
        drop(permit);
    }

    pub async fn serve_control<I>(self: Arc<Self>, io: I, token: Arc<String>, max_duration: u64)
    where
        I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let mut io = TimeoutStream::new(io);
        io.set_read_timeout(Some(Duration::from_secs(10)));
        io.set_write_timeout(Some(Duration::from_secs(10)));
        let connection = http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(Duration::from_secs(10))
            .max_headers(32)
            .max_buf_size(16384)
            .keep_alive(false)
            .serve_connection(
                TokioIo::new(Box::pin(io)),
                service_fn(move |request| {
                    self.clone().control(request, token.clone(), max_duration)
                }),
            );
        let _ = timeout(Duration::from_secs(15), connection).await;
    }

    async fn control(
        self: Arc<Self>,
        request: Request<Incoming>,
        token: Arc<String>,
        max_duration: u64,
    ) -> Result<Response<Body>, Infallible> {
        let status = async {
            if request.method() == Method::GET {
                return if matches!(request.uri().path(), "/health" | "/ready")
                    && request.uri().query().is_none()
                {
                    StatusCode::OK
                } else {
                    StatusCode::NOT_FOUND
                };
            }
            if request.method() != Method::POST {
                return StatusCode::METHOD_NOT_ALLOWED;
            }
            if request.headers().get_all("x-gateway-token").iter().count() != 1
                || !constant_time_eq::constant_time_eq(
                    request
                        .headers()
                        .get("x-gateway-token")
                        .map(|v| v.as_bytes())
                        .unwrap_or_default(),
                    token.as_bytes(),
                )
            {
                return StatusCode::FORBIDDEN;
            }
            if request.uri().query().is_some() {
                return StatusCode::NOT_FOUND;
            }
            if request.uri().path() == "/revoke" {
                self.revoke();
                return StatusCode::OK;
            }
            if request.uri().path() != "/configure" {
                return StatusCode::NOT_FOUND;
            }
            if validate_headers(request.headers(), MAX_POLICY as u64).is_err()
                || request.headers().contains_key(TRANSFER_ENCODING)
                || !matches!(declared_length(request.headers()), Ok(Some(1..)))
            {
                return StatusCode::BAD_REQUEST;
            }
            let body = match timeout(
                Duration::from_secs(10),
                bounded(request.into_body(), MAX_POLICY as u64).collect(),
            )
            .await
            {
                Ok(Ok(body)) => body.to_bytes(),
                _ => return StatusCode::BAD_REQUEST,
            };
            let data: PolicyData = match serde_json::from_slice(&body) {
                Ok(data) => data,
                Err(_) => return StatusCode::BAD_REQUEST,
            };
            if data.deadline > now() + max_duration as f64 {
                return StatusCode::BAD_REQUEST;
            }
            let policy = match Policy::new(data) {
                Ok(policy) => policy,
                Err(_) => return StatusCode::BAD_REQUEST,
            };
            if self.assign(policy).is_err() {
                StatusCode::CONFLICT
            } else {
                StatusCode::NO_CONTENT
            }
        }
        .await;
        Ok(empty(status, true))
    }
}

fn empty(status: StatusCode, close: bool) -> Response<Body> {
    let mut response = Response::new(
        Full::new(Bytes::new())
            .map_err(|never| match never {})
            .boxed_unsync(),
    );
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_LENGTH, "0".parse().unwrap());
    if close {
        response
            .headers_mut()
            .insert(CONNECTION, "close".parse().unwrap());
    }
    response
}

fn declared_length(headers: &HeaderMap) -> Result<Option<u64>, BoxError> {
    if headers.get_all(CONTENT_LENGTH).iter().count() > 1 {
        return Err("Duplicate content length".into());
    }
    headers
        .get(CONTENT_LENGTH)
        .map(|value| {
            let value = value.to_str()?;
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Invalid content length".into());
            }
            Ok(value.parse()?)
        })
        .transpose()
}

fn validate_headers(headers: &HeaderMap, max_body: u64) -> Result<(), BoxError> {
    for name in [
        "host",
        "authorization",
        "content-length",
        "transfer-encoding",
    ] {
        if headers.get_all(name).iter().count() > 1 {
            return Err("Duplicate framing or capability header".into());
        }
    }
    let length = declared_length(headers)?;
    if length.is_some_and(|length| length > max_body) {
        return Err("Request body exceeds limit".into());
    }
    if let Some(transfer) = headers.get(TRANSFER_ENCODING)
        && (length.is_some() || !transfer.as_bytes().eq_ignore_ascii_case(b"chunked"))
    {
        return Err("Ambiguous request framing".into());
    }
    if headers.contains_key("trailer") || headers.contains_key("upgrade") {
        return Err("Unsupported request framing".into());
    }
    Ok(())
}

fn strip_hop_headers(headers: &mut HeaderMap) {
    let nominated: Vec<_> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_owned())
        .collect();
    for name in nominated {
        headers.remove(name);
    }
    for name in [
        "connection",
        "proxy-connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

fn bounded<B>(body: B, max: u64) -> Body
where
    B: http_body::Body<Data = Bytes> + Send + Unpin + 'static,
    B::Error: Into<BoxError>,
{
    if body.is_end_stream() {
        return Full::new(Bytes::new())
            .map_err(|never| match never {})
            .boxed_unsync();
    }
    let frames = stream::try_unfold((body, 0_u64), move |(mut body, total)| async move {
        match timeout(IDLE, body.frame()).await? {
            None => Ok(None),
            Some(Err(error)) => Err(error.into()),
            Some(Ok(frame)) => {
                if frame.is_trailers() {
                    return Err("HTTP trailers are unsupported".into());
                }
                let data = frame.into_data().map_err(|_| "Unsupported body frame")?;
                let total = total
                    .checked_add(data.len() as u64)
                    .ok_or("Body exceeds limit")?;
                if total > max {
                    return Err("Body exceeds limit".into());
                }
                Ok(Some((Frame::data(data), (body, total))))
            }
        }
    });
    StreamBody::new(frames).boxed_unsync()
}

async fn tunnel<A, B>(mut client: A, mut upstream: B) -> io::Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let mut client_buf = [0_u8; 32768];
    let mut upstream_buf = [0_u8; 32768];
    let mut total = 0_u64;
    loop {
        tokio::select! {
            read = client.read(&mut client_buf) => {
                let count = read?; if count == 0 { return Ok(()); }
                total += count as u64; if total > MAX_BODY { return Err(io::Error::other("Tunnel exceeds limit")); }
                timeout(IDLE, upstream.write_all(&client_buf[..count])).await??;
            },
            read = upstream.read(&mut upstream_buf) => {
                let count = read?; if count == 0 { return Ok(()); }
                total += count as u64; if total > MAX_BODY { return Err(io::Error::other("Tunnel exceeds limit")); }
                timeout(IDLE, client.write_all(&upstream_buf[..count])).await??;
            },
            _ = sleep(IDLE) => return Err(io::Error::new(io::ErrorKind::TimedOut, "Tunnel idle timeout")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::net::{TcpListener, UnixListener, UnixStream};

    fn policy_data(origin: &str) -> PolicyData {
        PolicyData {
            callback_url: "http://callback:8080".into(),
            object_store_url: origin.into(),
            app_id: "app".into(),
            run_id: "run".into(),
            executor_jwt: "jwt".into(),
            deadline: now() + 60.0,
            buckets: vec!["bucket".into()],
            allowed_https_hosts: vec![],
            object_store_tls_gateway: false,
        }
    }

    async fn raw<I: AsyncRead + AsyncWrite + Unpin>(mut stream: I, request: &str) -> String {
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        timeout(Duration::from_secs(3), stream.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        String::from_utf8(response).unwrap()
    }

    async fn control(gateway: Arc<Gateway>) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                tokio::spawn(
                    gateway
                        .clone()
                        .serve_control(stream, Arc::new("g".repeat(64)), 3600),
                );
            }
        });
        (address, task)
    }

    async fn post(
        address: std::net::SocketAddr,
        path: &str,
        token: &str,
        data: &PolicyData,
    ) -> String {
        let body = serde_json::to_string(data).unwrap();
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {address}\r\nX-Gateway-Token: {token}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        raw(TcpStream::connect(address).await.unwrap(), &request).await
    }

    #[test]
    fn framing_and_hop_headers_cannot_change_authority() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_LENGTH, "3".parse().unwrap());
        headers.insert(TRANSFER_ENCODING, "chunked".parse().unwrap());
        assert!(validate_headers(&headers, MAX_BODY).is_err());
        headers.remove(TRANSFER_ENCODING);
        headers.append("authorization", "a".parse().unwrap());
        headers.append("authorization", "b".parse().unwrap());
        assert!(validate_headers(&headers, MAX_BODY).is_err());
        headers.clear();
        headers.insert(CONNECTION, "keep-alive, X-Private".parse().unwrap());
        headers.insert("x-private", "secret".parse().unwrap());
        headers.insert("x-public", "ok".parse().unwrap());
        strip_hop_headers(&mut headers);
        assert!(!headers.contains_key("x-private"));
        assert!(headers.contains_key("x-public"));
    }

    #[tokio::test]
    async fn streaming_body_limits_count_all_frames_and_reject_trailers() {
        let chunks = || {
            StreamBody::new(stream::iter([
                Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"ab"))),
                Ok(Frame::data(Bytes::from_static(b"cd"))),
            ]))
        };
        assert!(bounded(chunks(), 3).collect().await.is_err());
        assert_eq!(
            bounded(chunks(), 4).collect().await.unwrap().to_bytes(),
            "abcd"
        );
        let trailers = StreamBody::new(stream::iter([
            Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"a"))),
            Ok(Frame::trailers(HeaderMap::new())),
        ]));
        assert!(bounded(trailers, 10).collect().await.is_err());
    }

    #[tokio::test]
    async fn revoke_before_assignment_is_permanent() {
        let gateway = Gateway::new().unwrap();
        assert!(gateway.active_policy().is_err());
        gateway.revoke();
        let data = PolicyData {
            callback_url: "http://callback:8080".into(),
            object_store_url: "http://objects:9000".into(),
            app_id: "app".into(),
            run_id: "run".into(),
            executor_jwt: "jwt".into(),
            deadline: now() + 60.0,
            buckets: vec!["bucket".into()],
            allowed_https_hosts: vec![],
            object_store_tls_gateway: false,
        };
        assert!(gateway.assign(Policy::new(data).unwrap()).is_err());
        assert!(gateway.active_policy().is_err());
    }

    #[tokio::test]
    async fn execution_deadline_revokes_policy_and_drops_idle_pool() {
        let gateway = Gateway::new().unwrap();
        let mut data = policy_data("http://objects:9000");
        data.deadline = now() + 0.05;
        gateway.assign(Policy::new(data).unwrap()).unwrap();
        timeout(Duration::from_secs(1), gateway.revoked.cancelled())
            .await
            .unwrap();
        assert!(gateway.active_policy().is_err());
        assert!(gateway.client.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn real_unix_proxy_reuses_upstream_without_granting_admin_access() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", upstream.local_addr().unwrap());
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let upstream_task = tokio::spawn(async move {
            loop {
                let (stream, _) = upstream.accept().await.unwrap();
                count.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(async move {
                    let _ = http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(|request: Request<Incoming>| async move {
                                assert!(!request.headers().contains_key("x-strip"));
                                assert!(
                                    !request.headers().contains_key(TRANSFER_ENCODING),
                                    "An empty GET must stay empty"
                                );
                                let response =
                                    Response::new(Full::new(Bytes::from_static(b"object")));
                                Ok::<_, Infallible>(response)
                            }),
                        )
                        .await;
                });
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proxy.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let gateway = Gateway::new().unwrap();
        gateway
            .assign(Policy::new(policy_data(&origin)).unwrap())
            .unwrap();
        let gateway_task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let permit = gateway.connections.clone().acquire_owned().await.unwrap();
                tokio::spawn(gateway.clone().serve_proxy(stream, permit));
            }
        });
        for (path_part, status) in [
            ("/bucket/key", "200"),
            ("/bucket/second", "200"),
            ("/?Action=AssumeRole", "403"),
            ("/bucket?policy", "403"),
            ("/private/key", "403"),
        ] {
            let request = format!(
                "GET {origin}{path_part} HTTP/1.1\r\nHost: {}\r\nConnection: close, X-Strip\r\nX-Strip: private\r\n\r\n",
                origin.trim_start_matches("http://")
            );
            let response = raw(UnixStream::connect(&path).await.unwrap(), &request).await;
            assert!(
                response.starts_with(&format!("HTTP/1.1 {status}")),
                "{response}"
            );
            if status == "200" {
                assert!(response.contains("object"));
            }
        }
        assert_eq!(
            connections.load(Ordering::Relaxed),
            1,
            "Fully consumed responses should return their connection to this run's pool"
        );
        gateway_task.abort();
        upstream_task.abort();
    }

    #[tokio::test]
    async fn real_tcp_control_assigns_once_and_revokes_an_active_tunnel() {
        let gateway = Gateway::new().unwrap();
        let (control_address, control_task) = control(gateway.clone()).await;
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy_listener.local_addr().unwrap();
        let service = gateway.clone();
        let proxy_task = tokio::spawn(async move {
            loop {
                let (stream, _) = proxy_listener.accept().await.unwrap();
                let permit = service.connections.clone().acquire_owned().await.unwrap();
                tokio::spawn(service.clone().serve_proxy(stream, permit));
            }
        });
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_address = upstream.local_addr().unwrap();
        let mut data = policy_data(&format!("https://{upstream_address}"));
        data.object_store_tls_gateway = true;
        assert!(
            post(control_address, "/configure", "wrong", &data)
                .await
                .starts_with("HTTP/1.1 403")
        );
        assert!(gateway.active_policy().is_err());
        assert!(
            post(control_address, "/configure", &"g".repeat(64), &data)
                .await
                .starts_with("HTTP/1.1 204")
        );
        assert!(
            post(control_address, "/configure", &"g".repeat(64), &data)
                .await
                .starts_with("HTTP/1.1 409")
        );
        let mut client = TcpStream::connect(proxy_address).await.unwrap();
        client
            .write_all(
                format!("CONNECT {upstream_address} HTTP/1.1\r\nHost: {upstream_address}\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let (mut peer, _) = timeout(Duration::from_secs(3), upstream.accept())
            .await
            .unwrap()
            .unwrap();
        let mut response = vec![];
        timeout(Duration::from_secs(3), async {
            loop {
                response.push(client.read_u8().await.unwrap());
                if response.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(
            String::from_utf8(response)
                .unwrap()
                .starts_with("HTTP/1.1 200")
        );
        client.write_all(b"ping").await.unwrap();
        let mut ping = [0; 4];
        timeout(Duration::from_secs(3), peer.read_exact(&mut ping))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&ping, b"ping");
        assert!(
            post(control_address, "/revoke", &"g".repeat(64), &data)
                .await
                .starts_with("HTTP/1.1 200")
        );
        let mut closed = [0; 1];
        assert_eq!(
            timeout(Duration::from_secs(3), client.read(&mut closed))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(
            post(control_address, "/configure", &"g".repeat(64), &data)
                .await
                .starts_with("HTTP/1.1 409")
        );
        assert!(gateway.active_policy().is_err());
        proxy_task.abort();
        control_task.abort();
    }
}
