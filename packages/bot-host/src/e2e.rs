//! End-to-end encrypted conversations for account bots with the `encryption` capability.
//!
//! The private key lives in this desktop's keychain, never on the server. The host publishes the
//! public key as the bot, opens the sealed entries a turn reads and seals the reply (or failure
//! text) to every active member's published key. Nothing readable of such a turn is sent to the
//! hub, and the turn gets no hub memory.

use crate::hub::{HubFailure, TurnHub};
use flow_like_device_crypto::inbox::{
    ALGORITHM, InboxKeyVault, InboxKeyring, PublicInboxKey, Recipient, Scope, seal_all,
};
use flow_like_local_bots::InboxEntry;
use flow_like_types::bot_turn::BotRunCheckpoint;
use flow_like_types::inbox::{EnvelopeKind, check_envelope, plain_envelope};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex, PoisonError},
};

pub const CODE_E2E_UNAVAILABLE: &str = "BOT_E2E_UNAVAILABLE";
pub const CODE_E2E_UNREADABLE: &str = "BOT_E2E_UNREADABLE";
pub const CODE_E2E_MEMBER_KEY: &str = "BOT_E2E_MEMBER_KEY";
const ENCRYPTION_CAPABILITY: &str = "encryption";
const KEY_LOOKUP_LIMIT: usize = 50;

/// Why a turn in an encrypted conversation can't continue; nothing is posted for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct E2eFailure {
    pub code: &'static str,
    pub message: String,
}

impl E2eFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// What the bot panel shows of a bot's encryption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct E2eStatus {
    pub enabled: bool,
    pub key_id: Option<String>,
    pub fingerprint: Option<String>,
    pub published: bool,
    pub host_id: String,
}

pub fn is_sealed(entry: &InboxEntry) -> bool {
    matches!(check_envelope(&entry.envelope), Ok(EnvelopeKind::Sealed))
}

pub struct E2eHost {
    hub: Arc<dyn TurnHub>,
    keyring: InboxKeyring,
    host_id: String,
    enabled: Mutex<Option<HashSet<String>>>,
    published: Mutex<HashSet<(String, String)>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl E2eHost {
    pub fn new(hub: Arc<dyn TurnHub>, vault: Arc<dyn InboxKeyVault>, host_id: String) -> Self {
        Self {
            hub,
            keyring: InboxKeyring::new(vault),
            host_id,
            enabled: Mutex::new(None),
            published: Mutex::default(),
        }
    }

    async fn encryption_bots(&self) -> Result<HashSet<String>, HubFailure> {
        let list = self.hub.read("bots", Vec::new()).await?;
        Ok(list
            .get("bots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|bot| bot.get("location").and_then(Value::as_str) == Some("desktop"))
            .filter(|bot| {
                bot.get("capabilities")
                    .and_then(Value::as_array)
                    .is_some_and(|all| {
                        all.iter()
                            .any(|capability| capability.as_str() == Some(ENCRYPTION_CAPABILITY))
                    })
            })
            .filter_map(|bot| bot.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect())
    }

    async fn publish(&self, bot_id: &str, key: &PublicInboxKey) -> Result<(), HubFailure> {
        let token = (bot_id.to_owned(), key.key_id.clone());
        if lock(&self.published).contains(&token) {
            return Ok(());
        }
        self.hub
            .put(
                "inbox/keys",
                Vec::new(),
                json!({
                    "principal_id": bot_id,
                    "key_id": key.key_id,
                    "algorithm": key.algorithm,
                    "public_key": key.public_key,
                }),
            )
            .await?;
        lock(&self.published).insert(token);
        Ok(())
    }

    /// Makes and publishes the key of every encryption bot that lives here; the set of bots
    /// that may take part in encrypted conversations is the one the hub just listed.
    pub async fn sync(&self) -> Result<(), HubFailure> {
        let bots = self.encryption_bots().await?;
        for bot_id in &bots {
            match self.keyring.ensure(bot_id) {
                Ok((key, _)) => self.publish(bot_id, &key).await?,
                Err(error) => {
                    tracing::warn!(%error, bot_id, "Bot host: an encryption key could not be made")
                }
            }
        }
        *lock(&self.enabled) = Some(bots);
        Ok(())
    }

    async fn is_enabled(&self, bot_id: &str) -> bool {
        let known = lock(&self.enabled)
            .as_ref()
            .is_some_and(|bots| bots.contains(bot_id));
        if known {
            return true;
        }
        self.sync().await.is_ok()
            && lock(&self.enabled)
                .as_ref()
                .is_some_and(|bots| bots.contains(bot_id))
    }

    /// A new key for `bot_id`: published, the previous one revoked. Older mail still opens.
    pub async fn rotate(&self, bot_id: &str) -> Result<E2eStatus, String> {
        if !self.is_enabled(bot_id).await {
            return Err(format!(
                "'{bot_id}' is not a desktop bot with the encryption capability"
            ));
        }
        let rotation = self
            .keyring
            .rotate(bot_id)
            .map_err(|error| format!("The key could not be rotated: {error}"))?;
        self.publish(bot_id, &rotation.current)
            .await
            .map_err(|failure| format!("The new key could not be published: {failure}"))?;
        if let Some(retired) = rotation.retired {
            let revoked = self
                .hub
                .delete(
                    &format!("inbox/keys/{retired}"),
                    vec![("principal_id", bot_id.to_owned())],
                )
                .await;
            if let Err(failure) = revoked {
                tracing::warn!(%failure, bot_id, "Bot host: the retired key stays listed");
            }
        }
        Ok(self.status(bot_id))
    }

    pub fn status(&self, bot_id: &str) -> E2eStatus {
        let enabled = lock(&self.enabled)
            .as_ref()
            .is_some_and(|bots| bots.contains(bot_id));
        let key = self.keyring.active(bot_id).ok().flatten();
        let published = key.as_ref().is_some_and(|key| {
            lock(&self.published).contains(&(bot_id.to_owned(), key.key_id.clone()))
        });
        E2eStatus {
            enabled,
            key_id: key.as_ref().map(|key| key.key_id.clone()),
            fingerprint: key.map(|key| key.fingerprint),
            published,
            host_id: self.host_id.clone(),
        }
    }

    /// The entries of an encrypted turn as plain ones: sealed entries opened with the bot's
    /// keys, those that don't open left out. The trigger has to open.
    pub async fn open_entries(
        &self,
        bot_id: &str,
        trigger_message_id: &str,
        entries: Vec<InboxEntry>,
    ) -> Result<Vec<InboxEntry>, E2eFailure> {
        if !self.is_enabled(bot_id).await {
            return Err(E2eFailure::new(
                CODE_E2E_UNAVAILABLE,
                "This bot holds no encryption key on this desktop",
            ));
        }
        let mut opened = Vec::with_capacity(entries.len());
        let mut trigger_read = false;
        for mut entry in entries {
            if !is_sealed(&entry) {
                opened.push(entry);
                continue;
            }
            let Some(body) = self.open_body(bot_id, &entry) else {
                continue;
            };
            trigger_read |= entry.message_id == trigger_message_id;
            entry.envelope = plain_envelope(body);
            opened.push(entry);
        }
        if !trigger_read {
            return Err(E2eFailure::new(
                CODE_E2E_UNREADABLE,
                "The message this turn answers isn't sealed to a key this desktop holds",
            ));
        }
        Ok(opened)
    }

    /// Opens a later window of the same conversation without requiring the original trigger
    /// to remain in that window. Entries this host cannot decrypt are left out.
    pub(crate) async fn open_updates(
        &self,
        bot_id: &str,
        entries: Vec<InboxEntry>,
    ) -> Result<Vec<InboxEntry>, E2eFailure> {
        if !self.is_enabled(bot_id).await {
            return Err(E2eFailure::new(
                CODE_E2E_UNAVAILABLE,
                "This bot holds no encryption key on this desktop",
            ));
        }
        Ok(entries
            .into_iter()
            .filter_map(|mut entry| {
                if is_sealed(&entry) {
                    entry.envelope = plain_envelope(self.open_body(bot_id, &entry)?);
                }
                Some(entry)
            })
            .collect())
    }

    fn open_body(&self, bot_id: &str, entry: &InboxEntry) -> Option<Value> {
        let scope = Scope {
            conversation_id: &entry.conversation_id,
            message_id: &entry.message_id,
            sender_id: &entry.sender_id,
        };
        let plaintext = self.keyring.open(bot_id, &scope, &entry.envelope).ok()?;
        serde_json::from_slice(&plaintext).ok()
    }

    /// Keep recovery state readable only by this bot's retained inbox keys. The authenticated
    /// scope separates checkpoints from messages and prevents moving one to another turn.
    pub async fn seal_checkpoint(
        &self,
        bot_id: &str,
        conversation_id: &str,
        turn_id: &str,
        checkpoint: &BotRunCheckpoint,
    ) -> Result<BotRunCheckpoint, E2eFailure> {
        let unavailable = || {
            E2eFailure::new(
                CODE_E2E_UNAVAILABLE,
                "The recovery checkpoint could not be sealed",
            )
        };
        checkpoint.validate().map_err(|_| unavailable())?;
        let key = self
            .keyring
            .active(bot_id)
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?;
        let recipient = Recipient {
            principal_id: bot_id.to_owned(),
            key_id: key.key_id,
            public_key: key.public_key,
        };
        let message_id = format!("bot-checkpoint:{turn_id}:{}", checkpoint.revision);
        let scope = Scope {
            conversation_id,
            message_id: &message_id,
            sender_id: bot_id,
        };
        let plaintext = serde_json::to_vec(checkpoint).map_err(|_| unavailable())?;
        let envelope = flow_like_device_crypto::inbox::seal(&scope, &recipient, &plaintext)
            .map_err(|_| unavailable())?;
        let sealed = BotRunCheckpoint {
            state: json!({"sealed_checkpoint": 1, "envelope": envelope}),
            ..checkpoint.clone()
        };
        sealed.validate().map_err(|_| unavailable())?;
        Ok(sealed)
    }

    /// Open a saved checkpoint and verify the public storage metadata against its authenticated
    /// copy. A missing key or unreadable record must never restart the request from scratch.
    pub async fn open_checkpoint(
        &self,
        bot_id: &str,
        conversation_id: &str,
        turn_id: &str,
        checkpoint: &BotRunCheckpoint,
    ) -> Result<BotRunCheckpoint, E2eFailure> {
        let unreadable = || {
            E2eFailure::new(
                CODE_E2E_UNREADABLE,
                "The recovery checkpoint could not be opened",
            )
        };
        checkpoint.validate().map_err(|_| unreadable())?;
        if checkpoint
            .state
            .get("sealed_checkpoint")
            .and_then(Value::as_u64)
            != Some(1)
        {
            return Err(unreadable());
        }
        let envelope = checkpoint.state.get("envelope").ok_or_else(unreadable)?;
        let message_id = format!("bot-checkpoint:{turn_id}:{}", checkpoint.revision);
        let scope = Scope {
            conversation_id,
            message_id: &message_id,
            sender_id: bot_id,
        };
        let plaintext = self
            .keyring
            .open(bot_id, &scope, envelope)
            .map_err(|_| unreadable())?;
        let opened: BotRunCheckpoint =
            serde_json::from_slice(&plaintext).map_err(|_| unreadable())?;
        opened.validate().map_err(|_| unreadable())?;
        if opened.revision != checkpoint.revision
            || opened.started_at_ms != checkpoint.started_at_ms
            || opened.deadline_at_ms != checkpoint.deadline_at_ms
        {
            return Err(unreadable());
        }
        Ok(opened)
    }

    async fn recipients(
        &self,
        bot_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<Recipient>, E2eFailure> {
        let hub = |failure: HubFailure| {
            E2eFailure::new(
                CODE_E2E_UNAVAILABLE,
                format!("The members' keys could not be read: {failure}"),
            )
        };
        let conversation = self
            .hub
            .read(
                &format!("inbox/conversations/{conversation_id}"),
                vec![("as", bot_id.to_owned())],
            )
            .await
            .map_err(hub)?;
        let members: Vec<String> = conversation
            .get("members")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|member| member.get("principal_id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        if members.is_empty() || members.len() > KEY_LOOKUP_LIMIT {
            return Err(E2eFailure::new(
                CODE_E2E_UNAVAILABLE,
                format!("The conversation lists {} members", members.len()),
            ));
        }
        let listed = self
            .hub
            .read("inbox/keys", vec![("principals", members.join(","))])
            .await
            .map_err(hub)?;
        let mut newest: HashMap<&str, Recipient> = HashMap::new();
        for key in listed
            .get("keys")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let field = |name: &str| key.get(name).and_then(Value::as_str);
            let (Some(principal), Some(key_id), Some(public_key), Some(ALGORITHM)) = (
                field("principal_id"),
                field("key_id"),
                field("public_key"),
                field("algorithm"),
            ) else {
                continue;
            };
            if let Some(member) = members.iter().find(|member| member.as_str() == principal) {
                newest.insert(
                    member,
                    Recipient {
                        principal_id: principal.to_owned(),
                        key_id: key_id.to_owned(),
                        public_key: public_key.to_owned(),
                    },
                );
            }
        }
        members
            .iter()
            .map(|member| {
                newest.remove(member.as_str()).ok_or_else(|| {
                    E2eFailure::new(
                        CODE_E2E_MEMBER_KEY,
                        format!("'{member}' has published no key to seal to"),
                    )
                })
            })
            .collect()
    }

    /// `body` sealed once per active member, ready for the `envelopes` of a send.
    pub async fn seal(
        &self,
        bot_id: &str,
        conversation_id: &str,
        message_id: &str,
        body: &Value,
    ) -> Result<BTreeMap<String, Value>, E2eFailure> {
        let recipients = self.recipients(bot_id, conversation_id).await?;
        let plaintext = serde_json::to_vec(body)
            .map_err(|_| E2eFailure::new(CODE_E2E_UNAVAILABLE, "The reply could not be encoded"))?;
        let scope = Scope {
            conversation_id,
            message_id,
            sender_id: bot_id,
        };
        seal_all(&scope, &recipients, &plaintext).map_err(|error| {
            E2eFailure::new(
                CODE_E2E_UNAVAILABLE,
                format!("Sealing the reply failed: {error}"),
            )
        })
    }
}
