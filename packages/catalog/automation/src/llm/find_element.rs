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
#[derive(Debug, Serialize, Deserialize)]
struct FindElementTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct FindElementError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for FindElementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Find element error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for FindElementError {}

#[cfg(feature = "execute")]
impl Tool for FindElementTool {
    const NAME: &'static str = "submit_element_location";
    type Error = FindElementError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit the located element coordinates and details".to_string(),
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
        node.add_icon("/flow/icons/bot-search.svg");

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
            "Base64-encoded screenshot of the screen",
            VariableType::String,
        );

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
            "Element not found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "location",
            "Location",
            "Element location details",
            VariableType::Struct,
        )
        .set_schema::<ElementLocation>();

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
        context.deactivate_exec_pin("exec_not_found").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let screenshot: String = context.evaluate_pin("screenshot").await?;
        let description: String = context.evaluate_pin("description").await?;
        let ctx: String = context.evaluate_pin("context").await.unwrap_or_default();

        let tool_params = json::json!({
            "type": "object",
            "properties": {
                "found": { "type": "boolean", "description": "Whether the element was found" },
                "x": { "type": "integer", "description": "X coordinate of the element center" },
                "y": { "type": "integer", "description": "Y coordinate of the element center" },
                "width": { "type": "integer", "description": "Estimated width of the element" },
                "height": { "type": "integer", "description": "Estimated height of the element" },
                "confidence": { "type": "number", "description": "Confidence score 0-1" },
                "description": { "type": "string", "description": "Description of what was found" },
                "selector_hint": { "type": "string", "description": "Suggested CSS/accessibility selector if identifiable" }
            },
            "required": ["found", "confidence", "description"]
        });

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
                    "Find the UI element matching this description: \"{}\"\n{}",
                    description,
                    if ctx.is_empty() {
                        String::new()
                    } else {
                        format!("Context: {}", ctx)
                    }
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

        let preamble = "You are a UI element locator. Analyze the screenshot and find the element matching the description. Return precise pixel coordinates for the element's center position. If you cannot find the element, set found=false.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(FindElementTool {
                parameters: tool_params,
            })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        let response = agent
            .completion(description.clone(), vec![])
            .await
            .map_err(|e| anyhow!("LLM completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send request: {}", e))?;

        let mut result: Option<ElementLocation> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_element_location"
            {
                result = Some(json::from_value(arguments)?);
            }
        }

        let location = result.unwrap_or(ElementLocation {
            found: false,
            x: None,
            y: None,
            width: None,
            height: None,
            confidence: 0.0,
            description: "No element found".to_string(),
            selector_hint: None,
        });

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
