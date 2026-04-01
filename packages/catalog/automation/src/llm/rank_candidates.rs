use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{async_trait, json};
#[cfg(feature = "execute")]
use rig::completion::{Completion, ToolDefinition};
#[cfg(feature = "execute")]
use rig::message::{AssistantContent, ToolCall, ToolChoice, ToolFunction};
#[cfg(feature = "execute")]
use rig::tool::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CandidateInput {
    pub id: String,
    pub description: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub selector: Option<String>,
    pub additional_info: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RankedCandidate {
    pub id: String,
    pub rank: usize,
    pub score: f64,
    pub reasoning: String,
    pub is_recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RankingResult {
    pub ranked_candidates: Vec<RankedCandidate>,
    pub best_match_id: String,
    pub confidence: f64,
    pub ambiguity_warning: Option<String>,
}

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
struct RankCandidatesTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct RankCandidatesError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for RankCandidatesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rank candidates error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for RankCandidatesError {}

#[cfg(feature = "execute")]
impl Tool for RankCandidatesTool {
    const NAME: &'static str = "submit_ranking";
    type Error = RankCandidatesError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit the candidate ranking".to_string(),
            parameters: self.parameters.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        Ok(args)
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMRankCandidatesNode {}

impl LLMRankCandidatesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMRankCandidatesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_rank_candidates",
            "LLM Rank Candidates",
            "Uses LLM to rank multiple element candidates based on match quality",
            "Automation/LLM/Vision",
        );
        node.add_icon("/flow/icons/bot-search.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(4)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "model",
            "Model",
            "Vision-capable LLM model",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "screenshot",
            "Screenshot",
            "Base64-encoded screenshot",
            VariableType::String,
        );

        node.add_input_pin(
            "candidates",
            "Candidates",
            "Array of candidate elements to rank",
            VariableType::Generic,
        );

        node.add_input_pin(
            "criteria",
            "Criteria",
            "What the target element should match (description/intent)",
            VariableType::String,
        );

        node.add_input_pin(
            "context",
            "Context",
            "Additional context for ranking",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "result",
            "Result",
            "Full ranking result",
            VariableType::Struct,
        )
        .set_schema::<RankingResult>();

        node.add_output_pin(
            "best_match",
            "Best Match",
            "ID of the best matching candidate",
            VariableType::String,
        );

        node.add_output_pin(
            "ranked",
            "Ranked",
            "Candidates sorted by rank",
            VariableType::Generic,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_model_provider::history::{
            Content, ContentType, History, HistoryMessage, ImageUrl as HistoryImageUrl,
            MessageContent, Role,
        };

        context.deactivate_exec_pin("exec_out").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let screenshot: String = context.evaluate_pin("screenshot").await?;
        let candidates: Vec<CandidateInput> = context.evaluate_pin("candidates").await?;
        let criteria: String = context.evaluate_pin("criteria").await?;
        let ctx: String = context.evaluate_pin("context").await.unwrap_or_default();

        let tool_params = json::json!({
            "type": "object",
            "properties": {
                "ranked_candidates": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Candidate ID" },
                            "rank": { "type": "integer", "description": "Rank (1 = best)" },
                            "score": { "type": "number", "description": "Match score 0-1" },
                            "reasoning": { "type": "string", "description": "Why this ranking" },
                            "is_recommended": { "type": "boolean", "description": "Is this a good match?" }
                        },
                        "required": ["id", "rank", "score", "reasoning", "is_recommended"]
                    }
                },
                "best_match_id": { "type": "string", "description": "ID of the best matching candidate" },
                "confidence": { "type": "number", "description": "Overall confidence in ranking 0-1" },
                "ambiguity_warning": { "type": "string", "description": "Warning if multiple candidates are equally good" }
            },
            "required": ["ranked_candidates", "best_match_id", "confidence"]
        });

        let candidates_desc = candidates
            .iter()
            .map(|c| {
                let pos =
                    c.x.map(|x| format!(" at ({}, {})", x, c.y.unwrap_or(0)))
                        .unwrap_or_default();
                format!("- [{}]: {}{}", c.id, c.description, pos)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let context_text = if ctx.is_empty() {
            String::new()
        } else {
            format!("\nAdditional context: {}", ctx)
        };

        let content_parts = vec![
            Content::Image {
                content_type: ContentType::ImageUrl,
                image_url: HistoryImageUrl {
                    url: format!("data:image/png;base64,{}", screenshot),
                    detail: None,
                },
            },
            Content::Text {
                content_type: ContentType::Text,
                text: format!(
                    "Criteria: {}\n\nCandidates:\n{}{}\n\nRank these candidates from best to worst match.",
                    criteria, candidates_desc, context_text
                ),
            },
        ];

        let history = History::new(
            "".to_string(),
            vec![HistoryMessage {
                role: Role::User,
                content: MessageContent::Contents(content_parts),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                annotations: None,
            }],
        );

        let preamble = "You are an element matching expert. Rank multiple candidate elements based on how well they match the given criteria. Consider visual appearance, position, context, and semantics.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(RankCandidatesTool {
                parameters: tool_params,
            })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        let response = agent
            .completion(criteria.clone(), vec![])
            .await
            .map_err(|e| anyhow!("LLM completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send request: {}", e))?;

        let mut result: Option<RankingResult> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_ranking"
            {
                result = Some(json::from_value(arguments)?);
            }
        }

        let ranking = result.unwrap_or(RankingResult {
            ranked_candidates: vec![],
            best_match_id: String::new(),
            confidence: 0.0,
            ambiguity_warning: Some("Could not rank candidates".to_string()),
        });

        context
            .set_pin_value("result", json::json!(ranking.clone()))
            .await?;
        context
            .set_pin_value("best_match", json::json!(ranking.best_match_id))
            .await?;
        context
            .set_pin_value("ranked", json::json!(ranking.ranked_candidates))
            .await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }
}
