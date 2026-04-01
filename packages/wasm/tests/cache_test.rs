//! Tests for wasmtime cranelift compilation caching.
//!
//! Verifies that compiled WASM modules produce cache artifacts on disk
//! and that subsequent loads hit the cache (faster second load).

use flow_like_wasm::engine::{WasmConfig, WasmEngine};
use std::path::PathBuf;
use tempfile::TempDir;

fn templates_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("templates")
}

fn rust_template_path() -> PathBuf {
    templates_root().join(
        "wasm-node-rust/target/wasm32-unknown-unknown/release/flow_like_wasm_node_template.wasm",
    )
}

fn as_template_path() -> PathBuf {
    templates_root().join("wasm-node-assemblyscript/build/release.wasm")
}

fn go_template_path() -> PathBuf {
    templates_root().join("wasm-node-go/node.wasm")
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

#[cfg(feature = "component-model")]
fn python_template_path() -> PathBuf {
    templates_root().join("wasm-node-python/build/node.wasm")
}

#[cfg(feature = "component-model")]
fn typescript_template_path() -> PathBuf {
    templates_root().join("wasm-node-typescript/build/node.wasm")
}

fn walkdir(dir: &std::path::Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += walkdir(&path);
            } else {
                count += 1;
            }
        }
    }
    count
}

fn file_count_in(dir: &std::path::Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    walkdir(dir)
}

fn list_all_files(dir: &std::path::Path, depth: usize) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let indent = "  ".repeat(depth);
            if path.is_dir() {
                println!(
                    "{indent}[dir] {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                list_all_files(&path, depth + 1);
            } else {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                println!(
                    "{indent}{} ({size} bytes)",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
            }
        }
    }
}

#[tokio::test]
async fn test_cranelift_cache_produces_artifacts() {
    let wasm_path = rust_template_path();
    if !wasm_path.exists() {
        eprintln!(
            "Skipping test: Rust template not built. Run: cd templates/wasm-node-rust && cargo build --release --target wasm32-unknown-unknown"
        );
        return;
    }

    let cache_dir = TempDir::new().expect("Failed to create temp dir");
    let config = WasmConfig::default().with_cache_dir(cache_dir.path());

    let files_before = file_count_in(cache_dir.path());
    let engine = WasmEngine::new(config).expect("Failed to create engine");

    let files_after_init = file_count_in(cache_dir.path());

    // Print cache.toml for debugging
    let cache_toml = cache_dir.path().join("cache.toml");
    if cache_toml.exists() {
        let contents = std::fs::read_to_string(&cache_toml).unwrap_or_default();
        println!("cache.toml contents:\n{contents}");
    } else {
        println!("cache.toml does NOT exist");
    }

    let _module = engine
        .load_module_from_file(&wasm_path)
        .await
        .expect("Failed to load module");

    // Worker thread writes cache artifacts asynchronously; give it time
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let files_after_load = file_count_in(cache_dir.path());
    println!("Files: before={files_before}, after_init={files_after_init}, after_load={files_after_load}");
    list_all_files(cache_dir.path(), 0);

    assert!(
        files_after_load > files_after_init,
        "Expected new cranelift cache artifacts after compilation. Before engine: {files_before}, after engine init: {files_after_init}, after load: {files_after_load}"
    );
}

#[tokio::test]
async fn test_cranelift_cache_second_load_faster() {
    let wasm_path = rust_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust template not built.");
        return;
    }

    let cache_dir = TempDir::new().expect("Failed to create temp dir");
    let config = WasmConfig::default().with_cache_dir(cache_dir.path());

    // First load — cold compilation
    let engine1 = WasmEngine::new(config.clone()).expect("Failed to create engine");
    let start1 = std::time::Instant::now();
    let _module1 = engine1
        .load_module_from_file(&wasm_path)
        .await
        .expect("Failed first load");
    let cold_duration = start1.elapsed();

    // Second load — new engine, same cache dir → should hit disk cache
    let engine2 = WasmEngine::new(config).expect("Failed to create engine");
    engine2.clear_cache(); // clear in-memory cache
    let start2 = std::time::Instant::now();
    let _module2 = engine2
        .load_module_from_file(&wasm_path)
        .await
        .expect("Failed second load");
    let warm_duration = start2.elapsed();

    println!(
        "Cold: {:?}, Warm: {:?}, Speedup: {:.1}x",
        cold_duration,
        warm_duration,
        cold_duration.as_secs_f64() / warm_duration.as_secs_f64()
    );

    assert!(
        warm_duration < cold_duration,
        "Cached load ({warm_duration:?}) should be faster than cold compilation ({cold_duration:?})"
    );
}

#[tokio::test]
async fn test_cranelift_cache_works_for_assemblyscript() {
    let wasm_path = as_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: AssemblyScript template not built.");
        return;
    }

    let cache_dir = TempDir::new().expect("Failed to create temp dir");
    let config = WasmConfig::default().with_cache_dir(cache_dir.path());
    let engine = WasmEngine::new(config).expect("Failed to create engine");

    let _module = engine
        .load_module_from_file(&wasm_path)
        .await
        .expect("Failed to load AS module");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let count = file_count_in(cache_dir.path());
    assert!(
        count > 1,
        "Expected cranelift cache artifacts for AS module, found {count} files"
    );
}

#[tokio::test]
async fn test_cranelift_cache_works_for_go() {
    let wasm_path = go_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Go template not built.");
        return;
    }

    let cache_dir = TempDir::new().expect("Failed to create temp dir");
    let config = WasmConfig::default().with_cache_dir(cache_dir.path());
    let engine = WasmEngine::new(config).expect("Failed to create engine");

    let _module = engine
        .load_module_from_file(&wasm_path)
        .await
        .expect("Failed to load Go module");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let count = file_count_in(cache_dir.path());
    assert!(
        count > 1,
        "Expected cranelift cache artifacts for Go module, found {count} files"
    );
}

#[tokio::test]
async fn test_cranelift_cache_works_for_nim() {
    let wasm_path = nim_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Nim template not built. Run: cd templates/wasm-node-nim && nimble build");
        return;
    }

    let cache_dir = TempDir::new().expect("Failed to create temp dir");
    let config = WasmConfig::default().with_cache_dir(cache_dir.path());
    let engine = WasmEngine::new(config).expect("Failed to create engine");

    let _module = engine
        .load_module_from_file(&wasm_path)
        .await
        .expect("Failed to load Nim module");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let count = file_count_in(cache_dir.path());
    assert!(
        count > 1,
        "Expected cranelift cache artifacts for Nim module, found {count} files"
    );
}

#[tokio::test]
async fn test_cranelift_cache_works_for_grain() {
    let wasm_path = grain_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Grain template not built. Run: cd templates/wasm-node-grain && mise run build");
        return;
    }

    let cache_dir = TempDir::new().expect("Failed to create temp dir");
    let config = WasmConfig::default().with_cache_dir(cache_dir.path());
    let engine = WasmEngine::new(config).expect("Failed to create engine");

    let _module = engine
        .load_module_from_file(&wasm_path)
        .await
        .expect("Failed to load Grain module");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let count = file_count_in(cache_dir.path());
    assert!(
        count > 1,
        "Expected cranelift cache artifacts for Grain module, found {count} files"
    );
}

#[tokio::test]
async fn test_cranelift_cache_works_for_moonbit() {
    let wasm_path = moonbit_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: MoonBit template not built. Run: cd templates/wasm-node-moonbit && mise run build");
        return;
    }

    let cache_dir = TempDir::new().expect("Failed to create temp dir");
    let config = WasmConfig::default().with_cache_dir(cache_dir.path());
    let engine = WasmEngine::new(config).expect("Failed to create engine");

    let _module = engine
        .load_module_from_file(&wasm_path)
        .await
        .expect("Failed to load MoonBit module");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let count = file_count_in(cache_dir.path());
    assert!(
        count > 1,
        "Expected cranelift cache artifacts for MoonBit module, found {count} files"
    );
}

#[cfg(feature = "component-model")]
#[tokio::test]
async fn test_cranelift_cache_works_for_component_python() {
    let wasm_path = python_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Python template not built.");
        return;
    }

    let cache_dir = TempDir::new().expect("Failed to create temp dir");
    let config = WasmConfig::default().with_cache_dir(cache_dir.path());
    let engine = WasmEngine::new(config).expect("Failed to create engine");

    let _loaded = engine
        .load_auto_from_file(&wasm_path)
        .await
        .expect("Failed to load Python component");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let count = file_count_in(cache_dir.path());
    assert!(
        count > 1,
        "Expected cranelift cache artifacts for Python component, found {count} files"
    );
}

#[cfg(feature = "component-model")]
#[tokio::test]
async fn test_cranelift_cache_works_for_component_typescript() {
    let wasm_path = typescript_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: TypeScript template not built.");
        return;
    }

    let cache_dir = TempDir::new().expect("Failed to create temp dir");
    let config = WasmConfig::default().with_cache_dir(cache_dir.path());
    let engine = WasmEngine::new(config).expect("Failed to create engine");

    let _loaded = engine
        .load_auto_from_file(&wasm_path)
        .await
        .expect("Failed to load TypeScript component");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let count = file_count_in(cache_dir.path());
    assert!(
        count > 1,
        "Expected cranelift cache artifacts for TypeScript component, found {count} files"
    );
}

#[tokio::test]
async fn test_no_cache_when_disabled() {
    let wasm_path = rust_template_path();
    if !wasm_path.exists() {
        eprintln!("Skipping test: Rust template not built.");
        return;
    }

    let config = WasmConfig::development(); // cache_dir: None
    let engine = WasmEngine::new(config).expect("Failed to create engine");

    let _module = engine
        .load_module_from_file(&wasm_path)
        .await
        .expect("Failed to load module");

    // development config has no cache_dir — nothing should be written
    // (this just verifies it doesn't crash)
}
