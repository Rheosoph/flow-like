use super::add_screenshot_pins;
#[cfg(feature = "execute")]
use super::{SubmitTool, call_tool, missing_tool_call, require_screenshot, vision_history};
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
use flow_like_types::{Value, anyhow, async_trait, json};

#[cfg(feature = "execute")]
const TOOL: &str = "submit_extraction";

#[derive(Debug)]
struct PreparedSchema {
    output_schema: Value,
}

impl PreparedSchema {
    /// Tool arguments must be an object, so other schemas are wrapped as `{ "value": … }`.
    #[cfg(any(feature = "execute", test))]
    fn wrapped(&self) -> bool {
        self.output_schema.get("type").and_then(Value::as_str) != Some("object")
    }

    #[cfg(feature = "execute")]
    fn tool_parameters(&self) -> Value {
        if self.wrapped() {
            json::json!({
                "type": "object",
                "properties": {"value": self.output_schema.clone()},
                "required": ["value"],
                "additionalProperties": false
            })
        } else {
            self.output_schema.clone()
        }
    }
}

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

/// Compiles the schema (meta-schema checked, remote `$ref`s refused, linear-time regexes) and
/// lists up to five places where `instance` violates it. `None` only checks the schema.
fn schema_violations(
    schema: &Value,
    instance: Option<&Value>,
) -> flow_like_types::Result<Vec<String>> {
    let validator = flow_like_catalog_core::ontology_action_parameter_validator(schema)
        .map_err(|e| anyhow!("Schema is not a usable JSON Schema: {e}"))?;
    Ok(instance
        .map(|instance| {
            validator
                .iter_errors(instance)
                .take(5)
                .map(|error| {
                    let path = error.instance_path.to_string();
                    let path = if path.is_empty() { "/" } else { path.as_str() };
                    format!("{path}: {error}")
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Accepts a JSON Schema, or example JSON whose schema is inferred.
fn prepare_schema(raw: &str) -> flow_like_types::Result<PreparedSchema> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Schema cannot be empty"));
    }
    let user_json = json::from_str::<Value>(trimmed)
        .map_err(|e| anyhow!("Schema must be valid JSON (a JSON Schema or example JSON): {e}"))?;

    let output_schema =
        if looks_like_schema(&user_json) && schema_violations(&user_json, None).is_ok() {
            user_json
        } else {
            let inferred = json::to_value(schemars::schema_for_value!(&user_json))?;
            schema_violations(&inferred, None)?;
            inferred
        };

    Ok(PreparedSchema { output_schema })
}

/// Unwraps the tool arguments and checks them against the user's schema, naming each failing
/// path.
#[cfg(any(feature = "execute", test))]
fn extracted_value(prepared: &PreparedSchema, arguments: Value) -> flow_like_types::Result<Value> {
    let data = if prepared.wrapped() {
        arguments
            .get("value")
            .cloned()
            .ok_or_else(|| anyhow!("The model's `submit_extraction` call has no 'value' field"))?
    } else {
        arguments
    };
    let violations = schema_violations(&prepared.output_schema, Some(&data))?;
    if !violations.is_empty() {
        return Err(anyhow!(
            "Extracted data does not match the schema: {}",
            violations.join("; ")
        ));
    }
    Ok(data)
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMExtractFromScreenNode {}

impl LLMExtractFromScreenNode {
    pub fn new() -> Self {
        Self {}
    }
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
        node.set_flowscript_name("automation.llm", "extractFromScreen");
        node.add_icon("/flow/icons/bot-search.svg");
        node.set_version(5);

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

        add_screenshot_pins(&mut node, "Screenshot", false);

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
            "Extracted structured data, validated against the schema",
            VariableType::Generic,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let schema_str: String = context.evaluate_pin("schema").await?;
        let hint: String = context.evaluate_pin("hint").await.unwrap_or_default();
        let prepared = prepare_schema(&schema_str)?;
        let screenshot = require_screenshot(context).await?;

        let request = if hint.is_empty() {
            "Extract the requested data from this screenshot.".to_string()
        } else {
            format!("Extract the requested data from this screenshot. Hint: {hint}")
        };
        let instructions =
            format!("{request}\n\nCall `{TOOL}` with data matching its parameter schema.");

        let preamble = "You are a data extraction expert. Extract structured data from screenshots according to the provided schema. Report only what is visible; never invent values.";

        let arguments = call_tool(
            context,
            &model_bit,
            vision_history(&[&screenshot.image], &instructions),
            preamble,
            SubmitTool {
                name: TOOL,
                description: "Submit extracted structured data",
                parameters: prepared.tool_parameters(),
            },
        )
        .await?
        .ok_or_else(|| missing_tool_call(TOOL))?;

        let data = extracted_value(&prepared, arguments)?;

        context.set_pin_value("data", data).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
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

        let Some(raw) = schema_value else {
            return;
        };
        match prepare_schema(&raw) {
            Ok(prepared) => {
                let (pin_schema, value_type) =
                    match prepared.output_schema.get("type").and_then(Value::as_str) {
                        Some("array") => (
                            prepared
                                .output_schema
                                .get("items")
                                .cloned()
                                .unwrap_or(json::json!({})),
                            ValueType::Array,
                        ),
                        _ => (prepared.output_schema, ValueType::Normal),
                    };
                if let Some(pin) = node.get_pin_mut_by_name("data") {
                    pin.schema = json::to_string(&pin_schema).ok();
                    pin.value_type = value_type;
                    pin.data_type = VariableType::Struct;
                }
            }
            Err(e) => node.error = Some(format!("Schema error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_schema_is_used_directly_and_violations_name_the_path() {
        let prepared = prepare_schema(
            r#"{"type":"object","properties":{"total":{"type":"number"},"items":{"type":"array","items":{"type":"string"}}},"required":["total"]}"#,
        )
        .unwrap();
        assert!(!prepared.wrapped());
        assert!(extracted_value(&prepared, json::json!({"total": 3, "items": ["a"]})).is_ok());
        let error = extracted_value(&prepared, json::json!({"total": "3", "items": [1]}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("/total"), "{error}");
        assert!(error.contains("/items/0"), "{error}");
        let missing = extracted_value(&prepared, json::json!({}))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("total"), "{missing}");
    }

    #[test]
    fn example_json_and_array_schemas_are_wrapped() {
        let example = prepare_schema(r#"{"type": "invoice", "total": 3}"#).unwrap();
        assert!(!example.wrapped());
        assert!(example.output_schema["properties"]["total"].is_object());

        let list = prepare_schema(r#"{"type":"array","items":{"type":"string"}}"#).unwrap();
        assert!(list.wrapped());
        assert_eq!(
            extracted_value(&list, json::json!({"value": ["a", "b"]})).unwrap(),
            json::json!(["a", "b"])
        );
        assert!(extracted_value(&list, json::json!({"value": [1]})).is_err());
        assert!(extracted_value(&list, json::json!({"other": []})).is_err());
        assert!(prepare_schema("  ").is_err());
        assert!(prepare_schema("{not json").is_err());
    }
}
