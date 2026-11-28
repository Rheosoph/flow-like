//! Per-bot Discord settings in one JSON file of the app data dir, and the bot's token in the
//! system keychain.

use crate::event_sink::bot_vault;
use anyhow::{Context, Result, bail};
use flow_like_bots::{BotSettings, BotSpec, BotToken, Provider};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const MAX_CHANNELS: usize = 256;
const MAX_BOT_ID: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscordSettings {
    pub enabled: bool,
    #[serde(default = "on")]
    pub respond_to_mentions: bool,
    #[serde(default = "on")]
    pub respond_to_dms: bool,
    #[serde(default)]
    pub channel_allow: Vec<String>,
}

fn on() -> bool {
    true
}

impl DiscordSettings {
    /// The rule that admits messages: the allow list, mentions and replies in servers, DMs.
    pub fn spec(&self, bot_id: &str) -> BotSpec {
        BotSpec::unbounded(
            bot_id,
            Provider::Discord,
            BotSettings {
                allow: self.channel_allow.clone(),
                deny: Vec::new(),
                respond_to_mentions: self.respond_to_mentions,
                respond_to_private: self.respond_to_dms,
                command_prefix: String::new(),
            },
        )
    }

    pub fn validated(mut self) -> Result<Self> {
        if self.channel_allow.len() > MAX_CHANNELS {
            bail!(
                "The channel allow list has {} entries; at most {MAX_CHANNELS} are allowed",
                self.channel_allow.len()
            );
        }
        for id in &mut self.channel_allow {
            *id = id.trim().to_owned();
            if id.is_empty() || id.len() > 20 || !id.bytes().all(|byte| byte.is_ascii_digit()) {
                bail!("Discord channel ids are numbers; {id:?} is not one");
            }
        }
        self.channel_allow.sort();
        self.channel_allow.dedup();
        Ok(self)
    }
}

pub fn check_bot_id(bot_id: &str) -> Result<()> {
    let valid = !bot_id.is_empty()
        && bot_id.len() <= MAX_BOT_ID
        && bot_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        bail!("{bot_id:?} is not a bot id")
    }
}

pub fn token_account(bot_id: &str) -> String {
    format!("bot-channel-discord:{bot_id}")
}

pub fn token_of(bot_id: &str) -> Result<Option<String>> {
    bot_vault::get_account(&token_account(bot_id))
}

pub fn keep_token(bot_id: &str, token: &str) -> Result<()> {
    BotToken::parse(Provider::Discord, token.as_bytes())
        .map_err(|_| anyhow::anyhow!("That does not look like a Discord bot token"))?;
    bot_vault::put_account(&token_account(bot_id), token.trim())
        .context("Cannot store the Discord bot token")
}

pub fn forget_token(bot_id: &str) -> Result<()> {
    bot_vault::drop_account(&token_account(bot_id))
}

pub struct SettingsFile(PathBuf);

impl SettingsFile {
    pub fn new(dir: &Path) -> Self {
        Self(dir.join("bot-channels-discord.json"))
    }

    pub fn load(&self) -> BTreeMap<String, DiscordSettings> {
        match std::fs::read(&self.0) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!(%error, "Discord channel settings are unreadable; starting empty");
                BTreeMap::new()
            }),
            Err(_) => BTreeMap::new(),
        }
    }

    pub fn save(&self, all: &BTreeMap<String, DiscordSettings>) -> Result<()> {
        if let Some(dir) = self.0.parent() {
            std::fs::create_dir_all(dir).context("Cannot create the app data directory")?;
        }
        let tmp = self.0.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(all)?)
            .context("Cannot write the Discord channel settings")?;
        std::fs::rename(&tmp, &self.0).context("Cannot replace the Discord channel settings")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_bots::runner::Message;

    fn settings() -> DiscordSettings {
        DiscordSettings {
            enabled: true,
            respond_to_mentions: true,
            respond_to_dms: true,
            channel_allow: vec![],
        }
    }

    fn message(chat: &str, private: bool, addressed: bool) -> Message {
        let mut message = Message::new(chat.to_owned(), ());
        message.private = private;
        message.addressed = addressed;
        message
    }

    #[test]
    fn mentions_and_dms_map_to_the_spec() {
        let spec = settings().spec("b1");
        assert!(spec.admits(&message("1", false, true)));
        assert!(!spec.admits(&message("1", false, false)));
        assert!(spec.admits(&message("1", true, false)));

        let spec = DiscordSettings {
            respond_to_dms: false,
            respond_to_mentions: false,
            ..settings()
        }
        .spec("b1");
        assert!(spec.admits(&message("1", false, false)));
        assert!(!spec.admits(&message("1", true, false)));
    }

    #[test]
    fn the_allow_list_limits_channels() {
        let spec = DiscordSettings {
            channel_allow: vec!["5".into()],
            ..settings()
        }
        .spec("b1");
        assert!(spec.admits(&message("5", false, true)));
        assert!(!spec.admits(&message("6", false, true)));
    }

    #[test]
    fn channel_ids_are_validated_and_deduplicated() {
        let ok = DiscordSettings {
            channel_allow: vec![" 12 ".into(), "12".into(), "7".into()],
            ..settings()
        }
        .validated()
        .unwrap();
        assert_eq!(ok.channel_allow, ["12", "7"]);
        let bad = DiscordSettings {
            channel_allow: vec!["general".into()],
            ..settings()
        };
        assert!(bad.validated().is_err());
        let many = DiscordSettings {
            channel_allow: (0..=MAX_CHANNELS).map(|n| n.to_string()).collect(),
            ..settings()
        };
        assert!(many.validated().is_err());
    }

    #[test]
    fn bot_ids_are_checked() {
        assert!(check_bot_id("bot_1-a.b").is_ok());
        assert!(check_bot_id("").is_err());
        assert!(check_bot_id("a/b").is_err());
    }

    #[test]
    fn settings_round_trip_atomically() {
        let dir = std::env::temp_dir().join(format!("bc-{}", uuid::Uuid::new_v4()));
        let file = SettingsFile::new(&dir);
        assert!(file.load().is_empty());
        let mut all = BTreeMap::new();
        all.insert("b1".to_owned(), settings());
        file.save(&all).unwrap();
        assert_eq!(file.load(), all);
        let _ = std::fs::remove_dir_all(dir);
    }
}
