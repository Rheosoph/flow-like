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

pub(super) async fn run(
    device_id: String,
    management: Arc<ManagementService>,
    registry: Arc<SessionRegistry>,
    mut admission: watch::Receiver<Option<Arc<DeviceSignalingResponse>>>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut peers = JoinSet::new();
    let mut endpoint = 0usize;
    let mut failures = 0u32;
    loop {
        let current = admission.borrow_and_update().clone();
        let current = match current
            .filter(|value| value.expires_at > unix_time().unwrap_or(i64::MAX))
        {
            Some(value) if !value.signaling_urls.is_empty() => value,
            _ => {
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
            management.clone(),
            registry.clone(),
            &mut admission,
            &mut peers,
            cancel.clone(),
        )
        .await;
        if cancel.is_cancelled() {
            break;
        }
        if result.is_ok() {
            failures = 0;
        } else {
            failures = failures.saturating_add(1);
            tracing::warn!(
                "Device signaling connection interrupted; reconnecting without stopping workloads"
            );
        }
        let jitter = uuid::Uuid::new_v4().as_bytes()[0] as u64 % 500;
        let delay = Duration::from_millis((1u64 << failures.min(5)) * 1000 + jitter);
        tokio::select! {
            _ = cancel.cancelled() => break,
            changed = admission.changed() => if changed.is_err() { break; },
            _ = tokio::time::sleep(delay) => {},
        }
        while peers.try_join_next().is_some() {}
    }
    cancel.cancel();
    while peers.join_next().await.is_some() {}
    Ok(())
}

async fn connection(
    url: &str,
    device_id: &str,
    current: Arc<DeviceSignalingResponse>,
    management: Arc<ManagementService>,
    registry: Arc<SessionRegistry>,
    admission: &mut watch::Receiver<Option<Arc<DeviceSignalingResponse>>>,
    peers: &mut JoinSet<()>,
    cancel: CancellationToken,
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
    let sessions_cancel = cancel.child_token();
    let mut sessions = JoinSet::new();
    let mut inputs: NoiseSenders = HashMap::new();
    let (outbound, mut output) = mpsc::channel::<String>(32);
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
                    inputs.retain(|_, (_, sender)| !sender.is_closed());
                }
                value = output.recv() => {
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
                                    route_noise(envelope, &from, &management, &registry, &mut inputs, &mut sessions, &outbound, &sessions_cancel);
                                }
                                Channel::Signal => {
                                    let Ok(offer) = SignalEnvelope::parse(&bytes) else { continue; };
                                    let SignalEnvelope::Offer { session_id, grant_id, certificate_jws, .. } = &offer;
                                    let Ok(permit) = registry.reserve(session_id) else { continue; };
                                    let Ok(noise) = NoiseConnection::new(&management, session_id, grant_id, certificate_jws) else { continue; };
                                    let output = outbound.clone();
                                    let admission = current.clone();
                                    let cancel = cancel.child_token();
                                    peers.spawn(async move {
                                        let _permit = permit;
                                        let _ = rtc::serve(offer, noise, &from, admission, output, cancel).await;
                                    });
                                }
                            }
                        }
                    }
                }
                _ = sessions.join_next(), if !sessions.is_empty() => {},
                _ = peers.join_next(), if !peers.is_empty() => {},
            }
        }
        Ok(())
    }.await;
    // Only WebSocket sessions end here. Established WebRTC channels have their own lifetime.
    sessions_cancel.cancel();
    while sessions.join_next().await.is_some() {}
    let _ = tokio::time::timeout(Duration::from_secs(1), sender.close()).await;
    result
}

fn route_noise(
    envelope: NoiseEnvelope,
    from: &str,
    management: &Arc<ManagementService>,
    registry: &Arc<SessionRegistry>,
    inputs: &mut NoiseSenders,
    sessions: &mut JoinSet<()>,
    outbound: &mpsc::Sender<String>,
    cancel: &CancellationToken,
) {
    inputs.retain(|_, (_, sender)| !sender.is_closed());
    let session_id = envelope.session_id().to_owned();
    if let Some((owner, sender)) = inputs.get(&session_id) {
        if owner == from && sender.try_send(envelope).is_err() {
            inputs.remove(&session_id);
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
    let Ok(permit) = registry.reserve(&session_id) else {
        return;
    };
    let Ok(mut connection) =
        NoiseConnection::new(management, &session_id, grant_id, certificate_jws)
    else {
        return;
    };
    let (sender, mut receiver) = mpsc::channel(4);
    if sender.try_send(envelope).is_err() {
        return;
    }
    inputs.insert(session_id, (from.to_owned(), sender));
    let from = from.to_owned();
    let outbound = outbound.clone();
    let cancel = cancel.clone();
    sessions.spawn(async move {
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

    #[tokio::test]
    async fn websocket_noise_is_participant_bound_and_admission_replacement_releases_sessions()
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
        let running = tokio::spawn(async move {
            let mut peers = JoinSet::new();
            connection(
                &url,
                "device",
                admission,
                fixture.management,
                task_registry,
                &mut receiver,
                &mut peers,
                task_cancel,
            )
            .await
        });
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
        socket.send(Message::Text(serde_json::json!({"type":"ready","participant_id":"device","role":"device","expires_at":now+300}).to_string().into())).await?;
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
        // Another transport participant cannot consume a known session's Noise sequence.
        deliver(&mut socket, &envelope, "intruder").await?;
        deliver(&mut socket, &envelope, "controller").await?;
        let NoiseEnvelope::Message { data, .. } = response(&mut socket).await? else {
            anyhow::bail!("No command response");
        };
        let inspected: serde_json::Value =
            serde_json::from_slice(&session.decrypt(&wire::decode(&data)?)?)?;
        assert_eq!(inspected["state"], "completed");
        assert_eq!(inspected["result"]["device_id"], "device");
        renew.send(None)?;
        timeout(Duration::from_secs(15), running).await???;
        assert!(registry.ids.lock().unwrap().is_empty());
        cancel.cancel();
        Ok(())
    }
}
