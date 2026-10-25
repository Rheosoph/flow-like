//! What an apply changed, in terms that outlive it: node ids, node types, pin names and layers.
//!
//! The report diffs the board before and after the apply instead of reading the planned commands,
//! because a connection into an occupied data input, or out of an execution output, silently
//! replaces the edge that was there. No command names that removal; the diff does.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use flow_like_types::Value;
use serde::{Deserialize, Serialize};

use crate::flow::{
    board::{Board, Layer},
    copilot::BoardCommand,
    node::Node,
    pin::{Pin, PinType},
    variable::{Variable, VariableType},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowScriptConnectionKind {
    Execution,
    Data,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FlowScriptChangedNode {
    pub id: String,
    pub node_type: String,
    pub friendly_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
}

/// One end of a connection. A layer boundary pin names its layer: `node_id` is the layer id and
/// `node_type` is `layer:<type>` (`layer:function`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FlowScriptEndpoint {
    pub node_id: String,
    pub node_type: String,
    pub pin: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FlowScriptConnection {
    pub kind: FlowScriptConnectionKind,
    pub from: FlowScriptEndpoint,
    pub to: FlowScriptEndpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FlowScriptMovedNode {
    pub id: String,
    pub node_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_layer: Option<String>,
}

/// A literal input value that changed on a node that existed before and after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowScriptValueChange {
    pub node_id: String,
    pub node_type: String,
    pub pin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

/// A stored FlowScript name (`Node.code_names`) that changed. `key` is `""` for the node's own
/// binding, else an output pin name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FlowScriptNameChange {
    pub node_id: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// The resolved description of a node that existed before and after an apply.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FlowScriptDescriptionChange {
    pub node_id: String,
    pub node_type: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FlowScriptChangedLayer {
    pub id: String,
    pub name: String,
    pub layer_type: String,
}

/// Everything an apply changed on the board. Each list is sorted, so the same apply always
/// reports the same way.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowScriptChanges {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes_added: Vec<FlowScriptChangedNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes_removed: Vec<FlowScriptChangedNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes_moved: Vec<FlowScriptMovedNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections_added: Vec<FlowScriptConnection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections_removed: Vec<FlowScriptConnection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values_changed: Vec<FlowScriptValueChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names_changed: Vec<FlowScriptNameChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub descriptions_changed: Vec<FlowScriptDescriptionChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables_added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables_removed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables_changed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers_added: Vec<FlowScriptChangedLayer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers_removed: Vec<FlowScriptChangedLayer>,
    /// Layers whose name or boundary pins changed (a function signature, for example).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers_changed: Vec<FlowScriptChangedLayer>,
}

impl FlowScriptChanges {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Compare the board before an apply with the board after it.
    pub fn between(before: &Board, after: &Board) -> Self {
        let before = BoardIndex::new(before);
        let after = BoardIndex::new(after);
        let mut changes = Self::default();
        diff_nodes(&before, &after, &mut changes);
        diff_connections(&before, &after, &mut changes);
        diff_variables(&before, &after, &mut changes);
        diff_layers(&before, &after, &mut changes);
        changes
    }
}

struct BoardIndex<'a> {
    refs: &'a HashMap<String, String>,
    nodes: BTreeMap<&'a str, &'a Node>,
    layers: BTreeMap<&'a str, &'a Layer>,
    variables: BTreeMap<&'a str, &'a Variable>,
    pins: HashMap<&'a str, (&'a Pin, FlowScriptEndpoint)>,
    edges: BTreeSet<(&'a str, &'a str)>,
}

impl<'a> BoardIndex<'a> {
    fn new(board: &'a Board) -> Self {
        let mut nodes = BTreeMap::new();
        let mut pins = HashMap::new();
        for node in board
            .nodes
            .values()
            .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        {
            nodes.insert(node.id.as_str(), node);
            for pin in node.pins.values() {
                let endpoint = FlowScriptEndpoint {
                    node_id: node.id.clone(),
                    node_type: node.name.clone(),
                    pin: pin.name.clone(),
                };
                pins.insert(pin.id.as_str(), (pin, endpoint));
            }
        }
        let mut layers = BTreeMap::new();
        for layer in board.layers.values() {
            layers.insert(layer.id.as_str(), layer);
            for pin in layer.pins.values() {
                let endpoint = FlowScriptEndpoint {
                    node_id: layer.id.clone(),
                    node_type: format!("layer:{}", layer_type(layer)),
                    pin: pin.name.clone(),
                };
                pins.insert(pin.id.as_str(), (pin, endpoint));
            }
        }
        let mut edges = BTreeSet::new();
        for (pin, _) in pins.values() {
            for target in &pin.connected_to {
                edges.insert((pin.id.as_str(), target.as_str()));
            }
            for source in &pin.depends_on {
                edges.insert((source.as_str(), pin.id.as_str()));
            }
        }
        let variables = board
            .variables
            .values()
            .chain(
                board
                    .layers
                    .values()
                    .flat_map(|layer| layer.variables.values()),
            )
            .map(|variable| (variable.id.as_str(), variable))
            .collect();
        Self {
            refs: &board.refs,
            nodes,
            layers,
            variables,
            pins,
            edges,
        }
    }

    fn connection(&self, (from, to): (&str, &str)) -> Option<FlowScriptConnection> {
        let (from_pin, from) = self.pins.get(from)?;
        let (to_pin, to) = self.pins.get(to)?;
        let kind = if from_pin.data_type == VariableType::Execution
            || to_pin.data_type == VariableType::Execution
        {
            FlowScriptConnectionKind::Execution
        } else {
            FlowScriptConnectionKind::Data
        };
        Some(FlowScriptConnection {
            kind,
            from: from.clone(),
            to: to.clone(),
        })
    }
}

fn layer_type(layer: &Layer) -> String {
    format!("{:?}", layer.r#type).to_lowercase()
}

fn changed_node(node: &Node) -> FlowScriptChangedNode {
    FlowScriptChangedNode {
        id: node.id.clone(),
        node_type: node.name.clone(),
        friendly_name: node.friendly_name.clone(),
        layer: node.layer.clone(),
    }
}

fn changed_layer(layer: &Layer) -> FlowScriptChangedLayer {
    FlowScriptChangedLayer {
        id: layer.id.clone(),
        name: layer.name.clone(),
        layer_type: layer_type(layer),
    }
}

fn pin_value(pin: &Pin) -> Option<Value> {
    flow_like_types::json::from_slice(pin.default_value.as_deref()?).ok()
}

fn diff_nodes(before: &BoardIndex, after: &BoardIndex, changes: &mut FlowScriptChanges) {
    for (id, node) in &after.nodes {
        let Some(old) = before.nodes.get(id) else {
            changes.nodes_added.push(changed_node(node));
            continue;
        };
        if old.layer != node.layer {
            changes.nodes_moved.push(FlowScriptMovedNode {
                id: node.id.clone(),
                node_type: node.name.clone(),
                from_layer: old.layer.clone(),
                to_layer: node.layer.clone(),
            });
        }
        let old_description = crate::flow::pin::resolve_schema(&old.description, before.refs)
            .unwrap_or(&old.description);
        let new_description = crate::flow::pin::resolve_schema(&node.description, after.refs)
            .unwrap_or(&node.description);
        if old_description != new_description {
            changes
                .descriptions_changed
                .push(FlowScriptDescriptionChange {
                    node_id: node.id.clone(),
                    node_type: node.name.clone(),
                    before: old_description.to_string(),
                    after: new_description.to_string(),
                });
        }
        let mut inputs: Vec<&Pin> = node
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Input && pin.data_type != VariableType::Execution
            })
            .collect();
        inputs.sort_by(|a, b| a.index.cmp(&b.index).then(a.name.cmp(&b.name)));
        for pin in inputs {
            let previous = old.pins.get(&pin.id).and_then(pin_value);
            let current = pin_value(pin);
            if previous != current {
                changes.values_changed.push(FlowScriptValueChange {
                    node_id: node.id.clone(),
                    node_type: node.name.clone(),
                    pin: pin.name.clone(),
                    before: previous,
                    after: current,
                });
            }
        }
        let names = |node: &Node| node.code_names.clone().unwrap_or_default();
        let (old_names, new_names) = (names(old), names(node));
        let keys: BTreeSet<&String> = old_names.keys().chain(new_names.keys()).collect();
        for key in keys {
            let (previous, current) = (old_names.get(key), new_names.get(key));
            if previous != current {
                changes.names_changed.push(FlowScriptNameChange {
                    node_id: node.id.clone(),
                    key: key.clone(),
                    before: previous.cloned(),
                    after: current.cloned(),
                });
            }
        }
    }
    for (id, node) in &before.nodes {
        if !after.nodes.contains_key(id) {
            changes.nodes_removed.push(changed_node(node));
        }
    }
    changes.nodes_added.sort();
    changes.nodes_removed.sort();
    changes.nodes_moved.sort();
    changes.names_changed.sort();
    changes.descriptions_changed.sort();
}

fn diff_connections(before: &BoardIndex, after: &BoardIndex, changes: &mut FlowScriptChanges) {
    changes.connections_added = after
        .edges
        .difference(&before.edges)
        .filter_map(|edge| after.connection(*edge))
        .collect();
    changes.connections_removed = before
        .edges
        .difference(&after.edges)
        .filter_map(|edge| before.connection(*edge))
        .collect();
    changes.connections_added.sort();
    changes.connections_added.dedup();
    changes.connections_removed.sort();
    changes.connections_removed.dedup();
}

fn variable_shape(
    variable: &Variable,
) -> (String, String, String, Option<Vec<u8>>, Option<String>) {
    (
        variable.name.clone(),
        format!("{:?}", variable.data_type),
        format!("{:?}", variable.value_type),
        variable.default_value.clone(),
        variable.schema.clone(),
    )
}

fn diff_variables(before: &BoardIndex, after: &BoardIndex, changes: &mut FlowScriptChanges) {
    for (id, variable) in &after.variables {
        match before.variables.get(id) {
            None => changes.variables_added.push(variable.name.clone()),
            Some(old) if variable_shape(old) != variable_shape(variable) => {
                changes.variables_changed.push(variable.name.clone());
            }
            Some(_) => {}
        }
    }
    for (id, variable) in &before.variables {
        if !after.variables.contains_key(id) {
            changes.variables_removed.push(variable.name.clone());
        }
    }
    changes.variables_added.sort();
    changes.variables_removed.sort();
    changes.variables_changed.sort();
}

fn boundary(layer: &Layer) -> BTreeSet<(String, String, String, String)> {
    layer
        .pins
        .values()
        .map(|pin| {
            (
                format!("{:?}", pin.pin_type),
                pin.name.clone(),
                format!("{:?}", pin.data_type),
                format!("{:?}", pin.value_type),
            )
        })
        .collect()
}

fn diff_layers(before: &BoardIndex, after: &BoardIndex, changes: &mut FlowScriptChanges) {
    for (id, layer) in &after.layers {
        match before.layers.get(id) {
            None => changes.layers_added.push(changed_layer(layer)),
            Some(old) if old.name != layer.name || boundary(old) != boundary(layer) => {
                changes.layers_changed.push(changed_layer(layer));
            }
            Some(_) => {}
        }
    }
    for (id, layer) in &before.layers {
        if !after.layers.contains_key(id) {
            changes.layers_removed.push(changed_layer(layer));
        }
    }
    changes.layers_added.sort();
    changes.layers_removed.sort();
    changes.layers_changed.sort();
}

/// What a reconciled batch is about to do, for a reader who has not applied it yet: the
/// connections it makes and cuts, the nodes it adds and removes, the values, names and function
/// signatures it changes. A new node is its `$N` reference. A connection into an occupied input
/// replaces the old edge without a cut being listed; the apply result's [`FlowScriptChanges`]
/// reports that exactly.
pub fn planned_changes(commands: &[BoardCommand], limit: usize) -> Value {
    let mut groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for command in commands {
        let (group, entry) = match command {
            BoardCommand::ConnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => (
                "connect",
                format!("{from_node}.{from_pin} -> {to_node}.{to_pin}"),
            ),
            BoardCommand::DisconnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => (
                "disconnect",
                format!("{from_node}.{from_pin} -> {to_node}.{to_pin}"),
            ),
            BoardCommand::AddNode {
                node_type, ref_id, ..
            } => (
                "add_node",
                format!("{} {node_type}", ref_id.as_deref().unwrap_or("?")),
            ),
            BoardCommand::RemoveNode { node_id, .. } => ("remove_node", node_id.clone()),
            BoardCommand::UpdateNodePin {
                node_id, pin_id, ..
            } => ("set_value", format!("{node_id}.{pin_id}")),
            BoardCommand::SetNodeDescription { node_id, .. } => ("describe_node", node_id.clone()),
            BoardCommand::SetNodeCodeNames {
                node_id,
                code_names,
                ..
            } => (
                "name_values",
                code_names
                    .iter()
                    .map(|(key, name)| match key.as_str() {
                        "" => format!("{node_id} = {name}"),
                        pin => format!("{node_id}.{pin} = {name}"),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            BoardCommand::UpdateFunctionSignature { layer_id, pins, .. } => (
                "change_signature",
                format!(
                    "{layer_id}({})",
                    pins.iter()
                        .map(|pin| match pin.pin_type.as_str() {
                            "Output" => format!("-> {}", pin.name),
                            _ => pin.name.clone(),
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
            BoardCommand::MoveToLayer {
                ids, target_layer, ..
            } => (
                "move",
                format!(
                    "{} -> {}",
                    ids.join(", "),
                    target_layer.as_deref().unwrap_or("root")
                ),
            ),
            BoardCommand::CreateLayer { name, ref_id, .. } => (
                "add_layer",
                format!("{} {name}", ref_id.as_deref().unwrap_or("?")),
            ),
            BoardCommand::RenameLayer { layer_id, name, .. } => {
                ("rename_layer", format!("{layer_id} -> {name}"))
            }
            BoardCommand::CreateVariable { name, .. } => ("add_variable", name.clone()),
            BoardCommand::RemoveVariable { variable_id, .. } => {
                ("remove_variable", variable_id.clone())
            }
            BoardCommand::UpdateVariable { variable_id, .. } => {
                ("change_variable", variable_id.clone())
            }
            _ => continue,
        };
        groups.entry(group).or_default().push(entry);
    }
    Value::Object(
        groups
            .into_iter()
            .map(|(group, entries)| {
                let total = entries.len();
                let items: Vec<Value> = entries.into_iter().take(limit).map(Value::from).collect();
                (
                    group.to_string(),
                    flow_like_types::json::json!({ "total": total, "items": items }),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::pin::PinType;

    fn node_with(name: &str, pins: &[(&str, PinType)]) -> Node {
        let mut node = Node::new(name, name, "", "test");
        for (pin, pin_type) in pins {
            match pin_type {
                PinType::Input => node.add_input_pin(pin, pin, "", VariableType::String),
                PinType::Output => node.add_output_pin(pin, pin, "", VariableType::String),
            };
        }
        node
    }

    fn pin_id(node: &Node, name: &str) -> String {
        node.pins
            .values()
            .find(|pin| pin.name == name)
            .map(|pin| pin.id.clone())
            .expect("pin")
    }

    #[test]
    fn event_description_changes_compare_resolved_text_and_report_clearing() {
        let mut before = Board::new_detached(None, flow_like_storage::Path::default());
        let mut node = node_with("events_generic", &[]);
        node.id = "tool".into();
        node.description = "description-ref".into();
        before.nodes.insert(node.id.clone(), node);
        before
            .refs
            .insert("description-ref".into(), "nested-ref".into());
        before
            .refs
            .insert("nested-ref".into(), "Find matching tickets".into());

        let mut after = before.clone();
        after.nodes.get_mut("tool").unwrap().description = "Find matching tickets".into();
        assert!(FlowScriptChanges::between(&before, &after).is_empty());

        after.nodes.get_mut("tool").unwrap().description = "Find open tickets".into();
        let changes = FlowScriptChanges::between(&before, &after);
        assert_eq!(
            changes.descriptions_changed,
            vec![FlowScriptDescriptionChange {
                node_id: "tool".into(),
                node_type: "events_generic".into(),
                before: "Find matching tickets".into(),
                after: "Find open tickets".into(),
            }]
        );
        assert!(!changes.is_empty());

        let mut cleared = after.clone();
        cleared.nodes.get_mut("tool").unwrap().description.clear();
        let changes = FlowScriptChanges::between(&after, &cleared);
        assert_eq!(changes.descriptions_changed.len(), 1);
        assert_eq!(changes.descriptions_changed[0].before, "Find open tickets");
        assert_eq!(changes.descriptions_changed[0].after, "");
    }

    fn connect(board: &mut Board, from: (&str, &str), to: (&str, &str)) {
        let from_pin = pin_id(&board.nodes[from.0], from.1);
        let to_pin = pin_id(&board.nodes[to.0], to.1);
        let from_node = board.nodes.get_mut(from.0).expect("from");
        from_node
            .pins
            .get_mut(&from_pin)
            .expect("pin")
            .connected_to
            .insert(to_pin.clone());
        let to_node = board.nodes.get_mut(to.0).expect("to");
        let target = to_node.pins.get_mut(&to_pin).expect("pin");
        target.depends_on.clear();
        target.depends_on.insert(from_pin);
    }

    /// A connection into an occupied input replaces the old edge without any command naming the
    /// removal; the report shows both.
    #[test]
    fn a_replaced_edge_is_reported_as_removed_and_added() {
        let mut before = Board::new_detached(None, flow_like_storage::Path::default());
        for node in [
            node_with("first", &[("text", PinType::Output)]),
            node_with("second", &[("text", PinType::Output)]),
            node_with("sink", &[("message", PinType::Input)]),
        ] {
            before.nodes.insert(
                node.name.clone(),
                Node {
                    id: node.name.clone(),
                    ..node
                },
            );
        }
        connect(&mut before, ("first", "text"), ("sink", "message"));

        let mut after = before.clone();
        let old = pin_id(&after.nodes["first"], "text");
        after
            .nodes
            .get_mut("first")
            .unwrap()
            .pins
            .get_mut(&old)
            .unwrap()
            .connected_to
            .clear();
        connect(&mut after, ("second", "text"), ("sink", "message"));
        after.nodes.get_mut("sink").unwrap().code_names =
            Some(BTreeMap::from([(String::new(), "result".to_string())]));

        let changes = FlowScriptChanges::between(&before, &after);
        let edge = |from: &str| FlowScriptConnection {
            kind: FlowScriptConnectionKind::Data,
            from: FlowScriptEndpoint {
                node_id: from.to_string(),
                node_type: from.to_string(),
                pin: "text".to_string(),
            },
            to: FlowScriptEndpoint {
                node_id: "sink".to_string(),
                node_type: "sink".to_string(),
                pin: "message".to_string(),
            },
        };
        assert_eq!(changes.connections_removed, vec![edge("first")]);
        assert_eq!(changes.connections_added, vec![edge("second")]);
        assert_eq!(changes.names_changed.len(), 1);
        assert!(changes.nodes_added.is_empty() && changes.nodes_removed.is_empty());
        assert!(FlowScriptChanges::between(&after, &after).is_empty());
    }

    #[test]
    fn planned_changes_group_and_cap_a_batch() {
        let commands = vec![
            BoardCommand::ConnectPins {
                from_node: "$1".into(),
                from_pin: "exec_out".into(),
                to_node: "abc".into(),
                to_pin: "exec_in".into(),
                summary: None,
            },
            BoardCommand::ConnectPins {
                from_node: "$1".into(),
                from_pin: "text".into(),
                to_node: "abc".into(),
                to_pin: "message".into(),
                summary: None,
            },
            BoardCommand::SetNodeCodeNames {
                node_id: "abc".into(),
                code_names: BTreeMap::from([(String::new(), "selectedPerson".to_string())]),
                summary: None,
            },
        ];
        let planned = planned_changes(&commands, 1);
        assert_eq!(planned["connect"]["total"], 2);
        assert_eq!(planned["connect"]["items"].as_array().unwrap().len(), 1);
        assert_eq!(planned["name_values"]["items"][0], "abc = selectedPerson");
    }
}
