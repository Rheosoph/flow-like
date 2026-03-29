//! Telegram games operations

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
use teloxide::types::{MessageId, UserId};

/// Game high score information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GameHighScore {
    pub position: i64,
    pub user_id: i64,
    pub score: i64,
}

/// Result from sending a game
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SentGame {
    pub message_id: String,
    pub chat_id: String,
    pub date: i64,
}

// ============================================================================
// Send Game Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendGameNode;

impl SendGameNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendGameNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_game",
            "Send Game",
            "Sends a game to the chat. The game must be registered in BotFather first.",
            "Telegram/Games",
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
            "game_short_name",
            "Game Short Name",
            "Short name of the game as registered in BotFather",
            VariableType::String,
        );

        node.add_input_pin(
            "disable_notification",
            "Silent",
            "Send without notification sound",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "protect_content",
            "Protect Content",
            "Protect the game message from forwarding and saving",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "reply_to_message_id",
            "Reply To",
            "Optional message ID to reply to",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["optional".into()])
                .build(),
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after game is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_game",
            "Sent Game",
            "Information about the sent game message",
            VariableType::Struct,
        )
        .set_schema::<SentGame>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let game_short_name: String = context.evaluate_pin("game_short_name").await?;
        let disable_notification: bool = context
            .evaluate_pin::<bool>("disable_notification")
            .await
            .unwrap_or(false);
        let protect_content: bool = context
            .evaluate_pin::<bool>("protect_content")
            .await
            .unwrap_or(false);
        let reply_to: Option<String> = context
            .evaluate_pin::<String>("reply_to_message_id")
            .await
            .ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let mut request = bot
            .bot
            .send_game(chat_id, game_short_name)
            .disable_notification(disable_notification)
            .protect_content(protect_content);

        if let Some(reply_id) = reply_to
            && let Ok(msg_id) = reply_id.parse::<i32>()
        {
            request =
                request.reply_parameters(teloxide::types::ReplyParameters::new(MessageId(msg_id)));
        }

        let sent = request.await?;

        let sent_game = SentGame {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context.set_pin_value("sent_game", json!(sent_game)).await?;

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
// Set Game Score Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetGameScoreNode;

impl SetGameScoreNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetGameScoreNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_game_score",
            "Set Game Score",
            "Sets the score of the specified user in a game. Use either chat_id + message_id OR inline_message_id.",
            "Telegram/Games",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session (optional if using inline_message_id)",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "user_id",
            "User ID",
            "User identifier",
            VariableType::Integer,
        );

        node.add_input_pin(
            "score",
            "Score",
            "New score (must be non-negative)",
            VariableType::Integer,
        );

        node.add_input_pin(
            "force",
            "Force",
            "Pass true to update score even if new score is not greater than current",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "disable_edit_message",
            "Disable Edit Message",
            "Pass true to prevent automatic game message editing",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "message_id",
            "Message ID",
            "Required if inline_message_id is not specified. ID of the sent game message.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["optional".into()])
                .build(),
        );

        node.add_input_pin(
            "inline_message_id",
            "Inline Message ID",
            "Required if chat_id and message_id are not specified. ID of the inline message.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["optional".into()])
                .build(),
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after score is set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the score was set successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let score: i64 = context.evaluate_pin("score").await?;
        let force: bool = context.evaluate_pin::<bool>("force").await.unwrap_or(false);
        let disable_edit_message: bool = context
            .evaluate_pin::<bool>("disable_edit_message")
            .await
            .unwrap_or(false);
        let message_id: Option<String> = context.evaluate_pin::<String>("message_id").await.ok();
        let inline_message_id: Option<String> = context
            .evaluate_pin::<String>("inline_message_id")
            .await
            .ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = if let Some(inline_id) = inline_message_id {
            bot.bot
                .set_game_score_inline(UserId(user_id as u64), score as u64, inline_id)
                .force(force)
                .disable_edit_message(disable_edit_message)
                .await
                .map(|_| true)
        } else {
            let chat_id = session.chat_id()?;
            let msg_id = message_id
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Either message_id or inline_message_id must be provided"
                    )
                })?
                .parse::<i32>()
                .map_err(|_| flow_like_types::anyhow!("Invalid message_id format"))?;

            // Note: set_game_score expects chat_id as u32
            let chat_id_u32 = chat_id.0.unsigned_abs() as u32;

            bot.bot
                .set_game_score(
                    UserId(user_id as u64),
                    score as u64,
                    chat_id_u32,
                    MessageId(msg_id),
                )
                .force(force)
                .disable_edit_message(disable_edit_message)
                .await
                .map(|_| true)
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
// Get Game High Scores Node
// ============================================================================
// NOTE: This node is currently disabled due to a teloxide bug where
// GetGameHighScores payload incorrectly returns `True` instead of `Vec<GameHighScore>`.
// The teloxide library has a type error in its payload definition.
// Once teloxide fixes this, uncomment the following implementation.

/*
#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetGameHighScoresNode;

impl GetGameHighScoresNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetGameHighScoresNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_game_high_scores",
            "Get Game High Scores",
            "Gets high scores for a game. Use either chat_id + message_id OR inline_message_id.",
            "Telegram/Games",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session (optional if using inline_message_id)",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "user_id",
            "User ID",
            "Target user ID",
            VariableType::Integer,
        );

        node.add_input_pin(
            "message_id",
            "Message ID",
            "Required if inline_message_id is not specified. ID of the sent game message.",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_valid_values(vec!["optional".into()]).build());

        node.add_input_pin(
            "inline_message_id",
            "Inline Message ID",
            "Required if chat_id and message_id are not specified. ID of the inline message.",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_valid_values(vec!["optional".into()]).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after high scores are retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "high_scores",
            "High Scores",
            "List of game high scores",
            VariableType::Struct,
        )
        .set_schema::<Vec<GameHighScore>>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of high scores returned",
            VariableType::Integer,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let message_id: Option<String> = context.evaluate_pin::<String>("message_id").await.ok();
        let inline_message_id: Option<String> = context
            .evaluate_pin::<String>("inline_message_id")
            .await
            .ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let scores = if let Some(inline_id) = inline_message_id {
            let target = TargetMessage::Inline { inline_message_id: inline_id };
            bot.bot
                .get_game_high_scores(UserId(user_id as u64), target)
                .await?
        } else {
            let chat_id = session.chat_id()?;
            let msg_id = message_id
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Either message_id or inline_message_id must be provided"
                    )
                })?
                .parse::<i32>()
                .map_err(|_| flow_like_types::anyhow!("Invalid message_id format"))?;

            let target = TargetMessage::Common {
                chat_id: teloxide::types::Recipient::Id(chat_id),
                message_id: MessageId(msg_id)
            };
            bot.bot
                .get_game_high_scores(UserId(user_id as u64), target)
                .await?
        };

        let high_scores: Vec<GameHighScore> = scores
            .iter()
            .map(|s| GameHighScore {
                position: s.position as i64,
                user_id: s.user.id.0 as i64,
                score: s.score as i64,
            })
            .collect();

        let count = high_scores.len() as i64;

        context
            .set_pin_value("high_scores", json!(high_scores))
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
*/

// ============================================================================
// Set Game Score Inline Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetGameScoreInlineNode;

impl SetGameScoreInlineNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetGameScoreInlineNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_game_score_inline",
            "Set Game Score (Inline)",
            "Sets the score of the specified user in a game sent via an inline message. Returns an error if the new score is not greater than the user's current score unless force is true.",
            "Telegram/Games",
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
            "User identifier",
            VariableType::Integer,
        );

        node.add_input_pin(
            "score",
            "Score",
            "New score (must be non-negative)",
            VariableType::Integer,
        );

        node.add_input_pin(
            "inline_message_id",
            "Inline Message ID",
            "Identifier of the inline message",
            VariableType::String,
        );

        node.add_input_pin(
            "force",
            "Force",
            "Pass true to update score even if new score is not greater than current",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "disable_edit_message",
            "Disable Edit Message",
            "Pass true to prevent automatic game message editing to include the current scoreboard",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after score is set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the score was set successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let score: i64 = context.evaluate_pin("score").await?;
        let inline_message_id: String = context.evaluate_pin("inline_message_id").await?;
        let force: bool = context.evaluate_pin::<bool>("force").await.unwrap_or(false);
        let disable_edit_message: bool = context
            .evaluate_pin::<bool>("disable_edit_message")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot
            .bot
            .set_game_score_inline(UserId(user_id as u64), score as u64, inline_message_id)
            .force(force)
            .disable_edit_message(disable_edit_message)
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
