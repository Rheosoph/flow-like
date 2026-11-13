//! End-to-end encrypted conversations for the device's bots. Each encryption bot's private inbox
//! key is made here and kept like the device identity: a private file (owner-only, mode 0600) in
//! the agent's state directory. Only the public key leaves the device, published as the device.
//! A turn's key reads (the members of its conversation and their keys) use that turn's bot
//! token, so the device reads nothing a bot of another turn could not.

use super::hub::DeviceTurnHub;
use async_trait::async_trait;
use flow_like_bot_host::{
    e2e::E2eHost,
    hub::{HubFailure, TurnHub},
};
use flow_like_device_crypto::inbox::InboxKeyVault;
use flow_like_local_bots::InboxEntry;
use flow_like_types::{bot::BotLocation, bot_turn::BotTurn};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use zeroize::Zeroizing;

const VAULT_DIR: &str = "bot-inbox-keys";

/// The private inbox keys of the device's bots, one private file per bot.
pub struct DeviceKeyVault {
    directory: PathBuf,
}

impl DeviceKeyVault {
    pub fn open(state_dir: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            directory: crate::supervisor::prepare_state_dir(&state_dir.join(VAULT_DIR))?,
        })
    }

    /// The bot id hashed, so no id can name a path outside the vault.
    fn path(&self, owner: &str) -> PathBuf {
        let name: String = Sha256::digest(owner.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self.directory.join(format!("{name}.keys"))
    }
}

impl InboxKeyVault for DeviceKeyVault {
    fn load(&self, owner: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        let path = self.path(owner);
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "The inbox keys of bot '{owner}' could not be inspected: {error}"
                ));
            }
            Ok(_) => {}
        }
        crate::vault::read_private(&path)
            .map(Some)
            .map_err(|error| {
                format!("The inbox keys of bot '{owner}' could not be read: {error:#}")
            })
    }

    fn store(&self, owner: &str, secret: &[u8]) -> Result<(), String> {
        let failed = |error: &dyn std::fmt::Display| {
            format!("The inbox keys of bot '{owner}' could not be saved: {error}")
        };
        #[cfg(unix)]
        crate::vault::remove_abandoned_staging(&self.directory, |name| {
            name.starts_with('.') && name.ends_with(".tmp")
        });
        let temporary = self
            .directory
            .join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        crate::vault::write_new_private(&temporary, secret).map_err(|error| failed(&error))?;
        if let Err(error) = std::fs::rename(&temporary, self.path(owner)) {
            let _ = std::fs::remove_file(&temporary);
            return Err(failed(&error));
        }
        std::fs::File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| failed(&error))
    }

    fn remove(&self, owner: &str) -> Result<(), String> {
        match std::fs::remove_file(self.path(owner)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "The inbox keys of bot '{owner}' could not be deleted: {error}"
            )),
        }
    }
}

/// What an [`E2eHost`] asks of the hub, answered by the device: the bots come from the last
/// poll, keys are published as the device, and member and key reads go out with the bot token
/// of the turn the device holds in that conversation.
pub struct DeviceKeyHub {
    hub: DeviceTurnHub,
}

impl DeviceKeyHub {
    pub fn new(hub: DeviceTurnHub) -> Self {
        Self { hub }
    }

    fn bots(&self) -> Value {
        let bots: Vec<Value> = self
            .hub
            .device_bots()
            .into_iter()
            .map(|bot| {
                json!({
                    "id": bot.id,
                    "location": BotLocation::Device.as_str(),
                    "capabilities": bot.capabilities,
                })
            })
            .collect();
        json!({ "bots": bots })
    }
}

/// The encryption host of the device's bots, its keys in `state_dir`.
pub fn host(state_dir: &Path, hub: &DeviceTurnHub) -> anyhow::Result<Arc<E2eHost>> {
    let vault = Arc::new(DeviceKeyVault::open(state_dir)?);
    Ok(Arc::new(
        E2eHost::new(
            Arc::new(DeviceKeyHub::new(hub.clone())),
            vault,
            hub.host_id().to_owned(),
        )
        .at_location(BotLocation::Device.as_str()),
    ))
}

/// Publishes the keys of the device's encryption bots whenever a poll lists its bots anew, so
/// people can seal to a bot right after it got the ability or the agent started.
pub async fn publish_keys(hub: DeviceTurnHub, e2e: Arc<E2eHost>) {
    loop {
        hub.bots_changed().await;
        if let Err(failure) = e2e.sync().await {
            tracing::warn!(%failure, "Bot host: publishing the bots' inbox keys failed");
        }
    }
}

fn unsupported(what: &str) -> HubFailure {
    HubFailure::offline(format!(
        "the device's key host publishes and reads inbox keys only, not {what}"
    ))
}

fn named<'a>(query: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.as_str())
}

#[async_trait]
impl TurnHub for DeviceKeyHub {
    async fn list(&self) -> Result<Vec<BotTurn>, HubFailure> {
        Err(unsupported("turns"))
    }

    async fn claim(&self, _turn_id: &str, _host_id: &str) -> Result<BotTurn, HubFailure> {
        Err(unsupported("claims"))
    }

    async fn heartbeat(&self, _turn_id: &str, _host_id: &str) -> Result<BotTurn, HubFailure> {
        Err(unsupported("heartbeats"))
    }

    async fn entries(
        &self,
        _bot_id: &str,
        _conversation_id: &str,
    ) -> Result<Vec<InboxEntry>, HubFailure> {
        Err(unsupported("entries"))
    }

    async fn send(
        &self,
        _bot_id: &str,
        _conversation_id: &str,
        _message: Value,
        _turn_id: &str,
        _host_id: &str,
    ) -> Result<(), HubFailure> {
        Err(unsupported("replies"))
    }

    async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure> {
        Ok(Vec::new())
    }

    async fn report_host(
        &self,
        _host_id: &str,
        _bot_ids: &[String],
    ) -> Result<Vec<String>, HubFailure> {
        Err(unsupported("presence"))
    }

    async fn delete_host(&self, _host_id: &str) -> Result<(), HubFailure> {
        Err(unsupported("presence"))
    }

    async fn cancel_turn(&self, _bot_id: &str, _turn_id: &str) -> Result<(), HubFailure> {
        Err(unsupported("cancels"))
    }

    async fn read(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        if path == "bots" {
            return Ok(self.bots());
        }
        let conversation = path
            .strip_prefix("inbox/conversations/")
            .filter(|rest| !rest.contains('/'))
            .or_else(|| named(&query, "conversation_id"));
        let (Some(bot_id), Some(conversation_id)) = (named(&query, "as"), conversation) else {
            return Err(unsupported(path));
        };
        let as_bot = self.hub.as_bot_in(bot_id, conversation_id).ok_or_else(|| {
            HubFailure::offline(format!(
                "bot {bot_id} holds no turn in conversation {conversation_id} on this device"
            ))
        })?;
        as_bot.read(path, query).await
    }

    async fn put(
        &self,
        path: &str,
        _query: Vec<(&'static str, String)>,
        body: Value,
    ) -> Result<Value, HubFailure> {
        if path != "inbox/keys" {
            return Err(unsupported(path));
        }
        self.hub.publish_inbox_key(body).await
    }
}

#[cfg(all(test, unix))]
mod tests;
