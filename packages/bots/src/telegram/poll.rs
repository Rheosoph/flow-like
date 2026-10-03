//! The long poll over `getUpdates`: the client, the error classes and the pauses between
//! attempts (design §5.4).

use std::time::Duration;

use teloxide::prelude::*;
use teloxide::types::{AllowedUpdate, Update};
use teloxide::{ApiError, RequestError};

use crate::runner::LinkState;

pub(crate) const LONG_POLL_SECS: u32 = 25;
const BATCH: u8 = 100;
/// Longer than the long poll, so a quiet poll ends with an answer of Telegram, not a timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(LONG_POLL_SECS as u64 + 15);
const FIRST_PAUSE: Duration = Duration::from_secs(1);
const LONGEST_PAUSE: Duration = Duration::from_secs(64);
/// One attempt a minute while another consumer reads the bot or a webhook is set.
pub(crate) const BLOCKED_PAUSE: Duration = Duration::from_secs(60);
/// Polls in a row that must succeed after a conflict before the bot counts as connected.
const POLLS_AFTER_CONFLICT: u32 = 3;
/// Failures in a row from which the bot counts as reconnecting.
const FAILURES_UNTIL_RECONNECTING: u32 = 2;

/// What an error of a Telegram request means for the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Telegram refused the token: no further request in this process.
    TokenRefused,
    /// Another program reads the updates of this bot.
    Conflict,
    /// Telegram sends the updates of this bot to a webhook.
    WebhookSet,
    /// Telegram asked to wait this long.
    RetryAfter(Duration),
    /// The network, the server or an answer that could not be read: try again later.
    Transient,
}

impl Failure {
    pub(crate) fn of(error: &RequestError) -> Self {
        match error {
            RequestError::Api(ApiError::InvalidToken) => Self::TokenRefused,
            RequestError::Api(ApiError::TerminatedByOtherGetUpdates) => Self::Conflict,
            RequestError::Api(ApiError::CantGetUpdates) => Self::WebhookSet,
            RequestError::Api(ApiError::Unknown(text)) if text.contains("webhook is active") => {
                Self::WebhookSet
            }
            RequestError::Api(ApiError::Unknown(text))
                if text.contains("terminated by other getUpdates") =>
            {
                Self::Conflict
            }
            RequestError::RetryAfter(wait) => Self::RetryAfter(wait.duration()),
            _ => Self::Transient,
        }
    }

    /// A fixed code for log lines, never the text of an error of the library.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::TokenRefused => "token_refused",
            Self::Conflict => "conflict",
            Self::WebhookSet => "webhook_set",
            Self::RetryAfter(_) => "retry_after",
            Self::Transient => "unreachable",
        }
    }
}

/// Growing pauses after transient failures: 1, 2, 4 … 64 seconds, from the start again after
/// a success.
#[derive(Debug)]
struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { next: FIRST_PAUSE }
    }
}

impl Backoff {
    fn pause(&mut self) -> Duration {
        let pause = self.next;
        self.next = (self.next * 2).min(LONGEST_PAUSE);
        pause
    }
}

/// The health of a connection over its requests in a row: the state it reports and the pause
/// before its next attempt.
#[derive(Debug, Default)]
pub(crate) struct Health {
    backoff: Backoff,
    failures: u32,
    blocked: Option<Failure>,
    successes: u32,
}

impl Health {
    /// A request succeeded. True when the bot counts as connected (again): after a conflict
    /// only once three polls in a row succeeded.
    pub(crate) fn succeeded(&mut self) -> bool {
        self.backoff = Backoff::default();
        self.failures = 0;
        self.successes += 1;
        let healthy =
            self.blocked != Some(Failure::Conflict) || self.successes >= POLLS_AFTER_CONFLICT;
        if healthy {
            self.blocked = None;
        }
        healthy
    }

    /// A request failed: the state to report, if it changes, and the pause before the next
    /// attempt.
    pub(crate) fn failed(
        &mut self,
        failure: Failure,
        blocked_pause: Duration,
    ) -> (Option<LinkState>, Duration) {
        self.successes = 0;
        match failure {
            Failure::Conflict => {
                self.blocked = Some(failure);
                (Some(LinkState::Conflict), blocked_pause)
            }
            Failure::WebhookSet => {
                self.blocked = Some(failure);
                (Some(LinkState::WebhookSet), blocked_pause)
            }
            Failure::RetryAfter(wait) => (None, wait),
            Failure::Transient | Failure::TokenRefused => {
                self.failures += 1;
                let reconnecting =
                    self.failures >= FAILURES_UNTIL_RECONNECTING && self.blocked.is_none();
                (
                    reconnecting.then_some(LinkState::Reconnecting),
                    self.backoff.pause(),
                )
            }
        }
    }
}

/// A client for `token` whose requests outlast the long poll; `api_url` replaces the address
/// of Telegram (tests).
pub(crate) fn client(token: &str, api_url: Option<&str>) -> Option<Bot> {
    let http = teloxide::net::default_reqwest_settings()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .ok()?;
    let bot = Bot::with_client(token, http);
    match api_url {
        Some(address) => {
            let url = bot.api_url().join(address).ok()?;
            Some(bot.set_api_url(url))
        }
        None => Some(bot),
    }
}

/// The update kinds the poller asks for: new messages, and button presses for the flow nodes
/// that wait for them.
pub(crate) fn wanted() -> Vec<AllowedUpdate> {
    vec![AllowedUpdate::Message, AllowedUpdate::CallbackQuery]
}

/// One long poll of up to `wait_secs` from `offset`.
pub(crate) async fn updates(
    bot: &Bot,
    offset: Option<i32>,
    wanted: &[AllowedUpdate],
    wait_secs: u32,
) -> Result<Vec<Update>, Failure> {
    let mut request = bot
        .get_updates()
        .timeout(wait_secs)
        .limit(BATCH)
        .allowed_updates(wanted.to_vec());
    if let Some(offset) = offset {
        request = request.offset(offset);
    }
    request.await.map_err(|error| Failure::of(&error))
}

/// Tells Telegram that every update below `offset` was read, without taking a new one.
pub(crate) async fn confirm(bot: &Bot, offset: i32) -> Result<(), Failure> {
    bot.get_updates()
        .offset(offset)
        .timeout(0)
        .limit(1)
        .await
        .map(drop)
        .map_err(|error| Failure::of(&error))
}

/// The offset that confirms every update up to `id`.
pub(crate) fn after(id: i64) -> Option<i32> {
    i32::try_from(id.saturating_add(1)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAUSE: Duration = Duration::from_secs(60);

    fn api_error(description: &str) -> RequestError {
        RequestError::Api(
            serde_json::from_value(serde_json::Value::String(description.into()))
                .expect("a description of an error by Telegram"),
        )
    }

    #[test]
    fn errors_map_to_their_classes() {
        let webhook = "use getUpdates method while webhook is active";
        let cases = [
            ("Unauthorized", Failure::TokenRefused),
            ("Not Found", Failure::TokenRefused),
            (
                "Conflict: terminated by other getUpdates request; make sure that only one bot instance is running",
                Failure::Conflict,
            ),
            (
                &format!(
                    "Conflict: can\u{27}t {webhook}; use deleteWebhook to delete the webhook first"
                ),
                Failure::WebhookSet,
            ),
            (&format!("can\u{27}t {webhook}"), Failure::WebhookSet),
            ("Internal Server Error", Failure::Transient),
        ];
        for (description, class) in cases {
            assert_eq!(Failure::of(&api_error(description)), class, "{description}");
        }
        let wait = teloxide::types::Seconds::from_seconds(7);
        assert_eq!(
            Failure::of(&RequestError::RetryAfter(wait)),
            Failure::RetryAfter(Duration::from_secs(7))
        );
    }

    #[test]
    fn pauses_grow_to_a_minute_and_start_over_after_a_success() {
        let mut health = Health::default();
        let pauses: Vec<u64> = (0..8)
            .map(|_| health.failed(Failure::Transient, PAUSE).1.as_secs())
            .collect();
        assert_eq!(pauses, [1, 2, 4, 8, 16, 32, 64, 64]);
        assert!(health.succeeded());
        assert_eq!(
            health.failed(Failure::Transient, PAUSE).1,
            Duration::from_secs(1)
        );
    }

    #[test]
    fn reconnecting_starts_with_the_second_failure_in_a_row() {
        let mut health = Health::default();
        assert_eq!(health.failed(Failure::Transient, PAUSE).0, None);
        assert_eq!(
            health.failed(Failure::Transient, PAUSE).0,
            Some(LinkState::Reconnecting)
        );
        let wait = Duration::from_secs(3);
        assert_eq!(
            health.failed(Failure::RetryAfter(wait), PAUSE),
            (None, wait)
        );
    }

    #[test]
    fn a_conflict_needs_three_polls_in_a_row_and_a_webhook_one() {
        let mut health = Health::default();
        assert_eq!(
            health.failed(Failure::Conflict, PAUSE),
            (Some(LinkState::Conflict), PAUSE)
        );
        assert!(!health.succeeded());
        assert!(!health.succeeded());
        assert!(health.succeeded());

        assert_eq!(
            health.failed(Failure::WebhookSet, PAUSE),
            (Some(LinkState::WebhookSet), PAUSE)
        );
        assert!(health.succeeded());
    }

    #[test]
    fn the_offset_confirms_everything_up_to_an_id() {
        assert_eq!(after(412_345_678), Some(412_345_679));
        assert_eq!(after(i64::from(i32::MAX)), None);
    }
}
