//! Offline write buffering for online projects: per-account scopes, run storage modes, the
//! desktop replay host, the file stores and the Offline access commands.

pub(crate) mod commands;
pub(crate) mod data_studio;
mod host;
mod object_index;
pub(crate) mod run_storage;
mod scope;
mod stores;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use flow_like::{flow::execution::extract_sub_from_jwt, hub::hub_origin};
use flow_like_offline_writes::{
    Outbox,
    fs::{private_directory, validate_namespace},
};
use scope::{AppOfflineScope, CacheDirs, ScopeDescriptor, ScopeKey};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};
use tauri::{AppHandle, Manager};

pub(crate) const STATUS_EVENT: &str = "offline-writes:status";
pub(crate) const TABLES_EVENT: &str = "offline-writes:tables-changed";
pub(crate) const MIRROR_EVENT: &str = "offline-writes:mirror";

const INSTALLATION_FILE: &str = "installation-id";
const TOMBSTONE_FILE: &str = "forgotten";
const ENGINE_DIRECTORY: &str = ".standalone-outbox";
const SCOPES_DIRECTORY: &str = "scopes";

/// Every open offline scope of this process (Tauri managed state).
pub(crate) struct OfflineWrites {
    root: PathBuf,
    installation: OnceLock<String>,
    scopes: Mutex<HashMap<ScopeKey, Arc<AppOfflineScope>>>,
}

impl OfflineWrites {
    pub(crate) fn new() -> Self {
        Self::at(root_dir())
    }

    pub(crate) fn at(root: PathBuf) -> Self {
        Self {
            root,
            installation: OnceLock::new(),
            scopes: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn installation_id(&self) -> Result<String> {
        if let Some(id) = self.installation.get() {
            return Ok(id.clone());
        }
        let id = installation_id(&self.root)?;
        Ok(self.installation.get_or_init(|| id).clone())
    }

    fn scopes(&self) -> MutexGuard<'_, HashMap<ScopeKey, Arc<AppOfflineScope>>> {
        self.scopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn existing(&self, key: &ScopeKey) -> Option<Arc<AppOfflineScope>> {
        self.scopes().get(key).cloned()
    }

    /// The scope of `key`, loaded from its descriptor on first use.
    pub(crate) fn obtain(
        &self,
        app_handle: Option<AppHandle>,
        dirs: CacheDirs,
        key: ScopeKey,
    ) -> Result<Arc<AppOfflineScope>> {
        if let Some(scope) = self.existing(&key) {
            return Ok(scope);
        }
        let installation = self.installation_id()?;
        let id = scope_id(&key.hub, &key.subject, &key.app_id, &installation);
        let descriptor = scope::read_descriptor(&self.root, &id)?
            .filter(|descriptor| descriptor.key() == key)
            .unwrap_or_else(|| ScopeDescriptor::new(&id, &key));
        let created = AppOfflineScope::new(
            app_handle,
            self.root.clone(),
            installation,
            id,
            key.clone(),
            descriptor,
            dirs,
        );
        Ok(self.scopes().entry(key).or_insert(created).clone())
    }

    pub(crate) fn descriptors(&self) -> Vec<ScopeDescriptor> {
        scope::read_descriptors(&self.root)
    }

    /// Queued changes of `app_id` in the scopes of other accounts, read without opening them.
    pub(crate) fn other_accounts_pending(&self, app_id: &str, except: &str) -> u64 {
        self.descriptors()
            .into_iter()
            .filter(|descriptor| descriptor.app_id == app_id && descriptor.scope != except)
            .map(|descriptor| pending_count(&engine_dir(&self.root, app_id, &descriptor.scope)))
            .sum()
    }

    /// Closes and deletes the scopes of `app_id`: every account's with `all_accounts`,
    /// otherwise only `current`'s.
    pub(crate) async fn forget(
        &self,
        app_id: &str,
        current: Option<&ScopeKey>,
        all_accounts: bool,
    ) -> Result<()> {
        let mut targets = BTreeSet::new();
        if all_accounts {
            targets.extend(
                self.descriptors()
                    .into_iter()
                    .filter(|descriptor| descriptor.app_id == app_id)
                    .map(|descriptor| descriptor.scope),
            );
            targets.extend(engine_scopes(&self.root, app_id));
            targets.extend(
                self.scopes()
                    .values()
                    .filter(|scope| scope.app_id() == app_id)
                    .map(|scope| scope.id().to_owned()),
            );
        } else if let Some(key) = current.filter(|key| key.app_id == app_id) {
            targets.insert(scope_id(
                &key.hub,
                &key.subject,
                app_id,
                &self.installation_id()?,
            ));
        }
        let closing: Vec<Arc<AppOfflineScope>> = {
            let mut scopes = self.scopes();
            let keys: Vec<ScopeKey> = scopes
                .iter()
                .filter(|(_, scope)| targets.contains(scope.id()))
                .map(|(key, _)| key.clone())
                .collect();
            keys.iter().filter_map(|key| scopes.remove(key)).collect()
        };
        for scope in &closing {
            scope.close().await;
        }
        for scope in &targets {
            delete_scope(&self.root, app_id, scope);
        }
        Ok(())
    }

    /// Scopes whose deletion failed are deleted before anything opens them again.
    fn delete_tombstoned(&self) {
        let Ok(entries) = std::fs::read_dir(self.root.join(SCOPES_DIRECTORY)) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(app_id) = std::fs::read_to_string(entry.path().join(TOMBSTONE_FILE)) else {
                continue;
            };
            let scope = entry.file_name().to_string_lossy().into_owned();
            delete_scope(&self.root, app_id.trim(), &scope);
        }
    }
}

/// Startup scan: deletes tombstoned scopes, then opens scopes with tables or queued changes.
pub(crate) fn spawn(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let descriptors = match app_handle.try_state::<OfflineWrites>() {
            Some(registry) => {
                registry.delete_tombstoned();
                registry.descriptors()
            }
            None => return,
        };
        let dirs = match scope::cache_dirs(&app_handle).await {
            Ok(dirs) => dirs,
            Err(error) => {
                tracing::warn!(%error, "Offline changes could not resume: project directories are unavailable");
                return;
            }
        };
        for descriptor in descriptors {
            let Some(queued) = app_handle.try_state::<OfflineWrites>().map(|registry| {
                pending_count(&engine_dir(
                    registry.root(),
                    &descriptor.app_id,
                    &descriptor.scope,
                ))
            }) else {
                return;
            };
            if descriptor.tables.is_empty() && queued == 0 {
                continue;
            }
            let Some(scope) = app_handle.try_state::<OfflineWrites>().map(|registry| {
                registry.obtain(Some(app_handle.clone()), dirs.clone(), descriptor.key())
            }) else {
                return;
            };
            match scope {
                Ok(scope) => {
                    if let Err(error) = scope.manager().await {
                        tracing::warn!(app_id = %descriptor.app_id, %error, "Offline changes of a project could not resume");
                    }
                }
                Err(error) => {
                    tracing::warn!(app_id = %descriptor.app_id, %error, "Offline scope could not be loaded");
                }
            }
        }
    });
}

pub(crate) async fn scope_for(app_handle: &AppHandle, key: ScopeKey) -> Result<Arc<AppOfflineScope>> {
    if let Some(scope) = app_handle
        .try_state::<OfflineWrites>()
        .and_then(|registry| registry.existing(&key))
    {
        return Ok(scope);
    }
    let dirs = scope::cache_dirs(app_handle).await?;
    app_handle
        .try_state::<OfflineWrites>()
        .context("Offline changes are not initialized on this device")?
        .obtain(Some(app_handle.clone()), dirs, key)
}

/// The hub key of the current profile.
pub(crate) async fn current_hub(app_handle: &AppHandle) -> Result<String> {
    let profile = crate::state::TauriSettingsState::current_profile(app_handle).await?;
    hub_key(&profile.hub_profile.hub, profile.hub_profile.secure)
        .context("No hub is configured for this profile")
}

/// The subject of a UI session token; None when signed out.
pub(crate) fn token_subject(token: Option<&str>) -> Option<String> {
    token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .and_then(|token| extract_sub_from_jwt(token).ok())
        .filter(|subject| !subject.is_empty())
}

#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) fn root_dir() -> PathBuf {
    crate::settings::mobile_storage_root().join("offline-writes")
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub(crate) fn root_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_default()
        .join("flow-like")
        .join("offline-writes")
}

pub(crate) fn ensure_root(root: &Path) -> Result<()> {
    if let Some(parent) = root.parent() {
        std::fs::create_dir_all(parent)?;
    }
    private_directory(root)
}

/// The device's installation identifier: a lowercase UUID v4 created once, never rotated.
pub(crate) fn installation_id(root: &Path) -> Result<String> {
    ensure_root(root)?;
    let path = root.join(INSTALLATION_FILE);
    if let Some(id) = read_installation(&path)? {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            file.write_all(id.as_bytes())?;
            file.sync_all()?;
            sync_directory(root)?;
            Ok(id)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_installation(&path)?
            .context("The offline installation identifier of this device is unreadable"),
        Err(error) => Err(error.into()),
    }
}

fn read_installation(path: &Path) -> Result<Option<String>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let id = text.trim();
    if flow_like_device_protocol::validate_installation_id(id).is_ok() {
        return Ok(Some(id.to_owned()));
    }
    tracing::warn!(path = %path.display(), "Replacing an incomplete offline installation identifier");
    std::fs::remove_file(path)?;
    Ok(None)
}

pub(crate) fn sync_directory(directory: &Path) -> Result<()> {
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

/// Canonical hub URL of a profile: lowercase scheme and host, path prefix kept, no trailing
/// slash. It equals `execution_credentials::canonical_hub` of the same origin.
pub(crate) fn hub_key(hub: &str, secure: bool) -> Option<String> {
    let url = reqwest::Url::parse(&hub_origin(hub, secure)?).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| url.as_str().trim_end_matches('/').to_owned())
}

#[derive(Serialize)]
struct ScopeIdentity<'a> {
    app: &'a str,
    hub: &'a str,
    installation: &'a str,
    subject: &'a str,
    v: u8,
}

/// blake3 of {"app","hub","installation","subject","v":1} in key order, as 64 lowercase hex.
pub(crate) fn scope_id(hub: &str, subject: &str, app_id: &str, installation: &str) -> String {
    let identity = ScopeIdentity {
        app: app_id,
        hub,
        installation,
        subject,
        v: 1,
    };
    let bytes = serde_json::to_vec(&identity).expect("a scope identity always serializes");
    blake3::hash(&bytes).to_hex().to_string()
}

pub(crate) fn valid_scope(scope: &str) -> bool {
    scope.len() == 64
        && scope
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(crate) fn engine_dir(root: &Path, app_id: &str, scope: &str) -> PathBuf {
    root.join(ENGINE_DIRECTORY).join(app_id).join(scope)
}

pub(crate) fn scope_dir(root: &Path, scope: &str) -> PathBuf {
    root.join(SCOPES_DIRECTORY).join(scope)
}

pub(crate) fn scopes_dir(root: &Path) -> PathBuf {
    root.join(SCOPES_DIRECTORY)
}

pub(crate) fn descriptor_path(root: &Path, scope: &str) -> PathBuf {
    root.join(SCOPES_DIRECTORY).join(format!("{scope}.json"))
}

fn engine_scopes(root: &Path, app_id: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root.join(ENGINE_DIRECTORY).join(app_id)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|scope| valid_scope(scope))
        .collect()
}

/// Queued changes of an engine directory, read without creating, locking or writing anything.
pub(crate) fn pending_count(engine: &Path) -> u64 {
    Outbox::open_read_only(engine)
        .ok()
        .flatten()
        .and_then(|reader| reader.status().ok())
        .map_or(0, |status| status.pending_count)
}

/// Deletes a scope's engine directory, descriptor and index. A tombstone written first
/// finishes the deletion at the next startup when a file is still open.
fn delete_scope(root: &Path, app_id: &str, scope: &str) {
    if !valid_scope(scope) || validate_namespace(app_id).is_err() {
        return;
    }
    let directory = scope_dir(root, scope);
    let tombstone = || -> Result<()> {
        ensure_root(root)?;
        private_directory(&scopes_dir(root))?;
        private_directory(&directory)?;
        std::fs::write(directory.join(TOMBSTONE_FILE), app_id)?;
        Ok(())
    };
    if let Err(error) = tombstone() {
        tracing::warn!(%error, "Could not mark forgotten offline changes for deletion");
    }
    let removed = [
        remove_directory(&engine_dir(root, app_id, scope)),
        remove_file(&descriptor_path(root, scope)),
        remove_directory(&directory),
    ];
    if let Some(error) = removed.into_iter().find_map(Result::err) {
        tracing::warn!(%error, "Offline changes of a forgotten project are deleted at the next start");
        if let Err(error) = tombstone() {
            tracing::warn!(%error, "Could not mark forgotten offline changes for deletion");
        }
    }
}

fn remove_directory(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn remove_file(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

pub(crate) fn api_base(hub: &str) -> String {
    if hub.ends_with("/api/v1") {
        hub.to_owned()
    } else {
        format!("{hub}/api/v1")
    }
}

pub(crate) fn authorization(token: &str) -> String {
    if token.starts_with("pat_") {
        token.to_owned()
    } else {
        format!("Bearer {token}")
    }
}

pub(crate) fn emit<T: Serialize + Clone + Send + 'static>(
    app_handle: Option<&AppHandle>,
    event: &str,
    payload: T,
) {
    if let Some(app_handle) = app_handle {
        crate::utils::emit_to_ui(app_handle, event, payload);
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppEvent {
    pub app_id: String,
}

/// Normative user-facing texts (design §1.3, §3.9, §4.9, §4.12).
pub(crate) mod texts {
    use std::fmt::Display;

    pub(crate) const FORGOTTEN: &str =
        "Offline changes for this project were removed from this device";
    pub(crate) const UNATTRIBUTED: &str = "This automation has not run while connected on this device yet, so its offline changes cannot be attributed to an account. Run it once while connected.";
    pub(crate) const SIGN_IN_DATA: &str = "Sign in to use this project's data on this device.";
    pub(crate) const HUB_ONLY_TABLES: &str = "Data Studio reads this online project's tables through the hub. Reconnect to the hub to open them.";
    pub(crate) const HUB_UNSUPPORTED: &str = "This hub does not support offline access for desktop apps yet. Ask the hub operator to update it.";
    pub(crate) const ONLINE_PROJECTS_ONLY: &str = "Offline access is for online projects.";
    pub(crate) const SIGN_IN_MANAGE: &str = "Sign in to manage offline access for this project.";
    pub(crate) const HUB_FILES: &str = "This project's files are stored in the hub. Upload them from the Storage page while connected.";
    pub(crate) const FORBIDDEN: &str = "Your role in this project no longer allows this change. Ask a project admin for write access, then retry, or skip the change.";
    pub(crate) const ENDPOINT_MISSING: &str = "This hub does not accept offline changes from the desktop app yet. Your changes stay queued on this device; retry after the hub is updated.";
    pub(crate) const NO_TABLE_ACCESS: &str =
        "Connect to the hub to download or refresh offline tables";
    pub(crate) const NO_DATA_ACCESS: &str = "Connect to the hub to download offline data";
    pub(crate) const LAZY_MIRROR_MISSING: &str = "Download everything needs a lazy offline mirror";

    pub(crate) fn table_unavailable(table: &str) -> String {
        format!(
            "Table '{table}' is not available offline. Turn on \"Available offline\" for it in the project's Offline access settings while connected, or reconnect to the hub."
        )
    }

    pub(crate) fn database_unavailable(user: bool) -> String {
        format!(
            "The {} database is not available offline. Only tables marked \"Available offline\" on this device can be used while the hub is unreachable.",
            if user { "user" } else { "project" }
        )
    }

    pub(crate) fn table_not_ready(table: &str) -> String {
        format!(
            "Table '{table}' is not set up for offline use on this device yet. Reconnect to the hub to finish setting it up."
        )
    }

    pub(crate) fn write_needs_hub(path: &str) -> String {
        format!(
            "Writing '{path}' needs a connection to the hub. While offline, only new files in the project's upload and storage folders and in your user folder are kept for upload."
        )
    }

    pub(crate) fn unavailable(error: impl Display) -> String {
        format!("Offline changes are unavailable on this device: {error}")
    }

    pub(crate) fn not_cached(path: &str) -> String {
        format!("Cloud storage is unreachable and '{path}' is not cached on this device.")
    }

    pub(crate) fn waiting_to_upload(path: &str) -> String {
        format!("'{path}' is waiting to upload; a link is available once it reaches the cloud.")
    }

    pub(crate) fn not_configured(table: &str) -> String {
        format!(
            "Table '{table}' of this online project is not available offline on this device. Open it through the hub, or turn on \"Available offline\" for it in the project's Offline access settings."
        )
    }

    pub(crate) fn listing_unavailable(prefix: &str) -> String {
        format!(
            "The file list of '{prefix}' is not available offline. Open this folder once while connected so this device can keep its list."
        )
    }

    pub(crate) fn hub_rejects_files(path: &str) -> String {
        format!(
            "This hub does not accept offline changes from the desktop app. Reconnect to save '{path}'."
        )
    }

    pub(crate) fn other_scope(path: &str) -> String {
        format!("'{path}' belongs to another project or account and is not available to this run.")
    }

    pub(crate) fn stale_copy(table: &str, time: &str, reason: &str) -> String {
        format!(
            "Table '{table}' is read from this device's offline copy from {time}; newer cloud changes are not visible yet: {reason}"
        )
    }

    pub(crate) fn missing_in_cloud(table: &str) -> String {
        format!(
            "Table '{table}' does not exist in the cloud yet. Create it while connected, then turn on offline access."
        )
    }

    pub(crate) fn deleted_in_cloud(table: &str) -> String {
        format!(
            "Table '{table}' was deleted in the cloud. Turn off offline access for it on this device; its queued changes cannot be applied."
        )
    }

    pub(crate) fn key_change_pending(table: &str) -> String {
        format!(
            "Table '{table}' still has queued changes. Sync or skip them before changing its key column."
        )
    }

    pub(crate) fn disable_pending(table: &str) -> String {
        format!(
            "Table '{table}' still has queued changes. Sync or skip them before turning off offline access."
        )
    }

    pub(crate) fn limit_below_required(needed: &str) -> String {
        format!(
            "Tables that download everything and tables with queued changes need at least {needed} on this device; choose a limit of at least that size."
        )
    }

    pub(crate) fn subject_mismatch(subject: &str) -> String {
        format!(
            "These changes were queued for another account. Sign in as {subject} or update this automation's token, then retry, or skip the change."
        )
    }

    pub(crate) fn hub_limit(limit: &str) -> String {
        format!(
            "This change is larger than the hub accepts ({limit}). Ask the hub operator to raise the limit, then retry, or skip the change."
        )
    }

    pub(crate) fn sign_in_to_sync(subject: &str) -> String {
        format!("Sign in as {subject} to sync the changes queued on this device")
    }
}
