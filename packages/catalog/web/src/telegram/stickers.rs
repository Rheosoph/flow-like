//! Telegram sticker operations

use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    Value, async_trait,
    json::{from_str, json},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::{
    CustomEmojiId, FileId, InputFile, MaskPosition as TgMaskPosition, ReplyParameters, StickerType,
    UserId,
};

use super::message::SentMessage;

/// Sticker information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StickerInfo {
    pub file_id: String,
    pub file_unique_id: String,
    pub sticker_type: String,
    pub width: i64,
    pub height: i64,
    pub is_animated: bool,
    pub is_video: bool,
    pub emoji: Option<String>,
    pub set_name: Option<String>,
    pub custom_emoji_id: Option<String>,
}

/// Sticker set information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StickerSetInfo {
    pub name: String,
    pub title: String,
    pub sticker_type: String,
    pub stickers: Vec<StickerInfo>,
}

/// Mask position information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MaskPositionInfo {
    pub point: String,
    pub x_shift: f64,
    pub y_shift: f64,
    pub scale: f64,
}

#[cfg(feature = "execute")]
impl From<&MaskPositionInfo> for TgMaskPosition {
    fn from(info: &MaskPositionInfo) -> Self {
        let point = match info.point.as_str() {
            "forehead" => teloxide::types::MaskPoint::Forehead,
            "eyes" => teloxide::types::MaskPoint::Eyes,
            "mouth" => teloxide::types::MaskPoint::Mouth,
            "chin" => teloxide::types::MaskPoint::Chin,
            _ => teloxide::types::MaskPoint::Forehead,
        };
        TgMaskPosition {
            point,
            x_shift: info.x_shift,
            y_shift: info.y_shift,
            scale: info.scale,
        }
    }
}

#[cfg(feature = "execute")]
fn sticker_type_to_string(st: &StickerType) -> String {
    match st {
        StickerType::Regular => "regular".to_string(),
        StickerType::Mask => "mask".to_string(),
        StickerType::CustomEmoji => "custom_emoji".to_string(),
    }
}

#[cfg(feature = "execute")]
fn convert_sticker(s: &teloxide::types::Sticker) -> StickerInfo {
    StickerInfo {
        file_id: s.file.id.to_string(),
        file_unique_id: s.file.unique_id.to_string(),
        sticker_type: sticker_type_to_string(&s.kind.type_()),
        width: s.width as i64,
        height: s.height as i64,
        is_animated: s.is_animated(),
        is_video: s.is_video(),
        emoji: s.emoji.clone(),
        set_name: s.set_name.clone(),
        custom_emoji_id: s.custom_emoji_id().map(|id| id.to_string()),
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
            "Telegram/Stickers",
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
            "Sticker file_id or URL",
            VariableType::String,
        );

        node.add_input_pin(
            "emoji",
            "Emoji",
            "Emoji associated with the sticker (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "disable_notification",
            "Disable Notification",
            "Send silently without notification",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "protect_content",
            "Protect Content",
            "Protect the message from forwarding and saving",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "reply_to_message_id",
            "Reply To Message ID",
            "Message ID to reply to (optional)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

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
        let sticker: String = context.evaluate_pin("sticker").await?;
        let emoji: String = context
            .evaluate_pin::<String>("emoji")
            .await
            .unwrap_or_default();
        let disable_notification: bool = context
            .evaluate_pin::<bool>("disable_notification")
            .await
            .unwrap_or(false);
        let protect_content: bool = context
            .evaluate_pin::<bool>("protect_content")
            .await
            .unwrap_or(false);
        let reply_to: i64 = context
            .evaluate_pin::<i64>("reply_to_message_id")
            .await
            .unwrap_or(0);

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let input_sticker = if sticker.starts_with("http://") || sticker.starts_with("https://") {
            let url = sticker
                .parse()
                .map_err(|_| flow_like_types::anyhow!("Invalid URL"))?;
            InputFile::url(url)
        } else {
            InputFile::file_id(FileId(sticker))
        };

        let mut request = bot.bot.send_sticker(chat_id, input_sticker);

        if !emoji.is_empty() {
            request = request.emoji(emoji);
        }

        request = request.disable_notification(disable_notification);
        request = request.protect_content(protect_content);

        if reply_to > 0 {
            request = request.reply_parameters(ReplyParameters::new(teloxide::types::MessageId(
                reply_to as i32,
            )));
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
// Get Sticker Set Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetStickerSetNode;

impl GetStickerSetNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetStickerSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_sticker_set",
            "Get Sticker Set",
            "Gets a sticker set by name",
            "Telegram/Stickers",
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
            "Name of the sticker set",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sticker set is retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sticker_set",
            "Sticker Set",
            "Information about the sticker set",
            VariableType::Struct,
        )
        .set_schema::<StickerSetInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let name: String = context.evaluate_pin("name").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let set = bot.bot.get_sticker_set(&name).await?;

        let sticker_set_info = StickerSetInfo {
            name: set.name.clone(),
            title: set.title.clone(),
            sticker_type: sticker_type_to_string(&set.kind),
            stickers: set.stickers.iter().map(convert_sticker).collect(),
        };

        context
            .set_pin_value("sticker_set", json!(sticker_set_info))
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
// Get Custom Emoji Stickers Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetCustomEmojiStickersNode;

impl GetCustomEmojiStickersNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetCustomEmojiStickersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_custom_emoji_stickers",
            "Get Custom Emoji Stickers",
            "Gets information about custom emoji stickers by their identifiers",
            "Telegram/Stickers",
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
            "custom_emoji_ids",
            "Custom Emoji IDs",
            "Array of custom emoji identifiers (max 200)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after stickers are retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "stickers",
            "Stickers",
            "Array of sticker information",
            VariableType::Struct,
        )
        .set_schema::<Vec<StickerInfo>>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let custom_emoji_ids: Vec<String> = context.evaluate_pin("custom_emoji_ids").await?;

        if custom_emoji_ids.len() > 200 {
            return Err(flow_like_types::anyhow!(
                "Maximum 200 custom emoji IDs allowed"
            ));
        }

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let custom_emoji_ids: Vec<CustomEmojiId> =
            custom_emoji_ids.into_iter().map(CustomEmojiId).collect();
        let stickers = bot.bot.get_custom_emoji_stickers(custom_emoji_ids).await?;

        let sticker_infos: Vec<StickerInfo> = stickers.iter().map(convert_sticker).collect();

        context
            .set_pin_value("stickers", json!(sticker_infos))
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
// Set Sticker Set Thumbnail Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetStickerSetThumbnailNode;

impl SetStickerSetThumbnailNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetStickerSetThumbnailNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_sticker_set_thumbnail",
            "Set Sticker Set Thumbnail",
            "Sets the thumbnail of a sticker set",
            "Telegram/Stickers",
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

        node.add_input_pin("name", "Name", "Sticker set name", VariableType::String);

        node.add_input_pin(
            "user_id",
            "User ID",
            "User identifier of the sticker set owner",
            VariableType::Integer,
        );

        node.add_input_pin(
            "thumbnail",
            "Thumbnail",
            "Thumbnail file_id or URL (optional, pass empty to remove)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "format",
            "Format",
            "Format of the thumbnail (static, animated, video)",
            VariableType::String,
        )
        .set_default_value(Some(json!("static")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after thumbnail is set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let name: String = context.evaluate_pin("name").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let thumbnail: String = context
            .evaluate_pin::<String>("thumbnail")
            .await
            .unwrap_or_default();
        let format: String = context
            .evaluate_pin::<String>("format")
            .await
            .unwrap_or_else(|_| "static".to_string());

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let sticker_format = match format.as_str() {
            "animated" => teloxide::types::StickerFormat::Animated,
            "video" => teloxide::types::StickerFormat::Video,
            _ => teloxide::types::StickerFormat::Static,
        };

        let result = if thumbnail.is_empty() {
            bot.bot
                .set_sticker_set_thumbnail(&name, UserId(user_id as u64), sticker_format)
                .await
        } else {
            let input_thumbnail =
                if thumbnail.starts_with("http://") || thumbnail.starts_with("https://") {
                    let url = thumbnail
                        .parse()
                        .map_err(|_| flow_like_types::anyhow!("Invalid URL"))?;
                    InputFile::url(url)
                } else {
                    InputFile::file_id(FileId(thumbnail))
                };

            bot.bot
                .set_sticker_set_thumbnail(&name, UserId(user_id as u64), sticker_format)
                .thumbnail(input_thumbnail)
                .await
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
// Set Sticker Emoji List Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetStickerEmojiListNode;

impl SetStickerEmojiListNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetStickerEmojiListNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_sticker_emoji_list",
            "Set Sticker Emoji List",
            "Changes the list of emoji associated with a sticker",
            "Telegram/Stickers",
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
            "File identifier of the sticker",
            VariableType::String,
        );

        node.add_input_pin(
            "emoji_list",
            "Emoji List",
            "Array of 1-20 emoji associated with the sticker",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after emoji list is set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let sticker: String = context.evaluate_pin("sticker").await?;
        let emoji_list: Vec<String> = context.evaluate_pin("emoji_list").await?;

        if emoji_list.is_empty() || emoji_list.len() > 20 {
            return Err(flow_like_types::anyhow!("Emoji list must have 1-20 items"));
        }

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot.bot.set_sticker_emoji_list(&sticker, emoji_list).await;

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
// Set Sticker Keywords Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetStickerKeywordsNode;

impl SetStickerKeywordsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetStickerKeywordsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_sticker_keywords",
            "Set Sticker Keywords",
            "Changes search keywords associated with a sticker",
            "Telegram/Stickers",
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
            "File identifier of the sticker",
            VariableType::String,
        );

        node.add_input_pin(
            "keywords",
            "Keywords",
            "Array of 0-20 search keywords (optional, empty to clear)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after keywords are set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let sticker: String = context.evaluate_pin("sticker").await?;
        let keywords: Vec<String> = context
            .evaluate_pin::<Vec<String>>("keywords")
            .await
            .unwrap_or_default();

        if keywords.len() > 20 {
            return Err(flow_like_types::anyhow!(
                "Keywords must have at most 20 items"
            ));
        }

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot
            .bot
            .set_sticker_keywords(&sticker)
            .keywords(keywords)
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

// ============================================================================
// Set Sticker Mask Position Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetStickerMaskPositionNode;

impl SetStickerMaskPositionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetStickerMaskPositionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_sticker_mask_position",
            "Set Sticker Mask Position",
            "Changes the mask position of a mask sticker",
            "Telegram/Stickers",
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
            "File identifier of the sticker",
            VariableType::String,
        );

        node.add_input_pin(
            "mask_position",
            "Mask Position",
            "Mask position (optional, omit to remove)",
            VariableType::Struct,
        )
        .set_schema::<MaskPositionInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after mask position is set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let sticker: String = context.evaluate_pin("sticker").await?;
        let mask_position: Option<MaskPositionInfo> = context
            .evaluate_pin::<MaskPositionInfo>("mask_position")
            .await
            .ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = if let Some(pos) = mask_position {
            let tg_pos: TgMaskPosition = (&pos).into();
            bot.bot
                .set_sticker_mask_position(&sticker)
                .mask_position(tg_pos)
                .await
        } else {
            bot.bot.set_sticker_mask_position(&sticker).await
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
// Delete Sticker From Set Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteStickerFromSetNode;

impl DeleteStickerFromSetNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteStickerFromSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_delete_sticker_from_set",
            "Delete Sticker From Set",
            "Deletes a sticker from a set created by the bot",
            "Telegram/Stickers",
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
            "File identifier of the sticker to delete",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sticker is deleted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the deletion was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let sticker: String = context.evaluate_pin("sticker").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot.bot.delete_sticker_from_set(&sticker).await;

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
// Set Sticker Set Title Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetStickerSetTitleNode;

impl SetStickerSetTitleNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetStickerSetTitleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_sticker_set_title",
            "Set Sticker Set Title",
            "Sets the title of a sticker set",
            "Telegram/Stickers",
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

        node.add_input_pin("name", "Name", "Sticker set name", VariableType::String);

        node.add_input_pin(
            "title",
            "Title",
            "New sticker set title (1-64 characters)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after title is set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let name: String = context.evaluate_pin("name").await?;
        let title: String = context.evaluate_pin("title").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot.bot.set_sticker_set_title(&name, &title).await;

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
// Delete Sticker Set Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteStickerSetNode;

impl DeleteStickerSetNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteStickerSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_delete_sticker_set",
            "Delete Sticker Set",
            "Deletes a sticker set that was created by the bot",
            "Telegram/Stickers",
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

        node.add_input_pin("name", "Name", "Sticker set name", VariableType::String);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sticker set is deleted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the deletion was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let name: String = context.evaluate_pin("name").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot.bot.delete_sticker_set(&name).await;

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
// Upload Sticker File Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct UploadStickerFileNode;

impl UploadStickerFileNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UploadStickerFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_upload_sticker_file",
            "Upload Sticker File",
            "Uploads a sticker file for later use in sticker sets",
            "Telegram/Stickers",
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
            "User identifier of sticker file owner",
            VariableType::Integer,
        );

        node.add_input_pin(
            "sticker",
            "Sticker",
            "Sticker file path or URL",
            VariableType::String,
        );

        node.add_input_pin(
            "sticker_format",
            "Sticker Format",
            "Format of the sticker (static, animated, video)",
            VariableType::String,
        )
        .set_default_value(Some(json!("static")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sticker file is uploaded",
            VariableType::Execution,
        );

        node.add_output_pin(
            "file_id",
            "File ID",
            "The uploaded sticker file ID",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let sticker: String = context.evaluate_pin("sticker").await?;
        let sticker_format: String = context
            .evaluate_pin::<String>("sticker_format")
            .await
            .unwrap_or_else(|_| "static".to_string());

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let format = match sticker_format.as_str() {
            "animated" => teloxide::types::StickerFormat::Animated,
            "video" => teloxide::types::StickerFormat::Video,
            _ => teloxide::types::StickerFormat::Static,
        };

        let input_sticker = if sticker.starts_with("http://") || sticker.starts_with("https://") {
            let url = sticker
                .parse()
                .map_err(|_| flow_like_types::anyhow!("Invalid URL"))?;
            InputFile::url(url)
        } else {
            InputFile::file(sticker)
        };

        let file = bot
            .bot
            .upload_sticker_file(UserId(user_id as u64), input_sticker, format)
            .await?;

        context.set_pin_value("file_id", json!(file.id)).await?;

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
// Create New Sticker Set Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CreateNewStickerSetNode;

impl CreateNewStickerSetNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CreateNewStickerSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_create_new_sticker_set",
            "Create New Sticker Set",
            "Creates a new sticker set owned by a user",
            "Telegram/Stickers",
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
            "User identifier of the created sticker set owner",
            VariableType::Integer,
        );

        node.add_input_pin(
            "name",
            "Name",
            "Short name of sticker set (e.g., 'mypack_by_bot')",
            VariableType::String,
        );

        node.add_input_pin(
            "title",
            "Title",
            "Sticker set title (1-64 characters)",
            VariableType::String,
        );

        node.add_input_pin(
            "stickers",
            "Stickers",
            "JSON array of InputSticker objects",
            VariableType::String,
        );

        node.add_input_pin(
            "sticker_type",
            "Sticker Type",
            "Type of stickers (regular, mask, custom_emoji)",
            VariableType::String,
        )
        .set_default_value(Some(json!("regular")));

        node.add_input_pin(
            "needs_repainting",
            "Needs Repainting",
            "Whether stickers should be repainted to match emoji color",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sticker set is created",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the sticker set was created successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let name: String = context.evaluate_pin("name").await?;
        let title: String = context.evaluate_pin("title").await?;
        let stickers_json: String = context.evaluate_pin("stickers").await?;
        let sticker_type: String = context
            .evaluate_pin::<String>("sticker_type")
            .await
            .unwrap_or_else(|_| "regular".to_string());
        let needs_repainting: bool = context
            .evaluate_pin::<bool>("needs_repainting")
            .await
            .unwrap_or(false);

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let stickers_data: Vec<Value> = from_str(&stickers_json)?;

        let mut input_stickers = Vec::new();
        for sticker_val in stickers_data {
            let file_id = sticker_val["sticker"]
                .as_str()
                .ok_or_else(|| flow_like_types::anyhow!("Missing sticker field"))?;
            let emoji_list: Vec<String> = sticker_val["emoji_list"]
                .as_array()
                .map(|arr: &Vec<Value>| {
                    arr.iter()
                        .filter_map(|v: &Value| v.as_str().map(|s: &str| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            let format_str = sticker_val["format"].as_str().unwrap_or("static");
            let format = match format_str {
                "animated" => teloxide::types::StickerFormat::Animated,
                "video" => teloxide::types::StickerFormat::Video,
                _ => teloxide::types::StickerFormat::Static,
            };

            let input_file = InputFile::file_id(FileId(file_id.to_string()));
            let input_sticker = teloxide::types::InputSticker {
                sticker: input_file,
                format,
                emoji_list,
                mask_position: None,
                keywords: Vec::new(),
            };
            input_stickers.push(input_sticker);
        }

        let st_type = match sticker_type.as_str() {
            "mask" => StickerType::Mask,
            "custom_emoji" => StickerType::CustomEmoji,
            _ => StickerType::Regular,
        };

        let mut request =
            bot.bot
                .create_new_sticker_set(UserId(user_id as u64), &name, &title, input_stickers);

        request = request.sticker_type(st_type);
        request = request.needs_repainting(needs_repainting);

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

// ============================================================================
// Add Sticker To Set Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct AddStickerToSetNode;

impl AddStickerToSetNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AddStickerToSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_add_sticker_to_set",
            "Add Sticker To Set",
            "Adds a new sticker to a set created by the bot",
            "Telegram/Stickers",
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
            "User identifier of the sticker set owner",
            VariableType::Integer,
        );

        node.add_input_pin("name", "Name", "Sticker set name", VariableType::String);

        node.add_input_pin(
            "sticker",
            "Sticker",
            "JSON object with sticker, emoji_list, format fields",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sticker is added",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the sticker was added successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let name: String = context.evaluate_pin("name").await?;
        let sticker_json: String = context.evaluate_pin("sticker").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let sticker_val: Value = from_str(&sticker_json)?;

        let file_id = sticker_val["sticker"]
            .as_str()
            .ok_or_else(|| flow_like_types::anyhow!("Missing sticker field"))?;
        let emoji_list: Vec<String> = sticker_val["emoji_list"]
            .as_array()
            .map(|arr: &Vec<Value>| {
                arr.iter()
                    .filter_map(|v: &Value| v.as_str().map(|s: &str| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let format_str = sticker_val["format"].as_str().unwrap_or("static");
        let format = match format_str {
            "animated" => teloxide::types::StickerFormat::Animated,
            "video" => teloxide::types::StickerFormat::Video,
            _ => teloxide::types::StickerFormat::Static,
        };

        let input_file = InputFile::file_id(FileId(file_id.to_string()));
        let input_sticker = teloxide::types::InputSticker {
            sticker: input_file,
            format,
            emoji_list,
            mask_position: None,
            keywords: Vec::new(),
        };

        let result = bot
            .bot
            .add_sticker_to_set(UserId(user_id as u64), &name, input_sticker)
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

// ============================================================================
// Set Sticker Position In Set Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetStickerPositionInSetNode;

impl SetStickerPositionInSetNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetStickerPositionInSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_sticker_position_in_set",
            "Set Sticker Position In Set",
            "Moves a sticker in a set to a specific position",
            "Telegram/Stickers",
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
            "File identifier of the sticker",
            VariableType::String,
        );

        node.add_input_pin(
            "position",
            "Position",
            "New sticker position in the set (0-based)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after position is set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let sticker: String = context.evaluate_pin("sticker").await?;
        let position: i64 = context.evaluate_pin::<i64>("position").await.unwrap_or(0);

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot
            .bot
            .set_sticker_position_in_set(&sticker, position as u32)
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

// ============================================================================
// Replace Sticker In Set Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ReplaceStickerInSetNode;

impl ReplaceStickerInSetNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ReplaceStickerInSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_replace_sticker_in_set",
            "Replace Sticker In Set",
            "Replaces an existing sticker in a set with a new one",
            "Telegram/Stickers",
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
            "User identifier of the sticker set owner",
            VariableType::Integer,
        );

        node.add_input_pin("name", "Name", "Sticker set name", VariableType::String);

        node.add_input_pin(
            "old_sticker",
            "Old Sticker",
            "File identifier of the sticker to replace",
            VariableType::String,
        );

        node.add_input_pin(
            "sticker",
            "New Sticker",
            "JSON object with sticker, emoji_list, format fields",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after sticker is replaced",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the sticker was replaced successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let name: String = context.evaluate_pin("name").await?;
        let old_sticker: String = context.evaluate_pin("old_sticker").await?;
        let sticker_json: String = context.evaluate_pin("sticker").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let sticker_val: Value = from_str(&sticker_json)?;

        let file_id = sticker_val["sticker"]
            .as_str()
            .ok_or_else(|| flow_like_types::anyhow!("Missing sticker field"))?;
        let emoji_list: Vec<String> = sticker_val["emoji_list"]
            .as_array()
            .map(|arr: &Vec<Value>| {
                arr.iter()
                    .filter_map(|v: &Value| v.as_str().map(|s: &str| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let format_str = sticker_val["format"].as_str().unwrap_or("static");
        let format = match format_str {
            "animated" => teloxide::types::StickerFormat::Animated,
            "video" => teloxide::types::StickerFormat::Video,
            _ => teloxide::types::StickerFormat::Static,
        };

        let input_file = InputFile::file_id(FileId(file_id.to_string()));
        let input_sticker = teloxide::types::InputSticker {
            sticker: input_file,
            format,
            emoji_list,
            mask_position: None,
            keywords: Vec::new(),
        };

        let result = bot
            .bot
            .replace_sticker_in_set(UserId(user_id as u64), &name, &old_sticker, input_sticker)
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

// ============================================================================
// Set Custom Emoji Sticker Set Thumbnail Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SetCustomEmojiStickerSetThumbnailNode;

impl SetCustomEmojiStickerSetThumbnailNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetCustomEmojiStickerSetThumbnailNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_set_custom_emoji_sticker_set_thumbnail",
            "Set Custom Emoji Sticker Set Thumbnail",
            "Sets the thumbnail of a custom emoji sticker set",
            "Telegram/Stickers",
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

        node.add_input_pin("name", "Name", "Sticker set name", VariableType::String);

        node.add_input_pin(
            "custom_emoji_id",
            "Custom Emoji ID",
            "Custom emoji identifier for thumbnail (optional, omit to use first sticker)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after thumbnail is set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the operation was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let name: String = context.evaluate_pin("name").await?;
        let custom_emoji_id: String = context
            .evaluate_pin::<String>("custom_emoji_id")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = if custom_emoji_id.is_empty() {
            bot.bot.set_custom_emoji_sticker_set_thumbnail(&name).await
        } else {
            bot.bot
                .set_custom_emoji_sticker_set_thumbnail(&name)
                .custom_emoji_id(CustomEmojiId(custom_emoji_id))
                .await
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
