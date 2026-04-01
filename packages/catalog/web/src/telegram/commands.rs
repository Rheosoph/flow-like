//! Bot commands management nodes for Telegram
//!
//! This module provides nodes for managing bot commands and descriptions.

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Result, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(feature = "execute")]
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::{BotCommand, BotCommandScope, Recipient};

use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;

// =============================================================================
// DATA STRUCTURES
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BotCommandInfo {
    pub command: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdminRights {
    pub is_anonymous: bool,
    pub can_manage_chat: bool,
    pub can_delete_messages: bool,
    pub can_manage_video_chats: bool,
    pub can_restrict_members: bool,
    pub can_promote_members: bool,
    pub can_change_info: bool,
    pub can_invite_users: bool,
    pub can_post_messages: bool,
    pub can_edit_messages: bool,
    pub can_pin_messages: bool,
    pub can_manage_topics: bool,
}

#[cfg(feature = "execute")]
fn build_scope(
    scope_str: &str,
    chat_id: Option<i64>,
    user_id: Option<i64>,
) -> std::result::Result<BotCommandScope, &'static str> {
    match scope_str {
        "all_private_chats" => Ok(BotCommandScope::AllPrivateChats),
        "all_group_chats" => Ok(BotCommandScope::AllGroupChats),
        "all_chat_administrators" => Ok(BotCommandScope::AllChatAdministrators),
        "chat" => {
            if let Some(cid) = chat_id {
                Ok(BotCommandScope::Chat {
                    chat_id: Recipient::Id(ChatId(cid)),
                })
            } else {
                Err("chat_id required for chat scope")
            }
        }
        "chat_administrators" => {
            if let Some(cid) = chat_id {
                Ok(BotCommandScope::ChatAdministrators {
                    chat_id: Recipient::Id(ChatId(cid)),
                })
            } else {
                Err("chat_id required for chat_administrators scope")
            }
        }
        "chat_member" => {
            if let (Some(cid), Some(uid)) = (chat_id, user_id) {
                Ok(BotCommandScope::ChatMember {
                    chat_id: Recipient::Id(ChatId(cid)),
                    user_id: UserId(uid as u64),
                })
            } else {
                Err("chat_id and user_id required for chat_member scope")
            }
        }
        _ => Ok(BotCommandScope::Default),
    }
}

// =============================================================================
// SET MY COMMANDS NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetMyCommandsNode;

impl SetMyCommandsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetMyCommandsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_my_commands",
            "Set My Commands",
            "Sets the list of bot commands for a specific scope and language",
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
        node.add_input_pin(
            "commands",
            "Commands",
            "List of bot commands",
            VariableType::Struct,
        )
        .set_schema::<Vec<BotCommandInfo>>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("scope", "Scope", "Scope: default, all_private_chats, all_group_chats, all_chat_administrators, chat, chat_administrators, chat_member", VariableType::String)
            .set_default_value(Some(json!("default")));
        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "Chat ID (for chat-specific scopes)",
            VariableType::Integer,
        );
        node.add_input_pin(
            "user_id",
            "User ID",
            "User ID (for chat_member scope)",
            VariableType::Integer,
        );
        node.add_input_pin(
            "language_code",
            "Language Code",
            "Two-letter ISO 639-1 language code",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation succeeded",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let commands: Vec<BotCommandInfo> = context.evaluate_pin("commands").await?;
        let scope_str: String = context
            .evaluate_pin("scope")
            .await
            .unwrap_or_else(|_| "default".to_string());
        let chat_id: Option<i64> = context.evaluate_pin("chat_id").await.ok();
        let user_id: Option<i64> = context.evaluate_pin("user_id").await.ok();
        let language_code: Option<String> = context.evaluate_pin("language_code").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        let bot_commands: Vec<BotCommand> = commands
            .into_iter()
            .map(|c| BotCommand::new(c.command, c.description))
            .collect();

        let scope = match build_scope(&scope_str, chat_id, user_id) {
            Ok(s) => s,
            Err(e) => {
                context.set_pin_value("success", json!(false)).await?;
                context.set_pin_value("error", json!(e)).await?;
                return context.activate_exec_pin("exec_out").await;
            }
        };

        let mut request = cached_bot.bot.set_my_commands(bot_commands).scope(scope);
        if let Some(lang) = language_code {
            request = request.language_code(lang);
        }

        match request.await {
            Ok(_) => {
                context.set_pin_value("success", json!(true)).await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("success", json!(false)).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// DELETE MY COMMANDS NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteMyCommandsNode;

impl DeleteMyCommandsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteMyCommandsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_delete_my_commands",
            "Delete My Commands",
            "Deletes the list of bot commands for a specific scope and language",
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
        node.add_input_pin("scope", "Scope", "Scope: default, all_private_chats, all_group_chats, all_chat_administrators, chat, chat_administrators, chat_member", VariableType::String)
            .set_default_value(Some(json!("default")));
        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "Chat ID (for chat-specific scopes)",
            VariableType::Integer,
        );
        node.add_input_pin(
            "user_id",
            "User ID",
            "User ID (for chat_member scope)",
            VariableType::Integer,
        );
        node.add_input_pin(
            "language_code",
            "Language Code",
            "Two-letter ISO 639-1 language code",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation succeeded",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let scope_str: String = context
            .evaluate_pin("scope")
            .await
            .unwrap_or_else(|_| "default".to_string());
        let chat_id: Option<i64> = context.evaluate_pin("chat_id").await.ok();
        let user_id: Option<i64> = context.evaluate_pin("user_id").await.ok();
        let language_code: Option<String> = context.evaluate_pin("language_code").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        let scope = match build_scope(&scope_str, chat_id, user_id) {
            Ok(s) => s,
            Err(e) => {
                context.set_pin_value("success", json!(false)).await?;
                context.set_pin_value("error", json!(e)).await?;
                return context.activate_exec_pin("exec_out").await;
            }
        };

        let mut request = cached_bot.bot.delete_my_commands().scope(scope);
        if let Some(lang) = language_code {
            request = request.language_code(lang);
        }

        match request.await {
            Ok(_) => {
                context.set_pin_value("success", json!(true)).await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("success", json!(false)).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// GET MY COMMANDS NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetMyCommandsNode;

impl GetMyCommandsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetMyCommandsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_my_commands",
            "Get My Commands",
            "Gets the list of bot commands for a specific scope and language",
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
        node.add_input_pin("scope", "Scope", "Scope: default, all_private_chats, all_group_chats, all_chat_administrators, chat, chat_administrators, chat_member", VariableType::String)
            .set_default_value(Some(json!("default")));
        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "Chat ID (for chat-specific scopes)",
            VariableType::Integer,
        );
        node.add_input_pin(
            "user_id",
            "User ID",
            "User ID (for chat_member scope)",
            VariableType::Integer,
        );
        node.add_input_pin(
            "language_code",
            "Language Code",
            "Two-letter ISO 639-1 language code",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "commands",
            "Commands",
            "List of bot commands",
            VariableType::Struct,
        )
        .set_schema::<Vec<BotCommandInfo>>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let scope_str: String = context
            .evaluate_pin("scope")
            .await
            .unwrap_or_else(|_| "default".to_string());
        let chat_id: Option<i64> = context.evaluate_pin("chat_id").await.ok();
        let user_id: Option<i64> = context.evaluate_pin("user_id").await.ok();
        let language_code: Option<String> = context.evaluate_pin("language_code").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        let scope = match build_scope(&scope_str, chat_id, user_id) {
            Ok(s) => s,
            Err(e) => {
                context.set_pin_value("commands", json!([])).await?;
                context.set_pin_value("error", json!(e)).await?;
                return context.activate_exec_pin("exec_out").await;
            }
        };

        let mut request = cached_bot.bot.get_my_commands().scope(scope);
        if let Some(lang) = language_code {
            request = request.language_code(lang);
        }

        match request.await {
            Ok(commands) => {
                let command_infos: Vec<BotCommandInfo> = commands
                    .into_iter()
                    .map(|c| BotCommandInfo {
                        command: c.command,
                        description: c.description,
                    })
                    .collect();
                context
                    .set_pin_value("commands", flow_like_types::json::to_value(&command_infos)?)
                    .await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("commands", json!([])).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// SET MY NAME NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetMyNameNode;

impl SetMyNameNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetMyNameNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_my_name",
            "Set My Name",
            "Sets the bot's name for a specific language",
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
        node.add_input_pin(
            "name",
            "Name",
            "New bot name (0-64 characters)",
            VariableType::String,
        );
        node.add_input_pin(
            "language_code",
            "Language Code",
            "Two-letter ISO 639-1 language code",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation succeeded",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let name: Option<String> = context.evaluate_pin("name").await.ok();
        let language_code: Option<String> = context.evaluate_pin("language_code").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        let mut request = cached_bot.bot.set_my_name();
        if let Some(n) = name {
            request = request.name(n);
        }
        if let Some(lang) = language_code {
            request = request.language_code(lang);
        }

        match request.await {
            Ok(_) => {
                context.set_pin_value("success", json!(true)).await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("success", json!(false)).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// GET MY NAME NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetMyNameNode;

impl GetMyNameNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetMyNameNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_my_name",
            "Get My Name",
            "Gets the bot's name for a specific language",
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
        node.add_input_pin(
            "language_code",
            "Language Code",
            "Two-letter ISO 639-1 language code",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin("name", "Name", "Bot name", VariableType::String);
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let language_code: Option<String> = context.evaluate_pin("language_code").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        let mut request = cached_bot.bot.get_my_name();
        if let Some(lang) = language_code {
            request = request.language_code(lang);
        }

        match request.await {
            Ok(bot_name) => {
                context.set_pin_value("name", json!(bot_name.name)).await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("name", json!("")).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// SET MY DESCRIPTION NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetMyDescriptionNode;

impl SetMyDescriptionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetMyDescriptionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_my_description",
            "Set My Description",
            "Sets the bot's description (shown in empty chat with the bot)",
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
        node.add_input_pin(
            "description",
            "Description",
            "New bot description (0-512 characters)",
            VariableType::String,
        );
        node.add_input_pin(
            "language_code",
            "Language Code",
            "Two-letter ISO 639-1 language code",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation succeeded",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let description: Option<String> = context.evaluate_pin("description").await.ok();
        let language_code: Option<String> = context.evaluate_pin("language_code").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        let mut request = cached_bot.bot.set_my_description();
        if let Some(d) = description {
            request = request.description(d);
        }
        if let Some(lang) = language_code {
            request = request.language_code(lang);
        }

        match request.await {
            Ok(_) => {
                context.set_pin_value("success", json!(true)).await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("success", json!(false)).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// GET MY DESCRIPTION NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetMyDescriptionNode;

impl GetMyDescriptionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetMyDescriptionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_my_description",
            "Get My Description",
            "Gets the bot's description for a specific language",
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
        node.add_input_pin(
            "language_code",
            "Language Code",
            "Two-letter ISO 639-1 language code",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "description",
            "Description",
            "Bot description",
            VariableType::String,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let language_code: Option<String> = context.evaluate_pin("language_code").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        let mut request = cached_bot.bot.get_my_description();
        if let Some(lang) = language_code {
            request = request.language_code(lang);
        }

        match request.await {
            Ok(desc) => {
                context
                    .set_pin_value("description", json!(desc.description))
                    .await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("description", json!("")).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// SET MY SHORT DESCRIPTION NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetMyShortDescriptionNode;

impl SetMyShortDescriptionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetMyShortDescriptionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_my_short_description",
            "Set My Short Description",
            "Sets the bot's short description (shown on the bot's profile page)",
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
        node.add_input_pin(
            "short_description",
            "Short Description",
            "New short description (0-120 characters)",
            VariableType::String,
        );
        node.add_input_pin(
            "language_code",
            "Language Code",
            "Two-letter ISO 639-1 language code",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation succeeded",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let short_description: Option<String> =
            context.evaluate_pin("short_description").await.ok();
        let language_code: Option<String> = context.evaluate_pin("language_code").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        let mut request = cached_bot.bot.set_my_short_description();
        if let Some(d) = short_description {
            request = request.short_description(d);
        }
        if let Some(lang) = language_code {
            request = request.language_code(lang);
        }

        match request.await {
            Ok(_) => {
                context.set_pin_value("success", json!(true)).await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("success", json!(false)).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// GET MY SHORT DESCRIPTION NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetMyShortDescriptionNode;

impl GetMyShortDescriptionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetMyShortDescriptionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_my_short_description",
            "Get My Short Description",
            "Gets the bot's short description for a specific language",
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
        node.add_input_pin(
            "language_code",
            "Language Code",
            "Two-letter ISO 639-1 language code",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "short_description",
            "Short Description",
            "Bot short description",
            VariableType::String,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let language_code: Option<String> = context.evaluate_pin("language_code").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        let mut request = cached_bot.bot.get_my_short_description();
        if let Some(lang) = language_code {
            request = request.language_code(lang);
        }

        match request.await {
            Ok(desc) => {
                context
                    .set_pin_value("short_description", json!(desc.short_description))
                    .await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context
                    .set_pin_value("short_description", json!(""))
                    .await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// SET CHAT MENU BUTTON NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetChatMenuButtonNode;

impl SetChatMenuButtonNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetChatMenuButtonNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_chat_menu_button",
            "Set Chat Menu Button",
            "Changes the bot's menu button in a private chat or default menu button",
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
        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "User ID (leave empty for default)",
            VariableType::Integer,
        );
        node.add_input_pin(
            "menu_type",
            "Menu Type",
            "Menu type: default, commands, or web_app",
            VariableType::String,
        )
        .set_default_value(Some(json!("default")));
        node.add_input_pin(
            "web_app_text",
            "Web App Text",
            "Text on the button (for web_app type)",
            VariableType::String,
        );
        node.add_input_pin(
            "web_app_url",
            "Web App URL",
            "URL of the Web App (for web_app type)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation succeeded",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let chat_id: Option<i64> = context.evaluate_pin("chat_id").await.ok();
        let menu_type: String = context
            .evaluate_pin("menu_type")
            .await
            .unwrap_or_else(|_| "default".to_string());
        let web_app_text: Option<String> = context.evaluate_pin("web_app_text").await.ok();
        let web_app_url: Option<String> = context.evaluate_pin("web_app_url").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        #[cfg(feature = "execute")]
        use teloxide::types::{MenuButton, WebAppInfo};

        let menu_button = match menu_type.as_str() {
            "commands" => MenuButton::Commands,
            "web_app" => {
                if let (Some(text), Some(url)) = (web_app_text, web_app_url) {
                    let parsed_url = url
                        .parse()
                        .map_err(|e| flow_like_types::anyhow!("Invalid URL: {}", e))?;
                    MenuButton::WebApp {
                        text,
                        web_app: WebAppInfo { url: parsed_url },
                    }
                } else {
                    context.set_pin_value("success", json!(false)).await?;
                    context
                        .set_pin_value(
                            "error",
                            json!("web_app_text and web_app_url required for web_app menu"),
                        )
                        .await?;
                    return context.activate_exec_pin("exec_out").await;
                }
            }
            _ => MenuButton::Default,
        };

        let mut request = cached_bot
            .bot
            .set_chat_menu_button()
            .menu_button(menu_button);
        if let Some(cid) = chat_id {
            request = request.chat_id(ChatId(cid));
        }

        match request.await {
            Ok(_) => {
                context.set_pin_value("success", json!(true)).await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("success", json!(false)).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// GET CHAT MENU BUTTON NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetChatMenuButtonNode;

impl GetChatMenuButtonNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetChatMenuButtonNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_chat_menu_button",
            "Get Chat Menu Button",
            "Gets the current bot's menu button in a private chat or default menu button",
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
        node.add_input_pin(
            "chat_id",
            "Chat ID",
            "User ID (leave empty for default)",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "menu_type",
            "Menu Type",
            "Menu type: default, commands, or web_app",
            VariableType::String,
        );
        node.add_output_pin(
            "web_app_text",
            "Web App Text",
            "Text on the button (if web_app)",
            VariableType::String,
        );
        node.add_output_pin(
            "web_app_url",
            "Web App URL",
            "URL of the Web App (if web_app)",
            VariableType::String,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let chat_id: Option<i64> = context.evaluate_pin("chat_id").await.ok();

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        #[cfg(feature = "execute")]
        use teloxide::types::MenuButton;

        let mut request = cached_bot.bot.get_chat_menu_button();
        if let Some(cid) = chat_id {
            request = request.chat_id(ChatId(cid));
        }

        match request.await {
            Ok(menu_button) => {
                match menu_button {
                    MenuButton::Default => {
                        context.set_pin_value("menu_type", json!("default")).await?;
                        context.set_pin_value("web_app_text", json!("")).await?;
                        context.set_pin_value("web_app_url", json!("")).await?;
                    }
                    MenuButton::Commands => {
                        context
                            .set_pin_value("menu_type", json!("commands"))
                            .await?;
                        context.set_pin_value("web_app_text", json!("")).await?;
                        context.set_pin_value("web_app_url", json!("")).await?;
                    }
                    MenuButton::WebApp { text, web_app } => {
                        context.set_pin_value("menu_type", json!("web_app")).await?;
                        context.set_pin_value("web_app_text", json!(text)).await?;
                        context
                            .set_pin_value("web_app_url", json!(web_app.url.to_string()))
                            .await?;
                    }
                }
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("menu_type", json!("")).await?;
                context.set_pin_value("web_app_text", json!("")).await?;
                context.set_pin_value("web_app_url", json!("")).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// SET MY DEFAULT ADMINISTRATOR RIGHTS NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetMyDefaultAdminRightsNode;

impl SetMyDefaultAdminRightsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetMyDefaultAdminRightsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_my_default_admin_rights",
            "Set My Default Admin Rights",
            "Sets default administrator rights requested by the bot when added to groups/channels",
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
        node.add_input_pin(
            "for_channels",
            "For Channels",
            "Pass true for channel rights, false for group rights",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "is_anonymous",
            "Is Anonymous",
            "Anonymous admin",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_manage_chat",
            "Can Manage Chat",
            "Can manage chat",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_delete_messages",
            "Can Delete Messages",
            "Can delete messages",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_manage_video_chats",
            "Can Manage Video Chats",
            "Can manage video chats",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_restrict_members",
            "Can Restrict Members",
            "Can restrict members",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_promote_members",
            "Can Promote Members",
            "Can promote members",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_change_info",
            "Can Change Info",
            "Can change chat info",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_invite_users",
            "Can Invite Users",
            "Can invite users",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_post_messages",
            "Can Post Messages",
            "Can post messages (channels only)",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_edit_messages",
            "Can Edit Messages",
            "Can edit messages (channels only)",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_pin_messages",
            "Can Pin Messages",
            "Can pin messages",
            VariableType::Boolean,
        );
        node.add_input_pin(
            "can_manage_topics",
            "Can Manage Topics",
            "Can manage forum topics",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation succeeded",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let for_channels: bool = context.evaluate_pin("for_channels").await.unwrap_or(false);
        let is_anonymous: bool = context.evaluate_pin("is_anonymous").await.unwrap_or(false);
        let can_manage_chat: bool = context
            .evaluate_pin("can_manage_chat")
            .await
            .unwrap_or(false);
        let can_delete_messages: bool = context
            .evaluate_pin("can_delete_messages")
            .await
            .unwrap_or(false);
        let can_manage_video_chats: bool = context
            .evaluate_pin("can_manage_video_chats")
            .await
            .unwrap_or(false);
        let can_restrict_members: bool = context
            .evaluate_pin("can_restrict_members")
            .await
            .unwrap_or(false);
        let can_promote_members: bool = context
            .evaluate_pin("can_promote_members")
            .await
            .unwrap_or(false);
        let can_change_info: bool = context
            .evaluate_pin("can_change_info")
            .await
            .unwrap_or(false);
        let can_invite_users: bool = context
            .evaluate_pin("can_invite_users")
            .await
            .unwrap_or(false);
        let can_post_messages: bool = context
            .evaluate_pin("can_post_messages")
            .await
            .unwrap_or(false);
        let can_edit_messages: bool = context
            .evaluate_pin("can_edit_messages")
            .await
            .unwrap_or(false);
        let can_pin_messages: bool = context
            .evaluate_pin("can_pin_messages")
            .await
            .unwrap_or(false);
        let can_manage_topics: bool = context
            .evaluate_pin("can_manage_topics")
            .await
            .unwrap_or(false);

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        #[cfg(feature = "execute")]
        use teloxide::types::ChatAdministratorRights;

        let rights = ChatAdministratorRights {
            is_anonymous,
            can_manage_chat,
            can_delete_messages,
            can_manage_video_chats,
            can_restrict_members,
            can_promote_members,
            can_change_info,
            can_invite_users,
            can_post_messages: Some(can_post_messages),
            can_edit_messages: Some(can_edit_messages),
            can_pin_messages: Some(can_pin_messages),
            can_manage_topics: Some(can_manage_topics),
            can_post_stories: None,
            can_edit_stories: None,
            can_delete_stories: None,
        };

        let request = cached_bot
            .bot
            .set_my_default_administrator_rights()
            .rights(rights)
            .for_channels(for_channels);

        match request.await {
            Ok(_) => {
                context.set_pin_value("success", json!(true)).await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("success", json!(false)).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// =============================================================================
// GET MY DEFAULT ADMINISTRATOR RIGHTS NODE
// =============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetMyDefaultAdminRightsNode;

impl GetMyDefaultAdminRightsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetMyDefaultAdminRightsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_my_default_admin_rights",
            "Get My Default Admin Rights",
            "Gets default administrator rights requested by the bot",
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
        node.add_input_pin(
            "for_channels",
            "For Channels",
            "Pass true for channel rights, false for group rights",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after operation",
            VariableType::Execution,
        );
        node.add_output_pin(
            "rights",
            "Rights",
            "Administrator rights",
            VariableType::Struct,
        )
        .set_schema::<AdminRights>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "error",
            "Error",
            "Error message if operation failed",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let for_channels: bool = context.evaluate_pin("for_channels").await.unwrap_or(false);

        let cached_bot = get_telegram_bot(context, &session.ref_id).await?;

        let request = cached_bot
            .bot
            .get_my_default_administrator_rights()
            .for_channels(for_channels);

        match request.await {
            Ok(rights) => {
                let admin_rights = AdminRights {
                    is_anonymous: rights.is_anonymous,
                    can_manage_chat: rights.can_manage_chat,
                    can_delete_messages: rights.can_delete_messages,
                    can_manage_video_chats: rights.can_manage_video_chats,
                    can_restrict_members: rights.can_restrict_members,
                    can_promote_members: rights.can_promote_members,
                    can_change_info: rights.can_change_info,
                    can_invite_users: rights.can_invite_users,
                    can_post_messages: rights.can_post_messages.unwrap_or(false),
                    can_edit_messages: rights.can_edit_messages.unwrap_or(false),
                    can_pin_messages: rights.can_pin_messages.unwrap_or(false),
                    can_manage_topics: rights.can_manage_topics.unwrap_or(false),
                };
                context
                    .set_pin_value("rights", flow_like_types::json::to_value(&admin_rights)?)
                    .await?;
                context.set_pin_value("error", json!("")).await?;
            }
            Err(e) => {
                context.set_pin_value("rights", json!(null)).await?;
                context.set_pin_value("error", json!(e.to_string())).await?;
            }
        }

        context.activate_exec_pin("exec_out").await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}
