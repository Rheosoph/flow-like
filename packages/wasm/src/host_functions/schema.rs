use crate::limits::WasmCapabilities;
use flow_like::bit::Bit;
use flow_like_catalog_core::{FlowPath, NodeDBConnection, NodeImage};
use flow_like_catalog_llm::embedding::CachedEmbeddingModel;
use once_cell::sync::Lazy;
use schemars::schema_for;
use std::collections::HashMap;

static TYPE_SCHEMAS: Lazy<HashMap<&'static str, String>> = Lazy::new(|| {
    let mut m = HashMap::new();
    let types: Vec<(&str, String)> = vec![
        (
            "FlowPath",
            serde_json::to_string(&schema_for!(FlowPath)).unwrap_or_default(),
        ),
        (
            "NodeImage",
            serde_json::to_string(&schema_for!(NodeImage)).unwrap_or_default(),
        ),
        (
            "NodeDBConnection",
            serde_json::to_string(&schema_for!(NodeDBConnection)).unwrap_or_default(),
        ),
        (
            "CachedEmbeddingModel",
            serde_json::to_string(&schema_for!(CachedEmbeddingModel)).unwrap_or_default(),
        ),
        (
            "Bit",
            serde_json::to_string(&schema_for!(Bit)).unwrap_or_default(),
        ),
    ];
    for (name, schema) in types {
        m.insert(name, schema);
    }
    m
});

/// Returns the capability required to access a given type's schema.
/// FlowPath requires STORAGE_READ (it can resolve to filesystem paths),
/// all other types require MODELS (ML pipeline types).
pub fn required_capability(type_name: &str) -> WasmCapabilities {
    match type_name {
        "FlowPath" => WasmCapabilities::STORAGE_READ,
        _ => WasmCapabilities::MODELS,
    }
}

pub fn get_type_schema(type_name: &str) -> Option<&'static str> {
    TYPE_SCHEMAS.get(type_name).map(|s| s.as_str())
}

pub fn list_type_names() -> Vec<&'static str> {
    TYPE_SCHEMAS.keys().copied().collect()
}
