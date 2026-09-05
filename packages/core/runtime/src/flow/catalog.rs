use super::node::{Node, NodeLogic};
use std::sync::Arc;

use crate::state::FlowLikeState;

pub async fn node_to_dyn(
    app_state: &FlowLikeState,
    node: &Node,
) -> flow_like_types::Result<Arc<dyn NodeLogic>> {
    let registry_state = app_state.node_registry();
    let registry = registry_state.read().await;

    let node = registry.instantiate(node)?;
    Ok(node)
}
