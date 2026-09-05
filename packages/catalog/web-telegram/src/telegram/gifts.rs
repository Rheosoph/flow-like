//! Telegram gift operations
//!
//! Note: These operations require Telegram Bot API 8.0+ which is not fully supported
//! by the current teloxide version. Implementations use placeholder logic.

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

/// Gift information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GiftInfo {
    pub id: String,
    pub sticker_file_id: String,
    pub star_count: i64,
    pub total_count: Option<i64>,
    pub remaining_count: Option<i64>,
}

// ============================================================================
// Get Available Gifts Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetAvailableGiftsNode;

impl GetAvailableGiftsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetAvailableGiftsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_available_gifts",
            "Get Available Gifts",
            "Gets the list of gifts that can be sent by the bot",
            "Telegram/Gifts",
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

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after gifts are retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "gifts",
            "Gifts",
            "List of available gifts",
            VariableType::Struct,
        )
        .set_schema::<Vec<GiftInfo>>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of available gifts",
            VariableType::Integer,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        // Verify bot connection is valid
        let _bot = get_telegram_bot(context, &session.ref_id).await?;

        // Note: getAvailableGifts is not yet supported in teloxide 0.14
        // Return empty list as placeholder
        let gift_infos: Vec<GiftInfo> = Vec::new();
        let count = 0i64;

        context.set_pin_value("gifts", json!(gift_infos)).await?;
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
// Send Gift Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendGiftNode;

impl SendGiftNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendGiftNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_gift",
            "Send Gift",
            "Sends a gift to a user",
            "Telegram/Gifts",
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
            "gift_id",
            "Gift ID",
            "Identifier of the gift to send",
            VariableType::String,
        );

        node.add_input_pin(
            "text",
            "Text",
            "Optional text to accompany the gift (1-255 characters)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["optional".into()])
                .build(),
        );

        node.add_input_pin(
            "text_parse_mode",
            "Parse Mode",
            "Mode for parsing text (MarkdownV2, HTML)",
            VariableType::String,
        )
        .set_default_value(Some(json!("MarkdownV2")));

        node.add_input_pin(
            "text_entities",
            "Text Entities",
            "JSON array of special entities in the text",
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
            "Continues after gift is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the gift was sent successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let _user_id: i64 = context.evaluate_pin("user_id").await?;
        let _gift_id: String = context.evaluate_pin("gift_id").await?;
        let _text: Option<String> = context.evaluate_pin::<String>("text").await.ok();
        let _text_parse_mode: String = context
            .evaluate_pin::<String>("text_parse_mode")
            .await
            .unwrap_or_else(|_| "MarkdownV2".to_string());
        let _text_entities: Option<String> =
            context.evaluate_pin::<String>("text_entities").await.ok();

        // Verify bot connection is valid
        let _bot = get_telegram_bot(context, &session.ref_id).await?;

        // Note: sendGift is not yet supported in teloxide 0.14
        // Return failure as placeholder
        context.set_pin_value("success", json!(false)).await?;

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
// Send Gift Chat Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendGiftChatNode;

impl SendGiftChatNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendGiftChatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_gift_chat",
            "Send Gift to Chat",
            "Sends a gift to a chat (uses chat_id from session)",
            "Telegram/Gifts",
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
            "gift_id",
            "Gift ID",
            "Identifier of the gift to send",
            VariableType::String,
        );

        node.add_input_pin(
            "text",
            "Text",
            "Optional text to accompany the gift (1-255 characters)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["optional".into()])
                .build(),
        );

        node.add_input_pin(
            "text_parse_mode",
            "Parse Mode",
            "Mode for parsing text (MarkdownV2, HTML)",
            VariableType::String,
        )
        .set_default_value(Some(json!("MarkdownV2")));

        node.add_input_pin(
            "text_entities",
            "Text Entities",
            "JSON array of special entities in the text",
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
            "Continues after gift is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the gift was sent successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let _gift_id: String = context.evaluate_pin("gift_id").await?;
        let _text: Option<String> = context.evaluate_pin::<String>("text").await.ok();
        let _text_parse_mode: String = context
            .evaluate_pin::<String>("text_parse_mode")
            .await
            .unwrap_or_else(|_| "MarkdownV2".to_string());
        let _text_entities: Option<String> =
            context.evaluate_pin::<String>("text_entities").await.ok();

        // Verify bot connection is valid
        let _bot = get_telegram_bot(context, &session.ref_id).await?;
        let _chat_id = session.chat_id()?;

        // Note: sendGift is not yet supported in teloxide 0.14
        // Return failure as placeholder
        context.set_pin_value("success", json!(false)).await?;

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
// Gift Premium Subscription Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GiftPremiumSubscriptionNode;

impl GiftPremiumSubscriptionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GiftPremiumSubscriptionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_gift_premium_subscription",
            "Gift Premium Subscription",
            "Gifts a Telegram Premium subscription to a user",
            "Telegram/Gifts",
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
            "month_count",
            "Month Count",
            "Number of months of premium subscription (1, 3, 6, or 12)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "star_count",
            "Star Count",
            "Number of Telegram Stars to pay for the subscription",
            VariableType::Integer,
        );

        node.add_input_pin(
            "text",
            "Text",
            "Optional text to accompany the gift (1-128 characters)",
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
            "Continues after premium is gifted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the premium gift was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let _user_id: i64 = context.evaluate_pin("user_id").await?;
        let _month_count: i64 = context
            .evaluate_pin::<i64>("month_count")
            .await
            .unwrap_or(1);
        let _star_count: i64 = context.evaluate_pin("star_count").await?;
        let _text: Option<String> = context.evaluate_pin::<String>("text").await.ok();

        // Verify bot connection is valid
        let _bot = get_telegram_bot(context, &session.ref_id).await?;

        // Note: giftPremiumSubscription is not yet supported in teloxide 0.14
        // Return failure as placeholder
        context.set_pin_value("success", json!(false)).await?;

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
// Convert Gift to Stars Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ConvertGiftToStarsNode;

impl ConvertGiftToStarsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ConvertGiftToStarsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_convert_gift_to_stars",
            "Convert Gift to Stars",
            "Converts a gift to Telegram Stars",
            "Telegram/Gifts",
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
            "business_connection_id",
            "Business Connection ID",
            "Identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "owned_gift_id",
            "Owned Gift ID",
            "Identifier of the gift to convert",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after gift is converted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the conversion was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let _business_connection_id: String =
            context.evaluate_pin("business_connection_id").await?;
        let _owned_gift_id: String = context.evaluate_pin("owned_gift_id").await?;

        // Verify bot connection is valid
        let _bot = get_telegram_bot(context, &session.ref_id).await?;

        // Note: convertGiftToStars is not yet supported in teloxide 0.14
        // Return failure as placeholder
        context.set_pin_value("success", json!(false)).await?;

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
// Upgrade Gift Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct UpgradeGiftNode;

impl UpgradeGiftNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpgradeGiftNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_upgrade_gift",
            "Upgrade Gift",
            "Upgrades a gift to a unique gift",
            "Telegram/Gifts",
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
            "business_connection_id",
            "Business Connection ID",
            "Identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "owned_gift_id",
            "Owned Gift ID",
            "Identifier of the gift to upgrade",
            VariableType::String,
        );

        node.add_input_pin(
            "keep_original_details",
            "Keep Original Details",
            "Whether to keep the original gift details",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "star_count",
            "Star Count",
            "Number of Telegram Stars to pay for the upgrade",
            VariableType::Integer,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["optional".into()])
                .build(),
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after gift is upgraded",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the upgrade was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let _business_connection_id: String =
            context.evaluate_pin("business_connection_id").await?;
        let _owned_gift_id: String = context.evaluate_pin("owned_gift_id").await?;
        let _keep_original_details: bool = context
            .evaluate_pin::<bool>("keep_original_details")
            .await
            .unwrap_or(false);
        let _star_count: Option<i64> = context.evaluate_pin::<i64>("star_count").await.ok();

        // Verify bot connection is valid
        let _bot = get_telegram_bot(context, &session.ref_id).await?;

        // Note: upgradeGift is not yet supported in teloxide 0.14
        // Return failure as placeholder
        context.set_pin_value("success", json!(false)).await?;

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
// Transfer Gift Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct TransferGiftNode;

impl TransferGiftNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for TransferGiftNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_transfer_gift",
            "Transfer Gift",
            "Transfers an owned gift to another user",
            "Telegram/Gifts",
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
            "business_connection_id",
            "Business Connection ID",
            "Identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "owned_gift_id",
            "Owned Gift ID",
            "Identifier of the gift to transfer",
            VariableType::String,
        );

        node.add_input_pin(
            "new_owner_chat_id",
            "New Owner Chat ID",
            "Chat ID of the new owner",
            VariableType::Integer,
        );

        node.add_input_pin(
            "star_count",
            "Star Count",
            "Number of Telegram Stars to pay for the transfer",
            VariableType::Integer,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["optional".into()])
                .build(),
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after gift is transferred",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the transfer was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let _business_connection_id: String =
            context.evaluate_pin("business_connection_id").await?;
        let _owned_gift_id: String = context.evaluate_pin("owned_gift_id").await?;
        let _new_owner_chat_id: i64 = context.evaluate_pin("new_owner_chat_id").await?;
        let _star_count: Option<i64> = context.evaluate_pin::<i64>("star_count").await.ok();

        // Verify bot connection is valid
        let _bot = get_telegram_bot(context, &session.ref_id).await?;

        // Note: transferGift is not yet supported in teloxide 0.14
        // Return failure as placeholder
        context.set_pin_value("success", json!(false)).await?;

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
