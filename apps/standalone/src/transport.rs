mod rtc;
mod websocket;
mod wire;

use crate::{
    enrollment::{DeviceSession, unix_time},
    management::{ManagementConnection, ManagementService},
};
use anyhow::{Result, ensure};
use flow_like_device_protocol::DeviceSignalingResponse;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use wire::NoiseEnvelope;

const MAX_CONNECTIONS: usize = 8;
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// Signaling renews independently of peer connections and deployed workloads.
pub async fn run(
    device: Arc<DeviceSession>,
    management: Arc<ManagementService>,
    cancel: CancellationToken,
) -> Result<()> {
    let (sender, receiver) = watch::channel(None);
    let child = cancel.child_token();
    let renewal = tokio::spawn(renew_admission(
        device.clone(),
        management.clone(),
        sender,
        child.clone(),
    ));
    let registry = Arc::new(SessionRegistry::default());
    let result = websocket::run(
        device.manifest().device_id.clone(),
        management,
        registry,
        receiver,
        child.clone(),
    )
    .await;
    child.cancel();
    let _ = renewal.await;
    result
}

async fn renew_admission(
    device: Arc<DeviceSession>,
    management: Arc<ManagementService>,
    sender: watch::Sender<Option<Arc<DeviceSignalingResponse>>>,
    cancel: CancellationToken,
) {
    let mut failures = 0u32;
    loop {
        let result = tokio::select! {
            _ = cancel.cancelled() => break,
            value = async {
                let admission = device.signaling().await?;
                management.synchronize_policy(&admission).await?;
                Ok::<_, anyhow::Error>(admission)
            } => value,
        };
        let delay = match result {
            Ok(admission) if management.refresh_authority(admission.expires_at).is_ok() => {
                failures = 0;
                let remaining = admission
                    .expires_at
                    .saturating_sub(unix_time().unwrap_or(admission.expires_at));
                let jitter = uuid::Uuid::new_v4().as_bytes()[0] as u64 % 11;
                let delay = Duration::from_secs(
                    (remaining.max(0) as u64).saturating_sub(60 + jitter).max(1),
                );
                if sender.send(Some(Arc::new(admission))).is_err() {
                    break;
                }
                delay
            }
            _ => {
                failures = failures.saturating_add(1);
                tracing::warn!(
                    "Device management admission renewal failed; current admission expires normally"
                );
                Duration::from_secs(2u64.saturating_pow(failures.min(5)))
            }
        };
        tokio::select! { _ = cancel.cancelled() => break, _ = tokio::time::sleep(delay) => {} }
    }
}

#[derive(Default)]
struct SessionRegistry {
    ids: Mutex<HashSet<String>>,
}

impl SessionRegistry {
    fn reserve(self: &Arc<Self>, id: &str) -> Result<SessionPermit> {
        wire::identifier(id)?;
        let mut ids = self
            .ids
            .lock()
            .map_err(|_| anyhow::anyhow!("Management session registry unavailable"))?;
        ensure!(
            ids.len() < MAX_CONNECTIONS && !ids.contains(id),
            "Management session already exists or connection limit reached"
        );
        ids.insert(id.to_owned());
        Ok(SessionPermit {
            registry: self.clone(),
            id: id.to_owned(),
        })
    }
}

struct SessionPermit {
    registry: Arc<SessionRegistry>,
    id: String,
}
impl Drop for SessionPermit {
    fn drop(&mut self) {
        if let Ok(mut ids) = self.registry.ids.lock() {
            ids.remove(&self.id);
        }
    }
}

struct NoiseConnection {
    connection: ManagementConnection,
    session_id: String,
    certificate_jws: String,
    grant_id: String,
    step: u8,
}

impl NoiseConnection {
    fn new(
        management: &Arc<ManagementService>,
        session_id: &str,
        grant_id: &str,
        certificate_jws: &str,
    ) -> Result<Self> {
        Ok(Self {
            connection: management.connect(certificate_jws, grant_id, session_id)?,
            session_id: session_id.to_owned(),
            certificate_jws: certificate_jws.to_owned(),
            grant_id: grant_id.to_owned(),
            step: 0,
        })
    }

    async fn receive(&mut self, envelope: NoiseEnvelope) -> Result<Vec<u8>> {
        ensure!(
            envelope.session_id() == self.session_id,
            "Management session mismatch"
        );
        let data = match envelope {
            NoiseEnvelope::Hello {
                grant_id,
                certificate_jws,
                data,
                ..
            } => {
                ensure!(
                    self.step == 0
                        && grant_id == self.grant_id
                        && certificate_jws == self.certificate_jws,
                    "Unexpected management hello"
                );
                data
            }
            NoiseEnvelope::Handshake { data, .. } => {
                ensure!(self.step == 1, "Unexpected management handshake");
                data
            }
            NoiseEnvelope::Message { data, .. } => {
                ensure!(self.step == 2, "Management handshake incomplete");
                data
            }
        };
        let response = self.connection.receive(&wire::decode(&data)?).await?;
        let envelope = if self.step == 0 {
            self.step = 1;
            NoiseEnvelope::Handshake {
                session_id: self.session_id.clone(),
                data: wire::encode(&response),
            }
        } else {
            self.step = 2;
            NoiseEnvelope::Message {
                session_id: self.session_id.clone(),
                data: wire::encode(&response),
            }
        };
        wire::serialize(&envelope)
    }

    fn timeout(&self) -> Duration {
        if self.step < 2 {
            HANDSHAKE_TIMEOUT
        } else {
            IDLE_TIMEOUT
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::noise;
    use flow_like_device_protocol::{
        ControllerCertificate, SigningKey, sign_controller_certificate,
    };

    pub(super) struct Fixture {
        pub directory: tempfile::TempDir,
        pub management: Arc<ManagementService>,
        pub certificate: String,
        pub initiator: noise::Handshake,
    }

    pub(super) fn fixture() -> Result<Fixture> {
        let directory = tempfile::tempdir()?;
        let controller = SigningKey::generate();
        let device = Arc::new(DeviceSession::test_management_session(
            "http://127.0.0.1:1/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
            controller.public_key(),
        ));
        let management = ManagementService::new(directory.path().into(), device, "boot".into());
        let now = unix_time()?;
        management.refresh_authority(now + 300)?;
        let certificate = sign_controller_certificate(
            &ControllerCertificate {
                version: 1,
                device_id: "device".into(),
                grant_id: "owner".into(),
                session_id: "session".into(),
                management_key: x25519_dalek::x25519([7; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
                issued_at: now,
                expires_at: now + 300,
            },
            &controller,
        )?;
        let initiator = noise::Handshake::initiator(
            &[7; 32],
            x25519_dalek::x25519([42; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
            "device",
            "session",
        )?;
        Ok(Fixture {
            directory,
            management,
            certificate,
            initiator,
        })
    }

    #[test]
    fn session_capacity_is_shared_and_released_without_replacement() {
        let registry = Arc::new(SessionRegistry::default());
        let mut permits = (0..MAX_CONNECTIONS)
            .map(|id| registry.reserve(&format!("session-{id}")).unwrap())
            .collect::<Vec<_>>();
        assert!(registry.reserve("session-0").is_err());
        assert!(registry.reserve("extra").is_err());
        permits.pop();
        let extra = registry.reserve("extra").unwrap();
        drop(extra);
        drop(permits);
        assert!(registry.reserve("session-0").is_ok());
        assert!(registry.reserve("../session").is_err());
    }
}
