use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Runtime-independent node metadata stored alongside a package version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    deserialize = "Pin: Deserialize<'de>, Scores: Deserialize<'de>, FnRefs: Deserialize<'de>, Permission: Deserialize<'de>"
))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(bound = "Permission: utoipa::ToSchema", as = PackageNodeEntry))]
pub struct PackageNodeMetadata<Pin, Scores, FnRefs, Permission> {
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
    pub scores: Option<Scores>,
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = HashMap<String, Object>))]
    pub pins: HashMap<String, Pin>,
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
    pub fn_refs: Option<FnRefs>,
    #[serde(default)]
    pub oauth_providers: Vec<String>,
    #[serde(default)]
    pub required_oauth_scopes: Option<HashMap<String, Vec<String>>>,
    #[serde(default)]
    pub only_offline: bool,
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl<Pin, Scores, FnRefs, Permission> Default
    for PackageNodeMetadata<Pin, Scores, FnRefs, Permission>
{
    fn default() -> Self {
        Self {
            id: Default::default(),
            name: Default::default(),
            friendly_name: Default::default(),
            description: Default::default(),
            category: Default::default(),
            icon: Default::default(),
            scores: Default::default(),
            pins: Default::default(),
            start: Default::default(),
            long_running: Default::default(),
            docs: Default::default(),
            event_callback: Default::default(),
            fn_refs: Default::default(),
            oauth_providers: Default::default(),
            required_oauth_scopes: Default::default(),
            only_offline: Default::default(),
            version: Default::default(),
            permissions: Default::default(),
            metadata: Default::default(),
        }
    }
}
