//! Discord polls - create and track poll results

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
use serenity::all::{CreatePoll, CreatePollAnswer, PollLayoutType};
#[cfg(feature = "execute")]
use serenity::builder::CreateMessage;

/// Reference to a sent poll for tracking
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PollReference {
    /// The message ID containing the poll
    pub message_id: String,
    /// The channel ID where the poll was sent
    pub channel_id: String,
    /// The poll question
    pub question: String,
    /// Duration in hours
    pub duration_hours: u32,
    /// Whether multiple selections allowed
    pub allow_multiselect: bool,
}

/// Individual poll answer result
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PollAnswerResult {
    /// The answer ID
    pub id: u64,
    /// The answer text
    pub text: String,
    /// Vote count
    pub vote_count: u64,
}

/// Complete poll results
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PollResults {
    /// The poll question
    pub question: String,
    /// Whether the poll has ended
    pub is_finalized: bool,
    /// List of answers with vote counts
    pub answers: Vec<PollAnswerResult>,
    /// Total votes cast
    pub total_votes: u64,
}

// ============================================================================
// Send Poll Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendPollNode;

impl SendPollNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendPollNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_send_poll",
            "Send Poll",
            "Creates and sends a poll to a Discord channel",
            "Discord/Polls",
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
            "question",
            "Question",
            "The poll question",
            VariableType::String,
        );

        node.add_input_pin(
            "options",
            "Options",
            "Poll options (2-10 options)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "duration_hours",
            "Duration (hours)",
            "How long the poll runs (1-168 hours, default 24)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(24)));

        node.add_input_pin(
            "allow_multiselect",
            "Allow Multiselect",
            "Allow users to select multiple options",
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
            "poll_ref",
            "Poll Reference",
            "Reference to track the poll later",
            VariableType::Struct,
        )
        .set_schema::<PollReference>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the poll message",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;
        let question: String = context.evaluate_pin("question").await?;
        let options: Vec<String> = context.evaluate_pin("options").await?;
        let duration_hours: i64 = context.evaluate_pin("duration_hours").await.unwrap_or(24);
        let allow_multiselect: bool = context
            .evaluate_pin("allow_multiselect")
            .await
            .unwrap_or(false);
        let channel_override: Option<String> = context.evaluate_pin("channel_id").await.ok();

        if options.len() < 2 {
            return Err(flow_like_types::anyhow!("Poll requires at least 2 options"));
        }
        if options.len() > 10 {
            return Err(flow_like_types::anyhow!("Poll supports max 10 options"));
        }

        let duration = duration_hours.clamp(1, 168) as u64;

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = match channel_override.filter(|s| !s.is_empty()) {
            Some(id) => serenity::all::ChannelId::new(id.parse()?),
            None => session.channel_id()?,
        };

        let answers: Vec<CreatePollAnswer> = options
            .iter()
            .map(|opt| CreatePollAnswer::new().text(opt))
            .collect();

        let mut poll = CreatePoll::new()
            .question(&question)
            .answers(answers)
            .duration(std::time::Duration::from_secs(duration * 3600))
            .layout_type(PollLayoutType::Default);

        if allow_multiselect {
            poll = poll.allow_multiselect();
        }

        let message = CreateMessage::new().poll(poll);

        let sent = channel_id.send_message(&client.http, message).await?;

        let poll_ref = PollReference {
            message_id: sent.id.to_string(),
            channel_id: channel_id.to_string(),
            question,
            duration_hours: duration as u32,
            allow_multiselect,
        };

        context.set_pin_value("poll_ref", json!(poll_ref)).await?;
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
// Get Poll Results Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetPollResultsNode;

impl GetPollResultsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetPollResultsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_get_poll_results",
            "Get Poll Results",
            "Retrieves current results of a poll",
            "Discord/Polls",
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
            "poll_ref",
            "Poll Reference",
            "Reference to the poll",
            VariableType::Struct,
        )
        .set_schema::<PollReference>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "results",
            "Results",
            "Poll results with vote counts",
            VariableType::Struct,
        )
        .set_schema::<PollResults>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "is_finalized",
            "Is Finalized",
            "Whether the poll has ended",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "total_votes",
            "Total Votes",
            "Total number of votes cast",
            VariableType::Integer,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;
        let poll_ref: PollReference = context.evaluate_pin("poll_ref").await?;

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = serenity::all::ChannelId::new(poll_ref.channel_id.parse()?);
        let message_id = serenity::all::MessageId::new(poll_ref.message_id.parse()?);

        let message = client.http.get_message(channel_id, message_id).await?;

        let poll = message
            .poll
            .ok_or_else(|| flow_like_types::anyhow!("Message does not contain a poll"))?;

        let mut answers = Vec::new();
        let mut total_votes = 0u64;

        for answer in &poll.answers {
            let vote_count = poll
                .results
                .as_ref()
                .and_then(|r| r.answer_counts.iter().find(|ac| ac.id == answer.answer_id))
                .map(|ac| ac.count)
                .unwrap_or(0);

            answers.push(PollAnswerResult {
                id: answer.answer_id.get(),
                text: answer.poll_media.text.clone().unwrap_or_default(),
                vote_count,
            });
            total_votes += vote_count;
        }

        let is_finalized = poll
            .results
            .as_ref()
            .map(|r| r.is_finalized)
            .unwrap_or(false);

        let results = PollResults {
            question: poll.question.text.unwrap_or_default(),
            is_finalized,
            answers,
            total_votes,
        };

        context.set_pin_value("results", json!(results)).await?;
        context
            .set_pin_value("is_finalized", json!(is_finalized))
            .await?;
        context
            .set_pin_value("total_votes", json!(total_votes as i64))
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
// End Poll Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct EndPollNode;

impl EndPollNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EndPollNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_end_poll",
            "End Poll",
            "Immediately ends a poll and returns final results",
            "Discord/Polls",
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
            "poll_ref",
            "Poll Reference",
            "Reference to the poll to end",
            VariableType::Struct,
        )
        .set_schema::<PollReference>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Output", "Continues", VariableType::Execution);

        node.add_output_pin(
            "results",
            "Results",
            "Final poll results",
            VariableType::Struct,
        )
        .set_schema::<PollResults>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: DiscordSession = context.evaluate_pin("session").await?;
        let poll_ref: PollReference = context.evaluate_pin("poll_ref").await?;

        let client = get_discord_client(context, &session.ref_id).await?;

        let channel_id = serenity::all::ChannelId::new(poll_ref.channel_id.parse()?);
        let message_id = serenity::all::MessageId::new(poll_ref.message_id.parse()?);

        let message = client.http.expire_poll(channel_id, message_id).await?;

        let poll = message
            .poll
            .ok_or_else(|| flow_like_types::anyhow!("Message does not contain a poll"))?;

        let mut answers = Vec::new();
        let mut total_votes = 0u64;

        for answer in &poll.answers {
            let vote_count = poll
                .results
                .as_ref()
                .and_then(|r| r.answer_counts.iter().find(|ac| ac.id == answer.answer_id))
                .map(|ac| ac.count)
                .unwrap_or(0);

            answers.push(PollAnswerResult {
                id: answer.answer_id.get(),
                text: answer.poll_media.text.clone().unwrap_or_default(),
                vote_count,
            });
            total_votes += vote_count;
        }

        let results = PollResults {
            question: poll.question.text.unwrap_or_default(),
            is_finalized: true,
            answers,
            total_votes,
        };

        context.set_pin_value("results", json!(results)).await?;
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
