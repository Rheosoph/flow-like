//! Telegram bot management operations

use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::json;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use teloxide::prelude::*;

/// Bot information returned by GetMe
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BotInfo {
    pub id: i64,
    pub username: String,
    pub first_name: String,
    pub can_join_groups: bool,
    pub can_read_all_group_messages: bool,
    pub supports_inline_queries: bool,
}

// ============================================================================
// Get Me Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetMeNode;

impl GetMeNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetMeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_me",
            "Get Bot Info",
            "Returns basic information about the bot",
            "Telegram/Bot",
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
            "Continues after bot info is retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "bot_info",
            "Bot Info",
            "Information about the bot",
            VariableType::Struct,
        )
        .set_schema::<BotInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "bot_username",
            "Bot Username",
            "The bot's username",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let me = bot.bot.get_me().await?;

        let bot_info = BotInfo {
            id: me.id.0 as i64,
            username: me.username.clone().unwrap_or_default(),
            first_name: me.first_name.clone(),
            can_join_groups: me.can_join_groups,
            can_read_all_group_messages: me.can_read_all_group_messages,
            supports_inline_queries: me.supports_inline_queries,
        };

        context.set_pin_value("bot_info", json!(bot_info)).await?;
        context
            .set_pin_value("bot_username", json!(bot_info.username))
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
// Log Out Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct LogOutNode;

impl LogOutNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for LogOutNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_log_out",
            "Log Out",
            "Logs out from the cloud Bot API server. Use this before moving the bot to a self-hosted server.",
            "Telegram/Bot",
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
            "Continues after logout",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the logout was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot.bot.log_out().await;

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
// Close Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CloseNode;

impl CloseNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CloseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_close",
            "Close",
            "Closes the bot instance before moving to another local server. Returns error if webhook is set.",
            "Telegram/Bot",
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
            "Continues after close",
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

        let result = bot.bot.close().await;

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
