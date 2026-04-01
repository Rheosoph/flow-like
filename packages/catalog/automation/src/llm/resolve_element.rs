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
pub struct ElementCandidate {
    pub index: usize,
    pub x: i32,
    pub y: i32,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolvedElement {
    pub resolved: bool,
    pub selected_index: Option<usize>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub reasoning: String,
}

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
struct ResolveElementTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct ResolveElementError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for ResolveElementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Resolve element error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for ResolveElementError {}

#[cfg(feature = "execute")]
impl Tool for ResolveElementTool {
    const NAME: &'static str = "submit_resolution";
    type Error = ResolveElementError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit the resolved element selection".to_string(),
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
pub struct LLMResolveElementNode {}

impl LLMResolveElementNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMResolveElementNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_resolve_element",
            "LLM Resolve Element",
            "Uses LLM to disambiguate between multiple element candidates",
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
            "candidates",
            "Candidates",
            "Array of element candidates to choose from",
            VariableType::Generic,
        );

        node.add_input_pin(
            "intent",
            "Intent",
            "What the user is trying to accomplish",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "exec_ambiguous",
            "Ambiguous",
            "Could not resolve",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Resolution result",
            VariableType::Struct,
        )
        .set_schema::<ResolvedElement>();

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
        context.deactivate_exec_pin("exec_ambiguous").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let screenshot: String = context.evaluate_pin("screenshot").await?;
        let candidates: Vec<ElementCandidate> = context.evaluate_pin("candidates").await?;
        let intent: String = context.evaluate_pin("intent").await?;

        let tool_params = json::json!({
            "type": "object",
            "properties": {
                "resolved": { "type": "boolean", "description": "Whether a single best match was identified" },
                "selected_index": { "type": "integer", "description": "Index of the selected candidate (0-based)" },
                "x": { "type": "integer", "description": "X coordinate of resolved element" },
                "y": { "type": "integer", "description": "Y coordinate of resolved element" },
                "reasoning": { "type": "string", "description": "Explanation of why this element was selected" }
            },
            "required": ["resolved", "reasoning"]
        });

        let candidates_desc = candidates
            .iter()
            .map(|c| format!("[{}] at ({}, {}): {}", c.index, c.x, c.y, c.description))
            .collect::<Vec<_>>()
            .join("\n");

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
                    "User intent: {}\n\nMultiple elements found:\n{}\n\nSelect the best matching element.",
                    intent, candidates_desc
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

        let preamble = "You are a UI element resolver. Given multiple candidate elements and the user's intent, select the most appropriate one. Consider visual context, element type, and user goal.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(ResolveElementTool {
                parameters: tool_params,
            })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        let response = agent
            .completion(intent.clone(), vec![])
            .await
            .map_err(|e| anyhow!("LLM completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send request: {}", e))?;

        let mut result: Option<ResolvedElement> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_resolution"
            {
                result = Some(json::from_value(arguments)?);
            }
        }

        let resolved = result.unwrap_or(ResolvedElement {
            resolved: false,
            selected_index: None,
            x: None,
            y: None,
            reasoning: "Could not resolve element".to_string(),
        });

        context
            .set_pin_value("result", json::json!(resolved))
            .await?;

        if resolved.resolved {
            context.activate_exec_pin("exec_out").await?;
        } else {
            context.activate_exec_pin("exec_ambiguous").await?;
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
