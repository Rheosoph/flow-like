//! Compile-only stand-ins for WASM nodes.
//!
//! Compiling a board needs each node's `Node` definition — pins, defaults,
//! permissions — and nothing else; the registry fingerprint that binds an
//! artifact to a node catalog hashes those definitions alone. For WASM nodes
//! the API already holds them: the compiler workload instantiates each
//! uploaded module in its own sandbox, calls its `get_nodes` export, and
//! reports the result back, which the compilation callback stores in
//! `wasm_package_version.nodes`. Reading that row is the API's entire contact
//! with a WASM package — it never loads module bytes, so a sandbox escape in
//! user WASM cannot reach API credentials. The logic here exists only to fill
//! the registry's `(Node, NodeLogic)` shape and refuses to run.
//!
//! The row is only as good as the callback that wrote it, but a wrong row
//! cannot make a wrong artifact run: the executor derives its nodes from the
//! real module and rejects any artifact whose fingerprint disagrees.

use flow_like::flow::execution::context::ExecutionContext;
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::state::FlowNodeRegistryInner;
use flow_like_types::{Result, anyhow, async_trait};
use std::{collections::BTreeMap, sync::Arc};

/// Include the complete normalized definitions: defaults, options and permissions
/// affect compilation even when they do not change the registry fingerprint.
fn artifact_registry_cache_key(package_revision: &str, nodes: &[Node]) -> Result<String> {
    // Match registry insertion: if packages reuse a name, the last node wins.
    let effective_nodes: BTreeMap<_, _> = nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.artifact-registry-metadata/v1\0");
    for node in effective_nodes.values() {
        let mut value = serde_json::to_value(node)?;
        value.sort_all_objects();
        let definition = serde_json::to_vec(&value)?;
        hasher.update(&(definition.len() as u64).to_le_bytes());
        hasher.update(&definition);
    }
    Ok(format!("{package_revision}:{}", hasher.finalize().to_hex()))
}

/// Reuse an overlay only when the currently loaded package metadata matches it.
/// A metadata repair can change a definition without changing its package version
/// or binary, so callers must load the definitions before consulting this cache.
pub(crate) fn cached_artifact_registry(
    base: &Arc<FlowNodeRegistryInner>,
    cache: &moka::sync::Cache<String, Arc<FlowNodeRegistryInner>>,
    package_revision: &str,
    nodes: Vec<Node>,
) -> Result<Arc<FlowNodeRegistryInner>> {
    let key = artifact_registry_cache_key(package_revision, &nodes)?;
    if let Some(registry) = cache.get(&key) {
        return Ok(registry);
    }

    let mut overlay = (**base).clone();
    for node in nodes {
        let logic = WasmNodeStub::new(node.clone());
        overlay.insert(node, Arc::new(logic));
    }
    let overlay = Arc::new(overlay);
    cache.insert(key, overlay.clone());
    Ok(overlay)
}

pub struct WasmNodeStub {
    node: Node,
}

impl WasmNodeStub {
    pub fn new(node: Node) -> Self {
        Self { node }
    }
}

#[async_trait]
impl NodeLogic for WasmNodeStub {
    fn get_node(&self) -> Node {
        self.node.clone()
    }

    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(anyhow!(
            "WASM node '{}' is compile-only in the API and never executes here",
            self.node.name
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::node::{NodePermission, NodeWasm};
    use flow_like::flow::pin::PinOptions;
    use flow_like::flow::variable::VariableType;
    use std::collections::HashMap;

    fn wasm_node(name: &str) -> Node {
        let mut node = Node::new(name, name, "a package node", "Package");
        node.wasm = Some(NodeWasm {
            package_id: "com.example.pkg".into(),
            permissions: Vec::new(),
        });
        node
    }

    fn node_with_metadata(name: &str) -> Node {
        let mut node = wasm_node(name);
        node.add_input_pin("input", "Input", "Input value", VariableType::String)
            .set_default_value(Some(serde_json::json!("original")));
        node.add_output_pin("output", "Output", "Output value", VariableType::String);
        node.required_oauth_scopes = Some(HashMap::from([
            ("provider_a".into(), vec!["read".into()]),
            ("provider_b".into(), vec!["write".into()]),
        ]));
        node.ensure_flowscript_names();
        node
    }

    fn package_revision() -> String {
        let packages = HashMap::from([(
            "com.example.pkg".into(),
            flow_like_types::dispatch::WasmPackageRef {
                version: "1.0.0".into(),
                wasm_hash: "same-wasm-binary".into(),
                wasm_url: String::new(),
                cwasm_url: String::new(),
                cwasm_checksum: "same-compiled-binary".into(),
                permissions: None,
            },
        )]);
        flow_like_types::dispatch::wasm_package_set_revision(Some(&packages))
    }

    #[test]
    fn metadata_cache_ignores_node_and_object_key_order() {
        fn reverse_objects(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Object(object) => {
                    let entries: Vec<_> = std::mem::take(object).into_iter().collect();
                    for (key, mut value) in entries.into_iter().rev() {
                        reverse_objects(&mut value);
                        object.insert(key, value);
                    }
                }
                serde_json::Value::Array(values) => {
                    values.iter_mut().for_each(reverse_objects);
                }
                _ => {}
            }
        }

        let nodes = vec![node_with_metadata("first"), node_with_metadata("second")];
        let mut reordered_json = serde_json::to_value(&nodes).unwrap();
        reverse_objects(&mut reordered_json);
        let mut reordered: Vec<Node> = serde_json::from_value(reordered_json).unwrap();
        reordered.reverse();
        let revision = package_revision();
        assert_eq!(
            artifact_registry_cache_key(&revision, &nodes).unwrap(),
            artifact_registry_cache_key(&revision, &reordered).unwrap()
        );

        let base = Arc::new(FlowNodeRegistryInner::new(0));
        let cache = moka::sync::Cache::new(16);
        let first = cached_artifact_registry(&base, &cache, &revision, nodes).unwrap();
        let second = cached_artifact_registry(&base, &cache, &revision, reordered).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn metadata_repairs_replace_cached_definitions_without_a_package_change() {
        let base = Arc::new(FlowNodeRegistryInner::new(0));
        let cache = moka::sync::Cache::new(16);
        let revision = package_revision();
        let original = node_with_metadata("pkg_node");
        let cached =
            cached_artifact_registry(&base, &cache, &revision, vec![original.clone()]).unwrap();

        let changes: [(&str, fn(&mut Node)); 5] = [
            ("pin description", |node| {
                node.pins.values_mut().next().unwrap().description = "Repaired pin".into();
            }),
            ("pin schema", |node| {
                node.pins.values_mut().next().unwrap().schema =
                    Some(r#"{"type":"string","minLength":1}"#.into());
            }),
            ("pin default", |node| {
                node.pins
                    .values_mut()
                    .next()
                    .unwrap()
                    .set_default_value(Some(serde_json::json!("repaired")));
            }),
            ("pin options", |node| {
                let mut options = PinOptions::new();
                options.sensitive = Some(true);
                node.pins.values_mut().next().unwrap().options = Some(options);
            }),
            ("permissions", |node| {
                node.wasm
                    .as_mut()
                    .unwrap()
                    .permissions
                    .push(NodePermission::NetworkHttp);
            }),
        ];
        for (field, change) in changes {
            let mut repaired = original.clone();
            change(&mut repaired);
            let updated =
                cached_artifact_registry(&base, &cache, &revision, vec![repaired.clone()]).unwrap();
            assert!(
                !Arc::ptr_eq(&cached, &updated),
                "stale cache hit for {field}"
            );
            assert_eq!(
                serde_json::to_value(updated.get_node(&repaired.name).unwrap()).unwrap(),
                serde_json::to_value(&repaired).unwrap(),
                "registry must contain the repaired {field}"
            );
            assert_eq!(
                serde_json::to_value(updated.instantiate(&repaired).unwrap().get_node()).unwrap(),
                serde_json::to_value(&repaired).unwrap(),
                "compile stub must contain the repaired {field}"
            );
            if matches!(field, "pin default" | "pin options" | "permissions") {
                assert_eq!(cached.fingerprint(), updated.fingerprint());
            } else {
                assert_ne!(cached.fingerprint(), updated.fingerprint());
            }
        }
    }

    #[test]
    fn metadata_cache_preserves_package_revision_identity() {
        let base = Arc::new(FlowNodeRegistryInner::new(0));
        let cache = moka::sync::Cache::new(16);
        let node = node_with_metadata("pkg_node");
        let first = cached_artifact_registry(&base, &cache, "original-package", vec![node.clone()])
            .unwrap();
        let second =
            cached_artifact_registry(&base, &cache, "updated-package", vec![node]).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn metadata_cache_tracks_the_winner_when_packages_reuse_a_node_name() {
        let base = Arc::new(FlowNodeRegistryInner::new(0));
        let cache = moka::sync::Cache::new(16);
        let revision = package_revision();
        let first = node_with_metadata("shared_node");
        let mut second = first.clone();
        second.wasm.as_mut().unwrap().package_id = "com.example.other".into();
        second.description = "Definition from the other package".into();
        let first_order = cached_artifact_registry(
            &base,
            &cache,
            &revision,
            vec![first.clone(), second.clone()],
        )
        .unwrap();
        let reversed = cached_artifact_registry(
            &base,
            &cache,
            &revision,
            vec![second.clone(), first.clone()],
        )
        .unwrap();
        assert!(!Arc::ptr_eq(&first_order, &reversed));
        assert_eq!(
            first_order.get_node("shared_node").unwrap().wasm,
            second.wasm
        );
        assert_eq!(reversed.get_node("shared_node").unwrap().wasm, first.wasm);
    }

    /// The whole security argument rests on this: the crate that compiles
    /// user boards must not be able to load user WASM at all.
    #[test]
    fn the_api_never_links_the_wasm_runtime_outside_tests() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("Cargo.toml is readable");
        let dependencies = manifest
            .split("[dev-dependencies]")
            .next()
            .expect("manifest has a dependencies section");
        assert!(
            !dependencies.contains("flow-like-wasm.workspace")
                && !dependencies.contains("flow-like-wasm ="),
            "flow-like-wasm must stay a dev-dependency of the API"
        );
    }

    #[test]
    fn stubbed_wasm_nodes_change_the_fingerprint_deterministically() {
        let base = FlowNodeRegistryInner::new(0);
        let base_fingerprint = base.fingerprint();

        let build = || {
            let mut overlay = base.clone();
            let node = wasm_node("pkg_node");
            overlay.insert(node.clone(), Arc::new(WasmNodeStub::new(node)));
            overlay.fingerprint()
        };
        let first = build();
        let second = build();

        assert_ne!(
            first, base_fingerprint,
            "a WASM node must be part of the identity"
        );
        assert_eq!(first, second, "the same node set must always hash the same");
    }

    #[tokio::test]
    async fn a_stub_refuses_to_run() {
        let node = wasm_node("pkg_node");
        let stub = WasmNodeStub::new(node.clone());
        assert_eq!(stub.get_node().name, node.name);
        // `run` needs an ExecutionContext; the refusal is exercised through the
        // registry in `compiled_artifacts` — here we only pin the contract that
        // the stub yields the node it was built from.
    }
}
