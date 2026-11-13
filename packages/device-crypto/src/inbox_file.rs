//! Files of end-to-end encrypted conversations.
//!
//! The sender's device seals a file under a fresh 32-byte key and uploads only the ciphertext as
//! a conversation file, so the server learns its size and nothing else. The key travels with the
//! file's name, type and size inside the sealed message ([`crate::inbox`]); only the members that
//! message was sealed to can open the file.
//!
//! Format ([`FILE_ALGORITHM`]): a 19-byte random nonce prefix, then the file in 64 KiB chunks,
//! each sealed with XChaCha20-Poly1305 in the STREAM construction of `aead::stream` (big-endian
//! 32-bit counter and last-chunk flag), the construction app sharing uses. The AAD of every chunk
//! names the conversation, so a file opens only in the conversation it was sealed for, and a
//! truncated, reordered or extended file does not open at all.

use crate::{CryptoError, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305,
    aead::{
        Payload,
        generic_array::GenericArray,
        stream::{DecryptorBE32, EncryptorBE32},
    },
};
use rand_core::{OsRng, RngCore};
use serde_json::json;
use zeroize::Zeroizing;

/// The `alg` a sealed attachment names.
pub const FILE_ALGORITHM: &str = "xchacha20poly1305-stream-64k-v1";
/// Plaintext bytes per sealed chunk.
pub const FILE_CHUNK_BYTES: usize = 64 * 1024;
const NONCE_PREFIX_BYTES: usize = 19;
const TAG_BYTES: usize = 16;
const KEY_BYTES: usize = 32;
const FILE_LABEL: &str = "flow-like-inbox-file-v1";

/// A sealed file: the ciphertext to upload and its key, base64url, for the sealed message.
pub struct SealedFile {
    pub key: Zeroizing<String>,
    pub ciphertext: Vec<u8>,
}

/// The size of a `plain_len`-byte file once sealed: what the upload announces and stores.
pub fn sealed_len(plain_len: u64) -> u64 {
    let chunks = plain_len.div_ceil(FILE_CHUNK_BYTES as u64).max(1);
    NONCE_PREFIX_BYTES as u64 + plain_len + chunks * TAG_BYTES as u64
}

fn aad(conversation_id: &str) -> Vec<u8> {
    json!([FILE_LABEL, conversation_id])
        .to_string()
        .into_bytes()
}

fn cipher(key: &[u8; KEY_BYTES]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(key.into())
}

fn decode_key(key: &str) -> Result<Zeroizing<[u8; KEY_BYTES]>> {
    let bytes = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(key)
            .map_err(|_| CryptoError::InvalidInput("a file key is 32 bytes, base64url"))?,
    );
    let mut decoded = Zeroizing::new([0u8; KEY_BYTES]);
    if bytes.len() != KEY_BYTES {
        return Err(CryptoError::InvalidInput(
            "a file key is 32 bytes, base64url",
        ));
    }
    decoded.copy_from_slice(&bytes);
    Ok(decoded)
}

/// `plaintext` sealed for `conversation_id` under a key made for this file alone.
pub fn seal_file(conversation_id: &str, plaintext: &[u8]) -> Result<SealedFile> {
    let mut key = Zeroizing::new([0u8; KEY_BYTES]);
    OsRng.fill_bytes(key.as_mut());
    let mut nonce = [0u8; NONCE_PREFIX_BYTES];
    OsRng.fill_bytes(&mut nonce);
    let aad = aad(conversation_id);
    let failed = |_| CryptoError::Operation("sealing a conversation file failed");
    let mut ciphertext = Vec::with_capacity(sealed_len(plaintext.len() as u64) as usize);
    ciphertext.extend_from_slice(&nonce);
    let mut encryptor = EncryptorBE32::from_aead(cipher(&key), GenericArray::from_slice(&nonce));
    let mut chunks = plaintext.chunks(FILE_CHUNK_BYTES);
    let mut current = chunks.next().unwrap_or_default();
    for next in chunks {
        let sealed = encryptor
            .encrypt_next(Payload {
                msg: current,
                aad: &aad,
            })
            .map_err(failed)?;
        ciphertext.extend_from_slice(&sealed);
        current = next;
    }
    let last = encryptor
        .encrypt_last(Payload {
            msg: current,
            aad: &aad,
        })
        .map_err(failed)?;
    ciphertext.extend_from_slice(&last);
    Ok(SealedFile {
        key: Zeroizing::new(URL_SAFE_NO_PAD.encode(key.as_slice())),
        ciphertext,
    })
}

/// The file `ciphertext` holds, opened with `key` (base64url) for `conversation_id`.
pub fn open_file(conversation_id: &str, key: &str, ciphertext: &[u8]) -> Result<Vec<u8>> {
    let key = decode_key(key)?;
    if ciphertext.len() < NONCE_PREFIX_BYTES + TAG_BYTES {
        return Err(CryptoError::InvalidInput(
            "a sealed conversation file is shorter than its nonce and one tag",
        ));
    }
    let (nonce, body) = ciphertext.split_at(NONCE_PREFIX_BYTES);
    let aad = aad(conversation_id);
    let failed = |_| CryptoError::Operation("the conversation file does not open with this key");
    let mut decryptor = DecryptorBE32::from_aead(cipher(&key), GenericArray::from_slice(nonce));
    let mut plaintext = Vec::with_capacity(body.len());
    let mut chunks = body.chunks(FILE_CHUNK_BYTES + TAG_BYTES);
    let mut current = chunks.next().unwrap_or_default();
    for next in chunks {
        let opened = decryptor
            .decrypt_next(Payload {
                msg: current,
                aad: &aad,
            })
            .map_err(failed)?;
        plaintext.extend_from_slice(&opened);
        current = next;
    }
    let last = decryptor
        .decrypt_last(Payload {
            msg: current,
            aad: &aad,
        })
        .map_err(failed)?;
    plaintext.extend_from_slice(&last);
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::{InboxSecret, Member, RecipientKey, Scope, open_with, seal_all};
    use serde_json::Value;

    const ROOM: &str = "conv-1";

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index * 31 % 251) as u8).collect()
    }

    #[test]
    fn a_file_of_any_size_opens_as_it_was_and_has_its_announced_length() {
        for len in [
            0,
            1,
            FILE_CHUNK_BYTES - 1,
            FILE_CHUNK_BYTES,
            FILE_CHUNK_BYTES + 1,
            3 * FILE_CHUNK_BYTES + 5,
        ] {
            let plaintext = sample(len);
            let sealed = seal_file(ROOM, &plaintext).unwrap();
            assert_eq!(
                sealed.ciphertext.len() as u64,
                sealed_len(len as u64),
                "{len}"
            );
            assert_eq!(
                open_file(ROOM, &sealed.key, &sealed.ciphertext).unwrap(),
                plaintext
            );
        }
    }

    #[test]
    fn every_file_gets_its_own_key_and_the_ciphertext_hides_the_content() {
        let plaintext = b"quarterly numbers".repeat(100);
        let first = seal_file(ROOM, &plaintext).unwrap();
        let second = seal_file(ROOM, &plaintext).unwrap();
        assert_ne!(*first.key, *second.key);
        assert_ne!(first.ciphertext, second.ciphertext);
        assert!(
            !first
                .ciphertext
                .windows(b"quarterly".len())
                .any(|window| window == b"quarterly")
        );
        assert!(open_file(ROOM, &second.key, &first.ciphertext).is_err());
    }

    #[test]
    fn a_file_opens_only_in_the_conversation_it_was_sealed_for() {
        let sealed = seal_file(ROOM, b"minutes").unwrap();
        assert!(open_file("conv-2", &sealed.key, &sealed.ciphertext).is_err());
        assert_eq!(
            open_file(ROOM, &sealed.key, &sealed.ciphertext).unwrap(),
            b"minutes"
        );
    }

    #[test]
    fn a_changed_cut_or_extended_file_does_not_open() {
        let sealed = seal_file(ROOM, &sample(2 * FILE_CHUNK_BYTES + 10)).unwrap();
        let mut flipped = sealed.ciphertext.clone();
        flipped[NONCE_PREFIX_BYTES + 7] ^= 1;
        assert!(open_file(ROOM, &sealed.key, &flipped).is_err());
        let chunk = FILE_CHUNK_BYTES + TAG_BYTES;
        let cut = &sealed.ciphertext[..NONCE_PREFIX_BYTES + 2 * chunk];
        assert!(open_file(ROOM, &sealed.key, cut).is_err());
        let mut extended = sealed.ciphertext.clone();
        extended.extend_from_slice(&[0u8; TAG_BYTES]);
        assert!(open_file(ROOM, &sealed.key, &extended).is_err());
        let mut swapped = sealed.ciphertext.clone();
        let (first, second) = (NONCE_PREFIX_BYTES, NONCE_PREFIX_BYTES + chunk);
        let head: Vec<u8> = swapped[first..first + chunk].to_vec();
        let next: Vec<u8> = swapped[second..second + chunk].to_vec();
        swapped[first..first + chunk].copy_from_slice(&next);
        swapped[second..second + chunk].copy_from_slice(&head);
        assert!(open_file(ROOM, &sealed.key, &swapped).is_err());
        assert!(open_file(ROOM, &sealed.key, &sealed.ciphertext[..10]).is_err());
        assert!(open_file(ROOM, "not-a-key", &sealed.ciphertext).is_err());
    }

    #[test]
    fn the_largest_conversation_file_fits_once_sealed() {
        const LIMIT: u64 = 35 * 1024 * 1024;
        let overhead = sealed_len(LIMIT) - LIMIT;
        assert_eq!(overhead, NONCE_PREFIX_BYTES as u64 + 560 * TAG_BYTES as u64);
        assert!(sealed_len(LIMIT - overhead) <= LIMIT);
        assert_eq!(sealed_len(0), (NONCE_PREFIX_BYTES + TAG_BYTES) as u64);
    }

    fn member(id: &str, secret: &InboxSecret) -> Member {
        Member {
            principal_id: id.to_owned(),
            keys: vec![RecipientKey {
                key_id: secret.public().key_id.clone(),
                public_key: secret.public().public_key.clone(),
            }],
        }
    }

    fn opened_key(
        secret: &InboxSecret,
        scope: &Scope,
        id: &str,
        envelope: &Value,
    ) -> Option<String> {
        let plaintext = open_with(std::slice::from_ref(secret), scope, id, envelope).ok()?;
        let body: Value = serde_json::from_slice(&plaintext).ok()?;
        body["parts"][0]["sealed"]["key"]
            .as_str()
            .map(str::to_owned)
    }

    #[test]
    fn a_file_key_opens_only_for_its_message_and_the_members_it_was_sealed_to() {
        let (alice, bot, removed) = (
            InboxSecret::generate(),
            InboxSecret::generate(),
            InboxSecret::generate(),
        );
        let sealed = seal_file(ROOM, b"contract draft").unwrap();
        let body = json!({
            "text": "the draft",
            "parts": [{
                "kind": "attachment",
                "id": "a1",
                "name": "draft.pdf",
                "sealed": { "alg": FILE_ALGORITHM, "key": *sealed.key, "mime": "application/pdf", "bytes": 14 }
            }]
        });
        let scope = Scope {
            conversation_id: ROOM,
            message_id: "m1",
            sender_id: "alice",
        };
        let envelopes = seal_all(
            &scope,
            &[member("alice", &alice), member("bot", &bot)],
            &serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();
        assert!(!envelopes.contains_key("removed"));
        let to_bot = &envelopes["bot"];
        let key = opened_key(&bot, &scope, "bot", to_bot).unwrap();
        assert_eq!(
            open_file(ROOM, &key, &sealed.ciphertext).unwrap(),
            b"contract draft"
        );
        for elsewhere in [
            Scope {
                message_id: "m2",
                ..scope
            },
            Scope {
                conversation_id: "conv-2",
                ..scope
            },
        ] {
            assert!(opened_key(&bot, &elsewhere, "bot", to_bot).is_none());
        }
        for envelope in envelopes.values() {
            assert!(opened_key(&removed, &scope, "removed", envelope).is_none());
            assert!(opened_key(&removed, &scope, "bot", envelope).is_none());
        }
        let text = serde_json::to_string(&envelopes).unwrap();
        assert!(!text.contains(sealed.key.as_str()) && !text.contains("draft.pdf"));
    }
}
