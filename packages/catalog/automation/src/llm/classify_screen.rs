use super::add_screenshot_pins;
#[cfg(feature = "execute")]
use super::{
    SubmitTool, call_tool, missing_tool_call, parse_tool_args, require_screenshot, vision_history,
};
use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScreenClassification {
    pub screen_type: String,
    pub app_name: Option<String>,
    pub state: String,
    pub visible_elements: Vec<String>,
    #[serde(default)]
    pub suggested_actions: Vec<String>,
    pub confidence: f64,
}

#[cfg(feature = "execute")]
const TOOL: &str = "submit_classification";

#[crate::register_node]
#[derive(Default)]
pub struct LLMClassifyScreenNode {}

impl LLMClassifyScreenNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMClassifyScreenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_classify_screen",
            "LLM Classify Screen",
            "Uses vision LLM to classify screen state and identify visible elements",
            "Automation/LLM/Vision",
        );
        node.set_flowscript_name("automation.llm", "classifyScreen");
        node.add_icon("/flow/icons/bot-search.svg");
        node.set_version(5);

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

        add_screenshot_pins(&mut node, "Screenshot", false);

        node.add_input_pin(
            "expected_states",
            "Expected States",
            "Comma-separated list of possible states to classify into",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "classification",
            "Classification",
            "Screen classification result",
            VariableType::Struct,
        )
        .set_schema::<ScreenClassification>();

        node.add_output_pin(
            "screen_type",
            "Screen Type",
            "Detected screen type",
            VariableType::String,
        );

        node.add_output_pin(
            "state",
            "State",
            "Current screen state",
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let expected_states: String = context
            .evaluate_pin("expected_states")
            .await
            .unwrap_or_default();
        let screenshot = require_screenshot(context).await?;

        let parameters = json::json!({
            "type": "object",
            "properties": {
                "screen_type": { "type": "string", "description": "Type of screen (e.g., login, dashboard, form, error, loading)" },
                "app_name": { "type": "string", "description": "Name of the application if identifiable" },
                "state": { "type": "string", "description": "Current state of the screen" },
                "visible_elements": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of key visible UI elements"
                },
                "suggested_actions": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Suggested next actions based on screen state"
                },
                "confidence": { "type": "number", "description": "Confidence score 0-1" }
            },
            "required": ["screen_type", "state", "visible_elements", "confidence"]
        });

        let instructions = if expected_states.is_empty() {
            "Analyze this screenshot and classify the screen type and current state.".to_string()
        } else {
            format!(
                "Analyze this screenshot and classify into one of these states: {expected_states}"
            )
        };

        let preamble = "You are a screen analysis expert. Analyze screenshots to identify the type of screen, its current state, and visible UI elements. Be precise and thorough.";

        let arguments = call_tool(
            context,
            &model_bit,
            vision_history(&[&screenshot.image], &instructions),
            preamble,
            SubmitTool {
                name: TOOL,
                description: "Submit the screen classification result",
                parameters,
            },
        )
        .await?
        .ok_or_else(|| missing_tool_call(TOOL))?;

        let classification: ScreenClassification = parse_tool_args(TOOL, &arguments)?;

        context
            .set_pin_value("classification", json::json!(classification))
            .await?;
        context
            .set_pin_value("screen_type", json::json!(classification.screen_type))
            .await?;
        context
            .set_pin_value("state", json::json!(classification.state))
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

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;

    #[test]
    fn classification_without_optional_fields_parses() {
        let classification: ScreenClassification = parse_tool_args(
            TOOL,
            &json::json!({"screen_type": "login", "state": "idle", "visible_elements": [], "confidence": 0.8}),
        )
        .unwrap();
        assert!(classification.app_name.is_none() && classification.suggested_actions.is_empty());
    }
}
