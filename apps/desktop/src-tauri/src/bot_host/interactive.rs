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
    runs().as_ref().is_some_and(|runs| {
        runs.contains_key(&(bot_id.to_owned(), conversation_id.to_owned()))
    })
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
    *runs().get_or_insert_with(HashMap::new).entry(key.clone()).or_insert(0) += 1;
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
                std::time::Duration::from_secs(INTERACTIVE_GRACE_SECS.unsigned_abs()) + std::time::Duration::from_secs(1),
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
use flow_like::flow::copilot::MemoryTurnSource;
use flow_like_local_bots::{InboxEntry, SendEntryRequest};
use flow_like_types::{
    bot_turn::{INTERACTIVE_HOST_PREFIX, reply_message_id, turn_id},
    inbox::{InboxEntryKind, plain_envelope},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

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
    bot_id: String,
    args: InteractiveArgs,
    home: Home,
}

impl InteractiveRun {
    pub async fn open(
        store: Arc<dyn HostStore>,
        hub: Option<Arc<dyn TurnHub>>,
        bot_id: &str,
        args: InteractiveArgs,
    ) -> Option<Self> {
        let local = store.conversation(&args.conversation_id).await.ok()?.is_some();
        let home = if local {
            Home::Local(store)
        } else {
            Home::Account(hub?)
        };
        Some(Self {
            bot_id: bot_id.to_owned(),
            args,
            home,
        })
    }

    pub fn conversation_id(&self) -> &str {
        &self.args.conversation_id
    }

    pub fn host_id(&self) -> String {
        format!("{INTERACTIVE_HOST_PREFIX}{}", self.args.client_run_id)
    }

    fn turn_id(&self) -> String {
        turn_id(&self.bot_id, &self.args.trigger_message_id)
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
                source_for(store, &conversation, &self.bot_id, trigger.as_ref())
            }
            Home::Account(hub) => {
                let entries = hub
                    .entries(&self.bot_id, &self.args.conversation_id)
                    .await
                    .ok()?;
                let seq = trigger_of(entries, &self.args.trigger_message_id)?.seq;
                hub_source_for(hub, hub, &self.bot_id, &self.args.conversation_id, seq).await
            }
        }
    }

    /// Keeps the hub's hosts away while the run streams; local runs are held by [`begin`].
    pub fn hold(&self) -> Option<tokio::task::JoinHandle<()>> {
        let Home::Account(hub) = &self.home else {
            return None;
        };
        let (hub, path) = (
            hub.clone(),
            format!("bots/turns/{}/hold", self.turn_id()),
        );
        Some(tokio::spawn(async move {
            loop {
                match hub.write(&path, Vec::new(), json!({})).await {
                    Ok(value) if value.get("held").and_then(Value::as_bool) == Some(false) => return,
                    Ok(_) => {}
                    Err(failure) => tracing::warn!(%failure, "Interactive chat: hold failed"),
                }
                tokio::time::sleep(HOLD_EVERY).await;
            }
        }))
    }

    fn body(&self, text: &str, error_code: Option<&str>) -> Value {
        let mut body = json!({
            "text": text,
            "reply_to": self.args.trigger_message_id,
            "hops": 1,
        });
        if let Some(code) = error_code {
            body["error"] = json!({ "code": code });
        }
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
                store
                    .send(&self.args.conversation_id, &self.bot_id, request)
                    .await
                    .map(|_| true)
            }
            Home::Account(hub) => {
                let message = json!({
                    "message_id": message_id,
                    "kind": "message",
                    "envelope": plain_envelope(body),
                });
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
    pub hold: Option<tokio::task::JoinHandle<()>>,
    pub memory: Option<MemoryTurnSource>,
}

impl Finish {
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
