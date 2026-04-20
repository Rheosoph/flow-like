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
pub struct SubgraphNode {
    pub id: String,
    pub label: String,
    pub caption: Option<String>,
    pub props: Value,
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
    async fn cypher(&self, query: &str, params: Value, limit: Option<usize>)
        -> Result<Vec<Value>>;

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

    async fn search_nodes(&self, query: &str, limit: Option<usize>)
        -> Result<Vec<SubgraphNode>>;

    async fn schema(&self) -> Result<GraphSchemaResult>;

    async fn sample(&self, label: &str, n: usize) -> Result<Vec<Value>>;
}
