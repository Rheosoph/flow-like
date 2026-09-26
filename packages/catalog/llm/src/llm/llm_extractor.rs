use async_trait::async_trait;
#[cfg(feature = "execute")]
use flow_like::flow::{execution::LogLevel, pin::ValueType};
use flow_like::{
    bit::Bit,
    flow::{
        board::Board,
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
#[cfg(feature = "execute")]
use flow_like_model_provider::response::{LLMUsageStats, ModelCallEntry, Usage};
use flow_like_types::json;
#[cfg(feature = "execute")]
use flow_like_types::json::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use flow_like_types::{Value, anyhow};
#[cfg(feature = "execute")]
use rig::completion::{Completion, CompletionError, ToolDefinition};
#[cfg(feature = "execute")]
use rig::message::{AssistantContent, ToolCall, ToolChoice, ToolFunction};
#[cfg(feature = "execute")]
use rig::tool::Tool;
#[cfg(feature = "execute")]
use std::{
    fmt,
    time::{Duration, Instant},
};

#[crate::register_node]
#[derive(Default)]
pub struct LLMExtractNode {}

impl LLMExtractNode {
    pub fn new() -> Self {
        LLMExtractNode {}
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
        compile_validator(&self.output_schema)
            .map_err(|e| SubmitError(format!("{}", e)))?
            .validate(&args)
            .map_err(|e| SubmitError(format!("{}", e)))?;
        Ok(args)
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[cfg(feature = "execute")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ExtractionMode {
    Direct,
    Wrapped,
}

#[cfg(feature = "execute")]
pub(super) struct PreparedSchema {
    pub(super) tool_parameters: Value,
    pub(super) output_schema: Value,
    pub(super) mode: ExtractionMode,
    pub(super) was_inferred: bool,
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

/// Compiles a schema up front so a malformed `pattern`, a remote `$ref`, or an unknown
/// dialect surfaces as an error instead of panicking inside `jsonschema`. This runs at board
/// load (`on_update`) as well as at run time, so a `$ref` is never fetched: the schema is
/// authored by whoever edits the board, and the process compiling it may be the API server.
#[cfg(feature = "execute")]
pub(super) fn compile_validator(schema: &Value) -> flow_like_types::Result<jsonschema::Validator> {
    jsonschema::options()
        .with_retriever(flow_like_catalog_core::RejectExternalSchemaReferences)
        .build(schema)
        .map_err(|error| anyhow!("Schema is not a usable JSON Schema: {error}"))
}

#[cfg(feature = "execute")]
pub(super) fn prepare_schema(raw: &str) -> flow_like_types::Result<PreparedSchema> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Schema input cannot be empty"));
    }

    let user_json = json::from_str::<Value>(trimmed).map_err(|e| {
        anyhow!(
            "Schema must be valid JSON (either a JSON Schema or an example JSON). Parse error: {e}"
        )
    })?;

    let is_schema = if looks_like_schema(&user_json) {
        jsonschema::meta::try_is_valid(&user_json).map_err(|error| {
            anyhow!("Schema declares a $schema dialect that cannot be resolved: {error}")
        })?
    } else {
        false
    };

    let (inferred, was_inferred) = if is_schema {
        (user_json, false)
    } else {
        let schema = schemars::schema_for_value!(&user_json);
        let string = json::to_string_pretty(&schema)?;
        (json::from_str(&string)?, true)
    };

    compile_validator(&inferred)?;

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

#[cfg(feature = "execute")]
pub(super) fn prepare_reference_schema(raw: &str) -> flow_like_types::Result<PreparedSchema> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Reference struct schema cannot be empty"));
    }

    let schema = json::from_str::<Value>(trimmed)
        .map_err(|e| anyhow!("Reference struct schema must be valid JSON: {e}"))?;

    let meta_valid =
        looks_like_schema(&schema) && jsonschema::meta::try_is_valid(&schema).unwrap_or(false);
    if schema.get("type").and_then(Value::as_str) != Some("object") || !meta_valid {
        return Err(anyhow!(
            "Reference struct must carry a valid object JSON Schema"
        ));
    }

    compile_validator(&schema)?;

    Ok(PreparedSchema {
        tool_parameters: schema.clone(),
        output_schema: schema,
        mode: ExtractionMode::Direct,
        was_inferred: false,
    })
}

#[cfg(feature = "execute")]
pub(super) fn validate_extracted_value(
    prepared_schema: &PreparedSchema,
    args: Value,
) -> flow_like_types::Result<Value> {
    let extracted = match prepared_schema.mode {
        ExtractionMode::Direct => args,
        ExtractionMode::Wrapped => args
            .get("value")
            .cloned()
            .ok_or_else(|| anyhow!("Tool call missing 'value' field in wrapped mode"))?,
    };

    compile_validator(&prepared_schema.output_schema)?
        .validate(&extracted)
        .map_err(|error| anyhow!("Extracted data does not match the schema: {error}"))?;
    Ok(extracted)
}

#[cfg(feature = "execute")]
const MAX_EXTRACTION_ATTEMPTS: u32 = 3;
#[cfg(feature = "execute")]
const EXTRACTION_BACKOFF_MS: u64 = 250;

/// A failed call is worth repeating only when the failure is transport- or provider-side.
/// Malformed requests and serialization bugs repeat identically, so they fail fast.
#[cfg(feature = "execute")]
fn is_transient(error: &CompletionError) -> bool {
    match error {
        CompletionError::HttpError(_) | CompletionError::ResponseError(_) => true,
        CompletionError::ProviderError(message) => {
            let message = message.to_ascii_lowercase();
            [
                "429",
                "500",
                "502",
                "503",
                "504",
                "overload",
                "rate limit",
                "rate_limit",
                "timeout",
                "timed out",
                "temporarily",
                "unavailable",
            ]
            .iter()
            .any(|needle| message.contains(needle))
        }
        _ => false,
    }
}

/// Last resort when a model answers in prose instead of calling `submit`: pull the first
/// balanced JSON document out of the reply, unwrapping a Markdown fence if there is one.
#[cfg(feature = "execute")]
fn salvage_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.rsplit_once("```").map(|(body, _)| body))
        .unwrap_or(trimmed)
        .trim();

    if let Ok(value) = json::from_str::<Value>(body) {
        return Some(value);
    }

    let start = body.find(['{', '['])?;
    let end = body.rfind(['}', ']'])?;
    if end <= start {
        return None;
    }
    json::from_str::<Value>(&body[start..=end]).ok()
}

#[cfg(feature = "execute")]
pub(super) async fn run_text_extraction(
    context: &mut ExecutionContext,
    prepared_schema: PreparedSchema,
) -> flow_like_types::Result<()> {
    let model_bit = context.evaluate_pin::<Bit>("model").await?;
    let text: String = context.evaluate_pin::<String>("text").await?;
    let hint: String = context.evaluate_pin("hint").await.unwrap_or_default();
    let max_tokens: i64 = context.evaluate_pin("max_tokens").await.unwrap_or_default();

    context.log_message(
        &format!("Using extraction mode: {:?}", prepared_schema.mode),
        LogLevel::Debug,
    );

    let base_input = if hint.trim().is_empty() {
        format!(
            "Extract structured data from the following text according to the schema.\n\nText:\n{}",
            text
        )
    } else {
        format!(
            "Extract structured data from the following text according to the schema.\n\nExtraction hint: {}\n\nText:\n{}",
            hint, text
        )
    };

    let preamble = "You are a knowledge extraction assistant. Extract data by calling the 'submit' tool with structured data matching the provided schema.";
    let model_name = model_bit.meta.get("en").map(|m| m.name.clone());

    let start = Instant::now();
    let mut usage = Usage::default();
    let mut calls: Vec<ModelCallEntry> = Vec::new();
    let mut correction: Option<String> = None;
    let mut last_error: Option<flow_like_types::Error> = None;

    for attempt in 1..=MAX_EXTRACTION_ATTEMPTS {
        if attempt > 1 {
            let backoff = EXTRACTION_BACKOFF_MS * (1 << (attempt - 2));
            flow_like_types::tokio::time::sleep(Duration::from_millis(backoff)).await;
        }

        let input = match &correction {
            Some(correction) => format!("{base_input}\n\n{correction}"),
            None => base_input.clone(),
        };

        let mut agent_builder = model_bit
            .agent(context, &None)
            .await?
            .preamble(preamble)
            .tool(DynamicSubmitTool {
                parameters: prepared_schema.tool_parameters.clone(),
                output_schema: prepared_schema.output_schema.clone(),
            })
            .tool_choice(ToolChoice::Required);

        if max_tokens > 0 {
            agent_builder = agent_builder.max_tokens(max_tokens as u64);
        }

        let agent = agent_builder.build();
        let attempt_start = Instant::now();
        let outcome = match agent
            .completion(input, Vec::<rig::completion::Message>::new())
            .await
        {
            Ok(request) => request.send().await,
            Err(error) => Err(error),
        };

        let response = match outcome {
            Ok(response) => response,
            Err(error) => {
                let transient = is_transient(&error);
                context.log_message(
                    &format!(
                        "Extraction attempt {attempt}/{MAX_EXTRACTION_ATTEMPTS} failed: {error}"
                    ),
                    LogLevel::Warn,
                );
                last_error = Some(anyhow!("Extraction request failed: {error}"));
                if transient {
                    continue;
                }
                break;
            }
        };

        let attempt_usage = Usage::from_rig(response.usage);
        usage.prompt_tokens = usage
            .prompt_tokens
            .saturating_add(attempt_usage.prompt_tokens);
        usage.completion_tokens = usage
            .completion_tokens
            .saturating_add(attempt_usage.completion_tokens);
        usage.total_tokens = usage
            .total_tokens
            .saturating_add(attempt_usage.total_tokens);
        calls.push(ModelCallEntry {
            model: model_name.clone().unwrap_or_default(),
            usage: attempt_usage.clone(),
            duration_ms: Some(attempt_start.elapsed().as_millis() as u64),
        });

        let mut submitted: Option<Value> = None;
        let mut spoken = String::new();
        let mut returned: Vec<&'static str> = Vec::new();
        for content in response.choice {
            match content {
                AssistantContent::ToolCall(ToolCall {
                    function:
                        ToolFunction {
                            name, arguments, ..
                        },
                    ..
                }) => {
                    if name == "submit" {
                        submitted = Some(arguments);
                        returned.push("submit tool call");
                    } else {
                        returned.push("unexpected tool call");
                    }
                }
                AssistantContent::Text(text) => {
                    spoken.push_str(&text.text);
                    returned.push("text");
                }
                AssistantContent::Reasoning(_) => returned.push("reasoning"),
                _ => returned.push("unsupported content"),
            }
        }

        let candidate = match submitted {
            Some(arguments) => Some(arguments),
            None => salvage_json(&spoken).map(|value| match prepared_schema.mode {
                ExtractionMode::Direct => value,
                ExtractionMode::Wrapped => json::json!({ "value": value }),
            }),
        };

        let Some(candidate) = candidate else {
            returned.sort_unstable();
            returned.dedup();
            let observed = if returned.is_empty() {
                "nothing".to_string()
            } else {
                returned.join(", ")
            };
            let truncated =
                max_tokens > 0 && i64::from(attempt_usage.completion_tokens) >= max_tokens;
            context.log_message(
                &format!(
                    "Extraction attempt {attempt}/{MAX_EXTRACTION_ATTEMPTS} returned no submit tool call (returned: {observed})"
                ),
                LogLevel::Warn,
            );
            last_error = Some(anyhow!(
                "Model returned no 'submit' tool call and its reply held no usable JSON (returned: {observed}; {} completion tokens){}",
                attempt_usage.completion_tokens,
                if truncated {
                    ". The reply hit the Max Tokens limit, raise it"
                } else {
                    ""
                }
            ));
            correction = Some(
                "Your previous reply did not call the 'submit' tool. Call 'submit' exactly once with the structured data and return no prose."
                    .to_string(),
            );
            continue;
        };

        match validate_extracted_value(&prepared_schema, candidate) {
            Ok(extracted) => {
                context.log_message("Successfully extracted structured data", LogLevel::Debug);
                let stats = LLMUsageStats {
                    usage,
                    model: model_name,
                    duration_ms: Some(start.elapsed().as_millis() as u64),
                    iterations: Some(attempt),
                    calls,
                };
                context.set_pin_value("response", extracted).await?;
                context.set_pin_value("stats", json::json!(stats)).await?;
                context.activate_exec_pin("exec_out").await?;
                return Ok(());
            }
            Err(error) => {
                context.log_message(
                    &format!(
                        "Extraction attempt {attempt}/{MAX_EXTRACTION_ATTEMPTS} did not match the schema: {error}"
                    ),
                    LogLevel::Warn,
                );
                correction = Some(format!(
                    "Your previous reply did not match the schema: {error}. Fix exactly that problem and call 'submit' again."
                ));
                last_error = Some(error);
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow!("Extraction failed after {MAX_EXTRACTION_ATTEMPTS} attempts")))
}

#[cfg(all(test, feature = "execute"))]
mod extraction_tests {
    use super::*;

    #[test]
    fn reference_extraction_validates_returned_arguments() {
        let prepared = prepare_reference_schema(
            r#"{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false}"#,
        )
        .unwrap();

        assert!(validate_extracted_value(&prepared, json::json!({"name": "Ada"})).is_ok());
        assert!(validate_extracted_value(&prepared, json::json!({"name": 42})).is_err());
    }

    #[test]
    fn wrapped_extraction_validates_the_unwrapped_value() {
        let prepared = prepare_schema(r#"[1, 2]"#).unwrap();

        assert!(validate_extracted_value(&prepared, json::json!({"value": [3, 4]})).is_ok());
        assert!(validate_extracted_value(&prepared, json::json!({"value": ["wrong"]})).is_err());
    }

    #[test]
    fn unusable_schemas_are_rejected_instead_of_panicking() {
        for raw in [
            r#"{"type":"string","pattern":"("}"#,
            r#"{"$schema":"not-a-known-dialect","type":"string"}"#,
            r#"{"type":"object","properties":{"a":{"$ref":"https://example.invalid/x.json"}}}"#,
        ] {
            assert!(
                prepare_schema(raw).is_err(),
                "expected a rejection rather than a panic for {raw}"
            );
        }
    }

    #[test]
    fn external_references_are_refused_without_being_fetched() {
        for reference in [
            "http://169.254.169.254/latest/meta-data/",
            "https://example.invalid/x.json",
            "file:///etc/hostname",
            "file:///proc/self/environ",
        ] {
            let error = compile_validator(&json::json!({
                "type": "object",
                "properties": { "a": { "$ref": reference } }
            }))
            .expect_err(reference)
            .to_string();
            assert!(
                error.contains("is not allowed"),
                "{reference} should be refused by policy, got: {error}"
            );
        }
        assert!(
            compile_validator(&json::json!({
                "$defs": { "a": { "type": "string" } },
                "type": "object",
                "properties": { "a": { "$ref": "#/$defs/a" } }
            }))
            .is_ok()
        );
    }

    #[test]
    fn prose_replies_are_salvaged_into_json() {
        assert_eq!(
            salvage_json("```json\n{\"name\": \"Ada\"}\n```"),
            Some(json::json!({"name": "Ada"}))
        );
        assert_eq!(
            salvage_json("  {\"name\": \"Ada\"}  "),
            Some(json::json!({"name": "Ada"}))
        );
        assert_eq!(
            salvage_json("Sure! Here is the data: {\"name\": \"Ada\"} -- let me know."),
            Some(json::json!({"name": "Ada"}))
        );
        assert_eq!(salvage_json("I cannot help with that."), None);
        assert_eq!(salvage_json(""), None);
    }

    #[test]
    fn only_upstream_failures_are_retried() {
        for message in ["error code: 502", "Rate limit reached", "upstream timeout"] {
            assert!(is_transient(&CompletionError::ProviderError(
                message.to_string()
            )));
        }

        assert!(!is_transient(&CompletionError::ProviderError(
            "invalid api key".to_string()
        )));
    }
}

#[async_trait]
impl NodeLogic for LLMExtractNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_extractor",
            "AI Extractor",
            "Uses an LLM plus a JSON schema to extract structured data from free-form text",
            "AI/Generative",
        );
        node.set_flowscript_name("ai", "extract");
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(5);

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(4)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(8)
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
            "text",
            "Text",
            "Raw text that should be structured via the schema",
            VariableType::String,
        );

        node.add_input_pin(
            "hint",
            "Extraction Hint",
            "Optional hint to guide the extraction (e.g. 'only extract individual line items, not totals')",
            VariableType::String,
        ).set_default_value(Some(json::json!("")));

        node.add_input_pin(
            "max_tokens",
            "Max Tokens",
            "Output token budget for the model. 0 leaves it to the provider. Raise this if large extractions come back empty because the model ran out of room before calling the tool",
            VariableType::Integer,
        )
        .set_default_value(Some(json::json!(0)));

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

        let schema_str: String = context.evaluate_pin("schema").await?;
        let prepared_schema = prepare_schema(&schema_str)?;
        run_text_extraction(context, prepared_schema).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }

    #[cfg(feature = "execute")]
    async fn on_update(&self, node: &mut Node, _board: &Board) {
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
    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;
        node.harmonize_type(vec!["response"], true);
    }
}
