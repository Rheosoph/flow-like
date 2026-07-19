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
        if self.download_list.contains_key(&bit.hash) {
            return None;
        }
        self.download_list.insert(bit.hash.to_string(), bit.clone());
        self.save();
        Some(self.client.clone())
    }

    pub fn download_exists(&self, bit: &Bit) -> bool {
        self.download_list.contains_key(&bit.hash)
    }

    pub fn remove_download(&mut self, bit: &Bit) {
        self.download_list.remove(&bit.hash);
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
