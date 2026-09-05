use std::collections::HashMap;

use flow_like_types::reqwest;
use serde::{Deserialize, Serialize};

use crate::bit::Bit;

use super::cache::get_cache_dir;

const DOWNLOAD_MANAGER_FILE: &str = "download-manager.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct Download {
    pub url: String,
    pub file_name: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DownloadManager {
    pub download_list: HashMap<String, Bit>,
    resume: bool,
    #[serde(skip)]
    client: reqwest::Client,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadManager {
    fn artifact_key(bit: &Bit) -> String {
        let file_name = bit.file_name.as_deref().unwrap_or_default();
        format!(
            "v2:{}:{}:{}{}",
            bit.hash.len(),
            file_name.len(),
            bit.hash,
            file_name
        )
    }

    pub fn new() -> Self {
        DownloadManager {
            download_list: HashMap::new(),
            resume: false,
            client: reqwest::Client::new(),
        }
    }

    pub fn load(&mut self) -> HashMap<String, Bit> {
        let dir = get_cache_dir();
        let dir = dir.join(DOWNLOAD_MANAGER_FILE);
        if !dir.exists() {
            return HashMap::new();
        }

        let dl_manager = match std::fs::read_to_string(dir) {
            Ok(data) => match flow_like_types::json::from_str::<DownloadManager>(&data) {
                Ok(dl_manager) => dl_manager,
                Err(e) => {
                    println!("Error loading download manager: {:?}", e);
                    return HashMap::new();
                }
            },
            Err(e) => {
                println!("Error loading download manager: {:?}", e);
                return HashMap::new();
            }
        };

        dl_manager.download_list
    }

    pub fn block_resume(&mut self) {
        self.resume = true;
    }

    pub fn resumed(&self) -> bool {
        self.resume
    }

    pub fn add_download(&mut self, bit: &Bit) -> Option<reqwest::Client> {
        let key = Self::artifact_key(bit);
        if self.download_list.contains_key(&key) {
            return None;
        }
        self.download_list.insert(key, bit.clone());
        self.save();
        Some(self.client.clone())
    }

    pub fn download_exists(&self, bit: &Bit) -> bool {
        self.download_list.contains_key(&Self::artifact_key(bit))
    }

    pub fn remove_download(&mut self, bit: &Bit) {
        self.download_list.remove(&Self::artifact_key(bit));
        self.save();
    }

    pub fn save(&self) {
        let dir = get_cache_dir().join(DOWNLOAD_MANAGER_FILE);
        let data = match flow_like_types::json::to_string(self) {
            Ok(v) => v,
            Err(e) => {
                println!("Error serializing download manager: {:?}", e);
                return;
            }
        };
        // Offload blocking filesystem write to a dedicated thread to avoid stalling the async runtime.
        drop(flow_like_types::tokio::task::spawn_blocking(move || {
            if let Some(parent) = dir.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&dir, data) {
                println!("Error saving download manager: {:?}", e);
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_content_at_different_paths_has_distinct_download_keys() {
        let first = Bit {
            hash: "same-hash".to_string(),
            file_name: Some("config.json".to_string()),
            ..Bit::default()
        };
        let second = Bit {
            hash: first.hash.clone(),
            file_name: Some("nested/config.json".to_string()),
            ..Bit::default()
        };
        assert_ne!(
            DownloadManager::artifact_key(&first),
            DownloadManager::artifact_key(&second)
        );
    }
}
