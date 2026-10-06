//! A hub, a relay and a device that speak the wire protocol from the shared crates
//! without the agent. The device mirrors `apps/standalone/src/transport/{websocket,rtc,
//! tunnel}.rs` and records anything the client does that the agent would refuse.

use crate::{hub::HubClient, rtc::TUNNEL_PROTOCOL, unix_now};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::{Bytes, BytesMut};
use flow_like_device_crypto::noise;
use flow_like_device_protocol::{
    ControllerCertificate, DeviceSignalingResponse, Ed25519PublicKey, TUNNEL_INITIAL_WINDOW,
    TUNNEL_MAX_DATA, TUNNEL_MAX_STREAMS, TunnelDataOpen, TunnelEnvelope, TunnelEnvelopeBody,
    TunnelFrame, TunnelFrameBody, TunnelOpen, TunnelRenewStart, TunnelRenewed, TunnelReset,
    TunnelTarget, verify_controller_certificate,
};
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use http_body_util::BodyExt;
use hyper::{
    Request, Response,
    body::{Body, Frame, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use std::{
    collections::HashMap,
    convert::Infallible,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Notify, mpsc, watch},
    task::JoinHandle,
    time::Instant,
};
use tokio_tungstenite::{
    WebSocketStream, accept_hdr_async,
    tungstenite::{
        Message,
        handshake::server::{Request as Upgrade, Response as Upgraded},
        http::HeaderValue,
    },
};
use tokio_util::sync::CancellationToken;
use webrtc::{
    data_channel::{DataChannel, DataChannelEvent},
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
        RTCSessionDescription, SettingEngine,
    },
};

/// The agent queues this many envelopes per tunnel and direction; a relayed tunnel whose
/// queue overflows is closed.
const AGENT_QUEUE: usize = 32;
const PONG_DEADLINE: Duration = Duration::from_secs(5);
const PROTOCOL_PREFIX: &str = "flowlike.device-management.v1, flowlike.jwt.";

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Lane {
    Signal,
    Tunnel,
}

type Reply = mpsc::Sender<(Lane, Vec<u8>)>;
type Controller = WebSocketStream<TcpStream>;

pub(crate) struct FakeHub {
    pub relay_url: String,
    pub epoch: u64,
    lifetime: i64,
    pub admissions: AtomicUsize,
}

impl FakeHub {
    /// `lifetime` is how long each admission lasts, in seconds.
    pub fn new(relay_url: &str, epoch: u64, lifetime: i64) -> Arc<Self> {
        Arc::new(Self {
            relay_url: relay_url.into(),
            epoch,
            lifetime,
            admissions: AtomicUsize::new(0),
        })
    }
}

#[async_trait::async_trait]
impl HubClient for FakeHub {
    async fn controller_admission(
        &self,
        _device_id: &str,
        participant_id: &str,
    ) -> crate::Result<DeviceSignalingResponse> {
        let issued = self.admissions.fetch_add(1, Ordering::SeqCst);
        let expires_at = unix_now() + self.lifetime;
        Ok(DeviceSignalingResponse {
            token: format!("{participant_id}.{expires_at}.{issued}"),
            expires_at,
            device_auth_epoch: self.epoch,
            signaling_urls: vec![self.relay_url.clone()],
            ice_servers: Vec::new(),
            ice_expires_at: None,
            policy_version: 0,
            policy_digest: None,
        })
    }
}

/// The `/ws/devices` relay of the hub for one fake device.
pub(crate) struct FakeRelay {
    pub url: String,
    pub reauthorized: Arc<AtomicUsize>,
    connections: Arc<Mutex<Vec<CancellationToken>>>,
    cancel: CancellationToken,
}

impl FakeRelay {
    pub async fn start(device: Arc<FakeDevice>) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("ws://{}/ws/devices", listener.local_addr()?);
        let relay = Self {
            url,
            reauthorized: Arc::new(AtomicUsize::new(0)),
            connections: Arc::new(Mutex::new(Vec::new())),
            cancel: CancellationToken::new(),
        };
        tokio::spawn(accept_controllers(
            listener,
            device,
            relay.reauthorized.clone(),
            relay.connections.clone(),
            relay.cancel.clone(),
        ));
        Ok(relay)
    }

    /// Closes every controller socket, as a relay restart would.
    pub fn drop_connections(&self) {
        for connection in self.connections.lock().unwrap().drain(..) {
            connection.cancel();
        }
    }
}

impl Drop for FakeRelay {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn accept_controllers(
    listener: TcpListener,
    device: Arc<FakeDevice>,
    reauthorized: Arc<AtomicUsize>,
    connections: Arc<Mutex<Vec<CancellationToken>>>,
    cancel: CancellationToken,
) {
    loop {
        let socket = tokio::select! {
            _ = cancel.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok((socket, _)) => socket,
                Err(_) => return,
            },
        };
        let connection = cancel.child_token();
        connections.lock().unwrap().push(connection.clone());
        let device = device.clone();
        let reauthorized = reauthorized.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_controller(socket, device, reauthorized, connection).await {
                tracing::debug!("fake relay connection ended: {error:#}");
            }
        });
    }
}

/// Fake tokens read `<participant>.<expires_at>.<serial>`.
fn token_claims(token: &str) -> Result<(String, i64)> {
    let mut parts = token.split('.');
    let participant = parts.next().context("token without participant")?;
    let expires_at = parts.next().context("token without expiry")?.parse()?;
    Ok((participant.to_owned(), expires_at))
}

/// Like the hub, admits only the device subprotocol with a token, an origin and the
/// `/ws/devices` path.
#[allow(
    clippy::result_large_err,
    reason = "tungstenite fixes the signature of the upgrade callback"
)]
async fn accept_controller(stream: TcpStream) -> Result<(Controller, String)> {
    let mut token = None;
    let socket = accept_hdr_async(stream, |request: &Upgrade, mut response: Upgraded| {
        let admitted =
            request.headers().contains_key("Origin") && request.uri().path() == "/ws/devices";
        token = request
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix(PROTOCOL_PREFIX))
            .filter(|_| admitted)
            .map(str::to_owned);
        response.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            HeaderValue::from_static("flowlike.device-management.v1"),
        );
        Ok(response)
    })
    .await?;
    Ok((
        socket,
        token.context("controller presented no admission token")?,
    ))
}

async fn serve_controller(
    stream: TcpStream,
    device: Arc<FakeDevice>,
    reauthorized: Arc<AtomicUsize>,
    cancel: CancellationToken,
) -> Result<()> {
    let (socket, token) = accept_controller(stream).await?;
    let (participant, expires_at) = token_claims(&token)?;
    let (mut sink, mut stream) = socket.split();
    let ready = serde_json::json!({"type":"ready","participant_id":participant,"role":"controller","expires_at":expires_at});
    sink.send(Message::Text(ready.to_string().into())).await?;
    let (reply, mut outgoing) = mpsc::channel::<(Lane, Vec<u8>)>(1024);
    let peer = Peer {
        participant,
        device,
        reauthorized,
        reply,
    };
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            message = stream.next() => {
                if !peer.answer(message, &mut sink).await? {
                    return Ok(());
                }
            }
            Some((lane, payload)) = outgoing.recv() => peer.forward(&mut sink, lane, &payload).await?,
        }
    }
}

/// One admitted controller socket of the fake relay.
struct Peer {
    participant: String,
    device: Arc<FakeDevice>,
    reauthorized: Arc<AtomicUsize>,
    reply: Reply,
}

impl Peer {
    /// Returns false when the controller left.
    async fn answer(
        &self,
        message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
        sink: &mut SplitSink<Controller, Message>,
    ) -> Result<bool> {
        let Some(Ok(Message::Text(text))) = message else {
            return Ok(false);
        };
        let value: serde_json::Value = serde_json::from_str(&text)?;
        let answer = match value["type"].as_str() {
            Some("ping") => serde_json::json!({"type":"pong"}),
            Some("reauthorize") => self.reauthorize(&value)?,
            Some("frame") => {
                self.frame(&value)?;
                return Ok(true);
            }
            _ => bail!("unexpected controller frame {}", text.as_str()),
        };
        sink.send(Message::Text(answer.to_string().into())).await?;
        Ok(true)
    }

    fn reauthorize(&self, value: &serde_json::Value) -> Result<serde_json::Value> {
        let token = value["token"].as_str().context("renewal without token")?;
        let (renewed, expires_at) = token_claims(token)?;
        ensure!(
            renewed == self.participant,
            "a renewal for another participant"
        );
        self.reauthorized.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"type":"reauthorized","expires_at":expires_at}))
    }

    fn frame(&self, value: &serde_json::Value) -> Result<()> {
        ensure!(
            value["to"] == self.device.device_id.as_str(),
            "frame for another device"
        );
        let payload = value["payload"].as_str().context("no payload")?;
        let channel = value["channel"].as_str().unwrap_or_default();
        self.device.deliver(
            &self.participant,
            channel,
            URL_SAFE_NO_PAD.decode(payload)?,
            self.reply.clone(),
        );
        Ok(())
    }

    async fn forward(
        &self,
        sink: &mut SplitSink<Controller, Message>,
        lane: Lane,
        payload: &[u8],
    ) -> Result<()> {
        let frame = serde_json::json!({
            "type": "frame",
            "to": self.participant,
            "channel": lane,
            "payload": URL_SAFE_NO_PAD.encode(payload),
            "from": self.device.device_id,
            "from_role": "device",
        });
        sink.send(Message::Text(frame.to_string().into())).await?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) enum RtcMode {
    /// Answers with a real WebRTC peer.
    Answer,
    /// Answers for another session, so the controller falls back at once.
    Mismatch,
}

#[derive(Clone)]
pub(crate) enum Service {
    Tcp(SocketAddr),
    /// Accepts bytes and never consumes them.
    Stall,
}

#[derive(Default)]
pub(crate) struct Stats {
    pub hellos: AtomicUsize,
    pub refusals: AtomicUsize,
    pub opens: AtomicUsize,
    pub renewals: AtomicUsize,
    pub pongs: AtomicUsize,
    pub stalled_bytes: AtomicUsize,
    pub violations: Mutex<Vec<String>>,
}

impl Stats {
    pub fn violations(&self) -> Vec<String> {
        self.violations.lock().unwrap().clone()
    }
}

type Sessions = HashMap<String, (String, mpsc::Sender<Vec<u8>>, CancellationToken)>;

pub(crate) struct FakeDevice {
    pub device_id: String,
    pub grant_id: String,
    secret: [u8; 32],
    pub public: [u8; 32],
    options: DeviceOptions,
    pub stats: Stats,
    sessions: Mutex<Sessions>,
}

pub(crate) struct DeviceOptions {
    pub rtc: RtcMode,
    /// Caps the expiry the device reports, which moves the controller renewal earlier.
    pub lifetime: i64,
    pub heartbeat: Duration,
    pub services: HashMap<String, Service>,
    pub controller_key: Ed25519PublicKey,
}

impl DeviceOptions {
    pub fn new(controller_key: Ed25519PublicKey) -> Self {
        Self {
            rtc: RtcMode::Mismatch,
            lifetime: 300,
            heartbeat: Duration::from_secs(20),
            services: HashMap::new(),
            controller_key,
        }
    }
}

/// What `ManagementService::connect_tunnel` of the agent yields for an admitted hello or
/// offer: the verified certificate and the responder handshake bound to it.
struct Admission {
    certificate: ControllerCertificate,
    certificate_jws: String,
    handshake: noise::Handshake,
}

/// The `SignalEnvelope::parse` of the agent.
fn agent_accepts_offer(offer: &serde_json::Value) -> bool {
    const FIELDS: [&str; 6] = [
        "kind",
        "session_id",
        "sdp",
        "grant_id",
        "certificate_jws",
        "protocol",
    ];
    offer
        .as_object()
        .is_some_and(|fields| fields.keys().all(|key| FIELDS.contains(&key.as_str())))
        && offer["kind"] == "offer"
        && offer["protocol"] == TUNNEL_PROTOCOL
        && agent_accepts_sdp(offer["sdp"].as_str().unwrap_or_default())
}

/// The `validate_offer` of the agent.
fn agent_accepts_sdp(sdp: &str) -> bool {
    let media: Vec<&str> = sdp.lines().filter(|line| line.starts_with("m=")).collect();
    let candidates = sdp
        .lines()
        .filter(|line| line.starts_with("a=candidate:"))
        .count();
    sdp.len() <= 24 * 1024
        && !sdp.contains('\0')
        && matches!(media.as_slice(), [line] if line.starts_with("m=application ") && line.contains("UDP/DTLS/SCTP"))
        && candidates <= 32
}

impl FakeDevice {
    pub fn new(device_id: &str, grant_id: &str, options: DeviceOptions) -> Arc<Self> {
        let secret = crate::random_bytes::<32>();
        Arc::new(Self {
            device_id: device_id.into(),
            grant_id: grant_id.into(),
            secret,
            public: x25519_dalek::x25519(secret, x25519_dalek::X25519_BASEPOINT_BYTES),
            options,
            stats: Stats::default(),
            sessions: Mutex::new(HashMap::new()),
        })
    }

    fn violation(&self, message: impl Into<String>) -> anyhow::Error {
        let message = message.into();
        self.stats.violations.lock().unwrap().push(message.clone());
        anyhow::anyhow!(message)
    }

    fn service(&self, open: &TunnelOpen) -> Option<Service> {
        let key = match open.target {
            TunnelTarget::ModelGateway => "model_gateway",
            TunnelTarget::Service => open.service_id.as_str(),
        };
        self.options.services.get(key).cloned()
    }

    fn deliver(self: &Arc<Self>, from: &str, channel: &str, bytes: Vec<u8>, reply: Reply) {
        match channel {
            "signal" => {
                let device = self.clone();
                tokio::spawn(async move {
                    if let Err(error) = device.offer(bytes, reply).await {
                        tracing::debug!("fake device dropped an offer: {error:#}");
                    }
                });
            }
            "tunnel" => self.route(from, bytes, reply),
            other => {
                self.violation(format!("frame on channel {other}"));
            }
        }
    }

    /// The agent ignores a hello or offer it does not admit and never answers it.
    fn admit(&self, session_id: &str, grant_id: &str, certificate_jws: &str) -> Option<Admission> {
        let admitted = self.verify(session_id, grant_id, certificate_jws);
        if admitted.is_err() {
            self.stats.refusals.fetch_add(1, Ordering::SeqCst);
        }
        admitted.ok()
    }

    fn verify(&self, session_id: &str, grant_id: &str, certificate_jws: &str) -> Result<Admission> {
        let certificate = verify_controller_certificate(
            certificate_jws,
            &self.options.controller_key,
            unix_now(),
        )?;
        ensure!(
            certificate.device_id == self.device_id
                && certificate.grant_id == grant_id
                && grant_id == self.grant_id
                && certificate.session_id == session_id,
            "certificate binding mismatch"
        );
        let handshake = noise::Handshake::tunnel_responder(
            &self.secret,
            certificate.management_key,
            &self.device_id,
            &certificate.session_id,
        )?;
        Ok(Admission {
            certificate,
            certificate_jws: certificate_jws.into(),
            handshake,
        })
    }

    /// Mirrors the `route_tunnel` of the agent: sessions are keyed by envelope route,
    /// owned by the participant that sent the hello, and fed through a bounded queue.
    fn route(self: &Arc<Self>, from: &str, bytes: Vec<u8>, reply: Reply) {
        let Ok(envelope) = TunnelEnvelope::decode(&bytes) else {
            self.violation("undecodable relay envelope");
            return;
        };
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, (_, input, _)| !input.is_closed());
        if let Some((owner, input, cancel)) = sessions.get(&envelope.session_id) {
            if owner == from && envelope.body == TunnelEnvelopeBody::Close {
                cancel.cancel();
                sessions.remove(&envelope.session_id);
            } else if owner == from && input.try_send(bytes).is_err() {
                self.violation("the controller overran the tunnel queue of the agent");
                cancel.cancel();
            }
            return;
        }
        let TunnelEnvelopeBody::Hello(hello) = &envelope.body else {
            return;
        };
        let Some(admission) = self.admit(
            &envelope.session_id,
            &hello.grant_id,
            &hello.certificate_jws,
        ) else {
            return;
        };
        let (input, receiver) = mpsc::channel(AGENT_QUEUE);
        let _ = input.try_send(bytes);
        let cancel = CancellationToken::new();
        sessions.insert(
            envelope.session_id.clone(),
            (from.to_owned(), input, cancel.clone()),
        );
        tokio::spawn(self.clone().relay_tunnel(
            admission,
            receiver,
            reply,
            envelope.session_id,
            cancel,
        ));
    }

    /// A relayed tunnel that ends on its own tells the controller with a Close envelope.
    async fn relay_tunnel(
        self: Arc<Self>,
        admission: Admission,
        input: mpsc::Receiver<Vec<u8>>,
        reply: Reply,
        route: String,
        cancel: CancellationToken,
    ) {
        let (output, mut outgoing) = mpsc::channel::<Vec<u8>>(AGENT_QUEUE);
        let serve = self.serve_tunnel(admission, input, output);
        let forward = async {
            while let Some(bytes) = outgoing.recv().await {
                if reply.send((Lane::Tunnel, bytes)).await.is_err() {
                    break;
                }
            }
        };
        tokio::select! {
            _ = cancel.cancelled() => return,
            (served, ()) = async { tokio::join!(serve, forward) } => {
                if let Err(error) = served {
                    tracing::debug!("fake relayed tunnel ended: {error:#}");
                }
            }
        }
        let close = TunnelEnvelope {
            session_id: route,
            body: TunnelEnvelopeBody::Close,
        };
        if let Ok(bytes) = close.encode() {
            let _ = reply.try_send((Lane::Tunnel, bytes));
        }
    }

    async fn offer(self: Arc<Self>, bytes: Vec<u8>, reply: Reply) -> Result<()> {
        let offer =
            Offer::parse(&bytes).ok_or_else(|| self.violation("an offer the agent would drop"))?;
        if matches!(self.options.rtc, RtcMode::Mismatch) {
            return send_answer(&reply, "another-session", "v=0").await;
        }
        let admission = self
            .admit(&offer.session_id, &offer.grant_id, &offer.certificate_jws)
            .context("offer refused")?;
        let (peer, answering) = answering_peer().await?;
        let result = self
            .clone()
            .answer(&peer, answering, &offer, admission, reply)
            .await;
        let _ = tokio::time::timeout(Duration::from_secs(5), peer.close()).await;
        result
    }

    async fn answer(
        self: Arc<Self>,
        peer: &Arc<dyn PeerConnection>,
        answering: Answering,
        offer: &Offer,
        admission: Admission,
        reply: Reply,
    ) -> Result<()> {
        let Answering {
            gathered,
            mut opened,
        } = answering;
        let sdp = negotiate(peer, &offer.sdp, gathered).await?;
        send_answer(&reply, &offer.session_id, &sdp).await?;
        let channel = tokio::time::timeout(Duration::from_secs(15), opened.recv()).await?;
        let channel = channel.context("no data channel")?;
        self.check_channel(&channel).await?;
        self.serve_channel(channel, admission).await;
        Ok(())
    }

    /// The `validate_channel` of the agent.
    async fn check_channel(&self, channel: &Arc<dyn DataChannel>) -> Result<()> {
        let reliable = channel.ordered().await.unwrap_or(false)
            && channel
                .max_retransmits()
                .await
                .is_ok_and(|limit| limit.is_none())
            && channel
                .max_packet_life_time()
                .await
                .is_ok_and(|limit| limit.is_none())
            && !channel.negotiated().await.unwrap_or(true);
        let labelled = channel
            .label()
            .await
            .is_ok_and(|label| label == TUNNEL_PROTOCOL)
            && channel
                .protocol()
                .await
                .is_ok_and(|protocol| protocol == TUNNEL_PROTOCOL);
        if !(reliable && labelled) {
            return Err(self.violation("a data channel that is not the reliable tunnel"));
        }
        Ok(())
    }

    async fn serve_channel(self: Arc<Self>, channel: Arc<dyn DataChannel>, admission: Admission) {
        let (input_tx, input) = mpsc::channel(AGENT_QUEUE);
        let (output, mut outgoing) = mpsc::channel::<Vec<u8>>(AGENT_QUEUE);
        let source = channel.clone();
        let read = async move {
            while let Some(DataChannelEvent::OnMessage(message)) = next_message(&source).await {
                if input_tx.send(message.data.to_vec()).await.is_err() {
                    break;
                }
            }
        };
        let write = async {
            while let Some(bytes) = outgoing.recv().await {
                let sent = channel.send(BytesMut::from(bytes.as_slice())).await;
                if sent.is_err() {
                    break;
                }
            }
        };
        tokio::select! {
            _ = read => {}
            _ = async { tokio::join!(self.clone().serve_tunnel(admission, input, output), write) } => {}
        }
    }

    /// The `tunnel::Connection::serve` of the agent: the hello must repeat the admitted
    /// certificate, and the first frame after the handshake confirms the tunnel.
    async fn serve_tunnel(
        self: Arc<Self>,
        admission: Admission,
        mut input: mpsc::Receiver<Vec<u8>>,
        output: mpsc::Sender<Vec<u8>>,
    ) -> Result<()> {
        let certificate = admission.certificate.clone();
        let (route, session) = self.respond(admission, &mut input, &output).await?;
        let (events_tx, events) = mpsc::channel(256);
        let mut pump = Pump {
            session,
            device: self.clone(),
            route,
            certificate,
            output,
            sent: 0,
            received: 0,
            streams: HashMap::new(),
            last_stream: 0,
            renewal: None,
            ping: None,
            events_tx,
            events,
        };
        let expires_at = pump.reported_expiry();
        pump.send(0, TunnelFrameBody::Renewed(TunnelRenewed { expires_at }))
            .await?;
        pump.run(input).await
    }

    async fn respond(
        &self,
        admission: Admission,
        input: &mut mpsc::Receiver<Vec<u8>>,
        output: &mpsc::Sender<Vec<u8>>,
    ) -> Result<(String, noise::Session)> {
        let Admission {
            certificate,
            certificate_jws,
            mut handshake,
        } = admission;
        let (route, first) = self.hello(&certificate, &certificate_jws, input).await?;
        handshake.read(&first)?;
        send_handshake(output, &route, handshake.write()?).await?;
        handshake.read(&self.final_handshake(input).await?)?;
        Ok((route, handshake.finish()?))
    }

    /// The hello must repeat the admitted certificate. Returns its route and the first
    /// handshake message.
    async fn hello(
        &self,
        certificate: &ControllerCertificate,
        certificate_jws: &str,
        input: &mut mpsc::Receiver<Vec<u8>>,
    ) -> Result<(String, Vec<u8>)> {
        let first = TunnelEnvelope::decode(&input.recv().await.context("no hello")?)?;
        let TunnelEnvelopeBody::Hello(hello) = first.body else {
            return Err(self.violation("the tunnel did not start with a hello"));
        };
        if first.session_id != certificate.session_id
            || hello.grant_id != certificate.grant_id
            || hello.certificate_jws != certificate_jws
        {
            return Err(self.violation("the hello does not repeat the admitted certificate"));
        }
        self.stats.hellos.fetch_add(1, Ordering::SeqCst);
        Ok((first.session_id, URL_SAFE_NO_PAD.decode(&hello.data)?))
    }

    async fn final_handshake(&self, input: &mut mpsc::Receiver<Vec<u8>>) -> Result<Vec<u8>> {
        let last = input.recv().await.context("no final handshake")?;
        match TunnelEnvelope::decode(&last)?.body {
            TunnelEnvelopeBody::Handshake(last) => Ok(last),
            _ => Err(self.violation("the controller skipped its final handshake")),
        }
    }
}

/// The fields of an offer that the agent reads.
struct Offer {
    session_id: String,
    grant_id: String,
    certificate_jws: String,
    sdp: String,
}

impl Offer {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let offer: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let field = |name: &str| offer[name].as_str().map(str::to_owned);
        agent_accepts_offer(&offer).then_some(())?;
        Some(Self {
            session_id: field("session_id")?,
            grant_id: field("grant_id")?,
            certificate_jws: field("certificate_jws")?,
            sdp: field("sdp")?,
        })
    }
}

async fn negotiate(
    peer: &Arc<dyn PeerConnection>,
    offer: &str,
    mut gathered: watch::Receiver<bool>,
) -> Result<String> {
    peer.set_remote_description(RTCSessionDescription::offer(offer.to_owned())?)
        .await?;
    let answer = peer.create_answer(None).await?;
    peer.set_local_description(answer).await?;
    while !*gathered.borrow_and_update() {
        gathered.changed().await?;
    }
    Ok(peer.local_description().await.context("no answer")?.sdp)
}

async fn send_handshake(
    output: &mpsc::Sender<Vec<u8>>,
    route: &str,
    message: Vec<u8>,
) -> Result<()> {
    let envelope = TunnelEnvelope {
        session_id: route.into(),
        body: TunnelEnvelopeBody::Handshake(message),
    };
    output.send(envelope.encode()?).await?;
    Ok(())
}

async fn send_answer(reply: &Reply, session_id: &str, sdp: &str) -> Result<()> {
    let answer = serde_json::json!({"session_id":session_id,"kind":"answer","sdp":sdp});
    reply
        .send((Lane::Signal, answer.to_string().into_bytes()))
        .await?;
    Ok(())
}

async fn next_message(channel: &Arc<dyn DataChannel>) -> Option<DataChannelEvent> {
    loop {
        match channel.poll().await? {
            DataChannelEvent::OnMessage(message) => {
                return Some(DataChannelEvent::OnMessage(message));
            }
            DataChannelEvent::OnClose | DataChannelEvent::OnClosing | DataChannelEvent::OnError => {
                return None;
            }
            _ => {}
        }
    }
}

struct Answering {
    gathered: watch::Receiver<bool>,
    opened: mpsc::Receiver<Arc<dyn DataChannel>>,
}

async fn answering_peer() -> Result<(Arc<dyn PeerConnection>, Answering)> {
    let (gathered_tx, gathered) = watch::channel(false);
    let (channels_tx, opened) = mpsc::channel(1);
    let mut settings = SettingEngine::default();
    settings.set_include_loopback_candidate(true);
    let peer = PeerConnectionBuilder::new()
        .with_setting_engine(settings)
        .with_handler(Arc::new(Answerer {
            gathered: gathered_tx,
            channels: channels_tx,
        }))
        .with_dedicated_reactor_thread(true)
        .with_reactor_pool_size(1)
        .with_udp_addrs(vec!["127.0.0.1:0".to_owned()])
        .build()
        .await?;
    Ok((Arc::new(peer), Answering { gathered, opened }))
}

struct Answerer {
    gathered: watch::Sender<bool>,
    channels: mpsc::Sender<Arc<dyn DataChannel>>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Answerer {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            self.gathered.send_replace(true);
        }
    }

    async fn on_data_channel(&self, channel: Arc<dyn DataChannel>) {
        let _ = self.channels.try_send(channel);
    }
}

enum Event {
    Opened(u32),
    Data(u32, Vec<u8>),
    Consumed(u32, u32),
    Fin(u32),
    Closed(u32),
    Failed(u32),
}

struct DeviceStream {
    receive_credit: u32,
    send_credit: u32,
    opened: bool,
    received_fin: bool,
    input: mpsc::UnboundedSender<Option<Vec<u8>>>,
    credit: watch::Sender<u64>,
    task: JoinHandle<()>,
}

impl Drop for DeviceStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Renewal {
    certificate: ControllerCertificate,
    handshake: noise::Handshake,
    pending_ping: Option<[u8; 8]>,
}

type StreamInput = mpsc::UnboundedReceiver<Option<Vec<u8>>>;
type StreamTask = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

struct Pump {
    session: noise::Session,
    device: Arc<FakeDevice>,
    route: String,
    certificate: ControllerCertificate,
    output: mpsc::Sender<Vec<u8>>,
    sent: u64,
    received: u64,
    streams: HashMap<u32, DeviceStream>,
    last_stream: u32,
    renewal: Option<Renewal>,
    ping: Option<([u8; 8], Instant)>,
    events_tx: mpsc::Sender<Event>,
    events: mpsc::Receiver<Event>,
}

impl Pump {
    /// The agent reports the expiry of its certificate; tests may report less to force
    /// an early renewal.
    fn reported_expiry(&self) -> i64 {
        self.certificate
            .expires_at
            .min(unix_now() + self.device.options.lifetime)
    }

    fn violation(&self, message: impl Into<String>) -> anyhow::Error {
        self.device.violation(message)
    }

    async fn send(&mut self, stream_id: u32, body: TunnelFrameBody) -> Result<()> {
        let plaintext = TunnelFrame {
            sequence: self.sent,
            stream_id,
            body,
        }
        .encode()?;
        let ciphertext = self.session.encrypt(&plaintext)?;
        self.sent += 1;
        let envelope = TunnelEnvelope {
            session_id: self.route.clone(),
            body: TunnelEnvelopeBody::Message(ciphertext),
        };
        self.output.send(envelope.encode()?).await?;
        Ok(())
    }

    async fn run(&mut self, mut input: mpsc::Receiver<Vec<u8>>) -> Result<()> {
        let mut heartbeat = tokio::time::interval_at(
            Instant::now() + self.device.options.heartbeat,
            self.device.options.heartbeat,
        );
        loop {
            tokio::select! {
                bytes = input.recv() => self.incoming(&bytes.context("the controller left")?).await?,
                event = self.events.recv(), if self.renewal.is_none() => {
                    self.event(event.context("stream events stopped")?).await?;
                }
                _ = heartbeat.tick(), if self.renewal.is_none() => self.heartbeat().await?,
            }
        }
    }

    /// One heartbeat in flight, as the agent sends it, and a bounded wait for its pong.
    async fn heartbeat(&mut self) -> Result<()> {
        if let Some((_, sent)) = self.ping {
            if sent.elapsed() >= PONG_DEADLINE {
                return Err(self.violation("the controller left a heartbeat unanswered"));
            }
            return Ok(());
        }
        let nonce = crate::random_bytes::<8>();
        self.ping = Some((nonce, Instant::now()));
        self.send(0, TunnelFrameBody::Ping(nonce)).await
    }

    fn pong(&mut self, nonce: [u8; 8]) {
        if self.ping.is_some_and(|(expected, _)| expected == nonce) {
            self.ping = None;
            self.device.stats.pongs.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn incoming(&mut self, bytes: &[u8]) -> Result<()> {
        let frame = self.open_frame(bytes)?;
        if self.renewal.is_some() {
            return self.during_renewal(frame.body).await;
        }
        let id = frame.stream_id;
        match frame.body {
            TunnelFrameBody::Open(open) => self.open(id, open).await,
            TunnelFrameBody::OpenData(open) => self.open_data(id, open).await,
            TunnelFrameBody::Data(bytes) => self.data(id, bytes).await,
            TunnelFrameBody::Window(credit) => self.window(id, credit),
            TunnelFrameBody::Fin => self.fin(id),
            TunnelFrameBody::Reset(_) => {
                self.streams.remove(&id);
                Ok(())
            }
            TunnelFrameBody::Ping(nonce) => self.send(0, TunnelFrameBody::Pong(nonce)).await,
            TunnelFrameBody::Pong(nonce) => {
                self.pong(nonce);
                Ok(())
            }
            TunnelFrameBody::RenewStart(start) => self.renew_start(start).await,
            body => Err(self.violation(format!("frame kind {} from a controller", body.kind()))),
        }
    }

    fn open_frame(&mut self, bytes: &[u8]) -> Result<TunnelFrame> {
        let envelope = TunnelEnvelope::decode(bytes)?;
        if envelope.session_id != self.route {
            return Err(self.violation("an envelope for another tunnel"));
        }
        let ciphertext = match envelope.body {
            TunnelEnvelopeBody::Message(ciphertext) => ciphertext,
            TunnelEnvelopeBody::Close => bail!("the controller closed the tunnel"),
            _ => return Err(self.violation("a handshake envelope in an open tunnel")),
        };
        let frame = TunnelFrame::decode(&self.session.decrypt(&ciphertext)?)?;
        if frame.sequence != self.received {
            let (sequence, due) = (frame.sequence, self.received);
            return Err(self.violation(format!("frame {sequence} arrived where {due} was due")));
        }
        self.received += 1;
        Ok(frame)
    }

    async fn open(&mut self, id: u32, open: TunnelOpen) -> Result<()> {
        self.claim(id)?;
        if self.streams.len() >= TUNNEL_MAX_STREAMS {
            return self.reset(id, "limit").await;
        }
        let Some(service) = self.device.service(&open) else {
            return self.reset(id, "unauthorized").await;
        };
        self.device.stats.opens.fetch_add(1, Ordering::SeqCst);
        let events = self.events_tx.clone();
        let device = self.device.clone();
        self.insert(id, move |input, credits| match service {
            Service::Stall => Box::pin(stall(id, input, events, device)),
            Service::Tcp(address) => Box::pin(relay_tcp(id, address, input, credits, events)),
        });
        Ok(())
    }

    async fn open_data(&mut self, id: u32, open: TunnelDataOpen) -> Result<()> {
        self.claim(id)?;
        if self.streams.len() >= TUNNEL_MAX_STREAMS {
            return self.reset(id, "limit").await;
        }
        let events = self.events_tx.clone();
        self.insert(id, move |input, credits| {
            Box::pin(bulk_read(id, open, input, credits, events))
        });
        Ok(())
    }

    async fn data(&mut self, id: u32, bytes: Vec<u8>) -> Result<()> {
        let Some(stream) = self.streams.get_mut(&id) else {
            return self.reset(id, "flow_control").await;
        };
        let length = bytes.len();
        if !stream.opened || stream.received_fin || length > stream.receive_credit as usize {
            return Err(self.violation(format!(
                "stream {id} sent {length} bytes outside its window"
            )));
        }
        stream.receive_credit -= length as u32;
        let _ = stream.input.send(Some(bytes));
        Ok(())
    }

    fn window(&mut self, id: u32, credit: u32) -> Result<()> {
        let Some(stream) = self.streams.get_mut(&id) else {
            return Ok(());
        };
        let Some(total) = stream
            .send_credit
            .checked_add(credit)
            .filter(|total| *total <= TUNNEL_INITIAL_WINDOW)
        else {
            return Err(self.violation(format!("stream {id} over-granted {credit}")));
        };
        stream.send_credit = total;
        stream
            .credit
            .send_modify(|granted| *granted += u64::from(credit));
        Ok(())
    }

    fn fin(&mut self, id: u32) -> Result<()> {
        let Some(stream) = self.streams.get_mut(&id) else {
            return Ok(());
        };
        if !stream.opened || stream.received_fin {
            return Err(self.violation(format!("stream {id} finished out of order")));
        }
        stream.received_fin = true;
        let _ = stream.input.send(None);
        Ok(())
    }

    /// The `TunnelAuthority::renew` of the agent: a fresh session, the same grant, a
    /// rotated key and a later expiry.
    fn renews(&self, certificate: &ControllerCertificate) -> bool {
        certificate.session_id != self.certificate.session_id
            && certificate.grant_id == self.certificate.grant_id
            && certificate.management_key != self.certificate.management_key
            && certificate.expires_at > self.certificate.expires_at
    }

    async fn renew_start(&mut self, start: TunnelRenewStart) -> Result<()> {
        let certificate = verify_controller_certificate(
            &start.certificate_jws,
            &self.device.options.controller_key,
            unix_now(),
        )?;
        if !self.renews(&certificate) {
            return Err(self.violation("a renewal that does not rotate and extend"));
        }
        let mut handshake = noise::Handshake::tunnel_responder(
            &self.device.secret,
            certificate.management_key,
            &self.device.device_id,
            &certificate.session_id,
        )?;
        handshake.read(&URL_SAFE_NO_PAD.decode(&start.data)?)?;
        let reply = handshake.write()?;
        self.renewal = Some(Renewal {
            certificate,
            handshake,
            pending_ping: None,
        });
        self.send(0, TunnelFrameBody::RenewReply(reply)).await
    }

    /// The agent accepts only heartbeats and the finish while a renewal runs.
    async fn during_renewal(&mut self, body: TunnelFrameBody) -> Result<()> {
        match body {
            TunnelFrameBody::Pong(nonce) => {
                self.pong(nonce);
                Ok(())
            }
            TunnelFrameBody::Ping(nonce) => {
                let held = self
                    .renewal
                    .as_mut()
                    .and_then(|renewal| renewal.pending_ping.replace(nonce));
                if held.is_some() {
                    return Err(self.violation("a second heartbeat during renewal"));
                }
                Ok(())
            }
            TunnelFrameBody::RenewFinish(bytes) => self.finish_renewal(&bytes).await,
            body => Err(self.violation(format!("frame kind {} during renewal", body.kind()))),
        }
    }

    async fn finish_renewal(&mut self, bytes: &[u8]) -> Result<()> {
        let Renewal {
            certificate,
            mut handshake,
            pending_ping,
        } = self.renewal.take().context("no renewal")?;
        handshake.read(bytes)?;
        self.session = handshake.finish()?;
        self.certificate = certificate;
        self.ping = None;
        self.device.stats.renewals.fetch_add(1, Ordering::SeqCst);
        let expires_at = self.reported_expiry();
        self.send(0, TunnelFrameBody::Renewed(TunnelRenewed { expires_at }))
            .await?;
        if let Some(nonce) = pending_ping {
            self.send(0, TunnelFrameBody::Pong(nonce)).await?;
        }
        Ok(())
    }

    fn claim(&mut self, id: u32) -> Result<()> {
        if id <= self.last_stream || id.is_multiple_of(2) {
            return Err(self.violation(format!("stream {id} reused or even")));
        }
        self.last_stream = id;
        Ok(())
    }

    async fn reset(&mut self, id: u32, code: &str) -> Result<()> {
        self.streams.remove(&id);
        let reset = TunnelReset {
            code: code.into(),
            message: format!("fake device refused stream {id}"),
        };
        self.send(id, TunnelFrameBody::Reset(reset)).await
    }

    fn insert(
        &mut self,
        id: u32,
        task: impl FnOnce(StreamInput, watch::Receiver<u64>) -> StreamTask,
    ) {
        let (input, receiver) = mpsc::unbounded_channel();
        let (credit, credits) = watch::channel(u64::from(TUNNEL_INITIAL_WINDOW));
        let events = self.events_tx.clone();
        let body = task(receiver, credits);
        let task = tokio::spawn(async move {
            let event = if body.await.is_ok() {
                Event::Closed(id)
            } else {
                Event::Failed(id)
            };
            let _ = events.send(event).await;
        });
        self.streams.insert(
            id,
            DeviceStream {
                receive_credit: TUNNEL_INITIAL_WINDOW,
                send_credit: TUNNEL_INITIAL_WINDOW,
                opened: false,
                received_fin: false,
                input,
                credit,
                task,
            },
        );
    }

    async fn event(&mut self, event: Event) -> Result<()> {
        let (id, body) = match event {
            Event::Opened(id) => {
                let Some(stream) = self.streams.get_mut(&id) else {
                    return Ok(());
                };
                stream.opened = true;
                (id, TunnelFrameBody::Opened)
            }
            Event::Data(id, bytes) => {
                let Some(stream) = self.streams.get_mut(&id) else {
                    return Ok(());
                };
                ensure!(
                    bytes.len() <= stream.send_credit as usize,
                    "the fake device outran its own credit"
                );
                stream.send_credit -= bytes.len() as u32;
                (id, TunnelFrameBody::Data(bytes))
            }
            Event::Consumed(id, count) => {
                let Some(stream) = self.streams.get_mut(&id) else {
                    return Ok(());
                };
                stream.receive_credit += count;
                (id, TunnelFrameBody::Window(count))
            }
            Event::Fin(id) if self.streams.contains_key(&id) => (id, TunnelFrameBody::Fin),
            Event::Closed(id) => {
                self.streams.remove(&id);
                return Ok(());
            }
            Event::Failed(id) if self.streams.contains_key(&id) => {
                return self.reset(id, "stream_failed").await;
            }
            Event::Fin(_) | Event::Failed(_) => return Ok(()),
        };
        self.send(id, body).await
    }
}

/// Counts what it receives and never consumes it, so no credit returns.
async fn stall(
    id: u32,
    mut input: StreamInput,
    events: mpsc::Sender<Event>,
    device: Arc<FakeDevice>,
) -> Result<()> {
    events.send(Event::Opened(id)).await?;
    while let Some(Some(bytes)) = input.recv().await {
        device
            .stats
            .stalled_bytes
            .fetch_add(bytes.len(), Ordering::SeqCst);
    }
    std::future::pending::<()>().await;
    Ok(())
}

/// Like `streams.rs` of the agent: connect, then pump both directions within credit.
async fn relay_tcp(
    id: u32,
    address: SocketAddr,
    input: StreamInput,
    credits: watch::Receiver<u64>,
    events: mpsc::Sender<Event>,
) -> Result<()> {
    let socket = TcpStream::connect(address).await?;
    socket.set_nodelay(true)?;
    events.send(Event::Opened(id)).await?;
    let (mut read, write) = socket.into_split();
    let reading = read_into_events(id, &mut read, credits, &events);
    tokio::try_join!(reading, write_from_input(id, write, input, &events))?;
    Ok(())
}

/// Returns credit only for what the service took, as the agent does.
async fn write_from_input(
    id: u32,
    mut write: tokio::net::tcp::OwnedWriteHalf,
    mut input: StreamInput,
    events: &mpsc::Sender<Event>,
) -> Result<()> {
    while let Some(Some(bytes)) = input.recv().await {
        write.write_all(&bytes).await?;
        events.send(Event::Consumed(id, bytes.len() as u32)).await?;
    }
    write.shutdown().await?;
    Ok(())
}

/// Answers a bulk read with a fixed JSON document after the Fin of the controller.
async fn bulk_read(
    id: u32,
    open: TunnelDataOpen,
    mut input: StreamInput,
    credits: watch::Receiver<u64>,
    events: mpsc::Sender<Event>,
) -> Result<()> {
    let TunnelDataOpen::Request { request } = open else {
        bail!("the fake device answers only bulk reads")
    };
    events.send(Event::Opened(id)).await?;
    ensure!(
        matches!(input.recv().await, Some(None)),
        "a read stream received data"
    );
    let response = serde_json::to_vec(&serde_json::json!({
        "operation_id": request.operation_id,
        "state": "completed",
        "result": {"device_id": request.device_id},
    }))?;
    read_into_events(id, &mut response.as_slice(), credits, &events).await
}

/// Reads like the `read_service` of the agent: never more than the controller credit.
async fn read_into_events(
    id: u32,
    source: &mut (impl tokio::io::AsyncRead + Unpin),
    mut credits: watch::Receiver<u64>,
    events: &mpsc::Sender<Event>,
) -> Result<()> {
    let mut consumed = 0u64;
    loop {
        let available = *credits.borrow() - consumed;
        if available == 0 {
            credits.changed().await?;
            continue;
        }
        let mut bytes = vec![0; available.min(TUNNEL_MAX_DATA as u64) as usize];
        let count = source.read(&mut bytes).await?;
        if count == 0 {
            events.send(Event::Fin(id)).await?;
            return Ok(());
        }
        consumed += count as u64;
        bytes.truncate(count);
        events.send(Event::Data(id, bytes)).await?;
    }
}

pub(crate) async fn echo_server() -> Result<SocketAddr> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = socket.into_split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
                let _ = write.shutdown().await;
            });
        }
    });
    Ok(address)
}

/// A streamed response body fed by the test.
pub(crate) struct ChannelBody(pub mpsc::Receiver<Bytes>);

impl Body for ChannelBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.0
            .poll_recv(cx)
            .map(|chunk| chunk.map(|bytes| Ok(Frame::data(bytes))))
    }
}

pub(crate) type BoxedBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;

/// A plain HTTP/1.1 server with keep-alive for the device side or for the hub.
pub(crate) async fn http_server<F, Fut>(handler: F) -> Result<SocketAddr>
where
    F: Fn(Request<Incoming>) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Response<BoxedBody>> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let handler = handler.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let response = handler(request);
                    async move { Ok::<_, Infallible>(response.await) }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(socket), service)
                    .await;
            });
        }
    });
    Ok(address)
}

/// Server-sent events: the first event at once, the second after `release`.
pub(crate) async fn sse_server(release: Arc<Notify>) -> Result<SocketAddr> {
    http_server(move |request: Request<Incoming>| sse_response(request, release.clone())).await
}

async fn sse_response(request: Request<Incoming>, release: Arc<Notify>) -> Response<BoxedBody> {
    let saw_authorization = request.headers().contains_key("authorization");
    let host = request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let path = request.uri().path().to_owned();
    let _ = request.into_body().collect().await;
    let (sender, receiver) = mpsc::channel(4);
    tokio::spawn(async move {
        let _ = sender
            .send(Bytes::from(format!("data: one {path}\n\n")))
            .await;
        release.notified().await;
        let _ = sender.send(Bytes::from_static(b"data: two\n\n")).await;
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .header("x-saw-authorization", saw_authorization.to_string())
        .header("x-host", host)
        .body(ChannelBody(receiver).boxed())
        .expect("static response")
}
