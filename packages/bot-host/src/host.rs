//! The types a host and its runner share: a turn as a job, the runner and event traits, the
//! codes a turn ends with, and how a reply is shaped.

use crate::{gate::ApprovalGate, memory::JobMemory};
use async_trait::async_trait;
use chrono::Utc;
use flow_like::copilot::UnifiedChatMessage;
use flow_like_types::bot_turn::TurnStatus;
use serde::Serialize;
use serde_json::json;
use std::{any::Any, sync::Arc};

pub const CODE_TIMEOUT: &str = "BOT_TURN_TIMEOUT";
pub const CODE_MODEL: &str = "BOT_TURN_MODEL_ERROR";
pub const CODE_UNREADABLE: &str = "BOT_TURN_UNREADABLE";
pub const CODE_NOT_WAITING: &str = "BOT_TURN_NOT_WAITING";
pub const CODE_STORE_CHANGED: &str = "BOT_TURN_STORE_CHANGED";
pub const CODE_SESSION_CHANGED: &str = "BOT_TURN_SESSION_CHANGED";
pub const CODE_CANCELLED: &str = "BOT_TURN_CANCELLED";
pub const CODE_REPLY_REFUSED: &str = "BOT_TURN_REPLY_REFUSED";
pub const CODE_SETUP: &str = "BOT_TURN_SETUP";
pub const CODE_NO_MODEL: &str = "BOT_TURN_NO_MODEL";
pub const NO_DESKTOP_MODEL_TEXT: &str = "no language model it can use runs on this desktop";

pub const MAX_REPLY_CHARS: usize = 48_000;
pub const MAX_FAILURE_DETAIL_CHARS: usize = 500;
pub const FINISHED_KEPT: usize = 50;

#[derive(Debug, Clone)]
pub struct TurnJob {
    /// Final model usage snapshots retained by the host with the reply.
    pub usage: flow_like::flow::copilot::bot_usage::BotUsage,
    pub run_id: String,
    pub bot_id: String,
    pub conversation_id: String,
    pub profile_id: Option<String>,
    pub prompt: String,
    /// A call listening cue, rendered without tools or new work.
    pub acknowledgement: bool,
    /// The turn answers an utterance in a live call: its reply is spoken, so it runs fast and short.
    pub live_call: bool,
    /// Holds the run's tool calls until the host's participation decision, when both run together.
    pub tool_gate: Option<flow_like::flow::copilot::tool_gate::ToolGate>,
    /// The authenticated trigger shares temporary context, excluded from checkpoints and from
    /// the history memory assembles.
    pub ephemeral_user_context: bool,
    /// What the turn's history puts into its turn label before the first tool call.
    pub seed_taint: flow_like_types::data_classification::DataSensitivity,
    /// Session snapshots of user context still usable here, for a turn whose history comes from
    /// memory and so leaves them out.
    pub shared_context: Option<String>,
    /// Routine owner notices are delivered by the host for this background event.
    pub suppress_owner_notifications: bool,
    pub history: Vec<UnifiedChatMessage>,
    /// New messages and events from this room, checked while the turn is running.
    pub room_updates: Option<Arc<dyn flow_like::flow::copilot::platform::RoomUpdates>>,
    /// The conversation's memory when this turn remembers.
    pub memory: Option<JobMemory>,
    /// Where a call that asks the owner runs under a grant or parks the turn.
    pub gate: Arc<ApprovalGate>,
    pub requester: Option<flow_like::flow::copilot::TurnRequester>,
    /// The bot's own token for the hub during this turn (account turns); the owner's session
    /// is not used for what the bot does.
    pub hub_token: Option<HubToken>,
    /// The question whose late answer woke the turn; its follow-ups go on counting from there.
    pub answered_round: Option<flow_like::flow::copilot::bot_talk::AnsweredRound>,
    /// The run spec the hub returned with the claim, for a runner that needs it.
    pub claim: Option<Arc<dyn Any + Send + Sync>>,
    /// Durable, scoped execution state for a host that supports restart recovery.
    pub recovery: Option<Arc<dyn flow_like::flow::copilot::run_recovery::BotRunRecovery>>,
    /// The bot's notes in this conversation, when the host keeps them for it.
    pub notes: Option<flow_like::flow::copilot::bot_notes::NotesHandle>,
    /// The turn's tool trail, set only in the owner's direct conversation with the bot.
    pub trail: Option<flow_like::flow::copilot::ToolTrail>,
    /// The files the bot attaches to its reply, for a host that posts them with it.
    pub reply_files: Option<flow_like::flow::copilot::ReplyFiles>,
    /// Takes the reply while the model writes it, for a live call on this computer
    /// ([`crate::live_reply`]); a runner given one hands text on as it comes.
    pub reply_stream: Option<flow_like::flow::copilot::reply_stream::ReplySink>,
}

impl TurnJob {
    pub fn claim_as<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.claim.clone()?.downcast::<T>().ok()
    }
}

/// A bot runtime token that never prints.
#[derive(Clone)]
pub struct HubToken(pub String);

impl std::fmt::Debug for HubToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HubToken(..)")
    }
}

#[derive(Debug)]
pub enum RunError {
    Failed {
        code: String,
        message: String,
    },
    /// A transient failure before any tool with side effects ran: the turn may be taken again.
    Retry {
        code: String,
        message: String,
    },
    SessionChanged,
}

/// Whether a run's failure says the profile has no language model the host can run.
pub fn names_missing_model(message: &str) -> bool {
    let in_profile = flow_like::profile::NoRunnableModel::InProfile.to_string();
    message.contains(&in_profile) || message.contains(NO_DESKTOP_MODEL_TEXT)
}

/// The code a failed run is reported with: `BOT_TURN_NO_MODEL` when no model could run it.
pub fn failure_code<'a>(default: &'a str, message: &str) -> &'a str {
    if names_missing_model(message) {
        CODE_NO_MODEL
    } else {
        default
    }
}

/// What a run, bounded by its deadline, ends with.
pub type RunOutcome = Result<Result<String, RunError>, tokio::time::error::Elapsed>;
pub type RunFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = RunOutcome> + Send + 'a>>;

#[async_trait]
pub trait TurnRunner: Send + Sync + 'static {
    /// The optional configured decision model, shared by bounded classification tasks.
    async fn classify(
        &self,
        _job: &TurnJob,
        _request: &flow_like::flow_like_model_provider::systemone::SystemOneRequest,
    ) -> Result<Option<flow_like::flow_like_model_provider::systemone::SystemOneResponse>, RunError>
    {
        Ok(None)
    }
    /// Optional participation classification, before the primary model or tools run.
    async fn decide(
        &self,
        job: &TurnJob,
        request: &flow_like::flow_like_model_provider::systemone::SystemOneRequest,
    ) -> Result<flow_like::flow::copilot::decision::Decision, RunError> {
        use flow_like::flow::copilot::decision::{self, Decision};
        match self.classify(job, request).await? {
            Some(response) => {
                decision::parse(request, &response).map_err(|error| RunError::Failed {
                    code: CODE_SETUP.into(),
                    message: error.to_string(),
                })
            }
            None => Ok(Decision::AnswerNow),
        }
    }
    async fn run(&self, job: TurnJob) -> Result<String, RunError>;
    fn cancel(&self, run_id: &str);
}

/// Only text and speaker identity reach the classifier, within a bounded recent window.
pub fn participation_request(
    job: &TurnJob,
    trigger: &str,
    entries: &[flow_like_local_bots::InboxEntry],
    participants: &[flow_like::flow::copilot::decision::DecisionParticipant],
) -> flow_like_types::Result<flow_like::flow_like_model_provider::systemone::SystemOneRequest> {
    use flow_like::flow::copilot::decision;
    use flow_like_types::inbox::{InboxEntryKind, is_tombstone};
    let mut recent: Vec<_> = entries
        .iter()
        .rev()
        .filter(|entry| !is_tombstone(&entry.envelope))
        .filter_map(|entry| {
            let text = entry.envelope.get("body")?.get("text")?.as_str()?;
            Some(json!({"sender_id": entry.sender_id, "kind": entry.kind,
                "text": decision::excerpt(text, 1000)}))
        })
        .take(12)
        .collect();
    recent.reverse();
    let call = entries
        .iter()
        .any(|entry| entry.message_id == trigger && entry.kind == InboxEntryKind::Transcript);
    decision::request(
        &job.bot_id,
        json!({
            "prompt": decision::excerpt(&job.prompt, 4000),
            "addressed": job.requester.as_ref().is_none_or(|requester| requester.addressed),
            "interaction": if call { "call" } else { "chat" },
            "recent_messages": recent,
        }),
        participants,
    )
}

/// A reply the person did not ask for, to show as a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub bot_id: String,
    pub title: String,
    pub body: String,
    pub link: String,
    pub key: String,
}

pub trait TurnEvents: Send + Sync + 'static {
    fn turn(&self, info: &TurnInfo);

    /// A reply the person did not ask for was posted; show it as a notification.
    fn alert(&self, _alert: &Alert) {}

    /// Where a live-call reply goes while it is written, for a host whose caller is on this
    /// computer; `None` streams nothing.
    fn replies(&self) -> Option<Arc<dyn crate::live_reply::ReplyOutlet>> {
        None
    }
}

/// A turn as `bot-host://turn` and `bot_host_turns` show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TurnInfo {
    /// Also the run id: a window attaches to the run's tokens with `global_chat_resume`.
    pub id: String,
    pub bot_id: String,
    pub conversation_id: String,
    pub trigger_message_id: String,
    pub status: TurnStatus,
    pub hops: u32,
    pub requester_id: String,
    pub audience: flow_like_types::bot_turn::TurnAudience,
    pub batched: u32,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error_code: Option<String>,
    pub reply_message_id: Option<String>,
    /// The line the running bot shows while it works, from its notes.
    pub status_text: Option<String>,
}

pub fn now() -> String {
    Utc::now().to_rfc3339()
}

pub fn clipped(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub fn reply_body(
    reply_to: &str,
    hops: u32,
    text: &str,
    error_code: Option<&str>,
) -> serde_json::Value {
    let mut body = json!({
        "text": text,
        "reply_to": reply_to,
        "hops": hops.saturating_add(1),
    });
    if let Some(code) = error_code {
        body["error"] = json!({ "code": code });
    }
    body
}
