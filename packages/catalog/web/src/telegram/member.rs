//! Telegram chat member management operations

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
use teloxide::types::{ChatMemberKind, ChatPermissions, UserId};

/// Chat member information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatMemberInfo {
    pub user_id: i64,
    pub username: Option<String>,
    pub first_name: String,
    pub last_name: Option<String>,
    pub status: String,
    pub is_anonymous: bool,
    pub custom_title: Option<String>,
}

/// Administrator information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdminInfo {
    pub user_id: i64,
    pub username: Option<String>,
    pub first_name: String,
    pub status: String,
    pub can_be_edited: bool,
    pub can_manage_chat: bool,
    pub can_change_info: bool,
    pub can_delete_messages: bool,
    pub can_invite_users: bool,
    pub can_restrict_members: bool,
    pub can_pin_messages: bool,
    pub can_promote_members: bool,
    pub custom_title: Option<String>,
}

// ============================================================================
// Get Chat Member Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetChatMemberNode;

impl GetChatMemberNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetChatMemberNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_chat_member",
            "Get Chat Member",
            "Gets information about a member of a chat",
            "Telegram/Members",
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

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after member info is retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "member",
            "Member Info",
            "Information about the chat member",
            VariableType::Struct,
        )
        .set_schema::<ChatMemberInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "status",
            "Status",
            "Member status (creator, administrator, member, restricted, left, kicked)",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let member = bot
            .bot
            .get_chat_member(chat_id, UserId(user_id as u64))
            .await?;

        let (status, is_anonymous, custom_title) = match &member.kind {
            ChatMemberKind::Owner(o) => (
                "creator".to_string(),
                o.is_anonymous,
                o.custom_title.clone(),
            ),
            ChatMemberKind::Administrator(a) => (
                "administrator".to_string(),
                a.is_anonymous,
                a.custom_title.clone(),
            ),
            ChatMemberKind::Member(_) => ("member".to_string(), false, None),
            ChatMemberKind::Restricted(_) => ("restricted".to_string(), false, None),
            ChatMemberKind::Left => ("left".to_string(), false, None),
            ChatMemberKind::Banned(_) => ("kicked".to_string(), false, None),
        };

        let user = &member.user;
        let member_info = ChatMemberInfo {
            user_id: user.id.0 as i64,
            username: user.username.clone(),
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            status: status.clone(),
            is_anonymous,
            custom_title,
        };

        context.set_pin_value("member", json!(member_info)).await?;
        context.set_pin_value("status", json!(status)).await?;

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
// Get Chat Administrators Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetChatAdministratorsNode;

impl GetChatAdministratorsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetChatAdministratorsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_chat_administrators",
            "Get Chat Administrators",
            "Gets a list of administrators in a chat",
            "Telegram/Members",
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
            "Continues after administrators are retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "administrators",
            "Administrators",
            "List of chat administrators",
            VariableType::Struct,
        )
        .set_schema::<Vec<AdminInfo>>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of administrators",
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

        let admins = bot.bot.get_chat_administrators(chat_id).await?;

        let admin_infos: Vec<AdminInfo> = admins
            .iter()
            .filter_map(|member| {
                let user = &member.user;
                match &member.kind {
                    ChatMemberKind::Owner(o) => Some(AdminInfo {
                        user_id: user.id.0 as i64,
                        username: user.username.clone(),
                        first_name: user.first_name.clone(),
                        status: "creator".to_string(),
                        can_be_edited: false,
                        can_manage_chat: true,
                        can_change_info: true,
                        can_delete_messages: true,
                        can_invite_users: true,
                        can_restrict_members: true,
                        can_pin_messages: true,
                        can_promote_members: true,
                        custom_title: o.custom_title.clone(),
                    }),
                    ChatMemberKind::Administrator(a) => Some(AdminInfo {
                        user_id: user.id.0 as i64,
                        username: user.username.clone(),
                        first_name: user.first_name.clone(),
                        status: "administrator".to_string(),
                        can_be_edited: a.can_be_edited,
                        can_manage_chat: a.can_manage_chat,
                        can_change_info: a.can_change_info,
                        can_delete_messages: a.can_delete_messages,
                        can_invite_users: a.can_invite_users,
                        can_restrict_members: a.can_restrict_members,
                        can_pin_messages: a.can_pin_messages,
                        can_promote_members: a.can_promote_members,
                        custom_title: a.custom_title.clone(),
                    }),
                    _ => None,
                }
            })
            .collect();

        let count = admin_infos.len() as i64;

        context
            .set_pin_value("administrators", json!(admin_infos))
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
// Ban Chat Member Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct BanChatMemberNode;

impl BanChatMemberNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for BanChatMemberNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_ban_chat_member",
            "Ban Chat Member",
            "Bans a user from the chat (removes and prevents rejoining)",
            "Telegram/Members",
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
            "Unique identifier of the user to ban",
            VariableType::Integer,
        );

        node.add_input_pin(
            "until_date",
            "Until Date",
            "Unix timestamp when the ban will be lifted (0 = permanent)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "revoke_messages",
            "Revoke Messages",
            "Delete all messages from the user in the chat",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after user is banned",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the ban was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let until_date: i64 = context.evaluate_pin::<i64>("until_date").await.unwrap_or(0);
        let revoke_messages: bool = context
            .evaluate_pin::<bool>("revoke_messages")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let mut request = bot.bot.ban_chat_member(chat_id, UserId(user_id as u64));

        if until_date > 0
            && let Some(dt) = chrono::DateTime::from_timestamp(until_date, 0)
        {
            request = request.until_date(dt);
        }

        request = request.revoke_messages(revoke_messages);

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
// Unban Chat Member Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct UnbanChatMemberNode;

impl UnbanChatMemberNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UnbanChatMemberNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_unban_chat_member",
            "Unban Chat Member",
            "Unbans a previously banned user in a supergroup or channel",
            "Telegram/Members",
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
            "Unique identifier of the user to unban",
            VariableType::Integer,
        );

        node.add_input_pin(
            "only_if_banned",
            "Only If Banned",
            "Do nothing if the user is not banned",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after user is unbanned",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the unban was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let only_if_banned: bool = context
            .evaluate_pin::<bool>("only_if_banned")
            .await
            .unwrap_or(true);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .unban_chat_member(chat_id, UserId(user_id as u64))
            .only_if_banned(only_if_banned)
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
// Restrict Chat Member Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct RestrictChatMemberNode;

impl RestrictChatMemberNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RestrictChatMemberNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_restrict_chat_member",
            "Restrict Chat Member",
            "Restricts a user's permissions in a supergroup or channel",
            "Telegram/Members",
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
            "Unique identifier of the user to restrict",
            VariableType::Integer,
        );

        node.add_input_pin(
            "can_send_messages",
            "Can Send Messages",
            "Allow sending text messages",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "can_send_media",
            "Can Send Media",
            "Allow sending photos, videos, and other media",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "can_send_other",
            "Can Send Other",
            "Allow sending stickers, GIFs, games, inline bot results",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "can_add_previews",
            "Can Add Previews",
            "Allow adding web page previews to messages",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "until_date",
            "Until Date",
            "Unix timestamp when restrictions will be lifted (0 = permanent)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after restrictions are applied",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the restriction was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;

        let can_send_messages = context
            .evaluate_pin::<bool>("can_send_messages")
            .await
            .unwrap_or(true);
        let can_send_media = context
            .evaluate_pin::<bool>("can_send_media")
            .await
            .unwrap_or(true);
        let can_send_other = context
            .evaluate_pin::<bool>("can_send_other")
            .await
            .unwrap_or(true);
        let can_add_previews = context
            .evaluate_pin::<bool>("can_add_previews")
            .await
            .unwrap_or(true);
        let until_date: i64 = context.evaluate_pin::<i64>("until_date").await.unwrap_or(0);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        // Build permissions using bitflags
        let mut permissions = ChatPermissions::empty();
        if can_send_messages {
            permissions |= ChatPermissions::SEND_MESSAGES;
        }
        if can_send_media {
            permissions |= ChatPermissions::SEND_MEDIA_MESSAGES;
        }
        if can_send_other {
            permissions |= ChatPermissions::SEND_OTHER_MESSAGES;
        }
        if can_add_previews {
            permissions |= ChatPermissions::ADD_WEB_PAGE_PREVIEWS;
        }

        let mut request =
            bot.bot
                .restrict_chat_member(chat_id, UserId(user_id as u64), permissions);

        if until_date > 0
            && let Some(dt) = chrono::DateTime::from_timestamp(until_date, 0)
        {
            request = request.until_date(dt);
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
// Promote Chat Member Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct PromoteChatMemberNode;

impl PromoteChatMemberNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PromoteChatMemberNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_promote_chat_member",
            "Promote Chat Member",
            "Promotes or demotes a user in a supergroup or channel",
            "Telegram/Members",
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
            "Unique identifier of the user to promote",
            VariableType::Integer,
        );

        node.add_input_pin(
            "can_manage_chat",
            "Can Manage Chat",
            "Allow managing chat (view stats, manage groups, etc.)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "can_delete_messages",
            "Can Delete Messages",
            "Allow deleting messages",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "can_restrict_members",
            "Can Restrict Members",
            "Allow restricting and banning members",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "can_promote_members",
            "Can Promote Members",
            "Allow adding new administrators",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "can_change_info",
            "Can Change Info",
            "Allow changing chat title, photo, etc.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "can_invite_users",
            "Can Invite Users",
            "Allow inviting users via link",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "can_pin_messages",
            "Can Pin Messages",
            "Allow pinning messages",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after promotion",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the promotion was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;

        let can_manage_chat = context
            .evaluate_pin::<bool>("can_manage_chat")
            .await
            .unwrap_or(false);
        let can_delete_messages = context
            .evaluate_pin::<bool>("can_delete_messages")
            .await
            .unwrap_or(false);
        let can_restrict_members = context
            .evaluate_pin::<bool>("can_restrict_members")
            .await
            .unwrap_or(false);
        let can_promote_members = context
            .evaluate_pin::<bool>("can_promote_members")
            .await
            .unwrap_or(false);
        let can_change_info = context
            .evaluate_pin::<bool>("can_change_info")
            .await
            .unwrap_or(false);
        let can_invite_users = context
            .evaluate_pin::<bool>("can_invite_users")
            .await
            .unwrap_or(false);
        let can_pin_messages = context
            .evaluate_pin::<bool>("can_pin_messages")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .promote_chat_member(chat_id, UserId(user_id as u64))
            .can_manage_chat(can_manage_chat)
            .can_delete_messages(can_delete_messages)
            .can_restrict_members(can_restrict_members)
            .can_promote_members(can_promote_members)
            .can_change_info(can_change_info)
            .can_invite_users(can_invite_users)
            .can_pin_messages(can_pin_messages)
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
// Set Chat Administrator Custom Title Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetChatAdministratorCustomTitleNode;

impl SetChatAdministratorCustomTitleNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetChatAdministratorCustomTitleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_chat_admin_custom_title",
            "Set Admin Custom Title",
            "Sets a custom title for an administrator in a supergroup",
            "Telegram/Members",
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
            "Unique identifier of the administrator",
            VariableType::Integer,
        );

        node.add_input_pin(
            "custom_title",
            "Custom Title",
            "New custom title for the administrator (0-16 characters)",
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
            "Whether the title was set successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let custom_title: String = context.evaluate_pin("custom_title").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .set_chat_administrator_custom_title(chat_id, UserId(user_id as u64), custom_title)
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
// Approve Chat Join Request Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ApproveChatJoinRequestNode;

impl ApproveChatJoinRequestNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ApproveChatJoinRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_approve_chat_join_request",
            "Approve Join Request",
            "Approves a chat join request",
            "Telegram/Members",
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
            "Unique identifier of the user requesting to join",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after request is approved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the request was approved",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .approve_chat_join_request(chat_id, UserId(user_id as u64))
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
// Decline Chat Join Request Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeclineChatJoinRequestNode;

impl DeclineChatJoinRequestNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeclineChatJoinRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_decline_chat_join_request",
            "Decline Join Request",
            "Declines a chat join request",
            "Telegram/Members",
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
            "Unique identifier of the user whose request is declined",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after request is declined",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the request was declined",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let result = bot
            .bot
            .decline_chat_join_request(chat_id, UserId(user_id as u64))
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
