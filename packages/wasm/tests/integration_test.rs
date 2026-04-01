//! Integration tests for WASM node loading and execution
//!
//! These tests verify that the WASM runtime can correctly load and execute
//! nodes built from the templates.

use flow_like_wasm::abi::WasmExecutionInput;
#[cfg(feature = "component-model")]
use flow_like_wasm::component::WasmComponent;
use flow_like_wasm::engine::{WasmConfig, WasmEngine};
use flow_like_wasm::instance::WasmInstance;
use flow_like_wasm::limits::WasmSecurityConfig;
use flow_like_wasm::module::WasmModule;
use std::path::PathBuf;
use std::sync::Arc;

/// Get the path to the compiled Rust template WASM
fn rust_template_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates/wasm-node-rust/target/wasm32-unknown-unknown/release/flow_like_wasm_node_template.wasm")
}

/// Get the path to the compiled Rust math_nodes example
fn rust_math_nodes_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates/wasm-node-rust/target/wasm32-unknown-unknown/release/examples/math_nodes.wasm")
}

/// Get the path to the compiled AssemblyScript template WASM
fn as_template_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates/wasm-node-assemblyscript/build/release.wasm")
}

/// Get the path to the compiled AssemblyScript examples WASM
fn as_examples_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates/wasm-node-assemblyscript/build/examples.wasm")
}

fn templates_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates")
}

fn go_template_path() -> PathBuf {
    templates_root().join("wasm-node-go/node.wasm")
}

fn cpp_template_path() -> PathBuf {
    templates_root().join("wasm-node-cpp/build/node.wasm")
}

fn csharp_template_path() -> PathBuf {
    templates_root()
        .join("wasm-node-csharp/bin/Release/net10.0/wasi-wasm/AppBundle/FlowLikeWasmNode.wasm")
}

fn kotlin_template_path() -> PathBuf {
    templates_root().join("wasm-node-kotlin/build/compileSync/wasmWasi/main/productionExecutable/optimized/flow-like-wasm-node-kotlin.wasm")
}

fn zig_template_path() -> PathBuf {
    templates_root().join("wasm-node-zig/zig-out/bin/node.wasm")
}

fn nim_template_path() -> PathBuf {
    templates_root().join("wasm-node-nim/build/node.wasm")
}

fn grain_template_path() -> PathBuf {
    templates_root().join("wasm-node-grain/build/node.wasm")
}

fn moonbit_template_path() -> PathBuf {
    templates_root().join("wasm-node-moonbit/build/node.wasm")
}

fn lua_template_path() -> PathBuf {
    templates_root().join("wasm-node-lua/build/node.wasm")
}

fn swift_template_path() -> PathBuf {
    templates_root().join("wasm-node-swift/.build/release/Node.wasm")
}

fn java_template_path() -> PathBuf {
    templates_root().join("wasm-node-java/target/wasm/classes.wasm")
}

/// Helper to create a basic execution input
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

/// Helper to create execution input with node_name for multi-node packages
fn create_package_execution_input(
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

// ============================================================================
// Rust Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_rust_template() {
    let wasm_path = rust_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust template not built. Run: cd templates/wasm-node-rust && cargo build --release --target wasm32-unknown-unknown");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load module");

    // Verify module was loaded
    assert!(module.has_alloc());
}

#[tokio::test]
async fn test_rust_template_get_definition() {
    let wasm_path = rust_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get node definition");

    // Verify definition fields
    assert_eq!(definition.name, "my_custom_node");
    assert_eq!(definition.friendly_name, "My Custom Node");
    assert!(!definition.description.is_empty());
    assert_eq!(definition.category, "Custom/WASM");

    // Verify pins exist
    assert!(!definition.pins.is_empty());

    // Check for expected pins
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
async fn test_rust_template_execute() {
    let wasm_path = rust_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    // Create execution input
    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Hello"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute node");

    // Verify no error
    assert!(
        result.error.is_none(),
        "Execution returned error: {:?}",
        result.error
    );

    // Verify outputs
    assert!(
        result.outputs.contains_key("output_text"),
        "Missing output_text output"
    );
    assert!(
        result.outputs.contains_key("char_count"),
        "Missing char_count output"
    );

    // Verify output values
    let output_text = result.outputs.get("output_text").unwrap().as_str().unwrap();
    assert_eq!(output_text, "HelloHelloHello");

    let char_count = result.outputs.get("char_count").unwrap().as_i64().unwrap();
    assert_eq!(char_count, 15);

    // Verify exec pin was activated
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Rust Math Nodes Package Tests
// ============================================================================

#[tokio::test]
async fn test_rust_math_nodes_definitions() {
    let wasm_path = rust_math_nodes_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust math_nodes example not built. Run: cd templates/wasm-node-rust && cargo build --release --target wasm32-unknown-unknown --examples");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    // Verify this is a multi-node package
    assert!(
        instance.is_package(),
        "math_nodes should be a multi-node package"
    );

    // Get all node definitions
    let definitions = instance
        .call_get_nodes()
        .await
        .expect("Failed to get node definitions");

    // The math_nodes package should have multiple definitions
    println!("Got {} definitions", definitions.len());
    assert!(
        !definitions.is_empty(),
        "Package should have at least one node"
    );

    // Find the math_add node
    let add_node = definitions.iter().find(|d| d.name == "math_add");
    assert!(add_node.is_some(), "Should have math_add node");
}

#[tokio::test]
async fn test_rust_math_add_execute() {
    let wasm_path = rust_math_nodes_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust math_nodes example not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    // Test math_add
    let mut inputs = serde_json::Map::new();
    inputs.insert("a".to_string(), serde_json::json!(5.0));
    inputs.insert("b".to_string(), serde_json::json!(3.0));

    let exec_input = create_package_execution_input("math_add", inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute math_add");

    assert!(
        result.error.is_none(),
        "Execution returned error: {:?}",
        result.error
    );

    let sum = result.outputs.get("result").unwrap().as_f64().unwrap();
    assert!((sum - 8.0).abs() < 0.001, "Expected 8.0, got {}", sum);
}

#[tokio::test]
async fn test_rust_math_multiply_execute() {
    let wasm_path = rust_math_nodes_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust math_nodes example not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    // Test math_multiply
    let mut inputs = serde_json::Map::new();
    inputs.insert("a".to_string(), serde_json::json!(4.0));
    inputs.insert("b".to_string(), serde_json::json!(7.0));

    let exec_input = create_package_execution_input("math_multiply", inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute math_multiply");

    assert!(
        result.error.is_none(),
        "Execution returned error: {:?}",
        result.error
    );

    let product = result.outputs.get("result").unwrap().as_f64().unwrap();
    assert!(
        (product - 28.0).abs() < 0.001,
        "Expected 28.0, got {}",
        product
    );
}

#[tokio::test]
async fn test_rust_math_divide_by_zero() {
    let wasm_path = rust_math_nodes_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust math_nodes example not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    // Test math_divide with division by zero
    let mut inputs = serde_json::Map::new();
    inputs.insert("a".to_string(), serde_json::json!(10.0));
    inputs.insert("b".to_string(), serde_json::json!(0.0));

    let exec_input = create_package_execution_input("math_divide", inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute math_divide");

    assert!(
        result.error.is_none(),
        "Execution returned error: {:?}",
        result.error
    );

    // Should indicate invalid division
    let is_valid = result.outputs.get("is_valid").unwrap().as_bool().unwrap();
    assert!(!is_valid, "Division by zero should set is_valid to false");
}

// ============================================================================
// AssemblyScript Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_assemblyscript_template() {
    let wasm_path = as_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript template not built. Run: cd templates/wasm-node-assemblyscript && npm run build");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load module");

    // Verify module was loaded
    assert!(
        module.has_alloc(),
        "AssemblyScript module should export alloc"
    );
}

#[tokio::test]
async fn test_assemblyscript_template_get_definition() {
    let wasm_path = as_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get node definition");

    // Verify definition fields
    assert_eq!(definition.name, "my_custom_node_as");
    assert_eq!(definition.friendly_name, "My Custom Node (AS)");
    assert!(!definition.description.is_empty());
    assert_eq!(definition.category, "Custom/WASM");

    // Verify pins exist
    assert!(!definition.pins.is_empty());

    // Check for expected pins
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
async fn test_assemblyscript_template_execute() {
    let wasm_path = as_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    // Create execution input - AssemblyScript expects JSON string values
    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("World"));
    inputs.insert("multiplier".to_string(), serde_json::json!(2));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute node");

    // Verify no error
    assert!(
        result.error.is_none(),
        "Execution returned error: {:?}",
        result.error
    );

    // Verify outputs
    assert!(
        result.outputs.contains_key("output_text"),
        "Missing output_text output"
    );
    assert!(
        result.outputs.contains_key("char_count"),
        "Missing char_count output"
    );

    // Verify output values
    let output_text = result.outputs.get("output_text").unwrap().as_str().unwrap();
    assert_eq!(output_text, "WorldWorld");

    let char_count = result.outputs.get("char_count").unwrap().as_i64().unwrap();
    assert_eq!(char_count, 10);

    // Verify exec pin was activated
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// AssemblyScript Examples (Multi-Node Package) Tests
// ============================================================================

#[tokio::test]
async fn test_assemblyscript_examples_load_as_package() {
    let wasm_path = as_examples_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript examples not built. Run: cd templates/wasm-node-assemblyscript && npm run build:examples");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("AssemblyScript examples should load as a valid module with get_nodes + run"),
    );

    let instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    assert!(
        instance.is_package(),
        "examples.wasm should be a multi-node package"
    );
}

#[tokio::test]
async fn test_assemblyscript_examples_get_nodes() {
    let wasm_path = as_examples_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript examples not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definitions = instance
        .call_get_nodes()
        .await
        .expect("Failed to get node definitions from AS examples");

    // Should have math (5) + string (7) + control flow (6) = 18 nodes
    assert!(
        definitions.len() >= 18,
        "Expected at least 18 nodes, got {}",
        definitions.len()
    );

    let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"math_add_as"), "Missing math_add_as node");
    assert!(
        names.contains(&"math_subtract_as"),
        "Missing math_subtract_as node"
    );
    assert!(
        names.contains(&"math_multiply_as"),
        "Missing math_multiply_as node"
    );
    assert!(
        names.contains(&"math_divide_as"),
        "Missing math_divide_as node"
    );
    assert!(
        names.contains(&"math_clamp_as"),
        "Missing math_clamp_as node"
    );
    assert!(
        names.contains(&"string_uppercase_as"),
        "Missing string_uppercase_as node"
    );
    assert!(
        names.contains(&"string_lowercase_as"),
        "Missing string_lowercase_as node"
    );
    assert!(
        names.contains(&"string_concat_as"),
        "Missing string_concat_as node"
    );
    assert!(names.contains(&"if_branch_as"), "Missing if_branch_as node");
    assert!(names.contains(&"gate_as"), "Missing gate_as node");

    for def in &definitions {
        assert!(!def.name.is_empty(), "Node definition has empty name");
        assert!(
            !def.friendly_name.is_empty(),
            "Node {} has empty friendly_name",
            def.name
        );
        assert!(!def.pins.is_empty(), "Node {} has no pins", def.name);
    }
}

#[tokio::test]
async fn test_assemblyscript_examples_run_math_add() {
    let wasm_path = as_examples_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript examples not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("a".to_string(), serde_json::json!(5.0));
    inputs.insert("b".to_string(), serde_json::json!(3.0));

    let exec_input = create_package_execution_input("math_add_as", inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute math_add_as");

    assert!(
        result.error.is_none(),
        "math_add_as error: {:?}",
        result.error
    );

    let sum = result.outputs.get("result").unwrap().as_f64().unwrap();
    assert!((sum - 8.0).abs() < 0.001, "Expected 8.0, got {}", sum);
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

#[tokio::test]
async fn test_assemblyscript_examples_run_string_uppercase() {
    let wasm_path = as_examples_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript examples not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("text".to_string(), serde_json::json!("hello world"));

    let exec_input = create_package_execution_input("string_uppercase_as", inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute string_uppercase_as");

    assert!(
        result.error.is_none(),
        "string_uppercase_as error: {:?}",
        result.error
    );

    let output = result.outputs.get("result").unwrap().as_str().unwrap();
    assert_eq!(output, "HELLO WORLD");
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

#[tokio::test]
async fn test_assemblyscript_examples_run_if_branch() {
    let wasm_path = as_examples_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript examples not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module.clone(), WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    // Test true branch
    let mut inputs = serde_json::Map::new();
    inputs.insert("condition".to_string(), serde_json::json!(true));

    let exec_input = create_package_execution_input("if_branch_as", inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute if_branch_as (true)");

    assert!(
        result.error.is_none(),
        "if_branch_as error: {:?}",
        result.error
    );
    assert!(result.activate_exec.contains(&"true_branch".to_string()));
    assert!(!result.activate_exec.contains(&"false_branch".to_string()));

    // Test false branch (new instance needed — fuel may be consumed)
    let mut instance2 = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs2 = serde_json::Map::new();
    inputs2.insert("condition".to_string(), serde_json::json!(false));

    let exec_input2 = create_package_execution_input("if_branch_as", inputs2);
    let result2 = instance2
        .call_run(&exec_input2)
        .await
        .expect("Failed to execute if_branch_as (false)");

    assert!(result2.error.is_none());
    assert!(!result2.activate_exec.contains(&"true_branch".to_string()));
    assert!(result2.activate_exec.contains(&"false_branch".to_string()));
}

#[tokio::test]
async fn test_assemblyscript_examples_run_unknown_node() {
    let wasm_path = as_examples_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript examples not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let inputs = serde_json::Map::new();
    let exec_input = create_package_execution_input("nonexistent_node", inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute run for unknown node");

    assert!(
        result.error.is_some(),
        "Unknown node should return an error"
    );
    let err = result.error.unwrap();
    assert!(
        err.contains("nonexistent_node"),
        "Error should mention the unknown node name: {}",
        err
    );
}

#[tokio::test]
async fn test_assemblyscript_examples_run_divide_by_zero() {
    let wasm_path = as_examples_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript examples not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("a".to_string(), serde_json::json!(10.0));
    inputs.insert("b".to_string(), serde_json::json!(0.0));

    let exec_input = create_package_execution_input("math_divide_as", inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute math_divide_as");

    assert!(
        result.error.is_none(),
        "math_divide_as error: {:?}",
        result.error
    );

    let is_valid = result.outputs.get("is_valid").unwrap().as_bool().unwrap();
    assert!(!is_valid, "Division by zero should set is_valid to false");
}

#[tokio::test]
async fn test_assemblyscript_examples_run_string_concat() {
    let wasm_path = as_examples_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript examples not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("a".to_string(), serde_json::json!("Hello"));
    inputs.insert("b".to_string(), serde_json::json!("World"));
    inputs.insert("separator".to_string(), serde_json::json!(", "));

    let exec_input = create_package_execution_input("string_concat_as", inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute string_concat_as");

    assert!(
        result.error.is_none(),
        "string_concat_as error: {:?}",
        result.error
    );

    let output = result.outputs.get("result").unwrap().as_str().unwrap();
    assert_eq!(output, "Hello, World");
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Go Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_go_template() {
    let wasm_path = go_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Go template not built. Run: cd templates/wasm-node-go && tinygo build -o node.wasm -target wasm -no-debug ./");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load Go module");

    assert!(module.has_alloc(), "Go module should export alloc");
}

#[tokio::test]
async fn test_go_template_get_definition() {
    let wasm_path = go_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Go template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get Go node definition");

    assert_eq!(definition.name, "my_custom_node_go");
    assert!(!definition.pins.is_empty());

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
    assert!(pin_names.contains(&"char_count"));
}

#[tokio::test]
async fn test_go_template_execute() {
    let wasm_path = go_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Go template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Go"));
    inputs.insert("multiplier".to_string(), serde_json::json!(4));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute Go node");

    assert!(
        result.error.is_none(),
        "Go execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "GoGoGoGo"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        8
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// C++ Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_cpp_template() {
    let wasm_path = cpp_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: C++ template not built. Run: cd templates/wasm-node-cpp && mkdir -p build && cd build && emcmake cmake .. && emmake make");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load C++ module");

    assert!(module.has_alloc(), "C++ module should export alloc");
}

#[tokio::test]
async fn test_cpp_template_get_definition() {
    let wasm_path = cpp_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: C++ template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get C++ node definition");

    assert_eq!(definition.name, "my_custom_node_cpp");
    assert!(!definition.pins.is_empty());

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
    assert!(pin_names.contains(&"char_count"));
}

#[tokio::test]
async fn test_cpp_template_execute() {
    let wasm_path = cpp_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: C++ template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Cpp"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute C++ node");

    assert!(
        result.error.is_none(),
        "C++ execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "CppCppCpp"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        9
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// C# Template Tests
// ============================================================================

#[tokio::test]
#[cfg(feature = "component-model")]
async fn test_load_csharp_template() {
    let wasm_path = csharp_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: C# template not built. Run: cd templates/wasm-node-csharp && WASI_SDK_PATH=$PWD/../../.tools/wasi-sdk-25.0-arm64-macos/ dotnet publish -c Release /p:WasmSingleFileBundle=true /p:WasiClangLinkOptimizationFlag=-O0 /p:WasiClangCompileOptimizationFlag=-O0 /p:WasiBitcodeCompileOptimizationFlag=-O0");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let component = WasmComponent::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load C# WASM component");

    assert!(!component.hash().is_empty());
}

#[tokio::test]
#[cfg(feature = "component-model")]
async fn test_csharp_template_get_definition() {
    let wasm_path = csharp_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: C# template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load C# component"),
    );

    let mut instance = component
        .instantiate(&engine, &WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create component instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get C# node definition");

    assert_eq!(definition.name, "my_custom_node_csharp");
    assert!(!definition.pins.is_empty());
}

#[tokio::test]
#[cfg(feature = "component-model")]
async fn test_csharp_template_execute() {
    let wasm_path = csharp_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: C# template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let component = Arc::new(
        WasmComponent::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load C# component"),
    );

    let mut instance = component
        .instantiate(&engine, &WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create component instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("CS"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute C# node");

    assert!(
        result.error.is_none(),
        "C# execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "CSCSCS"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        6
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Kotlin Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_kotlin_template() {
    let wasm_path = kotlin_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Kotlin template not built. Run: cd templates/wasm-node-kotlin && ./gradlew build");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load Kotlin WASM module");

    assert!(module.has_alloc());

    let module = Arc::new(module);
    let instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create Kotlin instance");

    assert!(instance.remaining_fuel().unwrap() > 0);
}

#[tokio::test]
async fn test_kotlin_template_get_definition() {
    let wasm_path = kotlin_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Kotlin template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = match WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string()).await {
        Ok(m) => Arc::new(m),
        Err(_) => {
            eprintln!("Skipping: Kotlin wasm module not compatible");
            return;
        }
    };

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get Kotlin node definition");

    assert_eq!(definition.name, "my_custom_node_kt");
    assert!(!definition.pins.is_empty());

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
    assert!(pin_names.contains(&"char_count"));
}

#[tokio::test]
async fn test_kotlin_template_execute() {
    let wasm_path = kotlin_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Kotlin template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = match WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string()).await {
        Ok(m) => Arc::new(m),
        Err(_) => {
            eprintln!("Skipping: Kotlin wasm module not compatible");
            return;
        }
    };

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Kt"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute Kotlin node");

    assert!(
        result.error.is_none(),
        "Kotlin execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "KtKtKt"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        6
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Zig Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_zig_template() {
    let wasm_path = zig_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Zig template not built. Run: cd templates/wasm-node-zig && zig build -Doptimize=ReleaseSmall");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load Zig module");

    assert!(module.has_alloc(), "Zig module should export alloc");
}

#[tokio::test]
async fn test_zig_template_get_definition() {
    let wasm_path = zig_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Zig template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get Zig node definition");

    assert_eq!(definition.name, "my_custom_node_zig");
    assert!(!definition.pins.is_empty());

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
    assert!(pin_names.contains(&"char_count"));
}

#[tokio::test]
async fn test_zig_template_execute() {
    let wasm_path = zig_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Zig template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Zig"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute Zig node");

    assert!(
        result.error.is_none(),
        "Zig execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "ZigZigZig"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        9
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Nim Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_nim_template() {
    let wasm_path = nim_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Nim template not built. Run: cd templates/wasm-node-nim && nimble build");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load Nim module");

    assert!(module.has_alloc(), "Nim module should export alloc");
}

#[tokio::test]
async fn test_nim_template_get_definition() {
    let wasm_path = nim_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Nim template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get Nim node definition");

    assert_eq!(definition.name, "my_custom_node_nim");
    assert!(!definition.pins.is_empty());

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
    assert!(pin_names.contains(&"char_count"));
}

#[tokio::test]
async fn test_nim_template_execute() {
    let wasm_path = nim_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Nim template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Nim"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute Nim node");

    assert!(
        result.error.is_none(),
        "Nim execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "NimNimNim"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        9
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Grain Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_grain_template() {
    let wasm_path = grain_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Grain template not built. Run: cd templates/wasm-node-grain && mise run build");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load Grain module");

    assert!(module.has_alloc(), "Grain module should export alloc");
}

#[tokio::test]
async fn test_grain_template_get_definition() {
    let wasm_path = grain_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Grain template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get Grain node definition");

    assert_eq!(definition.name, "my_custom_node_grain");
    assert!(!definition.pins.is_empty());

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
    assert!(pin_names.contains(&"char_count"));
}

#[tokio::test]
async fn test_grain_template_execute() {
    let wasm_path = grain_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Grain template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Grain"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute Grain node");

    assert!(
        result.error.is_none(),
        "Grain execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "GrainGrainGrain"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        15
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Lua Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_moonbit_template() {
    let wasm_path = moonbit_template_path();
    if !wasm_path.exists() {
        eprintln!(
            "Skipping test: MoonBit template not built. Run: cd templates/wasm-node-moonbit && mise run build"
        );
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load MoonBit module");

    assert!(module.has_alloc(), "MoonBit module should export alloc");
}

#[tokio::test]
async fn test_moonbit_template_get_definition() {
    let wasm_path = moonbit_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: MoonBit template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get MoonBit node definition");

    assert_eq!(definition.name, "my_custom_node_moonbit");
    assert!(!definition.pins.is_empty());

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
    assert!(pin_names.contains(&"char_count"));
}

#[tokio::test]
async fn test_moonbit_template_execute() {
    let wasm_path = moonbit_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: MoonBit template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Moon"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute MoonBit node");

    assert!(
        result.error.is_none(),
        "MoonBit execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "MoonMoonMoon"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        12
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Lua Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_lua_template() {
    let wasm_path = lua_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Lua template not built. Run: cd templates/wasm-node-lua && mise run build");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load Lua WASM module");

    assert!(module.has_alloc(), "Lua module should export alloc");
}

#[tokio::test]
async fn test_lua_template_get_definition() {
    let wasm_path = lua_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Lua template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get Lua node definition");

    assert_eq!(definition.name, "my_custom_node_lua");
    assert!(!definition.pins.is_empty());

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
    assert!(pin_names.contains(&"char_count"));
}

#[tokio::test]
async fn test_lua_template_execute() {
    let wasm_path = lua_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Lua template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Lua"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute Lua node");

    assert!(
        result.error.is_none(),
        "Lua execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "LuaLuaLua"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        9
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Swift Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_swift_template() {
    let wasm_path = swift_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Swift template not built. Run: cd templates/wasm-node-swift && swift build --triple wasm32-unknown-wasi -c release");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load Swift WASM module");

    assert!(module.has_alloc(), "Swift module should export alloc");
}

#[tokio::test]
async fn test_swift_template_get_definition() {
    let wasm_path = swift_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Swift template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = match WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string()).await {
        Ok(m) => Arc::new(m),
        Err(_) => {
            eprintln!("Skipping: Swift wasm module not compatible");
            return;
        }
    };

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get Swift node definition");

    assert_eq!(definition.name, "my_custom_node_swift");
    assert!(!definition.pins.is_empty());

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
    assert!(pin_names.contains(&"char_count"));
}

#[tokio::test]
async fn test_swift_template_execute() {
    let wasm_path = swift_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Swift template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = match WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string()).await {
        Ok(m) => Arc::new(m),
        Err(_) => {
            eprintln!("Skipping: Swift wasm module not compatible");
            return;
        }
    };

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Sw"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute Swift node");

    assert!(
        result.error.is_none(),
        "Swift execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "SwSwSw"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        6
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Java (TeaVM) Template Tests
// ============================================================================

#[tokio::test]
async fn test_load_java_template() {
    let wasm_path = java_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Java template not built. Run: cd templates/wasm-node-java && mvn package");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
        .await
        .expect("Failed to load Java WASM module");

    assert!(module.has_alloc(), "Java module should export alloc");
}

#[tokio::test]
async fn test_java_template_get_definition() {
    let wasm_path = java_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Java template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = match WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string()).await {
        Ok(m) => Arc::new(m),
        Err(_) => {
            eprintln!("Skipping: Java wasm module not compatible");
            return;
        }
    };

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let definition = instance
        .call_get_node()
        .await
        .expect("Failed to get Java node definition");

    assert_eq!(definition.name, "my_custom_node_java");
    assert!(!definition.pins.is_empty());

    let pin_names: Vec<&str> = definition.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(pin_names.contains(&"exec"));
    assert!(pin_names.contains(&"input_text"));
    assert!(pin_names.contains(&"multiplier"));
    assert!(pin_names.contains(&"exec_out"));
    assert!(pin_names.contains(&"output_text"));
    assert!(pin_names.contains(&"char_count"));
}

#[tokio::test]
async fn test_java_template_execute() {
    let wasm_path = java_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Java template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");
    let module = match WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string()).await {
        Ok(m) => Arc::new(m),
        Err(_) => {
            eprintln!("Skipping: Java wasm module not compatible");
            return;
        }
    };

    let mut instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive())
        .await
        .expect("Failed to create instance");

    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Java"));
    inputs.insert("multiplier".to_string(), serde_json::json!(3));

    let exec_input = create_execution_input(inputs);
    let result = instance
        .call_run(&exec_input)
        .await
        .expect("Failed to execute Java node");

    assert!(
        result.error.is_none(),
        "Java execution error: {:?}",
        result.error
    );
    assert_eq!(
        result.outputs.get("output_text").unwrap().as_str().unwrap(),
        "JavaJavaJava"
    );
    assert_eq!(
        result.outputs.get("char_count").unwrap().as_i64().unwrap(),
        12
    );
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

// ============================================================================
// Security and Limits Tests
// ============================================================================

#[tokio::test]
async fn test_fuel_consumption() {
    let wasm_path = rust_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");

    // Create engine with fuel metering
    let mut config = WasmConfig::default();
    config.fuel_metering = true;
    let engine = WasmEngine::new(config).expect("Failed to create engine");

    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut security = WasmSecurityConfig::permissive();
    security.limits.fuel_limit = 10_000_000; // Generous limit

    let mut instance = WasmInstance::new(&engine, module, security)
        .await
        .expect("Failed to create instance");

    // Execute should succeed with enough fuel
    let mut inputs = serde_json::Map::new();
    inputs.insert("input_text".to_string(), serde_json::json!("Test"));
    inputs.insert("multiplier".to_string(), serde_json::json!(1));

    let exec_input = create_execution_input(inputs);
    let result = instance.call_run(&exec_input).await;

    assert!(result.is_ok(), "Execution should succeed with enough fuel");
}

#[tokio::test]
async fn test_memory_limits() {
    let wasm_path = rust_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust template not built");
        return;
    }

    let bytes = tokio::fs::read(&wasm_path)
        .await
        .expect("Failed to read WASM file");
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");

    let module = Arc::new(
        WasmModule::from_bytes(&engine, &bytes, "test_hash".to_string())
            .await
            .expect("Failed to load module"),
    );

    let mut security = WasmSecurityConfig::permissive();
    security.limits.memory_limit = 10 * 1024 * 1024; // 10MB limit

    let instance = WasmInstance::new(&engine, module, security).await;

    // Instance creation should succeed with reasonable memory limits
    assert!(instance.is_ok(), "Instance creation should succeed");
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[tokio::test]
async fn test_invalid_wasm_bytes() {
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");

    // Try to load invalid bytes
    let invalid_bytes = vec![0x00, 0x61, 0x73, 0x6D]; // Incomplete WASM header
    let result = WasmModule::from_bytes(&engine, &invalid_bytes, "test_hash".to_string()).await;

    assert!(result.is_err(), "Loading invalid WASM should fail");
}

#[tokio::test]
async fn test_module_without_required_exports() {
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");

    // A minimal valid WASM module without required exports
    // This is a valid WASM module with just a memory export
    let minimal_wasm = wat::parse_str(
        r#"
        (module
            (memory (export "memory") 1)
        )
    "#,
    )
    .expect("Failed to parse WAT");

    let result = WasmModule::from_bytes(&engine, &minimal_wasm, "test_hash".to_string()).await;

    assert!(
        result.is_err(),
        "Module without get_node should fail to load"
    );

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("get_node")
            || err_str.contains("get_nodes")
            || err_str.contains("MissingExport"),
        "Error should mention missing export: {}",
        err_str
    );
}
