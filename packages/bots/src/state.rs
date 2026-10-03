//! The bot state document (§1.6b). It holds no token, message, chat, user or channel id and no
//! error text; the host writes it durably and the agent parent reads it as untrusted input.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::{BotSpec, Provider};
use crate::host::RunEnd;

pub const STATE_VERSION: u32 = 1;
/// The parent reads at most this much of the file.
pub const MAX_STATE_BYTES: usize = 64 * 1024;
/// Telegram restarts its update ids after a quiet week: an older watermark is ignored.
pub const WATERMARK_TTL_SECS: i64 = 518_400;
const MAX_NAME_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Revisions {
    pub config_revision: u64,
    pub intent_revision: u64,
}

/// A bot's connection as its status shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not allowed to connect: see `hold`.
    #[default]
    Waiting,
    Connecting,
    Connected,
    Reconnecting,
    TokenRefused,
    IntentsRefused,
    Conflict,
    WebhookSet,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::TokenRefused => "token_refused",
            Self::IntentsRefused => "intents_refused",
            Self::Conflict => "conflict",
            Self::WebhookSet => "webhook_set",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl From<RunEnd> for Outcome {
    fn from(end: RunEnd) -> Self {
        match end {
            RunEnd::Succeeded => Self::Succeeded,
            RunEnd::Failed => Self::Failed,
            RunEnd::Cancelled => Self::Cancelled,
            RunEnd::TimedOut => Self::TimedOut,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastRun {
    pub at: i64,
    pub finished_at: i64,
    pub outcome: Outcome,
}

/// Runs of one UTC day, kept across restarts of that day.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Day {
    pub date: String,
    pub runs: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BotEntry {
    pub provider: Provider,
    pub state: Status,
    /// The gate's hold code while `state` is `waiting`.
    pub hold: Option<String>,
    /// The bot's own public id. `watermark`, `watermark_at` and `webhook_cleared` belong to it.
    pub bot_id: Option<u64>,
    /// The bot's public handle when it matches `^[A-Za-z0-9_.\- ]{1,64}$`.
    pub bot_name: Option<String>,
    pub connected_at: Option<i64>,
    /// Telegram: this service removed the bot's webhook once and never does again.
    pub webhook_cleared: bool,
    /// Telegram: the highest update id whose handling was started or skipped.
    pub watermark: Option<i64>,
    pub watermark_at: Option<i64>,
    pub last_message_at: Option<i64>,
    pub last: Option<LastRun>,
    pub running: u32,
    pub messages: u64,
    pub runs: u64,
    pub failed: u64,
    pub ignored: u64,
    pub dropped_flood: u64,
    pub dropped_busy: u64,
    pub skipped_stale: u64,
    pub images_left_out: u64,
    pub reconnects: u64,
    pub day: Option<Day>,
}

impl Default for BotEntry {
    fn default() -> Self {
        Self::new(Provider::Telegram)
    }
}

impl BotEntry {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            state: Status::Waiting,
            hold: None,
            bot_id: None,
            bot_name: None,
            connected_at: None,
            webhook_cleared: false,
            watermark: None,
            watermark_at: None,
            last_message_at: None,
            last: None,
            running: 0,
            messages: 0,
            runs: 0,
            failed: 0,
            ignored: 0,
            dropped_flood: 0,
            dropped_busy: 0,
            skipped_stale: 0,
            images_left_out: 0,
            reconnects: 0,
            day: None,
        }
    }

    /// The watermark while it can still name this bot's updates.
    pub fn live_watermark(&self, now: i64) -> Option<i64> {
        let at = self.watermark_at?;
        self.watermark.filter(|_| now - at <= WATERMARK_TTL_SECS)
    }

    /// Takes the facts of the bot that connected: another bot than the stored one starts
    /// without the old bot's watermark and webhook mark.
    pub fn adopt_bot(&mut self, bot_id: u64) {
        if self.bot_id != Some(bot_id) {
            self.watermark = None;
            self.watermark_at = None;
            self.webhook_cleared = false;
            self.bot_name = None;
        }
        self.bot_id = Some(bot_id);
    }

    /// Counts a run that starts at `now` in "runs today".
    pub fn count_run_today(&mut self, now: i64) {
        let date = utc_date(now);
        match &mut self.day {
            Some(day) if day.date == date => day.runs += 1,
            _ => self.day = Some(Day { date, runs: 1 }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BotsState {
    pub version: u32,
    pub config_revision: u64,
    pub intent_revision: u64,
    /// False until every bot of this process got its first state.
    pub decided: bool,
    pub bots: BTreeMap<String, BotEntry>,
}

impl Default for BotsState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            config_revision: 0,
            intent_revision: 0,
            decided: false,
            bots: BTreeMap::new(),
        }
    }
}

impl BotsState {
    /// The document a placement process starts from: one entry per bot it runs, with what
    /// survives of an earlier process. The bot's identity, watermark and webhook mark always
    /// do; counters and the last run within one intent revision; "runs today" within one UTC
    /// day. Entries of events the config no longer lists are dropped.
    pub fn prepare<'a>(
        previous: Option<BotsState>,
        specs: impl IntoIterator<Item = &'a BotSpec>,
        revisions: Revisions,
        now: i64,
    ) -> Self {
        let mut previous = previous
            .filter(|state| state.version == STATE_VERSION)
            .unwrap_or_default();
        let same_intent = previous.intent_revision == revisions.intent_revision;
        let today = utc_date(now);
        let mut bots = BTreeMap::new();
        for spec in specs {
            let mut entry = BotEntry::new(spec.provider);
            if let Some(stored) = previous
                .bots
                .remove(&spec.event_id)
                .filter(|stored| stored.provider == spec.provider)
            {
                entry.bot_id = stored.bot_id;
                entry.bot_name = stored.bot_name.as_deref().and_then(public_name);
                entry.webhook_cleared = stored.webhook_cleared;
                if let Some(watermark) = stored.live_watermark(now) {
                    entry.watermark = Some(watermark);
                    entry.watermark_at = stored.watermark_at;
                }
                entry.day = stored.day.filter(|day| day.date == today);
                if same_intent {
                    entry.last_message_at = stored.last_message_at;
                    entry.last = stored.last;
                    entry.messages = stored.messages;
                    entry.runs = stored.runs;
                    entry.failed = stored.failed;
                    entry.ignored = stored.ignored;
                    entry.dropped_flood = stored.dropped_flood;
                    entry.dropped_busy = stored.dropped_busy;
                    entry.skipped_stale = stored.skipped_stale;
                    entry.images_left_out = stored.images_left_out;
                    entry.reconnects = stored.reconnects;
                }
            }
            bots.insert(spec.event_id.clone(), entry);
        }
        Self {
            version: STATE_VERSION,
            config_revision: revisions.config_revision,
            intent_revision: revisions.intent_revision,
            decided: false,
            bots,
        }
    }
}

/// `name` when it may be shown as a bot's public handle.
pub fn public_name(name: &str) -> Option<String> {
    let length = name.chars().count();
    let allowed = |character: char| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-' | ' ')
    };
    ((1..=MAX_NAME_CHARS).contains(&length) && name.chars().all(allowed)).then(|| name.to_string())
}

/// The UTC date of a Unix time, `YYYY-MM-DD`.
pub fn utc_date(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}
