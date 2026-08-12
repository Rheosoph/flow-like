//! Shared helpers for package (micro) widget nodes: contract-driven pin
//! generation, element-ref tracing, and props assembly.

use flow_like::a2ui::micro_widget::{
    ContractInput, ContractInputType, WidgetContract, encode_package_widget_ref,
};
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, remove_pin},
    pin::{Pin, PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::Value;
use flow_like_types::json::{Map, json};
use std::collections::BTreeSet;

/// Prefix for contract-input pins (`dyn_in_{inputKey}`)
pub const DYN_INPUT_PREFIX: &str = "dyn_in_";
/// Prefix for query-argument pins (`dyn_arg_{argKey}`)
pub const DYN_ARG_PREFIX: &str = "dyn_arg_";

/// JSON-Schema extension carried by widget-reference output pins. Keeping the
/// frozen contract on the connection lets downstream dynamic nodes configure
/// themselves without reloading the package registry (which may be unavailable
/// or still starting on the host handling the board update).
pub const WIDGET_REF_METADATA_KEY: &str = "x-flow-like-widget";

#[derive(Debug, Clone)]
pub struct WidgetContractMetadata {
    pub selector: String,
    pub widget_name: String,
    pub contract: WidgetContract,
}

/// Attach a resolved package widget contract to an element-reference pin while
/// preserving any ordinary JSON Schema already used by struct tooling.
pub fn set_widget_ref_metadata(pin: &mut Pin, selector: &str, widget_name: &str, contract: &Value) {
    let mut schema = pin
        .schema
        .as_deref()
        .and_then(|value| flow_like_types::json::from_str::<Value>(value).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "type": "object" }));

    if let Some(object) = schema.as_object_mut() {
        object.insert(
            WIDGET_REF_METADATA_KEY.to_string(),
            json!({
                "selector": selector,
                "widgetName": widget_name,
                "contract": contract,
            }),
        );
    }
    pin.schema = schema_string(&schema);
}

/// Remove only the widget-specific extension, leaving the pin's structural
/// schema intact when the Instantiate Widget selector changes to a classic
/// declarative widget.
pub fn clear_widget_ref_metadata(pin: &mut Pin) {
    let Some(schema) = pin.schema.as_deref() else {
        return;
    };
    let Ok(mut schema) = flow_like_types::json::from_str::<Value>(schema) else {
        return;
    };
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if object.remove(WIDGET_REF_METADATA_KEY).is_some() {
        pin.schema = schema_string(&schema);
    }
}

fn metadata_from_schema(schema: &Value) -> Option<WidgetContractMetadata> {
    let metadata = schema.get(WIDGET_REF_METADATA_KEY)?;
    let selector = metadata.get("selector")?.as_str()?.to_string();
    let widget_name = metadata
        .get("widgetName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let contract = flow_like_types::json::from_value(metadata.get("contract")?.clone()).ok()?;
    Some(WidgetContractMetadata {
        selector,
        widget_name,
        contract,
    })
}

/// Read frozen widget metadata from the pin feeding an input. The traversal
/// crosses reroutes and layer boundaries, preferring a live upstream schema
/// over any copied boundary snapshot.
pub fn connected_widget_contract(
    board: &Board,
    node: &Node,
    pin_name: &str,
) -> Option<WidgetContractMetadata> {
    let input = node.get_pin_by_name(pin_name)?;
    let mut queue: Vec<(String, usize)> = input
        .depends_on
        .iter()
        .cloned()
        .map(|pin_id| (pin_id, 0))
        .collect();
    let mut visited = BTreeSet::new();
    let mut terminal_metadata: Option<(usize, WidgetContractMetadata)> = None;
    let mut relay_metadata: Option<(usize, WidgetContractMetadata)> = None;
    let mut reached_terminal = false;

    while let Some((source_id, depth)) = queue.pop() {
        if !visited.insert(source_id.clone()) || visited.len() > 128 {
            continue;
        }
        let Some(source) = board.get_pin_by_id(&source_id) else {
            continue;
        };

        let metadata = source.schema.as_deref().and_then(|schema_ref| {
            let schema = board
                .refs
                .get(schema_ref)
                .map(String::as_str)
                .unwrap_or(schema_ref);
            flow_like_types::json::from_str::<Value>(schema)
                .ok()
                .and_then(|parsed| metadata_from_schema(&parsed))
        });

        // Layer boundary pins retain a dependency on their internal source.
        let mut upstream: Vec<String> = source.depends_on.iter().cloned().collect();

        // Reroute outputs do not depend directly on route_in, so cross the
        // visual pass-through explicitly.
        if let Some(producer) = find_node_with_pin(board, &source.id)
            && producer.name == "reroute"
            && let Some(route_in) = producer.get_pin_by_name("route_in")
        {
            upstream.extend(route_in.depends_on.iter().cloned());
        }

        if upstream.is_empty() {
            reached_terminal = true;
            if let Some(metadata) = metadata
                && terminal_metadata
                    .as_ref()
                    .is_none_or(|(best_depth, _)| depth >= *best_depth)
            {
                terminal_metadata = Some((depth, metadata));
            }
        } else {
            if let Some(metadata) = metadata
                && relay_metadata
                    .as_ref()
                    .is_none_or(|(best_depth, _)| depth >= *best_depth)
            {
                relay_metadata = Some((depth, metadata));
            }
            queue.extend(upstream.into_iter().map(|pin_id| (pin_id, depth + 1)));
        }
    }

    if let Some((_, metadata)) = terminal_metadata {
        Some(metadata)
    } else if reached_terminal {
        None
    } else {
        relay_metadata.map(|(_, metadata)| metadata)
    }
}

/// Build the same frozen metadata from a page's concrete
/// `microWidgetInstance` component so `Get Element -> Query Widget` can expose
/// the contract for widgets placed directly in the visual builder.
pub fn widget_contract_from_component(component: &Value) -> Option<WidgetContractMetadata> {
    if component.get("type").and_then(Value::as_str) != Some("microWidgetInstance") {
        return None;
    }
    let package_id = component.get("packageId")?.as_str()?;
    let widget_id = component.get("widgetId")?.as_str()?;
    let contract_value = component.get("contract")?.clone();
    let contract = flow_like_types::json::from_value(contract_value).ok()?;
    Some(WidgetContractMetadata {
        selector: encode_package_widget_ref(package_id, widget_id),
        widget_name: widget_id.to_string(),
        contract,
    })
}

/// How a JSON Schema maps onto a pin: the type of a single element, whether the
/// pin carries a list of them, and the schema describing that element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinShape {
    pub data_type: VariableType,
    pub value_type: ValueType,
    pub schema: Option<String>,
}

impl PinShape {
    fn scalar(data_type: VariableType) -> Self {
        Self {
            data_type,
            value_type: ValueType::Normal,
            schema: None,
        }
    }

    /// Write the shape onto a pin. Struct and Generic pins keep their schema so
    /// downstream struct helpers can list the element's properties.
    pub fn apply(self, pin: &mut Pin) {
        pin.data_type = self.data_type;
        pin.value_type = self.value_type;
        pin.schema = self.schema;
    }
}

/// Pin `VariableType` for a raw JSON Schema (query args / results).
pub fn json_schema_variable_type(schema: &Value) -> VariableType {
    match schema.get("type").and_then(|t| t.as_str()) {
        Some("string") => VariableType::String,
        Some("number") => VariableType::Float,
        Some("integer") => VariableType::Integer,
        Some("boolean") => VariableType::Boolean,
        Some("object") => VariableType::Struct,
        _ if schema
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|values| !values.is_empty() && values.iter().all(Value::is_string)) =>
        {
            VariableType::String
        }
        _ => VariableType::Generic,
    }
}

/// Pin shape for a raw JSON Schema.
///
/// An array schema describes the container, but a pin models one element plus
/// `ValueType::Array`. Handing the array schema straight to the pin is what left
/// `Make Struct` reporting "Schema has no object properties" — it needs the
/// `items` schema.
pub fn json_schema_pin_shape(schema: &Value) -> PinShape {
    if schema.get("type").and_then(|t| t.as_str()) == Some("array") {
        let untyped_list = PinShape {
            data_type: VariableType::Generic,
            value_type: ValueType::Array,
            schema: None,
        };
        return match schema.get("items") {
            Some(items) if items.is_object() => {
                let element = json_schema_pin_shape(items);
                // A pin cannot express a list of lists — keep the outer list.
                if element.value_type == ValueType::Array {
                    untyped_list
                } else {
                    PinShape {
                        value_type: ValueType::Array,
                        ..element
                    }
                }
            }
            // Untyped or tuple-typed arrays have no single element schema.
            _ => untyped_list,
        };
    }

    let data_type = json_schema_variable_type(schema);
    let carries_schema = matches!(data_type, VariableType::Struct | VariableType::Generic);
    PinShape {
        data_type,
        value_type: ValueType::Normal,
        schema: carries_schema.then(|| schema_string(schema)).flatten(),
    }
}

/// Pin shape for a contract input: string/enum -> String, number -> Float,
/// integer -> Integer, boolean -> Boolean, json -> whatever its schema
/// describes (Generic when it declares none).
pub fn contract_input_pin_shape(input: &ContractInput) -> PinShape {
    match input.input_type {
        ContractInputType::String | ContractInputType::Enum => {
            PinShape::scalar(VariableType::String)
        }
        ContractInputType::Number => PinShape::scalar(VariableType::Float),
        ContractInputType::Integer => PinShape::scalar(VariableType::Integer),
        ContractInputType::Boolean => PinShape::scalar(VariableType::Boolean),
        ContractInputType::Json => match &input.schema {
            Some(schema) => json_schema_pin_shape(schema),
            None => PinShape::scalar(VariableType::Generic),
        },
    }
}

/// Serialize a JSON Schema value for `Pin.schema`.
pub fn schema_string(schema: &Value) -> Option<String> {
    flow_like_types::json::to_string(schema).ok()
}

/// Expected `dyn_in_*` pin names for a contract.
pub fn expected_contract_input_pin_names(contract: &WidgetContract) -> BTreeSet<String> {
    contract
        .inputs
        .keys()
        .map(|key| format!("{DYN_INPUT_PREFIX}{key}"))
        .collect()
}

fn contract_input_pin_options(input: &ContractInput) -> Option<PinOptions> {
    let mut options = PinOptions::new();
    let mut has_options = false;

    if input.input_type == ContractInputType::Enum
        && let Some(choices) = &input.choices
        && !choices.is_empty()
    {
        options.set_valid_values(choices.clone());
        has_options = true;
    }
    if let (Some(min), Some(max)) = (input.min, input.max) {
        options.set_range((min, max));
        has_options = true;
    }

    has_options.then(|| options.build())
}

/// Add one typed input pin per contract input. Existing pins keep their value
/// and connections but are re-shaped from the contract, so a contract change —
/// or a corrected mapping — reaches boards that already placed the node. With
/// `with_defaults`, the contract default becomes the pin default — used by
/// Instantiate Widget; Update Widget Inputs omits defaults so only
/// explicitly set pins produce patch entries.
pub fn add_contract_input_pins(node: &mut Node, contract: &WidgetContract, with_defaults: bool) {
    for (key, input) in &contract.inputs {
        let pin_name = format!("{DYN_INPUT_PREFIX}{key}");
        let shape = contract_input_pin_shape(input);
        let description = input.description.clone().unwrap_or_else(|| key.clone());
        let options = contract_input_pin_options(input);

        if let Some(existing) = node.get_pin_mut_by_name(&pin_name) {
            existing.friendly_name = key.clone();
            existing.description = description;
            shape.apply(existing);
            existing.options = options;
            continue;
        }

        let pin = node.add_input_pin(&pin_name, key, &description, shape.data_type.clone());
        shape.apply(pin);
        pin.options = options;

        if with_defaults
            && let Some(default) = &input.default
            && let Ok(bytes) = flow_like_types::json::to_vec(default)
        {
            pin.default_value = Some(bytes);
        }
    }
}

/// Remove pins with the given prefix that are not in the keep set.
pub fn remove_stale_prefixed_pins(node: &mut Node, prefix: &str, keep: &BTreeSet<String>) {
    let stale: Vec<_> = node
        .pins
        .values()
        .filter(|p| p.name.starts_with(prefix) && !keep.contains(&p.name))
        .cloned()
        .collect();
    for pin in stale {
        remove_pin(node, Some(pin));
    }
}

fn find_node_with_pin<'a>(board: &'a Board, pin_id: &str) -> Option<&'a Node> {
    if let Some(node) = board.nodes.values().find(|n| n.pins.contains_key(pin_id)) {
        return Some(node);
    }
    board
        .layers
        .values()
        .find_map(|layer| layer.nodes.values().find(|n| n.pins.contains_key(pin_id)))
}

fn pin_default_string(pin: Option<&Pin>) -> Option<String> {
    pin.and_then(|p| p.default_value.as_ref())
        .and_then(|bytes| flow_like_types::json::from_slice::<String>(bytes).ok())
        .filter(|s| !s.is_empty())
}

/// Trace an input pin's connection back to the producing
/// `a2ui_instantiate_widget` node (hopping through reroutes) and return that
/// node's selected `widget_selector` value.
pub fn trace_widget_selector(board: &Board, node: &Node, pin_name: &str) -> Option<String> {
    let pin = node.get_pin_by_name(pin_name)?;
    let mut queue: Vec<String> = pin.depends_on.iter().cloned().collect();
    let mut visited = BTreeSet::new();

    while let Some(pin_id) = queue.pop() {
        if !visited.insert(pin_id.clone()) || visited.len() > 128 {
            continue;
        }

        // Cross layer boundary pins and other graph relays that retain their
        // upstream dependency even when they are not owned by a Node.
        if let Some(source) = board.get_pin_by_id(&pin_id) {
            queue.extend(source.depends_on.iter().cloned());
        }

        let Some(producer) = find_node_with_pin(board, &pin_id) else {
            continue;
        };
        match producer.name.as_str() {
            "a2ui_instantiate_widget" => {
                return pin_default_string(producer.get_pin_by_name("widget_selector"));
            }
            "reroute" => {
                if let Some(route_in) = producer.get_pin_by_name("route_in") {
                    queue.extend(route_in.depends_on.iter().cloned());
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether an input pin is connected to another node.
pub fn pin_connected(node: &Node, pin_name: &str) -> bool {
    node.get_pin_by_name(pin_name)
        .map(|p| !p.depends_on.is_empty())
        .unwrap_or(false)
}

/// Collect values of set/connected input pins with the given prefix into a
/// map keyed by the pin name without the prefix. Null / unset pins are
/// omitted.
pub async fn collect_prefixed_pin_values(
    context: &mut ExecutionContext,
    prefix: &str,
) -> Map<String, Value> {
    let mut values = Map::new();
    let pins: Vec<_> = context.node.pins.values().cloned().collect();
    for pin in pins {
        if pin.pin_type != PinType::Input || !pin.name.starts_with(prefix) {
            continue;
        }
        let key = pin.name[prefix.len()..].to_string();
        if let Ok(val) = context.evaluate_pin_ref::<Value>(pin.clone()).await
            && !val.is_null()
        {
            values.insert(key, val);
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::a2ui::micro_widget::ContractInput;
    use flow_like_storage::object_store::path::Path;
    use flow_like_types::json::json;

    fn input(input_type: ContractInputType) -> ContractInput {
        ContractInput {
            input_type,
            description: None,
            default: None,
            choices: None,
            min: None,
            max: None,
            schema: None,
            optional: false,
        }
    }

    fn json_input(schema: Value) -> ContractInput {
        let mut input = input(ContractInputType::Json);
        input.schema = Some(schema);
        input
    }

    #[test]
    fn test_contract_input_pin_shape_mapping() {
        for (input_type, expected) in [
            (ContractInputType::String, VariableType::String),
            (ContractInputType::Enum, VariableType::String),
            (ContractInputType::Number, VariableType::Float),
            (ContractInputType::Integer, VariableType::Integer),
            (ContractInputType::Boolean, VariableType::Boolean),
            (ContractInputType::Json, VariableType::Generic),
        ] {
            let shape = contract_input_pin_shape(&input(input_type));
            assert_eq!(shape.data_type, expected);
            assert_eq!(shape.value_type, ValueType::Normal);
        }

        let object_schema = json!({"type": "object", "properties": {"a": {"type": "string"}}});
        let object = contract_input_pin_shape(&json_input(object_schema.clone()));
        assert_eq!(object.data_type, VariableType::Struct);
        assert_eq!(object.value_type, ValueType::Normal);
        assert_eq!(object.schema, schema_string(&object_schema));
    }

    #[test]
    fn test_array_schema_yields_a_list_pin_of_the_item_type() {
        let item_schema = json!({"type": "object", "properties": {"label": {"type": "string"}}});
        let shape = contract_input_pin_shape(&json_input(json!({
            "type": "array",
            "items": item_schema.clone()
        })));

        assert_eq!(shape.data_type, VariableType::Struct);
        assert_eq!(shape.value_type, ValueType::Array);
        // The item schema, not the array wrapper — Make Struct reads properties off it.
        assert_eq!(shape.schema, schema_string(&item_schema));
    }

    #[test]
    fn test_array_of_scalars_and_untyped_arrays() {
        let strings = json_schema_pin_shape(&json!({"type": "array", "items": {"type": "string"}}));
        assert_eq!(strings.data_type, VariableType::String);
        assert_eq!(strings.value_type, ValueType::Array);
        assert_eq!(strings.schema, None);

        let untyped = json_schema_pin_shape(&json!({"type": "array"}));
        assert_eq!(untyped.data_type, VariableType::Generic);
        assert_eq!(untyped.value_type, ValueType::Array);
        assert_eq!(untyped.schema, None);
    }

    #[test]
    fn test_json_schema_variable_type() {
        assert_eq!(
            json_schema_variable_type(&json!({"type": "string"})),
            VariableType::String
        );
        assert_eq!(
            json_schema_variable_type(&json!({"type": "integer"})),
            VariableType::Integer
        );
        assert_eq!(
            json_schema_variable_type(&json!({"type": "number"})),
            VariableType::Float
        );
        assert_eq!(
            json_schema_variable_type(&json!({"type": "boolean"})),
            VariableType::Boolean
        );
        assert_eq!(
            json_schema_variable_type(&json!({"type": "object"})),
            VariableType::Struct
        );
        assert_eq!(
            json_schema_variable_type(&json!({"type": "array"})),
            VariableType::Generic
        );
        assert_eq!(
            json_schema_variable_type(&json!({"enum": ["a", "b"]})),
            VariableType::String
        );
        assert_eq!(
            json_schema_variable_type(&json!({"type": "integer", "enum": [1, 2]})),
            VariableType::Integer
        );
        assert_eq!(
            json_schema_variable_type(&json!({"type": "boolean", "enum": [true, false]})),
            VariableType::Boolean
        );
    }

    #[test]
    fn test_contract_input_pin_generation() {
        let contract: WidgetContract = flow_like_types::json::from_value(json!({
            "contractVersion": 1,
            "id": "sales-chart",
            "inputs": {
                "title": { "type": "string", "default": "Sales", "description": "Chart headline" },
                "variant": { "type": "enum", "choices": ["bar", "line"], "default": "bar" },
                "limit": { "type": "integer", "min": 1.0, "max": 500.0, "default": 50 },
                "rows": {
                    "type": "json",
                    "schema": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": { "label": { "type": "string" } }
                        }
                    },
                    "optional": true
                }
            }
        }))
        .unwrap();

        let expected = expected_contract_input_pin_names(&contract);
        assert_eq!(
            expected,
            BTreeSet::from([
                "dyn_in_limit".to_string(),
                "dyn_in_rows".to_string(),
                "dyn_in_title".to_string(),
                "dyn_in_variant".to_string(),
            ])
        );

        let mut node = Node::new("test", "Test", "Test", "Test");
        add_contract_input_pins(&mut node, &contract, true);

        let title = node.get_pin_by_name("dyn_in_title").unwrap();
        assert_eq!(title.data_type, VariableType::String);
        assert_eq!(title.description, "Chart headline");
        assert_eq!(
            title.default_value.as_deref(),
            Some(b"\"Sales\"".as_slice())
        );

        let variant = node.get_pin_by_name("dyn_in_variant").unwrap();
        assert_eq!(
            variant.options.as_ref().unwrap().valid_values,
            Some(vec!["bar".to_string(), "line".to_string()])
        );

        let limit = node.get_pin_by_name("dyn_in_limit").unwrap();
        assert_eq!(limit.data_type, VariableType::Integer);
        assert_eq!(limit.options.as_ref().unwrap().range, Some((1.0, 500.0)));

        let rows = node.get_pin_by_name("dyn_in_rows").unwrap();
        assert_eq!(rows.data_type, VariableType::Struct);
        assert_eq!(rows.value_type, ValueType::Array);
        let rows_schema = rows.schema.as_deref().unwrap();
        assert!(rows_schema.contains("label"));
        assert!(!rows_schema.contains("array"));
        assert!(rows.default_value.is_none());

        // Without defaults, no pin carries a default value
        let mut patch_node = Node::new("test2", "Test2", "Test2", "Test");
        add_contract_input_pins(&mut patch_node, &contract, false);
        assert!(
            patch_node
                .get_pin_by_name("dyn_in_title")
                .unwrap()
                .default_value
                .is_none()
        );

        // Stale removal keeps expected pins only
        let mut keep = expected.clone();
        keep.remove("dyn_in_rows");
        remove_stale_prefixed_pins(&mut node, DYN_INPUT_PREFIX, &keep);
        assert!(node.get_pin_by_name("dyn_in_rows").is_none());
        assert!(node.get_pin_by_name("dyn_in_title").is_some());
    }

    #[test]
    fn existing_contract_input_pin_refreshes_contract_fields_without_losing_state() {
        let initial: WidgetContract = flow_like_types::json::from_value(json!({
            "contractVersion": 1,
            "id": "changing-widget",
            "inputs": {
                "mode": {
                    "type": "json",
                    "description": "Original payload",
                    "default": [],
                    "schema": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": { "label": { "type": "string" } }
                        }
                    }
                }
            }
        }))
        .unwrap();

        let mut node = Node::new("test", "Test", "Test", "Test");
        add_contract_input_pins(&mut node, &initial, true);
        let pin = node.get_pin_mut_by_name("dyn_in_mode").unwrap();
        let pin_id = pin.id.clone();
        assert_eq!(pin.data_type, VariableType::Struct);
        assert_eq!(pin.value_type, ValueType::Array);
        pin.friendly_name = "Stale label".to_string();
        pin.description = "Stale description".to_string();
        pin.depends_on.insert("source-pin".to_string());
        pin.connected_to.insert("target-pin".to_string());
        pin.set_default_value(Some(json!("bar")));

        let enum_contract: WidgetContract = flow_like_types::json::from_value(json!({
            "contractVersion": 1,
            "id": "changing-widget",
            "inputs": {
                "mode": {
                    "type": "enum",
                    "description": "Rendering style",
                    "choices": ["bar", "line"],
                    "default": "line"
                }
            }
        }))
        .unwrap();
        add_contract_input_pins(&mut node, &enum_contract, true);

        let pin = node.get_pin_by_name("dyn_in_mode").unwrap();
        assert_eq!(pin.id, pin_id);
        assert_eq!(pin.friendly_name, "mode");
        assert_eq!(pin.description, "Rendering style");
        assert_eq!(pin.data_type, VariableType::String);
        assert_eq!(pin.value_type, ValueType::Normal);
        assert_eq!(pin.schema, None);
        assert_eq!(
            pin.options.as_ref().unwrap().valid_values,
            Some(vec!["bar".to_string(), "line".to_string()])
        );
        assert_eq!(pin.options.as_ref().unwrap().range, None);
        assert_eq!(pin.default_value.as_deref(), Some(b"\"bar\"".as_slice()));
        assert!(pin.depends_on.contains("source-pin"));
        assert!(pin.connected_to.contains("target-pin"));

        // A later refresh also replaces enum options with the new range while
        // retaining a newly configured value and the same graph identity.
        node.get_pin_mut_by_name("dyn_in_mode")
            .unwrap()
            .set_default_value(Some(json!(7)));
        let ranged_contract: WidgetContract = flow_like_types::json::from_value(json!({
            "contractVersion": 1,
            "id": "changing-widget",
            "inputs": {
                "mode": {
                    "type": "integer",
                    "description": "Bucket count",
                    "min": 1.0,
                    "max": 10.0,
                    "default": 10
                }
            }
        }))
        .unwrap();
        add_contract_input_pins(&mut node, &ranged_contract, true);

        let pin = node.get_pin_by_name("dyn_in_mode").unwrap();
        assert_eq!(pin.id, pin_id);
        assert_eq!(pin.friendly_name, "mode");
        assert_eq!(pin.description, "Bucket count");
        assert_eq!(pin.data_type, VariableType::Integer);
        assert_eq!(pin.options.as_ref().unwrap().valid_values, None);
        assert_eq!(pin.options.as_ref().unwrap().range, Some((1.0, 10.0)));
        assert_eq!(pin.default_value.as_deref(), Some(b"7".as_slice()));
        assert!(pin.depends_on.contains("source-pin"));
        assert!(pin.connected_to.contains("target-pin"));
    }

    #[test]
    fn widget_reference_metadata_survives_a_pin_connection() {
        let contract = json!({
            "contractVersion": 1,
            "id": "sales-chart",
            "queries": {
                "getTotal": {
                    "argsSchema": null,
                    "resultSchema": { "type": "number" }
                }
            }
        });
        let mut source = Node::new("source", "Source", "Source", "Test");
        let source_pin = source.add_output_pin(
            "element_ref",
            "Element Ref",
            "Widget reference",
            VariableType::Struct,
        );
        set_widget_ref_metadata(
            source_pin,
            "pkg:com.example.sales/sales-chart",
            "Sales Chart",
            &contract,
        );
        let source_pin_id = source_pin.id.clone();

        let mut target = Node::new("target", "Target", "Target", "Test");
        target
            .add_input_pin(
                "element_ref",
                "Element Ref",
                "Widget reference",
                VariableType::Struct,
            )
            .depends_on
            .insert(source_pin_id);

        let mut board = Board::new_detached(Some("metadata-test".into()), Path::default());
        board.nodes.insert(source.id.clone(), source);

        let metadata = connected_widget_contract(&board, &target, "element_ref").unwrap();
        assert_eq!(metadata.selector, "pkg:com.example.sales/sales-chart");
        assert_eq!(metadata.widget_name, "Sales Chart");
        assert!(metadata.contract.queries.contains_key("getTotal"));
    }

    #[test]
    fn widget_reference_metadata_traces_through_a_reroute() {
        let contract = json!({
            "contractVersion": 1,
            "id": "sales-chart",
            "queries": { "getTotal": {} }
        });
        let mut source = Node::new("source", "Source", "Source", "Test");
        let source_pin = source.add_output_pin(
            "element_ref",
            "Element Ref",
            "Widget reference",
            VariableType::Struct,
        );
        set_widget_ref_metadata(
            source_pin,
            "pkg:com.example.sales/sales-chart",
            "Sales Chart",
            &contract,
        );
        let source_pin_id = source_pin.id.clone();

        let mut reroute = Node::new("reroute", "Reroute", "Reroute", "Control");
        reroute
            .add_input_pin("route_in", "In", "", VariableType::Generic)
            .depends_on
            .insert(source_pin_id);
        let reroute_output_id = reroute
            .add_output_pin("route_out", "Out", "", VariableType::Generic)
            .id
            .clone();

        let mut target = Node::new("target", "Target", "Target", "Test");
        target
            .add_input_pin(
                "element_ref",
                "Element Ref",
                "Widget reference",
                VariableType::Struct,
            )
            .depends_on
            .insert(reroute_output_id);

        let mut board = Board::new_detached(Some("metadata-reroute-test".into()), Path::default());
        board.nodes.insert(source.id.clone(), source);
        board.nodes.insert(reroute.id.clone(), reroute);

        let metadata = connected_widget_contract(&board, &target, "element_ref").unwrap();
        assert_eq!(metadata.widget_name, "Sales Chart");
        assert!(metadata.contract.queries.contains_key("getTotal"));
    }

    #[test]
    fn upstream_widget_metadata_wins_over_a_stale_layer_bridge_snapshot() {
        let old_contract = json!({
            "contractVersion": 1,
            "id": "old-chart",
            "queries": { "oldQuery": {} }
        });
        let current_contract = json!({
            "contractVersion": 1,
            "id": "sales-chart",
            "queries": { "getTotal": {} }
        });

        let mut source = Node::new("source", "Source", "Source", "Test");
        let source_pin = source.add_output_pin(
            "element_ref",
            "Element Ref",
            "Widget reference",
            VariableType::Struct,
        );
        set_widget_ref_metadata(
            source_pin,
            "pkg:com.example.sales/sales-chart",
            "Sales Chart",
            &current_contract,
        );
        let source_pin_id = source_pin.id.clone();

        // A layer output pin is cloned from the producer during cleanup. Its
        // copied schema can be older, but it still depends on the live source.
        let mut bridge = Node::new("bridge", "Bridge", "Bridge", "Test");
        let bridge_pin = bridge.add_output_pin(
            "layer_output",
            "Layer Output",
            "Boundary snapshot",
            VariableType::Struct,
        );
        set_widget_ref_metadata(
            bridge_pin,
            "pkg:com.example.old/old-chart",
            "Old Chart",
            &old_contract,
        );
        bridge_pin.depends_on.insert(source_pin_id);
        let bridge_pin_id = bridge_pin.id.clone();

        let mut target = Node::new("target", "Target", "Target", "Test");
        target
            .add_input_pin(
                "element_ref",
                "Element Ref",
                "Widget reference",
                VariableType::Struct,
            )
            .depends_on
            .insert(bridge_pin_id);

        let mut board = Board::new_detached(Some("metadata-bridge-test".into()), Path::default());
        board.nodes.insert(source.id.clone(), source);
        board.nodes.insert(bridge.id.clone(), bridge);

        let metadata = connected_widget_contract(&board, &target, "element_ref").unwrap();
        assert_eq!(metadata.widget_name, "Sales Chart");
        assert!(metadata.contract.queries.contains_key("getTotal"));
        assert!(!metadata.contract.queries.contains_key("oldQuery"));
    }

    #[test]
    fn cleared_upstream_widget_metadata_invalidates_a_stale_bridge_snapshot() {
        let old_contract = json!({
            "contractVersion": 1,
            "id": "old-chart",
            "queries": { "oldQuery": {} }
        });

        let mut source = Node::new("source", "Source", "Source", "Test");
        let source_pin_id = source
            .add_output_pin(
                "element_ref",
                "Element Ref",
                "Current non-package reference",
                VariableType::Struct,
            )
            .id
            .clone();

        let mut bridge = Node::new("bridge", "Bridge", "Bridge", "Test");
        let bridge_pin = bridge.add_output_pin(
            "layer_output",
            "Layer Output",
            "Boundary snapshot",
            VariableType::Struct,
        );
        set_widget_ref_metadata(
            bridge_pin,
            "pkg:com.example.old/old-chart",
            "Old Chart",
            &old_contract,
        );
        bridge_pin.depends_on.insert(source_pin_id);
        let bridge_pin_id = bridge_pin.id.clone();

        let mut target = Node::new("target", "Target", "Target", "Test");
        target
            .add_input_pin(
                "element_ref",
                "Element Ref",
                "Widget reference",
                VariableType::Struct,
            )
            .depends_on
            .insert(bridge_pin_id);

        let mut board = Board::new_detached(Some("metadata-cleared-test".into()), Path::default());
        board.nodes.insert(source.id.clone(), source);
        board.nodes.insert(bridge.id.clone(), bridge);

        assert!(connected_widget_contract(&board, &target, "element_ref").is_none());
    }

    #[test]
    fn legacy_selector_tracing_crosses_a_layer_bridge_pin() {
        let mut instantiate = Node::new(
            "a2ui_instantiate_widget",
            "Instantiate Widget",
            "Instantiate",
            "UI",
        );
        instantiate
            .add_input_pin(
                "widget_selector",
                "Widget",
                "Selected widget",
                VariableType::String,
            )
            .set_default_value(Some(json!("classic-widget")));
        let source_pin_id = instantiate
            .add_output_pin(
                "element_ref",
                "Element Ref",
                "Widget reference",
                VariableType::Struct,
            )
            .id
            .clone();

        let mut bridge = Node::new("bridge", "Bridge", "Bridge", "Test");
        let bridge_pin = bridge.add_output_pin(
            "layer_output",
            "Layer Output",
            "Boundary",
            VariableType::Struct,
        );
        bridge_pin.depends_on.insert(source_pin_id);
        let bridge_pin_id = bridge_pin.id.clone();

        let mut target = Node::new("target", "Target", "Target", "Test");
        target
            .add_input_pin(
                "element_ref",
                "Element Ref",
                "Widget reference",
                VariableType::Struct,
            )
            .depends_on
            .insert(bridge_pin_id);

        let mut board = Board::new_detached(Some("selector-bridge-test".into()), Path::default());
        board.nodes.insert(instantiate.id.clone(), instantiate);
        board.nodes.insert(bridge.id.clone(), bridge);

        assert_eq!(
            trace_widget_selector(&board, &target, "element_ref").as_deref(),
            Some("classic-widget")
        );
    }

    #[test]
    fn visual_builder_component_exposes_its_frozen_contract() {
        let metadata = widget_contract_from_component(&json!({
            "type": "microWidgetInstance",
            "instanceId": "sales-1",
            "packageId": "com.example.sales",
            "widgetId": "sales-chart",
            "contract": {
                "contractVersion": 1,
                "id": "sales-chart",
                "queries": { "getSelection": {} }
            }
        }))
        .unwrap();

        assert_eq!(metadata.selector, "pkg:com.example.sales/sales-chart");
        assert_eq!(metadata.widget_name, "sales-chart");
        assert!(metadata.contract.queries.contains_key("getSelection"));
        assert!(widget_contract_from_component(&json!({ "type": "button" })).is_none());
    }
}
