//! Telegram media operations (photos, documents, videos, etc.)

use super::message::SentMessage;
use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::{FileId, InputFile, ParseMode, ReplyParameters};

/// Helper to convert FlowPath to InputFile by reading bytes
#[cfg(feature = "execute")]
async fn flow_path_to_input_file(
    context: &mut ExecutionContext,
    flow_path: &FlowPath,
    filename: Option<&str>,
) -> flow_like_types::Result<InputFile> {
    let bytes = flow_path.get(context, false).await?;
    let name = filename
        .map(|s| s.to_string())
        .or_else(|| {
            std::path::Path::new(&flow_path.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "file".to_string());

    Ok(InputFile::memory(bytes).file_name(name))
}

// ============================================================================
// Send Photo Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendPhotoNode;

impl SendPhotoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendPhotoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_photo",
            "Send Photo",
            "Sends a photo to the Telegram chat",
            "Telegram/Media",
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
            "photo_url",
            "Photo URL",
            "URL of the photo to send",
            VariableType::String,
        );

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the photo (supports Markdown)",
            VariableType::String,
        );

        node.add_input_pin(
            "reply_to",
            "Reply To",
            "Optional message ID to reply to",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after photo is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let photo_url: String = context.evaluate_pin("photo_url").await?;

        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();
        let reply_to: Option<String> = context.evaluate_pin::<String>("reply_to").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let url = photo_url
            .parse()
            .map_err(|_| flow_like_types::anyhow!("Invalid URL"))?;
        let photo = InputFile::url(url);

        let mut request = bot.bot.send_photo(chat_id, photo);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }

        if let Some(reply_id) = reply_to
            && let Ok(msg_id) = reply_id.parse::<i32>()
        {
            request =
                request.reply_parameters(ReplyParameters::new(teloxide::types::MessageId(msg_id)));
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
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
// Send Document Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendDocumentNode;

impl SendDocumentNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendDocumentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_document",
            "Send Document",
            "Sends a document/file to the Telegram chat",
            "Telegram/Media",
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
            "document_url",
            "Document URL",
            "URL of the document to send",
            VariableType::String,
        );

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the document",
            VariableType::String,
        );

        node.add_input_pin(
            "filename",
            "Filename",
            "Optional custom filename",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after document is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let document_url: String = context.evaluate_pin("document_url").await?;

        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let url = document_url
            .parse()
            .map_err(|_| flow_like_types::anyhow!("Invalid URL"))?;
        let document = InputFile::url(url);

        let mut request = bot.bot.send_document(chat_id, document);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
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
// Send Video Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendVideoNode;

impl SendVideoNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendVideoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_video",
            "Send Video",
            "Sends a video to the Telegram chat",
            "Telegram/Media",
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
            "video_url",
            "Video URL",
            "URL of the video to send",
            VariableType::String,
        );

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the video",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after video is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let video_url: String = context.evaluate_pin("video_url").await?;

        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let url = video_url
            .parse()
            .map_err(|_| flow_like_types::anyhow!("Invalid URL"))?;
        let video = InputFile::url(url);

        let mut request = bot.bot.send_video(chat_id, video);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
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
// Send Audio Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendAudioNode;

impl SendAudioNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendAudioNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_audio",
            "Send Audio",
            "Sends an audio file to the Telegram chat",
            "Telegram/Media",
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
            "audio_url",
            "Audio URL",
            "URL of the audio file to send",
            VariableType::String,
        );

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the audio",
            VariableType::String,
        );

        node.add_input_pin(
            "title",
            "Title",
            "Optional track title",
            VariableType::String,
        );

        node.add_input_pin(
            "performer",
            "Performer",
            "Optional performer/artist name",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after audio is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let audio_url: String = context.evaluate_pin("audio_url").await?;

        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();
        let title: Option<String> = context.evaluate_pin::<String>("title").await.ok();
        let performer: Option<String> = context.evaluate_pin::<String>("performer").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let url = audio_url
            .parse()
            .map_err(|_| flow_like_types::anyhow!("Invalid URL"))?;
        let audio = InputFile::url(url);

        let mut request = bot.bot.send_audio(chat_id, audio);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }
        if let Some(t) = title {
            request = request.title(t);
        }
        if let Some(p) = performer {
            request = request.performer(p);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
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
// Send Voice Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendVoiceNode;

impl SendVoiceNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendVoiceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_voice",
            "Send Voice Message",
            "Sends a voice message (OGG format) to the Telegram chat",
            "Telegram/Media",
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
            "voice_url",
            "Voice URL",
            "URL of the voice file to send (OGG format with OPUS)",
            VariableType::String,
        );

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the voice message",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after voice message is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let voice_url: String = context.evaluate_pin("voice_url").await?;

        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let url = voice_url
            .parse()
            .map_err(|_| flow_like_types::anyhow!("Invalid URL"))?;
        let voice = InputFile::url(url);

        let mut request = bot.bot.send_voice(chat_id, voice);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
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
// Send Sticker Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendStickerNode;

impl SendStickerNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendStickerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_sticker",
            "Send Sticker",
            "Sends a sticker to the Telegram chat",
            "Telegram/Media",
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
            "sticker",
            "Sticker",
            "Sticker file_id, URL, or InputFile",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sticker is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let sticker_str: String = context.evaluate_pin("sticker").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let sticker = if sticker_str.starts_with("http") {
            let url = sticker_str
                .parse()
                .map_err(|_| flow_like_types::anyhow!("Invalid URL"))?;
            InputFile::url(url)
        } else {
            InputFile::file_id(FileId(sticker_str))
        };

        let sent = bot.bot.send_sticker(chat_id, sticker).await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
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
// Send Animation Node (GIF)
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendAnimationNode;

impl SendAnimationNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendAnimationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_animation",
            "Send Animation",
            "Sends an animation (GIF) to the Telegram chat",
            "Telegram/Media",
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
            "animation_url",
            "Animation URL",
            "URL of the animation (GIF/MP4) to send",
            VariableType::String,
        );

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the animation",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after animation is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let animation_url: String = context.evaluate_pin("animation_url").await?;

        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let url = animation_url
            .parse()
            .map_err(|_| flow_like_types::anyhow!("Invalid URL"))?;
        let animation = InputFile::url(url);

        let mut request = bot.bot.send_animation(chat_id, animation);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
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
// FlowPath-based Media Nodes (for files from storage)
// ============================================================================

// ============================================================================
// Send Photo From File Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendPhotoFromFileNode;

impl SendPhotoFromFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendPhotoFromFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_photo_file",
            "Send Photo (File)",
            "Sends a photo from FlowPath storage to the Telegram chat",
            "Telegram/Media",
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
            "file",
            "File",
            "Photo file from storage",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the photo",
            VariableType::String,
        );

        node.add_input_pin(
            "reply_to",
            "Reply To",
            "Optional message ID to reply to",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after photo is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();
        let reply_to: Option<String> = context.evaluate_pin::<String>("reply_to").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let photo = flow_path_to_input_file(context, &file, None).await?;
        let mut request = bot.bot.send_photo(chat_id, photo);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }

        if let Some(reply_id) = reply_to
            && let Ok(msg_id) = reply_id.parse::<i32>()
        {
            request =
                request.reply_parameters(ReplyParameters::new(teloxide::types::MessageId(msg_id)));
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;
        context.activate_exec_pin("exec_out").await?;

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
// Send Document From File Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendDocumentFromFileNode;

impl SendDocumentFromFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendDocumentFromFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_document_file",
            "Send Document (File)",
            "Sends a document from FlowPath storage to the Telegram chat",
            "Telegram/Media",
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
            "file",
            "File",
            "Document file from storage",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the document",
            VariableType::String,
        );

        node.add_input_pin(
            "filename",
            "Filename",
            "Optional custom filename (overrides path filename)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after document is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();
        let filename: Option<String> = context.evaluate_pin::<String>("filename").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let document = flow_path_to_input_file(context, &file, filename.as_deref()).await?;
        let mut request = bot.bot.send_document(chat_id, document);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;
        context.activate_exec_pin("exec_out").await?;

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
// Send Video From File Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendVideoFromFileNode;

impl SendVideoFromFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendVideoFromFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_video_file",
            "Send Video (File)",
            "Sends a video from FlowPath storage to the Telegram chat",
            "Telegram/Media",
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
            "file",
            "File",
            "Video file from storage",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the video",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after video is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let video = flow_path_to_input_file(context, &file, None).await?;
        let mut request = bot.bot.send_video(chat_id, video);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;
        context.activate_exec_pin("exec_out").await?;

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
// Send Audio From File Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendAudioFromFileNode;

impl SendAudioFromFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendAudioFromFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_audio_file",
            "Send Audio (File)",
            "Sends an audio file from FlowPath storage to the Telegram chat",
            "Telegram/Media",
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
            "file",
            "File",
            "Audio file from storage",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the audio",
            VariableType::String,
        );

        node.add_input_pin(
            "title",
            "Title",
            "Optional track title",
            VariableType::String,
        );

        node.add_input_pin(
            "performer",
            "Performer",
            "Optional performer/artist name",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after audio is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();
        let title: Option<String> = context.evaluate_pin::<String>("title").await.ok();
        let performer: Option<String> = context.evaluate_pin::<String>("performer").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let audio = flow_path_to_input_file(context, &file, None).await?;
        let mut request = bot.bot.send_audio(chat_id, audio);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }
        if let Some(t) = title {
            request = request.title(t);
        }
        if let Some(p) = performer {
            request = request.performer(p);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;
        context.activate_exec_pin("exec_out").await?;

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
// Send Voice From File Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendVoiceFromFileNode;

impl SendVoiceFromFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendVoiceFromFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_voice_file",
            "Send Voice (File)",
            "Sends a voice message from FlowPath storage (OGG/OPUS format)",
            "Telegram/Media",
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
            "file",
            "File",
            "Voice file from storage (OGG format with OPUS)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the voice message",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after voice is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let voice = flow_path_to_input_file(context, &file, None).await?;
        let mut request = bot.bot.send_voice(chat_id, voice);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;
        context.activate_exec_pin("exec_out").await?;

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
// Send Animation From File Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendAnimationFromFileNode;

impl SendAnimationFromFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendAnimationFromFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_animation_file",
            "Send Animation (File)",
            "Sends an animation (GIF/MP4) from FlowPath storage to the Telegram chat",
            "Telegram/Media",
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
            "file",
            "File",
            "Animation file from storage (GIF/MP4)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the animation",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after animation is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let caption: Option<String> = context.evaluate_pin::<String>("caption").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let animation = flow_path_to_input_file(context, &file, None).await?;
        let mut request = bot.bot.send_animation(chat_id, animation);

        if let Some(cap) = caption {
            request = request.caption(cap).parse_mode(ParseMode::MarkdownV2);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}
