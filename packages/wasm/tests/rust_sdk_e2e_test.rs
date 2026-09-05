//! End-to-end integration tests for Rust WASM SDK (Component Model)
//!
//! Tests the `#[register_node]` + `impl WasmNode` + `wasm_main!()` pattern
//! by loading the compiled Rust template and exercising it through the host runtime.
//!
//! Requires `component-model` feature (default) and a pre-built template binary:
//! ```bash
//! cd templates/wasm-node-rust && cargo build --release
//! ```

#![cfg(feature = "component-model")]

use flow_like_wasm::abi::WasmExecutionInput;
use flow_like_wasm::component::instance::WasmComponentInstance;
use flow_like_wasm::component::WasmComponent;
use flow_like_wasm::engine::{WasmConfig, WasmEngine};
use flow_like_wasm::limits::WasmSecurityConfig;
use flow_like_wasm::WASM_ABI_VERSION;
use std::path::PathBuf;
use std::sync::Arc;

fn rust_cm_template_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates/wasm-node-rust/target/wasm32-wasip2/release/flow_like_wasm_node_template.wasm")
}

fn skip_if_not_built() -> Option<PathBuf> {
    let path = rust_cm_template_path();
    if !path.exists() {
        eprintln!(
            "Skipping test: Rust CM template not built. Run: cd templates/wasm-node-rust && cargo build --release"
        );
        return None;
    }
    Some(path)
}

async fn load_template() -> Option<(WasmEngine, Arc<WasmComponent>)> {
    let path = skip_if_not_built()?;
    let bytes = tokio::fs::read(&path).await.unwrap();
    let engine = WasmEngine::new(WasmConfig::default()).unwrap();
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "rust_sdk_test".to_string())
            .await
            .expect("Failed to load Rust CM template"),
    );
    Some((engine, component))
}

fn create_execution_input(
    node_name: &str,
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
        node_name: node_name.to_string(),
    }
}

// ── Detection ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_rust_sdk_is_component_model() {
    let Some(path) = skip_if_not_built() else {
        return;
    };
    let bytes = tokio::fs::read(&path).await.unwrap();
    assert!(
        flow_like_wasm::component::is_component_model(&bytes),
        "Rust wasm32-wasip2 binary should be Component Model format"
    );
}

// ── Loading ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_rust_sdk_load_component() {
    let Some((_, component)) = load_template().await else {
        return;
    };
    assert_eq!(component.hash(), "rust_sdk_test");
}

// ── get-nodes (multi-node) ─────────────────────────────────────────────

#[tokio::test]
async fn test_rust_sdk_get_nodes_returns_template_nodes() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let nodes = instance
        .call_get_nodes()
        .await
        .expect("call_get_nodes failed");
    let expected_names = [
        "repeat_text",
        "char_count",
        "greeting",
        "file_writer",
        "file_reader",
        "weather_agent",
        "object_create_buffer",
        "object_append_buffer",
        "object_read_buffer",
        "object_close_buffer",
        "object_create_cursor",
        "object_next_item",
        "object_finish_cursor",
        "tcp_start_listener",
        "tcp_accept_connection",
        "tcp_send_text",
        "tcp_poll_send",
        "tcp_close_listener",
        "tcp_close_connection",
    ];
    assert_eq!(
        nodes.len(),
        expected_names.len(),
        "Template should expose all example nodes"
    );
    let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
    for expected_name in expected_names {
        assert!(
            names.contains(&expected_name),
            "Missing {expected_name} node"
        );
    }
}

#[tokio::test]
async fn test_rust_sdk_repeat_text_definition() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let nodes = instance.call_get_nodes().await.unwrap();
    let repeat = nodes.iter().find(|n| n.name == "repeat_text").unwrap();

    assert_eq!(repeat.friendly_name, "Repeat Text");
    assert_eq!(repeat.category, "Custom/WASM");
    assert!(!repeat.description.is_empty());

    let pin_names: Vec<&str> = repeat.pins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(pin_names.len(), 5, "repeat_text should have 5 pins");
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
}

#[tokio::test]
async fn test_rust_sdk_char_count_definition() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let nodes = instance.call_get_nodes().await.unwrap();
    let counter = nodes.iter().find(|n| n.name == "char_count").unwrap();

    assert_eq!(counter.friendly_name, "Character Count");
    assert_eq!(counter.category, "Custom/WASM");

    let pin_names: Vec<&str> = counter.pins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(pin_names.len(), 4, "char_count should have 4 pins");
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"char_count"));
}

// ── ABI version ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_rust_sdk_abi_version() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let version = instance.call_get_abi_version().await.unwrap();
    assert_eq!(
        version, WASM_ABI_VERSION,
        "ABI version should match runtime"
    );
}

// ── Execution: repeat_text ─────────────────────────────────────────────

#[tokio::test]
async fn test_rust_sdk_execute_repeat_text() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Hello"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let result = instance
        .call_run(&create_execution_input("repeat_text", inputs))
        .await
        .expect("repeat_text execution failed");

    assert!(result.error.is_none(), "Error: {:?}", result.error);

    let output = result.outputs.get("output_text").unwrap().as_str().unwrap();
    assert_eq!(output, "HelloHelloHello");

    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

#[tokio::test]
async fn test_rust_sdk_repeat_text_single_repetition() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("A"));
    inputs.insert("multiplier".to_string(), serde_json::json!(1));

    let result = instance
        .call_run(&create_execution_input("repeat_text", inputs))
        .await
        .unwrap();

    assert!(result.error.is_none());
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "A"
    );
}

#[tokio::test]
async fn test_rust_sdk_repeat_text_zero_multiplier() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Hello"));
    inputs.insert("multiplier".to_string(), serde_json::json!(0));

    let result = instance
        .call_run(&create_execution_input("repeat_text", inputs))
        .await
        .unwrap();

    assert!(result.error.is_none());
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        ""
    );
}

#[tokio::test]
async fn test_rust_sdk_repeat_text_defaults() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let result = instance
        .call_run(&create_execution_input(
            "repeat_text",
            serde_json::Map::new(),
        ))
        .await
        .unwrap();

    assert!(result.error.is_none());
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        ""
    );
}

// ── Execution: char_count ──────────────────────────────────────────────

#[tokio::test]
async fn test_rust_sdk_execute_char_count() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Hello"));

    let result = instance
        .call_run(&create_execution_input("char_count", inputs))
        .await
        .expect("char_count execution failed");

    assert!(result.error.is_none(), "Error: {:?}", result.error);

    let count = result.outputs.get("char_count").unwrap().as_i64().unwrap();
    assert_eq!(count, 5);

    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

#[tokio::test]
async fn test_rust_sdk_char_count_empty_string() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!(""));

    let result = instance
        .call_run(&create_execution_input("char_count", inputs))
        .await
        .unwrap();

    assert!(result.error.is_none());
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        0
    );
}

#[tokio::test]
async fn test_rust_sdk_char_count_unicode() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("héllo 🌍"));

    let result = instance
        .call_run(&create_execution_input("char_count", inputs))
        .await
        .unwrap();

    assert!(result.error.is_none());
    // "héllo 🌍" is 7 chars but .len() counts bytes (11 bytes: é=2, 🌍=4)
    let count = result.outputs.get("char_count").unwrap().as_i64().unwrap();
    assert_eq!(count, 11, "len() counts bytes, not chars");
}

// ── Unknown node dispatch ──────────────────────────────────────────────

#[tokio::test]
async fn test_rust_sdk_unknown_node_returns_error() {
    let Some((engine, component)) = load_template().await else {
        return;
    };
    let mut instance =
        WasmComponentInstance::new(&engine, component, WasmSecurityConfig::permissive())
            .await
            .unwrap();

    let result = instance
        .call_run(&create_execution_input(
            "nonexistent_node",
            serde_json::Map::new(),
        ))
        .await
        .expect("Should return result, not crash");

    assert!(result.error.is_some(), "Unknown node should produce error");
    let err = result.error.unwrap();
    assert!(
        err.contains("nonexistent_node"),
        "Error should mention the unknown node name: {err}"
    );
}
