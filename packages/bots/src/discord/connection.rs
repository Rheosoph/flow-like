//! The gateway connection: which intents it asks for, what its stages mean for the bot's
//! state, and how the client ended. Pure functions, testable without a gateway.

use serenity::gateway::{ConnectionStage, GatewayError};
use serenity::model::gateway::GatewayIntents;

use crate::config::BotSpec;
use crate::runner::{End, LinkState, Refusal};

/// The intents of one connection: the union of its events' settings, plus `DirectMessages`
/// for every event that answers direct messages (Discord sends none without it).
pub fn intents<'a>(specs: impl IntoIterator<Item = &'a BotSpec>) -> GatewayIntents {
    specs
        .into_iter()
        .fold(GatewayIntents::empty(), |mut intents, spec| {
            for name in &spec.intents {
                intents |= intent(name);
            }
            if spec.respond_to_private {
                intents |= GatewayIntents::DIRECT_MESSAGES;
            }
            intents
        })
}

/// An intent by the name the settings use; a name the settings rule refuses is none.
pub fn intent(name: &str) -> GatewayIntents {
    match name {
        "Guilds" => GatewayIntents::GUILDS,
        "GuildMembers" => GatewayIntents::GUILD_MEMBERS,
        "GuildModeration" => GatewayIntents::GUILD_MODERATION,
        "GuildEmojisAndStickers" => GatewayIntents::GUILD_EMOJIS_AND_STICKERS,
        "GuildIntegrations" => GatewayIntents::GUILD_INTEGRATIONS,
        "GuildWebhooks" => GatewayIntents::GUILD_WEBHOOKS,
        "GuildInvites" => GatewayIntents::GUILD_INVITES,
        "GuildVoiceStates" => GatewayIntents::GUILD_VOICE_STATES,
        "GuildPresences" => GatewayIntents::GUILD_PRESENCES,
        "GuildMessages" => GatewayIntents::GUILD_MESSAGES,
        "GuildMessageReactions" => GatewayIntents::GUILD_MESSAGE_REACTIONS,
        "GuildMessageTyping" => GatewayIntents::GUILD_MESSAGE_TYPING,
        "DirectMessages" => GatewayIntents::DIRECT_MESSAGES,
        "DirectMessageReactions" => GatewayIntents::DIRECT_MESSAGE_REACTIONS,
        "DirectMessageTyping" => GatewayIntents::DIRECT_MESSAGE_TYPING,
        "MessageContent" => GatewayIntents::MESSAGE_CONTENT,
        "GuildScheduledEvents" => GatewayIntents::GUILD_SCHEDULED_EVENTS,
        "AutoModerationConfiguration" => GatewayIntents::AUTO_MODERATION_CONFIGURATION,
        "AutoModerationExecution" => GatewayIntents::AUTO_MODERATION_EXECUTION,
        _ => GatewayIntents::empty(),
    }
}

/// The state a stage change reports once the bot connected (`READY`). Serenity reconnects and
/// resumes by itself and reports each step as a stage; before the first `READY` the runner
/// already shows "connecting".
pub fn stage_link(stage: ConnectionStage, connected: bool) -> Option<LinkState> {
    if !connected {
        return None;
    }
    Some(match stage {
        ConnectionStage::Connected => LinkState::Connected,
        _ => LinkState::Reconnecting,
    })
}

/// Why `Client::start` returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientEnd {
    /// Discord refused the token.
    TokenRefused,
    /// Discord refused the intents: a privileged one is not enabled in the Developer Portal.
    IntentsRefused,
    /// Any other end: a close code serenity does not retry, a failed boot, a shutdown nobody
    /// asked for.
    Ended,
}

impl ClientEnd {
    pub fn of(result: &serenity::Result<()>) -> Self {
        match result {
            Err(serenity::Error::Gateway(GatewayError::InvalidAuthentication)) => {
                Self::TokenRefused
            }
            Err(serenity::Error::Gateway(
                GatewayError::DisallowedGatewayIntents | GatewayError::InvalidGatewayIntents,
            )) => Self::IntentsRefused,
            _ => Self::Ended,
        }
    }

    /// The runner's end: a refusal stops the bot until the service restarts; any other end
    /// goes to the runner's bounded restart.
    pub fn end(self) -> End {
        match self {
            Self::TokenRefused => End::Refused(Refusal::Token),
            Self::IntentsRefused => End::Refused(Refusal::Intents),
            Self::Ended => End::Ended,
        }
    }

    /// A fixed code for log lines: the library's error text is never logged.
    pub fn code(self) -> &'static str {
        match self {
            Self::TokenRefused => "token_refused",
            Self::IntentsRefused => "intents_refused",
            Self::Ended => "client_ended",
        }
    }
}
