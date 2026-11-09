//! Telling the person about a bot's reply they did not ask for. The host hands an [`Alert`] to its
//! [`super::host::TurnEvents`]; the UI shows it as an OS notification, a toast and a stored entry,
//! titled with the bot's name and opening the bot's chat. The rules are shared with the server
//! (`flow_like_types::bot_notify`).

use flow_like_local_bots::Bot;
use flow_like_types::{
    bot::BotSystemKind,
    bot_notify::{chat_link, group_key, notifications_muted, preview},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

const SEEN_KEPT: usize = 256;
pub(crate) const NOTIFICATION_EVENT: &str = "flow_notification";
const NO_PREVIEW: &str = "Sent you a message";

pub use flow_like_bot_host::host::Alert;

pub(crate) fn bot_title(bot: &Bot) -> String {
    bot.name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| match bot.system {
            Some(BotSystemKind::Flux) => "Flux".to_owned(),
            _ => "Assistant".to_owned(),
        })
}

pub(crate) fn alert_for(bot: &Bot, body: String, link: Option<String>, key: &str) -> Alert {
    Alert {
        bot_id: bot.id.clone(),
        title: bot_title(bot),
        body,
        link: link.unwrap_or_else(|| chat_link(&bot.id)),
        key: key.to_owned(),
    }
}

/// The alert for a posted reply; `None` when the person asked for it or muted the bot.
pub(super) fn reply_alert(
    bot: &Bot,
    proactive: bool,
    message_id: &str,
    text: &str,
) -> Option<Alert> {
    if !proactive || notifications_muted(&bot.appearance) {
        return None;
    }
    let body = Some(preview(text))
        .filter(|body| !body.is_empty())
        .unwrap_or_else(|| NO_PREVIEW.to_owned());
    Some(alert_for(bot, body, None, message_id))
}

/// The `flow_notification` batch the window turns into an OS notification, a toast and a stored
/// entry. `source_run_id` groups every notification of the bot.
pub(crate) fn batch(alert: &Alert) -> Value {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    json!([{
        "event_id": format!("bot-alert-{}", alert.key),
        "event_type": "notification",
        "timestamp": { "secs_since_epoch": millis / 1000, "nanos_since_epoch": (millis % 1000) * 1_000_000 },
        "payload": {
            "title": alert.title,
            "description": alert.body,
            "link": alert.link,
            "show_desktop": true,
            "source_run_id": group_key(&alert.bot_id),
            "source_node_id": alert.key,
        },
    }])
}

/// Remembers the last alerts, so a reply that is posted again alerts once.
#[derive(Default)]
pub(crate) struct Seen(Mutex<VecDeque<String>>);

impl Seen {
    pub(crate) fn first(&self, alert: &Alert) -> bool {
        let id = format!("{}\u{1f}{}", alert.bot_id, alert.key);
        let mut seen = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if seen.contains(&id) {
            return false;
        }
        seen.push_back(id);
        while seen.len() > SEEN_KEPT {
            seen.pop_front();
        }
        true
    }
}

#[cfg(test)]
pub(super) fn test_bot(appearance: Value) -> Bot {
    use chrono::{DateTime, FixedOffset};
    let at: DateTime<FixedOffset> =
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").unwrap();
    Bot {
        id: "bot_1".into(),
        owner_id: flow_like_local_bots::LOCAL_OWNER_ID.into(),
        system: None,
        preset_id: None,
        name: Some("Ada".into()),
        description: None,
        instructions: None,
        appearance,
        model: Default::default(),
        embedding_model: None,
        stt_model: None,
        tts_model: None,
        image_model: None,
        vision_model: None,
        subtask_model: None,
        classification: Default::default(),
        capabilities: Vec::new(),
        profile_id: None,
        location: Default::default(),
        device_id: None,
        workspace_app_id: None,
        created_at: at,
        updated_at: at,
        local: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bot(appearance: Value) -> Bot {
        test_bot(appearance)
    }

    #[test]
    fn an_asked_for_reply_makes_no_alert() {
        assert_eq!(
            reply_alert(&bot(json!({ "kind": "pip" })), false, "re-1", "Hi"),
            None
        );
    }

    #[test]
    fn a_proactive_reply_alerts_under_the_bots_name_and_opens_its_chat() {
        let alert = reply_alert(
            &bot(json!({ "kind": "pip" })),
            true,
            "re-1",
            "Report\nready",
        )
        .unwrap();
        assert_eq!(alert.title, "Ada");
        assert_eq!(alert.body, "Report ready");
        assert_eq!(alert.link, "/bots?bot=bot_1");
        let batch = batch(&alert);
        let payload = &batch[0]["payload"];
        assert_eq!(payload["source_run_id"], "bot:bot_1");
        assert_eq!(payload["source_node_id"], "re-1");
        assert_eq!(payload["show_desktop"], true);
        assert_eq!(payload["title"], "Ada");
    }

    #[test]
    fn a_muted_bot_makes_no_alert() {
        let muted = bot(json!({ "kind": "pip", "mute_notifications": true }));
        assert_eq!(reply_alert(&muted, true, "re-1", "Hi"), None);
    }

    #[test]
    fn an_alert_shows_once() {
        let seen = Seen::default();
        let alert = reply_alert(&bot(json!({ "kind": "pip" })), true, "re-1", "Hi").unwrap();
        assert!(seen.first(&alert));
        assert!(!seen.first(&alert));
        let other = Alert {
            key: "re-2".into(),
            ..alert
        };
        assert!(seen.first(&other));
    }
}
