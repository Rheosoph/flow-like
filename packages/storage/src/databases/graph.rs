#[cfg(feature = "graph")]
pub mod lancegraph;

use flow_like_types::{Result, Value, async_trait};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraversalDirection {
    Outgoing,
    Incoming,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeLabelCount {
    pub label: String,
    pub count: usize,
}

/// Population fan-out of one object, as seen through the sampling window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphNodeStats {
    pub out_by_label: Vec<EdgeLabelCount>,
    /// False when the window did not cover the whole relationship table, which
    /// makes every count a lower bound.
    pub exact: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphNode {
    pub id: String,
    pub label: String,
    pub caption: Option<String>,
    pub props: Value,
    /// Only the seedless sampler knows population counts; every other path
    /// leaves this unset. `flow-like-catalog-core` mirrors this struct without
    /// the field, so a subgraph routed through that type loses the stats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<SubgraphNodeStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub label: String,
    pub props: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphResult {
    pub nodes: Vec<SubgraphNode>,
    pub edges: Vec<SubgraphEdge>,
    pub truncated: bool,
    /// Non-fatal problems encountered while assembling the result, e.g. an
    /// edge mapping whose table failed to load. An empty list means the
    /// result is complete up to `truncated`.
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPath {
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
    pub length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPathsResult {
    pub found: bool,
    pub paths: Vec<GraphPath>,
    /// Union of all path nodes/edges, hydrated for direct rendering.
    pub nodes: Vec<SubgraphNode>,
    pub edges: Vec<SubgraphEdge>,
    pub truncated: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelCount {
    pub label: String,
    pub nodes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetric {
    pub id: String,
    pub label: String,
    pub caption: Option<String>,
    pub degree_in: usize,
    pub degree_out: usize,
    pub pagerank: f64,
    pub component: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphAnalyticsResult {
    /// Exact sum of mapped node-table row counts (for tables that could be
    /// counted), independent of the edge sampling limit.
    pub node_count: usize,
    /// Edges included in the bounded analytics snapshot.
    pub edge_count: usize,
    /// True when the edge snapshot was capped. If true, component, isolation,
    /// degree, and PageRank fields describe the sampled snapshot.
    pub truncated: bool,
    pub label_counts: Vec<LabelCount>,
    pub component_count: usize,
    /// Sizes of the largest weakly connected components, descending.
    pub largest_components: Vec<usize>,
    /// Nodes with no incident edge in the bounded snapshot. This is exact for
    /// an untruncated snapshot and a sample-relative upper bound otherwise.
    pub isolated_node_count: usize,
    pub top_by_degree: Vec<NodeMetric>,
    pub top_by_pagerank: Vec<NodeMetric>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphLabelInfo {
    pub label: String,
    pub table: String,
    pub properties: Vec<GraphPropertyInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPropertyInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSchemaResult {
    pub node_labels: Vec<GraphLabelInfo>,
    pub edge_labels: Vec<GraphLabelInfo>,
}

#[async_trait]
pub trait GraphStore: Send + Sync {
    async fn cypher(&self, query: &str, params: Value, limit: Option<usize>) -> Result<Vec<Value>>;

    async fn sql(&self, query: &str, limit: Option<usize>) -> Result<Vec<Value>>;

    async fn neighbors(
        &self,
        label: &str,
        id: Value,
        depth: usize,
        direction: TraversalDirection,
        limit: Option<usize>,
    ) -> Result<SubgraphResult>;

    async fn subgraph(
        &self,
        seeds: Vec<(String, Value)>,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<SubgraphResult>;

    /// Single containment hop from a parent object to its children, following
    /// only edges flagged `containment` (`src_label` = parent). Children mapped
    /// by a sibling local overlay (`dst_ontology`) are resolved from that
    /// overlay's tables; remote children (`dst_binding_id`) are surfaced as a
    /// warning rather than resolved here.
    async fn overlay_children(
        &self,
        parent_label: &str,
        parent_id: Value,
        limit: Option<usize>,
    ) -> Result<SubgraphResult>;

    async fn search_nodes(&self, query: &str, limit: Option<usize>) -> Result<Vec<SubgraphNode>>;

    async fn schema(&self) -> Result<GraphSchemaResult>;

    async fn sample(&self, label: &str, n: usize) -> Result<Vec<Value>>;

    /// Shortest paths between two objects, discovered via bounded traversal.
    async fn shortest_paths(
        &self,
        from: (String, Value),
        to: (String, Value),
        max_depth: usize,
        limit: Option<usize>,
    ) -> Result<GraphPathsResult>;

    /// Structural metrics (degree, PageRank, connected components) over a
    /// bounded snapshot of the overlay graph.
    async fn analytics(&self, limit: Option<usize>) -> Result<GraphAnalyticsResult>;
}
