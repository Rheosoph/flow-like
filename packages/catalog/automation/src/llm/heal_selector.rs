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
pub struct HealedSelector {
    pub healed: bool,
    pub new_selector: Option<String>,
    pub selector_type: String,
    pub confidence: f64,
    pub reasoning: String,
    pub alternatives: Vec<String>,
}

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
struct HealSelectorTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct HealSelectorError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for HealSelectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Heal selector error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for HealSelectorError {}

#[cfg(feature = "execute")]
impl Tool for HealSelectorTool {
    const NAME: &'static str = "submit_healed_selector";
    type Error = HealSelectorError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit the healed selector".to_string(),
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
        node.add_icon("/flow/icons/bot-fix.svg");

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

        node.add_input_pin(
            "screenshot",
            "Screenshot",
            "Base64-encoded screenshot (optional but recommended)",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_input_pin(
            "page_html",
            "Page HTML",
            "Current page HTML or DOM structure",
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
        use flow_like_model_provider::history::{
            Content, ContentType, History, HistoryMessage, ImageUrl as HistoryImageUrl,
            MessageContent, Role,
        };

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_failed").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let screenshot: String = context.evaluate_pin("screenshot").await.unwrap_or_default();
        let page_html: String = context.evaluate_pin("page_html").await?;
        let broken_selector: String = context.evaluate_pin("broken_selector").await?;
        let element_description: String = context.evaluate_pin("element_description").await?;
        let selector_type: String = context
            .evaluate_pin("selector_type")
            .await
            .unwrap_or_else(|_| "css".to_string());

        let tool_params = json::json!({
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

        let truncated_html = if page_html.len() > 50000 {
            format!("{}...[truncated]", &page_html[..50000])
        } else {
            page_html
        };

        let mut content_items = vec![];

        if !screenshot.is_empty() {
            content_items.push(Content::Image {
                content_type: ContentType::ImageUrl,
                image_url: HistoryImageUrl {
                    url: format!("data:image/png;base64,{}", screenshot),
                    detail: None,
                },
            });
        }

        content_items.push(Content::Text {
            content_type: ContentType::Text,
            text: format!(
                "Fix this broken {} selector:\n\nBroken selector: {}\nElement description: {}\n\nPage HTML:\n{}",
                selector_type, broken_selector, element_description, truncated_html
            ),
        });

        let history = History::new(
            "".to_string(),
            vec![HistoryMessage {
                role: Role::User,
                content: MessageContent::Contents(content_items),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                annotations: None,
            }],
        );

        let preamble = format!(
            "You are a web automation expert specializing in {} selectors. Analyze the page structure and fix the broken selector. Consider:\n\
            1. Changes in element IDs, classes, or structure\n\
            2. More robust selector strategies (data attributes, aria labels)\n\
            3. Unique identifying characteristics of the target element",
            selector_type
        );

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(&preamble)
            .tool(HealSelectorTool {
                parameters: tool_params,
            })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        let response = agent
            .completion(element_description.clone(), vec![])
            .await
            .map_err(|e| anyhow!("LLM completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send request: {}", e))?;

        let mut result: Option<HealedSelector> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_healed_selector"
            {
                result = Some(json::from_value(arguments)?);
            }
        }

        let healed = result.unwrap_or(HealedSelector {
            healed: false,
            new_selector: None,
            selector_type,
            confidence: 0.0,
            reasoning: "Could not heal selector".to_string(),
            alternatives: vec![],
        });

        context
            .set_pin_value("result", json::json!(healed.clone()))
            .await?;
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
