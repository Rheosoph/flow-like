//! Discord media operations - file attachments with FlowPath support

use super::session::DiscordSession;
#[cfg(feature = "execute")]
use super::session::get_discord_client;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use serenity::builder::{CreateAttachment, CreateMessage, EditMessage};

/// Reference to a sent message with attachment info
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SentAttachment {
    /// The message ID
    pub message_id: String,
    /// The channel ID
    pub channel_id: String,
    /// Attachment URL (if available)
    pub attachment_url: Option<String>,
    /// Attachment filename
    pub filename: String,
}

/// Helper to convert FlowPath to CreateAttachment
#[cfg(feature = "execute")]
async fn flow_path_to_attachment(
    context: &mut ExecutionContext,
    flow_path: &FlowPath,
    filename: Option<&str>,
) -> flow_like_types::Result<CreateAttachment> {
    let bytes = flow_path.get(context, false).await?;
    let name = filename
        .map(|s| s.to_string())
        .or_else(|| {
            std::path::Path::new(&flow_path.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "file".to_string());

    Ok(CreateAttachment::bytes(bytes, name))
}

// ============================================================================
// Send File Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendFileNode;

impl SendFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_send_file",
            "Send File",
            "Sends a file attachment to a Discord channel using FlowPath",
            "Discord/Media",
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

        node.add_input_pin("file", "File", "File to send", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "filename",
            "Filename",
            "Override filename (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "content",
            "Content",
            "Optional message content with the file",
            VariableType::String,
        );

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Target channel (optional, defaults to session channel)",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "attachment",
            "Attachment",
            "Information about the sent attachment",
            VariableType::Struct,
        )
        .set_schema::<SentAttachment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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
        let flow_path: FlowPath = context.evaluate_pin("file").await?;
        let filename: Option<String> = context.evaluate_pin("filename").await.ok();
        let content: Option<String> = context.evaluate_pin("content").await.ok();
        let channel_override: Option<String> = context.evaluate_pin("channel_id").await.ok();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = match channel_override.filter(|s| !s.is_empty()) {
            Some(id) => serenity::all::ChannelId::new(id.parse()?),
            None => session.channel_id()?,
        };

        let attachment = flow_path_to_attachment(context, &flow_path, filename.as_deref()).await?;
        let attachment_name = attachment.filename.clone();

        let mut message = CreateMessage::new().add_file(attachment);

        if let Some(text) = content.filter(|s| !s.is_empty()) {
            message = message.content(text);
        }

        let sent = channel_id.send_message(&client.http, message).await?;

        let attachment_url = sent.attachments.first().map(|a| a.url.clone());

        let sent_attachment = SentAttachment {
            message_id: sent.id.to_string(),
            channel_id: channel_id.to_string(),
            attachment_url,
            filename: attachment_name,
        };

        context
            .set_pin_value("attachment", json!(sent_attachment))
            .await?;
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
// Send Image Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendImageNode;

impl SendImageNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_send_image",
            "Send Image",
            "Sends an image to a Discord channel using FlowPath",
            "Discord/Media",
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

        node.add_input_pin("image", "Image", "Image file to send", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "filename",
            "Filename",
            "Override filename (optional, e.g., 'image.png')",
            VariableType::String,
        );

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption/message with the image",
            VariableType::String,
        );

        node.add_input_pin(
            "spoiler",
            "Spoiler",
            "Mark image as spoiler",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "Target channel (optional)",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "attachment",
            "Attachment",
            "Information about the sent image",
            VariableType::Struct,
        )
        .set_schema::<SentAttachment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;
        let flow_path: FlowPath = context.evaluate_pin("image").await?;
        let filename: Option<String> = context.evaluate_pin("filename").await.ok();
        let caption: Option<String> = context.evaluate_pin("caption").await.ok();
        let spoiler: bool = context.evaluate_pin("spoiler").await.unwrap_or(false);
        let channel_override: Option<String> = context.evaluate_pin("channel_id").await.ok();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = match channel_override.filter(|s| !s.is_empty()) {
            Some(id) => serenity::all::ChannelId::new(id.parse()?),
            None => session.channel_id()?,
        };

        let bytes = flow_path.get(context, false).await?;
        let name = filename
            .or_else(|| {
                std::path::Path::new(&flow_path.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "image.png".to_string());

        let final_name = if spoiler && !name.starts_with("SPOILER_") {
            format!("SPOILER_{}", name)
        } else {
            name.clone()
        };

        let attachment = CreateAttachment::bytes(bytes, &final_name);

        let mut message = CreateMessage::new().add_file(attachment);

        if let Some(text) = caption.filter(|s| !s.is_empty()) {
            message = message.content(text);
        }

        let sent = channel_id.send_message(&client.http, message).await?;

        let attachment_url = sent.attachments.first().map(|a| a.url.clone());

        let sent_attachment = SentAttachment {
            message_id: sent.id.to_string(),
            channel_id: channel_id.to_string(),
            attachment_url,
            filename: final_name,
        };

        context
            .set_pin_value("attachment", json!(sent_attachment))
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
// Send Multiple Files Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendMultipleFilesNode;

impl SendMultipleFilesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendMultipleFilesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_send_multiple_files",
            "Send Multiple Files",
            "Sends multiple files in a single message (up to 10 files)",
            "Discord/Media",
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
            "files",
            "Files",
            "Array of files to send (max 10)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "content",
            "Content",
            "Optional message content",
            VariableType::String,
        );

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

        node.add_output_pin(
            "file_count",
            "File Count",
            "Number of files sent",
            VariableType::Integer,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;
        let files: Vec<FlowPath> = context.evaluate_pin("files").await?;
        let content: Option<String> = context.evaluate_pin("content").await.ok();
        let channel_override: Option<String> = context.evaluate_pin("channel_id").await.ok();

        if files.is_empty() {
            return Err(flow_like_types::anyhow!("No files provided"));
        }

        if files.len() > 10 {
            return Err(flow_like_types::anyhow!(
                "Discord supports max 10 files per message"
            ));
        }

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = match channel_override.filter(|s| !s.is_empty()) {
            Some(id) => serenity::all::ChannelId::new(id.parse()?),
            None => session.channel_id()?,
        };

        let mut attachments = Vec::new();
        for flow_path in &files {
            let attachment = flow_path_to_attachment(context, flow_path, None).await?;
            attachments.push(attachment);
        }

        let mut message = CreateMessage::new();
        for attachment in attachments {
            message = message.add_file(attachment);
        }

        if let Some(text) = content.filter(|s| !s.is_empty()) {
            message = message.content(text);
        }

        let sent = channel_id.send_message(&client.http, message).await?;

        context
            .set_pin_value("message_id", json!(sent.id.to_string()))
            .await?;
        context
            .set_pin_value("file_count", json!(files.len()))
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
// Edit Message With File Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditMessageWithFileNode;

impl EditMessageWithFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditMessageWithFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_edit_message_with_file",
            "Edit Message With File",
            "Edits a message and adds/replaces file attachment",
            "Discord/Media",
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
            "ID of message to edit",
            VariableType::String,
        );

        node.add_input_pin("file", "File", "New file attachment", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "content",
            "Content",
            "New message content (optional)",
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
        let flow_path: FlowPath = context.evaluate_pin("file").await?;
        let content: Option<String> = context.evaluate_pin("content").await.ok();
        let channel_override: Option<String> = context.evaluate_pin("channel_id").await.ok();

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = match channel_override.filter(|s| !s.is_empty()) {
            Some(id) => serenity::all::ChannelId::new(id.parse()?),
            None => session.channel_id()?,
        };

        let msg_id = serenity::all::MessageId::new(message_id.parse()?);

        let attachment = flow_path_to_attachment(context, &flow_path, None).await?;

        let mut edit = EditMessage::new().new_attachment(attachment);

        if let Some(text) = content {
            edit = edit.content(text);
        }

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
