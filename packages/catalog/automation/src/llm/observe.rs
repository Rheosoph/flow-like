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
pub struct ScreenObservation {
    pub description: String,
    pub app_context: String,
    pub interactive_elements: Vec<ObservedElement>,
    pub text_content: Vec<String>,
    pub notable_features: Vec<String>,
    pub possible_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ObservedElement {
    pub element_type: String,
    pub description: String,
    pub approximate_location: String,
    pub is_interactive: bool,
    pub current_state: Option<String>,
}

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
struct ObserveScreenTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct ObserveScreenError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for ObserveScreenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Observe screen error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for ObserveScreenError {}

#[cfg(feature = "execute")]
impl Tool for ObserveScreenTool {
    const NAME: &'static str = "submit_observation";
    type Error = ObserveScreenError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit screen observation".to_string(),
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
        node.add_icon("/flow/icons/bot-search.svg");

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
            "Base64-encoded screenshot",
            VariableType::String,
        );

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
            "List of observed elements",
            VariableType::Generic,
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

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let screenshot: String = context.evaluate_pin("screenshot").await?;
        let focus_area: String = context.evaluate_pin("focus_area").await.unwrap_or_default();

        let tool_params = json::json!({
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
                            "current_state": { "type": "string", "description": "enabled, disabled, selected, etc." }
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

        let prompt = if focus_area.is_empty() {
            "Observe this screen comprehensively. Identify all interactive elements, text content, and possible actions.".to_string()
        } else {
            format!(
                "Observe this screen, focusing especially on: {}",
                focus_area
            )
        };

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
                text: prompt.clone(),
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

        let preamble = "You are a screen observation expert. Analyze screenshots thoroughly to identify all UI elements, their purposes, states, and possible interactions. Be comprehensive but precise.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(ObserveScreenTool {
                parameters: tool_params,
            })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        let response = agent
            .completion(prompt, vec![])
            .await
            .map_err(|e| anyhow!("LLM completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send request: {}", e))?;

        let mut result: Option<ScreenObservation> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_observation"
            {
                result = Some(json::from_value(arguments)?);
            }
        }

        let observation = result.unwrap_or(ScreenObservation {
            description: "Could not observe screen".to_string(),
            app_context: "Unknown".to_string(),
            interactive_elements: vec![],
            text_content: vec![],
            notable_features: vec![],
            possible_actions: vec![],
        });

        context
            .set_pin_value("observation", json::json!(observation.clone()))
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

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
struct DescribeElementTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
impl Tool for DescribeElementTool {
    const NAME: &'static str = "submit_element_description";
    type Error = ObserveScreenError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit element description".to_string(),
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
        node.add_icon("/flow/icons/bot-search.svg");

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

        node.add_input_pin(
            "screenshot",
            "Screenshot",
            "Base64-encoded screenshot",
            VariableType::String,
        );

        node.add_input_pin("x", "X", "X coordinate of element", VariableType::Integer);

        node.add_input_pin("y", "Y", "Y coordinate of element", VariableType::Integer);

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
        use flow_like_model_provider::history::{
            Content, ContentType, History, HistoryMessage, ImageUrl as HistoryImageUrl,
            MessageContent, Role,
        };

        context.deactivate_exec_pin("exec_out").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let screenshot: String = context.evaluate_pin("screenshot").await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;

        let tool_params = json::json!({
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
                text: format!("Describe the UI element at coordinates ({}, {}).", x, y),
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

        let preamble = "You are a UI element expert. Given coordinates on a screenshot, identify and describe the element at or near that location.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(DescribeElementTool {
                parameters: tool_params,
            })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        let response = agent
            .completion(format!("Describe element at ({}, {})", x, y), vec![])
            .await
            .map_err(|e| anyhow!("LLM completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send request: {}", e))?;

        let mut result: Option<ElementDescription> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_element_description"
            {
                result = Some(json::from_value(arguments)?);
            }
        }

        let element = result.unwrap_or(ElementDescription {
            element_type: "unknown".to_string(),
            visual_description: "Could not identify element".to_string(),
            purpose: "Unknown".to_string(),
            current_state: "Unknown".to_string(),
            text_content: None,
            accessibility_info: None,
        });

        context
            .set_pin_value("element", json::json!(element.clone()))
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
