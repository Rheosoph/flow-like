use super::*;

fn definition() -> serde_json::Value {
    serde_json::json!({
        "name": "metadata", "friendly_name": "Metadata", "description": "A metadata probe.",
        "category": "Testing", "pins": [], "permissions": ["models", "database:read"]
    })
}

fn core_guest(imports: &str, probe: &str) -> Vec<u8> {
    let json = definition().to_string();
    let data = json
        .bytes()
        .map(|byte| format!("\\{byte:02x}"))
        .collect::<String>();
    wat::parse_str(format!(
        r#"(module
        {imports}
        (memory (export "memory") 1)
        (data (i32.const 128) "{data}")
        (func (export "run") (param i32 i32) (result i64) i64.const 0)
        (func (export "get_node") (result i64)
            {probe}
            i64.const {packed}))"#,
        packed = (128u64 << 32) | json.len() as u64
    ))
    .unwrap()
}

#[tokio::test]
async fn metadata_core_imports_deny_resources_and_preserve_sdk_observations() {
    let bytes = core_guest(
        r#"
        (import "flowlike_db" "query" (func $db (param i32 i32 i32 i32 i32) (result i64)))
        (import "flowlike_models" "embed_text" (func $model (param i32 i32 i32 i32) (result i64)))
        (import "flowlike_storage" "write_request" (func $write (param i32 i32 i32 i32) (result i32)))
        (import "flowlike_meta" "time_now" (func $time (result i64)))
        (import "flowlike_meta" "random" (func $random (result i64)))
        (import "env" "host_time_now" (func $legacy_time (result i64)))
        (import "env" "host_random" (func $legacy_random (result i64)))
        (import "wasi_snapshot_preview1" "environ_sizes_get" (func $env (param i32 i32) (result i32)))
    "#,
        r#"
        i32.const -1 i32.const -1 i32.const -1 i32.const -1 i32.const -1 call $db
        i64.eqz if else unreachable end
        i32.const -1 i32.const -1 i32.const -1 i32.const -1 call $model
        i64.eqz if else unreachable end
        i32.const -1 i32.const -1 i32.const -1 i32.const -1 call $write
        i32.const -1 i32.eq if else unreachable end
        call $time i64.const 0 i64.gt_u if else unreachable end
        call $random drop
        call $legacy_time i64.const 0 i64.gt_u if else unreachable end
        call $legacy_random drop
        i32.const 0 i32.const 4 call $env i32.eqz if else unreachable end
        i32.const 0 i32.load i32.eqz if else unreachable end
        i32.const 4 i32.load i32.eqz if else unreachable end
    "#,
    );
    let nodes = extract_nodes(&bytes, WasmLimits::restrictive())
        .await
        .unwrap();
    assert_eq!(nodes[0].name, "metadata");
    assert_eq!(nodes[0].permissions, ["models", "database:read"]);
}

#[tokio::test]
async fn unknown_import_is_rejected_before_guest_execution() {
    let bytes = core_guest(
        r#"(import "flowlike_models" "future_model" (func))"#,
        "unreachable",
    );
    let error = extract_nodes(&bytes, WasmLimits::restrictive())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("future_model"), "{error}");
    assert!(!error.contains("unreachable"), "{error}");
}

#[tokio::test]
async fn core_metadata_rejects_negative_and_out_of_bounds_pointer_results() {
    for (packed, expected) in [
        (-1, "Guest returned an error"),
        (((65_530i64) << 32) | 100, "outside guest memory"),
    ] {
        let bytes = core_guest("", &format!("i64.const {packed} return"));
        let error = extract_nodes(&bytes, WasmLimits::restrictive())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
    }
}

#[tokio::test]
async fn legacy_variable_scratch_map_remains_available() {
    // Guest memory starts with the scratch key and a JSON value. The return
    // buffer ABI reports the serialized value's length in its lower 32 bits.
    let bytes = core_guest(
        r#"
        (import "env" "host_set_variable" (func $set (param i32 i32 i32 i32) (result i32)))
        (import "env" "host_get_variable" (func $get (param i32 i32) (result i64)))
        (data (i32.const 0) "key123")
        "#,
        r#"
        i32.const 0 i32.const 3 i32.const 3 i32.const 3 call $set
        i32.eqz if else unreachable end
        i32.const 0 i32.const 3 call $get
        i32.wrap_i64 i32.const 3 i32.eq if else unreachable end
        "#,
    );
    assert_eq!(
        extract_nodes(&bytes, WasmLimits::restrictive())
            .await
            .unwrap()[0]
            .name,
        "metadata"
    );
}

#[tokio::test]
async fn reactor_initialization_precedes_start_and_metadata() {
    let bytes = core_guest(
        r#"
        (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
        (global $phase (mut i32) (i32.const 0))
        (func (export "_initialize")
            global.get $phase i32.eqz if else unreachable end
            i32.const 1 global.set $phase)
        (func (export "_start")
            global.get $phase i32.const 1 i32.eq if else unreachable end
            i32.const 2 global.set $phase
            i32.const 0 call $exit)
        "#,
        "global.get $phase i32.const 2 i32.eq if else unreachable end",
    );
    assert_eq!(
        extract_nodes(&bytes, WasmLimits::restrictive())
            .await
            .unwrap()[0]
            .name,
        "metadata"
    );
}

#[tokio::test]
async fn metadata_cannot_run_past_its_fuel_limit() {
    let bytes = core_guest("", "(loop $forever br $forever)");
    let limits = WasmLimits {
        fuel_limit: 1_000,
        ..WasmLimits::restrictive()
    };
    let error = extract_nodes(&bytes, limits).await.unwrap_err().to_string();
    assert!(error.contains("fuel"), "{error}");
}

#[tokio::test]
async fn metadata_timeout_interrupts_guests_even_on_a_single_thread_runtime() {
    let bytes = core_guest("", "(loop $forever br $forever)");
    let limits = WasmLimits {
        fuel_limit: u64::MAX,
        timeout: std::time::Duration::from_millis(20),
        ..WasmLimits::restrictive()
    };
    let error = extract_nodes(&bytes, limits).await.unwrap_err().to_string();
    assert!(error.contains("interrupt"), "{error}");
}

#[cfg(feature = "component-model")]
#[tokio::test]
async fn command_component_metadata_is_read_from_stdout() {
    let bytes = wat::parse_str(include_str!("command.wat")).unwrap();
    let limits = WasmLimits {
        max_instances: 4,
        ..WasmLimits::restrictive()
    };
    let nodes = extract_nodes(&bytes, limits).await.unwrap();
    assert_eq!(nodes[0].name, "command");
    assert_eq!(nodes[0].description, "CLI metadata");
}

#[test]
fn command_context_receives_only_the_metadata_arguments() {
    use wasmtime_wasi::cli::WasiCliCtxView;
    use wasmtime_wasi::p2::bindings::cli::environment::Host;
    let mut state = MetadataState::new(
        &WasmLimits::restrictive(),
        Some(wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(1024)),
    );
    let mut cli = WasiCliCtxView {
        ctx: state.wasi.cli(),
        table: &mut state.table,
    };
    assert_eq!(
        Host::get_arguments(&mut cli).unwrap(),
        ["flow-like-wasm-node", "get-node"]
    );
    assert!(Host::get_environment(&mut cli).unwrap().is_empty());
}

#[cfg(feature = "component-model")]
#[test]
fn metadata_component_linker_accepts_the_complete_sdk_abi() {
    let engine = CompilationEngine::new(CompilationConfig::default())
        .unwrap()
        .engine;
    let component = wasmtime::component::Component::new(
        &engine,
        wat::parse_str(include_str!("imports.wat")).unwrap(),
    )
    .unwrap();
    let mut linker = wasmtime::component::Linker::new(&engine);
    component_imports::register(&mut linker).unwrap();
    linker.instantiate_pre(&component).unwrap();
}

#[cfg(feature = "component-model")]
#[tokio::test]
async fn component_imports_deny_resources_and_preserve_sdk_observations() {
    let engine = CompilationEngine::new(CompilationConfig::default())
        .unwrap()
        .engine;
    let bytes = wat::parse_str(
        r#"(component
        (import "flow-like:node/models@0.1.0" (instance $models
            (export "embed-text" (func (param "model" string) (param "texts" string) (result (option string))))))
        (import "flow-like:node/storage@0.1.0" (instance $storage
            (export "write-file" (func (param "path" string) (param "data" (list u8)) (result bool)))))
        (import "flow-like:node/metadata@0.1.0" (instance $metadata
            (export "time-now" (func (result u64)))
            (export "random" (func (result f64)))))
        (alias export $models "embed-text" (func $model))
        (alias export $storage "write-file" (func $write))
        (alias export $metadata "time-now" (func $time))
        (alias export $metadata "random" (func $random))
        (export "probe-model" (func $model))
        (export "probe-write" (func $write))
        (export "probe-time" (func $time))
        (export "probe-random" (func $random)))"#,
    )
    .unwrap();
    let component = wasmtime::component::Component::new(&engine, bytes).unwrap();
    let mut linker = wasmtime::component::Linker::new(&engine);
    component_imports::register(&mut linker).unwrap();
    let mut store = store(&engine, &WasmLimits::restrictive(), None).unwrap();
    let instance = linker
        .instantiate_async(&mut store, &component)
        .await
        .unwrap();
    let model = instance
        .get_typed_func::<(String, String), (Option<String>,)>(&mut store, "probe-model")
        .unwrap();
    assert_eq!(
        model
            .call_async(&mut store, ("invalid".into(), "invalid".into()))
            .await
            .unwrap(),
        (None,)
    );
    let write = instance
        .get_typed_func::<(String, Vec<u8>), (bool,)>(&mut store, "probe-write")
        .unwrap();
    assert_eq!(
        write
            .call_async(&mut store, ("invalid".into(), vec![1]))
            .await
            .unwrap(),
        (false,)
    );
    let time = instance
        .get_typed_func::<(), (u64,)>(&mut store, "probe-time")
        .unwrap();
    let random = instance
        .get_typed_func::<(), (f64,)>(&mut store, "probe-random")
        .unwrap();
    assert!(time.call_async(&mut store, ()).await.unwrap().0 > 0);
    assert!((0.0..=1.0).contains(&random.call_async(&mut store, ()).await.unwrap().0));
}

#[cfg(all(feature = "execution", feature = "component-model"))]
#[test]
fn compiler_and_execution_hosts_accept_the_same_component_imports() {
    let engine = crate::WasmEngine::new(crate::WasmConfig::default().without_cache()).unwrap();
    let component = wasmtime::component::Component::new(
        engine.engine(),
        wat::parse_str(include_str!("imports.wat")).unwrap(),
    )
    .unwrap();
    let mut linker = wasmtime::component::Linker::new(engine.engine());
    crate::component::linker::register_component_host_functions(
        &mut linker,
        &crate::WasmSecurityConfig::restrictive(),
    )
    .unwrap();
    linker.instantiate_pre(&component).unwrap();
}

#[cfg(feature = "execution")]
#[test]
fn compiler_core_denial_signatures_match_the_execution_host() {
    let engine = crate::WasmEngine::new(crate::WasmConfig::default().without_cache()).unwrap();
    let mut linker = wasmtime::Linker::new(engine.engine());
    crate::host_functions::register_host_functions(&mut linker).unwrap();
    let ty = |ty: &wasmtime::ValType| match ty {
        wasmtime::ValType::I32 => "i32",
        wasmtime::ValType::I64 => "i64",
        _ => unreachable!(),
    };
    let mut imports = String::from("(module");
    for (module, name, params, result, _) in core_imports::IMPORTS {
        imports.push_str(&format!("(import \"{module}\" \"{name}\" (func"));
        for param in *params {
            imports.push_str(&format!("(param {})", ty(param)));
        }
        if let Some(result) = result {
            imports.push_str(&format!("(result {})", ty(result)));
        }
        imports.push_str("))");
    }
    imports.push(')');
    let module = Module::new(engine.engine(), wat::parse_str(imports).unwrap()).unwrap();
    linker.instantiate_pre(&module).unwrap();
}

#[cfg(feature = "execution")]
#[test]
fn compiler_node_wire_data_matches_runtime_conversion() {
    let mut definition = definition();
    definition["abi_version"] = serde_json::json!(2);
    definition["pins"] = serde_json::json!([
        { "name": "value", "friendly_name": "Value", "description": "Input", "pin_type": "Input", "data_type": "i64", "value_type": "Array", "default_value": [1,2], "range": [0,100], "step": 1, "sensitive": true, "valid_values": ["1","2"], "enforce_schema": true, "schema": "{}", "enforce_generic_value_type": true },
        { "name": "out", "friendly_name": "Out", "description": "Output", "pin_type": "Output", "data_type": "Geometry" }
    ]);
    let runtime: crate::abi::WasmNodeDefinition =
        serde_json::from_value(definition.clone()).unwrap();
    let mut expected = serde_json::to_value(crate::definition_to_package_entry(&runtime)).unwrap();
    let mut actual =
        serde_json::to_value(&nodes::parse(&definition.to_string()).unwrap()[0]).unwrap();
    for value in [&mut expected, &mut actual] {
        for pin in value["pins"].as_object_mut().unwrap().values_mut() {
            pin.as_object_mut().unwrap().remove("id");
        }
    }
    assert_eq!(actual, expected);
    assert!(actual["version"].is_null());
}

#[cfg(feature = "execution")]
#[test]
fn metadata_type_schemas_match_execution_types() {
    let mut expected_names = crate::host_functions::schema::list_type_names();
    expected_names.sort_unstable();
    assert_eq!(expected_names, schemas::TYPES);
    for name in schemas::TYPES {
        let expected: serde_json::Value =
            serde_json::from_str(crate::host_functions::schema::get_type_schema(name).unwrap())
                .unwrap();
        let actual: serde_json::Value = serde_json::from_str(&schemas::get(name).unwrap()).unwrap();
        assert_eq!(actual, expected, "schema for {name}");
    }
}
