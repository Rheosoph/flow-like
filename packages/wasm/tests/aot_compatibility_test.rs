use flow_like_wasm::aot_cache::{host_platform_key, portable_target, WASMTIME_MAJOR_VERSION};
use flow_like_wasm::{WasmConfig, WasmEngine};
use tempfile::TempDir;
use wasmtime::{Config, Engine, ModuleVersionStrategy};

fn incompatible_engine() -> Engine {
    let previous_version = WASMTIME_MAJOR_VERSION.parse::<u32>().unwrap() - 1;
    let mut config = Config::new();
    // Wasmtime writes the version into its own artifact format. This exercises
    // upgrade rejection using trusted artifacts without modifying native code.
    config
        .module_version(ModuleVersionStrategy::Custom(previous_version.to_string()))
        .unwrap();
    Engine::new(&config).unwrap()
}

fn portable_compiler_and_runtime() -> (WasmEngine, WasmEngine) {
    let target = portable_target(std::env::consts::OS, std::env::consts::ARCH).unwrap();
    let compiler =
        WasmEngine::new(WasmConfig::default().without_cache().with_target(target)).unwrap();
    let runtime = WasmEngine::new(WasmConfig::lambda().without_cache()).unwrap();
    (compiler, runtime)
}

#[test]
fn portable_module_deserializes_in_the_executor_configuration() {
    let (compiler, runtime) = portable_compiler_and_runtime();
    let wasm =
        wat::parse_str("(module (func (export \"answer\") (result i32) i32.const 42))").unwrap();
    let artifact = compiler.precompile(&wasm).unwrap();
    // The bytes were produced above by our trusted compiler configuration.
    let module = unsafe { wasmtime::Module::deserialize(runtime.engine(), artifact) }.unwrap();
    let mut store = wasmtime::Store::new(runtime.engine(), ());
    store.set_fuel(1000).unwrap();
    store.set_epoch_deadline(100);
    let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
    let answer = instance
        .get_typed_func::<(), i32>(&mut store, "answer")
        .unwrap();
    assert_eq!(answer.call(&mut store, ()).unwrap(), 42);
}

#[cfg(feature = "component-model")]
#[test]
fn portable_component_deserializes_in_the_executor_configuration() {
    let (compiler, runtime) = portable_compiler_and_runtime();
    let wasm = wat::parse_str("(component)").unwrap();
    let artifact = compiler.precompile(&wasm).unwrap();
    // The bytes were produced above by our trusted compiler configuration.
    unsafe { wasmtime::component::Component::deserialize(runtime.engine(), artifact) }.unwrap();
}

#[tokio::test]
async fn incompatible_module_is_evicted_and_recompiled() {
    let temp = TempDir::new().unwrap();
    let engine = WasmEngine::new(WasmConfig::development().with_cache_dir(temp.path())).unwrap();
    let wasm = wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "get_node") (result i64) i64.const 0)
            (func (export "run") (param i32 i32) (result i64) i64.const 0))"#,
    )
    .unwrap();
    let hash = blake3::hash(&wasm).to_hex().to_string();
    let stale = incompatible_engine().precompile_module(&wasm).unwrap();
    let cache = engine.aot_cache().unwrap();
    let path = temp
        .path()
        .join("modules")
        .join(format!("{hash}-{}.cwasm", host_platform_key()));

    cache.inject_module(&hash, &stale).unwrap();
    assert!(path.exists());
    assert!(cache.load_module(engine.engine(), &hash).is_none());
    assert!(!path.exists(), "incompatible artifact must be evicted");

    cache.inject_module(&hash, &stale).unwrap();
    let loaded = engine.load_module(&wasm).await.unwrap();
    assert_eq!(loaded.hash(), hash);
    assert!(cache.load_module(engine.engine(), &hash).is_some());
    assert_ne!(std::fs::read(path).unwrap(), stale);
}

#[cfg(feature = "component-model")]
#[tokio::test]
async fn incompatible_component_is_evicted_and_recompiled() {
    let temp = TempDir::new().unwrap();
    let engine = WasmEngine::new(WasmConfig::development().with_cache_dir(temp.path())).unwrap();
    let wasm = wat::parse_str("(component)").unwrap();
    let hash = blake3::hash(&wasm).to_hex().to_string();
    let stale = incompatible_engine().precompile_component(&wasm).unwrap();
    let cache = engine.aot_cache().unwrap();
    let path = temp
        .path()
        .join("components")
        .join(format!("{hash}-{}.cwasm", host_platform_key()));

    cache.inject_component(&hash, &stale).unwrap();
    assert!(path.exists());
    assert!(cache.load_component(engine.engine(), &hash).is_none());
    assert!(!path.exists(), "incompatible artifact must be evicted");

    cache.inject_component(&hash, &stale).unwrap();
    let loaded = engine.load_component(&wasm).await.unwrap();
    assert_eq!(loaded.hash(), hash);
    assert!(cache.load_component(engine.engine(), &hash).is_some());
    assert_ne!(std::fs::read(path).unwrap(), stale);
}
