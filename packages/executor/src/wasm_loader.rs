//! WASM package loader for server-side execution
//!
//! Downloads pre-compiled `.cwasm` artifacts via presigned URLs provided
//! by the API, verifies blake3 checksums, and registers them into the
//! node registry so boards referencing WASM nodes can execute.

use crate::error::ExecutorError;
use crate::types::WasmPackageRef;
use flow_like::flow::node::NodeLogic;
use flow_like_wasm::{WasmConfig, WasmEngine, WasmModule, WasmNodeLogic};
use std::collections::HashMap;
use std::sync::Arc;

/// Load WASM packages from presigned URLs and return node logic instances.
///
/// For each package, downloads the pre-compiled `.cwasm` artifact and its blake3
/// checksum via presigned URLs, verifies integrity, deserializes, and extracts
/// node definitions.
pub async fn load_wasm_packages(
    wasm_packages: &HashMap<String, WasmPackageRef>,
) -> Result<Vec<Arc<dyn NodeLogic>>, ExecutorError> {
    if wasm_packages.is_empty() {
        return Ok(Vec::new());
    }

    let engine =
        Arc::new(WasmEngine::new(WasmConfig::default()).map_err(|e| {
            ExecutorError::Execution(format!("Failed to create WASM engine: {}", e))
        })?);
    engine.start_epoch_ticker();

    let http = reqwest::Client::new();
    let mut all_nodes: Vec<Arc<dyn NodeLogic>> = Vec::new();

    for (package_id, pkg_ref) in wasm_packages {
        match load_single_package(&engine, &http, package_id, pkg_ref).await {
            Ok(nodes) => {
                tracing::info!(
                    package_id = %package_id,
                    version = %pkg_ref.version,
                    node_count = nodes.len(),
                    "Loaded WASM package"
                );
                all_nodes.extend(nodes);
            }
            Err(e) => {
                tracing::error!(
                    package_id = %package_id,
                    version = %pkg_ref.version,
                    error = %e,
                    "Failed to load WASM package — skipping"
                );
            }
        }
    }

    Ok(all_nodes)
}

async fn load_single_package(
    engine: &Arc<WasmEngine>,
    http: &reqwest::Client,
    package_id: &str,
    pkg_ref: &WasmPackageRef,
) -> Result<Vec<Arc<dyn NodeLogic>>, ExecutorError> {
    let cwasm_resp = http.get(&pkg_ref.cwasm_url).send().await.map_err(|e| {
        ExecutorError::Storage(format!(
            "Failed to download cwasm for {}: {}",
            package_id, e
        ))
    })?;

    if !cwasm_resp.status().is_success() {
        return Err(ExecutorError::Storage(format!(
            "cwasm download failed for {} v{}: HTTP {}",
            package_id,
            pkg_ref.version,
            cwasm_resp.status()
        )));
    }

    let cwasm_bytes = cwasm_resp
        .bytes()
        .await
        .map_err(|e| ExecutorError::Storage(format!("Failed to read cwasm bytes: {}", e)))?;

    let checksum_resp = http
        .get(&pkg_ref.cwasm_checksum_url)
        .send()
        .await
        .map_err(|e| {
            ExecutorError::Storage(format!(
                "Failed to download checksum for {}: {}",
                package_id, e
            ))
        })?;

    if !checksum_resp.status().is_success() {
        return Err(ExecutorError::Storage(format!(
            "checksum download failed for {} v{}: HTTP {}",
            package_id,
            pkg_ref.version,
            checksum_resp.status()
        )));
    }

    let expected_hash = checksum_resp
        .text()
        .await
        .map_err(|e| ExecutorError::Storage(format!("Failed to read checksum: {}", e)))?;

    let expected = expected_hash.trim().to_string();
    let actual = blake3::hash(&cwasm_bytes).to_hex().to_string();

    if expected != actual {
        return Err(ExecutorError::Execution(format!(
            "Checksum mismatch for {} v{}: expected {}, got {}",
            package_id, pkg_ref.version, expected, actual
        )));
    }

    let wasm_module = match deserialize_cwasm(engine, package_id, pkg_ref, &cwasm_bytes) {
        Ok(module) => module,
        Err(deserialize_error) => {
            tracing::warn!(
                package_id = %package_id,
                version = %pkg_ref.version,
                error = %deserialize_error,
                "Failed to deserialize cwasm, falling back to raw wasm compilation"
            );
            compile_raw_wasm(engine, http, package_id, pkg_ref).await?
        }
    };

    let init_security = flow_like_wasm::WasmSecurityConfig::default();
    let loaded = flow_like_wasm::LoadedWasm::Module(wasm_module);

    let mut instance = loaded
        .instantiate(engine, init_security.clone())
        .await
        .map_err(|e| {
            ExecutorError::Execution(format!(
                "Failed to instantiate {} v{}: {}",
                package_id, pkg_ref.version, e
            ))
        })?;

    let definitions = instance.call_get_nodes().await.map_err(|e| {
        ExecutorError::Execution(format!(
            "Failed to get node definitions from {} v{}: {}",
            package_id, pkg_ref.version, e
        ))
    })?;

    let nodes: Vec<Arc<dyn NodeLogic>> = definitions
        .into_iter()
        .map(|def| {
            let node_security =
                flow_like_wasm::WasmSecurityConfig::from_node_permissions(&def.permissions);
            let logic = WasmNodeLogic::from_loaded_with_target(
                loaded.clone(),
                engine.clone(),
                node_security,
                def,
            )
            .with_package_id(package_id.to_string());
            Arc::new(logic) as Arc<dyn NodeLogic>
        })
        .collect();

    Ok(nodes)
}

fn deserialize_cwasm(
    engine: &Arc<WasmEngine>,
    package_id: &str,
    pkg_ref: &WasmPackageRef,
    cwasm_bytes: &[u8],
) -> Result<Arc<WasmModule>, ExecutorError> {
    let module =
        unsafe { wasmtime::Module::deserialize(engine.engine(), cwasm_bytes) }.map_err(|e| {
            ExecutorError::Execution(format!(
                "Failed to deserialize cwasm for {} v{}: {}",
                package_id, pkg_ref.version, e
            ))
        })?;

    Ok(Arc::new(
        WasmModule::from_precompiled(module, pkg_ref.wasm_hash.clone()).map_err(|e| {
            ExecutorError::Execution(format!(
                "Failed to build WasmModule for {} v{}: {}",
                package_id, pkg_ref.version, e
            ))
        })?,
    ))
}

async fn compile_raw_wasm(
    engine: &Arc<WasmEngine>,
    http: &reqwest::Client,
    package_id: &str,
    pkg_ref: &WasmPackageRef,
) -> Result<Arc<WasmModule>, ExecutorError> {
    let wasm_resp = http.get(&pkg_ref.wasm_url).send().await.map_err(|e| {
        ExecutorError::Storage(format!(
            "Failed to download raw wasm for {}: {}",
            package_id, e
        ))
    })?;

    if !wasm_resp.status().is_success() {
        return Err(ExecutorError::Storage(format!(
            "raw wasm download failed for {} v{}: HTTP {}",
            package_id,
            pkg_ref.version,
            wasm_resp.status()
        )));
    }

    let wasm_bytes = wasm_resp
        .bytes()
        .await
        .map_err(|e| ExecutorError::Storage(format!("Failed to read raw wasm bytes: {}", e)))?;

    let actual = blake3::hash(&wasm_bytes).to_hex().to_string();
    if actual != pkg_ref.wasm_hash {
        return Err(ExecutorError::Execution(format!(
            "Checksum mismatch for raw wasm {} v{}: expected {}, got {}",
            package_id, pkg_ref.version, pkg_ref.wasm_hash, actual
        )));
    }

    Ok(Arc::new(
        WasmModule::from_bytes(engine, &wasm_bytes, pkg_ref.wasm_hash.clone())
            .await
            .map_err(|e| {
                ExecutorError::Execution(format!(
                    "Failed to compile raw wasm for {} v{}: {}",
                    package_id, pkg_ref.version, e
                ))
            })?,
    ))
}
