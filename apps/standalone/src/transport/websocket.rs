use super::{
    IO_TIMEOUT, NoiseConnection, SessionRegistry, rtc,
    wire::{self, Channel, NoiseEnvelope, ServerFrame, SignalEnvelope},
};
use crate::{enrollment::unix_time, management::ManagementService};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::DeviceSignalingResponse;
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        Message, client::IntoClientRequest, http::HeaderValue, protocol::WebSocketConfig,
    },
};
use tokio_util::sync::CancellationToken;

type NoiseSenders = HashMap<String, (String, mpsc::Sender<NoiseEnvelope>)>;

/// Relayed Noise sessions and WebRTC peers outlive any one signaling socket. The relay may
/// close the shared device socket (renewal, per-socket limits); the next socket carries the
/// same sessions, and only a lost admission releases them.
struct Relayed {
    inputs: NoiseSenders,
    sessions: JoinSet<()>,
    peers: JoinSet<()>,
    outbound: mpsc::Sender<String>,
    output: mpsc::Receiver<String>,
    sessions_cancel: CancellationToken,
    cancel: CancellationToken,
}

impl Relayed {
    fn new(cancel: CancellationToken) -> Self {
        let (outbound, output) = mpsc::channel(32);
        Self {
            inputs: HashMap::new(),
            sessions: JoinSet::new(),
            peers: JoinSet::new(),
            outbound,
            output,
            sessions_cancel: cancel.child_token(),
            cancel,
        }
    }

    fn reap(&mut self) {
        while self.sessions.try_join_next().is_some() {}
        while self.peers.try_join_next().is_some() {}
        self.inputs.retain(|_, (_, sender)| !sender.is_closed());
    }

    async fn release_sessions(&mut self) {
        self.sessions_cancel.cancel();
        while self.sessions.join_next().await.is_some() {}
        self.inputs.clear();
        while self.output.try_recv().is_ok() {}
        self.sessions_cancel = self.cancel.child_token();
    }

    async fn shutdown(mut self) {
        self.cancel.cancel();
        while self.sessions.join_next().await.is_some() {}
        while self.peers.join_next().await.is_some() {}
    }

    fn route_noise(
        &mut self,
        envelope: NoiseEnvelope,
        from: &str,
        management: &Arc<ManagementService>,
        registry: &Arc<SessionRegistry>,
    ) {
        self.inputs.retain(|_, (_, sender)| !sender.is_closed());
        let session_id = envelope.session_id().to_owned();
        if let Some((owner, sender)) = self.inputs.get(&session_id) {
            if owner == from
                && (matches!(envelope, NoiseEnvelope::Close { .. })
                    || sender.try_send(envelope).is_err())
            {
                self.inputs.remove(&session_id);
            }
            return;
        }
        let NoiseEnvelope::Hello {
            grant_id,
            certificate_jws,
            ..
        } = &envelope
        else {
            return;
        };
        let mut connection = match NoiseConnection::new(
            management,
            &session_id,
            grant_id,
            certificate_jws,
        ) {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(session_id = %session_id, grant_id = %grant_id, "Management session was not admitted: {error:#}");
                return;
            }
        };
        let permit = match registry.reserve(&session_id, grant_id, &self.sessions_cancel) {
            Ok(permit) => permit,
            Err(error) => {
                tracing::warn!(session_id = %session_id, grant_id = %grant_id, "Management session was refused: {error:#}");
                return;
            }
        };
        let (sender, mut receiver) = mpsc::channel(4);
        if sender.try_send(envelope).is_err() {
            return;
        }
        self.inputs.insert(session_id, (from.to_owned(), sender));
        let from = from.to_owned();
        let outbound = self.outbound.clone();
        self.sessions.spawn(async move {
            let cancel = permit.cancel.clone();
            let _permit = permit;
            let result: Result<()> = async {
                loop {
                    let message = tokio::select! {
                        _ = cancel.cancelled() => break,
                        value = tokio::time::timeout(connection.timeout(), receiver.recv()) => value?.context("Management transport closed")?,
                    };
                    let response = connection.receive(message).await?;
                    let frame = wire::outbound(&from, Channel::Noise, &response)?;
                    tokio::select! { _ = cancel.cancelled() => break, value = tokio::time::timeout(IO_TIMEOUT, outbound.send(frame)) => value??, }
                }
                Ok(())
            }.await;
            drop(result);
        });
    }

    fn accept_offer(
        &mut self,
        offer: SignalEnvelope,
        from: &str,
        management: &Arc<ManagementService>,
        registry: &Arc<SessionRegistry>,
        admission: Arc<DeviceSignalingResponse>,
    ) {
        let SignalEnvelope::Offer {
            session_id,
            grant_id,
            certificate_jws,
            ..
        } = &offer;
        let noise = match NoiseConnection::new(management, session_id, grant_id, certificate_jws) {
            Ok(noise) => noise,
            Err(error) => {
                tracing::warn!(session_id = %session_id, grant_id = %grant_id, "Management peer was not admitted: {error:#}");
                return;
            }
        };
        let permit = match registry.reserve(session_id, grant_id, &self.cancel) {
            Ok(permit) => permit,
            Err(error) => {
                tracing::warn!(session_id = %session_id, grant_id = %grant_id, "Management peer was refused: {error:#}");
                return;
            }
        };
        let output = self.outbound.clone();
        let from = from.to_owned();
        self.peers.spawn(async move {
            let cancel = permit.cancel.clone();
            let _permit = permit;
            let _ = rtc::serve(offer, noise, &from, admission, output, cancel).await;
        });
    }
}

pub(super) async fn run(
    device_id: String,
    management: Arc<ManagementService>,
    registry: Arc<SessionRegistry>,
    mut admission: watch::Receiver<Option<Arc<DeviceSignalingResponse>>>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut relayed = Relayed::new(cancel.clone());
    let mut endpoint = 0usize;
    let mut failures = 0u32;
    loop {
        let current = admission.borrow_and_update().clone();
        let current = match current
            .filter(|value| value.expires_at > unix_time().unwrap_or(i64::MAX))
        {
            Some(value) if !value.signaling_urls.is_empty() => value,
            _ => {
                relayed.release_sessions().await;
                tokio::select! { _ = cancel.cancelled() => break, changed = admission.changed() => if changed.is_err() { break; } }
                continue;
            }
        };
        let url = &current.signaling_urls[endpoint % current.signaling_urls.len()];
        endpoint = endpoint.wrapping_add(1);
        let result = connection(
            url,
            &device_id,
            current.clone(),
            &management,
            &registry,
            &mut admission,
            &mut relayed,
            &cancel,
        )
        .await;
        if cancel.is_cancelled() {
            break;
        }
        match result {
            Ok(()) => failures = 0,
            Err(error) => {
                failures = failures.saturating_add(1);
                tracing::warn!(
                    "Device signaling connection interrupted; reconnecting without stopping workloads or management sessions: {error:#}"
                );
            }
        }
        let jitter = uuid::Uuid::new_v4().as_bytes()[0] as u64 % 500;
        let delay = Duration::from_millis((1u64 << failures.min(5)) * 1000 + jitter);
        tokio::select! {
            _ = cancel.cancelled() => break,
            changed = admission.changed() => if changed.is_err() { break; },
            _ = tokio::time::sleep(delay) => {},
        }
        relayed.reap();
    }
    relayed.shutdown().await;
    Ok(())
}

async fn connection(
    url: &str,
    device_id: &str,
    current: Arc<DeviceSignalingResponse>,
    management: &Arc<ManagementService>,
    registry: &Arc<SessionRegistry>,
    admission: &mut watch::Receiver<Option<Arc<DeviceSignalingResponse>>>,
    relayed: &mut Relayed,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(&format!(
            "{}, flowlike.jwt.{}",
            wire::PROTOCOL,
            current.token
        ))?,
    );
    let config = WebSocketConfig::default()
        .read_buffer_size(16 * 1024)
        .write_buffer_size(0)
        .max_write_buffer_size(2 * wire::MAX_FRAME)
        .max_message_size(Some(wire::MAX_FRAME))
        .max_frame_size(Some(wire::MAX_FRAME));
    let (socket, response) = tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        result = tokio::time::timeout(IO_TIMEOUT, connect_async_with_config(request, Some(config), true)) => result??,
    };
    ensure!(
        response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|value| value.to_str().ok())
            == Some(wire::PROTOCOL),
        "Signaling subprotocol mismatch"
    );
    let (mut sender, mut receiver) = socket.split();
    let first = tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        value = tokio::time::timeout(IO_TIMEOUT, receiver.next()) => value?.context("Signaling closed before ready")??,
    };
    let Message::Text(first) = first else {
        anyhow::bail!("Signaling did not confirm admission");
    };
    match serde_json::from_str::<ServerFrame>(&first)? {
        ServerFrame::Ready {
            participant_id,
            role,
            expires_at,
        } => ensure!(
            participant_id == device_id && role == "device" && expires_at == current.expires_at,
            "Signaling admission binding mismatch"
        ),
        _ => anyhow::bail!("Signaling did not confirm admission"),
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_pong = tokio::time::Instant::now();
    let result: Result<()> = async {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                changed = admission.changed() => { changed?; break; }
                _ = heartbeat.tick() => {
                    ensure!(unix_time()? < current.expires_at && last_pong.elapsed() < Duration::from_secs(60), "Signaling admission or heartbeat expired");
                    tokio::time::timeout(IO_TIMEOUT, sender.send(Message::Text("{\"type\":\"ping\"}".into()))).await??;
                    relayed.inputs.retain(|_, (_, sender)| !sender.is_closed());
                }
                value = relayed.output.recv() => {
                    let value = value.context("Signaling output closed")?;
                    tokio::time::timeout(IO_TIMEOUT, sender.send(Message::Text(value.into()))).await??;
                }
                value = receiver.next() => {
                    let value = value.context("Signaling socket closed")??;
                    let text = match value {
                        Message::Text(value) => value,
                        Message::Ping(bytes) => { tokio::time::timeout(IO_TIMEOUT, sender.send(Message::Pong(bytes))).await??; continue; }
                        Message::Pong(_) => { last_pong = tokio::time::Instant::now(); continue; }
                        Message::Close(_) => break,
                        _ => anyhow::bail!("Unexpected signaling frame"),
                    };
                    let frame: ServerFrame = serde_json::from_str(&text)?;
                    match frame {
                        ServerFrame::Pong => last_pong = tokio::time::Instant::now(),
                        ServerFrame::Ready { .. } => anyhow::bail!("Repeated signaling admission"),
                        ServerFrame::Frame { to, channel, payload, from, from_role } => {
                            ensure!(to == device_id && from_role == "controller", "Signaling route mismatch");
                            wire::identifier(&from)?;
                            let Ok(bytes) = wire::decode(&payload) else { continue; };
                            match channel {
                                Channel::Noise => {
                                    let Ok(envelope) = NoiseEnvelope::parse(&bytes) else { continue; };
                                    relayed.route_noise(envelope, &from, management, registry);
                                }
                                Channel::Signal => {
                                    let Ok(offer) = SignalEnvelope::parse(&bytes) else { continue; };
                                    relayed.accept_offer(offer, &from, management, registry, current.clone());
                                }
                            }
                        }
                    }
                }
                _ = relayed.sessions.join_next(), if !relayed.sessions.is_empty() => {},
                _ = relayed.peers.join_next(), if !relayed.peers.is_empty() => {},
            }
        }
        Ok(())
    }.await;
    let _ = tokio::time::timeout(Duration::from_secs(1), sender.close()).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{ManagementCommand, ManagementRequest};
    use tokio::{
        net::{TcpListener, TcpStream},
        time::timeout,
    };
    use tokio_tungstenite::{
        WebSocketStream, accept_hdr_async,
        tungstenite::handshake::server::{Request, Response},
    };

    async fn accept(listener: &TcpListener, expires_at: i64) -> Result<WebSocketStream<TcpStream>> {
        let (stream, _) = timeout(Duration::from_secs(10), listener.accept()).await??;
        let mut socket = accept_hdr_async(stream, |request: &Request, mut response: Response| {
            assert_eq!(
                request.headers()["Sec-WebSocket-Protocol"],
                format!("{}, flowlike.jwt.redacted", wire::PROTOCOL)
            );
            response.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                HeaderValue::from_static(wire::PROTOCOL),
            );
            Ok(response)
        })
        .await?;
        socket.send(Message::Text(serde_json::json!({"type":"ready","participant_id":"device","role":"device","expires_at":expires_at}).to_string().into())).await?;
        Ok(socket)
    }

    async fn deliver(
        socket: &mut WebSocketStream<TcpStream>,
        envelope: &NoiseEnvelope,
        from: &str,
    ) -> Result<()> {
        let value = serde_json::json!({"type":"frame","to":"device","from":from,"from_role":"controller","channel":"noise","payload":wire::encode(&wire::serialize(envelope)?)});
        socket.send(Message::Text(value.to_string().into())).await?;
        Ok(())
    }

    async fn response(socket: &mut WebSocketStream<TcpStream>) -> Result<NoiseEnvelope> {
        loop {
            let frame = timeout(Duration::from_secs(10), socket.next())
                .await?
                .context("Agent socket closed")??;
            let Message::Text(text) = frame else {
                anyhow::bail!("Unexpected WebSocket data");
            };
            let value: serde_json::Value = serde_json::from_str(&text)?;
            if value["type"] == "ping" {
                socket
                    .send(Message::Text("{\"type\":\"pong\"}".into()))
                    .await?;
                continue;
            }
            assert_eq!(value["to"], "controller");
            assert_eq!(value["channel"], "noise");
            return NoiseEnvelope::parse(&wire::decode(
                value["payload"].as_str().context("Missing payload")?,
            )?);
        }
    }

    fn held(registry: &SessionRegistry) -> usize {
        registry.slots.lock().unwrap().sessions.len()
    }

    #[tokio::test]
    async fn relayed_sessions_survive_socket_replacement_and_release_with_their_admission()
    -> Result<()> {
        let fixture = super::super::tests::fixture()?;
        let _directory = fixture.directory;
        let mut initiator = fixture.initiator;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("ws://{}/ws/devices", listener.local_addr()?);
        let now = unix_time()?;
        let admission = Arc::new(DeviceSignalingResponse {
            token: "redacted".into(),
            expires_at: now + 300,
            device_auth_epoch: 1,
            signaling_urls: vec![url.clone()],
            ice_servers: vec![],
            ice_expires_at: None,
            policy_version: 0,
            policy_digest: None,
        });
        let (renew, mut receiver) = watch::channel(Some(admission.clone()));
        let registry = Arc::new(SessionRegistry::default());
        let task_registry = registry.clone();
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let management = fixture.management;
        let running = tokio::spawn(async move {
            let mut relayed = Relayed::new(task_cancel.clone());
            for _ in 0..2 {
                connection(
                    &url,
                    "device",
                    admission.clone(),
                    &management,
                    &task_registry,
                    &mut receiver,
                    &mut relayed,
                    &task_cancel,
                )
                .await?;
            }
            relayed.release_sessions().await;
            Ok::<_, anyhow::Error>(())
        });
        let mut socket = accept(&listener, now + 300).await?;
        deliver(
            &mut socket,
            &NoiseEnvelope::Hello {
                session_id: "session".into(),
                grant_id: "owner".into(),
                certificate_jws: fixture.certificate,
                data: wire::encode(&initiator.write()?),
            },
            "controller",
        )
        .await?;
        let NoiseEnvelope::Handshake { data, .. } = response(&mut socket).await? else {
            anyhow::bail!("No Noise handshake");
        };
        initiator.read(&wire::decode(&data)?)?;
        deliver(
            &mut socket,
            &NoiseEnvelope::Handshake {
                session_id: "session".into(),
                data: wire::encode(&initiator.write()?),
            },
            "controller",
        )
        .await?;
        let mut session = initiator.finish()?;
        let NoiseEnvelope::Message { data, .. } = response(&mut socket).await? else {
            anyhow::bail!("No encrypted ready");
        };
        let ready: serde_json::Value =
            serde_json::from_slice(&session.decrypt(&wire::decode(&data)?)?)?;
        assert_eq!(ready["ready"], true);
        // The relay closes the shared device socket, as its per-socket limits do.
        socket.close(None).await?;
        drop(socket);
        let mut socket = accept(&listener, now + 300).await?;
        assert_eq!(held(&registry), 1);
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
        // Another transport participant cannot consume or close a known session.
        deliver(&mut socket, &envelope, "intruder").await?;
        deliver(
            &mut socket,
            &NoiseEnvelope::Close {
                session_id: "session".into(),
            },
            "intruder",
        )
        .await?;
        deliver(&mut socket, &envelope, "controller").await?;
        let NoiseEnvelope::Message { data, .. } = response(&mut socket).await? else {
            anyhow::bail!("No command response");
        };
        let inspected: serde_json::Value =
            serde_json::from_slice(&session.decrypt(&wire::decode(&data)?)?)?;
        assert_eq!(inspected["state"], "completed");
        assert_eq!(inspected["result"]["device_id"], "device");
        deliver(
            &mut socket,
            &NoiseEnvelope::Close {
                session_id: "session".into(),
            },
            "controller",
        )
        .await?;
        timeout(Duration::from_secs(5), async {
            while held(&registry) != 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await?;
        renew.send(None)?;
        timeout(Duration::from_secs(15), running).await???;
        assert_eq!(held(&registry), 0);
        cancel.cancel();
        Ok(())
    }
}
