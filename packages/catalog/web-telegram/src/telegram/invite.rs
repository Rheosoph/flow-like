//! Telegram chat invite link operations

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

/// Chat invite link information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatInviteLink {
    pub invite_link: String,
    pub creator_id: i64,
    pub creates_join_request: bool,
    pub is_primary: bool,
    pub is_revoked: bool,
    pub name: Option<String>,
    pub expire_date: Option<i64>,
    pub member_limit: Option<i32>,
    pub pending_join_request_count: Option<i32>,
}

// ============================================================================
// Export Chat Invite Link Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ExportChatInviteLinkNode;

impl ExportChatInviteLinkNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ExportChatInviteLinkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_export_chat_invite_link",
            "Export Chat Invite Link",
            "Generates a new primary invite link for a chat. Any previously generated primary link is revoked.",
            "Telegram/Invite",
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
            "Continues after invite link is exported",
            VariableType::Execution,
        );

        node.add_output_pin(
            "invite_link",
            "Invite Link",
            "The new primary invite link",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let link = bot.bot.export_chat_invite_link(chat_id).await?;

        context.set_pin_value("invite_link", json!(link)).await?;

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
// Create Chat Invite Link Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CreateChatInviteLinkNode;

impl CreateChatInviteLinkNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CreateChatInviteLinkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_create_chat_invite_link",
            "Create Chat Invite Link",
            "Creates an additional invite link for a chat",
            "Telegram/Invite",
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
            "Invite link name (0-32 characters)",
            VariableType::String,
        );

        node.add_input_pin(
            "expire_date",
            "Expire Date",
            "Unix timestamp when the link will expire (0 = no expiration)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "member_limit",
            "Member Limit",
            "Max number of users that can join (0 = unlimited, max 99999)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "creates_join_request",
            "Creates Join Request",
            "If True, users need approval to join",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after invite link is created",
            VariableType::Execution,
        );

        node.add_output_pin(
            "link_info",
            "Link Info",
            "Information about the created invite link",
            VariableType::Struct,
        )
        .set_schema::<ChatInviteLink>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let name: Option<String> = context.evaluate_pin::<String>("name").await.ok();
        let expire_date: i64 = context
            .evaluate_pin::<i64>("expire_date")
            .await
            .unwrap_or(0);
        let member_limit: i64 = context
            .evaluate_pin::<i64>("member_limit")
            .await
            .unwrap_or(0);
        let creates_join_request: bool = context
            .evaluate_pin::<bool>("creates_join_request")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let mut request = bot.bot.create_chat_invite_link(chat_id);

        if let Some(n) = name
            && !n.is_empty()
        {
            request = request.name(n);
        }

        if expire_date > 0
            && let Some(dt) = chrono::DateTime::from_timestamp(expire_date, 0)
        {
            request = request.expire_date(dt);
        }

        if member_limit > 0 {
            request = request.member_limit(member_limit as u32);
        }

        request = request.creates_join_request(creates_join_request);

        let link = request.await?;

        let link_info = ChatInviteLink {
            invite_link: link.invite_link.clone(),
            creator_id: link.creator.id.0 as i64,
            creates_join_request: link.creates_join_request,
            is_primary: link.is_primary,
            is_revoked: link.is_revoked,
            name: link.name.clone(),
            expire_date: link.expire_date.map(|d| d.timestamp()),
            member_limit: link.member_limit.map(|l| l as i32),
            pending_join_request_count: link.pending_join_request_count.map(|c| c as i32),
        };

        context.set_pin_value("link_info", json!(link_info)).await?;

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
// Edit Chat Invite Link Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditChatInviteLinkNode;

impl EditChatInviteLinkNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditChatInviteLinkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_edit_chat_invite_link",
            "Edit Chat Invite Link",
            "Edits a non-primary invite link created by the bot",
            "Telegram/Invite",
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
            "invite_link",
            "Invite Link",
            "The invite link to edit",
            VariableType::String,
        );

        node.add_input_pin(
            "name",
            "Name",
            "Invite link name (0-32 characters)",
            VariableType::String,
        );

        node.add_input_pin(
            "expire_date",
            "Expire Date",
            "Unix timestamp when the link will expire (0 = no expiration)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "member_limit",
            "Member Limit",
            "Max number of users that can join (0 = unlimited, max 99999)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "creates_join_request",
            "Creates Join Request",
            "If True, users need approval to join",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after invite link is edited",
            VariableType::Execution,
        );

        node.add_output_pin(
            "edited_link",
            "Edited Link",
            "The edited invite link URL",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let invite_link: String = context.evaluate_pin("invite_link").await?;
        let name: Option<String> = context.evaluate_pin::<String>("name").await.ok();
        let expire_date: i64 = context
            .evaluate_pin::<i64>("expire_date")
            .await
            .unwrap_or(0);
        let member_limit: i64 = context
            .evaluate_pin::<i64>("member_limit")
            .await
            .unwrap_or(0);
        let creates_join_request: bool = context
            .evaluate_pin::<bool>("creates_join_request")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let mut request = bot.bot.edit_chat_invite_link(chat_id, invite_link);

        if let Some(n) = name
            && !n.is_empty()
        {
            request = request.name(n);
        }

        if expire_date > 0
            && let Some(dt) = chrono::DateTime::from_timestamp(expire_date, 0)
        {
            request = request.expire_date(dt);
        }

        if member_limit > 0 {
            request = request.member_limit(member_limit as u32);
        }

        request = request.creates_join_request(creates_join_request);

        // Note: teloxide 0.11.2 returns String, not ChatInviteLink struct
        let edited_link = request.await?;

        context
            .set_pin_value("edited_link", json!(edited_link))
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
// Revoke Chat Invite Link Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct RevokeChatInviteLinkNode;

impl RevokeChatInviteLinkNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RevokeChatInviteLinkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_revoke_chat_invite_link",
            "Revoke Chat Invite Link",
            "Revokes an invite link created by the bot",
            "Telegram/Invite",
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
            "invite_link",
            "Invite Link",
            "The invite link to revoke",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after invite link is revoked",
            VariableType::Execution,
        );

        node.add_output_pin(
            "revoked_link",
            "Revoked Link",
            "The revoked invite link URL",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let invite_link: String = context.evaluate_pin("invite_link").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        // Note: teloxide 0.11.2 returns String, not ChatInviteLink struct
        let revoked_link = bot
            .bot
            .revoke_chat_invite_link(chat_id, invite_link)
            .await?;

        context
            .set_pin_value("revoked_link", json!(revoked_link))
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
// Create Chat Subscription Invite Link Node
// ============================================================================
// NOTE: These nodes are currently disabled because create_chat_subscription_invite_link
// and edit_chat_subscription_invite_link are not available in teloxide 0.14
// (requires Bot API 7.x support). Once teloxide supports these methods,
// uncomment the following implementations.

/*
/// Subscription invite link information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubscriptionInviteLink {
    pub invite_link: String,
    pub creator_id: i64,
    pub name: Option<String>,
    pub is_revoked: bool,
    pub subscription_period: i32,
    pub subscription_price: i32,
}

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CreateChatSubscriptionInviteLinkNode;

impl CreateChatSubscriptionInviteLinkNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CreateChatSubscriptionInviteLinkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_create_chat_subscription_invite_link",
            "Create Chat Subscription Invite Link",
            "Creates a subscription invite link for a channel chat. The bot must have the can_invite_users administrator right.",
            "Telegram/Invite",
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
            "Invite link name (0-32 characters)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "subscription_period",
            "Subscription Period",
            "The number of seconds the subscription will be active (e.g., 2592000 for 30 days)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2592000))); // 30 days

        node.add_input_pin(
            "subscription_price",
            "Subscription Price",
            "The amount of Telegram Stars a user must pay to join (1-2500)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after subscription invite link is created",
            VariableType::Execution,
        );

        node.add_output_pin(
            "link_info",
            "Link Info",
            "Information about the created subscription invite link",
            VariableType::Struct,
        )
        .set_schema::<SubscriptionInviteLink>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "invite_link",
            "Invite Link",
            "The created subscription invite link URL",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let name: String = context
            .evaluate_pin::<String>("name")
            .await
            .unwrap_or_default();
        let subscription_period: i64 = context
            .evaluate_pin::<i64>("subscription_period")
            .await
            .unwrap_or(2592000);
        let subscription_price: i64 = context
            .evaluate_pin::<i64>("subscription_price")
            .await
            .unwrap_or(1);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let mut request = bot.bot.create_chat_subscription_invite_link(
            chat_id,
            subscription_period as u32,
            subscription_price as u32,
        );

        if !name.is_empty() {
            request = request.name(name);
        }

        let link = request.await?;

        let link_info = SubscriptionInviteLink {
            invite_link: link.invite_link.clone(),
            creator_id: link.creator.id.0 as i64,
            name: link.name.clone(),
            is_revoked: link.is_revoked,
            subscription_period: link.subscription_period.unwrap_or(0) as i32,
            subscription_price: link.subscription_price.unwrap_or(0) as i32,
        };

        context.set_pin_value("link_info", json!(link_info)).await?;
        context
            .set_pin_value("invite_link", json!(link.invite_link))
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
// Edit Chat Subscription Invite Link Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditChatSubscriptionInviteLinkNode;

impl EditChatSubscriptionInviteLinkNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditChatSubscriptionInviteLinkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_edit_chat_subscription_invite_link",
            "Edit Chat Subscription Invite Link",
            "Edits a subscription invite link created by the bot. The bot must have the can_invite_users administrator right.",
            "Telegram/Invite",
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
            "invite_link",
            "Invite Link",
            "The subscription invite link to edit",
            VariableType::String,
        );

        node.add_input_pin(
            "name",
            "Name",
            "Invite link name (0-32 characters)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after subscription invite link is edited",
            VariableType::Execution,
        );

        node.add_output_pin(
            "edited_link",
            "Edited Link",
            "The edited subscription invite link URL",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let invite_link: String = context.evaluate_pin("invite_link").await?;
        let name: String = context
            .evaluate_pin::<String>("name")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let mut request = bot
            .bot
            .edit_chat_subscription_invite_link(chat_id, invite_link);

        if !name.is_empty() {
            request = request.name(name);
        }

        let link = request.await?;

        context
            .set_pin_value("edited_link", json!(link.invite_link))
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
*/
