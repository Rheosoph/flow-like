//! Telegram Business API operations

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
use teloxide::types::BusinessConnectionId;

/// Business connection information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BusinessConnection {
    pub id: String,
    pub user_id: i64,
    pub user_chat_id: i64,
    pub date: i64,
    pub is_enabled: bool,
}

/// Star balance information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StarBalance {
    pub amount: i64,
}

// ============================================================================
// Get Business Connection Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetBusinessConnectionNode;

impl GetBusinessConnectionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetBusinessConnectionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_business_connection",
            "Get Business Connection",
            "Gets information about a business connection",
            "Telegram/Business",
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
            "business_connection_id",
            "Business Connection ID",
            "Unique identifier of the business connection",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after connection info is retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "connection",
            "Connection",
            "Business connection information",
            VariableType::Struct,
        )
        .set_schema::<BusinessConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let business_connection_id: String = context.evaluate_pin("business_connection_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let connection = bot
            .bot
            .get_business_connection(BusinessConnectionId(business_connection_id))
            .await?;

        let connection_info = BusinessConnection {
            id: connection.id.0,
            user_id: connection.user.id.0 as i64,
            user_chat_id: connection.user_chat_id.0 as i64,
            date: connection.date.timestamp(),
            is_enabled: connection.is_enabled,
        };

        context
            .set_pin_value("connection", json!(connection_info))
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
// Send Business Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendBusinessMessageNode;

impl SendBusinessMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendBusinessMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_business_message",
            "Send Business Message",
            "Sends a message on behalf of a business account",
            "Telegram/Business",
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
            "business_connection_id",
            "Business Connection ID",
            "Unique identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "Unique identifier of the target chat",
            VariableType::Integer,
        );

        node.add_input_pin(
            "text",
            "Text",
            "Text of the message to send",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after message is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the sent message",
            VariableType::Integer,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the message was sent successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let business_connection_id: String = context.evaluate_pin("business_connection_id").await?;
        let chat_id: i64 = context.evaluate_pin("chat_id").await?;
        let text: String = context.evaluate_pin("text").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot
            .bot
            .send_message(teloxide::types::ChatId(chat_id), text)
            .business_connection_id(BusinessConnectionId(business_connection_id))
            .await;

        match result {
            Ok(msg) => {
                context
                    .set_pin_value("message_id", json!(msg.id.0 as i64))
                    .await?;
                context.set_pin_value("success", json!(true)).await?;
            }
            Err(_) => {
                context.set_pin_value("message_id", json!(0)).await?;
                context.set_pin_value("success", json!(false)).await?;
            }
        }

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
// Forward Business Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ForwardBusinessMessageNode;

impl ForwardBusinessMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ForwardBusinessMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_forward_business_message",
            "Forward Business Message",
            "Forwards a message on behalf of a business account",
            "Telegram/Business",
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
            "business_connection_id",
            "Business Connection ID",
            "Unique identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "Unique identifier of the target chat",
            VariableType::Integer,
        );

        node.add_input_pin(
            "from_chat_id",
            "From Chat ID",
            "Unique identifier of the chat where the original message was sent",
            VariableType::Integer,
        );

        node.add_input_pin(
            "message_id",
            "Message ID",
            "Message identifier in the chat specified in from_chat_id",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after message is forwarded",
            VariableType::Execution,
        );

        node.add_output_pin(
            "new_message_id",
            "New Message ID",
            "ID of the forwarded message",
            VariableType::Integer,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the message was forwarded successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let _business_connection_id: String =
            context.evaluate_pin("business_connection_id").await?;
        let chat_id: i64 = context.evaluate_pin("chat_id").await?;
        let from_chat_id: i64 = context.evaluate_pin("from_chat_id").await?;
        let message_id: i64 = context.evaluate_pin("message_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        // Note: forward_message doesn't support business_connection_id in this teloxide version
        let result = bot
            .bot
            .forward_message(
                teloxide::types::ChatId(chat_id),
                teloxide::types::ChatId(from_chat_id),
                teloxide::types::MessageId(message_id as i32),
            )
            .await;

        match result {
            Ok(msg) => {
                context
                    .set_pin_value("new_message_id", json!(msg.id.0 as i64))
                    .await?;
                context.set_pin_value("success", json!(true)).await?;
            }
            Err(_) => {
                context.set_pin_value("new_message_id", json!(0)).await?;
                context.set_pin_value("success", json!(false)).await?;
            }
        }

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
// Edit Business Message Text Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditBusinessMessageTextNode;

impl EditBusinessMessageTextNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditBusinessMessageTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_edit_business_message_text",
            "Edit Business Message Text",
            "Edits text of a message sent on behalf of a business account",
            "Telegram/Business",
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
            "business_connection_id",
            "Business Connection ID",
            "Unique identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "Unique identifier of the chat",
            VariableType::Integer,
        );

        node.add_input_pin(
            "message_id",
            "Message ID",
            "Identifier of the message to edit",
            VariableType::Integer,
        );

        node.add_input_pin(
            "text",
            "Text",
            "New text of the message",
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
            "Whether the message was edited successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let business_connection_id: String = context.evaluate_pin("business_connection_id").await?;
        let chat_id: i64 = context.evaluate_pin("chat_id").await?;
        let message_id: i64 = context.evaluate_pin("message_id").await?;
        let text: String = context.evaluate_pin("text").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot
            .bot
            .edit_message_text(
                teloxide::types::ChatId(chat_id),
                teloxide::types::MessageId(message_id as i32),
                text,
            )
            .business_connection_id(BusinessConnectionId(business_connection_id))
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
// Stop Business Poll Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct StopBusinessPollNode;

impl StopBusinessPollNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for StopBusinessPollNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_stop_business_poll",
            "Stop Business Poll",
            "Stops a poll sent on behalf of a business account",
            "Telegram/Business",
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
            "business_connection_id",
            "Business Connection ID",
            "Unique identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "Unique identifier of the chat",
            VariableType::Integer,
        );

        node.add_input_pin(
            "message_id",
            "Message ID",
            "Identifier of the poll message",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after poll is stopped",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the poll was stopped successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let business_connection_id: String = context.evaluate_pin("business_connection_id").await?;
        let chat_id: i64 = context.evaluate_pin("chat_id").await?;
        let message_id: i64 = context.evaluate_pin("message_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot
            .bot
            .stop_poll(
                teloxide::types::ChatId(chat_id),
                teloxide::types::MessageId(message_id as i32),
            )
            .business_connection_id(BusinessConnectionId(business_connection_id))
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
// Delete Business Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteBusinessMessageNode;

impl DeleteBusinessMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteBusinessMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_delete_business_message",
            "Delete Business Message",
            "Deletes a message sent on behalf of a business account",
            "Telegram/Business",
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
            "chat_id",
            "Chat ID",
            "Unique identifier of the chat",
            VariableType::Integer,
        );

        node.add_input_pin(
            "message_id",
            "Message ID",
            "Identifier of the message to delete",
            VariableType::Integer,
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
            "Whether the message was deleted successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let chat_id: i64 = context.evaluate_pin("chat_id").await?;
        let message_id: i64 = context.evaluate_pin("message_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot
            .bot
            .delete_message(
                teloxide::types::ChatId(chat_id),
                teloxide::types::MessageId(message_id as i32),
            )
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
// Send Business Photo Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendBusinessPhotoNode;

impl SendBusinessPhotoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendBusinessPhotoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_business_photo",
            "Send Business Photo",
            "Sends a photo on behalf of a business account",
            "Telegram/Business",
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
            "business_connection_id",
            "Business Connection ID",
            "Unique identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "Unique identifier of the target chat",
            VariableType::Integer,
        );

        node.add_input_pin(
            "photo",
            "Photo",
            "Photo to send (file_id, URL, or file path)",
            VariableType::String,
        );

        node.add_input_pin(
            "caption",
            "Caption",
            "Photo caption (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after photo is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the sent message",
            VariableType::Integer,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the photo was sent successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let business_connection_id: String = context.evaluate_pin("business_connection_id").await?;
        let chat_id: i64 = context.evaluate_pin("chat_id").await?;
        let photo: String = context.evaluate_pin("photo").await?;
        let caption: String = context
            .evaluate_pin::<String>("caption")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let input_file = teloxide::types::InputFile::url(photo.parse()?);
        let mut request = bot
            .bot
            .send_photo(teloxide::types::ChatId(chat_id), input_file)
            .business_connection_id(BusinessConnectionId(business_connection_id));

        if !caption.is_empty() {
            request = request.caption(caption);
        }

        let result = request.await;

        match result {
            Ok(msg) => {
                context
                    .set_pin_value("message_id", json!(msg.id.0 as i64))
                    .await?;
                context.set_pin_value("success", json!(true)).await?;
            }
            Err(_) => {
                context.set_pin_value("message_id", json!(0)).await?;
                context.set_pin_value("success", json!(false)).await?;
            }
        }

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
// Send Business Document Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendBusinessDocumentNode;

impl SendBusinessDocumentNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendBusinessDocumentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_business_document",
            "Send Business Document",
            "Sends a document on behalf of a business account",
            "Telegram/Business",
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
            "business_connection_id",
            "Business Connection ID",
            "Unique identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "Unique identifier of the target chat",
            VariableType::Integer,
        );

        node.add_input_pin(
            "document",
            "Document",
            "Document to send (file_id, URL, or file path)",
            VariableType::String,
        );

        node.add_input_pin(
            "caption",
            "Caption",
            "Document caption (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after document is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the sent message",
            VariableType::Integer,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the document was sent successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let business_connection_id: String = context.evaluate_pin("business_connection_id").await?;
        let chat_id: i64 = context.evaluate_pin("chat_id").await?;
        let document: String = context.evaluate_pin("document").await?;
        let caption: String = context
            .evaluate_pin::<String>("caption")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let input_file = teloxide::types::InputFile::url(document.parse()?);
        let mut request = bot
            .bot
            .send_document(teloxide::types::ChatId(chat_id), input_file)
            .business_connection_id(BusinessConnectionId(business_connection_id));

        if !caption.is_empty() {
            request = request.caption(caption);
        }

        let result = request.await;

        match result {
            Ok(msg) => {
                context
                    .set_pin_value("message_id", json!(msg.id.0 as i64))
                    .await?;
                context.set_pin_value("success", json!(true)).await?;
            }
            Err(_) => {
                context.set_pin_value("message_id", json!(0)).await?;
                context.set_pin_value("success", json!(false)).await?;
            }
        }

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
