//! Telegram user types and conversion nodes

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

/// Generic user structure from chat events
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GenericUser {
    pub id: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub is_bot: bool,
}

/// Telegram-typed user with additional fields
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TelegramUser {
    pub id: i64,
    pub username: Option<String>,
    pub first_name: String,
    pub last_name: Option<String>,
    pub language_code: Option<String>,
    pub is_bot: bool,
    pub is_premium: bool,
}

impl TelegramUser {
    pub fn full_name(&self) -> String {
        match &self.last_name {
            Some(last) => format!("{} {}", self.first_name, last),
            None => self.first_name.clone(),
        }
    }

    pub fn mention(&self) -> String {
        match &self.username {
            Some(username) => format!("@{}", username),
            None => format!("[{}](tg://user?id={})", self.full_name(), self.id),
        }
    }
}

impl From<GenericUser> for TelegramUser {
    fn from(user: GenericUser) -> Self {
        let id: i64 = user.id.parse().unwrap_or(0);
        let (first_name, last_name) = user
            .display_name
            .clone()
            .map(|name| {
                let parts: Vec<&str> = name.splitn(2, ' ').collect();
                if parts.len() > 1 {
                    (parts[0].to_string(), Some(parts[1].to_string()))
                } else {
                    (name, None)
                }
            })
            .unwrap_or_else(|| (user.username.clone().unwrap_or_default(), None));

        Self {
            id,
            username: user.username,
            first_name,
            last_name,
            language_code: None,
            is_bot: user.is_bot,
            is_premium: false,
        }
    }
}

// ============================================================================
// To Telegram User Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ToTelegramUserNode;

impl ToTelegramUserNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ToTelegramUserNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_to_user",
            "To Telegram User",
            "Converts a generic user from Chat Event to a typed Telegram user",
            "Telegram",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin(
            "user_in",
            "User",
            "Generic user from Chat Event",
            VariableType::Struct,
        );

        node.add_output_pin(
            "user_out",
            "Telegram User",
            "Typed Telegram user",
            VariableType::Struct,
        )
        .set_schema::<TelegramUser>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(false);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let generic_user: GenericUser = context.evaluate_pin("user_in").await?;

        let telegram_user = TelegramUser::from(generic_user);

        context
            .set_pin_value("user_out", json!(telegram_user))
            .await?;

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
// Telegram User ID Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetTelegramUserIdNode;

impl GetTelegramUserIdNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetTelegramUserIdNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_user_id",
            "Get User ID",
            "Gets the Telegram user ID as an integer",
            "Telegram/User",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("user", "User", "Telegram user", VariableType::Struct)
            .set_schema::<TelegramUser>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("id", "User ID", "User ID as integer", VariableType::Integer);

        node.set_long_running(false);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let user: TelegramUser = context.evaluate_pin("user").await?;

        context.set_pin_value("id", json!(user.id)).await?;

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
// Get User Chat Boosts Node
// ============================================================================

use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;
#[cfg(feature = "execute")]
use teloxide::prelude::*;

/// User chat boost information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserChatBoost {
    pub boost_id: String,
    pub add_date: i64,
    pub expiration_date: i64,
}

/// Result from getting user chat boosts
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserChatBoosts {
    pub boosts: Vec<UserChatBoost>,
}

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetUserChatBoostsNode;

impl GetUserChatBoostsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetUserChatBoostsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_user_chat_boosts",
            "Get User Chat Boosts",
            "Gets the list of boosts added to a chat by a user. Requires administrator rights in the chat.",
            "Telegram/User",
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
            "Continues after boosts are retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "boosts",
            "Boosts",
            "List of user chat boosts",
            VariableType::Struct,
        )
        .set_schema::<UserChatBoosts>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("count", "Count", "Number of boosts", VariableType::Integer);

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
            .get_user_chat_boosts(chat_id, teloxide::types::UserId(user_id as u64))
            .await?;

        let boosts: Vec<UserChatBoost> = result
            .boosts
            .iter()
            .map(|b| UserChatBoost {
                boost_id: b.boost_id.to_string(),
                add_date: b.add_date.timestamp(),
                expiration_date: b.expiration_date.timestamp(),
            })
            .collect();

        let count = boosts.len() as i64;

        let user_boosts = UserChatBoosts { boosts };

        context.set_pin_value("boosts", json!(user_boosts)).await?;
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
// Set User Emoji Status Node
// ============================================================================
// NOTE: This node is currently disabled because set_user_emoji_status
// is not available in teloxide 0.14 (requires Bot API 7.x support).
// Once teloxide supports this method, uncomment the following implementation.

/*
#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetUserEmojiStatusNode;

impl SetUserEmojiStatusNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetUserEmojiStatusNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_user_emoji_status",
            "Set User Emoji Status",
            "Changes the emoji status for a given user that previously allowed the bot to manage their emoji status via allowUserToWriteToChat.",
            "Telegram/User",
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
            "emoji_status_custom_emoji_id",
            "Emoji Status Custom Emoji ID",
            "Custom emoji identifier of the emoji status to set. Pass empty string to remove status.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "emoji_status_expiration_date",
            "Emoji Status Expiration Date",
            "Unix timestamp when the emoji status should expire (0 = no expiration)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after emoji status is set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the emoji status was set successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let emoji_status_custom_emoji_id: String = context
            .evaluate_pin::<String>("emoji_status_custom_emoji_id")
            .await
            .unwrap_or_default();
        let emoji_status_expiration_date: i64 = context
            .evaluate_pin::<i64>("emoji_status_expiration_date")
            .await
            .unwrap_or(0);

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let mut request = bot
            .bot
            .set_user_emoji_status(teloxide::types::UserId(user_id as u64));

        if !emoji_status_custom_emoji_id.is_empty() {
            request = request.emoji_status_custom_emoji_id(emoji_status_custom_emoji_id);
        }

        if emoji_status_expiration_date > 0 {
            request = request.emoji_status_expiration_date(emoji_status_expiration_date as u32);
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
*/
