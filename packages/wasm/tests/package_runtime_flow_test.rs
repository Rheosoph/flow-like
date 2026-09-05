extern crate flow_like_runtime as flow_like;

use flow_like::flow::board::commands::pins::connect_pins::connect_pins;
use flow_like::flow::board::Board;
use flow_like::flow::execution::{InternalRun, RunPayload, RunStatus};
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::profile::Profile;
use flow_like::state::{FlowLikeConfig, FlowLikeState, FlowNodeRegistryInner};
use flow_like::utils::http::HTTPClient;
use flow_like_storage::Path;
use flow_like_types::intercom::BufferedInterComHandler;
use flow_like_wasm::abi::WasmNodeDefinition;
use flow_like_wasm::package_runtime::{package_runtime_key, PackageRuntime};
use flow_like_wasm::{LoadedWasm, WasmConfig, WasmEngine, WasmNodeLogic, WasmSecurityConfig};
use serde_json::json;
use std::sync::Arc;

const PACKAGE: &str = "local::run-counter-test";

async fn counter(engine: &WasmEngine) -> LoadedWasm {
    let result = r#"{"outputs":{"count":0,"observed":0},"activate_exec":["exec_out"]}"#;
    let count_digit = 512 + result.find('0').unwrap();
    let observed_digit = 512 + result.rfind('0').unwrap();
    // The fixture reads its sole numeric input before the execution metadata.
    // Its counter lives in guest memory, which must survive the next node call.
    let wat = format!(
        r#"(module
            (memory (export "memory") 1)
            (data (i32.const 512) {result:?})
            (func (export "get_node") (result i64) i64.const 0)
            (func (export "run") (param $ptr i32) (param $len i32) (result i64)
                (local $cursor i32) (local $digit i32)
                local.get $ptr local.set $cursor
                (block $found
                    (loop $scan
                        local.get $cursor local.get $ptr local.get $len i32.add i32.ge_u
                        if unreachable end
                        local.get $cursor i32.load8_u local.tee $digit
                        i32.const 48 i32.ge_u
                        local.get $digit i32.const 57 i32.le_u i32.and br_if $found
                        local.get $cursor i32.const 1 i32.add local.set $cursor
                        br $scan))
                i32.const {observed_digit} local.get $digit i32.store8
                i32.const 64 i32.const 64 i32.load i32.const 1 i32.add i32.store
                i32.const {count_digit} i32.const 64 i32.load i32.const 48 i32.add i32.store8
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

fn definition(name: &str) -> WasmNodeDefinition {
    serde_json::from_value(json!({
        "name": name,
        "friendly_name": name,
        "description": "Share a counter for one run",
        "category": "Tests",
        "pins": [
            {"name": "exec_in", "friendly_name": "In", "description": "", "pin_type": "Input", "data_type": "Execution"},
            {"name": "exec_out", "friendly_name": "Out", "description": "", "pin_type": "Output", "data_type": "Execution"},
            {"name": "previous", "friendly_name": "Previous", "description": "", "pin_type": "Input", "data_type": "Integer", "default_value": 0},
            {"name": "count", "friendly_name": "Count", "description": "", "pin_type": "Output", "data_type": "Integer"},
            {"name": "observed", "friendly_name": "Observed Input", "description": "", "pin_type": "Output", "data_type": "Integer"}
        ]
    }))
    .unwrap()
}

fn pin_id(node: &Node, name: &str) -> String {
    node.pins
        .values()
        .find(|pin| pin.name == name)
        .unwrap()
        .id
        .clone()
}

async fn make_run(board: Arc<Board>, state: &Arc<FlowLikeState>) -> InternalRun {
    InternalRun::new(
        "test-app",
        board,
        None,
        state,
        &Profile::default(),
        &RunPayload {
            id: "start".into(),
            payload: None,
            runtime_variables: None,
            filter_secrets: Some(true),
        },
        false,
        BufferedInterComHandler::new(
            Arc::new(|_events| Box::pin(async { Ok(()) })),
            Some(100),
            Some(400),
            Some(false),
        )
        .into_callback(),
        None,
        None,
        std::collections::HashMap::new(),
    )
    .await
    .unwrap()
}

async fn output(run: &InternalRun, node: &str, pin: &str) -> Option<i64> {
    run.nodes[node]
        .get_pin_by_name(pin)
        .await
        .unwrap()
        .get_value()
        .await
}

async fn assert_completed(run: &InternalRun) {
    assert!(
        matches!(run.get_status().await, RunStatus::Success),
        "{}",
        serde_json::to_string(&run.get_traces().await).unwrap()
    );
    assert_eq!(output(run, "start", "count").await, Some(1));
    assert_eq!(output(run, "start", "observed").await, Some(0));
    assert_eq!(output(run, "next", "count").await, Some(2));
    assert_eq!(output(run, "next", "observed").await, Some(1));
    assert!(run.meta.resources.is_closed());
}

#[tokio::test]
async fn flow_nodes_share_the_package_instance_and_run_completion_drops_it() {
    let engine = Arc::new(WasmEngine::new(WasmConfig::development()).unwrap());
    let loaded = counter(&engine).await;
    let security = WasmSecurityConfig::default();
    let logics = ["counter_start", "counter_next"].map(|name| {
        Arc::new(
            WasmNodeLogic::from_loaded_with_target(
                loaded.clone(),
                engine.clone(),
                security.clone(),
                definition(name),
            )
            .with_package_id(PACKAGE.into()),
        )
    });
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let mut registry = FlowNodeRegistryInner::new(logics.len());
    for logic in &logics {
        registry.insert(logic.get_node(), logic.clone());
    }
    state.node_registry.write().await.node_registry = Arc::new(registry);

    let mut start = logics[0].get_node();
    start.id = "start".into();
    start.set_start(true);
    let mut next = logics[1].get_node();
    next.id = "next".into();
    let mut board = Board::new_detached(Some("wasm-package-runtime".into()), Path::default());
    board.nodes.insert(start.id.clone(), start.clone());
    board.nodes.insert(next.id.clone(), next.clone());
    for (from, to) in [("exec_out", "exec_in"), ("count", "previous")] {
        connect_pins(
            &mut board,
            &start.id,
            &pin_id(&start, from),
            &next.id,
            &pin_id(&next, to),
        )
        .unwrap();
    }
    let board = Arc::new(board);
    let mut run = make_run(board.clone(), &state).await;

    assert!(run.debug_step(state.clone()).await);
    assert_eq!(output(&run, "start", "count").await, Some(1));
    assert!(!run.meta.resources.is_closed());
    let mut run_security = security.clone();
    run_security.execution_environment = run.meta.environment;
    let key =
        package_runtime_key(PACKAGE, loaded.hash(), &run_security, &run.meta.sub, false).unwrap();
    let runtime = run
        .meta
        .resources
        .get_or_insert_with::<PackageRuntime>(key, || panic!("node did not create package runtime"))
        .unwrap();
    let weak_runtime = Arc::downgrade(&runtime);
    drop(runtime);
    let first_resources = run.meta.resources.clone();

    run.execute(state.clone()).await;
    assert_completed(&run).await;
    assert!(
        weak_runtime.upgrade().is_none(),
        "run retained the package instance"
    );

    run.fork().await.unwrap();
    assert!(!Arc::ptr_eq(&first_resources, &run.meta.resources));
    run.execute(state.clone()).await;
    assert_completed(&run).await;

    let mut independent = make_run(board, &state).await;
    assert!(!Arc::ptr_eq(
        &run.meta.resources,
        &independent.meta.resources
    ));
    independent.execute(state).await;
    assert_completed(&independent).await;
}
