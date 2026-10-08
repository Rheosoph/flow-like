mod driver;

use crate::{
    Error, Result,
    client::{DeviceTarget, TransportKind},
    keys::ControllerKeys,
    pipe::Pipe,
    stream::TunnelStream,
    unix_now,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_crypto::{CryptoError, controller::CertifiedHandshake, noise};
use flow_like_device_protocol::{
    TUNNEL_RENEW_BEFORE_SECONDS, TUNNEL_RENEW_TIMEOUT_SECONDS, TunnelDataOpen, TunnelEnvelope,
    TunnelEnvelopeBody, TunnelFrame, TunnelFrameBody, TunnelHello, TunnelOpen, TunnelRenewed,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};
use zeroize::Zeroizing;

/// Agents never answer a hello they refuse, so only this timeout ends that wait; tests
/// shorten it.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(if cfg!(test) {
    5
} else {
    TUNNEL_RENEW_TIMEOUT_SECONDS
});
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// How long an open waits for one of the tunnel's streams to free up, and then for the
/// device to accept it.
pub(crate) const OPEN_TIMEOUT: Duration = Duration::from_secs(20);
/// A device certificate lasts 300 s; anything later than this is not a real expiry.
const MAX_EXPIRY_AHEAD: i64 = 305;

/// Only transport failures may try another route; identity and protocol refusals stay final.
#[derive(Debug)]
pub(crate) enum EstablishError {
    Transport(String),
    Rejected(String),
}

impl EstablishError {
    pub(crate) fn into_error(self, device_id: &str) -> Error {
        let (Self::Transport(message) | Self::Rejected(message)) = self;
        Error::Unreachable {
            device_id: device_id.into(),
            message,
        }
    }
}

impl From<String> for EstablishError {
    fn from(message: String) -> Self {
        Self::Rejected(message)
    }
}

impl From<&str> for EstablishError {
    fn from(message: &str) -> Self {
        Self::Rejected(message.into())
    }
}

#[derive(Clone)]
pub(crate) enum OpenBody {
    Service(TunnelOpen),
    Data(TunnelDataOpen),
}

pub(crate) enum Command {
    Open {
        body: Box<OpenBody>,
        reply: oneshot::Sender<Result<TunnelStream>>,
    },
    Wake(u32),
    Release(u32),
    Close,
}

#[derive(Clone)]
enum State {
    Open,
    Closed(Error),
}

/// A live encrypted tunnel. The last handle to go away closes it.
#[derive(Clone)]
pub(crate) struct Tunnel {
    handle: Arc<Handle>,
}

struct Handle {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<State>,
    kind: TransportKind,
    device_id: String,
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Close);
    }
}

impl Tunnel {
    fn new(
        commands: mpsc::UnboundedSender<Command>,
        state: watch::Receiver<State>,
        kind: TransportKind,
        device_id: &str,
    ) -> Self {
        Self {
            handle: Arc::new(Handle {
                commands,
                state,
                kind,
                device_id: device_id.into(),
            }),
        }
    }

    pub(crate) fn kind(&self) -> TransportKind {
        self.handle.kind
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(*self.handle.state.borrow(), State::Open)
    }

    pub(crate) fn close(&self) {
        let _ = self.handle.commands.send(Command::Close);
    }

    /// Resolves once the device has opened the stream, or with its reset.
    pub(crate) async fn open(&self, body: OpenBody) -> Result<TunnelStream> {
        let (reply, opened) = oneshot::channel();
        if self
            .handle
            .commands
            .send(Command::Open {
                body: Box::new(body),
                reply,
            })
            .is_err()
        {
            return Err(self.closed_error());
        }
        opened.await.map_err(|_| self.closed_error())?
    }

    /// The reason the tunnel ended, once it has.
    #[cfg(test)]
    pub(crate) async fn closed(&self) -> Error {
        let mut state = self.handle.state.clone();
        loop {
            if let State::Closed(error) = &*state.borrow_and_update() {
                return error.clone();
            }
            if state.changed().await.is_err() {
                return self.closed_error();
            }
        }
    }

    fn closed_error(&self) -> Error {
        match &*self.handle.state.borrow() {
            State::Closed(error) => error.clone(),
            State::Open => Error::Closed {
                device_id: self.handle.device_id.clone(),
                message: "the tunnel driver stopped".into(),
            },
        }
    }
}

/// Runs the Noise XX initiator inside `pipe` and starts the driver once the device
/// confirms the tunnel with its first `Renewed` frame.
pub(crate) async fn establish(
    mut pipe: Pipe,
    certified: CertifiedHandshake,
    keys: Arc<ControllerKeys>,
    target: DeviceTarget,
) -> Result<Tunnel, EstablishError> {
    let route = certified.certificate.session_id.clone();
    let confirmed = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        handshake(&mut pipe, certified, &target.grant_id, &route),
    )
    .await
    .unwrap_or_else(|_| {
        Err(EstablishError::Transport(format!(
            "the device did not finish the tunnel handshake within {HANDSHAKE_TIMEOUT:?}"
        )))
    });
    let (session, expires_at) = confirmed?;
    Ok(driver::start(
        pipe, session, expires_at, route, keys, &target,
    ))
}

async fn handshake(
    pipe: &mut Pipe,
    certified: CertifiedHandshake,
    grant_id: &str,
    route: &str,
) -> Result<(noise::Session, i64), EstablishError> {
    let CertifiedHandshake {
        certificate_jws,
        mut handshake,
        ..
    } = certified;
    let first = handshake
        .write()
        .map_err(|error| format!("starting the tunnel handshake: {error}"))?;
    let hello = TunnelHello {
        grant_id: grant_id.into(),
        certificate_jws,
        data: URL_SAFE_NO_PAD.encode(first),
    };
    send_envelope(pipe, route, TunnelEnvelopeBody::Hello(hello)).await?;
    let mut session = respond(pipe, route, handshake).await?;
    let expires_at = confirmation(pipe, &mut session, route).await?;
    Ok((session, expires_at))
}

/// The reply of the device proves its pinned static key; the last message completes XX.
async fn respond(
    pipe: &mut Pipe,
    route: &str,
    mut handshake: noise::Handshake,
) -> Result<noise::Session, EstablishError> {
    let TunnelEnvelopeBody::Handshake(reply) = receive_envelope(pipe, route).await? else {
        return Err("the device skipped its handshake reply".into());
    };
    handshake
        .read(&reply)
        .map_err(|error| handshake_failure(&error))?;
    let last = handshake
        .write()
        .map_err(|error| format!("finishing the tunnel handshake: {error}"))?;
    send_envelope(pipe, route, TunnelEnvelopeBody::Handshake(last)).await?;
    handshake
        .finish()
        .map_err(|error| handshake_failure(&error).into())
}

/// The device confirms the tunnel with `Renewed` at sequence 0, under the new session.
async fn confirmation(
    pipe: &mut Pipe,
    session: &mut noise::Session,
    route: &str,
) -> Result<i64, EstablishError> {
    let TunnelEnvelopeBody::Message(ciphertext) = receive_envelope(pipe, route).await? else {
        return Err("the device did not confirm the tunnel".into());
    };
    let plaintext = Zeroizing::new(
        session
            .decrypt(&ciphertext)
            .map_err(|error| format!("decrypting the tunnel confirmation: {error}"))?,
    );
    match TunnelFrame::decode(&plaintext) {
        Ok(TunnelFrame {
            sequence: 0,
            stream_id: 0,
            body: TunnelFrameBody::Renewed(TunnelRenewed { expires_at }),
        }) => valid_expiry(expires_at, unix_now()).map_err(EstablishError::Rejected),
        _ => Err("the device confirmed the tunnel with another frame".into()),
    }
}

fn handshake_failure(error: &CryptoError) -> String {
    match error {
        CryptoError::UntrustedPeer => {
            "the Noise key of the device does not match its pinned identity".into()
        }
        error => format!("the tunnel handshake failed: {error}"),
    }
}

fn valid_expiry(expires_at: i64, now: i64) -> Result<i64, String> {
    if expires_at <= now || expires_at > now + MAX_EXPIRY_AHEAD {
        return Err(format!(
            "the device reported tunnel expiry {expires_at}, outside the next {MAX_EXPIRY_AHEAD} s"
        ));
    }
    Ok(expires_at)
}

fn renewal_time(expires_at: i64) -> Instant {
    let lead = (expires_at - unix_now() - TUNNEL_RENEW_BEFORE_SECONDS).max(1);
    Instant::now() + Duration::from_secs(lead as u64)
}

async fn send_envelope(
    pipe: &Pipe,
    route: &str,
    body: TunnelEnvelopeBody,
) -> Result<(), EstablishError> {
    let bytes = TunnelEnvelope {
        session_id: route.into(),
        body,
    }
    .encode()
    .map_err(|error| format!("encoding a tunnel envelope: {error}"))?;
    tokio::time::timeout(SEND_TIMEOUT, pipe.outgoing.send(bytes))
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| {
            EstablishError::Transport("the transport stopped accepting the handshake".into())
        })
}

async fn receive_envelope(
    pipe: &mut Pipe,
    route: &str,
) -> Result<TunnelEnvelopeBody, EstablishError> {
    let bytes = pipe.incoming.recv().await.ok_or_else(|| {
        EstablishError::Transport("the transport closed during the handshake".into())
    })?;
    let envelope = TunnelEnvelope::decode(&bytes)
        .map_err(|error| format!("the device sent an invalid envelope: {error}"))?;
    if envelope.session_id != route {
        return Err(format!(
            "the device answered tunnel {}, not {route}",
            envelope.session_id
        )
        .into());
    }
    if envelope.body == TunnelEnvelopeBody::Close {
        return Err("the device refused the tunnel".into());
    }
    Ok(envelope.body)
}

fn closed(device_id: &str, message: impl Into<String>) -> Error {
    Error::Closed {
        device_id: device_id.into(),
        message: message.into(),
    }
}

fn violation(device_id: &str, message: impl std::fmt::Display) -> Error {
    closed(device_id, format!("protocol violation: {message}"))
}
