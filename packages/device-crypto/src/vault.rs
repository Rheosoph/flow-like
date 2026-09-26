use anyhow::{Result, ensure};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"FLVAULT1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = MAGIC.len() + SALT_LEN + NONCE_LEN;
const CONTROLLER_CONTEXT_PREFIX: &[u8] = b"flow-like/standalone/controller-vault/v1\0";
const INVITATION_CONTEXT_PREFIX: &[u8] = b"flow-like/standalone/invitation-vault/v1\0";

/// Bind routine management key storage to its device and purpose.
pub fn controller_context(device_id: &str) -> Vec<u8> {
    [CONTROLLER_CONTEXT_PREFIX, device_id.as_bytes()].concat()
}

/// Invitation authority is unlocked separately from routine management keys.
pub fn invitation_context(device_id: &str) -> Vec<u8> {
    [INVITATION_CONTEXT_PREFIX, device_id.as_bytes()].concat()
}

/// Fixed parameters bound memory use when opening an untrusted vault file.
fn password_key(password: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    ensure!(password.len() <= 4096, "Password exceeds the size limit");
    let params = Params::new(65_536, 3, 1, Some(32))
        .map_err(|_| anyhow::anyhow!("Invalid vault KDF parameters"))?;
    let mut key = Zeroizing::new([0; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, key.as_mut())
        .map_err(|_| anyhow::anyhow!("Could not derive vault key"))?;
    Ok(key)
}

/// Operator secrets stay on the operator's machine, outside the deployment package.
pub fn seal(password: &[u8], context: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    ensure!(password.len() >= 12, "Use a password of at least 12 bytes");
    ensure!(
        plaintext.len() <= 65_536,
        "Vault payload exceeds the size limit"
    );
    let mut header = vec![0; HEADER_LEN];
    header[..MAGIC.len()].copy_from_slice(MAGIC);
    OsRng.fill_bytes(&mut header[MAGIC.len()..]);
    let key = password_key(password, &header[MAGIC.len()..MAGIC.len() + SALT_LEN])?;
    let mut aad = header.clone();
    aad.extend_from_slice(context);
    let ciphertext = XChaCha20Poly1305::new((&*key).into())
        .encrypt(
            XNonce::from_slice(&header[MAGIC.len() + SALT_LEN..]),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("Could not encrypt vault"))?;
    header.extend_from_slice(&ciphertext);
    Ok(header)
}

pub fn open(password: &[u8], context: &[u8], ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        (HEADER_LEN + 16..=HEADER_LEN + 65_536 + 16).contains(&ciphertext.len())
            && ciphertext.starts_with(MAGIC),
        "Unsupported or malformed vault"
    );
    let header = &ciphertext[..HEADER_LEN];
    let key = password_key(password, &header[MAGIC.len()..MAGIC.len() + SALT_LEN])?;
    let mut aad = header.to_vec();
    aad.extend_from_slice(context);
    let plaintext = XChaCha20Poly1305::new((&*key).into())
        .decrypt(
            XNonce::from_slice(&header[MAGIC.len() + SALT_LEN..]),
            Payload {
                msg: &ciphertext[HEADER_LEN..],
                aad: &aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("Incorrect password or damaged vault"))?;
    Ok(Zeroizing::new(plaintext))
}
