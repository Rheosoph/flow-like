//! Discord direct message operations

use super::session::DiscordSession;
#[cfg(feature = "execute")]
use super::session::get_discord_client;
use super::user::DiscordUser;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::json;
#[cfg(feature = "execute")]
use serenity::builder::CreateMessage;

// ============================================================================
// Send DM Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendDMNode;

impl SendDMNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendDMNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_send_dm",
            "Send DM",
            "Sends a direct message to a Discord user",
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
            "user",
            "User",
            "Discord user to send DM to",
            VariableType::Struct,
        )
        .set_schema::<DiscordUser>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "content",
            "Content",
            "Message content",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the sent DM",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let user: DiscordUser = context.evaluate_pin("user").await?;

        let content: String = context.evaluate_pin("content").await?;

        let client = get_discord_client(context, &session.ref_id).await?;

        let user_id = user.user_id()?;
        let dm_channel = user_id.create_dm_channel(&client.http).await?;

        let message = CreateMessage::new().content(&content);
        let sent = dm_channel.send_message(&client.http, message).await?;

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
// Send DM by User ID Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendDMByIdNode;

impl SendDMByIdNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendDMByIdNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_send_dm_by_id",
            "Send DM by ID",
            "Sends a direct message to a Discord user by their ID",
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
            "user_id",
            "User ID",
            "Discord user ID to send DM to",
            VariableType::String,
        );

        node.add_input_pin(
            "content",
            "Content",
            "Message content",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the sent DM",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let user_id_str: String = context.evaluate_pin("user_id").await?;

        let content: String = context.evaluate_pin("content").await?;

        let client = get_discord_client(context, &session.ref_id).await?;

        let user_id = serenity::all::UserId::new(user_id_str.parse()?);
        let dm_channel = user_id.create_dm_channel(&client.http).await?;

        let message = CreateMessage::new().content(&content);
        let sent = dm_channel.send_message(&client.http, message).await?;

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
