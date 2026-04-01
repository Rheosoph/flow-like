use flow_like::{
    bit::Bit,
    flow::{
        board::Board,
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
};
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Value, async_trait, json};
#[cfg(feature = "execute")]
use rig::completion::{Completion, ToolDefinition};
#[cfg(feature = "execute")]
use rig::message::{AssistantContent, ToolCall, ToolChoice, ToolFunction};
#[cfg(feature = "execute")]
use rig::tool::Tool;
use std::sync::Arc;

#[cfg(feature = "execute")]
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ExtractTool {
    parameters: Value,
    output_schema: Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct ExtractError(String);

#[cfg(feature = "execute")]
impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Extract error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for ExtractError {}

#[cfg(feature = "execute")]
impl Tool for ExtractTool {
    const NAME: &'static str = "submit_extraction";
    type Error = ExtractError;
    type Args = Value;
    type Output = Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit extracted structured data".to_string(),
            parameters: self.parameters.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        jsonschema::validate(&self.output_schema, &args)
            .map_err(|e| ExtractError(format!("Schema validation failed: {}", e)))?;
        Ok(args)
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMExtractFromScreenNode {}

impl LLMExtractFromScreenNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(feature = "execute")]
fn looks_like_schema(value: &Value) -> bool {
    const SCHEMA_KEYWORDS: &[&str] = &[
        "type",
        "properties",
        "items",
        "$schema",
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "not",
        "required",
        "additionalProperties",
        "enum",
        "const",
    ];
    value
        .as_object()
        .is_some_and(|obj| SCHEMA_KEYWORDS.iter().any(|kw| obj.contains_key(*kw)))
}

#[cfg(feature = "execute")]
fn prepare_schema(raw: &str) -> flow_like_types::Result<(Value, Value)> {
    let user_json =
        json::from_str::<Value>(raw.trim()).map_err(|e| anyhow!("Invalid JSON schema: {e}"))?;

    let is_schema = looks_like_schema(&user_json) && jsonschema::meta::is_valid(&user_json);
    let schema = if is_schema {
        user_json
    } else {
        let inferred = schemars::schema_for_value!(&user_json);
        json::from_str(&json::to_string_pretty(&inferred)?)?
    };

    let tool_params = if schema.get("type").and_then(|t| t.as_str()) == Some("object") {
        schema.clone()
    } else {
        json::json!({
            "type": "object",
            "properties": {"value": schema.clone()},
            "required": ["value"],
            "additionalProperties": false
        })
    };

    Ok((tool_params, schema))
}

#[async_trait]
impl NodeLogic for LLMExtractFromScreenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_extract_from_screen",
            "LLM Extract From Screen",
            "Uses vision LLM to extract structured data from a screenshot",
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
            "schema",
            "Schema",
            "JSON Schema describing what to extract (or example JSON)",
            VariableType::String,
        );

        node.add_input_pin(
            "hint",
            "Hint",
            "Optional extraction hint",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "data",
            "Data",
            "Extracted structured data",
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
        let schema_str: String = context.evaluate_pin("schema").await?;
        let hint: String = context.evaluate_pin("hint").await.unwrap_or_default();

        let (tool_params, output_schema) = prepare_schema(&schema_str)?;

        let prompt = if hint.is_empty() {
            "Extract the requested data from this screenshot according to the schema.".to_string()
        } else {
            format!("Extract data from this screenshot. Hint: {}", hint)
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

        let preamble = "You are a data extraction expert. Extract structured data from screenshots according to the provided schema.";

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(preamble)
            .tool(ExtractTool {
                parameters: tool_params,
                output_schema: output_schema.clone(),
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

        let mut extracted: Option<Value> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_extraction"
            {
                extracted = Some(arguments);
            }
        }

        let data = extracted.ok_or_else(|| anyhow!("LLM did not return extracted data"))?;

        let final_data = if output_schema.get("type").and_then(|t| t.as_str()) != Some("object") {
            data.get("value").cloned().unwrap_or(data)
        } else {
            data
        };

        context.set_pin_value("data", final_data).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }

    #[cfg(feature = "execute")]
    async fn on_update(&self, node: &mut Node, _board: Arc<Board>) {
        node.error = None;
        node.harmonize_type(vec!["data"], true);

        let schema_value = node
            .get_pin_by_name("schema")
            .and_then(|pin| {
                pin.default_value
                    .as_ref()
                    .and_then(|bytes| json::from_slice::<Value>(bytes).ok())
            })
            .and_then(|value| value.as_str().map(|s| s.to_string()));

        if let Some(raw) = schema_value {
            if raw.trim().is_empty() {
                node.error = Some("Schema cannot be empty".to_string());
                return;
            }
            match prepare_schema(&raw) {
                Ok((_, output_schema)) => {
                    let schema_type = output_schema.get("type").and_then(|t| t.as_str());
                    let (pin_schema, value_type) = match schema_type {
                        Some("array") => {
                            let items = output_schema
                                .get("items")
                                .cloned()
                                .unwrap_or(json::json!({}));
                            (items, ValueType::Array)
                        }
                        _ => (output_schema, ValueType::Normal),
                    };
                    if let Some(pin) = node.get_pin_mut_by_name("data") {
                        pin.schema = json::to_string(&pin_schema).ok();
                        pin.value_type = value_type;
                        pin.data_type = VariableType::Struct;
                    }
                }
                Err(e) => node.error = Some(format!("Schema error: {}", e)),
            }
        }
    }

    #[cfg(not(feature = "execute"))]
    async fn on_update(&self, node: &mut Node, _board: Arc<Board>) {
        node.error = None;
        node.harmonize_type(vec!["data"], true);
    }
}
