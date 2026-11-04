//! When a bot's message notifies its owner, and how the notification looks. Servers and desktop
//! hosts decide with the same rules: a reply the owner did not just ask for notifies them, grouped
//! under the bot, and opens the bot's chat.

use serde_json::Value;

use crate::inbox::InboxEntryKind;

/// Appearance key the owner's per-bot "Notifications" switch writes (`true` mutes the bot).
pub const MUTE_KEY: &str = "mute_notifications";
/// Messages one bot's `message_owner` tool may send per [`TOOL_WINDOW_SECS`].
pub const TOOL_MAX_PER_WINDOW: usize = 10;
pub const TOOL_WINDOW_SECS: i64 = 3_600;
pub const TITLE_MAX_CHARS: usize = 120;
pub const BODY_MAX_CHARS: usize = 500;
/// A message the owner is written in their chat.
pub const MESSAGE_MAX_CHARS: usize = 4_000;
/// Marks the notifications a bot's tool sent (`source_node_id`), against replies (the message id).
pub const TOOL_SOURCE: &str = "bot-notify-tool";
const CHAT_LINK_PREFIX: &str = "/bots?bot=";

/// Routing hints for native notifications. The API still authorizes every action.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BotNotificationContext {
    pub bot_id: String,
    #[serde(default)]
    pub review_only: bool,
    pub conversation_id: Option<String>,
    pub conversation_encryption: Option<String>,
    pub message_id: Option<String>,
    pub turn_id: Option<String>,
    pub approval_id: Option<String>,
    pub task_id: Option<String>,
    /// RFC 3339 deadline for an approval action.
    pub expires_at: Option<String>,
}

impl BotNotificationContext {
    pub fn category(&self) -> &'static str {
        if self.review_only {
            "flow_like_bot_review"
        } else if self.approval_id.is_some() {
            "flow_like_bot_approval"
        } else {
            "flow_like_bot_message"
        }
    }

    pub fn fields(&self) -> Vec<(&'static str, &str)> {
        let mut fields = vec![
            ("bot_id", self.bot_id.as_str()),
            ("category", self.category()),
        ];
        for (key, value) in [
            ("conversation_id", &self.conversation_id),
            ("conversation_encryption", &self.conversation_encryption),
            ("message_id", &self.message_id),
            ("turn_id", &self.turn_id),
            ("approval_id", &self.approval_id),
            ("task_id", &self.task_id),
            ("expires_at", &self.expires_at),
        ] {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        fields
    }
}

pub fn notifications_muted(appearance: &Value) -> bool {
    appearance
        .get(MUTE_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Where a bot's notifications open.
pub fn chat_link(bot_id: &str) -> String {
    format!("{CHAT_LINK_PREFIX}{bot_id}")
}

pub fn bot_id_from_link(link: &str) -> Option<&str> {
    link.strip_prefix(CHAT_LINK_PREFIX)
        .map(|rest| rest.split(['&', '#']).next().unwrap_or(rest))
        .filter(|id| !id.is_empty())
}

/// The key a bot's notifications group under: `source_run_id` of the notification row.
pub fn group_key(bot_id: &str) -> String {
    format!("bot:{bot_id}")
}

/// Whether a bot's reply was not asked for by a person's message: the turn it answers was woken by
/// anything but a message, or by another bot's message (hops above 0).
pub fn reply_is_proactive(trigger_kind: InboxEntryKind, hops: u32) -> bool {
    trigger_kind != InboxEntryKind::Message || hops > 0
}

/// A one-line preview of a reply for a notification body.
pub fn preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= BODY_MAX_CHARS {
        return flat;
    }
    let mut clipped: String = flat.chars().take(BODY_MAX_CHARS - 1).collect();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_a_message_from_a_person_is_asked_for() {
        assert!(!reply_is_proactive(InboxEntryKind::Message, 0));
        assert!(reply_is_proactive(InboxEntryKind::Message, 1));
        assert!(reply_is_proactive(InboxEntryKind::Trigger, 0));
        assert!(reply_is_proactive(InboxEntryKind::Trigger, 2));
    }

    #[test]
    fn native_context_only_contains_routing_and_approval_hints() {
        let context = BotNotificationContext {
            bot_id: "b1".into(),
            approval_id: Some("ap1".into()),
            ..Default::default()
        };
        let fields: std::collections::BTreeMap<_, _> = context.fields().into_iter().collect();
        assert_eq!(fields.get("category"), Some(&"flow_like_bot_approval"));
        assert_eq!(fields.get("approval_id"), Some(&"ap1"));
        assert!(!fields.contains_key("conversation_id"));
        assert_eq!(fields.len(), 3);
    }

    #[test]
    fn links_round_trip() {
        let link = chat_link("bot_1");
        assert_eq!(link, "/bots?bot=bot_1");
        assert_eq!(bot_id_from_link(&link), Some("bot_1"));
        assert_eq!(bot_id_from_link("/bots?bot=b2&x=1"), Some("b2"));
        assert_eq!(bot_id_from_link("/bots?bot="), None);
        assert_eq!(bot_id_from_link("/store"), None);
    }

    #[test]
    fn mute_reads_a_boolean_only() {
        assert!(notifications_muted(
            &json!({ "kind": "pip", "mute_notifications": true })
        ));
        assert!(!notifications_muted(&json!({ "kind": "pip" })));
        assert!(!notifications_muted(
            &json!({ "mute_notifications": "yes" })
        ));
    }

    #[test]
    fn previews_are_flat_and_bounded() {
        assert_eq!(preview("a\n  b"), "a b");
        let long = preview(&"x".repeat(900));
        assert_eq!(long.chars().count(), BODY_MAX_CHARS);
        assert!(long.ends_with('…'));
    }
}
