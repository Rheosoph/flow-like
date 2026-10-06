use super::{COORDINATE_SPACE, add_screenshot_pins};
#[cfg(feature = "execute")]
use super::{
    ModelView, SubmitTool, call_tool, parse_tool_args, require_screenshot, vision_history,
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
pub struct ElementLocation {
    pub found: bool,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub confidence: f64,
    pub description: String,
    pub selector_hint: Option<String>,
}

#[cfg(feature = "execute")]
const TOOL: &str = "submit_element_location";

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct LocateArgs {
    found: bool,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    confidence: f64,
    description: String,
    selector_hint: Option<String>,
}

/// Maps the model's answer into the node's coordinate space. A "found" answer without a point
/// on the screenshot becomes not found, never a guessed location.
#[cfg(feature = "execute")]
fn locate(args: LocateArgs, view: &ModelView) -> ElementLocation {
    let mut location = ElementLocation {
        found: false,
        x: None,
        y: None,
        width: None,
        height: None,
        confidence: args.confidence,
        description: args.description,
        selector_hint: args.selector_hint,
    };
    if !args.found {
        return location;
    }
    let (Some(x), Some(y)) = (args.x, args.y) else {
        location.description = format!(
            "Model reported the element as found without coordinates: {}",
            location.description
        );
        return location;
    };
    match view.point_from_model(x, y) {
        Ok((x, y)) => {
            location.found = true;
            location.x = Some(x);
            location.y = Some(y);
            location.width = args.width.and_then(|w| view.width_from_model(w));
            location.height = args.height.and_then(|h| view.height_from_model(h));
        }
        Err(error) => {
            location.description = format!("{error}: {}", location.description);
        }
    }
    location
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMFindElementNode {}

impl LLMFindElementNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMFindElementNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_find_element",
            "LLM Find Element",
            "Uses a vision LLM to locate UI elements based on natural language description",
            "Automation/LLM/Vision",
        );
        node.set_flowscript_name("automation.llm", "findElement");
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

        add_screenshot_pins(&mut node, "Screenshot of the screen", true);

        node.add_input_pin(
            "description",
            "Description",
            "Natural language description of the element to find (e.g., 'the blue submit button')",
            VariableType::String,
        );

        node.add_input_pin(
            "context",
            "Context",
            "Optional context about the application or page",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "Element not found, or the model did not give a point on the screenshot",
            VariableType::Execution,
        );

        node.add_output_pin(
            "location",
            "Location",
            &format!(
                "Element location. x/y is the element's center and width/height its size, in {COORDINATE_SPACE}"
            ),
            VariableType::Struct,
        )
        .set_schema::<ElementLocation>();

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like::flow::execution::LogLevel;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let description: String = context.evaluate_pin("description").await?;
        let ctx: String = context.evaluate_pin("context").await.unwrap_or_default();
        let screenshot = require_screenshot(context).await?;

        let parameters = json::json!({
            "type": "object",
            "properties": {
                "found": { "type": "boolean", "description": "Whether the element was found" },
                "x": { "type": "number", "description": "X pixel of the element center in the screenshot" },
                "y": { "type": "number", "description": "Y pixel of the element center in the screenshot" },
                "width": { "type": "number", "description": "Estimated width of the element in pixels" },
                "height": { "type": "number", "description": "Estimated height of the element in pixels" },
                "confidence": { "type": "number", "description": "Confidence score 0-1" },
                "description": { "type": "string", "description": "Description of what was found" },
                "selector_hint": { "type": "string", "description": "Suggested CSS/accessibility selector if identifiable" }
            },
            "required": ["found", "confidence", "description"]
        });

        let context_line = if ctx.is_empty() {
            String::new()
        } else {
            format!("\nContext: {ctx}")
        };
        let instructions = format!(
            "Find the UI element matching this description: \"{description}\"{context_line}\n\n{}",
            screenshot.view.coordinate_hint()
        );

        let preamble = "You are a UI element locator. Analyze the screenshot and find the element matching the description. Return precise pixel coordinates for the element's center position. If you cannot find the element, set found=false.";

        let arguments = call_tool(
            context,
            &model_bit,
            vision_history(&[&screenshot.image], &instructions),
            preamble,
            SubmitTool {
                name: TOOL,
                description: "Submit the located element coordinates and details",
                parameters,
            },
        )
        .await?;

        let location = match arguments {
            Some(arguments) => locate(parse_tool_args(TOOL, &arguments)?, &screenshot.view),
            None => {
                context.log_message(
                    &format!(
                        "Model answered without calling `{TOOL}`; treating the element as not found"
                    ),
                    LogLevel::Warn,
                );
                ElementLocation {
                    found: false,
                    x: None,
                    y: None,
                    width: None,
                    height: None,
                    confidence: 0.0,
                    description: format!("The model answered without calling `{TOOL}`"),
                    selector_hint: None,
                }
            }
        };

        context
            .set_pin_value("location", json::json!(location))
            .await?;

        if location.found {
            context.activate_exec_pin("exec_out").await?;
        } else {
            context.activate_exec_pin("exec_not_found").await?;
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

    fn args(found: bool, x: Option<f64>, y: Option<f64>) -> LocateArgs {
        LocateArgs {
            found,
            x,
            y,
            width: Some(100.0),
            height: Some(20.0),
            confidence: 0.9,
            description: "Submit".into(),
            selector_hint: None,
        }
    }

    #[test]
    fn found_point_maps_to_desktop_and_off_image_points_are_not_found() {
        let frame = ScreenFrame::new(None, (100, 50, 1440, 900), (2880, 1800)).unwrap();
        let view = ModelView::new(frame.resized(1440, 900).unwrap());
        let location = locate(args(true, Some(720.0), Some(450.0)), &view);
        assert!(location.found);
        assert_eq!((location.x, location.y), (Some(820), Some(500)));
        assert_eq!((location.width, location.height), (Some(100), Some(20)));

        assert!(!locate(args(true, Some(1500.0), Some(10.0)), &view).found);
        assert!(!locate(args(true, None, Some(10.0)), &view).found);
        assert!(!locate(args(false, Some(1.0), Some(1.0)), &view).found);
    }

    #[test]
    fn omitted_optional_arguments_deserialize() {
        let args: LocateArgs = parse_tool_args(
            TOOL,
            &json::json!({"found": false, "confidence": 0.1, "description": "none"}),
        )
        .unwrap();
        assert!(args.x.is_none() && args.selector_hint.is_none());
    }
}
