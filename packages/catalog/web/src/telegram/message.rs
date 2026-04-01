//! Telegram message operations

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
#[cfg(feature = "execute")]
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::{CustomEmojiId, FileId, ParseMode, ReplyParameters};

/// Result from sending a message
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SentMessage {
    pub message_id: String,
    pub chat_id: String,
    pub date: i64,
}

// ============================================================================
// Send Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendMessageNode;

impl SendMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_message",
            "Send Message",
            "Sends a text message to the Telegram chat",
            "Telegram/Message",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session from 'To Telegram Session' node",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "message",
            "Message",
            "The text message to send (supports Markdown)",
            VariableType::String,
        );

        node.add_input_pin(
            "reply_to",
            "Reply To",
            "Optional message ID to reply to",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["optional".into()])
                .build(),
        );

        node.add_input_pin(
            "disable_notification",
            "Silent",
            "Send without notification sound",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after message is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
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

        let message: String = context.evaluate_pin("message").await?;

        let reply_to: Option<String> = context.evaluate_pin::<String>("reply_to").await.ok();
        let silent: bool = context
            .evaluate_pin::<bool>("disable_notification")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let mut request = bot
            .bot
            .send_message(chat_id, &message)
            .parse_mode(ParseMode::MarkdownV2)
            .disable_notification(silent);

        if let Some(reply_id) = reply_to
            && let Ok(msg_id) = reply_id.parse::<i32>()
        {
            request =
                request.reply_parameters(ReplyParameters::new(teloxide::types::MessageId(msg_id)));
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

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Edit Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditMessageNode;

impl EditMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_edit_message",
            "Edit Message",
            "Edits an existing text message",
            "Telegram/Message",
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
            "message_id",
            "Message ID",
            "ID of the message to edit (defaults to triggering message)",
            VariableType::String,
        );

        node.add_input_pin(
            "new_text",
            "New Text",
            "New message content (supports Markdown)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after message is edited",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the edit was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let message_id: String = context
            .evaluate_pin::<String>("message_id")
            .await
            .unwrap_or(session.message_id.clone());

        let new_text: String = context.evaluate_pin("new_text").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;
        let msg_id: i32 = message_id.parse()?;

        let result = bot
            .bot
            .edit_message_text(chat_id, teloxide::types::MessageId(msg_id), &new_text)
            .parse_mode(ParseMode::MarkdownV2)
            .await;

        context
            .set_pin_value("success", json!(result.is_ok()))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Delete Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteMessageNode;

impl DeleteMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_delete_message",
            "Delete Message",
            "Deletes a message from the chat",
            "Telegram/Message",
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
            "message_id",
            "Message ID",
            "ID of the message to delete (defaults to triggering message)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after message is deleted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the deletion was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let message_id: String = context
            .evaluate_pin::<String>("message_id")
            .await
            .unwrap_or(session.message_id.clone());

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;
        let msg_id: i32 = message_id.parse()?;

        let result = bot
            .bot
            .delete_message(chat_id, teloxide::types::MessageId(msg_id))
            .await;

        context
            .set_pin_value("success", json!(result.is_ok()))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Forward Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ForwardMessageNode;

impl ForwardMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ForwardMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_forward_message",
            "Forward Message",
            "Forwards a message to another chat",
            "Telegram/Message",
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
            "message_id",
            "Message ID",
            "ID of the message to forward (defaults to triggering message)",
            VariableType::String,
        );

        node.add_input_pin(
            "target_chat_id",
            "Target Chat ID",
            "ID of the chat to forward to",
            VariableType::String,
        );

        node.add_input_pin(
            "disable_notification",
            "Silent",
            "Forward without notification",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after message is forwarded",
            VariableType::Execution,
        );

        node.add_output_pin(
            "forwarded_message",
            "Forwarded Message",
            "Information about the forwarded message",
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

        let message_id: String = context
            .evaluate_pin::<String>("message_id")
            .await
            .unwrap_or(session.message_id.clone());

        let target_chat_id: String = context.evaluate_pin("target_chat_id").await?;

        let silent: bool = context
            .evaluate_pin::<bool>("disable_notification")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let from_chat = session.chat_id()?;
        let to_chat = teloxide::types::ChatId(target_chat_id.parse()?);
        let msg_id: i32 = message_id.parse()?;

        let sent = bot
            .bot
            .forward_message(to_chat, from_chat, teloxide::types::MessageId(msg_id))
            .disable_notification(silent)
            .await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("forwarded_message", json!(sent_message))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Reply To Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ReplyToMessageNode;

impl ReplyToMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ReplyToMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_reply_to_message",
            "Reply To Message",
            "Sends a reply to the triggering message",
            "Telegram/Message",
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
            "message",
            "Message",
            "Reply text (supports Markdown)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after reply is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent reply",
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

        let message: String = context.evaluate_pin("message").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;
        let reply_to_id = session.message_id()?;

        let sent = bot
            .bot
            .send_message(chat_id, &message)
            .parse_mode(ParseMode::MarkdownV2)
            .reply_parameters(ReplyParameters::new(reply_to_id))
            .await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Copy Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CopyMessageNode;

impl CopyMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CopyMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_copy_message",
            "Copy Message",
            "Copies a message to another chat without 'Forwarded from' header",
            "Telegram/Message",
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
            "message_id",
            "Message ID",
            "ID of the message to copy (defaults to triggering message)",
            VariableType::String,
        );

        node.add_input_pin(
            "target_chat_id",
            "Target Chat ID",
            "ID of the chat to copy to",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after message is copied",
            VariableType::Execution,
        );

        node.add_output_pin(
            "copied_message_id",
            "Copied Message ID",
            "ID of the new copied message",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let message_id: String = context
            .evaluate_pin::<String>("message_id")
            .await
            .unwrap_or(session.message_id.clone());

        let target_chat_id: String = context.evaluate_pin("target_chat_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let from_chat = session.chat_id()?;
        let to_chat = teloxide::types::ChatId(target_chat_id.parse()?);
        let msg_id: i32 = message_id.parse()?;

        let copied = bot
            .bot
            .copy_message(to_chat, from_chat, teloxide::types::MessageId(msg_id))
            .await?;

        context
            .set_pin_value("copied_message_id", json!(copied.0.to_string()))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Edit Message Live Location Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditMessageLiveLocationNode;

impl EditMessageLiveLocationNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditMessageLiveLocationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_edit_message_live_location",
            "Edit Live Location",
            "Edits a live location message with new coordinates",
            "Telegram/Message",
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
            "message_id",
            "Message ID",
            "ID of the live location message to edit",
            VariableType::String,
        );

        node.add_input_pin(
            "latitude",
            "Latitude",
            "New latitude (-90 to 90)",
            VariableType::Float,
        );

        node.add_input_pin(
            "longitude",
            "Longitude",
            "New longitude (-180 to 180)",
            VariableType::Float,
        );

        node.add_input_pin(
            "horizontal_accuracy",
            "Horizontal Accuracy",
            "Radius of uncertainty in meters (0-1500)",
            VariableType::Float,
        );

        node.add_input_pin(
            "heading",
            "Heading",
            "Direction of movement in degrees (1-360)",
            VariableType::Integer,
        );

        node.add_input_pin(
            "proximity_alert_radius",
            "Proximity Alert Radius",
            "Maximum distance in meters for proximity alerts (0-100000)",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after location is edited",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the edit was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let message_id: String = context.evaluate_pin("message_id").await?;
        let latitude: f64 = context.evaluate_pin("latitude").await?;
        let longitude: f64 = context.evaluate_pin("longitude").await?;
        let horizontal_accuracy: Option<f64> =
            context.evaluate_pin("horizontal_accuracy").await.ok();
        let heading: Option<i64> = context.evaluate_pin("heading").await.ok();
        let proximity_alert_radius: Option<i64> =
            context.evaluate_pin("proximity_alert_radius").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;
        let msg_id: i32 = message_id.parse()?;

        let mut request = bot.bot.edit_message_live_location(
            chat_id,
            teloxide::types::MessageId(msg_id),
            latitude,
            longitude,
        );

        if let Some(acc) = horizontal_accuracy {
            request = request.horizontal_accuracy(acc);
        }
        if let Some(h) = heading {
            request = request.heading(h as u16);
        }
        if let Some(radius) = proximity_alert_radius {
            request = request.proximity_alert_radius(radius as u32);
        }

        let result = request.await;
        context
            .set_pin_value("success", json!(result.is_ok()))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Stop Message Live Location Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct StopMessageLiveLocationNode;

impl StopMessageLiveLocationNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for StopMessageLiveLocationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_stop_message_live_location",
            "Stop Live Location",
            "Stops updating a live location message",
            "Telegram/Message",
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
            "message_id",
            "Message ID",
            "ID of the live location message to stop",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after location is stopped",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the stop was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let message_id: String = context.evaluate_pin("message_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;
        let msg_id: i32 = message_id.parse()?;

        let result = bot
            .bot
            .stop_message_live_location(chat_id, teloxide::types::MessageId(msg_id))
            .await;

        context
            .set_pin_value("success", json!(result.is_ok()))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Send Media Group Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendMediaGroupNode;

impl SendMediaGroupNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendMediaGroupNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_media_group",
            "Send Media Group",
            "Sends a group of photos or videos as an album (2-10 items)",
            "Telegram/Message",
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
            "media_json",
            "Media JSON",
            "JSON array of InputMedia objects: [{\"type\":\"photo\",\"media\":\"url\"},{\"type\":\"video\",\"media\":\"url\",\"caption\":\"text\"}]",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after media group is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "message_ids",
            "Message IDs",
            "IDs of the sent messages (JSON array)",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use teloxide::types::{InputFile, InputMedia, InputMediaPhoto, InputMediaVideo};

        let session: TelegramSession = context.evaluate_pin("session").await?;
        let media_json: String = context.evaluate_pin("media_json").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let media_items: Vec<flow_like_types::Value> =
            flow_like_types::json::from_str(&media_json)?;

        let mut input_media: Vec<InputMedia> = Vec::new();
        for item in media_items {
            let media_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("photo");
            let media_url = item
                .get("media")
                .and_then(|m| m.as_str())
                .ok_or_else(|| flow_like_types::anyhow!("Missing 'media' field in InputMedia"))?;
            let caption = item
                .get("caption")
                .and_then(|c| c.as_str())
                .map(String::from);

            let url = media_url
                .parse()
                .map_err(|_| flow_like_types::anyhow!("Invalid media URL: {}", media_url))?;
            let input_file = InputFile::url(url);

            let im = match media_type {
                "video" => {
                    let mut video = InputMediaVideo::new(input_file);
                    if let Some(cap) = caption {
                        video = video.caption(cap);
                    }
                    InputMedia::Video(video)
                }
                _ => {
                    let mut photo = InputMediaPhoto::new(input_file);
                    if let Some(cap) = caption {
                        photo = photo.caption(cap);
                    }
                    InputMedia::Photo(photo)
                }
            };
            input_media.push(im);
        }

        if input_media.len() < 2 || input_media.len() > 10 {
            return Err(flow_like_types::anyhow!(
                "Media group must contain 2-10 items, got {}",
                input_media.len()
            ));
        }

        let messages = bot.bot.send_media_group(chat_id, input_media).await?;
        let message_ids: Vec<String> = messages.iter().map(|m| m.id.0.to_string()).collect();

        context
            .set_pin_value("message_ids", json!(message_ids))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Send Video Note Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendVideoNoteNode;

impl SendVideoNoteNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendVideoNoteNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_video_note",
            "Send Video Note",
            "Sends a video note (rounded square video message)",
            "Telegram/Message",
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
            "video_note",
            "Video Note",
            "File ID or URL of the video note",
            VariableType::String,
        );

        node.add_input_pin(
            "duration",
            "Duration",
            "Duration of the video in seconds",
            VariableType::Integer,
        );

        node.add_input_pin(
            "length",
            "Length",
            "Video width and height (square, max 640)",
            VariableType::Integer,
        );

        node.add_input_pin(
            "thumbnail",
            "Thumbnail",
            "Optional thumbnail URL",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after video note is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use teloxide::types::InputFile;

        let session: TelegramSession = context.evaluate_pin("session").await?;
        let video_note: String = context.evaluate_pin("video_note").await?;
        let duration: Option<i64> = context.evaluate_pin("duration").await.ok();
        let length: Option<i64> = context.evaluate_pin("length").await.ok();
        let thumbnail: Option<String> = context.evaluate_pin("thumbnail").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let video_file = if video_note.starts_with("http://") || video_note.starts_with("https://")
        {
            let url = video_note
                .parse()
                .map_err(|_| flow_like_types::anyhow!("Invalid video note URL"))?;
            InputFile::url(url)
        } else {
            InputFile::file_id(FileId(video_note))
        };

        let mut request = bot.bot.send_video_note(chat_id, video_file);

        if let Some(d) = duration {
            request = request.duration(d as u32);
        }
        if let Some(l) = length {
            request = request.length(l as u32);
        }
        if let Some(thumb_url) = thumbnail
            && let Ok(url) = thumb_url.parse()
        {
            request = request.thumbnail(InputFile::url(url));
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

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Set Message Reaction Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetMessageReactionNode;

impl SetMessageReactionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetMessageReactionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_message_reaction",
            "Set Message Reaction",
            "Sets reaction(s) on a message using emoji or custom emoji",
            "Telegram/Message",
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
            "message_id",
            "Message ID",
            "ID of the message to react to (defaults to triggering message)",
            VariableType::String,
        );

        node.add_input_pin(
            "reaction_json",
            "Reaction JSON",
            "JSON array of reactions: [{\"type\":\"emoji\",\"emoji\":\"👍\"}] or [{\"type\":\"custom_emoji\",\"custom_emoji_id\":\"id\"}]",
            VariableType::String,
        );

        node.add_input_pin(
            "is_big",
            "Is Big",
            "Whether to show big animation",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after reaction is set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the reaction was set successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use teloxide::types::ReactionType;

        let session: TelegramSession = context.evaluate_pin("session").await?;
        let message_id: String = context
            .evaluate_pin::<String>("message_id")
            .await
            .unwrap_or(session.message_id.clone());
        let reaction_json: String = context.evaluate_pin("reaction_json").await?;
        let is_big: bool = context.evaluate_pin("is_big").await.unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;
        let msg_id: i32 = message_id.parse()?;

        let reaction_items: Vec<flow_like_types::Value> =
            flow_like_types::json::from_str(&reaction_json)?;

        let mut reactions: Vec<ReactionType> = Vec::new();
        for item in reaction_items {
            let reaction_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("emoji");
            match reaction_type {
                "custom_emoji" => {
                    let custom_emoji_id = item
                        .get("custom_emoji_id")
                        .and_then(|c| c.as_str())
                        .ok_or_else(|| {
                            flow_like_types::anyhow!(
                                "Missing 'custom_emoji_id' for custom_emoji reaction"
                            )
                        })?;
                    reactions.push(ReactionType::CustomEmoji {
                        custom_emoji_id: CustomEmojiId(custom_emoji_id.to_string()),
                    });
                }
                _ => {
                    let emoji = item.get("emoji").and_then(|e| e.as_str()).ok_or_else(|| {
                        flow_like_types::anyhow!("Missing 'emoji' for emoji reaction")
                    })?;
                    reactions.push(ReactionType::Emoji {
                        emoji: emoji.to_string(),
                    });
                }
            }
        }

        let result = bot
            .bot
            .set_message_reaction(chat_id, teloxide::types::MessageId(msg_id))
            .reaction(reactions)
            .is_big(is_big)
            .await;

        context
            .set_pin_value("success", json!(result.is_ok()))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

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
// Edit Message Media Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditMessageMediaNode;

impl EditMessageMediaNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditMessageMediaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_edit_message_media",
            "Edit Message Media",
            "Edits the media content of a message (photo, video, animation, document)",
            "Telegram/Message",
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
            "message_id",
            "Message ID",
            "ID of the message to edit",
            VariableType::String,
        );

        node.add_input_pin(
            "media_json",
            "Media JSON",
            "InputMedia JSON: {\"type\":\"photo\"|\"video\"|\"animation\"|\"document\",\"media\":\"url\",\"caption\":\"optional\"}",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after media is edited",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the edit was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use teloxide::types::{
            InputFile, InputMedia, InputMediaAnimation, InputMediaDocument, InputMediaPhoto,
            InputMediaVideo,
        };

        let session: TelegramSession = context.evaluate_pin("session").await?;
        let message_id: String = context.evaluate_pin("message_id").await?;
        let media_json: String = context.evaluate_pin("media_json").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;
        let msg_id: i32 = message_id.parse()?;

        let item: flow_like_types::Value = flow_like_types::json::from_str(&media_json)?;
        let media_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("photo");
        let media_url = item
            .get("media")
            .and_then(|m| m.as_str())
            .ok_or_else(|| flow_like_types::anyhow!("Missing 'media' field in InputMedia"))?;
        let caption = item
            .get("caption")
            .and_then(|c| c.as_str())
            .map(String::from);

        let url = media_url
            .parse()
            .map_err(|_| flow_like_types::anyhow!("Invalid media URL: {}", media_url))?;
        let input_file = InputFile::url(url);

        let input_media = match media_type {
            "video" => {
                let mut video = InputMediaVideo::new(input_file);
                if let Some(cap) = caption {
                    video = video.caption(cap);
                }
                InputMedia::Video(video)
            }
            "animation" => {
                let mut animation = InputMediaAnimation::new(input_file);
                if let Some(cap) = caption {
                    animation = animation.caption(cap);
                }
                InputMedia::Animation(animation)
            }
            "document" => {
                let mut document = InputMediaDocument::new(input_file);
                if let Some(cap) = caption {
                    document = document.caption(cap);
                }
                InputMedia::Document(document)
            }
            _ => {
                let mut photo = InputMediaPhoto::new(input_file);
                if let Some(cap) = caption {
                    photo = photo.caption(cap);
                }
                InputMedia::Photo(photo)
            }
        };

        let result = bot
            .bot
            .edit_message_media(chat_id, teloxide::types::MessageId(msg_id), input_media)
            .await;

        context
            .set_pin_value("success", json!(result.is_ok()))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}
