//! The node catalog as clients download it.
//!
//! Serialized node by node, a catalog repeats itself: one JSON schema sits on hundreds of pins,
//! most fields hold the same value on every pin, and the ids of built-in nodes are random per
//! process although whoever places a node replaces them. A section sends each schema once,
//! states the usual value of every field once, and leaves those ids out, so two processes
//! serving the same catalog produce the same bytes and one ETag covers all of them.
//!
//! Nothing a reader needs is lost: expanding a section gives back every node field for field,
//! except the ids that were left out on purpose.
//!
//! The built-in nodes and what an app adds to them travel as separate documents. The built-in
//! one is the same for every app, so an app's document only names it by digest and a reader
//! fetches it once for all apps.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use serde::ser::Error as _;
use serde_json::{Map, Value};

/// Layout version of [`CatalogSection`]. A reader that meets a newer one asks for the plain list.
pub const CATALOG_WIRE_VERSION: u32 = 1;

/// A default is only worth stating for values this short; longer text stays on its node.
const MAX_DEFAULT_TEXT: usize = 64;

type Object = Map<String, Value>;

/// Whether the ids of a section's nodes mean anything to its readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeIds {
    /// Random per process (built-in nodes). Left out, and the nodes are ordered by name.
    Unstable,
    /// Chosen by whoever defined the node (presets, package nodes). Sent as they are and in the
    /// order given: readers tell same-named nodes apart by id and keep the first of a name.
    Stable,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogSection {
    /// Every distinct pin schema once. A pin's `schema` is its position in this list.
    pub schemas: Vec<String>,
    /// The value most nodes hold per field. A node states only the fields that differ.
    pub node_defaults: Object,
    pub pin_defaults: Object,
    pub option_defaults: Object,
    /// Nodes with `pins` as a list ordered by index, and without pin ids.
    pub nodes: Vec<Value>,
}

/// One catalog document around an encoded section.
///
/// `built_in` is the digest of the built-in section: the one this document carries, or the one
/// its app nodes extend. `etag` is the validator the document is served under; it is repeated
/// here because a page on another origin cannot read a response header its hub does not expose.
pub fn catalog_document(built_in: &str, etag: Option<&str>, section: &str) -> String {
    let etag = etag
        .map(|etag| format!(r#""etag":{},"#, Value::from(etag)))
        .unwrap_or_default();
    format!(
        r#"{{"v":{CATALOG_WIRE_VERSION},"built_in":"{built_in}",{etag}"sections":[{section}]}}"#
    )
}

pub fn encode_section<T: Serialize>(
    nodes: &[T],
    ids: NodeIds,
) -> serde_json::Result<CatalogSection> {
    let mut entries = nodes
        .iter()
        .map(|node| split_node(node, ids))
        .collect::<serde_json::Result<Vec<_>>>()?;
    if ids == NodeIds::Unstable {
        entries.sort_by_cached_key(node_order);
    }

    let mut schemas = SchemaTable::default();
    for (_, pins) in &mut entries {
        for pin in pins {
            schemas.reference(pin);
        }
    }

    let option_defaults = strip_common(
        entries
            .iter_mut()
            .flat_map(|(_, pins)| pins.iter_mut())
            .filter_map(|pin| pin.get_mut("options").and_then(Value::as_object_mut))
            .collect(),
    )?;
    let pin_defaults = strip_common(
        entries
            .iter_mut()
            .flat_map(|(_, pins)| pins.iter_mut())
            .collect(),
    )?;
    let node_defaults = strip_common(entries.iter_mut().map(|(node, _)| node).collect())?;

    let nodes = entries
        .into_iter()
        .map(|(mut node, pins)| {
            let pins = pins.into_iter().map(Value::Object).collect();
            node.insert("pins".to_string(), Value::Array(pins));
            Value::Object(node)
        })
        .collect();

    Ok(CatalogSection {
        schemas: schemas.list,
        node_defaults,
        pin_defaults,
        option_defaults,
        nodes,
    })
}

/// A node without its pins, and its pins as a list in display order.
fn split_node<T: Serialize>(node: &T, ids: NodeIds) -> serde_json::Result<(Object, Vec<Object>)> {
    let Value::Object(mut node) = sorted_keys(serde_json::to_value(node)?) else {
        return Err(serde_json::Error::custom(
            "a catalog node must serialize to a JSON object",
        ));
    };
    if ids == NodeIds::Unstable {
        node.remove("id");
    }

    let mut pins = Vec::new();
    match node.remove("pins") {
        None | Some(Value::Null) => {}
        Some(Value::Object(by_id)) => {
            for (_, pin) in by_id {
                let Value::Object(mut pin) = pin else {
                    return Err(serde_json::Error::custom(
                        "a catalog pin must serialize to a JSON object",
                    ));
                };
                pin.remove("id");
                pins.push(pin);
            }
        }
        Some(_) => {
            return Err(serde_json::Error::custom(
                "a catalog node's pins must serialize to a JSON object keyed by pin id",
            ));
        }
    }
    pins.sort_by_cached_key(pin_order);
    Ok((node, pins))
}

/// Object keys in one order everywhere, so a map field serializes the same in every process.
fn sorted_keys(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, sorted_keys(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(sorted_keys).collect()),
        other => other,
    }
}

fn text(object: &Object, key: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn json(object: &Object) -> String {
    serde_json::to_string(object).unwrap_or_default()
}

fn node_order(entry: &(Object, Vec<Object>)) -> (String, String, String) {
    let (node, pins) = entry;
    let pins = pins.iter().map(json).collect::<Vec<_>>().join(",");
    (text(node, "name"), json(node), pins)
}

fn pin_order(pin: &Object) -> (u64, String, String, String) {
    let index = pin.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX);
    (index, text(pin, "pin_type"), text(pin, "name"), json(pin))
}

#[derive(Default)]
struct SchemaTable {
    list: Vec<String>,
    positions: HashMap<String, usize>,
}

impl SchemaTable {
    fn reference(&mut self, pin: &mut Object) {
        let Some(Value::String(schema)) = pin.get("schema") else {
            return;
        };
        let position = match self.positions.get(schema) {
            Some(position) => *position,
            None => {
                let position = self.list.len();
                self.list.push(schema.clone());
                self.positions.insert(schema.clone(), position);
                position
            }
        };
        pin.insert("schema".to_string(), Value::from(position));
    }
}

/// Only values a reader can copy without sharing state between nodes.
fn can_be_default(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => true,
        Value::String(text) => text.len() <= MAX_DEFAULT_TEXT,
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
    }
}

#[derive(Default)]
struct FieldTally {
    present: usize,
    uses_by_json: BTreeMap<String, usize>,
}

impl FieldTally {
    /// The JSON text of the value most objects hold, when more than one object holds it.
    fn most_used(self) -> Option<String> {
        // `max_by_key` keeps the last maximum; reversed, a tie goes to the smallest JSON text.
        let (json, uses) = self
            .uses_by_json
            .into_iter()
            .rev()
            .max_by_key(|(_, uses)| *uses)?;
        (uses > 1).then_some(json)
    }
}

fn tally_fields<'a>(objects: &'a [&mut Object]) -> BTreeMap<&'a str, FieldTally> {
    let mut tallies: BTreeMap<&str, FieldTally> = BTreeMap::new();
    for object in objects {
        for (key, value) in object.iter() {
            let tally = tallies.entry(key.as_str()).or_default();
            tally.present += 1;
            if can_be_default(value) {
                *tally.uses_by_json.entry(value.to_string()).or_default() += 1;
            }
        }
    }
    tallies
}

/// The most used value of every field that all `objects` carry. A field some object lacks gets
/// no default: a reader could not tell "absent" from "equal to the default".
fn common_values(objects: &[&mut Object]) -> serde_json::Result<Object> {
    let mut defaults = Object::new();
    for (key, tally) in tally_fields(objects) {
        if tally.present != objects.len() {
            continue;
        }
        if let Some(json) = tally.most_used() {
            defaults.insert(key.to_string(), serde_json::from_str(&json)?);
        }
    }
    Ok(defaults)
}

/// Remove from every object the fields that equal the common value, and return those values.
fn strip_common(mut objects: Vec<&mut Object>) -> serde_json::Result<Object> {
    let defaults = common_values(&objects)?;
    for object in &mut objects {
        object.retain(|key, value| defaults.get(key) != Some(&*value));
    }
    Ok(defaults)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_runtime::flow::{node::Node, variable::VariableType};
    use serde_json::json;

    /// What a reader does with a section; mirrors `decodeCatalog` in `packages/ui`.
    fn expand(section: &CatalogSection) -> Vec<Value> {
        let with_defaults = |defaults: &Object, stated: &Object| {
            let mut full = defaults.clone();
            full.extend(stated.clone());
            full
        };
        section
            .nodes
            .iter()
            .map(|node| {
                let node = node.as_object().expect("node object");
                let mut full = with_defaults(&section.node_defaults, node);
                let pins = node["pins"]
                    .as_array()
                    .expect("pin list")
                    .iter()
                    .map(|pin| {
                        let mut pin =
                            with_defaults(&section.pin_defaults, pin.as_object().expect("pin"));
                        if let Some(position) = pin.get("schema").and_then(Value::as_u64) {
                            let schema = section.schemas[position as usize].clone();
                            pin.insert("schema".to_string(), Value::String(schema));
                        }
                        if let Some(Value::Object(options)) = pin.get("options") {
                            let options = with_defaults(&section.option_defaults, options);
                            pin.insert("options".to_string(), Value::Object(options));
                        }
                        Value::Object(pin)
                    })
                    .collect();
                full.insert("pins".to_string(), Value::Array(pins));
                sorted_keys(Value::Object(full))
            })
            .collect()
    }

    /// The node as a reader should get it back: no pin ids, pins as a list in display order.
    fn expected<T: Serialize>(node: &T, ids: NodeIds) -> Value {
        let (mut node, pins) = split_node(node, ids).expect("node splits");
        let pins = pins.into_iter().map(Value::Object).collect();
        node.insert("pins".to_string(), Value::Array(pins));
        sorted_keys(Value::Object(node))
    }

    fn sample_node(name: &str, schema: &str) -> Node {
        let mut node = Node::new(name, "Sample", "A node used by the wire tests", "Tests");
        node.add_input_pin("exec_in", "In", "Starts the node", VariableType::Execution);
        node.add_input_pin("payload", "Payload", "What to send", VariableType::Struct)
            .schema = Some(schema.to_string());
        node.add_output_pin("exec_out", "Out", "Continues", VariableType::Execution);
        node.add_output_pin("result", "Result", "What came back", VariableType::Struct)
            .schema = Some(schema.to_string());
        node
    }

    #[test]
    fn a_section_expands_to_the_nodes_it_was_built_from() {
        let nodes = vec![
            sample_node("alpha", r#"{"type":"object"}"#),
            sample_node("beta", r#"{"type":"string"}"#),
            sample_node("gamma", r#"{"type":"object"}"#),
        ];
        for ids in [NodeIds::Unstable, NodeIds::Stable] {
            let section = encode_section(&nodes, ids).expect("section");
            let want = nodes
                .iter()
                .map(|node| expected(node, ids))
                .collect::<Vec<_>>();
            assert_eq!(expand(&section), want, "{ids:?}");
        }
    }

    #[test]
    fn a_schema_shared_by_many_pins_is_sent_once() {
        let nodes = vec![
            sample_node("alpha", r#"{"type":"object"}"#),
            sample_node("beta", r#"{"type":"object"}"#),
        ];
        let section = encode_section(&nodes, NodeIds::Unstable).expect("section");
        assert_eq!(section.schemas, vec![r#"{"type":"object"}"#.to_string()]);
        let body = serde_json::to_string(&section).expect("json");
        assert_eq!(body.matches(r#"{\"type\":\"object\"}"#).count(), 1);
    }

    #[test]
    fn built_in_nodes_encode_the_same_in_every_process() {
        // Each call builds the nodes anew, with fresh random ids, as a second process would.
        let build = |names: [&str; 3]| {
            let nodes = names.map(|name| sample_node(name, r#"{"type":"object"}"#));
            let section = encode_section(&nodes, NodeIds::Unstable).expect("section");
            serde_json::to_string(&section).expect("json")
        };
        let first = build(["alpha", "beta", "gamma"]);
        assert_eq!(first, build(["gamma", "alpha", "beta"]));
        assert!(!first.contains(r#""id""#), "ids must not reach the wire");
    }

    #[test]
    fn preset_nodes_keep_their_ids_and_their_order() {
        let mut second = sample_node("preset", r#"{"type":"object"}"#);
        second.id = "preset_b".to_string();
        let mut first = sample_node("preset", r#"{"type":"object"}"#);
        first.id = "preset_a".to_string();
        let section = encode_section(&[second, first], NodeIds::Stable).expect("section");
        let ids = section
            .nodes
            .iter()
            .map(|node| node["id"].as_str().expect("id"))
            .collect::<Vec<_>>();
        assert_eq!(ids, ["preset_b", "preset_a"]);
        assert!(section.nodes.iter().all(|node| {
            let pins = node["pins"].as_array().expect("pins");
            pins.iter().all(|pin| pin.get("id").is_none())
        }));
    }

    #[test]
    fn a_field_some_nodes_lack_never_becomes_a_default() {
        let nodes = vec![
            json!({"name": "a", "category": "Tests", "pins": {}, "docs": null}),
            json!({"name": "b", "category": "Tests", "pins": {}, "docs": null}),
            json!({"name": "c", "category": "Tests", "pins": {}}),
        ];
        let section = encode_section(&nodes, NodeIds::Stable).expect("section");
        assert_eq!(section.node_defaults.get("category"), Some(&json!("Tests")));
        assert!(!section.node_defaults.contains_key("docs"));
        let want = nodes
            .iter()
            .map(|node| expected(node, NodeIds::Stable))
            .collect::<Vec<_>>();
        assert_eq!(expand(&section), want);
    }

    #[test]
    fn a_document_names_its_built_in_catalog_and_quotes_the_validator() {
        let section = encode_section(&[sample_node("alpha", "{}")], NodeIds::Stable)
            .and_then(|section| serde_json::to_string(&section))
            .expect("section");

        let tagged = catalog_document("abc123", Some("\"tag\""), &section);
        let tagged: Value = serde_json::from_str(&tagged).expect("document");
        assert_eq!(tagged["v"], CATALOG_WIRE_VERSION);
        assert_eq!(tagged["built_in"], "abc123");
        assert_eq!(tagged["etag"], "\"tag\"");
        assert_eq!(tagged["sections"].as_array().map(Vec::len), Some(1));

        let untagged = catalog_document("abc123", None, &section);
        let untagged: Value = serde_json::from_str(&untagged).expect("document");
        assert!(untagged.get("etag").is_none());
    }

    #[test]
    fn a_node_that_is_not_an_object_is_refused() {
        let error = encode_section(&[json!("node")], NodeIds::Stable).expect_err("refused");
        assert!(error.to_string().contains("JSON object"), "{error}");
    }
}
