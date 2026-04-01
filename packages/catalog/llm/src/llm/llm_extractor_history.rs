use async_trait::async_trait;
use flow_like::{
    bit::Bit,
    flow::{
        board::Board,
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic, NodeScores},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
};
use flow_like_model_provider::history::History;
#[cfg(feature = "execute")]
use flow_like_model_provider::response::{LLMUsageStats, Usage};
use flow_like_types::json::{self, Deserialize, Serialize};
use flow_like_types::{Value, anyhow};
#[cfg(feature = "execute")]
use rig::completion::{Completion, ToolDefinition};
#[cfg(feature = "execute")]
use rig::message::{AssistantContent, ToolCall, ToolChoice, ToolFunction};
#[cfg(feature = "execute")]
use rig::tool::Tool;
use std::{fmt, sync::Arc, time::Instant};

#[crate::register_node]
#[derive(Default)]
pub struct LLMExtractHistoryNode {}

impl LLMExtractHistoryNode {
    pub fn new() -> Self {
        LLMExtractHistoryNode {}
    }
}

// --- Dynamic knowledge extraction submit tool that takes a runtime JSON Schema ---
#[cfg(feature = "execute")]
#[derive(Debug, Deserialize, Serialize)]
struct DynamicSubmitTool {
    parameters: Value,
    output_schema: Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct SubmitError(String);

#[cfg(feature = "execute")]
impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Schema validation failed: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for SubmitError {}

#[cfg(feature = "execute")]
impl Tool for DynamicSubmitTool {
    const NAME: &'static str = "submit";
    type Error = SubmitError;
    type Args = Value;
    type Output = Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Knowledge extraction submit tool. Return structured data that matches the provided schema.".to_string(),
            parameters: self.parameters.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        jsonschema::validate(&self.output_schema, &args)
            .map_err(|e| SubmitError(format!("{}", e)))?;
        Ok(args)
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[cfg(feature = "execute")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ExtractionMode {
    Direct,
    Wrapped,
}

#[cfg(feature = "execute")]
struct PreparedSchema {
    tool_parameters: Value,
    output_schema: Value,
    mode: ExtractionMode,
    was_inferred: bool,
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
        "patternProperties",
        "enum",
        "const",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "pattern",
        "format",
        "definitions",
        "$defs",
    ];

    value
        .as_object()
        .is_some_and(|obj| SCHEMA_KEYWORDS.iter().any(|kw| obj.contains_key(*kw)))
}

#[cfg(feature = "execute")]
fn prepare_schema(raw: &str) -> flow_like_types::Result<PreparedSchema> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Schema input cannot be empty"));
    }

    let user_json = json::from_str::<Value>(trimmed).map_err(|e| {
        anyhow!(
            "Schema must be valid JSON (either a JSON Schema or an example JSON). Parse error: {e}"
        )
    })?;

    let is_schema = looks_like_schema(&user_json) && jsonschema::meta::is_valid(&user_json);
    let (inferred, was_inferred) = if is_schema {
        (user_json, false)
    } else {
        let schema = schemars::schema_for_value!(&user_json);
        let string = json::to_string_pretty(&schema)?;
        (json::from_str(&string)?, true)
    };

    let mode = match inferred.get("type").and_then(|t| t.as_str()) {
        Some("object") => ExtractionMode::Direct,
        _ => ExtractionMode::Wrapped,
    };

    let tool_parameters = if mode == ExtractionMode::Direct {
        inferred.clone()
    } else {
        json::json!({
            "type": "object",
            "properties": {"value": inferred.clone()},
            "required": ["value"],
            "additionalProperties": false
        })
    };

    Ok(PreparedSchema {
        tool_parameters,
        output_schema: inferred,
        mode,
        was_inferred,
    })
}

#[async_trait]
impl NodeLogic for LLMExtractHistoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_extractor_history",
            "AI Extractor from History",
            "Extracts structured data by replaying an entire chat history through an LLM",
            "AI/Generative",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(1);

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(4)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start the extraction",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Bit pointing to the LLM that will perform the extraction",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "schema",
            "Schema",
            "JSON Schema (or example JSON) describing the structure to extract",
            VariableType::String,
        );

        node.add_input_pin(
            "history",
            "History",
            "Chat history to replay when extracting data",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "hint",
            "Extraction Hint",
            "Optional hint to guide the extraction (e.g. 'only extract individual line items, not totals')",
            VariableType::String,
        ).set_default_value(Some(json::json!("")));

        node.add_output_pin(
            "exec_out",
            "Execution Output",
            "Executes after extraction succeeds",
            VariableType::Execution,
        );

        node.add_output_pin(
            "response",
            "Json",
            "Structured JSON value that matches the schema",
            VariableType::Generic,
        );

        node.add_output_pin(
            "stats",
            "Stats",
            "Token usage, cost, and model statistics",
            VariableType::Struct,
        )
        .set_schema::<flow_like_model_provider::response::LLMUsageStats>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model_bit = context.evaluate_pin::<Bit>("model").await?;
        let schema_str: String = context.evaluate_pin("schema").await?;
        let history: History = context.evaluate_pin("history").await?;
        let hint: String = context.evaluate_pin("hint").await.unwrap_or_default();

        let prepared_schema = prepare_schema(&schema_str)?;

        context.log_message(
            &format!("Using extraction mode: {:?}", prepared_schema.mode),
            LogLevel::Debug,
        );

        let preamble = if hint.trim().is_empty() {
            "You are a knowledge extraction assistant. Extract data by calling the 'submit' tool with structured data matching the provided schema.".to_string()
        } else {
            format!(
                "You are a knowledge extraction assistant. Extract data by calling the 'submit' tool with structured data matching the provided schema.\n\nExtraction hint: {}",
                hint
            )
        };

        let (prompt, chat_history) = history
            .extract_prompt_and_history()
            .map_err(|e| anyhow!("Failed to convert history into rig messages: {e}"))?;

        let agent_builder = model_bit
            .agent(context, &Some(history))
            .await?
            .preamble(&preamble)
            .tool(DynamicSubmitTool {
                parameters: prepared_schema.tool_parameters,
                output_schema: prepared_schema.output_schema.clone(),
            })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        let start = Instant::now();
        let response = agent
            .completion(prompt, chat_history.into_iter().collect())
            .await
            .map_err(|e| anyhow!("Model completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send completion request: {}", e))?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let stats = LLMUsageStats {
            usage: Usage::from_rig(response.usage),
            model: model_bit.meta.get("en").map(|m| m.name.clone()),
            duration_ms: Some(duration_ms),
            iterations: None,
            calls: vec![],
        };

        let mut last_args: Option<Value> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit"
            {
                last_args = Some(arguments);
            }
        }

        let args = last_args.ok_or_else(|| {
            anyhow!("Model did not return a 'submit' tool call. Ensure the model supports function calling.")
        })?;

        let extracted = match prepared_schema.mode {
            ExtractionMode::Direct => args,
            ExtractionMode::Wrapped => args
                .get("value")
                .cloned()
                .ok_or_else(|| anyhow!("Tool call missing 'value' field in wrapped mode"))?,
        };

        context.log_message("Successfully extracted structured data", LogLevel::Debug);

        context.set_pin_value("response", extracted).await?;
        context.set_pin_value("stats", json::json!(stats)).await?;
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

        node.harmonize_type(vec!["response"], true);

        let schema_value = node
            .get_pin_by_name("schema")
            .and_then(|pin| {
                pin.default_value
                    .as_ref()
                    .and_then(|bytes| json::from_slice::<Value>(bytes).ok())
            })
            .and_then(|value| value.as_str().map(|s| s.to_string()));

        match schema_value {
            Some(raw) if raw.trim().is_empty() => {
                node.error = Some("Schema input cannot be empty".to_string());
            }
            Some(raw) => match prepare_schema(&raw) {
                Ok(prepared) => {
                    if prepared.was_inferred
                        && let Some(pin) = node.get_pin_mut_by_name("schema")
                    {
                        let schema_str = json::to_string_pretty(&prepared.output_schema)
                            .unwrap_or_else(|_| prepared.output_schema.to_string());
                        let _ = pin.set_default_value(Some(json::json!(schema_str)));
                    }

                    let schema_type = prepared.output_schema.get("type").and_then(|t| t.as_str());

                    let (pin_schema, value_type) = match schema_type {
                        Some("array") => {
                            let items_schema = prepared
                                .output_schema
                                .get("items")
                                .cloned()
                                .unwrap_or(json::json!({}));
                            (items_schema, ValueType::Array)
                        }
                        _ => (prepared.output_schema.clone(), ValueType::Normal),
                    };

                    if let Some(response_pin) = node.get_pin_mut_by_name("response") {
                        response_pin.schema = json::to_string(&pin_schema).ok();
                        response_pin.value_type = value_type;
                        response_pin.data_type = VariableType::Struct;
                    }
                }
                Err(err) => {
                    node.error = Some(format!("Schema error: {}", err));
                }
            },
            None => {
                node.error = Some("Schema input cannot be empty".to_string());
            }
        }
    }

    #[cfg(not(feature = "execute"))]
    async fn on_update(&self, node: &mut Node, _board: Arc<Board>) {
        node.error = None;
        node.harmonize_type(vec!["response"], true);
    }
}
