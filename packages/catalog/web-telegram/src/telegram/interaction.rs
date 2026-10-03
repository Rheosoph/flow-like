//! Telegram user interaction - wait for replies, callbacks, and reactions

use super::message::SentMessage;
use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::{
    TelegramBroadcastEvent, UpdateMatch, UpdateWaiter, get_telegram_bot, has_poller,
    wait_for_update,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};
#[cfg(feature = "execute")]
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::{AllowedUpdate, Message as TgMessage, ParseMode};
#[cfg(feature = "execute")]
use tokio::{sync::oneshot, task::AbortHandle};

/// Represents a user's reply message
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserReply {
    /// The message ID of the reply
    pub message_id: String,
    /// The chat ID where the reply was received
    pub chat_id: String,
    /// The user who replied
    pub user_id: Option<String>,
    /// The username (if available)
    pub username: Option<String>,
    /// The text content of the reply
    pub text: Option<String>,
    /// Unix timestamp
    pub date: i64,
    /// Whether the reply has a photo
    pub has_photo: bool,
    /// Whether the reply has a document
    pub has_document: bool,
    /// Whether the reply has a video
    pub has_video: bool,
    /// Whether the reply has an audio
    pub has_audio: bool,
}

#[cfg(feature = "execute")]
impl From<&TgMessage> for UserReply {
    fn from(msg: &TgMessage) -> Self {
        Self {
            message_id: msg.id.0.to_string(),
            chat_id: msg.chat.id.0.to_string(),
            user_id: msg.from.as_ref().map(|u| u.id.0.to_string()),
            username: msg.from.as_ref().and_then(|u| u.username.clone()),
            text: msg.text().map(|s| s.to_string()),
            date: msg.date.timestamp(),
            has_photo: msg.photo().is_some(),
            has_document: msg.document().is_some(),
            has_video: msg.video().is_some(),
            has_audio: msg.audio().is_some(),
        }
    }
}

/// Callback query from inline keyboard button
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CallbackResponse {
    /// The callback query ID
    pub callback_id: String,
    /// The data associated with the callback button
    pub data: String,
    /// The user who clicked the button
    pub user_id: String,
    /// The username (if available)
    pub username: Option<String>,
    /// The chat ID where the interaction happened
    pub chat_id: Option<String>,
    /// The message ID of the message with the button
    pub message_id: Option<String>,
}

// ============================================================================
// Waiting for an update
// ============================================================================

#[cfg(feature = "execute")]
struct AbortOnDrop(AbortHandle);

#[cfg(feature = "execute")]
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Where a wait gets its update: its own poll task, or the device poller of its token. Each
/// ends its part when dropped.
#[cfg(feature = "execute")]
enum Source {
    Poll { _task: AbortOnDrop },
    Device { _queue: UpdateWaiter },
}

/// One wait of a node. Its poll task ends, or its place in the device poller's queue is
/// given up, when the wait ends, however it ends.
#[cfg(feature = "execute")]
pub(crate) struct PendingWait {
    received: oneshot::Receiver<TelegramBroadcastEvent>,
    source: Source,
}

#[cfg(feature = "execute")]
impl PendingWait {
    #[cfg(test)]
    pub(crate) fn poll_task(&self) -> Option<&AbortHandle> {
        match &self.source {
            Source::Poll { _task: task } => Some(&task.0),
            Source::Device { .. } => None,
        }
    }

    /// The first matching update, or `None` when `limit` passed first.
    pub(crate) async fn finish(self, limit: Option<Duration>) -> Option<TelegramBroadcastEvent> {
        let Self { received, source } = self;
        let event = match limit {
            Some(limit) => tokio::time::timeout(limit, received)
                .await
                .ok()
                .and_then(Result::ok),
            None => received.await.ok(),
        };
        drop(source);
        event
    }
}

/// Starts waiting for the first `allowed` update that `matches`. A token that a device
/// poller reads is never polled here: the wait queues for that poller's updates.
#[cfg(feature = "execute")]
pub(crate) fn begin_wait(bot: &Bot, allowed: AllowedUpdate, matches: UpdateMatch) -> PendingWait {
    if has_poller(bot.token()) {
        let (queue, received) = wait_for_update(bot.token(), matches);
        return PendingWait {
            received,
            source: Source::Device { _queue: queue },
        };
    }
    let (sender, received) = oneshot::channel();
    let task = tokio::spawn(poll_for_update(bot.clone(), allowed, matches, sender));
    PendingWait {
        received,
        source: Source::Poll {
            _task: AbortOnDrop(task.abort_handle()),
        },
    }
}

#[cfg(feature = "execute")]
async fn poll_for_update(
    bot: Bot,
    allowed: AllowedUpdate,
    matches: UpdateMatch,
    sender: oneshot::Sender<TelegramBroadcastEvent>,
) {
    let mut offset: i32 = 0;
    while !sender.is_closed() {
        if let Ok(updates) = bot
            .get_updates()
            .offset(offset)
            .timeout(5_u32)
            .allowed_updates(vec![allowed])
            .await
        {
            for update in updates {
                offset = (update.id.0 as i32).saturating_add(1);
                if let Some(event) = TelegramBroadcastEvent::from_update(update)
                    && matches(&event)
                {
                    let _ = sender.send(event);
                    return;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(feature = "execute")]
fn wait_limit(timeout_secs: i64) -> Option<Duration> {
    u64::try_from(timeout_secs)
        .ok()
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
}

// ============================================================================
// Wait For Reply Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct WaitForReplyNode;

impl WaitForReplyNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for WaitForReplyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_wait_for_reply",
            "Wait For Reply",
            "Waits for a user to reply to a message. Useful for dialogs and verification flows.",
            "Telegram/Interaction",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "message_ref",
            "Message Reference",
            "The message to wait for a reply to (optional - if not set, waits for any message)",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>();

        node.add_input_pin(
            "from_user_id",
            "From User ID",
            "Only accept replies from this user ID (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "timeout_seconds",
            "Timeout (seconds)",
            "Maximum time to wait (0 or negative = no timeout)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(60)));

        node.add_output_pin(
            "on_reply",
            "On Reply",
            "Triggered when a reply is received",
            VariableType::Execution,
        );

        node.add_output_pin(
            "on_timeout",
            "On Timeout",
            "Triggered when timeout is reached",
            VariableType::Execution,
        );

        node.add_output_pin(
            "reply",
            "Reply",
            "The user's reply message",
            VariableType::Struct,
        )
        .set_schema::<UserReply>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "reply_text",
            "Reply Text",
            "The text content of the reply (convenience output)",
            VariableType::String,
        );

        node.add_output_pin(
            "timed_out",
            "Timed Out",
            "Whether the wait timed out",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let timeout_secs: i64 = context
            .evaluate_pin::<i64>("timeout_seconds")
            .await
            .unwrap_or(60);

        let message_ref: Option<SentMessage> = context.evaluate_pin("message_ref").await.ok();
        let from_user_id: Option<String> = context.evaluate_pin("from_user_id").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let reply_to_id = message_ref
            .as_ref()
            .and_then(|m| m.message_id.parse::<i32>().ok());
        let filter_user_id = from_user_id.and_then(|id| id.parse::<u64>().ok());

        let wait = begin_wait(
            &bot.bot,
            AllowedUpdate::Message,
            Arc::new(move |event| {
                let TelegramBroadcastEvent::Message(msg) = event else {
                    return false;
                };
                msg.chat.id == chat_id
                    && reply_to_id.is_none_or(|expected| {
                        msg.reply_to_message()
                            .is_some_and(|replied| replied.id.0 == expected)
                    })
                    && filter_user_id.is_none_or(|expected| {
                        msg.from.as_ref().is_some_and(|from| from.id.0 == expected)
                    })
            }),
        );

        if let Some(TelegramBroadcastEvent::Message(msg)) =
            wait.finish(wait_limit(timeout_secs)).await
        {
            let reply = UserReply::from(msg.as_ref());
            let text = reply.text.clone().unwrap_or_default();
            context.set_pin_value("reply", json!(reply)).await?;
            context.set_pin_value("reply_text", json!(text)).await?;
            context.set_pin_value("timed_out", json!(false)).await?;
            context.activate_exec_pin("on_reply").await?;
        } else {
            context.set_pin_value("timed_out", json!(true)).await?;
            context.set_pin_value("reply_text", json!("")).await?;
            context.activate_exec_pin("on_timeout").await?;
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Send and Wait Node (Combined convenience node)
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendAndWaitNode;

impl SendAndWaitNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendAndWaitNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_and_wait",
            "Send and Wait",
            "Sends a message and waits for a reply. Perfect for dialogs and human-in-the-loop flows.",
            "Telegram/Interaction",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "prompt",
            "Prompt",
            "The message to send to the user",
            VariableType::String,
        );

        node.add_input_pin(
            "parse_mode",
            "Parse Mode",
            "Text formatting (Text, HTML, or Markdown)",
            VariableType::String,
        )
        .set_default_value(Some(json!("Text")));

        node.add_input_pin(
            "timeout_seconds",
            "Timeout (seconds)",
            "Maximum time to wait for reply (0 = no timeout)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(120)));

        node.add_output_pin(
            "on_reply",
            "On Reply",
            "Triggered when user replies",
            VariableType::Execution,
        );

        node.add_output_pin(
            "on_timeout",
            "On Timeout",
            "Triggered when timeout is reached",
            VariableType::Execution,
        );

        node.add_output_pin("reply", "Reply", "The user's reply", VariableType::Struct)
            .set_schema::<UserReply>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "reply_text",
            "Reply Text",
            "The text content of the reply",
            VariableType::String,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Reference to the sent prompt message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let prompt: String = context.evaluate_pin("prompt").await?;
        let parse_mode_str: String = context
            .evaluate_pin::<String>("parse_mode")
            .await
            .unwrap_or_else(|_| "Text".to_string());
        let timeout_secs: i64 = context
            .evaluate_pin::<i64>("timeout_seconds")
            .await
            .unwrap_or(120);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let prompt_id = Arc::new(OnceLock::new());
        let expected = prompt_id.clone();
        let wait = begin_wait(
            &bot.bot,
            AllowedUpdate::Message,
            Arc::new(move |event| {
                matches!(event, TelegramBroadcastEvent::Message(msg)
                    if msg.chat.id == chat_id
                        && msg.reply_to_message().is_some_and(|replied| expected.get() == Some(&replied.id.0)))
            }),
        );

        let mut request = bot.bot.send_message(chat_id, &prompt);

        match parse_mode_str.to_lowercase().as_str() {
            "html" => request = request.parse_mode(ParseMode::Html),
            "markdown" | "markdownv2" => request = request.parse_mode(ParseMode::MarkdownV2),
            _ => {}
        }

        let sent = request.await?;
        let _ = prompt_id.set(sent.id.0);
        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;

        if let Some(TelegramBroadcastEvent::Message(msg)) =
            wait.finish(wait_limit(timeout_secs)).await
        {
            let reply = UserReply::from(msg.as_ref());
            let text = reply.text.clone().unwrap_or_default();
            context.set_pin_value("reply", json!(reply)).await?;
            context.set_pin_value("reply_text", json!(text)).await?;
            context.activate_exec_pin("on_reply").await?;
        } else {
            context.set_pin_value("reply_text", json!("")).await?;
            context.activate_exec_pin("on_timeout").await?;
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Wait For Callback Node (for inline keyboards)
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct WaitForCallbackNode;

impl WaitForCallbackNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for WaitForCallbackNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_wait_for_callback",
            "Wait For Callback",
            "Waits for an inline keyboard button click. Great for confirmation dialogs.",
            "Telegram/Interaction",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "message_ref",
            "Message Reference",
            "The message with the inline keyboard (optional)",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>();

        node.add_input_pin(
            "expected_data",
            "Expected Data",
            "Only trigger on callbacks with this data (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "timeout_seconds",
            "Timeout (seconds)",
            "Maximum time to wait (0 = no timeout)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(60)));

        node.add_input_pin(
            "answer_callback",
            "Answer Callback",
            "Automatically answer the callback query",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "on_callback",
            "On Callback",
            "Triggered when callback is received",
            VariableType::Execution,
        );

        node.add_output_pin(
            "on_timeout",
            "On Timeout",
            "Triggered when timeout is reached",
            VariableType::Execution,
        );

        node.add_output_pin(
            "callback",
            "Callback",
            "The callback response data",
            VariableType::Struct,
        )
        .set_schema::<CallbackResponse>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "callback_data",
            "Callback Data",
            "The callback data string (convenience output)",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let timeout_secs: i64 = context
            .evaluate_pin::<i64>("timeout_seconds")
            .await
            .unwrap_or(60);
        let answer_callback: bool = context
            .evaluate_pin::<bool>("answer_callback")
            .await
            .unwrap_or(true);

        let message_ref: Option<SentMessage> = context.evaluate_pin("message_ref").await.ok();
        let expected_data: Option<String> = context.evaluate_pin("expected_data").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let message_id_filter = message_ref.and_then(|m| m.message_id.parse::<i32>().ok());

        let wait = begin_wait(
            &bot.bot,
            AllowedUpdate::CallbackQuery,
            Arc::new(move |event| {
                let TelegramBroadcastEvent::CallbackQuery(callback) = event else {
                    return false;
                };
                let message = callback.message.as_ref();
                message.is_none_or(|m| m.chat().id == chat_id)
                    && message_id_filter
                        .is_none_or(|expected| message.map_or(0, |m| m.id().0) == expected)
                    && expected_data
                        .as_deref()
                        .is_none_or(|expected| callback.data.as_deref() == Some(expected))
            }),
        );

        if let Some(TelegramBroadcastEvent::CallbackQuery(callback)) =
            wait.finish(wait_limit(timeout_secs)).await
        {
            if answer_callback {
                let _ = bot.bot.answer_callback_query(callback.id.clone()).await;
            }
            let response = CallbackResponse {
                callback_id: callback.id.to_string(),
                data: callback.data.clone().unwrap_or_default(),
                user_id: callback.from.id.0.to_string(),
                username: callback.from.username.clone(),
                chat_id: callback.message.as_ref().map(|m| m.chat().id.0.to_string()),
                message_id: callback.message.as_ref().map(|m| m.id().0.to_string()),
            };
            let data = response.data.clone();
            context.set_pin_value("callback", json!(response)).await?;
            context.set_pin_value("callback_data", json!(data)).await?;
            context.activate_exec_pin("on_callback").await?;
        } else {
            context.set_pin_value("callback_data", json!("")).await?;
            context.activate_exec_pin("on_timeout").await?;
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Answer Callback Query Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct AnswerCallbackQueryNode;

impl AnswerCallbackQueryNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AnswerCallbackQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_answer_callback",
            "Answer Callback Query",
            "Answers a callback query (acknowledges button click)",
            "Telegram/Interaction",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "callback",
            "Callback",
            "The callback response to answer",
            VariableType::Struct,
        )
        .set_schema::<CallbackResponse>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "text",
            "Text",
            "Text to display as notification or alert",
            VariableType::String,
        );

        node.add_input_pin(
            "show_alert",
            "Show Alert",
            "Show as modal alert instead of notification",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after answering",
            VariableType::Execution,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let callback: CallbackResponse = context.evaluate_pin("callback").await?;
        let text: Option<String> = context.evaluate_pin::<String>("text").await.ok();
        let show_alert: bool = context
            .evaluate_pin::<bool>("show_alert")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let mut request = bot
            .bot
            .answer_callback_query(teloxide::types::CallbackQueryId(
                callback.callback_id.clone(),
            ));

        if let Some(txt) = text {
            request = request.text(txt);
        }

        if show_alert {
            request = request.show_alert(true);
        }

        request.await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Confirmation Dialog Node (High-level convenience)
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ConfirmationDialogNode;

impl ConfirmationDialogNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ConfirmationDialogNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_confirmation_dialog",
            "Confirmation Dialog",
            "Sends a yes/no confirmation dialog and waits for user response",
            "Telegram/Interaction",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "question",
            "Question",
            "The confirmation question to ask",
            VariableType::String,
        );

        node.add_input_pin(
            "confirm_text",
            "Confirm Text",
            "Text for the confirm button",
            VariableType::String,
        )
        .set_default_value(Some(json!("✅ Yes")));

        node.add_input_pin(
            "cancel_text",
            "Cancel Text",
            "Text for the cancel button",
            VariableType::String,
        )
        .set_default_value(Some(json!("❌ No")));

        node.add_input_pin(
            "timeout_seconds",
            "Timeout (seconds)",
            "Maximum time to wait (0 = no timeout)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(60)));

        node.add_output_pin(
            "on_confirm",
            "On Confirm",
            "Triggered when user confirms",
            VariableType::Execution,
        );

        node.add_output_pin(
            "on_cancel",
            "On Cancel",
            "Triggered when user cancels",
            VariableType::Execution,
        );

        node.add_output_pin(
            "on_timeout",
            "On Timeout",
            "Triggered when timeout is reached",
            VariableType::Execution,
        );

        node.add_output_pin(
            "confirmed",
            "Confirmed",
            "Whether user confirmed",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

        let session: TelegramSession = context.evaluate_pin("session").await?;
        let question: String = context.evaluate_pin("question").await?;
        let confirm_text: String = context
            .evaluate_pin::<String>("confirm_text")
            .await
            .unwrap_or_else(|_| "✅ Yes".to_string());
        let cancel_text: String = context
            .evaluate_pin::<String>("cancel_text")
            .await
            .unwrap_or_else(|_| "❌ No".to_string());
        let timeout_secs: i64 = context
            .evaluate_pin::<i64>("timeout_seconds")
            .await
            .unwrap_or(60);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let dialog_id = Arc::new(OnceLock::new());
        let expected = dialog_id.clone();
        let wait = begin_wait(
            &bot.bot,
            AllowedUpdate::CallbackQuery,
            Arc::new(move |event| {
                matches!(event, TelegramBroadcastEvent::CallbackQuery(callback)
                    if callback.message.as_ref().is_some_and(|m| expected.get() == Some(&m.id().0))
                        && matches!(callback.data.as_deref(), Some("confirm" | "cancel")))
            }),
        );

        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback(confirm_text, "confirm"),
            InlineKeyboardButton::callback(cancel_text, "cancel"),
        ]]);

        let sent = bot
            .bot
            .send_message(chat_id, &question)
            .reply_markup(keyboard)
            .await?;

        let message_id = sent.id.0;
        let _ = dialog_id.set(message_id);

        let response = match wait.finish(wait_limit(timeout_secs)).await {
            Some(TelegramBroadcastEvent::CallbackQuery(callback)) => {
                let _ = bot.bot.answer_callback_query(callback.id.clone()).await;
                callback.data
            }
            _ => None,
        };

        let _ = bot
            .bot
            .edit_message_reply_markup(chat_id, teloxide::types::MessageId(message_id))
            .await;

        if let Some(data) = response {
            let confirmed = data == "confirm";
            context.set_pin_value("confirmed", json!(confirmed)).await?;

            if confirmed {
                context.activate_exec_pin("on_confirm").await?;
            } else {
                context.activate_exec_pin("on_cancel").await?;
            }
        } else {
            context.set_pin_value("confirmed", json!(false)).await?;
            context.activate_exec_pin("on_timeout").await?;
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::telegram::session::{
        CachedTelegramBot, broadcast_update_json, forget_bot_credential, register_bot_credential,
    };
    use flow_like_types::Value;

    fn closed_port_bot(token: &str) -> Bot {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener.local_addr().expect("local address").port();
        drop(listener);
        let url = flow_like_types::reqwest::Url::parse(&format!("http://127.0.0.1:{port}"))
            .expect("local url");
        Bot::new(token).set_api_url(url)
    }

    fn message_in(chat: i64) -> UpdateMatch {
        Arc::new(
            move |event| matches!(event, TelegramBroadcastEvent::Message(msg) if msg.chat.id.0 == chat),
        )
    }

    fn update(id: u32, chat: i64, text: &str) -> Value {
        json!({"update_id": id, "message": {"message_id": id, "date": 1_790_000_000,
            "chat": {"id": chat, "type": "private", "first_name": "Ada"},
            "from": {"id": chat, "is_bot": false, "first_name": "Ada"}, "text": text}})
    }

    fn alive_tasks() -> usize {
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
    }

    async fn ended(task: &AbortHandle) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !task.is_finished() {
            if tokio::time::Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        true
    }

    #[tokio::test]
    async fn wait_that_timed_out_ends_its_poll_task() {
        let bot = closed_port_bot("700100400:interaction-timeout-secret-abcdefghij");
        let wait = begin_wait(&bot, AllowedUpdate::Message, message_in(4242));
        let task = wait.poll_task().expect("a poll task").clone();
        assert!(!task.is_finished());

        let limit = Some(Duration::from_millis(250));
        assert!(wait.finish(limit).await.is_none());
        assert!(ended(&task).await, "the poll task outlived its wait");
    }

    #[tokio::test]
    async fn dropped_wait_ends_its_poll_task() {
        let bot = closed_port_bot("700100500:interaction-dropped-secret-abcdefghij");
        let wait = begin_wait(&bot, AllowedUpdate::CallbackQuery, message_in(4242));
        let task = wait.poll_task().expect("a poll task").clone();

        let cancelled = tokio::time::timeout(Duration::from_millis(100), wait.finish(None)).await;
        assert!(cancelled.is_err());
        assert!(
            ended(&task).await,
            "the poll task outlived a cancelled wait"
        );
    }

    #[tokio::test]
    async fn token_with_a_device_poller_waits_for_the_poller() {
        let handle = "device-bot:evt_interaction_poller";
        let token = "700100600:interaction-poller-secret-abcdefghij";
        register_bot_credential(handle, token);
        let bot = CachedTelegramBot::new(handle).bot;
        let before = alive_tasks();

        let wait = begin_wait(&bot, AllowedUpdate::Message, message_in(4242));
        assert!(wait.poll_task().is_none());
        assert_eq!(alive_tasks(), before, "a poll task was started");

        let other_bot = "700100601:interaction-other-secret-abcdefghij";
        assert!(!broadcast_update_json(token, &update(1, 777, "elsewhere")));
        assert!(!broadcast_update_json(
            other_bot,
            &update(2, 4242, "other bot")
        ));
        assert!(!broadcast_update_json(
            token,
            &json!({"update_id": 3, "poll": {}})
        ));
        assert!(broadcast_update_json(token, &update(4, 4242, "yes")));

        let taken = wait.finish(Some(Duration::from_secs(1))).await;
        let Some(TelegramBroadcastEvent::Message(reply)) = taken else {
            panic!("the poller's update did not reach the wait");
        };
        assert_eq!(reply.text(), Some("yes"));
        assert!(!broadcast_update_json(token, &update(5, 4242, "again")));
        forget_bot_credential(handle);
    }

    #[tokio::test]
    async fn dropped_device_wait_leaves_the_queue() {
        let handle = "device-bot:evt_interaction_dropped";
        let token = "700100700:interaction-queue-secret-abcdefghij";
        register_bot_credential(handle, token);
        let bot = CachedTelegramBot::new(handle).bot;

        drop(begin_wait(&bot, AllowedUpdate::Message, message_in(4242)));
        assert!(!broadcast_update_json(token, &update(1, 4242, "late")));
        forget_bot_credential(handle);
    }
}
