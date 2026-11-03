//! Bots that live on this device without an account, with their own inbox. The store is
//! `local_bots.db` next to `event_sinks.db`; secrets are sealed under a key kept in the system
//! keychain, and every committed change reaches the UI as `local-bots-changed` or
//! `local-inbox-changed`. Commands reject with `{ code, message }`: the store's code (e.g.
//! `NOT_FOUND`, `LOCAL_CHANGED`) and what the person reads. [`moves`] moves bots into the
//! signed-in account; [`workspace`] keeps each bot's own app; [`history`] reads a chat by
//! position and keeps its topics and summaries.

pub mod history;
mod hub;
pub mod moves;
pub mod workspace;

use flow_like_local_bots::{
    Bot, BotApp, BotMover, BotPermission, BotSecretInfo, ChangeNotifier, Conversation, CreateBotRequest,
    CreateConversationRequest, EntriesQuery, HubBody, HubCall, HubMethod, InboxEntriesPage,
    InboxEntry, KeyError, LOCAL_OWNER_ID, LocalBotBundle, LocalBotStore, MarkReadResponse,
    SecretKeyProvider, SendEntryRequest, UpdateBotRequest, codes,
};
use flow_like_types::{bot::BotSystemKind, bot_restrictions::BotRestrictions};
use hub::HubClient;
use serde::Serialize;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, Manager, State};
use zeroize::Zeroizing;

#[cfg(all(not(test), not(target_os = "android")))]
const KEYCHAIN_SERVICE: &str = "com.flow-like.local-bots.v1";
#[cfg(all(not(test), not(target_os = "android")))]
const KEYCHAIN_ACCOUNT: &str = "secret-key";

/// The hub could not take a secret during a move; the message names the secret, never its value.
const SECRET_UPLOAD_FAILED: &str = "SECRET_UPLOAD_FAILED";

/// Next to `event_sinks.db`: the parent of the project directory.
pub fn database_path(project_dir: &Path) -> PathBuf {
    project_dir
        .parent()
        .unwrap_or(project_dir)
        .join("local_bots.db")
}

/// What a command rejects with, serialized as `{ code, message }`.
#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    code: &'static str,
    message: String,
}

impl CommandError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<flow_like_local_bots::Error> for CommandError {
    fn from(error: flow_like_local_bots::Error) -> Self {
        Self::new(error.code(), error.message())
    }
}

/// The store, or why it could not be opened (every command then answers with the reason), and
/// the moves into the account, one at a time.
pub struct LocalBots {
    store: Result<Arc<LocalBotStore>, CommandError>,
    mover: BotMover,
}

impl LocalBots {
    pub fn open(app: &AppHandle, path: &Path) -> Self {
        let store = LocalBotStore::open(
            path,
            Arc::new(KeychainKey),
            Arc::new(UiNotifier(app.clone())),
        )
        .map(Arc::new)
        .map_err(|error| {
            tracing::error!(%error, path = %path.display(), "Opening the local bots store failed");
            CommandError::new(
                error.code(),
                format!("Bots on this device are unavailable: {error}"),
            )
        });
        Self {
            store,
            mover: BotMover::default(),
        }
    }

    fn store(&self) -> Result<Arc<LocalBotStore>, CommandError> {
        self.store.clone()
    }
}

#[derive(Clone, Serialize)]
struct BotsChanged {
    bot_id: Option<String>,
}

#[derive(Clone, Serialize)]
struct InboxChanged {
    conversation_id: Option<String>,
}

struct UiNotifier(AppHandle);

impl ChangeNotifier for UiNotifier {
    fn bots_changed(&self, bot_id: Option<&str>) {
        crate::utils::emit_to_ui(
            &self.0,
            "local-bots-changed",
            BotsChanged {
                bot_id: bot_id.map(str::to_string),
            },
        );
    }

    fn inbox_changed(&self, conversation_id: Option<&str>) {
        if let Some(host) = self.0.try_state::<crate::bot_host::BotHost>() {
            host.poke(conversation_id);
        }
        crate::utils::emit_to_ui(
            &self.0,
            "local-inbox-changed",
            InboxChanged {
                conversation_id: conversation_id.map(str::to_string),
            },
        );
    }
}

struct KeychainKey;

impl SecretKeyProvider for KeychainKey {
    fn key(&self, create: bool) -> Result<Zeroizing<[u8; 32]>, KeyError> {
        keychain_key(create)
    }
}

/// A key that is gone or damaged is `Missing` (nothing sealed under it opens again); a keychain
/// that can't be asked now is `Unavailable`, and nothing is overwritten.
#[cfg(all(not(test), not(target_os = "android")))]
fn keychain_key(create: bool) -> Result<Zeroizing<[u8; 32]>, KeyError> {
    use chacha20poly1305::{KeyInit, XChaCha20Poly1305, aead::OsRng};

    let unavailable = |reason: &str| KeyError::Unavailable(reason.into());
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|_| unavailable("Cannot open the system credential store"))?;
    let secret = match entry.get_secret() {
        Ok(secret) => Zeroizing::new(secret),
        Err(keyring::Error::NoEntry) if create => {
            let secret = Zeroizing::new(XChaCha20Poly1305::generate_key(&mut OsRng).to_vec());
            save_key(&entry, &secret).map_err(KeyError::Unavailable)?;
            secret
        }
        Err(keyring::Error::NoEntry) => {
            return Err(KeyError::Missing(
                "This device's key for bot secrets is missing from the keychain".into(),
            ));
        }
        Err(_) => {
            return Err(unavailable(
                "The system credential store is unavailable or locked",
            ));
        }
    };
    let key: [u8; 32] = secret
        .as_slice()
        .try_into()
        .map_err(|_| KeyError::Missing("This device's key for bot secrets is damaged".into()))?;
    Ok(Zeroizing::new(key))
}

#[cfg(all(not(test), not(target_os = "android"), not(target_os = "ios")))]
fn save_key(entry: &keyring::Entry, secret: &[u8]) -> Result<(), String> {
    entry
        .set_secret(secret)
        .map_err(|_| "Cannot save the key for bot secrets".into())
}

#[cfg(all(not(test), target_os = "ios"))]
fn save_key(_: &keyring::Entry, secret: &[u8]) -> Result<(), String> {
    use security_framework::{
        access_control::{ProtectionMode, SecAccessControl},
        passwords::{PasswordOptions, set_generic_password_options},
    };
    let mut options = PasswordOptions::new_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT);
    // Bots may run in the background after the first unlock. The key stays on this device and
    // out of iCloud keychain backups.
    let access = SecAccessControl::create_with_protection(
        Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
        0,
    )
    .map_err(|_| "Cannot configure the keychain protection of the key for bot secrets")?;
    options.set_access_control(access);
    set_generic_password_options(secret, options)
        .map_err(|_| "Cannot save the key for bot secrets".into())
}

// Android has no OS keyring backend in this dependency, and secrets must never fall back to a
// key stored next to them.
#[cfg(all(not(test), target_os = "android"))]
fn keychain_key(_: bool) -> Result<Zeroizing<[u8; 32]>, KeyError> {
    Err(KeyError::Unavailable(
        "Secrets need a system keychain, which this device doesn't offer".into(),
    ))
}

#[cfg(test)]
fn keychain_key(create: bool) -> Result<Zeroizing<[u8; 32]>, KeyError> {
    static KEY: std::sync::Mutex<Option<[u8; 32]>> = std::sync::Mutex::new(None);
    let mut key = KEY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if key.is_none() && create {
        *key = Some([5; 32]);
    }
    key.map(Zeroizing::new).ok_or_else(|| {
        KeyError::Missing("This device's key for bot secrets is missing from the keychain".into())
    })
}

/// Runs `work` on the store off the async runtime.
async fn with_store<T, F>(state: State<'_, LocalBots>, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&LocalBotStore) -> flow_like_local_bots::Result<T> + Send + 'static,
{
    on_store(&state, work).await
}

/// [`with_store`] on the bots themselves, for callers that don't hold Tauri's state.
pub(crate) async fn on_store<T, F>(bots: &LocalBots, work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&LocalBotStore) -> flow_like_local_bots::Result<T> + Send + 'static,
{
    let store = bots.store()?;
    tokio::task::spawn_blocking(move || work(&store))
        .await
        .map_err(|error| {
            CommandError::new(
                codes::STORAGE,
                format!("The local bots worker failed: {error}"),
            )
        })?
        .map_err(CommandError::from)
}

/// Points local bots at a profile's server id once profile sync renamed it. Never fails the
/// profile remap: a failure is logged.
pub async fn remap_profile(app: &AppHandle, local_id: &str, server_id: &str) {
    let Some(state) = app.try_state::<LocalBots>() else {
        return;
    };
    let (local_id, server_id) = (local_id.to_string(), server_id.to_string());
    if let Err(error) = with_store(state, move |store| {
        store.remap_profile(&local_id, &server_id)
    })
    .await
    {
        tracing::warn!(%error, "Remapping local bots to the synced profile failed");
    }
}

/// An account bot id as the hub mints it, so it stays one path segment.
fn check_account_bot_id(id: &str) -> Result<(), CommandError> {
    let valid = id.strip_prefix("bot_").is_some_and(|rest| {
        (1..=64).contains(&rest.len())
            && rest
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    });
    if valid {
        return Ok(());
    }
    Err(CommandError::new(
        codes::INVALID,
        format!(
            "'{}' is not a bot id of your account",
            id.chars().take(80).collect::<String>()
        ),
    ))
}

/// Assistant and Flux first (their rows are made on the first list, like the API's), then the
/// person's own bots, newest first.
#[tauri::command]
pub async fn local_bots_list(state: State<'_, LocalBots>) -> Result<Vec<Bot>, CommandError> {
    with_store(state, |store| {
        store.ensure_built_ins()?;
        store.list_bots()
    })
    .await
}

#[tauri::command]
pub async fn local_bots_get(
    state: State<'_, LocalBots>,
    bot_id: String,
) -> Result<Option<Bot>, CommandError> {
    with_store(state, move |store| store.get_bot(&bot_id)).await
}

/// Assistant's or Flux's row on this device, with the person's edits; `None` before the Bots tab
/// made it. Never makes one.
pub async fn local_built_in(
    state: State<'_, LocalBots>,
    kind: BotSystemKind,
) -> Result<Option<Bot>, CommandError> {
    with_store(state, move |store| store.built_in_bot(kind)).await
}

#[tauri::command]
pub async fn local_bots_create(
    state: State<'_, LocalBots>,
    request: CreateBotRequest,
) -> Result<Bot, CommandError> {
    with_store(state, move |store| store.create_bot(request)).await
}

#[tauri::command]
pub async fn local_bots_update(
    state: State<'_, LocalBots>,
    bot_id: String,
    request: UpdateBotRequest,
) -> Result<Bot, CommandError> {
    with_store(state, move |store| store.update_bot(&bot_id, request)).await
}

/// Deletes the bot with its workspace app and its memory; Assistant and Flux stay.
#[tauri::command]
pub async fn local_bots_delete(
    app_handle: AppHandle,
    state: State<'_, LocalBots>,
    bot_id: String,
) -> Result<(), CommandError> {
    let host = workspace::DesktopWorkspaces::new(&app_handle);
    Ok(state
        .store()?
        .delete_bot_with_workspace(&host, &bot_id)
        .await?)
}

/// A new local bot like `botId`, named "<name> copy", with its apps; never its secrets,
/// workspace, memory or chats.
#[tauri::command]
pub async fn local_bots_clone(
    state: State<'_, LocalBots>,
    bot_id: String,
) -> Result<Bot, CommandError> {
    with_store(state, move |store| store.clone_bot(&bot_id)).await
}

#[tauri::command]
pub async fn local_bots_apps(
    state: State<'_, LocalBots>,
    bot_id: String,
) -> Result<Vec<BotApp>, CommandError> {
    with_store(state, move |store| store.bot_apps(&bot_id)).await
}

#[tauri::command]
pub async fn local_bots_attach_app(
    state: State<'_, LocalBots>,
    bot_id: String,
    app_id: String,
) -> Result<(), CommandError> {
    with_store(state, move |store| store.attach_app(&bot_id, &app_id)).await
}

#[tauri::command]
pub async fn local_bots_detach_app(
    state: State<'_, LocalBots>,
    bot_id: String,
    app_id: String,
) -> Result<(), CommandError> {
    with_store(state, move |store| store.detach_app(&bot_id, &app_id)).await
}

#[tauri::command]
pub async fn local_bots_app_restrictions(
    state: State<'_, LocalBots>,
    bot_id: String,
    app_id: String,
) -> Result<BotRestrictions, CommandError> {
    with_store(state, move |store| store.app_restrictions(&bot_id, &app_id)).await
}

#[tauri::command]
pub async fn local_bots_set_app_restrictions(
    state: State<'_, LocalBots>,
    bot_id: String,
    app_id: String,
    restrictions: BotRestrictions,
) -> Result<BotRestrictions, CommandError> {
    with_store(state, move |store| {
        store.set_app_restrictions(&bot_id, &app_id, restrictions)
    })
    .await
}

#[tauri::command]
pub async fn local_bots_secrets(
    state: State<'_, LocalBots>,
    bot_id: String,
) -> Result<Vec<BotSecretInfo>, CommandError> {
    with_store(state, move |store| store.secrets(&bot_id)).await
}

/// An omitted `description` keeps the current one; an empty one clears it.
#[tauri::command]
pub async fn local_bots_set_secret(
    state: State<'_, LocalBots>,
    bot_id: String,
    name: String,
    value: String,
    description: Option<String>,
) -> Result<BotSecretInfo, CommandError> {
    let value = Zeroizing::new(value);
    with_store(state, move |store| {
        store.set_secret(&bot_id, &name, &value, description.map(Some))
    })
    .await
}

#[tauri::command]
pub async fn local_bots_delete_secret(
    state: State<'_, LocalBots>,
    bot_id: String,
    name: String,
) -> Result<(), CommandError> {
    with_store(state, move |store| store.delete_secret(&bot_id, &name)).await
}

/// The bot with its apps, conversations and secrets; secrets say only whether they open here.
#[tauri::command]
pub async fn local_bots_export(
    state: State<'_, LocalBots>,
    bot_id: String,
) -> Result<LocalBotBundle, CommandError> {
    with_store(state, move |store| store.export_bot(&bot_id)).await
}

/// What [`local_bots_upload_secrets`] put on the account bot, and what doesn't open here.
#[derive(Debug, Serialize)]
pub struct SecretUpload {
    uploaded: Vec<String>,
    unreadable: Vec<String>,
}

/// Opens the local bot's secrets here and puts each one this device's key opens on the account
/// bot `accountBotId` at the active profile's hub. Values never reach the webview.
#[tauri::command]
pub async fn local_bots_upload_secrets(
    app_handle: AppHandle,
    state: State<'_, LocalBots>,
    bot_id: String,
    account_bot_id: String,
    access_token: String,
) -> Result<SecretUpload, CommandError> {
    let access_token = Zeroizing::new(access_token);
    check_account_bot_id(&account_bot_id)?;
    let opened = with_store(state, move |store| store.open_secrets(&bot_id)).await?;
    let mut uploaded = Vec::with_capacity(opened.readable.len());
    if !opened.readable.is_empty() {
        let hub =
            HubClient::active(&app_handle, &access_token, "move secrets to your account").await?;
        for secret in &opened.readable {
            let path: [&str; 4] = ["bots", &account_bot_id, "secrets", &secret.name];
            hub.send(HubCall::new(HubMethod::Put, &path).with(HubBody::Secret(secret)))
                .await
                .map_err(|error| {
                    CommandError::new(
                        SECRET_UPLOAD_FAILED,
                        format!(
                            "Moving secret '{}' to bot '{account_bot_id}' failed: {}",
                            secret.name, error.message
                        ),
                    )
                })?;
            uploaded.push(secret.name.clone());
        }
    }
    Ok(SecretUpload {
        uploaded,
        unreadable: opened.unreadable,
    })
}

/// Points what stays on this device (memberships, senders, member events) at the account bot
/// local bot `localId` moved to. The local bot and its `updated_at` stay as they are.
#[tauri::command]
pub async fn local_bots_rebind(
    state: State<'_, LocalBots>,
    local_id: String,
    account_id: String,
) -> Result<(), CommandError> {
    with_store(state, move |store| {
        store.rebind_bot(&local_id, &account_id).map(drop)
    })
    .await
}

/// With `expectedUpdatedAt`, the moved bot's `updated_at`, a bot changed since stays and the
/// call rejects with `LOCAL_CHANGED`.
#[tauri::command]
pub async fn local_bots_forget(
    state: State<'_, LocalBots>,
    bot_id: String,
    expected_updated_at: Option<String>,
) -> Result<(), CommandError> {
    with_store(state, move |store| {
        store.forget_bot(&bot_id, expected_updated_at.as_deref())
    })
    .await
}

/// Every standing grant: which bot may ask which without asking the person again.
#[tauri::command]
pub async fn local_bots_permissions(
    state: State<'_, LocalBots>,
) -> Result<Vec<BotPermission>, CommandError> {
    with_store(state, |store| store.permissions()).await
}

/// Lets `from_bot_id` ask `to_bot_id` from now on; granting again returns the first grant.
#[tauri::command]
pub async fn local_bots_grant_permission(
    state: State<'_, LocalBots>,
    from_bot_id: String,
    to_bot_id: String,
) -> Result<BotPermission, CommandError> {
    with_store(state, move |store| {
        store.grant_permission(&from_bot_id, &to_bot_id)
    })
    .await
}

/// Takes a grant back; `false` when there was none.
#[tauri::command]
pub async fn local_bots_revoke_permission(
    state: State<'_, LocalBots>,
    from_bot_id: String,
    to_bot_id: String,
) -> Result<bool, CommandError> {
    with_store(state, move |store| {
        store.revoke_permission(&from_bot_id, &to_bot_id)
    })
    .await
}

#[tauri::command]
pub async fn local_inbox_conversations(
    state: State<'_, LocalBots>,
) -> Result<Vec<Conversation>, CommandError> {
    with_store(state, |store| store.conversations()).await
}

/// `null` for a conversation that isn't on this device, so callers tell it from a failure.
#[tauri::command]
pub async fn local_inbox_conversation(
    state: State<'_, LocalBots>,
    conversation_id: String,
) -> Result<Option<Conversation>, CommandError> {
    with_store(state, move |store| store.conversation(&conversation_id)).await
}

#[tauri::command]
pub async fn local_inbox_create(
    state: State<'_, LocalBots>,
    request: CreateConversationRequest,
) -> Result<Conversation, CommandError> {
    with_store(state, move |store| store.create_conversation(request)).await
}

#[tauri::command]
pub async fn local_inbox_rename(
    state: State<'_, LocalBots>,
    conversation_id: String,
    title: Option<String>,
) -> Result<Conversation, CommandError> {
    with_store(state, move |store| {
        store.rename_conversation(&conversation_id, title)
    })
    .await
}

#[tauri::command]
pub async fn local_inbox_add_members(
    state: State<'_, LocalBots>,
    conversation_id: String,
    members: Vec<String>,
) -> Result<Conversation, CommandError> {
    with_store(state, move |store| {
        store.add_members(&conversation_id, members)
    })
    .await
}

#[tauri::command]
pub async fn local_inbox_remove_member(
    state: State<'_, LocalBots>,
    conversation_id: String,
    principal_id: String,
) -> Result<(), CommandError> {
    with_store(state, move |store| {
        store.remove_member(&conversation_id, &principal_id)
    })
    .await
}

#[tauri::command]
pub async fn local_inbox_entries(
    state: State<'_, LocalBots>,
    conversation_id: String,
    before: Option<String>,
    after: Option<String>,
    limit: Option<u64>,
) -> Result<InboxEntriesPage, CommandError> {
    let query = EntriesQuery {
        before,
        after,
        limit,
    };
    with_store(state, move |store| store.entries(&conversation_id, &query)).await
}

#[tauri::command]
pub async fn local_inbox_send(
    state: State<'_, LocalBots>,
    conversation_id: String,
    request: SendEntryRequest,
) -> Result<InboxEntry, CommandError> {
    with_store(state, move |store| {
        store.send(&conversation_id, LOCAL_OWNER_ID, request)
    })
    .await
}

#[tauri::command]
pub async fn local_inbox_mark_read(
    state: State<'_, LocalBots>,
    conversation_id: String,
    up_to: Option<String>,
) -> Result<MarkReadResponse, CommandError> {
    with_store(state, move |store| {
        store.mark_read(&conversation_id, up_to.as_deref())
    })
    .await
}

/// With `upTo`, the cursor of the newest event that moved (`0:0` when none did), a
/// conversation holding a newer event stays and the call rejects with `LOCAL_CHANGED`.
#[tauri::command]
pub async fn local_inbox_forget(
    state: State<'_, LocalBots>,
    conversation_id: String,
    up_to: Option<String>,
) -> Result<(), CommandError> {
    with_store(state, move |store| {
        store.forget_conversation(&conversation_id, up_to.as_deref())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_reach_the_webview_as_code_and_message() {
        let error = CommandError::new(codes::LOCAL_CHANGED, "Bot 'bot_1' changed");
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({ "code": "LOCAL_CHANGED", "message": "Bot 'bot_1' changed" })
        );
    }

    #[test]
    fn only_account_bot_ids_name_the_upload_target() {
        assert!(check_account_bot_id("bot_tz4a98xxat96iws9zmbrgj3a").is_ok());
        for bad in [
            "",
            "bot_",
            "..",
            "bot_../apps",
            "bot_a/b",
            "user-1",
            "bot_a%2F",
        ] {
            let error = check_account_bot_id(bad).unwrap_err();
            assert_eq!(error.code, codes::INVALID, "{bad}");
        }
    }
}
