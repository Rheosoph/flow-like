use chrono::{DateTime, Utc};
use flow_like_catalog_core::{BoundingBox, FlowPath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct DomSnapshotRef {
    pub snapshot_id: String,
    pub artifact_path: FlowPath,
    pub page_url: String,
    pub created_at: DateTime<Utc>,
    pub node_count: usize,
}

impl DomSnapshotRef {
    pub fn new(
        snapshot_id: impl Into<String>,
        artifact_path: FlowPath,
        page_url: impl Into<String>,
        node_count: usize,
    ) -> Self {
        Self {
            snapshot_id: snapshot_id.into(),
            artifact_path,
            page_url: page_url.into(),
            created_at: Utc::now(),
            node_count,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AxSnapshotRef {
    pub snapshot_id: String,
    pub artifact_path: FlowPath,
    pub source: AxSnapshotSource,
    pub created_at: DateTime<Utc>,
    pub node_count: usize,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum AxSnapshotSource {
    Browser { page_url: String },
    Desktop { window_title: Option<String> },
}

impl AxSnapshotRef {
    pub fn browser(
        snapshot_id: impl Into<String>,
        artifact_path: FlowPath,
        page_url: impl Into<String>,
        node_count: usize,
    ) -> Self {
        Self {
            snapshot_id: snapshot_id.into(),
            artifact_path,
            source: AxSnapshotSource::Browser {
                page_url: page_url.into(),
            },
            created_at: Utc::now(),
            node_count,
        }
    }

    pub fn desktop(
        snapshot_id: impl Into<String>,
        artifact_path: FlowPath,
        window_title: Option<String>,
        node_count: usize,
    ) -> Self {
        Self {
            snapshot_id: snapshot_id.into(),
            artifact_path,
            source: AxSnapshotSource::Desktop { window_title },
            created_at: Utc::now(),
            node_count,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct DomNode {
    pub node_id: String,
    pub tag_name: String,
    pub attributes: std::collections::HashMap<String, String>,
    pub text_content: Option<String>,
    pub children: Vec<String>,
    pub parent: Option<String>,
    pub bounding_box: Option<BoundingBox>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AxNode {
    pub node_id: String,
    pub role: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    pub children: Vec<String>,
    pub parent: Option<String>,
    pub bounding_box: Option<BoundingBox>,
    pub focusable: bool,
    pub focused: bool,
    pub disabled: bool,
}
