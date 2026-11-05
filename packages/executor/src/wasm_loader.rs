//! WASM package loader for server-side execution
//!
//! Downloads pre-compiled `.cwasm` artifacts via presigned URLs provided
//! by the API, verifies blake3 checksums, and registers them into the
//! node registry so boards referencing WASM nodes can execute.

use crate::error::ExecutorError;
use crate::types::WasmPackageRef;
use flow_like::flow::board::Board;
use flow_like::flow::node::NodeLogic;
use flow_like_wasm::{
    LoadedWasm, PackagePermissions, WasmConfig, WasmEngine, WasmModule, WasmNodeLogic,
    WasmSecurityConfig,
};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::OnceCell;

pub(crate) struct WasmLoadReport {
    pub nodes: Vec<Arc<dyn NodeLogic>>,
    pub failed_package_ids: BTreeSet<String>,
}

static WASM_ENGINE: OnceCell<Arc<WasmEngine>> = OnceCell::const_new();
static WASM_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);
/// Nodes registered by one WASM package, keyed by the package cache key.
type WasmPackageNodeCache = moka::sync::Cache<String, Arc<Vec<Arc<dyn NodeLogic>>>>;

static WASM_PACKAGE_CACHE: LazyLock<WasmPackageNodeCache> = LazyLock::new(|| {
    moka::sync::Cache::builder()
        .max_capacity(256)
        .time_to_live(Duration::from_secs(30 * 60))
        .time_to_idle(Duration::from_secs(10 * 60))
        .build()
});

async fn wasm_engine() -> Result<Arc<WasmEngine>, ExecutorError> {
    WASM_ENGINE
        .get_or_try_init(|| async {
            let engine = Arc::new(WasmEngine::new(WasmConfig::default()).map_err(|e| {
                ExecutorError::Execution(format!("Failed to create WASM engine: {}", e))
            })?);
            engine.start_epoch_ticker();
            Ok::<Arc<WasmEngine>, ExecutorError>(engine)
        })
        .await
        .cloned()
}

fn package_cache_key(app_id: &str, package_id: &str, pkg_ref: &WasmPackageRef) -> String {
    // Definitions and compiled code contain no board or run state. Keep the
    // app boundary, but reuse them across boards and rotating presigned URLs.
    serde_json::json!([
        app_id,
        package_id,
        pkg_ref.version,
        pkg_ref.wasm_hash,
        pkg_ref.cwasm_checksum,
        pkg_ref.permissions,
        flow_like_wasm::aot_cache::WASM_ARTIFACT_VERSION,
    ])
    .to_string()
}

/// The package-wide security settings the manifest sets: memory and time limits and the host
/// allowlist. A ref without permissions comes from an API that does not send them and gets the
/// default limits; permissions that cannot be read refuse the package rather than widen it.
fn package_security(
    package_id: &str,
    pkg_ref: &WasmPackageRef,
) -> Result<WasmSecurityConfig, ExecutorError> {
    let Some(permissions) = &pkg_ref.permissions else {
        return Ok(WasmSecurityConfig::default());
    };
    let permissions: PackagePermissions =
        serde_json::from_value(permissions.clone()).map_err(|e| {
            ExecutorError::Execution(format!(
                "Unreadable permissions for {} v{}: {}",
                package_id, pkg_ref.version, e
            ))
        })?;
    let mut security: WasmSecurityConfig = permissions.to_security_config();
    security.limits.memory_limit =
        capped_memory(package_id, security.limits.memory_limit, memory_ceiling());
    Ok(security)
}

const DEFAULT_MEMORY_CEILING_MB: usize = 1024;

/// The most memory one package instance may get on this executor, `EXECUTOR_WASM_MAX_MEMORY_MB`.
fn memory_ceiling() -> usize {
    std::env::var("EXECUTOR_WASM_MAX_MEMORY_MB")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|mb| *mb > 0)
        .unwrap_or(DEFAULT_MEMORY_CEILING_MB)
        .saturating_mul(1024 * 1024)
}

fn capped_memory(package_id: &str, requested: usize, ceiling: usize) -> usize {
    if requested > ceiling {
        tracing::warn!(
            package_id = %package_id,
            requested_bytes = requested,
            ceiling_bytes = ceiling,
            "Package asks for more memory than this executor allows; capping it"
        );
    }
    requested.min(ceiling)
}

/// Load WASM packages from presigned URLs and return node logic instances.
///
/// For each package, downloads the pre-compiled `.cwasm` artifact and its blake3
/// checksum via presigned URLs, verifies integrity, deserializes, and extracts
/// node definitions.
pub(crate) async fn load_wasm_packages(
    app_id: &str,
    board_id: &str,
    _board_version: Option<(u32, u32, u32)>,
    wasm_packages: &HashMap<String, WasmPackageRef>,
) -> Result<WasmLoadReport, ExecutorError> {
    if wasm_packages.is_empty() {
        return Ok(WasmLoadReport {
            nodes: Vec::new(),
            failed_package_ids: BTreeSet::new(),
        });
    }

    let engine = wasm_engine().await?;
    let http = &*WASM_HTTP_CLIENT;
    let mut all_nodes: Vec<Arc<dyn NodeLogic>> = Vec::new();
    let mut failed_package_ids = BTreeSet::new();

    for (package_id, pkg_ref) in wasm_packages {
        let cache_key = package_cache_key(app_id, package_id, pkg_ref);
        if let Some(cached_nodes) = WASM_PACKAGE_CACHE.get(&cache_key) {
            tracing::debug!(
                app_id = %app_id,
                board_id = %board_id,
                package_id = %package_id,
                version = %pkg_ref.version,
                node_count = cached_nodes.len(),
                "WASM package cache hit"
            );
            all_nodes.extend(cached_nodes.iter().cloned());
            continue;
        }

        match load_single_package(&engine, http, package_id, pkg_ref).await {
            Ok(nodes) => {
                tracing::info!(
                    package_id = %package_id,
                    version = %pkg_ref.version,
                    node_count = nodes.len(),
                    "Loaded WASM package"
                );
                let nodes = Arc::new(nodes);
                WASM_PACKAGE_CACHE.insert(cache_key, nodes.clone());
                all_nodes.extend(nodes.iter().cloned());
            }
            Err(e) => {
                tracing::error!(
                    package_id = %package_id,
                    version = %pkg_ref.version,
                    error = %e,
                    "Failed to load WASM package"
                );
                failed_package_ids.insert(package_id.clone());
            }
        }
    }

    Ok(WasmLoadReport {
        nodes: all_nodes,
        failed_package_ids,
    })
}

pub(crate) fn unavailable_board_wasm_packages(
    board: &Board,
    wasm_packages: Option<&HashMap<String, WasmPackageRef>>,
    failed_package_ids: &BTreeSet<String>,
) -> Vec<String> {
    let available = match wasm_packages {
        Some(packages) if !packages.is_empty() => packages,
        _ => {
            let mut required = BTreeSet::new();
            collect_required_packages(board, &mut required);
            return required.into_iter().collect();
        }
    };

    let mut missing = BTreeSet::new();
    let mut required = BTreeSet::new();
    collect_required_packages(board, &mut required);
    for package_id in required {
        if !available.contains_key(&package_id) || failed_package_ids.contains(&package_id) {
            missing.insert(package_id);
        }
    }

    missing.into_iter().collect()
}

fn collect_required_packages(board: &Board, package_ids: &mut BTreeSet<String>) {
    for node in board.nodes.values() {
        if let Some(wasm) = &node.wasm {
            package_ids.insert(wasm.package_id.clone());
        }
    }

    for layer in board.layers.values() {
        for node in layer.nodes.values() {
            if let Some(wasm) = &node.wasm {
                package_ids.insert(wasm.package_id.clone());
            }
        }
    }
}

async fn load_single_package(
    engine: &Arc<WasmEngine>,
    http: &reqwest::Client,
    package_id: &str,
    pkg_ref: &WasmPackageRef,
) -> Result<Vec<Arc<dyn NodeLogic>>, ExecutorError> {
    let nodes = load_package_nodes(engine, http, package_id, pkg_ref).await?;
    Ok(nodes
        .into_iter()
        .map(|logic| Arc::new(logic) as Arc<dyn NodeLogic>)
        .collect())
}

/// The nodes of a bot skill's package, loaded like a board run loads them.
#[cfg(feature = "bot-tools")]
pub(crate) async fn load_skill_nodes(
    package_id: &str,
    pkg_ref: &WasmPackageRef,
) -> Result<Vec<WasmNodeLogic>, ExecutorError> {
    let engine = wasm_engine().await?;
    load_package_nodes(&engine, &WASM_HTTP_CLIENT, package_id, pkg_ref).await
}

async fn load_package_nodes(
    engine: &Arc<WasmEngine>,
    http: &reqwest::Client,
    package_id: &str,
    pkg_ref: &WasmPackageRef,
) -> Result<Vec<WasmNodeLogic>, ExecutorError> {
    let cwasm_resp = http.get(&pkg_ref.cwasm_url).send().await.map_err(|e| {
        ExecutorError::Storage(format!(
            "Failed to download cwasm for {}: {}",
            package_id,
            e.without_url()
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

    let cwasm_bytes = cwasm_resp.bytes().await.map_err(|e| {
        ExecutorError::Storage(format!("Failed to read cwasm bytes: {}", e.without_url()))
    })?;

    let actual = blake3::hash(&cwasm_bytes).to_hex().to_string();
    if actual != pkg_ref.cwasm_checksum {
        return Err(ExecutorError::Execution(format!(
            "Checksum mismatch for {} v{}: expected {}, got {}",
            package_id, pkg_ref.version, pkg_ref.cwasm_checksum, actual
        )));
    }

    let loaded = match deserialize_cwasm(engine, http, package_id, pkg_ref, &cwasm_bytes).await {
        Ok(loaded) => loaded,
        Err(deserialize_error) => {
            tracing::warn!(
                package_id = %package_id,
                version = %pkg_ref.version,
                error = %deserialize_error,
                "Failed to deserialize cwasm, falling back to raw wasm loading"
            );
            let wasm_bytes = download_raw_wasm(http, package_id, pkg_ref).await?;
            compile_raw_wasm(engine, package_id, pkg_ref, &wasm_bytes).await?
        }
    };

    let manifest_security = package_security(package_id, pkg_ref)?;
    let init_security = manifest_security.for_metadata();

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

    let nodes: Vec<WasmNodeLogic> = definitions
        .into_iter()
        .map(|def| {
            let node_security = WasmSecurityConfig::from_node_permissions(&def.permissions)
                .with_package_settings(&manifest_security);
            let logic = WasmNodeLogic::from_loaded_with_target(
                loaded.clone(),
                engine.clone(),
                node_security,
                def,
            )
            .with_package_id(package_id.to_string());
            logic
        })
        .collect();

    Ok(nodes)
}

async fn deserialize_cwasm(
    engine: &Arc<WasmEngine>,
    http: &reqwest::Client,
    package_id: &str,
    pkg_ref: &WasmPackageRef,
    cwasm_bytes: &[u8],
) -> Result<LoadedWasm, ExecutorError> {
    match unsafe { wasmtime::Module::deserialize(engine.engine(), cwasm_bytes) } {
        Ok(module) => {
            let module =
                WasmModule::from_precompiled(module, pkg_ref.wasm_hash.clone()).map_err(|e| {
                    ExecutorError::Execution(format!(
                        "Failed to build WasmModule for {} v{}: {}",
                        package_id, pkg_ref.version, e
                    ))
                })?;
            Ok(LoadedWasm::Module(Arc::new(module)))
        }
        Err(module_error) => {
            match unsafe {
                wasmtime::component::Component::deserialize(engine.engine(), cwasm_bytes)
            } {
                Ok(component) => {
                    let wasm_bytes = download_raw_wasm(http, package_id, pkg_ref).await?;
                    let component = flow_like_wasm::component::WasmComponent::from_precompiled(
                        component,
                        &wasm_bytes,
                        pkg_ref.wasm_hash.clone(),
                    )
                    .map_err(|e| {
                        ExecutorError::Execution(format!(
                            "Failed to build WasmComponent for {} v{}: {}",
                            package_id, pkg_ref.version, e
                        ))
                    })?;
                    Ok(LoadedWasm::Component(Arc::new(component)))
                }
                Err(component_error) => Err(ExecutorError::Execution(format!(
                    "Failed to deserialize cwasm for {} v{} as module ({:#}) or component ({:#})",
                    package_id, pkg_ref.version, module_error, component_error
                ))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> WasmPackageRef {
        WasmPackageRef {
            version: "1.0.0".into(),
            wasm_hash: "source-hash".into(),
            wasm_url: "https://example.test/source?token=one".into(),
            cwasm_url: "https://example.test/compiled?token=one".into(),
            cwasm_checksum: "compiled-hash".into(),
            permissions: None,
        }
    }

    #[test]
    fn package_identity_reuses_rotating_urls_but_isolates_apps_and_recompiled_artifacts() {
        let first = package();
        let key = package_cache_key("app-one", "package", &first);
        let mut refreshed = first.clone();
        refreshed.wasm_url.push_str("-new");
        refreshed.cwasm_url.push_str("-new");
        assert_eq!(key, package_cache_key("app-one", "package", &refreshed));
        assert_ne!(key, package_cache_key("app-two", "package", &first));
        assert_ne!(key, package_cache_key("app-one", "other-package", &first));
        refreshed.cwasm_checksum = "portable-rebuild".into();
        assert_ne!(key, package_cache_key("app-one", "package", &refreshed));
        refreshed = first.clone();
        refreshed.wasm_hash = "new-source".into();
        assert_ne!(key, package_cache_key("app-one", "package", &refreshed));
    }

    #[test]
    fn changed_permissions_never_reuse_nodes_built_with_the_old_limits() {
        let first = package();
        let mut stricter = first.clone();
        stricter.permissions = Some(serde_json::json!({"memory": "minimal"}));
        assert_ne!(
            package_cache_key("app", "package", &first),
            package_cache_key("app", "package", &stricter)
        );
    }

    fn with_permissions(permissions: serde_json::Value) -> WasmPackageRef {
        WasmPackageRef {
            permissions: Some(permissions),
            ..package()
        }
    }

    #[test]
    fn the_manifest_sets_the_memory_and_time_limits_and_the_allowed_hosts() {
        let security = package_security(
            "package",
            &with_permissions(serde_json::json!({
                "memory": "heavy",
                "timeout": "extended",
                "network": {"http_enabled": true, "allowed_hosts": ["api.example.com"]},
            })),
        )
        .unwrap();

        assert_eq!(security.limits.memory_limit, 128 * 1024 * 1024);
        assert_eq!(security.limits.timeout, Duration::from_secs(60));
        assert_eq!(
            security.allowed_hosts.as_deref(),
            Some(&["api.example.com".to_string()][..])
        );
    }

    #[test]
    fn node_security_takes_limits_and_hosts_from_the_manifest_and_capabilities_from_the_node() {
        use flow_like::flow::node::NodePermission;
        use flow_like_wasm::WasmCapabilities;

        let manifest = package_security(
            "package",
            &with_permissions(serde_json::json!({
                "memory": "light",
                "timeout": "quick",
                "network": {"allowed_hosts": ["api.example.com"]},
            })),
        )
        .unwrap();
        let node = WasmSecurityConfig::from_node_permissions(&[NodePermission::NetworkHttp])
            .with_package_settings(&manifest);

        assert_eq!(node.limits.memory_limit, 32 * 1024 * 1024);
        assert_eq!(node.limits.timeout, Duration::from_secs(5));
        assert_eq!(
            node.allowed_hosts.as_deref(),
            Some(&["api.example.com".to_string()][..])
        );
        assert!(node.capabilities.contains(WasmCapabilities::HTTP_ALL));
        assert_eq!(
            manifest.for_metadata().allowed_hosts.as_deref(),
            Some(&[][..])
        );
        assert_eq!(
            manifest.for_metadata().limits.memory_limit,
            32 * 1024 * 1024
        );
    }

    #[test]
    fn a_package_without_permissions_keeps_the_default_limits() {
        let security = package_security("package", &package()).unwrap();
        let default = WasmSecurityConfig::default();

        assert_eq!(security.limits.memory_limit, default.limits.memory_limit);
        assert_eq!(security.limits.timeout, default.limits.timeout);
        assert!(security.allowed_hosts.is_none());
    }

    #[test]
    fn a_memory_tier_above_the_executor_ceiling_is_capped() {
        assert_eq!(capped_memory("p", 4096 << 20, 1024 << 20), 1024 << 20);
        assert_eq!(capped_memory("p", 128 << 20, 1024 << 20), 128 << 20);
        let security = package_security(
            "package",
            &with_permissions(serde_json::json!({"memory": "maximum"})),
        )
        .unwrap();
        assert_eq!(
            security.limits.memory_limit,
            DEFAULT_MEMORY_CEILING_MB << 20
        );
    }

    #[test]
    fn unreadable_permissions_refuse_the_package() {
        for permissions in [
            serde_json::json!({"memory": "gigantic"}),
            serde_json::json!("heavy"),
            serde_json::json!({"network": {"allowed_hosts": "api.example.com"}}),
        ] {
            assert!(
                package_security("package", &with_permissions(permissions)).is_err(),
                "permissions that cannot be read must not become the defaults"
            );
        }
    }
}

async fn download_raw_wasm(
    http: &reqwest::Client,
    package_id: &str,
    pkg_ref: &WasmPackageRef,
) -> Result<Vec<u8>, ExecutorError> {
    let wasm_resp = http.get(&pkg_ref.wasm_url).send().await.map_err(|e| {
        ExecutorError::Storage(format!(
            "Failed to download raw wasm for {}: {}",
            package_id,
            e.without_url()
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

    let wasm_bytes = wasm_resp.bytes().await.map_err(|e| {
        ExecutorError::Storage(format!(
            "Failed to read raw wasm bytes: {}",
            e.without_url()
        ))
    })?;

    let actual = blake3::hash(&wasm_bytes).to_hex().to_string();
    if actual != pkg_ref.wasm_hash {
        return Err(ExecutorError::Execution(format!(
            "Checksum mismatch for raw wasm {} v{}: expected {}, got {}",
            package_id, pkg_ref.version, pkg_ref.wasm_hash, actual
        )));
    }

    Ok(wasm_bytes.to_vec())
}

async fn compile_raw_wasm(
    engine: &Arc<WasmEngine>,
    package_id: &str,
    pkg_ref: &WasmPackageRef,
    wasm_bytes: &[u8],
) -> Result<LoadedWasm, ExecutorError> {
    engine.load_auto(wasm_bytes).await.map_err(|e| {
        ExecutorError::Execution(format!(
            "Failed to compile raw wasm for {} v{}: {}",
            package_id, pkg_ref.version, e
        ))
    })
}
