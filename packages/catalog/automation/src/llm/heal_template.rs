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
pub struct HealedTemplate {
    pub healed: bool,
    pub found_at_x: Option<i32>,
    pub found_at_y: Option<i32>,
    pub confidence: f64,
    pub reasoning: String,
    pub suggested_region: Option<TemplateRegion>,
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
#[derive(Debug, Serialize, Deserialize)]
struct HealTemplateTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct HealTemplateError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for HealTemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Heal template error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for HealTemplateError {}

#[cfg(feature = "execute")]
impl Tool for HealTemplateTool {
    const NAME: &'static str = "submit_healed_template";
    type Error = HealTemplateError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit the healed template match result".to_string(),
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
            "template",
            "Template",
            "Base64-encoded template image that failed to match",
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
            "Where the element was previously found (x,y)",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

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
            "Healed template result",
            VariableType::Struct,
        )
        .set_schema::<HealedTemplate>();

        node.add_output_pin(
            "x",
            "X",
            "X coordinate of found element",
            VariableType::Integer,
        );

        node.add_output_pin(
            "y",
            "Y",
            "Y coordinate of found element",
            VariableType::Integer,
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
        let screenshot: String = context.evaluate_pin("screenshot").await?;
        let template: String = context.evaluate_pin("template").await?;
        let element_description: String = context.evaluate_pin("element_description").await?;
        let last_known: String = context
            .evaluate_pin("last_known_position")
            .await
            .unwrap_or_default();

        let tool_params = json::json!({
            "type": "object",
            "properties": {
                "healed": { "type": "boolean", "description": "Whether the element was found" },
                "found_at_x": { "type": "integer", "description": "X coordinate of the found element" },
                "found_at_y": { "type": "integer", "description": "Y coordinate of the found element" },
                "confidence": { "type": "number", "description": "Confidence score 0-1" },
                "reasoning": { "type": "string", "description": "Explanation of how the element was identified" },
                "suggested_region": {
                    "type": "object",
                    "properties": {
                        "x": { "type": "integer" },
                        "y": { "type": "integer" },
                        "width": { "type": "integer" },
                        "height": { "type": "integer" }
                    },
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
            format!("\nThe element was previously located at: {}", last_known)
        };

        let content_parts = vec![
            Content::Image {
                content_type: ContentType::ImageUrl,
                image_url: HistoryImageUrl {
                    url: format!("data:image/png;base64,{}", screenshot),
                    detail: None,
                },
            },
            Content::Image {
                content_type: ContentType::ImageUrl,
                image_url: HistoryImageUrl {
                    url: format!("data:image/png;base64,{}", template),
                    detail: None,
                },
            },
            Content::Text {
                content_type: ContentType::Text,
                text: format!(
                    "The first image is the current screen. The second image is a template that failed to match.\n\
                    Element description: {}\n{}\n\n\
                    Find where this element is now located on screen, accounting for possible visual changes.",
                    element_description, position_hint
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

        let preamble = "You are a visual UI analysis expert. When template matching fails due to visual changes (scaling, color changes, minor layout shifts), you can identify the same logical element by understanding its purpose and visual characteristics.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(HealTemplateTool {
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

        let mut result: Option<HealedTemplate> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_healed_template"
            {
                result = Some(json::from_value(arguments)?);
            }
        }

        let healed = result.unwrap_or(HealedTemplate {
            healed: false,
            found_at_x: None,
            found_at_y: None,
            confidence: 0.0,
            reasoning: "Could not locate element".to_string(),
            suggested_region: None,
            visual_changes_detected: vec![],
        });

        context
            .set_pin_value("result", json::json!(healed.clone()))
            .await?;
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
