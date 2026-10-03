//! Telegram: a long poll over `getUpdates` written by hand (no dispatcher, no signal handler),
//! the filter, the payload with downloaded images, and the sender (design §5.4, §5.5).
//!
//! A `Bot` of teloxide prints its token when formatted; none is ever formatted here, and log
//! lines carry fixed codes instead of the library's error texts.

mod filter;
mod payload;
mod poll;
mod sender;

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use teloxide::prelude::*;
use teloxide::types::{Message as TelegramMessage, Update, UpdateKind};
use tokio_util::sync::CancellationToken;

use crate::config::{BotSpec, BotToken};
use crate::limits::ImageBudget;
use crate::reply::ReplySink;
use crate::runner::{Connection, End, Intake, LinkState, Message, Refusal};
use poll::{Failure, Health};
use sender::{Body, Target};

pub use filter::{Me, facts_of};

/// How long the read position may take to be confirmed at a stop.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(2);

/// The pauses of a connection. Tests shorten them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// How long one `getUpdates` waits for updates, seconds.
    pub long_poll_secs: u32,
    /// Between attempts while another program reads the bot or a webhook is set.
    pub blocked_pause: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            long_poll_secs: poll::LONG_POLL_SECS,
            blocked_pause: poll::BLOCKED_PAUSE,
        }
    }
}

/// The connection of one Telegram bot. The runner calls it once per token of a service.
pub fn connect(token: &BotToken, specs: &[BotSpec]) -> Arc<dyn Connection> {
    connect_to(None, Timing::default(), token, specs)
}

/// [`connect`] against another Bot API address, with other pauses (tests).
pub fn connect_to(
    api_url: Option<&str>,
    timing: Timing,
    token: &BotToken,
    specs: &[BotSpec],
) -> Arc<dyn Connection> {
    Arc::new(Telegram {
        bot: poll::client(token.expose(), api_url),
        timing,
        event_id: specs
            .first()
            .map(|spec| spec.event_id.clone())
            .unwrap_or_default(),
        me: Mutex::new(Me::default()),
    })
}

struct Telegram {
    bot: Option<Bot>,
    timing: Timing,
    /// The first event of the token, for log lines.
    event_id: String,
    me: Mutex<Me>,
}

fn native(message: &Message) -> Option<&TelegramMessage> {
    message.native::<TelegramMessage>()
}

/// A message that passed rules 0 to 2 in the runner's terms: its facts, its update and date,
/// and the message itself for the payload, the reply and notices.
fn incoming(me: &Me, update_id: i64, observed: bool, message: TelegramMessage) -> Message {
    let mut incoming = facts_of(&message, me);
    incoming.update_id = Some(update_id);
    incoming.sent_at = Some(message.date.timestamp());
    incoming.observed = observed;
    incoming.native = Box::new(message);
    incoming
}

/// What an update is to the intake: rule 0 (a new message, nothing else) and rules 1 and 2.
enum Arrival {
    Message(Box<TelegramMessage>),
    NotStartable,
    NoMessage,
}

fn arrival(kind: UpdateKind) -> Arrival {
    match kind {
        UpdateKind::Message(message) if filter::startable(&message) => {
            Arrival::Message(Box::new(message))
        }
        UpdateKind::Message(_) => Arrival::NotStartable,
        _ => Arrival::NoMessage,
    }
}

fn target(message: &TelegramMessage) -> Target {
    Target {
        chat: message.chat.id,
        thread: message.thread_id.filter(|_| message.is_topic_message),
        reply_to: Some(message.id),
    }
}

impl Telegram {
    fn me(&self) -> Me {
        self.me
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Reports a failed request and waits before the next attempt. False when the stop came
    /// first.
    async fn pause_after(
        &self,
        health: &mut Health,
        failure: Failure,
        intake: &Intake,
        stop: &CancellationToken,
    ) -> bool {
        let (state, pause) = health.failed(failure, self.timing.blocked_pause);
        if let Some(state) = state {
            intake.state(state);
        }
        tracing::warn!(
            event_id = self.event_id.as_str(),
            code = failure.code(),
            seconds = pause.as_secs(),
            "Telegram request failed; trying again after a pause"
        );
        tokio::select! {
            () = tokio::time::sleep(pause) => true,
            () = stop.cancelled() => false,
        }
    }

    /// Repeats `request` until it succeeds. A refused token or the stop ends it.
    async fn retried<T, F, Fut>(
        &self,
        intake: &Intake,
        stop: &CancellationToken,
        mut request: F,
    ) -> Result<T, End>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, Failure>>,
    {
        let mut health = Health::default();
        loop {
            let result = tokio::select! {
                result = request() => result,
                () = stop.cancelled() => return Err(End::Stopped),
            };
            match result {
                Ok(value) => return Ok(value),
                Err(Failure::TokenRefused) => return Err(End::Refused(Refusal::Token)),
                Err(failure) => {
                    if !self.pause_after(&mut health, failure, intake, stop).await {
                        return Err(End::Stopped);
                    }
                }
            }
        }
    }

    async fn identify(
        &self,
        bot: &Bot,
        intake: &Intake,
        stop: &CancellationToken,
    ) -> Result<Me, End> {
        let me = self
            .retried(intake, stop, || async {
                bot.get_me().await.map_err(|error| Failure::of(&error))
            })
            .await?;
        Ok(Me::of(&me))
    }

    async fn clear_webhook(
        &self,
        bot: &Bot,
        intake: &Intake,
        stop: &CancellationToken,
    ) -> Result<(), End> {
        self.retried(intake, stop, || async {
            bot.delete_webhook()
                .drop_pending_updates(false)
                .await
                .map(drop)
                .map_err(|error| Failure::of(&error))
        })
        .await?;
        intake.webhook_cleared();
        Ok(())
    }

    /// One update: a waiting flow node may take it; a message goes to the runner's intake.
    async fn take(&self, intake: &Intake, me: &Me, update: Update) {
        let update_id = i64::from(update.id.0);
        let observed = serde_json::to_value(&update).is_ok_and(|update| intake.observed(&update));
        match arrival(update.kind) {
            Arrival::Message(message) => {
                intake
                    .message(incoming(me, update_id, observed, *message))
                    .await;
            }
            Arrival::NotStartable => intake.ignore(Some(update_id)),
            Arrival::NoMessage => intake.pass(Some(update_id)),
        }
    }

    async fn poll(
        &self,
        bot: &Bot,
        intake: &Intake,
        stop: &CancellationToken,
        me: &Me,
        mut offset: Option<i32>,
    ) -> End {
        let wanted = poll::wanted();
        let mut health = Health::default();
        loop {
            let result = tokio::select! {
                result = poll::updates(bot, offset, &wanted, self.timing.long_poll_secs) => result,
                () = stop.cancelled() => break,
            };
            match result {
                Ok(updates) => {
                    if health.succeeded() {
                        intake.state(LinkState::Connected);
                    }
                    for update in updates {
                        offset = poll::after(i64::from(update.id.0)).or(offset);
                        self.take(intake, me, update).await;
                    }
                }
                Err(Failure::TokenRefused) => return End::Refused(Refusal::Token),
                Err(failure) => {
                    if !self.pause_after(&mut health, failure, intake, stop).await {
                        break;
                    }
                }
            }
        }
        if let Some(offset) = offset {
            let _ = tokio::time::timeout(CONFIRM_TIMEOUT, poll::confirm(bot, offset)).await;
        }
        End::Stopped
    }
}

#[async_trait]
impl Connection for Telegram {
    async fn serve(&self, intake: Intake, stop: CancellationToken) -> End {
        let Some(bot) = &self.bot else {
            tracing::error!(
                event_id = self.event_id.as_str(),
                "The Telegram client could not be built"
            );
            return End::Ended;
        };
        let me = match self.identify(bot, &intake, &stop).await {
            Ok(me) => me,
            Err(end) => return end,
        };
        *self.me.lock().unwrap_or_else(PoisonError::into_inner) = me.clone();
        let resume = intake.connected(me.id, Some(&me.username));
        if !resume.webhook_cleared
            && let Err(end) = self.clear_webhook(bot, &intake, &stop).await
        {
            return end;
        }
        let offset = resume.watermark.and_then(poll::after);
        self.poll(bot, &intake, &stop, &me, offset).await
    }

    async fn payload(&self, message: &Message, handle: &str, images: &mut ImageBudget) -> Value {
        match (&self.bot, native(message)) {
            (Some(bot), Some(native)) => {
                payload::build(bot, handle, &self.me(), native, images).await
            }
            _ => json!({"local_session": {"bot_token": handle}, "messages": [], "attachments": []}),
        }
    }

    fn reply(&self, message: &Message) -> Box<dyn ReplySink> {
        match (&self.bot, native(message)) {
            (Some(bot), Some(native)) => Box::new(sender::Reply::new(
                bot.clone(),
                target(native),
                native.chat.is_private(),
            )),
            _ => Box::new(sender::Silent),
        }
    }

    async fn notice(&self, message: &Message, text: &str) {
        if let (Some(bot), Some(native)) = (&self.bot, native(message)) {
            let _ = sender::send(bot, target(native), &Body::plain(text)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOT_ID: u64 = 7_123_456_789;

    fn me() -> Me {
        Me {
            id: BOT_ID,
            username: "helper_bot".to_string(),
        }
    }

    fn spec(config: Value) -> BotSpec {
        BotSpec::from_config("evt_helper", "telegram", config.to_string().as_bytes())
            .expect("valid settings")
    }

    fn person() -> Value {
        json!({"id": 4242, "is_bot": false, "first_name": "Ada"})
    }

    fn private() -> Value {
        json!({"id": 4242, "type": "private", "first_name": "Ada"})
    }

    fn group() -> Value {
        json!({"id": -100_200, "type": "supergroup", "title": "Team"})
    }

    fn message(chat: Value, fields: Value) -> Value {
        let mut message =
            json!({"message_id": 10, "date": 1_790_000_000, "chat": chat, "from": person()});
        for (key, value) in fields.as_object().expect("an object") {
            message[key] = value.clone();
        }
        message
    }

    fn update(kind: &str, message: Value) -> Update {
        let mut update = json!({"update_id": 41});
        update[kind] = message;
        serde_json::from_str(&update.to_string()).expect("a Telegram update")
    }

    /// Whether `update` starts a run of an event with `config`.
    fn starts(config: Value, update: Update) -> bool {
        match arrival(update.kind) {
            Arrival::Message(message) => spec(config).admits(&incoming(&me(), 41, false, *message)),
            Arrival::NotStartable | Arrival::NoMessage => false,
        }
    }

    fn new_message(chat: Value, fields: Value) -> Update {
        update("message", message(chat, fields))
    }

    fn mention(text: &str, offset: usize, length: usize) -> Value {
        json!({"text": text, "entities": [{"type": "mention", "offset": offset, "length": length}]})
    }

    fn command(text: &str) -> Value {
        let length = text.split(' ').next().unwrap_or_default().len();
        json!({"text": text, "entities": [{"type": "bot_command", "offset": 0, "length": length}]})
    }

    fn photo() -> Value {
        json!([{"file_id": "p1", "file_unique_id": "u1", "width": 90, "height": 90, "file_size": 1000}])
    }

    fn reply_to_the_bot() -> Value {
        json!({"text": "yes", "reply_to_message": {"message_id": 9, "date": 1_789_999_990,
            "chat": group(), "from": {"id": BOT_ID, "is_bot": true, "first_name": "Helper"}, "text": "Sure?"}})
    }

    fn slash() -> Value {
        json!({"command_prefix": "/"})
    }

    /// Whether a new message with `fields` in the group starts a run of an event with `config`.
    fn in_group(config: Value, fields: Value) -> bool {
        starts(config, new_message(group(), fields))
    }

    #[test]
    fn only_new_messages_start_runs() {
        let text = json!({"text": "hello"});
        assert!(starts(json!({}), new_message(private(), text.clone())));
        assert!(!starts(
            json!({}),
            update("edited_message", message(private(), text.clone()))
        ));
        assert!(!starts(
            json!({}),
            update("channel_post", message(group(), text))
        ));
    }

    #[test]
    fn stickers_locations_service_messages_and_bots_start_nothing() {
        let sticker = json!({"file_id": "s1", "file_unique_id": "su1", "type": "regular",
            "width": 512, "height": 512, "is_animated": false, "is_video": false});
        assert!(!starts(
            json!({}),
            new_message(private(), json!({"sticker": sticker}))
        ));
        assert!(!starts(
            json!({}),
            new_message(
                private(),
                json!({"location": {"latitude": 52.5, "longitude": 13.4}})
            )
        ));
        assert!(!starts(
            json!({}),
            new_message(group(), json!({"new_chat_members": [person()]}))
        ));
        let from_a_bot =
            json!({"text": "/ask", "from": {"id": 99, "is_bot": true, "first_name": "Other"}});
        assert!(!starts(json!({}), new_message(group(), from_a_bot)));
    }

    #[test]
    fn private_chats_follow_respond_to_private() {
        let hello = json!({"text": "hello"});
        assert!(starts(json!({}), new_message(private(), hello.clone())));
        assert!(!starts(
            json!({"respond_to_private": false}),
            new_message(private(), hello)
        ));
        assert!(starts(
            json!({}),
            new_message(private(), json!({"photo": photo()}))
        ));
    }

    #[test]
    fn groups_need_a_mention_a_reply_or_the_prefix() {
        assert!(!in_group(slash(), json!({"text": "just chatting"})));
        assert!(in_group(slash(), mention("hey @Helper_Bot look", 4, 11)));
        assert!(!in_group(slash(), mention("hey @helper_bot_fan", 4, 15)));
        let text_mention = json!({"text": "Helper, look", "entities": [{"type": "text_mention", "offset": 0,
            "length": 6, "user": {"id": BOT_ID, "is_bot": true, "first_name": "Helper"}}]});
        assert!(in_group(slash(), text_mention));
        assert!(in_group(slash(), reply_to_the_bot()));
        assert!(in_group(slash(), command("/ask what time is it")));
        assert!(!in_group(json!({"command_prefix": "!"}), command("/ask")));
    }

    #[test]
    fn groups_need_the_prefix_with_mentions_off() {
        let only = || json!({"command_prefix": "/", "respond_to_mentions": false});
        assert!(in_group(only(), command("/ask")));
        assert!(!in_group(only(), json!({"text": "just chatting"})));
        assert!(!in_group(only(), mention("hey @helper_bot look", 4, 11)));
        assert!(!in_group(only(), reply_to_the_bot()));
    }

    #[test]
    fn groups_without_a_prefix_answer_every_message() {
        let unset = [
            json!({}),
            json!({"command_prefix": null}),
            json!({"command_prefix": ""}),
            json!({"respond_to_mentions": false}),
        ];
        for config in unset {
            let answers = |fields: Value| in_group(config.clone(), fields);
            assert!(answers(json!({"text": "just chatting"})), "{config}");
            assert!(answers(command("/x@otherbot")), "{config}");
            assert!(answers(reply_to_the_bot()), "{config}");
            assert!(answers(json!({"photo": photo()})), "{config}");
            let joined = json!({"new_chat_members": [person()]});
            assert!(!answers(joined), "{config}: rules 0 to 2 come first");
        }
    }

    #[test]
    fn commands_for_another_bot_are_not_a_prefix_match() {
        assert!(!in_group(slash(), command("/x@otherbot")));
        assert!(!in_group(slash(), command("/x@helper_bot_fan now")));
        assert!(in_group(slash(), command("/x@helper_bot")));
        assert!(in_group(slash(), command("/x@HELPER_BOT now")));
    }

    #[test]
    fn a_caption_counts_like_text() {
        let captioned = |caption: &str| json!({"photo": photo(), "caption": caption});
        assert!(in_group(slash(), captioned("/describe this")));
        assert!(!in_group(slash(), captioned("nice")));
        let pdf = json!({"file_id": "d2", "file_unique_id": "du2", "mime_type": "application/pdf", "file_size": 10});
        assert!(!starts(
            json!({}),
            new_message(private(), json!({"document": pdf.clone()}))
        ));
        assert!(starts(
            json!({}),
            new_message(private(), json!({"document": pdf, "caption": "read this"}))
        ));
        let png = json!({"file_id": "d1", "file_unique_id": "du1", "mime_type": "image/png", "file_size": 10});
        assert!(starts(
            json!({}),
            new_message(private(), json!({"document": png}))
        ));
    }

    #[test]
    fn chat_lists_name_the_chat_id() {
        let ask = || new_message(group(), command("/ask"));
        assert!(starts(json!({"chat_whitelist": ["-100200"]}), ask()));
        assert!(!starts(json!({"chat_whitelist": ["123"]}), ask()));
        assert!(!starts(json!({"chat_blacklist": ["-100200"]}), ask()));
    }

    #[test]
    fn incoming_messages_carry_their_update_and_date() {
        let Arrival::Message(native) = arrival(new_message(private(), json!({"text": "hi"})).kind)
        else {
            panic!("a message");
        };
        let incoming = incoming(&me(), 41, true, *native);
        assert_eq!(incoming.update_id, Some(41));
        assert_eq!(incoming.sent_at, Some(1_790_000_000));
        assert_eq!(incoming.chat, "4242");
        assert_eq!(incoming.text, "hi");
        assert!(incoming.private && incoming.observed);
        assert!(incoming.native::<TelegramMessage>().is_some());
    }
}
