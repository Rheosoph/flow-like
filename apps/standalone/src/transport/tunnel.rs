#[cfg(feature = "runtime")]
mod gateway;
mod streams;
#[cfg(test)]
mod tests;
mod tls;

use super::{HANDSHAKE_TIMEOUT, IO_TIMEOUT, wire};
#[cfg(feature = "runtime")]
use crate::management::tunnel::{GatewayTarget, ModelAssetTarget};
use crate::{
    crypto::noise,
    management::{
        ManagementService, RejectionCode, rejection_code,
        tunnel::{
            InternalTarget, LiveTunnelAuthority, OpenTarget, ServiceTarget, TunnelAuthority,
            bounded_response, live_authority,
        },
    },
};
use anyhow::{Context, Result, ensure};
#[cfg(feature = "runtime")]
use flow_like_device_protocol::{ModelAssetState, ModelAssetStatus};
use flow_like_device_protocol::{
    TUNNEL_INITIAL_WINDOW as INITIAL_WINDOW, TUNNEL_MAX_DATA as MAX_DATA, TUNNEL_MAX_STREAMS,
    TunnelEnvelope, TunnelEnvelopeBody, TunnelFrame, TunnelFrameBody, TunnelRenewed, TunnelReset,
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::Instant};
use tokio_util::sync::CancellationToken;

pub(super) struct Connection {
    authority: TunnelAuthority,
    handshake: noise::Handshake,
    session_id: String,
    certificate_jws: String,
    grant_id: String,
}

impl Connection {
    pub(super) fn new(
        management: &Arc<ManagementService>,
        session_id: &str,
        grant_id: &str,
        certificate_jws: &str,
    ) -> Result<Self> {
        let (authority, handshake) =
            management.connect_tunnel(certificate_jws, grant_id, session_id)?;
        Ok(Self {
            authority,
            handshake,
            session_id: session_id.into(),
            grant_id: grant_id.into(),
            certificate_jws: certificate_jws.into(),
        })
    }

    pub(super) async fn serve<T: AsRef<[u8]> + Send>(
        mut self,
        mut input: mpsc::Receiver<T>,
        output: mpsc::Sender<Vec<u8>>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let setup = async {
            let envelope = receive(&mut input, &self.session_id).await?;
            let TunnelEnvelopeBody::Hello(hello) = envelope else {
                anyhow::bail!("Expected tunnel hello")
            };
            ensure!(
                hello.grant_id == self.grant_id && hello.certificate_jws == self.certificate_jws,
                "Tunnel hello binding mismatch"
            );
            self.handshake.read(&wire::decode(&hello.data)?)?;
            send_envelope(
                &output,
                &self.session_id,
                TunnelEnvelopeBody::Handshake(self.handshake.write()?),
            )
            .await?;
            let TunnelEnvelopeBody::Handshake(bytes) =
                receive(&mut input, &self.session_id).await?
            else {
                anyhow::bail!("Expected tunnel handshake")
            };
            self.handshake.read(&bytes)?;
            self.authority.check()?;
            Ok::<_, anyhow::Error>(())
        };
        tokio::select! { _ = cancel.cancelled() => return Ok(()), result = tokio::time::timeout(HANDSHAKE_TIMEOUT, setup) => result?? }
        let session = self.handshake.finish()?;
        let access = Arc::new(std::sync::RwLock::new(self.authority.clone()));
        let mut pump = Pump {
            authority: self.authority,
            session,
            session_id: self.session_id,
            output,
            sent: 0,
            received: 0,
            streams: HashMap::new(),
            last_stream_id: 0,
            renewal: None,
            ping: None,
            last_received: Instant::now(),
            access,
        };
        pump.send(
            0,
            TunnelFrameBody::Renewed(TunnelRenewed {
                expires_at: pump.authority.expires_at(),
            }),
        )
        .await?;
        pump.run(input, cancel).await
    }
}

struct Renewal {
    authority: TunnelAuthority,
    handshake: noise::Handshake,
    deadline: Instant,
    pending_ping: Option<[u8; 8]>,
}

struct Pump {
    authority: TunnelAuthority,
    session: noise::Session,
    session_id: String,
    output: mpsc::Sender<Vec<u8>>,
    sent: u64,
    received: u64,
    streams: HashMap<u32, streams::Stream>,
    last_stream_id: u32,
    renewal: Option<Renewal>,
    ping: Option<([u8; 8], Instant)>,
    last_received: Instant,
    access: LiveTunnelAuthority,
}

impl Pump {
    async fn send(&mut self, stream_id: u32, body: TunnelFrameBody) -> Result<()> {
        self.authority.check_lease()?;
        let bytes = TunnelFrame {
            sequence: self.sent,
            stream_id,
            body,
        }
        .encode()?;
        let ciphertext = self.session.encrypt(&bytes)?;
        self.sent = self
            .sent
            .checked_add(1)
            .context("Tunnel sequence exhausted")?;
        send_envelope(
            &self.output,
            &self.session_id,
            TunnelEnvelopeBody::Message(ciphertext),
        )
        .await
    }

    async fn reset(&mut self, id: u32, code: &str, message: &str) -> Result<()> {
        self.streams.remove(&id);
        self.send(
            id,
            TunnelFrameBody::Reset(TunnelReset {
                code: code.into(),
                message: message.into(),
            }),
        )
        .await
    }

    async fn run<T: AsRef<[u8]> + Send>(
        &mut self,
        mut input: mpsc::Receiver<T>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let (events, mut pending) = mpsc::channel(32);
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut heartbeat = Instant::now();
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = timer.tick() => {
                    self.authority.check()?;
                    ensure!(self.last_received.elapsed() < Duration::from_secs(60), "Tunnel peer stopped responding");
                    if let Some(renewal) = &self.renewal { ensure!(Instant::now() < renewal.deadline, "Tunnel renewal timed out"); }
                    else {
                        let mut checked = HashMap::new();
                        #[cfg(feature = "runtime")]
                        let mut gateway = None;
                        let revoked = self.streams.iter().filter_map(|(id, stream)| {
                            if stream.sent_fin && stream.received_fin { return None; }
                            let allowed = match &stream.target {
                                streams::Target::Service(target) => *checked.entry(target.clone()).or_insert_with(|| self.authority.check_target(target).is_ok()),
                                streams::Target::Internal(target) => self.authority.check_internal(target).is_ok(),
                                #[cfg(feature = "runtime")]
                                streams::Target::Gateway(target) => *gateway.get_or_insert_with(|| self.authority.check_gateway(target).is_ok()),
                                streams::Target::Pending => true,
                            };
                            (!allowed).then_some(*id)
                        }).collect::<Vec<_>>();
                        for id in revoked { self.reset(id, "unauthorized", "Stream authorization ended").await?; }
                        if let Some((_, sent)) = self.ping { ensure!(sent.elapsed() < Duration::from_secs(60), "Tunnel heartbeat expired"); }
                        if heartbeat.elapsed() >= Duration::from_secs(20) && self.ping.is_none() {
                            let nonce: [u8; 8] = uuid::Uuid::new_v4().as_bytes()[..8].try_into()?;
                            self.send(0, TunnelFrameBody::Ping(nonce)).await?;
                            self.ping = Some((nonce, Instant::now()));
                            heartbeat = Instant::now();
                        }
                    }
                },
                envelope = receive(&mut input, &self.session_id) => {
                    let TunnelEnvelopeBody::Message(ciphertext) = envelope? else { anyhow::bail!("Unexpected tunnel envelope") };
                    let plaintext = zeroize::Zeroizing::new(self.session.decrypt(&ciphertext)?);
                    let frame = TunnelFrame::decode(&plaintext)?;
                    ensure!(frame.sequence == self.received, "Tunnel frame lost or reordered");
                    self.received = self.received.checked_add(1).context("Tunnel sequence exhausted")?;
                    self.last_received = Instant::now();
                    self.authority.check_lease()?;
                    self.incoming(frame, &events, &cancel).await?;
                },
                event = pending.recv(), if self.renewal.is_none() => {
                    self.event(event.context("Tunnel streams closed")?).await?;
                },
            }
        }
    }

    async fn incoming(
        &mut self,
        frame: TunnelFrame,
        events: &mpsc::Sender<streams::Event>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let id = frame.stream_id;
        if self.renewal.is_some() {
            if let TunnelFrameBody::Pong(nonce) = &frame.body {
                if self.ping.is_some_and(|(expected, _)| expected == *nonce) {
                    self.ping = None;
                }
                return Ok(());
            }
            if let TunnelFrameBody::Ping(nonce) = &frame.body {
                let renewal = self.renewal.as_mut().context("Missing tunnel renewal")?;
                ensure!(
                    renewal.pending_ping.is_none(),
                    "Repeated tunnel heartbeat during renewal"
                );
                renewal.pending_ping = Some(*nonce);
                return Ok(());
            }
            let TunnelFrameBody::RenewFinish(bytes) = frame.body else {
                anyhow::bail!("Tunnel traffic during renewal")
            };
            let mut renewal = self.renewal.take().context("Missing tunnel renewal")?;
            renewal.handshake.read(&bytes)?;
            renewal.authority.check()?;
            self.session = renewal.handshake.finish()?;
            self.authority = renewal.authority;
            *self
                .access
                .write()
                .map_err(|_| anyhow::anyhow!("Tunnel authority unavailable"))? =
                self.authority.clone();
            // The fresh authenticated exchange establishes liveness even if a heartbeat
            // crossed the old epoch's final frame.
            self.ping = None;
            self.send(
                0,
                TunnelFrameBody::Renewed(TunnelRenewed {
                    expires_at: self.authority.expires_at(),
                }),
            )
            .await?;
            if let Some(nonce) = renewal.pending_ping {
                self.send(0, TunnelFrameBody::Pong(nonce)).await?;
            }
            return Ok(());
        }
        match frame.body {
            TunnelFrameBody::OpenData(open) => {
                ensure!(
                    id > self.last_stream_id && id % 2 == 1,
                    "Tunnel stream identity was reused"
                );
                self.last_stream_id = id;
                if self.streams.len() >= TUNNEL_MAX_STREAMS {
                    return self
                        .reset(id, "limit", "Concurrent data stream limit reached")
                        .await;
                }
                self.streams.insert(
                    id,
                    streams::Stream::start_internal(
                        id,
                        open,
                        self.access.clone(),
                        events.clone(),
                        cancel,
                    ),
                );
            }
            TunnelFrameBody::Open(open) => {
                ensure!(
                    id > self.last_stream_id && id % 2 == 1,
                    "Tunnel stream identity was reused"
                );
                self.last_stream_id = id;
                if self.streams.len() >= TUNNEL_MAX_STREAMS {
                    return self
                        .reset(id, "limit", "Concurrent service stream limit reached")
                        .await;
                }
                let target = match self.authority.open_target(&open) {
                    Ok(target) => target,
                    Err(error) => {
                        let (code, message) = open_refusal(&error);
                        return self.reset(id, code, message).await;
                    }
                };
                self.streams.insert(
                    id,
                    streams::Stream::open(id, target, events.clone(), cancel),
                );
            }
            TunnelFrameBody::Data(bytes) => {
                if self
                    .streams
                    .get_mut(&id)
                    .is_none_or(|stream| stream.write(bytes).is_err())
                {
                    self.reset(id, "flow_control", "Service stream rejected data")
                        .await?;
                }
            }
            TunnelFrameBody::Window(delta) => {
                if id <= self.last_stream_id && !self.streams.contains_key(&id) {
                    return Ok(());
                }
                if self
                    .streams
                    .get_mut(&id)
                    .is_none_or(|stream| stream.grant(delta).is_err())
                {
                    self.reset(id, "flow_control", "Invalid service stream credit")
                        .await?;
                }
            }
            TunnelFrameBody::Fin => {
                if self
                    .streams
                    .get_mut(&id)
                    .is_none_or(|stream| stream.finish_write().is_err())
                {
                    self.reset(id, "closed", "Service stream is closed").await?;
                }
            }
            TunnelFrameBody::Reset(_) => {
                self.streams.remove(&id);
            }
            TunnelFrameBody::Ping(nonce) => self.send(0, TunnelFrameBody::Pong(nonce)).await?,
            TunnelFrameBody::Pong(nonce) => {
                if self.ping.is_some_and(|(expected, _)| expected == nonce) {
                    self.ping = None;
                }
            }
            TunnelFrameBody::RenewStart(start) => {
                let (authority, mut handshake) = self.authority.renew(&start.certificate_jws)?;
                handshake.read(&wire::decode(&start.data)?)?;
                let reply = handshake.write()?;
                self.renewal = Some(Renewal {
                    authority,
                    handshake,
                    deadline: Instant::now() + HANDSHAKE_TIMEOUT,
                    pending_ping: None,
                });
                self.send(0, TunnelFrameBody::RenewReply(reply)).await?;
            }
            _ => anyhow::bail!("Unexpected tunnel frame direction"),
        }
        Ok(())
    }

    async fn event(&mut self, event: streams::Event) -> Result<()> {
        use streams::Event;
        let id = match &event {
            Event::Opened(id)
            | Event::InternalOpened(id, _)
            | Event::Data(id, _)
            | Event::Consumed(id, _)
            | Event::Fin(id)
            | Event::Closed(id)
            | Event::Failed(id, _) => *id,
        };
        let Some(stream) = self.streams.get_mut(&id) else {
            return Ok(());
        };
        match event {
            Event::InternalOpened(_, target) => {
                stream.target = streams::Target::Internal(target);
                stream.opened = true;
                self.send(id, TunnelFrameBody::Opened).await?;
            }
            Event::Opened(_) => {
                stream.opened = true;
                self.send(id, TunnelFrameBody::Opened).await?;
            }
            Event::Data(_, bytes) => {
                ensure!(
                    bytes.len() <= stream.send_credit as usize && !stream.sent_fin,
                    "Invalid tunnel reader credit"
                );
                stream.send_credit -= bytes.len() as u32;
                self.send(id, TunnelFrameBody::Data(bytes)).await?;
            }
            Event::Consumed(_, bytes) => {
                stream.receive_credit = stream
                    .receive_credit
                    .checked_add(bytes)
                    .filter(|credit| *credit <= INITIAL_WINDOW)
                    .context("Invalid tunnel writer credit")?;
                self.send(id, TunnelFrameBody::Window(bytes)).await?;
            }
            Event::Fin(_) => {
                stream.sent_fin = true;
                self.send(id, TunnelFrameBody::Fin).await?;
            }
            Event::Closed(_) => {
                self.streams.remove(&id);
            }
            Event::Failed(_, reset) => {
                self.reset(id, reset.code(), "The data stream could not complete")
                    .await?
            }
        }
        Ok(())
    }
}

/// The reset of a refused open: `unsupported` when this agent does not serve the target,
/// else `unauthorized`, which names no reason.
fn open_refusal(error: &anyhow::Error) -> (&str, &str) {
    if rejection_code(error) == RejectionCode::Unsupported {
        (
            "unsupported",
            "This device agent does not serve the requested tunnel target",
        )
    } else {
        (
            "unauthorized",
            "The deployed service is unavailable or access was denied",
        )
    }
}

async fn receive<T: AsRef<[u8]>>(
    input: &mut mpsc::Receiver<T>,
    session_id: &str,
) -> Result<TunnelEnvelopeBody> {
    let bytes = input.recv().await.context("Tunnel transport closed")?;
    let envelope = TunnelEnvelope::decode(bytes.as_ref())?;
    ensure!(
        envelope.session_id == session_id,
        "Tunnel routing identity mismatch"
    );
    Ok(envelope.body)
}

async fn send_envelope(
    output: &mpsc::Sender<Vec<u8>>,
    session_id: &str,
    body: TunnelEnvelopeBody,
) -> Result<()> {
    let bytes = TunnelEnvelope {
        session_id: session_id.into(),
        body,
    }
    .encode()?;
    tokio::time::timeout(IO_TIMEOUT, output.send(bytes)).await??;
    Ok(())
}
