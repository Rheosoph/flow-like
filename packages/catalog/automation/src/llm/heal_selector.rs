use super::add_screenshot_pins;
#[cfg(feature = "execute")]
use super::{
    SubmitTool, call_tool, load_screenshot, parse_tool_args, truncate_on_char_boundary,
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
pub struct HealedSelector {
    pub healed: bool,
    pub new_selector: Option<String>,
    #[serde(default)]
    pub selector_type: String,
    pub confidence: f64,
    pub reasoning: String,
    #[serde(default)]
    pub alternatives: Vec<String>,
}

#[cfg(feature = "execute")]
const TOOL: &str = "submit_healed_selector";
#[cfg(feature = "execute")]
const MAX_HTML_BYTES: usize = 50_000;

/// "Healed" needs a non-empty selector; the requested selector type fills an omitted one.
#[cfg(feature = "execute")]
fn settle(mut healed: HealedSelector, selector_type: &str) -> HealedSelector {
    if healed.selector_type.is_empty() {
        healed.selector_type = selector_type.to_string();
    }
    healed.new_selector = healed
        .new_selector
        .map(|selector| selector.trim().to_string())
        .filter(|selector| !selector.is_empty());
    if healed.healed && healed.new_selector.is_none() {
        healed.healed = false;
        healed.reasoning = format!(
            "Model reported success without a selector: {}",
            healed.reasoning
        );
    }
    healed
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMHealSelectorNode {}

impl LLMHealSelectorNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMHealSelectorNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_heal_selector",
            "LLM Heal Selector",
            "Uses LLM to fix a broken CSS/XPath selector based on page context",
            "Automation/LLM/Healing",
        );
        node.set_flowscript_name("automation.llm", "healSelector");
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

        add_screenshot_pins(&mut node, "Optional (recommended) screenshot", false);

        node.add_input_pin(
            "page_html",
            "Page HTML",
            "Current page HTML or DOM structure; only the first 50,000 bytes are sent",
            VariableType::String,
        );

        node.add_input_pin(
            "broken_selector",
            "Broken Selector",
            "The selector that no longer works",
            VariableType::String,
        );

        node.add_input_pin(
            "element_description",
            "Element Description",
            "Description of what the selector should match",
            VariableType::String,
        );

        node.add_input_pin(
            "selector_type",
            "Selector Type",
            "Type of selector: css, xpath, or accessibility",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("css")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "exec_failed",
            "Failed",
            "Could not heal",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Healed selector result",
            VariableType::Struct,
        )
        .set_schema::<HealedSelector>();

        node.add_output_pin(
            "new_selector",
            "New Selector",
            "The healed selector string",
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
        let page_html: String = context.evaluate_pin("page_html").await?;
        let broken_selector: String = context.evaluate_pin("broken_selector").await?;
        let element_description: String = context.evaluate_pin("element_description").await?;
        let selector_type: String = context
            .evaluate_pin("selector_type")
            .await
            .unwrap_or_else(|_| "css".to_string());
        let screenshot = load_screenshot(context).await?;

        let parameters = json::json!({
            "type": "object",
            "properties": {
                "healed": { "type": "boolean", "description": "Whether a working selector was found" },
                "new_selector": { "type": "string", "description": "The new working selector" },
                "selector_type": { "type": "string", "description": "Type of selector (css, xpath, accessibility)" },
                "confidence": { "type": "number", "description": "Confidence score 0-1" },
                "reasoning": { "type": "string", "description": "Explanation of changes made" },
                "alternatives": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Alternative selectors that might work"
                }
            },
            "required": ["healed", "confidence", "reasoning"]
        });

        let html = if page_html.len() > MAX_HTML_BYTES {
            format!(
                "{}...[truncated]",
                truncate_on_char_boundary(&page_html, MAX_HTML_BYTES)
            )
        } else {
            page_html
        };

        let instructions = format!(
            "Fix this broken {selector_type} selector:\n\nBroken selector: {broken_selector}\nElement description: {element_description}\n\nPage HTML:\n{html}"
        );

        let preamble = format!(
            "You are a web automation expert specializing in {selector_type} selectors. Analyze the page structure and fix the broken selector. Consider:\n\
            1. Changes in element IDs, classes, or structure\n\
            2. More robust selector strategies (data attributes, aria labels)\n\
            3. Unique identifying characteristics of the target element"
        );

        let images: Vec<_> = screenshot
            .iter()
            .map(|screenshot| &screenshot.image)
            .collect();
        let arguments = call_tool(
            context,
            &model_bit,
            vision_history(&images, &instructions),
            &preamble,
            SubmitTool {
                name: TOOL,
                description: "Submit the healed selector",
                parameters,
            },
        )
        .await?;

        let healed = match arguments {
            Some(arguments) => settle(parse_tool_args(TOOL, &arguments)?, &selector_type),
            None => HealedSelector {
                healed: false,
                new_selector: None,
                selector_type,
                confidence: 0.0,
                reasoning: format!("The model answered without calling `{TOOL}`"),
                alternatives: vec![],
            },
        };

        context.set_pin_value("result", json::json!(healed)).await?;
        context
            .set_pin_value(
                "new_selector",
                json::json!(healed.new_selector.clone().unwrap_or_default()),
            )
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

    #[test]
    fn omitted_fields_default_and_success_needs_a_selector() {
        let parsed: HealedSelector = parse_tool_args(
            TOOL,
            &json::json!({"healed": true, "confidence": 0.9, "reasoning": "ok", "new_selector": "  "}),
        )
        .unwrap();
        let settled = settle(parsed, "xpath");
        assert_eq!(settled.selector_type, "xpath");
        assert!(settled.alternatives.is_empty());
        assert!(!settled.healed && settled.new_selector.is_none());

        let parsed: HealedSelector = parse_tool_args(
            TOOL,
            &json::json!({"healed": true, "confidence": 0.9, "reasoning": "ok", "new_selector": "#go"}),
        )
        .unwrap();
        assert!(settle(parsed, "css").healed);
    }
}
