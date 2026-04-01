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
pub struct ScreenClassification {
    pub screen_type: String,
    pub app_name: Option<String>,
    pub state: String,
    pub visible_elements: Vec<String>,
    pub suggested_actions: Vec<String>,
    pub confidence: f64,
}

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
struct ClassifyScreenTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct ClassifyScreenError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for ClassifyScreenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Classify screen error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for ClassifyScreenError {}

#[cfg(feature = "execute")]
impl Tool for ClassifyScreenTool {
    const NAME: &'static str = "submit_classification";
    type Error = ClassifyScreenError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit the screen classification result".to_string(),
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
pub struct LLMClassifyScreenNode {}

impl LLMClassifyScreenNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMClassifyScreenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_classify_screen",
            "LLM Classify Screen",
            "Uses vision LLM to classify screen state and identify visible elements",
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
            "Base64-encoded screenshot",
            VariableType::String,
        );

        node.add_input_pin(
            "expected_states",
            "Expected States",
            "Comma-separated list of possible states to classify into",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "classification",
            "Classification",
            "Screen classification result",
            VariableType::Struct,
        )
        .set_schema::<ScreenClassification>();

        node.add_output_pin(
            "screen_type",
            "Screen Type",
            "Detected screen type",
            VariableType::String,
        );

        node.add_output_pin(
            "state",
            "State",
            "Current screen state",
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
        let expected_states: String = context
            .evaluate_pin("expected_states")
            .await
            .unwrap_or_default();

        let tool_params = json::json!({
            "type": "object",
            "properties": {
                "screen_type": { "type": "string", "description": "Type of screen (e.g., login, dashboard, form, error, loading)" },
                "app_name": { "type": "string", "description": "Name of the application if identifiable" },
                "state": { "type": "string", "description": "Current state of the screen" },
                "visible_elements": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of key visible UI elements"
                },
                "suggested_actions": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Suggested next actions based on screen state"
                },
                "confidence": { "type": "number", "description": "Confidence score 0-1" }
            },
            "required": ["screen_type", "state", "visible_elements", "confidence"]
        });

        let prompt = if expected_states.is_empty() {
            "Analyze this screenshot and classify the screen type and current state.".to_string()
        } else {
            format!(
                "Analyze this screenshot and classify into one of these states: {}",
                expected_states
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

        let preamble = "You are a screen analysis expert. Analyze screenshots to identify the type of screen, its current state, and visible UI elements. Be precise and thorough.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(ClassifyScreenTool {
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

        let mut result: Option<ScreenClassification> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_classification"
            {
                result = Some(json::from_value(arguments)?);
            }
        }

        let classification = result.unwrap_or(ScreenClassification {
            screen_type: "unknown".to_string(),
            app_name: None,
            state: "unknown".to_string(),
            visible_elements: vec![],
            suggested_actions: vec![],
            confidence: 0.0,
        });

        context
            .set_pin_value("classification", json::json!(classification.clone()))
            .await?;
        context
            .set_pin_value("screen_type", json::json!(classification.screen_type))
            .await?;
        context
            .set_pin_value("state", json::json!(classification.state))
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
