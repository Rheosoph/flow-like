//! Telegram poll management - send polls and retrieve results

use super::message::SentMessage;
use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;
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
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::InputPollOption;

/// Reference to a poll that can be used to retrieve results later
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PollReference {
    /// The message ID containing the poll
    pub message_id: String,
    /// The chat ID where the poll was sent
    pub chat_id: String,
    /// The poll ID from Telegram
    pub poll_id: String,
    /// Session ref_id for retrieving the bot
    pub session_ref_id: String,
    /// Original question for reference
    pub question: String,
    /// Original options for reference
    pub options: Vec<String>,
    /// Whether the poll allows multiple answers
    pub allows_multiple_answers: bool,
    /// Whether the poll is anonymous
    pub is_anonymous: bool,
}

/// Result of a poll option
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PollOptionResult {
    /// The option text
    pub text: String,
    /// Number of votes for this option
    pub voter_count: u32,
}

/// Poll results
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PollResults {
    /// The poll ID
    pub poll_id: String,
    /// The original question
    pub question: String,
    /// Total number of voters
    pub total_voter_count: u32,
    /// Whether the poll is closed
    pub is_closed: bool,
    /// Results for each option
    pub options: Vec<PollOptionResult>,
}

// ============================================================================
// Send Poll Node (Enhanced with Poll Reference)
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendPollWithRefNode;

impl SendPollWithRefNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendPollWithRefNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_poll_with_ref",
            "Send Poll (Trackable)",
            "Sends a poll and returns a reference for tracking results",
            "Telegram/Poll",
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
            "question",
            "Question",
            "Poll question (1-300 characters)",
            VariableType::String,
        );

        node.add_input_pin(
            "options",
            "Options",
            "Array of poll options (2-10 items)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "is_anonymous",
            "Anonymous",
            "Whether the poll is anonymous (default: true)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "allows_multiple_answers",
            "Multiple Answers",
            "Allow selecting multiple options",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after poll is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "poll_ref",
            "Poll Reference",
            "Reference to track poll results",
            VariableType::Struct,
        )
        .set_schema::<PollReference>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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
        let question: String = context.evaluate_pin("question").await?;
        let options: Vec<String> = context.evaluate_pin("options").await?;

        let is_anonymous: bool = context
            .evaluate_pin::<bool>("is_anonymous")
            .await
            .unwrap_or(true);
        let allows_multiple: bool = context
            .evaluate_pin::<bool>("allows_multiple_answers")
            .await
            .unwrap_or(false);

        if options.len() < 2 || options.len() > 10 {
            return Err(flow_like_types::anyhow!("Poll must have 2-10 options"));
        }

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let poll_options: Vec<InputPollOption> = options.iter().map(InputPollOption::new).collect();

        let sent = bot
            .bot
            .send_poll(chat_id, &question, poll_options)
            .is_anonymous(is_anonymous)
            .allows_multiple_answers(allows_multiple)
            .await?;

        let poll = sent
            .poll()
            .ok_or_else(|| flow_like_types::anyhow!("No poll in response"))?;

        let poll_ref = PollReference {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            poll_id: poll.id.to_string(),
            session_ref_id: session.ref_id.clone(),
            question: question.clone(),
            options: options.clone(),
            allows_multiple_answers: allows_multiple,
            is_anonymous,
        };

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context.set_pin_value("poll_ref", json!(poll_ref)).await?;
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
// Stop Poll Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct StopPollNode;

impl StopPollNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for StopPollNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_stop_poll",
            "Stop Poll",
            "Stops a poll and retrieves final results",
            "Telegram/Poll",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session (optional if poll_ref is provided)",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>();

        node.add_input_pin(
            "poll_ref",
            "Poll Reference",
            "Reference from Send Poll node",
            VariableType::Struct,
        )
        .set_schema::<PollReference>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after poll is stopped",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Final poll results",
            VariableType::Struct,
        )
        .set_schema::<PollResults>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "success",
            "Success",
            "Whether the poll was stopped successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let poll_ref: PollReference = context.evaluate_pin("poll_ref").await?;

        // Try to get session from input, fall back to poll_ref's session
        let session_ref_id = context
            .evaluate_pin::<TelegramSession>("session")
            .await
            .map(|s| s.ref_id)
            .unwrap_or(poll_ref.session_ref_id.clone());

        let bot = get_telegram_bot(context, &session_ref_id).await?;
        let chat_id = teloxide::types::ChatId(poll_ref.chat_id.parse()?);
        let message_id: i32 = poll_ref.message_id.parse()?;

        let result = bot
            .bot
            .stop_poll(chat_id, teloxide::types::MessageId(message_id))
            .await;

        match result {
            Ok(poll) => {
                let results = PollResults {
                    poll_id: poll.id.to_string(),
                    question: poll.question.clone(),
                    total_voter_count: poll.total_voter_count,
                    is_closed: poll.is_closed,
                    options: poll
                        .options
                        .iter()
                        .map(|o| PollOptionResult {
                            text: o.text.clone(),
                            voter_count: o.voter_count,
                        })
                        .collect(),
                };

                context.set_pin_value("results", json!(results)).await?;
                context.set_pin_value("success", json!(true)).await?;
            }
            Err(e) => {
                context.log_message(
                    &format!("Failed to stop poll: {}", e),
                    flow_like::flow::execution::LogLevel::Warn,
                );
                context.set_pin_value("success", json!(false)).await?;

                // Return empty results
                let empty_results = PollResults {
                    poll_id: poll_ref.poll_id.clone(),
                    question: poll_ref.question.clone(),
                    total_voter_count: 0,
                    is_closed: false,
                    options: poll_ref
                        .options
                        .iter()
                        .map(|o| PollOptionResult {
                            text: o.clone(),
                            voter_count: 0,
                        })
                        .collect(),
                };
                context
                    .set_pin_value("results", json!(empty_results))
                    .await?;
            }
        }

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
// Send Quiz Poll Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendQuizNode;

impl SendQuizNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendQuizNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_quiz",
            "Send Quiz",
            "Sends a quiz poll with a correct answer",
            "Telegram/Poll",
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
            "question",
            "Question",
            "Quiz question",
            VariableType::String,
        );

        node.add_input_pin(
            "options",
            "Options",
            "Array of answer options (2-10 items)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "correct_option_index",
            "Correct Option Index",
            "0-based index of the correct answer",
            VariableType::Integer,
        );

        node.add_input_pin(
            "explanation",
            "Explanation",
            "Optional explanation shown after answering",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after quiz is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "poll_ref",
            "Poll Reference",
            "Reference to track quiz results",
            VariableType::Struct,
        )
        .set_schema::<PollReference>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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
        let question: String = context.evaluate_pin("question").await?;
        let options: Vec<String> = context.evaluate_pin("options").await?;
        let correct_index: i64 = context.evaluate_pin("correct_option_index").await?;
        let explanation: Option<String> = context.evaluate_pin::<String>("explanation").await.ok();

        if options.len() < 2 || options.len() > 10 {
            return Err(flow_like_types::anyhow!("Quiz must have 2-10 options"));
        }

        if correct_index < 0 || correct_index >= options.len() as i64 {
            return Err(flow_like_types::anyhow!(
                "Correct option index out of range"
            ));
        }

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let poll_options: Vec<InputPollOption> = options.iter().map(InputPollOption::new).collect();

        let mut request = bot
            .bot
            .send_poll(chat_id, &question, poll_options)
            .type_(teloxide::types::PollType::Quiz)
            .correct_option_id(correct_index as u8);

        if let Some(exp) = explanation {
            request = request.explanation(exp);
        }

        let sent = request.await?;

        let poll = sent
            .poll()
            .ok_or_else(|| flow_like_types::anyhow!("No poll in response"))?;

        let poll_ref = PollReference {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            poll_id: poll.id.to_string(),
            session_ref_id: session.ref_id.clone(),
            question: question.clone(),
            options: options.clone(),
            allows_multiple_answers: false,
            is_anonymous: true,
        };

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context.set_pin_value("poll_ref", json!(poll_ref)).await?;
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
