use anyhow::ensure;
use bytes::BytesMut;
use flow_like_bot_machine::stream::{Pace, Playground, serve};
use flow_like_types::{Context, Error, Result};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot, watch},
};
use tokio_util::sync::CancellationToken;
use webrtc::{
    data_channel::{DataChannel, DataChannelEvent},
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
        RTCIceGatheringState, RTCIceServer, RTCPeerConnectionState, RTCSessionDescription,
        SettingEngine,
    },
};

use super::records::{MAX_PLAINTEXT, Opener, Sealer, StreamKeys};

pub(super) const PROTOCOL: &str = "flowlike.bot-screen.v1";
const SETUP_TIMEOUT: Duration = Duration::from_secs(15);
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const BUFFER_BYTES: usize = 1024 * 1024;
const PIPE_BYTES: usize = 64 * 1024;

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

async fn validate_channel(channel: &Arc<dyn DataChannel>) -> Result<()> {
    ensure!(
        channel.label().await? == PROTOCOL && channel.protocol().await? == PROTOCOL,
        "Unexpected bot screen data channel"
    );
    ensure!(
        channel.ordered().await?
            && channel.max_packet_life_time().await?.is_none()
            && channel.max_retransmits().await?.is_none()
            && !channel.negotiated().await?,
        "Bot screens require a reliable ordered data channel"
    );
    Ok(())
}

/// Answers one viewer's offer: the answer SDP goes out through `answered`, then the viewer's data
/// channel carries the encrypted playground stream of `bot_id` until either side ends it.
pub(super) async fn serve_offer<P: Playground>(
    offer_sdp: String,
    ice_servers: Vec<RTCIceServer>,
    keys: StreamKeys,
    host: P,
    bot_id: String,
    answered: oneshot::Sender<String>,
    cancel: CancellationToken,
) -> Result<()> {
    let (gathered_sender, mut gathered) = watch::channel(false);
    let (channel_sender, mut channels) = mpsc::channel(1);
    let closed = cancel.child_token();
    let handler = Arc::new(Handler {
        gathered: gathered_sender,
        channel: channel_sender,
        accepted: AtomicBool::new(false),
        closed: closed.clone(),
    });
    let mut settings = SettingEngine::default();
    settings.set_include_loopback_candidate(cfg!(test));
    let setup = async {
        let peer = PeerConnectionBuilder::new()
            .with_configuration(
                RTCConfigurationBuilder::new()
                    .with_ice_servers(ice_servers)
                    .build(),
            )
            .with_setting_engine(settings)
            .with_handler(handler)
            .with_dedicated_reactor_thread(true)
            .with_reactor_pool_size(1)
            .with_udp_addrs(vec!["0.0.0.0:0".to_owned(), "[::]:0".to_owned()])
            .with_sctp_receive_buffer_size(BUFFER_BYTES as u32)
            .with_data_channel_send_buffer_limit(BUFFER_BYTES)
            .build()
            .await
            .context("Build WebRTC peer for a bot screen")?;
        let negotiated = async {
            peer.set_remote_description(RTCSessionDescription::offer(offer_sdp)?)
                .await?;
            let answer = peer.create_answer(None).await?;
            peer.set_local_description(answer).await?;
            while !*gathered.borrow_and_update() {
                gathered.changed().await?;
            }
            let answer = peer
                .local_description()
                .await
                .context("Missing WebRTC answer")?;
            let _ = answered.send(answer.sdp);
            let channel = channels
                .recv()
                .await
                .context("The viewer opened no data channel")?;
            validate_channel(&channel).await?;
            Ok::<_, Error>(channel)
        };
        let channel = tokio::select! {
            _ = closed.cancelled() => None,
            result = tokio::time::timeout(SETUP_TIMEOUT, negotiated) => Some(
                result.context("Bot screen WebRTC setup timed out")??
            ),
        };
        Ok::<_, Error>((peer, channel))
    };
    let (peer, channel) = match setup.await {
        Ok(parts) => parts,
        Err(error) => {
            closed.cancel();
            return Err(error);
        }
    };
    let result = match channel {
        Some(channel) => bridge(channel, keys, host, bot_id, closed.clone()).await,
        None => Ok(()),
    };
    closed.cancel();
    let _ = tokio::time::timeout(WRITE_TIMEOUT, peer.close()).await;
    result
}

async fn next_record(channel: &Arc<dyn DataChannel>) -> Result<Option<BytesMut>> {
    loop {
        match channel
            .poll()
            .await
            .context("Bot screen data channel closed")?
        {
            DataChannelEvent::OnMessage(message) => {
                ensure!(!message.is_string, "Bot screens carry binary messages only");
                return Ok(Some(message.data));
            }
            DataChannelEvent::OnError | DataChannelEvent::OnClosing | DataChannelEvent::OnClose => {
                return Ok(None);
            }
            _ => {}
        }
    }
}

async fn confirm_key(channel: &Arc<dyn DataChannel>, opener: &mut Opener) -> Result<()> {
    let record = tokio::time::timeout(CONFIRM_TIMEOUT, next_record(channel))
        .await
        .context("The viewer did not confirm the bot screen key in time")??
        .context("The viewer closed the bot screen channel before confirming the key")?;
    ensure!(
        opener.open(&record)?.is_empty(),
        "The first bot screen record must be empty"
    );
    Ok(())
}

async fn bridge<P: Playground>(
    channel: Arc<dyn DataChannel>,
    keys: StreamKeys,
    host: P,
    bot_id: String,
    cancel: CancellationToken,
) -> Result<()> {
    let (mut sealer, mut opener): (Sealer, Opener) = keys.host();
    confirm_key(&channel, &mut opener).await?;
    let (mine, theirs) = tokio::io::duplex(PIPE_BYTES);
    let (mut from_stream, mut to_stream) = tokio::io::split(mine);
    let serving = serve(theirs, host, bot_id, true, Pace::default());
    let inbound = async {
        while let Some(record) = next_record(&channel).await? {
            let plaintext = opener.open(&record)?;
            if !plaintext.is_empty() {
                to_stream
                    .write_all(&plaintext)
                    .await
                    .context("Pass viewer bytes to the stream")?;
            }
        }
        Ok::<(), Error>(())
    };
    let outbound = async {
        let mut buffer = vec![0u8; MAX_PLAINTEXT];
        loop {
            let read = from_stream
                .read(&mut buffer)
                .await
                .context("Read the playground stream")?;
            if read == 0 {
                return Ok::<(), Error>(());
            }
            let record = sealer.seal(&buffer[..read])?;
            tokio::time::timeout(WRITE_TIMEOUT, channel.send(BytesMut::from(&record[..])))
                .await
                .context("Send to the viewer timed out")?
                .context("Send to the viewer")?;
        }
    };
    tokio::select! {
        _ = cancel.cancelled() => Ok(()),
        result = serving => result,
        result = inbound => result,
        result = outbound => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_bot_machine::stream::LiveFrame;
    use flow_like_device_protocol::{
        PlaygroundControl, PlaygroundDecoder, PlaygroundDisplay, PlaygroundEvent, PlaygroundInput,
    };
    use tokio::task::JoinHandle;
    use webrtc::data_channel::RTCDataChannelInit;

    #[derive(Clone)]
    struct Quiet;

    impl Playground for Quiet {
        fn latest(&self, _: &str) -> Option<LiveFrame> {
            None
        }
        async fn refresh(&self, _: &str) -> Option<LiveFrame> {
            None
        }
        async fn capture(&self, _: &str) -> Option<LiveFrame> {
            None
        }
        fn is_taken_over(&self, _: &str) -> bool {
            false
        }
        fn take_over(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn hand_back(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn inject(&self, _: &str, _: PlaygroundInput) -> Result<()> {
            Ok(())
        }
        async fn stop(&self, _: &str) {}
        fn display(&self) -> PlaygroundDisplay {
            PlaygroundDisplay::None
        }
    }

    struct Viewer {
        peer: Arc<dyn PeerConnection>,
        channel: Arc<dyn DataChannel>,
        sealer: Sealer,
        opener: Opener,
        serving: JoinHandle<Result<()>>,
    }

    async fn connect(
        host_keys: StreamKeys,
        viewer_keys: StreamKeys,
        label: &str,
    ) -> Result<Viewer> {
        let (gathered_sender, mut gathered) = watch::channel(false);
        let (channels, _unused) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let mut settings = SettingEngine::default();
        settings.set_include_loopback_candidate(true);
        let peer: Arc<dyn PeerConnection> = Arc::new(
            PeerConnectionBuilder::new()
                .with_setting_engine(settings)
                .with_handler(Arc::new(Handler {
                    gathered: gathered_sender,
                    channel: channels,
                    accepted: AtomicBool::new(false),
                    closed: cancel.child_token(),
                }))
                .with_udp_addrs(vec!["127.0.0.1:0".to_owned()])
                .build()
                .await?,
        );
        let channel = peer
            .create_data_channel(
                label,
                Some(RTCDataChannelInit {
                    protocol: label.into(),
                    ..Default::default()
                }),
            )
            .await?;
        peer.set_local_description(peer.create_offer(None).await?)
            .await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            while !*gathered.borrow_and_update() {
                gathered.changed().await?;
            }
            Ok::<_, Error>(())
        })
        .await??;
        let offer = peer.local_description().await.context("No offer")?;
        let (answer_sender, answer) = oneshot::channel();
        let serving = tokio::spawn(serve_offer(
            offer.sdp,
            Vec::new(),
            host_keys,
            Quiet,
            "bot_1".into(),
            answer_sender,
            cancel,
        ));
        let sdp = tokio::time::timeout(Duration::from_secs(20), answer)
            .await?
            .context("The host never answered")?;
        peer.set_remote_description(RTCSessionDescription::answer(sdp)?)
            .await?;
        loop {
            if matches!(
                tokio::time::timeout(Duration::from_secs(10), channel.poll())
                    .await?
                    .context("The data channel never opened")?,
                DataChannelEvent::OnOpen
            ) {
                break;
            }
        }
        let (sealer, opener) = viewer_keys.viewer();
        Ok(Viewer {
            peer,
            channel,
            sealer,
            opener,
            serving,
        })
    }

    async fn send(viewer: &mut Viewer, plaintext: &[u8]) -> Result<()> {
        let record = viewer.sealer.seal(plaintext)?;
        viewer.channel.send(BytesMut::from(&record[..])).await?;
        Ok(())
    }

    async fn event(
        viewer: &mut Viewer,
        decoder: &mut PlaygroundDecoder,
    ) -> Result<PlaygroundEvent> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(event) = decoder.next_event()? {
                    return Ok(event);
                }
                match viewer
                    .channel
                    .poll()
                    .await
                    .context("The data channel closed")?
                {
                    DataChannelEvent::OnMessage(message) => {
                        let plaintext = viewer.opener.open(&message.data)?;
                        decoder.push(&plaintext);
                    }
                    DataChannelEvent::OnClose | DataChannelEvent::OnError => {
                        return Err(anyhow::anyhow!("The data channel failed"));
                    }
                    _ => {}
                }
            }
        })
        .await?
    }

    fn keys(host_secret: u8) -> (StreamKeys, StreamKeys) {
        let make =
            |host: u8| StreamKeys::derive(&[0x11; 32], &[host; 32], "session0123456789").unwrap();
        (make(host_secret), make(0x22))
    }

    #[tokio::test]
    async fn a_confirmed_viewer_receives_the_encrypted_playground_stream() -> Result<()> {
        let (host_keys, viewer_keys) = keys(0x22);
        let mut viewer = connect(host_keys, viewer_keys, PROTOCOL).await?;
        send(&mut viewer, &[]).await?;
        let mut decoder = PlaygroundDecoder::default();
        assert!(matches!(
            event(&mut viewer, &mut decoder).await?,
            PlaygroundEvent::State(state) if state.asleep
        ));
        send(&mut viewer, &PlaygroundControl::TakeOver.encode()?).await?;
        viewer.peer.close().await?;
        let _ = tokio::time::timeout(Duration::from_secs(10), viewer.serving).await;
        Ok(())
    }

    #[tokio::test]
    async fn a_viewer_with_the_wrong_key_never_starts_the_stream() -> Result<()> {
        let (host_keys, viewer_keys) = keys(0x33);
        let mut viewer = connect(host_keys, viewer_keys, PROTOCOL).await?;
        send(&mut viewer, &[]).await?;
        let ended = tokio::time::timeout(Duration::from_secs(10), &mut viewer.serving).await??;
        assert!(ended.is_err());
        viewer.peer.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn another_channel_label_is_refused() -> Result<()> {
        let (host_keys, viewer_keys) = keys(0x22);
        let mut viewer = connect(host_keys, viewer_keys, "something-else").await?;
        let ended = tokio::time::timeout(Duration::from_secs(10), &mut viewer.serving).await??;
        assert!(ended.is_err());
        viewer.peer.close().await?;
        Ok(())
    }
}
