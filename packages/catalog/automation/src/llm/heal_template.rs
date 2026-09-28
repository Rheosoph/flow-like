use super::{COORDINATE_SPACE, add_screenshot_pins};
#[cfg(feature = "execute")]
use super::{
    ImageSource, ModelView, SubmitTool, call_tool, parse_point, parse_tool_args,
    prepare_screenshot, require_screenshot, vision_history,
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
pub struct HealedTemplate {
    pub healed: bool,
    pub found_at_x: Option<i32>,
    pub found_at_y: Option<i32>,
    pub confidence: f64,
    pub reasoning: String,
    pub suggested_region: Option<TemplateRegion>,
    #[serde(default)]
    pub visual_changes_detected: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemplateRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(feature = "execute")]
const TOOL: &str = "submit_healed_template";

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct HealTemplateArgs {
    healed: bool,
    found_at_x: Option<f64>,
    found_at_y: Option<f64>,
    confidence: f64,
    reasoning: String,
    suggested_region: Option<RegionArgs>,
    #[serde(default)]
    visual_changes_detected: Vec<String>,
}

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct RegionArgs {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(feature = "execute")]
fn map_region(region: &RegionArgs, view: &ModelView) -> Option<TemplateRegion> {
    let (x, y) = view.point_from_model(region.x, region.y).ok()?;
    Some(TemplateRegion {
        x,
        y,
        width: view.width_from_model(region.width).filter(|w| *w > 0)?,
        height: view.height_from_model(region.height).filter(|h| *h > 0)?,
    })
}

/// "Healed" needs a point on the screenshot; it is mapped to the node's coordinate space.
#[cfg(feature = "execute")]
fn to_healed(args: HealTemplateArgs, view: &ModelView) -> HealedTemplate {
    let mut healed = HealedTemplate {
        healed: false,
        found_at_x: None,
        found_at_y: None,
        confidence: args.confidence,
        reasoning: args.reasoning,
        suggested_region: args
            .suggested_region
            .as_ref()
            .and_then(|region| map_region(region, view)),
        visual_changes_detected: args.visual_changes_detected,
    };
    if !args.healed {
        return healed;
    }
    let point = match (args.found_at_x, args.found_at_y) {
        (Some(x), Some(y)) => view.point_from_model(x, y),
        _ => Err(anyhow!(
            "Model reported the element as found without coordinates"
        )),
    };
    match point {
        Ok((x, y)) => {
            healed.healed = true;
            healed.found_at_x = Some(x);
            healed.found_at_y = Some(y);
        }
        Err(error) => healed.reasoning = format!("{error}: {}", healed.reasoning),
    }
    healed
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMHealTemplateNode {}

impl LLMHealTemplateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMHealTemplateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_heal_template",
            "LLM Heal Template",
            "Uses vision LLM to find a visually similar element when template matching fails",
            "Automation/LLM/Healing",
        );
        node.set_flowscript_name("automation.llm", "healTemplate");
        node.add_icon("/flow/icons/bot-fix.svg");
        node.set_version(4);

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

        add_screenshot_pins(&mut node, "Current screenshot", true);

        node.add_input_pin(
            "template",
            "Template",
            "Base64-encoded template image (PNG, JPEG, WebP or GIF) that failed to match",
            VariableType::String,
        );

        node.add_input_pin(
            "element_description",
            "Element Description",
            "Description of what the template represents",
            VariableType::String,
        );

        node.add_input_pin(
            "last_known_position",
            "Last Known Position",
            &format!("Where the element was previously found (x,y in {COORDINATE_SPACE})"),
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "exec_failed",
            "Failed",
            "Could not heal, or the model gave no point on the screenshot",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            &format!("Healed template result; points and regions are in {COORDINATE_SPACE}"),
            VariableType::Struct,
        )
        .set_schema::<HealedTemplate>();

        node.add_output_pin(
            "x",
            "X",
            &format!("X coordinate of the found element's center, in {COORDINATE_SPACE}"),
            VariableType::Integer,
        );

        node.add_output_pin(
            "y",
            "Y",
            &format!("Y coordinate of the found element's center, in {COORDINATE_SPACE}"),
            VariableType::Integer,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_failed").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let template: String = context.evaluate_pin("template").await?;
        let element_description: String = context.evaluate_pin("element_description").await?;
        let last_known: String = context
            .evaluate_pin("last_known_position")
            .await
            .unwrap_or_default();
        let screenshot = require_screenshot(context).await?;
        let template = tokio::task::spawn_blocking(move || {
            prepare_screenshot(ImageSource::Encoded(template), None)
        })
        .await
        .map_err(|e| anyhow!("Template preparation task failed: {e}"))?
        .map_err(|e| anyhow!("Template image: {e}"))?
        .image;

        let parameters = json::json!({
            "type": "object",
            "properties": {
                "healed": { "type": "boolean", "description": "Whether the element was found" },
                "found_at_x": { "type": "number", "description": "X pixel of the found element's center in the first image" },
                "found_at_y": { "type": "number", "description": "Y pixel of the found element's center in the first image" },
                "confidence": { "type": "number", "description": "Confidence score 0-1" },
                "reasoning": { "type": "string", "description": "Explanation of how the element was identified" },
                "suggested_region": {
                    "type": "object",
                    "properties": {
                        "x": { "type": "number", "description": "Left edge in pixels of the first image" },
                        "y": { "type": "number", "description": "Top edge in pixels of the first image" },
                        "width": { "type": "number" },
                        "height": { "type": "number" }
                    },
                    "required": ["x", "y", "width", "height"],
                    "description": "Suggested region for new template capture"
                },
                "visual_changes_detected": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of visual changes that might have caused match failure"
                }
            },
            "required": ["healed", "confidence", "reasoning"]
        });

        let position_hint = if last_known.is_empty() {
            String::new()
        } else {
            let position = parse_point(&last_known)
                .and_then(|(x, y)| {
                    screenshot
                        .view
                        .point_to_model(x.round() as i32, y.round() as i32)
                })
                .map_or_else(
                    || last_known.clone(),
                    |(x, y)| format!("{x},{y} (pixels of the first image)"),
                );
            format!("\nThe element was previously located at: {position}")
        };

        let instructions = format!(
            "The first image is the current screen. The second image is a template that failed to match.\nElement description: {element_description}{position_hint}\n\nFind where this element is now located on screen, accounting for possible visual changes. {}",
            screenshot.view.coordinate_hint()
        );

        let preamble = "You are a visual UI analysis expert. When template matching fails due to visual changes (scaling, color changes, minor layout shifts), you can identify the same logical element by understanding its purpose and visual characteristics.";

        let arguments = call_tool(
            context,
            &model_bit,
            vision_history(&[&screenshot.image, &template], &instructions),
            preamble,
            SubmitTool {
                name: TOOL,
                description: "Submit the healed template match result",
                parameters,
            },
        )
        .await?;

        let healed = match arguments {
            Some(arguments) => to_healed(parse_tool_args(TOOL, &arguments)?, &screenshot.view),
            None => HealedTemplate {
                healed: false,
                found_at_x: None,
                found_at_y: None,
                confidence: 0.0,
                reasoning: format!("The model answered without calling `{TOOL}`"),
                suggested_region: None,
                visual_changes_detected: vec![],
            },
        };

        context.set_pin_value("result", json::json!(healed)).await?;
        context
            .set_pin_value("x", json::json!(healed.found_at_x.unwrap_or(0)))
            .await?;
        context
            .set_pin_value("y", json::json!(healed.found_at_y.unwrap_or(0)))
            .await?;

        if healed.healed {
            context.activate_exec_pin("exec_out").await?;
        } else {
            context.activate_exec_pin("exec_failed").await?;
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
        let frame = ScreenFrame::new(Some(0), (0, 0, 1440, 900), (2880, 1800)).unwrap();
        ModelView::new(frame.resized(1440, 900).unwrap())
    }

    #[test]
    fn healed_point_and_region_map_to_desktop_input() {
        let args: HealTemplateArgs = parse_tool_args(
            TOOL,
            &json::json!({
                "healed": true, "found_at_x": 300.4, "found_at_y": 200, "confidence": 0.8, "reasoning": "moved",
                "suggested_region": { "x": 280, "y": 190, "width": 40, "height": 20 }
            }),
        )
        .unwrap();
        let healed = to_healed(args, &view());
        assert!(healed.healed);
        assert_eq!(
            (healed.found_at_x, healed.found_at_y),
            (Some(300), Some(200))
        );
        let region = healed.suggested_region.unwrap();
        assert_eq!(
            (region.x, region.y, region.width, region.height),
            (280, 190, 40, 20)
        );
    }

    #[test]
    fn healed_without_a_point_on_screen_fails() {
        let args: HealTemplateArgs = parse_tool_args(
            TOOL,
            &json::json!({"healed": true, "found_at_x": 2000, "found_at_y": 10, "confidence": 0.8, "reasoning": "r"}),
        )
        .unwrap();
        assert!(!to_healed(args, &view()).healed);
        let args: HealTemplateArgs = parse_tool_args(
            TOOL,
            &json::json!({"healed": true, "confidence": 0.8, "reasoning": "r"}),
        )
        .unwrap();
        let healed = to_healed(args, &view());
        assert!(!healed.healed && healed.visual_changes_detected.is_empty());
    }
}
