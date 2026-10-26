use super::{internal_node::InternalNode, internal_pin::InternalPin, resources::RunResource};
use crate::flow::compiled::template::FunctionTopology;
use ahash::AHashMap;
use std::sync::{Arc, Weak};

pub(super) const FUNCTION_LOOKUP_RESOURCE: &str = "flow-like:compiled-function-lookups";

/// Bind shared topology to one run without constructing another pin index.
pub(super) struct RunFunctionLookup {
    pub(super) topology: Arc<FunctionTopology>,
    pub(super) pins: Arc<[Arc<InternalPin>]>,
    nodes: Weak<AHashMap<String, Arc<InternalNode>>>,
}

impl RunFunctionLookup {
    pub(super) fn new(
        topology: Arc<FunctionTopology>,
        pins: &[Arc<InternalPin>],
        nodes: &Arc<AHashMap<String, Arc<InternalNode>>>,
    ) -> Self {
        Self {
            topology,
            pins: pins.into(),
            nodes: Arc::downgrade(nodes),
        }
    }

    pub(super) fn matches_nodes(&self, nodes: &Arc<AHashMap<String, Arc<InternalNode>>>) -> bool {
        self.nodes.as_ptr() == Arc::as_ptr(nodes)
    }
}

impl RunResource for RunFunctionLookup {
    // The holder owns no background work. Retained contexts can keep reading
    // their pins after shutdown, matching direct node access.
    fn abort(&self) {}
}
