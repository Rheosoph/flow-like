//! Discord channel operations

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
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use serenity::builder::CreateThread;

/// Discord channel information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscordChannel {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub guild_id: Option<String>,
    pub topic: Option<String>,
    pub nsfw: bool,
}

// ============================================================================
// Create Thread Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CreateThreadNode;

impl CreateThreadNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CreateThreadNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_create_thread",
            "Create Thread",
            "Creates a thread from a message in Discord",
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
            "name",
            "Thread Name",
            "Name for the new thread",
            VariableType::String,
        );

        node.add_input_pin(
            "message_id",
            "Message ID",
            "Message to create thread from (defaults to current message)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "auto_archive_minutes",
            "Auto Archive",
            "Minutes of inactivity before auto-archiving (60, 1440, 4320, 10080)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1440)));

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "thread_id",
            "Thread ID",
            "ID of the created thread",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let name: String = context.evaluate_pin("name").await?;

        let message_id_override: String =
            context.evaluate_pin("message_id").await.unwrap_or_default();

        let auto_archive: i64 = context
            .evaluate_pin("auto_archive_minutes")
            .await
            .unwrap_or(1440);

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = session.channel_id()?;
        let msg_id = if message_id_override.is_empty() {
            session.message_id()?
        } else {
            serenity::all::MessageId::new(message_id_override.parse()?)
        };

        let archive_duration = match auto_archive {
            60 => serenity::all::AutoArchiveDuration::OneHour,
            1440 => serenity::all::AutoArchiveDuration::OneDay,
            4320 => serenity::all::AutoArchiveDuration::ThreeDays,
            10080 => serenity::all::AutoArchiveDuration::OneWeek,
            _ => serenity::all::AutoArchiveDuration::OneDay,
        };

        let thread = CreateThread::new(name).auto_archive_duration(archive_duration);

        let created = channel_id
            .create_thread_from_message(&client.http, msg_id, thread)
            .await?;

        context
            .set_pin_value("thread_id", json!(created.id.to_string()))
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
// Get Channel Info Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetChannelInfoNode;

impl GetChannelInfoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetChannelInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_get_channel",
            "Get Channel Info",
            "Gets information about a Discord channel",
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
            "channel_id",
            "Channel ID",
            "Channel ID (defaults to session channel)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "channel",
            "Channel",
            "Channel information",
            VariableType::Struct,
        )
        .set_schema::<DiscordChannel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let channel_id_override: String =
            context.evaluate_pin("channel_id").await.unwrap_or_default();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = if channel_id_override.is_empty() {
            session.channel_id()?
        } else {
            serenity::all::ChannelId::new(channel_id_override.parse()?)
        };

        let channel = channel_id.to_channel(&client.http).await?;

        let channel_info = match channel {
            serenity::all::Channel::Guild(gc) => DiscordChannel {
                id: gc.id.to_string(),
                name: gc.name.clone(),
                channel_type: format!("{:?}", gc.kind),
                guild_id: Some(gc.guild_id.to_string()),
                topic: gc.topic.clone(),
                nsfw: gc.nsfw,
            },
            serenity::all::Channel::Private(pc) => DiscordChannel {
                id: pc.id.to_string(),
                name: pc.name(),
                channel_type: "Private".to_string(),
                guild_id: None,
                topic: None,
                nsfw: false,
            },
            _ => {
                return Err(flow_like_types::anyhow!("Unknown channel type"));
            }
        };

        context
            .set_pin_value("channel", json!(channel_info))
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
// Set Channel Topic Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetChannelTopicNode;

impl SetChannelTopicNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetChannelTopicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_set_channel_topic",
            "Set Channel Topic",
            "Sets the topic of a Discord text channel",
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

        node.add_input_pin("topic", "Topic", "New channel topic", VariableType::String);

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Channel ID (defaults to session channel)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let topic: String = context.evaluate_pin("topic").await?;

        let channel_id_override: String =
            context.evaluate_pin("channel_id").await.unwrap_or_default();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = if channel_id_override.is_empty() {
            session.channel_id()?
        } else {
            serenity::all::ChannelId::new(channel_id_override.parse()?)
        };

        let edit = serenity::builder::EditChannel::new().topic(&topic);
        channel_id.edit(&client.http, edit).await?;

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
// Typing Indicator Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct TypingIndicatorNode;

impl TypingIndicatorNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for TypingIndicatorNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_typing",
            "Typing Indicator",
            "Shows typing indicator in a Discord channel",
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
            "channel_id",
            "Channel ID",
            "Channel ID (defaults to session channel)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;

        let channel_id_override: String =
            context.evaluate_pin("channel_id").await.unwrap_or_default();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = if channel_id_override.is_empty() {
            session.channel_id()?
        } else {
            serenity::all::ChannelId::new(channel_id_override.parse()?)
        };

        channel_id.broadcast_typing(&client.http).await?;

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
