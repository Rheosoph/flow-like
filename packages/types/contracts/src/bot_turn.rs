//! Bot turns: the work ledger every bot host shares. A message that wakes a bot queues one turn
//! per bot; the bot's host claims it with a lease, answers it as the bot and the turn completes
//! with the reply. The lowercase ids are the wire format; the database stores them uppercase.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bot turns that may lead to one message: a person's message carries 0, a bot's reply the
/// answered turn's hops + 1, and a message with more than this many queues no turn.
pub const MAX_HOPS: u32 = 3;
/// Turns one owner may have running at once.
pub const MAX_RUNNING_PER_OWNER: usize = 3;
/// Seconds a claim or heartbeat holds a turn before another host may take it.
pub const LEASE_SECS: i64 = 60;
/// Seconds between a host's heartbeats.
pub const HEARTBEAT_SECS: u64 = 15;
/// A turn queued this long without a host is failed.
pub const QUEUED_EXPIRY_SECS: i64 = 3_600;
/// Claims a turn may take, takeovers of an expired lease included.
pub const MAX_ATTEMPTS: u32 = 3;
/// How long a host leaves a message from an interactive chat to the chat that sent it, and how
/// long the chat's hold on the turn lasts between refreshes.
pub const INTERACTIVE_GRACE_SECS: i64 = 120;
/// Host ids of interactive chats start with this: they answer the turn of a message they sent
/// instead of claiming it.
pub const INTERACTIVE_HOST_PREFIX: &str = "interactive:";

pub fn is_interactive_host(host_id: &str) -> bool {
    host_id.starts_with(INTERACTIVE_HOST_PREFIX)
}

pub const STORED_QUEUED: &str = "QUEUED";
pub const STORED_RUNNING: &str = "RUNNING";
pub const STORED_WAITING: &str = "WAITING";
pub const STORED_DONE: &str = "DONE";
pub const STORED_FAILED: &str = "FAILED";
pub const STORED_CANCELLED: &str = "CANCELLED";

pub const STORED_AUDIENCE_OWNER: &str = "OWNER";
pub const STORED_AUDIENCE_ROOM: &str = "ROOM";

pub const STORED_SERVER: &str = "SERVER";
pub const STORED_DESKTOP: &str = "DESKTOP";
pub const STORED_DEVICE: &str = "DEVICE";

/// The `error_code` of a turn that an earlier turn of the same bot in the same conversation
/// answered together with it: `reply_message_id` names that answer.
pub const ERROR_BATCHED: &str = "BATCHED";
/// The bot was deleted while the turn waited or ran.
pub const ERROR_BOT_DELETED: &str = "BOT_DELETED";
/// The turn waited for a host longer than [`QUEUED_EXPIRY_SECS`].
pub const ERROR_QUEUE_EXPIRED: &str = "BOT_TURN_EXPIRED";
/// Its host stopped heartbeating and the turn used every attempt.
pub const ERROR_HOST_LOST: &str = "BOT_TURN_HOST_LOST";

/// Where a turn runs: the location of the bot when the message arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnHostKind {
    /// The API's own worker.
    Server,
    /// The owner's desktop client.
    Desktop,
    /// A managed device the owner registered.
    Device,
}

impl TurnHostKind {
    pub const ALL: [Self; 3] = [Self::Server, Self::Desktop, Self::Device];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Desktop => "desktop",
            Self::Device => "device",
        }
    }

    pub fn stored(self) -> &'static str {
        match self {
            Self::Server => STORED_SERVER,
            Self::Desktop => STORED_DESKTOP,
            Self::Device => STORED_DEVICE,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(value))
    }
}

/// Where a turn stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    /// Waiting for its host.
    Queued,
    /// Claimed by a host that holds the lease.
    Running,
    /// Parked: the turn asked its owner to approve a call and sleeps until the answer re-queues it.
    Waiting,
    /// Answered: `reply_message_id` is the bot's reply.
    Done,
    /// Failed; `error_code` says why and the owner may retry.
    Failed,
    /// Dropped: the bot was deleted, or another turn answered it (`BATCHED`).
    Cancelled,
}

impl TurnStatus {
    pub const ALL: [Self; 6] = [
        Self::Queued,
        Self::Running,
        Self::Waiting,
        Self::Done,
        Self::Failed,
        Self::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn stored(self) -> &'static str {
        match self {
            Self::Queued => STORED_QUEUED,
            Self::Running => STORED_RUNNING,
            Self::Waiting => STORED_WAITING,
            Self::Done => STORED_DONE,
            Self::Failed => STORED_FAILED,
            Self::Cancelled => STORED_CANCELLED,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|status| status.as_str().eq_ignore_ascii_case(value))
    }

    /// No host will touch the turn again unless its owner retries it.
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// Who can read the bot's reply: only its owner, or the people of a shared room.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnAudience {
    /// A direct conversation with the owner.
    #[default]
    Owner,
    /// A group conversation: other people read the answer.
    Room,
}

impl TurnAudience {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Room => "room",
        }
    }

    pub fn stored(self) -> &'static str {
        match self {
            Self::Owner => STORED_AUDIENCE_OWNER,
            Self::Room => STORED_AUDIENCE_ROOM,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        [Self::Owner, Self::Room]
            .into_iter()
            .find(|audience| audience.as_str().eq_ignore_ascii_case(value))
    }

    /// The audience of a conversation of this kind.
    pub fn of(kind: crate::inbox::ConversationKind) -> Self {
        match kind {
            crate::inbox::ConversationKind::Direct => Self::Owner,
            crate::inbox::ConversationKind::Group => Self::Room,
        }
    }
}

/// How the model reads a message of someone else in a shared room.
pub fn room_message(name: &str, is_owner: bool, text: &str) -> String {
    if is_owner {
        format!("{name} (your owner) wrote: {text}")
    } else {
        format!("{name} (member, not your owner) wrote: {text}")
    }
}

/// A turn as hosts and watchers read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotTurn {
    /// `bt_` and a hash of the bot and the message that woke it.
    pub id: String,
    pub bot_id: String,
    pub conversation_id: String,
    /// The message that woke the bot: what the turn answers.
    pub trigger_message_id: String,
    pub host_kind: TurnHostKind,
    pub status: TurnStatus,
    /// Claims so far, takeovers of an expired lease included.
    pub attempts: u32,
    /// The host holding the lease.
    pub host_id: Option<String>,
    /// RFC 3339; a running turn belongs to its host until then.
    pub lease_until: Option<String>,
    /// The bot's answer, once there is one.
    pub reply_message_id: Option<String>,
    /// Why the turn failed or was dropped, e.g. `BOT_TURN_TIMEOUT`.
    pub error_code: Option<String>,
    /// Bot turns that led to the trigger message.
    pub hops: u32,
    #[serde(default)]
    pub owner_id: String,
    /// The person or bot whose message woke the turn; `None` on turns queued before it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester_id: Option<String>,
    /// The requester's display name, set on a claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester_name: Option<String>,
    #[serde(default)]
    pub audience: TurnAudience,
    /// RFC 3339.
    pub created_at: String,
    /// RFC 3339.
    pub updated_at: String,
    /// Older messages queued turns for the same bot in the same conversation; the turn answers
    /// them too. Set on a claim and on a listing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub batched: Vec<String>,
}

/// A list of turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotTurnList {
    pub turns: Vec<BotTurn>,
}

fn digest(prefix: &str, bot_id: &str, trigger_message_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.bot-turn/v1\0");
    hasher.update(&(bot_id.len() as u64).to_be_bytes());
    hasher.update(bot_id.as_bytes());
    hasher.update(trigger_message_id.as_bytes());
    let hex = hasher.finalize().to_hex();
    format!("{prefix}{}", &hex.as_str()[..32])
}

/// The id of the turn `bot_id` takes for `trigger_message_id`: one turn per bot and message.
pub fn turn_id(bot_id: &str, trigger_message_id: &str) -> String {
    digest("bt_", bot_id, trigger_message_id)
}

/// The message id of the bot's answer to `trigger_message_id`: sending it twice sends it once.
pub fn reply_message_id(bot_id: &str, trigger_message_id: &str) -> String {
    digest("re-", bot_id, trigger_message_id)
}

/// The message id of the bot's report that its turn for `trigger_message_id` failed.
pub fn failure_message_id(bot_id: &str, trigger_message_id: &str) -> String {
    digest("rf-", bot_id, trigger_message_id)
}

/// The message id of the bot's notice that a requester used up their turns for the day.
pub fn rate_limit_message_id(bot_id: &str, trigger_message_id: &str) -> String {
    digest("rl-", bot_id, trigger_message_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::is_valid_message_id;

    #[test]
    fn ids_are_stable_distinct_and_valid_message_ids() {
        let turn = turn_id("bot_a", "m1");
        assert_eq!(turn, turn_id("bot_a", "m1"));
        assert_ne!(turn, turn_id("bot_b", "m1"));
        assert_ne!(turn, turn_id("bot_a", "m2"));
        assert_ne!(turn_id("bot_a", "bm1"), turn_id("bot_ab", "m1"));
        assert!(turn.starts_with("bt_") && turn.len() == 35);
        for id in [
            reply_message_id("bot_a", "m1"),
            failure_message_id("bot_a", "m1"),
        ] {
            assert!(is_valid_message_id(&id), "{id}");
        }
        assert_ne!(
            reply_message_id("bot_a", "m1"),
            failure_message_id("bot_a", "m1")
        );
        assert_ne!(
            reply_message_id("bot_a", "m1"),
            reply_message_id("bot_b", "m1")
        );
    }

    #[test]
    fn statuses_and_host_kinds_round_trip_between_wire_and_storage() {
        for status in TurnStatus::ALL {
            assert_eq!(TurnStatus::parse(status.stored()), Some(status));
            assert_eq!(TurnStatus::parse(status.as_str()), Some(status));
            assert_eq!(status.stored(), status.as_str().to_ascii_uppercase());
        }
        for kind in TurnHostKind::ALL {
            assert_eq!(TurnHostKind::parse(kind.stored()), Some(kind));
            assert_eq!(kind.stored(), kind.as_str().to_ascii_uppercase());
        }
        assert_eq!(TurnStatus::parse("paused"), None);
        assert!(TurnStatus::Failed.is_finished() && !TurnStatus::Running.is_finished());
    }

    #[test]
    fn audiences_round_trip_and_follow_the_conversation_kind() {
        use crate::inbox::ConversationKind;
        for audience in [TurnAudience::Owner, TurnAudience::Room] {
            assert_eq!(TurnAudience::parse(audience.stored()), Some(audience));
        }
        assert_eq!(TurnAudience::of(ConversationKind::Direct), TurnAudience::Owner);
        assert_eq!(TurnAudience::of(ConversationKind::Group), TurnAudience::Room);
    }

    #[test]
    fn room_messages_name_their_speaker_and_whether_it_is_the_owner() {
        assert_eq!(
            room_message("Mia", false, "hi"),
            "Mia (member, not your owner) wrote: hi"
        );
        assert_eq!(room_message("Ben", true, "hi"), "Ben (your owner) wrote: hi");
    }

    #[test]
    fn a_turn_serializes_with_lowercase_ids_and_no_empty_batch() {
        let turn = BotTurn {
            id: "bt_1".into(),
            bot_id: "bot_a".into(),
            conversation_id: "c1".into(),
            trigger_message_id: "m1".into(),
            host_kind: TurnHostKind::Desktop,
            status: TurnStatus::Running,
            attempts: 1,
            host_id: Some("host".into()),
            lease_until: Some("2026-11-02T10:00:00+00:00".into()),
            reply_message_id: None,
            error_code: None,
            hops: 0,
            owner_id: "user_a".into(),
            requester_id: Some("user_a".into()),
            requester_name: None,
            audience: TurnAudience::Owner,
            created_at: "2026-11-02T09:59:00+00:00".into(),
            updated_at: "2026-11-02T09:59:30+00:00".into(),
            batched: Vec::new(),
        };
        let json = serde_json::to_value(&turn).unwrap();
        assert_eq!(json["status"], "running");
        assert_eq!(json["host_kind"], "desktop");
        assert!(json.get("batched").is_none());
        assert_eq!(serde_json::from_value::<BotTurn>(json).unwrap(), turn);
    }
}
