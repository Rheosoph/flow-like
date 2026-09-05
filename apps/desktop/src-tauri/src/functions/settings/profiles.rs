use crate::{
    functions::TauriFunctionError,
    profile::UserProfile,
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::{
    bit::Bit,
    hub::Hub,
    profile::{Profile, ProfileApp, ProfileShortcut},
    utils::{cache::get_cache_dir, hash::hash_file, http::HTTPClient},
};
use flow_like_types::tokio::task::JoinHandle;
use futures::future::join_all;
use serde::Deserialize;
use std::path::PathBuf;
use std::{collections::HashMap, sync::Arc};
use tauri::{AppHandle, Url};
use tauri_plugin_dialog::DialogExt;
use tracing::instrument;
use urlencoding::encode;

fn presign_icon(icon: &str) -> Result<String, TauriFunctionError> {
    // if it already looks like a URL (has a scheme), return it as-is to avoid double-presigning
    if icon.contains("://") {
        return Ok(icon.to_string());
    }

    #[cfg(any(windows, target_os = "android"))]
    let base = "http://asset.localhost/";
    #[cfg(not(any(windows, target_os = "android")))]
    let base = "asset://localhost/";
    let urlencoded_path = encode(icon);
    let url = format!("{base}{urlencoded_path}");
    let url = Url::parse(&url).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    Ok(url.to_string())
}

fn decode_asset_proxy_path(path: &str) -> Option<String> {
    let url = Url::parse(path).ok()?;
    let host = url.host_str()?;
    let is_asset_proxy = (url.scheme() == "asset" && host == "localhost")
        || ((url.scheme() == "http" || url.scheme() == "https") && host == "asset.localhost");
    if !is_asset_proxy {
        return None;
    }

    let encoded_path = url.path().trim_start_matches('/');
    if encoded_path.is_empty() {
        return None;
    }

    let decoded_path = urlencoding::decode(encoded_path).ok()?.into_owned();
    if decoded_path.is_empty() {
        return None;
    }

    Some(decoded_path)
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn get_profiles(
    app_handle: AppHandle,
) -> Result<HashMap<String, UserProfile>, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;

    let mut profiles = {
        let settings_guard = settings.lock().await;
        settings_guard.profiles.clone()
    };

    for profile in profiles.values_mut() {
        if let Some(icon) = profile.hub_profile.icon.clone()
            && !icon.starts_with("http://")
            && !icon.starts_with("https://")
            && let Ok(icon) = presign_icon(&icon)
        {
            profile.hub_profile.icon = Some(icon);
        }
    }

    Ok(profiles)
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn get_profiles_raw(
    app_handle: AppHandle,
) -> Result<HashMap<String, UserProfile>, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let profiles = {
        let settings_guard = settings.lock().await;
        settings_guard.profiles.clone()
    };
    Ok(profiles)
}

/// Hub profiles paired with the bits (models, embedding models) each one references.
pub type ProfilesWithBits = Vec<(UserProfile, Vec<Bit>)>;

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn get_default_profiles(
    app_handle: AppHandle,
) -> Result<(ProfilesWithBits, Hub), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let default_hub = settings.lock().await.default_hub.clone();
    let http_client = TauriFlowLikeState::http_client(&app_handle).await?;
    let default_hub = Hub::new(&default_hub, http_client.clone()).await?;

    let profiles = default_hub.get_profiles().await?;
    let profiles = get_bits(profiles.clone(), http_client).await?;

    Ok((profiles, default_hub))
}

#[instrument(skip_all)]
async fn get_bits(
    profiles: Vec<Profile>,
    http_client: Arc<HTTPClient>,
) -> flow_like_types::Result<ProfilesWithBits> {
    // Collect all futures for models and embedding models
    let mut bits: HashMap<&str, &str> = HashMap::new();
    let mut hubs: HashMap<&str, Hub> = HashMap::new();

    for profile in profiles.iter() {
        for bit_id in profile.bits.iter() {
            let (hub, bit) = bit_id.split_once(':').unwrap_or(("", bit_id));
            bits.insert(bit, hub);
            if !hubs.contains_key(hub) {
                hubs.insert(hub, Hub::new(hub, http_client.clone()).await?);
            }
        }
    }

    let bit_features = bits.iter().map(|(bit_id, hub_id)| {
        let hub = hubs.get(hub_id).unwrap();
        hub.get_bit(bit_id)
    });

    let bits_results = join_all(bit_features).await;

    let bits: Vec<Bit> = bits_results
        .into_iter()
        .filter_map(|res| res.ok())
        .collect();

    let bits_map: HashMap<String, Bit> = bits
        .iter()
        .map(|bit| (bit.id.clone(), bit.clone()))
        .collect();

    let output = profiles
        .iter()
        .map(|profile| {
            let bits = profile
                .bits
                .iter()
                .map(|bit_url| {
                    let (_hub, bit) = bit_url.split_once(':').unwrap_or(("", bit_url));
                    let bit = bits_map.get(bit).unwrap();
                    bit.clone()
                })
                .collect();
            let user_profile = UserProfile::new(profile.clone());
            (user_profile, bits)
        })
        .collect();

    Ok(output)
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn get_current_profile(app_handle: AppHandle) -> Result<UserProfile, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let mut profile = TauriSettingsState::current_profile(&app_handle)
        .await?
        .clone();

    state
        .model_factory
        .lock()
        .await
        .set_execution_settings(profile.execution_settings.clone());

    if let Some(icon) = profile.hub_profile.icon.clone()
        && !icon.starts_with("http://")
        && !icon.starts_with("https://")
        && let Ok(icon) = presign_icon(&icon)
    {
        profile.hub_profile.icon = Some(icon);
    }

    Ok(profile)
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn get_bits_in_current_profile(
    app_handle: AppHandle,
) -> Result<Vec<Bit>, TauriFunctionError> {
    let profile = TauriSettingsState::current_profile(&app_handle).await?;
    let http_client = TauriFlowLikeState::http_client(&app_handle).await?;

    let mut tasks: Vec<JoinHandle<Option<Bit>>> = vec![];

    for bit_id in profile.hub_profile.bits.iter() {
        let (hub, bit) = bit_id.split_once(':').unwrap_or(("", bit_id));
        if hub.is_empty() {
            continue; // Skip bits without a hub
        }
        let hub = hub.to_string();
        let bit = bit.to_string();
        let http_client = http_client.clone();
        let task = flow_like_types::tokio::spawn(async move {
            let hub = Hub::new(&hub, http_client).await.ok()?;
            let bit = hub.get_bit(&bit).await.ok()?;
            Some(bit)
        });
        tasks.push(task);
    }

    let results = join_all(tasks).await;
    let mut found_bits: Vec<Bit> = results
        .into_iter()
        .filter_map(|res| res.ok().flatten())
        .collect();

    // Custom bits this profile activated resolve locally, not over a hub.
    found_bits.extend(
        profile
            .hub_profile
            .custom_bits
            .into_iter()
            .map(|custom| custom.0),
    );

    Ok(found_bits)
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn get_current_profile_id(app_handle: AppHandle) -> Result<String, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let settings = settings.lock().await;
    let current_profile = settings.get_current_profile()?;
    Ok(current_profile.hub_profile.id)
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn set_current_profile(
    app_handle: AppHandle,
    profile_id: String,
) -> Result<UserProfile, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;
    let profile = settings
        .profiles
        .get(&profile_id)
        .cloned()
        .ok_or(anyhow::anyhow!("Profile not found"))?;
    settings.set_current_profile(&profile, &app_handle).await?;
    settings.serialize();
    Ok(profile.clone())
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn upsert_profile(
    app_handle: AppHandle,
    profile: UserProfile,
) -> Result<UserProfile, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;

    // Custom bits are managed only through upsert_custom_bit/remove_custom_bit;
    // a client-provided profile copy must never wipe them.
    let mut profile = profile;
    if let Some(icon) = profile
        .hub_profile
        .icon
        .as_deref()
        .and_then(decode_asset_proxy_path)
    {
        profile.hub_profile.icon = Some(icon);
    }
    if let Some(existing) = settings.profiles.get(&profile.hub_profile.id) {
        profile.hub_profile.custom_bits = existing.hub_profile.custom_bits.clone();
    }

    settings
        .profiles
        .insert(profile.hub_profile.id.clone(), profile.clone());

    if settings.current_profile == profile.hub_profile.id || settings.current_profile.is_empty() {
        settings.set_current_profile(&profile, &app_handle).await?;
    };

    settings.serialize();
    Ok(profile.clone())
}

fn apply_profile_settings(
    existing: &mut UserProfile,
    changes: UserProfile,
) -> Result<(), TauriFunctionError> {
    let name = changes.hub_profile.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(TauriFunctionError::new(
            "Enter a profile name with 1 to 100 characters.",
        ));
    }
    if changes.execution_settings.max_context_size > u32::MAX as usize {
        return Err(TauriFunctionError::new(
            "The context size must fit within a 32-bit unsigned integer.",
        ));
    }
    let now = now_iso();
    existing.hub_profile.name = name.to_string();
    existing.hub_profile.description = changes.hub_profile.description;
    existing.hub_profile.interests = changes.hub_profile.interests;
    existing.hub_profile.tags = changes.hub_profile.tags;
    existing.hub_profile.theme = changes.hub_profile.theme;
    existing.hub_profile.settings = changes.hub_profile.settings;
    existing.execution_settings = changes.execution_settings;
    existing.hub_profile.updated = now.clone();
    existing.updated = now;
    Ok(())
}

/// Edit workspace preferences without overwriting concurrently updated app membership or media.
#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn update_profile_settings(
    app_handle: AppHandle,
    profile: UserProfile,
) -> Result<UserProfile, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;
    let existing = settings
        .profiles
        .get_mut(&profile.hub_profile.id)
        .ok_or_else(|| TauriFunctionError::new("This profile no longer exists."))?;
    apply_profile_settings(existing, profile)?;
    let updated = existing.clone();
    if settings.current_profile == updated.hub_profile.id {
        settings.set_current_profile(&updated, &app_handle).await?;
    }
    settings.serialize();
    Ok(updated)
}

/// Remap a profile's ID from local to server ID after sync
#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn remap_profile_id(
    app_handle: AppHandle,
    local_id: String,
    server_id: String,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;

    // Get and remove the profile with old ID
    let mut profile = settings
        .profiles
        .remove(&local_id)
        .ok_or(anyhow::anyhow!("Profile not found"))?;

    // Update the profile's ID
    profile.hub_profile.id = server_id.clone();

    // Re-insert with new ID
    settings.profiles.insert(server_id.clone(), profile);

    // Update current_profile if it was pointing to the old ID
    if settings.current_profile == local_id {
        settings.current_profile = server_id;
    }

    settings.serialize();
    Ok(())
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn delete_profile(
    app_handle: AppHandle,
    profile_id: String,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;

    if !settings.profiles.contains_key(&profile_id) {
        return Ok(());
    }

    let deletes_current_profile = settings.current_profile == profile_id
        || settings
            .get_current_profile()
            .map(|profile| profile.hub_profile.id == profile_id)
            .unwrap_or(false);

    settings.profiles.remove(&profile_id);

    if deletes_current_profile || !settings.profiles.contains_key(&settings.current_profile) {
        settings.current_profile = settings.profiles.keys().next().cloned().unwrap_or_default();
    }

    settings.serialize();
    Ok(())
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn add_bit(
    app_handle: AppHandle,
    profile: UserProfile,
    bit: Bit,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;
    let profile = settings
        .profiles
        .get_mut(&profile.hub_profile.id)
        .ok_or(anyhow::anyhow!("Profile not found"))?;
    profile.hub_profile.add_bit(&bit).await;
    let now = now_iso();
    profile.hub_profile.updated = now.clone();
    profile.updated = now;
    settings.serialize();
    Ok(())
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn remove_bit(
    app_handle: AppHandle,
    profile: UserProfile,
    bit: Bit,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;
    let profile = settings
        .profiles
        .get_mut(&profile.hub_profile.id)
        .ok_or(anyhow::anyhow!("Profile not found"))?;
    profile.hub_profile.remove_bit(&bit);
    let now = now_iso();
    profile.hub_profile.updated = now.clone();
    profile.updated = now;
    settings.serialize();
    Ok(())
}

/// Creates or updates a bit in the user-wide custom-model library. Works fully
/// offline — this is the desktop's local store for private model bits; the
/// frontend additionally syncs to the API when a session exists. Which
/// profiles use the bit is decided separately via `add_bit`/`remove_bit`.
#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn upsert_custom_bit(app_handle: AppHandle, bit: Bit) -> Result<Bit, TauriFunctionError> {
    let mut bit = bit;
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;
    let previous = settings
        .custom_bits
        .iter()
        .find(|existing| existing.id == bit.id)
        .cloned();

    // Keep offline/local-only behavior identical to the API: a pinned-source
    // edit gets a fresh artifact, dependency-pack and runtime cache identity.
    // If an edit form carried an old explicit checksum to a new source, clear
    // it; an explicit checksum for an unchanged source remains verified.
    bit.normalize_edited_user_local_artifact_identity(previous.as_ref());
    if bit.hash.is_empty() {
        bit.hash = bit.id.clone();
    }
    if bit.dependency_tree_hash.is_empty() {
        bit.dependency_tree_hash = bit.id.clone();
    }

    match settings
        .custom_bits
        .iter_mut()
        .find(|existing| existing.id == bit.id)
    {
        Some(existing) => *existing = bit.clone(),
        None => settings.custom_bits.push(bit.clone()),
    }

    settings.serialize();
    Ok(bit)
}

/// Deletes a bit from the library and drops it from every profile that used it.
#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn remove_custom_bit(
    app_handle: AppHandle,
    bit_id: String,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;

    settings.custom_bits.retain(|bit| bit.id != bit_id);

    let now = now_iso();
    for profile in settings.profiles.values_mut() {
        let before = profile.hub_profile.bits.len();
        profile.hub_profile.bits.retain(|reference| {
            reference
                .rsplit_once(':')
                .map_or(reference.as_str(), |(_, id)| id)
                != bit_id
        });
        profile
            .hub_profile
            .custom_bits
            .retain(|existing| existing.0.id != bit_id);
        if profile.hub_profile.bits.len() != before {
            profile.hub_profile.updated = now.clone();
            profile.updated = now.clone();
        }
    }

    settings.serialize();
    Ok(())
}

/// The user's whole custom-model library, independent of profile membership —
/// this is what the catalog lists so credentials are only ever entered once.
#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn get_custom_bits(app_handle: AppHandle) -> Result<Vec<Bit>, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let settings = settings.lock().await;
    Ok(settings
        .custom_bits
        .iter()
        .cloned()
        .map(|mut bit| {
            bit.normalize_user_local_artifact_identity();
            bit
        })
        .collect())
}

fn validate_profile_image(bytes: &[u8]) -> Result<&'static str, TauriFunctionError> {
    if bytes.is_empty() || bytes.len() > 10 * 1024 * 1024 {
        return Err(TauriFunctionError::new(
            "Choose an image smaller than 10 MB.",
        ));
    }
    let format = image::guess_format(bytes)
        .map_err(|_| TauriFunctionError::new("Choose a PNG, JPEG or WebP image."))?;
    let extension = match format {
        image::ImageFormat::Png => "png",
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::WebP => "webp",
        _ => return Err(TauriFunctionError::new("Choose a PNG, JPEG or WebP image.")),
    };
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| {
        TauriFunctionError::new("This image could not be read. Choose a valid image no larger than 8192 pixels on each side.")
    })?;
    if decoded.width() == 0 || decoded.height() == 0 {
        return Err(TauriFunctionError::new("This image could not be read."));
    }
    Ok(extension)
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn change_profile_image(
    app_handle: AppHandle,
    profile: UserProfile,
) -> Result<(), TauriFunctionError> {
    let dir = get_cache_dir();
    let dir = dir.join("icons");
    let Some(file_path) = app_handle
        .dialog()
        .file()
        .add_filter("Profile images", &["png", "jpg", "jpeg", "webp"])
        .blocking_pick_file()
    else {
        return Ok(());
    };
    let file_path = file_path
        .into_path()
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let metadata =
        std::fs::metadata(&file_path).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    if metadata.len() == 0 || metadata.len() > 10 * 1024 * 1024 {
        return Err(TauriFunctionError::new(
            "Choose an image smaller than 10 MB.",
        ));
    }
    let bytes = std::fs::read(&file_path).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let extension = validate_profile_image(&bytes)?;
    let hash = hash_file(&file_path);
    let new_path = dir.join(format!("{hash}.{extension}"));
    std::fs::create_dir_all(&dir).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    if file_path != new_path {
        std::fs::write(&new_path, &bytes).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    }
    let icon = new_path.to_string_lossy().to_string();
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;
    let profile = settings
        .profiles
        .get_mut(&profile.hub_profile.id)
        .ok_or(anyhow::anyhow!("Profile not found"))?;

    let mut icon_to_delete = None;
    if let Some(old_icon) = profile.hub_profile.icon.take()
        && !old_icon.starts_with("http://")
        && !old_icon.starts_with("https://")
    {
        icon_to_delete = Some(old_icon);
    }

    println!("Setting icon to {}", icon);
    profile.hub_profile.icon = Some(icon);
    let now = now_iso();
    profile.hub_profile.updated = now.clone();
    profile.updated = now;
    settings.serialize();

    if let Some(icon) = icon_to_delete {
        let profiles_using_icon = settings
            .profiles
            .values()
            .filter(|p| p.hub_profile.icon == Some(icon.clone()))
            .count();
        if profiles_using_icon == 0 {
            if let Err(error) = std::fs::remove_file(&icon) {
                tracing::warn!(%error, "Could not remove the previous profile image");
            }
        }
    }

    Ok(())
}

#[derive(Clone, Deserialize)]
pub enum ProfileAppUpdateOperation {
    Upsert,
    Remove,
}

fn merge_profile_app(existing: Option<&ProfileApp>, app: ProfileApp) -> ProfileApp {
    let favorite_order = if app.favorite {
        app.favorite_order
            .or_else(|| existing.and_then(|existing| existing.favorite_order))
    } else {
        app.favorite_order
    };
    let pinned_order = if app.pinned {
        app.pinned_order
            .or_else(|| existing.and_then(|existing| existing.pinned_order))
    } else {
        app.pinned_order
    };

    ProfileApp {
        favorite_order,
        pinned_order,
        ..app
    }
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn profile_update_app(
    app_handle: AppHandle,
    profile: UserProfile,
    app: ProfileApp,
    operation: ProfileAppUpdateOperation,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;
    let profile = settings
        .profiles
        .get_mut(&profile.hub_profile.id)
        .ok_or(anyhow::anyhow!("Profile not found"))?;
    match operation {
        ProfileAppUpdateOperation::Upsert => {
            let apps = profile.hub_profile.apps.get_or_insert(vec![]);
            if let Some(existing_index) = apps.iter().position(|a| a.app_id == app.app_id) {
                let merged = merge_profile_app(apps.get(existing_index), app);
                apps[existing_index] = merged;
            } else {
                apps.push(app);
            }
        }
        ProfileAppUpdateOperation::Remove => {
            if let Some(apps) = profile.hub_profile.apps.as_mut() {
                apps.retain(|a| a.app_id != app.app_id);
            }
        }
    }

    let now = now_iso();
    profile.hub_profile.updated = now.clone();
    profile.updated = now;
    settings.serialize();
    Ok(())
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn profile_update_shortcuts(
    app_handle: AppHandle,
    profile_id: String,
    shortcuts: Vec<ProfileShortcut>,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;
    let profile = settings
        .profiles
        .get_mut(&profile_id)
        .ok_or(anyhow::anyhow!("Profile not found"))?;

    profile.hub_profile.shortcuts = Some(shortcuts);
    let now = now_iso();
    profile.hub_profile.updated = now.clone();
    profile.updated = now;
    settings.serialize();
    Ok(())
}

#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn profile_update_home_layout(
    app_handle: AppHandle,
    profile_id: String,
    layout: Option<flow_like_types::Value>,
) -> Result<(), TauriFunctionError> {
    if let Some(layout) = &layout {
        flow_like::profile::validate_home_layout(layout)
            .map_err(|message| TauriFunctionError::new(&message))?;
    }
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;
    let profile = settings
        .profiles
        .get_mut(&profile_id)
        .ok_or(anyhow::anyhow!("Profile not found"))?;
    profile.hub_profile.home_layout = layout;
    let now = now_iso();
    profile.hub_profile.updated = now.clone();
    profile.updated = now;
    settings.serialize();
    Ok(())
}

/// Read a profile icon file and return its bytes
#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn read_profile_icon(icon_path: String) -> Result<Vec<u8>, TauriFunctionError> {
    let decoded_path = urlencoding::decode(&icon_path)
        .map_err(|e| TauriFunctionError::new(&format!("Failed to decode path: {}", e)))?;

    let resolved_path =
        decode_asset_proxy_path(decoded_path.as_ref()).unwrap_or_else(|| decoded_path.into_owned());
    let path = PathBuf::from(resolved_path);

    if !path.exists() {
        return Err(TauriFunctionError::new(&format!(
            "Icon file not found: {}",
            path.display()
        )));
    }

    let bytes = std::fs::read(&path)
        .map_err(|e| TauriFunctionError::new(&format!("Failed to read icon file: {}", e)))?;

    Ok(bytes)
}

/// Get the raw filesystem path for a profile's icon or thumbnail
#[instrument(skip_all)]
#[tauri::command(async)]
pub async fn get_profile_icon_path(
    app_handle: AppHandle,
    profile_id: String,
    field: String,
) -> Result<Option<String>, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let settings = settings.lock().await;

    let profile = settings
        .profiles
        .get(&profile_id)
        .ok_or(anyhow::anyhow!("Profile not found"))?;

    let path = match field.as_str() {
        "icon" => profile.hub_profile.icon.clone(),
        "thumbnail" => profile.hub_profile.thumbnail.clone(),
        _ => None,
    };

    match path {
        Some(p) => {
            if let Some(decoded_path) = decode_asset_proxy_path(&p) {
                return Ok(Some(decoded_path));
            }

            if p.starts_with("http://") || p.starts_with("https://") {
                return Ok(None);
            }

            Ok(Some(p))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod profile_settings_tests {
    use super::*;

    #[test]
    fn settings_patch_preserves_newer_membership_and_media() {
        let mut existing = UserProfile::new(Profile::default());
        existing.hub_profile.id = "profile".into();
        existing.hub_profile.name = "Original".into();
        existing.hub_profile.icon = Some("/new-icon.webp".into());
        existing.hub_profile.bits = vec!["hub:new-model".into()];
        existing.hub_profile.apps = Some(vec![ProfileApp::new("new-app".into())]);
        existing.hub_profile.home_default_id = Some("new-home".into());
        existing.hub_profile.theme = Some(flow_like_types::Value::String("old-theme".into()));
        let mut stale_draft = existing.clone();
        stale_draft.hub_profile.name = " Renamed ".into();
        stale_draft.hub_profile.icon = Some("/old-icon.webp".into());
        stale_draft.hub_profile.bits.clear();
        stale_draft.hub_profile.apps = Some(vec![]);
        stale_draft.hub_profile.home_default_id = None;
        stale_draft.hub_profile.theme = None;
        stale_draft.execution_settings.gpu_mode = false;
        stale_draft.execution_settings.max_context_size = 0;

        apply_profile_settings(&mut existing, stale_draft).unwrap();

        assert_eq!(existing.hub_profile.name, "Renamed");
        assert_eq!(existing.hub_profile.icon.as_deref(), Some("/new-icon.webp"));
        assert_eq!(existing.hub_profile.bits, vec!["hub:new-model"]);
        assert_eq!(
            existing.hub_profile.apps.as_ref().unwrap()[0].app_id,
            "new-app"
        );
        assert_eq!(
            existing.hub_profile.home_default_id.as_deref(),
            Some("new-home")
        );
        assert!(existing.hub_profile.theme.is_none());
        assert!(!existing.execution_settings.gpu_mode);
        assert_eq!(existing.execution_settings.max_context_size, 0);
    }

    #[test]
    fn invalid_settings_patch_keeps_existing_values() {
        let mut existing = UserProfile::new(Profile::default());
        existing.hub_profile.name = "Original".into();
        let before = existing.clone();
        let mut changes = existing.clone();
        changes.hub_profile.name = " ".into();
        changes.hub_profile.description = Some("Should not save".into());
        assert!(apply_profile_settings(&mut existing, changes).is_err());
        assert_eq!(existing, before);
    }
}

#[cfg(test)]
mod profile_image_tests {
    use super::*;
    use std::io::Cursor;

    fn encoded_image(format: image::ImageFormat, width: u32) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(width, 2)
            .write_to(&mut output, format)
            .unwrap();
        output.into_inner()
    }

    #[test]
    fn accepts_complete_supported_images() {
        for (format, extension) in [
            (image::ImageFormat::Png, "png"),
            (image::ImageFormat::Jpeg, "jpg"),
            (image::ImageFormat::WebP, "webp"),
        ] {
            assert_eq!(
                validate_profile_image(&encoded_image(format, 3)).unwrap(),
                extension
            );
        }
    }

    #[test]
    fn rejects_corrupt_pixels_even_when_dimensions_are_readable() {
        let mut bytes = encoded_image(image::ImageFormat::Png, 3);
        let idat = bytes
            .windows(4)
            .position(|window| window == b"IDAT")
            .unwrap();
        bytes[idat + 4] ^= 0xff;
        let dimensions =
            image::ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Png)
                .into_dimensions()
                .unwrap();
        assert_eq!(dimensions, (3, 2));
        assert!(validate_profile_image(&bytes).is_err());
        assert!(validate_profile_image(&bytes[..idat + 4]).is_err());
    }

    #[test]
    fn rejects_oversized_dimensions_and_invalid_headers() {
        assert!(validate_profile_image(&encoded_image(image::ImageFormat::Png, 8193)).is_err());
        assert!(validate_profile_image(b"invalid image").is_err());
    }
}
