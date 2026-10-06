use super::{
    Command, HANDSHAKE_TIMEOUT, OPEN_TIMEOUT, OpenBody, State, Tunnel, closed, renewal_time,
    valid_expiry, violation,
};
use crate::{
    Error, Result,
    client::{DeviceTarget, TransportKind},
    keys::ControllerKeys,
    pipe::Pipe,
    random_bytes,
    stream::{Shared, TunnelStream},
    unix_now,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_crypto::{CryptoError, noise};
use flow_like_device_protocol::{
    TUNNEL_HEARTBEAT_SECONDS, TUNNEL_IDLE_TIMEOUT_SECONDS, TUNNEL_MAX_STREAMS, TunnelEnvelope,
    TunnelEnvelopeBody, TunnelFrame, TunnelFrameBody, TunnelRenewStart, TunnelRenewed, TunnelReset,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{
        mpsc::{self, OwnedPermit, error::SendError},
        oneshot, watch,
    },
    time::{Instant, MissedTickBehavior},
};
use zeroize::Zeroizing;

const HEARTBEAT: Duration = Duration::from_secs(TUNNEL_HEARTBEAT_SECONDS);
const IDLE_TIMEOUT: Duration = Duration::from_secs(TUNNEL_IDLE_TIMEOUT_SECONDS);
const MAX_CONTROL_FRAMES: usize = 1024;
/// Over the relay the agent queues at most 32 envelopes per tunnel and closes the tunnel
/// when that queue overflows (`transport/websocket.rs` `route_tunnel`). Pings serve as
/// barriers: the pong to a ping proves the device took every envelope sent before it.
const RELAY_IN_FLIGHT: u64 = 24;
const RELAY_BARRIER_EVERY: u64 = 8;

struct Entry {
    shared: Arc<Shared>,
    opening: Option<Opening>,
}

struct Opening {
    reply: oneshot::Sender<Result<TunnelStream>>,
    deadline: Instant,
}

struct Waiting {
    body: OpenBody,
    reply: oneshot::Sender<Result<TunnelStream>>,
    deadline: Instant,
}

/// While renewing, only renewal frames leave; everything else waits for `Renewed`.
enum Renewal {
    Started {
        handshake: Box<noise::Handshake>,
        start: Option<TunnelFrameBody>,
        deadline: Instant,
    },
    Finishing {
        finish: Option<TunnelFrameBody>,
        session: Option<noise::Session>,
        deadline: Instant,
    },
    Confirming {
        deadline: Instant,
    },
}

impl Renewal {
    fn deadline(&self) -> Instant {
        match self {
            Self::Started { deadline, .. }
            | Self::Finishing { deadline, .. }
            | Self::Confirming { deadline } => *deadline,
        }
    }

    fn has_frame(&self) -> bool {
        matches!(self, Self::Started { start: Some(_), .. })
            || matches!(
                self,
                Self::Finishing {
                    finish: Some(_),
                    ..
                }
            )
    }

    fn take_frame(&mut self) -> Option<TunnelFrameBody> {
        match self {
            Self::Started { start, .. } => start.take(),
            Self::Finishing { finish, .. } => finish.take(),
            Self::Confirming { .. } => None,
        }
    }
}

/// What a sent frame changes beyond the sequence.
enum Sent {
    RenewFinish,
    Fin,
    Other,
}

impl Sent {
    fn of(body: &TunnelFrameBody) -> Self {
        match body {
            TunnelFrameBody::RenewFinish(_) => Self::RenewFinish,
            TunnelFrameBody::Fin => Self::Fin,
            _ => Self::Other,
        }
    }
}

/// Envelopes the device has provably taken, by the sequence of each pinged barrier.
#[derive(Default)]
struct Pacing {
    barriers: VecDeque<([u8; 8], u64)>,
    taken: u64,
    last_barrier: u64,
}

pub(super) struct Driver {
    keys: Arc<ControllerKeys>,
    locked: watch::Receiver<bool>,
    device_id: String,
    grant_id: String,
    device_key: [u8; 32],
    route: String,
    pipe: Pipe,
    session: noise::Session,
    renewal: Option<Renewal>,
    sent: u64,
    received: u64,
    commands: mpsc::UnboundedReceiver<Command>,
    wake: mpsc::UnboundedSender<Command>,
    state: watch::Sender<State>,
    streams: HashMap<u32, Entry>,
    waiting: VecDeque<Waiting>,
    next_stream: u32,
    control: VecDeque<(u32, TunnelFrameBody)>,
    ready: VecDeque<u32>,
    expires_at: i64,
    renew_at: Instant,
    last_ping: Instant,
    last_received: Instant,
    tick: tokio::time::Interval,
    pacing: Option<Pacing>,
}

/// Spawns the driver of a confirmed tunnel. The device has sent frame 0 already.
pub(super) fn start(
    pipe: Pipe,
    session: noise::Session,
    expires_at: i64,
    route: String,
    keys: Arc<ControllerKeys>,
    target: &DeviceTarget,
) -> Tunnel {
    let (commands, receiver) = mpsc::unbounded_channel();
    let (state, watcher) = watch::channel(State::Open);
    let kind = pipe.kind;
    let now = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let driver = Driver {
        locked: keys.watch_lock(),
        keys,
        device_id: target.device_id.clone(),
        grant_id: target.grant_id.clone(),
        device_key: target.management_key,
        route,
        pipe,
        session,
        renewal: None,
        sent: 0,
        received: 1,
        commands: receiver,
        wake: commands.clone(),
        state,
        streams: HashMap::new(),
        waiting: VecDeque::new(),
        next_stream: 1,
        control: VecDeque::new(),
        ready: VecDeque::new(),
        expires_at,
        renew_at: renewal_time(expires_at),
        last_ping: now,
        last_received: now,
        tick,
        pacing: (kind == TransportKind::Relay).then(Pacing::default),
    };
    tokio::spawn(driver.run());
    Tunnel::new(commands, watcher, kind, &target.device_id)
}

impl Driver {
    async fn run(mut self) {
        let error = self.serve().await;
        tracing::debug!(device_id = %self.device_id, "Device tunnel ended: {error}");
        self.state.send_replace(State::Closed(error.clone()));
        for (_, entry) in self.streams.drain() {
            if let Some(opening) = entry.opening {
                let _ = opening.reply.send(Err(error.clone()));
            }
            entry.shared.fail(error.clone());
        }
        for waiting in self.waiting.drain(..) {
            let _ = waiting.reply.send(Err(error.clone()));
        }
        let close = TunnelEnvelope {
            session_id: self.route.clone(),
            body: TunnelEnvelopeBody::Close,
        }
        .encode();
        let Self { pipe, .. } = self;
        if let Ok(bytes) = close {
            let _ = pipe.outgoing.try_send(bytes);
        }
        pipe.close().await;
    }

    /// Runs until the tunnel ends and returns why it ended.
    async fn serve(&mut self) -> Error {
        let outgoing = self.pipe.outgoing.clone();
        let mut result = self.lock_changed(true);
        while result.is_ok() {
            let sendable = self.sendable();
            result = tokio::select! {
                changed = self.locked.changed() => self.lock_changed(changed.is_ok()),
                bytes = self.pipe.incoming.recv() => self.receive(bytes),
                command = self.commands.recv() => self.command(command),
                _ = self.tick.tick() => self.on_tick(),
                permit = outgoing.clone().reserve_owned(), if sendable => self.send(permit),
            };
        }
        result
            .err()
            .unwrap_or_else(|| self.closed("the tunnel driver stopped"))
    }

    fn lock_changed(&self, watching: bool) -> Result<()> {
        if !watching || *self.locked.borrow() {
            return Err(Error::Locked {
                device_id: self.device_id.clone(),
            });
        }
        Ok(())
    }

    fn closed(&self, message: impl Into<String>) -> Error {
        closed(&self.device_id, message)
    }

    fn violation(&self, message: impl std::fmt::Display) -> Error {
        violation(&self.device_id, message)
    }

    fn sendable(&self) -> bool {
        match &self.renewal {
            Some(renewal) => renewal.has_frame(),
            None => {
                (!self.control.is_empty() || !self.ready.is_empty())
                    && self
                        .pacing
                        .as_ref()
                        .is_none_or(|pacing| self.sent - pacing.taken < RELAY_IN_FLIGHT)
            }
        }
    }

    fn command(&mut self, command: Option<Command>) -> Result<()> {
        match command {
            Some(Command::Open { body, reply }) => self.open(*body, reply),
            Some(Command::Wake(id)) => {
                if self.streams.contains_key(&id) {
                    self.ready.push_back(id);
                }
                Ok(())
            }
            Some(Command::Release(id)) => self.release(id),
            Some(Command::Close) | None => Err(self.closed("this app closed the tunnel")),
        }
    }

    fn open(&mut self, body: OpenBody, reply: oneshot::Sender<Result<TunnelStream>>) -> Result<()> {
        if self.streams.len() < TUNNEL_MAX_STREAMS {
            return self.start_open(body, reply);
        }
        self.waiting.push_back(Waiting {
            body,
            reply,
            deadline: Instant::now() + OPEN_TIMEOUT,
        });
        Ok(())
    }

    fn release(&mut self, id: u32) -> Result<()> {
        if self
            .streams
            .get(&id)
            .is_some_and(|entry| !entry.shared.finished())
        {
            self.queue_reset(id, "The controller closed the stream")?;
        }
        self.remove(id)
    }

    fn start_open(
        &mut self,
        body: OpenBody,
        reply: oneshot::Sender<Result<TunnelStream>>,
    ) -> Result<()> {
        if reply.is_closed() {
            return Ok(());
        }
        let id = self.next_stream;
        let Some(next) = id.checked_add(2) else {
            let error = self.closed("stream identifiers are exhausted; reconnect");
            let _ = reply.send(Err(error.clone()));
            return Err(error);
        };
        self.next_stream = next;
        let frame = match body {
            OpenBody::Service(open) => TunnelFrameBody::Open(open),
            OpenBody::Data(open) => TunnelFrameBody::OpenData(open),
        };
        let opening = Opening {
            reply,
            deadline: Instant::now() + OPEN_TIMEOUT,
        };
        self.streams.insert(
            id,
            Entry {
                shared: Shared::new(id, self.wake.clone()),
                opening: Some(opening),
            },
        );
        self.queue_control(id, frame)
    }

    /// Frees the slot of the stream, which may start a queued open.
    fn remove(&mut self, id: u32) -> Result<()> {
        if self.streams.remove(&id).is_none() {
            return Ok(());
        }
        while self.streams.len() < TUNNEL_MAX_STREAMS
            && let Some(waiting) = self.waiting.pop_front()
        {
            self.start_open(waiting.body, waiting.reply)?;
        }
        Ok(())
    }

    fn queue_control(&mut self, stream_id: u32, body: TunnelFrameBody) -> Result<()> {
        if self.control.len() >= MAX_CONTROL_FRAMES {
            return Err(self.closed(format!(
                "more than {MAX_CONTROL_FRAMES} control frames are waiting to leave"
            )));
        }
        self.control.push_back((stream_id, body));
        Ok(())
    }

    fn queue_reset(&mut self, id: u32, message: &str) -> Result<()> {
        let reset = TunnelReset {
            code: "cancelled".into(),
            message: message.into(),
        };
        self.queue_control(id, TunnelFrameBody::Reset(reset))
    }

    fn on_tick(&mut self) -> Result<()> {
        let now = Instant::now();
        self.check_alive(now)?;
        if self.renewal.is_none() {
            self.keep_alive(now)?;
        }
        self.expire_opens(now)?;
        self.expire_waiting(now);
        Ok(())
    }

    fn check_alive(&self, now: Instant) -> Result<()> {
        if unix_now() >= self.expires_at {
            return Err(self.closed(format!(
                "its authorization expired at {} without renewal",
                self.expires_at
            )));
        }
        if now.duration_since(self.last_received) >= IDLE_TIMEOUT {
            return Err(self.closed(format!("the device sent nothing for {IDLE_TIMEOUT:?}")));
        }
        if self
            .renewal
            .as_ref()
            .is_some_and(|renewal| now >= renewal.deadline())
        {
            return Err(self.closed(format!(
                "certificate renewal did not finish within {HANDSHAKE_TIMEOUT:?}"
            )));
        }
        Ok(())
    }

    fn keep_alive(&mut self, now: Instant) -> Result<()> {
        if now >= self.renew_at {
            return self.start_renewal();
        }
        if now.duration_since(self.last_ping) >= HEARTBEAT {
            self.last_ping = now;
            self.queue_control(0, TunnelFrameBody::Ping(random_bytes::<8>()))?;
        }
        Ok(())
    }

    fn expire_opens(&mut self, now: Instant) -> Result<()> {
        let expired: Vec<u32> = self
            .streams
            .iter()
            .filter(|(_, entry)| {
                entry
                    .opening
                    .as_ref()
                    .is_some_and(|opening| now >= opening.deadline)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(opening) = self
                .streams
                .get_mut(&id)
                .and_then(|entry| entry.opening.take())
            {
                let _ = opening.reply.send(Err(Error::Unreachable {
                    device_id: self.device_id.clone(),
                    message: format!("stream {id} did not open within {OPEN_TIMEOUT:?}"),
                }));
            }
            self.queue_reset(id, "The controller stopped waiting for the stream")?;
            self.remove(id)?;
        }
        Ok(())
    }

    fn expire_waiting(&mut self, now: Instant) {
        while let Some(waiting) = self.waiting.pop_front_if(|waiting| now >= waiting.deadline) {
            let _ = waiting.reply.send(Err(Error::Unreachable {
                device_id: self.device_id.clone(),
                message: format!(
                    "all {TUNNEL_MAX_STREAMS} streams of the tunnel stayed busy for {OPEN_TIMEOUT:?}"
                ),
            }));
        }
    }

    /// Each renewal certifies a fresh Noise key; the device keeps its streams.
    fn start_renewal(&mut self) -> Result<()> {
        let certified = self
            .keys
            .begin_tunnel(&self.grant_id, self.device_key, unix_now())?;
        let mut handshake = certified.handshake;
        let first = handshake
            .write()
            .map_err(|error| self.closed(format!("starting certificate renewal: {error}")))?;
        let start = TunnelRenewStart {
            certificate_jws: certified.certificate_jws,
            data: URL_SAFE_NO_PAD.encode(first),
        };
        self.renewal = Some(Renewal::Started {
            handshake: Box::new(handshake),
            start: Some(TunnelFrameBody::RenewStart(start)),
            deadline: Instant::now() + HANDSHAKE_TIMEOUT,
        });
        Ok(())
    }

    fn barrier_due(&self) -> bool {
        self.pacing
            .as_ref()
            .is_some_and(|pacing| self.sent - pacing.last_barrier >= RELAY_BARRIER_EVERY)
    }

    /// Renewal frames first, then pacing barriers, control frames and stream turns.
    fn next_frame(&mut self) -> Option<(u32, TunnelFrameBody)> {
        if let Some(renewal) = &mut self.renewal {
            return renewal.take_frame().map(|body| (0, body));
        }
        if self.barrier_due() {
            return Some((0, TunnelFrameBody::Ping(random_bytes::<8>())));
        }
        if let Some(frame) = self.control.pop_front() {
            return Some(frame);
        }
        while let Some(id) = self.ready.pop_front() {
            let Some(entry) = self.streams.get(&id) else {
                continue;
            };
            let (frame, more) = entry.shared.next_frame();
            if more {
                self.ready.push_back(id);
            }
            if let Some(body) = frame {
                return Some((id, body));
            }
        }
        None
    }

    fn send(&mut self, permit: Result<OwnedPermit<Vec<u8>>, SendError<()>>) -> Result<()> {
        let permit = permit.map_err(|_| self.closed("the transport stopped accepting frames"))?;
        let Some((stream_id, body)) = self.next_frame() else {
            return Ok(());
        };
        if let (TunnelFrameBody::Ping(nonce), Some(pacing)) = (&body, &mut self.pacing) {
            pacing.barriers.push_back((*nonce, self.sent));
            pacing.last_barrier = self.sent;
        }
        let sent = Sent::of(&body);
        permit.send(self.seal(stream_id, body)?);
        match sent {
            Sent::RenewFinish => self.install_renewed_session(),
            Sent::Fin => self.remove_finished(stream_id),
            Sent::Other => Ok(()),
        }
    }

    fn remove_finished(&mut self, id: u32) -> Result<()> {
        if self
            .streams
            .get(&id)
            .is_some_and(|entry| entry.shared.finished())
        {
            return self.remove(id);
        }
        Ok(())
    }

    /// Encrypts the next frame of the shared sequence into an envelope.
    fn seal(&mut self, stream_id: u32, body: TunnelFrameBody) -> Result<Vec<u8>> {
        let sequence = self.sent;
        let frame = TunnelFrame {
            sequence,
            stream_id,
            body,
        };
        let plaintext = Zeroizing::new(frame.encode().map_err(|error| {
            self.closed(format!(
                "encoding frame {sequence} of stream {stream_id}: {error}"
            ))
        })?);
        let ciphertext = self
            .session
            .encrypt(&plaintext)
            .map_err(|error| self.closed(format!("encrypting frame {sequence}: {error}")))?;
        self.sent = sequence
            .checked_add(1)
            .ok_or_else(|| self.closed("the frame sequence is exhausted"))?;
        TunnelEnvelope {
            session_id: self.route.clone(),
            body: TunnelEnvelopeBody::Message(ciphertext),
        }
        .encode()
        .map_err(|error| {
            self.closed(format!(
                "encoding the envelope of frame {sequence}: {error}"
            ))
        })
    }

    /// The finish left under the old keys; every later frame uses the renewed session.
    fn install_renewed_session(&mut self) -> Result<()> {
        let Some(Renewal::Finishing {
            session: Some(session),
            deadline,
            ..
        }) = self.renewal.take()
        else {
            return Err(self.closed("certificate renewal lost its new session"));
        };
        self.session = session;
        self.renewal = Some(Renewal::Confirming { deadline });
        Ok(())
    }

    fn receive(&mut self, bytes: Option<Vec<u8>>) -> Result<()> {
        let bytes = bytes.ok_or_else(|| self.closed("the transport closed"))?;
        let frame = self.open_frame(&bytes)?;
        match &self.renewal {
            Some(Renewal::Confirming { .. })
                if !matches!(frame.body, TunnelFrameBody::Renewed(_)) =>
            {
                return Err(self.violation("traffic before the renewed tunnel was confirmed"));
            }
            Some(Renewal::Finishing { .. }) => {
                return Err(self.violation("traffic between renewal reply and finish"));
            }
            _ => {}
        }
        if frame.stream_id == 0 {
            self.control_frame(frame.body)
        } else {
            self.stream_frame(frame.stream_id, frame.body)
        }
    }

    /// Decrypts one envelope and checks that it continues the sequence of the device.
    fn open_frame(&mut self, bytes: &[u8]) -> Result<TunnelFrame> {
        let envelope = TunnelEnvelope::decode(bytes)
            .map_err(|error| self.violation(format!("invalid envelope: {error}")))?;
        if envelope.session_id != self.route {
            return Err(self.violation(format!(
                "envelope for tunnel {} on tunnel {}",
                envelope.session_id, self.route
            )));
        }
        let ciphertext = match envelope.body {
            TunnelEnvelopeBody::Message(ciphertext) => ciphertext,
            TunnelEnvelopeBody::Close => return Err(self.closed("the device closed the tunnel")),
            _ => return Err(self.violation("handshake envelope inside an open tunnel")),
        };
        let expected = self.received;
        let plaintext = Zeroizing::new(self.session.decrypt(&ciphertext).map_err(|error| {
            self.violation(format!("frame {expected} did not decrypt: {error}"))
        })?);
        let frame = TunnelFrame::decode(&plaintext)
            .map_err(|error| self.violation(format!("frame {expected}: {error}")))?;
        if frame.sequence != expected {
            return Err(self.violation(format!(
                "frame {} arrived where frame {expected} was due",
                frame.sequence
            )));
        }
        self.received += 1;
        self.last_received = Instant::now();
        Ok(frame)
    }

    fn control_frame(&mut self, body: TunnelFrameBody) -> Result<()> {
        match body {
            TunnelFrameBody::Ping(nonce) => self.queue_control(0, TunnelFrameBody::Pong(nonce)),
            TunnelFrameBody::Pong(nonce) => {
                self.acknowledge(nonce);
                Ok(())
            }
            TunnelFrameBody::RenewReply(reply) => self.renewal_reply(&reply),
            TunnelFrameBody::Renewed(TunnelRenewed { expires_at }) => self.renewed(expires_at),
            body => Err(self.violation(format!("control frame kind {}", body.kind()))),
        }
    }

    /// The device answers pings in order, so a pong covers every envelope before its ping.
    fn acknowledge(&mut self, nonce: [u8; 8]) {
        if let Some(pacing) = &mut self.pacing
            && pacing
                .barriers
                .front()
                .is_some_and(|(expected, _)| *expected == nonce)
            && let Some((_, sequence)) = pacing.barriers.pop_front()
        {
            pacing.taken = sequence + 1;
        }
    }

    fn renewal_reply(&mut self, reply: &[u8]) -> Result<()> {
        let Some(Renewal::Started {
            mut handshake,
            start: None,
            deadline,
        }) = self.renewal.take()
        else {
            return Err(self.violation("a renewal reply nobody asked for"));
        };
        let renewal_failure = |error: CryptoError| self.closed(super::handshake_failure(&error));
        handshake.read(reply).map_err(renewal_failure)?;
        let last = handshake
            .write()
            .map_err(|error| self.closed(format!("finishing certificate renewal: {error}")))?;
        let session = handshake.finish().map_err(renewal_failure)?;
        self.renewal = Some(Renewal::Finishing {
            finish: Some(TunnelFrameBody::RenewFinish(last)),
            session: Some(session),
            deadline,
        });
        Ok(())
    }

    fn renewed(&mut self, expires_at: i64) -> Result<()> {
        if !matches!(self.renewal, Some(Renewal::Confirming { .. })) {
            return Err(self.violation("an unexpected renewal confirmation"));
        }
        let expires_at =
            valid_expiry(expires_at, unix_now()).map_err(|message| self.violation(message))?;
        if expires_at <= self.expires_at {
            return Err(self.violation(format!(
                "renewal kept the expiry at {expires_at}, not after {}",
                self.expires_at
            )));
        }
        self.renewal = None;
        self.expires_at = expires_at;
        self.renew_at = renewal_time(expires_at);
        self.last_ping = Instant::now();
        Ok(())
    }

    fn stream_frame(&mut self, id: u32, body: TunnelFrameBody) -> Result<()> {
        let Some(entry) = self.streams.get(&id) else {
            return self.unknown_stream(id);
        };
        if entry.opening.is_some()
            && !matches!(body, TunnelFrameBody::Opened | TunnelFrameBody::Reset(_))
        {
            return Err(self.violation(format!("stream {id} carried data before it opened")));
        }
        let shared = entry.shared.clone();
        match body {
            TunnelFrameBody::Opened => self.opened(id, shared),
            TunnelFrameBody::Data(bytes) => shared
                .receive(bytes)
                .map_err(|message| self.violation(message)),
            TunnelFrameBody::Window(credit) => shared
                .grant(credit)
                .map_err(|message| self.violation(message)),
            TunnelFrameBody::Fin => {
                if shared
                    .finish_remote()
                    .map_err(|message| self.violation(message))?
                {
                    self.remove(id)?;
                }
                Ok(())
            }
            TunnelFrameBody::Reset(reset) => self.remote_reset(id, shared, reset),
            body => {
                Err(self.violation(format!("stream frame kind {} from the device", body.kind())))
            }
        }
    }

    /// Frames for streams this side already released are dropped; others were never opened.
    fn unknown_stream(&self, id: u32) -> Result<()> {
        if id < self.next_stream {
            return Ok(());
        }
        Err(self.violation(format!("frame for stream {id}, which was never opened")))
    }

    fn opened(&mut self, id: u32, shared: Arc<Shared>) -> Result<()> {
        let Some(opening) = self
            .streams
            .get_mut(&id)
            .and_then(|entry| entry.opening.take())
        else {
            return Err(self.violation(format!("stream {id} opened twice")));
        };
        let _ = opening.reply.send(Ok(TunnelStream::new(shared)));
        Ok(())
    }

    fn remote_reset(&mut self, id: u32, shared: Arc<Shared>, reset: TunnelReset) -> Result<()> {
        let error = Error::Reset {
            stream_id: id,
            code: reset.code,
            message: reset.message,
        };
        if let Some(opening) = self
            .streams
            .get_mut(&id)
            .and_then(|entry| entry.opening.take())
        {
            let _ = opening.reply.send(Err(error.clone()));
        }
        shared.fail(error);
        self.remove(id)
    }
}
