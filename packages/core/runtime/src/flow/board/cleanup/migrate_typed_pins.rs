use std::collections::{HashMap, HashSet};

use crate::{
    flow::{
        board::Board,
        node::Node,
        pin::{
            Pin, PinType, ValueType, is_open_object_schema, resolve_schema, schemas_are_compatible,
        },
        variable::VariableType,
    },
    state::FlowNodeRegistryInner,
};
use flow_like_types::Value;

struct Change {
    node: String,
    layer: Option<String>,
    pin: Pin,
}

fn expected_contract(node: &str, pin: &str) -> Option<(u32, VariableType, ValueType, PinType)> {
    match (node, pin) {
        ("browser_upload_multiple_files", "file_paths") => {
            Some((1, VariableType::String, ValueType::Array, PinType::Input))
        }
        ("llm_observe_screen", "elements")
        | ("llm_plan_actions", "actions")
        | ("llm_rank_candidates", "ranked") => {
            Some((4, VariableType::Struct, ValueType::Array, PinType::Output))
        }
        ("llm_resolve_element" | "llm_rank_candidates", "candidates") => {
            Some((4, VariableType::Struct, ValueType::Array, PinType::Input))
        }
        ("ai_systemone_noul" | "ai_systemone_choice" | "ai_systemone_score", "state") => {
            Some((2, VariableType::String, ValueType::Normal, PinType::Input))
        }
        _ => None,
    }
}

fn valid_literal(pin: &Pin, bytes: &[u8]) -> bool {
    let Ok(value) = flow_like_types::json::from_slice::<Value>(bytes) else {
        return false;
    };
    if pin.value_type == ValueType::Normal {
        return pin.data_type == VariableType::String && value.is_string();
    }
    let Value::Array(values) = value else {
        return false;
    };
    match pin.data_type {
        VariableType::String => values.iter().all(Value::is_string),
        VariableType::Struct => {
            let Some(schema) = pin.schema.as_deref() else {
                return false;
            };
            let Ok(schema) = flow_like_types::json::from_str::<Value>(schema) else {
                return false;
            };
            let Ok(validator) = jsonschema::validator_for(&schema) else {
                return false;
            };
            values.iter().all(|value| validator.is_valid(value))
        }
        _ => false,
    }
}

fn collect_changes(node: &Node, layer: Option<&str>, catalog: &Node, changes: &mut Vec<Change>) {
    for old in node.pins.values() {
        let Some((version, data_type, value_type, direction)) =
            expected_contract(&node.name, &old.name)
        else {
            continue;
        };
        if catalog.version.is_none_or(|catalog| catalog < version)
            || node.version.is_some_and(|placed| placed >= version)
            || old.data_type != VariableType::Generic
            || !matches!(old.value_type, ValueType::Normal | ValueType::Array)
            || (value_type == ValueType::Normal && old.value_type != ValueType::Normal)
            || old.pin_type != direction
        {
            continue;
        }
        let Some(target) = catalog.get_pin_by_name(&old.name) else {
            continue;
        };
        if target.data_type != data_type
            || target.value_type != value_type
            || target.pin_type != direction
        {
            continue;
        }
        // Require a usable contract even when this particular pin has no literal.
        let empty_literal = if value_type == ValueType::Normal {
            &b"\"\""[..]
        } else {
            &b"[]"[..]
        };
        if !valid_literal(target, empty_literal) {
            continue;
        }
        let mut pin = old.clone();
        pin.data_type = target.data_type.clone();
        pin.value_type = target.value_type.clone();
        pin.schema = target.schema.clone();
        pin.options = target.options.clone();
        pin.default_value = old
            .default_value
            .as_ref()
            .filter(|bytes| valid_literal(target, bytes))
            .cloned()
            .or_else(|| target.default_value.clone());
        changes.push(Change {
            node: node.id.clone(),
            layer: layer.map(str::to_owned),
            pin,
        });
    }
}

fn compatible(source: &(Pin, bool), target: &(Pin, bool), refs: &HashMap<String, String>) -> bool {
    let (source, source_is_layer) = source;
    let (target, target_is_layer) = target;
    if (!source_is_layer && source.pin_type != PinType::Output)
        || (!target_is_layer && target.pin_type != PinType::Input)
        || source.data_type == VariableType::Execution
        || target.data_type == VariableType::Execution
    {
        return false;
    }

    if target.data_type == VariableType::Generic {
        let enforce_container = [source, target].iter().any(|pin| {
            pin.options
                .as_ref()
                .and_then(|options| options.enforce_generic_value_type)
                .unwrap_or(false)
        });
        if enforce_container && source.value_type != target.value_type {
            return false;
        }
    } else if source.data_type == VariableType::Generic
        || source.data_type != target.data_type
        || source.value_type != target.value_type
    {
        // A Generic producer does not promise that its next value matches a typed input.
        return false;
    }

    let schema = |pin: &Pin| -> Option<Option<String>> {
        pin.schema
            .as_deref()
            .map(|schema| {
                resolve_schema(schema, refs)
                    .ok()
                    .filter(|resolved| {
                        flow_like_types::json::from_str::<Value>(resolved)
                            .is_ok_and(|value| value.is_object() || value.is_boolean())
                    })
                    .map(str::to_owned)
            })
            .map_or(Some(None), |resolved| resolved.map(Some))
    };
    let (Some(source_schema), Some(target_schema)) = (schema(source), schema(target)) else {
        return false;
    };
    if target_schema
        .as_deref()
        .is_some_and(|schema| !is_open_object_schema(schema))
        && source_schema.as_deref().is_none_or(is_open_object_schema)
    {
        return false;
    }
    schemas_are_compatible(source_schema.as_deref(), target_schema.as_deref())
}

/// Preserve compatible values for known pin upgrades before schema synchronization clears them.
pub(super) fn migrate_typed_pins(board: &mut Board, registry: &FlowNodeRegistryInner) {
    let mut catalogs = HashMap::new();
    let mut changes = Vec::new();
    for (node, layer) in
        board
            .nodes
            .values()
            .map(|node| (node, None))
            .chain(board.layers.iter().flat_map(|(id, layer)| {
                layer
                    .nodes
                    .values()
                    .map(move |node| (node, Some(id.as_str())))
            }))
    {
        if !node
            .pins
            .values()
            .any(|pin| expected_contract(&node.name, &pin.name).is_some())
        {
            continue;
        }
        let catalog = catalogs
            .entry(node.name.clone())
            .or_insert_with(|| registry.get_node(&node.name).ok());
        if let Some(catalog) = catalog {
            collect_changes(node, layer, catalog, &mut changes);
        }
    }
    if changes.is_empty() {
        return;
    }

    // Compare against every endpoint's contract after its own pending catalog upgrade.
    let mut projected: HashMap<String, (Pin, bool)> = HashMap::new();
    for node in board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
    {
        let mut projected_node = node.clone();
        if let Ok(catalog) = registry.get_node(&node.name)
            && catalog
                .version
                .is_some_and(|version| node.version.is_none_or(|placed| version > placed))
        {
            super::sync_node_schema::sync_node_with_catalog(&mut projected_node, &catalog);
        }
        projected.extend(
            projected_node
                .pins
                .into_values()
                .map(|pin| (pin.id.clone(), (pin, false))),
        );
    }
    for layer in board.layers.values() {
        projected.extend(
            layer
                .pins
                .values()
                .cloned()
                .map(|pin| (pin.id.clone(), (pin, true))),
        );
    }

    let mut removed = HashSet::new();
    for change in &changes {
        for target in &change.pin.connected_to {
            if !projected
                .get(&change.pin.id)
                .zip(projected.get(target))
                .is_some_and(|(source, target)| compatible(source, target, &board.refs))
            {
                removed.insert((change.pin.id.clone(), target.clone()));
            }
        }
        for source in &change.pin.depends_on {
            if !projected
                .get(source)
                .zip(projected.get(&change.pin.id))
                .is_some_and(|(source, target)| compatible(source, target, &board.refs))
            {
                removed.insert((source.clone(), change.pin.id.clone()));
            }
        }
    }
    for change in changes {
        let node = match change.layer {
            Some(layer) => board
                .layers
                .get_mut(&layer)
                .and_then(|layer| layer.nodes.get_mut(&change.node)),
            None => board.nodes.get_mut(&change.node),
        };
        if let Some(node) = node {
            node.pins.insert(change.pin.id.clone(), change.pin);
        }
    }
    let prune = |pin: &mut Pin| {
        pin.connected_to
            .retain(|target| !removed.contains(&(pin.id.clone(), target.clone())));
        pin.depends_on
            .retain(|source| !removed.contains(&(source.clone(), pin.id.clone())));
    };
    for node in board.nodes.values_mut() {
        node.pins.values_mut().for_each(prune);
    }
    for layer in board.layers.values_mut() {
        layer.pins.values_mut().for_each(prune);
        for node in layer.nodes.values_mut() {
            node.pins.values_mut().for_each(prune);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{
        board::cleanup::sync_node_schema::sync_board_node_schemas,
        execution::context::ExecutionContext, node::NodeLogic,
    };
    use flow_like_types::{Result, async_trait, json::json};
    use std::sync::Arc;

    struct Definition(Node);

    #[async_trait]
    impl NodeLogic for Definition {
        fn get_node(&self) -> Node {
            self.0.clone()
        }

        async fn run(&self, _: &mut ExecutionContext) -> Result<()> {
            Ok(())
        }
    }

    fn state_node(name: &str, version: u32, data_type: VariableType, value: Value) -> Node {
        let mut node = Node::new(name, name, "", "AI/Decisions");
        node.set_version(version);
        node.add_input_pin("state", "State", "", data_type)
            .set_default_value(Some(value));
        node
    }

    fn registry(catalog: Node) -> FlowNodeRegistryInner {
        let mut registry = FlowNodeRegistryInner::new(1);
        registry.insert(catalog.clone(), Arc::new(Definition(catalog)));
        registry
    }

    #[tokio::test]
    async fn systemone_state_upgrade_preserves_existing_text_literals() {
        for name in [
            "ai_systemone_noul",
            "ai_systemone_choice",
            "ai_systemone_score",
        ] {
            for text in ["The delivery arrived damaged.", ""] {
                let old = state_node(name, 1, VariableType::Generic, json!(text));
                let node_id = old.id.clone();
                let original = old.get_pin_by_name("state").unwrap().clone();
                let catalog = state_node(name, 2, VariableType::String, json!("new default"));
                let mut board = Board::new_detached(None, "typed-pin-test".into());
                board.nodes.insert(node_id.clone(), old);

                sync_board_node_schemas(&mut board, &registry(catalog)).await;

                let updated = &board.nodes[&node_id];
                let state = updated.get_pin_by_name("state").unwrap();
                assert_eq!(updated.version, Some(2));
                assert_eq!(state.id, original.id);
                assert_eq!(state.data_type, VariableType::String);
                assert_eq!(state.value_type, ValueType::Normal);
                assert_eq!(state.default_value, original.default_value);
            }
        }
    }

    #[tokio::test]
    async fn systemone_state_upgrade_retains_only_string_connections() {
        for (source_type, source_container, retain) in [
            (VariableType::String, ValueType::Normal, true),
            (VariableType::String, ValueType::Array, false),
            (VariableType::Struct, ValueType::Normal, false),
            (VariableType::Generic, ValueType::Normal, false),
        ] {
            let name = "ai_systemone_choice";
            let mut old = state_node(name, 1, VariableType::Generic, json!("saved text"));
            let node_id = old.id.clone();
            let state_id = old.get_pin_by_name("state").unwrap().id.clone();
            let mut source = Node::new("source", "Source", "", "");
            let source_id = source.id.clone();
            let source_pin = source.add_output_pin("value", "Value", "", source_type);
            source_pin.value_type = source_container;
            source_pin.connected_to.insert(state_id.clone());
            let output_id = source_pin.id.clone();
            old.pins
                .get_mut(&state_id)
                .unwrap()
                .depends_on
                .insert(output_id.clone());
            let catalog = state_node(name, 2, VariableType::String, json!("new default"));
            let mut board = Board::new_detached(None, "typed-pin-test".into());
            board.nodes.insert(node_id.clone(), old);
            board.nodes.insert(source_id.clone(), source);

            sync_board_node_schemas(&mut board, &registry(catalog)).await;

            let state = &board.nodes[&node_id].pins[&state_id];
            let output = &board.nodes[&source_id].pins[&output_id];
            assert_eq!(state.data_type, VariableType::String);
            assert_eq!(state.depends_on.contains(&output_id), retain);
            assert_eq!(output.connected_to.contains(&state_id), retain);
        }
    }

    #[tokio::test]
    async fn systemone_state_upgrade_resets_incompatible_literals() {
        for literal in [
            json!({"ticket": "damaged"}),
            json!(["damaged"]),
            json!(true),
        ] {
            let name = "ai_systemone_score";
            let old = state_node(name, 1, VariableType::Generic, literal);
            let node_id = old.id.clone();
            let catalog = state_node(name, 2, VariableType::String, json!("new default"));
            let expected = catalog
                .get_pin_by_name("state")
                .unwrap()
                .default_value
                .clone();
            let mut board = Board::new_detached(None, "typed-pin-test".into());
            board.nodes.insert(node_id.clone(), old);

            sync_board_node_schemas(&mut board, &registry(catalog)).await;

            let state = board.nodes[&node_id].get_pin_by_name("state").unwrap();
            assert_eq!(state.data_type, VariableType::String);
            assert_eq!(state.default_value, expected);
        }
    }
}
