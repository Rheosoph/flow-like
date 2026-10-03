mod rtc;
mod websocket;
mod wire;

use crate::{
    enrollment::{DeviceSession, device_proof_rejected, unix_time},
    management::{ManagementConnection, ManagementService},
};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::DeviceSignalingResponse;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use wire::NoiseEnvelope;

const MAX_CONNECTIONS: usize = 8;
/// Grantees share the remaining slots, so the owner can always open a session.
const OWNER_RESERVED_CONNECTIONS: usize = 2;
const GRANT_CONNECTIONS: usize = 2;
const OWNER_GRANT: &str = "owner";
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
                let admission = device.signaling().await.context("Request signaling admission")?;
                management
                    .synchronize_policy(&admission)
                    .await
                    .context("Synchronize management policy")?;
                let local_expiry = management.refresh_authority(admission.expires_at)?;
                Ok::<_, anyhow::Error>((admission, local_expiry))
            } => value,
        };
        let delay = match result {
            Ok((admission, local_expiry)) => {
                failures = 0;
                let remaining = local_expiry.saturating_sub(unix_time().unwrap_or(local_expiry));
                let jitter = uuid::Uuid::new_v4().as_bytes()[0] as u64 % 11;
                let delay = Duration::from_secs(
                    (remaining.max(0) as u64).saturating_sub(60 + jitter).max(1),
                );
                if sender.send(Some(Arc::new(admission))).is_err() {
                    break;
                }
                delay
            }
            Err(error) => {
                failures = failures.saturating_add(1);
                renewal_failed(&error);
                Duration::from_secs(2u64.saturating_pow(failures.min(5)))
            }
        };
        tokio::select! { _ = cancel.cancelled() => break, _ = tokio::time::sleep(delay) => {} }
    }
}

/// Marks the transport task failing; only a confirmed signaling socket reports it healthy
/// again.
fn renewal_failed(error: &anyhow::Error) {
    crate::diagnostics::global().report(
        crate::diagnostics::MANAGEMENT_TRANSPORT,
        Err(crate::diagnostics::TaskFailure::classify(error)),
    );
    if device_proof_rejected(error) {
        tracing::warn!(
            "Device management admission proof was rejected, most likely because the device clock is skewed; enable time synchronization. Retrying: {error:#}"
        );
    } else {
        tracing::warn!(
            "Device management admission renewal failed; current admission expires normally: {error:#}"
        );
    }
}

#[derive(Default)]
struct SessionRegistry {
    slots: Mutex<Slots>,
}

#[derive(Default)]
struct Slots {
    serial: u64,
    sessions: HashMap<String, Slot>,
}

struct Slot {
    grant_id: String,
    serial: u64,
    cancel: CancellationToken,
}

impl SessionRegistry {
    /// Reserve after the controller certificate is verified, so the grant is authentic.
    /// A grant at its limit replaces its own oldest session, never another grant's.
    fn reserve(
        self: &Arc<Self>,
        id: &str,
        grant_id: &str,
        parent: &CancellationToken,
    ) -> Result<SessionPermit> {
        wire::identifier(id)?;
        wire::identifier(grant_id)?;
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| anyhow::anyhow!("Management session registry unavailable"))?;
        ensure!(
            !slots.sessions.contains_key(id),
            "Management session {id} already exists"
        );
        let owner = grant_id == OWNER_GRANT;
        let own = slots
            .sessions
            .values()
            .filter(|slot| slot.grant_id == grant_id)
            .count();
        let shared = slots
            .sessions
            .values()
            .filter(|slot| slot.grant_id != OWNER_GRANT)
            .count();
        let full = slots.sessions.len() >= MAX_CONNECTIONS
            || (!owner
                && (own >= GRANT_CONNECTIONS
                    || shared >= MAX_CONNECTIONS - OWNER_RESERVED_CONNECTIONS));
        if full {
            let oldest = slots
                .sessions
                .iter()
                .filter(|(_, slot)| slot.grant_id == grant_id)
                .min_by_key(|(_, slot)| slot.serial)
                .map(|(id, _)| id.clone())
                .with_context(|| {
                    format!(
                        "Management session limit reached; grant {grant_id} has no session to replace"
                    )
                })?;
            if let Some(slot) = slots.sessions.remove(&oldest) {
                slot.cancel.cancel();
                tracing::info!(
                    grant_id,
                    session_id = %oldest,
                    "Replaced the oldest management session of this grant"
                );
            }
        }
        slots.serial += 1;
        let serial = slots.serial;
        let cancel = parent.child_token();
        slots.sessions.insert(
            id.to_owned(),
            Slot {
                grant_id: grant_id.to_owned(),
                serial,
                cancel: cancel.clone(),
            },
        );
        Ok(SessionPermit {
            registry: self.clone(),
            id: id.to_owned(),
            serial,
            cancel,
        })
    }
}

struct SessionPermit {
    registry: Arc<SessionRegistry>,
    id: String,
    serial: u64,
    cancel: CancellationToken,
}
impl Drop for SessionPermit {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.registry.slots.lock()
            && slots
                .sessions
                .get(&self.id)
                .is_some_and(|slot| slot.serial == self.serial)
        {
            slots.sessions.remove(&self.id);
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
            NoiseEnvelope::Close { .. } => {
                anyhow::bail!("Controller closed the management session")
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

    fn held(registry: &SessionRegistry) -> Vec<String> {
        let mut ids: Vec<_> = registry
            .slots
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn grantees_cannot_take_the_owner_slots_and_only_replace_their_own_sessions() {
        let registry = Arc::new(SessionRegistry::default());
        let parent = CancellationToken::new();
        let first = registry.reserve("a-1", "grant-a", &parent).unwrap();
        let _second = registry.reserve("a-2", "grant-a", &parent).unwrap();
        let _third = registry.reserve("a-3", "grant-a", &parent).unwrap();
        assert!(first.cancel.is_cancelled());
        assert_eq!(held(&registry), ["a-2", "a-3"]);
        let _others = ["b-1", "b-2", "c-1", "c-2"].map(|id| {
            registry
                .reserve(id, &format!("grant-{}", &id[..1]), &parent)
                .unwrap()
        });
        assert!(registry.reserve("d-1", "grant-d", &parent).is_err());
        let owner = [
            registry.reserve("owner-1", OWNER_GRANT, &parent).unwrap(),
            registry.reserve("owner-2", OWNER_GRANT, &parent).unwrap(),
        ];
        assert_eq!(held(&registry).len(), MAX_CONNECTIONS);
        let _replacement = registry.reserve("owner-3", OWNER_GRANT, &parent).unwrap();
        assert!(owner[0].cancel.is_cancelled());
        assert!(!owner[1].cancel.is_cancelled());
        assert!(held(&registry).contains(&"owner-3".to_owned()));
        assert!(!held(&registry).contains(&"owner-1".to_owned()));
        assert!(registry.reserve("d-1", "grant-d", &parent).is_err());
        assert!(registry.reserve("a-2", "grant-a", &parent).is_err());
        assert!(registry.reserve("../session", "grant-a", &parent).is_err());
    }

    #[test]
    fn a_replaced_permit_never_releases_a_reused_session_id() {
        let registry = Arc::new(SessionRegistry::default());
        let parent = CancellationToken::new();
        let replaced = registry.reserve("s-1", "grant-a", &parent).unwrap();
        let kept = registry.reserve("s-2", "grant-a", &parent).unwrap();
        let newest = registry.reserve("s-3", "grant-a", &parent).unwrap();
        assert!(replaced.cancel.is_cancelled());
        drop(newest);
        let reused = registry.reserve("s-1", "grant-a", &parent).unwrap();
        drop(replaced);
        assert_eq!(held(&registry), ["s-1", "s-2"]);
        drop(reused);
        assert_eq!(held(&registry), ["s-2"]);
        parent.cancel();
        assert!(kept.cancel.is_cancelled());
    }
}
