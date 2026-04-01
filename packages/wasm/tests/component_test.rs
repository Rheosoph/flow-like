//! Integration tests for WASM Component Model (Python & TypeScript nodes)
//!
//! Tests loading and executing WASM components built with componentize-py / componentize-js.
//! Requires the `component-model` feature and pre-built WASM components.

#![cfg(feature = "component-model")]

use flow_like_wasm::abi::WasmExecutionInput;
use flow_like_wasm::component::instance::WasmComponentInstance;
use flow_like_wasm::component::WasmComponent;
use flow_like_wasm::engine::{WasmConfig, WasmEngine};
use flow_like_wasm::limits::WasmSecurityConfig;
use std::path::PathBuf;
use std::sync::Arc;

fn python_template_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates/wasm-node-python/build/node.wasm")
}

fn typescript_template_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates/wasm-node-typescript/build/node.wasm")
}

fn create_execution_input(
    inputs: serde_json::Map<String, serde_json::Value>,
) -> WasmExecutionInput {
    WasmExecutionInput {
        inputs,
        node_id: "test_node_id".to_string(),
        run_id: "test_run_id".to_string(),
        app_id: "test_app".to_string(),
        board_id: "test_board".to_string(),
        user_id: "test_user".to_string(),
        stream_state: false,
        log_level: 1,
        node_name: String::new(),
    }
}

#[tokio::test]
async fn test_detect_component_model_format() {
    let wasm_path = python_template_path();
    if !wasm_path.exists() {
        eprintln!(
            "Skipping test: Python template not built. Run: cd templates/wasm-node-python && uv run python build.py"
        );
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    assert!(
        flow_like_wasm::component::is_component_model(&bytes),
        "Python WASM should be Component Model format"
    );
}

#[tokio::test]
async fn test_load_python_component() {
    let wasm_path = python_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Python template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = WasmComponent::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load Python WASM component");

    assert_eq!(component.hash(), "test_hash");
}

#[tokio::test]
async fn test_python_component_get_definition() {
    let wasm_path = python_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Python template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .unwrap(),
    );

    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .expect("Failed to create component instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get node definition");

    assert_eq!(definition.name, "my_custom_node_py");
    assert_eq!(definition.friendly_name, "My Custom Node");
    assert!(!definition.description.is_empty());
    assert_eq!(definition.category, "Custom/WASM");

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"), "Missing exec pin");
    assert!(pin_names.contains(&"input_text"), "Missing input_text pin");
    assert!(pin_names.contains(&"multiplier"), "Missing multiplier pin");
    assert!(pin_names.contains(&"exec_out"), "Missing exec_out pin");
    assert!(
        pin_names.contains(&"output_text"),
        "Missing output_text pin"
    );
    assert!(pin_names.contains(&"char_count"), "Missing char_count pin");
}

#[tokio::test]
async fn test_python_component_get_abi_version() {
    let wasm_path = python_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Python template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .unwrap(),
    );

    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let version = instance.call_get_abi_version().await.unwrap();
    assert_eq!(version, 1, "ABI version should be 1");
}

#[tokio::test]
async fn test_python_component_execute() {
    let wasm_path = python_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Python template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .unwrap(),
    );

    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Hello"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute Python node");

    assert!(
        result.error.is_none(),
        "Execution returned error: {:?}",
        result.error
    );

    let output_text = result.outputs.get("output_text").unwrap().as_str().unwrap();
    assert_eq!(output_text, "HelloHelloHello");

    let char_count = result.outputs.get("char_count").unwrap().as_i64().unwrap();
    assert_eq!(char_count, 15);

    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

#[tokio::test]
async fn test_python_component_empty_input() {
    let wasm_path = python_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Python template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .unwrap(),
    );

    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let exec_input = create_execution_input(serde_json::Map::new());
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Empty input execution should succeed");

    assert!(result.error.is_none());

    let output_text = result.outputs.get("output_text").unwrap().as_str().unwrap();
    assert_eq!(output_text, "");

    let char_count = result.outputs.get("char_count").unwrap().as_i64().unwrap();
    assert_eq!(char_count, 0);
}

#[tokio::test]
async fn test_component_model_detection() {
    // Core WASM magic: \0asm\x01\x00\x00\x00
    let core_wasm = [0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
    assert!(!flow_like_wasm::component::is_component_model(&core_wasm));

    // Component Model magic: \0asm\x0d\x00\x01\x00
    let component_wasm = [0x00, 0x61, 0x73, 0x6D, 0x0D, 0x00, 0x01, 0x00];
    assert!(flow_like_wasm::component::is_component_model(
        &component_wasm
    ));

    // Too short
    assert!(!flow_like_wasm::component::is_component_model(&[
        0x00, 0x61
    ]));
}

// ── TypeScript Component Model Tests ──────────────────────────────────────────

#[tokio::test]
async fn test_detect_typescript_component_model_format() {
    let wasm_path = typescript_template_path();
    if !wasm_path.exists() {
        eprintln!(
            "Skipping test: TypeScript template not built. Run: cd templates/wasm-node-typescript && npm run build"
        );
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    assert!(
        flow_like_wasm::component::is_component_model(&bytes),
        "TypeScript WASM should be Component Model format"
    );
}

#[tokio::test]
async fn test_load_typescript_component() {
    let wasm_path = typescript_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: TypeScript template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = WasmComponent::from_bytes(&engine, &bytes, "ts_test_hash".to_string())
        .await
        .expect("Failed to load TypeScript WASM component");

    assert_eq!(component.hash(), "ts_test_hash");
}

#[tokio::test]
async fn test_typescript_component_get_definition() {
    let wasm_path = typescript_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: TypeScript template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "ts_test_hash".to_string())
            .await
            .unwrap(),
    );

    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .expect("Failed to create component instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get node definition");

    assert_eq!(definition.name, "my_custom_node_ts");
    assert_eq!(definition.friendly_name, "My Custom Node");
    assert!(!definition.description.is_empty());
    assert_eq!(definition.category, "Custom/WASM");

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"), "Missing exec pin");
    assert!(pin_names.contains(&"input_text"), "Missing input_text pin");
    assert!(pin_names.contains(&"multiplier"), "Missing multiplier pin");
    assert!(pin_names.contains(&"exec_out"), "Missing exec_out pin");
    assert!(
        pin_names.contains(&"output_text"),
        "Missing output_text pin"
    );
    assert!(pin_names.contains(&"char_count"), "Missing char_count pin");
}

#[tokio::test]
async fn test_typescript_component_get_abi_version() {
    let wasm_path = typescript_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: TypeScript template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "ts_test_hash".to_string())
            .await
            .unwrap(),
    );

    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let version = instance.call_get_abi_version().await.unwrap();
    assert_eq!(version, 1, "ABI version should be 1");
}

#[tokio::test]
async fn test_typescript_component_execute() {
    let wasm_path = typescript_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: TypeScript template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "ts_test_hash".to_string())
            .await
            .unwrap(),
    );

    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Hello"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute TypeScript node");

    assert!(
        result.error.is_none(),
        "Execution returned error: {:?}",
        result.error
    );

    let output_text = result.outputs.get("output_text").unwrap().as_str().unwrap();
    assert_eq!(output_text, "HelloHelloHello");

    let char_count = result.outputs.get("char_count").unwrap().as_i64().unwrap();
    assert_eq!(char_count, 15);

    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

#[tokio::test]
async fn test_typescript_component_empty_input() {
    let wasm_path = typescript_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: TypeScript template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "ts_test_hash".to_string())
            .await
            .unwrap(),
    );

    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let exec_input = create_execution_input(serde_json::Map::new());
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Empty input execution should succeed");

    assert!(result.error.is_none());

    let output_text = result.outputs.get("output_text").unwrap().as_str().unwrap();
    assert_eq!(output_text, "");

    let char_count = result.outputs.get("char_count").unwrap().as_i64().unwrap();
    assert_eq!(char_count, 0);
}
