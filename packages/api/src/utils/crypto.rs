//! Symmetric at-rest encryption for user/app secrets stored in the database.
//!
//! AES-256-GCM with a fresh random 12-byte nonce per value; output is
//! base64(nonce || ciphertext). The key is `AppState.encryption_key`
//! (blake3 of the SINK_TOKEN_ENCRYPTION_KEY secret).

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::Engine;

/// Encrypt a secret string, returning base64-encoded nonce-prefixed ciphertext.
pub fn encrypt_secret(plaintext: &str, key: &[u8; 32]) -> String {
    let cipher = Aes256Gcm::new_from_slice(key).expect("Invalid key length");

    let mut nonce_bytes = [0u8; 12];
    getrandom::fill(&mut nonce_bytes).expect("Failed to generate random nonce");
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .expect("Encryption failed");

    let mut combined = nonce_bytes.to_vec();
    combined.extend(ciphertext);
    base64::engine::general_purpose::STANDARD.encode(combined)
}

/// Decrypt a base64-encoded nonce-prefixed ciphertext. Returns `None` on any
/// failure (wrong key, corruption, truncation).
pub fn decrypt_secret(encrypted: &str, key: &[u8; 32]) -> Option<String> {
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;

    let combined = base64::engine::general_purpose::STANDARD
        .decode(encrypted)
        .ok()?;

    if combined.len() < 12 {
        return None;
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;
    String::from_utf8(plaintext).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = [42u8; 32];
        let encrypted = encrypt_secret("my-api-key", &key);
        assert_eq!(
            decrypt_secret(&encrypted, &key).as_deref(),
            Some("my-api-key")
        );
    }

    #[test]
    fn wrong_key_returns_none() {
        let encrypted = encrypt_secret("my-api-key", &[1u8; 32]);
        assert_eq!(decrypt_secret(&encrypted, &[2u8; 32]), None);
    }

    #[test]
    fn garbage_returns_none() {
        assert_eq!(decrypt_secret("not-base64!!", &[0u8; 32]), None);
        assert_eq!(decrypt_secret("aGVsbG8=", &[0u8; 32]), None);
    }
}
