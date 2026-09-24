//! Browser persistence is asynchronous. Each transition keeps its output private
//! until the caller confirms an atomic ciphertext and checkpoint replacement.

use crate::{mls::MemberIdentity, mls_store::*};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::SigningKey;
use serde::Serialize;
use zeroize::Zeroizing;

#[derive(Clone, Serialize)]
pub struct MlsPreparation {
    pub previous_checkpoint: Option<MlsCheckpoint>,
    pub checkpoint: MlsCheckpoint,
    pub snapshot: ProtectedSnapshot,
}

struct PendingTransition {
    prepared: MlsPreparation,
    output: Zeroizing<Vec<u8>>,
}

/// This private backend commits to a temporary buffer. Its enclosing endpoint
/// supplies the durable boundary and prevents any result from escaping early.
#[derive(Default)]
struct BufferedBackend {
    snapshot: Option<ProtectedSnapshot>,
}

impl SnapshotBackend for BufferedBackend {
    fn transact<R>(
        &mut self,
        operation: impl FnOnce(Option<ProtectedSnapshot>) -> Result<(ProtectedSnapshot, R)>,
    ) -> Result<R> {
        let (next, result) = operation(self.snapshot.clone())?;
        self.snapshot = Some(next);
        Ok(result)
    }
}

/// One browser endpoint, held under an exclusive Web Lock across tabs. Persist
/// `prepared()` in one IndexedDB transaction that compares previous_checkpoint,
/// replaces the ciphertext, and advances its witness. Only then call
/// `confirm_commit`. A failed comparison requires discarding this object and
/// reopening the winning snapshot. Never copy private endpoint state to another
/// browser or restore a backup as an active leaf.
pub struct PreparedMlsEndpoint {
    pins: MlsPins,
    key: Zeroizing<[u8; 32]>,
    committed: Option<ProtectedSnapshot>,
    checkpoint: Option<MlsCheckpoint>,
    pending: Option<PendingTransition>,
}

impl PreparedMlsEndpoint {
    pub fn create(pins: MlsPins, key: [u8; 32], local_key: &SigningKey) -> Result<Self> {
        let store =
            ProtectedMlsStore::create(BufferedBackend::default(), pins.clone(), key, local_key)?;
        let checkpoint = store.checkpoint();
        let snapshot = store
            .into_backend()
            .snapshot
            .context("Missing prepared MLS state")?;
        Ok(Self {
            pins,
            key: Zeroizing::new(key),
            committed: None,
            checkpoint: None,
            pending: Some(PendingTransition {
                prepared: MlsPreparation {
                    previous_checkpoint: None,
                    checkpoint,
                    snapshot,
                },
                output: Zeroizing::new(br#"{"kind":"created"}"#.to_vec()),
            }),
        })
    }

    pub fn open(
        pins: MlsPins,
        key: [u8; 32],
        snapshot: ProtectedSnapshot,
        minimum: MlsCheckpoint,
    ) -> Result<Self> {
        let store = ProtectedMlsStore::open(
            BufferedBackend {
                snapshot: Some(snapshot),
            },
            pins.clone(),
            key,
            minimum,
        )?;
        let checkpoint = store.checkpoint();
        Ok(Self {
            pins,
            key: Zeroizing::new(key),
            committed: store.into_backend().snapshot,
            checkpoint: Some(checkpoint),
            pending: None,
        })
    }

    /// This result contains only authenticated ciphertext and public witnesses.
    pub fn prepared(&self) -> Result<MlsPreparation> {
        Ok(self
            .pending
            .as_ref()
            .context("No MLS transition awaits persistence")?
            .prepared
            .clone())
    }

    pub fn checkpoint(&self) -> Option<MlsCheckpoint> {
        self.checkpoint.clone()
    }

    pub fn reader_position(&self) -> Result<(bool, bool, u64)> {
        ensure!(
            self.pending.is_none(),
            "Persist the pending MLS transition first"
        );
        let mut store = ProtectedMlsStore::open(
            BufferedBackend {
                snapshot: self.committed.clone(),
            },
            self.pins.clone(),
            *self.key,
            self.checkpoint
                .clone()
                .context("Initial MLS state has not been committed")?,
        )?;
        store.reader_position()
    }

    pub fn delivery_receipts(
        &self,
    ) -> Result<Vec<(flow_like_device_protocol::TelemetryDeliveryReceipt, String)>> {
        ensure!(
            self.pending.is_none(),
            "Persist the pending MLS transition first"
        );
        let mut store = ProtectedMlsStore::open(
            BufferedBackend {
                snapshot: self.committed.clone(),
            },
            self.pins.clone(),
            *self.key,
            self.checkpoint
                .clone()
                .context("Initial MLS state has not been committed")?,
        )?;
        store
            .pending_delivery_receipts()?
            .into_iter()
            .map(|receipt| {
                let signed = store.signed_delivery_receipt(receipt.sequence)?;
                Ok((receipt, signed))
            })
            .collect()
    }
    pub fn prepare_receipt_confirmation(
        &mut self,
        sequence: u64,
        envelope_digest: &str,
    ) -> Result<()> {
        self.prepare(|store| {
            store.confirm_delivery_receipt(sequence, envelope_digest)?;
            encode_output(&serde_json::json!({"kind":"receipt_confirmed","sequence":sequence}))
        })
    }

    /// The caller has committed exactly this snapshot and checkpoint. Passing
    /// an unrelated confirmation leaves the pending result inaccessible.
    pub fn confirm_commit(&mut self, checkpoint: &MlsCheckpoint) -> Result<Zeroizing<Vec<u8>>> {
        let pending = self
            .pending
            .as_ref()
            .context("No MLS transition awaits persistence")?;
        ensure!(
            &pending.prepared.checkpoint == checkpoint,
            "MLS persistence confirmation changed"
        );
        let pending = self.pending.take().expect("checked pending transition");
        self.committed = Some(pending.prepared.snapshot);
        self.checkpoint = Some(pending.prepared.checkpoint);
        Ok(pending.output)
    }

    pub fn discard_prepared(&mut self) {
        self.pending = None;
    }

    fn prepare(
        &mut self,
        operation: impl FnOnce(&mut ProtectedMlsStore<BufferedBackend>) -> Result<Zeroizing<Vec<u8>>>,
    ) -> Result<()> {
        ensure!(
            self.pending.is_none(),
            "Persist or discard the pending MLS transition first"
        );
        let minimum = self
            .checkpoint
            .clone()
            .context("Initial MLS state has not been committed")?;
        let mut store = ProtectedMlsStore::open(
            BufferedBackend {
                snapshot: self.committed.clone(),
            },
            self.pins.clone(),
            *self.key,
            minimum,
        )?;
        let output = operation(&mut store)?;
        let checkpoint = store.checkpoint();
        let snapshot = store
            .into_backend()
            .snapshot
            .context("Missing prepared MLS state")?;
        self.pending = Some(PendingTransition {
            prepared: MlsPreparation {
                previous_checkpoint: self.checkpoint.clone(),
                checkpoint,
                snapshot,
            },
            output,
        });
        Ok(())
    }

    pub fn prepare_key_package(&mut self) -> Result<()> {
        self.prepare(|store| {
            encode_output(&serde_json::json!({
                "kind": "key_package", "wire": URL_SAFE_NO_PAD.encode(store.key_package()?),
            }))
        })
    }

    pub fn prepare_join(&mut self, delivery: &MlsDelivery, now: i64) -> Result<()> {
        self.prepare(|store| {
            store.join(delivery, now)?;
            encode_output(&serde_json::json!({"kind":"joined"}))
        })
    }

    pub fn prepare_receive(&mut self, delivery: &MlsDelivery, now: i64) -> Result<()> {
        self.prepare(|store| match store.receive(delivery, now)? {
            MlsReceipt::Application(plaintext) => {
                let encoded = Zeroizing::new(URL_SAFE_NO_PAD.encode(&plaintext));
                #[derive(Serialize)]
                struct Application<'a> {
                    kind: &'static str,
                    plaintext: &'a str,
                }
                encode_output(&Application {
                    kind: "application",
                    plaintext: &encoded,
                })
            }
            MlsReceipt::EpochChanged(epoch) => {
                encode_output(&serde_json::json!({"kind":"epoch_changed","epoch":epoch}))
            }
            MlsReceipt::Removed => encode_output(&serde_json::json!({"kind":"removed"})),
            MlsReceipt::Duplicate => encode_output(&serde_json::json!({"kind":"duplicate"})),
        })
    }

    pub fn local_identity(&self) -> &MemberIdentity {
        &self.pins.local
    }
}

fn encode_output(value: &impl Serialize) -> Result<Zeroizing<Vec<u8>>> {
    Ok(Zeroizing::new(serde_json::to_vec(value)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{TelemetryMember, TelemetryRoster, sign_telemetry_roster};

    fn member(id: &str, key: &SigningKey) -> MemberIdentity {
        MemberIdentity::new(id.as_bytes().to_vec(), key.public_key().to_bytes().unwrap()).unwrap()
    }

    #[test]
    fn initial_state_and_key_package_are_released_only_after_matching_commit() {
        let owner = SigningKey::generate();
        let publisher = SigningKey::generate();
        let reader = SigningKey::generate();
        let pins = MlsPins {
            device_id: "device".into(),
            scope: "device".into(),
            owner_invitation_key: owner.public_key(),
            publisher: member("publisher", &publisher),
            local: member("browser", &reader),
        };
        let mut endpoint = PreparedMlsEndpoint::create(pins.clone(), [41; 32], &reader).unwrap();
        assert!(endpoint.prepare_key_package().is_err());
        let initial = endpoint.prepared().unwrap();
        assert!(initial.previous_checkpoint.is_none());
        let mut wrong = initial.checkpoint.clone();
        wrong.revision += 1;
        assert!(endpoint.confirm_commit(&wrong).is_err());
        assert!(endpoint.checkpoint().is_none());
        endpoint.confirm_commit(&initial.checkpoint).unwrap();
        endpoint.prepare_key_package().unwrap();
        let prepared = endpoint.prepared().unwrap();
        assert_eq!(prepared.previous_checkpoint, Some(initial.checkpoint));
        assert!(endpoint.prepare_key_package().is_err());
        let output = endpoint.confirm_commit(&prepared.checkpoint).unwrap();
        let output: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(output["kind"], "key_package");
        let package = output["wire"].as_str().unwrap().to_owned();
        let mut endpoint =
            PreparedMlsEndpoint::open(pins, [41; 32], prepared.snapshot, prepared.checkpoint)
                .unwrap();
        endpoint.prepare_key_package().unwrap();
        let again = endpoint.prepared().unwrap();
        let output = endpoint.confirm_commit(&again.checkpoint).unwrap();
        let output: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(output["wire"], package);
    }

    #[test]
    fn discarded_decryption_exposes_no_output_and_can_retry_from_committed_snapshot() {
        let owner = SigningKey::generate();
        let publisher = SigningKey::generate();
        let reader = SigningKey::generate();
        let reader_pins = MlsPins {
            device_id: "device".into(),
            scope: "device".into(),
            owner_invitation_key: owner.public_key(),
            publisher: member("publisher", &publisher),
            local: member("browser", &reader),
        };
        let mut endpoint =
            PreparedMlsEndpoint::create(reader_pins.clone(), [41; 32], &reader).unwrap();
        endpoint
            .confirm_commit(&endpoint.prepared().unwrap().checkpoint)
            .unwrap();
        endpoint.prepare_key_package().unwrap();
        let output = endpoint
            .confirm_commit(&endpoint.prepared().unwrap().checkpoint)
            .unwrap();
        let output: serde_json::Value = serde_json::from_slice(&output).unwrap();
        let package = URL_SAFE_NO_PAD
            .decode(output["wire"].as_str().unwrap())
            .unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let members = vec![
            TelemetryMember {
                endpoint_id: "publisher".into(),
                signing_key: publisher.public_key(),
            },
            TelemetryMember {
                endpoint_id: "browser".into(),
                signing_key: reader.public_key(),
            },
        ];
        let policy = sign_telemetry_roster(
            &TelemetryRoster {
                version: 1,
                device_id: "device".into(),
                scope: "device".into(),
                policy_version: 1,
                previous_policy_digest: None,
                management_policy_digest: None,
                publisher: members[0].clone(),
                members,
                issued_at: now - 1,
                expires_at: now + 300,
            },
            &owner,
        )
        .unwrap();
        let mut publisher_pins = reader_pins;
        publisher_pins.local = publisher_pins.publisher.clone();
        let mut publisher_store = ProtectedMlsStore::create(
            BufferedBackend::default(),
            publisher_pins,
            [42; 32],
            &publisher,
        )
        .unwrap();
        let admission = publisher_store
            .apply_policy(
                "admit",
                1,
                &policy,
                &[(member("browser", &reader), package)],
                now,
            )
            .unwrap();
        endpoint
            .prepare_join(admission.welcome.as_ref().unwrap(), now)
            .unwrap();
        assert!(endpoint.delivery_receipts().is_err());
        endpoint
            .confirm_commit(&endpoint.prepared().unwrap().checkpoint)
            .unwrap();
        assert_eq!(endpoint.delivery_receipts().unwrap().len(), 1);
        let sent = publisher_store
            .publish("sample", 2, b"private decrypted sample", now)
            .unwrap();
        let before = endpoint.checkpoint();
        endpoint.prepare_receive(&sent.message, now).unwrap();
        assert!(endpoint.delivery_receipts().is_err());
        let pending = endpoint.prepared().unwrap();
        let serialized = serde_json::to_string(&pending).unwrap();
        assert!(!serialized.contains("private decrypted sample"));
        assert!(!serialized.contains(&URL_SAFE_NO_PAD.encode(b"private decrypted sample")));
        endpoint.discard_prepared();
        assert_eq!(endpoint.delivery_receipts().unwrap().len(), 1);
        assert!(endpoint.confirm_commit(&pending.checkpoint).is_err());
        assert_eq!(endpoint.checkpoint(), before);
        endpoint.prepare_receive(&sent.message, now).unwrap();
        let output = endpoint
            .confirm_commit(&endpoint.prepared().unwrap().checkpoint)
            .unwrap();
        let output: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            output["plaintext"],
            URL_SAFE_NO_PAD.encode(b"private decrypted sample")
        );
        endpoint.prepare_receive(&sent.message, now).unwrap();
        let output = endpoint
            .confirm_commit(&endpoint.prepared().unwrap().checkpoint)
            .unwrap();
        let output: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(output["kind"], "duplicate");
    }
}
