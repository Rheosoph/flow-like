//! Regression guards for Flow-Like's WASM environment isolation boundary.

use std::fs;
use std::path::{Path, PathBuf};

use flow_like_wasm::host_functions::linker::{register_host_functions, StoreData};
use flow_like_wasm::WasmCapabilities;
use wasmtime::{Engine, Linker, Module, Store};

fn rust_sources_under(roots: &[PathBuf]) -> Vec<PathBuf> {
    fn visit(directory: &Path, sources: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).expect("repository directory should be readable") {
            let entry = entry.expect("repository entry should be readable");
            let path = entry.path();
            let file_type = entry
                .file_type()
                .expect("repository entry type should be readable");

            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                visit(&path, sources);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                sources.push(path);
            }
        }
    }

    let mut sources = Vec::new();
    for root in roots {
        visit(root, &mut sources);
    }
    sources
}

fn contains_identifier(source: &str, identifier: &str) -> bool {
    source.match_indices(identifier).any(|(start, _)| {
        let before = source[..start].chars().next_back();
        let end = start + identifier.len();
        let after = source[end..].chars().next();
        let is_identifier_character =
            |character: char| character == '_' || character.is_alphanumeric();

        !before.is_some_and(is_identifier_character) && !after.is_some_and(is_identifier_character)
    })
}

#[test]
fn wasm_runtime_sources_cannot_inherit_the_host_environment() {
    let package_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = package_dir
        .parent()
        .and_then(Path::parent)
        .expect("flow-like-wasm should live at packages/wasm");

    let runtime_roots = [
        repository_root.join("packages/wasm/src"),
        repository_root.join("libs/nodes/code-interpreter/src/pyodide"),
    ];
    let raw_builder_file = repository_root.join("packages/wasm/src/wasi.rs");
    let raw_builder_type = ["WasiCtx", "Builder"].concat();
    let forbidden_inheritance = ["inherit", "_env"].concat();

    let mut violations = Vec::new();
    for source_path in rust_sources_under(&runtime_roots) {
        let source = fs::read_to_string(&source_path).expect("Rust source should be readable");
        if source.contains(&forbidden_inheritance)
            || (source_path != raw_builder_file && contains_identifier(&source, &raw_builder_type))
        {
            violations.push(
                source_path
                    .strip_prefix(repository_root)
                    .expect("source should be inside repository")
                    .to_path_buf(),
            );
        }
    }

    assert!(
        violations.is_empty(),
        "WASM runtimes must use the isolated builder and cannot inherit the host environment: {violations:?}"
    );
}

#[test]
fn core_wasm_preview1_environment_stub_returns_an_empty_environment() {
    assert!(
        std::env::var_os("PATH").is_some(),
        "the test host needs a non-empty environment sentinel"
    );

    let wasm = wat::parse_str(
        r#"
            (module
                (import "wasi_snapshot_preview1" "environ_sizes_get"
                    (func $environ_sizes_get (param i32 i32) (result i32)))
                (import "wasi_snapshot_preview1" "environ_get"
                    (func $environ_get (param i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff")
                (func (export "environment_probe") (result i32 i32 i32 i32 i32 i32)
                    (local $sizes_errno i32)
                    (local $get_errno i32)
                    i32.const 0
                    i32.const 4
                    call $environ_sizes_get
                    local.set $sizes_errno
                    i32.const 8
                    i32.const 12
                    call $environ_get
                    local.set $get_errno
                    local.get $sizes_errno
                    i32.const 0
                    i32.load
                    i32.const 4
                    i32.load
                    local.get $get_errno
                    i32.const 8
                    i32.load
                    i32.const 12
                    i32.load))
        "#,
    )
    .expect("environment probe should compile from WAT");

    let engine = Engine::default();
    let module = Module::new(&engine, wasm).expect("environment probe should compile");
    let mut linker: Linker<StoreData> = Linker::new(&engine);
    register_host_functions(&mut linker).expect("Flow-Like host functions should link");
    let mut store = Store::new(&engine, StoreData::new(WasmCapabilities::NONE));
    let instance = linker
        .instantiate(&mut store, &module)
        .expect("environment probe should instantiate");
    let environment_probe = instance
        .get_typed_func::<(), (i32, i32, i32, i32, i32, i32)>(&mut store, "environment_probe")
        .expect("environment probe export should exist")
        .call(&mut store, ())
        .expect("environment probe should run");

    assert_eq!(
        environment_probe,
        (0, 0, 0, 0, -1, -1),
        "core WASM received host environment entries (sizes errno/count/bytes, get errno/buffers)"
    );
}
