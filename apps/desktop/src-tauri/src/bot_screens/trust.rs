use flow_like_types::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TrustedKey {
    pub key_id: String,
    pub public_key: String,
    pub label: String,
    pub paired_at: i64,
}

type Entries = BTreeMap<String, Vec<TrustedKey>>;

/// The viewer keys this computer pinned by QR pairing, per signed-in account.
pub(super) struct TrustStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl TrustStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn read(&self) -> Entries {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Entries::new(),
            Err(error) => {
                tracing::warn!(%error, path = %self.path.display(), "Bot screens: the trust file is unreadable, treating it as empty");
                return Entries::new();
            }
        };
        serde_json::from_str(&text).unwrap_or_else(|error| {
            tracing::warn!(%error, path = %self.path.display(), "Bot screens: the trust file is corrupt, treating it as empty");
            Entries::new()
        })
    }

    fn write(&self, entries: &Entries) -> Result<()> {
        let temp = self.path.with_extension("json.tmp");
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|error| anyhow!("Create {}: {error}", dir.display()))?;
        }
        std::fs::write(&temp, serde_json::to_vec_pretty(entries)?)
            .map_err(|error| anyhow!("Write {}: {error}", temp.display()))?;
        std::fs::rename(&temp, &self.path)
            .map_err(|error| anyhow!("Replace {}: {error}", self.path.display()))
    }

    fn modify(&self, change: impl FnOnce(&mut Entries) -> bool) -> Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.read();
        if change(&mut entries) {
            self.write(&entries)?;
        }
        Ok(())
    }

    pub fn list(&self, sub: &str) -> Vec<TrustedKey> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.read().remove(sub).unwrap_or_default()
    }

    pub fn get(&self, sub: &str, key_id: &str) -> Option<TrustedKey> {
        self.list(sub).into_iter().find(|key| key.key_id == key_id)
    }

    pub fn pin(&self, sub: &str, key: TrustedKey) -> Result<()> {
        self.modify(|entries| {
            let keys = entries.entry(sub.to_owned()).or_default();
            keys.retain(|known| known.key_id != key.key_id);
            keys.push(key);
            true
        })
    }

    pub fn unpin(&self, sub: &str, key_id: &str) -> Result<()> {
        self.modify(|entries| {
            let Some(keys) = entries.get_mut(sub) else {
                return false;
            };
            let before = keys.len();
            keys.retain(|known| known.key_id != key_id);
            let changed = keys.len() != before;
            if keys.is_empty() {
                entries.remove(sub);
            }
            changed
        })
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub struct TempFile(pub PathBuf);

    impl TempFile {
        pub fn new() -> Self {
            let id = uuid::Uuid::new_v4().simple();
            Self(std::env::temp_dir().join(format!("bot-screen-trust-{id}.json")))
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("json.tmp"));
        }
    }

    pub fn key(id: &str) -> TrustedKey {
        TrustedKey {
            key_id: id.to_owned(),
            public_key: format!("pk-{id}"),
            label: format!("Phone {id}"),
            paired_at: 1_763_337_600,
        }
    }

    #[test]
    fn pins_survive_a_reload_and_are_kept_per_sub() {
        let file = TempFile::new();
        let store = TrustStore::new(file.0.clone());
        assert!(store.list("a").is_empty());
        store.pin("a", key("k1")).unwrap();
        store.pin("b", key("k2")).unwrap();
        let reloaded = TrustStore::new(file.0.clone());
        assert_eq!(reloaded.list("a"), vec![key("k1")]);
        assert!(reloaded.get("a", "k2").is_none());
        assert_eq!(reloaded.get("b", "k2"), Some(key("k2")));
    }

    #[test]
    fn pinning_again_replaces_and_unpin_removes() {
        let file = TempFile::new();
        let store = TrustStore::new(file.0.clone());
        store.pin("a", key("k1")).unwrap();
        let mut renamed = key("k1");
        renamed.label = "Tablet".into();
        store.pin("a", renamed.clone()).unwrap();
        assert_eq!(store.list("a"), vec![renamed]);
        store.unpin("a", "k1").unwrap();
        store.unpin("a", "k1").unwrap();
        assert!(store.list("a").is_empty());
    }

    #[test]
    fn a_corrupt_file_reads_as_empty_and_is_replaced_on_the_next_pin() {
        let file = TempFile::new();
        std::fs::write(&file.0, b"{ not json").unwrap();
        let store = TrustStore::new(file.0.clone());
        assert!(store.list("a").is_empty());
        store.pin("a", key("k1")).unwrap();
        assert_eq!(store.list("a"), vec![key("k1")]);
    }
}
