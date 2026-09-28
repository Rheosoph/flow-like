use super::{COORDINATE_SPACE, add_screenshot_pins};
#[cfg(feature = "execute")]
use super::{
    ModelView, SubmitTool, call_tool, missing_tool_call, parse_tool_args, require_screenshot,
    vision_history,
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
pub struct NextStepSuggestion {
    pub action_type: String,
    pub target_description: String,
    /// Target point: desktop input coordinates when a frame was connected, otherwise
    /// screenshot pixels.
    pub target_coordinates: Option<(i32, i32)>,
    #[serde(default)]
    pub parameters: flow_like_types::Value,
    pub reasoning: String,
    pub confidence: f64,
    #[serde(default)]
    pub alternatives: Vec<AlternativeAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AlternativeAction {
    pub action_type: String,
    pub description: String,
    pub confidence: f64,
}

#[cfg(feature = "execute")]
const TOOL: &str = "submit_next_step";

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct NextStepArgs {
    goal_reached: bool,
    action_type: String,
    target_description: String,
    target_coordinates: Option<Vec<f64>>,
    #[serde(default)]
    parameters: flow_like_types::Value,
    reasoning: String,
    confidence: f64,
    #[serde(default)]
    alternatives: Vec<AlternativeAction>,
}

/// Returns whether the goal was reached and the suggestion with its target point mapped to
/// the node's coordinate space; a point off the screenshot is dropped, never passed on.
#[cfg(feature = "execute")]
fn to_suggestion(args: NextStepArgs, view: &ModelView) -> (bool, NextStepSuggestion) {
    let mut reasoning = args.reasoning;
    let target_coordinates = match args.target_coordinates.as_deref() {
        None | Some([]) => None,
        Some(&[x, y]) => match view.point_from_model(x, y) {
            Ok(point) => Some(point),
            Err(error) => {
                reasoning.push_str(&format!(" (target coordinates dropped: {error})"));
                None
            }
        },
        Some(other) => {
            reasoning.push_str(&format!(
                " (target coordinates dropped: expected [x, y], got {other:?})"
            ));
            None
        }
    };
    let parameters = if args.parameters.is_null() {
        json::json!({})
    } else {
        args.parameters
    };
    (
        args.goal_reached,
        NextStepSuggestion {
            action_type: args.action_type,
            target_description: args.target_description,
            target_coordinates,
            parameters,
            reasoning,
            confidence: args.confidence,
            alternatives: args.alternatives,
        },
    )
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
        node.set_flowscript_name("automation.llm", "suggestNextStep");
        node.add_icon("/flow/icons/bot-plan.svg");
        node.set_version(4);

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

        add_screenshot_pins(&mut node, "Current screenshot", true);

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
            &format!("Next step suggestion; target_coordinates are in {COORDINATE_SPACE}"),
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
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_goal_reached").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let goal: String = context.evaluate_pin("goal").await?;
        let completed_actions: String = context
            .evaluate_pin("completed_actions")
            .await
            .unwrap_or_else(|_| "[]".to_string());
        let last_result: String = context
            .evaluate_pin("last_result")
            .await
            .unwrap_or_default();
        let screenshot = require_screenshot(context).await?;

        let parameters = json::json!({
            "type": "object",
            "properties": {
                "goal_reached": { "type": "boolean", "description": "Whether the goal appears to be reached" },
                "action_type": { "type": "string", "description": "Type of action (click, type, scroll, wait, verify)" },
                "target_description": { "type": "string", "description": "What to interact with" },
                "target_coordinates": {
                    "type": "array",
                    "items": { "type": "number" },
                    "minItems": 2,
                    "maxItems": 2,
                    "description": "[x, y] pixel of the target in the screenshot, if applicable"
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
                        },
                        "required": ["action_type", "description", "confidence"]
                    },
                    "description": "Alternative actions to consider"
                }
            },
            "required": ["goal_reached", "action_type", "target_description", "reasoning", "confidence"]
        });

        let progress_context = if completed_actions.trim() == "[]" {
            "This is the first action.".to_string()
        } else {
            format!("Actions taken so far: {completed_actions}")
        };

        let last_result_text = if last_result.is_empty() {
            String::new()
        } else {
            format!("\nLast action result: {last_result}")
        };

        let instructions = format!(
            "Goal: {goal}\n\n{progress_context}{last_result_text}\n\nWhat should be the next action?\n\n{}",
            screenshot.view.coordinate_hint()
        );

        let preamble = "You are an intelligent automation assistant. Given the current screen, goal, and progress, suggest the single best next action. If the goal is already achieved, indicate that. Be precise about what to interact with.";

        let arguments = call_tool(
            context,
            &model_bit,
            vision_history(&[&screenshot.image], &instructions),
            preamble,
            SubmitTool {
                name: TOOL,
                description: "Submit the suggested next step",
                parameters,
            },
        )
        .await?
        .ok_or_else(|| missing_tool_call(TOOL))?;

        let (goal_reached, suggestion) =
            to_suggestion(parse_tool_args(TOOL, &arguments)?, &screenshot.view);

        context
            .set_pin_value("suggestion", json::json!(suggestion))
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

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::types::screen_frame::ScreenFrame;

    fn view() -> ModelView {
        let frame = ScreenFrame::new(None, (-1440, 0, 1440, 900), (2880, 1800)).unwrap();
        ModelView::new(frame.resized(1440, 900).unwrap())
    }

    fn suggest(arguments: flow_like_types::Value) -> (bool, NextStepSuggestion) {
        to_suggestion(parse_tool_args(TOOL, &arguments).unwrap(), &view())
    }

    #[test]
    fn target_coordinates_map_to_desktop_and_bad_ones_are_dropped() {
        let base = json::json!({"goal_reached": false, "action_type": "click", "target_description": "OK", "reasoning": "r", "confidence": 0.9});
        let (reached, suggestion) = suggest(base.clone());
        assert!(!reached);
        assert_eq!(suggestion.target_coordinates, None);
        assert_eq!(suggestion.parameters, json::json!({}));
        assert!(suggestion.alternatives.is_empty());

        let mut with_point = base.clone();
        with_point["target_coordinates"] = json::json!([100.6, 20]);
        assert_eq!(suggest(with_point).1.target_coordinates, Some((-1339, 20)));

        let mut off_image = base.clone();
        off_image["target_coordinates"] = json::json!([5000, 20]);
        let (_, dropped) = suggest(off_image);
        assert_eq!(dropped.target_coordinates, None);
        assert!(dropped.reasoning.contains("dropped"));

        let mut malformed = base;
        malformed["target_coordinates"] = json::json!([1, 2, 3]);
        assert_eq!(suggest(malformed).1.target_coordinates, None);
    }
}
