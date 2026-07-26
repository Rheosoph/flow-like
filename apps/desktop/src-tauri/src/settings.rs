use crate::profile::UserProfile;
use flow_like::{state::FlowLikeConfig, utils::cache::get_cache_dir};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::SystemTime};
use tauri::AppHandle;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct LogRetentionSettings {
    pub enabled: bool,
    pub days: u32,
    pub last_cleanup_ms: Option<u64>,
}

impl Default for LogRetentionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            days: 30,
            last_cleanup_ms: None,
        }
    }
}

// Mobile-only centralized, sandbox-safe roots (iOS + Android).
#[cfg(target_os = "ios")]
fn app_data_root() -> PathBuf {
    if let Some(dir) = dirs_next::data_dir() {
        dir.join("flow-like")
    } else if let Some(dir) = dirs_next::cache_dir() {
        dir.join("flow-like")
    } else {
        PathBuf::from("flow-like")
    }
}

// On Android, Tauri sets HOME to the app's writable `filesDir`.
// dirs_next derives XDG paths like $HOME/.local/share which are non-standard on Android
// and may fail to create. Use HOME directly as the sandbox root.
#[cfg(target_os = "android")]
fn app_data_root() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join("flow-like")
    } else if let Some(dir) = dirs_next::data_dir() {
        dir.join("flow-like")
    } else {
        PathBuf::from("flow-like")
    }
}

#[cfg(target_os = "ios")]
fn app_cache_root() -> PathBuf {
    if let Some(dir) = dirs_next::cache_dir() {
        dir.join("flow-like")
    } else if let Some(dir) = dirs_next::data_dir() {
        dir.join("flow-like").join("cache")
    } else {
        PathBuf::from("flow-like").join("cache")
    }
}

#[cfg(target_os = "android")]
fn app_cache_root() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".cache").join("flow-like")
    } else if let Some(dir) = dirs_next::cache_dir() {
        dir.join("flow-like")
    } else {
        PathBuf::from("flow-like").join("cache")
    }
}

// Single source of truth for mobile storage root. All app data is placed under this.
#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) fn mobile_storage_root() -> PathBuf {
    app_data_root()
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn default_logs_dir() -> PathBuf {
    mobile_storage_root().join("logs")
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn default_logs_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_default()
        .join("flow-like")
        .join("logs")
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn default_temporary_dir() -> PathBuf {
    mobile_storage_root().join("tmp")
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn default_temporary_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_default()
        .join("flow-like")
        .join("tmp")
}

// Keep target-specific storage selection in a helper so non-mobile builds do not
// need conditionally mutable locals that trigger `unused_mut`.
#[cfg(any(target_os = "ios", target_os = "android"))]
fn default_storage_dirs() -> (PathBuf, PathBuf, PathBuf) {
    let root = mobile_storage_root();
    (root.join("bits"), root.join("projects"), root)
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn default_storage_dirs() -> (PathBuf, PathBuf, PathBuf) {
    (
        dirs_next::data_dir()
            .unwrap_or_default()
            .join("flow-like")
            .join("bits"),
        dirs_next::data_dir()
            .unwrap_or_default()
            .join("flow-like")
            .join("projects"),
        dirs_next::cache_dir().unwrap_or_default().join("flow-like"),
    )
}

fn ensure_dir(p: &PathBuf) -> std::io::Result<()> {
    if !p.exists() {
        std::fs::create_dir_all(p)?;
    }
    Ok(())
}

#[cfg(any(target_os = "ios", target_os = "android"))]
pub fn ensure_app_dirs() -> std::io::Result<()> {
    let root = mobile_storage_root();
    let bit_dir = root.join("bits");
    let project_dir = root.join("projects");
    let cache_dir = root.clone();

    ensure_dir(&bit_dir)?;
    ensure_dir(&project_dir)?;
    ensure_dir(&cache_dir)?;
    ensure_dir(&default_logs_dir())?;
    ensure_dir(&default_temporary_dir())?;
    Ok(())
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn ensure_app_dirs() -> std::io::Result<()> {
    let bit_dir = dirs_next::data_dir()
        .ok_or_else(|| std::io::Error::other("data_dir() is None"))?
        .join("flow-like/bits");
    let project_dir = dirs_next::data_dir().unwrap().join("flow-like/projects");
    let cache_dir = dirs_next::cache_dir()
        .ok_or_else(|| std::io::Error::other("cache_dir() is None"))?
        .join("flow-like");

    ensure_dir(&bit_dir)?;
    ensure_dir(&project_dir)?;
    ensure_dir(&cache_dir)?;
    Ok(())
}

fn resolve_default_hub() -> String {
    if let Ok(url) = std::env::var("FLOW_LIKE_API_URL") {
        return url;
    }

    let config_domain = option_env!("FLOW_LIKE_CONFIG_DOMAIN");
    let config_secure = option_env!("FLOW_LIKE_CONFIG_SECURE");

    if let Some(domain) = config_domain {
        let secure = config_secure.map(|s| s == "true").unwrap_or(true);
        let protocol = if secure { "https" } else { "http" };
        return format!("{}://{}", protocol, domain);
    }

    String::from("https://api.flow-like.com")
}

#[derive(Serialize, Deserialize)]
pub struct Settings {
    loaded: bool,
    pub default_hub: String,
    pub dev_mode: bool,
    pub current_profile: String,
    pub bit_dir: PathBuf,
    pub project_dir: PathBuf,
    #[serde(default = "default_logs_dir")]
    pub logs_dir: PathBuf,
    #[serde(default = "default_temporary_dir")]
    pub temporary_dir: PathBuf,
    pub user_dir: PathBuf,
    #[serde(default)]
    pub log_retention: LogRetentionSettings,
    pub profiles: HashMap<String, UserProfile>,
    /// User-wide library of custom model bits. Definitions (and their
    /// credentials) are configured once here; each profile activates the ones
    /// it wants through its own `bits` list, exactly like public bits.
    #[serde(default)]
    pub custom_bits: Vec<flow_like::bit::Bit>,
    pub updated: SystemTime,
    pub created: SystemTime,

    #[serde(skip)]
    config: Option<Arc<FlowLikeConfig>>,
}

impl Settings {
    pub fn new() -> Self {
        // Prefer new stable settings path; fallback to legacy cache path for one-time backward compatibility.
        let new_settings_path = settings_store_path();
        let legacy_settings_path = get_cache_dir().join("global-settings.json");

        if new_settings_path.exists() || legacy_settings_path.exists() {
            let path = if new_settings_path.exists() {
                &new_settings_path
            } else {
                &legacy_settings_path
            };
            let settings = std::fs::read(path);
            if let Ok(settings) = settings {
                let settings = serde_json::from_slice::<Settings>(&settings);
                if let Ok(mut settings) = settings {
                    settings.loaded = false;
                    // Normalize platform paths (on iOS: always derive from current container roots)
                    settings.normalize_platform_paths();
                    // Make sure required directories exist after normalization.
                    let _ = ensure_app_dirs();
                    let _ = ensure_dir(&settings.logs_dir);
                    let _ = ensure_dir(&settings.temporary_dir);
                    // Persist any normalization so subsequent boots are clean.
                    Settings::serialize(&mut settings);
                    println!("Loaded settings from: {:?}", path);
                    return settings;
                }

                println!(
                    "Failed to load settings from cache, {}",
                    settings.err().unwrap()
                );
            }
        }

        ensure_app_dirs().ok();

        let (bit_dir, project_dir, user_dir) = default_storage_dirs();

        println!(
            "Settings::new() bit_dir={:?} project_dir={:?} user_dir={:?}",
            bit_dir, project_dir, user_dir
        );

        Self {
            loaded: false,
            dev_mode: false,
            default_hub: resolve_default_hub(),
            current_profile: String::from("default"),
            bit_dir,
            project_dir,
            logs_dir: default_logs_dir(),
            temporary_dir: default_temporary_dir(),
            user_dir,
            log_retention: LogRetentionSettings::default(),
            profiles: HashMap::new(),
            custom_bits: Vec::new(),
            created: SystemTime::now(),
            updated: SystemTime::now(),
            config: None,
        }
    }

    pub fn set_config(&mut self, config: &FlowLikeConfig) {
        self.config = Some(Arc::new(config.clone()));
    }

    pub fn get_current_profile(&self) -> anyhow::Result<UserProfile> {
        let mut profile = self
            .profiles
            .get(&self.current_profile)
            .or_else(|| self.profiles.values().next())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No profiles found"))?;

        // Resolve the custom bits this profile activated out of the user-wide
        // library, so model lookups see exactly the profile's line-up.
        profile.hub_profile.custom_bits = self.profile_custom_bits(&profile);

        // Proxied model calls resolve their endpoint from a process-wide base
        // URL. The desktop only learns it from the active profile, and the
        // profile can change at runtime.
        flow_like::flow_like_model_provider::embedding::proxy_config::set_api_base_url(
            &profile.hub_profile.hub,
        );

        Ok(profile)
    }

    /// The subset of the custom-bit library a profile has activated, matched
    /// against its `bits` references (`hub:id` or bare id).
    pub fn profile_custom_bits(
        &self,
        profile: &UserProfile,
    ) -> Vec<flow_like::profile::ProfileCustomBit> {
        let wanted: std::collections::HashSet<&str> = profile
            .hub_profile
            .bits
            .iter()
            .map(|reference| {
                reference
                    .rsplit_once(':')
                    .map_or(reference.as_str(), |(_, id)| id)
            })
            .collect();

        self.custom_bits
            .iter()
            .filter(|bit| wanted.contains(bit.id.as_str()))
            .cloned()
            .map(flow_like::profile::ProfileCustomBit)
            .collect()
    }

    pub async fn set_current_profile(
        &mut self,
        profile: &UserProfile,
        _app_handle: &AppHandle,
    ) -> anyhow::Result<UserProfile> {
        let profile = self
            .profiles
            .get(&profile.hub_profile.id)
            .cloned()
            .ok_or(anyhow::anyhow!("Profile not found"))?;

        self.current_profile = profile.hub_profile.id.clone();
        self.serialize();

        Ok(profile)
    }

    pub fn serialize(&mut self) {
        let dir = settings_store_path();
        if let Some(parent) = dir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let settings = serde_json::to_vec(&self);
        if let Ok(settings) = settings {
            let _res = std::fs::write(dir, settings);
        }
    }
}

impl Drop for Settings {
    fn drop(&mut self) {
        self.serialize();
    }
}

impl Settings {
    fn normalize_platform_paths(&mut self) {
        #[cfg(any(target_os = "ios", target_os = "android"))]
        {
            let root = mobile_storage_root();
            let new_bit = root.join("bits");
            let new_project = root.join("projects");
            let new_user = root.clone();
            let new_logs = default_logs_dir();
            let new_tmp = default_temporary_dir();
            // Always rebase to the current container's data root on mobile.
            self.bit_dir = new_bit;
            self.project_dir = new_project;
            self.user_dir = new_user;
            self.logs_dir = new_logs;
            self.temporary_dir = new_tmp;
        }
    }
}

// Compute the path to persist global settings. On mobile, prefer data_dir for durability.
fn settings_store_path() -> std::path::PathBuf {
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        return mobile_storage_root().join("global-settings.json");
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        get_cache_dir().join("global-settings.json")
    }
}
