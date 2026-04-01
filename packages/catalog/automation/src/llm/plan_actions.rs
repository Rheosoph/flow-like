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
pub struct PlannedAction {
    pub action_type: String,
    pub target: String,
    pub parameters: flow_like_types::Value,
    pub reasoning: String,
    pub expected_result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActionPlan {
    pub goal_understood: bool,
    pub current_state_assessment: String,
    pub actions: Vec<PlannedAction>,
    pub success_criteria: Vec<String>,
    pub potential_obstacles: Vec<String>,
    pub confidence: f64,
}

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
struct PlanActionsTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct PlanActionsError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for PlanActionsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Plan actions error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for PlanActionsError {}

#[cfg(feature = "execute")]
impl Tool for PlanActionsTool {
    const NAME: &'static str = "submit_action_plan";
    type Error = PlanActionsError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit the planned sequence of actions".to_string(),
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
pub struct LLMPlanActionsNode {}

impl LLMPlanActionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMPlanActionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_plan_actions",
            "LLM Plan Actions",
            "Uses LLM to plan a sequence of automation actions to achieve a goal",
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
            "What the automation should accomplish",
            VariableType::String,
        );

        node.add_input_pin(
            "available_actions",
            "Available Actions",
            "JSON array of available action types and their parameters",
            VariableType::String,
        )
        .set_default_value(Some(json::json!(
            "[\"click\", \"type\", \"scroll\", \"wait\", \"hover\"]"
        )));

        node.add_input_pin(
            "constraints",
            "Constraints",
            "Any constraints or preferences for the plan",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin("plan", "Plan", "Complete action plan", VariableType::Struct)
            .set_schema::<ActionPlan>();

        node.add_output_pin(
            "actions",
            "Actions",
            "List of planned actions",
            VariableType::Generic,
        );

        node.add_output_pin(
            "first_action",
            "First Action",
            "The first action to execute",
            VariableType::Struct,
        )
        .set_schema::<PlannedAction>();

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
        let goal: String = context.evaluate_pin("goal").await?;
        let available_actions: String = context.evaluate_pin("available_actions").await?;
        let constraints: String = context
            .evaluate_pin("constraints")
            .await
            .unwrap_or_default();

        let tool_params = json::json!({
            "type": "object",
            "properties": {
                "goal_understood": { "type": "boolean", "description": "Whether the goal was understood" },
                "current_state_assessment": { "type": "string", "description": "Assessment of current screen state" },
                "actions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action_type": { "type": "string", "description": "Type of action (click, type, scroll, etc.)" },
                            "target": { "type": "string", "description": "What to target (element description or coordinates)" },
                            "parameters": { "type": "object", "description": "Action-specific parameters" },
                            "reasoning": { "type": "string", "description": "Why this action is needed" },
                            "expected_result": { "type": "string", "description": "What should happen after this action" }
                        },
                        "required": ["action_type", "target", "reasoning"]
                    },
                    "description": "Ordered list of actions to execute"
                },
                "success_criteria": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "How to verify the goal was achieved"
                },
                "potential_obstacles": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Things that might go wrong"
                },
                "confidence": { "type": "number", "description": "Confidence in the plan 0-1" }
            },
            "required": ["goal_understood", "current_state_assessment", "actions", "confidence"]
        });

        let constraints_text = if constraints.is_empty() {
            String::new()
        } else {
            format!("\n\nConstraints: {}", constraints)
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
                    "Goal: {}\n\nAvailable actions: {}{}\n\nPlan a sequence of actions to achieve this goal.",
                    goal, available_actions, constraints_text
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

        let preamble = "You are an automation planning expert. Given a screenshot and a goal, create a detailed action plan. Each action should be specific and executable. Consider the current screen state and plan realistic, achievable steps.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(PlanActionsTool {
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

        let mut result: Option<ActionPlan> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_action_plan"
            {
                result = Some(json::from_value(arguments)?);
            }
        }

        let plan = result.unwrap_or(ActionPlan {
            goal_understood: false,
            current_state_assessment: "Could not assess".to_string(),
            actions: vec![],
            success_criteria: vec![],
            potential_obstacles: vec![],
            confidence: 0.0,
        });

        let first_action = plan.actions.first().cloned();

        context
            .set_pin_value("plan", json::json!(plan.clone()))
            .await?;
        context
            .set_pin_value("actions", json::json!(plan.actions))
            .await?;
        if let Some(action) = first_action {
            context
                .set_pin_value("first_action", json::json!(action))
                .await?;
        }

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
