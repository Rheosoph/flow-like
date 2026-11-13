//! End-to-end inbox keys and sealed envelopes (envelope v1, sealed form).
//!
//! A principal's inbox key is an X25519 key; the public half is published in the key directory,
//! the 32-byte seed stays on the host. A person holds one key per device.
//!
//! Two sealed shapes exist, both bound to the conversation, message, sender and recipient so an
//! envelope opens nowhere else:
//! - single key `{v, enc, key_id, enc_key, ct}`: HPKE (DhKem25519, HKDF-SHA256,
//!   ChaCha20-Poly1305) of the plaintext to one key;
//! - keys `{v, enc, ct, keys: [{key_id, enc_key, ck}]}`: the plaintext under a one-time content
//!   key (ChaCha20-Poly1305, zero nonce), and that key HPKE-sealed to each of the recipient's keys.
//!
//! A bot secret the owner seals to the bot's keys uses the keys shape under its own labels, bound
//! to the bot and the secret's name instead of a message.

use crate::{CryptoError, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, KeyInit, Payload},
};
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
/// Keys one member's envelope is sealed to: the server's active key limit per principal.
pub const MAX_ENVELOPE_KEYS: usize = 20;
const CONTEXT_LABEL: &str = "flow-like-inbox-envelope-v1";
const BODY_LABEL: &str = "flow-like-inbox-body-v1";
const SECRET_KEY_LABEL: &str = "flow-like-bot-secret-key-v1";
const SECRET_BODY_LABEL: &str = "flow-like-bot-secret-v1";
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

fn body_context(scope: &Scope, recipient_id: &str) -> Vec<u8> {
    json!([
        BODY_LABEL,
        scope.conversation_id,
        scope.message_id,
        scope.sender_id,
        recipient_id
    ])
    .to_string()
    .into_bytes()
}

/// What binds a sealed bot secret: the bot that may open it and the name it is stored under.
#[derive(Debug, Clone, Copy)]
pub struct SecretScope<'a> {
    pub bot_id: &'a str,
    pub name: &'a str,
}

/// What the wrapped keys and the body of a keys-form envelope are bound to.
#[derive(Clone, Copy)]
enum Binding<'a> {
    Message {
        scope: &'a Scope<'a>,
        recipient_id: &'a str,
    },
    Secret(&'a SecretScope<'a>),
}

impl Binding<'_> {
    fn key_info(&self, key_id: &str) -> Vec<u8> {
        match self {
            Self::Message {
                scope,
                recipient_id,
            } => context(scope, recipient_id, key_id),
            Self::Secret(secret) => json!([SECRET_KEY_LABEL, secret.bot_id, secret.name, key_id])
                .to_string()
                .into_bytes(),
        }
    }

    fn body_aad(&self) -> Vec<u8> {
        match self {
            Self::Message {
                scope,
                recipient_id,
            } => body_context(scope, recipient_id),
            Self::Secret(secret) => json!([SECRET_BODY_LABEL, secret.bot_id, secret.name])
                .to_string()
                .into_bytes(),
        }
    }
}

/// A member's published key, to seal to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    pub principal_id: String,
    pub key_id: String,
    pub public_key: String,
}

/// One published key of a member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipientKey {
    pub key_id: String,
    pub public_key: String,
}

/// A member and the active keys of their devices, to seal one envelope to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub principal_id: String,
    pub keys: Vec<RecipientKey>,
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

fn hpke_seal(public_key: &str, info: &[u8], plaintext: &[u8]) -> Result<HpkeCiphertext> {
    let public = decode_public(public_key)?;
    OpenMlsRustCrypto::default()
        .crypto()
        .hpke_seal(suite(), &public, info, info, plaintext)
        .map_err(|_| CryptoError::Operation("sealing an inbox envelope failed"))
}

fn hpke_open(seed: &[u8; 32], sealed: &HpkeCiphertext, info: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    OpenMlsRustCrypto::default()
        .crypto()
        .hpke_open(suite(), sealed, seed, info, info)
        .map(Zeroizing::new)
        .map_err(|_| CryptoError::Operation("the envelope does not open with this key"))
}

/// One single-key envelope for `recipient`.
pub fn seal(scope: &Scope, recipient: &Recipient, plaintext: &[u8]) -> Result<Value> {
    let info = context(scope, &recipient.principal_id, &recipient.key_id);
    let sealed = hpke_seal(&recipient.public_key, &info, plaintext)?;
    Ok(json!({
        "v": ENVELOPE_VERSION,
        "enc": ALGORITHM,
        "key_id": recipient.key_id,
        "enc_key": URL_SAFE_NO_PAD.encode(sealed.kem_output.as_slice()),
        "ct": URL_SAFE_NO_PAD.encode(sealed.ciphertext.as_slice()),
    }))
}

/// One envelope for `member` that each of their keys opens.
pub fn seal_member(scope: &Scope, member: &Member, plaintext: &[u8]) -> Result<Value> {
    let binding = Binding::Message {
        scope,
        recipient_id: &member.principal_id,
    };
    seal_keys(binding, &member.keys, plaintext)
}

/// A bot secret sealed to the bot's published `keys`; it opens only for `scope`'s bot and name.
pub fn seal_secret(scope: &SecretScope, keys: &[RecipientKey], value: &[u8]) -> Result<Value> {
    seal_keys(Binding::Secret(scope), keys, value)
}

fn seal_keys(binding: Binding, keys: &[RecipientKey], plaintext: &[u8]) -> Result<Value> {
    if keys.is_empty() {
        return Err(CryptoError::InvalidInput(
            "a member needs a published key to seal to",
        ));
    }
    if keys.len() > MAX_ENVELOPE_KEYS {
        return Err(CryptoError::InvalidInput(
            "a member has more keys than one envelope holds",
        ));
    }
    let mut content_key = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(content_key.as_mut());
    let ct = ChaCha20Poly1305::new((&*content_key).into())
        .encrypt(
            &Nonce::default(),
            Payload {
                msg: plaintext,
                aad: &binding.body_aad(),
            },
        )
        .map_err(|_| CryptoError::Operation("sealing an inbox envelope failed"))?;
    let mut seen = BTreeSet::new();
    let keys = keys
        .iter()
        .map(|key| {
            if !seen.insert(key.key_id.as_str()) {
                return Err(CryptoError::InvalidInput(
                    "a key appears twice among a member's keys",
                ));
            }
            let info = binding.key_info(&key.key_id);
            let wrapped = hpke_seal(&key.public_key, &info, content_key.as_slice())?;
            Ok(json!({
                "key_id": key.key_id,
                "enc_key": URL_SAFE_NO_PAD.encode(wrapped.kem_output.as_slice()),
                "ck": URL_SAFE_NO_PAD.encode(wrapped.ciphertext.as_slice()),
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "v": ENVELOPE_VERSION,
        "enc": ALGORITHM,
        "ct": URL_SAFE_NO_PAD.encode(ct),
        "keys": keys,
    }))
}

/// One envelope per member, keyed by principal id, as a send to an end-to-end encrypted
/// conversation wants them.
pub fn seal_all(
    scope: &Scope,
    members: &[Member],
    plaintext: &[u8],
) -> Result<BTreeMap<String, Value>> {
    if members.is_empty() {
        return Err(CryptoError::InvalidInput("there is nobody to seal to"));
    }
    let mut envelopes = BTreeMap::new();
    for member in members {
        if envelopes.contains_key(&member.principal_id) {
            return Err(CryptoError::InvalidInput(
                "a principal appears twice among the recipients",
            ));
        }
        envelopes.insert(
            member.principal_id.clone(),
            seal_member(scope, member, plaintext)?,
        );
    }
    Ok(envelopes)
}

fn is_sealed_v1(envelope: &Value) -> bool {
    envelope.get("v").and_then(Value::as_u64) == Some(ENVELOPE_VERSION)
        && envelope.get("enc").and_then(Value::as_str) == Some(ALGORITHM)
}

/// The key id a single-key envelope names, `None` for any other envelope.
pub fn envelope_key_id(envelope: &Value) -> Option<&str> {
    is_sealed_v1(envelope)
        .then(|| envelope.get("key_id").and_then(Value::as_str))
        .flatten()
}

/// The keys a keys-form envelope is sealed to, by key id.
fn wrapped_keys(envelope: &Value) -> Vec<(&str, &Value)> {
    if !is_sealed_v1(envelope) {
        return Vec::new();
    }
    envelope
        .get("keys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|key| Some((key.get("key_id")?.as_str()?, key)))
        .collect()
}

/// Every key id that may open `envelope`, in either sealed shape.
pub fn envelope_key_ids(envelope: &Value) -> Vec<&str> {
    match envelope_key_id(envelope) {
        Some(key_id) => vec![key_id],
        None => wrapped_keys(envelope)
            .into_iter()
            .map(|(key_id, _)| key_id)
            .collect(),
    }
}

fn field(envelope: &Value, name: &str) -> Result<Vec<u8>> {
    envelope
        .get(name)
        .and_then(Value::as_str)
        .and_then(|value| URL_SAFE_NO_PAD.decode(value).ok())
        .ok_or(CryptoError::InvalidInput(
            "a sealed envelope needs base64url enc_key, ck and ct",
        ))
}

/// Opens a single-key `envelope` sent to `recipient_id` with the key `key_id`'s `seed`.
pub fn open(
    seed: &[u8; 32],
    key_id: &str,
    scope: &Scope,
    recipient_id: &str,
    envelope: &Value,
) -> Result<Zeroizing<Vec<u8>>> {
    let binding = Binding::Message {
        scope,
        recipient_id,
    };
    open_single(seed, key_id, binding, envelope)
}

fn open_single(
    seed: &[u8; 32],
    key_id: &str,
    binding: Binding,
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
    hpke_open(seed, &sealed, &binding.key_info(key_id))
}

fn open_wrapped(
    seed: &[u8; 32],
    key_id: &str,
    wrapped: &Value,
    binding: Binding,
    envelope: &Value,
) -> Result<Zeroizing<Vec<u8>>> {
    let sealed = HpkeCiphertext {
        kem_output: field(wrapped, "enc_key")?.into(),
        ciphertext: field(wrapped, "ck")?.into(),
    };
    let content_key = hpke_open(seed, &sealed, &binding.key_info(key_id))?;
    let content_key: &[u8; 32] = content_key
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::InvalidInput("a wrapped content key is 32 bytes"))?;
    ChaCha20Poly1305::new(content_key.into())
        .decrypt(
            &Nonce::default(),
            Payload {
                msg: &field(envelope, "ct")?,
                aad: &binding.body_aad(),
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| CryptoError::Operation("the envelope does not open with this key"))
}

/// A private inbox key: the seed and what others see of it.
pub struct InboxSecret {
    seed: Zeroizing<[u8; 32]>,
    public: PublicInboxKey,
}

impl InboxSecret {
    pub fn generate() -> Self {
        let mut seed = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(seed.as_mut());
        Self::from_seed(seed)
    }

    fn from_seed(seed: Zeroizing<[u8; 32]>) -> Self {
        Self {
            public: public_key_of(&seed),
            seed,
        }
    }

    /// A seed as [`Self::encoded_seed`] wrote it.
    pub fn from_encoded(seed: &str) -> Result<Self> {
        let bytes = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(seed)
                .map_err(|_| CryptoError::InvalidInput("an inbox seed is 32 bytes, base64url"))?,
        );
        let seed: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::InvalidInput("an inbox seed is 32 bytes, base64url"))?;
        Ok(Self::from_seed(Zeroizing::new(seed)))
    }

    pub fn encoded_seed(&self) -> Zeroizing<String> {
        Zeroizing::new(URL_SAFE_NO_PAD.encode(*self.seed))
    }

    pub fn public(&self) -> &PublicInboxKey {
        &self.public
    }

    fn open(&self, binding: Binding, envelope: &Value) -> Option<Result<Zeroizing<Vec<u8>>>> {
        let key_id = self.public.key_id.as_str();
        if envelope_key_id(envelope) == Some(key_id) {
            return Some(open_single(&self.seed, key_id, binding, envelope));
        }
        wrapped_keys(envelope)
            .into_iter()
            .find(|(id, _)| *id == key_id)
            .map(|(_, wrapped)| open_wrapped(&self.seed, key_id, wrapped, binding, envelope))
    }
}

fn open_bound(
    secrets: &[InboxSecret],
    binding: Binding,
    envelope: &Value,
) -> Result<Zeroizing<Vec<u8>>> {
    if envelope_key_ids(envelope).is_empty() {
        return Err(CryptoError::InvalidInput("not a sealed inbox envelope"));
    }
    secrets
        .iter()
        .find_map(|secret| secret.open(binding, envelope))
        .unwrap_or(Err(CryptoError::InvalidInput(
            "this host holds no key for the envelope",
        )))
}

/// Opens `envelope` sent to `recipient_id` with whichever of `secrets` it names.
pub fn open_with(
    secrets: &[InboxSecret],
    scope: &Scope,
    recipient_id: &str,
    envelope: &Value,
) -> Result<Zeroizing<Vec<u8>>> {
    let binding = Binding::Message {
        scope,
        recipient_id,
    };
    open_bound(secrets, binding, envelope)
}

/// Opens a bot secret sealed for `scope` with whichever of `secrets` it names.
pub fn open_secret_with(
    secrets: &[InboxSecret],
    scope: &SecretScope,
    envelope: &Value,
) -> Result<Zeroizing<Vec<u8>>> {
    open_bound(secrets, Binding::Secret(scope), envelope)
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

    fn held(&self, owner: &str) -> Result<Vec<InboxSecret>> {
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
                InboxSecret::from_encoded(&key.seed)
                    .map_err(|_| CryptoError::InvalidInput("the stored inbox keys are damaged"))
            })
            .collect()
    }

    fn save(&self, owner: &str, keys: &[InboxSecret]) -> Result<()> {
        let stored = StoredKeys {
            keys: keys
                .iter()
                .map(|key| StoredKey {
                    seed: key.encoded_seed().to_string(),
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

    /// Every key `owner` keeps, oldest first: to open a batch with one vault read.
    pub fn secrets(&self, owner: &str) -> Result<Vec<InboxSecret>> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        self.held(owner)
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
        let key = InboxSecret::generate();
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
        let key = InboxSecret::generate();
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
        open_with(&self.secrets(owner)?, scope, owner, envelope)
    }

    /// Opens a secret the owner sealed to `scope.bot_id`'s keys.
    pub fn open_secret(&self, scope: &SecretScope, envelope: &Value) -> Result<Zeroizing<Vec<u8>>> {
        open_secret_with(&self.secrets(scope.bot_id)?, scope, envelope)
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

    fn member(id: &str, keys: &[&PublicInboxKey]) -> Member {
        Member {
            principal_id: id.into(),
            keys: keys
                .iter()
                .map(|key| RecipientKey {
                    key_id: key.key_id.clone(),
                    public_key: key.public_key.clone(),
                })
                .collect(),
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
            &[member("bot", &[&bot_key]), member("person", &[&person_key])],
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
        let twice = [member("bot", &[&key]), member("bot", &[&key])];
        assert!(seal_all(&scope(), &twice, b"x").is_err());
        assert!(seal_member(&scope(), &member("bot", &[&key, &key]), b"x").is_err());
        assert!(seal_member(&scope(), &member("bot", &[]), b"x").is_err());
        let bad = Recipient {
            principal_id: "p".into(),
            key_id: "k".into(),
            public_key: "short".into(),
        };
        assert!(seal(&scope(), &bad, b"x").is_err());
        let bad_member = Member {
            principal_id: "p".into(),
            keys: vec![RecipientKey {
                key_id: "k".into(),
                public_key: "short".into(),
            }],
        };
        assert!(seal_member(&scope(), &bad_member, b"x").is_err());
    }

    #[test]
    fn every_device_of_a_member_opens_the_one_envelope() {
        let (_, phone) = keyring();
        let (_, laptop) = keyring();
        let (_, stranger) = keyring();
        let (phone_key, _) = phone.ensure("person").unwrap();
        let (laptop_key, _) = laptop.ensure("person").unwrap();
        stranger.ensure("person").unwrap();
        let envelope =
            seal_member(&scope(), &member("person", &[&phone_key, &laptop_key]), b"hi").unwrap();
        assert_eq!(&**phone.open("person", &scope(), &envelope).unwrap(), b"hi");
        assert_eq!(&**laptop.open("person", &scope(), &envelope).unwrap(), b"hi");
        assert!(stranger.open("person", &scope(), &envelope).is_err());
        assert_eq!(
            envelope_key_ids(&envelope),
            [phone_key.key_id.as_str(), laptop_key.key_id.as_str()]
        );
        assert_eq!(envelope_key_id(&envelope), None);
    }

    #[test]
    fn a_keys_envelope_opens_only_where_it_was_sealed_for() {
        let (_, ring) = keyring();
        let (key, _) = ring.ensure("bot").unwrap();
        let envelope = seal_member(&scope(), &member("bot", &[&key]), b"secret").unwrap();
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
        let mut swapped = envelope.clone();
        swapped["keys"][0]["ck"] = envelope["keys"][0]["enc_key"].clone();
        assert!(ring.open("bot", &scope(), &swapped).is_err());
    }

    #[test]
    fn a_keys_envelope_has_exactly_its_fields() {
        let (_, ring) = keyring();
        let (key, _) = ring.ensure("bot").unwrap();
        let envelope = seal_member(&scope(), &member("bot", &[&key]), b"x").unwrap();
        let mut names: Vec<_> = envelope.as_object().unwrap().keys().cloned().collect();
        names.sort();
        assert_eq!(names, ["ct", "enc", "keys", "v"]);
        let mut wrapped: Vec<_> = envelope["keys"][0].as_object().unwrap().keys().cloned().collect();
        wrapped.sort();
        assert_eq!(wrapped, ["ck", "enc_key", "key_id"]);
    }

    fn recipient_keys(keys: &[&PublicInboxKey]) -> Vec<RecipientKey> {
        member("unused", keys).keys
    }

    const SECRET: SecretScope<'static> = SecretScope {
        bot_id: "bot_a",
        name: "API_KEY",
    };

    #[test]
    fn a_sealed_secret_opens_only_for_its_bot_and_name() {
        let (_, ring) = keyring();
        let (key, _) = ring.ensure("bot_a").unwrap();
        let envelope = seal_secret(&SECRET, &recipient_keys(&[&key]), b"sk-123").unwrap();
        assert!(!envelope.to_string().contains("sk-123"));
        assert_eq!(&**ring.open_secret(&SECRET, &envelope).unwrap(), b"sk-123");
        for other in [
            SecretScope {
                name: "OTHER_KEY",
                ..SECRET
            },
            SecretScope {
                bot_id: "bot_b",
                ..SECRET
            },
        ] {
            let secrets = ring.secrets("bot_a").unwrap();
            assert!(open_secret_with(&secrets, &other, &envelope).is_err());
        }
        let mut names: Vec<_> = envelope.as_object().unwrap().keys().cloned().collect();
        names.sort();
        assert_eq!(names, ["ct", "enc", "keys", "v"]);
    }

    #[test]
    fn a_secret_and_a_message_never_open_as_each_other() {
        let (_, ring) = keyring();
        let (key, _) = ring.ensure("bot_a").unwrap();
        let secret = seal_secret(&SECRET, &recipient_keys(&[&key]), b"x").unwrap();
        let message = seal_member(&scope(), &member("bot_a", &[&key]), b"x").unwrap();
        assert!(ring.open("bot_a", &scope(), &secret).is_err());
        assert!(ring.open_secret(&SECRET, &message).is_err());
    }

    #[test]
    fn a_secret_for_keys_this_host_lacks_does_not_open() {
        let (_, here) = keyring();
        let (_, elsewhere) = keyring();
        here.ensure("bot_a").unwrap();
        let (other_key, _) = elsewhere.ensure("bot_a").unwrap();
        let envelope = seal_secret(&SECRET, &recipient_keys(&[&other_key]), b"x").unwrap();
        assert!(here.open_secret(&SECRET, &envelope).is_err());
        assert_eq!(&**elsewhere.open_secret(&SECRET, &envelope).unwrap(), b"x");
        assert!(seal_secret(&SECRET, &[], b"x").is_err());
    }

    #[test]
    fn a_seed_survives_encoding() {
        let secret = InboxSecret::generate();
        let again = InboxSecret::from_encoded(&secret.encoded_seed()).unwrap();
        assert_eq!(again.public(), secret.public());
        assert!(InboxSecret::from_encoded("short").is_err());
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
