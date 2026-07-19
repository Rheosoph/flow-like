use std::fmt;

use flow_like_types::Value;

const MAX_SCHEMA_BYTES: usize = 64 * 1024;

#[derive(Debug)]
struct ExternalSchemaReferenceDenied(String);

impl fmt::Display for ExternalSchemaReferenceDenied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "external JSON Schema reference '{}' is not allowed",
            self.0
        )
    }
}

impl std::error::Error for ExternalSchemaReferenceDenied {}

#[derive(Debug, Clone, Copy)]
struct RejectExternalSchemaReferences;

impl jsonschema::Retrieve for RejectExternalSchemaReferences {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(Box::new(ExternalSchemaReferenceDenied(
            uri.as_str().to_string(),
        )))
    }
}

/// Compiles a governed action parameter schema without network or filesystem
/// resolution. The linear-time regex engine also prevents authored patterns
/// from introducing catastrophic backtracking in an invocation path.
pub fn ontology_action_parameter_validator(
    schema: &Value,
) -> Result<jsonschema::Validator, String> {
    if !schema.is_object() {
        return Err("the parameter schema must be a JSON object".to_string());
    }
    let encoded_size = flow_like_types::json::to_vec(schema)
        .map_err(|error| format!("could not encode the parameter schema: {error}"))?
        .len();
    if encoded_size > MAX_SCHEMA_BYTES {
        return Err(format!(
            "the parameter schema exceeds the {} KiB limit",
            MAX_SCHEMA_BYTES / 1024
        ));
    }

    jsonschema::options()
        .with_retriever(RejectExternalSchemaReferences)
        .with_pattern_options(
            jsonschema::PatternOptions::regex()
                .size_limit(1_000_000)
                .dfa_size_limit(1_000_000),
        )
        .build(schema)
        .map_err(|error| error.to_string())
}

pub fn validate_ontology_action_parameters(
    schema: &Value,
    parameters: &Value,
) -> Result<(), String> {
    let validator = ontology_action_parameter_validator(schema)?;
    validator
        .validate(parameters)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use flow_like_types::json::json;

    use super::{ontology_action_parameter_validator, validate_ontology_action_parameters};

    #[test]
    fn malformed_action_schema_fails_without_panicking() {
        let error = ontology_action_parameter_validator(&json!({ "type": 42 }))
            .expect_err("invalid schema should be rejected");
        assert!(!error.is_empty());
    }

    #[test]
    fn external_action_schema_references_are_denied() {
        let error = ontology_action_parameter_validator(&json!({
            "$ref": "https://example.invalid/action.json"
        }))
        .expect_err("external references should be rejected");
        assert!(error.contains("external JSON Schema reference"));
    }

    #[test]
    fn local_action_schema_references_remain_supported() {
        let schema = json!({
            "$defs": {
                "reason": { "type": "string", "minLength": 1 }
            },
            "type": "object",
            "required": ["reason"],
            "properties": {
                "reason": { "$ref": "#/$defs/reason" }
            }
        });
        assert!(
            validate_ontology_action_parameters(&schema, &json!({ "reason": "Delay" })).is_ok()
        );
        assert!(validate_ontology_action_parameters(&schema, &json!({ "reason": "" })).is_err());
    }
}
