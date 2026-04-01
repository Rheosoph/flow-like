//! Discord message operations

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
use serenity::builder::{CreateMessage, EditMessage};

// ============================================================================
// Send Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendMessageNode;

impl SendMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_send_message",
            "Send Message",
            "Sends a message to a Discord channel",
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
            "content",
            "Content",
            "Message content",
            VariableType::String,
        );

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Target channel ID (optional, defaults to session channel)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "reply_to",
            "Reply To",
            "Message ID to reply to (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the sent message",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let content: String = context.evaluate_pin("content").await?;

        let channel_id_override: String =
            context.evaluate_pin("channel_id").await.unwrap_or_default();

        let reply_to: String = context.evaluate_pin("reply_to").await.unwrap_or_default();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = if channel_id_override.is_empty() {
            session.channel_id()?
        } else {
            serenity::all::ChannelId::new(channel_id_override.parse()?)
        };

        let mut message = CreateMessage::new().content(&content);

        if !reply_to.is_empty() {
            let msg_id = serenity::all::MessageId::new(reply_to.parse()?);
            message = message.reference_message((channel_id, msg_id));
        }

        let sent = channel_id.send_message(&client.http, message).await?;

        context
            .set_pin_value("message_id", json!(sent.id.to_string()))
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
// Edit Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditMessageNode;

impl EditMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_edit_message",
            "Edit Message",
            "Edits an existing Discord message",
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
            "ID of the message to edit",
            VariableType::String,
        );

        node.add_input_pin(
            "content",
            "Content",
            "New message content",
            VariableType::String,
        );

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Channel ID (optional, defaults to session channel)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let message_id: String = context.evaluate_pin("message_id").await?;

        let content: String = context.evaluate_pin("content").await?;

        let channel_id_override: String =
            context.evaluate_pin("channel_id").await.unwrap_or_default();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = if channel_id_override.is_empty() {
            session.channel_id()?
        } else {
            serenity::all::ChannelId::new(channel_id_override.parse()?)
        };

        let msg_id = serenity::all::MessageId::new(message_id.parse()?);
        let edit = EditMessage::new().content(&content);

        channel_id.edit_message(&client.http, msg_id, edit).await?;

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
// Delete Message Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteMessageNode;

impl DeleteMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_delete_message",
            "Delete Message",
            "Deletes a Discord message",
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
            "ID of the message to delete",
            VariableType::String,
        );

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Channel ID (optional, defaults to session channel)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let message_id: String = context.evaluate_pin("message_id").await?;

        let channel_id_override: String =
            context.evaluate_pin("channel_id").await.unwrap_or_default();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = if channel_id_override.is_empty() {
            session.channel_id()?
        } else {
            serenity::all::ChannelId::new(channel_id_override.parse()?)
        };

        let msg_id = serenity::all::MessageId::new(message_id.parse()?);

        channel_id.delete_message(&client.http, msg_id).await?;

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
            "discord_pin_message",
            "Pin Message",
            "Pins a message in a Discord channel",
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
            "ID of the message to pin (defaults to current message)",
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

        channel_id.pin(&client.http, msg_id).await?;

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
            "discord_unpin_message",
            "Unpin Message",
            "Unpins a message in a Discord channel",
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
            "ID of the message to unpin (defaults to current message)",
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

        channel_id.unpin(&client.http, msg_id).await?;

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
