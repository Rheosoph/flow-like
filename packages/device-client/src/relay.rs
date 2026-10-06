use crate::{
    Error, Result,
    client::TransportKind,
    endpoint::checked_url,
    hub::HubClient,
    pipe::{AbortOnDrop, PIPE_DEPTH, Pipe},
    tls, unix_now,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::DeviceSignalingResponse;
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
    time::Instant,
};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
    tungstenite::{
        self, Message, Utf8Bytes, client::IntoClientRequest, handshake::client::Request,
        http::HeaderValue, protocol::WebSocketConfig,
    },
};
use tokio_util::sync::{CancellationToken, DropGuard};

const SIGNALING_PROTOCOL: &str = "flowlike.device-management.v1";
const TOKEN_PROTOCOL_PREFIX: &str = "flowlike.jwt.";
const MAX_PAYLOAD: usize = 32 * 1024;
const MAX_FRAME: usize = 48 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const HEARTBEAT: Duration = Duration::from_secs(20);
const PONG_TIMEOUT: Duration = Duration::from_secs(60);
const REAUTHORIZE_TIMEOUT: Duration = Duration::from_secs(30);
const RENEW_BEFORE_SECONDS: i64 = 60;
const RENEW_RETRY: Duration = Duration::from_secs(10);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Channel {
    Signal,
    Noise,
    Tunnel,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ServerFrame {
    Ready {
        participant_id: String,
        role: String,
        expires_at: i64,
    },
    Pong,
    Reauthorized {
        expires_at: i64,
    },
    Frame {
        to: String,
        channel: Channel,
        payload: String,
        from: String,
        from_role: String,
    },
}

#[derive(Serialize)]
struct ClientFrame<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    to: &'a str,
    channel: Channel,
    payload: String,
}

enum Control {
    Signal(Vec<u8>),
    Reauthorize {
        token: String,
        expires_at: i64,
        done: oneshot::Sender<Result<()>>,
    },
}

struct Shared {
    device_id: String,
    participant: String,
    expires_at: AtomicI64,
    last_pong: Mutex<Instant>,
    reauthorizing: Mutex<Option<(i64, oneshot::Sender<Result<()>>)>>,
}

impl Shared {
    fn unreachable(&self, message: impl Into<String>) -> Error {
        Error::Unreachable {
            device_id: self.device_id.clone(),
            message: message.into(),
        }
    }
}

/// The relay of the hub for one admitted participant. It carries the WebRTC offer and
/// answer, and tunnel envelopes when no direct path exists.
pub(crate) struct Relay {
    shared: Arc<Shared>,
    control: mpsc::Sender<Control>,
    signal: mpsc::Receiver<Vec<u8>>,
    tunnel_in: mpsc::Receiver<Vec<u8>>,
    tunnel_out: mpsc::Sender<Vec<u8>>,
    writer: tokio::task::JoinHandle<()>,
    reader: AbortOnDrop,
    cancel: DropGuard,
}

impl Relay {
    /// Tries each admitted signaling endpoint in order.
    pub(crate) async fn open(
        admission: &DeviceSignalingResponse,
        participant: &str,
        device_id: &str,
        origin: &str,
    ) -> Result<Self> {
        let mut failures = Vec::new();
        for url in &admission.signaling_urls {
            match connect(url, admission, participant, origin).await {
                Ok(socket) => return Ok(Self::spawn(socket, admission, participant, device_id)),
                Err(error) => failures.push(format!("{url}: {error}")),
            }
        }
        Err(Error::Unreachable {
            device_id: device_id.into(),
            message: format!(
                "no signaling relay accepted the connection ({})",
                failures.join("; ")
            ),
        })
    }

    fn spawn(
        socket: Socket,
        admission: &DeviceSignalingResponse,
        participant: &str,
        device_id: &str,
    ) -> Self {
        let shared = Arc::new(Shared {
            device_id: device_id.into(),
            participant: participant.into(),
            expires_at: AtomicI64::new(admission.expires_at),
            last_pong: Mutex::new(Instant::now()),
            reauthorizing: Mutex::new(None),
        });
        let (sink, stream) = socket.split();
        let (control, control_rx) = mpsc::channel(8);
        let (signal_tx, signal) = mpsc::channel(4);
        let (tunnel_tx, tunnel_in) = mpsc::channel(PIPE_DEPTH);
        let (tunnel_out, tunnel_rx) = mpsc::channel(PIPE_DEPTH);
        let cancel = CancellationToken::new();
        let reader = tokio::spawn(read(
            stream,
            shared.clone(),
            signal_tx,
            tunnel_tx,
            cancel.clone(),
        ));
        let writer = tokio::spawn(write(
            sink,
            shared.clone(),
            control_rx,
            tunnel_rx,
            cancel.clone(),
        ));
        Self {
            shared,
            control,
            signal,
            tunnel_in,
            tunnel_out,
            writer,
            reader: AbortOnDrop(reader),
            cancel: cancel.drop_guard(),
        }
    }

    pub(crate) async fn send_signal(&self, payload: Vec<u8>) -> Result<()> {
        if payload.len() > MAX_PAYLOAD {
            return Err(Error::Invalid(format!(
                "a signaling message of {} bytes exceeds the relay limit of {MAX_PAYLOAD}",
                payload.len()
            )));
        }
        tokio::time::timeout(IO_TIMEOUT, self.control.send(Control::Signal(payload)))
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| {
                self.shared
                    .unreachable("the relay closed before the offer left")
            })
    }

    pub(crate) async fn next_signal(&mut self) -> Result<Vec<u8>> {
        self.signal.recv().await.ok_or_else(|| {
            self.shared
                .unreachable("the relay closed before the device answered")
        })
    }

    /// Carries the tunnel itself and renews the admission before it lapses.
    pub(crate) fn into_pipe(self, hub: Arc<dyn HubClient>, auth_epoch: u64) -> Pipe {
        let renewal = tokio::spawn(keep_admitted(
            hub,
            self.shared.clone(),
            self.control.clone(),
            auth_epoch,
        ));
        Pipe::new(
            TransportKind::Relay,
            self.tunnel_out,
            self.tunnel_in,
            self.writer,
            (self.cancel, self.reader, AbortOnDrop(renewal), self.control),
        )
    }
}

/// The hub admits a controller by the admission token in its subprotocols and by its
/// origin, then confirms the participant, role and expiry it admitted.
async fn connect(
    url: &str,
    admission: &DeviceSignalingResponse,
    participant: &str,
    origin: &str,
) -> Result<Socket, String> {
    let url = checked_url(url, "wss", "ws")
        .filter(|url| url.path().ends_with("/ws/devices"))
        .ok_or_else(|| "the endpoint is not a WSS /ws/devices URL".to_owned())?;
    let connector = match url.scheme() {
        "wss" => Connector::Rustls(tls::client_config().map_err(|error| error.to_string())?),
        _ => Connector::Plain,
    };
    let config = WebSocketConfig::default()
        .read_buffer_size(16 * 1024)
        .write_buffer_size(0)
        .max_write_buffer_size(2 * MAX_FRAME)
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let request = upgrade_request(&url, &admission.token, origin)?;
    let (mut socket, response) = tokio::time::timeout(
        IO_TIMEOUT,
        connect_async_tls_with_config(request, Some(config), true, Some(connector)),
    )
    .await
    .map_err(|_| format!("the relay did not accept within {IO_TIMEOUT:?}"))?
    .map_err(|error| format!("the relay handshake failed: {error}"))?;
    let protocol = response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok());
    if protocol != Some(SIGNALING_PROTOCOL) {
        return Err(format!(
            "the relay answered subprotocol {protocol:?} instead of {SIGNALING_PROTOCOL}"
        ));
    }
    confirm(&mut socket, admission, participant).await?;
    Ok(socket)
}

fn upgrade_request(url: &reqwest::Url, token: &str, origin: &str) -> Result<Request, String> {
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| format!("invalid relay request: {error}"))?;
    let protocols = format!("{SIGNALING_PROTOCOL}, {TOKEN_PROTOCOL_PREFIX}{token}");
    let headers = request.headers_mut();
    headers.insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(&protocols)
            .map_err(|_| "the admission token is not a valid header value".to_owned())?,
    );
    headers.insert(
        "Origin",
        HeaderValue::from_str(origin)
            .map_err(|_| format!("origin {origin} is not a valid header value"))?,
    );
    Ok(request)
}

async fn confirm(
    socket: &mut Socket,
    admission: &DeviceSignalingResponse,
    participant: &str,
) -> Result<(), String> {
    let ready = tokio::time::timeout(IO_TIMEOUT, socket.next())
        .await
        .map_err(|_| "the relay sent no admission confirmation".to_owned())?;
    let Some(Ok(Message::Text(ready))) = ready else {
        return Err("the relay closed before confirming admission".into());
    };
    match serde_json::from_str::<ServerFrame>(&ready) {
        Ok(ServerFrame::Ready {
            participant_id,
            role,
            expires_at,
        }) if participant_id == participant
            && role == "controller"
            && expires_at == admission.expires_at =>
        {
            Ok(())
        }
        _ => Err("the relay confirmed a different admission than the hub issued".into()),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn decode_payload(payload: &str) -> Option<Vec<u8>> {
    if payload.is_empty() || payload.len() > MAX_PAYLOAD.div_ceil(3) * 4 {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    (bytes.len() <= MAX_PAYLOAD && URL_SAFE_NO_PAD.encode(&bytes) == payload).then_some(bytes)
}

async fn read(
    mut stream: SplitStream<Socket>,
    shared: Arc<Shared>,
    signal: mpsc::Sender<Vec<u8>>,
    tunnel: mpsc::Sender<Vec<u8>>,
    cancel: CancellationToken,
) {
    let result: Result<()> = async {
        loop {
            let message = tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                message = stream.next() => message,
            };
            let Some(text) = text_of(&shared, message)? else {
                continue;
            };
            let frame = serde_json::from_str::<ServerFrame>(&text).map_err(|error| {
                shared.unreachable(format!("the relay sent an invalid frame: {error}"))
            })?;
            if !route(&shared, frame, &signal, &tunnel).await? {
                return Ok(());
            }
        }
    }
    .await;
    if let Err(error) = result {
        tracing::debug!("Device relay reader stopped: {error}");
    }
    cancel.cancel();
}

/// Relay frames are text; WebSocket pings are answered by the socket itself.
fn text_of(
    shared: &Shared,
    message: Option<Result<Message, tungstenite::Error>>,
) -> Result<Option<Utf8Bytes>> {
    match message {
        Some(Ok(Message::Text(text))) => Ok(Some(text)),
        Some(Ok(Message::Ping(_) | Message::Pong(_))) => Ok(None),
        Some(Ok(Message::Close(_))) | None => {
            Err(shared.unreachable("the relay closed the connection"))
        }
        Some(Ok(_)) => Err(shared.unreachable("the relay sent binary data")),
        Some(Err(error)) => {
            Err(shared.unreachable(format!("the relay connection failed: {error}")))
        }
    }
}

/// Returns false once nobody consumes the tunnel any more.
async fn route(
    shared: &Shared,
    frame: ServerFrame,
    signal: &mpsc::Sender<Vec<u8>>,
    tunnel: &mpsc::Sender<Vec<u8>>,
) -> Result<bool> {
    match frame {
        ServerFrame::Pong => *lock(&shared.last_pong) = Instant::now(),
        ServerFrame::Reauthorized { expires_at } => reauthorized(shared, expires_at)?,
        ServerFrame::Ready { .. } => {
            return Err(shared.unreachable("the relay repeated its admission"));
        }
        ServerFrame::Frame {
            to,
            channel,
            payload,
            from,
            from_role,
        } => {
            if to == shared.participant && from == shared.device_id && from_role == "device" {
                return deliver(shared, channel, &payload, signal, tunnel).await;
            }
        }
    }
    Ok(true)
}

async fn deliver(
    shared: &Shared,
    channel: Channel,
    payload: &str,
    signal: &mpsc::Sender<Vec<u8>>,
    tunnel: &mpsc::Sender<Vec<u8>>,
) -> Result<bool> {
    let bytes = decode_payload(payload)
        .ok_or_else(|| shared.unreachable("the relay sent a non-canonical payload"))?;
    match channel {
        Channel::Signal => {
            let _ = signal.try_send(bytes);
            Ok(true)
        }
        Channel::Tunnel => Ok(tunnel.send(bytes).await.is_ok()),
        Channel::Noise => Ok(true),
    }
}

fn reauthorized(shared: &Shared, expires_at: i64) -> Result<()> {
    let Some((expected, done)) = lock(&shared.reauthorizing).take() else {
        return Err(shared.unreachable("the relay confirmed a renewal nobody asked for"));
    };
    if expires_at != expected {
        let error = shared.unreachable(format!(
            "the relay renewed admission until {expires_at}, not {expected}"
        ));
        let _ = done.send(Err(error.clone()));
        return Err(error);
    }
    shared.expires_at.store(expires_at, Ordering::Release);
    let _ = done.send(Ok(()));
    Ok(())
}

async fn write(
    mut sink: SplitSink<Socket, Message>,
    shared: Arc<Shared>,
    mut control: mpsc::Receiver<Control>,
    mut tunnel: mpsc::Receiver<Vec<u8>>,
    cancel: CancellationToken,
) {
    let mut heartbeat = tokio::time::interval_at(Instant::now() + HEARTBEAT, HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let result: Result<()> = async {
        loop {
            let text = tokio::select! {
                _ = cancel.cancelled() => None,
                command = control.recv() => command.map(|command| control_text(&shared, command)),
                payload = tunnel.recv() => payload.map(|payload| frame(&shared, Channel::Tunnel, &payload)),
                _ = heartbeat.tick() => Some(ping(&shared)),
            };
            let Some(text) = text.transpose()? else {
                return Ok(());
            };
            tokio::time::timeout(IO_TIMEOUT, sink.send(Message::Text(text.into())))
                .await
                .map_err(|_| shared.unreachable("the relay stopped accepting frames"))?
                .map_err(|error| shared.unreachable(format!("the relay write failed: {error}")))?;
        }
    }
    .await;
    if let Err(error) = result {
        tracing::debug!("Device relay writer stopped: {error}");
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), sink.close()).await;
    cancel.cancel();
}

fn control_text(shared: &Shared, command: Control) -> Result<String> {
    match command {
        Control::Signal(payload) => frame(shared, Channel::Signal, &payload),
        Control::Reauthorize {
            token,
            expires_at,
            done,
        } => {
            *lock(&shared.reauthorizing) = Some((expires_at, done));
            Ok(serde_json::json!({"type": "reauthorize", "token": token}).to_string())
        }
    }
}

/// The hub drops a participant whose admission lapsed or whose pings went unanswered.
fn ping(shared: &Shared) -> Result<String> {
    if unix_now() >= shared.expires_at.load(Ordering::Acquire) {
        return Err(shared.unreachable("the relay admission expired"));
    }
    if lock(&shared.last_pong).elapsed() >= PONG_TIMEOUT {
        return Err(shared.unreachable("the relay stopped answering heartbeats"));
    }
    Ok("{\"type\":\"ping\"}".to_owned())
}

fn frame(shared: &Shared, channel: Channel, payload: &[u8]) -> Result<String> {
    if payload.is_empty() || payload.len() > MAX_PAYLOAD {
        return Err(Error::Invalid(format!(
            "a relay payload of {} bytes is outside 1..={MAX_PAYLOAD}",
            payload.len()
        )));
    }
    serde_json::to_string(&ClientFrame {
        kind: "frame",
        to: &shared.device_id,
        channel,
        payload: URL_SAFE_NO_PAD.encode(payload),
    })
    .map_err(|error| Error::Invalid(format!("could not encode a relay frame: {error}")))
}

/// A failed renewal is retried until shortly before expiry; the relay then closes and the
/// session reconnects on its next use.
async fn keep_admitted(
    hub: Arc<dyn HubClient>,
    shared: Arc<Shared>,
    control: mpsc::Sender<Control>,
    auth_epoch: u64,
) {
    loop {
        let expires_at = shared.expires_at.load(Ordering::Acquire);
        let wait = (expires_at - unix_now() - RENEW_BEFORE_SECONDS).max(1) as u64;
        tokio::time::sleep(Duration::from_secs(wait)).await;
        loop {
            match renew(&hub, &shared, &control, auth_epoch, expires_at).await {
                Ok(()) => break,
                Err(error) if unix_now() + (RENEW_RETRY.as_secs() as i64) < expires_at => {
                    tracing::warn!("Device relay admission renewal failed, retrying: {error}");
                    tokio::time::sleep(RENEW_RETRY).await;
                }
                Err(error) => {
                    tracing::warn!("Device relay admission lapses: {error}");
                    return;
                }
            }
        }
    }
}

async fn renew(
    hub: &Arc<dyn HubClient>,
    shared: &Shared,
    control: &mpsc::Sender<Control>,
    auth_epoch: u64,
    current: i64,
) -> Result<()> {
    let admission = hub
        .controller_admission(&shared.device_id, &shared.participant)
        .await?;
    if admission.device_auth_epoch != auth_epoch || admission.expires_at <= current {
        return Err(shared.unreachable(format!(
            "the renewed admission (epoch {}, expiry {}) does not extend epoch {auth_epoch} past {current}",
            admission.device_auth_epoch, admission.expires_at
        )));
    }
    let (done, confirmed) = oneshot::channel();
    control
        .send(Control::Reauthorize {
            token: admission.token,
            expires_at: admission.expires_at,
            done,
        })
        .await
        .map_err(|_| shared.unreachable("the relay closed before renewal"))?;
    tokio::time::timeout(REAUTHORIZE_TIMEOUT, confirmed)
        .await
        .map_err(|_| shared.unreachable("the relay did not confirm the renewal"))?
        .map_err(|_| shared.unreachable("the relay closed during renewal"))?
}
