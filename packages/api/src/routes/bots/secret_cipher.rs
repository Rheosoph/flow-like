//! Server-side encryption of bot secrets. Each bot has its own AES-256-GCM key, derived from
//! the deployment key and the bot id, and each ciphertext is bound to its bot and secret name
//! through the associated data, so a stored value can't be moved to another bot or name.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use base64::Engine;
use flow_like_types::{Result, anyhow};

const KEY_CONTEXT: &str = "flow-like bot secret v1";
const PREFIX: &str = "v1:";
const NONCE_LEN: usize = 12;

fn bot_key(master: &[u8; 32], bot_id: &str) -> [u8; 32] {
    let mut material = Vec::with_capacity(master.len() + bot_id.len());
    material.extend_from_slice(master);
    material.extend_from_slice(bot_id.as_bytes());
    blake3::derive_key(KEY_CONTEXT, &material)
}

fn associated_data(bot_id: &str, name: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(bot_id.len() + 1 + name.len());
    aad.extend_from_slice(bot_id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(name.as_bytes());
    aad
}

fn cipher(master: &[u8; 32], bot_id: &str) -> Result<Aes256Gcm> {
    Aes256Gcm::new_from_slice(&bot_key(master, bot_id))
        .map_err(|_| anyhow!("Bot secret key for bot '{bot_id}' has an invalid length"))
}

/// `v1:` + base64(nonce || ciphertext) of `value`, for secret `name` of bot `bot_id`.
pub(crate) fn encrypt(master: &[u8; 32], bot_id: &str, name: &str, value: &str) -> Result<String> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce)
        .map_err(|error| anyhow!("No nonce for bot secret '{name}' of bot '{bot_id}': {error}"))?;
    let aad = associated_data(bot_id, name);
    let ciphertext = cipher(master, bot_id)?
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: value.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("Encrypting bot secret '{name}' of bot '{bot_id}' failed"))?;
    let mut sealed = nonce.to_vec();
    sealed.extend(ciphertext);
    Ok(format!(
        "{PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(sealed)
    ))
}

/// The value [`encrypt`] stored for secret `name` of bot `bot_id`. Fails for any other bot,
/// name or deployment key, and for anything that was not written by [`encrypt`].
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the bot runtime reads secrets in a later phase")
)]
pub(crate) fn decrypt(master: &[u8; 32], bot_id: &str, name: &str, stored: &str) -> Result<String> {
    let encoded = stored
        .strip_prefix(PREFIX)
        .ok_or_else(|| anyhow!("Bot secret '{name}' of bot '{bot_id}' has no '{PREFIX}' prefix"))?;
    let sealed = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| anyhow!("Bot secret '{name}' of bot '{bot_id}' is not valid base64"))?;
    if sealed.len() <= NONCE_LEN {
        return Err(anyhow!(
            "Bot secret '{name}' of bot '{bot_id}' is too short to hold a nonce and a tag"
        ));
    }
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
    let aad = associated_data(bot_id, name);
    let plaintext = cipher(master, bot_id)?
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| {
            anyhow!("Bot secret '{name}' of bot '{bot_id}' does not open with this bot's key")
        })?;
    String::from_utf8(plaintext)
        .map_err(|_| anyhow!("Bot secret '{name}' of bot '{bot_id}' is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn round_trips_with_a_fresh_nonce_each_time() {
        let first = encrypt(&KEY, "bot_a", "API_KEY", "sk-123").unwrap();
        let second = encrypt(&KEY, "bot_a", "API_KEY", "sk-123").unwrap();
        assert_ne!(first, second);
        assert_eq!(decrypt(&KEY, "bot_a", "API_KEY", &first).unwrap(), "sk-123");
        assert_eq!(
            decrypt(&KEY, "bot_a", "API_KEY", &second).unwrap(),
            "sk-123"
        );
    }

    #[test]
    fn stores_a_versioned_base64_text() {
        let stored = encrypt(&KEY, "bot_a", "API_KEY", "").unwrap();
        let encoded = stored.strip_prefix("v1:").unwrap();
        let sealed = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(sealed.len(), NONCE_LEN + 16);
        assert!(!stored.contains("sk-"));
        assert!(
            decrypt(&KEY, "bot_a", "API_KEY", encoded)
                .unwrap_err()
                .to_string()
                .contains("prefix")
        );
    }

    #[test]
    fn a_ciphertext_does_not_move_to_another_bot_name_or_key() {
        let stored = encrypt(&KEY, "bot_a", "API_KEY", "sk-123").unwrap();
        assert!(decrypt(&KEY, "bot_b", "API_KEY", &stored).is_err());
        assert!(decrypt(&KEY, "bot_a", "OTHER_KEY", &stored).is_err());
        assert!(decrypt(&[8; 32], "bot_a", "API_KEY", &stored).is_err());
    }

    #[test]
    fn errors_never_carry_the_value() {
        let stored = encrypt(&KEY, "bot_a", "API_KEY", "sk-secret-value").unwrap();
        let error = decrypt(&KEY, "bot_b", "API_KEY", &stored).unwrap_err();
        assert!(!error.to_string().contains("sk-secret-value"));
        assert!(decrypt(&KEY, "bot_a", "API_KEY", "v1:not base64!").is_err());
        assert!(decrypt(&KEY, "bot_a", "API_KEY", "v1:AAAA").is_err());
    }
}
