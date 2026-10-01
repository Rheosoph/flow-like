use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::transport::memory::{InMemoryControl, PeerFrame, ScriptedLink, scripted_pair};

const BROWSER_WS_PATH: &str = "/devtools/browser";
const MAX_REQUEST_HEAD: usize = 16 * 1024;

pub struct MockServerOptions {
    pub json_version: MockJsonVersion,
    pub expect_header: Option<(String, String)>,
}

pub enum MockJsonVersion {
    Ok,
    NotFound,
    Refuse,
}

pub struct MockCdpServer {
    pub address: std::net::SocketAddr,
    pub control: InMemoryControl,
}

impl MockCdpServer {
    pub async fn start(options: MockServerOptions) -> MockCdpServer {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("MockCdpServer could not bind 127.0.0.1:0");
        let address = listener
            .local_addr()
            .expect("MockCdpServer listener has no local address");
        let (link, control) = scripted_pair();
        let json_version = match options.json_version {
            MockJsonVersion::Ok => Some(version_body(address)),
            MockJsonVersion::NotFound => None,
            MockJsonVersion::Refuse => return MockCdpServer { address, control },
        };
        let state = Arc::new(ServerState {
            json_version,
            expect_header: options.expect_header,
            link: Mutex::new(Some(link)),
        });
        tokio::spawn(serve(listener, state));
        MockCdpServer { address, control }
    }

    pub fn ws_url(&self) -> String {
        browser_ws_url(self.address)
    }
}

struct ServerState {
    json_version: Option<String>,
    expect_header: Option<(String, String)>,
    link: Mutex<Option<ScriptedLink>>,
}

impl ServerState {
    fn take_link(&self) -> Option<ScriptedLink> {
        self.link
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn rejection_for(&self, request: &Request) -> Option<ErrorResponse> {
        if !request.uri().path().starts_with(BROWSER_WS_PATH) {
            return Some(rejection(StatusCode::FORBIDDEN, "Connection rejected"));
        }
        let (name, expected) = self.expect_header.as_ref()?;
        let actual = request
            .headers()
            .get(name.as_str())
            .and_then(|value| value.to_str().ok());
        (actual != Some(expected.as_str())).then(|| {
            rejection(
                StatusCode::UNAUTHORIZED,
                &format!("MockCdpServer expected the header {name}"),
            )
        })
    }
}

struct UpgradeCheck {
    state: Arc<ServerState>,
    granted: oneshot::Sender<ScriptedLink>,
}

impl Callback for UpgradeCheck {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        if let Some(rejected) = self.state.rejection_for(request) {
            return Err(rejected);
        }
        let Some(link) = self.state.take_link() else {
            return Err(rejection(
                StatusCode::CONFLICT,
                "MockCdpServer bridges a single WebSocket connection",
            ));
        };
        if self.granted.send(link).is_err() {
            tracing::debug!("MockCdpServer upgrade finished after its connection task ended");
        }
        Ok(response)
    }
}

fn browser_ws_url(address: SocketAddr) -> String {
    format!("ws://{address}{BROWSER_WS_PATH}/mock")
}

fn version_body(address: SocketAddr) -> String {
    serde_json::json!({
        "Browser": "Chrome/154.0.8037.92",
        "Protocol-Version": "1.3",
        "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/154.0.0.0 Safari/537.36",
        "V8-Version": "15.4.80.19",
        "WebKit-Version": "537.36 (@334b65d254ccc35df4fca82706d1753227b01039)",
        "webSocketDebuggerUrl": browser_ws_url(address),
    })
    .to_string()
}

fn rejection(status: StatusCode, text: &str) -> ErrorResponse {
    let mut response = ErrorResponse::new(Some(text.to_owned()));
    *response.status_mut() = status;
    response
}

async fn serve(listener: TcpListener, state: Arc<ServerState>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(handle_connection(stream, state.clone()));
            }
            Err(error) => {
                tracing::debug!("MockCdpServer stopped accepting connections: {error}");
                return;
            }
        }
    }
}

async fn handle_connection(mut stream: TcpStream, state: Arc<ServerState>) {
    let Some(head) = read_request_head(&mut stream).await else {
        return;
    };
    let head_text = String::from_utf8_lossy(&head).into_owned();
    if is_websocket_upgrade(&head_text) {
        accept_websocket(stream, head, state).await;
    } else {
        answer_http(stream, &head_text, &state).await;
    }
}

async fn read_request_head(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if head.len() > MAX_REQUEST_HEAD {
            return None;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    Some(head)
}

fn is_websocket_upgrade(head: &str) -> bool {
    head.lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .any(|(name, value)| {
            name.trim().eq_ignore_ascii_case("upgrade")
                && value.trim().eq_ignore_ascii_case("websocket")
        })
}

async fn answer_http(mut stream: TcpStream, head: &str, state: &ServerState) {
    let path = head.split_whitespace().nth(1).unwrap_or_default();
    let path = path.split('?').next().unwrap_or_default();
    let response = match (&state.json_version, path) {
        (Some(body), "/json/version") => {
            http_response("200 OK", "application/json; charset=UTF-8", body)
        }
        _ => http_response("404 Not Found", "text/plain", "Not Found"),
    };
    if let Err(error) = stream.write_all(response.as_bytes()).await {
        tracing::debug!("MockCdpServer could not answer {path}: {error}");
    }
    let _ = stream.shutdown().await;
}

fn http_response(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn accept_websocket(stream: TcpStream, head: Vec<u8>, state: Arc<ServerState>) {
    let (reader, writer) = stream.into_split();
    let io = tokio::io::join(Cursor::new(head).chain(reader), writer);
    let (granted, grant) = oneshot::channel();
    let check = UpgradeCheck { state, granted };
    let socket = match tokio_tungstenite::accept_hdr_async(io, check).await {
        Ok(socket) => socket,
        Err(error) => {
            tracing::debug!("MockCdpServer refused a WebSocket upgrade: {error}");
            return;
        }
    };
    if let Ok(link) = grant.await {
        bridge(socket, link).await;
    }
}

async fn bridge<S>(socket: WebSocketStream<S>, link: ScriptedLink)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let ScriptedLink { to_peer, from_peer } = link;
    let (sink, stream) = socket.split();
    tokio::select! {
        () = forward_client_frames(stream, to_peer) => {}
        () = forward_peer_frames(from_peer, sink) => {}
    }
}

async fn forward_client_frames<S>(
    mut stream: SplitStream<WebSocketStream<S>>,
    to_peer: UnboundedSender<String>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    while let Some(Ok(message)) = stream.next().await {
        let text = match message {
            Message::Text(text) => text.as_str().to_owned(),
            Message::Binary(bytes) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => text,
                Err(_) => continue,
            },
            _ => continue,
        };
        if to_peer.send(text).is_err() {
            return;
        }
    }
}

async fn forward_peer_frames<S>(
    mut from_peer: UnboundedReceiver<String>,
    mut sink: SplitSink<WebSocketStream<S>, Message>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let reason = loop {
        let Some(frame) = from_peer.recv().await else {
            break "the scripted browser went away".to_owned();
        };
        let message = match PeerFrame::decode(frame) {
            PeerFrame::Text(text) => Message::text(text),
            PeerFrame::Binary(bytes) => Message::binary(bytes),
            PeerFrame::Close(reason) => break reason,
        };
        if sink.send(message).await.is_err() {
            return;
        }
    };
    let frame = CloseFrame {
        code: CloseCode::Normal,
        reason: reason.into(),
    };
    let _ = sink.send(Message::Close(Some(frame))).await;
    std::future::pending::<()>().await;
}
