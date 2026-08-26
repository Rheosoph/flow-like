/// # Parse With Schema Node
/// Like utils > types > From String Node but with additional schema validation
/// Input strings must not only by JSON-serializable but also follow the provided schema
/// Schema definitions can either be JSON schemas or OpenAI function definitions.
/// Produces detailed error messages in case of violation.
/// Additionally, this module bundles JSON- and schema-related utility functions.
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_model_provider::history::{Tool, ToolCall, ToolCallFunction};
use flow_like_types::{Error, Value, anyhow, async_trait, json};

//#[derive(Debug, Deserialize, Serialize)]
//pub struct OpenAIFunction {
//    pub r#type: String,
//    pub name: String,
//    pub description: Option<String>,
//    pub parameters: Option<Value>,
//    pub strict: bool,
//}

//#[derive(Debug, Deserialize, Serialize)]
//pub struct OpenAIToolCall {
//    pub name: String,
//    pub args: Value,
//}

/// Is this JSON Value an OpenAI Function Definition?
/// https://platform.openai.com/docs/guides/function-calling?api-mode=responses#defining-functions
pub fn is_openai(data: &Value) -> bool {
    let map = match data.as_object() {
        Some(m) => m,
        None => return false,
    };

    // check "type" is "function"
    if map.get("type") != Some(&Value::String("function".into())) {
        return false;
    }

    // check required fields
    for key in ["name", "description", "parameters", "strict"] {
        if map.get(key).is_none_or(Value::is_null) {
            return false;
        }
    }
    true
}

/// Converts OpenAI Function Defintion to JSON Schema
/// Returns data as-is if not OpenAI
pub fn into_json_schema(data: Value) -> Result<Value, Error> {
    if is_openai(&data) {
        if let Some(obj) = data.as_object()
            && let Some(parameters) = obj.get("parameters")
            && let Some(mut parameters_obj) = parameters.as_object().cloned()
        {
            parameters_obj.remove("additionalProperties");
            let name = match obj.get("name").cloned() {
                Some(name) => name,
                None => Value::Null,
            };
            parameters_obj.insert("title".to_string(), name);
            return Ok(Value::Object(parameters_obj));
        }
        Err(anyhow!("Failed to convert OpenAI function to JSON Schema"))
    } else {
        Ok(data)
    }
}

/// Creates a JSON Schema Validator for a JSON Schema or OpenAI Function Definition
fn get_schema_validator(definition_str: &str) -> Result<jsonschema::Validator, Error> {
    // Is definition input pin value valid JSON?
    let definition: Value = match json::from_str(definition_str) {
        Ok(definition) => definition,
        Err(e) => {
            return Err(anyhow!(format!(
                "Failed to load definition/schema from input string: {}",
                e
            )));
        }
    };

    // Convert defintion into JSON Schema spec
    let schema = into_json_schema(definition)?;

    // Create Schema Validator
    let validator = match jsonschema::validator_for(&schema) {
        Ok(validator) => validator,
        Err(e) => return Err(anyhow!(format!("Failed to load schema validator: {}", e))),
    };
    Ok(validator)
}

/// Validates JSON data against JSON/OpenAI Schema and returns JSON data as Value
/// Returns a JSON Value that is compliant with the given schema
pub fn validate_json_data(schema: &str, data: &str) -> Result<Value, Error> {
    // Get schema validator
    let validator = get_schema_validator(schema)?;

    // Is data input pin value valid JSON?
    let data: Value = match json::from_str(data) {
        Ok(data) => data,
        Err(e) => return Err(anyhow!(format!("Failed to parse JSON data: {}", e))),
    };

    // Validate input data againts JSON schema
    let errors = validator.iter_errors(&data);
    let error_msg = errors
        .map(|e| format!("Error: {}, Location: {}", e, e.instance_path))
        .collect::<Vec<_>>()
        .join("\n");

    if error_msg.is_empty() {
        Ok(data)
    } else {
        Err(anyhow!(format!("Schema validation failed: {}", error_msg)))
    }
}

/// Validates a Tool Call Function str against a list of Tools and returns the Tool Call Object
pub fn tool_call_from_str(tools: &[Tool], tool_call_function: &str) -> Result<ToolCall, Error> {
    // Deserialize tool call
    let tool_call_function: ToolCallFunction = match json::from_str(tool_call_function) {
        Ok(tool_call_function) => tool_call_function,
        Err(e) => return Err(anyhow!(format!("Failed to parse tool call: {}", e))),
    };
    let cuid = flow_like_types::create_id();
    let tool_call = ToolCall {
        id: cuid,
        r#type: "function".to_string(),
        function: tool_call_function,
    };

    for tool in tools.iter() {
        if tool_call.function.name == tool.function.name {
            let schema = json::to_value(&tool.function.parameters)?;
            let validator = match jsonschema::validator_for(&schema) {
                Ok(validator) => validator,
                Err(e) => return Err(anyhow!(format!("Failed to load schema validator: {}", e))),
            };

            // Validate tool call agains function schema
            let tool_call_args: Value = match json::from_str(&tool_call.function.arguments) {
                Ok(tool_call_args) => tool_call_args,
                Err(e) => return Err(anyhow!(format!("Failed to parse tool call args: {}", e))),
            };
            let errors = validator.iter_errors(&tool_call_args);
            let error_msg = errors
                .map(|e| format!("Error: {}, Location: {}", e, e.instance_path))
                .collect::<Vec<_>>()
                .join("\n");

            if error_msg.is_empty() {
                return Ok(tool_call);
            } else {
                return Err(anyhow!(format!("Invalid tool call args: {}", error_msg)));
            }
        }
    }
    Err(anyhow!(format!(
        "No matching function found for tool call: {}",
        tool_call.function.name
    )))
}

#[crate::register_node]
#[derive(Default)]
pub struct ParseWithSchema {}

impl ParseWithSchema {
    pub fn new() -> Self {
        ParseWithSchema {}
    }
}

#[async_trait]
impl NodeLogic for ParseWithSchema {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "parse_with_schema",
            "Parse JSON with Schema",
            "Parse JSON input Data With JSON/OpenAI Schema and Return Value",
            "Utils/JSON",
        );
        node.set_flowscript_name("json", "parseWithSchema");

        node.add_icon("/flow/icons/repair.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "schema",
            "Schema",
            "JSON Schema or OpenAI Function Definition",
            VariableType::String,
        );

        node.add_input_pin(
            "data",
            "Data",
            "JSON Input Data to be parsed",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution continues if parsing succeeds",
            VariableType::Execution,
        );

        node.add_output_pin(
            "parsed",
            "Parsed",
            "Parsed and Validated JSON",
            VariableType::Struct,
        )
        .set_open_schema();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let schema_str: String = context.evaluate_pin("schema").await?;
        let data_str: String = context.evaluate_pin("data").await?;

        let validated = validate_json_data(&schema_str, &data_str)?;

        context.set_pin_value("parsed", validated).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
