//! Discord reaction operations

use super::session::DiscordSession;
#[cfg(feature = "execute")]
use super::session::get_discord_client;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use serenity::all::ReactionType;

// ============================================================================
// Add Reaction Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct AddReactionNode;

impl AddReactionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AddReactionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_add_reaction",
            "Add Reaction",
            "Adds a reaction to a Discord message",
            "Discord",
        );
        node.add_icon("/flow/icons/discord.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Discord session",
            VariableType::Struct,
        )
        .set_schema::<DiscordSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "emoji",
            "Emoji",
            "Emoji to react with (e.g., '👍' or custom emoji ID)",
            VariableType::String,
        );

        node.add_input_pin(
            "message_id",
            "Message ID",
            "Message ID to react to (defaults to current message)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let emoji: String = context.evaluate_pin("emoji").await?;

        let message_id_override: String =
            context.evaluate_pin("message_id").await.unwrap_or_default();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = session.channel_id()?;
        let msg_id = if message_id_override.is_empty() {
            session.message_id()?
        } else {
            serenity::all::MessageId::new(message_id_override.parse()?)
        };

        let reaction = if emoji.chars().count() <= 2 {
            ReactionType::Unicode(emoji)
        } else if let Ok(id) = emoji.parse::<u64>() {
            ReactionType::Custom {
                animated: false,
                id: serenity::all::EmojiId::new(id),
                name: None,
            }
        } else {
            ReactionType::Unicode(emoji)
        };

        client
            .http
            .create_reaction(channel_id, msg_id, &reaction)
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Discord functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Remove Reaction Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct RemoveReactionNode;

impl RemoveReactionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RemoveReactionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_remove_reaction",
            "Remove Reaction",
            "Removes the bot's reaction from a Discord message",
            "Discord",
        );
        node.add_icon("/flow/icons/discord.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Discord session",
            VariableType::Struct,
        )
        .set_schema::<DiscordSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("emoji", "Emoji", "Emoji to remove", VariableType::String);

        node.add_input_pin(
            "message_id",
            "Message ID",
            "Message ID (defaults to current message)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let emoji: String = context.evaluate_pin("emoji").await?;

        let message_id_override: String =
            context.evaluate_pin("message_id").await.unwrap_or_default();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = session.channel_id()?;
        let msg_id = if message_id_override.is_empty() {
            session.message_id()?
        } else {
            serenity::all::MessageId::new(message_id_override.parse()?)
        };

        let reaction = if emoji.chars().count() <= 2 {
            ReactionType::Unicode(emoji)
        } else if let Ok(id) = emoji.parse::<u64>() {
            ReactionType::Custom {
                animated: false,
                id: serenity::all::EmojiId::new(id),
                name: None,
            }
        } else {
            ReactionType::Unicode(emoji)
        };

        client
            .http
            .delete_reaction_me(channel_id, msg_id, &reaction)
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Discord functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Clear All Reactions Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ClearReactionsNode;

impl ClearReactionsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ClearReactionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_clear_reactions",
            "Clear Reactions",
            "Removes all reactions from a Discord message",
            "Discord",
        );
        node.add_icon("/flow/icons/discord.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Discord session",
            VariableType::Struct,
        )
        .set_schema::<DiscordSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "message_id",
            "Message ID",
            "Message ID (defaults to current message)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let message_id_override: String =
            context.evaluate_pin("message_id").await.unwrap_or_default();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = session.channel_id()?;
        let msg_id = if message_id_override.is_empty() {
            session.message_id()?
        } else {
            serenity::all::MessageId::new(message_id_override.parse()?)
        };

        client
            .http
            .delete_message_reactions(channel_id, msg_id)
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Discord functionality requires the 'execute' feature"
        ))
    }
}
