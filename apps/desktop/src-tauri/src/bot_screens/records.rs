use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use flow_like_types::{Result, anyhow};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

pub(super) const MAX_PLAINTEXT: usize = 16 * 1024 - 16;
pub(super) const SECRET_BYTES: usize = 32;
const KEY_BYTES: usize = 32;
const INFO_PREFIX: &str = "flow-like/bot-screens/stream/v1|";

type Key = Zeroizing<[u8; KEY_BYTES]>;

pub(super) struct StreamKeys {
    viewer_to_host: Key,
    host_to_viewer: Key,
}

impl StreamKeys {
    pub fn derive(viewer_secret: &[u8], host_secret: &[u8], session: &str) -> Result<Self> {
        let ikm = Zeroizing::new([viewer_secret, host_secret].concat());
        let hkdf = Hkdf::<Sha256>::new(Some(session.as_bytes()), &ikm);
        let expand = |direction: &str| -> Result<Key> {
            let mut key = Zeroizing::new([0u8; KEY_BYTES]);
            hkdf.expand(format!("{INFO_PREFIX}{direction}").as_bytes(), &mut key[..])
                .map_err(|_| anyhow!("Derive the bot screen {direction} stream key"))?;
            Ok(key)
        };
        Ok(Self {
            viewer_to_host: expand("viewer->host")?,
            host_to_viewer: expand("host->viewer")?,
        })
    }

    pub fn host(&self) -> (Sealer, Opener) {
        (
            Sealer::new(&self.host_to_viewer),
            Opener::new(&self.viewer_to_host),
        )
    }

    #[cfg(test)]
    pub fn viewer(&self) -> (Sealer, Opener) {
        (
            Sealer::new(&self.viewer_to_host),
            Opener::new(&self.host_to_viewer),
        )
    }
}

fn nonce(counter: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

pub(super) struct Sealer {
    cipher: Aes256Gcm,
    counter: u64,
}

impl Sealer {
    fn new(key: &Key) -> Self {
        Self {
            cipher: Aes256Gcm::new(aes_gcm::Key::<Aes256Gcm>::from_slice(&key[..])),
            counter: 0,
        }
    }

    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(anyhow!(
                "A bot screen record carries at most {MAX_PLAINTEXT} bytes, got {}",
                plaintext.len()
            ));
        }
        let record = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce(self.counter)), plaintext)
            .map_err(|_| anyhow!("Encrypt bot screen record {}", self.counter))?;
        self.counter += 1;
        Ok(record)
    }
}

pub(super) struct Opener {
    cipher: Aes256Gcm,
    counter: u64,
}

impl Opener {
    fn new(key: &Key) -> Self {
        Self {
            cipher: Aes256Gcm::new(aes_gcm::Key::<Aes256Gcm>::from_slice(&key[..])),
            counter: 0,
        }
    }

    pub fn open(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(&nonce(self.counter)), record)
            .map_err(|_| {
                anyhow!(
                    "Bot screen record {} does not decrypt: wrong key, tampered or out of order",
                    self.counter
                )
            })?;
        self.counter += 1;
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "session0123456789";
    const TAKE_OVER: &str = "10000000147b2274797065223a2274616b655f6f766572227d";

    fn keys() -> StreamKeys {
        StreamKeys::derive(&[0x11; 32], &[0x22; 32], SESSION).unwrap()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn derives_the_published_keys() {
        let keys = keys();
        assert_eq!(
            hex(&keys.viewer_to_host[..]),
            "d6d73f83549a18a32cc11027fa390b1e6e717684c11181e523a918d55b56f6c0"
        );
        assert_eq!(
            hex(&keys.host_to_viewer[..]),
            "cf7d536a0d49bd7f289ceffa6d820f3956a6b25079cefdbd88c9e0cda6dacc22"
        );
    }

    #[test]
    fn viewer_records_match_the_vectors() {
        let (mut sealer, _) = keys().viewer();
        assert_eq!(
            hex(&sealer.seal(&[]).unwrap()),
            "582154017df1dbe93e30aecc32f9812a"
        );
        assert_eq!(
            hex(&sealer.seal(&unhex(TAKE_OVER)).unwrap()),
            "7d499b12975d3bb1e70fcbc3d8396c85e087c6a208869c76ef636fd9067aa5001ccf672c80712f7e73"
        );
    }

    #[test]
    fn host_record_matches_the_vector() {
        let (mut sealer, _) = keys().host();
        assert_eq!(
            hex(&sealer.seal(&[1, 2, 3]).unwrap()),
            "eb42a74f10aabc216519434cc7eeab8fda527a"
        );
    }

    #[test]
    fn host_opens_what_the_viewer_sealed_in_order() {
        let (mut viewer, _) = keys().viewer();
        let (_, mut host) = keys().host();
        let first = viewer.seal(&[]).unwrap();
        let second = viewer.seal(b"payload").unwrap();
        assert!(host.open(&first).unwrap().is_empty());
        assert_eq!(host.open(&second).unwrap(), b"payload");
    }

    #[test]
    fn a_gap_a_replay_or_tampering_fails() {
        let (mut viewer, _) = keys().viewer();
        let first = viewer.seal(b"a").unwrap();
        let second = viewer.seal(b"b").unwrap();
        let (_, mut skipped) = keys().host();
        assert!(skipped.open(&second).is_err());
        let (_, mut replayed) = keys().host();
        replayed.open(&first).unwrap();
        assert!(replayed.open(&first).is_err());
        let mut tampered = first.clone();
        tampered[0] ^= 1;
        let (_, mut host) = keys().host();
        assert!(host.open(&tampered).is_err());
    }

    #[test]
    fn oversized_plaintext_is_refused() {
        let (mut sealer, _) = keys().host();
        assert!(sealer.seal(&vec![0; MAX_PLAINTEXT]).is_ok());
        assert!(sealer.seal(&vec![0; MAX_PLAINTEXT + 1]).is_err());
    }
}
