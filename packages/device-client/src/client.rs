use crate::{
    Error, Result,
    hub::HubClient,
    keys::ControllerKeys,
    relay::Relay,
    rtc,
    stream::TunnelStream,
    tunnel::{self, OpenBody, Tunnel},
    unix_now,
};
use flow_like_device_protocol::{
    DeviceReceipt, DeviceSignalingResponse, TunnelDataOpen, TunnelOpen, validate_management_id,
    validate_management_key,
};
use std::{
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

/// The Tauri webview's origin, which the hub's relay admits for desktop controllers.
const DESKTOP_ORIGIN: &str = if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};
const OPEN_ATTEMPTS: u32 = 4;
const MAX_SIGNALING_URLS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportKind {
    WebRtc,
    Relay,
}

/// The device to reach and the authority to reach it with.
#[derive(Clone, Debug)]
pub struct DeviceTarget {
    pub device_id: String,
    /// The device's Noise static key from its verified receipt; never from signaling.
    pub management_key: [u8; 32],
    pub auth_epoch: u64,
    /// `owner`, or the grant that holds this controller's key.
    pub grant_id: String,
}

impl DeviceTarget {
    /// `receipt` must already be verified against the pinned onboarding manifest.
    pub fn from_receipt(receipt: &DeviceReceipt, grant_id: impl Into<String>) -> Self {
        Self {
            device_id: receipt.device_id.clone(),
            management_key: receipt.identity.management_key,
            auth_epoch: receipt.auth_epoch,
            grant_id: grant_id.into(),
        }
    }

    fn validate(&self) -> Result<()> {
        validate_management_id(&self.device_id)
            .and_then(|()| validate_management_id(&self.grant_id))
            .and_then(|()| validate_management_key(&self.management_key))
            .map_err(|error| {
                Error::Invalid(format!(
                    "device target {} with grant {}: {error}",
                    self.device_id, self.grant_id
                ))
            })
    }
}

/// Opens [`DeviceSession`]s through one hub.
#[derive(Clone)]
pub struct DeviceClient {
    hub: Arc<dyn HubClient>,
}

impl DeviceClient {
    pub fn new(hub: Arc<dyn HubClient>) -> Self {
        Self { hub }
    }

    /// Establishes the first tunnel before returning, so a refusal surfaces here.
    pub async fn connect(
        &self,
        target: DeviceTarget,
        keys: Arc<ControllerKeys>,
    ) -> Result<DeviceSession> {
        target.validate()?;
        if keys.device_id() != target.device_id {
            return Err(Error::Invalid(format!(
                "the controller keys of device {} cannot open device {}",
                keys.device_id(),
                target.device_id
            )));
        }
        let session = DeviceSession {
            inner: Arc::new(SessionInner {
                hub: self.hub.clone(),
                target,
                keys,
                tunnel: Mutex::new(None),
                connecting: tokio::sync::Mutex::new(None),
                attempts: AtomicU64::new(0),
                closed: AtomicBool::new(false),
            }),
        };
        session.tunnel().await?;
        Ok(session)
    }
}

/// One long-lived tunnel to a device. A lost tunnel is replaced on the next open; streams
/// of the lost tunnel fail and are never replayed.
#[derive(Clone)]
pub struct DeviceSession {
    inner: Arc<SessionInner>,
}

struct SessionInner {
    hub: Arc<dyn HubClient>,
    target: DeviceTarget,
    keys: Arc<ControllerKeys>,
    tunnel: Mutex<Option<Tunnel>>,
    /// Held while connecting; keeps the last attempt's failure.
    connecting: tokio::sync::Mutex<Option<Error>>,
    attempts: AtomicU64,
    closed: AtomicBool,
}

impl SessionInner {
    fn current(&self) -> MutexGuard<'_, Option<Tunnel>> {
        self.tunnel.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl DeviceSession {
    pub fn device_id(&self) -> &str {
        &self.inner.target.device_id
    }

    /// The transport of the live tunnel, if one is up.
    pub fn transport(&self) -> Option<TransportKind> {
        self.inner
            .current()
            .as_ref()
            .filter(|tunnel| tunnel.is_open())
            .map(Tunnel::kind)
    }

    /// Opens a service or model-gateway stream; resolves once the device accepted it.
    pub async fn open_stream(&self, open: TunnelOpen) -> Result<TunnelStream> {
        open.validate().map_err(|error| {
            Error::Invalid(format!(
                "tunnel open for device {}: {error}",
                self.device_id()
            ))
        })?;
        self.open(OpenBody::Service(open)).await
    }

    /// Opens an internal data stream: a bulk management read, an artifact upload or a
    /// model-asset push.
    pub async fn open_data(&self, open: TunnelDataOpen) -> Result<TunnelStream> {
        open.validate().map_err(|error| {
            Error::Invalid(format!(
                "data stream open for device {}: {error}",
                self.device_id()
            ))
        })?;
        self.open(OpenBody::Data(open)).await
    }

    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::Release);
        if let Some(tunnel) = self.inner.current().take() {
            tunnel.close();
        }
    }

    /// The device refuses an open while its slot for a just-finished stream is still
    /// draining (`limit`), and a tunnel can end under an open; both are retried.
    async fn open(&self, body: OpenBody) -> Result<TunnelStream> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let tunnel = self.tunnel().await?;
            match tunnel.open(body.clone()).await {
                Err(Error::Reset { code, .. }) if code == "limit" && attempt < OPEN_ATTEMPTS => {
                    tokio::time::sleep(Duration::from_millis(25 << attempt)).await;
                }
                Err(Error::Locked { device_id }) => return Err(Error::Locked { device_id }),
                Err(_) if attempt == 1 && !tunnel.is_open() => {}
                result => return result,
            }
        }
    }

    /// Callers that waited for an attempt share its failure instead of each starting one.
    async fn tunnel(&self) -> Result<Tunnel> {
        if let Some(tunnel) = self.live() {
            return Ok(tunnel);
        }
        let seen = self.inner.attempts.load(Ordering::Acquire);
        let mut failure = self.inner.connecting.lock().await;
        if let Some(tunnel) = self.live() {
            return Ok(tunnel);
        }
        if let Some(error) = failure
            .as_ref()
            .filter(|_| self.inner.attempts.load(Ordering::Acquire) != seen)
        {
            return Err(error.clone());
        }
        self.ensure_open()?;
        let attempt = connect(&self.inner.hub, &self.inner.target, &self.inner.keys).await;
        self.inner.attempts.fetch_add(1, Ordering::AcqRel);
        *failure = attempt.as_ref().err().cloned();
        let tunnel = attempt?;
        let mut current = self.inner.current();
        if self.inner.closed.load(Ordering::Acquire) {
            tunnel.close();
            return Err(self.session_closed());
        }
        *current = Some(tunnel.clone());
        Ok(tunnel)
    }

    fn live(&self) -> Option<Tunnel> {
        self.inner
            .current()
            .as_ref()
            .filter(|tunnel| tunnel.is_open())
            .cloned()
    }

    fn ensure_open(&self) -> Result<()> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(self.session_closed());
        }
        Ok(())
    }

    fn session_closed(&self) -> Error {
        Error::Closed {
            device_id: self.device_id().into(),
            message: "the device session was closed".into(),
        }
    }

    #[cfg(test)]
    pub(crate) fn live_tunnel(&self) -> Option<Tunnel> {
        self.live()
    }
}

/// WebRTC is usable only after the encrypted tunnel opens. A failed transport setup
/// falls back to the relay before any application stream is sent.
async fn connect(
    hub: &Arc<dyn HubClient>,
    target: &DeviceTarget,
    keys: &Arc<ControllerKeys>,
) -> Result<Tunnel> {
    if keys.is_locked() {
        return Err(Error::Locked {
            device_id: target.device_id.clone(),
        });
    }
    let participant = uuid::Uuid::new_v4().to_string();
    let admission = hub
        .controller_admission(&target.device_id, &participant)
        .await?;
    validate_admission(&admission, target, unix_now())?;
    let mut relay =
        Relay::open(&admission, &participant, &target.device_id, DESKTOP_ORIGIN).await?;
    let direct = keys.begin_tunnel(&target.grant_id, target.management_key, unix_now())?;
    let direct_failure = match rtc::connect(
        &mut relay,
        &admission,
        &direct,
        &target.grant_id,
        &target.device_id,
    )
    .await
    {
        Ok(pipe) => match tunnel::establish(pipe, direct, keys.clone(), target.clone()).await {
            Ok(tunnel) => return Ok(tunnel),
            Err(error @ tunnel::EstablishError::Rejected(_)) => {
                return Err(error.into_error(&target.device_id));
            }
            Err(error) => error.into_error(&target.device_id),
        },
        Err(error) => error,
    };
    tracing::info!(device_id = %target.device_id, "Device tunnel uses the relay: {direct_failure}");
    // The failed offer may still occupy a device slot. Use another certificate and Noise key.
    let certified = keys.begin_tunnel(&target.grant_id, target.management_key, unix_now())?;
    let pipe = relay.into_pipe(hub.clone(), target.auth_epoch);
    tunnel::establish(pipe, certified, keys.clone(), target.clone())
        .await
        .map_err(|error| {
            let error = error.into_error(&target.device_id);
            Error::Unreachable {
                device_id: target.device_id.clone(),
                message: format!("WebRTC failed: {direct_failure}; relay failed: {error}"),
            }
        })
}

pub(crate) fn validate_admission(
    admission: &DeviceSignalingResponse,
    target: &DeviceTarget,
    now: i64,
) -> Result<()> {
    let refused = |message: String| Error::Refused {
        device_id: target.device_id.clone(),
        status: None,
        message,
    };
    if admission.device_auth_epoch != target.auth_epoch {
        return Err(refused(format!(
            "the hub admitted auth epoch {}, but the trusted receipt is at epoch {}; refresh the device identity",
            admission.device_auth_epoch, target.auth_epoch
        )));
    }
    if admission.expires_at <= now + 10 || admission.expires_at > now + 305 {
        return Err(refused(format!(
            "the admission expires at {}, outside 10 to 305 s from now ({now})",
            admission.expires_at
        )));
    }
    if admission.signaling_urls.is_empty() || admission.signaling_urls.len() > MAX_SIGNALING_URLS {
        return Err(refused(format!(
            "the admission lists {} signaling endpoints, not 1 to {MAX_SIGNALING_URLS}",
            admission.signaling_urls.len()
        )));
    }
    Ok(())
}
