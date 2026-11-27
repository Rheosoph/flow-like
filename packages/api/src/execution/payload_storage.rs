//! Encrypted payload storage for execution runs.
//!
//! Stores input payloads in object storage with AES-256-GCM encryption.
//! The encryption key is stored in the database, the encrypted data in object storage.

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use flow_like_storage::Path as StoragePath;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::object_store::{ObjectStore, PutPayload};
use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use std::sync::Arc;

/// Encryption key length (256 bits)
const KEY_LEN: usize = 32;
/// Nonce length for AES-GCM (96 bits)
const NONCE_LEN: usize = 12;

/// Error type for payload storage operations
#[derive(Debug, thiserror::Error)]
pub enum PayloadStorageError {
    #[error("Encryption error: {0}")]
    Encryption(String),
    #[error("Decryption error: {0}")]
    Decryption(String),
    #[error("Storage error: {0}")]
    Storage(#[from] flow_like_storage::object_store::Error),
    #[error("Base64 decode error: {0}")]
    Base64(#[from] flow_like_types::base64::DecodeError),
}

/// Result of storing a payload - contains the encryption key
pub struct StoredPayload {
    /// Base64-encoded encryption key (includes nonce prefix)
    pub key: String,
    /// Path where the payload was stored
    pub path: String,
}

fn payload_path(app_id: &str, run_id: &str) -> StoragePath {
    StoragePath::from(format!("runs/{}/{}/payload", app_id, run_id))
}

/// Store an encrypted payload in object storage.
///
/// Returns the base64-encoded key (which includes the nonce) to be stored in the database.
pub async fn store_payload(
    store: Arc<dyn ObjectStore>,
    app_id: &str,
    run_id: &str,
    payload: &[u8],
) -> Result<StoredPayload, PayloadStorageError> {
    // Generate random key and nonce using getrandom
    let mut key = [0u8; KEY_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut key)
        .map_err(|e| PayloadStorageError::Encryption(format!("Failed to generate key: {}", e)))?;
    getrandom::fill(&mut nonce_bytes)
        .map_err(|e| PayloadStorageError::Encryption(format!("Failed to generate nonce: {}", e)))?;

    // Create cipher and encrypt
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| PayloadStorageError::Encryption(e.to_string()))?;
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce, payload)
        .map_err(|e| PayloadStorageError::Encryption(e.to_string()))?;

    let path = payload_path(app_id, run_id);
    store.put(&path, PutPayload::from(ciphertext)).await?;

    // Combine nonce + key for storage (nonce is not secret, but convenient to store together)
    let mut key_with_nonce = Vec::with_capacity(NONCE_LEN + KEY_LEN);
    key_with_nonce.extend_from_slice(&nonce_bytes);
    key_with_nonce.extend_from_slice(&key);

    let encoded_key = BASE64.encode(&key_with_nonce);

    Ok(StoredPayload {
        key: encoded_key,
        path: path.to_string(),
    })
}

/// Retrieve and decrypt a payload from object storage.
pub async fn retrieve_payload(
    store: Arc<dyn ObjectStore>,
    app_id: &str,
    run_id: &str,
    encoded_key: &str,
) -> Result<Vec<u8>, PayloadStorageError> {
    // Decode key (nonce + key)
    let key_with_nonce = BASE64.decode(encoded_key)?;
    if key_with_nonce.len() != NONCE_LEN + KEY_LEN {
        return Err(PayloadStorageError::Decryption(
            "Invalid key length".to_string(),
        ));
    }

    let nonce_bytes = &key_with_nonce[..NONCE_LEN];
    let key = &key_with_nonce[NONCE_LEN..];

    let result = store.get(&payload_path(app_id, run_id)).await?;
    let ciphertext = result.bytes().await?;

    // Decrypt
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| PayloadStorageError::Decryption(e.to_string()))?;
    let nonce_bytes: [u8; NONCE_LEN] = nonce_bytes
        .try_into()
        .map_err(|_| PayloadStorageError::Decryption("Invalid nonce length".to_string()))?;
    let nonce = Nonce::from(nonce_bytes);
    let plaintext = cipher
        .decrypt(&nonce, ciphertext.as_ref())
        .map_err(|e| PayloadStorageError::Decryption(e.to_string()))?;

    Ok(plaintext)
}

/// Remove the payload of a run that was never recorded, whose key therefore exists nowhere.
pub async fn discard_payload(
    store: Arc<dyn ObjectStore>,
    app_id: &str,
    run_id: &str,
) -> Result<(), PayloadStorageError> {
    store.delete(&payload_path(app_id, run_id)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;

    #[tokio::test]
    async fn test_roundtrip() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let payload = b"test payload data";

        let stored = store_payload(store.clone(), "app1", "run1", payload)
            .await
            .unwrap();

        let retrieved = retrieve_payload(store, "app1", "run1", &stored.key)
            .await
            .unwrap();

        assert_eq!(payload.as_slice(), retrieved.as_slice());
    }

    #[tokio::test]
    async fn a_discarded_payload_is_gone() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let stored = store_payload(store.clone(), "app1", "run1", b"refused")
            .await
            .unwrap();

        discard_payload(store.clone(), "app1", "run1")
            .await
            .unwrap();

        assert!(matches!(
            retrieve_payload(store, "app1", "run1", &stored.key).await,
            Err(PayloadStorageError::Storage(
                flow_like_storage::object_store::Error::NotFound { .. }
            ))
        ));
    }
}
