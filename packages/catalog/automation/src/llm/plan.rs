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
pub struct NextStepSuggestion {
    pub action_type: String,
    pub target_description: String,
    pub target_coordinates: Option<(i32, i32)>,
    pub parameters: flow_like_types::Value,
    pub reasoning: String,
    pub confidence: f64,
    pub alternatives: Vec<AlternativeAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AlternativeAction {
    pub action_type: String,
    pub description: String,
    pub confidence: f64,
}

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
struct SuggestNextStepTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct SuggestNextStepError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for SuggestNextStepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Suggest next step error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for SuggestNextStepError {}

#[cfg(feature = "execute")]
impl Tool for SuggestNextStepTool {
    const NAME: &'static str = "submit_next_step";
    type Error = SuggestNextStepError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit the suggested next step".to_string(),
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
pub struct LLMSuggestNextStepNode {}

impl LLMSuggestNextStepNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMSuggestNextStepNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_suggest_next_step",
            "LLM Suggest Next Step",
            "Uses LLM to suggest the most appropriate next action given current screen and goal",
            "Automation/LLM/Planning",
        );
        node.add_icon("/flow/icons/bot-plan.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(4)
                .set_governance(5)
                .set_reliability(6)
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
            "Base64-encoded current screenshot",
            VariableType::String,
        );

        node.add_input_pin(
            "goal",
            "Goal",
            "Ultimate goal we're trying to achieve",
            VariableType::String,
        );

        node.add_input_pin(
            "completed_actions",
            "Completed Actions",
            "JSON array of actions already taken",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("[]")));

        node.add_input_pin(
            "last_result",
            "Last Result",
            "Result/outcome of the last action",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "exec_goal_reached",
            "Goal Reached",
            "Goal appears to be achieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "suggestion",
            "Suggestion",
            "Next step suggestion",
            VariableType::Struct,
        )
        .set_schema::<NextStepSuggestion>();

        node.add_output_pin(
            "action_type",
            "Action Type",
            "Type of suggested action",
            VariableType::String,
        );

        node.add_output_pin(
            "target",
            "Target",
            "Target description",
            VariableType::String,
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
        context.deactivate_exec_pin("exec_goal_reached").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let screenshot: String = context.evaluate_pin("screenshot").await?;
        let goal: String = context.evaluate_pin("goal").await?;
        let completed_actions: String = context
            .evaluate_pin("completed_actions")
            .await
            .unwrap_or_else(|_| "[]".to_string());
        let last_result: String = context
            .evaluate_pin("last_result")
            .await
            .unwrap_or_default();

        let tool_params = json::json!({
            "type": "object",
            "properties": {
                "goal_reached": { "type": "boolean", "description": "Whether the goal appears to be reached" },
                "action_type": { "type": "string", "description": "Type of action (click, type, scroll, wait, verify)" },
                "target_description": { "type": "string", "description": "What to interact with" },
                "target_coordinates": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Approximate [x, y] coordinates if applicable"
                },
                "parameters": { "type": "object", "description": "Action-specific parameters" },
                "reasoning": { "type": "string", "description": "Why this action is suggested" },
                "confidence": { "type": "number", "description": "Confidence in this suggestion 0-1" },
                "alternatives": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action_type": { "type": "string" },
                            "description": { "type": "string" },
                            "confidence": { "type": "number" }
                        }
                    },
                    "description": "Alternative actions to consider"
                }
            },
            "required": ["goal_reached", "action_type", "target_description", "reasoning", "confidence"]
        });

        let progress_context = if completed_actions == "[]" {
            "This is the first action.".to_string()
        } else {
            format!("Actions taken so far: {}", completed_actions)
        };

        let last_result_text = if last_result.is_empty() {
            String::new()
        } else {
            format!("\nLast action result: {}", last_result)
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
                    "Goal: {}\n\n{}{}\n\nWhat should be the next action?",
                    goal, progress_context, last_result_text
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

        let preamble = "You are an intelligent automation assistant. Given the current screen, goal, and progress, suggest the single best next action. If the goal is already achieved, indicate that. Be precise about what to interact with.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(SuggestNextStepTool {
                parameters: tool_params,
            })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        let response = agent
            .completion(goal.clone(), vec![])
            .await
            .map_err(|e| anyhow!("LLM completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send request: {}", e))?;

        let mut goal_reached = false;
        let mut result: Option<NextStepSuggestion> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_next_step"
            {
                goal_reached = arguments
                    .get("goal_reached")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                result = Some(json::from_value(arguments)?);
            }
        }

        let suggestion = result.unwrap_or(NextStepSuggestion {
            action_type: "wait".to_string(),
            target_description: "Unable to determine next step".to_string(),
            target_coordinates: None,
            parameters: json::json!({}),
            reasoning: "Could not analyze screen".to_string(),
            confidence: 0.0,
            alternatives: vec![],
        });

        context
            .set_pin_value("suggestion", json::json!(suggestion.clone()))
            .await?;
        context
            .set_pin_value("action_type", json::json!(suggestion.action_type))
            .await?;
        context
            .set_pin_value("target", json::json!(suggestion.target_description))
            .await?;

        if goal_reached {
            context.activate_exec_pin("exec_goal_reached").await?;
        } else {
            context.activate_exec_pin("exec_out").await?;
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }
}
