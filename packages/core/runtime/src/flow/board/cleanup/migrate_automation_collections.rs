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

fn expected_contract(node: &str, pin: &str) -> Option<(u32, VariableType, PinType)> {
    match (node, pin) {
        ("browser_upload_multiple_files", "file_paths") => {
            Some((1, VariableType::String, PinType::Input))
        }
        ("llm_observe_screen", "elements")
        | ("llm_plan_actions", "actions")
        | ("llm_rank_candidates", "ranked") => Some((4, VariableType::Struct, PinType::Output)),
        ("llm_resolve_element" | "llm_rank_candidates", "candidates") => {
            Some((4, VariableType::Struct, PinType::Input))
        }
        _ => None,
    }
}

fn valid_literal(pin: &Pin, bytes: &[u8]) -> bool {
    let Ok(Value::Array(values)) = flow_like_types::json::from_slice::<Value>(bytes) else {
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
        let Some((version, data_type, direction)) = expected_contract(&node.name, &old.name) else {
            continue;
        };
        if catalog.version != Some(version)
            || node.version.is_some_and(|placed| placed >= version)
            || old.data_type != VariableType::Generic
            || !matches!(old.value_type, ValueType::Normal | ValueType::Array)
            || old.pin_type != direction
        {
            continue;
        }
        let Some(target) = catalog.get_pin_by_name(&old.name) else {
            continue;
        };
        if target.data_type != data_type
            || target.value_type != ValueType::Array
            || target.pin_type != direction
        {
            continue;
        }
        // Require a usable item contract even when this particular pin has no literal.
        if !valid_literal(target, b"[]") {
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

/// Preserve known array values before normal schema synchronization clears changed pin types.
pub(super) fn migrate_automation_collections(board: &mut Board, registry: &FlowNodeRegistryInner) {
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
