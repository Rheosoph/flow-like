//! Local bot secrets: XChaCha20-Poly1305 under the device key, a fresh random nonce per value,
//! stored as `nonce || ciphertext`. The associated data `bot_id \0 name` binds a ciphertext to
//! its bot and name, so it can't be moved to another row.

use crate::error::{Error, Result, codes};
use chacha20poly1305::{
    Key, XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
};
use zeroize::{Zeroize, Zeroizing};

const NONCE_LEN: usize = 24;

fn associated_data(bot_id: &str, name: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(bot_id.len() + 1 + name.len());
    aad.extend_from_slice(bot_id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(name.as_bytes());
    aad
}

fn unreadable(bot_id: &str, name: &str) -> Error {
    Error::new(
        codes::SECRET_UNREADABLE,
        format!("Secret '{name}' of bot '{bot_id}' does not open with this device's key"),
    )
}

pub(crate) fn seal(key: &[u8; 32], bot_id: &str, name: &str, value: &str) -> Result<Vec<u8>> {
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = XChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(
            &nonce,
            Payload {
                msg: value.as_bytes(),
                aad: &associated_data(bot_id, name),
            },
        )
        .map_err(|_| {
            Error::new(
                codes::SECRET_KEY,
                format!("Encrypting secret '{name}' of bot '{bot_id}' failed"),
            )
        })?;
    let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

/// The value [`seal`] stored for secret `name` of bot `bot_id`; fails for any other bot, name
/// or key.
pub(crate) fn open(
    key: &[u8; 32],
    bot_id: &str,
    name: &str,
    sealed: &[u8],
) -> Result<Zeroizing<String>> {
    if sealed.len() <= NONCE_LEN {
        return Err(unreadable(bot_id, name));
    }
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
    let plaintext = XChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: &associated_data(bot_id, name),
            },
        )
        .map_err(|_| unreadable(bot_id, name))?;
    String::from_utf8(plaintext)
        .map(Zeroizing::new)
        .map_err(|error| {
            error.into_bytes().zeroize();
            unreadable(bot_id, name)
        })
}
