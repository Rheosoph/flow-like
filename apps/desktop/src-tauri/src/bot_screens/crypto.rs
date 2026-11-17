use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, OsRng, Payload, rand_core::RngCore},
};
use flow_like_types::{
    Result, anyhow,
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64},
};
use serde::{Deserialize, Serialize};

const IV_LEN: usize = 12;
const KEY_LEN: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Envelope {
    pub v: u8,
    pub key_id: String,
    pub iv: String,
    pub ct: String,
}

#[derive(Clone, Deserialize)]
pub(super) struct RawKey {
    pub id: String,
    pub key: String,
}

#[derive(Clone)]
pub(super) struct Key {
    pub id: String,
    bytes: [u8; KEY_LEN],
}

impl Key {
    pub fn parse(raw: &RawKey) -> Result<Self> {
        let decoded = B64
            .decode(&raw.key)
            .map_err(|error| anyhow!("Bot screen key {} is not base64url: {error}", raw.id))?;
        let bytes: [u8; KEY_LEN] = decoded.try_into().map_err(|bytes: Vec<u8>| {
            anyhow!(
                "Bot screen key {} has {} bytes, expected {KEY_LEN}",
                raw.id,
                bytes.len()
            )
        })?;
        Ok(Self {
            id: raw.id.clone(),
            bytes,
        })
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(&self.bytes.into())
    }

    fn aad(&self, room: &str) -> Vec<u8> {
        format!("flow-like/bot-screens/v1|{room}|{}", self.id).into_bytes()
    }

    pub fn seal(&self, room: &str, plaintext: &[u8]) -> Result<Envelope> {
        let mut iv = [0u8; IV_LEN];
        OsRng.fill_bytes(&mut iv);
        self.seal_with(room, iv, plaintext)
    }

    fn seal_with(&self, room: &str, iv: [u8; IV_LEN], plaintext: &[u8]) -> Result<Envelope> {
        let aad = self.aad(room);
        let ct = self
            .cipher()
            .encrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow!("Encrypt bot screen message for key {}", self.id))?;
        Ok(Envelope {
            v: 1,
            key_id: self.id.clone(),
            iv: B64.encode(iv),
            ct: B64.encode(ct),
        })
    }

    pub fn open(&self, room: &str, envelope: &Envelope) -> Option<Vec<u8>> {
        if envelope.v != 1 || envelope.key_id != self.id {
            return None;
        }
        let iv: [u8; IV_LEN] = B64.decode(&envelope.iv).ok()?.try_into().ok()?;
        let ct = B64.decode(&envelope.ct).ok()?;
        let aad = self.aad(room);
        self.cipher()
            .decrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &ct,
                    aad: &aad,
                },
            )
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: &str = "0123456789abcdef0123456789abcdef";
    const KEY: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";
    const IV: &str = "oKGio6Slpqeoqaqr";
    const PLAIN: &str =
        r#"{"type":"hello","from":"viewer0123456789","bot_id":"bot-1","at":1763337600000}"#;
    const CT: &str = "wvNJH43QeD5dZscHneoCqHWxwnY1SlqmUTKwijc86AQD4q392eWIDQ9a4vvJQhFOfZRK5q70ivh5xq_So0IlUp11ldedGHvH6qB2xDKnTNPhf2plgt4ppT_5xq-LNA";

    fn key() -> Key {
        Key::parse(&RawKey {
            id: "2026-11-17".into(),
            key: KEY.into(),
        })
        .unwrap()
    }

    fn vector() -> Envelope {
        Envelope {
            v: 1,
            key_id: "2026-11-17".into(),
            iv: IV.into(),
            ct: CT.into(),
        }
    }

    #[test]
    fn decrypts_the_published_vector() {
        assert_eq!(key().open(ROOM, &vector()).unwrap(), PLAIN.as_bytes());
    }

    #[test]
    fn encrypting_with_the_fixed_iv_reproduces_the_vector() {
        let iv: [u8; IV_LEN] = B64.decode(IV).unwrap().try_into().unwrap();
        assert_eq!(
            key().seal_with(ROOM, iv, PLAIN.as_bytes()).unwrap(),
            vector()
        );
    }

    #[test]
    fn wrong_room_key_or_tampering_is_dropped() {
        let key = key();
        assert!(
            key.open("fedcba9876543210fedcba9876543210", &vector())
                .is_none()
        );
        let mut tampered = vector();
        tampered.ct.replace_range(0..1, "x");
        assert!(key.open(ROOM, &tampered).is_none());
        let mut other_id = vector();
        other_id.key_id = "2026-11-16".into();
        assert!(key.open(ROOM, &other_id).is_none());
    }

    #[test]
    fn random_iv_roundtrips() {
        let key = key();
        let sealed = key.seal(ROOM, b"payload").unwrap();
        assert_eq!(key.open(ROOM, &sealed).unwrap(), b"payload");
    }

    #[test]
    fn keys_must_be_32_bytes() {
        assert!(
            Key::parse(&RawKey {
                id: "d".into(),
                key: "AQID".into()
            })
            .is_err()
        );
    }
}
