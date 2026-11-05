//! Device-hosted bots: what a managed device exchanges with the hub to answer the turns of the
//! bots bound to it (`Bot.location = device`, `Bot.deviceId = <device>`). Every call is a POST
//! to `/devices/{id}/bots/...` authenticated with the device's own DPoP session; the lifecycle
//! mirrors the desktop host's (poll, claim with a lease, heartbeat, then reply, stay silent,
//! park for an approval or release). The grant of a claim carries everything the device needs
//! to run the turn without a user sign-in: tokens bound to the turn, the bot's run spec and the
//! hub-resolved models.

use crate::{
    bot::{BotCapability, BotModelPolicy, BotSystemKind},
    bot_connector::ConnectorRule,
    bot_message::ApprovalRequest,
    bot_skill::SkillIndexEntry,
    bot_turn::{BotTurn, TurnAudience},
    data_classification::DataSensitivity,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// The longest a device waits between polls, so its bots stay online (150 s window).
pub const MAX_POLL_MS: u64 = 60_000;
/// The poll rhythm while queued turns wait for this device.
pub const BUSY_POLL_MS: u64 = 2_000;
/// The poll rhythm while nothing waits.
pub const IDLE_POLL_MS: u64 = 30_000;
/// Tokens of a grant or heartbeat with less than this many seconds left are replaced by fresh ones.
pub const TOKEN_REFRESH_SECS: i64 = 300;
/// The most bytes a reply request body may carry.
pub const MAX_REPLY_BYTES: usize = 256 * 1024;
/// Lifetime of tokens a heartbeat issues, seconds; the lease, not the token, bounds the turn.
pub const REFRESHED_TOKEN_SECS: i64 = 900;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceHostRequest {
    /// Names this host in the turn's lease. 1 to 128 characters without spaces; one per agent boot.
    pub host_id: String,
}

/// `POST /devices/{id}/bots/poll`: reports presence for the device's bots and lists its work.
pub type DevicePollRequest = DeviceHostRequest;

/// A bot bound to the polling device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceBotInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<BotCapability>,
    /// RFC 3339; changes whenever the bot's settings change.
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePollResponse {
    /// The live bots bound to this device, now reported online.
    #[serde(default)]
    pub bots: Vec<DeviceBotInfo>,
    /// Queued turns of those bots, oldest first (one per bot and conversation, `batched` set).
    #[serde(default)]
    pub turns: Vec<BotTurn>,
    /// Milliseconds until the next poll: short while turns wait, at most [`MAX_POLL_MS`].
    pub next_poll_ms: u64,
}

/// `POST /devices/{id}/bots/turns/{turn}/claim`.
pub type DeviceClaimRequest = DeviceHostRequest;

/// A credential bound to the turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceToken {
    pub token: String,
    /// Unix seconds.
    pub expires_at: i64,
}

/// The metered model allowance reserved for the turn; echo `operation_id` in the call that ends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceUsageGrant {
    pub operation_id: String,
    /// Unix milliseconds; the reservation lapses then.
    pub deadline_ms: i64,
}

/// What the device reports of the model usage when it ends the turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceUsageReport {
    pub operation_id: String,
    /// Wall-clock milliseconds the model-driven part of the turn ran.
    pub elapsed_ms: i64,
}

/// The fields of the core `BotRunContext` that come from the hub, in contract types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceBotSpec {
    pub bot_id: String,
    #[serde(default)]
    pub system: Option<BotSystemKind>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<BotCapability>,
    #[serde(default)]
    pub classification: DataSensitivity,
    #[serde(default)]
    pub selected_app_ids: BTreeSet<String>,
    #[serde(default)]
    pub profile_app_ids: BTreeSet<String>,
    /// The bot's workspace app, made on the first claim.
    #[serde(default)]
    pub workspace_app_id: Option<String>,
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub stt_model: Option<String>,
    #[serde(default)]
    pub tts_model: Option<String>,
    #[serde(default)]
    pub image_model: Option<String>,
    #[serde(default)]
    pub vision_model: Option<String>,
    #[serde(default)]
    pub subtask_model: Option<String>,
    #[serde(default)]
    pub skills: Vec<SkillIndexEntry>,
    #[serde(default)]
    pub connectors: Vec<ConnectorRule>,
    #[serde(default)]
    pub model_policy: BotModelPolicy,
}

/// Who woke the turn. `addressed` and `reactions` come from reading the conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRequester {
    pub requester_id: String,
    #[serde(default)]
    pub requester_name: Option<String>,
    pub owner_id: String,
    #[serde(default)]
    pub owner_name: Option<String>,
    pub audience: TurnAudience,
    /// The room's title, when the turn runs in a group.
    #[serde(default)]
    pub room: Option<String>,
}

/// The models the hub resolved for the turn, exactly as for a server turn (hosted or
/// customer-provider Bits only). Each Bit is the serialized core `Bit`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceModels {
    /// The id of the selected completion model.
    pub model_id: String,
    pub completion: Value,
    /// The bot's fallback Bits that run here, in order.
    #[serde(default)]
    pub fallbacks: Vec<Value>,
    /// The embedding Bit the bot's memory uses, when it remembers.
    #[serde(default)]
    pub embedding: Option<Value>,
    #[serde(default)]
    pub embedding_model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceConversation {
    pub id: String,
    /// `direct` or `group`.
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// A claim's result: the turn is the device's until `turn.lease_until`, kept by heartbeats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceTurnGrant {
    /// The claimed turn with `batched` and `requester_name`.
    pub turn: BotTurn,
    pub host_id: String,
    pub conversation: DeviceConversation,
    pub bot: DeviceBotSpec,
    #[serde(default)]
    pub requester: Option<DeviceRequester>,
    pub models: DeviceModels,
    /// Acts as the bot for this turn (inbox reads, secrets, grants, memory, workspace presign,
    /// notify).
    pub bot_token: DeviceToken,
    /// The owner-paid token for hosted model calls and app callbacks, bound to the bot, the
    /// conversation, the turn and the workspace app.
    #[serde(default)]
    pub executor_token: Option<DeviceToken>,
    #[serde(default)]
    pub usage: Option<DeviceUsageGrant>,
    /// Unix seconds; neither token outlives the turn's deadline.
    pub deadline: i64,
}

/// `POST …/turns/{turn}/heartbeat`: keeps the lease and reports when the tokens in hand expire,
/// so the hub can replace the ones that are about to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceHeartbeatRequest {
    pub host_id: String,
    /// Unix seconds, from the grant or the last refresh.
    #[serde(default)]
    pub bot_token_expires_at: Option<i64>,
    #[serde(default)]
    pub executor_token_expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceHeartbeatResponse {
    pub turn: BotTurn,
    /// Set when the bot token has less than [`TOKEN_REFRESH_SECS`] left.
    #[serde(default)]
    pub bot_token: Option<DeviceToken>,
    /// Set when the executor token has less than [`TOKEN_REFRESH_SECS`] left.
    #[serde(default)]
    pub executor_token: Option<DeviceToken>,
}

/// `POST …/turns/{turn}/reply`: the bot's answer, or its failure report, and the turn's
/// completion, in one step. `message_id` is `reply_message_id(bot, trigger)`, or
/// `failure_message_id(bot, trigger)` together with `failure_code`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceReplyRequest {
    pub host_id: String,
    pub message_id: String,
    /// `{ v: 1, enc: "none", body }` as `POST /inbox/conversations/{id}/entries` takes it.
    pub envelope: Value,
    #[serde(default)]
    pub failure_code: Option<String>,
    #[serde(default)]
    pub usage: Option<DeviceUsageReport>,
}

/// A reply's result: the first copy on a repeat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceReplyResponse {
    pub turn: BotTurn,
    pub message_id: String,
    /// Whether this call posted the reply (`false`: it was already posted).
    pub posted: bool,
}

/// `POST …/turns/{turn}/silent`: the bot read along and stays quiet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSilentRequest {
    pub host_id: String,
    #[serde(default)]
    pub usage: Option<DeviceUsageReport>,
}

/// `POST …/turns/{turn}/park`: the turn needs its owner's approval and sleeps until answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceParkRequest {
    pub host_id: String,
    pub approval: ApprovalRequest,
    #[serde(default)]
    pub usage: Option<DeviceUsageReport>,
}

/// `POST …/turns/{turn}/release`: the device gives the turn up. `retry` queues it again with a
/// backoff; otherwise it ends `FAILED` with `error_code` and posts nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceReleaseRequest {
    pub host_id: String,
    pub error_code: String,
    #[serde(default)]
    pub retry: bool,
    #[serde(default)]
    pub usage: Option<DeviceUsageReport>,
}

/// The turn after silent, park or release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTurnState {
    pub turn: BotTurn,
}

/// `POST /devices/{id}/bots/hosts/forget`: the device shuts down; its bots stop reading online.
pub type DeviceForgetRequest = DeviceHostRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceForgetResponse {
    pub removed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_fields_may_be_absent_on_the_wire() {
        let reply: DeviceReplyRequest = serde_json::from_value(serde_json::json!({
            "host_id": "device:1",
            "message_id": "re-1",
            "envelope": { "v": 1, "enc": "none", "body": { "text": "hi" } }
        }))
        .unwrap();
        assert!(reply.failure_code.is_none() && reply.usage.is_none());
        let poll: DevicePollResponse =
            serde_json::from_value(serde_json::json!({ "next_poll_ms": 2000 })).unwrap();
        assert!(poll.bots.is_empty() && poll.turns.is_empty());
        let release: DeviceReleaseRequest = serde_json::from_value(
            serde_json::json!({ "host_id": "d", "error_code": "BOT_TURN_FAILED" }),
        )
        .unwrap();
        assert!(!release.retry);
    }

    #[test]
    fn polling_never_outlasts_the_presence_window() {
        assert!(
            MAX_POLL_MS <= 60_000 && IDLE_POLL_MS <= MAX_POLL_MS && BUSY_POLL_MS < IDLE_POLL_MS
        );
    }
}
