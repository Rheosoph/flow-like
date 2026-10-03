//! Which Discord gateway events start a run (design §5.5, rules 0 to 2) and the facts the
//! settings rules 3 to 5 read. Pure functions over serenity's models: no client, no cache.

use serenity::model::channel::{Attachment, Message, MessageType};
use serenity::model::event::Event;
use serenity::model::id::UserId;

use crate::runner;

/// What a gateway event is to the filter.
#[derive(Debug)]
pub enum Verdict<'a> {
    /// A new message of a person with text or an image: rules 3 to 5 decide.
    Candidate(&'a Message),
    /// A new message that fails rules 0 to 2: counted as ignored.
    Ignored,
    /// No new message, or one the bot sent itself.
    Unrelated,
}

/// Rules 0 to 2. Rule 0: a `MESSAGE_CREATE` of the kind Default or Reply; an edit arrives as
/// `MESSAGE_UPDATE` (Discord sends one for every link preview) and never starts a run. Rule 1:
/// a person wrote it, not a bot and not a webhook. Rule 2: it has text or an image.
pub fn verdict(event: &Event, bot: Option<UserId>) -> Verdict<'_> {
    let Event::MessageCreate(created) = event else {
        return Verdict::Unrelated;
    };
    let message = &created.message;
    if bot.is_some_and(|bot| message.author.id == bot) {
        return Verdict::Unrelated;
    }
    let kind = matches!(
        message.kind,
        MessageType::Regular | MessageType::InlineReply
    );
    if kind && startable(message) {
        Verdict::Candidate(message)
    } else {
        Verdict::Ignored
    }
}

/// Rules 1 and 2.
pub fn startable(message: &Message) -> bool {
    !message.author.bot
        && message.webhook_id.is_none()
        && (!message.content.trim().is_empty() || message.attachments.iter().any(is_image))
}

pub fn is_image(attachment: &Attachment) -> bool {
    attachment
        .content_type
        .as_deref()
        .is_some_and(|kind| kind.starts_with("image/"))
}

/// A direct message: Discord names no server for it.
pub fn private(message: &Message) -> bool {
    message.guild_id.is_none()
}

/// The bot is mentioned in the message, or the message replies to one of the bot's.
pub fn addressed(message: &Message, bot: UserId) -> bool {
    message.mentions.iter().any(|user| user.id == bot)
        || message
            .referenced_message
            .as_ref()
            .is_some_and(|replied| replied.author.id == bot)
}

/// What the settings rules 3 to 5 read of a message: its channel, whether it is a direct
/// message, whether it is addressed to the bot, and its content. The message is not kept, so
/// the desktop app reads its own messages with it.
pub fn facts_of(message: &Message, bot: Option<UserId>) -> runner::Message {
    let mut facts = runner::Message::new(message.channel_id.to_string(), ());
    facts.private = private(message);
    facts.addressed = bot.is_some_and(|bot| addressed(message, bot));
    facts.text = message.content.clone();
    facts
}

/// A candidate in the runner's terms; the message itself travels along for the payload, the
/// reply and notices. Discord keeps no backlog, so there is no age to check.
pub fn facts(message: Message, bot: Option<UserId>) -> runner::Message {
    let mut facts = facts_of(&message, bot);
    facts.native = Box::new(message);
    facts
}
