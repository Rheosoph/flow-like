//! Telegram forum topic management operations

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
use teloxide::types::{MessageId, Rgb, ThreadId};

/// Forum topic information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ForumTopicInfo {
    pub message_thread_id: i64,
    pub name: String,
    pub icon_color: i64,
    pub icon_custom_emoji_id: Option<String>,
}

// ============================================================================
// Create Forum Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CreateForumTopicNode;

impl CreateForumTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CreateForumTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_create_forum_topic",
            "Create Forum Topic",
            "Creates a topic in a forum supergroup chat",
            "Telegram/Forum",
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
            "name",
            "Name",
            "Topic name (1-128 characters)",
            VariableType::String,
        );

        node.add_input_pin(
            "icon_color",
            "Icon Color",
            "Color of the topic icon in RGB format. Must be one of: 7322096 (blue), 16766590 (yellow), 13338331 (purple), 9367192 (green), 16749490 (pink), 16478047 (red)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(7322096))); // Default to blue (0x6FB9F0)

        node.add_input_pin(
            "icon_custom_emoji_id",
            "Icon Custom Emoji ID",
            "Custom emoji identifier for the topic icon (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after topic is created",
            VariableType::Execution,
        );

        node.add_output_pin(
            "topic",
            "Topic Info",
            "Information about the created forum topic",
            VariableType::Struct,
        )
        .set_schema::<ForumTopicInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "message_thread_id",
            "Message Thread ID",
            "Unique identifier of the created forum topic",
            VariableType::Integer,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let name: String = context.evaluate_pin("name").await?;
        let icon_color: i64 = context
            .evaluate_pin::<i64>("icon_color")
            .await
            .unwrap_or(7322096); // Default to blue (0x6FB9F0)
        let icon_custom_emoji_id: String = context
            .evaluate_pin::<String>("icon_custom_emoji_id")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        // teloxide 0.17 uses builder pattern for create_forum_topic
        let rgb_color = Rgb::from_u32(icon_color as u32);
        let mut request = bot.bot.create_forum_topic(chat_id, &name);
        request = request.icon_color(rgb_color);
        if !icon_custom_emoji_id.is_empty() {
            request =
                request.icon_custom_emoji_id(teloxide::types::CustomEmojiId(icon_custom_emoji_id));
        }

        let topic = request.await?;

        let topic_info = ForumTopicInfo {
            message_thread_id: topic.thread_id.0.0 as i64,
            name: topic.name,
            icon_color: topic.icon_color.to_u32() as i64,
            icon_custom_emoji_id: topic.icon_custom_emoji_id.map(|id| id.to_string()),
        };

        context.set_pin_value("topic", json!(topic_info)).await?;
        context
            .set_pin_value("message_thread_id", json!(topic_info.message_thread_id))
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
// Edit Forum Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditForumTopicNode;

impl EditForumTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditForumTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_edit_forum_topic",
            "Edit Forum Topic",
            "Edits name and icon of a topic in a forum supergroup chat",
            "Telegram/Forum",
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
            "message_thread_id",
            "Message Thread ID",
            "Unique identifier of the target forum topic",
            VariableType::Integer,
        );

        node.add_input_pin(
            "name",
            "Name",
            "New topic name (1-128 characters, optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "icon_custom_emoji_id",
            "Icon Custom Emoji ID",
            "Custom emoji identifier for the topic icon (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after topic is edited",
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
        let message_thread_id: i64 = context.evaluate_pin("message_thread_id").await?;
        let name: String = context
            .evaluate_pin::<String>("name")
            .await
            .unwrap_or_default();
        let icon_custom_emoji_id: String = context
            .evaluate_pin::<String>("icon_custom_emoji_id")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let mut request = bot
            .bot
            .edit_forum_topic(chat_id, ThreadId(MessageId(message_thread_id as i32)));

        if !name.is_empty() {
            request = request.name(&name);
        }

        if !icon_custom_emoji_id.is_empty() {
            request =
                request.icon_custom_emoji_id(teloxide::types::CustomEmojiId(icon_custom_emoji_id));
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
// Close Forum Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CloseForumTopicNode;

impl CloseForumTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CloseForumTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_close_forum_topic",
            "Close Forum Topic",
            "Closes an open topic in a forum supergroup chat",
            "Telegram/Forum",
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
            "message_thread_id",
            "Message Thread ID",
            "Unique identifier of the target forum topic",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after topic is closed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the close was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let message_thread_id: i64 = context.evaluate_pin("message_thread_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .close_forum_topic(chat_id, ThreadId(MessageId(message_thread_id as i32)))
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
// Reopen Forum Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ReopenForumTopicNode;

impl ReopenForumTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ReopenForumTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_reopen_forum_topic",
            "Reopen Forum Topic",
            "Reopens a closed topic in a forum supergroup chat",
            "Telegram/Forum",
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
            "message_thread_id",
            "Message Thread ID",
            "Unique identifier of the target forum topic",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after topic is reopened",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the reopen was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let message_thread_id: i64 = context.evaluate_pin("message_thread_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .reopen_forum_topic(chat_id, ThreadId(MessageId(message_thread_id as i32)))
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
// Delete Forum Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteForumTopicNode;

impl DeleteForumTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteForumTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_delete_forum_topic",
            "Delete Forum Topic",
            "Deletes a forum topic along with all its messages in a forum supergroup chat",
            "Telegram/Forum",
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
            "message_thread_id",
            "Message Thread ID",
            "Unique identifier of the target forum topic",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after topic is deleted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the delete was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let message_thread_id: i64 = context.evaluate_pin("message_thread_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .delete_forum_topic(chat_id, ThreadId(MessageId(message_thread_id as i32)))
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
// Unpin All Forum Topic Messages Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct UnpinAllForumTopicMessagesNode;

impl UnpinAllForumTopicMessagesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UnpinAllForumTopicMessagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_unpin_all_forum_topic_messages",
            "Unpin All Forum Topic Messages",
            "Clears the list of pinned messages in a forum topic",
            "Telegram/Forum",
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
            "message_thread_id",
            "Message Thread ID",
            "Unique identifier of the target forum topic",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after messages are unpinned",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the unpin was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let message_thread_id: i64 = context.evaluate_pin("message_thread_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .unpin_all_forum_topic_messages(chat_id, ThreadId(MessageId(message_thread_id as i32)))
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
// Edit General Forum Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditGeneralForumTopicNode;

impl EditGeneralForumTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditGeneralForumTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_edit_general_forum_topic",
            "Edit General Forum Topic",
            "Edits the name of the 'General' topic in a forum supergroup chat",
            "Telegram/Forum",
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
            "name",
            "Name",
            "New name for the General topic (1-128 characters)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after General topic is edited",
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
        let name: String = context.evaluate_pin("name").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot.bot.edit_general_forum_topic(chat_id, &name).await;

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
// Close General Forum Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CloseGeneralForumTopicNode;

impl CloseGeneralForumTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CloseGeneralForumTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_close_general_forum_topic",
            "Close General Forum Topic",
            "Closes the 'General' topic in a forum supergroup chat",
            "Telegram/Forum",
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

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after General topic is closed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the close was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot.bot.close_general_forum_topic(chat_id).await;

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
// Reopen General Forum Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ReopenGeneralForumTopicNode;

impl ReopenGeneralForumTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ReopenGeneralForumTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_reopen_general_forum_topic",
            "Reopen General Forum Topic",
            "Reopens a closed 'General' topic in a forum supergroup chat",
            "Telegram/Forum",
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

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after General topic is reopened",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the reopen was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot.bot.reopen_general_forum_topic(chat_id).await;

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
// Hide General Forum Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct HideGeneralForumTopicNode;

impl HideGeneralForumTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for HideGeneralForumTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_hide_general_forum_topic",
            "Hide General Forum Topic",
            "Hides the 'General' topic in a forum supergroup chat",
            "Telegram/Forum",
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

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after General topic is hidden",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the hide was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot.bot.hide_general_forum_topic(chat_id).await;

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
// Unhide General Forum Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct UnhideGeneralForumTopicNode;

impl UnhideGeneralForumTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UnhideGeneralForumTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_unhide_general_forum_topic",
            "Unhide General Forum Topic",
            "Unhides the 'General' topic in a forum supergroup chat",
            "Telegram/Forum",
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

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after General topic is unhidden",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the unhide was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot.bot.unhide_general_forum_topic(chat_id).await;

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
// Get Forum Topic Icon Stickers Node
// ============================================================================

/// Sticker information for forum topic icons
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StickerInfo {
    pub file_id: String,
    pub file_unique_id: String,
    pub emoji: Option<String>,
    pub set_name: Option<String>,
}

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetForumTopicIconStickersNode;

impl GetForumTopicIconStickersNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetForumTopicIconStickersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_forum_topic_icon_stickers",
            "Get Forum Topic Icon Stickers",
            "Gets custom emoji stickers that can be used as forum topic icons",
            "Telegram/Forum",
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

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after stickers are retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "stickers",
            "Stickers",
            "List of available stickers for forum topic icons",
            VariableType::Struct,
        )
        .set_schema::<Vec<StickerInfo>>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of available stickers",
            VariableType::Integer,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let stickers = bot.bot.get_forum_topic_icon_stickers().await?;

        let sticker_infos: Vec<StickerInfo> = stickers
            .iter()
            .map(|s| StickerInfo {
                file_id: s.file.id.to_string(),
                file_unique_id: s.file.unique_id.to_string(),
                emoji: s.emoji.clone(),
                set_name: s.set_name.clone(),
            })
            .collect();

        let count = sticker_infos.len() as i64;

        context
            .set_pin_value("stickers", json!(sticker_infos))
            .await?;
        context.set_pin_value("count", json!(count)).await?;

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
// Unpin All General Forum Topic Messages Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct UnpinAllGeneralForumTopicMessagesNode;

impl UnpinAllGeneralForumTopicMessagesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UnpinAllGeneralForumTopicMessagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_unpin_all_general_forum_topic_messages",
            "Unpin All General Forum Topic Messages",
            "Clears the list of pinned messages in a General forum topic. The bot must be an administrator and have the can_pin_messages admin right.",
            "Telegram/Forum",
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

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after messages are unpinned",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .unpin_all_general_forum_topic_messages(chat_id)
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
