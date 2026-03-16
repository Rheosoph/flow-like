use std::path::PathBuf;
use tokio::fs;
use tokio::sync::RwLock;

const HMAC_KEY_FILE: &str = ".hmac_key";
const HMAC_KEY_LEN: usize = 32;

pub struct BlobStore {
    base_dir: RwLock<PathBuf>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct BlobRef {
    pub hash: String,
    pub mac: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct BlobEntry {
    pub key: String,
    pub data: Vec<u8>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct BlobRefEntry {
    pub key: String,
    pub blob_ref: BlobRef,
}

impl BlobStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir: RwLock::new(base_dir),
        }
    }

    pub async fn set_base_dir(&self, new_dir: PathBuf) {
        *self.base_dir.write().await = new_dir;
    }

    pub async fn get_base_dir(&self) -> PathBuf {
        self.base_dir.read().await.clone()
    }

    async fn blob_path(&self, hash: &str) -> PathBuf {
        let base = self.base_dir.read().await;
        let prefix = &hash[..2.min(hash.len())];
        base.join(prefix).join(hash)
    }

    pub async fn ensure_hmac_key(&self) -> Result<[u8; HMAC_KEY_LEN], String> {
        let base = self.base_dir.read().await;
        let key_path = base.join(HMAC_KEY_FILE);
        if key_path.exists() {
            let bytes = fs::read(&key_path)
                .await
                .map_err(|e| format!("Failed to read HMAC key: {e}"))?;
            if bytes.len() == HMAC_KEY_LEN {
                let mut key = [0u8; HMAC_KEY_LEN];
                key.copy_from_slice(&bytes);
                return Ok(key);
            }
        }
        let key: [u8; HMAC_KEY_LEN] = rand::random();
        fs::create_dir_all(&*base)
            .await
            .map_err(|e| format!("Failed to create blob dir: {e}"))?;
        fs::write(&key_path, &key)
            .await
            .map_err(|e| format!("Failed to write HMAC key: {e}"))?;
        Ok(key)
    }

    pub fn compute_hash(data: &[u8]) -> String {
        blake3::hash(data).to_hex().to_string()
    }

    pub fn compute_mac(key: &[u8; HMAC_KEY_LEN], hash: &str) -> String {
        blake3::keyed_hash(key, hash.as_bytes())
            .to_hex()
            .to_string()
    }

    pub fn verify_mac(key: &[u8; HMAC_KEY_LEN], hash: &str, mac: &str) -> bool {
        let expected = Self::compute_mac(key, hash);
        constant_time_eq(expected.as_bytes(), mac.as_bytes())
    }

    pub async fn store(&self, data: &[u8]) -> Result<BlobRef, String> {
        let key = self.ensure_hmac_key().await?;
        let hash = Self::compute_hash(data);
        let mac = Self::compute_mac(&key, &hash);
        let path = self.blob_path(&hash).await;

        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("Failed to create dir: {e}"))?;
            }
            fs::write(&path, data)
                .await
                .map_err(|e| format!("Failed to write blob: {e}"))?;
        }

        Ok(BlobRef { hash, mac })
    }

    pub async fn get(&self, hash: &str, mac: &str) -> Result<Vec<u8>, String> {
        let key = self.ensure_hmac_key().await?;
        if !Self::verify_mac(&key, hash, mac) {
            return Err("Invalid blob reference".into());
        }
        let path = self.blob_path(hash).await;
        fs::read(&path)
            .await
            .map_err(|e| format!("Blob not found: {e}"))
    }

    pub async fn delete(&self, hash: &str, mac: &str) -> Result<(), String> {
        let key = self.ensure_hmac_key().await?;
        if !Self::verify_mac(&key, hash, mac) {
            return Err("Invalid blob reference".into());
        }
        let path = self.blob_path(hash).await;
        if path.exists() {
            fs::remove_file(&path)
                .await
                .map_err(|e| format!("Failed to delete blob: {e}"))?;
        }
        Ok(())
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store() -> (TempDir, BlobStore) {
        let tmp = TempDir::new().unwrap();
        let store = BlobStore::new(tmp.path().to_path_buf());
        (tmp, store)
    }

    #[tokio::test]
    async fn store_and_retrieve() {
        let (_tmp, store) = make_store();
        let data = b"hello world";
        let blob_ref = store.store(data).await.unwrap();
        let retrieved = store.get(&blob_ref.hash, &blob_ref.mac).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn store_is_content_addressed() {
        let (_tmp, store) = make_store();
        let data = b"duplicate content";
        let ref1 = store.store(data).await.unwrap();
        let ref2 = store.store(data).await.unwrap();
        assert_eq!(ref1.hash, ref2.hash);
        assert_eq!(ref1.mac, ref2.mac);
    }

    #[tokio::test]
    async fn get_with_wrong_mac_fails() {
        let (_tmp, store) = make_store();
        let data = b"secret data";
        let blob_ref = store.store(data).await.unwrap();
        let result = store.get(&blob_ref.hash, "bogus_mac").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid blob reference"));
    }

    #[tokio::test]
    async fn delete_removes_blob() {
        let (_tmp, store) = make_store();
        let data = b"to be deleted";
        let blob_ref = store.store(data).await.unwrap();
        store.delete(&blob_ref.hash, &blob_ref.mac).await.unwrap();
        let result = store.get(&blob_ref.hash, &blob_ref.mac).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_with_wrong_mac_fails() {
        let (_tmp, store) = make_store();
        let data = b"protected data";
        let blob_ref = store.store(data).await.unwrap();
        let result = store.delete(&blob_ref.hash, "wrong_mac").await;
        assert!(result.is_err());
        // Blob should still exist
        let retrieved = store.get(&blob_ref.hash, &blob_ref.mac).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn set_base_dir_switches_storage() {
        let (tmp1, store) = make_store();
        let data = b"stored in dir1";
        let ref1 = store.store(data).await.unwrap();

        let tmp2 = TempDir::new().unwrap();
        store.set_base_dir(tmp2.path().to_path_buf()).await;

        // Old ref should fail (blob not in new dir, and HMAC key differs)
        let result = store.get(&ref1.hash, &ref1.mac).await;
        assert!(result.is_err());

        // New store should work independently
        let data2 = b"stored in dir2";
        let ref2 = store.store(data2).await.unwrap();
        let retrieved = store.get(&ref2.hash, &ref2.mac).await.unwrap();
        assert_eq!(retrieved, data2);

        drop(tmp1);
    }

    #[tokio::test]
    async fn hmac_key_persists_across_instances() {
        let tmp = TempDir::new().unwrap();
        let data = b"persistent key test";

        let blob_ref = {
            let store = BlobStore::new(tmp.path().to_path_buf());
            store.store(data).await.unwrap()
        };

        // New store instance, same dir — should read the same HMAC key
        let store2 = BlobStore::new(tmp.path().to_path_buf());
        let retrieved = store2.get(&blob_ref.hash, &blob_ref.mac).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn large_binary_data() {
        let (_tmp, store) = make_store();
        let data: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
        let blob_ref = store.store(&data).await.unwrap();
        let retrieved = store.get(&blob_ref.hash, &blob_ref.mac).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn compute_hash_is_deterministic() {
        let h1 = BlobStore::compute_hash(b"test data");
        let h2 = BlobStore::compute_hash(b"test data");
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_hash_differs_for_different_data() {
        let h1 = BlobStore::compute_hash(b"data a");
        let h2 = BlobStore::compute_hash(b"data b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn verify_mac_rejects_wrong_mac() {
        let key: [u8; HMAC_KEY_LEN] = [42u8; HMAC_KEY_LEN];
        let hash = "somehash";
        let mac = BlobStore::compute_mac(&key, hash);
        assert!(BlobStore::verify_mac(&key, hash, &mac));
        assert!(!BlobStore::verify_mac(&key, hash, "wrong"));
    }

    #[test]
    fn verify_mac_rejects_different_key() {
        let key1: [u8; HMAC_KEY_LEN] = [1u8; HMAC_KEY_LEN];
        let key2: [u8; HMAC_KEY_LEN] = [2u8; HMAC_KEY_LEN];
        let hash = "somehash";
        let mac = BlobStore::compute_mac(&key1, hash);
        assert!(!BlobStore::verify_mac(&key2, hash, &mac));
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(super::constant_time_eq(b"abc", b"abc"));
        assert!(!super::constant_time_eq(b"abc", b"abd"));
        assert!(!super::constant_time_eq(b"abc", b"ab"));
        assert!(super::constant_time_eq(b"", b""));
    }
}
