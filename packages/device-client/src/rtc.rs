use crate::{
    Error, Result,
    client::TransportKind,
    pipe::{AbortOnDrop, PIPE_DEPTH, Pipe},
    relay::Relay,
    unix_now,
};
use bytes::BytesMut;
use flow_like_device_crypto::controller::CertifiedHandshake;
use flow_like_device_protocol::{DeviceIceServer, DeviceSignalingResponse, TUNNEL_MAX_ENVELOPE};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use webrtc::{
    data_channel::{DataChannel, DataChannelEvent, RTCDataChannelInit},
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
        RTCIceGatheringState, RTCIceServer, RTCPeerConnectionState, RTCSessionDescription,
        SettingEngine,
    },
};

pub(crate) const TUNNEL_PROTOCOL: &str = "flowlike.device-tunnel.v1";
const SETUP_TIMEOUT: Duration = Duration::from_secs(15);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const BUFFER_BYTES: usize = 1024 * 1024;
const MAX_ICE_SERVERS: usize = 16;
const MAX_OFFER_BYTES: usize = 24 * 1024;
const MAX_OFFER_CANDIDATES: usize = 32;

struct Handler {
    gathered: watch::Sender<bool>,
    failed: CancellationToken,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            self.gathered.send_replace(true);
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        if matches!(
            state,
            RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed
        ) {
            self.failed.cancel();
        }
    }

    async fn on_data_channel(&self, channel: Arc<dyn DataChannel>) {
        let _ = channel.close().await;
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    session_id: String,
    kind: String,
    sdp: String,
}

/// Closes the peer connection when the pipe or a failed setup drops it.
struct PeerGuard(Arc<dyn PeerConnection>);

impl Drop for PeerGuard {
    fn drop(&mut self) {
        let peer = self.0.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, peer.close()).await;
            });
        }
    }
}

/// Offers one reliable ordered channel labelled with the tunnel protocol. The device
/// admits the offer with the certificate that the Noise hello carries afterwards.
pub(crate) async fn connect(
    relay: &mut Relay,
    admission: &DeviceSignalingResponse,
    certified: &CertifiedHandshake,
    grant_id: &str,
    device_id: &str,
) -> Result<Pipe> {
    let unreachable = |message: String| Error::Unreachable {
        device_id: device_id.into(),
        message: format!("direct WebRTC connection failed: {message}"),
    };
    let failed = CancellationToken::new();
    let (gathered, gathering) = watch::channel(false);
    let handler = Handler {
        gathered,
        failed: failed.clone(),
    };
    let peer = build_peer(admission, handler).await.map_err(unreachable)?;
    let guard = PeerGuard(peer.clone());
    let setup = async {
        let (channel, sdp) = create_offer(&peer, gathering).await?;
        let answer = exchange(relay, certified, grant_id, sdp).await?;
        peer.set_remote_description(answer)
            .await
            .map_err(|error| format!("applying the device answer: {error}"))?;
        opened(&channel).await?;
        Ok::<_, String>(channel)
    };
    let channel = tokio::select! {
        _ = failed.cancelled() => return Err(unreachable("the peer connection failed".into())),
        result = tokio::time::timeout(SETUP_TIMEOUT, setup) => result
            .map_err(|_| unreachable(format!("no open data channel within {SETUP_TIMEOUT:?}")))?
            .map_err(unreachable)?,
    };
    Ok(pipe(channel, guard, failed))
}

async fn build_peer(
    admission: &DeviceSignalingResponse,
    handler: Handler,
) -> Result<Arc<dyn PeerConnection>, String> {
    let mut settings = SettingEngine::default();
    settings.set_include_loopback_candidate(cfg!(test));
    let addresses = if cfg!(test) {
        vec!["127.0.0.1:0".to_owned()]
    } else {
        vec!["0.0.0.0:0".to_owned(), "[::]:0".to_owned()]
    };
    let configuration = RTCConfigurationBuilder::new()
        .with_ice_servers(ice_servers(admission, unix_now())?)
        .build();
    let peer = PeerConnectionBuilder::new()
        .with_configuration(configuration)
        .with_setting_engine(settings)
        .with_handler(Arc::new(handler))
        .with_dedicated_reactor_thread(true)
        .with_reactor_pool_size(1)
        .with_udp_addrs(addresses)
        .with_sctp_receive_buffer_size(BUFFER_BYTES as u32)
        .with_data_channel_send_buffer_limit(BUFFER_BYTES)
        .build()
        .await
        .map_err(|error| format!("building the peer connection: {error}"))?;
    Ok(Arc::new(peer))
}

async fn create_offer(
    peer: &Arc<dyn PeerConnection>,
    gathering: watch::Receiver<bool>,
) -> Result<(Arc<dyn DataChannel>, String), String> {
    let init = RTCDataChannelInit {
        protocol: TUNNEL_PROTOCOL.into(),
        ..Default::default()
    };
    let channel = peer
        .create_data_channel(TUNNEL_PROTOCOL, Some(init))
        .await
        .map_err(|error| format!("creating the data channel: {error}"))?;
    let offer = peer
        .create_offer(None)
        .await
        .map_err(|error| format!("creating the offer: {error}"))?;
    peer.set_local_description(offer)
        .await
        .map_err(|error| format!("applying the offer: {error}"))?;
    Ok((channel, gathered_offer(peer, gathering).await?))
}

/// The agent takes one complete offer, so every candidate is gathered first.
async fn gathered_offer(
    peer: &Arc<dyn PeerConnection>,
    mut gathering: watch::Receiver<bool>,
) -> Result<String, String> {
    while !*gathering.borrow_and_update() {
        gathering
            .changed()
            .await
            .map_err(|_| "ICE gathering stopped".to_owned())?;
    }
    let sdp = peer
        .local_description()
        .await
        .ok_or_else(|| "the offer disappeared after ICE gathering".to_owned())?
        .sdp;
    check_offer(&sdp)?;
    Ok(sdp)
}

/// Sends the offer on the signal channel of the relay and waits for its answer.
async fn exchange(
    relay: &mut Relay,
    certified: &CertifiedHandshake,
    grant_id: &str,
    sdp: String,
) -> Result<RTCSessionDescription, String> {
    let session_id = &certified.certificate.session_id;
    let offer = serde_json::to_vec(&serde_json::json!({
        "kind": "offer",
        "protocol": TUNNEL_PROTOCOL,
        "session_id": session_id,
        "grant_id": grant_id,
        "certificate_jws": certified.certificate_jws,
        "sdp": sdp,
    }))
    .map_err(|error| format!("encoding the offer: {error}"))?;
    relay
        .send_signal(offer)
        .await
        .map_err(|error| error.to_string())?;
    let answer = relay
        .next_signal()
        .await
        .map_err(|error| error.to_string())?;
    let answer: Answer = serde_json::from_slice(&answer)
        .map_err(|error| format!("the device answer is malformed: {error}"))?;
    if answer.kind != "answer" || answer.session_id != *session_id {
        return Err(format!(
            "the device answered {} for session {}, not this offer",
            answer.kind, answer.session_id
        ));
    }
    RTCSessionDescription::answer(answer.sdp)
        .map_err(|error| format!("the device answer is invalid: {error}"))
}

async fn opened(channel: &Arc<dyn DataChannel>) -> Result<(), String> {
    loop {
        match channel.poll().await {
            Some(DataChannelEvent::OnOpen) => return Ok(()),
            Some(
                DataChannelEvent::OnClose | DataChannelEvent::OnClosing | DataChannelEvent::OnError,
            )
            | None => return Err("the data channel closed while opening".into()),
            Some(_) => {}
        }
    }
}

/// The agent never answers an offer that fails its `validate_offer`, so such an offer
/// would only stall the connect until the setup timeout.
fn check_offer(sdp: &str) -> Result<(), String> {
    let media: Vec<&str> = sdp.lines().filter(|line| line.starts_with("m=")).collect();
    let candidates = sdp
        .lines()
        .filter(|line| line.starts_with("a=candidate:"))
        .count();
    let data_channel_only = matches!(media.as_slice(), [line] if line.starts_with("m=application ") && line.contains("UDP/DTLS/SCTP"));
    if sdp.len() > MAX_OFFER_BYTES
        || sdp.contains('\0')
        || !data_channel_only
        || candidates > MAX_OFFER_CANDIDATES
    {
        return Err(format!(
            "the offer has {} bytes, {} media sections and {candidates} candidates; the device accepts {MAX_OFFER_BYTES} bytes, one data channel section and {MAX_OFFER_CANDIDATES} candidates",
            sdp.len(),
            media.len()
        ));
    }
    Ok(())
}

fn pipe(channel: Arc<dyn DataChannel>, guard: PeerGuard, failed: CancellationToken) -> Pipe {
    let (outgoing, mut outbox) = mpsc::channel::<Vec<u8>>(PIPE_DEPTH);
    let (inbox, incoming) = mpsc::channel(PIPE_DEPTH);
    let reader = tokio::spawn(receive(channel.clone(), inbox, failed));
    let writer = tokio::spawn(async move {
        while let Some(bytes) = outbox.recv().await {
            if channel
                .send(BytesMut::from(bytes.as_slice()))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    Pipe::new(
        TransportKind::WebRtc,
        outgoing,
        incoming,
        writer,
        (guard, AbortOnDrop(reader)),
    )
}

/// Ends with the channel, with the peer connection, or at the first message that is not a
/// bounded binary envelope.
async fn receive(
    channel: Arc<dyn DataChannel>,
    inbox: mpsc::Sender<Vec<u8>>,
    failed: CancellationToken,
) {
    loop {
        let event = tokio::select! {
            _ = failed.cancelled() => return,
            event = channel.poll() => event,
        };
        match event {
            Some(DataChannelEvent::OnMessage(message))
                if !message.is_string && message.data.len() <= TUNNEL_MAX_ENVELOPE =>
            {
                if inbox.send(message.data.to_vec()).await.is_err() {
                    return;
                }
            }
            Some(
                DataChannelEvent::OnOpen
                | DataChannelEvent::OnBufferedAmountLow
                | DataChannelEvent::OnBufferedAmountHigh,
            ) => {}
            _ => return,
        }
    }
}

/// Expired TURN credentials leave host and server-reflexive candidates usable.
fn ice_servers(admission: &DeviceSignalingResponse, now: i64) -> Result<Vec<RTCIceServer>, String> {
    if admission.ice_servers.len() > MAX_ICE_SERVERS {
        return Err(format!(
            "the admission lists {} ICE servers, more than {MAX_ICE_SERVERS}",
            admission.ice_servers.len()
        ));
    }
    if admission
        .ice_expires_at
        .is_some_and(|expires| expires <= now + 30)
    {
        return Ok(Vec::new());
    }
    admission.ice_servers.iter().map(ice_server).collect()
}

fn ice_server(server: &DeviceIceServer) -> Result<RTCIceServer, String> {
    let bounded = (1..=16).contains(&server.urls.len())
        && server.urls.iter().all(|url| url.len() <= 2048)
        && server
            .username
            .as_ref()
            .is_none_or(|value| value.len() <= 1024)
        && server
            .credential
            .as_ref()
            .is_none_or(|value| value.len() <= 4096);
    if !bounded {
        return Err("an ICE server entry exceeds its size bounds".to_owned());
    }
    let server = RTCIceServer {
        urls: server.urls.clone(),
        username: server.username.clone().unwrap_or_default(),
        credential: server.credential.clone().unwrap_or_default(),
    };
    server
        .urls()
        .map_err(|error| format!("invalid ICE server URL: {error}"))?;
    Ok(server)
}
