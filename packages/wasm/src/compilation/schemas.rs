use serde_json::{json, Value};

// Schema lookups expose wire formats, not the implementations of these handles.
// The execution-host parity test checks these against the runtime's schema derives.
pub(super) const TYPES: &[&str] = &[
    "Bit",
    "CachedEmbeddingModel",
    "DataFusionSession",
    "FlowPath",
    "NodeDBConnection",
    "NodeImage",
];

pub(super) fn get(name: &str) -> Option<String> {
    let bit: Value = serde_json::from_str(include_str!("../../../schema/bit/bit.json")).ok()?;
    let schema = match name {
        "Bit" => bit,
        "CachedEmbeddingModel" => json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema", "title": name,
            "type": "object", "properties": {
                "cache_key": {"type":"string"}, "model_type": {"$ref":"#/$defs/BitTypes"}
            }, "required": ["cache_key", "model_type"], "$defs": {"BitTypes":bit["$defs"]["BitTypes"]}
        }),
        "FlowPath" => json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema", "title": name,
            "type":"object", "properties": {
                "path":{"type":"string"}, "store_ref":{"type":"string"},
                "cache_store_ref":{"type":["string","null"]}
            }, "required":["path","store_ref"]
        }),
        "NodeDBConnection" | "DataFusionSession" | "NodeImage" => {
            let field = if name == "NodeImage" {
                "image_ref"
            } else {
                "cache_key"
            };
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema", "title": name,
                "type":"object", "properties": {field:{"type":"string"}}, "required":[field]
            })
        }
        _ => return None,
    };
    serde_json::to_string(&schema).ok()
}
