//! Model gateway streams: the peer speaks HTTP/1.1 over the stream, and each request goes to
//! the agent's loopback gateway as the stream's principal. Only the method, the path, the
//! body and its content type leave the stream; the agent alone sets the principal headers.

use crate::{
    management::tunnel::GatewayTarget,
    models::gateway::{AGENT_SECRET_HEADER, PRINCIPAL_HEADER, principal_header},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::sync::{Arc, OnceLock};
use tokio::io::DuplexStream;

/// Bytes in flight between a stream and its HTTP server, each way.
pub(super) const PIPE_BYTES: usize = 256 * 1024;
/// The gateway's own request body limit.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const RELAYED_HEADERS: [header::HeaderName; 3] = [
    header::CONTENT_TYPE,
    header::RETRY_AFTER,
    header::CACHE_CONTROL,
];

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// One client for every stream, so requests reuse loopback connections to the gateway.
fn client() -> reqwest::Result<reqwest::Client> {
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .tcp_nodelay(true)
        .build()?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

/// The listener of one stream: it yields that stream once, then waits until dropped.
struct OneConnection(Option<DuplexStream>);

impl axum::serve::Listener for OneConnection {
    type Io = DuplexStream;
    type Addr = ();

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.0.take() {
            Some(io) => (io, ()),
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(())
    }
}

/// Serves the HTTP of one stream, keep-alive included, until the caller drops it.
pub(super) async fn serve(io: DuplexStream, target: GatewayTarget) {
    let router = Router::new().fallback(forward).with_state(Arc::new(target));
    if let Err(error) = axum::serve(OneConnection(Some(io)), router).await {
        tracing::debug!("A model gateway stream stopped serving HTTP: {error}");
    }
}

/// Why a request did not reach the gateway; the peer reads it as an API error.
struct Refused(StatusCode, String);

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        let Self(status, message) = self;
        let error = json!({"error": {"message": message, "type": "tunnel_error"}});
        (status, Json(error)).into_response()
    }
}

async fn forward(State(target): State<Arc<GatewayTarget>>, request: Request) -> Response {
    relay(&target, request)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn relay(target: &GatewayTarget, request: Request) -> Result<Response, Refused> {
    let (parts, body) = request.into_parts();
    if parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|length| length.to_str().ok()?.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
    {
        return Err(Refused(
            StatusCode::PAYLOAD_TOO_LARGE,
            "The request body exceeds 32 MiB".into(),
        ));
    }
    let principal = principal_header(&target.consumer)
        .and_then(|principal| HeaderValue::from_str(&principal).ok())
        .ok_or_else(|| {
            Refused(
                StatusCode::FORBIDDEN,
                "This stream has no gateway principal".into(),
            )
        })?;
    let mut secret = HeaderValue::from_str(&target.secret).map_err(|_| {
        Refused(
            StatusCode::BAD_GATEWAY,
            "The gateway secret is unusable".into(),
        )
    })?;
    secret.set_sensitive(true);
    let client = client().map_err(|error| {
        Refused(
            StatusCode::BAD_GATEWAY,
            format!("The model gateway client could not start: {error}"),
        )
    })?;
    let path = parts.uri.path_and_query().map_or("/", |path| path.as_str());
    let mut upstream = client
        .request(parts.method, format!("http://{}{path}", target.address))
        .header(AGENT_SECRET_HEADER, secret)
        .header(PRINCIPAL_HEADER, principal)
        // The gateway reserves its bounded body budget before reading these chunks.
        .body(reqwest::Body::wrap_stream(body.into_data_stream()));
    if let Some(length) = parts.headers.get(header::CONTENT_LENGTH) {
        upstream = upstream.header(header::CONTENT_LENGTH, length);
    }
    if let Some(content_type) = parts.headers.get(header::CONTENT_TYPE) {
        upstream = upstream.header(header::CONTENT_TYPE, content_type);
    }
    let answer = upstream.send().await.map_err(|error| {
        Refused(
            StatusCode::BAD_GATEWAY,
            format!("The model gateway did not answer: {error}"),
        )
    })?;
    Ok(streamed(answer))
}

/// The gateway's answer as it arrives, so server-sent events reach the peer token by token.
fn streamed(answer: reqwest::Response) -> Response {
    let mut response = Response::builder().status(answer.status());
    for name in RELAYED_HEADERS {
        if let Some(value) = answer.headers().get(&name) {
            response = response.header(name, value.clone());
        }
    }
    let chunks = futures_util::stream::unfold(Some(answer), |answer| async move {
        let mut answer = answer?;
        match answer.chunk().await {
            Ok(Some(chunk)) => Some((Ok(chunk), Some(answer))),
            Ok(None) => None,
            Err(error) => Some((Err(error), None)),
        }
    });
    response
        .body(Body::from_stream(chunks))
        .unwrap_or_else(|error| {
            let message = format!("The model gateway answer could not be relayed: {error}");
            Refused(StatusCode::BAD_GATEWAY, message).into_response()
        })
}

#[cfg(test)]
mod tests {
    use super::{
        super::{
            open_refusal,
            streams::{Event, Reset, Stream},
        },
        *,
    };
    use crate::{
        management::{RejectionCode, test_refusal, tunnel::OpenTarget},
        models::{
            engines::fake::{FakeBehaviour, FakeLauncher},
            fetch::{AddressPolicy, Fetcher},
            host::{HostConfig, HostParts, ModelHost},
        },
    };
    use axum::{body::Bytes, http::HeaderMap, routing::get};
    use flow_like_device_protocol::ModelConsumer;
    use serde_json::Value;
    use std::{future::Future, net::SocketAddr, time::Duration};
    use tokio::{
        net::TcpListener,
        sync::{Notify, mpsc},
    };
    use tokio_util::sync::CancellationToken;

    const WAIT: Duration = Duration::from_secs(10);

    async fn within<T>(request: impl Future<Output = reqwest::Result<T>>) -> T {
        tokio::time::timeout(WAIT, request)
            .await
            .expect("an answer in time")
            .expect("a successful request")
    }

    async fn loopback() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("a loopback listener");
        let address = listener.local_addr().expect("its address");
        (listener, address)
    }

    /// A loopback address whose one connection reaches `serve` through a pipe, as a
    /// tunnel stream does; a second connection is never accepted.
    async fn stream_to(target: GatewayTarget) -> SocketAddr {
        let (listener, address) = loopback().await;
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await?;
            let (mut client, server) = tokio::io::duplex(PIPE_BYTES);
            tokio::spawn(serve(server, target));
            tokio::io::copy_bidirectional(&mut socket, &mut client).await
        });
        address
    }

    fn header_text(headers: &HeaderMap, name: &str) -> Value {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map_or(Value::Null, |value| json!(value))
    }

    /// Answers what reached it: the headers that matter, the method, path and body.
    async fn echo(request: Request) -> Json<Value> {
        let (parts, body) = request.into_parts();
        let body = axum::body::to_bytes(body, 1024).await.unwrap_or_default();
        Json(json!({
            "method": parts.method.as_str(),
            "path": parts.uri.to_string(),
            "secret": header_text(&parts.headers, AGENT_SECRET_HEADER),
            "principal": header_text(&parts.headers, PRINCIPAL_HEADER),
            "authorization": header_text(&parts.headers, "authorization"),
            "other": header_text(&parts.headers, "x-flowlike-other"),
            "content_type": header_text(&parts.headers, "content-type"),
            "body": String::from_utf8_lossy(&body),
        }))
    }

    /// Two server-sent events; the second waits for `release`.
    fn events(release: Arc<Notify>) -> Response {
        let chunks = futures_util::stream::unfold(0, move |sent| {
            let release = Arc::clone(&release);
            async move {
                if sent == 1 {
                    release.notified().await;
                }
                let event = Bytes::from(format!("data: {}\n\n", sent + 1));
                (sent < 2).then_some((Ok::<_, std::io::Error>(event), sent + 1))
            }
        });
        let headers = [(header::CONTENT_TYPE, "text/event-stream")];
        (headers, Body::from_stream(chunks)).into_response()
    }

    async fn gateway(release: Arc<Notify>) -> SocketAddr {
        let router = Router::new()
            .route("/v1/events", get(move || async move { events(release) }))
            .fallback(echo);
        let (listener, address) = loopback().await;
        tokio::spawn(async move { axum::serve(listener, router).await });
        address
    }

    fn target(address: SocketAddr, consumer: ModelConsumer) -> GatewayTarget {
        GatewayTarget {
            address,
            secret: zeroize::Zeroizing::new("per-boot-secret".into()),
            consumer,
        }
    }

    #[tokio::test]
    async fn requests_carry_the_streams_principal_and_never_the_clients() {
        let gateway = gateway(Arc::new(Notify::new())).await;
        let grant = ModelConsumer::Grant {
            grant_id: "grant-1".into(),
        };
        let stream = stream_to(target(gateway, grant)).await;
        let http = reqwest::Client::new();
        let forged = http
            .post(format!("http://{stream}/v1/chat/completions?x=1"))
            .header(PRINCIPAL_HEADER, "owner")
            .header(AGENT_SECRET_HEADER, "guessed")
            .header("x-flowlike-other", "forged")
            .bearer_auth("stolen")
            .json(&json!({"model": "qwen"}));
        let echoed: Value = within(within(forged.send()).await.json()).await;
        assert_eq!(echoed["principal"], "grant:grant-1");
        assert_eq!(echoed["secret"], "per-boot-secret");
        for dropped in ["authorization", "other"] {
            assert_eq!(
                echoed[dropped],
                Value::Null,
                "{dropped} reached the gateway"
            );
        }
        assert_eq!(echoed["method"], "POST");
        assert_eq!(echoed["path"], "/v1/chat/completions?x=1");
        assert_eq!(echoed["content_type"], "application/json");
        assert_eq!(echoed["body"], r#"{"model":"qwen"}"#);
        let again = within(http.get(format!("http://{stream}/v1/models")).send()).await;
        let again: Value = within(again.json()).await;
        assert_eq!(again["method"], "GET", "a second request shares the stream");
    }

    #[tokio::test]
    async fn a_request_reaches_the_gateway_before_its_last_body_chunk() {
        use futures_util::StreamExt;
        let read_first = Arc::new(Notify::new());
        let server_read = Arc::clone(&read_first);
        let router = Router::new().fallback(move |request: Request| {
            let read = Arc::clone(&server_read);
            async move {
                let mut chunks = request.into_body().into_data_stream();
                assert_eq!(
                    chunks.next().await.unwrap().unwrap(),
                    Bytes::from_static(b"one")
                );
                read.notify_one();
                assert_eq!(
                    chunks.next().await.unwrap().unwrap(),
                    Bytes::from_static(b"two")
                );
                StatusCode::OK
            }
        });
        let (listener, address) = loopback().await;
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let stream = stream_to(target(address, ModelConsumer::Owner)).await;
        let chunks = futures_util::stream::unfold(0, move |index| {
            let read = Arc::clone(&read_first);
            async move {
                if index >= 2 {
                    return None;
                }
                if index == 1 {
                    read.notified().await;
                }
                let bytes = if index == 0 { b"one" } else { b"two" };
                Some((
                    Ok::<_, std::io::Error>(Bytes::from_static(bytes)),
                    index + 1,
                ))
            }
        });
        let response = within(
            reqwest::Client::new()
                .post(format!("http://{stream}/v1/chat/completions"))
                .body(reqwest::Body::wrap_stream(chunks))
                .send(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        server.abort();
    }

    #[tokio::test]
    async fn answers_stream_as_they_arrive() {
        let release = Arc::new(Notify::new());
        let gateway = gateway(Arc::clone(&release)).await;
        let stream = stream_to(target(gateway, ModelConsumer::Owner)).await;
        let mut response = within(reqwest::get(format!("http://{stream}/v1/events"))).await;
        let content_type = &response.headers()[header::CONTENT_TYPE.as_str()];
        assert_eq!(content_type, "text/event-stream");
        let first = within(response.chunk()).await;
        assert_eq!(first.as_deref(), Some(&b"data: 1\n\n"[..]));
        release.notify_one();
        let second = within(response.chunk()).await;
        assert_eq!(second.as_deref(), Some(&b"data: 2\n\n"[..]));
    }

    #[tokio::test]
    async fn an_unreachable_gateway_answers_bad_gateway() {
        let (closed, address) = loopback().await;
        drop(closed);
        let stream = stream_to(target(address, ModelConsumer::Owner)).await;
        let response = within(reqwest::get(format!("http://{stream}/v1/models"))).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body: Value = within(response.json()).await;
        assert_eq!(body["error"]["type"], "tunnel_error");
    }

    /// One request as a peer writes it into a stream, naming a principal and secret of its own.
    fn forged_request(body: &str) -> Vec<u8> {
        format!(
            "POST /v1/chat/completions HTTP/1.1\r\nhost: device\r\n{PRINCIPAL_HEADER}: owner\r\n{AGENT_SECRET_HEADER}: guessed\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// The body of an HTTP/1.1 answer once all of it arrived, framed by its length or, as the
    /// relayed stream is, by chunks.
    fn complete_body(answer: &[u8]) -> Option<Vec<u8>> {
        let head_end = answer.windows(4).position(|window| window == b"\r\n\r\n")? + 4;
        let head = String::from_utf8_lossy(&answer[..head_end]).to_ascii_lowercase();
        let body = &answer[head_end..];
        if head.contains("transfer-encoding: chunked") {
            return dechunked(body);
        }
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))?;
        let length = length.trim().parse::<usize>().ok()?;
        body.get(..length).map(<[u8]>::to_vec)
    }

    /// A chunked body once its last chunk arrived.
    fn dechunked(mut rest: &[u8]) -> Option<Vec<u8>> {
        let mut body = Vec::new();
        loop {
            let line_end = rest.windows(2).position(|window| window == b"\r\n")?;
            let size = std::str::from_utf8(&rest[..line_end]).ok()?;
            let size = usize::from_str_radix(size, 16).ok()?;
            let chunk = rest.get(line_end + 2..line_end + 4 + size)?;
            if size == 0 {
                return Some(body);
            }
            body.extend_from_slice(&chunk[..size]);
            rest = &rest[line_end + 4 + size..];
        }
    }

    async fn next_event(events: &mut mpsc::Receiver<Event>) -> Event {
        tokio::time::timeout(WAIT, events.recv())
            .await
            .expect("a stream event in time")
            .expect("a live stream task")
    }

    #[tokio::test]
    async fn a_gateway_stream_relays_http_as_its_grant_and_closes_after_the_peers_fin() {
        let gateway = gateway(Arc::new(Notify::new())).await;
        let grant = ModelConsumer::Grant {
            grant_id: "grant-1".into(),
        };
        let open = OpenTarget::ModelGateway(target(gateway, grant));
        let (events, mut received) = mpsc::channel(32);
        let cancel = CancellationToken::new();
        let mut stream = Stream::open(1, open, events, &cancel);
        assert!(matches!(next_event(&mut received).await, Event::Opened(1)));
        stream.opened = true;
        let request = forged_request(r#"{"model":"qwen"}"#);
        stream.write(request).expect("a request within the window");
        let (mut answer, mut finished) = (Vec::new(), false);
        loop {
            match next_event(&mut received).await {
                Event::Data(1, bytes) => answer.extend(bytes),
                Event::Fin(1) => finished = true,
                Event::Closed(1) => break,
                Event::Failed(_, reset) => panic!("the stream failed: {}", reset.code()),
                _ => {}
            }
            if !stream.received_fin && complete_body(&answer).is_some() {
                stream.finish_write().expect("the peer ends its requests");
            }
        }
        assert!(finished, "the stream sent Fin before it closed");
        let body = complete_body(&answer).expect("a complete answer");
        let echoed: Value = serde_json::from_slice(&body).expect("the echoed request");
        assert_eq!(echoed["principal"], "grant:grant-1");
        assert_eq!(echoed["secret"], "per-boot-secret");
        assert_eq!(echoed["body"], r#"{"model":"qwen"}"#);
    }

    /// The desktop connector reads `unauthorized` as "not granted" and `unsupported` as "this
    /// device serves no models"; other refusals of a data stream fail it.
    #[test]
    fn refused_opens_reset_unsupported_only_when_the_agent_lacks_the_target() {
        for (code, open, data) in [
            (RejectionCode::Unsupported, "unsupported", "unsupported"),
            (RejectionCode::Unauthorized, "unauthorized", "unauthorized"),
            (RejectionCode::Busy, "unauthorized", "stream_failed"),
        ] {
            let refused = test_refusal(code);
            assert_eq!(open_refusal(&refused).0, open, "{code:?}");
            assert_eq!(Reset::of(&refused).code(), data, "{code:?}");
        }
    }

    #[tokio::test]
    async fn the_host_gateway_admits_the_forwarded_principal_only_with_its_secret() {
        let directory = tempfile::tempdir().expect("a state directory");
        let parts = HostParts {
            fetcher: Fetcher::new(AddressPolicy::global_only()).expect("a fetcher"),
            runtime_source: None,
            launcher: Arc::new(FakeLauncher {
                behaviour: FakeBehaviour::default(),
            }),
        };
        let host = ModelHost::start(directory.path(), HostConfig::default(), parts)
            .await
            .expect("a model host");
        let owner = GatewayTarget {
            address: host.gateway().address(),
            secret: zeroize::Zeroizing::new(host.gateway().agent_secret().to_owned()),
            consumer: ModelConsumer::Owner,
        };
        let guessed = GatewayTarget {
            secret: zeroize::Zeroizing::new("guessed".into()),
            ..owner.clone()
        };
        let stream = stream_to(owner).await;
        let models = within(reqwest::get(format!("http://{stream}/v1/models"))).await;
        assert_eq!(models.status(), StatusCode::OK);
        assert_eq!(within(models.json::<Value>()).await["object"], "list");
        let stream = stream_to(guessed).await;
        let refused = within(reqwest::get(format!("http://{stream}/v1/models"))).await;
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
        host.shutdown().await;
    }
}
