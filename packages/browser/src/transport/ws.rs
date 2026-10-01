// Derived from agent-browser cli/src/native/cdp/client.rs @d01253d, Copyright 2025 Vercel Inc., Apache-2.0; modified by Rheosoph GmbH. See NOTICE.
use std::net::Shutdown;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rustls::CertificateError;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::{Instant, Interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL;
use tokio_tungstenite::tungstenite::http::{HeaderMap, HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Bytes, Error as WsError, Message};
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};
use url::{Host, Url};

use super::{
    Inbound, Outbound, Transport, TransportChannels, TransportControl, TransportShutdown,
    WriteStatus,
};
use crate::BrowserError;

const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const CLOSE_FLUSH: Duration = Duration::from_millis(500);
const CLOSE_TOTAL: Duration = Duration::from_secs(1);
const MANAGED_HEADERS: [&str; 9] = [
    "host",
    "connection",
    "upgrade",
    "content-length",
    "transfer-encoding",
    "sec-websocket-key",
    "sec-websocket-version",
    "sec-websocket-extensions",
    "sec-websocket-accept",
];
const TOKEN_SYMBOLS: &[u8] = b"!#$%&'*+-.^_`|~";

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Debug)]
pub struct ConnectOptions {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub subprotocols: Vec<String>,
    pub extra_root_certificates: Vec<Vec<u8>>,
    pub handshake_timeout: std::time::Duration,
}

impl ConnectOptions {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            subprotocols: Vec::new(),
            extra_root_certificates: Vec::new(),
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }
}

pub struct WsTransport {
    socket: WsStream,
    shutdown: socket2::Socket,
    subprotocol: Option<String>,
    label: String,
}

impl WsTransport {
    pub async fn connect(options: &ConnectOptions) -> crate::Result<WsTransport> {
        let url = normalize_websocket_url(&options.url)?;
        let request = build_request(&url, options)?;
        let config = super::tls::client_config(&options.extra_root_certificates)?;
        let attempt = Handshake { url: &url, options };
        let opened = tokio::time::timeout(options.handshake_timeout, attempt.open(request, config))
            .await
            .unwrap_or(Err(HandshakeFailure::TimedOut));
        let (socket, shutdown, subprotocol) = opened.map_err(|failure| attempt.error(failure))?;
        let label = redact_url(url.as_str());
        tracing::debug!("WebSocket connected to {label}");
        Ok(WsTransport {
            socket,
            shutdown,
            subprotocol,
            label,
        })
    }

    pub fn negotiated_subprotocol(&self) -> Option<&str> {
        self.subprotocol.as_deref()
    }
}

impl Transport for WsTransport {
    fn start(self: Box<Self>) -> TransportChannels {
        let Self {
            socket,
            shutdown,
            label,
            ..
        } = *self;
        let (inbound_tx, inbound) = unbounded_channel();
        let (outbound, outbound_rx) = unbounded_channel();
        let (closing, _) = watch::channel(false);
        let link = Arc::new(Link {
            inlet: Mutex::new(Some(inbound_tx)),
            socket: shutdown,
            closing,
            label,
        });
        let (sink, stream) = socket.split();
        let reader = tokio::spawn(read_loop(stream, link.clone())).abort_handle();
        let writer = tokio::spawn(write_loop(sink, outbound_rx, keepalive(), link.clone()));
        let control = WsShutdown {
            link,
            reader,
            writer: Mutex::new(Some(writer)),
        };
        TransportChannels {
            inbound,
            outbound,
            control: TransportControl::new(Arc::new(control)),
        }
    }
}

pub fn validate_headers(
    headers: &[(String, String)],
) -> crate::Result<tokio_tungstenite::tungstenite::http::HeaderMap> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let (name, value) = validate_header(name, value)?;
        map.append(name, value);
    }
    Ok(map)
}

pub fn normalize_websocket_url(url: &str) -> crate::Result<url::Url> {
    let mut parsed = Url::parse(url).map_err(|error| {
        invalid_argument(format!(
            "Invalid WebSocket URL {}: {error}",
            redact_url(url)
        ))
    })?;
    if !matches!(parsed.scheme(), "ws" | "wss") {
        return Err(invalid_argument(format!(
            "WebSocket URL {} must start with ws:// or wss://",
            redact_url(url)
        )));
    }
    parsed.set_fragment(None);
    Ok(parsed)
}

pub fn redact_url(url: &str) -> String {
    match Url::parse(url) {
        Ok(parsed) => redact_parsed(parsed),
        Err(_) => redact_unparsed(url),
    }
}

fn redact_parsed(mut url: Url) -> String {
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    if let Some(query) = url.query().map(redact_query) {
        url.set_query(Some(&query));
    }
    url.into()
}

fn redact_query(query: &str) -> String {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, _)) => format!("{key}=***"),
            None => "***".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn redact_unparsed(url: &str) -> String {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    let (base, tail) = url.split_at(end);
    let authority_start = base.find("://").map_or(0, |index| index + 3);
    let authority_end = base[authority_start..]
        .find('/')
        .map_or(base.len(), |index| authority_start + index);
    let host_start = base[authority_start..authority_end]
        .rfind('@')
        .map_or(authority_start, |index| authority_start + index + 1);
    let suffix = if tail.is_empty() { "" } else { "?***" };
    format!(
        "{}{}{suffix}",
        &base[..authority_start],
        &base[host_start..]
    )
}

fn validate_header(name: &str, value: &str) -> crate::Result<(HeaderName, HeaderValue)> {
    let header = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
        invalid_argument(format!(
            "WebSocket header name {name:?} is not a valid HTTP header name"
        ))
    })?;
    if header == SEC_WEBSOCKET_PROTOCOL {
        return Err(invalid_argument(format!(
            "WebSocket header '{header}' cannot be set directly; pass the subprotocols separately"
        )));
    }
    if MANAGED_HEADERS.contains(&header.as_str()) {
        return Err(invalid_argument(format!(
            "WebSocket header '{header}' is set by the WebSocket handshake and cannot be overridden"
        )));
    }
    let invalid_value = || {
        invalid_argument(format!(
            "WebSocket header '{header}' has an invalid value: only visible ASCII characters, spaces and tabs are allowed"
        ))
    };
    if !value.bytes().all(is_header_value_byte) {
        return Err(invalid_value());
    }
    let value = HeaderValue::from_str(value).map_err(|_| invalid_value())?;
    Ok((header, value))
}

fn is_header_value_byte(byte: u8) -> bool {
    byte == b'\t' || (b' '..=b'~').contains(&byte)
}

fn subprotocol_header(subprotocols: &[String]) -> crate::Result<Option<HeaderValue>> {
    if subprotocols.is_empty() {
        return Ok(None);
    }
    if let Some(index) = subprotocols.iter().position(|protocol| !is_token(protocol)) {
        return Err(invalid_argument(format!(
            "WebSocket subprotocol #{} is not a valid HTTP token (RFC 9110 token characters only)",
            index + 1
        )));
    }
    HeaderValue::from_str(&subprotocols.join(", "))
        .map(Some)
        .map_err(|error| invalid_argument(format!("Invalid WebSocket subprotocol list: {error}")))
}

fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || TOKEN_SYMBOLS.contains(&byte))
}

fn build_request(url: &Url, options: &ConnectOptions) -> crate::Result<Request> {
    let headers = validate_headers(&options.headers)?;
    let subprotocols = subprotocol_header(&options.subprotocols)?;
    let mut request = url.as_str().into_client_request().map_err(|error| {
        invalid_argument(format!(
            "Invalid WebSocket URL {}: {error}",
            redact_url(url.as_str())
        ))
    })?;
    let target = request.headers_mut();
    for (name, value) in &headers {
        target.append(name, value.clone());
    }
    if let Some(subprotocols) = subprotocols {
        target.insert(SEC_WEBSOCKET_PROTOCOL, subprotocols);
    }
    Ok(request)
}

fn invalid_argument(message: String) -> BrowserError {
    BrowserError::InvalidArgument { message }
}

fn connect_error(message: String) -> BrowserError {
    BrowserError::Connect { message }
}

enum HandshakeFailure {
    Address,
    Dial(std::io::Error),
    Upgrade(WsError),
    TimedOut,
}

struct Handshake<'a> {
    url: &'a Url,
    options: &'a ConnectOptions,
}

type Opened = (WsStream, socket2::Socket, Option<String>);

impl Handshake<'_> {
    async fn open(
        &self,
        request: Request,
        config: Arc<rustls::ClientConfig>,
    ) -> Result<Opened, HandshakeFailure> {
        let (host, port) = dial_target(self.url).ok_or(HandshakeFailure::Address)?;
        let (stream, shutdown) = super::dial::dial(&host, port, self.options.handshake_timeout)
            .await
            .map_err(HandshakeFailure::Dial)?;
        let websocket_config = WebSocketConfig::default()
            .max_message_size(None)
            .max_frame_size(None);
        let (socket, response) = tokio_tungstenite::client_async_tls_with_config(
            request,
            stream,
            Some(websocket_config),
            Some(Connector::Rustls(config)),
        )
        .await
        .map_err(HandshakeFailure::Upgrade)?;
        let subprotocol = response
            .headers()
            .get(SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        Ok((socket, shutdown, subprotocol))
    }

    fn error(&self, failure: HandshakeFailure) -> BrowserError {
        let redacted = redact_url(self.url.as_str());
        match failure {
            HandshakeFailure::Address => {
                connect_error(format!("WebSocket URL {redacted} has no host or port"))
            }
            HandshakeFailure::Dial(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                self.timed_out(&redacted)
            }
            HandshakeFailure::Dial(error) => {
                connect_error(format!("Could not connect to {redacted}: {error}"))
            }
            HandshakeFailure::TimedOut => self.timed_out(&redacted),
            HandshakeFailure::Upgrade(error) => self.upgrade_error(&redacted, error),
        }
    }

    fn timed_out(&self, redacted: &str) -> BrowserError {
        connect_error(format!(
            "WebSocket handshake to {redacted} timed out after {} s",
            self.options.handshake_timeout.as_secs_f64()
        ))
    }

    fn upgrade_error(&self, redacted: &str, error: WsError) -> BrowserError {
        use tokio_tungstenite::tungstenite::error::{ProtocolError, SubProtocolError};
        match error {
            WsError::Http(response) => connect_error(format!(
                "WebSocket handshake to {redacted} rejected: HTTP {}",
                response.status()
            )),
            WsError::Io(error) => self.io_error(redacted, &error),
            WsError::Protocol(ProtocolError::SecWebSocketSubProtocolError(
                SubProtocolError::ServerSentSubProtocolNoneRequested,
            )) => connect_error(format!(
                "WebSocket handshake to {redacted} failed: the server selected a subprotocol although none was requested"
            )),
            WsError::Protocol(ProtocolError::SecWebSocketSubProtocolError(_)) => {
                connect_error(format!(
                    "Server did not accept subprotocol(s) {}",
                    self.options.subprotocols.join(", ")
                ))
            }
            error => connect_error(format!("WebSocket handshake to {redacted} failed: {error}")),
        }
    }

    fn io_error(&self, redacted: &str, error: &std::io::Error) -> BrowserError {
        let host = self.url.host_str().unwrap_or_default();
        match error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        {
            Some(tls) if is_untrusted_issuer(tls) => connect_error(format!(
                "TLS certificate of {host} is not trusted by this machine (platform roots plus {} extra CA); add the issuing CA",
                self.options.extra_root_certificates.len()
            )),
            Some(tls) => connect_error(format!("TLS handshake with {host} failed: {tls}")),
            None => connect_error(format!("WebSocket handshake to {redacted} failed: {error}")),
        }
    }
}

fn dial_target(url: &Url) -> Option<(String, u16)> {
    let host = match url.host()? {
        Host::Domain(domain) => domain.to_owned(),
        Host::Ipv4(address) => address.to_string(),
        Host::Ipv6(address) => address.to_string(),
    };
    Some((host, url.port_or_known_default()?))
}

fn is_untrusted_issuer(error: &rustls::Error) -> bool {
    match error {
        rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer) => true,
        // Security.framework reports an untrusted anchor as errSecNotTrusted (-67843), which
        // rustls-platform-verifier passes through as a description ending in the status code.
        rustls::Error::InvalidCertificate(CertificateError::Other(other)) => {
            other.to_string().ends_with(": -67843")
        }
        _ => false,
    }
}

struct Link {
    inlet: Mutex<Option<UnboundedSender<Inbound>>>,
    socket: socket2::Socket,
    closing: watch::Sender<bool>,
    label: String,
}

impl Link {
    fn deliver(&self, inbound: Inbound) -> bool {
        let guard = self.inlet.lock().unwrap_or_else(PoisonError::into_inner);
        guard
            .as_ref()
            .is_some_and(|sender| sender.send(inbound).is_ok())
    }

    fn finish(&self, reason: String) {
        let sender = self
            .inlet
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(sender) = sender {
            tracing::debug!("WebSocket to {} closed: {reason}", self.label);
            let _ = sender.send(Inbound::Closed { reason });
        }
    }

    fn begin_close(&self, reason: &str) {
        self.finish(reason.to_owned());
        self.closing.send_replace(true);
    }

    fn shutdown_socket(&self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}

async fn read_loop(mut stream: SplitStream<WsStream>, link: Arc<Link>) {
    let reason = loop {
        let inbound = match stream.next().await {
            Some(Ok(Message::Text(text))) => Inbound::Text(text.as_str().to_owned()),
            Some(Ok(Message::Binary(bytes))) => decode_binary(bytes),
            Some(Ok(Message::Close(frame))) => break close_reason(frame),
            Some(Ok(_)) => continue,
            Some(Err(error)) => break format!("WebSocket read failed: {error}"),
            None => break "the WebSocket connection ended".to_owned(),
        };
        if !link.deliver(inbound) {
            break "the transport stopped reading".to_owned();
        }
    };
    link.begin_close(&reason);
}

fn decode_binary(bytes: Bytes) -> Inbound {
    match String::from_utf8(Vec::from(bytes)) {
        Ok(text) => Inbound::Text(text),
        Err(error) => Inbound::InvalidUtf8(error.into_bytes()),
    }
}

fn close_reason(frame: Option<CloseFrame>) -> String {
    match frame {
        Some(frame) if frame.reason.is_empty() => {
            format!(
                "the peer closed the WebSocket with code {}",
                u16::from(frame.code)
            )
        }
        Some(frame) => format!(
            "the peer closed the WebSocket with code {}: {}",
            u16::from(frame.code),
            frame.reason
        ),
        None => "the peer closed the WebSocket".to_owned(),
    }
}

fn keepalive() -> Interval {
    let mut keepalive =
        tokio::time::interval_at(Instant::now() + KEEPALIVE_INTERVAL, KEEPALIVE_INTERVAL);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
    keepalive
}

async fn write_loop(
    mut sink: SplitSink<WsStream, Message>,
    mut outbound: UnboundedReceiver<Outbound>,
    mut keepalive: Interval,
    link: Arc<Link>,
) {
    let mut closing = link.closing.subscribe();
    let outcome = loop {
        tokio::select! {
            biased;
            _ = closing_requested(&mut closing) => {
                break drain(&mut sink, &mut outbound).await;
            }
            message = outbound.recv() => {
                let Some(message) = message else {
                    break Ok(());
                };
                if let Err(error) = write(&mut sink, message).await {
                    break Err(error);
                }
            }
            _ = keepalive.tick() => {
                if let Err(error) = sink.send(Message::Ping(Bytes::new())).await {
                    break Err(error);
                }
            }
        }
    };
    match outcome {
        Ok(()) => {
            let _ = tokio::time::timeout(CLOSE_FLUSH, sink.close()).await;
        }
        Err(error) => link.begin_close(&format!("WebSocket write failed: {error}")),
    }
    link.shutdown_socket();
}

async fn closing_requested(closing: &mut watch::Receiver<bool>) {
    let _ = closing.wait_for(|closing| *closing).await;
}

async fn drain(
    sink: &mut SplitSink<WsStream, Message>,
    outbound: &mut UnboundedReceiver<Outbound>,
) -> Result<(), WsError> {
    let flush = async {
        while let Ok(message) = outbound.try_recv() {
            write(sink, message).await?;
        }
        Ok(())
    };
    tokio::time::timeout(CLOSE_FLUSH, flush)
        .await
        .unwrap_or_else(|_| {
            Err(WsError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "queued messages were not flushed within 500 ms",
            )))
        })
}

async fn write(sink: &mut SplitSink<WsStream, Message>, message: Outbound) -> Result<(), WsError> {
    if message
        .deadline
        .is_some_and(|deadline| deadline <= Instant::now())
    {
        message.ticket.set(WriteStatus::NotWritten);
        return Ok(());
    }
    message.ticket.set(WriteStatus::Indeterminate);
    sink.send(Message::Text(message.text.into())).await?;
    message.ticket.set(WriteStatus::Written);
    Ok(())
}

struct WsShutdown {
    link: Arc<Link>,
    reader: AbortHandle,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl WsShutdown {
    fn take_writer(&self) -> Option<JoinHandle<()>> {
        self.writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl TransportShutdown for WsShutdown {
    fn abort(&self) {
        self.link.begin_close("aborted by the client");
        self.reader.abort();
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(self.close());
            return;
        }
        if let Some(writer) = self.take_writer() {
            writer.abort();
        }
        self.link.shutdown_socket();
    }

    fn close(&self) -> BoxFuture<'static, ()> {
        self.link.begin_close("closed by the client");
        self.reader.abort();
        let writer = self.take_writer();
        let link = self.link.clone();
        Box::pin(async move {
            if let Some(mut writer) = writer
                && tokio::time::timeout(CLOSE_TOTAL, &mut writer)
                    .await
                    .is_err()
            {
                writer.abort();
            }
            link.shutdown_socket();
        })
    }
}
