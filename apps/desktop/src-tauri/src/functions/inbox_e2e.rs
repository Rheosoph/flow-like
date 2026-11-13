//! The signed-in person's end-to-end inbox key on this device. The seed is made on first use,
//! kept in the OS keychain and never leaves this process: the webview gets the public half to
//! register, seals with published keys and gets opened bodies back.

use crate::bot_host::e2e_vault::KeychainVault;
use flow_like_device_crypto::inbox::{
    InboxKeyring, Member, PublicInboxKey, RecipientKey, Scope, SecretScope, open_with, seal_all,
    seal_secret,
};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use tauri::State;

pub struct PersonKeys(Arc<InboxKeyring>);

impl Default for PersonKeys {
    fn default() -> Self {
        Self(Arc::new(InboxKeyring::new(Arc::new(KeychainVault::PEOPLE))))
    }
}

/// A sealed entry addressed to the reader.
#[derive(Deserialize)]
pub struct SealedEntry {
    conversation_id: String,
    message_id: String,
    sender_id: String,
    envelope: Value,
}

impl SealedEntry {
    fn scope(&self) -> Scope<'_> {
        Scope {
            conversation_id: &self.conversation_id,
            message_id: &self.message_id,
            sender_id: &self.sender_id,
        }
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("The inbox key task stopped: {error}"))?
}

/// This device's active key for `principal_id`, made and stored on first use.
#[tauri::command]
pub async fn inbox_e2e_key(
    keys: State<'_, PersonKeys>,
    principal_id: String,
) -> Result<PublicInboxKey, String> {
    let ring = keys.0.clone();
    blocking(move || {
        ring.ensure(&principal_id)
            .map(|(key, _)| key)
            .map_err(|error| format!("The inbox key of '{principal_id}' is unavailable: {error}"))
    })
    .await
}

/// `body` sealed once per member, to every key each member published.
#[tauri::command]
pub async fn inbox_e2e_seal(
    conversation_id: String,
    message_id: String,
    sender_id: String,
    members: Vec<Member>,
    body: Value,
) -> Result<BTreeMap<String, Value>, String> {
    blocking(move || {
        let plaintext = serde_json::to_vec(&body)
            .map_err(|error| format!("Message '{message_id}' could not be encoded: {error}"))?;
        let scope = Scope {
            conversation_id: &conversation_id,
            message_id: &message_id,
            sender_id: &sender_id,
        };
        seal_all(&scope, &members, &plaintext)
            .map_err(|error| format!("Sealing message '{message_id}' failed: {error}"))
    })
    .await
}

/// A bot secret's `value` sealed to the bot's published `keys`, bound to its bot id and name; the
/// host holding the bot's key opens it, the server cannot.
#[tauri::command]
pub async fn inbox_e2e_seal_secret(
    bot_id: String,
    name: String,
    keys: Vec<RecipientKey>,
    value: String,
) -> Result<Value, String> {
    let value = zeroize::Zeroizing::new(value);
    blocking(move || {
        let scope = SecretScope {
            bot_id: &bot_id,
            name: &name,
        };
        seal_secret(&scope, &keys, value.as_bytes())
            .map_err(|error| format!("Sealing secret '{name}' of bot '{bot_id}' failed: {error}"))
    })
    .await
}

/// The bodies of `entries` sent to `principal_id`, opened with this device's keys; `null` where
/// an entry is sealed to none of them.
#[tauri::command]
pub async fn inbox_e2e_open(
    keys: State<'_, PersonKeys>,
    principal_id: String,
    entries: Vec<SealedEntry>,
) -> Result<Vec<Option<Value>>, String> {
    let ring = keys.0.clone();
    blocking(move || {
        let secrets = ring.secrets(&principal_id).map_err(|error| {
            format!("The inbox keys of '{principal_id}' are unavailable: {error}")
        })?;
        Ok(entries
            .iter()
            .map(|entry| {
                open_with(&secrets, &entry.scope(), &principal_id, &entry.envelope)
                    .ok()
                    .and_then(|plaintext| serde_json::from_slice(&plaintext).ok())
            })
            .collect())
    })
    .await
}
