//! The bot settings rule (§1.4) and the bot token (§1.10). Device and client apply the same rule.

use std::fmt;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::runner::Message;

/// The gateway intents a Discord bot's settings may name.
pub const DISCORD_INTENTS: [&str; 19] = [
    "Guilds",
    "GuildMembers",
    "GuildModeration",
    "GuildEmojisAndStickers",
    "GuildIntegrations",
    "GuildWebhooks",
    "GuildInvites",
    "GuildVoiceStates",
    "GuildPresences",
    "GuildMessages",
    "GuildMessageReactions",
    "GuildMessageTyping",
    "DirectMessages",
    "DirectMessageReactions",
    "DirectMessageTyping",
    "MessageContent",
    "GuildScheduledEvents",
    "AutoModerationConfiguration",
    "AutoModerationExecution",
];

/// The intents of a Discord bot whose settings name none.
pub const DEFAULT_DISCORD_INTENTS: [&str; 3] = ["Guilds", "GuildMessages", "MessageContent"];

const MAX_LIST_ENTRIES: usize = 256;
const MAX_LIST_ENTRY_CHARS: usize = 64;
const MAX_PREFIX_CHARS: usize = 16;
/// `event.<id>.bot_token` must stay a valid id of at most 128 characters.
const MAX_EVENT_ID_CHARS: usize = 112;
const QUOTED_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Telegram,
    Discord,
}

impl Provider {
    pub fn from_event_type(event_type: &str) -> Option<Self> {
        match event_type {
            "telegram" => Some(Self::Telegram),
            "discord" => Some(Self::Discord),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Discord => "discord",
        }
    }

    /// The provider's name in a sentence.
    pub fn name(self) -> &'static str {
        match self {
            Self::Telegram => "Telegram",
            Self::Discord => "Discord",
        }
    }

    /// The most characters one message of the provider holds.
    pub fn message_limit(self) -> usize {
        match self {
            Self::Telegram => 4096,
            Self::Discord => 2000,
        }
    }

    fn keys(self) -> Keys {
        match self {
            Self::Telegram => Keys {
                allow: "chat_whitelist",
                deny: "chat_blacklist",
                private: "respond_to_private",
            },
            Self::Discord => Keys {
                allow: "channel_whitelist",
                deny: "channel_blacklist",
                private: "respond_to_dms",
            },
        }
    }
}

struct Keys {
    allow: &'static str,
    deny: &'static str,
    private: &'static str,
}

/// Why a bot event cannot run on a device. The codes are shared with clients.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BotProblem {
    #[error("{0}")]
    Invalid(String),
    #[error("The bot token is missing")]
    TokenMissing,
    #[error("The stored bot token is not in the shape of a bot token")]
    TokenInvalid,
}

impl BotProblem {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "bot_invalid",
            Self::TokenMissing => "bot_token_missing",
            Self::TokenInvalid => "bot_token_invalid",
        }
    }
}

/// The settings of one bot event (§1.4), read from a device's config or built from what the
/// desktop app saved. `Debug` prints no chat or channel id.
#[derive(Clone, PartialEq, Eq)]
pub struct BotSpec {
    pub event_id: String,
    pub provider: Provider,
    /// The allow list is empty: anyone who can message the bot starts runs.
    pub open: bool,
    /// Chats (Telegram) or channels (Discord) that may start runs; empty allows every one.
    pub allow: Vec<String>,
    /// Chats or channels that never start a run.
    pub deny: Vec<String>,
    pub respond_to_mentions: bool,
    /// Private chats (Telegram) or direct messages (Discord).
    pub respond_to_private: bool,
    /// Empty: no prefix is set.
    pub command_prefix: String,
    /// Discord gateway intents by name, without duplicates; empty for Telegram.
    pub intents: Vec<String>,
}

/// The settings of a bot event as the desktop app saved them, for [`BotSpec::unbounded`].
/// Each field means what it means in [`BotSpec`].
#[derive(Clone, PartialEq, Eq)]
pub struct BotSettings {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub respond_to_mentions: bool,
    pub respond_to_private: bool,
    pub command_prefix: String,
}

impl BotSpec {
    /// Reads the settings of a bot event's config. The token keys are ignored: a device never
    /// receives them in the config.
    pub fn from_config(
        event_id: &str,
        event_type: &str,
        config: &[u8],
    ) -> Result<Self, BotProblem> {
        let provider = provider_of(event_type)?;
        let length = event_id.chars().count();
        if length > MAX_EVENT_ID_CHARS {
            return Err(BotProblem::Invalid(format!(
                "The event id is {length} characters long; a bot event's id has at most {MAX_EVENT_ID_CHARS}"
            )));
        }
        Self::read(event_id, provider, &object(config)?)
    }

    /// The desktop app's settings as they are: no bound of a device is checked, so any list and
    /// any prefix is taken. `intents` stays empty; the desktop app opens its own connection.
    pub fn unbounded(event_id: &str, provider: Provider, settings: BotSettings) -> Self {
        Self {
            event_id: event_id.to_string(),
            provider,
            open: settings.allow.is_empty(),
            allow: settings.allow,
            deny: settings.deny,
            respond_to_mentions: settings.respond_to_mentions,
            respond_to_private: settings.respond_to_private,
            command_prefix: settings.command_prefix,
            intents: Vec::new(),
        }
    }

    /// Checks the settings in the order a client names them: lists, flags, the prefix, then
    /// the provider's own.
    fn read(
        event_id: &str,
        provider: Provider,
        config: &Map<String, Value>,
    ) -> Result<Self, BotProblem> {
        let keys = provider.keys();
        let settings = BotSettings {
            allow: list(config, keys.allow)?,
            deny: list(config, keys.deny)?,
            respond_to_mentions: flag(config, "respond_to_mentions")?,
            respond_to_private: flag(config, keys.private)?,
            command_prefix: prefix(config)?,
        };
        let mut spec = Self::unbounded(event_id, provider, settings);
        if provider == Provider::Discord {
            spec.intents = intents(config)?;
        }
        Ok(spec)
    }

    /// Rules 3 to 5 of the filter (§5.5) for a message that passed the provider's own rules 0
    /// to 2. A device and the desktop app both decide with it.
    pub fn admits(&self, message: &Message) -> bool {
        if !self.listens_to(message) {
            return false;
        }
        if message.private {
            return self.respond_to_private;
        }
        self.starts_in_group(message)
    }

    /// Rule 3: the allow list is empty or names the chat, and the deny list does not name it.
    fn listens_to(&self, message: &Message) -> bool {
        (self.allow.is_empty() || self.allow.contains(&message.chat))
            && !self.deny.contains(&message.chat)
    }

    /// Rule 5, a group or a server channel. With a prefix: the text starts with it, or the
    /// message is addressed to the bot while it responds to mentions. Without one a Telegram
    /// bot answers every message, commands for other bots included; a Discord bot answers the
    /// messages addressed to it, and every message once it does not respond to mentions.
    fn starts_in_group(&self, message: &Message) -> bool {
        if self.command_prefix.is_empty() {
            return match self.provider {
                Provider::Telegram => true,
                Provider::Discord => !self.respond_to_mentions || message.addressed,
            };
        }
        (self.respond_to_mentions && message.addressed) || self.prefixed(message)
    }

    /// The text starts with the prefix; a command addressed to another bot is no match.
    fn prefixed(&self, message: &Message) -> bool {
        !message.foreign_command && message.text.starts_with(&self.command_prefix)
    }
}

fn provider_of(event_type: &str) -> Result<Provider, BotProblem> {
    Provider::from_event_type(event_type).ok_or_else(|| {
        BotProblem::Invalid(format!(
            "Events of type {:?} are not bots",
            shortened(event_type)
        ))
    })
}

fn object(config: &[u8]) -> Result<Map<String, Value>, BotProblem> {
    match serde_json::from_slice::<Value>(config) {
        Ok(Value::Object(config)) => Ok(config),
        _ => Err(BotProblem::Invalid(
            "The bot settings are not a JSON object".into(),
        )),
    }
}

impl fmt::Debug for BotSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BotSpec")
            .field("event_id", &self.event_id)
            .field("provider", &self.provider)
            .field("open", &self.open)
            .field("allow", &self.allow.len())
            .field("deny", &self.deny.len())
            .field("respond_to_mentions", &self.respond_to_mentions)
            .field("respond_to_private", &self.respond_to_private)
            .field("command_prefix", &self.command_prefix)
            .field("intents", &self.intents)
            .finish()
    }
}

/// A key that is absent or `null` takes its default.
fn present<'a>(config: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    config.get(key).filter(|value| !value.is_null())
}

fn list(config: &Map<String, Value>, key: &str) -> Result<Vec<String>, BotProblem> {
    let Some(value) = present(config, key) else {
        return Ok(Vec::new());
    };
    let wrong = || BotProblem::Invalid(format!("The bot setting {key:?} must be a list of texts"));
    let entries = value.as_array().ok_or_else(wrong)?;
    if entries.len() > MAX_LIST_ENTRIES {
        return Err(BotProblem::Invalid(format!(
            "The bot setting {key:?} lists {} entries; a device reads at most {MAX_LIST_ENTRIES}",
            entries.len()
        )));
    }
    let mut texts = Vec::with_capacity(entries.len());
    for entry in entries {
        let text = entry.as_str().ok_or_else(wrong)?;
        let length = text.chars().count();
        if !(1..=MAX_LIST_ENTRY_CHARS).contains(&length) {
            return Err(BotProblem::Invalid(format!(
                "An entry of the bot setting {key:?} is {length} characters long; each has 1 to {MAX_LIST_ENTRY_CHARS}"
            )));
        }
        if !texts.iter().any(|known: &String| known == text) {
            texts.push(text.to_string());
        }
    }
    Ok(texts)
}

fn flag(config: &Map<String, Value>, key: &str) -> Result<bool, BotProblem> {
    match present(config, key) {
        None => Ok(true),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(BotProblem::Invalid(format!(
            "The bot setting {key:?} must be true or false"
        ))),
    }
}

/// No key and `null` are no prefix, like `""`.
fn prefix(config: &Map<String, Value>) -> Result<String, BotProblem> {
    const KEY: &str = "command_prefix";
    match present(config, KEY) {
        None => Ok(String::new()),
        Some(Value::String(prefix)) => {
            let length = prefix.chars().count();
            if length > MAX_PREFIX_CHARS {
                return Err(BotProblem::Invalid(format!(
                    "The bot setting {KEY:?} is {length} characters long; a device reads at most {MAX_PREFIX_CHARS}"
                )));
            }
            Ok(prefix.clone())
        }
        Some(_) => Err(BotProblem::Invalid(format!(
            "The bot setting {KEY:?} must be a text"
        ))),
    }
}

fn intents(config: &Map<String, Value>) -> Result<Vec<String>, BotProblem> {
    const KEY: &str = "intents";
    let Some(value) = present(config, KEY) else {
        return Ok(DEFAULT_DISCORD_INTENTS.map(str::to_string).to_vec());
    };
    let wrong = || {
        BotProblem::Invalid(format!(
            "The bot setting {KEY:?} must be a list of Discord intent names"
        ))
    };
    let mut names: Vec<String> = Vec::new();
    for entry in value.as_array().ok_or_else(wrong)? {
        let name = entry.as_str().ok_or_else(wrong)?;
        if !DISCORD_INTENTS.contains(&name) {
            return Err(BotProblem::Invalid(format!(
                "The bot setting {KEY:?} names {:?}, which is not a Discord intent",
                shortened(name)
            )));
        }
        if !names.iter().any(|known| known == name) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

fn shortened(text: &str) -> &str {
    crate::render::cut(text, QUOTED_CHARS)
}

/// A bot's token. It is never formatted: `Debug` prints no part of it.
#[derive(Clone, PartialEq, Eq)]
pub struct BotToken(String);

impl BotToken {
    /// Reads a stored secret, a JSON string or raw text, and checks its shape without network:
    /// Telegram `^[0-9]{3,20}:[A-Za-z0-9_-]{20,128}$`; Discord three dot-separated base64url
    /// parts, 40 to 256 characters in all.
    pub fn parse(provider: Provider, secret: &[u8]) -> Result<Self, BotProblem> {
        let text = match serde_json::from_slice::<Value>(secret) {
            Ok(Value::String(text)) => text,
            _ => String::from_utf8_lossy(secret).into_owned(),
        };
        let token = text.trim();
        if token.is_empty() {
            return Err(BotProblem::TokenMissing);
        }
        let shaped = match provider {
            Provider::Telegram => telegram_shape(token),
            Provider::Discord => discord_shape(token),
        };
        if !shaped {
            return Err(BotProblem::TokenInvalid);
        }
        Ok(Self(token.to_string()))
    }

    /// The token itself, for the provider's client only.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The bot's public numeric id as the token names it: Telegram the digits before the
    /// colon, Discord the decoded first part. The provider confirms it when it connects.
    pub fn bot_id(&self) -> Option<u64> {
        let first = self.0.split([':', '.']).next()?;
        if self.0.contains(':') {
            return first.parse().ok();
        }
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(first.trim_end_matches('='))
            .ok()?;
        std::str::from_utf8(&decoded).ok()?.parse().ok()
    }
}

impl fmt::Debug for BotToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BotToken(…)")
    }
}

fn base64url(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn telegram_shape(token: &str) -> bool {
    let Some((id, secret)) = token.split_once(':') else {
        return false;
    };
    (3..=20).contains(&id.len())
        && id.bytes().all(|byte| byte.is_ascii_digit())
        && (20..=128).contains(&secret.len())
        && base64url(secret)
}

fn discord_shape(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    (40..=256).contains(&token.len())
        && parts.len() == 3
        && parts.iter().all(|part| !part.is_empty() && base64url(part))
}
