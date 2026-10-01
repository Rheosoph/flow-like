#[path = "support/header_proxy.rs"]
mod header_proxy;
#[path = "support/tls.rs"]
mod tls_support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use flow_like_browser::BrowserError;
use flow_like_browser::transport::tls::{HttpProxy, client_config, http_client};
use flow_like_browser::transport::ws::{
    ConnectOptions, WsTransport, normalize_websocket_url, redact_url, validate_headers,
};
use flow_like_browser::transport::{
    Inbound, Outbound, Transport, TransportChannels, WriteStatus, WriteTicket,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::Instant;

const TOKEN: &str = "Bearer test-secret";
const RECEIVE_WAIT: Duration = Duration::from_secs(10);

fn outbound(text: &str) -> (Outbound, WriteTicket) {
    let ticket = WriteTicket::default();
    let message = Outbound {
        text: text.to_owned(),
        method: Arc::from("Test.send"),
        deadline: None,
        ticket: ticket.clone(),
    };
    (message, ticket)
}

fn send(channels: &TransportChannels, text: &str) -> WriteTicket {
    let (message, ticket) = outbound(text);
    let sent = channels.outbound.send(message);
    assert!(sent.is_ok(), "the transport writer is gone");
    ticket
}

async fn receive(channels: &mut TransportChannels) -> Inbound {
    tokio::time::timeout(RECEIVE_WAIT, channels.inbound.recv())
        .await
        .expect("an inbound frame within 10 s")
        .expect("the inbound channel is open")
}

async fn receive_text(channels: &mut TransportChannels) -> String {
    match receive(channels).await {
        Inbound::Text(text) => text,
        Inbound::InvalidUtf8(bytes) => panic!("expected text, got {} invalid bytes", bytes.len()),
        Inbound::Closed { reason } => panic!("expected text, the transport closed: {reason}"),
    }
}

async fn start(options: &ConnectOptions) -> TransportChannels {
    let transport = WsTransport::connect(options)
        .await
        .unwrap_or_else(|error| panic!("connect to {}: {error}", redact_url(&options.url)));
    Box::new(transport).start()
}

fn connect_message(error: BrowserError) -> String {
    match error {
        BrowserError::Connect { message } => message,
        other => panic!("expected a Connect error, got {other:?}"),
    }
}

async fn tls_proxy(ca: &tls_support::TestCa) -> header_proxy::ProxyHandle {
    let upstream = header_proxy::start_upstream().await;
    let tls = Some(tls_support::server_config(ca));
    header_proxy::start(upstream.url, Some(TOKEN.to_owned()), tls).await
}

fn authorized(url: &str, ca: &tls_support::TestCa) -> ConnectOptions {
    let mut options = ConnectOptions::new(url);
    options.headers = vec![("Authorization".to_owned(), TOKEN.to_owned())];
    options.extra_root_certificates = vec![ca.ca_der.clone()];
    options
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wss_with_an_extra_ca_passes_the_platform_verifier() {
    let ca = tls_support::test_ca();
    let proxy = tls_proxy(&ca).await;
    let mut channels = start(&authorized(&proxy.url, &ca)).await;
    let ticket = send(&channels, r#"{"id":1,"method":"Browser.getVersion"}"#);
    assert_eq!(
        receive_text(&mut channels).await,
        r#"{"id":1,"method":"Browser.getVersion"}"#
    );
    assert_eq!(ticket.status(), WriteStatus::Written);
    assert_eq!(proxy.transcript.lock().expect("transcript").len(), 2);
    channels.control.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsupplied_ca_is_reported_as_untrusted() {
    let ca = tls_support::test_ca();
    let other = tls_support::test_ca();
    let proxy = tls_proxy(&ca).await;
    let error = WsTransport::connect(&authorized(&proxy.url, &other))
        .await
        .err()
        .expect("a certificate from an unknown CA must be rejected");
    assert_eq!(
        connect_message(error),
        "TLS certificate of localhost is not trusted by this machine (platform roots plus 1 extra CA); add the issuing CA"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorization_decides_between_http_401_and_101() {
    let upstream = header_proxy::start_upstream().await;
    let proxy = header_proxy::start(upstream.url, Some(TOKEN.to_owned()), None).await;
    let error = WsTransport::connect(&ConnectOptions::new(&proxy.url))
        .await
        .err()
        .expect("a missing Authorization header must be rejected");
    let message = connect_message(error);
    assert!(message.contains("rejected: HTTP 401"), "{message}");

    let mut options = ConnectOptions::new(&proxy.url);
    options.headers = vec![("authorization".to_owned(), TOKEN.to_owned())];
    let mut channels = start(&options).await;
    send(&channels, "authorized");
    assert_eq!(receive_text(&mut channels).await, "authorized");
    channels.control.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_echoed_subprotocol_is_exposed() {
    let upstream = header_proxy::start_upstream().await;
    let proxy = header_proxy::start(upstream.url, None, None).await;
    let mut options = ConnectOptions::new(&proxy.url);
    options.subprotocols = vec!["cdp.test.v1".to_owned()];
    let transport = WsTransport::connect(&options)
        .await
        .expect("connect with a subprotocol");
    assert_eq!(transport.negotiated_subprotocol(), Some("cdp.test.v1"));
    assert_eq!(
        proxy
            .subprotocol_seen
            .lock()
            .expect("subprotocol")
            .as_deref(),
        Some("cdp.test.v1")
    );
    Box::new(transport).start().control.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_subprotocol_echo_fails_the_handshake() {
    let upstream = header_proxy::start_upstream().await;
    let mut options = ConnectOptions::new(&upstream.url);
    options.subprotocols = vec!["cdp.test.v1".to_owned(), "cdp.test.v2".to_owned()];
    let error = WsTransport::connect(&options)
        .await
        .err()
        .expect("the upstream never echoes a subprotocol");
    assert_eq!(
        connect_message(error),
        "Server did not accept subprotocol(s) cdp.test.v1, cdp.test.v2"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_twenty_mebibyte_text_frame_is_received_over_wss() {
    let ca = tls_support::test_ca();
    let proxy = tls_proxy(&ca).await;
    let mut channels = start(&authorized(&proxy.url, &ca)).await;
    send(&channels, "big");
    let text = receive_text(&mut channels).await;
    assert_eq!(text.len(), header_proxy::BIG_FRAME_BYTES);
    assert!(text.bytes().all(|byte| byte == b'x'));
    channels.control.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn binary_frames_are_decoded_as_text() {
    let upstream = header_proxy::start_upstream().await;
    let proxy = header_proxy::start(upstream.url, None, None).await;
    proxy.binary_frames.store(true, Ordering::SeqCst);
    let mut channels = start(&ConnectOptions::new(&proxy.url)).await;
    send(&channels, r#"{"method":"Page.loadEventFired"}"#);
    assert_eq!(
        receive_text(&mut channels).await,
        r#"{"method":"Page.loadEventFired"}"#
    );
    channels.control.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_invalid_utf8_binary_frame_arrives_as_invalid_utf8() {
    let upstream = header_proxy::start_upstream().await;
    let mut channels = start(&ConnectOptions::new(&upstream.url)).await;
    send(&channels, "invalid-utf8");
    match receive(&mut channels).await {
        Inbound::InvalidUtf8(bytes) => assert_eq!(bytes, header_proxy::INVALID_UTF8_FRAME),
        Inbound::Text(text) => panic!("expected invalid UTF-8, got text {text:?}"),
        Inbound::Closed { reason } => panic!("expected invalid UTF-8, closed: {reason}"),
    }
    channels.control.close().await;
}

fn header_error(name: &str, value: &str) -> String {
    let headers = [(name.to_owned(), value.to_owned())];
    match validate_headers(&headers) {
        Err(BrowserError::InvalidArgument { message }) => message,
        Err(other) => panic!("expected InvalidArgument for {name}, got {other:?}"),
        Ok(_) => panic!("header {name} must be rejected"),
    }
}

#[test]
fn invalid_headers_are_rejected_with_the_header_name() {
    let cases = [
        ("Host", "example.com", "'host'"),
        ("X-Api-Key", "secret\r\nInjected: yes", "'x-api-key'"),
        ("X-Api-Key", "sécret", "'x-api-key'"),
        ("Sec-WebSocket-Protocol", "cdp", "'sec-websocket-protocol'"),
        ("Bad Name", "secret", "\"Bad Name\""),
    ];
    for (name, value, expected) in cases {
        let message = header_error(name, value);
        assert!(message.contains(expected), "{message}");
        assert!(
            !message.contains("secret") && !message.contains("sécret"),
            "{message}"
        );
    }
}

#[test]
fn duplicate_headers_append() {
    let headers = [
        ("X-Trace".to_owned(), "a".to_owned()),
        ("x-trace".to_owned(), "b\tc".to_owned()),
    ];
    let map = validate_headers(&headers).expect("valid headers");
    let values: Vec<_> = map.get_all("x-trace").iter().collect();
    assert_eq!(values, ["a", "b\tc"]);
}

#[tokio::test]
async fn invalid_headers_fail_before_dialing() {
    let mut options = ConnectOptions::new("ws://127.0.0.1:9/devtools/browser");
    options.headers = vec![("Connection".to_owned(), "close".to_owned())];
    let error = WsTransport::connect(&options)
        .await
        .err()
        .expect("a managed header must be rejected");
    let BrowserError::InvalidArgument { message } = error else {
        panic!("expected InvalidArgument, got {error:?}");
    };
    assert!(message.contains("'connection'"), "{message}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_query_on_the_root_is_normalised() {
    let normalized = normalize_websocket_url("wss://cdp.example.com?token=abc").expect("valid");
    assert_eq!(normalized.as_str(), "wss://cdp.example.com/?token=abc");
    assert!(normalize_websocket_url("http://127.0.0.1:9222").is_err());

    let upstream = header_proxy::start_upstream().await;
    let root = upstream
        .url
        .trim_end_matches("/devtools/browser/upstream")
        .to_owned();
    let channels = start(&ConnectOptions::new(format!("{root}?token=abc"))).await;
    channels.control.close().await;
    let requests = upstream.requests.lock().expect("requests").clone();
    assert_eq!(requests, ["/?token=abc"]);
}

#[test]
fn urls_are_redacted() {
    assert_eq!(
        redact_url("wss://user:pw@cdp.example.com:9222/devtools/browser?token=abc&mode=x#frag"),
        "wss://cdp.example.com:9222/devtools/browser?token=***&mode=***"
    );
    assert_eq!(redact_url("ws://host/?opaque"), "ws://host/?***");
    let unparsed = redact_url("wss://user:pw@[bad/path?token=abc");
    assert_eq!(unparsed, "wss://[bad/path?***");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn errors_redact_query_values() {
    let upstream = header_proxy::start_upstream().await;
    let proxy = header_proxy::start(upstream.url, Some(TOKEN.to_owned()), None).await;
    let url = format!("{}?token=top-secret", proxy.url);
    let error = WsTransport::connect(&ConnectOptions::new(&url))
        .await
        .err()
        .expect("the proxy rejects a missing Authorization header");
    let message = connect_message(error);
    assert!(message.contains("?token=***"), "{message}");
    assert!(!message.contains("top-secret"), "{message}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_silent_server_times_the_handshake_out() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let hold = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(stream);
    });
    let mut options = ConnectOptions::new(format!("ws://{address}/devtools?token=abc"));
    options.handshake_timeout = Duration::from_millis(300);
    let error = WsTransport::connect(&options)
        .await
        .err()
        .expect("the handshake must time out");
    assert_eq!(
        connect_message(error),
        format!("WebSocket handshake to ws://{address}/devtools?token=*** timed out after 0.3 s")
    );
    hold.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_ws_works_with_the_rustls_connector() {
    let upstream = header_proxy::start_upstream().await;
    let mut channels = start(&ConnectOptions::new(&upstream.url)).await;
    send(&channels, "plain");
    assert_eq!(receive_text(&mut channels).await, "plain");
    assert_eq!(
        client_config(&[]).expect("TLS config").alpn_protocols,
        [b"http/1.1".to_vec()]
    );
    channels.control.close().await;
}

async fn https_responder(
    ca: &tls_support::TestCa,
) -> (u16, tokio::task::JoinHandle<Option<Vec<u8>>>) {
    let mut config = (*tls_support::server_config(ca)).clone();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("address").port();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.expect("accept");
        let mut tls = acceptor.accept(tcp).await.expect("TLS accept");
        let alpn = tls.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
        let mut head = Vec::new();
        let mut chunk = [0u8; 1024];
        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = tls.read(&mut chunk).await.expect("read the request");
            assert!(read > 0, "the client closed before sending a request");
            head.extend_from_slice(&chunk[..read]);
        }
        let response = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok";
        tls.write_all(response).await.expect("write the response");
        let _ = tls.shutdown().await;
        alpn
    });
    (port, server)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_http_client_trusts_extra_roots_and_offers_only_http_1_1() {
    let ca = tls_support::test_ca();
    let (port, server) = https_responder(&ca).await;
    let roots = std::slice::from_ref(&ca.ca_der);
    let client = http_client(roots, RECEIVE_WAIT, HttpProxy::None).expect("HTTP client");
    let body = client
        .get(format!("https://localhost:{port}/json/version"))
        .send()
        .await
        .expect("HTTPS request trusted through the extra CA")
        .text()
        .await
        .expect("response body");
    assert_eq!(body, "ok");
    let alpn = server.await.expect("the responder finished");
    assert_eq!(alpn.as_deref(), Some(b"http/1.1".as_slice()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_commands_are_not_written() {
    let upstream = header_proxy::start_upstream().await;
    let mut channels = start(&ConnectOptions::new(&upstream.url)).await;
    let (mut expired, expired_ticket) = outbound("expired");
    expired.deadline = Some(Instant::now() - Duration::from_millis(1));
    assert!(channels.outbound.send(expired).is_ok());
    let fresh = send(&channels, "fresh");
    assert_eq!(receive_text(&mut channels).await, "fresh");
    assert_eq!(expired_ticket.status(), WriteStatus::NotWritten);
    assert_eq!(fresh.status(), WriteStatus::Written);
    channels.control.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_peer_close_is_reported_once() {
    let upstream = header_proxy::start_upstream().await;
    let mut channels = start(&ConnectOptions::new(&upstream.url)).await;
    send(&channels, "close");
    match receive(&mut channels).await {
        Inbound::Closed { reason } => assert!(reason.contains("peer closed"), "{reason}"),
        _ => panic!("expected the close to be reported"),
    }
    let after = tokio::time::timeout(RECEIVE_WAIT, channels.inbound.recv())
        .await
        .expect("the inbound channel closes");
    assert!(after.is_none());
}

async fn stalled_peer() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("server handshake");
        tokio::time::sleep(Duration::from_secs(60)).await;
        drop(socket);
    });
    (format!("ws://{address}/devtools/browser/stalled"), peer)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_is_bounded_when_the_peer_stops_reading() {
    let (url, peer) = stalled_peer().await;
    let mut channels = start(&ConnectOptions::new(url)).await;
    let payload = "x".repeat(1 << 20);
    let tickets: Vec<_> = (0..64).map(|_| send(&channels, &payload)).collect();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        tickets
            .iter()
            .any(|ticket| ticket.status() != WriteStatus::Written),
        "the stalled peer must leave some of 64 MiB unwritten"
    );
    let started = Instant::now();
    channels.control.close().await;
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(2), "close took {elapsed:?}");
    match receive(&mut channels).await {
        Inbound::Closed { reason } => assert_eq!(reason, "closed by the client"),
        _ => panic!("expected the close to be reported"),
    }
    peer.abort();
}

async fn ping_listener() -> (String, tokio::sync::mpsc::UnboundedReceiver<usize>) {
    use futures_util::StreamExt;
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let (pings, received) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("server handshake");
        while let Some(Ok(message)) = socket.next().await {
            if let tokio_tungstenite::tungstenite::Message::Ping(payload) = message {
                let _ = pings.send(payload.len());
            }
        }
    });
    (format!("ws://{address}/devtools/browser/pings"), received)
}

#[tokio::test(flavor = "current_thread")]
async fn the_writer_pings_every_thirty_seconds() {
    let (url, mut pings) = ping_listener().await;
    let channels = start(&ConnectOptions::new(url)).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(29)).await;
    tokio::task::yield_now().await;
    assert!(pings.try_recv().is_err(), "no ping before 30 s");
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::time::resume();
    let ping = tokio::time::timeout(RECEIVE_WAIT, pings.recv())
        .await
        .expect("a ping after 30 s");
    assert_eq!(ping, Some(0));
    channels.control.close().await;
}
