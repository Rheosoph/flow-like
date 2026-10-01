extern crate flow_like_runtime as flow_like;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use flow_like::flow::node::Node;
use flow_like::flow::pin::{Pin, PinOptions, PinType, ValueType};
use flow_like::flow::variable::VariableType;
use flow_like_catalog_automation::get_catalog;
use flow_like_types::{Value, json};
use serde::Serialize;
use sha2::{Digest, Sha256};

const UPDATE_ENV: &str = "FLOW_LIKE_UPDATE_GOLDEN";
const GOLDEN_FILE: &str = "tests/fixtures/browser_node_contracts.json";
const EXTRA_NODES: [&str; 4] = [
    "totp_code",
    "automation_start_session",
    "automation_stop_session",
    "fingerprint_match",
];
const UPDATE_HINT: &str = "if the change is intended, bump the node version when pins changed and \
     regenerate with `FLOW_LIKE_UPDATE_GOLDEN=1 cargo test -p flow-like-catalog-automation \
     --features execute --test browser_node_contracts`";

#[derive(Serialize)]
struct ContractFile {
    node_count: usize,
    nodes: Vec<NodeContract>,
}

#[derive(Serialize)]
struct NodeContract {
    name: String,
    friendly_name: String,
    version: Option<u32>,
    category: String,
    flowscript: FlowScriptName,
    pins: Vec<PinContract>,
}

#[derive(Serialize)]
struct FlowScriptName {
    namespace: String,
    name: String,
}

#[derive(Serialize)]
struct PinContract {
    name: String,
    friendly_name: String,
    pin_type: PinType,
    data_type: VariableType,
    value_type: ValueType,
    index: u16,
    default_value: Option<Value>,
    options: Option<PinOptions>,
    schema_sha256: Option<String>,
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_FILE)
}

fn update_requested() -> bool {
    std::env::var_os(UPDATE_ENV).is_some_and(|value| value == "1")
}

fn is_contract_node(name: &str) -> bool {
    name.starts_with("browser_") || EXTRA_NODES.contains(&name)
}

fn pin_order(pin: &Pin) -> (u8, u16, &str) {
    let direction = match pin.pin_type {
        PinType::Input => 0,
        PinType::Output => 1,
    };
    (direction, pin.index, &pin.name)
}

fn schema_sha256(schema: &str) -> String {
    Sha256::digest(schema.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn decode_default(node: &str, pin: &Pin) -> Option<Value> {
    let bytes = pin.default_value.as_deref()?;
    Some(json::from_slice(bytes).unwrap_or_else(|err| {
        panic!(
            "{node}.{}: default_value is not JSON ({err}): {:?}",
            pin.name,
            String::from_utf8_lossy(bytes)
        )
    }))
}

fn pin_contract(node: &str, pin: &Pin) -> PinContract {
    PinContract {
        name: pin.name.clone(),
        friendly_name: pin.friendly_name.clone(),
        pin_type: pin.pin_type.clone(),
        data_type: pin.data_type.clone(),
        value_type: pin.value_type.clone(),
        index: pin.index,
        default_value: decode_default(node, pin),
        options: pin.options.clone(),
        schema_sha256: pin.schema.as_deref().map(schema_sha256),
    }
}

fn node_contract(node: &Node) -> NodeContract {
    let mut pins: Vec<&Pin> = node.pins.values().collect();
    pins.sort_by(|a, b| pin_order(a).cmp(&pin_order(b)));
    NodeContract {
        name: node.name.clone(),
        friendly_name: node.friendly_name.clone(),
        version: node.version,
        category: node.category.clone(),
        flowscript: FlowScriptName {
            namespace: node.flowscript_namespace(),
            name: node.flowscript_alias(),
        },
        pins: pins
            .into_iter()
            .map(|pin| pin_contract(&node.name, pin))
            .collect(),
    }
}

fn current_contract() -> ContractFile {
    let mut nodes: Vec<Node> = get_catalog()
        .iter()
        .map(|logic| logic.get_node())
        .filter(|node| is_contract_node(&node.name))
        .collect();
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    if let Some(pair) = nodes.windows(2).find(|pair| pair[0].name == pair[1].name) {
        panic!(
            "get_catalog() registers node `{}` twice; the contract keys nodes by name",
            pair[0].name
        );
    }
    let nodes: Vec<NodeContract> = nodes.iter().map(node_contract).collect();
    ContractFile {
        node_count: nodes.len(),
        nodes,
    }
}

fn render(contract: &ContractFile) -> String {
    let mut text = json::to_string_pretty(contract)
        .unwrap_or_else(|err| panic!("serializing the browser node contract failed: {err}"));
    text.push('\n');
    text
}

fn parse(text: &str, origin: &str) -> Value {
    json::from_str(text).unwrap_or_else(|err| panic!("{origin} is not valid JSON: {err}"))
}

fn read_golden() -> Value {
    let path = golden_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "reading the golden {} failed: {err}; generate it with {UPDATE_ENV}=1",
            path.display()
        )
    });
    parse(&text, &path.display().to_string())
}

fn nodes_by_name<'a>(contract: &'a Value, origin: &str) -> BTreeMap<&'a str, &'a Value> {
    let nodes = contract["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("{origin}: `nodes` is not an array"));
    let by_name: BTreeMap<&str, &Value> = nodes
        .iter()
        .map(|node| {
            let name = node["name"]
                .as_str()
                .unwrap_or_else(|| panic!("{origin}: node without a string `name`: {node}"));
            (name, node)
        })
        .collect();
    assert_eq!(
        by_name.len(),
        nodes.len(),
        "{origin}: node names are not unique"
    );
    by_name
}

fn assert_node_count(contract: &Value, expected: usize, origin: &str) {
    let recorded = contract["node_count"].as_u64();
    let listed = contract["nodes"].as_array().map(Vec::len);
    assert_eq!(
        recorded,
        Some(expected as u64),
        "{origin}: node_count is {recorded:?}, expected {expected}"
    );
    assert_eq!(
        listed,
        Some(expected),
        "{origin}: lists {listed:?} nodes, expected {expected}"
    );
}

fn pretty(value: &Value) -> String {
    json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn describe_pin_diff(expected: &Value, actual: &Value) -> Option<String> {
    let expected = expected.as_array()?;
    let actual = actual.as_array()?;
    let position = (0..expected.len().max(actual.len()))
        .find(|&position| expected.get(position) != actual.get(position))?;
    let show = |pin: Option<&Value>| pin.map_or_else(|| "<none>".to_owned(), pretty);
    Some(format!(
        "pins[{position}] (sorted by pin_type, index, name; golden has {}, catalog has {}):\n\
         golden: {}\ncatalog: {}",
        expected.len(),
        actual.len(),
        show(expected.get(position)),
        show(actual.get(position))
    ))
}

fn describe_field_diff(expected: &Value, actual: &Value) -> String {
    let fields: BTreeSet<&String> = expected
        .as_object()
        .into_iter()
        .chain(actual.as_object())
        .flat_map(|object| object.keys())
        .collect();
    let Some(field) = fields
        .into_iter()
        .find(|field| expected.get(field.as_str()) != actual.get(field.as_str()))
    else {
        return format!("golden: {}\ncatalog: {}", pretty(expected), pretty(actual));
    };
    let golden = expected.get(field.as_str()).unwrap_or(&Value::Null);
    let catalog = actual.get(field.as_str()).unwrap_or(&Value::Null);
    if field == "pins"
        && let Some(pin_diff) = describe_pin_diff(golden, catalog)
    {
        return pin_diff;
    }
    format!("`{field}` changed:\ngolden: {golden}\ncatalog: {catalog}")
}

fn describe_node_diff(name: &str, expected: Option<&Value>, actual: Option<&Value>) -> String {
    match (expected, actual) {
        (Some(_), None) => format!("node `{name}` is in the golden but no longer in get_catalog()"),
        (None, Some(node)) => format!(
            "node `{name}` is new in get_catalog() and missing from the golden:\n{}",
            pretty(node)
        ),
        (Some(expected), Some(actual)) => format!(
            "node `{name}` differs from the golden; {}",
            describe_field_diff(expected, actual)
        ),
        (None, None) => format!("node `{name}` is in neither contract"),
    }
}

fn assert_matches_golden(golden: &Value, current: &Value) {
    let golden_nodes = nodes_by_name(golden, "golden");
    let current_nodes = nodes_by_name(current, "get_catalog()");
    let names: BTreeSet<&str> = golden_nodes
        .keys()
        .chain(current_nodes.keys())
        .copied()
        .collect();
    if let Some(name) = names
        .into_iter()
        .find(|name| golden_nodes.get(name) != current_nodes.get(name))
    {
        panic!(
            "{}\n{UPDATE_HINT}",
            describe_node_diff(
                name,
                golden_nodes.get(name).copied(),
                current_nodes.get(name).copied()
            )
        );
    }
}

fn assert_extra_nodes_present(contract: &ContractFile) {
    for extra in EXTRA_NODES {
        assert!(
            contract.nodes.iter().any(|node| node.name == extra),
            "get_catalog() no longer registers `{extra}`, which the browser node contract covers"
        );
    }
}

fn write_golden(text: &str) {
    let path = golden_path();
    std::fs::write(&path, text)
        .unwrap_or_else(|err| panic!("writing the golden {} failed: {err}", path.display()));
}

#[test]
fn browser_node_contracts_match_golden() {
    let current = current_contract();
    assert_extra_nodes_present(&current);
    let rendered = render(&current);
    let expected_count = current.node_count;

    if update_requested() {
        write_golden(&rendered);
        assert_node_count(&read_golden(), expected_count, "written golden");
        println!(
            "wrote {expected_count} browser node contracts to {}",
            golden_path().display()
        );
        return;
    }

    let golden = read_golden();
    let current = parse(&rendered, "rendered catalog contract");
    assert_matches_golden(&golden, &current);
    assert_node_count(&golden, expected_count, "golden");
}
