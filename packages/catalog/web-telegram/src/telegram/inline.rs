//! Telegram inline query operations

use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::json::from_str;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::{InlineQueryId, InlineQueryResult, SentWebAppMessage};

// ============================================================================
// Answer Inline Query Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct AnswerInlineQueryNode;

impl AnswerInlineQueryNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AnswerInlineQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_answer_inline_query",
            "Answer Inline Query",
            "Sends answers to an inline query",
            "Telegram/Inline",
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
            "inline_query_id",
            "Inline Query ID",
            "Unique identifier for the answered query",
            VariableType::String,
        );

        node.add_input_pin(
            "results",
            "Results",
            "JSON array of InlineQueryResult objects (max 50)",
            VariableType::String,
        );

        node.add_input_pin(
            "cache_time",
            "Cache Time",
            "Maximum time in seconds to cache results (default 300)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(300)));

        node.add_input_pin(
            "is_personal",
            "Is Personal",
            "Pass true if results may be cached only for the user that sent the query",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "next_offset",
            "Next Offset",
            "Offset for pagination (pass empty string if no more results)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "button",
            "Button",
            "JSON object for InlineQueryResultsButton (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after inline query is answered",
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
        let inline_query_id: String = context.evaluate_pin("inline_query_id").await?;
        let results_json: String = context.evaluate_pin("results").await?;
        let cache_time: i64 = context
            .evaluate_pin::<i64>("cache_time")
            .await
            .unwrap_or(300);
        let is_personal: bool = context
            .evaluate_pin::<bool>("is_personal")
            .await
            .unwrap_or(false);
        let next_offset: String = context
            .evaluate_pin::<String>("next_offset")
            .await
            .unwrap_or_default();
        let button_json: String = context
            .evaluate_pin::<String>("button")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let results: Vec<InlineQueryResult> = from_str(&results_json)?;

        let mut request = bot
            .bot
            .answer_inline_query(InlineQueryId(inline_query_id), results)
            .cache_time(cache_time as u32)
            .is_personal(is_personal);

        if !next_offset.is_empty() {
            request = request.next_offset(next_offset);
        }

        if !button_json.is_empty() {
            let button: teloxide::types::InlineQueryResultsButton = from_str(&button_json)?;
            request = request.button(button);
        }

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
// Answer Web App Query Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct AnswerWebAppQueryNode;

impl AnswerWebAppQueryNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AnswerWebAppQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_answer_web_app_query",
            "Answer Web App Query",
            "Sends a result of an interaction with a Web App to be sent on behalf of the user",
            "Telegram/Inline",
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
            "web_app_query_id",
            "Web App Query ID",
            "Unique identifier of the query to be answered",
            VariableType::String,
        );

        node.add_input_pin(
            "result",
            "Result",
            "JSON object describing the message to be sent (InlineQueryResult)",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after web app query is answered",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentWebAppMessageInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let web_app_query_id: String = context.evaluate_pin("web_app_query_id").await?;
        let result_json: String = context.evaluate_pin("result").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result: InlineQueryResult = from_str(&result_json)?;

        let response: SentWebAppMessage = bot
            .bot
            .answer_web_app_query(&web_app_query_id, result)
            .await?;

        let sent_message_info = SentWebAppMessageInfo {
            inline_message_id: response.inline_message_id,
        };

        context
            .set_pin_value("sent_message", json!(sent_message_info))
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

/// Information about a sent web app message
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SentWebAppMessageInfo {
    pub inline_message_id: Option<String>,
}
