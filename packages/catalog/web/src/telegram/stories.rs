//! Telegram story operations
//!
//! Note: Story operations require Telegram Bot API 7.3+ which is not fully supported
//! by the current teloxide version (0.14). Implementations return errors until
//! teloxide is updated to support these methods.

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

/// Story information returned after posting/editing
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StoryInfo {
    pub story_id: i32,
    pub chat_id: i64,
    pub date: i64,
    pub expire_date: i64,
    pub content_type: String,
}

// ============================================================================
// Post Story Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct PostStoryNode;

impl PostStoryNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PostStoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_post_story",
            "Post Story",
            "Posts a story to Telegram on behalf of a business account",
            "Telegram/Stories",
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
            "Unique identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "content",
            "Content",
            "Story content as JSON with 'photo_url' or 'video_url' field",
            VariableType::Struct,
        );

        node.add_input_pin(
            "active_period",
            "Active Period",
            "Period in seconds the story will be active (default: 86400 = 24 hours)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(86400)));

        node.add_input_pin(
            "caption",
            "Caption",
            "Optional caption for the story",
            VariableType::String,
        );

        node.add_input_pin(
            "parse_mode",
            "Parse Mode",
            "Parse mode for the caption (HTML, Markdown, MarkdownV2)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "".to_string(),
                    "HTML".to_string(),
                    "Markdown".to_string(),
                    "MarkdownV2".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "protect_content",
            "Protect Content",
            "Protect the story content from forwarding and saving",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "areas",
            "Areas",
            "Optional JSON array of clickable areas in the story",
            VariableType::Struct,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after story is posted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "story",
            "Story Info",
            "Information about the posted story",
            VariableType::Struct,
        )
        .set_schema::<StoryInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let _business_connection_id: String =
            context.evaluate_pin("business_connection_id").await?;

        // Verify bot connection is valid
        let _bot = get_telegram_bot(context, &session.ref_id).await?;

        // Note: postStory is not yet supported in teloxide 0.14
        // This API requires Bot API 7.3+
        Err(flow_like_types::anyhow!(
            "postStory is not yet supported in the current teloxide version. \
             This API requires Telegram Bot API 7.3+ support."
        ))
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Edit Story Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EditStoryNode;

impl EditStoryNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EditStoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_edit_story",
            "Edit Story",
            "Edits a previously posted story",
            "Telegram/Stories",
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
            "Unique identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "story_id",
            "Story ID",
            "Unique identifier of the story to edit",
            VariableType::Integer,
        );

        node.add_input_pin(
            "content",
            "Content",
            "New story content as JSON with 'photo_url' or 'video_url' field",
            VariableType::Struct,
        );

        node.add_input_pin(
            "caption",
            "Caption",
            "New caption for the story",
            VariableType::String,
        );

        node.add_input_pin(
            "parse_mode",
            "Parse Mode",
            "Parse mode for the caption (HTML, Markdown, MarkdownV2)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "".to_string(),
                    "HTML".to_string(),
                    "Markdown".to_string(),
                    "MarkdownV2".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "areas",
            "Areas",
            "Optional JSON array of clickable areas in the story",
            VariableType::Struct,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after story is edited",
            VariableType::Execution,
        );

        node.add_output_pin(
            "story",
            "Story Info",
            "Information about the edited story",
            VariableType::Struct,
        )
        .set_schema::<StoryInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let _business_connection_id: String =
            context.evaluate_pin("business_connection_id").await?;
        let _story_id: i64 = context.evaluate_pin("story_id").await?;

        // Verify bot connection is valid
        let _bot = get_telegram_bot(context, &session.ref_id).await?;

        // Note: editStory is not yet supported in teloxide 0.14
        // This API requires Bot API 7.3+
        Err(flow_like_types::anyhow!(
            "editStory is not yet supported in the current teloxide version. \
             This API requires Telegram Bot API 7.3+ support."
        ))
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Delete Story Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct DeleteStoryNode;

impl DeleteStoryNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteStoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_delete_story",
            "Delete Story",
            "Deletes a previously posted story",
            "Telegram/Stories",
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
            "Unique identifier of the business connection",
            VariableType::String,
        );

        node.add_input_pin(
            "story_id",
            "Story ID",
            "Unique identifier of the story to delete",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after story is deleted",
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
        let _business_connection_id: String =
            context.evaluate_pin("business_connection_id").await?;
        let _story_id: i64 = context.evaluate_pin("story_id").await?;

        // Verify bot connection is valid
        let _bot = get_telegram_bot(context, &session.ref_id).await?;

        // Note: deleteStory is not yet supported in teloxide 0.14
        // This API requires Bot API 7.3+
        Err(flow_like_types::anyhow!(
            "deleteStory is not yet supported in the current teloxide version. \
             This API requires Telegram Bot API 7.3+ support."
        ))
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}
