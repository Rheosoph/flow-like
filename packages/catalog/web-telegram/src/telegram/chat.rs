//! Telegram chat management operations

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
use teloxide::types::ChatAction;

/// Chat information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatInfo {
    pub id: String,
    pub title: Option<String>,
    pub chat_type: String,
    pub description: Option<String>,
    pub member_count: Option<i64>,
    pub invite_link: Option<String>,
}

// ============================================================================
// Get Chat Info Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetChatInfoNode;

impl GetChatInfoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetChatInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_chat_info",
            "Get Chat Info",
            "Gets information about the current chat",
            "Telegram/Chat",
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
            "Continues after chat info is retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "chat_info",
            "Chat Info",
            "Information about the chat",
            VariableType::Struct,
        )
        .set_schema::<ChatInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let chat = bot.bot.get_chat(chat_id).await?;

        let chat_type = match &chat.kind {
            teloxide::types::ChatFullInfoKind::Public(public) => match &public.kind {
                teloxide::types::ChatFullInfoPublicKind::Group(_) => "group",
                teloxide::types::ChatFullInfoPublicKind::Supergroup(_) => "supergroup",
                teloxide::types::ChatFullInfoPublicKind::Channel(_) => "channel",
            },
            teloxide::types::ChatFullInfoKind::Private(_) => "private",
        };

        let chat_info = ChatInfo {
            id: chat.id.0.to_string(),
            title: chat.title().map(String::from),
            chat_type: chat_type.to_string(),
            description: chat.description().map(String::from),
            member_count: None,
            invite_link: chat.invite_link().map(String::from),
        };

        context.set_pin_value("chat_info", json!(chat_info)).await?;

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
// Get Chat Member Count Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetChatMemberCountNode;

impl GetChatMemberCountNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetChatMemberCountNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_member_count",
            "Get Member Count",
            "Gets the number of members in a chat",
            "Telegram/Chat",
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
            "Continues after count is retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "count",
            "Member Count",
            "Number of members in the chat",
            VariableType::Integer,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let count = bot.bot.get_chat_member_count(chat_id).await?;

        context.set_pin_value("count", json!(count as i64)).await?;

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
// Send Chat Action Node (Typing Indicator)
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendChatActionNode;

impl SendChatActionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendChatActionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_chat_action",
            "Send Chat Action",
            "Sends a chat action like typing, uploading, etc.",
            "Telegram/Chat",
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
            "action",
            "Action",
            "Type of action to show",
            VariableType::String,
        )
        .set_default_value(Some(json!("typing")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "typing".into(),
                    "upload_photo".into(),
                    "record_video".into(),
                    "upload_video".into(),
                    "record_voice".into(),
                    "upload_voice".into(),
                    "upload_document".into(),
                    "find_location".into(),
                    "record_video_note".into(),
                    "upload_video_note".into(),
                ])
                .build(),
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after action is sent",
            VariableType::Execution,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let action_str: String = context
            .evaluate_pin::<String>("action")
            .await
            .unwrap_or_else(|_| "typing".to_string());

        let action = match action_str.as_str() {
            "upload_photo" => ChatAction::UploadPhoto,
            "record_video" => ChatAction::RecordVideo,
            "upload_video" => ChatAction::UploadVideo,
            "record_voice" => ChatAction::RecordVoice,
            "upload_voice" => ChatAction::UploadVoice,
            "upload_document" => ChatAction::UploadDocument,
            "find_location" => ChatAction::FindLocation,
            "record_video_note" => ChatAction::RecordVideoNote,
            "upload_video_note" => ChatAction::UploadVideoNote,
            _ => ChatAction::Typing,
        };

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        bot.bot.send_chat_action(chat_id, action).await?;

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
// Pin Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct PinMessageNode;

impl PinMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PinMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_pin_message",
            "Pin Message",
            "Pins a message in the chat",
            "Telegram/Chat",
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
            "ID of the message to pin (defaults to triggering message)",
            VariableType::String,
        );

        node.add_input_pin(
            "disable_notification",
            "Silent",
            "Don't notify members about the pin",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after message is pinned",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the pin was successful",
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

        let silent: bool = context
            .evaluate_pin::<bool>("disable_notification")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;
        let msg_id: i32 = message_id.parse()?;

        let result = bot
            .bot
            .pin_chat_message(chat_id, teloxide::types::MessageId(msg_id))
            .disable_notification(silent)
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
// Unpin Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct UnpinMessageNode;

impl UnpinMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UnpinMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_unpin_message",
            "Unpin Message",
            "Unpins a message in the chat",
            "Telegram/Chat",
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
            "ID of the message to unpin (leave empty to unpin most recent)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after message is unpinned",
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

        let message_id: Option<String> = context.evaluate_pin::<String>("message_id").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = if let Some(mid) = message_id {
            let msg_id: i32 = mid.parse()?;
            bot.bot
                .unpin_chat_message(chat_id)
                .message_id(teloxide::types::MessageId(msg_id))
                .await
        } else {
            bot.bot.unpin_chat_message(chat_id).await
        };

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
// Unpin All Messages Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct UnpinAllMessagesNode;

impl UnpinAllMessagesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UnpinAllMessagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_unpin_all_messages",
            "Unpin All Messages",
            "Unpins all pinned messages in the chat",
            "Telegram/Chat",
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

        let result = bot.bot.unpin_all_chat_messages(chat_id).await;

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
// Leave Chat Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct LeaveChatNode;

impl LeaveChatNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for LeaveChatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_leave_chat",
            "Leave Chat",
            "Makes the bot leave the chat",
            "Telegram/Chat",
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
            "Continues after leaving",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the bot left successfully",
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

        let result = bot.bot.leave_chat(chat_id).await;

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
// Chat Permissions Struct
// ============================================================================

/// Chat permissions that can be set for default chat permissions
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatPermissionsInput {
    pub can_send_messages: bool,
    pub can_send_media_messages: bool,
    pub can_send_polls: bool,
    pub can_send_other_messages: bool,
    pub can_add_web_page_previews: bool,
    pub can_change_info: bool,
    pub can_invite_users: bool,
    pub can_pin_messages: bool,
}

impl Default for ChatPermissionsInput {
    fn default() -> Self {
        Self {
            can_send_messages: true,
            can_send_media_messages: true,
            can_send_polls: true,
            can_send_other_messages: true,
            can_add_web_page_previews: true,
            can_change_info: false,
            can_invite_users: true,
            can_pin_messages: false,
        }
    }
}

/// Full chat information returned from GetChat
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatFullInfo {
    pub id: String,
    pub chat_type: String,
    pub title: Option<String>,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub description: Option<String>,
    pub invite_link: Option<String>,
    pub has_protected_content: bool,
    pub sticker_set_name: Option<String>,
    pub can_set_sticker_set: bool,
}

// ============================================================================
// Set Chat Permissions Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetChatPermissionsNode;

impl SetChatPermissionsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetChatPermissionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_chat_permissions",
            "Set Chat Permissions",
            "Sets default chat permissions for all members",
            "Telegram/Chat",
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
            "permissions",
            "Permissions",
            "Chat permissions to set",
            VariableType::Struct,
        )
        .set_schema::<ChatPermissionsInput>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after permissions are set",
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
        use teloxide::types::ChatPermissions;

        let session: TelegramSession = context.evaluate_pin("session").await?;
        let perms_input: ChatPermissionsInput = context
            .evaluate_pin::<ChatPermissionsInput>("permissions")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let mut permissions = ChatPermissions::empty();
        if perms_input.can_send_messages {
            permissions |= ChatPermissions::SEND_MESSAGES;
        }
        if perms_input.can_send_media_messages {
            permissions |= ChatPermissions::SEND_MEDIA_MESSAGES;
        }
        if perms_input.can_send_polls {
            permissions |= ChatPermissions::SEND_POLLS;
        }
        if perms_input.can_send_other_messages {
            permissions |= ChatPermissions::SEND_OTHER_MESSAGES;
        }
        if perms_input.can_add_web_page_previews {
            permissions |= ChatPermissions::ADD_WEB_PAGE_PREVIEWS;
        }
        if perms_input.can_change_info {
            permissions |= ChatPermissions::CHANGE_INFO;
        }
        if perms_input.can_invite_users {
            permissions |= ChatPermissions::INVITE_USERS;
        }
        if perms_input.can_pin_messages {
            permissions |= ChatPermissions::PIN_MESSAGES;
        }

        let result = bot.bot.set_chat_permissions(chat_id, permissions).await;

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
// Set Chat Photo Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetChatPhotoNode;

impl SetChatPhotoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetChatPhotoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_chat_photo",
            "Set Chat Photo",
            "Sets a new chat photo",
            "Telegram/Chat",
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
            "photo",
            "Photo",
            "Photo file to set as chat photo",
            VariableType::Struct,
        )
        .set_schema::<flow_like_catalog_core::FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after photo is set",
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
        use flow_like_catalog_core::FlowPath;
        use teloxide::types::InputFile;

        let session: TelegramSession = context.evaluate_pin("session").await?;
        let photo: FlowPath = context.evaluate_pin("photo").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let bytes = photo.get(context, false).await?;
        let name = std::path::Path::new(&photo.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "photo.jpg".to_string());
        let input_file = InputFile::memory(bytes).file_name(name);

        let result = bot.bot.set_chat_photo(chat_id, input_file).await;

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
// Delete Chat Photo Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteChatPhotoNode;

impl DeleteChatPhotoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteChatPhotoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_delete_chat_photo",
            "Delete Chat Photo",
            "Deletes the chat photo",
            "Telegram/Chat",
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
            "Continues after photo is deleted",
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

        let result = bot.bot.delete_chat_photo(chat_id).await;

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
// Set Chat Title Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetChatTitleNode;

impl SetChatTitleNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetChatTitleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_chat_title",
            "Set Chat Title",
            "Sets a new chat title",
            "Telegram/Chat",
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
            "title",
            "Title",
            "New chat title (1-255 characters)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after title is set",
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
        let title: String = context.evaluate_pin("title").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot.bot.set_chat_title(chat_id, title).await;

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
// Set Chat Description Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetChatDescriptionNode;

impl SetChatDescriptionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetChatDescriptionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_chat_description",
            "Set Chat Description",
            "Sets a new chat description",
            "Telegram/Chat",
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
            "description",
            "Description",
            "New chat description (0-255 characters)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after description is set",
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
        let description: String = context
            .evaluate_pin::<String>("description")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .set_chat_description(chat_id)
            .description(description)
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
// Get Chat (Full Info) Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetChatNode;

impl GetChatNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetChatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_chat",
            "Get Chat",
            "Gets full information about a chat",
            "Telegram/Chat",
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
            "Continues after chat info is retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "chat",
            "Chat",
            "Full chat information",
            VariableType::Struct,
        )
        .set_schema::<ChatFullInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let chat = bot.bot.get_chat(chat_id).await?;

        let chat_type = match &chat.kind {
            teloxide::types::ChatFullInfoKind::Public(public) => match &public.kind {
                teloxide::types::ChatFullInfoPublicKind::Group(_) => "group",
                teloxide::types::ChatFullInfoPublicKind::Supergroup(sg) => {
                    // Extract supergroup-specific fields
                    let sticker_set_name = sg.sticker_set_name.clone();
                    let can_set_sticker_set = sg.can_set_sticker_set;

                    let chat_info = ChatFullInfo {
                        id: chat.id.0.to_string(),
                        chat_type: "supergroup".to_string(),
                        title: chat.title().map(String::from),
                        username: chat.username().map(String::from),
                        first_name: None,
                        last_name: None,
                        description: chat.description().map(String::from),
                        invite_link: chat.invite_link().map(String::from),
                        has_protected_content: chat.has_protected_content(),
                        sticker_set_name,
                        can_set_sticker_set,
                    };

                    context.set_pin_value("chat", json!(chat_info)).await?;
                    let exec_out = context.get_pin_by_name("exec_out").await?;
                    context.activate_exec_pin_ref(&exec_out).await?;
                    return Ok(());
                }
                teloxide::types::ChatFullInfoPublicKind::Channel(_) => "channel",
            },
            teloxide::types::ChatFullInfoKind::Private(private) => {
                let chat_info = ChatFullInfo {
                    id: chat.id.0.to_string(),
                    chat_type: "private".to_string(),
                    title: None,
                    username: chat.username().map(String::from),
                    first_name: private.first_name.clone(),
                    last_name: private.last_name.clone(),
                    description: chat.description().map(String::from),
                    invite_link: None,
                    has_protected_content: false,
                    sticker_set_name: None,
                    can_set_sticker_set: false,
                };

                context.set_pin_value("chat", json!(chat_info)).await?;
                let exec_out = context.get_pin_by_name("exec_out").await?;
                context.activate_exec_pin_ref(&exec_out).await?;
                return Ok(());
            }
        };

        let chat_info = ChatFullInfo {
            id: chat.id.0.to_string(),
            chat_type: chat_type.to_string(),
            title: chat.title().map(String::from),
            username: chat.username().map(String::from),
            first_name: None,
            last_name: None,
            description: chat.description().map(String::from),
            invite_link: chat.invite_link().map(String::from),
            has_protected_content: chat.has_protected_content(),
            sticker_set_name: None,
            can_set_sticker_set: false,
        };

        context.set_pin_value("chat", json!(chat_info)).await?;

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
// Set Chat Sticker Set Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetChatStickerSetNode;

impl SetChatStickerSetNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetChatStickerSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_chat_sticker_set",
            "Set Chat Sticker Set",
            "Sets a new group sticker set for a supergroup",
            "Telegram/Chat",
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
            "sticker_set_name",
            "Sticker Set Name",
            "Name of the sticker set to set",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sticker set is applied",
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
        let sticker_set_name: String = context.evaluate_pin("sticker_set_name").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .set_chat_sticker_set(chat_id, sticker_set_name)
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
// Delete Chat Sticker Set Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteChatStickerSetNode;

impl DeleteChatStickerSetNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteChatStickerSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_delete_chat_sticker_set",
            "Delete Chat Sticker Set",
            "Deletes the group sticker set from a supergroup",
            "Telegram/Chat",
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
            "Continues after sticker set is removed",
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

        let result = bot.bot.delete_chat_sticker_set(chat_id).await;

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
// Ban Chat Sender Chat Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct BanChatSenderChatNode;

impl BanChatSenderChatNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for BanChatSenderChatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_ban_chat_sender_chat",
            "Ban Chat Sender Chat",
            "Bans a channel chat in a supergroup or channel",
            "Telegram/Chat",
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
            "sender_chat_id",
            "Sender Chat ID",
            "Unique identifier of the target sender chat",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sender chat is banned",
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
        let sender_chat_id: i64 = context.evaluate_pin("sender_chat_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .ban_chat_sender_chat(chat_id, teloxide::types::ChatId(sender_chat_id))
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
// Unban Chat Sender Chat Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct UnbanChatSenderChatNode;

impl UnbanChatSenderChatNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UnbanChatSenderChatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_unban_chat_sender_chat",
            "Unban Chat Sender Chat",
            "Unbans a previously banned channel chat",
            "Telegram/Chat",
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
            "sender_chat_id",
            "Sender Chat ID",
            "Unique identifier of the target sender chat",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sender chat is unbanned",
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
        let sender_chat_id: i64 = context.evaluate_pin("sender_chat_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .unban_chat_sender_chat(chat_id, teloxide::types::ChatId(sender_chat_id))
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
