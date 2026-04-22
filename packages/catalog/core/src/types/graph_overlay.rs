use flow_like_types::JsonSchema;
use flow_like_types::json::{Deserialize, Serialize};

pub const DEFAULT_GRAPH_OVERLAY_LIMIT: usize = 100;
pub const DEFAULT_GRAPH_QUERY_LIMIT: usize = 100;
pub const DEFAULT_GRAPH_SAMPLE_SIZE: usize = 10;
pub const DEFAULT_GRAPH_NEIGHBORS_DIRECTION: &str = "outgoing";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphOverlay {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub nodes: Vec<NodeLabelMapping>,
    pub edges: Vec<EdgeLabelMapping>,
    pub default_limit: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PropertyColumn {
    pub name: String,
    pub data_type: String,
    #[serde(default)]
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeLabelMapping {
    pub label: String,
    pub table: String,
    pub id_column: String,
    pub display_column: Option<String>,
    pub property_columns: Vec<PropertyColumn>,
    pub style: LabelStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EdgeLabelMapping {
    pub label: String,
    pub table: String,
    pub src_column: String,
    pub dst_column: String,
    pub src_label: String,
    pub dst_label: String,
    #[serde(default)]
    pub src_node_column: Option<String>,
    #[serde(default)]
    pub dst_node_column: Option<String>,
    pub property_columns: Vec<PropertyColumn>,
    pub style: LabelStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LabelStyle {
    pub color: String,
    pub icon: String,
    pub size: NodeSize,
    #[serde(default)]
    pub shape: Option<String>,
    #[serde(default)]
    pub width: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode")]
pub enum NodeSize {
    #[serde(rename = "fixed")]
    Fixed { value: f32 },
    #[serde(rename = "by-degree")]
    ByDegree { min: f32, max: f32 },
    #[serde(rename = "by-column")]
    ByColumn { column: String, min: f32, max: f32 },
}

impl Default for NodeSize {
    fn default() -> Self {
        NodeSize::Fixed { value: 5.0 }
    }
}

impl Default for LabelStyle {
    fn default() -> Self {
        LabelStyle {
            color: "#64748b".to_string(),
            icon: "database".to_string(),
            size: NodeSize::default(),
            shape: None,
            width: None,
        }
    }
}

impl Default for GraphOverlay {
    fn default() -> Self {
        GraphOverlay {
            id: String::new(),
            name: String::new(),
            description: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            default_limit: DEFAULT_GRAPH_OVERLAY_LIMIT,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubgraphNode {
    pub id: String,
    pub label: String,
    pub caption: Option<String>,
    pub props: flow_like_types::Value,
    pub style: LabelStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubgraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub label: String,
    pub props: flow_like_types::Value,
    pub style: LabelStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubgraphPayload {
    pub nodes: Vec<SubgraphNode>,
    pub edges: Vec<SubgraphEdge>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphSchema {
    pub node_labels: Vec<GraphLabelInfo>,
    pub edge_labels: Vec<GraphLabelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphLabelInfo {
    pub label: String,
    pub table: String,
    pub properties: Vec<GraphPropertyInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphPropertyInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

pub const GRAPH_OVERLAYS_TABLE: &str = "__graph_overlays__";
pub const RESERVED_TABLE_PREFIX: &str = "__";
pub const RESERVED_TABLE_SUFFIX: &str = "__";

pub fn is_reserved_table(name: &str) -> bool {
    name.starts_with(RESERVED_TABLE_PREFIX)
        && name.ends_with(RESERVED_TABLE_SUFFIX)
        && name.len() > 4
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeGraphConnection {
    pub cache_key: String,
}
