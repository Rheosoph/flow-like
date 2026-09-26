use super::{
    HANDSHAKE_TIMEOUT, IO_TIMEOUT, NoiseConnection,
    wire::{self, Channel, NoiseEnvelope, SignalEnvelope},
};
use crate::enrollment::unix_time;
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::DeviceSignalingResponse;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use webrtc::{
    data_channel::{DataChannel, DataChannelEvent},
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
        RTCIceGatheringState, RTCIceServer, RTCPeerConnectionState, RTCSessionDescription,
        SettingEngine,
    },
};

struct Handler {
    gathered: watch::Sender<bool>,
    channel: mpsc::Sender<Arc<dyn DataChannel>>,
    accepted: AtomicBool,
    closed: CancellationToken,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.gathered.send(true);
        }
    }
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        if matches!(
            state,
            RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed
        ) {
            self.closed.cancel();
        }
    }
    async fn on_data_channel(&self, channel: Arc<dyn DataChannel>) {
        if self.accepted.swap(true, Ordering::AcqRel)
            || self.channel.try_send(channel.clone()).is_err()
        {
            self.closed.cancel();
            let _ = channel.close().await;
        }
    }
}

pub(super) async fn serve(
    offer: SignalEnvelope,
    mut noise: NoiseConnection,
    controller: &str,
    admission: Arc<DeviceSignalingResponse>,
    outbound: mpsc::Sender<String>,
    cancel: CancellationToken,
) -> Result<()> {
    let SignalEnvelope::Offer {
        session_id, sdp, ..
    } = offer;
    let ice_servers = ice_servers(&admission, unix_time()?)?;
    let (gathered_sender, mut gathered) = watch::channel(false);
    let (channel_sender, mut channels) = mpsc::channel(1);
    let closed = cancel.child_token();
    let handler = Arc::new(Handler {
        gathered: gathered_sender,
        channel: channel_sender,
        accepted: AtomicBool::new(false),
        closed: closed.clone(),
    });
    let configuration = RTCConfigurationBuilder::new()
        .with_ice_servers(ice_servers)
        .build();
    let mut settings = SettingEngine::default();
    settings.set_include_loopback_candidate(cfg!(test));
    let peer = tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        result = tokio::time::timeout(IO_TIMEOUT, PeerConnectionBuilder::new()
            .with_configuration(configuration)
            .with_setting_engine(settings)
            .with_handler(handler)
            // This crate's pooled reactor closes its driver when a cancelled build is dropped.
            .with_dedicated_reactor_thread(true)
            .with_reactor_pool_size(1)
            .with_udp_addrs(vec!["0.0.0.0:0".to_owned(), "[::]:0".to_owned()])
            .with_sctp_receive_buffer_size(64 * 1024)
            .with_data_channel_send_buffer_limit(64 * 1024)
            .build()) => result??,
    };
    let result: Result<()> = async {
        let setup = async {
            peer.set_remote_description(RTCSessionDescription::offer(sdp)?).await?;
            let answer = peer.create_answer(None).await?;
            peer.set_local_description(answer).await?;
            while !*gathered.borrow_and_update() { gathered.changed().await?; }
            let answer = peer.local_description().await.context("Missing WebRTC answer")?;
            let payload = wire::serialize(&serde_json::json!({ "session_id":session_id, "kind":"answer", "sdp":answer.sdp }))?;
            let frame = wire::outbound(controller, Channel::Signal, &payload)?;
            tokio::time::timeout(IO_TIMEOUT, outbound.send(frame)).await??;
            Ok::<_, anyhow::Error>(())
        };
        tokio::select! {
            _ = closed.cancelled() => return Ok(()),
            result = tokio::time::timeout(Duration::from_secs(15), setup) => result??,
        }
        let channel = tokio::select! {
            _ = closed.cancelled() => return Ok(()),
            result = tokio::time::timeout(HANDSHAKE_TIMEOUT, channels.recv()) => result?.context("WebRTC data channel missing")?,
        };
        validate_channel(&channel).await?;
        loop {
            let event = tokio::select! {
                _ = closed.cancelled() => break,
                result = tokio::time::timeout(noise.timeout(), channel.poll()) => result?.context("WebRTC data channel closed")?,
            };
            match event {
                DataChannelEvent::OnMessage(message) => {
                    ensure!(message.is_string, "Management data channel requires text envelopes");
                    let envelope = NoiseEnvelope::parse(&message.data)?;
                    let response = noise.receive(envelope).await?;
                    let text = std::str::from_utf8(&response)?;
                    tokio::select! {
                        _ = closed.cancelled() => break,
                        result = tokio::time::timeout(IO_TIMEOUT, channel.send_text(text)) => result??,
                    }
                }
                DataChannelEvent::OnError | DataChannelEvent::OnClosing | DataChannelEvent::OnClose => break,
                _ => {},
            }
        }
        Ok(())
    }.await;
    closed.cancel();
    let _ = tokio::time::timeout(IO_TIMEOUT, peer.close()).await;
    result
}

async fn validate_channel(channel: &Arc<dyn DataChannel>) -> Result<()> {
    ensure!(
        channel.label().await? == wire::PROTOCOL && channel.protocol().await? == wire::PROTOCOL,
        "Unexpected management data channel"
    );
    ensure!(
        channel.ordered().await?
            && channel.max_packet_life_time().await?.is_none()
            && channel.max_retransmits().await?.is_none()
            && !channel.negotiated().await?,
        "Management requires a reliable ordered data channel"
    );
    Ok(())
}

fn ice_servers(admission: &DeviceSignalingResponse, now: i64) -> Result<Vec<RTCIceServer>> {
    ensure!(
        admission.ice_servers.len() <= 16,
        "Too many management ICE servers"
    );
    // Expired TURN credentials do not prevent host-candidate or WebSocket reachability.
    if admission
        .ice_expires_at
        .is_some_and(|expires| expires <= now + 30)
    {
        return Ok(Vec::new());
    }
    admission
        .ice_servers
        .iter()
        .map(|server| {
            ensure!(
                !server.urls.is_empty()
                    && server.urls.len() <= 16
                    && server.urls.iter().all(|url| url.len() <= 2048),
                "Invalid management ICE configuration"
            );
            ensure!(
                server
                    .username
                    .as_ref()
                    .is_none_or(|value| value.len() <= 1024)
                    && server
                        .credential
                        .as_ref()
                        .is_none_or(|value| value.len() <= 4096),
                "Invalid management ICE credential size"
            );
            let server = RTCIceServer {
                urls: server.urls.clone(),
                username: server.username.clone().unwrap_or_default(),
                credential: server.credential.clone().unwrap_or_default(),
            };
            server.urls()?;
            Ok(server)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::DeviceIceServer;

    #[test]
    fn stale_turn_credentials_are_not_used_for_new_peers() {
        let mut response = DeviceSignalingResponse {
            token: "redacted".into(),
            expires_at: 400,
            device_auth_epoch: 1,
            signaling_urls: vec![],
            ice_servers: vec![DeviceIceServer {
                urls: vec!["turn:relay.example:3478".into()],
                username: Some("temporary".into()),
                credential: Some("temporary".into()),
            }],
            ice_expires_at: Some(130),
            policy_version: 0,
            policy_digest: None,
        };
        assert!(ice_servers(&response, 100).unwrap().is_empty());
        response.ice_expires_at = Some(400);
        assert_eq!(ice_servers(&response, 100).unwrap().len(), 1);
        response.ice_servers[0].urls = vec!["https://not-ice.example".into()];
        assert!(ice_servers(&response, 100).is_err());
    }

    async fn message(channel: &Arc<dyn DataChannel>) -> Result<NoiseEnvelope> {
        loop {
            match tokio::time::timeout(Duration::from_secs(10), channel.poll())
                .await?
                .context("Data channel closed")?
            {
                DataChannelEvent::OnMessage(message) => return NoiseEnvelope::parse(&message.data),
                DataChannelEvent::OnError | DataChannelEvent::OnClose => {
                    anyhow::bail!("Data channel failed")
                }
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn authenticated_noise_commands_survive_signaling_output_loss_on_real_rtc() -> Result<()>
    {
        use flow_like_device_protocol::{ManagementCommand, ManagementRequest};
        use webrtc::data_channel::RTCDataChannelInit;
        let fixture = super::super::tests::fixture()?;
        let _directory = fixture.directory;
        let mut initiator = fixture.initiator;
        let (gathered_sender, mut gathered) = watch::channel(false);
        let (channels, _unused) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let mut settings = SettingEngine::default();
        settings.set_include_loopback_candidate(true);
        let controller = PeerConnectionBuilder::new()
            .with_setting_engine(settings)
            .with_handler(Arc::new(Handler {
                gathered: gathered_sender,
                channel: channels,
                accepted: AtomicBool::new(false),
                closed: cancel.child_token(),
            }))
            .with_udp_addrs(vec!["127.0.0.1:0".to_owned()])
            .with_data_channel_send_buffer_limit(64 * 1024)
            .build()
            .await?;
        let channel = controller
            .create_data_channel(
                wire::PROTOCOL,
                Some(RTCDataChannelInit {
                    protocol: wire::PROTOCOL.into(),
                    ..Default::default()
                }),
            )
            .await?;
        controller
            .set_local_description(controller.create_offer(None).await?)
            .await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            while !*gathered.borrow_and_update() {
                gathered.changed().await?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        let offer = controller.local_description().await.context("No offer")?;
        let noise = NoiseConnection::new(
            &fixture.management,
            "session",
            "owner",
            &fixture.certificate,
        )?;
        let now = unix_time()?;
        let admission = Arc::new(DeviceSignalingResponse {
            token: "redacted".into(),
            expires_at: now + 300,
            device_auth_epoch: 1,
            signaling_urls: vec![],
            ice_servers: vec![],
            ice_expires_at: None,
            policy_version: 0,
            policy_digest: None,
        });
        let (output, mut received) = mpsc::channel(4);
        let task_cancel = cancel.child_token();
        let certificate = fixture.certificate.clone();
        let serving = tokio::spawn(async move {
            serve(
                SignalEnvelope::Offer {
                    session_id: "session".into(),
                    sdp: offer.sdp,
                    grant_id: "owner".into(),
                    certificate_jws: certificate,
                },
                noise,
                "controller",
                admission,
                output,
                task_cancel,
            )
            .await
        });
        let answer = tokio::time::timeout(Duration::from_secs(20), received.recv())
            .await?
            .context("No answer")?;
        let answer: serde_json::Value = serde_json::from_str(&answer)?;
        let answer: serde_json::Value = serde_json::from_slice(&wire::decode(
            answer["payload"].as_str().context("No answer payload")?,
        )?)?;
        controller
            .set_remote_description(RTCSessionDescription::answer(
                answer["sdp"].as_str().context("No answer SDP")?.to_owned(),
            )?)
            .await?;
        loop {
            if matches!(
                tokio::time::timeout(Duration::from_secs(10), channel.poll())
                    .await?
                    .context("No data channel open")?,
                DataChannelEvent::OnOpen
            ) {
                break;
            }
        }
        let hello = NoiseEnvelope::Hello {
            session_id: "session".into(),
            grant_id: "owner".into(),
            certificate_jws: fixture.certificate,
            data: wire::encode(&initiator.write()?),
        };
        channel
            .send_text(std::str::from_utf8(&wire::serialize(&hello)?)?)
            .await?;
        let NoiseEnvelope::Handshake { data, .. } = message(&channel).await? else {
            anyhow::bail!("No Noise handshake");
        };
        initiator.read(&wire::decode(&data)?)?;
        let handshake = NoiseEnvelope::Handshake {
            session_id: "session".into(),
            data: wire::encode(&initiator.write()?),
        };
        let mut session = initiator.finish()?;
        channel
            .send_text(std::str::from_utf8(&wire::serialize(&handshake)?)?)
            .await?;
        let NoiseEnvelope::Message { data, .. } = message(&channel).await? else {
            anyhow::bail!("No encrypted ready");
        };
        let ready: serde_json::Value =
            serde_json::from_slice(&session.decrypt(&wire::decode(&data)?)?)?;
        assert_eq!(ready["ready"], true);
        // The signaling output can disappear after establishment without changing Noise state.
        drop(received);
        let request = ManagementRequest {
            operation_id: "inspect-one".into(),
            device_id: "device".into(),
            issued_at: now,
            expires_at: now + 60,
            command: ManagementCommand::Inspect,
        };
        let encrypted = session.encrypt(&serde_json::to_vec(&request)?)?;
        let envelope = NoiseEnvelope::Message {
            session_id: "session".into(),
            data: wire::encode(&encrypted),
        };
        channel
            .send_text(std::str::from_utf8(&wire::serialize(&envelope)?)?)
            .await?;
        let NoiseEnvelope::Message { data, .. } = message(&channel).await? else {
            anyhow::bail!("No encrypted command response");
        };
        let response: serde_json::Value =
            serde_json::from_slice(&session.decrypt(&wire::decode(&data)?)?)?;
        assert_eq!(response["state"], "completed");
        assert_eq!(response["result"]["device_id"], "device");
        cancel.cancel();
        controller.close().await?;
        tokio::time::timeout(Duration::from_secs(15), serving).await???;
        Ok(())
    }
}
