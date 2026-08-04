//! Verifies a package's built artifacts end to end, outside the templates.
//!
//! Point it at any package built with `mise run build`:
//! ```bash
//! FLOW_LIKE_PACKAGE_DIR=examples/sales-insights \
//!   cargo test -p flow-like-wasm --test external_package_test -- --ignored --nocapture
//! ```
//! It loads `node.wasm` through the host runtime (listing and executing nodes)
//! and validates `widgets.flwb` with the bundle reader, so both halves of a
//! node + micro-widget package are checked the way the app loads them.

#![cfg(feature = "component-model")]

use flow_like_wasm::abi::WasmExecutionInput;
use flow_like_wasm::component::instance::WasmComponentInstance;
use flow_like_wasm::component::WasmComponent;
use flow_like_wasm::engine::{WasmConfig, WasmEngine};
use flow_like_wasm::limits::WasmSecurityConfig;
use flow_like_wasm::widget_bundle::WidgetBundleReader;
use std::path::PathBuf;
use std::sync::Arc;

fn package_dir() -> PathBuf {
    let raw = std::env::var("FLOW_LIKE_PACKAGE_DIR").expect("FLOW_LIKE_PACKAGE_DIR not set");
    let path = PathBuf::from(&raw);
    if path.is_absolute() {
        return path;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(path)
}

fn execution_input(
    node_name: &str,
    inputs: serde_json::Map<String, serde_json::Value>,
) -> WasmExecutionInput {
    WasmExecutionInput {
        inputs,
        node_id: "external_node".to_string(),
        run_id: "external_run".to_string(),
        app_id: "external_app".to_string(),
        board_id: "external_board".to_string(),
        user_id: "external_user".to_string(),
        stream_state: false,
        log_level: 1,
        node_name: node_name.to_string(),
    }
}

#[tokio::test]
#[ignore = "requires FLOW_LIKE_PACKAGE_DIR pointing at a built package"]
async fn test_external_package_node_artifact_loads_and_runs() {
    let wasm_path = package_dir().join("node.wasm");
    assert!(
        wasm_path.exists(),
        "node.wasm missing at {} — run `mise run build` in the package",
        wasm_path.display()
    );

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "external_package".to_string())
            .await
            .expect("node.wasm is not a loadable WASM component"),
    );

    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let nodes = instance
        .call_get_nodes()
        .await
        .expect("call_get_nodes failed");
    assert!(!nodes.is_empty(), "package exposes no nodes");
    println!(
        "node.wasm exposes {} node(s): {}",
        nodes.len(),
        nodes
            .iter()
            .map(|node| node.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Execute the first node with its declared pin defaults — a package whose
    // nodes cannot run at all would fail here.
    let first = &nodes[0];
    let mut inputs = serde_json::Map::new();
    for pin in &first.pins {
        if let Some(default) = &pin.default_value {
            inputs.insert(pin.name.clone(), default.clone());
        }
    }
    let result = instance
        .call_run(&execution_input(&first.name, inputs))
        .await
        .unwrap_or_else(|e| panic!("executing '{}' failed: {e}", first.name));
    assert!(
        result.error.is_none(),
        "node '{}' reported an error: {:?}",
        first.name,
        result.error
    );
    println!(
        "executed '{}' → outputs: {}",
        first.name,
        result
            .outputs
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
}

#[tokio::test]
#[ignore = "requires FLOW_LIKE_PACKAGE_DIR pointing at a built package"]
async fn test_external_package_widget_bundle_is_valid() {
    let bundle_path = package_dir().join("widgets.flwb");
    assert!(
        bundle_path.exists(),
        "widgets.flwb missing at {} — run `mise run build` in the package",
        bundle_path.display()
    );

    let mut reader = WidgetBundleReader::open(&bundle_path).expect("failed to open widgets.flwb");
    reader
        .validate()
        .map_err(|errors| errors.join("\n"))
        .expect("bundle validation failed");

    let manifest = reader.manifest().clone();
    assert!(!manifest.widgets.is_empty(), "bundle declares no widgets");

    for widget in &manifest.widgets {
        let contract = reader.contract(&widget.id).expect("contract unreadable");
        contract
            .validate()
            .map_err(|errors| errors.join("\n"))
            .expect("contract invalid");
        println!(
            "widget '{}' — {} input(s), {} event(s), {} query(ies)",
            widget.id,
            contract.inputs.len(),
            contract.events.len(),
            contract.queries.len()
        );

        // The served document must reference only chunks the bundle declares.
        let entry = reader.read_entry(&widget.entry).expect("entry unreadable");
        let html = String::from_utf8(entry).expect("entry is not UTF-8");
        assert!(
            html.contains("__FLW_CONTRACT__"),
            "widget '{}' is missing its injected contract",
            widget.id
        );
        assert!(
            html.contains("Content-Security-Policy"),
            "widget '{}' is missing its CSP meta tag",
            widget.id
        );
    }
}
