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
use flow_like_types::{anyhow, async_trait};

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
    use flow_like::flow::node::NodeWasm;
    use flow_like::state::FlowNodeRegistryInner;
    use std::sync::Arc;

    fn wasm_node(name: &str) -> Node {
        let mut node = Node::new(name, name, "a package node", "Package");
        node.wasm = Some(NodeWasm {
            package_id: "com.example.pkg".into(),
            permissions: Vec::new(),
        });
        node
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

        assert_ne!(first, base_fingerprint, "a WASM node must be part of the identity");
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
