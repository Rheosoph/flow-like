//! Which Telegram messages start a run (design §5.5). Rules 0 to 2 are decided here; the
//! chat lists, private chats and the prefix (rules 3 to 5) by [`crate::config::BotSpec::admits`]
//! on the facts this module reads from the message.

use teloxide::types::{Document, Message, MessageEntityKind, User};

use crate::runner;

/// The bot itself, as `getMe` reported it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Me {
    pub id: u64,
    /// Without the `@`.
    pub username: String,
}

impl Me {
    /// The bot from the user `getMe` answers with.
    pub fn of(user: &User) -> Self {
        Self {
            id: user.id.0,
            username: user.username.clone().unwrap_or_default(),
        }
    }
}

/// What the settings rules 3 to 5 read of a message: its chat, whether that is a private
/// chat, whether it is addressed to the bot, its text or caption, and whether its leading
/// command names another bot. The message is not kept, so the desktop app reads its own
/// messages with it.
pub fn facts_of(message: &Message, me: &Me) -> runner::Message {
    let mut facts = runner::Message::new(message.chat.id.0.to_string(), ());
    facts.private = message.chat.is_private();
    facts.addressed = addressed(me, message);
    facts.text = text_of(message).to_string();
    facts.foreign_command = foreign_command(me, message);
    facts
}

/// Rules 1 and 2: a person wrote it, and it has text, a caption or an image.
pub(crate) fn startable(message: &Message) -> bool {
    let person = message.from.as_ref().is_some_and(|author| !author.is_bot);
    person && (message.text().is_some() || message.caption().is_some() || has_image(message))
}

pub(crate) fn has_image(message: &Message) -> bool {
    message.photo().is_some() || message.document().is_some_and(is_image_document)
}

pub(crate) fn is_image_document(document: &Document) -> bool {
    document
        .mime_type
        .as_ref()
        .is_some_and(|mime| mime.type_().as_str() == "image")
}

/// The text the prefix is matched against: the text, or the caption.
fn text_of(message: &Message) -> &str {
    message
        .text()
        .or_else(|| message.caption())
        .unwrap_or_default()
}

/// The bot is mentioned, or the message replies to one of the bot's messages.
fn addressed(me: &Me, message: &Message) -> bool {
    mentions(me, message) || replies_to(me, message)
}

fn mentions(me: &Me, message: &Message) -> bool {
    let entities = message
        .parse_entities()
        .or_else(|| message.parse_caption_entities())
        .unwrap_or_default();
    entities.iter().any(|entity| match entity.kind() {
        MessageEntityKind::Mention => entity
            .text()
            .strip_prefix('@')
            .is_some_and(|name| name.eq_ignore_ascii_case(&me.username)),
        MessageEntityKind::TextMention { user } => user.id.0 == me.id,
        _ => false,
    })
}

fn replies_to(me: &Me, message: &Message) -> bool {
    message
        .reply_to_message()
        .and_then(|replied| replied.from.as_ref())
        .is_some_and(|author| author.id.0 == me.id)
}

/// The leading command names another bot, as `/start@otherbot` does.
fn foreign_command(me: &Me, message: &Message) -> bool {
    let command = text_of(message)
        .split(char::is_whitespace)
        .next()
        .unwrap_or_default();
    command.rsplit_once('@').is_some_and(|(_, addressee)| {
        !addressee.is_empty() && !addressee.eq_ignore_ascii_case(&me.username)
    })
}
