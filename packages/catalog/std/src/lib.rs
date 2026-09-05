//! Standard catalog nodes, collected from independent implementation crates.

use std::{path::Path, sync::Arc};

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};
pub use flow_like_catalog_std_numbers::{faker, math};
pub use flow_like_catalog_std_runtime::{control, logging, notifications, testing, variables};
pub use flow_like_catalog_std_ui::a2ui;
pub use flow_like_catalog_std_values::structs;

pub mod utils;

/// Collect every standard node in the historical source traversal order.
pub fn collect_nodes() -> Vec<Arc<dyn NodeLogic>> {
    let mut entries = flow_like_catalog_std_ui::collect_node_entries();
    entries.extend(flow_like_catalog_std_values::collect_node_entries());
    entries.extend(flow_like_catalog_std_numbers::collect_node_entries());
    entries.extend(flow_like_catalog_std_text::collect_node_entries());
    entries.extend(flow_like_catalog_std_runtime::collect_node_entries());
    entries.sort_by(|left, right| Path::new(left.0).cmp(Path::new(right.0)));
    entries.into_iter().map(|(_, node)| node).collect()
}

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
