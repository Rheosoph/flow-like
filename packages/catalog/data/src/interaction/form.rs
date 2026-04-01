use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic},
    pin::{Pin, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    Value, async_trait, create_id,
    interaction::{InteractionRequest, InteractionStatus, InteractionType},
    json::{Map, from_slice, json},
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::wait::wait_for_interaction_response;

fn pin_default_value(pin: &Pin) -> Option<Value> {
    pin.default_value
        .as_ref()
        .and_then(|value| from_slice::<Value>(value).ok())
}

fn pin_valid_values(pin: &Pin) -> Option<Vec<String>> {
    pin.options
        .as_ref()
        .and_then(|opts| opts.valid_values.as_ref())
        .cloned()
}

fn parse_pin_schema(pin: &Pin) -> Option<Value> {
    pin.schema
        .as_deref()
        .and_then(|schema| flow_like_types::json::from_str::<Value>(schema).ok())
        .filter(|schema| schema.is_object())
}

/// Resolve a string value through board refs if needed.
/// If the value exists as a key in refs, return the resolved value.
/// Otherwise, return the original value.
fn resolve_ref(value: &str, refs: &HashMap<String, String>) -> String {
    refs.get(value)
        .cloned()
        .unwrap_or_else(|| value.to_string())
}

fn with_pin_metadata(mut property: Value, pin: &Pin, refs: &HashMap<String, String>) -> Value {
    if let Some(prop_obj) = property.as_object_mut() {
        if !pin.friendly_name.is_empty() {
            let resolved_name = resolve_ref(&pin.friendly_name, refs);
            prop_obj
                .entry("title".to_string())
                .or_insert_with(|| json!(resolved_name));
        }
        if !pin.description.is_empty() {
            let resolved_description = resolve_ref(&pin.description, refs);
            prop_obj
                .entry("description".to_string())
                .or_insert_with(|| json!(resolved_description));
        }
        if let Some(default_value) = pin_default_value(pin) {
            prop_obj
                .entry("default".to_string())
                .or_insert(default_value);
        }
        if let Some(valid_values) = pin_valid_values(pin)
            && !valid_values.is_empty()
        {
            prop_obj
                .entry("enum".to_string())
                .or_insert_with(|| json!(valid_values));
        }
    }

    property
}

fn pin_to_schema_property(pin: &Pin, refs: &HashMap<String, String>) -> Value {
    let base_property = match pin.data_type {
        VariableType::String | VariableType::PathBuf => json!({ "type": "string" }),
        VariableType::Integer => json!({ "type": "integer" }),
        VariableType::Float => json!({ "type": "number" }),
        VariableType::Boolean => json!({ "type": "boolean" }),
        VariableType::Date => json!({ "type": "string", "format": "date-time" }),
        VariableType::Byte => {
            json!({ "type": "integer", "minimum": 0, "maximum": 255 })
        }
        VariableType::Struct | VariableType::Generic => {
            parse_pin_schema(pin).unwrap_or_else(|| json!({ "type": "object" }))
        }
        VariableType::Execution => json!({ "type": "null" }),
    };

    let property = match pin.value_type {
        ValueType::Array | ValueType::HashSet => {
            json!({
                "type": "array",
                "items": base_property
            })
        }
        ValueType::HashMap => {
            json!({
                "type": "object",
                "additionalProperties": base_property
            })
        }
        ValueType::Normal => base_property,
    };

    with_pin_metadata(property, pin, refs)
}

fn callback_form_pins(pins: &HashMap<String, Pin>) -> Vec<Pin> {
    let mut callback_pins = pins
        .values()
        .filter(|pin| {
            pin.pin_type == PinType::Output
                && pin.data_type != VariableType::Execution
                && pin.name != "payload"
        })
        .cloned()
        .collect::<Vec<_>>();

    callback_pins.sort_by(|a, b| a.index.cmp(&b.index));
    callback_pins
}

fn build_form_json_schema(pins: &[Pin], refs: &HashMap<String, String>) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();

    for pin in pins {
        properties.insert(pin.name.clone(), pin_to_schema_property(pin, refs));
        if pin_default_value(pin).is_none() {
            required.push(pin.name.clone());
        }
    }

    let mut root = Map::new();
    root.insert("type".to_string(), json!("object"));
    root.insert("properties".to_string(), Value::Object(properties));
    root.insert("additionalProperties".to_string(), json!(false));

    if !required.is_empty() {
        root.insert("required".to_string(), json!(required));
    }

    Value::Object(root)
}

async fn build_form_schema_from_callback(
    callback_function: &Arc<InternalNode>,
    refs: &HashMap<String, String>,
) -> flow_like_types::Result<Value> {
    let callback = callback_function.node.lock().await;

    let pins = callback_form_pins(&callback.pins);
    if pins.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Referenced callback function has no form-compatible output pins"
        ));
    }

    Ok(build_form_json_schema(&pins, refs))
}

fn parse_form_node_inputs(
    name: String,
    description: String,
    ttl_seconds: i64,
    schema: Value,
) -> (InteractionRequest, String) {
    let ttl_seconds = ttl_seconds.max(10) as u64;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let interaction_id = create_id();
    let request = InteractionRequest {
        id: interaction_id.clone(),
        name,
        description,
        interaction_type: InteractionType::Form {
            schema: Some(schema),
            fields: Vec::new(),
        },
        status: InteractionStatus::Pending,
        ttl_seconds,
        expires_at: now + ttl_seconds,
        run_id: None,
        app_id: None,
        responder_jwt: None,
    };

    (request, interaction_id)
}

fn select_callback_function(
    context: &mut ExecutionContext,
    referenced_functions: &[Arc<InternalNode>],
) -> flow_like_types::Result<Arc<InternalNode>> {
    let callback_function = referenced_functions
        .first()
        .ok_or_else(|| flow_like_types::anyhow!("No callback function referenced"))?
        .clone();

    if referenced_functions.len() > 1 {
        context.log_message(
            "Multiple callback functions referenced; using the first one",
            LogLevel::Warn,
        );
    }

    Ok(callback_function)
}

fn normalize_form_response(value: &Value) -> flow_like_types::Result<Map<String, Value>> {
    let object = value
        .as_object()
        .ok_or_else(|| flow_like_types::anyhow!("Form response is not a JSON object"))?;

    if let Some(fields) = object.get("fields")
        && let Some(fields_obj) = fields.as_object()
    {
        return Ok(fields_obj.clone());
    }

    Ok(object.clone())
}

fn parse_rfc3339_or_common(value: &str) -> flow_like_types::Result<String> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.with_timezone(&Utc).to_rfc3339());
    }

    if let Ok(parsed) = DateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f %:z") {
        return Ok(parsed.with_timezone(&Utc).to_rfc3339());
    }

    let formats = [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
    ];
    for format in formats {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(parsed.and_utc().to_rfc3339());
        }
    }

    if let Ok(parsed) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        let midnight = NaiveTime::from_hms_opt(0, 0, 0)
            .ok_or_else(|| flow_like_types::anyhow!("Failed to construct midnight time"))?;
        return Ok(parsed.and_time(midnight).and_utc().to_rfc3339());
    }

    Err(flow_like_types::anyhow!(
        "Unsupported date format '{}', expected RFC3339 or ISO date",
        value
    ))
}

fn coerce_to_bool(value: &Value) -> flow_like_types::Result<Value> {
    if let Some(boolean) = value.as_bool() {
        return Ok(Value::Bool(boolean));
    }
    if let Some(number) = value.as_f64() {
        return Ok(Value::Bool(number != 0.0));
    }
    if let Some(string) = value.as_str() {
        let lower = string.trim().to_lowercase();
        if ["true", "yes", "y", "1", "on"].contains(&lower.as_str()) {
            return Ok(Value::Bool(true));
        }
        if ["false", "no", "n", "0", "off"].contains(&lower.as_str()) {
            return Ok(Value::Bool(false));
        }
    }
    Err(flow_like_types::anyhow!(
        "Cannot coerce value '{}' to boolean",
        value
    ))
}

fn coerce_to_integer(value: &Value) -> flow_like_types::Result<Value> {
    if let Some(integer) = value.as_i64() {
        return Ok(json!(integer));
    }
    if let Some(number) = value.as_f64() {
        return Ok(json!(number as i64));
    }
    if let Some(string) = value.as_str() {
        if let Ok(integer) = string.trim().parse::<i64>() {
            return Ok(json!(integer));
        }
        if let Ok(number) = string.trim().parse::<f64>() {
            return Ok(json!(number as i64));
        }
    }
    Err(flow_like_types::anyhow!(
        "Cannot coerce value '{}' to integer",
        value
    ))
}

fn coerce_to_number(value: &Value) -> flow_like_types::Result<Value> {
    if let Some(number) = value.as_f64()
        && let Some(valid_number) = flow_like_types::json::Number::from_f64(number)
    {
        return Ok(Value::Number(valid_number));
    }
    if let Some(string) = value.as_str()
        && let Ok(number) = string.trim().parse::<f64>()
        && let Some(valid_number) = flow_like_types::json::Number::from_f64(number)
    {
        return Ok(Value::Number(valid_number));
    }
    Err(flow_like_types::anyhow!(
        "Cannot coerce value '{}' to number",
        value
    ))
}

fn coerce_to_date_string(value: &Value) -> flow_like_types::Result<Value> {
    use chrono::DateTime;

    if let Some(string) = value.as_str() {
        return Ok(Value::String(parse_rfc3339_or_common(string)?));
    }

    if let Some(integer) = value.as_i64() {
        let dt = if integer > 946_684_800_000 {
            DateTime::from_timestamp_millis(integer)
        } else {
            DateTime::from_timestamp(integer, 0)
        }
        .ok_or_else(|| flow_like_types::anyhow!("Invalid unix timestamp '{}'", integer))?;

        return Ok(Value::String(dt.to_rfc3339()));
    }

    if let Some(number) = value.as_f64() {
        let seconds = number.floor() as i64;
        let nanos = ((number.fract().abs()) * 1_000_000_000.0) as u32;
        let dt = DateTime::from_timestamp(seconds, nanos)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid timestamp '{}'", number))?;
        return Ok(Value::String(dt.to_rfc3339()));
    }

    Err(flow_like_types::anyhow!(
        "Cannot coerce value '{}' to date string",
        value
    ))
}

fn coerce_to_string(value: &Value) -> flow_like_types::Result<Value> {
    if let Some(string) = value.as_str() {
        return Ok(Value::String(string.to_string()));
    }
    let as_string = flow_like_types::json::to_string(value)?;
    Ok(Value::String(as_string))
}

fn coerce_json_string(value: &Value) -> flow_like_types::Result<Value> {
    if let Some(string) = value.as_str() {
        if let Ok(parsed) = flow_like_types::json::from_str::<Value>(string) {
            return Ok(parsed);
        }
        return Ok(Value::String(string.to_string()));
    }
    Ok(value.clone())
}

fn schema_type(schema: &Value) -> Option<&str> {
    schema.get("type").and_then(Value::as_str).or_else(|| {
        if schema.get("properties").is_some() {
            Some("object")
        } else if schema.get("items").is_some() {
            Some("array")
        } else {
            None
        }
    })
}

fn validate_enum(schema: &Value, value: &Value) -> flow_like_types::Result<()> {
    let Some(values) = schema.get("enum").and_then(Value::as_array) else {
        return Ok(());
    };

    if values.iter().any(|candidate| candidate == value) {
        return Ok(());
    }

    Err(flow_like_types::anyhow!(
        "Value '{}' is not part of enum",
        value
    ))
}

fn coerce_value_by_schema(value: &Value, schema: &Value) -> flow_like_types::Result<Value> {
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
        for candidate in any_of {
            if let Ok(coerced) = coerce_value_by_schema(value, candidate) {
                return Ok(coerced);
            }
        }
    }

    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        for candidate in one_of {
            if let Ok(coerced) = coerce_value_by_schema(value, candidate) {
                return Ok(coerced);
            }
        }
    }

    let coerced = match schema_type(schema) {
        Some("boolean") => coerce_to_bool(value)?,
        Some("integer") => coerce_to_integer(value)?,
        Some("number") => coerce_to_number(value)?,
        Some("string") => {
            let format = schema
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if matches!(format, "date" | "date-time") {
                coerce_to_date_string(value)?
            } else {
                coerce_to_string(value)?
            }
        }
        Some("array") => {
            let array_value = if let Some(array) = value.as_array() {
                Value::Array(array.clone())
            } else if let Some(string) = value.as_str() {
                flow_like_types::json::from_str::<Value>(string).map_err(|error| {
                    flow_like_types::anyhow!("Failed to parse array JSON: {error}")
                })?
            } else {
                return Err(flow_like_types::anyhow!("Expected array value"));
            };

            let mut array = array_value
                .as_array()
                .cloned()
                .ok_or_else(|| flow_like_types::anyhow!("Coerced value is not an array"))?;

            if let Some(item_schema) = schema.get("items") {
                for item in &mut array {
                    *item = coerce_value_by_schema(item, item_schema)?;
                }
            }

            Value::Array(array)
        }
        Some("object") => {
            let object_value = if let Some(object) = value.as_object() {
                Value::Object(object.clone())
            } else if let Some(string) = value.as_str() {
                flow_like_types::json::from_str::<Value>(string).map_err(|error| {
                    flow_like_types::anyhow!("Failed to parse object JSON: {error}")
                })?
            } else {
                return Err(flow_like_types::anyhow!("Expected object value"));
            };

            let mut object = object_value
                .as_object()
                .cloned()
                .ok_or_else(|| flow_like_types::anyhow!("Coerced value is not an object"))?;

            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for (name, property_schema) in properties {
                    if let Some(field_value) = object.get(name).cloned() {
                        let coerced_field = coerce_value_by_schema(&field_value, property_schema)?;
                        object.insert(name.clone(), coerced_field);
                    }
                }
            }

            Value::Object(object)
        }
        _ => coerce_json_string(value)?,
    };

    validate_enum(schema, &coerced)?;
    Ok(coerced)
}

fn coerce_value_for_pin(pin: &Pin, value: &Value) -> flow_like_types::Result<Value> {
    let typed = match pin.data_type {
        VariableType::String | VariableType::PathBuf => coerce_to_string(value)?,
        VariableType::Integer => coerce_to_integer(value)?,
        VariableType::Float => coerce_to_number(value)?,
        VariableType::Boolean => coerce_to_bool(value)?,
        VariableType::Date => coerce_to_date_string(value)?,
        VariableType::Byte => {
            let byte = coerce_to_integer(value)?;
            let Some(number) = byte.as_i64() else {
                return Err(flow_like_types::anyhow!("Byte conversion failed"));
            };
            if !(0..=255).contains(&number) {
                return Err(flow_like_types::anyhow!(
                    "Byte value '{}' is out of range 0..255",
                    number
                ));
            }
            json!(number as u8)
        }
        VariableType::Struct | VariableType::Generic => {
            if let Some(schema) = parse_pin_schema(pin) {
                coerce_value_by_schema(value, &schema)?
            } else {
                coerce_json_string(value)?
            }
        }
        VariableType::Execution => {
            return Err(flow_like_types::anyhow!(
                "Execution pins are not valid form data"
            ));
        }
    };

    if let Some(valid_values) = pin_valid_values(pin)
        && let Some(string_value) = typed.as_str()
        && !valid_values
            .iter()
            .any(|candidate| candidate == string_value)
    {
        return Err(flow_like_types::anyhow!(
            "Value '{}' is not a valid option for pin '{}'",
            string_value,
            pin.name
        ));
    }

    Ok(typed)
}

async fn write_form_outputs(
    context: &mut ExecutionContext,
    responded: bool,
    response_value: &Value,
) -> flow_like_types::Result<()> {
    context
        .set_pin_value("response", json!(response_value.to_string()))
        .await?;
    context.set_pin_value("responded", json!(responded)).await?;

    if responded {
        context.activate_exec_pin("exec_out").await?;
    } else {
        context.log_message("Interaction timed out", LogLevel::Warn);
        context.activate_exec_pin("exec_timeout").await?;
    }

    Ok(())
}

async fn execute_callback_if_responded(
    context: &mut ExecutionContext,
    callback_function: &Arc<InternalNode>,
    responded: bool,
    response_value: &Value,
) -> flow_like_types::Result<()> {
    if responded {
        execute_callback_function(context, callback_function, response_value).await?;
    }
    Ok(())
}

fn configure_form_node(node: &mut Node) {
    node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);
    node.add_output_pin(
        "exec_out",
        "Done",
        "Continues after response received",
        VariableType::Execution,
    );
    node.add_output_pin(
        "exec_timeout",
        "Timeout",
        "Continues if no response within TTL",
        VariableType::Execution,
    );

    node.add_input_pin(
        "name",
        "Name",
        "Display name for this interaction",
        VariableType::String,
    )
    .set_default_value(Some(json!("Fill out form")));

    node.add_input_pin(
        "description",
        "Description",
        "Prompt shown to the user",
        VariableType::String,
    )
    .set_default_value(Some(json!("Please fill out the following form:")));

    node.add_input_pin(
        "ttl_seconds",
        "Timeout (seconds)",
        "How long to wait for response",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(300)));

    node.add_output_pin(
        "response",
        "Response",
        "JSON object with pin name -> typed value mappings",
        VariableType::String,
    );
    node.add_output_pin(
        "responded",
        "Responded",
        "Whether the user responded (vs timeout)",
        VariableType::Boolean,
    );
}

async fn execute_callback_function(
    context: &mut ExecutionContext,
    callback_function: &Arc<InternalNode>,
    response_value: &Value,
) -> flow_like_types::Result<()> {
    let response_obj = normalize_form_response(response_value)?;

    let callback_pin_metadata = {
        let callback = callback_function.node.lock().await;
        callback_form_pins(&callback.pins)
    };

    for pin in callback_pin_metadata {
        if let Some(value) = response_obj.get(&pin.name) {
            let coerced = coerce_value_for_pin(&pin, value)?;
            if let Some(internal_pin) = callback_function
                .pins
                .values()
                .find(|internal_pin| internal_pin.name == pin.name)
            {
                internal_pin.set_value(coerced).await;
            }
        }
    }

    let mut sub_context = context.create_sub_context(callback_function).await;
    sub_context.delegated = true;

    let run_result = InternalNode::trigger(&mut sub_context, &mut None, true).await;
    sub_context.end_trace();
    context.push_sub_context(&mut sub_context);

    if let Err(error) = run_result {
        return Err(flow_like_types::anyhow!(
            "Failed to execute callback function: {:?}",
            error
        ));
    }

    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct FormInteraction {}

impl FormInteraction {
    /// Get board refs from the execution context to resolve pin metadata.
    async fn get_board_refs(context: &ExecutionContext) -> HashMap<String, String> {
        let Some(run) = context.run.upgrade() else {
            return HashMap::new();
        };
        let run = run.lock().await;
        run.board.refs.clone()
    }

    async fn run_internal(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let name: String = context.evaluate_pin("name").await?;
        let description: String = context.evaluate_pin("description").await?;
        let ttl_seconds_input: i64 = context.evaluate_pin("ttl_seconds").await?;

        let referenced_functions = context.get_referenced_functions().await?;
        let callback_function = select_callback_function(context, &referenced_functions)?;

        // Get board refs to resolve pin metadata
        let board_refs = Self::get_board_refs(context).await;

        let schema = build_form_schema_from_callback(&callback_function, &board_refs).await?;
        let (request, _interaction_id) =
            parse_form_node_inputs(name, description, ttl_seconds_input, schema);
        let ttl_seconds = request.ttl_seconds;

        let result = wait_for_interaction_response(context, request, ttl_seconds).await?;
        let responded = result.responded;
        let response_value = result.value;

        execute_callback_if_responded(context, &callback_function, responded, &response_value)
            .await?;

        write_form_outputs(context, responded, &response_value).await?;

        Ok(())
    }
}

#[async_trait]
impl NodeLogic for FormInteraction {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "interaction_form",
            "Chat Form",
            "Builds a JSON Schema form from a referenced callback function's pins and executes it with typed submitted values.",
            "Events/Chat/Interaction",
        );
        node.add_icon("/flow/icons/interaction.svg");
        node.set_can_reference_fns(true);

        configure_form_node(&mut node);

        node
    }

    async fn on_update(&self, node: &mut Node, _board: Arc<Board>) {
        if let Some(fn_refs) = &mut node.fn_refs
            && fn_refs.fn_refs.len() > 1
        {
            fn_refs.fn_refs.truncate(1);
        }
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        self.run_internal(context).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::{Pin, PinOptions, PinType, ValueType};
    use flow_like::flow::variable::VariableType;
    use flow_like_types::json::json;

    /// Empty refs for tests that don't need ref resolution
    fn empty_refs() -> HashMap<String, String> {
        HashMap::new()
    }

    fn make_pin(name: &str, data_type: VariableType, value_type: ValueType) -> Pin {
        Pin {
            id: name.to_string(),
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            pin_type: PinType::Output,
            data_type,
            value_type,
            default_value: None,
            options: None,
            schema: None,
            depends_on: std::collections::BTreeSet::new(),
            connected_to: std::collections::BTreeSet::new(),
            index: 0,
            value: None,
        }
    }

    // ==================== pin_to_schema_property tests ====================

    #[test]
    fn test_string_pin_schema() {
        let pin = make_pin("test", VariableType::String, ValueType::Normal);
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("string"));
    }

    #[test]
    fn test_integer_pin_schema() {
        let pin = make_pin("test", VariableType::Integer, ValueType::Normal);
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("integer"));
    }

    #[test]
    fn test_float_pin_schema() {
        let pin = make_pin("test", VariableType::Float, ValueType::Normal);
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("number"));
    }

    #[test]
    fn test_boolean_pin_schema() {
        let pin = make_pin("test", VariableType::Boolean, ValueType::Normal);
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("boolean"));
    }

    #[test]
    fn test_date_pin_schema() {
        let pin = make_pin("test", VariableType::Date, ValueType::Normal);
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("string"));
        assert_eq!(
            schema.get("format").and_then(|v| v.as_str()),
            Some("date-time")
        );
    }

    #[test]
    fn test_byte_pin_schema() {
        let pin = make_pin("test", VariableType::Byte, ValueType::Normal);
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("integer"));
        assert_eq!(schema.get("minimum").and_then(|v| v.as_i64()), Some(0));
        assert_eq!(schema.get("maximum").and_then(|v| v.as_i64()), Some(255));
    }

    #[test]
    fn test_array_value_type_wraps_schema() {
        let pin = make_pin("tags", VariableType::String, ValueType::Array);
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("array"));
        let items = schema.get("items").expect("should have items");
        assert_eq!(items.get("type").and_then(|v| v.as_str()), Some("string"));
    }

    #[test]
    fn test_hashset_value_type_wraps_schema() {
        let pin = make_pin("ids", VariableType::Integer, ValueType::HashSet);
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("array"));
        let items = schema.get("items").expect("should have items");
        assert_eq!(items.get("type").and_then(|v| v.as_str()), Some("integer"));
    }

    #[test]
    fn test_hashmap_value_type_creates_object() {
        let pin = make_pin("metadata", VariableType::String, ValueType::HashMap);
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let additional = schema
            .get("additionalProperties")
            .expect("should have additionalProperties");
        assert_eq!(
            additional.get("type").and_then(|v| v.as_str()),
            Some("string")
        );
    }

    #[test]
    fn test_struct_with_schema_uses_pin_schema() {
        let mut pin = make_pin("message", VariableType::Struct, ValueType::Normal);
        pin.schema = Some(
            r#"{"type":"object","properties":{"role":{"type":"string"},"content":{"type":"string"}}}"#
                .to_string(),
        );
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let props = schema.get("properties").expect("should have properties");
        assert!(props.get("role").is_some());
        assert!(props.get("content").is_some());
    }

    #[test]
    fn test_array_of_struct_with_schema() {
        let mut pin = make_pin("messages", VariableType::Struct, ValueType::Array);
        pin.schema = Some(
            r#"{"type":"object","properties":{"role":{"type":"string"},"content":{"type":"string"}}}"#
                .to_string(),
        );
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("array"));
        let items = schema.get("items").expect("should have items");
        assert_eq!(items.get("type").and_then(|v| v.as_str()), Some("object"));
        let props = items
            .get("properties")
            .expect("items should have properties");
        assert!(props.get("role").is_some());
    }

    #[test]
    fn test_pin_metadata_added() {
        let mut pin = make_pin("user_name", VariableType::String, ValueType::Normal);
        pin.friendly_name = "User Name".to_string();
        pin.description = "The user's display name".to_string();
        pin.default_value = Some(flow_like_types::json::to_vec(&json!("Anonymous")).unwrap());
        let schema = pin_to_schema_property(&pin, &empty_refs());
        assert_eq!(
            schema.get("title").and_then(|v| v.as_str()),
            Some("User Name")
        );
        assert_eq!(
            schema.get("description").and_then(|v| v.as_str()),
            Some("The user's display name")
        );
        assert_eq!(
            schema.get("default").and_then(|v| v.as_str()),
            Some("Anonymous")
        );
    }

    #[test]
    fn test_enum_values_from_pin_options() {
        let mut pin = make_pin("status", VariableType::String, ValueType::Normal);
        pin.options = Some(PinOptions {
            valid_values: Some(vec![
                "pending".to_string(),
                "active".to_string(),
                "completed".to_string(),
            ]),
            ..Default::default()
        });
        let schema = pin_to_schema_property(&pin, &empty_refs());
        let enums = schema.get("enum").expect("should have enum");
        let arr = enums.as_array().expect("enum should be array");
        assert_eq!(arr.len(), 3);
        assert!(arr.iter().any(|v| v.as_str() == Some("pending")));
        assert!(arr.iter().any(|v| v.as_str() == Some("active")));
    }

    // ==================== coerce_to_bool tests ====================

    #[test]
    fn test_coerce_bool_from_bool() {
        assert_eq!(coerce_to_bool(&json!(true)).unwrap(), json!(true));
        assert_eq!(coerce_to_bool(&json!(false)).unwrap(), json!(false));
    }

    #[test]
    fn test_coerce_bool_from_number() {
        assert_eq!(coerce_to_bool(&json!(1)).unwrap(), json!(true));
        assert_eq!(coerce_to_bool(&json!(0)).unwrap(), json!(false));
        assert_eq!(coerce_to_bool(&json!(42)).unwrap(), json!(true));
    }

    #[test]
    fn test_coerce_bool_from_string() {
        assert_eq!(coerce_to_bool(&json!("true")).unwrap(), json!(true));
        assert_eq!(coerce_to_bool(&json!("yes")).unwrap(), json!(true));
        assert_eq!(coerce_to_bool(&json!("Y")).unwrap(), json!(true));
        assert_eq!(coerce_to_bool(&json!("1")).unwrap(), json!(true));
        assert_eq!(coerce_to_bool(&json!("on")).unwrap(), json!(true));
        assert_eq!(coerce_to_bool(&json!("false")).unwrap(), json!(false));
        assert_eq!(coerce_to_bool(&json!("no")).unwrap(), json!(false));
        assert_eq!(coerce_to_bool(&json!("0")).unwrap(), json!(false));
    }

    #[test]
    fn test_coerce_bool_invalid() {
        assert!(coerce_to_bool(&json!("maybe")).is_err());
        assert!(coerce_to_bool(&json!(null)).is_err());
    }

    // ==================== coerce_to_integer tests ====================

    #[test]
    fn test_coerce_integer_from_int() {
        assert_eq!(coerce_to_integer(&json!(42)).unwrap(), json!(42));
        assert_eq!(coerce_to_integer(&json!(-10)).unwrap(), json!(-10));
    }

    #[test]
    fn test_coerce_integer_from_float() {
        assert_eq!(coerce_to_integer(&json!(42.9)).unwrap(), json!(42));
        assert_eq!(coerce_to_integer(&json!(3.14)).unwrap(), json!(3));
    }

    #[test]
    fn test_coerce_integer_from_string() {
        assert_eq!(coerce_to_integer(&json!("123")).unwrap(), json!(123));
        assert_eq!(coerce_to_integer(&json!(" 456 ")).unwrap(), json!(456));
        assert_eq!(coerce_to_integer(&json!("7.8")).unwrap(), json!(7));
    }

    #[test]
    fn test_coerce_integer_invalid() {
        assert!(coerce_to_integer(&json!("not a number")).is_err());
        assert!(coerce_to_integer(&json!(null)).is_err());
    }

    // ==================== coerce_to_number tests ====================

    #[test]
    fn test_coerce_number_from_number() {
        let result = coerce_to_number(&json!(3.14)).unwrap();
        assert!((result.as_f64().unwrap() - 3.14).abs() < 0.001);
    }

    #[test]
    fn test_coerce_number_from_string() {
        let result = coerce_to_number(&json!("2.718")).unwrap();
        assert!((result.as_f64().unwrap() - 2.718).abs() < 0.001);
    }

    #[test]
    fn test_coerce_number_invalid() {
        assert!(coerce_to_number(&json!("not a number")).is_err());
        assert!(coerce_to_number(&json!(null)).is_err());
    }

    // ==================== coerce_to_date_string tests ====================

    #[test]
    fn test_coerce_date_from_rfc3339() {
        let result = coerce_to_date_string(&json!("2024-06-15T10:30:00Z")).unwrap();
        let s = result.as_str().unwrap();
        assert!(s.contains("2024-06-15"));
        assert!(s.contains("10:30:00"));
    }

    #[test]
    fn test_coerce_date_from_iso_date() {
        let result = coerce_to_date_string(&json!("2024-06-15")).unwrap();
        let s = result.as_str().unwrap();
        assert!(s.contains("2024-06-15"));
    }

    #[test]
    fn test_coerce_date_from_timestamp_seconds() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        let result = coerce_to_date_string(&json!(1704067200)).unwrap();
        let s = result.as_str().unwrap();
        assert!(s.contains("2024-01-01"));
    }

    #[test]
    fn test_coerce_date_from_timestamp_millis() {
        // 2024-01-01 00:00:00 UTC in millis = 1704067200000
        let result = coerce_to_date_string(&json!(1704067200000_i64)).unwrap();
        let s = result.as_str().unwrap();
        assert!(s.contains("2024-01-01"));
    }

    // ==================== coerce_to_string tests ====================

    #[test]
    fn test_coerce_string_from_string() {
        assert_eq!(coerce_to_string(&json!("hello")).unwrap(), json!("hello"));
    }

    #[test]
    fn test_coerce_string_from_other() {
        let result = coerce_to_string(&json!(42)).unwrap();
        assert_eq!(result.as_str(), Some("42"));
        let result = coerce_to_string(&json!(true)).unwrap();
        assert_eq!(result.as_str(), Some("true"));
    }

    // ==================== normalize_form_response tests ====================

    #[test]
    fn test_normalize_flat_response() {
        let response = json!({ "name": "Alice", "age": 30 });
        let result = normalize_form_response(&response).unwrap();
        assert_eq!(result.get("name").and_then(|v| v.as_str()), Some("Alice"));
        assert_eq!(result.get("age").and_then(|v| v.as_i64()), Some(30));
    }

    #[test]
    fn test_normalize_legacy_fields_response() {
        let response = json!({ "fields": { "name": "Bob", "email": "bob@example.com" } });
        let result = normalize_form_response(&response).unwrap();
        assert_eq!(result.get("name").and_then(|v| v.as_str()), Some("Bob"));
        assert_eq!(
            result.get("email").and_then(|v| v.as_str()),
            Some("bob@example.com")
        );
    }

    // ==================== coerce_value_by_schema tests ====================

    #[test]
    fn test_coerce_by_schema_string() {
        let schema = json!({"type": "string"});
        assert_eq!(
            coerce_value_by_schema(&json!(42), &schema).unwrap(),
            json!("42")
        );
    }

    #[test]
    fn test_coerce_by_schema_integer() {
        let schema = json!({"type": "integer"});
        assert_eq!(
            coerce_value_by_schema(&json!("123"), &schema).unwrap(),
            json!(123)
        );
    }

    #[test]
    fn test_coerce_by_schema_boolean() {
        let schema = json!({"type": "boolean"});
        assert_eq!(
            coerce_value_by_schema(&json!("yes"), &schema).unwrap(),
            json!(true)
        );
    }

    #[test]
    fn test_coerce_by_schema_date_time() {
        let schema = json!({"type": "string", "format": "date-time"});
        let result = coerce_value_by_schema(&json!("2024-06-15"), &schema).unwrap();
        assert!(result.as_str().unwrap().contains("2024-06-15"));
    }

    #[test]
    fn test_coerce_by_schema_array() {
        let schema = json!({"type": "array", "items": {"type": "integer"}});
        let result = coerce_value_by_schema(&json!(["1", "2", "3"]), &schema).unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0], json!(1));
        assert_eq!(arr[1], json!(2));
        assert_eq!(arr[2], json!(3));
    }

    #[test]
    fn test_coerce_by_schema_object() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "count": {"type": "integer"}
            }
        });
        let value = json!({"name": "test", "count": "42"});
        let result = coerce_value_by_schema(&value, &schema).unwrap();
        assert_eq!(result.get("name"), Some(&json!("test")));
        assert_eq!(result.get("count"), Some(&json!(42)));
    }

    #[test]
    fn test_coerce_by_schema_nested_array_of_objects() {
        let schema = json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "role": {"type": "string"},
                    "enabled": {"type": "boolean"}
                }
            }
        });
        let value = json!([
            {"role": "admin", "enabled": "yes"},
            {"role": "user", "enabled": "0"}
        ]);
        let result = coerce_value_by_schema(&value, &schema).unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0].get("enabled"), Some(&json!(true)));
        assert_eq!(arr[1].get("enabled"), Some(&json!(false)));
    }

    #[test]
    fn test_coerce_by_schema_enum_valid() {
        let schema = json!({"type": "string", "enum": ["a", "b", "c"]});
        assert_eq!(
            coerce_value_by_schema(&json!("b"), &schema).unwrap(),
            json!("b")
        );
    }

    #[test]
    fn test_coerce_by_schema_enum_invalid() {
        let schema = json!({"type": "string", "enum": ["a", "b", "c"]});
        assert!(coerce_value_by_schema(&json!("d"), &schema).is_err());
    }

    // ==================== build_form_json_schema tests ====================

    #[test]
    fn test_build_form_schema_from_pins() {
        let pins = vec![
            make_pin("name", VariableType::String, ValueType::Normal),
            make_pin("age", VariableType::Integer, ValueType::Normal),
            make_pin("active", VariableType::Boolean, ValueType::Normal),
        ];
        let schema = build_form_json_schema(&pins, &empty_refs());
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let props = schema.get("properties").expect("should have properties");
        assert!(props.get("name").is_some());
        assert!(props.get("age").is_some());
        assert!(props.get("active").is_some());
    }

    #[test]
    fn test_build_form_schema_required_fields() {
        let mut pin_with_default = make_pin("optional", VariableType::String, ValueType::Normal);
        pin_with_default.default_value =
            Some(flow_like_types::json::to_vec(&json!("default")).unwrap());
        let pin_required = make_pin("required_field", VariableType::String, ValueType::Normal);

        let schema = build_form_json_schema(&[pin_required, pin_with_default], &empty_refs());
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("should have required array");
        assert!(
            required
                .iter()
                .any(|v| v.as_str() == Some("required_field"))
        );
        assert!(!required.iter().any(|v| v.as_str() == Some("optional")));
    }
}
