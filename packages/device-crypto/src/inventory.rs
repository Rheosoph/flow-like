use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use flow_like_device_protocol::{EncryptedInventory, InventoryBinding, MAX_INVENTORY_PLAINTEXT};
use openmls_rust_crypto::RustCrypto;
use openmls_traits::{crypto::OpenMlsCrypto, types::HashType};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

const DOMAIN: &[u8] = b"flow-like/device-inventory/v1";

fn key(seed: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>> {
    let crypto = RustCrypto::default();
    let extracted = crypto
        .hkdf_extract(HashType::Sha2_256, DOMAIN, seed)
        .map_err(|_| anyhow::anyhow!("Inventory key derivation failed"))?;
    let expanded = crypto
        .hkdf_expand(HashType::Sha2_256, extracted.as_slice(), DOMAIN, 32)
        .map_err(|_| anyhow::anyhow!("Inventory key derivation failed"))?;
    Ok(Zeroizing::new(expanded.as_slice().to_vec()))
}

fn aad(binding: &InventoryBinding) -> Result<Vec<u8>> {
    binding.validate()?;
    Ok(serde_json::to_vec(&(
        "flow-like/device-inventory/v1",
        binding,
    ))?)
}

pub(crate) fn seal(
    seed: &[u8; 32],
    binding: InventoryBinding,
    plaintext: &[u8],
) -> Result<EncryptedInventory> {
    ensure!(
        plaintext.len() <= MAX_INVENTORY_PLAINTEXT,
        "Inventory exceeds its retention limit"
    );
    let associated = aad(&binding)?;
    let key = key(seed)?;
    let mut nonce = [0; 24];
    OsRng.fill_bytes(&mut nonce);
    let mut ciphertext = nonce.to_vec();
    ciphertext.extend(
        XChaCha20Poly1305::new_from_slice(&key)?
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &associated,
                },
            )
            .map_err(|_| anyhow::anyhow!("Inventory encryption failed"))?,
    );
    Ok(EncryptedInventory {
        binding,
        ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
    })
}

pub(crate) fn open(
    seed: &[u8; 32],
    expected: &InventoryBinding,
    encrypted: &EncryptedInventory,
) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        &encrypted.binding == expected,
        "Inventory belongs to another account, device, scope or revision"
    );
    let associated = aad(expected)?;
    ensure!(
        encrypted.ciphertext.len() <= (MAX_INVENTORY_PLAINTEXT + 40).div_ceil(3) * 4,
        "Inventory exceeds its retention limit"
    );
    let ciphertext = URL_SAFE_NO_PAD.decode(&encrypted.ciphertext)?;
    ensure!(
        (40..=MAX_INVENTORY_PLAINTEXT + 40).contains(&ciphertext.len()),
        "Invalid inventory envelope"
    );
    let key = key(seed)?;
    Ok(Zeroizing::new(
        XChaCha20Poly1305::new_from_slice(&key)?
            .decrypt(
                XNonce::from_slice(&ciphertext[..24]),
                Payload {
                    msg: &ciphertext[24..],
                    aad: &associated,
                },
            )
            .map_err(|_| anyhow::anyhow!("Inventory authentication failed"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::ManagementScope;
    #[test]
    fn ciphertext_is_bound_to_every_routing_field_and_revision() {
        let binding = InventoryBinding {
            issuer: "issuer".into(),
            api_origin: "https://hub.example".into(),
            account_id: "auth0|u".into(),
            device_id: "device".into(),
            controller_key: flow_like_device_protocol::SigningKey::from_bytes(&[1; 32])
                .public_key(),
            scope: ManagementScope::Project {
                project_id: "project".into(),
            },
            revision: 4,
        };
        let encrypted = seal(&[7; 32], binding.clone(), b"private observations").unwrap();
        assert_eq!(
            &**open(&[7; 32], &binding, &encrypted).unwrap(),
            b"private observations"
        );
        assert!(open(&[8; 32], &binding, &encrypted).is_err());
        for i in 0..6 {
            let mut changed = binding.clone();
            match i {
                0 => changed.account_id.push('x'),
                1 => changed.issuer.push('x'),
                2 => changed.api_origin.push('x'),
                3 => changed.device_id.push('x'),
                4 => changed.scope = ManagementScope::Device,
                _ => changed.revision += 1,
            }
            let mut tampered = encrypted.clone();
            tampered.binding = changed.clone();
            assert!(open(&[7; 32], &changed, &tampered).is_err());
        }
    }
}
