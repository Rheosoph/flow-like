extern crate flow_like_runtime as flow_like;

use flow_like::flow::execution::resources::RunResources;
use flow_like_wasm::abi::WasmExecutionInput;
use flow_like_wasm::host_functions::HostState;
use flow_like_wasm::package_runtime::{package_runtime_key, PackageRuntime};
use flow_like_wasm::{LoadedWasm, WasmCapabilities, WasmConfig, WasmEngine, WasmSecurityConfig};
use serde_json::{json, Map};
use std::sync::Arc;
use std::time::Duration;

fn input(node: &str) -> WasmExecutionInput {
    WasmExecutionInput {
        inputs: Map::new(),
        node_id: node.to_string(),
        node_name: node.to_string(),
        run_id: "run".to_string(),
        app_id: "app".to_string(),
        board_id: "board".to_string(),
        user_id: "user".to_string(),
        stream_state: false,
        log_level: 1,
    }
}

async fn counter(engine: &WasmEngine) -> LoadedWasm {
    let result = r#"{"outputs":{"count":0},"activate_exec":[]}"#;
    let digit = 512 + result.find('0').unwrap();
    let wat = format!(
        r#"(module
            (memory (export "memory") 1)
            (global $count (mut i32) (i32.const 0))
            (data (i32.const 512) {result:?})
            (func (export "get_node") (result i64) i64.const 0)
            (func (export "run") (param i32 i32) (result i64)
                (local $spin i32)
                i32.const 100000 local.set $spin
                (loop $work
                    local.get $spin i32.const 1 i32.sub local.tee $spin br_if $work)
                global.get $count i32.const 1 i32.add global.set $count
                i32.const {digit} global.get $count i32.const 48 i32.add i32.store8
                i64.const {packed})
        )"#,
        packed = flow_like_wasm::WasmAbi::pack_ptr_len(512, result.len() as u32),
    );
    LoadedWasm::Module(
        engine
            .load_module(&wat::parse_str(wat).unwrap())
            .await
            .unwrap(),
    )
}

fn runtime(
    run: &RunResources,
    package: &str,
    loaded: &LoadedWasm,
    security: &WasmSecurityConfig,
) -> Arc<PackageRuntime> {
    let key = package_runtime_key(package, loaded.hash(), security, "user", false).unwrap();
    run.get_or_insert_with(key, || Arc::new(PackageRuntime::default()))
        .unwrap()
}

#[tokio::test]
async fn nodes_share_guest_memory_only_within_the_same_package_and_run() {
    let engine = WasmEngine::new(WasmConfig::development()).unwrap();
    let loaded = counter(&engine).await;
    let security = WasmSecurityConfig::default();
    let first_run = RunResources::default();
    let package = runtime(&first_run, "p2p", &loaded, &security);
    let second_node = runtime(&first_run, "p2p", &loaded, &security);
    assert!(Arc::ptr_eq(&package, &second_node));

    let first = package
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &input("start"),
        )
        .await
        .unwrap();
    let second = second_node
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &input("send"),
        )
        .await
        .unwrap();
    assert_eq!(first.result.outputs["count"], json!(1));
    assert_eq!(second.result.outputs["count"], json!(2));

    let other_package = runtime(&first_run, "signing", &loaded, &security);
    let isolated = other_package
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &input("sign"),
        )
        .await
        .unwrap();
    assert_eq!(isolated.result.outputs["count"], json!(1));

    let second_run = RunResources::default();
    let fresh = runtime(&second_run, "p2p", &loaded, &security);
    let isolated = fresh
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &input("start"),
        )
        .await
        .unwrap();
    assert_eq!(isolated.result.outputs["count"], json!(1));

    first_run.shutdown().await;
    assert!(package
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &input("send")
        )
        .await
        .is_err());
    assert!(first_run
        .get_or_insert_with("new", || Arc::new(PackageRuntime::default()))
        .is_err());
    second_run.shutdown().await;
}

#[tokio::test]
async fn each_call_gets_fresh_logs_and_a_full_fuel_budget() {
    let engine = WasmEngine::new(WasmConfig::development()).unwrap();
    let loaded = counter(&engine).await;
    let mut security = WasmSecurityConfig::default();
    security.limits.fuel_limit = 1_000_000;
    let run = RunResources::default();
    let package = runtime(&run, "counter", &loaded, &security);
    for index in 1..=5 {
        let host = HostState::with_security(&security);
        host.log(1, format!("call {index}"), None);
        let call = package
            .call(&loaded, &engine, &security, host, &input("count"))
            .await
            .unwrap();
        assert_eq!(call.result.outputs["count"], json!(index));
        assert_eq!(call.logs.len(), 1);
        assert_eq!(call.logs[0].message, format!("call {index}"));
    }
    run.shutdown().await;
}

#[test]
fn host_call_permissions_do_not_split_package_state_but_wasi_grants_do() {
    let mut first = WasmSecurityConfig::default();
    first.capabilities = WasmCapabilities::WEBSOCKET;
    let mut second = first.clone();
    second.capabilities |= WasmCapabilities::CACHE_ALL;
    let key = |security: &WasmSecurityConfig| {
        package_runtime_key("package", "hash", security, "user", false).unwrap()
    };
    assert_eq!(key(&first), key(&second));
    second.capabilities |= WasmCapabilities::TCP;
    assert_ne!(key(&first), key(&second));
    assert_ne!(
        key(&first),
        package_runtime_key("package", "new-hash", &first, "user", false).unwrap()
    );
    assert_ne!(
        key(&first),
        package_runtime_key("package", "hash", &first, "other-user", false).unwrap()
    );
    assert_ne!(
        key(&first),
        package_runtime_key("package", "hash", &first, "user", true).unwrap()
    );
}

#[tokio::test]
async fn scratch_reset_reclaims_abi_buffers_without_resetting_package_globals() {
    let engine = WasmEngine::new(WasmConfig::development()).unwrap();
    let result = r#"{"outputs":{"count":0}}"#;
    let digit = 512 + result.find('0').unwrap();
    let wat = format!(
        r#"(module
        (memory (export "memory") 1)
        (global $cursor (mut i32) (i32.const 2048))
        (global $count (mut i32) (i32.const 0))
        (data (i32.const 512) {result:?})
        (func (export "get_node") (result i64) i64.const 0)
        (func (export "reset_scratch") i32.const 2048 global.set $cursor)
        (func (export "alloc") (param $size i32) (result i32)
            (local $ptr i32)
            global.get $cursor local.tee $ptr local.get $size i32.add global.set $cursor
            global.get $cursor i32.const 4096 i32.gt_u if unreachable end
            local.get $ptr)
        (func (export "dealloc") (param i32 i32))
        (func (export "run") (param i32 i32) (result i64)
            global.get $count i32.const 1 i32.add global.set $count
            i32.const {digit} global.get $count i32.const 10 i32.rem_u i32.const 48 i32.add i32.store8
            i64.const {packed})
    )"#,
        packed = flow_like_wasm::WasmAbi::pack_ptr_len(512, result.len() as u32)
    );
    let loaded = LoadedWasm::Module(
        engine
            .load_module(&wat::parse_str(wat).unwrap())
            .await
            .unwrap(),
    );
    let security = WasmSecurityConfig::default();
    let run = RunResources::default();
    let package = runtime(&run, "scratch", &loaded, &security);
    for index in 1..=20 {
        let call = package
            .call(
                &loaded,
                &engine,
                &security,
                HostState::with_security(&security),
                &input("run"),
            )
            .await
            .unwrap();
        assert_eq!(call.result.outputs["count"], json!(index % 10));
    }
    run.shutdown().await;
}

#[tokio::test]
async fn modules_without_alloc_reuse_host_input_scratch_within_the_memory_limit() {
    let engine = WasmEngine::new(WasmConfig::development()).unwrap();
    let loaded = counter(&engine).await;
    let mut security = WasmSecurityConfig::default();
    security.limits.memory_limit = 2 * 65536;
    let run = RunResources::default();
    let package = runtime(&run, "scratch", &loaded, &security);
    // A full page of input fits in the second memory page. Reallocating on
    // every invocation would exceed the package limit on the second call.
    let mut call_input = input("run");
    call_input
        .inputs
        .insert("large".into(), json!("x".repeat(40_000)));
    for index in 1..=5 {
        let call = package
            .call(
                &loaded,
                &engine,
                &security,
                HostState::with_security(&security),
                &call_input,
            )
            .await
            .unwrap();
        assert_eq!(call.result.outputs["count"], json!(index));
    }
    run.shutdown().await;
}

#[tokio::test]
async fn traps_and_timeouts_invalidate_the_package_until_the_run_ends() {
    for body in ["unreachable", "(loop $forever br $forever)"] {
        let engine = WasmEngine::new(WasmConfig::development()).unwrap();
        let bytes = wat::parse_str(format!(
            r#"(module
            (memory (export "memory") 1)
            (func (export "get_node") (result i64) i64.const 0)
            (func (export "run") (param i32 i32) (result i64) {body} i64.const 0)
        )"#
        ))
        .unwrap();
        let loaded = LoadedWasm::Module(engine.load_module(&bytes).await.unwrap());
        let mut security = WasmSecurityConfig::default();
        security.limits.timeout = Duration::from_millis(50);
        let run = RunResources::default();
        let package = runtime(&run, "broken", &loaded, &security);
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            package.call(
                &loaded,
                &engine,
                &security,
                HostState::with_security(&security),
                &input("run"),
            ),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(package
            .call(
                &loaded,
                &engine,
                &security,
                HostState::with_security(&security),
                &input("retry")
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("closed"));
        run.shutdown().await;
    }
}

#[tokio::test]
async fn run_shutdown_interrupts_guest_code_and_prevents_reentry() {
    let engine = Arc::new(WasmEngine::new(WasmConfig::development()).unwrap());
    let bytes = wat::parse_str(
        r#"(module
        (memory (export "memory") 1)
        (func (export "get_node") (result i64) i64.const 0)
        (func (export "run") (param i32 i32) (result i64)
            (loop $forever br $forever) i64.const 0)
    )"#,
    )
    .unwrap();
    let loaded = LoadedWasm::Module(engine.load_module(&bytes).await.unwrap());
    let mut security = WasmSecurityConfig::default();
    security.limits.fuel_limit = u64::MAX;
    let run = RunResources::default();
    let package = runtime(&run, "busy", &loaded, &security);
    let task = tokio::spawn({
        let package = package.clone();
        let loaded = loaded.clone();
        let engine = engine.clone();
        let security = security.clone();
        async move {
            package
                .call(
                    &loaded,
                    &engine,
                    &security,
                    HostState::with_security(&security),
                    &input("busy"),
                )
                .await
        }
    });
    tokio::task::yield_now().await;
    tokio::time::timeout(Duration::from_secs(2), run.shutdown())
        .await
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(package
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &input("retry")
        )
        .await
        .is_err());
}

#[cfg(feature = "component-model")]
#[tokio::test]
#[ignore = "Build the Rust template with cargo build --target wasm32-wasip2 before running"]
async fn rust_template_hands_a_server_between_nodes_and_run_end_closes_it() {
    use futures::StreamExt;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates/wasm-node-rust/target/wasm32-wasip2/debug/flow_like_wasm_node_template.wasm");
    let engine = WasmEngine::new(WasmConfig::development()).unwrap();
    let loaded = engine
        .load_auto_from_file(&path)
        .await
        .expect("build the Rust template first");
    let mut security = WasmSecurityConfig::default();
    security.capabilities = WasmCapabilities::WEBSOCKET;
    let run = RunResources::default();
    let package = runtime(&run, "websocket-example", &loaded, &security);

    let mut start = input("ws_start_server");
    start
        .inputs
        .insert("bind_address".into(), json!("127.0.0.1:0"));
    let started = package
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &start,
        )
        .await
        .unwrap();
    assert!(started.result.error.is_none(), "{:?}", started.result.error);
    let listener = started.result.outputs["listener"]
        .as_str()
        .unwrap()
        .to_string();
    let address = started.result.outputs["address"]
        .as_str()
        .unwrap()
        .to_string();

    let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
        .await
        .unwrap();
    let mut accept = input("ws_accept_connection");
    accept.inputs.insert("listener".into(), json!(listener));
    accept.inputs.insert("timeout_ms".into(), json!(1000));
    let accepted = package
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &accept,
        )
        .await
        .unwrap();
    assert!(
        accepted.result.error.is_none(),
        "{:?}",
        accepted.result.error
    );
    let connection = accepted.result.outputs["connection"]
        .as_str()
        .unwrap()
        .to_string();

    let mut send = input("ws_send_text");
    send.inputs.insert("connection".into(), json!(connection));
    send.inputs
        .insert("text".into(), json!("sent by another node"));
    let mut denied_security = security.clone();
    denied_security.capabilities = WasmCapabilities::NONE;
    assert!(Arc::ptr_eq(
        &package,
        &runtime(&run, "websocket-example", &loaded, &denied_security)
    ));
    let denied = package
        .call(
            &loaded,
            &engine,
            &denied_security,
            HostState::with_security(&denied_security),
            &send,
        )
        .await
        .unwrap();
    assert!(denied.result.error.is_some());
    let sent = package
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &send,
        )
        .await
        .unwrap();
    assert!(sent.result.error.is_none(), "{:?}", sent.result.error);
    let message = tokio::time::timeout(Duration::from_secs(2), client.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(message.into_text().unwrap(), "sent by another node");

    let other_package = runtime(&run, "unrelated-package", &loaded, &security);
    let denied = other_package
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &send,
        )
        .await
        .unwrap();
    assert!(denied.result.error.is_some());
    let other_run = RunResources::default();
    let other_instance = runtime(&other_run, "websocket-example", &loaded, &security);
    let denied = other_instance
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &send,
        )
        .await
        .unwrap();
    assert!(denied.result.error.is_some());

    run.shutdown().await;
    assert!(tokio::net::TcpStream::connect(&address).await.is_err());
    let closed = tokio::time::timeout(Duration::from_secs(2), client.next())
        .await
        .unwrap();
    assert!(
        closed.is_none()
            || matches!(
                closed,
                Some(Err(_)) | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))
            )
    );
    assert!(package
        .call(
            &loaded,
            &engine,
            &security,
            HostState::with_security(&security),
            &send
        )
        .await
        .is_err());
    other_run.shutdown().await;
}
