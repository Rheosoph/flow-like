//! A bot's `@Name` in its room message becomes a mention, as a person's composer makes one, so the
//! bot it names wakes for it. Only bots are named this way: a bot naming a person would raise that
//! person's notification to a mention.

use super::entries::{Content, Outgoing};
use crate::{bot_host::history::bot_display_name, entity::bot};
use flow_like_types::{bot_mentions::named_mentions, inbox::InboxEntryKind};
use serde_json::Value;

/// `outgoing` with the other bots that its sender, a bot, names as `@Name` added to `mentions`,
/// in `envelopes` too. `None` when it names no bot it doesn't mention already.
pub(super) fn with_named_bots(
    sender: &str,
    bots: &[bot::Model],
    outgoing: &Outgoing,
    envelopes: &mut [(String, Value)],
) -> Option<Outgoing> {
    if outgoing.kind != InboxEntryKind::Message || bots.iter().all(|bot| bot.id != sender) {
        return None;
    }
    let Content::Plain(envelope) = &outgoing.content else {
        return None;
    };
    let text = envelope["body"]["text"].as_str()?;
    let names: Vec<(&str, String)> = bots
        .iter()
        .filter(|bot| bot.id != sender)
        .filter_map(|bot| {
            bot_display_name(bot.name.clone(), bot.system_key.as_deref())
                .map(|name| (bot.id.as_str(), name))
        })
        .collect();
    let named: Vec<String> =
        named_mentions(text, names.iter().map(|(id, name)| (*id, name.as_str())))
            .into_iter()
            .filter(|id| !outgoing.links.mentions.contains(id))
            .collect();
    if named.is_empty() {
        return None;
    }
    let mut mentioned = outgoing.clone();
    mentioned.links.mentions.extend(named);
    let mentions = Value::from(mentioned.links.mentions.clone());
    let bodies =
        envelopes
            .iter_mut()
            .map(|(_, envelope)| envelope)
            .chain(match &mut mentioned.content {
                Content::Plain(envelope) => Some(envelope),
                Content::Sealed(_) => None,
            });
    for envelope in bodies {
        if let Some(body) = envelope.get_mut("body").and_then(Value::as_object_mut) {
            body.insert("mentions".to_owned(), mentions.clone());
        }
    }
    Some(mentioned)
}
