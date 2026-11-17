use super::{
    access::Access,
    answerer::serve_offer,
    crypto::Envelope,
    messages::{Incoming, Outgoing, RefusedCode, now_ms},
    pairing::{Identity, PairRequest, Shared},
    records::StreamKeys,
    sealed::{self, OfferError},
    signaling::{self, Frame, Writer},
    trust::TrustStore,
};
use crate::{
    bot_host::hub::{HubFailure, ProfileHub},
    functions::{ai::bot_tools::view, inbox_e2e::PersonKeys},
};
use flow_like_device_crypto::inbox::{InboxSecret, PublicInboxKey, RecipientKey};
use flow_like_types::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::{
    ops::ControlFlow,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    time::{Instant, interval_at, sleep, sleep_until},
};
use tokio_util::sync::CancellationToken;
use webrtc::peer_connection::RTCIceServer;

const MAX_SESSIONS: usize = 4;
const ACTIVE_FOR: Duration = Duration::from_secs(180);
const PING_EVERY: Duration = Duration::from_secs(25);
const ACCESS_REFRESH: Duration = Duration::from_secs(15 * 60);
const KEY_REFRESH_GAP: Duration = Duration::from_secs(30);
const EXPIRY_MARGIN_SECS: i64 = 60;
const MIN_CONNECTION: Duration = Duration::from_secs(30);
const MAX_CONNECTION: Duration = Duration::from_secs(24 * 60 * 60);
const SIGNED_OUT_WAIT: Duration = Duration::from_secs(30);
const UNAVAILABLE_WAIT: Duration = Duration::from_secs(600);
const RECONNECT_PAUSE: Duration = Duration::from_secs(1);
const BACKOFF: (Duration, Duration) = (Duration::from_secs(2), Duration::from_secs(300));
const REPLY_QUEUE: usize = 16;
const REFUSAL_CHARS: usize = 200;

enum End {
    Expiring,
    RoomChanged,
    SignedOut,
}

struct Backoff {
    bounds: (Duration, Duration),
    next: Duration,
}

impl Backoff {
    fn new(bounds: (Duration, Duration)) -> Self {
        Self {
            bounds,
            next: bounds.0,
        }
    }

    fn reset(&mut self) {
        self.next = self.bounds.0;
    }

    fn step(&mut self) -> Duration {
        let wait = self.next;
        self.next = (self.next * 2).min(self.bounds.1);
        wait
    }
}

struct Host {
    app: AppHandle,
    hub: ProfileHub,
    peer: String,
    slots: Arc<Semaphore>,
    shared: Arc<Shared>,
}

fn unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .context("The bot screen key task stopped")?
}

fn refusal(message: &str) -> String {
    message.chars().take(REFUSAL_CHARS).collect()
}

pub(super) async fn run(app: AppHandle, shared: Arc<Shared>) {
    let host = Arc::new(Host {
        hub: ProfileHub::new(app.clone()),
        app,
        peer: shared.peer.clone(),
        slots: Arc::new(Semaphore::new(MAX_SESSIONS)),
        shared,
    });
    let mut backoff = Backoff::new(BACKOFF);
    let mut room: Option<(String, CancellationToken)> = None;
    loop {
        let wait = match host.fetch_access().await {
            Ok(access) => {
                if room.as_ref().is_none_or(|(known, _)| *known != access.room) {
                    if let Some((_, sessions)) = room.take() {
                        sessions.cancel();
                    }
                    room = Some((access.room.clone(), CancellationToken::new()));
                }
                let sessions = room
                    .as_ref()
                    .map(|(_, token)| token.clone())
                    .unwrap_or_default();
                let ended = host.connected(access, &sessions, &mut backoff).await;
                host.shared.set_identity(None);
                match ended {
                    Ok(End::Expiring) => RECONNECT_PAUSE,
                    Ok(End::RoomChanged | End::SignedOut) => {
                        sessions.cancel();
                        room = None;
                        RECONNECT_PAUSE
                    }
                    Err(error) => {
                        let wait = backoff.step();
                        tracing::warn!(%error, ?wait, "Bot screens: the signaling connection ended");
                        wait
                    }
                }
            }
            Err(failure) if failure.signed_out() => {
                host.shared.set_identity(None);
                if let Some((_, sessions)) = room.take() {
                    sessions.cancel();
                }
                SIGNED_OUT_WAIT
            }
            Err(failure) if failure.status == Some(503) => UNAVAILABLE_WAIT,
            Err(failure) => {
                let wait = backoff.step();
                tracing::warn!(error = %failure.message, ?wait, "Bot screens: the hub gave no access");
                wait
            }
        };
        sleep(wait).await;
    }
}

impl Host {
    async fn fetch_access(&self) -> Result<Access, HubFailure> {
        let value = self.hub.bot_screen_access().await?;
        Access::parse(value).map_err(|error| HubFailure::offline(error.to_string()))
    }

    fn keyring(&self) -> Arc<flow_like_device_crypto::inbox::InboxKeyring> {
        self.app.state::<PersonKeys>().ring()
    }

    async fn ensure_key(&self, sub: &str) -> Result<PublicInboxKey> {
        let ring = self.keyring();
        let owner = sub.to_owned();
        let (key, _) = blocking(move || {
            ring.ensure(&owner)
                .map_err(|error| anyhow!("The inbox key of '{owner}' is unavailable: {error}"))
        })
        .await?;
        Ok(key)
    }

    async fn secrets(&self, sub: &str) -> Result<Vec<InboxSecret>> {
        let ring = self.keyring();
        let owner = sub.to_owned();
        blocking(move || {
            ring.secrets(&owner)
                .map_err(|error| anyhow!("The inbox keys of '{owner}' are unavailable: {error}"))
        })
        .await
    }

    async fn connected(
        self: &Arc<Self>,
        access: Access,
        sessions: &CancellationToken,
        backoff: &mut Backoff,
    ) -> Result<End> {
        let sub = sealed::sub_of_jwt(&access.jwt)
            .ok_or_else(|| anyhow!("The bot screen token carries no subject"))?;
        let key = self.ensure_key(&sub).await?;
        let identity = Identity {
            sub: sub.clone(),
            key_id: key.key_id,
            public_key: key.public_key,
        };
        let (writer, mut reader) = signaling::connect(&access).await?;
        backoff.reset();
        let lifetime = (access.expires_at - EXPIRY_MARGIN_SECS - unix_secs()).max(0) as u64;
        let deadline =
            Instant::now() + Duration::from_secs(lifetime).clamp(MIN_CONNECTION, MAX_CONNECTION);
        let (replies, mut replied) = mpsc::channel(REPLY_QUEUE);
        let mut connection = Connection {
            host: self.clone(),
            access,
            sub,
            identity: identity.clone(),
            writer,
            replies,
            sessions: sessions.clone(),
            keys_read: None,
        };
        let mut ping = interval_at(Instant::now() + PING_EVERY, PING_EVERY);
        let mut refresh = interval_at(Instant::now() + ACCESS_REFRESH, ACCESS_REFRESH);
        self.shared.set_identity(Some(identity));
        loop {
            let flow = tokio::select! {
                _ = sleep_until(deadline) => ControlFlow::Break(End::Expiring),
                _ = ping.tick() => if connection.still_signed_in().await {
                    connection.writer.ping().await.map(ControlFlow::Continue)?
                } else {
                    ControlFlow::Break(End::SignedOut)
                },
                _ = refresh.tick() => connection.refresh().await?,
                reply = replied.recv() => match reply {
                    Some(reply) => connection.publish(reply).await.map(ControlFlow::Continue)?,
                    None => ControlFlow::Continue(()),
                },
                frame = reader.next() => match frame? {
                    Frame::Publish(data) => connection.on_data(data).await?,
                    Frame::Pong | Frame::Other => ControlFlow::Continue(()),
                },
            };
            if let ControlFlow::Break(end) = flow {
                return Ok(end);
            }
        }
    }

    fn active(&self, bot_id: &str) -> bool {
        view::running_machine_host()
            .and_then(|host| host.latest_frame(bot_id))
            .is_some_and(|frame| frame.at.elapsed() <= ACTIVE_FOR)
    }
}

struct Connection {
    host: Arc<Host>,
    access: Access,
    sub: String,
    identity: Identity,
    writer: Writer,
    replies: mpsc::Sender<Outgoing>,
    sessions: CancellationToken,
    keys_read: Option<Instant>,
}

struct OfferRequest {
    from: String,
    session: String,
    bot_id: String,
    sealed: Value,
}

impl Connection {
    async fn publish(&mut self, message: Outgoing) -> Result<()> {
        let plaintext = serde_json::to_vec(&message)?;
        let envelope = self
            .access
            .sealing_key()
            .seal(&self.access.room, &plaintext)?;
        self.writer.publish(&envelope).await
    }

    async fn refresh(&mut self) -> Result<ControlFlow<End>> {
        match self.host.fetch_access().await {
            Ok(fresh) if fresh.room != self.access.room => Ok(ControlFlow::Break(End::RoomChanged)),
            Ok(fresh) => {
                self.access.keys = fresh.keys;
                self.access.ice_servers = fresh.ice_servers;
                self.access.ice_expires_at = fresh.ice_expires_at;
                Ok(ControlFlow::Continue(()))
            }
            Err(failure) if failure.signed_out() => Ok(ControlFlow::Break(End::SignedOut)),
            Err(failure) => {
                tracing::debug!(error = %failure.message, "Bot screens: refreshing the access failed");
                Ok(ControlFlow::Continue(()))
            }
        }
    }

    async fn on_data(&mut self, data: Value) -> Result<ControlFlow<End>> {
        let Ok(envelope) = serde_json::from_value::<Envelope>(data) else {
            return Ok(ControlFlow::Continue(()));
        };
        if self.access.key(&envelope.key_id).is_none() {
            let allowed = self
                .keys_read
                .is_none_or(|at| at.elapsed() >= KEY_REFRESH_GAP);
            if !allowed {
                return Ok(ControlFlow::Continue(()));
            }
            self.keys_read = Some(Instant::now());
            if let ControlFlow::Break(end) = self.refresh().await? {
                return Ok(ControlFlow::Break(end));
            }
        }
        let plaintext = self
            .access
            .key(&envelope.key_id)
            .and_then(|key| key.open(&self.access.room, &envelope));
        let message = plaintext.as_deref().and_then(Incoming::parse);
        let Some(message) = message.filter(|message| message.valid(now_ms())) else {
            return Ok(ControlFlow::Continue(()));
        };
        let grants = matches!(message, Incoming::Pair { .. } | Incoming::Offer { .. });
        if grants && !self.still_signed_in().await {
            return Ok(ControlFlow::Break(End::SignedOut));
        }
        self.on_message(message).await?;
        Ok(ControlFlow::Continue(()))
    }

    /// False once the account this connection serves is no longer signed in to the current
    /// profile; a local session read that fails keeps the connection.
    async fn still_signed_in(&self) -> bool {
        match self.host.hub.signed_in_subject().await {
            Ok(subject) => subject.as_deref() == Some(self.sub.as_str()),
            Err(error) => {
                tracing::debug!(%error, "Bot screens: reading the signed-in account failed");
                true
            }
        }
    }

    async fn on_message(&mut self, message: Incoming) -> Result<()> {
        match message {
            Incoming::Hello { from, bot_id, .. } => {
                let active = self.host.active(&bot_id);
                self.publish(Outgoing::Here {
                    from: self.host.peer.clone(),
                    to: from,
                    bot_id,
                    active,
                    key_id: self.identity.key_id.clone(),
                    label: self.host.shared.label.clone(),
                    at: now_ms(),
                })
                .await
            }
            Incoming::Pair {
                from,
                to,
                session,
                key_id,
                public_key,
                label,
                mac,
                ..
            } if to == self.host.peer => {
                let request = PairRequest {
                    session: &session,
                    key_id: &key_id,
                    public_key: &public_key,
                    label: &label,
                    mac: &mac,
                };
                let outcome = self
                    .host
                    .shared
                    .accept(&self.identity, &request, unix_secs());
                let reply = match outcome {
                    Ok(mac) => {
                        crate::utils::emit_to_ui(
                            &self.host.app,
                            "bot-screens-paired",
                            json!({ "key_id": key_id, "label": label }),
                        );
                        Outgoing::Paired {
                            from: self.host.peer.clone(),
                            to: from,
                            session,
                            mac,
                            at: now_ms(),
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Bot screens: a pairing request was refused");
                        Outgoing::Refused {
                            from: self.host.peer.clone(),
                            to: from,
                            session,
                            code: RefusedCode::NotPaired,
                            message: refusal(&error.to_string()),
                            at: now_ms(),
                        }
                    }
                };
                self.publish(reply).await
            }
            Incoming::Offer {
                from,
                to,
                bot_id,
                session,
                sealed,
                ..
            } if to == self.host.peer => {
                let request = OfferRequest {
                    from,
                    session,
                    bot_id,
                    sealed,
                };
                match self.host.slots.clone().try_acquire_owned() {
                    Ok(permit) => {
                        self.spawn_session(request, permit);
                        Ok(())
                    }
                    Err(_) => {
                        let busy = refused(
                            &self.host.peer,
                            &request,
                            RefusedCode::Busy,
                            "This device is already streaming to the most viewers it supports",
                        );
                        self.publish(busy).await
                    }
                }
            }
            _ => Ok(()),
        }
    }

    fn spawn_session(&self, request: OfferRequest, permit: OwnedSemaphorePermit) {
        let host = self.host.clone();
        let sub = self.sub.clone();
        let replies = self.replies.clone();
        let ice = self.access.usable_ice(unix_secs());
        let cancel = self.sessions.child_token();
        tauri::async_runtime::spawn(async move {
            let _permit = permit;
            let peer = host.peer.clone();
            let reported = request.session.clone();
            let result = run_session(&host, &sub, &request, ice, cancel, &replies).await;
            if let Err((code, message)) = result {
                tracing::warn!(session = %reported, %message, "Bot screens: a session ended in a refusal");
                let _ = replies
                    .send(refused(&peer, &request, code, &refusal(&message)))
                    .await;
            }
        });
    }
}

fn refused(peer: &str, request: &OfferRequest, code: RefusedCode, message: &str) -> Outgoing {
    Outgoing::Refused {
        from: peer.to_owned(),
        to: request.from.clone(),
        session: request.session.clone(),
        code,
        message: message.to_owned(),
        at: now_ms(),
    }
}

fn pinned_viewer(
    trust: &TrustStore,
    sub: &str,
    key_id: &str,
) -> Result<RecipientKey, (RefusedCode, String)> {
    trust
        .get(sub, key_id)
        .map(|key| RecipientKey {
            key_id: key.key_id,
            public_key: key.public_key,
        })
        .ok_or_else(|| {
            (
                RefusedCode::NotPaired,
                "This device is not paired with this computer; scan its pairing code first"
                    .to_owned(),
            )
        })
}

fn failed(error: impl std::fmt::Display) -> (RefusedCode, String) {
    (RefusedCode::Failed, error.to_string())
}

async fn run_session(
    host: &Host,
    sub: &str,
    request: &OfferRequest,
    ice: Vec<RTCIceServer>,
    cancel: CancellationToken,
    replies: &mpsc::Sender<Outgoing>,
) -> Result<(), (RefusedCode, String)> {
    let _ended = cancel.clone().drop_guard();
    let secrets = host.secrets(sub).await.map_err(failed)?;
    let offer = sealed::open_offer(
        &secrets,
        sub,
        &request.session,
        &request.bot_id,
        &request.sealed,
    )
    .map_err(|error| match error {
        OfferError::NoKey => (
            RefusedCode::NoKey,
            "None of this device's keys opens the offer".to_owned(),
        ),
        OfferError::Invalid(message) => (RefusedCode::Failed, message),
    })?;
    let viewer = pinned_viewer(&host.shared.trust, sub, &offer.key_id)?;
    host.shared.track(&viewer.key_id, cancel.clone());
    let host_secret = sealed::random_secret();
    let keys = StreamKeys::derive(&offer.secret, &host_secret, &request.session).map_err(failed)?;
    let machines = view::machine_host(&host.app);
    let (answer_sender, answer_receiver) = oneshot::channel();
    let mut serving = tokio::spawn(serve_offer(
        offer.sdp,
        ice,
        keys,
        machines,
        offer.bot_id,
        answer_sender,
        cancel,
    ));
    let sdp = tokio::select! {
        sdp = answer_receiver => match sdp {
            Ok(sdp) => sdp,
            Err(_) => return Err(ended(serving.await)),
        },
        ended_early = &mut serving => return Err(ended(ended_early)),
    };
    let sealed =
        sealed::seal_answer(sub, &viewer, &request.session, &sdp, &host_secret).map_err(failed)?;
    let answer = Outgoing::Answer {
        from: host.peer.clone(),
        to: request.from.clone(),
        session: request.session.clone(),
        sealed,
        at: now_ms(),
    };
    replies
        .send(answer)
        .await
        .map_err(|_| failed("The signaling connection closed before the answer was sent"))?;
    match serving.await {
        Ok(Err(error)) => {
            tracing::debug!(%error, session = %request.session, "Bot screens: the stream ended")
        }
        Err(error) => {
            tracing::warn!(%error, session = %request.session, "Bot screens: the stream task stopped")
        }
        Ok(Ok(())) => {}
    }
    Ok(())
}

fn ended(joined: Result<Result<()>, tokio::task::JoinError>) -> (RefusedCode, String) {
    match joined {
        Ok(Err(error)) => failed(error),
        Ok(Ok(())) => failed("The bot screen session ended before it was answered"),
        Err(error) => failed(format!("The bot screen session stopped: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::trust::tests::{TempFile, key};
    use super::*;

    #[test]
    fn only_a_pinned_viewer_key_of_this_account_is_accepted() {
        let file = TempFile::new();
        let trust = TrustStore::new(file.0.clone());
        trust.pin("user-1", key("k1")).unwrap();
        let pinned = pinned_viewer(&trust, "user-1", "k1").unwrap();
        assert_eq!(pinned.key_id, "k1");
        assert_eq!(pinned.public_key, "pk-k1");
        let (code, _) = pinned_viewer(&trust, "user-1", "k2").unwrap_err();
        assert_eq!(code, RefusedCode::NotPaired);
        let (code, _) = pinned_viewer(&trust, "user-2", "k1").unwrap_err();
        assert_eq!(code, RefusedCode::NotPaired);
    }
}
