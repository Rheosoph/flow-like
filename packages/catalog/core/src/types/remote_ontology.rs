use flow_like_types::JsonSchema;
use flow_like_types::json::{Deserialize, Serialize};

use super::graph_overlay::GraphOverlay;

/// A pinned ontology contract installed from a connected project.
///
/// The contract is a sanitized snapshot. Runtime bindings resolve the target
/// project and remote ontology through this record instead of trusting editable
/// node defaults containing database coordinates.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoteOntologyImport {
    pub id: String,
    pub target_app_id: String,
    pub remote_ontology_id: String,
    pub contract: GraphOverlay,
    pub source_updated_at: String,
    #[serde(default = "default_true")]
    pub bindings_enabled: bool,
    pub installed_at: String,
    pub updated_at: String,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use flow_like_types::json::{from_value, json};

    use super::RemoteOntologyImport;

    #[test]
    fn imported_bindings_default_to_enabled() {
        let import: RemoteOntologyImport = from_value(json!({
            "id": "provider::operations",
            "target_app_id": "provider",
            "remote_ontology_id": "operations",
            "contract": super::GraphOverlay::default(),
            "source_updated_at": "2026-07-12T12:00:00Z",
            "installed_at": "2026-07-12T12:00:00Z",
            "updated_at": "2026-07-12T12:00:00Z"
        }))
        .expect("remote ontology import should deserialize");

        assert!(import.bindings_enabled);
    }
}
