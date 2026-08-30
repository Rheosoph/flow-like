use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Runtime-independent node metadata stored alongside a package version.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackageNodeEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub friendly_name: Option<String>,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub scores: Option<flow_like::flow::node::NodeScores>,
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = HashMap<String, Object>))]
    pub pins: HashMap<String, flow_like::flow::pin::Pin>,
    #[serde(default)]
    pub start: Option<bool>,
    #[serde(default)]
    pub long_running: Option<bool>,
    #[serde(default)]
    pub docs: Option<String>,
    #[serde(default)]
    pub event_callback: Option<bool>,
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub fn_refs: Option<flow_like::flow::node::FnRefs>,
    #[serde(default)]
    pub oauth_providers: Vec<String>,
    #[serde(default)]
    pub required_oauth_scopes: Option<HashMap<String, Vec<String>>>,
    #[serde(default)]
    pub only_offline: bool,
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default)]
    pub permissions: Vec<flow_like::flow::node::NodePermission>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}
