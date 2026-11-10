//! The hub calls a bot's turn needs. A host talks to the hub through [`TurnHub`]; tests answer
//! from a fake.

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};
use flow_like::flow::copilot::tool_grants::{GrantKey, GrantStore};
use flow_like_local_bots::InboxEntry;
use flow_like_types::{bot_message::ApprovalRequest, bot_turn::BotTurn, inbox::InboxEntryKind};
use serde::Deserialize;
use serde_json::Value;
use std::{any::Any, sync::Arc, time::Duration};

pub const CODE_LEASE_LOST: &str = "BOT_TURN_LEASE_LOST";
pub const CODE_NOT_CLAIMED: &str = "BOT_TURN_NOT_CLAIMED";
pub const CODE_SIGNED_OUT: &str = "ACCOUNT_REQUIRED";
pub const ENTRIES: u64 = 100;
pub const MAX_HOST_BOTS: usize = 100;
const MESSAGE_CHARS: usize = 300;

#[derive(Debug, Clone)]
pub struct HubFailure {
    /// `None`: the hub wasn't reached, or nobody is signed in.
    pub status: Option<u16>,
    pub code: Option<String>,
    pub message: String,
}

impl HubFailure {
    pub fn offline(message: impl Into<String>) -> Self {
        Self {
            status: None,
            code: None,
            message: message.into(),
        }
    }

    pub fn answered(status: u16, code: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            status: Some(status),
            code: code.map(str::to_owned),
            message: message.into(),
        }
    }

    /// The host no longer holds the turn: another host took it over or the lease ran out.
    /// Nobody is signed in, or the session was refused.
    pub fn signed_out(&self) -> bool {
        self.status == Some(401) || self.code.as_deref() == Some(CODE_SIGNED_OUT)
    }

    pub fn lease_lost(&self) -> bool {
        self.status == Some(409)
            && matches!(
                self.code.as_deref(),
                Some(CODE_LEASE_LOST | CODE_NOT_CLAIMED)
            )
    }

    /// Another host (or an earlier claim) has the turn, or it is no longer ours to run.
    pub fn taken(&self) -> bool {
        matches!(self.status, Some(404 | 409))
    }
}

impl std::fmt::Display for HubFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.status, &self.code) {
            (Some(status), Some(code)) => write!(f, "{status} {code}: {}", self.message),
            (Some(status), None) => write!(f, "{status}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

/// What a hub hands the host along with a claimed turn when it mints the turn's credentials
/// itself (a device hub): the bot's identity for the turn's hub calls and whatever the runner
/// needs, opaque to the host.
#[derive(Clone, Default)]
pub struct TurnSession {
    /// The bot's token for the hub during this turn.
    pub token: Option<String>,
    /// The hub to use for the turn's reads and posts; the claiming hub when `None`.
    pub hub: Option<Arc<dyn TurnHub>>,
    /// The run spec for the runner; read back with `TurnJob::claim_as`.
    pub run: Option<Arc<dyn Any + Send + Sync>>,
}

impl std::fmt::Debug for TurnSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TurnSession(..)")
    }
}

#[derive(Debug, Clone)]
pub struct Claimed {
    pub turn: BotTurn,
    /// `None`: the host asks [`TurnHub::runtime_token`] for the turn's bot token.
    pub session: Option<TurnSession>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitOutcome {
    Parked,
    AlreadyAnswered,
    Unavailable,
    Superseded,
}

#[async_trait]
pub trait TurnHub: Send + Sync + 'static {
    async fn previous_monitor_result(
        &self,
        _bot_id: &str,
        _turn_id: &str,
        _host_id: &str,
        _attempt: u32,
    ) -> Result<Option<String>, HubFailure> {
        Ok(None)
    }
    fn supports_recovery(&self) -> bool {
        false
    }
    async fn checkpoint_load(
        &self,
        _bot_id: &str,
        _turn_id: &str,
        _host_id: &str,
        _attempt: u32,
    ) -> Result<Option<flow_like_types::bot_turn::BotRunCheckpoint>, HubFailure> {
        Err(HubFailure::offline("This hub cannot recover bot turns"))
    }
    async fn checkpoint_save(
        &self,
        _bot_id: &str,
        _turn_id: &str,
        _host_id: &str,
        _attempt: u32,
        _checkpoint: &flow_like_types::bot_turn::BotRunCheckpoint,
    ) -> Result<(), HubFailure> {
        Err(HubFailure::offline("This hub cannot recover bot turns"))
    }
    async fn list(&self) -> Result<Vec<BotTurn>, HubFailure>;
    async fn claim(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure>;
    /// Claims the turn together with the credentials and run spec the hub returns with the claim.
    async fn claim_turn(&self, turn_id: &str, host_id: &str) -> Result<Claimed, HubFailure> {
        Ok(Claimed {
            turn: self.claim(turn_id, host_id).await?,
            session: None,
        })
    }
    /// How long the hub asks hosts to wait before the next poll; the host's own interval when `None`.
    fn next_poll(&self) -> Option<Duration> {
        None
    }
    /// Ends a held turn failed with `code` without posting anything: it can never run here.
    async fn abandon(&self, _turn_id: &str, _host_id: &str, _code: &str) -> Result<(), HubFailure> {
        Ok(())
    }
    /// Gives a claimed turn back unanswered, failed with `code`, so the hub queues it again
    /// with a backoff. `true` when it is queued again; `false` when its retries are spent and the
    /// host still holds it, so the host reports the failure.
    async fn release(
        &self,
        _turn_id: &str,
        _host_id: &str,
        _code: &str,
    ) -> Result<bool, HubFailure> {
        Err(HubFailure::offline("This hub cannot release turns"))
    }
    async fn heartbeat(&self, turn_id: &str, host_id: &str) -> Result<BotTurn, HubFailure>;
    /// The newest entries of the bot's copy of the conversation, newest first.
    async fn entries(
        &self,
        bot_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<InboxEntry>, HubFailure>;
    /// Include the original trigger when resumed work has fallen outside recent history.
    async fn entries_for_turn(
        &self,
        bot_id: &str,
        conversation_id: &str,
        trigger_message_id: &str,
    ) -> Result<Vec<InboxEntry>, HubFailure> {
        let mut entries = self.entries(bot_id, conversation_id).await?;
        if entries
            .iter()
            .any(|entry| entry.message_id == trigger_message_id)
            || !self.supports_recovery()
        {
            return Ok(entries);
        }
        let page = self
            .read(
                &format!("inbox/conversations/{conversation_id}/entries"),
                vec![
                    ("as", bot_id.to_owned()),
                    ("message_id", trigger_message_id.to_owned()),
                ],
            )
            .await?;
        let original = entries_of(page)?;
        if original.iter().any(|entry| {
            entry.conversation_id != conversation_id || entry.message_id != trigger_message_id
        }) || original.len() > 1
        {
            return Err(HubFailure::offline(
                "The hub returned an unrelated turn trigger",
            ));
        }
        entries.extend(original);
        entries.sort_by(|a, b| {
            b.seq
                .cmp(&a.seq)
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(entries)
    }
    async fn send(
        &self,
        bot_id: &str,
        conversation_id: &str,
        message: Value,
        turn_id: &str,
        host_id: &str,
    ) -> Result<(), HubFailure>;
    /// The ids of the signed-in account's bots that run on a desktop, at most [`MAX_HOST_BOTS`].
    async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure>;
    /// What this host reports to the hub: [`Self::desktop_bots`] plus the server bots that hand
    /// their turns to the owner's desktop while one is online (Flux). Their tasks stay the
    /// server's, so only the report lists them.
    async fn reported_bots(&self) -> Result<Vec<String>, HubFailure> {
        self.desktop_bots().await
    }
    /// Reports the bots this host runs; answers the ids the hub refused.
    async fn report_host(
        &self,
        host_id: &str,
        bot_ids: &[String],
    ) -> Result<Vec<String>, HubFailure>;
    async fn delete_host(&self, host_id: &str) -> Result<(), HubFailure>;
    /// The owner's cancel of a turn, whoever holds it.
    async fn cancel_turn(&self, bot_id: &str, turn_id: &str) -> Result<(), HubFailure>;
    /// A token that acts as the bot during one of its turns, minted from the owner's session; the
    /// hub reads the bot's rights, not the owner's, from it. `None` for a hub that mints none.
    async fn runtime_token(
        &self,
        _bot_id: &str,
        _turn_id: &str,
    ) -> Result<Option<String>, HubFailure> {
        Ok(None)
    }
    /// The same hub with every call made as the bot by `token`, for what the bot does during its
    /// turn; `None` for a hub that has no such identity. Owner-only calls keep `self`.
    fn with_runtime_token(&self, _token: &str) -> Option<Arc<dyn TurnHub>> {
        None
    }
    /// Every grant and answer token of the account bot as `(tool, scope)`.
    async fn tool_keys(&self, _bot_id: &str) -> Result<Vec<(String, String)>, HubFailure> {
        Ok(Vec::new())
    }
    async fn put_tool_key(
        &self,
        _bot_id: &str,
        _tool: &str,
        _scope: &str,
    ) -> Result<(), HubFailure> {
        Err(HubFailure::offline("This hub keeps no tool grants"))
    }
    async fn delete_tool_key(
        &self,
        _bot_id: &str,
        _tool: &str,
        _scope: &str,
    ) -> Result<bool, HubFailure> {
        Ok(false)
    }
    /// Parks the turn this host holds behind an approval card the hub posts as the bot.
    async fn park(
        &self,
        _turn_id: &str,
        _host_id: &str,
        _card: &ApprovalRequest,
    ) -> Result<(), HubFailure> {
        Err(HubFailure::offline("This hub cannot park turns"))
    }
    /// Ends the turn this host holds `done` without a reply: the bot read along and stayed silent.
    async fn silent(&self, _turn_id: &str, _host_id: &str) -> Result<(), HubFailure> {
        Err(HubFailure::offline("This hub cannot end turns silently"))
    }
    /// Completes this turn silently and records its dependency on another room participant.
    async fn wait_for(
        &self,
        _turn_id: &str,
        _host_id: &str,
        _participant_id: &str,
    ) -> Result<WaitOutcome, HubFailure> {
        Ok(WaitOutcome::Unavailable)
    }
    /// For a turn a late answer woke: the question and the answer. `None` for any other turn.
    async fn ask_result(
        &self,
        _bot_id: &str,
        _turn_id: &str,
    ) -> Result<Option<AskResult>, HubFailure> {
        Ok(None)
    }
    /// A GET with the signed-in session: how an account bot's memory reads its conversation and
    /// summaries by position.
    async fn read(
        &self,
        _path: &str,
        _query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        Err(HubFailure::offline("This hub offers no inbox reads"))
    }
    /// A POST with the signed-in session: how an account bot's memory writes its summaries.
    async fn write(
        &self,
        _path: &str,
        _query: Vec<(&'static str, String)>,
        _body: Value,
    ) -> Result<Value, HubFailure> {
        Err(HubFailure::offline("This hub offers no inbox writes"))
    }
    /// A PUT with the signed-in session: how an encryption bot publishes its inbox key.
    async fn put(
        &self,
        _path: &str,
        _query: Vec<(&'static str, String)>,
        _body: Value,
    ) -> Result<Value, HubFailure> {
        Err(HubFailure::offline("This hub offers no key writes"))
    }
    /// A PATCH with the signed-in session: how a bot's notes are written.
    async fn patch(
        &self,
        _path: &str,
        _query: Vec<(&'static str, String)>,
        _body: Value,
    ) -> Result<Value, HubFailure> {
        Err(HubFailure::offline("This hub offers no note writes"))
    }
    /// A DELETE with the signed-in session: how a rotated inbox key is revoked.
    async fn delete(
        &self,
        _path: &str,
        _query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        Err(HubFailure::offline("This hub offers no key revocation"))
    }
}

#[derive(Deserialize)]
struct HubEntry {
    id: String,
    message_id: String,
    conversation_id: String,
    sender_id: String,
    kind: InboxEntryKind,
    envelope: Value,
    created_at: DateTime<FixedOffset>,
    seq: Option<i64>,
    #[serde(default)]
    cursor: String,
}

#[derive(Deserialize)]
struct HubEntries {
    entries: Vec<HubEntry>,
}

impl From<HubEntry> for InboxEntry {
    fn from(entry: HubEntry) -> Self {
        InboxEntry {
            id: entry.id,
            message_id: entry.message_id,
            conversation_id: entry.conversation_id,
            sender_id: entry.sender_id,
            kind: entry.kind,
            envelope: entry.envelope,
            created_at: entry.created_at,
            seq: entry.seq,
            cursor: entry.cursor,
        }
    }
}

pub fn entries_of(value: Value) -> Result<Vec<InboxEntry>, HubFailure> {
    serde_json::from_value::<HubEntries>(value)
        .map(|page| page.entries.into_iter().map(InboxEntry::from).collect())
        .map_err(|_| HubFailure::offline("the hub's entries are not in the expected shape"))
}

/// The question a late answer belongs to, as `GET /bots/{id}/turns/{turn}/ask-result` says.
#[derive(Debug, Clone, Deserialize)]
pub struct AskResult {
    pub origin_conversation: String,
    #[serde(default)]
    pub callee_id: String,
    pub callee_name: String,
    pub question: String,
    pub answer: String,
    pub failed: bool,
    /// The question's round; 0 from a hub that predates rounds.
    #[serde(default)]
    pub round: u32,
}

impl AskResult {
    /// The count the woken turn goes on from; `None` without a callee or round.
    pub fn answered_round(&self) -> Option<flow_like::flow::copilot::bot_talk::AnsweredRound> {
        (self.round > 0 && !self.callee_id.is_empty()).then(|| {
            flow_like::flow::copilot::bot_talk::AnsweredRound {
                callee_id: self.callee_id.clone(),
                round: self.round,
            }
        })
    }
}

pub fn answer(status: u16, body: &[u8]) -> Result<Value, HubFailure> {
    if (200..300).contains(&status) {
        if body.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        return serde_json::from_slice(body)
            .map_err(|_| HubFailure::offline("the hub's answer is not JSON"));
    }
    let error: Option<Value> = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("error").cloned());
    let text = |field: &str| {
        error
            .as_ref()
            .and_then(|error| error.get(field))
            .and_then(Value::as_str)
    };
    let message: String = text("message")
        .map(|message| message.chars().take(MESSAGE_CHARS).collect())
        .unwrap_or_else(|| format!("the hub answered HTTP {status}"));
    Err(HubFailure::answered(status, text("code"), message))
}

pub fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, HubFailure> {
    serde_json::from_value(value)
        .map_err(|_| HubFailure::offline("the hub's answer is not in the expected shape"))
}

/// An account bot's grants on the hub.
pub struct HubGrants {
    pub hub: Arc<dyn TurnHub>,
    pub bot_id: String,
}

fn failed(failure: HubFailure) -> flow_like_types::Error {
    flow_like_types::anyhow!("{failure}")
}

#[async_trait]
impl GrantStore for HubGrants {
    async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>> {
        let keys = self.hub.tool_keys(&self.bot_id).await.map_err(failed)?;
        Ok(keys
            .into_iter()
            .map(|(tool, scope)| GrantKey::new(tool, scope))
            .collect())
    }

    async fn put(&self, tool: &str, scope: &str) -> flow_like_types::Result<()> {
        self.hub
            .put_tool_key(&self.bot_id, tool, scope)
            .await
            .map_err(failed)
    }

    async fn delete(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool> {
        self.hub
            .delete_tool_key(&self.bot_id, tool, scope)
            .await
            .map_err(failed)
    }
}
