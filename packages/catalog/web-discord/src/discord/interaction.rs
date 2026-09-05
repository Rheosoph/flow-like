//! Discord user interaction - waiting for messages and sending interactive components

use super::session::DiscordSession;
#[cfg(feature = "execute")]
use super::session::get_discord_client;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use serenity::all::{
    ButtonStyle, ChannelId, CreateActionRow, CreateButton, CreateMessage, CreateSelectMenu,
    CreateSelectMenuKind, CreateSelectMenuOption, EditMessage,
};
#[cfg(feature = "execute")]
use std::sync::Arc;

/// Represents a user's reply message
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserReply {
    /// The message ID of the reply
    pub message_id: String,
    /// The channel ID
    pub channel_id: String,
    /// The user who replied
    pub user_id: String,
    /// The username
    pub username: String,
    /// The text content
    pub text: String,
    /// Unix timestamp
    pub timestamp: i64,
}

/// Button click response (note: requires gateway/webhook for real-time)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ButtonResponse {
    /// The custom ID of the clicked button
    pub custom_id: String,
    /// The user who clicked
    pub user_id: String,
    /// The username
    pub username: String,
    /// The message ID containing the button
    pub message_id: String,
}

/// Select menu response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SelectMenuResponse {
    /// The custom ID of the select menu
    pub custom_id: String,
    /// Selected values
    pub values: Vec<String>,
    /// The user who selected
    pub user_id: String,
    /// The username
    pub username: String,
    /// The message ID
    pub message_id: String,
}

// ============================================================================
// Send and Wait for Reply Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendAndWaitNode;

impl SendAndWaitNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendAndWaitNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_send_and_wait",
            "Send and Wait",
            "Sends a message and waits for a reply. Perfect for dialogs and human-in-the-loop flows.",
            "Discord/Interaction",
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
            "prompt",
            "Prompt",
            "The message to send",
            VariableType::String,
        );

        node.add_input_pin(
            "from_user_id",
            "From User ID",
            "Only accept replies from this user (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "timeout_seconds",
            "Timeout (seconds)",
            "Maximum time to wait (0 = no timeout)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(120)));

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Target channel (optional)",
            VariableType::String,
        );

        node.add_output_pin(
            "on_reply",
            "On Reply",
            "Triggered when user replies",
            VariableType::Execution,
        );

        node.add_output_pin(
            "on_timeout",
            "On Timeout",
            "Triggered when timeout is reached",
            VariableType::Execution,
        );

        node.add_output_pin("reply", "Reply", "The user's reply", VariableType::Struct)
            .set_schema::<UserReply>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "reply_text",
            "Reply Text",
            "The text content of the reply",
            VariableType::String,
        );

        node.add_output_pin(
            "sent_message_id",
            "Sent Message ID",
            "ID of the prompt message",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;
        let prompt: String = context.evaluate_pin("prompt").await?;
        let from_user_id: Option<String> = context.evaluate_pin("from_user_id").await.ok();
        let timeout_secs: i64 = context.evaluate_pin("timeout_seconds").await.unwrap_or(120);
        let channel_override: Option<String> = context.evaluate_pin("channel_id").await.ok();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = match channel_override.filter(|s| !s.is_empty()) {
            Some(id) => ChannelId::new(id.parse()?),
            None => session.channel_id()?,
        };

        let message = CreateMessage::new().content(&prompt);
        let sent = channel_id.send_message(&client.http, message).await?;
        let sent_id = sent.id;

        context
            .set_pin_value("sent_message_id", json!(sent_id.to_string()))
            .await?;

        let filter_user = from_user_id.and_then(|id| id.parse::<u64>().ok());
        let http = client.http.clone();
        let result: Arc<tokio::sync::Mutex<Option<UserReply>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let result_clone = result.clone();

        let poll_task = tokio::spawn(async move {
            let mut last_message_id = sent_id;

            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                let messages = channel_id
                    .messages(
                        &http,
                        serenity::builder::GetMessages::new()
                            .after(last_message_id)
                            .limit(10),
                    )
                    .await;

                if let Ok(msgs) = messages {
                    for msg in msgs.iter().rev() {
                        if msg.id <= last_message_id {
                            continue;
                        }
                        last_message_id = msg.id;

                        if msg.author.bot {
                            continue;
                        }

                        if let Some(expected_user) = filter_user
                            && msg.author.id.get() != expected_user
                        {
                            continue;
                        }

                        if let Some(ref reply) = msg.referenced_message
                            && reply.id == sent_id
                        {
                            let user_reply = UserReply {
                                message_id: msg.id.to_string(),
                                channel_id: channel_id.to_string(),
                                user_id: msg.author.id.to_string(),
                                username: msg.author.name.clone(),
                                text: msg.content.clone(),
                                timestamp: msg.timestamp.unix_timestamp(),
                            };

                            let mut result_guard = result_clone.lock().await;
                            *result_guard = Some(user_reply);
                            return;
                        }
                    }
                }
            }
        });

        let got_reply = if timeout_secs > 0 {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs as u64),
                poll_task,
            )
            .await
            .is_ok()
        } else {
            let _ = poll_task.await;
            true
        };

        let reply_opt = result.lock().await.take();

        if got_reply {
            if let Some(reply) = reply_opt {
                let text = reply.text.clone();
                context.set_pin_value("reply", json!(reply)).await?;
                context.set_pin_value("reply_text", json!(text)).await?;
                context.activate_exec_pin("on_reply").await?;
            } else {
                context.set_pin_value("reply_text", json!("")).await?;
                context.activate_exec_pin("on_timeout").await?;
            }
        } else {
            context.set_pin_value("reply_text", json!("")).await?;
            context.activate_exec_pin("on_timeout").await?;
        }

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
// Wait for Message Node (Generic)
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct WaitForMessageNode;

impl WaitForMessageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for WaitForMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_wait_for_message",
            "Wait For Message",
            "Waits for a new message in the channel from a specific user or anyone",
            "Discord/Interaction",
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
            "from_user_id",
            "From User ID",
            "Only accept messages from this user (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "timeout_seconds",
            "Timeout (seconds)",
            "Maximum time to wait",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(60)));

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Channel to watch (optional)",
            VariableType::String,
        );

        node.add_output_pin(
            "on_message",
            "On Message",
            "Triggered when message received",
            VariableType::Execution,
        );

        node.add_output_pin(
            "on_timeout",
            "On Timeout",
            "Triggered on timeout",
            VariableType::Execution,
        );

        node.add_output_pin(
            "message",
            "Message",
            "The received message",
            VariableType::Struct,
        )
        .set_schema::<UserReply>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "message_text",
            "Message Text",
            "The text content",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;
        let from_user_id: Option<String> = context.evaluate_pin("from_user_id").await.ok();
        let timeout_secs: i64 = context.evaluate_pin("timeout_seconds").await.unwrap_or(60);
        let channel_override: Option<String> = context.evaluate_pin("channel_id").await.ok();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = match channel_override.filter(|s| !s.is_empty()) {
            Some(id) => ChannelId::new(id.parse()?),
            None => session.channel_id()?,
        };

        let filter_user = from_user_id.and_then(|id| id.parse::<u64>().ok());
        let http = client.http.clone();

        let initial_messages = channel_id
            .messages(&http, serenity::builder::GetMessages::new().limit(1))
            .await;
        let start_id = initial_messages
            .ok()
            .and_then(|m| m.first().map(|msg| msg.id))
            .unwrap_or(serenity::all::MessageId::new(0));

        let result: Arc<tokio::sync::Mutex<Option<UserReply>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let result_clone = result.clone();

        let poll_task = tokio::spawn(async move {
            let mut last_message_id = start_id;

            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                let messages = channel_id
                    .messages(
                        &http,
                        serenity::builder::GetMessages::new()
                            .after(last_message_id)
                            .limit(10),
                    )
                    .await;

                if let Ok(msgs) = messages {
                    for msg in msgs.iter().rev() {
                        if msg.id <= last_message_id {
                            continue;
                        }
                        last_message_id = msg.id;

                        if msg.author.bot {
                            continue;
                        }

                        if let Some(expected_user) = filter_user
                            && msg.author.id.get() != expected_user
                        {
                            continue;
                        }

                        let user_reply = UserReply {
                            message_id: msg.id.to_string(),
                            channel_id: channel_id.to_string(),
                            user_id: msg.author.id.to_string(),
                            username: msg.author.name.clone(),
                            text: msg.content.clone(),
                            timestamp: msg.timestamp.unix_timestamp(),
                        };

                        let mut result_guard = result_clone.lock().await;
                        *result_guard = Some(user_reply);
                        return;
                    }
                }
            }
        });

        let got_message = if timeout_secs > 0 {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs as u64),
                poll_task,
            )
            .await
            .is_ok()
        } else {
            let _ = poll_task.await;
            true
        };

        let reply_opt = result.lock().await.take();

        if got_message {
            if let Some(reply) = reply_opt {
                let text = reply.text.clone();
                context.set_pin_value("message", json!(reply)).await?;
                context.set_pin_value("message_text", json!(text)).await?;
                context.activate_exec_pin("on_message").await?;
            } else {
                context.set_pin_value("message_text", json!("")).await?;
                context.activate_exec_pin("on_timeout").await?;
            }
        } else {
            context.set_pin_value("message_text", json!("")).await?;
            context.activate_exec_pin("on_timeout").await?;
        }

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
// Send Buttons Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendButtonsNode;

impl SendButtonsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendButtonsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_send_buttons",
            "Send Buttons",
            "Sends a message with custom buttons. Note: Button interactions require Discord Gateway/Webhooks.",
            "Discord/Interaction",
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
            "buttons",
            "Buttons",
            "Button labels (will use as custom IDs too)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Target channel (optional)",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the sent message",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;
        let content: String = context.evaluate_pin("content").await?;
        let buttons: Vec<String> = context.evaluate_pin("buttons").await?;
        let channel_override: Option<String> = context.evaluate_pin("channel_id").await.ok();

        if buttons.is_empty() {
            return Err(flow_like_types::anyhow!("At least one button required"));
        }
        if buttons.len() > 25 {
            return Err(flow_like_types::anyhow!("Maximum 25 buttons allowed"));
        }

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = match channel_override.filter(|s| !s.is_empty()) {
            Some(id) => ChannelId::new(id.parse()?),
            None => session.channel_id()?,
        };

        let mut action_rows = Vec::new();
        let mut current_row = Vec::new();

        for (idx, label) in buttons.iter().enumerate() {
            let custom_id = format!("btn_{}_{}", flow_like_types::create_id(), idx);
            let button = CreateButton::new(custom_id)
                .label(label)
                .style(ButtonStyle::Primary);
            current_row.push(button);

            if current_row.len() == 5 {
                action_rows.push(CreateActionRow::Buttons(current_row));
                current_row = Vec::new();
            }
        }

        if !current_row.is_empty() {
            action_rows.push(CreateActionRow::Buttons(current_row));
        }

        let message = CreateMessage::new()
            .content(&content)
            .components(action_rows);

        let sent = channel_id.send_message(&client.http, message).await?;

        context
            .set_pin_value("message_id", json!(sent.id.to_string()))
            .await?;
        context.activate_exec_pin("exec_out").await?;

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
// Send Select Menu Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendSelectMenuNode;

impl SendSelectMenuNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendSelectMenuNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_send_select_menu",
            "Send Select Menu",
            "Sends a message with a dropdown select menu. Note: Selections require Discord Gateway/Webhooks.",
            "Discord/Interaction",
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
            "placeholder",
            "Placeholder",
            "Placeholder text for the select menu",
            VariableType::String,
        )
        .set_default_value(Some(json!("Select an option...")));

        node.add_input_pin(
            "options",
            "Options",
            "Select menu options (label = value)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "min_values",
            "Min Values",
            "Minimum selections required",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "max_values",
            "Max Values",
            "Maximum selections allowed",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Target channel (optional)",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the sent message",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;
        let content: String = context.evaluate_pin("content").await?;
        let placeholder: String = context
            .evaluate_pin("placeholder")
            .await
            .unwrap_or_else(|_| "Select an option...".to_string());
        let options: Vec<String> = context.evaluate_pin("options").await?;
        let min_values: i64 = context.evaluate_pin("min_values").await.unwrap_or(1);
        let max_values: i64 = context.evaluate_pin("max_values").await.unwrap_or(1);
        let channel_override: Option<String> = context.evaluate_pin("channel_id").await.ok();

        if options.is_empty() {
            return Err(flow_like_types::anyhow!("At least one option required"));
        }
        if options.len() > 25 {
            return Err(flow_like_types::anyhow!("Maximum 25 options allowed"));
        }

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = match channel_override.filter(|s| !s.is_empty()) {
            Some(id) => ChannelId::new(id.parse()?),
            None => session.channel_id()?,
        };

        let custom_id = format!("select_{}", flow_like_types::create_id());

        let menu_options: Vec<CreateSelectMenuOption> = options
            .iter()
            .map(|opt| CreateSelectMenuOption::new(opt, opt))
            .collect();

        let select_menu = CreateSelectMenu::new(
            &custom_id,
            CreateSelectMenuKind::String {
                options: menu_options,
            },
        )
        .placeholder(&placeholder)
        .min_values(min_values.clamp(1, 25) as u8)
        .max_values(max_values.clamp(1, 25) as u8);

        let action_row = CreateActionRow::SelectMenu(select_menu);

        let message = CreateMessage::new()
            .content(&content)
            .components(vec![action_row]);

        let sent = channel_id.send_message(&client.http, message).await?;

        context
            .set_pin_value("message_id", json!(sent.id.to_string()))
            .await?;
        context.activate_exec_pin("exec_out").await?;

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
// Disable Components Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DisableComponentsNode;

impl DisableComponentsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DisableComponentsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_disable_components",
            "Disable Components",
            "Disables all buttons/select menus on a message",
            "Discord/Interaction",
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
            "ID of message with components",
            VariableType::String,
        );

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Channel ID (optional)",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;
        let message_id: String = context.evaluate_pin("message_id").await?;
        let channel_override: Option<String> = context.evaluate_pin("channel_id").await.ok();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = match channel_override.filter(|s| !s.is_empty()) {
            Some(id) => ChannelId::new(id.parse()?),
            None => session.channel_id()?,
        };

        let msg_id = serenity::all::MessageId::new(message_id.parse()?);

        let edit = EditMessage::new().components(vec![]);
        channel_id.edit_message(&client.http, msg_id, edit).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Discord functionality requires the 'execute' feature"
        ))
    }
}
