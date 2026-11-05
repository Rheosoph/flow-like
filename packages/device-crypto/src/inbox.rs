//! End-to-end inbox keys and sealed envelopes (envelope v1, sealed form).
//!
//! A principal's inbox key is an X25519 key; the public half is published in the key directory,
//! the 32-byte seed stays on the host. An envelope is HPKE (DhKem25519, HKDF-SHA256,
//! ChaCha20-Poly1305) to the recipient's public key, bound to the conversation, message, sender,
//! recipient and key id, so it opens nowhere else.

use crate::{CryptoError, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::{
    OpenMlsProvider,
    crypto::OpenMlsCrypto,
    types::{HpkeAeadType, HpkeCiphertext, HpkeConfig, HpkeKdfType, HpkeKemType},
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, PoisonError},
};
use zeroize::Zeroizing;

/// The key directory's `algorithm` and the sealed envelope's `enc`.
pub const ALGORITHM: &str = "hpke-x25519-sha256-chacha20poly1305";
pub const ENVELOPE_VERSION: u64 = 1;
/// Keys one principal keeps on its host: the active one and the retired ones older mail needs.
pub const MAX_RETAINED_KEYS: usize = 8;
const CONTEXT_LABEL: &str = "flow-like-inbox-envelope-v1";
const MAX_STORED_BYTES: usize = 16 * 1024;

fn suite() -> HpkeConfig {
    HpkeConfig(
        HpkeKemType::DhKem25519,
        HpkeKdfType::HkdfSha256,
        HpkeAeadType::ChaCha20Poly1305,
    )
}

/// What binds an envelope to its place.
#[derive(Debug, Clone, Copy)]
pub struct Scope<'a> {
    pub conversation_id: &'a str,
    pub message_id: &'a str,
    pub sender_id: &'a str,
}

fn context(scope: &Scope, recipient_id: &str, key_id: &str) -> Vec<u8> {
    json!([
        CONTEXT_LABEL,
        scope.conversation_id,
        scope.message_id,
        scope.sender_id,
        recipient_id,
        key_id
    ])
    .to_string()
    .into_bytes()
}

/// A member's published key, to seal to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    pub principal_id: String,
    pub key_id: String,
    pub public_key: String,
}

fn decode_public(public_key: &str) -> Result<[u8; 32]> {
    URL_SAFE_NO_PAD
        .decode(public_key)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or(CryptoError::InvalidInput(
            "an inbox public key is 32 bytes, base64url",
        ))
}

/// One sealed envelope for `recipient`.
pub fn seal(scope: &Scope, recipient: &Recipient, plaintext: &[u8]) -> Result<Value> {
    let public = decode_public(&recipient.public_key)?;
    let info = context(scope, &recipient.principal_id, &recipient.key_id);
    let sealed = OpenMlsRustCrypto::default()
        .crypto()
        .hpke_seal(suite(), &public, &info, &info, plaintext)
        .map_err(|_| CryptoError::Operation("sealing an inbox envelope failed"))?;
    Ok(json!({
        "v": ENVELOPE_VERSION,
        "enc": ALGORITHM,
        "key_id": recipient.key_id,
        "enc_key": URL_SAFE_NO_PAD.encode(sealed.kem_output.as_slice()),
        "ct": URL_SAFE_NO_PAD.encode(sealed.ciphertext.as_slice()),
    }))
}

/// One envelope per recipient, keyed by principal id, as a send to an end-to-end encrypted
/// conversation wants them.
pub fn seal_all(
    scope: &Scope,
    recipients: &[Recipient],
    plaintext: &[u8],
) -> Result<BTreeMap<String, Value>> {
    if recipients.is_empty() {
        return Err(CryptoError::InvalidInput("there is nobody to seal to"));
    }
    let mut seen = BTreeSet::new();
    let mut envelopes = BTreeMap::new();
    for recipient in recipients {
        if !seen.insert(recipient.principal_id.as_str()) {
            return Err(CryptoError::InvalidInput(
                "a principal appears twice among the recipients",
            ));
        }
        envelopes.insert(
            recipient.principal_id.clone(),
            seal(scope, recipient, plaintext)?,
        );
    }
    Ok(envelopes)
}

/// The key id a sealed envelope names, `None` when it isn't a sealed v1 envelope.
pub fn envelope_key_id(envelope: &Value) -> Option<&str> {
    (envelope.get("v").and_then(Value::as_u64) == Some(ENVELOPE_VERSION)
        && envelope.get("enc").and_then(Value::as_str) == Some(ALGORITHM))
    .then(|| envelope.get("key_id").and_then(Value::as_str))
    .flatten()
}

fn field(envelope: &Value, name: &str) -> Result<Vec<u8>> {
    envelope
        .get(name)
        .and_then(Value::as_str)
        .and_then(|value| URL_SAFE_NO_PAD.decode(value).ok())
        .ok_or(CryptoError::InvalidInput(
            "a sealed envelope needs base64url enc_key and ct",
        ))
}

/// Opens `envelope` sent to `recipient_id` with the key `key_id`'s `seed`.
pub fn open(
    seed: &[u8; 32],
    key_id: &str,
    scope: &Scope,
    recipient_id: &str,
    envelope: &Value,
) -> Result<Zeroizing<Vec<u8>>> {
    if envelope_key_id(envelope) != Some(key_id) {
        return Err(CryptoError::InvalidInput(
            "the envelope is not sealed to this key",
        ));
    }
    let sealed = HpkeCiphertext {
        kem_output: field(envelope, "enc_key")?.into(),
        ciphertext: field(envelope, "ct")?.into(),
    };
    let info = context(scope, recipient_id, key_id);
    OpenMlsRustCrypto::default()
        .crypto()
        .hpke_open(suite(), &sealed, seed, &info, &info)
        .map(Zeroizing::new)
        .map_err(|_| CryptoError::Operation("the envelope does not open with this key"))
}

fn public_of(seed: &[u8; 32]) -> [u8; 32] {
    x25519_dalek::x25519(*seed, x25519_dalek::X25519_BASEPOINT_BYTES)
}

/// Eight groups of four hex digits: what two people compare to confirm a key.
pub fn fingerprint(public_key: &[u8]) -> String {
    let digest = Sha256::digest(public_key);
    digest[..16]
        .chunks(2)
        .map(|pair| format!("{:02x}{:02x}", pair[0], pair[1]))
        .collect::<Vec<_>>()
        .join("-")
}

pub fn key_id_of(public_key: &[u8]) -> String {
    let digest = Sha256::digest(public_key);
    let hex: String = digest[..8].iter().map(|byte| format!("{byte:02x}")).collect();
    format!("ik_{hex}")
}

/// What the key directory and the UI know of a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicInboxKey {
    pub key_id: String,
    pub algorithm: String,
    pub public_key: String,
    pub fingerprint: String,
}

fn public_key_of(seed: &[u8; 32]) -> PublicInboxKey {
    let public = public_of(seed);
    PublicInboxKey {
        key_id: key_id_of(&public),
        algorithm: ALGORITHM.to_owned(),
        public_key: URL_SAFE_NO_PAD.encode(public),
        fingerprint: fingerprint(&public),
    }
}

/// Where a host keeps private seeds: the OS keychain on a desktop, the device vault on a
/// device. One opaque secret per owner (bot id).
pub trait InboxKeyVault: Send + Sync {
    fn load(&self, owner: &str) -> std::result::Result<Option<Zeroizing<Vec<u8>>>, String>;
    fn store(&self, owner: &str, secret: &[u8]) -> std::result::Result<(), String>;
    fn remove(&self, owner: &str) -> std::result::Result<(), String>;
}

#[derive(Serialize, Deserialize)]
struct StoredKey {
    seed: String,
}

#[derive(Serialize, Deserialize, Default)]
struct StoredKeys {
    keys: Vec<StoredKey>,
}

struct HeldKey {
    seed: Zeroizing<[u8; 32]>,
    public: PublicInboxKey,
}

/// A key that was replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rotation {
    pub current: PublicInboxKey,
    /// The key id that was active before and is retired now: revoke it in the directory.
    pub retired: Option<String>,
}

/// The inbox keys of the principals one host runs.
pub struct InboxKeyring {
    vault: Arc<dyn InboxKeyVault>,
    lock: Mutex<()>,
}

impl InboxKeyring {
    pub fn new(vault: Arc<dyn InboxKeyVault>) -> Self {
        Self {
            vault,
            lock: Mutex::new(()),
        }
    }

    fn held(&self, owner: &str) -> Result<Vec<HeldKey>> {
        let Some(bytes) = self
            .vault
            .load(owner)
            .map_err(|_| CryptoError::Operation("the key store could not be read"))?
        else {
            return Ok(Vec::new());
        };
        if bytes.len() > MAX_STORED_BYTES {
            return Err(CryptoError::InvalidInput("the stored inbox keys are damaged"));
        }
        let stored: StoredKeys = serde_json::from_slice(&bytes)
            .map_err(|_| CryptoError::InvalidInput("the stored inbox keys are damaged"))?;
        stored
            .keys
            .iter()
            .map(|key| {
                let seed = URL_SAFE_NO_PAD
                    .decode(&key.seed)
                    .ok()
                    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                    .ok_or(CryptoError::InvalidInput("the stored inbox keys are damaged"))?;
                let seed = Zeroizing::new(seed);
                Ok(HeldKey {
                    public: public_key_of(&seed),
                    seed,
                })
            })
            .collect()
    }

    fn save(&self, owner: &str, keys: &[HeldKey]) -> Result<()> {
        let stored = StoredKeys {
            keys: keys
                .iter()
                .map(|key| StoredKey {
                    seed: URL_SAFE_NO_PAD.encode(*key.seed),
                })
                .collect(),
        };
        let bytes = Zeroizing::new(
            serde_json::to_vec(&stored)
                .map_err(|_| CryptoError::Operation("the inbox keys could not be encoded"))?,
        );
        self.vault
            .store(owner, &bytes)
            .map_err(|_| CryptoError::Operation("the key store could not be written"))
    }

    fn fresh() -> HeldKey {
        let mut seed = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(seed.as_mut());
        HeldKey {
            public: public_key_of(&seed),
            seed,
        }
    }

    /// The active key of `owner`, if it has one.
    pub fn active(&self, owner: &str) -> Result<Option<PublicInboxKey>> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(self.held(owner)?.pop().map(|key| key.public))
    }

    /// The active key of `owner`, made on first use. Second value: it was made now.
    pub fn ensure(&self, owner: &str) -> Result<(PublicInboxKey, bool)> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut keys = self.held(owner)?;
        if let Some(key) = keys.last() {
            return Ok((key.public.clone(), false));
        }
        let key = Self::fresh();
        let public = key.public.clone();
        keys.push(key);
        self.save(owner, &keys)?;
        Ok((public, true))
    }

    /// A new active key; the old seed stays so earlier envelopes still open.
    pub fn rotate(&self, owner: &str) -> Result<Rotation> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut keys = self.held(owner)?;
        let retired = keys.last().map(|key| key.public.key_id.clone());
        let key = Self::fresh();
        let current = key.public.clone();
        keys.push(key);
        if keys.len() > MAX_RETAINED_KEYS {
            keys.remove(0);
        }
        self.save(owner, &keys)?;
        Ok(Rotation { current, retired })
    }

    /// Opens an envelope sent to `owner` with whichever retained key it names.
    pub fn open(
        &self,
        owner: &str,
        scope: &Scope,
        envelope: &Value,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let key_id = envelope_key_id(envelope)
            .ok_or(CryptoError::InvalidInput("not a sealed inbox envelope"))?;
        let keys = {
            let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
            self.held(owner)?
        };
        let key = keys
            .iter()
            .find(|key| key.public.key_id == key_id)
            .ok_or(CryptoError::InvalidInput(
                "this host holds no key for the envelope",
            ))?;
        open(&key.seed, key_id, scope, owner, envelope)
    }

    /// Deletes every key of `owner`; nothing sealed to them opens afterwards.
    pub fn forget(&self, owner: &str) -> Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        self.vault
            .remove(owner)
            .map_err(|_| CryptoError::Operation("the key store could not be written"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    pub struct FakeVault {
        pub secrets: Mutex<HashMap<String, Vec<u8>>>,
        pub locked: Mutex<bool>,
    }

    impl InboxKeyVault for FakeVault {
        fn load(&self, owner: &str) -> std::result::Result<Option<Zeroizing<Vec<u8>>>, String> {
            if *self.locked.lock().unwrap() {
                return Err("locked".into());
            }
            Ok(self
                .secrets
                .lock()
                .unwrap()
                .get(owner)
                .cloned()
                .map(Zeroizing::new))
        }

        fn store(&self, owner: &str, secret: &[u8]) -> std::result::Result<(), String> {
            if *self.locked.lock().unwrap() {
                return Err("locked".into());
            }
            self.secrets
                .lock()
                .unwrap()
                .insert(owner.to_owned(), secret.to_vec());
            Ok(())
        }

        fn remove(&self, owner: &str) -> std::result::Result<(), String> {
            self.secrets.lock().unwrap().remove(owner);
            Ok(())
        }
    }

    fn keyring() -> (Arc<FakeVault>, InboxKeyring) {
        let vault = Arc::new(FakeVault::default());
        (vault.clone(), InboxKeyring::new(vault))
    }

    fn scope() -> Scope<'static> {
        Scope {
            conversation_id: "conv",
            message_id: "m1",
            sender_id: "bot",
        }
    }

    fn recipient(id: &str, key: &PublicInboxKey) -> Recipient {
        Recipient {
            principal_id: id.into(),
            key_id: key.key_id.clone(),
            public_key: key.public_key.clone(),
        }
    }

    #[test]
    fn a_key_is_made_once_and_kept() {
        let (vault, ring) = keyring();
        assert_eq!(ring.active("bot").unwrap(), None);
        let (first, made) = ring.ensure("bot").unwrap();
        assert!(made);
        assert_eq!(first.algorithm, ALGORITHM);
        assert!(first.key_id.starts_with("ik_"));
        assert_eq!(first.fingerprint.split('-').count(), 8);
        let (again, made) = ring.ensure("bot").unwrap();
        assert!(!made);
        assert_eq!(again, first);
        assert_eq!(InboxKeyring::new(vault).active("bot").unwrap(), Some(first));
    }

    #[test]
    fn the_stored_secret_holds_seeds_and_no_public_material() {
        let (vault, ring) = keyring();
        let (key, _) = ring.ensure("bot").unwrap();
        let stored = String::from_utf8(vault.secrets.lock().unwrap()["bot"].clone()).unwrap();
        assert!(!stored.contains(&key.public_key));
    }

    #[test]
    fn two_members_each_open_only_their_own_envelope() {
        let (_, bot_ring) = keyring();
        let (_, person_ring) = keyring();
        let (bot_key, _) = bot_ring.ensure("bot").unwrap();
        let (person_key, _) = person_ring.ensure("person").unwrap();
        let envelopes = seal_all(
            &scope(),
            &[recipient("bot", &bot_key), recipient("person", &person_key)],
            b"hello",
        )
        .unwrap();
        assert_eq!(envelopes.len(), 2);
        let for_bot = &envelopes["bot"];
        let for_person = &envelopes["person"];
        assert_eq!(&**bot_ring.open("bot", &scope(), for_bot).unwrap(), b"hello");
        assert_eq!(
            &**person_ring.open("person", &scope(), for_person).unwrap(),
            b"hello"
        );
        assert!(person_ring.open("person", &scope(), for_bot).is_err());
        assert!(!for_bot.to_string().contains("hello"));
    }

    #[test]
    fn an_envelope_opens_only_where_it_was_sealed_for() {
        let (_, ring) = keyring();
        let (key, _) = ring.ensure("bot").unwrap();
        let envelope = seal(&scope(), &recipient("bot", &key), b"secret").unwrap();
        for other in [
            Scope { conversation_id: "other", ..scope() },
            Scope { message_id: "m2", ..scope() },
            Scope { sender_id: "mallory", ..scope() },
        ] {
            assert!(ring.open("bot", &other, &envelope).is_err());
        }
        assert!(ring.open("someone-else", &scope(), &envelope).is_err());
        let mut tampered = envelope.clone();
        tampered["ct"] = json!(URL_SAFE_NO_PAD.encode(b"not the ciphertext at all"));
        assert!(ring.open("bot", &scope(), &tampered).is_err());
    }

    #[test]
    fn rotating_keeps_older_mail_readable_and_names_the_retired_key() {
        let (_, ring) = keyring();
        let (old, _) = ring.ensure("bot").unwrap();
        let before = seal(&scope(), &recipient("bot", &old), b"old mail").unwrap();
        let rotation = ring.rotate("bot").unwrap();
        assert_eq!(rotation.retired.as_deref(), Some(old.key_id.as_str()));
        assert_ne!(rotation.current.key_id, old.key_id);
        assert_eq!(ring.active("bot").unwrap(), Some(rotation.current.clone()));
        assert_eq!(&**ring.open("bot", &scope(), &before).unwrap(), b"old mail");
        let after = seal(&scope(), &recipient("bot", &rotation.current), b"new").unwrap();
        assert_eq!(&**ring.open("bot", &scope(), &after).unwrap(), b"new");
    }

    #[test]
    fn only_the_newest_keys_are_retained() {
        let (_, ring) = keyring();
        let (first, _) = ring.ensure("bot").unwrap();
        let sealed = seal(&scope(), &recipient("bot", &first), b"x").unwrap();
        for _ in 0..MAX_RETAINED_KEYS {
            ring.rotate("bot").unwrap();
        }
        assert!(ring.open("bot", &scope(), &sealed).is_err());
    }

    #[test]
    fn a_locked_vault_neither_creates_nor_overwrites() {
        let (vault, ring) = keyring();
        let (key, _) = ring.ensure("bot").unwrap();
        *vault.locked.lock().unwrap() = true;
        assert!(ring.ensure("bot").is_err());
        assert!(ring.rotate("bot").is_err());
        *vault.locked.lock().unwrap() = false;
        assert_eq!(ring.active("bot").unwrap(), Some(key));
    }

    #[test]
    fn forgetting_ends_every_key() {
        let (_, ring) = keyring();
        let (key, _) = ring.ensure("bot").unwrap();
        let sealed = seal(&scope(), &recipient("bot", &key), b"x").unwrap();
        ring.forget("bot").unwrap();
        assert_eq!(ring.active("bot").unwrap(), None);
        assert!(ring.open("bot", &scope(), &sealed).is_err());
    }

    #[test]
    fn sealing_refuses_nobody_duplicates_and_bad_keys() {
        let (_, ring) = keyring();
        let (key, _) = ring.ensure("bot").unwrap();
        assert!(seal_all(&scope(), &[], b"x").is_err());
        let twice = [recipient("bot", &key), recipient("bot", &key)];
        assert!(seal_all(&scope(), &twice, b"x").is_err());
        let bad = Recipient {
            principal_id: "p".into(),
            key_id: "k".into(),
            public_key: "short".into(),
        };
        assert!(seal(&scope(), &bad, b"x").is_err());
    }

    #[test]
    fn the_envelope_has_exactly_the_sealed_v1_fields() {
        let (_, ring) = keyring();
        let (key, _) = ring.ensure("bot").unwrap();
        let envelope = seal(&scope(), &recipient("bot", &key), b"x").unwrap();
        let mut names: Vec<_> = envelope.as_object().unwrap().keys().cloned().collect();
        names.sort();
        assert_eq!(names, ["ct", "enc", "enc_key", "key_id", "v"]);
        assert_eq!(envelope_key_id(&envelope), Some(key.key_id.as_str()));
    }
}
