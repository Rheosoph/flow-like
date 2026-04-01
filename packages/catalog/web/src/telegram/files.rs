//! Telegram file operations

use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::{FileId, MessageId, UserId};

/// File information from Telegram
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileInfo {
    pub file_id: String,
    pub file_unique_id: String,
    pub file_size: Option<i64>,
    pub file_path: Option<String>,
}

/// Photo information from Telegram
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PhotoInfo {
    pub file_id: String,
    pub file_unique_id: String,
    pub width: i64,
    pub height: i64,
    pub file_size: Option<i64>,
}

/// User profile photos result
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserProfilePhotosResult {
    pub total_count: i64,
    pub photos: Vec<Vec<PhotoInfo>>,
}

// ============================================================================
// Get File Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetFileNode;

impl GetFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_file",
            "Get File",
            "Gets basic information about a file and prepares it for downloading",
            "Telegram/Files",
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
            "file_id",
            "File ID",
            "File identifier to get information about",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after file info is retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "file_info",
            "File Info",
            "Information about the file",
            VariableType::Struct,
        )
        .set_schema::<FileInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "file_path",
            "File Path",
            "File path for downloading (use with bot token to construct URL)",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let file_id: String = context.evaluate_pin("file_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let file = bot.bot.get_file(FileId(file_id)).await?;

        let file_info = FileInfo {
            file_id: file.id.to_string(),
            file_unique_id: file.unique_id.to_string(),
            file_size: Some(file.size as i64),
            file_path: Some(file.path.clone()),
        };

        context.set_pin_value("file_info", json!(file_info)).await?;
        context.set_pin_value("file_path", json!(file.path)).await?;

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
// Get User Profile Photos Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetUserProfilePhotosNode;

impl GetUserProfilePhotosNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetUserProfilePhotosNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_user_profile_photos",
            "Get User Profile Photos",
            "Gets a list of profile pictures for a user",
            "Telegram/Files",
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
            "user_id",
            "User ID",
            "Unique identifier of the target user",
            VariableType::Integer,
        );

        node.add_input_pin(
            "offset",
            "Offset",
            "Sequential number of the first photo to return (default: 0)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of photos to retrieve (1-100, default: 100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after photos are retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "photos",
            "Photos",
            "User profile photos result",
            VariableType::Struct,
        )
        .set_schema::<UserProfilePhotosResult>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total_count",
            "Total Count",
            "Total number of profile pictures the user has",
            VariableType::Integer,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let offset: i64 = context.evaluate_pin::<i64>("offset").await.unwrap_or(0);
        let limit: i64 = context.evaluate_pin::<i64>("limit").await.unwrap_or(100);

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let photos = bot
            .bot
            .get_user_profile_photos(UserId(user_id as u64))
            .offset(offset as u32)
            .limit(limit as u8)
            .await?;

        let photos_result = UserProfilePhotosResult {
            total_count: photos.total_count as i64,
            photos: photos
                .photos
                .iter()
                .map(|photo_sizes| {
                    photo_sizes
                        .iter()
                        .map(|ps| PhotoInfo {
                            file_id: ps.file.id.to_string(),
                            file_unique_id: ps.file.unique_id.to_string(),
                            width: ps.width as i64,
                            height: ps.height as i64,
                            file_size: Some(ps.file.size as i64),
                        })
                        .collect()
                })
                .collect(),
        };

        context
            .set_pin_value("photos", json!(photos_result))
            .await?;
        context
            .set_pin_value("total_count", json!(photos_result.total_count))
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
// Delete Messages Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteMessagesNode;

impl DeleteMessagesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteMessagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_delete_messages",
            "Delete Messages",
            "Deletes multiple messages simultaneously (up to 100)",
            "Telegram/Files",
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
            "message_ids",
            "Message IDs",
            "Array of message identifiers to delete (1-100 messages)",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after messages are deleted",
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
        let message_ids: Vec<i64> = context.evaluate_pin("message_ids").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let msg_ids: Vec<MessageId> = message_ids.iter().map(|id| MessageId(*id as i32)).collect();

        let result = bot.bot.delete_messages(chat_id, msg_ids).await;

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
// Forward Messages Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ForwardMessagesNode;

impl ForwardMessagesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ForwardMessagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_forward_messages",
            "Forward Messages",
            "Forwards multiple messages simultaneously (up to 100)",
            "Telegram/Files",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session (target chat)",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "from_chat_id",
            "From Chat ID",
            "ID of the chat to forward messages from",
            VariableType::String,
        );

        node.add_input_pin(
            "message_ids",
            "Message IDs",
            "Array of message identifiers to forward (1-100 messages)",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "disable_notification",
            "Silent",
            "Forward without notification",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "protect_content",
            "Protect Content",
            "Protects the contents of the forwarded messages from forwarding and saving",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after messages are forwarded",
            VariableType::Execution,
        );

        node.add_output_pin(
            "forwarded_message_ids",
            "Forwarded Message IDs",
            "Array of IDs of the forwarded messages",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array);

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let from_chat_id: String = context.evaluate_pin("from_chat_id").await?;
        let message_ids: Vec<i64> = context.evaluate_pin("message_ids").await?;
        let disable_notification: bool = context
            .evaluate_pin::<bool>("disable_notification")
            .await
            .unwrap_or(false);
        let protect_content: bool = context
            .evaluate_pin::<bool>("protect_content")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let to_chat = session.chat_id()?;
        let from_chat = teloxide::types::ChatId(from_chat_id.parse()?);

        let msg_ids: Vec<MessageId> = message_ids.iter().map(|id| MessageId(*id as i32)).collect();

        let forwarded = bot
            .bot
            .forward_messages(to_chat, from_chat, msg_ids)
            .disable_notification(disable_notification)
            .protect_content(protect_content)
            .await?;

        let forwarded_ids: Vec<i64> = forwarded.iter().map(|msg_id| msg_id.0 as i64).collect();

        context
            .set_pin_value("forwarded_message_ids", json!(forwarded_ids))
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
// Copy Messages Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CopyMessagesNode;

impl CopyMessagesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CopyMessagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_copy_messages",
            "Copy Messages",
            "Copies multiple messages without 'Forwarded from' header (up to 100)",
            "Telegram/Files",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session (target chat)",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "from_chat_id",
            "From Chat ID",
            "ID of the chat to copy messages from",
            VariableType::String,
        );

        node.add_input_pin(
            "message_ids",
            "Message IDs",
            "Array of message identifiers to copy (1-100 messages)",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "disable_notification",
            "Silent",
            "Copy without notification",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "protect_content",
            "Protect Content",
            "Protects the contents of the copied messages from forwarding and saving",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "remove_caption",
            "Remove Caption",
            "Pass True to remove captions from the copied messages",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after messages are copied",
            VariableType::Execution,
        );

        node.add_output_pin(
            "copied_message_ids",
            "Copied Message IDs",
            "Array of IDs of the copied messages",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array);

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let from_chat_id: String = context.evaluate_pin("from_chat_id").await?;
        let message_ids: Vec<i64> = context.evaluate_pin("message_ids").await?;
        let disable_notification: bool = context
            .evaluate_pin::<bool>("disable_notification")
            .await
            .unwrap_or(false);
        let protect_content: bool = context
            .evaluate_pin::<bool>("protect_content")
            .await
            .unwrap_or(false);
        let remove_caption: bool = context
            .evaluate_pin::<bool>("remove_caption")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let to_chat = session.chat_id()?;
        let from_chat = teloxide::types::ChatId(from_chat_id.parse()?);

        let msg_ids: Vec<MessageId> = message_ids.iter().map(|id| MessageId(*id as i32)).collect();

        let copied = bot
            .bot
            .copy_messages(to_chat, from_chat, msg_ids)
            .disable_notification(disable_notification)
            .protect_content(protect_content)
            .remove_caption(remove_caption)
            .await?;

        let copied_ids: Vec<i64> = copied.iter().map(|id| id.0 as i64).collect();

        context
            .set_pin_value("copied_message_ids", json!(copied_ids))
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
