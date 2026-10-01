#![allow(dead_code, clippy::result_large_err)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::http::header::{AUTHORIZATION, SEC_WEBSOCKET_PROTOCOL};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

pub const BIG_FRAME_BYTES: usize = 20 << 20;
pub const INVALID_UTF8_FRAME: &[u8] = &[b'{', 0xff, b'}'];

pub struct ProxyHandle {
    pub url: String,
    pub subprotocol_seen: Arc<Mutex<Option<String>>>,
    pub transcript: Arc<Mutex<Vec<String>>>,
    pub binary_frames: Arc<AtomicBool>,
}

pub struct Upstream {
    pub url: String,
    pub requests: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct Proxy {
    upstream: String,
    expect_authorization: Option<String>,
    subprotocol_seen: Arc<Mutex<Option<String>>>,
    transcript: Arc<Transcript>,
    binary_frames: Arc<AtomicBool>,
}

struct Transcript {
    start: Instant,
    lines: Arc<Mutex<Vec<String>>>,
}

impl Transcript {
    fn push(&self, direction: &str, text: &str) {
        let frame: Value = serde_json::from_str(text).unwrap_or_else(|_| Value::from(text));
        let millis = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let line = json!({"t": millis, "dir": direction, "frame": frame});
        lock(&self.lines).push(line.to_string());
    }
}

pub async fn start(
    upstream_ws: String,
    expect_authorization: Option<String>,
    tls: Option<Arc<rustls::ServerConfig>>,
) -> ProxyHandle {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the header proxy");
    let port = listener.local_addr().expect("header proxy address").port();
    let url = match tls {
        Some(_) => format!("wss://localhost:{port}/devtools/browser"),
        None => format!("ws://127.0.0.1:{port}/devtools/browser"),
    };
    let lines = Arc::new(Mutex::new(Vec::new()));
    let proxy = Proxy {
        upstream: upstream_ws,
        expect_authorization,
        subprotocol_seen: Arc::new(Mutex::new(None)),
        transcript: Arc::new(Transcript {
            start: Instant::now(),
            lines: lines.clone(),
        }),
        binary_frames: Arc::new(AtomicBool::new(false)),
    };
    let handle = ProxyHandle {
        url,
        subprotocol_seen: proxy.subprotocol_seen.clone(),
        transcript: lines,
        binary_frames: proxy.binary_frames.clone(),
    };
    tokio::spawn(accept_loop(listener, proxy, tls));
    handle
}

async fn accept_loop(listener: TcpListener, proxy: Proxy, tls: Option<Arc<rustls::ServerConfig>>) {
    let acceptor = tls.map(tokio_rustls::TlsAcceptor::from);
    while let Ok((stream, _)) = listener.accept().await {
        let proxy = proxy.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            match acceptor {
                Some(acceptor) => {
                    if let Ok(stream) = acceptor.accept(stream).await {
                        proxy.serve(stream).await;
                    }
                }
                None => proxy.serve(stream).await,
            }
        });
    }
}

impl Proxy {
    fn check(&self, request: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
        let authorized = self.expect_authorization.as_ref().is_none_or(|expected| {
            request
                .headers()
                .get(AUTHORIZATION)
                .is_some_and(|value| value.as_bytes() == expected.as_bytes())
        });
        if !authorized {
            let mut rejection = ErrorResponse::new(Some("missing or wrong Authorization".into()));
            *rejection.status_mut() = StatusCode::UNAUTHORIZED;
            return Err(rejection);
        }
        if let Some(offered) = request.headers().get(SEC_WEBSOCKET_PROTOCOL) {
            let offered = offered.to_str().unwrap_or_default().to_owned();
            let first = offered
                .split(',')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            *lock(&self.subprotocol_seen) = Some(offered);
            let echo = first
                .parse()
                .expect("an offered subprotocol is a valid header value");
            response.headers_mut().insert(SEC_WEBSOCKET_PROTOCOL, echo);
        }
        Ok(response)
    }

    async fn serve<S>(self, stream: S)
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let callback = |request: &Request, response: Response| self.check(request, response);
        let Ok(downstream) =
            tokio_tungstenite::accept_hdr_async_with_config(stream, callback, Some(unlimited()))
                .await
        else {
            return;
        };
        let Ok((upstream, _)) = tokio_tungstenite::connect_async_with_config(
            self.upstream.as_str(),
            Some(unlimited()),
            false,
        )
        .await
        else {
            return;
        };
        let (mut down_tx, mut down_rx) = downstream.split();
        let (mut up_tx, mut up_rx) = upstream.split();
        let client_to_browser = async {
            while let Some(Ok(message)) = down_rx.next().await {
                if let Message::Text(text) = &message {
                    self.transcript.push("send", text);
                }
                if message.is_close() || up_tx.send(message).await.is_err() {
                    break;
                }
            }
        };
        let browser_to_client = async {
            while let Some(Ok(message)) = up_rx.next().await {
                let message = self.reframe(message);
                if down_tx.send(message).await.is_err() {
                    break;
                }
            }
        };
        tokio::select! {
            _ = client_to_browser => {}
            _ = browser_to_client => {}
        }
    }

    fn reframe(&self, message: Message) -> Message {
        match message {
            Message::Text(text) => {
                self.transcript.push("recv", &text);
                if self.binary_frames.load(Ordering::SeqCst) {
                    Message::Binary(text.into())
                } else {
                    Message::Text(text)
                }
            }
            other => other,
        }
    }
}

pub async fn start_upstream() -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the scripted upstream");
    let address = listener.local_addr().expect("scripted upstream address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    tokio::spawn(upstream_loop(listener, requests.clone()));
    Upstream {
        url: format!("ws://{address}/devtools/browser/upstream"),
        requests,
    }
}

async fn upstream_loop(listener: TcpListener, requests: Arc<Mutex<Vec<String>>>) {
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(serve_upstream(stream, requests.clone()));
    }
}

async fn serve_upstream(stream: TcpStream, requests: Arc<Mutex<Vec<String>>>) {
    let record = |request: &Request, response: Response| {
        let target = request.uri().path_and_query().map(ToString::to_string);
        lock(&requests).push(target.unwrap_or_default());
        Ok(response)
    };
    let Ok(mut socket) =
        tokio_tungstenite::accept_hdr_async_with_config(stream, record, Some(unlimited())).await
    else {
        return;
    };
    while let Some(Ok(message)) = socket.next().await {
        let reply = match message {
            Message::Text(text) => upstream_reply(text.as_str()),
            Message::Binary(bytes) => Message::Binary(bytes),
            Message::Close(_) => break,
            _ => continue,
        };
        let closing = reply.is_close();
        if socket.send(reply).await.is_err() || closing {
            break;
        }
    }
}

fn upstream_reply(text: &str) -> Message {
    match text {
        "big" => Message::Text("x".repeat(BIG_FRAME_BYTES).into()),
        "invalid-utf8" => Message::Binary(INVALID_UTF8_FRAME.to_vec().into()),
        "close" => Message::Close(None),
        other => Message::Text(other.into()),
    }
}

fn unlimited() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
