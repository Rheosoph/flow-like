//! Exercises the SDK object registry through the compiled Rust template.

#![cfg(feature = "component-model")]

extern crate flow_like_runtime as flow_like;

use flow_like::flow::execution::resources::RunResources;
use flow_like_wasm::abi::{WasmExecutionInput, WasmExecutionResult};
use flow_like_wasm::host_functions::HostState;
use flow_like_wasm::package_runtime::{package_runtime_key, PackageRuntime};
use flow_like_wasm::{LoadedWasm, WasmConfig, WasmEngine, WasmSecurityConfig};
use serde_json::{json, Value};
use std::sync::Arc;

fn package(
    run: &RunResources,
    package_id: &str,
    loaded: &LoadedWasm,
    security: &WasmSecurityConfig,
) -> Arc<PackageRuntime> {
    let key = package_runtime_key(package_id, loaded.hash(), security, "user", false).unwrap();
    run.get_or_insert_with(key, || Arc::new(PackageRuntime::default()))
        .unwrap()
}

async fn call(
    runtime: &PackageRuntime,
    loaded: &LoadedWasm,
    engine: &WasmEngine,
    security: &WasmSecurityConfig,
    node: &str,
    inputs: Value,
) -> WasmExecutionResult {
    runtime
        .call(
            loaded,
            engine,
            security,
            HostState::with_security(security),
            &WasmExecutionInput {
                inputs: inputs.as_object().unwrap().clone(),
                node_id: node.to_string(),
                node_name: node.to_string(),
                // Deliberately reused across every RunResources registry. Handle
                // isolation must follow live ownership, not the printable run ID.
                run_id: "same-run-id".into(),
                app_id: "app".into(),
                board_id: "board".into(),
                user_id: "user".into(),
                stream_state: false,
                log_level: 1,
            },
        )
        .await
        .unwrap()
        .result
}

#[tokio::test]
#[ignore = "Build the Rust template with cargo build --target wasm32-wasip2 before running"]
async fn arbitrary_objects_survive_node_calls_but_handles_cannot_cross_instances_or_runs() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/wasm-node-rust/target/wasm32-wasip2/debug/flow_like_wasm_node_template.wasm");
    let engine = WasmEngine::new(WasmConfig::development()).unwrap();
    let loaded = engine
        .load_auto_from_file(&path)
        .await
        .expect("build the Rust template first");
    // The registry needs no cache or network permissions.
    let security = WasmSecurityConfig::from_node_permissions(&[]);
    let run = RunResources::default();
    let owner = package(&run, "objects", &loaded, &security);

    let created = call(
        &owner,
        &loaded,
        &engine,
        &security,
        "object_create_buffer",
        json!({"initial_text": "Hello"}),
    )
    .await;
    assert!(created.error.is_none(), "{:?}", created.error);
    let handle = created.outputs["handle"].as_str().unwrap().to_owned();
    let created = call(
        &owner,
        &loaded,
        &engine,
        &security,
        "object_create_buffer",
        json!({"initial_text": "independent"}),
    )
    .await;
    assert!(created.error.is_none(), "{:?}", created.error);
    let other = created.outputs["handle"].as_str().unwrap().to_owned();
    assert_ne!(handle, other);

    let appended = call(
        &owner,
        &loaded,
        &engine,
        &security,
        "object_append_buffer",
        json!({"handle": handle, "text": " 🌍"}),
    )
    .await;
    assert!(appended.error.is_none(), "{:?}", appended.error);
    assert_eq!(appended.outputs["byte_len"], json!(10));
    let read = call(
        &owner,
        &loaded,
        &engine,
        &security,
        "object_read_buffer",
        json!({"handle": handle}),
    )
    .await;
    assert!(read.error.is_none(), "{:?}", read.error);
    assert_eq!(read.outputs["text"], json!("Hello 🌍"));

    let next_run = RunResources::default();
    for isolated in [
        package(&run, "different-package", &loaded, &security),
        package(&next_run, "objects", &loaded, &security),
    ] {
        // Populate the second instance before replaying the first handle. A
        // registry whose counter restarts at zero would alias this new object.
        let created = call(
            &isolated,
            &loaded,
            &engine,
            &security,
            "object_create_buffer",
            json!({"initial_text": "private to this instance"}),
        )
        .await;
        assert!(created.error.is_none(), "{:?}", created.error);
        assert_ne!(created.outputs["handle"], json!(handle));
        let denied = call(
            &isolated,
            &loaded,
            &engine,
            &security,
            "object_read_buffer",
            json!({"handle": handle}),
        )
        .await;
        assert!(denied.error.is_some());
    }

    let closed = call(
        &owner,
        &loaded,
        &engine,
        &security,
        "object_close_buffer",
        json!({"handle": handle}),
    )
    .await;
    assert!(closed.error.is_none(), "{:?}", closed.error);
    for node in ["object_read_buffer", "object_close_buffer"] {
        let denied = call(
            &owner,
            &loaded,
            &engine,
            &security,
            node,
            json!({"handle": handle}),
        )
        .await;
        assert!(denied.error.is_some());
    }
    let read = call(
        &owner,
        &loaded,
        &engine,
        &security,
        "object_read_buffer",
        json!({"handle": other}),
    )
    .await;
    assert!(read.error.is_none(), "{:?}", read.error);
    assert_eq!(read.outputs["text"], json!("independent"));

    // Leave the second object live. The run owns and disposes of its instance.
    let weak = Arc::downgrade(&owner);
    drop(owner);
    run.shutdown().await;
    assert!(weak.upgrade().is_none());
    let fresh = package(&next_run, "objects", &loaded, &security);
    let denied = call(
        &fresh,
        &loaded,
        &engine,
        &security,
        "object_read_buffer",
        json!({"handle": other}),
    )
    .await;
    assert!(denied.error.is_some());
    next_run.shutdown().await;
}
