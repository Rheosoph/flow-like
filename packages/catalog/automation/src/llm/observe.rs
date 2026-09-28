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
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{async_trait, json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScreenObservation {
    pub description: String,
    pub app_context: String,
    pub interactive_elements: Vec<ObservedElement>,
    #[serde(default)]
    pub text_content: Vec<String>,
    #[serde(default)]
    pub notable_features: Vec<String>,
    #[serde(default)]
    pub possible_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ObservedElement {
    pub element_type: String,
    pub description: String,
    pub approximate_location: String,
    pub is_interactive: bool,
    pub current_state: Option<String>,
    /// Center x: desktop input coordinates when a frame was connected, otherwise screenshot
    /// pixels. Absent when the model gave no point on the screenshot.
    #[serde(default)]
    pub x: Option<i32>,
    /// Center y, in the same space as `x`.
    #[serde(default)]
    pub y: Option<i32>,
}

#[cfg(feature = "execute")]
const OBSERVE_TOOL: &str = "submit_observation";
#[cfg(feature = "execute")]
const DESCRIBE_TOOL: &str = "submit_element_description";

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct ObservationArgs {
    description: String,
    app_context: String,
    interactive_elements: Vec<ObservedElementArgs>,
    #[serde(default)]
    text_content: Vec<String>,
    #[serde(default)]
    notable_features: Vec<String>,
    #[serde(default)]
    possible_actions: Vec<String>,
}

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct ObservedElementArgs {
    element_type: String,
    description: String,
    approximate_location: String,
    is_interactive: bool,
    current_state: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
}

#[cfg(feature = "execute")]
fn to_observation(args: ObservationArgs, view: &ModelView) -> ScreenObservation {
    let interactive_elements = args
        .interactive_elements
        .into_iter()
        .map(|element| {
            let point = element
                .x
                .zip(element.y)
                .and_then(|(x, y)| view.point_from_model(x, y).ok());
            ObservedElement {
                element_type: element.element_type,
                description: element.description,
                approximate_location: element.approximate_location,
                is_interactive: element.is_interactive,
                current_state: element.current_state,
                x: point.map(|(x, _)| x),
                y: point.map(|(_, y)| y),
            }
        })
        .collect();
    ScreenObservation {
        description: args.description,
        app_context: args.app_context,
        interactive_elements,
        text_content: args.text_content,
        notable_features: args.notable_features,
        possible_actions: args.possible_actions,
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMObserveScreenNode {}

impl LLMObserveScreenNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMObserveScreenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_observe_screen",
            "LLM Observe Screen",
            "Uses vision LLM to comprehensively observe and describe the current screen",
            "Automation/LLM/Vision",
        );
        node.set_flowscript_name("automation.llm", "observeScreen");
        node.add_icon("/flow/icons/bot-search.svg");
        node.set_version(5);

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

        add_screenshot_pins(&mut node, "Screenshot", true);

        node.add_input_pin(
            "focus_area",
            "Focus Area",
            "Specific area or aspect to focus on (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "observation",
            "Observation",
            "Complete screen observation",
            VariableType::Struct,
        )
        .set_schema::<ScreenObservation>();

        node.add_output_pin(
            "description",
            "Description",
            "Text description of the screen",
            VariableType::String,
        );

        node.add_output_pin(
            "elements",
            "Elements",
            &format!(
                "Observed elements. Optional x/y is the element's center in {COORDINATE_SPACE}"
            ),
            VariableType::Struct,
        )
        .set_schema::<ObservedElement>()
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let focus_area: String = context.evaluate_pin("focus_area").await.unwrap_or_default();
        let screenshot = require_screenshot(context).await?;

        let parameters = json::json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "Overall description of what's on screen" },
                "app_context": { "type": "string", "description": "What application/website/context this appears to be" },
                "interactive_elements": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "element_type": { "type": "string", "description": "Type: button, input, link, dropdown, etc." },
                            "description": { "type": "string", "description": "What the element is for" },
                            "approximate_location": { "type": "string", "description": "Where on screen (top-left, center, etc.)" },
                            "is_interactive": { "type": "boolean" },
                            "current_state": { "type": "string", "description": "enabled, disabled, selected, etc." },
                            "x": { "type": "number", "description": "X pixel of the element center in the screenshot" },
                            "y": { "type": "number", "description": "Y pixel of the element center in the screenshot" }
                        },
                        "required": ["element_type", "description", "approximate_location", "is_interactive"]
                    }
                },
                "text_content": { "type": "array", "items": { "type": "string" }, "description": "Notable text visible on screen" },
                "notable_features": { "type": "array", "items": { "type": "string" }, "description": "Other notable visual features" },
                "possible_actions": { "type": "array", "items": { "type": "string" }, "description": "Actions that appear possible from this screen" }
            },
            "required": ["description", "app_context", "interactive_elements"]
        });

        let request = if focus_area.is_empty() {
            "Observe this screen comprehensively. Identify all interactive elements, text content, and possible actions.".to_string()
        } else {
            format!("Observe this screen, focusing especially on: {focus_area}")
        };
        let instructions = format!("{request}\n\n{}", screenshot.view.coordinate_hint());

        let preamble = "You are a screen observation expert. Analyze screenshots thoroughly to identify all UI elements, their purposes, states, and possible interactions. Be comprehensive but precise.";

        let arguments = call_tool(
            context,
            &model_bit,
            vision_history(&[&screenshot.image], &instructions),
            preamble,
            SubmitTool {
                name: OBSERVE_TOOL,
                description: "Submit screen observation",
                parameters,
            },
        )
        .await?
        .ok_or_else(|| missing_tool_call(OBSERVE_TOOL))?;

        let observation =
            to_observation(parse_tool_args(OBSERVE_TOOL, &arguments)?, &screenshot.view);

        context
            .set_pin_value("observation", json::json!(observation))
            .await?;
        context
            .set_pin_value("description", json::json!(observation.description))
            .await?;
        context
            .set_pin_value("elements", json::json!(observation.interactive_elements))
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ElementDescription {
    pub element_type: String,
    pub visual_description: String,
    pub purpose: String,
    pub current_state: String,
    pub text_content: Option<String>,
    pub accessibility_info: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMDescribeElementNode {}

impl LLMDescribeElementNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMDescribeElementNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_describe_element",
            "LLM Describe Element",
            "Uses vision LLM to describe a specific UI element at given coordinates",
            "Automation/LLM/Vision",
        );
        node.set_flowscript_name("automation.llm", "describeElement");
        node.add_icon("/flow/icons/bot-search.svg");
        node.set_version(4);

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(5)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(4)
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

        add_screenshot_pins(&mut node, "Screenshot", true);

        node.add_input_pin(
            "x",
            "X",
            &format!("X coordinate of the element, in {COORDINATE_SPACE}"),
            VariableType::Integer,
        );

        node.add_input_pin(
            "y",
            "Y",
            &format!("Y coordinate of the element, in {COORDINATE_SPACE}"),
            VariableType::Integer,
        );

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "element",
            "Element",
            "Element description",
            VariableType::Struct,
        )
        .set_schema::<ElementDescription>();

        node.add_output_pin(
            "description",
            "Description",
            "Text description",
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let screenshot = require_screenshot(context).await?;

        let (model_x, model_y) = i32::try_from(x)
            .ok()
            .zip(i32::try_from(y).ok())
            .and_then(|(x, y)| screenshot.view.point_to_model(x, y))
            .ok_or_else(|| {
                let (width, height) = screenshot.view.size();
                anyhow!(
                    "Point ({x}, {y}) lies outside the screenshot (sent to the model at {width}x{height}); x/y must be in {COORDINATE_SPACE}"
                )
            })?;

        let parameters = json::json!({
            "type": "object",
            "properties": {
                "element_type": { "type": "string", "description": "Type of element (button, input, text, image, etc.)" },
                "visual_description": { "type": "string", "description": "Visual appearance description" },
                "purpose": { "type": "string", "description": "What the element is for" },
                "current_state": { "type": "string", "description": "Current state (enabled, disabled, focused, etc.)" },
                "text_content": { "type": "string", "description": "Any text in or on the element" },
                "accessibility_info": { "type": "string", "description": "Inferred accessibility information" }
            },
            "required": ["element_type", "visual_description", "purpose", "current_state"]
        });

        let (width, height) = screenshot.view.size();
        let instructions = format!(
            "Describe the UI element at pixel ({model_x}, {model_y}) of this {width}x{height} screenshot (origin at the top-left corner)."
        );

        let preamble = "You are a UI element expert. Given coordinates on a screenshot, identify and describe the element at or near that location.";

        let arguments = call_tool(
            context,
            &model_bit,
            vision_history(&[&screenshot.image], &instructions),
            preamble,
            SubmitTool {
                name: DESCRIBE_TOOL,
                description: "Submit element description",
                parameters,
            },
        )
        .await?
        .ok_or_else(|| missing_tool_call(DESCRIBE_TOOL))?;

        let element: ElementDescription = parse_tool_args(DESCRIBE_TOOL, &arguments)?;

        context
            .set_pin_value("element", json::json!(element))
            .await?;
        context
            .set_pin_value("description", json::json!(element.visual_description))
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
    use crate::types::screen_frame::ScreenFrame;

    #[test]
    fn observation_without_optional_lists_parses_and_maps_points() {
        let arguments = json::json!({
            "description": "Login form",
            "app_context": "Browser",
            "interactive_elements": [
                { "element_type": "button", "description": "Sign in", "approximate_location": "center", "is_interactive": true, "x": 100, "y": 50.5 },
                { "element_type": "link", "description": "Help", "approximate_location": "bottom", "is_interactive": true, "x": 5000, "y": 10 },
                { "element_type": "text", "description": "Title", "approximate_location": "top", "is_interactive": false }
            ]
        });
        let args: ObservationArgs = parse_tool_args(OBSERVE_TOOL, &arguments).unwrap();
        let frame = ScreenFrame::new(None, (0, 0, 800, 600), (1600, 1200)).unwrap();
        let observation = to_observation(args, &ModelView::new(frame.resized(800, 600).unwrap()));
        let points: Vec<_> = observation
            .interactive_elements
            .iter()
            .map(|e| (e.x, e.y))
            .collect();
        assert_eq!(
            points,
            vec![(Some(100), Some(51)), (None, None), (None, None)]
        );
        assert!(observation.text_content.is_empty() && observation.possible_actions.is_empty());
    }

    #[test]
    fn element_description_without_optional_fields_parses() {
        let element: ElementDescription = parse_tool_args(
            DESCRIBE_TOOL,
            &json::json!({"element_type": "button", "visual_description": "blue", "purpose": "submit", "current_state": "enabled"}),
        )
        .unwrap();
        assert!(element.text_content.is_none());
    }
}
