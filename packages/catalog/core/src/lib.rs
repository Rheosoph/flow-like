//! Core catalog types for Flow-Like
//!
//! This crate contains shared types used across all catalog crates:
//! - NodeImage, BoundingBox
//! - FlowPath, FlowPathRuntime, FlowPathStore
//! - NodeDBConnection, CachedDB
//! - Attachment
//! - NodeConstructor and get_catalog()

use std::sync::Arc;

pub use flow_like::flow::node::NodeLogic;

pub use flow_like_catalog_macros::register_node;

mod types;

/// Returns true if the `execute` feature is enabled, allowing node execution.
/// When disabled, nodes should return an error from `run()` instead of executing.
#[inline]
pub const fn is_execution_enabled() -> bool {
    cfg!(feature = "execute")
}

/// Macro to gate run() method body behind the `execute` feature.
/// When `execute` is enabled, executes the provided block.
/// When disabled, returns an error indicating execution is not available.
///
/// # Example
/// ```ignore
/// async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
///     run_with_execute_gate!(context, {
///         // execution logic here
///         context.deactivate_exec_pin("exec_out").await?;
///         // ...
///         Ok(())
///     })
/// }
/// ```
#[macro_export]
macro_rules! run_with_execute_gate {
    ($context:ident, $body:block) => {{
        #[cfg(feature = "execute")]
        {
            $body
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = $context;
            Err(flow_like_types::anyhow!(
                "Node execution is not enabled. Rebuild with the 'execute' feature flag."
            ))
        }
    }};
}

pub use types::attachment::Attachment;
pub use types::bounding_box::BoundingBox;
pub use types::class_prediction::ClassPrediction;
pub use types::db_connection::{CachedDB, NodeDBConnection};
pub use types::flow_path::{FlowPath, FlowPathRuntime, FlowPathStore};
pub use types::graph_overlay::{
    DEFAULT_GRAPH_NEIGHBORS_DIRECTION, DEFAULT_GRAPH_OVERLAY_LIMIT, DEFAULT_GRAPH_QUERY_LIMIT,
    DEFAULT_GRAPH_SAMPLE_SIZE, EdgeLabelMapping, GRAPH_OVERLAYS_TABLE, GraphLabelInfo,
    GraphOverlay, GraphPropertyInfo, GraphSchema, LabelStyle, NodeGraphConnection,
    NodeLabelMapping, NodeSize, ObjectViewDefinition, OntologyActionDefinition, PropertyColumn,
    RESERVED_TABLE_PREFIX, RESERVED_TABLE_SUFFIX, SubgraphEdge, SubgraphNode, SubgraphPayload,
    is_reserved_table,
};
pub use types::keypoint::{
    COCO_KEYPOINT_NAMES, COCO_SKELETON_CONNECTIONS, Keypoint, PoseDetection, SkeletonConnection,
};
pub use types::node_image::{NodeImage, NodeImageWrapper};
pub use types::ontology_action_schema::{
    ontology_action_parameter_validator, validate_ontology_action_parameters,
};
pub use types::ontology_bindings::{ontology_binding_nodes, remote_ontology_binding_nodes};
pub use types::remote_ontology::RemoteOntologyImport;
pub use types::segmentation_mask::{InstanceSegmentation, SegmentationMask};

/// A node constructor function type
pub struct NodeConstructor {
    constructor: fn() -> Arc<dyn NodeLogic>,
}

impl NodeConstructor {
    pub const fn new(constructor: fn() -> Arc<dyn NodeLogic>) -> Self {
        Self { constructor }
    }

    pub fn construct(&self) -> Arc<dyn NodeLogic> {
        (self.constructor)()
    }
}
