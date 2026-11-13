//! Interactive chat runs of this desktop: while a window streams the answer to a message it
//! marked `interactive`, the host leaves that message alone. A run that ends without its reply
//! hands the message back after the grace period.

use super::{BotHost, rule::Hold};
use chrono::{Duration, Utc};
use flow_like_types::bot_turn::INTERACTIVE_GRACE_SECS;
use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};
use tauri::{AppHandle, Manager};

type Key = (String, String);

static RUNS: Mutex<Option<HashMap<Key, usize>>> = Mutex::new(None);

fn runs() -> MutexGuard<'static, Option<HashMap<Key, usize>>> {
    RUNS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn is_live(bot_id: &str, conversation_id: &str) -> bool {
    runs()
        .as_ref()
        .is_some_and(|runs| runs.contains_key(&(bot_id.to_owned(), conversation_id.to_owned())))
}

pub fn hold_now(bot_id: &str, conversation_id: &str) -> Hold {
    Hold {
        now: Utc::now().fixed_offset(),
        grace: Duration::seconds(INTERACTIVE_GRACE_SECS),
        live: is_live(bot_id, conversation_id),
    }
}

/// Registered while an interactive run streams; dropping it hands the conversation back to the
/// host now and again once the grace has passed.
pub struct RunGuard {
    key: Key,
    app: Option<AppHandle>,
}

pub fn begin(app: Option<&AppHandle>, bot_id: &str, conversation_id: &str) -> RunGuard {
    let key = (bot_id.to_owned(), conversation_id.to_owned());
    *runs()
        .get_or_insert_with(HashMap::new)
        .entry(key.clone())
        .or_insert(0) += 1;
    RunGuard {
        key,
        app: app.cloned(),
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        {
            let mut guard = runs();
            if let Some(runs) = guard.as_mut()
                && let Some(count) = runs.get_mut(&self.key)
            {
                *count -= 1;
                if *count == 0 {
                    runs.remove(&self.key);
                }
            }
        }
        let Some(app) = self.app.take() else { return };
        let conversation_id = self.key.1.clone();
        tauri::async_runtime::spawn(async move {
            poke(&app, &conversation_id);
            tokio::time::sleep(
                std::time::Duration::from_secs(INTERACTIVE_GRACE_SECS.unsigned_abs())
                    + std::time::Duration::from_secs(1),
            )
            .await;
            poke(&app, &conversation_id);
        });
    }
}

fn poke(app: &AppHandle, conversation_id: &str) {
    if let Some(host) = app.try_state::<BotHost>() {
        host.poke(Some(conversation_id));
    }
}

use super::{
    hub::TurnHub,
    memory::{hub_source_for, source_for},
    store::HostStore,
};
use flow_like::flow::copilot::{MemoryTurnSource, ReplyFiles, ToolTrail};
use flow_like_local_bots::{InboxEntry, SendEntryRequest};
use flow_like_types::{
    bot_message::AttachmentPart,
    bot_turn::{INTERACTIVE_HOST_PREFIX, reply_message_id, turn_id},
    inbox::{InboxEntryKind, plain_envelope},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

/// The loop that extends a hub hold; it stops when the owner of the run lets go of it.
pub struct HoldLoop(tokio::task::JoinHandle<()>);

impl HoldLoop {
    pub fn abort(&self) {
        self.0.abort();
    }
}

impl Drop for HoldLoop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

const HOLD_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// The message an interactive run answers, as the window names it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractiveArgs {
    pub conversation_id: String,
    pub trigger_message_id: String,
    pub client_run_id: String,
}

enum Home {
    Local(Arc<dyn HostStore>),
    Account(Arc<dyn TurnHub>),
}

/// An interactive run's way to the bot's inbox: the local store for a conversation on this
/// computer, the hub for an account conversation.
pub struct InteractiveRun {
    pub usage: flow_like::flow::copilot::bot_usage::BotUsage,
    bot_id: String,
    args: InteractiveArgs,
    home: Home,
    encryption: Option<Arc<super::e2e::E2eHost>>,
    notes: Option<flow_like::flow::copilot::bot_notes::NotesHandle>,
    trail: Option<ToolTrail>,
    reply_files: ReplyFiles,
}

impl InteractiveRun {
    pub async fn open(
        store: Arc<dyn HostStore>,
        hub: Option<Arc<dyn TurnHub>>,
        bot_id: &str,
        args: InteractiveArgs,
    ) -> Option<Self> {
        let local = store
            .conversation(&args.conversation_id)
            .await
            .ok()?
            .is_some();
        let home = if local {
            Home::Local(store)
        } else {
            Home::Account(hub?)
        };
        Some(Self {
            usage: Default::default(),
            bot_id: bot_id.to_owned(),
            args,
            home,
            encryption: None,
            notes: None,
            trail: None,
            reply_files: ReplyFiles::new(),
        })
    }

    pub fn conversation_id(&self) -> &str {
        &self.args.conversation_id
    }

    /// The conversation's newest entries around the trigger, newest first.
    async fn entries(&self) -> Result<Vec<InboxEntry>, String> {
        match &self.home {
            Home::Local(store) => store.newest(&self.args.conversation_id).await,
            Home::Account(hub) => hub
                .entries_for_turn(
                    &self.bot_id,
                    &self.args.conversation_id,
                    &self.args.trigger_message_id,
                )
                .await
                .map_err(|error| error.to_string()),
        }
    }

    /// Read the stored trigger rather than accepting a memory policy from the window.
    pub async fn ephemeral_user_context(&self) -> Result<bool, String> {
        let entries = self.entries().await?;
        Ok(
            trigger_of(entries, &self.args.trigger_message_id).is_some_and(|entry| {
                entry.envelope.get("body").is_some_and(|body| {
                    flow_like_types::bot_user_context::response_for(
                        body,
                        &entry.sender_id,
                        &self.bot_id,
                    )
                })
            }),
        )
    }

    /// The files the stored trigger carries that were uploaded to this conversation.
    pub async fn trigger_files(&self) -> Vec<AttachmentPart> {
        let Ok(entries) = self.entries().await else {
            return Vec::new();
        };
        trigger_of(entries, &self.args.trigger_message_id)
            .and_then(|entry| super::rule::parse_body(&entry))
            .map(|body| {
                body.conversation_attachments(&self.args.conversation_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the stored trigger is a spoken transcript: the run is a turn of a live call.
    pub async fn live_call(&self) -> Result<bool, String> {
        let entries = self.entries().await?;
        Ok(trigger_of(entries, &self.args.trigger_message_id)
            .is_some_and(|entry| entry.kind == InboxEntryKind::Transcript))
    }

    /// Session snapshots of user context still usable up to the trigger, for a turn whose
    /// history comes from memory.
    pub async fn shared_context(&self) -> Option<String> {
        let mut entries = self.entries().await.ok()?;
        entries.reverse();
        super::transcript::shared_context(&entries, &self.args.trigger_message_id, &self.bot_id)
    }

    /// Whether legacy owner memory is available, and whether it must be read-only. This is
    /// checked independently of the trigger position, which may not have arrived yet.
    pub async fn legacy_memory_access(&self) -> Option<bool> {
        match &self.home {
            Home::Local(store) => {
                let conversation = store
                    .conversation(&self.args.conversation_id)
                    .await
                    .ok()??;
                if super::memory::is_owner_direct(&conversation, &self.bot_id) {
                    return Some(false);
                }
                if conversation.kind != flow_like_types::inbox::ConversationKind::Group {
                    return None;
                }
                store
                    .room_limits(&conversation.id, &self.bot_id)
                    .await
                    .ok()
                    .flatten()
                    .filter(|limits| limits.use_owner_memory)
                    .map(|_| true)
            }
            Home::Account(hub) => {
                super::memory::hub_memory_access(hub, &self.bot_id, &self.args.conversation_id)
                    .await
                    .filter(|access| access.use_owner_memory)
                    .map(|access| access.room)
            }
        }
    }

    /// The bot's notes for this run, unless the conversation is sealed. The reply carries the
    /// note the run touched. No live status: the person watches the stream.
    pub async fn notes(&mut self) -> Option<flow_like::flow::copilot::bot_notes::NotesHandle> {
        let handle = match &self.home {
            Home::Local(store) => {
                let entries = store.newest(&self.args.conversation_id).await.ok()?;
                super::notes::LocalNotes::handle(
                    store.clone(),
                    &self.bot_id,
                    &self.args.conversation_id,
                    None,
                    flow_like_bot_host::transcript::topic_at(&entries),
                )?
            }
            Home::Account(hub) => {
                let entries = hub
                    .entries_for_turn(
                        &self.bot_id,
                        &self.args.conversation_id,
                        &self.args.trigger_message_id,
                    )
                    .await
                    .ok()?;
                let trigger = trigger_of(entries.clone(), &self.args.trigger_message_id)?;
                if flow_like_bot_host::e2e::is_sealed(&trigger) {
                    return None;
                }
                flow_like_bot_host::notes::HubNotes::new(
                    hub.clone(),
                    &self.bot_id,
                    &self.args.conversation_id,
                    None,
                )
                .handle(flow_like_bot_host::transcript::topic_at(&entries))
            }
        };
        self.notes = Some(handle.clone());
        Some(handle)
    }

    /// The run's tool trail: the bot's earlier calls in this direct conversation for the prompt,
    /// and this run's calls for the reply. Sealed replies stay out of the section.
    pub async fn trail(&mut self) -> Option<ToolTrail> {
        let mut entries = self.entries().await.ok()?;
        entries.reverse();
        let upto = entries
            .iter()
            .position(|entry| entry.message_id == self.args.trigger_message_id)
            .map_or(entries.len(), |index| index + 1);
        let trail = ToolTrail::new(flow_like_bot_host::transcript::tool_trail(
            &self.bot_id,
            &entries[..upto],
        ));
        self.trail = Some(trail.clone());
        Some(trail)
    }

    /// The files the run attaches to its reply.
    pub fn reply_files(&self) -> ReplyFiles {
        self.reply_files.clone()
    }

    pub fn host_id(&self) -> String {
        format!("{INTERACTIVE_HOST_PREFIX}{}", self.args.client_run_id)
    }

    fn turn_id(&self) -> String {
        turn_id(&self.bot_id, &self.args.trigger_message_id)
    }

    /// Bind foreground work to the same checkpoint a background host will read after restart.
    pub async fn recovery(
        &mut self,
        encryption: Option<Arc<super::e2e::E2eHost>>,
    ) -> Result<Option<Arc<dyn flow_like::flow::copilot::run_recovery::BotRunRecovery>>, String>
    {
        let deadline = Utc::now().timestamp_millis()
            + flow_like_types::bot_turn::DEFAULT_TURN_DEADLINE_SECS as i64 * 1000;
        match &self.home {
            Home::Local(store) if store.supports_recovery() => {
                Ok(Some(Arc::new(super::recovery::LocalRecovery::new(
                    store.clone(),
                    &self.bot_id,
                    &self.args.conversation_id,
                    &self.args.trigger_message_id,
                    deadline,
                ))))
            }
            Home::Account(hub) if hub.supports_recovery() => {
                let held = hub
                    .write(
                        &format!("bots/turns/{}/hold", self.turn_id()),
                        Vec::new(),
                        json!({}),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                if held.get("held").and_then(Value::as_bool) != Some(true) {
                    return Err("This bot turn is already being answered or has finished".into());
                }
                let value = hub
                    .read(
                        &format!(
                            "inbox/conversations/{}/turns/{}",
                            self.args.conversation_id,
                            self.turn_id()
                        ),
                        Vec::new(),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                let turn: flow_like_types::bot_turn::BotTurn = serde_json::from_value(value)
                    .map_err(|_| "The bot turn could not be read".to_string())?;
                if turn.bot_id != self.bot_id
                    || turn.conversation_id != self.args.conversation_id
                    || turn.trigger_message_id != self.args.trigger_message_id
                    || turn.status != flow_like_types::bot_turn::TurnStatus::Queued
                    || turn.attempts != 0
                    || turn
                        .host_id
                        .as_deref()
                        .is_some_and(|host| host != self.host_id())
                {
                    return Err("This bot turn is already being answered or has finished".into());
                }
                let entries = hub
                    .entries_for_turn(
                        &self.bot_id,
                        &self.args.conversation_id,
                        &self.args.trigger_message_id,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                let trigger = entries
                    .iter()
                    .find(|entry| entry.message_id == self.args.trigger_message_id)
                    .ok_or_else(|| {
                        "The message this bot turn answers could not be read".to_string()
                    })?;
                if flow_like_bot_host::e2e::is_sealed(trigger) {
                    let encryption = encryption
                        .ok_or_else(|| "This bot's encryption keys are unavailable".to_string())?;
                    encryption
                        .open_entries(&self.bot_id, &self.args.trigger_message_id, entries)
                        .await
                        .map_err(|error| error.message)?;
                    self.encryption = Some(encryption);
                }
                // The hub keeps checkpoints only for desktop turns: a server or device turn is
                // recovered by its own host, so this chat answers it without a checkpoint.
                if turn.host_kind != flow_like_types::bot_turn::TurnHostKind::Desktop {
                    return Ok(None);
                }
                let mut recovery = flow_like_bot_host::recovery::HubRecovery::new(
                    hub.clone(),
                    &turn,
                    &self.host_id(),
                    deadline,
                );
                if let Some(encryption) = &self.encryption {
                    recovery = recovery
                        .with_encryption(encryption.clone(), self.args.conversation_id.clone());
                }
                Ok(Some(Arc::new(recovery)))
            }
            _ => Ok(None),
        }
    }

    async fn cancel(&self) -> Result<(), String> {
        match &self.home {
            Home::Local(store) => {
                store
                    .cancel_run(
                        &self.bot_id,
                        &self.args.conversation_id,
                        &self.args.trigger_message_id,
                    )
                    .await
            }
            Home::Account(hub) => hub
                .cancel_turn(&self.bot_id, &self.turn_id())
                .await
                .map_err(|error| error.to_string()),
        }
    }

    pub async fn memory(&self) -> Option<MemoryTurnSource> {
        match &self.home {
            Home::Local(store) => {
                let conversation = store
                    .conversation(&self.args.conversation_id)
                    .await
                    .ok()??;
                let trigger = trigger_of(
                    store.newest(&self.args.conversation_id).await.ok()?,
                    &self.args.trigger_message_id,
                );
                source_for(store, &conversation, &self.bot_id, trigger.as_ref()).await
            }
            Home::Account(hub) => {
                let entries = hub
                    .entries_for_turn(
                        &self.bot_id,
                        &self.args.conversation_id,
                        &self.args.trigger_message_id,
                    )
                    .await
                    .ok()?;
                let seq = trigger_of(entries, &self.args.trigger_message_id)?.seq;
                hub_source_for(
                    hub,
                    hub,
                    &self.bot_id,
                    &self.args.conversation_id,
                    seq,
                    None,
                )
                .await
            }
        }
    }

    /// Keeps the hub's hosts away while the run streams; local runs are held by [`begin`].
    pub fn hold(&self) -> Option<HoldLoop> {
        let Home::Account(hub) = &self.home else {
            return None;
        };
        let (hub, path) = (hub.clone(), format!("bots/turns/{}/hold", self.turn_id()));
        Some(HoldLoop(tokio::spawn(async move {
            loop {
                match hub.write(&path, Vec::new(), json!({})).await {
                    Ok(value) if value.get("held").and_then(Value::as_bool) == Some(false) => {
                        return;
                    }
                    Ok(_) => {}
                    Err(failure) => tracing::warn!(%failure, "Interactive chat: hold failed"),
                }
                tokio::time::sleep(HOLD_EVERY).await;
            }
        })))
    }

    fn body(&self, text: &str, error_code: Option<&str>) -> Value {
        let mut body = json!({
            "text": text,
            "reply_to": self.args.trigger_message_id,
            "hops": 1,
        });
        if let Some(code) = error_code {
            body["error"] = json!({ "code": code });
            self.usage.attach(&mut body);
            return body;
        }
        if let Some(notes) = &self.notes {
            flow_like_bot_host::notes::attach_note(&mut body, notes.touched().as_ref());
        }
        self.reply_files.attach(&mut body);
        if let Some(trail) = &self.trail {
            trail.attach(&mut body);
        }
        self.usage.attach(&mut body);
        body
    }

    /// Stores the run's answer as the bot. `Ok(false)`: a host answered first.
    pub async fn store_reply(&self, text: &str, error_code: Option<&str>) -> Result<bool, String> {
        let message_id = reply_message_id(&self.bot_id, &self.args.trigger_message_id);
        let body = self.body(text, error_code);
        match &self.home {
            Home::Local(store) => {
                let request = SendEntryRequest {
                    message_id,
                    kind: InboxEntryKind::Message,
                    envelope: Some(plain_envelope(body)),
                    envelopes: None,
                };
                let sent = request
                    .envelope
                    .as_ref()
                    .and_then(|envelope| envelope["body"]["text"].as_str().map(str::to_owned));
                let stored = store
                    .send(&self.args.conversation_id, &self.bot_id, request)
                    .await?;
                Ok(stored.envelope["body"]["text"].as_str() == sent.as_deref())
            }
            Home::Account(hub) => {
                let mut message = json!({
                    "message_id": message_id,
                    "kind": "message",
                });
                if let Some(encryption) = &self.encryption {
                    message["envelopes"] = json!(
                        encryption
                            .seal(&self.bot_id, &self.args.conversation_id, &message_id, &body)
                            .await
                            .map_err(|error| error.message)?
                    );
                } else {
                    message["envelope"] = plain_envelope(body);
                }
                match hub
                    .send(
                        &self.bot_id,
                        &self.args.conversation_id,
                        message,
                        &self.turn_id(),
                        &self.host_id(),
                    )
                    .await
                {
                    Ok(()) => Ok(true),
                    Err(failure) if failure.taken() || failure.lease_lost() => Ok(false),
                    Err(failure) => Err(failure.to_string()),
                }
            }
        }
    }
}

fn trigger_of(entries: Vec<InboxEntry>, message_id: &str) -> Option<InboxEntry> {
    entries
        .into_iter()
        .find(|entry| entry.message_id == message_id)
}

/// The interactive run of a window's message to `bot_id`, `None` when its conversation is
/// unknown to both the local store and the account.
pub(crate) async fn open_for_app(
    app: &AppHandle,
    bot_id: &str,
    args: InteractiveArgs,
) -> Option<InteractiveRun> {
    InteractiveRun::open(
        Arc::new(super::store::AppStore(app.clone())),
        Some(Arc::new(super::hub::ProfileHub::new(app.clone()))),
        bot_id,
        args,
    )
    .await
}

/// What a finished run leaves behind: the reply stored as the bot, the hold released and the
/// memory told that the conversation moved on.
pub(crate) struct Finish {
    pub run: InteractiveRun,
    pub guard: RunGuard,
    pub hold: Option<HoldLoop>,
    pub memory: Option<MemoryTurnSource>,
}

impl Finish {
    pub(crate) async fn cancel(self) -> Result<(), String> {
        if let Some(hold) = &self.hold {
            hold.abort();
        }
        let result = self.run.cancel().await;
        drop(self.guard);
        result
    }

    pub(crate) async fn end(self, text: &str, error_code: Option<&str>) {
        if let Some(hold) = &self.hold {
            hold.abort();
        }
        let stored = self.run.store_reply(text, error_code).await;
        match stored {
            Ok(true) => {
                if let Some(source) = self.memory {
                    flow_like::flow::copilot::memory_open::follow_up(&self.run.bot_id, source);
                }
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, bot_id = %self.run.bot_id, "Interactive chat: the reply was not stored")
            }
        }
        drop(self.guard);
    }
}
