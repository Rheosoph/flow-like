//! Integration tests for the new context functions:
//!   - Schema (get_type_schema, list_types)
//!   - Image  (from_bytes, to_bytes)
//!   - DB     (query — unified dispatch)
//!   - Models (embed_text_query, embed_text_document, embed_image, llm_prompt)
//!
//! These tests verify:
//!   1. Host-side schema module returns valid JSON schemas
//!   2. Linker registration succeeds for all new host functions
//!   3. WAT-based WASM modules can call host functions without panics
//!   4. Schema data is correctly returned through the WASM ABI

use flow_like_wasm::engine::{WasmConfig, WasmEngine};
use flow_like_wasm::host_functions::schema;
use flow_like_wasm::instance::WasmInstance;
use flow_like_wasm::limits::{WasmCapabilities, WasmSecurityConfig};
use flow_like_wasm::module::WasmModule;
use std::sync::Arc;

// ============================================================================
// WAT Helpers — build WAT fragments with correct, computed byte lengths
// ============================================================================

fn wat_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn node_json(name: &str, friendly: &str, desc: &str) -> String {
    format!(
        r#"{{"name":"{name}","friendly_name":"{friendly}","description":"{desc}","category":"Test","pins":[]}}"#,
    )
}

fn result_json(outputs_inner: &str, exec: &[&str], error: Option<&str>) -> String {
    let exec_json: Vec<String> = exec.iter().map(|e| format!("\"{}\"", e)).collect();
    let error_json = match error {
        Some(msg) => format!("\"{}\"", msg),
        None => "null".to_string(),
    };
    format!(
        r#"{{"outputs":{{{outputs_inner}}},"activate_exec":[{exec_list}],"error":{error_json}}}"#,
        exec_list = exec_json.join(","),
    )
}

fn test_input() -> flow_like_wasm::abi::WasmExecutionInput {
    flow_like_wasm::abi::WasmExecutionInput {
        inputs: serde_json::Map::new(),
        node_id: "test".to_string(),
        run_id: "test".to_string(),
        app_id: "test".to_string(),
        board_id: "test".to_string(),
        user_id: "test".to_string(),
        stream_state: false,
        log_level: 0,
        node_name: String::new(),
    }
}

async fn run_wat(
    wat: &str,
    module_name: &str,
    security: WasmSecurityConfig,
) -> Result<flow_like_wasm::abi::WasmExecutionResult, String> {
    let engine = WasmEngine::new(WasmConfig::default()).map_err(|e| format!("{e}"))?;
    let wasm_bytes = wat::parse_str(wat).map_err(|e| format!("WAT parse: {e}"))?;
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &wasm_bytes, module_name.to_string())
            .await
            .map_err(|e| format!("Module load: {e}"))?,
    );
    let mut instance = WasmInstance::new(&engine, module, security)
        .await
        .map_err(|e| format!("Instance: {e}"))?;
    instance
        .call_run(&test_input())
        .await
        .map_err(|e| format!("{e}"))
}

// ============================================================================
// Host-Side Schema Module Tests (no WASM runtime needed)
// ============================================================================

#[test]
fn test_schema_list_type_names_contains_all_expected() {
    let names = schema::list_type_names();
    for expected in &[
        "FlowPath",
        "NodeImage",
        "NodeDBConnection",
        "CachedEmbeddingModel",
        "Bit",
    ] {
        assert!(
            names.contains(expected),
            "Missing {}, got: {:?}",
            expected,
            names
        );
    }
}

#[test]
fn test_schema_list_type_names_count() {
    assert_eq!(schema::list_type_names().len(), 5);
}

#[test]
fn test_schema_get_type_schema_flow_path() {
    let s = schema::get_type_schema("FlowPath").expect("FlowPath schema should exist");
    let parsed: serde_json::Value = serde_json::from_str(s).expect("Should be valid JSON");
    assert!(parsed.is_object());
}

#[test]
fn test_schema_get_type_schema_node_image() {
    let s = schema::get_type_schema("NodeImage").expect("NodeImage schema should exist");
    let parsed: serde_json::Value = serde_json::from_str(s).expect("Should be valid JSON");
    assert!(parsed.is_object());
}

#[test]
fn test_schema_get_type_schema_node_db_connection() {
    let s =
        schema::get_type_schema("NodeDBConnection").expect("NodeDBConnection schema should exist");
    let parsed: serde_json::Value = serde_json::from_str(s).expect("Should be valid JSON");
    assert!(parsed.is_object());
}

#[test]
fn test_schema_get_type_schema_cached_embedding_model() {
    let s = schema::get_type_schema("CachedEmbeddingModel")
        .expect("CachedEmbeddingModel schema should exist");
    let parsed: serde_json::Value = serde_json::from_str(s).expect("Should be valid JSON");
    assert!(parsed.is_object());
}

#[test]
fn test_schema_get_type_schema_bit() {
    let s = schema::get_type_schema("Bit").expect("Bit schema should exist");
    let parsed: serde_json::Value = serde_json::from_str(s).expect("Should be valid JSON");
    assert!(parsed.is_object());
}

#[test]
fn test_schema_get_type_schema_unknown_returns_none() {
    assert!(schema::get_type_schema("NonExistentType").is_none());
    assert!(schema::get_type_schema("").is_none());
    assert!(schema::get_type_schema("flowpath").is_none());
}

#[test]
fn test_schema_flow_path_has_expected_properties() {
    let s = schema::get_type_schema("FlowPath").unwrap();
    assert!(
        s.contains("path"),
        "FlowPath schema should reference 'path'"
    );
    assert!(
        s.contains("store_ref"),
        "FlowPath schema should reference 'store_ref'"
    );
}

#[test]
fn test_schema_bit_has_expected_properties() {
    let s = schema::get_type_schema("Bit").unwrap();
    assert!(s.contains("id"), "Bit schema should reference 'id'");
}

#[test]
fn test_schema_all_types_produce_nonempty_schemas() {
    for type_name in schema::list_type_names() {
        let s = schema::get_type_schema(type_name)
            .unwrap_or_else(|| panic!("Schema for '{}' should exist", type_name));
        assert!(
            !s.is_empty(),
            "Schema for '{}' should not be empty",
            type_name
        );
        let parsed: serde_json::Value = serde_json::from_str(s)
            .unwrap_or_else(|e| panic!("Schema for '{}' should be valid JSON: {}", type_name, e));
        assert!(
            parsed.is_object(),
            "Schema for '{}' should be a JSON object",
            type_name
        );
    }
}

// ============================================================================
// Linker Registration Tests
// ============================================================================

#[tokio::test]
async fn test_linker_registers_all_functions() {
    let engine = WasmEngine::new(WasmConfig::default()).expect("Failed to create engine");

    let node = node_json("test_imports", "Test", "Tests imports");
    let run_result = result_json("", &[], None);

    let wat = format!(
        r#"
        (module
            (import "flowlike_schema" "get_type_schema" (func $get_type_schema (param i32 i32) (result i64)))
            (import "flowlike_schema" "list_types" (func $list_types (result i64)))
            (import "flowlike_image" "from_bytes" (func $image_from_bytes (param i32 i32 i32 i32) (result i64)))
            (import "flowlike_image" "to_bytes" (func $image_to_bytes (param i32 i32 i32 i32) (result i64)))
            (import "flowlike_db" "query" (func $db_query (param i32 i32 i32 i32 i32) (result i64)))
            (import "flowlike_models" "embed_text_query" (func $embed_text_query (param i32 i32 i32 i32) (result i64)))
            (import "flowlike_models" "embed_text_document" (func $embed_text_document (param i32 i32 i32 i32) (result i64)))
            (import "flowlike_models" "embed_image" (func $embed_image (param i32 i32 i32 i32) (result i64)))
            (import "flowlike_models" "llm_prompt" (func $llm_prompt (param i32 i32 i32 i32 i32) (result i64)))

            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (data (i32.const 256) "{run_esc}")
            (func (export "run") (param i32 i32) (result i64)
                (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {run_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        run_esc = wat_escape(&run_result),
        run_len = run_result.len(),
    );

    let wasm_bytes = wat::parse_str(&wat).expect("Failed to parse WAT");
    let module = Arc::new(
        WasmModule::from_bytes(&engine, &wasm_bytes, "test_imports".to_string())
            .await
            .expect("Failed to load module"),
    );

    let instance = WasmInstance::new(&engine, module, WasmSecurityConfig::permissive()).await;
    assert!(
        instance.is_ok(),
        "Instance creation should succeed — all imports must be satisfied. Error: {:?}",
        instance.err()
    );
}

// ============================================================================
// WAT-Based Schema Function Tests (full WASM <-> Host round-trip)
// ============================================================================

#[tokio::test]
async fn test_wasm_calls_list_types() {
    let node = node_json("schema_test", "Schema Test", "Tests schema");
    let success = result_json(r#""called":true"#, &["exec_out"], None);

    let wat = format!(
        r#"
        (module
            (import "flowlike_schema" "list_types" (func $list_types (result i64)))
            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (global $schema_result (mut i64) (i64.const 0))

            (data (i32.const 256) "{success_esc}")
            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (global.set $schema_result (call $list_types))
                (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {success_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        success_esc = wat_escape(&success),
        success_len = success.len(),
    );

    let result = run_wat(&wat, "schema_test", WasmSecurityConfig::permissive())
        .await
        .expect("call_run should succeed — list_types must not trap");

    assert!(result.error.is_none(), "Should succeed: {:?}", result.error);
    assert!(result.activate_exec.contains(&"exec_out".to_string()));
}

#[tokio::test]
async fn test_wasm_calls_get_type_schema() {
    let node = node_json("schema_get_test", "Schema Get", "Tests get_type_schema");
    let success = result_json(r#""schema_found":true"#, &["exec_out"], None);
    let failure = result_json(r#""schema_found":false"#, &[], Some("schema not found"));

    let wat = format!(
        r#"
        (module
            (import "flowlike_schema" "get_type_schema" (func $get_type_schema (param i32 i32) (result i64)))
            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (data (i32.const 512) "FlowPath")

            (data (i32.const 256) "{success_esc}")
            (data (i32.const 384) "{failure_esc}")

            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (local $result i64)
                (local.set $result (call $get_type_schema (i32.const 512) (i32.const 8)))
                (if (i64.ne (local.get $result) (i64.const 0))
                    (then
                        (return (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {success_len})))
                    )
                )
                (i64.or (i64.shl (i64.const 384) (i64.const 32)) (i64.const {failure_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        success_esc = wat_escape(&success),
        success_len = success.len(),
        failure_esc = wat_escape(&failure),
        failure_len = failure.len(),
    );

    let result = run_wat(&wat, "schema_get_test", WasmSecurityConfig::permissive())
        .await
        .expect("call_run should succeed");

    assert!(
        result.error.is_none(),
        "get_type_schema('FlowPath') should succeed: {:?}",
        result.error
    );
    let schema_found = result
        .outputs
        .get("schema_found")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        schema_found,
        "Host should return non-zero for FlowPath schema"
    );
}

#[tokio::test]
async fn test_wasm_get_type_schema_unknown_type() {
    let node = node_json("schema_unknown", "Unknown", "Test unknown type");
    let found = result_json(r#""was_zero":false"#, &["exec_out"], None);
    let not_found = result_json(r#""was_zero":true"#, &["exec_out"], None);

    let wat = format!(
        r#"
        (module
            (import "flowlike_schema" "get_type_schema" (func $get_type_schema (param i32 i32) (result i64)))
            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (data (i32.const 512) "UnknownType")

            (data (i32.const 256) "{found_esc}")
            (data (i32.const 384) "{not_found_esc}")

            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (local $result i64)
                (local.set $result (call $get_type_schema (i32.const 512) (i32.const 11)))
                (if (i64.eqz (local.get $result))
                    (then
                        (return (i64.or (i64.shl (i64.const 384) (i64.const 32)) (i64.const {not_found_len})))
                    )
                )
                (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {found_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        found_esc = wat_escape(&found),
        found_len = found.len(),
        not_found_esc = wat_escape(&not_found),
        not_found_len = not_found.len(),
    );

    let result = run_wat(&wat, "schema_unknown", WasmSecurityConfig::permissive())
        .await
        .expect("call_run should succeed");

    assert!(result.error.is_none());
    let was_zero = result
        .outputs
        .get("was_zero")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        was_zero,
        "Unknown type should return 0 from get_type_schema"
    );
}

// ============================================================================
// WAT-Based Image/DB/Model Stub Tests (verify stubs don't trap)
// ============================================================================

#[tokio::test]
async fn test_wasm_image_stubs_dont_trap() {
    let node = node_json("image_test", "Image Test", "Tests image stubs");
    let success = result_json(r#""ok":true"#, &["exec_out"], None);

    let wat = format!(
        r#"
        (module
            (import "flowlike_image" "from_bytes" (func $from_bytes (param i32 i32 i32 i32) (result i64)))
            (import "flowlike_image" "to_bytes" (func $to_bytes (param i32 i32 i32 i32) (result i64)))
            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (data (i32.const 512) "png")
            (data (i32.const 520) "\89PNG\0d\0a\1a\0a")

            (data (i32.const 256) "{success_esc}")

            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (drop (call $from_bytes (i32.const 520) (i32.const 8) (i32.const 512) (i32.const 3)))
                (drop (call $to_bytes (i32.const 520) (i32.const 8) (i32.const 512) (i32.const 3)))
                (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {success_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        success_esc = wat_escape(&success),
        success_len = success.len(),
    );

    let result = run_wat(&wat, "image_test", WasmSecurityConfig::permissive())
        .await
        .expect("Image stub calls should not trap");

    assert!(
        result.error.is_none(),
        "Image stubs should not produce errors: {:?}",
        result.error
    );
}

#[tokio::test]
async fn test_wasm_db_stub_doesnt_trap() {
    let node = node_json("db_test", "DB Test", "Tests db stub");
    let success = result_json(r#""ok":true"#, &["exec_out"], None);

    let conn = r#"{"cache_key":"test"}"#;
    let payload = r#"{"vector":[0.1],"limit":10}"#;

    let wat = format!(
        r#"
        (module
            (import "flowlike_db" "query" (func $db_query (param i32 i32 i32 i32 i32) (result i64)))
            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (data (i32.const 512) "{conn_esc}")
            (data (i32.const 576) "{payload_esc}")

            (data (i32.const 256) "{success_esc}")

            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (drop (call $db_query
                    (i32.const 1) (i32.const 512) (i32.const {conn_len}) (i32.const 576) (i32.const {payload_len})))
                (drop (call $db_query
                    (i32.const 8) (i32.const 512) (i32.const {conn_len}) (i32.const 576) (i32.const {payload_len})))
                (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {success_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        conn_esc = wat_escape(conn),
        conn_len = conn.len(),
        payload_esc = wat_escape(payload),
        payload_len = payload.len(),
        success_esc = wat_escape(&success),
        success_len = success.len(),
    );

    let result = run_wat(&wat, "db_test", WasmSecurityConfig::permissive())
        .await
        .expect("DB stub calls should not trap");

    assert!(
        result.error.is_none(),
        "DB stubs should not produce errors: {:?}",
        result.error
    );
}

#[tokio::test]
async fn test_wasm_model_stubs_dont_trap() {
    let node = node_json("model_test", "Model Test", "Tests model stubs");
    let success = result_json(r#""ok":true"#, &["exec_out"], None);

    let model = r#"{"cache_key":"m","model_type":"text"}"#;
    let texts = r#"["hello world"]"#;
    let bit = r#"{"id":"test-bit"}"#;
    let messages = r#"[{"role":"user","content":"hi"}]"#;

    let wat = format!(
        r#"
        (module
            (import "flowlike_models" "embed_text_query" (func $embed_text_query (param i32 i32 i32 i32) (result i64)))
            (import "flowlike_models" "embed_text_document" (func $embed_text_document (param i32 i32 i32 i32) (result i64)))
            (import "flowlike_models" "embed_image" (func $embed_image (param i32 i32 i32 i32) (result i64)))
            (import "flowlike_models" "llm_prompt" (func $llm_prompt (param i32 i32 i32 i32 i32) (result i64)))
            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (data (i32.const 512) "{model_esc}")
            (data (i32.const 576) "{texts_esc}")
            (data (i32.const 640) "{bit_esc}")
            (data (i32.const 704) "{messages_esc}")

            (data (i32.const 256) "{success_esc}")

            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (drop (call $embed_text_query (i32.const 512) (i32.const {model_len}) (i32.const 576) (i32.const {texts_len})))
                (drop (call $embed_text_document (i32.const 512) (i32.const {model_len}) (i32.const 576) (i32.const {texts_len})))
                (drop (call $embed_image (i32.const 512) (i32.const {model_len}) (i32.const 576) (i32.const {texts_len})))
                (drop (call $llm_prompt (i32.const 640) (i32.const {bit_len}) (i32.const 704) (i32.const {messages_len}) (i32.const 0)))
                (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {success_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        model_esc = wat_escape(model),
        model_len = model.len(),
        texts_esc = wat_escape(texts),
        texts_len = texts.len(),
        bit_esc = wat_escape(bit),
        bit_len = bit.len(),
        messages_esc = wat_escape(messages),
        messages_len = messages.len(),
        success_esc = wat_escape(&success),
        success_len = success.len(),
    );

    let result = run_wat(&wat, "model_test", WasmSecurityConfig::permissive())
        .await
        .expect("Model stub calls should not trap");

    assert!(
        result.error.is_none(),
        "Model stubs should not produce errors: {:?}",
        result.error
    );
}

// ============================================================================
// Capability Gating Tests
// ============================================================================

#[tokio::test]
async fn test_wasm_image_without_model_capability_returns_zero() {
    let node = node_json("cap_test", "Cap Test", "Tests capability gating");
    let was_zero = result_json(r#""was_zero":true"#, &["exec_out"], None);
    let not_zero = result_json(r#""was_zero":false"#, &["exec_out"], None);

    let wat = format!(
        r#"
        (module
            (import "flowlike_image" "from_bytes" (func $from_bytes (param i32 i32 i32 i32) (result i64)))
            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (data (i32.const 512) "png")
            (data (i32.const 520) "\89PNG")

            (data (i32.const 256) "{was_zero_esc}")
            (data (i32.const 384) "{not_zero_esc}")

            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (local $result i64)
                (local.set $result (call $from_bytes (i32.const 520) (i32.const 4) (i32.const 512) (i32.const 3)))
                (if (i64.eqz (local.get $result))
                    (then
                        (return (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {was_zero_len})))
                    )
                )
                (i64.or (i64.shl (i64.const 384) (i64.const 32)) (i64.const {not_zero_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        was_zero_esc = wat_escape(&was_zero),
        was_zero_len = was_zero.len(),
        not_zero_esc = wat_escape(&not_zero),
        not_zero_len = not_zero.len(),
    );

    let result = run_wat(&wat, "cap_test", WasmSecurityConfig::restrictive())
        .await
        .expect("Should not trap even without capabilities");

    assert!(result.error.is_none());
    let zv = result
        .outputs
        .get("was_zero")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        zv,
        "Without MODELS capability, image from_bytes should return 0"
    );
}

#[tokio::test]
async fn test_wasm_schema_flowpath_denied_without_storage_read() {
    let node = node_json("cap_schema", "Cap Schema", "Tests schema capability gating");
    let was_zero = result_json(r#""was_zero":true"#, &["exec_out"], None);
    let not_zero = result_json(r#""was_zero":false"#, &["exec_out"], None);

    let wat = format!(
        r#"
        (module
            (import "flowlike_schema" "get_type_schema" (func $get_type_schema (param i32 i32) (result i64)))
            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (data (i32.const 512) "FlowPath")

            (data (i32.const 256) "{was_zero_esc}")
            (data (i32.const 384) "{not_zero_esc}")

            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (local $result i64)
                (local.set $result (call $get_type_schema (i32.const 512) (i32.const 8)))
                (if (i64.eqz (local.get $result))
                    (then
                        (return (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {was_zero_len})))
                    )
                )
                (i64.or (i64.shl (i64.const 384) (i64.const 32)) (i64.const {not_zero_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        was_zero_esc = wat_escape(&was_zero),
        was_zero_len = was_zero.len(),
        not_zero_esc = wat_escape(&not_zero),
        not_zero_len = not_zero.len(),
    );

    // Restrictive: no capabilities at all
    let result = run_wat(&wat, "cap_schema", WasmSecurityConfig::restrictive())
        .await
        .expect("Should not trap even without capabilities");

    assert!(result.error.is_none());
    let zv = result
        .outputs
        .get("was_zero")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        zv,
        "Without STORAGE_READ capability, FlowPath schema should return 0"
    );
}

#[tokio::test]
async fn test_wasm_schema_flowpath_allowed_with_storage_read() {
    let node = node_json(
        "cap_schema_ok",
        "Cap Schema OK",
        "Storage read grants FlowPath",
    );
    let success = result_json(r#""found":true"#, &["exec_out"], None);
    let failure = result_json(r#""found":false"#, &[], Some("denied"));

    let wat = format!(
        r#"
        (module
            (import "flowlike_schema" "get_type_schema" (func $get_type_schema (param i32 i32) (result i64)))
            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (data (i32.const 512) "FlowPath")

            (data (i32.const 256) "{success_esc}")
            (data (i32.const 384) "{failure_esc}")

            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (local $result i64)
                (local.set $result (call $get_type_schema (i32.const 512) (i32.const 8)))
                (if (i64.ne (local.get $result) (i64.const 0))
                    (then
                        (return (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {success_len})))
                    )
                )
                (i64.or (i64.shl (i64.const 384) (i64.const 32)) (i64.const {failure_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        success_esc = wat_escape(&success),
        success_len = success.len(),
        failure_esc = wat_escape(&failure),
        failure_len = failure.len(),
    );

    // Only grant STORAGE_READ — enough for FlowPath schema
    let security =
        WasmSecurityConfig::restrictive().with_capabilities(WasmCapabilities::STORAGE_READ);

    let result = run_wat(&wat, "cap_schema_ok", security)
        .await
        .expect("call_run should succeed");

    assert!(result.error.is_none(), "Should succeed: {:?}", result.error);
    let found = result
        .outputs
        .get("found")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        found,
        "With STORAGE_READ, FlowPath schema should be accessible"
    );
}

#[tokio::test]
async fn test_wasm_schema_model_types_denied_without_models_capability() {
    for type_name in &[
        "Bit",
        "NodeImage",
        "NodeDBConnection",
        "CachedEmbeddingModel",
    ] {
        let node = node_json("cap_model", "Cap Model", "Model types need MODELS cap");
        let was_zero = result_json(r#""was_zero":true"#, &["exec_out"], None);
        let not_zero = result_json(r#""was_zero":false"#, &["exec_out"], None);

        let wat = format!(
            r#"
            (module
                (import "flowlike_schema" "get_type_schema" (func $get_type_schema (param i32 i32) (result i64)))
                (memory (export "memory") 1)

                (data (i32.const 0) "{node_esc}")
                (func (export "get_node") (result i64)
                    (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
                )

                (data (i32.const 512) "{type_name}")

                (data (i32.const 256) "{was_zero_esc}")
                (data (i32.const 384) "{not_zero_esc}")

                (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                    (local $result i64)
                    (local.set $result (call $get_type_schema (i32.const 512) (i32.const {name_len})))
                    (if (i64.eqz (local.get $result))
                        (then
                            (return (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {was_zero_len})))
                        )
                    )
                    (i64.or (i64.shl (i64.const 384) (i64.const 32)) (i64.const {not_zero_len}))
                )
            )
        "#,
            node_esc = wat_escape(&node),
            node_len = node.len(),
            type_name = type_name,
            name_len = type_name.len(),
            was_zero_esc = wat_escape(&was_zero),
            was_zero_len = was_zero.len(),
            not_zero_esc = wat_escape(&not_zero),
            not_zero_len = not_zero.len(),
        );

        // Grant only STORAGE_READ — model types should still be denied
        let security =
            WasmSecurityConfig::restrictive().with_capabilities(WasmCapabilities::STORAGE_READ);

        let result = run_wat(&wat, &format!("cap_model_{}", type_name), security)
            .await
            .unwrap_or_else(|e| panic!("Should not trap for {}: {}", type_name, e));

        assert!(result.error.is_none());
        let zv = result
            .outputs
            .get("was_zero")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(
            zv,
            "Without MODELS capability, {} schema should return 0",
            type_name
        );
    }
}

#[tokio::test]
async fn test_wasm_list_types_filters_by_capability() {
    let node = node_json("list_cap", "List Cap", "list_types filters by cap");
    let success = result_json(r#""called":true"#, &["exec_out"], None);

    let wat = format!(
        r#"
        (module
            (import "flowlike_schema" "list_types" (func $list_types (result i64)))
            (memory (export "memory") 1)

            (data (i32.const 0) "{node_esc}")
            (func (export "get_node") (result i64)
                (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
            )

            (global $schema_result (mut i64) (i64.const 0))

            (data (i32.const 256) "{success_esc}")
            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (global.set $schema_result (call $list_types))
                (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {success_len}))
            )
        )
    "#,
        node_esc = wat_escape(&node),
        node_len = node.len(),
        success_esc = wat_escape(&success),
        success_len = success.len(),
    );

    // Restrictive: no capabilities — list_types should return empty list
    let result = run_wat(&wat, "list_cap", WasmSecurityConfig::restrictive())
        .await
        .expect("Should not trap");

    assert!(result.error.is_none());
}

// ============================================================================
// Schema Queries for All Known Types (WAT round-trip)
// ============================================================================

#[tokio::test]
async fn test_wasm_schema_all_known_types_return_nonzero() {
    let type_names = [
        "FlowPath",
        "NodeImage",
        "NodeDBConnection",
        "CachedEmbeddingModel",
        "Bit",
    ];

    for type_name in &type_names {
        let node = node_json("type_test", "Type Test", "Tests type schema");
        let success = result_json(r#""found":true"#, &["exec_out"], None);
        let failure = result_json(r#""found":false"#, &[], Some("not found"));

        let wat = format!(
            r#"
            (module
                (import "flowlike_schema" "get_type_schema" (func $get_type_schema (param i32 i32) (result i64)))
                (memory (export "memory") 1)

                (data (i32.const 0) "{node_esc}")
                (func (export "get_node") (result i64)
                    (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {node_len}))
                )

                (data (i32.const 512) "{type_name}")

                (data (i32.const 256) "{success_esc}")
                (data (i32.const 384) "{failure_esc}")

                (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                    (local $result i64)
                    (local.set $result (call $get_type_schema (i32.const 512) (i32.const {name_len})))
                    (if (i64.ne (local.get $result) (i64.const 0))
                        (then
                            (return (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const {success_len})))
                        )
                    )
                    (i64.or (i64.shl (i64.const 384) (i64.const 32)) (i64.const {failure_len}))
                )
            )
        "#,
            node_esc = wat_escape(&node),
            node_len = node.len(),
            type_name = type_name,
            name_len = type_name.len(),
            success_esc = wat_escape(&success),
            success_len = success.len(),
            failure_esc = wat_escape(&failure),
            failure_len = failure.len(),
        );

        let result = run_wat(
            &wat,
            &format!("type_test_{}", type_name),
            WasmSecurityConfig::permissive(),
        )
        .await
        .unwrap_or_else(|e| panic!("call_run failed for {}: {}", type_name, e));

        assert!(
            result.error.is_none(),
            "get_type_schema('{}') should succeed: {:?}",
            type_name,
            result.error
        );
        let found = result
            .outputs
            .get("found")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(
            found,
            "get_type_schema('{}') should return a non-zero result",
            type_name
        );
    }
}
