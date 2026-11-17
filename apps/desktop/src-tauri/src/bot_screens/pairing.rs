use super::{
    sealed::random_secret,
    trust::{TrustStore, TrustedKey},
};
use flow_like_device_crypto::inbox::{fingerprint, key_id_of};
use flow_like_types::{
    Result, anyhow,
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64},
};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub(super) const MAX_LABEL_CHARS: usize = 64;
pub(super) const PENDING_TTL: Duration = Duration::from_secs(5 * 60);
const FALLBACK_LABEL: &str = "This computer";
const PAIR_PREFIX: &str = "flow-like/bot-screens/pair/v1";
const PAIRED_PREFIX: &str = "flow-like/bot-screens/paired/v1";
const LINK_PREFIX: &str = "flowlike://pair-screen?";
const NOT_RUNNING: &str =
    "Sign in and keep the app open: this computer's bot screen host is not connected yet";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    pub sub: String,
    pub key_id: String,
    pub public_key: String,
}

struct Pending {
    secret: [u8; 32],
    expires: Instant,
}

pub(super) struct PairRequest<'a> {
    pub session: &'a str,
    pub key_id: &'a str,
    pub public_key: &'a str,
    pub label: &'a str,
    pub mac: &'a str,
}

/// State shared by the host loop and the pairing commands.
pub(super) struct Shared {
    pub peer: String,
    pub label: String,
    pub trust: TrustStore,
    identity: Mutex<Option<Identity>>,
    pending: Mutex<Option<Pending>>,
    live: Mutex<Vec<(String, CancellationToken)>>,
}

/// Labels are bounded in UTF-16 units, as the TS viewer measures `string.length`.
fn label_len(label: &str) -> usize {
    label.encode_utf16().count()
}

fn clamp_label(label: &str) -> String {
    let mut used = 0;
    label
        .chars()
        .take_while(|char| {
            used += char.len_utf16();
            used <= MAX_LABEL_CHARS
        })
        .collect()
}

pub(super) fn host_label() -> String {
    let name = gethostname::gethostname();
    let label = clamp_label(name.to_string_lossy().trim());
    if label.is_empty() {
        FALLBACK_LABEL.to_owned()
    } else {
        label
    }
}

fn mac_of(secret: &[u8], message: &str) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts a key of any length");
    mac.update(message.as_bytes());
    mac
}

fn pair_message(host_peer: &str, host_key_id: &str, request: &PairRequest) -> String {
    format!(
        "{PAIR_PREFIX}|{host_peer}|{host_key_id}|{}|{}|{}",
        request.key_id, request.public_key, request.session
    )
}

fn paired_message(session: &str) -> String {
    format!("{PAIRED_PREFIX}|{session}")
}

fn paired_mac(secret: &[u8], session: &str) -> String {
    B64.encode(
        mac_of(secret, &paired_message(session))
            .finalize()
            .into_bytes(),
    )
}

pub(super) fn pairing_link(
    host_peer: &str,
    identity: &Identity,
    secret: &[u8],
    label: &str,
) -> String {
    format!(
        "{LINK_PREFIX}v=1&h={host_peer}&k={}&p={}&s={}&n={}",
        urlencoding::encode(&identity.key_id),
        urlencoding::encode(&identity.public_key),
        B64.encode(secret),
        urlencoding::encode(label)
    )
}

#[derive(Serialize)]
pub struct PairingStart {
    pub link: String,
    pub expires_at: i64,
    pub host_label: String,
}

#[derive(Serialize)]
pub struct PairedDevice {
    pub key_id: String,
    pub label: String,
    pub fingerprint: String,
    pub paired_at: i64,
}

impl Shared {
    pub fn new(peer: String, trust: TrustStore) -> Self {
        Self {
            peer,
            label: host_label(),
            trust,
            identity: Mutex::new(None),
            pending: Mutex::new(None),
            live: Mutex::new(Vec::new()),
        }
    }

    /// Registers a running session of a viewer key so unpairing that key ends it.
    pub fn track(&self, key_id: &str, session: CancellationToken) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.retain(|(_, token)| !token.is_cancelled());
        live.push((key_id.to_owned(), session));
    }

    fn end_sessions(&self, key_id: &str) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.retain(|(owner, token)| {
            if owner == key_id {
                token.cancel();
            }
            owner != key_id && !token.is_cancelled()
        });
    }

    pub fn set_identity(&self, identity: Option<Identity>) {
        *self.identity.lock().unwrap_or_else(|e| e.into_inner()) = identity;
    }

    pub fn identity(&self) -> Result<Identity, String> {
        self.identity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| NOT_RUNNING.to_owned())
    }

    pub fn start(&self, ttl: Duration) -> Result<(String, Duration), String> {
        let identity = self.identity()?;
        let secret = random_secret();
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pending {
            secret,
            expires: Instant::now() + ttl,
        });
        Ok((
            pairing_link(&self.peer, &identity, &secret, &self.label),
            ttl,
        ))
    }

    pub fn paired(&self) -> Result<Vec<PairedDevice>, String> {
        let identity = self.identity()?;
        Ok(self
            .trust
            .list(&identity.sub)
            .into_iter()
            .map(|key| PairedDevice {
                fingerprint: B64
                    .decode(&key.public_key)
                    .map(|bytes| fingerprint(&bytes))
                    .unwrap_or_default(),
                key_id: key.key_id,
                label: key.label,
                paired_at: key.paired_at,
            })
            .collect())
    }

    pub fn unpair(&self, key_id: &str) -> Result<(), String> {
        let identity = self.identity()?;
        self.trust
            .unpin(&identity.sub, key_id)
            .map_err(|error| error.to_string())?;
        self.end_sessions(key_id);
        Ok(())
    }

    /// Checks a viewer's pair request, pins its key and consumes the secret; returns the `paired` mac.
    pub fn accept(
        &self,
        identity: &Identity,
        request: &PairRequest,
        now_secs: i64,
    ) -> Result<String> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let secret = match pending.as_ref() {
            Some(open) if open.expires > Instant::now() => open.secret,
            Some(_) => {
                *pending = None;
                return Err(anyhow!("The pairing code expired"));
            }
            None => return Err(anyhow!("No pairing code is open")),
        };
        let expected = pair_message(&self.peer, &identity.key_id, request);
        let mac = B64
            .decode(request.mac)
            .map_err(|_| anyhow!("The pairing mac is not base64url"))?;
        mac_of(&secret, &expected)
            .verify_slice(&mac)
            .map_err(|_| anyhow!("The pairing mac does not match"))?;
        let public = B64
            .decode(request.public_key)
            .ok()
            .filter(|bytes| bytes.len() == 32)
            .ok_or_else(|| anyhow!("The viewer key is not a 32 byte base64url key"))?;
        if key_id_of(&public) != request.key_id {
            return Err(anyhow!(
                "The viewer key id does not belong to its public key"
            ));
        }
        if label_len(request.label) > MAX_LABEL_CHARS {
            return Err(anyhow!(
                "The viewer label is longer than {MAX_LABEL_CHARS} UTF-16 units"
            ));
        }
        self.trust.pin(
            &identity.sub,
            TrustedKey {
                key_id: request.key_id.to_owned(),
                public_key: request.public_key.to_owned(),
                label: request.label.to_owned(),
                paired_at: now_secs,
            },
        )?;
        *pending = None;
        Ok(paired_mac(&secret, request.session))
    }
}

#[cfg(test)]
mod tests {
    use super::super::trust::tests::TempFile;
    use super::*;
    use flow_like_device_crypto::inbox::InboxSecret;

    const SECRET: [u8; 32] = [0x33; 32];
    const PEER: &str = "0123456789abcdef0123456789abcdef";
    const HOST_KEY: &str = "ik_0102030405060708";
    const VIEWER_KEY: &str = "ik_a1a2a3a4a5a6a7a8";
    const VIEWER_PK: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";
    const SESSION: &str = "session0123456789";

    fn identity() -> Identity {
        Identity {
            sub: "user-1".into(),
            key_id: HOST_KEY.into(),
            public_key: "host-pk".into(),
        }
    }

    fn shared(file: &TempFile) -> Shared {
        let shared = Shared::new(PEER.into(), TrustStore::new(file.0.clone()));
        shared.set_identity(Some(identity()));
        shared
    }

    fn open(shared: &Shared, ttl: Duration) {
        *shared.pending.lock().unwrap() = Some(Pending {
            secret: SECRET,
            expires: Instant::now() + ttl,
        });
    }

    fn request_mac(request: &PairRequest) -> String {
        B64.encode(
            mac_of(&SECRET, &pair_message(PEER, HOST_KEY, request))
                .finalize()
                .into_bytes(),
        )
    }

    struct Viewer {
        key_id: String,
        public_key: String,
    }

    fn viewer() -> Viewer {
        let public = InboxSecret::generate().public().clone();
        Viewer {
            key_id: public.key_id,
            public_key: public.public_key,
        }
    }

    fn pair(shared: &Shared, viewer: &Viewer, label: &str, mac: Option<&str>) -> Result<String> {
        let mut request = PairRequest {
            session: SESSION,
            key_id: &viewer.key_id,
            public_key: &viewer.public_key,
            label,
            mac: "",
        };
        let computed = request_mac(&request);
        request.mac = mac.unwrap_or(&computed);
        shared.accept(&identity(), &request, 100)
    }

    #[test]
    fn the_macs_match_the_ts_vectors() {
        let request = PairRequest {
            session: SESSION,
            key_id: VIEWER_KEY,
            public_key: VIEWER_PK,
            label: "Phone",
            mac: "",
        };
        assert_eq!(
            request_mac(&request),
            "s1JkC-BQQnPvhhCRptnAtNIjdcJp5M7ntT4iym56mIw"
        );
        assert_eq!(
            paired_mac(&SECRET, SESSION),
            "IJSa8czhetKUT6kphhn_Iq0ot5nvSUhrrj11NMGEJ_Q"
        );
    }

    #[test]
    fn the_link_matches_the_ts_parser() {
        let identity = Identity {
            sub: "u".into(),
            key_id: HOST_KEY.into(),
            public_key: VIEWER_PK.into(),
        };
        let link = pairing_link(PEER, &identity, &SECRET, "Felix's Mac & Co");
        assert_eq!(
            link,
            format!(
                "flowlike://pair-screen?v=1&h={PEER}&k={HOST_KEY}&p={VIEWER_PK}&s=MzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzM&n=Felix%27s%20Mac%20%26%20Co"
            )
        );
    }

    #[test]
    fn a_valid_request_pins_the_key_and_the_secret_is_single_use() {
        let file = TempFile::new();
        let shared = shared(&file);
        let viewer = viewer();
        open(&shared, PENDING_TTL);
        let mac = pair(&shared, &viewer, "Phone", None).unwrap();
        assert_eq!(mac, paired_mac(&SECRET, SESSION));
        let pinned = shared.trust.get("user-1", &viewer.key_id).unwrap();
        assert_eq!((pinned.label.as_str(), pinned.paired_at), ("Phone", 100));
        assert!(pair(&shared, &viewer, "Phone", None).is_err());
    }

    #[test]
    fn a_wrong_mac_is_refused_and_keeps_the_secret_open() {
        let file = TempFile::new();
        let shared = shared(&file);
        let viewer = viewer();
        open(&shared, PENDING_TTL);
        let bad = B64.encode([1u8; 32]);
        assert!(pair(&shared, &viewer, "Phone", Some(&bad)).is_err());
        assert!(pair(&shared, &viewer, "Phone", Some("***")).is_err());
        assert!(shared.trust.list("user-1").is_empty());
        assert!(pair(&shared, &viewer, "Phone", None).is_ok());
    }

    #[test]
    fn an_expired_or_missing_secret_is_refused() {
        let file = TempFile::new();
        let shared = shared(&file);
        let viewer = viewer();
        assert!(pair(&shared, &viewer, "Phone", None).is_err());
        open(&shared, Duration::ZERO);
        assert!(pair(&shared, &viewer, "Phone", None).is_err());
        assert!(shared.trust.list("user-1").is_empty());
    }

    #[test]
    fn a_key_id_that_does_not_match_the_key_is_refused() {
        let file = TempFile::new();
        let shared = shared(&file);
        let mut viewer = viewer();
        viewer.key_id = VIEWER_KEY.into();
        open(&shared, PENDING_TTL);
        assert!(pair(&shared, &viewer, "Phone", None).is_err());
        assert!(shared.trust.list("user-1").is_empty());
    }

    #[test]
    fn a_long_label_is_refused() {
        let file = TempFile::new();
        let shared = shared(&file);
        let viewer = viewer();
        open(&shared, PENDING_TTL);
        assert!(pair(&shared, &viewer, &"x".repeat(65), None).is_err());
        assert!(pair(&shared, &viewer, &"x".repeat(64), None).is_ok());
    }

    #[test]
    fn a_new_start_replaces_the_old_secret_and_commands_need_the_host() {
        let file = TempFile::new();
        let shared = shared(&file);
        let (first, _) = shared.start(PENDING_TTL).unwrap();
        let (second, _) = shared.start(PENDING_TTL).unwrap();
        assert_ne!(first, second);
        assert!(second.starts_with(&format!(
            "{LINK_PREFIX}v=1&h={PEER}&k={HOST_KEY}&p=host-pk&s="
        )));
        shared.set_identity(None);
        assert!(shared.start(PENDING_TTL).is_err());
        assert!(shared.paired().is_err());
        assert!(shared.unpair("k").is_err());
    }

    #[test]
    fn the_paired_list_carries_fingerprints_and_unpair_removes() {
        let file = TempFile::new();
        let shared = shared(&file);
        let viewer = viewer();
        open(&shared, PENDING_TTL);
        pair(&shared, &viewer, "Phone", None).unwrap();
        let list = shared.paired().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].fingerprint.split('-').count(), 8);
        shared.unpair(&viewer.key_id).unwrap();
        assert!(shared.paired().unwrap().is_empty());
    }

    #[test]
    fn unpairing_ends_only_that_keys_live_sessions() {
        let file = TempFile::new();
        let shared = shared(&file);
        let (phone, laptop) = (CancellationToken::new(), CancellationToken::new());
        shared.track("ik_phone", phone.clone());
        shared.track("ik_laptop", laptop.clone());
        shared.unpair("ik_phone").unwrap();
        assert!(phone.is_cancelled());
        assert!(!laptop.is_cancelled());
        assert_eq!(shared.live.lock().unwrap().len(), 1);
    }

    #[test]
    fn labels_are_measured_in_utf16_units_like_the_viewer() {
        let emoji = "😀".repeat(40);
        assert_eq!(label_len(&emoji), 80);
        let clamped = clamp_label(&emoji);
        assert_eq!((label_len(&clamped), clamped.chars().count()), (64, 32));
        let file = TempFile::new();
        let shared = shared(&file);
        let viewer = viewer();
        open(&shared, PENDING_TTL);
        assert!(pair(&shared, &viewer, &"😀".repeat(33), None).is_err());
        assert!(pair(&shared, &viewer, &"😀".repeat(32), None).is_ok());
    }
}
