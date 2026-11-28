use super::{extract_nodes, metadata_limits};

fn metadata_component(instances: usize, memory_pages: u32, table_elements: u32) -> Vec<u8> {
    assert!(instances >= 1);
    let definition = serde_json::json!([{
        "name": "metadata_probe",
        "friendly_name": "Metadata Probe",
        "description": "A node returned by the component metadata export.",
        "category": "Testing/Metadata",
        "pins": [{
            "name": "result",
            "friendly_name": "Result",
            "description": "The probe result.",
            "pin_type": "Output",
            "data_type": "String"
        }]
    }])
    .to_string();
    let data = definition
        .bytes()
        .map(|byte| format!("\\{byte:02x}"))
        .collect::<String>();
    let adapters = (1..instances)
        .map(|index| format!("(core instance $adapter{index} (instantiate $adapter))"))
        .collect::<String>();

    wat::parse_str(format!(
        r#"(component
            (core module $adapter)
            {adapters}
            (core module $main
                (memory (export "memory") {memory_pages})
                (table {table_elements} funcref)
                (data (i32.const 16) "{data}")
                (func (export "get-nodes") (result i32)
                    (i32.store (i32.const 0) (i32.const 16))
                    (i32.store (i32.const 4) (i32.const {length}))
                    i32.const 0))
            (core instance $main (instantiate $main))
            (alias core export $main "memory" (core memory $memory))
            (alias core export $main "get-nodes" (core func $get-nodes))
            (func (export "get-nodes") (result string)
                (canon lift (core func $get-nodes) (memory $memory))))"#,
        length = definition.len(),
    ))
    .expect("metadata fixture must be valid WAT")
}

#[tokio::test]
async fn extracts_metadata_from_a_component_with_four_core_instances() {
    let nodes = extract_nodes(&metadata_component(4, 1, 1))
        .await
        .expect("component adapters must fit the metadata instance budget");

    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].id, "metadata_probe");
    assert_eq!(nodes[0].friendly_name.as_deref(), Some("Metadata Probe"));
    assert_eq!(nodes[0].category, "Testing/Metadata");
    assert!(nodes[0].pins.contains_key("result"));
}

#[tokio::test]
async fn extracts_metadata_with_the_largest_observed_package_footprint() {
    // The inspected packages need up to 84.5 MiB of initial memory and
    // 14,915 table elements before their metadata export can run.
    let nodes = extract_nodes(&metadata_component(4, 1_352, 14_915))
        .await
        .expect("observed package initialization requirements must fit");
    assert_eq!(nodes[0].id, "metadata_probe");
}

#[tokio::test]
async fn rejects_component_initialization_above_each_resource_budget() {
    for (instances, pages, elements, resource) in [
        (5, 1, 1, "instance"),
        (4, 2_049, 1, "memory"),
        (4, 1, 20_001, "table"),
    ] {
        let error = extract_nodes(&metadata_component(instances, pages, elements))
            .await
            .expect_err("metadata extraction must enforce its resource budgets")
            .to_string();
        assert!(
            error.contains("Failed to instantiate WASM for node extraction"),
            "expected {resource} rejection during instantiation, got: {error}"
        );
        assert!(
            error.contains(resource),
            "expected an error identifying {resource}, got: {error}"
        );
    }
}

#[test]
fn metadata_resource_budget_preserves_guest_limits() {
    let limits = metadata_limits();
    assert_eq!(limits.max_instances, 4);
    assert_eq!(limits.memory_limit, 128 * 1024 * 1024);
    assert_eq!(limits.max_table_elements, 20_000);
    assert_eq!(
        limits.fuel_limit,
        flow_like_wasm::compilation::WasmLimits::restrictive().fuel_limit
    );
}
