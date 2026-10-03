//! The `Chat` payload of a run (design §5.5): the desktop's shape, with the token's handle in
//! `local_session.bot_token`, at most ten earlier messages of the channel from one history read,
//! and names taken from the messages themselves.

use std::num::NonZeroU16;

use serde_json::{Value, json};
use serenity::model::channel::Message;
use serenity::model::id::UserId;

use super::filter::is_image;

/// Earlier messages of the channel a run receives.
pub const HISTORY: u8 = 10;

/// The payload of a run for `message`. `history` is what one read of the channel returned, in
/// any order; messages at or after `message` are left out. `bot` is the bot's own user: its
/// messages are the assistant's.
pub fn chat_payload(
    message: &Message,
    history: &[Message],
    handle: &str,
    bot: Option<UserId>,
) -> Value {
    let mut earlier: Vec<&Message> = history
        .iter()
        .filter(|earlier| earlier.id < message.id)
        .collect();
    earlier.sort_by_key(|earlier| earlier.id);
    let skipped = earlier.len().saturating_sub(usize::from(HISTORY));
    let mut messages: Vec<Value> = earlier[skipped..]
        .iter()
        .filter_map(|earlier| entry(earlier, bot))
        .collect();
    messages.extend(entry(message, bot));

    let files: Vec<&str> = message
        .attachments
        .iter()
        .filter(|attachment| !is_image(attachment))
        .map(|attachment| attachment.url.as_str())
        .collect();

    json!({
        "local_session": {
            "bot_token": handle,
            "bot_user_id": bot.map(|id| id.to_string()),
            "guild_id": message.guild_id.map(|id| id.to_string()),
            "message_id": message.id.to_string(),
            "channel_id": message.channel_id.to_string(),
            "user": {
                "id": message.author.id.to_string(),
                "name": message.author.name,
                "discriminator": message.author.discriminator.map(NonZeroU16::get),
                "bot": message.author.bot,
            },
        },
        "messages": messages,
        "attachments": files,
    })
}

/// The name a person sees: the server nickname the message carries, else the display name,
/// else the username.
pub fn display_name(message: &Message) -> &str {
    message
        .member
        .as_ref()
        .and_then(|member| member.nick.as_deref())
        .or(message.author.global_name.as_deref())
        .unwrap_or(&message.author.name)
}

/// One message of `messages`, or none when it has neither text nor an image. The bot's own
/// messages carry no `name`: providers read an assistant message's name as its id.
fn entry(message: &Message, bot: Option<UserId>) -> Option<Value> {
    let mut content = Vec::new();
    if !message.content.trim().is_empty() {
        content.push(json!({
            "type": "text",
            "text": format!("{}[id: {}]: {}", display_name(message), message.author.id, message.content),
        }));
    }
    for attachment in message
        .attachments
        .iter()
        .filter(|attachment| is_image(attachment))
    {
        content.push(json!({"type": "image_url", "image_url": {"url": attachment.url}}));
    }
    if content.is_empty() {
        return None;
    }
    let own = bot.map_or(message.author.bot, |bot| message.author.id == bot);
    Some(if own {
        json!({"role": "assistant", "content": content})
    } else {
        json!({"role": "user", "content": content, "name": message.author.name})
    })
}
