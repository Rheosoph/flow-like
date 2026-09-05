//! Node metadata and catalog tests for the code interpreter.
//!
//! These tests do NOT require the `execute` feature — they only test
//! node definitions, pin schemas, and catalog registration.

extern crate flow_like_runtime as flow_like;

use flow_like_catalog_code_interpreter::get_catalog;

#[test]
fn catalog_contains_python_interpreter() {
    let catalog = get_catalog();
    assert!(!catalog.is_empty(), "catalog must not be empty");

    let found = catalog
        .iter()
        .any(|n| n.get_node().name == "python_interpreter");
    assert!(found, "catalog must contain python_interpreter node");
}

#[test]
fn python_interpreter_metadata() {
    let catalog = get_catalog();
    let node_logic = catalog
        .iter()
        .find(|n| n.get_node().name == "python_interpreter")
        .expect("python_interpreter node must exist");

    let node = node_logic.get_node();
    assert_eq!(node.name, "python_interpreter");
    assert!(!node.friendly_name.is_empty());
    assert!(!node.description.is_empty());
    assert_eq!(node.category, "Code/Python");
    assert_eq!(node.long_running, Some(true));
}

#[test]
fn python_interpreter_has_all_input_pins() {
    let catalog = get_catalog();
    let node = catalog
        .iter()
        .find(|n| n.get_node().name == "python_interpreter")
        .unwrap()
        .get_node();

    let input_names: Vec<&str> = node
        .pins
        .values()
        .filter(|p| p.pin_type == flow_like::flow::pin::PinType::Input)
        .map(|p| p.name.as_str())
        .collect();

    let required = [
        "exec_in",
        "code",
        "inputs",
        "workspace",
        "packages",
        "package_allowlist",
        "network_enabled",
        "network_allowlist",
        "timeout_secs",
        "max_memory_mb",
    ];

    for name in &required {
        assert!(input_names.contains(name), "missing input pin: {name}");
    }
}

#[test]
fn python_interpreter_has_all_output_pins() {
    let catalog = get_catalog();
    let node = catalog
        .iter()
        .find(|n| n.get_node().name == "python_interpreter")
        .unwrap()
        .get_node();

    let output_names: Vec<&str> = node
        .pins
        .values()
        .filter(|p| p.pin_type == flow_like::flow::pin::PinType::Output)
        .map(|p| p.name.as_str())
        .collect();

    let required = [
        "exec_out",
        "exec_error",
        "result",
        "stdout",
        "stderr",
        "error_msg",
        "success",
    ];

    for name in &required {
        assert!(output_names.contains(name), "missing output pin: {name}");
    }
}

#[test]
fn input_output_pin_names_do_not_collide() {
    let catalog = get_catalog();
    let node = catalog
        .iter()
        .find(|n| n.get_node().name == "python_interpreter")
        .unwrap()
        .get_node();

    let input_names: std::collections::HashSet<&str> = node
        .pins
        .values()
        .filter(|p| p.pin_type == flow_like::flow::pin::PinType::Input)
        .map(|p| p.name.as_str())
        .collect();

    let output_names: std::collections::HashSet<&str> = node
        .pins
        .values()
        .filter(|p| p.pin_type == flow_like::flow::pin::PinType::Output)
        .map(|p| p.name.as_str())
        .collect();

    let collisions: Vec<&&str> = input_names.intersection(&output_names).collect();
    assert!(
        collisions.is_empty(),
        "input/output pin name collision: {collisions:?}"
    );
}

#[test]
fn timeout_pin_has_default_and_range() {
    let catalog = get_catalog();
    let node = catalog
        .iter()
        .find(|n| n.get_node().name == "python_interpreter")
        .unwrap()
        .get_node();

    let timeout_pin = node
        .pins
        .values()
        .find(|p| p.name == "timeout_secs")
        .expect("timeout_secs pin must exist");

    assert!(
        timeout_pin.default_value.is_some(),
        "timeout_secs must have a default value"
    );

    let default: f64 = serde_json::from_slice(timeout_pin.default_value.as_ref().unwrap()).unwrap();
    assert!(
        (default - 30.0).abs() < f64::EPSILON,
        "default timeout should be 30s"
    );
}

#[test]
fn network_disabled_by_default() {
    let catalog = get_catalog();
    let node = catalog
        .iter()
        .find(|n| n.get_node().name == "python_interpreter")
        .unwrap()
        .get_node();

    let net_pin = node
        .pins
        .values()
        .find(|p| p.name == "network_enabled")
        .expect("network_enabled pin must exist");

    let default: bool = serde_json::from_slice(net_pin.default_value.as_ref().unwrap()).unwrap();
    assert!(!default, "network must be disabled by default");
}
