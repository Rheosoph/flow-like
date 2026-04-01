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
pub struct FailureDiagnosis {
    pub failure_type: String,
    pub root_cause: String,
    pub severity: String,
    pub recoverable: bool,
    pub recommended_actions: Vec<String>,
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
#[derive(Debug, Serialize, Deserialize)]
struct DiagnoseAndHealTool {
    parameters: flow_like_types::Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct DiagnoseAndHealError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for DiagnoseAndHealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Diagnose and heal error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for DiagnoseAndHealError {}

#[cfg(feature = "execute")]
impl Tool for DiagnoseAndHealTool {
    const NAME: &'static str = "submit_diagnosis_and_healing";
    type Error = DiagnoseAndHealError;
    type Args = flow_like_types::Value;
    type Output = flow_like_types::Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit failure diagnosis and healing result".to_string(),
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
            "Base64-encoded screenshot at time of failure",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

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
            "The target of the failed action (selector, coordinates, text)",
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
            "Current page HTML (for selector-based failures)",
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
            "Healed value (new selector, coordinates, etc.)",
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
        let error_message: String = context.evaluate_pin("error_message").await?;
        let action_type: String = context.evaluate_pin("action_type").await?;
        let action_target: String = context.evaluate_pin("action_target").await?;
        let ctx: String = context.evaluate_pin("context").await.unwrap_or_default();
        let page_html: String = context.evaluate_pin("page_html").await.unwrap_or_default();

        let tool_params = json::json!({
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
                "new_value": { "type": "string", "description": "New selector/coordinates/value to use" },
                "confidence": { "type": "number", "description": "Confidence in the healing 0-1" }
            },
            "required": ["healed", "diagnosis", "confidence"]
        });

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

        let truncated_html = if page_html.len() > 30000 {
            format!("\n\nPage HTML (truncated):\n{}...", &page_html[..30000])
        } else if !page_html.is_empty() {
            format!("\n\nPage HTML:\n{}", page_html)
        } else {
            String::new()
        };

        content_items.push(Content::Text {
            content_type: ContentType::Text,
            text: format!(
                "Automation Failure:\n\
                Action Type: {}\n\
                Action Target: {}\n\
                Error: {}\n\
                Context: {}\n\
                {}\n\n\
                Diagnose the failure and provide a healing solution if possible.",
                action_type,
                action_target,
                error_message,
                if ctx.is_empty() {
                    "None provided"
                } else {
                    &ctx
                },
                truncated_html
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

        let preamble = "You are an automation failure diagnosis and self-healing expert. Analyze failures and provide actionable healing solutions. Common failure types include:\n\
        - element_not_found: Selector/element no longer exists or changed\n\
        - timeout: Operation took too long\n\
        - stale_element: Element reference became invalid\n\
        - visibility: Element exists but not visible/interactable\n\
        - state_change: Application state changed unexpectedly\n\
        - template_mismatch: Visual template no longer matches\n\
        Provide specific, actionable healing solutions when possible.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(DiagnoseAndHealTool {
                parameters: tool_params,
            })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        let response = agent
            .completion(
                format!("Diagnose and heal: {} - {}", action_type, error_message),
                vec![],
            )
            .await
            .map_err(|e| anyhow!("LLM completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send request: {}", e))?;

        let mut result: Option<HealingResult> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_diagnosis_and_healing"
            {
                result = Some(json::from_value(arguments)?);
            }
        }

        let healing = result.unwrap_or(HealingResult {
            healed: false,
            diagnosis: FailureDiagnosis {
                failure_type: "unknown".to_string(),
                root_cause: "Could not diagnose failure".to_string(),
                severity: "high".to_string(),
                recoverable: false,
                recommended_actions: vec![],
                context_clues: vec![],
            },
            healing_action: None,
            new_value: None,
            confidence: 0.0,
        });

        context
            .set_pin_value("result", json::json!(healing.clone()))
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
