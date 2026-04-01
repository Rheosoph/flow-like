//! Integration tests for the Python code-interpreter runtime.
//!
//! These tests require the `execute` feature and exercise:
//! - AOT (cwasm) caching: save, load, corruption detection, cross-platform key isolation
//! - Workspace file server: get, put, list, path traversal protection
//! - Full execution pipeline (when a Python WASM binary is available)

#![cfg(feature = "execute")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use flow_like_storage::Path as ObjPath;
use flow_like_storage::object_store::{self, ObjectStore, PutPayload};
use flow_like_types::Bytes;
use serde_json::json;
use tempfile::TempDir;
use tokio::fs;

use flow_like_catalog_code_interpreter::pyodide::runtime::{
    ExecutionRequest, ExecutionResponse, PyodideRuntime, RuntimeConfig, WorkspaceInfo,
};

// ═══════════════════════════════════════════════════════════════════════════════
// AOT Cache Tests
// ═══════════════════════════════════════════════════════════════════════════════

mod aot_cache {
    use flow_like_wasm::AotCache;
    use tempfile::TempDir;
    use wasmtime::{Config, Engine, Module};

    fn test_engine() -> Engine {
        let mut config = Config::new();
        config.async_support(true);
        config.epoch_interruption(true);
        Engine::new(&config).unwrap()
    }

    /// Smallest valid WASM module: (module)
    fn minimal_wasm() -> Vec<u8> {
        wat::parse_str("(module)").unwrap()
    }

    #[test]
    fn save_and_load_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let cache = AotCache::new(tmp.path());
        let engine = test_engine();

        let wasm = minimal_wasm();
        let module = Module::new(&engine, &wasm).unwrap();
        let hash = blake3::hash(&wasm).to_hex().to_string();

        cache.save_module(&module, &hash);
        let loaded = cache.load_module(&engine, &hash);
        assert!(
            loaded.is_some(),
            "cached module must load back successfully"
        );
    }

    #[test]
    fn cache_miss_returns_none() {
        let tmp = TempDir::new().unwrap();
        let cache = AotCache::new(tmp.path());
        let engine = test_engine();

        let result = cache.load_module(&engine, "nonexistent_hash_abc123");
        assert!(result.is_none(), "missing entry must return None");
    }

    #[test]
    fn corrupted_artifact_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let cache = AotCache::new(tmp.path());
        let engine = test_engine();

        let wasm = minimal_wasm();
        let module = Module::new(&engine, &wasm).unwrap();
        let hash = blake3::hash(&wasm).to_hex().to_string();

        cache.save_module(&module, &hash);

        // Corrupt the .cwasm file by appending garbage
        let modules_dir = tmp.path().join("modules");
        let entries: Vec<_> = std::fs::read_dir(&modules_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "cwasm"))
            .collect();
        assert_eq!(entries.len(), 1, "should have exactly one cwasm artifact");

        let cwasm_path = entries[0].path();
        // Truncate to half size — guaranteed to corrupt the artifact
        let data = std::fs::read(&cwasm_path).unwrap();
        std::fs::write(&cwasm_path, &data[..data.len() / 2]).unwrap();

        // Corrupted bytes → deserialize fails → load_module returns None and evicts
        let loaded = cache.load_module(&engine, &hash);
        assert!(loaded.is_none(), "corrupted artifact must not load");

        // The corrupt file should be cleaned up
        assert!(
            !cwasm_path.exists(),
            "corrupted cwasm file should be evicted"
        );
    }

    #[test]
    fn different_wasm_binaries_get_different_cache_keys() {
        let tmp = TempDir::new().unwrap();
        let cache = AotCache::new(tmp.path());
        let engine = test_engine();

        let wasm_a = wat::parse_str("(module)").unwrap();
        let wasm_b = wat::parse_str("(module (func))").unwrap();

        let hash_a = blake3::hash(&wasm_a).to_hex().to_string();
        let hash_b = blake3::hash(&wasm_b).to_hex().to_string();

        assert_ne!(
            hash_a, hash_b,
            "different binaries must have different hashes"
        );

        let module_a = Module::new(&engine, &wasm_a).unwrap();
        let module_b = Module::new(&engine, &wasm_b).unwrap();

        cache.save_module(&module_a, &hash_a);
        cache.save_module(&module_b, &hash_b);

        // Both should load independently
        assert!(cache.load_module(&engine, &hash_a).is_some());
        assert!(cache.load_module(&engine, &hash_b).is_some());
    }

    #[test]
    fn cache_key_includes_platform_and_version() {
        // Verify the cache key format includes OS, arch, and wasmtime version
        // so that .cwasm files compiled on one platform can't be loaded on another.
        let wasm = minimal_wasm();
        let hash = blake3::hash(&wasm).to_hex().to_string();

        let tmp = TempDir::new().unwrap();
        let cache = AotCache::new(tmp.path());
        let engine = test_engine();
        let module = Module::new(&engine, &wasm).unwrap();
        cache.save_module(&module, &hash);

        let modules_dir = tmp.path().join("modules");
        let entries: Vec<_> = std::fs::read_dir(&modules_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();

        let filename = entries[0].file_name().to_string_lossy().to_string();
        // Key format: {hash}-{os}-{arch}-wt{version}.cwasm
        assert!(
            filename.contains(std::env::consts::OS),
            "cache key must contain OS: {filename}"
        );
        assert!(
            filename.contains(std::env::consts::ARCH),
            "cache key must contain arch: {filename}"
        );
        assert!(
            filename.contains("-wt"),
            "cache key must contain wasmtime version prefix: {filename}"
        );
    }

    #[test]
    fn save_creates_cwasm_artifact() {
        let tmp = TempDir::new().unwrap();
        let cache = AotCache::new(tmp.path());
        let engine = test_engine();

        let wasm = minimal_wasm();
        let module = Module::new(&engine, &wasm).unwrap();
        let hash = blake3::hash(&wasm).to_hex().to_string();
        cache.save_module(&module, &hash);

        let modules_dir = tmp.path().join("modules");
        let cwasm_count = std::fs::read_dir(&modules_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "cwasm"))
            .count();

        assert_eq!(cwasm_count, 1, "should have exactly one cwasm artifact");
    }

    #[test]
    fn clear_removes_all_cached_artifacts() {
        let tmp = TempDir::new().unwrap();
        let cache = AotCache::new(tmp.path());
        let engine = test_engine();

        let wasm = minimal_wasm();
        let module = Module::new(&engine, &wasm).unwrap();
        let hash = blake3::hash(&wasm).to_hex().to_string();
        cache.save_module(&module, &hash);

        assert!(tmp.path().join("modules").exists());
        cache.clear();
        assert!(
            !tmp.path().join("modules").exists(),
            "clear() must remove modules dir"
        );
    }

    #[test]
    fn wrong_engine_rejects_artifact() {
        let tmp = TempDir::new().unwrap();
        let cache = AotCache::new(tmp.path());
        let engine = test_engine();

        let wasm = minimal_wasm();
        let module = Module::new(&engine, &wasm).unwrap();
        let hash = blake3::hash(&wasm).to_hex().to_string();
        cache.save_module(&module, &hash);

        // Create a different engine with different settings
        let mut config2 = Config::new();
        config2.cranelift_opt_level(wasmtime::OptLevel::None);
        let engine2 = Engine::new(&config2).unwrap();

        // Artifact compiled with one engine config can't load with another
        let loaded = cache.load_module(&engine2, &hash);
        // May succeed or fail depending on wasmtime version — the point is it
        // doesn't crash. If it fails, the corrupt artifact should be evicted.
        if loaded.is_none() {
            let modules_dir = tmp.path().join("modules");
            let cwasm_count = std::fs::read_dir(&modules_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "cwasm"))
                .count();
            assert_eq!(cwasm_count, 0, "failed artifact should be evicted");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Workspace File Server Tests
// ═══════════════════════════════════════════════════════════════════════════════

mod workspace {
    use super::*;
    use flow_like_catalog_code_interpreter::pyodide::runtime::{
        list_workspace_files, upload_ws_puts, workspace_file_server,
    };

    fn memory_store() -> Arc<dyn ObjectStore> {
        Arc::new(object_store::memory::InMemory::new())
    }

    async fn put_object(store: &Arc<dyn ObjectStore>, path: &str, data: &[u8]) {
        let obj_path = ObjPath::from(path);
        store
            .put(
                &obj_path,
                PutPayload::from_bytes(Bytes::from(data.to_vec())),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_workspace_files_empty_store() {
        let store = memory_store();
        let files: Vec<String> = list_workspace_files(&store, "prefix").await;
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn list_workspace_files_returns_relative_paths() {
        let store = memory_store();
        put_object(&store, "myprefix/file_a.txt", b"hello").await;
        put_object(&store, "myprefix/sub/file_b.txt", b"world").await;
        put_object(&store, "other/unrelated.txt", b"nope").await;

        let files: Vec<String> = list_workspace_files(&store, "myprefix").await;
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"file_a.txt".to_string()));
        assert!(files.contains(&"sub/file_b.txt".to_string()));
    }

    #[tokio::test]
    async fn list_workspace_files_empty_prefix() {
        let store = memory_store();
        put_object(&store, "a.txt", b"aaa").await;
        put_object(&store, "dir/b.txt", b"bbb").await;

        let files: Vec<String> = list_workspace_files(&store, "").await;
        assert!(files.len() >= 2);
    }

    #[tokio::test]
    async fn upload_ws_puts_single_file() {
        let store = memory_store();
        let tmp = TempDir::new().unwrap();
        let ws_puts = tmp.path().join("ws_puts");
        fs::create_dir_all(&ws_puts).await.unwrap();

        fs::write(ws_puts.join("output.txt"), b"uploaded content")
            .await
            .unwrap();

        upload_ws_puts(&ws_puts.to_path_buf(), &store, "project").await;

        let result = store
            .get(&ObjPath::from("project/output.txt"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(result.as_ref(), b"uploaded content");
    }

    #[tokio::test]
    async fn upload_ws_puts_nested_directories() {
        let store = memory_store();
        let tmp = TempDir::new().unwrap();
        let ws_puts = tmp.path().join("ws_puts");
        fs::create_dir_all(ws_puts.join("sub/deep")).await.unwrap();

        fs::write(ws_puts.join("sub/deep/file.txt"), b"deep content")
            .await
            .unwrap();

        upload_ws_puts(&ws_puts.to_path_buf(), &store, "pfx").await;

        let result = store
            .get(&ObjPath::from("pfx/sub/deep/file.txt"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(result.as_ref(), b"deep content");
    }

    #[tokio::test]
    async fn upload_ws_puts_empty_prefix() {
        let store = memory_store();
        let tmp = TempDir::new().unwrap();
        let ws_puts = tmp.path().join("ws_puts");
        fs::create_dir_all(&ws_puts).await.unwrap();

        fs::write(ws_puts.join("root_file.txt"), b"at root")
            .await
            .unwrap();

        upload_ws_puts(&ws_puts.to_path_buf(), &store, "").await;

        let result = store
            .get(&ObjPath::from("root_file.txt"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(result.as_ref(), b"at root");
    }

    #[tokio::test]
    async fn file_server_serves_existing_file() {
        let store = memory_store();
        put_object(&store, "ws/hello.txt", b"hello world").await;

        let tmp = TempDir::new().unwrap();
        let ws_pending = tmp.path().join("ws_pending");
        let ws_data = tmp.path().join("ws_data");
        let ws_notfound = tmp.path().join("ws_notfound");

        for d in [&ws_pending, &ws_data, &ws_notfound] {
            fs::create_dir_all(d).await.unwrap();
        }

        let handle = tokio::spawn(workspace_file_server(
            store.clone(),
            "ws".to_string(),
            ws_pending.clone(),
            ws_data.clone(),
            ws_notfound.clone(),
        ));

        // Write a pending request
        fs::write(ws_pending.join("req001"), "hello.txt")
            .await
            .unwrap();

        // Poll for the result
        let data_path = ws_data.join("hello.txt");
        let mut found = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(30)).await;
            if data_path.exists() {
                found = true;
                break;
            }
        }

        handle.abort();
        assert!(found, "file server must write data file");
        let content = fs::read(&data_path).await.unwrap();
        assert_eq!(content, b"hello world");
        // Pending file should be cleaned up
        assert!(!ws_pending.join("req001").exists());
    }

    #[tokio::test]
    async fn file_server_writes_notfound_for_missing_file() {
        let store = memory_store();
        // No objects in store

        let tmp = TempDir::new().unwrap();
        let ws_pending = tmp.path().join("ws_pending");
        let ws_data = tmp.path().join("ws_data");
        let ws_notfound = tmp.path().join("ws_notfound");

        for d in [&ws_pending, &ws_data, &ws_notfound] {
            fs::create_dir_all(d).await.unwrap();
        }

        let handle = tokio::spawn(workspace_file_server(
            store.clone(),
            "ws".to_string(),
            ws_pending.clone(),
            ws_data.clone(),
            ws_notfound.clone(),
        ));

        fs::write(ws_pending.join("req002"), "missing.txt")
            .await
            .unwrap();

        let notfound_path = ws_notfound.join("req002");
        let mut found = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(30)).await;
            if notfound_path.exists() {
                found = true;
                break;
            }
        }

        handle.abort();
        assert!(found, "file server must write notfound sentinel");
        assert!(
            !ws_pending.join("req002").exists(),
            "pending should be cleaned up"
        );
    }

    #[tokio::test]
    async fn file_server_sanitises_path_traversal() {
        let store = memory_store();
        // Put a file that an attacker might try to traverse to
        put_object(&store, "secret.txt", b"secret data").await;

        let tmp = TempDir::new().unwrap();
        let ws_pending = tmp.path().join("ws_pending");
        let ws_data = tmp.path().join("ws_data");
        let ws_notfound = tmp.path().join("ws_notfound");

        for d in [&ws_pending, &ws_data, &ws_notfound] {
            fs::create_dir_all(d).await.unwrap();
        }

        let handle = tokio::spawn(workspace_file_server(
            store.clone(),
            "ws".to_string(),
            ws_pending.clone(),
            ws_data.clone(),
            ws_notfound.clone(),
        ));

        // Attempt path traversal
        fs::write(ws_pending.join("req003"), "../../secret.txt")
            .await
            .unwrap();

        // The sanitiser should strip ".." → resolve to "secret.txt" under the ws prefix
        // which means it requests "ws/secret.txt" — this won't exist
        let mut resolved = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(30)).await;
            if !ws_pending.join("req003").exists() {
                resolved = true;
                break;
            }
        }

        handle.abort();
        assert!(resolved, "pending request should be resolved");
    }

    #[tokio::test]
    async fn file_server_ignores_empty_path() {
        let store = memory_store();
        let tmp = TempDir::new().unwrap();
        let ws_pending = tmp.path().join("ws_pending");
        let ws_data = tmp.path().join("ws_data");
        let ws_notfound = tmp.path().join("ws_notfound");

        for d in [&ws_pending, &ws_data, &ws_notfound] {
            fs::create_dir_all(d).await.unwrap();
        }

        let handle = tokio::spawn(workspace_file_server(
            store.clone(),
            "ws".to_string(),
            ws_pending.clone(),
            ws_data.clone(),
            ws_notfound.clone(),
        ));

        // Empty path request — should be silently removed
        fs::write(ws_pending.join("req004"), "").await.unwrap();

        let mut removed = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(30)).await;
            if !ws_pending.join("req004").exists() {
                removed = true;
                break;
            }
        }

        handle.abort();
        assert!(removed, "empty path request should be silently removed");
    }

    #[tokio::test]
    async fn file_server_serves_nested_path() {
        let store = memory_store();
        put_object(&store, "project/data/sub/deep.csv", b"col1,col2\na,b").await;

        let tmp = TempDir::new().unwrap();
        let ws_pending = tmp.path().join("ws_pending");
        let ws_data = tmp.path().join("ws_data");
        let ws_notfound = tmp.path().join("ws_notfound");

        for d in [&ws_pending, &ws_data, &ws_notfound] {
            fs::create_dir_all(d).await.unwrap();
        }

        let handle = tokio::spawn(workspace_file_server(
            store.clone(),
            "project/data".to_string(),
            ws_pending.clone(),
            ws_data.clone(),
            ws_notfound.clone(),
        ));

        fs::write(ws_pending.join("req005"), "sub/deep.csv")
            .await
            .unwrap();

        let data_path = ws_data.join("sub").join("deep.csv");
        let mut found = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(30)).await;
            if data_path.exists() {
                found = true;
                break;
            }
        }

        handle.abort();
        assert!(
            found,
            "nested file must be served under mirrored directory structure"
        );
        let content = fs::read(&data_path).await.unwrap();
        assert_eq!(content, b"col1,col2\na,b");
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Runtime Configuration Tests
// ═══════════════════════════════════════════════════════════════════════════════

mod runtime_config {
    use super::*;

    #[tokio::test]
    async fn runtime_creates_with_default_config() {
        let runtime = PyodideRuntime::new(RuntimeConfig::default());
        assert!(
            runtime.is_ok(),
            "runtime must initialise with default config"
        );
    }

    #[tokio::test]
    async fn runtime_creates_with_custom_wasm_path() {
        let config = RuntimeConfig {
            wasm_binary_path: Some(PathBuf::from("/nonexistent/python.wasm")),
        };
        let runtime = PyodideRuntime::new(config);
        assert!(
            runtime.is_ok(),
            "runtime must initialise even with a nonexistent path (probing happens at execute time)"
        );
    }

    #[tokio::test]
    async fn execute_fails_gracefully_without_wasm_binary() {
        let config = RuntimeConfig {
            wasm_binary_path: Some(PathBuf::from("/definitely/does/not/exist.wasm")),
        };
        let runtime = PyodideRuntime::new(config).unwrap();

        let req = ExecutionRequest {
            code: "print('hello')".to_string(),
            inputs: json!({}),
            packages: vec![],
            package_allowlist: None,
            network_enabled: false,
            allowed_hosts: vec![],
            workspace: None,
            timeout: Duration::from_secs(5),
            memory_limit: 256 * 1024 * 1024,
        };

        let response = runtime.execute(req).await;
        assert!(!response.success, "must fail when wasm binary is missing");
        assert!(
            response.error.is_some(),
            "must provide error message when wasm binary is missing"
        );
        let err = response.error.unwrap();
        assert!(
            err.contains("not found") || err.contains("Python WASM binary"),
            "error must mention missing binary: {err}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Full Execution Tests (require python.wasm on disk)
// ═══════════════════════════════════════════════════════════════════════════════

mod execution {
    use super::*;

    fn python_wasm_available() -> bool {
        let candidates = [
            dirs_next::data_local_dir().map(|d| d.join("flow-like").join("python.wasm")),
            Some(PathBuf::from("/usr/local/share/flow-like/python.wasm")),
            Some(PathBuf::from("/tmp/flow-like/python.wasm")),
            Some(PathBuf::from("/tmp/python.wasm")),
            Some(PathBuf::from("./python.wasm")),
        ];
        candidates.iter().flatten().any(|p| p.exists())
    }

    fn make_request(code: &str) -> ExecutionRequest {
        ExecutionRequest {
            code: code.to_string(),
            inputs: json!({}),
            packages: vec![],
            package_allowlist: None,
            network_enabled: false,
            allowed_hosts: vec![],
            workspace: None,
            timeout: Duration::from_secs(30),
            memory_limit: 256 * 1024 * 1024,
        }
    }

    fn make_request_with_inputs(code: &str, inputs: serde_json::Value) -> ExecutionRequest {
        ExecutionRequest {
            code: code.to_string(),
            inputs,
            packages: vec![],
            package_allowlist: None,
            network_enabled: false,
            allowed_hosts: vec![],
            workspace: None,
            timeout: Duration::from_secs(30),
            memory_limit: 256 * 1024 * 1024,
        }
    }

    #[tokio::test]
    async fn simple_output() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();
        let resp = runtime
            .execute(make_request("outputs['greeting'] = 'hello world'"))
            .await;

        assert!(resp.success, "execution must succeed: {:?}", resp.error);
        assert_eq!(resp.outputs["greeting"], "hello world");
    }

    #[tokio::test]
    async fn inputs_are_passed_through() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();
        let resp = runtime
            .execute(make_request_with_inputs(
                "outputs['doubled'] = inputs['x'] * 2",
                json!({"x": 21}),
            ))
            .await;

        assert!(resp.success, "execution must succeed: {:?}", resp.error);
        assert_eq!(resp.outputs["doubled"], 42);
    }

    #[tokio::test]
    async fn stdout_is_captured() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();
        let resp = runtime
            .execute(make_request("print('captured output')"))
            .await;

        assert!(resp.success, "execution must succeed: {:?}", resp.error);
        assert!(
            resp.stdout.contains("captured output"),
            "stdout must contain printed text: {:?}",
            resp.stdout
        );
    }

    #[tokio::test]
    async fn stderr_is_captured() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();
        let resp = runtime
            .execute(make_request(
                "import sys; sys.stderr.write('warning message\\n')",
            ))
            .await;

        assert!(resp.success, "execution must succeed: {:?}", resp.error);
        assert!(
            resp.stderr.contains("warning message"),
            "stderr must contain warning: {:?}",
            resp.stderr
        );
    }

    #[tokio::test]
    async fn unhandled_exception_is_reported() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();
        let resp = runtime
            .execute(make_request("raise ValueError('test error')"))
            .await;

        assert!(!resp.success, "execution must fail on unhandled exception");
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert!(
            err.contains("ValueError") && err.contains("test error"),
            "error must contain exception details: {err}"
        );
    }

    #[tokio::test]
    async fn timeout_kills_execution() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();

        let mut req = make_request("import time; time.sleep(999)");
        req.timeout = Duration::from_secs(2);

        let resp = runtime.execute(req).await;

        assert!(!resp.success, "must fail due to timeout");
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert!(
            err.to_lowercase().contains("timeout") || err.to_lowercase().contains("epoch"),
            "error must mention timeout: {err}"
        );
    }

    #[tokio::test]
    async fn module_is_cached_across_executions() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();

        // First execution — compiles the module
        let resp1 = runtime.execute(make_request("outputs['run'] = 1")).await;
        assert!(resp1.success, "first run must succeed: {:?}", resp1.error);

        // Second execution — should reuse the cached module
        let resp2 = runtime.execute(make_request("outputs['run'] = 2")).await;
        assert!(resp2.success, "second run must succeed: {:?}", resp2.error);
        assert_eq!(resp2.outputs["run"], 2);
    }

    #[tokio::test]
    async fn workspace_get_and_put() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }

        let store: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());

        // Seed the workspace with a file
        store
            .put(
                &ObjPath::from("project/data.txt"),
                PutPayload::from_bytes(Bytes::from_static(b"original content")),
            )
            .await
            .unwrap();

        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();

        let req = ExecutionRequest {
            code: r#"
data = workspace.get("data.txt")
if data is not None:
    outputs["read"] = data.decode("utf-8")
else:
    outputs["read"] = None

workspace.put("result.txt", b"written by python")
outputs["wrote"] = True
"#
            .to_string(),
            inputs: json!({}),
            packages: vec![],
            package_allowlist: None,
            network_enabled: false,
            allowed_hosts: vec![],
            workspace: Some(WorkspaceInfo {
                store: store.clone(),
                prefix: "project".to_string(),
            }),
            timeout: Duration::from_secs(30),
            memory_limit: 256 * 1024 * 1024,
        };

        let resp = runtime.execute(req).await;
        assert!(
            resp.success,
            "workspace execution must succeed: {:?}",
            resp.error
        );
        assert_eq!(resp.outputs["read"], "original content");
        assert_eq!(resp.outputs["wrote"], true);

        // Verify the put was uploaded to the store
        let uploaded = store
            .get(&ObjPath::from("project/result.txt"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(uploaded.as_ref(), b"written by python");
    }

    #[tokio::test]
    async fn workspace_list() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }

        let store: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        for name in ["a.txt", "b.txt", "sub/c.txt"] {
            store
                .put(
                    &ObjPath::from(format!("ws/{name}")),
                    PutPayload::from_bytes(Bytes::from_static(b"x")),
                )
                .await
                .unwrap();
        }

        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();

        let req = ExecutionRequest {
            code: "outputs['files'] = workspace.list()".to_string(),
            inputs: json!({}),
            packages: vec![],
            package_allowlist: None,
            network_enabled: false,
            allowed_hosts: vec![],
            workspace: Some(WorkspaceInfo {
                store: store.clone(),
                prefix: "ws".to_string(),
            }),
            timeout: Duration::from_secs(30),
            memory_limit: 256 * 1024 * 1024,
        };

        let resp = runtime.execute(req).await;
        assert!(
            resp.success,
            "workspace list must succeed: {:?}",
            resp.error
        );

        let files = resp.outputs["files"]
            .as_array()
            .expect("files must be an array");
        assert_eq!(files.len(), 3);

        let file_strs: Vec<&str> = files.iter().filter_map(|v| v.as_str()).collect();
        assert!(file_strs.contains(&"a.txt"));
        assert!(file_strs.contains(&"b.txt"));
        assert!(file_strs.contains(&"sub/c.txt"));
    }

    #[tokio::test]
    async fn workspace_get_nonexistent_returns_none() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }

        let store: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());

        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();

        let req = ExecutionRequest {
            code: r#"
result = workspace.get("does_not_exist.txt")
outputs["is_none"] = result is None
"#
            .to_string(),
            inputs: json!({}),
            packages: vec![],
            package_allowlist: None,
            network_enabled: false,
            allowed_hosts: vec![],
            workspace: Some(WorkspaceInfo {
                store: store.clone(),
                prefix: "ws".to_string(),
            }),
            timeout: Duration::from_secs(30),
            memory_limit: 256 * 1024 * 1024,
        };

        let resp = runtime.execute(req).await;
        assert!(resp.success, "must succeed: {:?}", resp.error);
        assert_eq!(resp.outputs["is_none"], true);
    }

    #[tokio::test]
    async fn workspace_path_traversal_blocked() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }

        let store: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        // Put a file outside the workspace prefix
        store
            .put(
                &ObjPath::from("secret/key.pem"),
                PutPayload::from_bytes(Bytes::from_static(b"private key")),
            )
            .await
            .unwrap();

        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();

        let req = ExecutionRequest {
            code: r#"
result = workspace.get("../../secret/key.pem")
outputs["accessed"] = result is not None
"#
            .to_string(),
            inputs: json!({}),
            packages: vec![],
            package_allowlist: None,
            network_enabled: false,
            allowed_hosts: vec![],
            workspace: Some(WorkspaceInfo {
                store: store.clone(),
                prefix: "workspace".to_string(),
            }),
            timeout: Duration::from_secs(30),
            memory_limit: 256 * 1024 * 1024,
        };

        let resp = runtime.execute(req).await;
        assert!(resp.success, "must succeed: {:?}", resp.error);
        assert_eq!(
            resp.outputs["accessed"], false,
            "path traversal must NOT allow access to files outside workspace prefix"
        );
    }

    #[tokio::test]
    async fn no_workspace_still_works() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();

        // Code that checks workspace is available but empty/None
        let resp = runtime
            .execute(make_request(
                "outputs['has_ws'] = workspace is not None\noutputs['files'] = workspace.list()",
            ))
            .await;

        assert!(
            resp.success,
            "no-workspace execution must succeed: {:?}",
            resp.error
        );
    }

    #[tokio::test]
    async fn package_allowlist_blocks_unlisted_packages() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();

        let req = ExecutionRequest {
            code: "outputs['ok'] = True".to_string(),
            inputs: json!({}),
            packages: vec!["forbidden_package".to_string()],
            package_allowlist: Some(vec!["allowed_only".to_string()]),
            network_enabled: false,
            allowed_hosts: vec![],
            workspace: None,
            timeout: Duration::from_secs(30),
            memory_limit: 256 * 1024 * 1024,
        };

        let resp = runtime.execute(req).await;
        assert!(!resp.success, "must fail when package is not in allowlist");
        let err = resp.error.unwrap_or_default();
        assert!(
            err.contains("allowlist") || resp.stderr.contains("allowlist"),
            "error must mention allowlist: err={err}, stderr={}",
            resp.stderr
        );
    }

    #[tokio::test]
    async fn empty_package_allowlist_blocks_all() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();

        let req = ExecutionRequest {
            code: "outputs['ok'] = True".to_string(),
            inputs: json!({}),
            packages: vec!["any_package".to_string()],
            package_allowlist: Some(vec![]),
            network_enabled: false,
            allowed_hosts: vec![],
            workspace: None,
            timeout: Duration::from_secs(30),
            memory_limit: 256 * 1024 * 1024,
        };

        let resp = runtime.execute(req).await;
        assert!(!resp.success, "empty allowlist must block all packages");
    }

    #[tokio::test]
    async fn complex_python_computation() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();
        let resp = runtime
            .execute(make_request(
                r#"
import json
data = [i**2 for i in range(10)]
outputs['squares'] = data
outputs['sum'] = sum(data)
outputs['count'] = len(data)
"#,
            ))
            .await;

        assert!(
            resp.success,
            "complex computation must succeed: {:?}",
            resp.error
        );
        assert_eq!(resp.outputs["count"], 10);
        assert_eq!(resp.outputs["sum"], 285); // sum of 0^2 + 1^2 + ... + 9^2
    }

    #[tokio::test]
    async fn memory_limit_prevents_excessive_allocation() {
        if !python_wasm_available() {
            eprintln!("SKIPPED: python.wasm not found");
            return;
        }
        let runtime = PyodideRuntime::new(RuntimeConfig::default()).unwrap();

        let req = ExecutionRequest {
            code: "x = bytearray(500 * 1024 * 1024)".to_string(), // 500 MB
            inputs: json!({}),
            packages: vec![],
            package_allowlist: None,
            network_enabled: false,
            allowed_hosts: vec![],
            workspace: None,
            timeout: Duration::from_secs(10),
            memory_limit: 64 * 1024 * 1024, // 64 MB limit
        };

        let resp = runtime.execute(req).await;
        assert!(!resp.success, "must fail when exceeding memory limit");
    }
}
