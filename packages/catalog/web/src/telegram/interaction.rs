//! Telegram user interaction - wait for replies, callbacks, and reactions

use super::message::SentMessage;
use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[cfg(feature = "execute")]
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::{Message as TgMessage, ParseMode};

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

        let result: Arc<tokio::sync::Mutex<Option<UserReply>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let result_clone = result.clone();

        let poll_task = tokio::spawn(async move {
            let mut offset: i32 = 0;

            loop {
                let updates = bot
                    .bot
                    .get_updates()
                    .offset(offset)
                    .timeout(5_u32)
                    .allowed_updates(vec![teloxide::types::AllowedUpdate::Message])
                    .await;

                if let Ok(updates) = updates {
                    for update in updates {
                        offset = (update.id.0 as i32) + 1;

                        if let teloxide::types::UpdateKind::Message(msg) = update.kind {
                            if msg.chat.id.0 != chat_id.0 {
                                continue;
                            }

                            if let Some(required_reply_to) = reply_to_id {
                                if let Some(reply_to_msg) = msg.reply_to_message() {
                                    if reply_to_msg.id.0 != required_reply_to {
                                        continue;
                                    }
                                } else {
                                    continue;
                                }
                            }

                            if let Some(expected_user) = filter_user_id {
                                if let Some(from) = msg.from.as_ref() {
                                    if from.id.0 != expected_user {
                                        continue;
                                    }
                                } else {
                                    continue;
                                }
                            }

                            let reply = UserReply::from(&msg);
                            let mut result_guard = result_clone.lock().await;
                            *result_guard = Some(reply);
                            return;
                        }
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        });

        let got_reply = if timeout_secs > 0 {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs as u64),
                poll_task,
            )
            .await
            .is_ok()
        } else {
            let _ = poll_task.await;
            true
        };

        let reply_opt = result.lock().await.take();

        if got_reply {
            if let Some(reply) = reply_opt {
                let text = reply.text.clone().unwrap_or_default();
                context.set_pin_value("reply", json!(reply)).await?;
                context.set_pin_value("reply_text", json!(text)).await?;
                context.set_pin_value("timed_out", json!(false)).await?;
                context.activate_exec_pin("on_reply").await?;
            } else {
                context.set_pin_value("timed_out", json!(true)).await?;
                context.activate_exec_pin("on_timeout").await?;
            }
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

        let mut request = bot.bot.send_message(chat_id, &prompt);

        match parse_mode_str.to_lowercase().as_str() {
            "html" => request = request.parse_mode(ParseMode::Html),
            "markdown" | "markdownv2" => request = request.parse_mode(ParseMode::MarkdownV2),
            _ => {}
        }

        let sent = request.await?;
        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;

        let message_id = sent.id.0;
        let result: Arc<tokio::sync::Mutex<Option<UserReply>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let result_clone = result.clone();

        let poll_task = tokio::spawn(async move {
            let mut offset: i32 = 0;

            loop {
                let updates = bot
                    .bot
                    .get_updates()
                    .offset(offset)
                    .timeout(5_u32)
                    .allowed_updates(vec![teloxide::types::AllowedUpdate::Message])
                    .await;

                if let Ok(updates) = updates {
                    for update in updates {
                        offset = (update.id.0 as i32) + 1;

                        if let teloxide::types::UpdateKind::Message(msg) = update.kind {
                            if msg.chat.id.0 != chat_id.0 {
                                continue;
                            }

                            if let Some(reply_to) = msg.reply_to_message()
                                && reply_to.id.0 == message_id
                            {
                                let reply = UserReply::from(&msg);
                                let mut result_guard = result_clone.lock().await;
                                *result_guard = Some(reply);
                                return;
                            }
                        }
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        });

        let got_reply = if timeout_secs > 0 {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs as u64),
                poll_task,
            )
            .await
            .is_ok()
        } else {
            let _ = poll_task.await;
            true
        };

        let reply_opt = result.lock().await.take();

        if got_reply {
            if let Some(reply) = reply_opt {
                let text = reply.text.clone().unwrap_or_default();
                context.set_pin_value("reply", json!(reply)).await?;
                context.set_pin_value("reply_text", json!(text)).await?;
                context.activate_exec_pin("on_reply").await?;
            } else {
                context.set_pin_value("reply_text", json!("")).await?;
                context.activate_exec_pin("on_timeout").await?;
            }
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

        let result: Arc<tokio::sync::Mutex<Option<CallbackResponse>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let result_clone = result.clone();
        let bot_clone = bot.bot.clone();

        let poll_task = tokio::spawn(async move {
            let mut offset: i32 = 0;

            loop {
                let updates = bot_clone
                    .get_updates()
                    .offset(offset)
                    .timeout(5_u32)
                    .allowed_updates(vec![teloxide::types::AllowedUpdate::CallbackQuery])
                    .await;

                if let Ok(updates) = updates {
                    for update in updates {
                        offset = (update.id.0 as i32) + 1;

                        if let teloxide::types::UpdateKind::CallbackQuery(callback) = update.kind {
                            let msg_chat_id = callback.message.as_ref().map(|m| m.chat().id.0);

                            if let Some(cid) = msg_chat_id
                                && cid != chat_id.0
                            {
                                continue;
                            }

                            if let Some(expected_msg_id) = message_id_filter {
                                let actual_msg_id =
                                    callback.message.as_ref().map(|m| m.id().0).unwrap_or(0);
                                if actual_msg_id != expected_msg_id {
                                    continue;
                                }
                            }

                            if let Some(ref expected) = expected_data {
                                if let Some(ref data) = callback.data {
                                    if data != expected {
                                        continue;
                                    }
                                } else {
                                    continue;
                                }
                            }

                            let response = CallbackResponse {
                                callback_id: callback.id.to_string(),
                                data: callback.data.clone().unwrap_or_default(),
                                user_id: callback.from.id.0.to_string(),
                                username: callback.from.username.clone(),
                                chat_id: msg_chat_id.map(|id| id.to_string()),
                                message_id: callback.message.as_ref().map(|m| m.id().0.to_string()),
                            };

                            if answer_callback {
                                let _ = bot_clone.answer_callback_query(callback.id.clone()).await;
                            }

                            let mut result_guard = result_clone.lock().await;
                            *result_guard = Some(response);
                            return;
                        }
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        });

        let got_callback = if timeout_secs > 0 {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs as u64),
                poll_task,
            )
            .await
            .is_ok()
        } else {
            let _ = poll_task.await;
            true
        };

        let callback_opt = result.lock().await.take();

        if got_callback {
            if let Some(callback) = callback_opt {
                let data = callback.data.clone();
                context.set_pin_value("callback", json!(callback)).await?;
                context.set_pin_value("callback_data", json!(data)).await?;
                context.activate_exec_pin("on_callback").await?;
            } else {
                context.set_pin_value("callback_data", json!("")).await?;
                context.activate_exec_pin("on_timeout").await?;
            }
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

        let result: Arc<tokio::sync::Mutex<Option<String>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let result_clone = result.clone();
        let bot_clone = bot.bot.clone();

        let poll_task = tokio::spawn(async move {
            let mut offset: i32 = 0;

            loop {
                let updates = bot_clone
                    .get_updates()
                    .offset(offset)
                    .timeout(5_u32)
                    .allowed_updates(vec![teloxide::types::AllowedUpdate::CallbackQuery])
                    .await;

                if let Ok(updates) = updates {
                    for update in updates {
                        offset = (update.id.0 as i32) + 1;

                        if let teloxide::types::UpdateKind::CallbackQuery(callback) = update.kind {
                            let msg_id = callback.message.as_ref().map(|m| m.id().0).unwrap_or(0);

                            if msg_id != message_id {
                                continue;
                            }

                            if let Some(data) = callback.data
                                && (data == "confirm" || data == "cancel")
                            {
                                let _ = bot_clone.answer_callback_query(callback.id.clone()).await;

                                let mut result_guard = result_clone.lock().await;
                                *result_guard = Some(data);
                                return;
                            }
                        }
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        });

        let got_response = if timeout_secs > 0 {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs as u64),
                poll_task,
            )
            .await
            .is_ok()
        } else {
            let _ = poll_task.await;
            true
        };

        let response = result.lock().await.take();

        let _ = bot
            .bot
            .edit_message_reply_markup(chat_id, teloxide::types::MessageId(message_id))
            .await;

        if got_response {
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
