use flow_like_wasm_node_template as _;
use flow_like_wasm_sdk::*;

/// Auto-discovers all `#[register_node]` nodes via `inventory`.
fn all_template_nodes() -> Vec<NodeDefinition> {
    let nodes: Vec<_> = inventory::iter::<WasmNodeEntry>
        .into_iter()
        .map(|entry| (entry.get_node)())
        .collect();
    assert!(
        !nodes.is_empty(),
        "template nodes must be linked into this test"
    );
    nodes
}

fn uses_host_schema(node: &NodeDefinition, pin: &PinDefinition) -> bool {
    // Bit::schema() is supplied by the runtime and unavailable in native tests.
    node.name == "weather_agent" && pin.name == "model"
}

#[test]
fn lint_no_duplicate_input_output_pin_names() {
    for node in all_template_nodes() {
        let input_names: std::collections::HashSet<&str> = node
            .pins
            .iter()
            .filter(|p| p.pin_type == PinType::Input)
            .map(|p| p.name.as_str())
            .collect();

        for pin in node.pins.iter().filter(|p| p.pin_type == PinType::Output) {
            assert!(
                !input_names.contains(pin.name.as_str()),
                "[{}] Input and output pin share the name \"{}\"",
                node.name,
                pin.name
            );
        }
    }
}

#[test]
fn lint_impure_nodes_have_both_exec_sides() {
    for node in all_template_nodes() {
        let has_input_exec = node
            .pins
            .iter()
            .any(|p| p.pin_type == PinType::Input && p.data_type == VariableType::Execution);
        let has_output_exec = node
            .pins
            .iter()
            .any(|p| p.pin_type == PinType::Output && p.data_type == VariableType::Execution);

        if has_input_exec || has_output_exec {
            assert!(
                has_input_exec && has_output_exec,
                "[{}] Impure node must have both input and output exec pins",
                node.name
            );
        }
    }
}

#[test]
fn lint_struct_pins_have_schema() {
    for node in all_template_nodes() {
        for pin in &node.pins {
            if uses_host_schema(&node, pin) {
                continue;
            }
            if pin.data_type == VariableType::Struct {
                assert!(
                    pin.schema.as_ref().map_or(false, |s| !s.trim().is_empty()),
                    "[{}] Struct pin \"{}\" ({:?}) has no schema",
                    node.name,
                    pin.name,
                    pin.pin_type
                );
            }
        }
    }
}

#[test]
fn lint_no_root_array_schemas() {
    for node in all_template_nodes() {
        for pin in &node.pins {
            if uses_host_schema(&node, pin) {
                continue;
            }
            if let Some(schema_str) = &pin.schema {
                let schema: serde_json::Value =
                    serde_json::from_str(schema_str).expect("schema must be valid JSON");
                assert_ne!(
                    schema.get("type").and_then(|t| t.as_str()),
                    Some("array"),
                    "[{}] Pin \"{}\" has a root-level array schema — use ValueType::Array",
                    node.name,
                    pin.name
                );
            }
        }
    }
}

#[test]
fn lint_every_node_has_description_and_category() {
    for node in all_template_nodes() {
        assert!(
            !node.description.trim().is_empty(),
            "[{}] Missing description",
            node.name
        );
        assert!(
            !node.category.trim().is_empty(),
            "[{}] Missing category",
            node.name
        );
    }
}

#[test]
fn lint_no_generic_pins() {
    for node in all_template_nodes() {
        for pin in &node.pins {
            assert_ne!(
                pin.data_type,
                VariableType::Generic,
                "[{}] Pin \"{}\" uses Generic type — use a specific type",
                node.name,
                pin.name
            );
        }
    }
}

#[test]
fn lint_no_pathbuf_pins() {
    for node in all_template_nodes() {
        for pin in &node.pins {
            assert_ne!(
                pin.data_type,
                VariableType::PathBuf,
                "[{}] Pin \"{}\" uses PathBuf type — use FlowPath (Struct) instead",
                node.name,
                pin.name
            );
        }
    }
}
