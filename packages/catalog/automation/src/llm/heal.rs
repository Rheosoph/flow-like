use super::{COORDINATE_SPACE, add_screenshot_pins};
#[cfg(feature = "execute")]
use super::{
    ModelView, SubmitTool, call_tool, load_screenshot, parse_point, parse_tool_args,
    truncate_on_char_boundary, vision_history,
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
pub struct FailureDiagnosis {
    pub failure_type: String,
    pub root_cause: String,
    pub severity: String,
    pub recoverable: bool,
    #[serde(default)]
    pub recommended_actions: Vec<String>,
    #[serde(default)]
    pub context_clues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HealingResult {
    pub healed: bool,
    pub diagnosis: FailureDiagnosis,
    pub healing_action: Option<String>,
    pub new_value: Option<String>,
    pub confidence: f64,
}

#[cfg(feature = "execute")]
const TOOL: &str = "submit_diagnosis_and_healing";
#[cfg(feature = "execute")]
const MAX_HTML_BYTES: usize = 30_000;

/// A target given as a point in the node's coordinate space, rewritten as pixels of the image
/// the model sees.
#[cfg(feature = "execute")]
fn target_for_model(target: &str, view: Option<&ModelView>) -> String {
    let (Some(view), Some((x, y))) = (view, parse_point(target)) else {
        return target.to_string();
    };
    match view.point_to_model(x.round() as i32, y.round() as i32) {
        Some((x, y)) => format!("{x},{y} (pixels of the screenshot)"),
        None => format!("{target} (outside the screenshot)"),
    }
}

/// A healed point comes back in pixels of the model's image; it is mapped to the node's
/// coordinate space, and a point off the screenshot means healing failed.
#[cfg(feature = "execute")]
fn map_healed_point(mut healing: HealingResult, view: Option<&ModelView>) -> HealingResult {
    let point = healing.new_value.as_deref().and_then(parse_point);
    let (Some(view), Some((x, y))) = (view, point) else {
        return healing;
    };
    match view.point_from_model(x, y) {
        Ok((x, y)) => healing.new_value = Some(format!("{x},{y}")),
        Err(error) => {
            healing.healed = false;
            healing.new_value = None;
            healing.diagnosis.root_cause = format!(
                "{} (healed point rejected: {error})",
                healing.diagnosis.root_cause
            );
        }
    }
    healing
}

#[cfg(feature = "execute")]
fn not_healed(root_cause: String) -> HealingResult {
    HealingResult {
        healed: false,
        diagnosis: FailureDiagnosis {
            failure_type: "unknown".to_string(),
            root_cause,
            severity: "high".to_string(),
            recoverable: false,
            recommended_actions: vec![],
            context_clues: vec![],
        },
        healing_action: None,
        new_value: None,
        confidence: 0.0,
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMDiagnoseAndHealNode {}

impl LLMDiagnoseAndHealNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMDiagnoseAndHealNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_diagnose_and_heal",
            "LLM Diagnose & Heal",
            "Uses LLM to diagnose automation failures and suggest/apply healing actions",
            "Automation/LLM/Healing",
        );
        node.set_flowscript_name("automation.llm", "diagnoseAndHeal");
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
            "LLM model (vision-capable preferred)",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        add_screenshot_pins(
            &mut node,
            "Optional screenshot at the time of failure",
            true,
        );

        node.add_input_pin(
            "error_message",
            "Error Message",
            "The error message from the failed action",
            VariableType::String,
        );

        node.add_input_pin(
            "action_type",
            "Action Type",
            "Type of action that failed (click, type, wait, find, etc.)",
            VariableType::String,
        );

        node.add_input_pin(
            "action_target",
            "Action Target",
            &format!(
                "The target of the failed action (selector, text, or an x,y point in {COORDINATE_SPACE})"
            ),
            VariableType::String,
        );

        node.add_input_pin(
            "context",
            "Context",
            "Additional context about what the automation was trying to do",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_input_pin(
            "page_html",
            "Page HTML",
            "Current page HTML (for selector-based failures); only the first 30,000 bytes are sent",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin(
            "exec_out",
            "▶",
            "Continue (Healed)",
            VariableType::Execution,
        );

        node.add_output_pin(
            "exec_failed",
            "Failed",
            "Could not heal",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Full healing result",
            VariableType::Struct,
        )
        .set_schema::<HealingResult>();

        node.add_output_pin(
            "diagnosis",
            "Diagnosis",
            "Failure diagnosis",
            VariableType::Struct,
        )
        .set_schema::<FailureDiagnosis>();

        node.add_output_pin(
            "new_value",
            "New Value",
            &format!("Healed value (new selector, text, or an x,y point in {COORDINATE_SPACE})"),
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_failed").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let error_message: String = context.evaluate_pin("error_message").await?;
        let action_type: String = context.evaluate_pin("action_type").await?;
        let action_target: String = context.evaluate_pin("action_target").await?;
        let ctx: String = context.evaluate_pin("context").await.unwrap_or_default();
        let page_html: String = context.evaluate_pin("page_html").await.unwrap_or_default();
        let screenshot = load_screenshot(context).await?;
        let view = screenshot.as_ref().map(|screenshot| &screenshot.view);

        let parameters = json::json!({
            "type": "object",
            "properties": {
                "healed": { "type": "boolean", "description": "Whether healing was successful" },
                "diagnosis": {
                    "type": "object",
                    "properties": {
                        "failure_type": { "type": "string", "description": "Type: element_not_found, timeout, stale_element, etc." },
                        "root_cause": { "type": "string", "description": "Root cause analysis" },
                        "severity": { "type": "string", "description": "low, medium, high, critical" },
                        "recoverable": { "type": "boolean", "description": "Whether the failure is recoverable" },
                        "recommended_actions": { "type": "array", "items": { "type": "string" } },
                        "context_clues": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["failure_type", "root_cause", "severity", "recoverable"]
                },
                "healing_action": { "type": "string", "description": "The healing action taken" },
                "new_value": { "type": "string", "description": "New selector, text or value to use; a screen position as x,y pixels of the screenshot" },
                "confidence": { "type": "number", "description": "Confidence in the healing 0-1" }
            },
            "required": ["healed", "diagnosis", "confidence"]
        });

        let html_section = if page_html.len() > MAX_HTML_BYTES {
            format!(
                "\n\nPage HTML (truncated):\n{}...",
                truncate_on_char_boundary(&page_html, MAX_HTML_BYTES)
            )
        } else if !page_html.is_empty() {
            format!("\n\nPage HTML:\n{page_html}")
        } else {
            String::new()
        };
        let coordinate_hint = view
            .map(|view| format!("\n{}", view.coordinate_hint()))
            .unwrap_or_default();
        let context_text = if ctx.is_empty() {
            "None provided"
        } else {
            &ctx
        };

        let instructions = format!(
            "Automation Failure:\nAction Type: {action_type}\nAction Target: {}\nError: {error_message}\nContext: {context_text}{html_section}\n\nDiagnose the failure and provide a healing solution if possible.{coordinate_hint}",
            target_for_model(&action_target, view)
        );

        let preamble = "You are an automation failure diagnosis and self-healing expert. Analyze failures and provide actionable healing solutions. Common failure types include:\n\
        - element_not_found: Selector/element no longer exists or changed\n\
        - timeout: Operation took too long\n\
        - stale_element: Element reference became invalid\n\
        - visibility: Element exists but not visible/interactable\n\
        - state_change: Application state changed unexpectedly\n\
        - template_mismatch: Visual template no longer matches\n\
        Provide specific, actionable healing solutions when possible.";

        let images: Vec<_> = screenshot
            .iter()
            .map(|screenshot| &screenshot.image)
            .collect();
        let arguments = call_tool(
            context,
            &model_bit,
            vision_history(&images, &instructions),
            preamble,
            SubmitTool {
                name: TOOL,
                description: "Submit failure diagnosis and healing result",
                parameters,
            },
        )
        .await?;

        let healing = match arguments {
            Some(arguments) => map_healed_point(parse_tool_args(TOOL, &arguments)?, view),
            None => not_healed(format!("The model answered without calling `{TOOL}`")),
        };

        context
            .set_pin_value("result", json::json!(healing))
            .await?;
        context
            .set_pin_value("diagnosis", json::json!(healing.diagnosis))
            .await?;
        context
            .set_pin_value(
                "new_value",
                json::json!(healing.new_value.clone().unwrap_or_default()),
            )
            .await?;

        if healing.healed {
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
        let frame = ScreenFrame::new(None, (0, 0, 1440, 900), (2880, 1800)).unwrap();
        ModelView::new(frame.resized(1440, 900).unwrap())
    }

    fn healing(new_value: &str) -> HealingResult {
        let mut healing = not_healed("moved".into());
        healing.healed = true;
        healing.new_value = Some(new_value.into());
        healing
    }

    #[test]
    fn points_parse_in_common_notations() {
        assert_eq!(parse_point("12,34"), Some((12.0, 34.0)));
        assert_eq!(parse_point(" (12.5, 34) "), Some((12.5, 34.0)));
        assert_eq!(parse_point("[1, 2]"), Some((1.0, 2.0)));
        assert_eq!(parse_point("#submit"), None);
        assert_eq!(parse_point("a,b"), None);
    }

    #[test]
    fn healed_points_are_mapped_and_off_image_points_fail() {
        let view = view();
        assert_eq!(
            map_healed_point(healing("(100, 200)"), Some(&view)).new_value,
            Some("100,200".into())
        );
        let rejected = map_healed_point(healing("5000,10"), Some(&view));
        assert!(!rejected.healed && rejected.new_value.is_none());
        assert_eq!(
            map_healed_point(healing("#next"), Some(&view)).new_value,
            Some("#next".into())
        );
        assert_eq!(
            target_for_model("100,200", Some(&view)),
            "100,200 (pixels of the screenshot)"
        );
        assert_eq!(target_for_model("#a", Some(&view)), "#a");
    }

    #[test]
    fn diagnosis_without_optional_lists_parses() {
        let result: HealingResult = parse_tool_args(
            TOOL,
            &json::json!({
                "healed": false,
                "confidence": 0.2,
                "diagnosis": { "failure_type": "timeout", "root_cause": "slow", "severity": "low", "recoverable": true }
            }),
        )
        .unwrap();
        assert!(result.diagnosis.recommended_actions.is_empty() && result.new_value.is_none());
    }
}
