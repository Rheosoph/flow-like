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

pub const MAX_REPLY_CHARS: usize = 48_000;
pub const MAX_FAILURE_DETAIL_CHARS: usize = 500;
pub const FINISHED_KEPT: usize = 50;

#[derive(Debug, Clone)]
pub struct TurnJob {
    pub run_id: String,
    pub bot_id: String,
    pub conversation_id: String,
    pub profile_id: Option<String>,
    pub prompt: String,
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

#[async_trait]
pub trait TurnRunner: Send + Sync + 'static {
    async fn run(&self, job: TurnJob) -> Result<String, RunError>;
    fn cancel(&self, run_id: &str);
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
