//! Exercises the SDK object registry through the compiled Rust template.

#![cfg(feature = "component-model")]

extern crate flow_like_runtime as flow_like;

use flow_like::flow::execution::resources::RunResources;
use flow_like::flow::node::NodePermission;
use flow_like_wasm::abi::{WasmExecutionInput, WasmExecutionResult};
use flow_like_wasm::host_functions::HostState;
use flow_like_wasm::package_runtime::{package_runtime_key, PackageRuntime};
use flow_like_wasm::{LoadedWasm, WasmConfig, WasmEngine, WasmSecurityConfig};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;

async fn load_template() -> (WasmEngine, LoadedWasm) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/wasm-node-rust/target/wasm32-wasip2/debug/flow_like_wasm_node_template.wasm");
    let engine = WasmEngine::new(WasmConfig::development()).unwrap();
    let loaded = engine
        .load_auto_from_file(&path)
        .await
        .expect("build the Rust template first");
    (engine, loaded)
}

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
    let (engine, loaded) = load_template().await;
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

#[tokio::test]
#[ignore = "Build the Rust template with cargo build --target wasm32-wasip2 before running"]
async fn an_owned_iterator_preserves_its_position_and_can_be_consumed() {
    let (engine, loaded) = load_template().await;
    let security = WasmSecurityConfig::from_node_permissions(&[]);
    let run = RunResources::default();
    let owner = package(&run, "iterator", &loaded, &security);
    let created = call(
        &owner,
        &loaded,
        &engine,
        &security,
        "object_create_cursor",
        json!({"items": ["first", "second", "third"]}),
    )
    .await;
    assert!(created.error.is_none(), "{:?}", created.error);
    let cursor = created.outputs["cursor"].as_str().unwrap();
    let next = call(
        &owner,
        &loaded,
        &engine,
        &security,
        "object_next_item",
        json!({"cursor": cursor}),
    )
    .await;
    assert!(next.error.is_none(), "{:?}", next.error);
    assert_eq!(next.outputs["has_item"], json!(true));
    assert_eq!(next.outputs["item"], json!("first"));
    assert_eq!(next.outputs["remaining"], json!(2));
    let finished = call(
        &owner,
        &loaded,
        &engine,
        &security,
        "object_finish_cursor",
        json!({"cursor": cursor}),
    )
    .await;
    assert!(finished.error.is_none(), "{:?}", finished.error);
    assert_eq!(
        finished.outputs["remaining_items"],
        json!(["second", "third"])
    );
    let stale = call(
        &owner,
        &loaded,
        &engine,
        &security,
        "object_next_item",
        json!({"cursor": cursor}),
    )
    .await;
    assert!(stale.error.is_some());
    run.shutdown().await;
}

#[tokio::test]
#[ignore = "Build the Rust template with cargo build --target wasm32-wasip2 before running"]
async fn socket_resources_require_tcp_permission_and_an_allowed_address() {
    let (engine, loaded) = load_template().await;
    for security in [
        WasmSecurityConfig::from_node_permissions(&[]),
        WasmSecurityConfig::from_node_permissions(&[NodePermission::NetworkWebsocket]),
        WasmSecurityConfig::from_node_permissions(&[NodePermission::NetworkTcp])
            .with_allowed_hosts(vec![]),
    ] {
        let run = RunResources::default();
        let owner = package(&run, "sockets", &loaded, &security);
        let denied = call(
            &owner,
            &loaded,
            &engine,
            &security,
            "tcp_start_listener",
            json!({"bind_address": "127.0.0.1:0"}),
        )
        .await;
        assert!(
            denied.error.is_some(),
            "binding must require TCP and address access"
        );
        assert!(!denied.outputs.contains_key("listener"));
        run.shutdown().await;
    }
}

#[tokio::test]
#[ignore = "Build the Rust template with cargo build --target wasm32-wasip2 before running"]
async fn wasi_sockets_pass_between_nodes_and_close_with_their_run() {
    let (engine, loaded) = load_template().await;
    let security = WasmSecurityConfig::from_node_permissions(&[NodePermission::NetworkTcp]);
    for shutdown in [true, false] {
        let run = RunResources::default();
        let owner = package(&run, "sockets", &loaded, &security);
        let started = call(
            &owner,
            &loaded,
            &engine,
            &security,
            "tcp_start_listener",
            json!({"bind_address": "127.0.0.1:0"}),
        )
        .await;
        assert!(started.error.is_none(), "{:?}", started.error);
        let listener = started.outputs["listener"].as_str().unwrap();
        let address = started.outputs["address"].as_str().unwrap();
        let empty = call(
            &owner,
            &loaded,
            &engine,
            &security,
            "tcp_accept_connection",
            json!({"listener": listener}),
        )
        .await;
        assert!(empty.error.is_none(), "{:?}", empty.error);
        assert_eq!(empty.outputs["ready"], json!(false));

        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        let connection = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let accepted = call(
                    &owner,
                    &loaded,
                    &engine,
                    &security,
                    "tcp_accept_connection",
                    json!({"listener": listener}),
                )
                .await;
                assert!(accepted.error.is_none(), "{:?}", accepted.error);
                if accepted.outputs["ready"] == json!(true) {
                    break accepted.outputs["connection"].as_str().unwrap().to_owned();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("guest should accept the connected client");

        // A message larger than one send step exercises queued partial writes.
        let text = "Hello 🌍\n".repeat(9_000);
        let byte_len = text.len();
        let receive = tokio::spawn(async move {
            let mut bytes = vec![0; byte_len];
            client.read_exact(&mut bytes).await.unwrap();
            (client, bytes)
        });
        let mut sent = call(
            &owner,
            &loaded,
            &engine,
            &security,
            "tcp_send_text",
            json!({"connection": connection, "text": text}),
        )
        .await;
        assert!(sent.error.is_none(), "{:?}", sent.error);
        assert!(sent.outputs["pending_bytes"].as_u64().unwrap() > 0);
        tokio::time::timeout(Duration::from_secs(5), async {
            while sent.outputs["drained"] != json!(true) {
                sent = call(
                    &owner,
                    &loaded,
                    &engine,
                    &security,
                    "tcp_poll_send",
                    json!({"connection": connection}),
                )
                .await;
                assert!(sent.error.is_none(), "{:?}", sent.error);
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queued bytes should drain");
        let (mut client, received) = tokio::time::timeout(Duration::from_secs(5), receive)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, text.as_bytes());

        let foreign = package(&run, "other-package", &loaded, &security);
        let denied = call(
            &foreign,
            &loaded,
            &engine,
            &security,
            "tcp_send_text",
            json!({"connection": connection, "text": "foreign"}),
        )
        .await;
        assert!(denied.error.is_some());

        // Retain the PackageRuntime Arc to prove run ownership closes the store.
        if shutdown {
            run.shutdown().await;
        }
        drop(run);
        let closed = tokio::time::timeout(Duration::from_secs(2), client.read(&mut [0u8; 1]))
            .await
            .expect("run cleanup must close its WASI connection")
            .unwrap();
        assert_eq!(closed, 0);
        let rebound = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match tokio::net::TcpListener::bind(address).await {
                    Ok(listener) => break listener,
                    Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("failed to rebind closed listener: {error}"),
                }
            }
        })
        .await
        .expect("run cleanup must release its listening port");
        drop(rebound);
    }
}
