//! Typed widget contracts for micro-frontend package widgets
//!
//! A contract declares a widget's inputs, events, and queries so catalog
//! nodes can generate exact pins without string matching. Contracts are
//! authored as plain TypeScript types and compiled to `contract.json` by
//! `@flow-like/widget-bundler`; this module is the Rust representation used
//! for manifest validation, publish validation, and pin generation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Current widget contract version
pub const CONTRACT_VERSION: u32 = 1;

/// Host <-> widget postMessage protocol version
pub const WIDGET_PROTOCOL: &str = "flw/1";

/// Simple pin-type tag derived from each input schema's top-level `type`/`enum`,
/// so pin generation never parses full JSON Schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ContractInputType {
    String,
    Number,
    Integer,
    Boolean,
    Enum,
    Json,
}

/// A single typed widget input
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ContractInput {
    #[serde(rename = "type")]
    pub input_type: ContractInputType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Default value; required for standalone dev and pin defaults unless optional
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub default: Option<serde_json::Value>,
    /// Valid values for `enum` inputs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<String>>,
    /// Minimum for numeric inputs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Maximum for numeric inputs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Full JSON Schema for `json` inputs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub schema: Option<serde_json::Value>,
    /// Optional inputs may be omitted without a default
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

/// A widget event the host can bind workflow event nodes to
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ContractEvent {
    /// JSON Schema of the event payload; `null` for payload-less events
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub payload_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A request/response query the host can invoke on a widget instance
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ContractQuery {
    /// JSON Schema of the query arguments; `null` for argument-less queries
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub args_schema: Option<serde_json::Value>,
    /// JSON Schema of the query result
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub result_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Sizing hints for the host iframe
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WidgetSizing {
    #[serde(default = "default_height")]
    pub default_height: u32,
    #[serde(default = "default_resizable")]
    pub resizable: bool,
    /// Clamp for widget-requested auto-height resizes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
}

fn default_height() -> u32 {
    320
}

fn default_resizable() -> bool {
    true
}

impl Default for WidgetSizing {
    fn default() -> Self {
        Self {
            default_height: default_height(),
            resizable: default_resizable(),
            max_height: None,
        }
    }
}

/// Typed contract of a package widget (`contract.json`)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WidgetContract {
    pub contract_version: u32,
    /// Widget identifier, unique within the package
    pub id: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, ContractInput>,
    #[serde(default)]
    pub events: BTreeMap<String, ContractEvent>,
    #[serde(default)]
    pub queries: BTreeMap<String, ContractQuery>,
    #[serde(default)]
    pub sizing: WidgetSizing,
}

impl WidgetContract {
    pub fn new(id: &str) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            id: id.to_string(),
            inputs: BTreeMap::new(),
            events: BTreeMap::new(),
            queries: BTreeMap::new(),
            sizing: WidgetSizing::default(),
        }
    }

    pub fn from_json(content: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(content)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Validate the contract; returns all problems found
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.contract_version == 0 || self.contract_version > CONTRACT_VERSION {
            errors.push(format!(
                "Unsupported contract version {} for widget '{}' (supported: 1..={})",
                self.contract_version, self.id, CONTRACT_VERSION
            ));
        }

        if !is_valid_widget_id(&self.id) {
            errors.push(format!(
                "Invalid widget id '{}': must be non-empty lowercase kebab-case ([a-z0-9-])",
                self.id
            ));
        }

        for (key, input) in &self.inputs {
            if !is_valid_member_key(key) {
                errors.push(format!(
                    "Invalid input key '{}' in widget '{}': must match [a-zA-Z_][a-zA-Z0-9_]*",
                    key, self.id
                ));
            }
            if input.input_type == ContractInputType::Enum {
                match &input.choices {
                    Some(choices) if !choices.is_empty() => {}
                    _ => errors.push(format!(
                        "Enum input '{}' in widget '{}' must declare non-empty choices",
                        key, self.id
                    )),
                }
            }
            if let (Some(min), Some(max)) = (input.min, input.max) {
                if min > max {
                    errors.push(format!(
                        "Input '{}' in widget '{}' has min {} > max {}",
                        key, self.id, min, max
                    ));
                }
            }
            if let Some(default) = &input.default {
                if !default_matches_type(default, input) {
                    errors.push(format!(
                        "Default value for input '{}' in widget '{}' does not match its declared type",
                        key, self.id
                    ));
                }
            }
        }

        for key in self.events.keys() {
            if !is_valid_member_key(key) {
                errors.push(format!(
                    "Invalid event key '{}' in widget '{}': must match [a-zA-Z_][a-zA-Z0-9_]*",
                    key, self.id
                ));
            }
        }

        for key in self.queries.keys() {
            if !is_valid_member_key(key) {
                errors.push(format!(
                    "Invalid query key '{}' in widget '{}': must match [a-zA-Z_][a-zA-Z0-9_]*",
                    key, self.id
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn is_valid_widget_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_valid_member_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn default_matches_type(value: &serde_json::Value, input: &ContractInput) -> bool {
    match input.input_type {
        ContractInputType::String => value.is_string(),
        ContractInputType::Number => value.is_number(),
        ContractInputType::Integer => value.is_i64() || value.is_u64(),
        ContractInputType::Boolean => value.is_boolean(),
        ContractInputType::Enum => value
            .as_str()
            .map(|s| {
                input
                    .choices
                    .as_ref()
                    .map(|c| c.iter().any(|choice| choice == s))
                    .unwrap_or(false)
            })
            .unwrap_or(false),
        ContractInputType::Json => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_contract() -> WidgetContract {
        let mut contract = WidgetContract::new("sales-chart");
        contract.inputs.insert(
            "title".into(),
            ContractInput {
                input_type: ContractInputType::String,
                description: Some("Chart headline".into()),
                default: Some(json!("Sales")),
                choices: None,
                min: None,
                max: None,
                schema: None,
                optional: false,
            },
        );
        contract.inputs.insert(
            "variant".into(),
            ContractInput {
                input_type: ContractInputType::Enum,
                description: None,
                default: Some(json!("bar")),
                choices: Some(vec!["bar".into(), "line".into()]),
                min: None,
                max: None,
                schema: None,
                optional: false,
            },
        );
        contract.events.insert(
            "pointSelected".into(),
            ContractEvent {
                payload_schema: Some(json!({"type": "object"})),
                description: None,
            },
        );
        contract.queries.insert(
            "getValue".into(),
            ContractQuery {
                args_schema: None,
                result_schema: Some(json!({"type": "string"})),
                description: None,
            },
        );
        contract
    }

    #[test]
    fn test_contract_roundtrip_camel_case() {
        let contract = sample_contract();
        let serialized = contract.to_json().unwrap();
        assert!(serialized.contains("contractVersion"));
        assert!(serialized.contains("payloadSchema"));
        assert!(serialized.contains("resultSchema"));
        assert!(serialized.contains("defaultHeight"));

        let parsed = WidgetContract::from_json(&serialized).unwrap();
        assert_eq!(parsed.id, "sales-chart");
        assert_eq!(parsed.inputs.len(), 2);
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn test_contract_validation_errors() {
        let mut contract = sample_contract();
        contract.id = "Bad_Id".into();
        contract.inputs.insert(
            "broken enum".into(),
            ContractInput {
                input_type: ContractInputType::Enum,
                description: None,
                default: Some(json!("missing")),
                choices: Some(vec![]),
                min: None,
                max: None,
                schema: None,
                optional: false,
            },
        );

        let errors = contract.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("Invalid widget id")));
        assert!(errors.iter().any(|e| e.contains("Invalid input key")));
        assert!(errors.iter().any(|e| e.contains("non-empty choices")));
    }

    #[test]
    fn test_enum_default_must_be_choice() {
        let mut contract = WidgetContract::new("widget");
        contract.inputs.insert(
            "variant".into(),
            ContractInput {
                input_type: ContractInputType::Enum,
                description: None,
                default: Some(json!("pie")),
                choices: Some(vec!["bar".into(), "line".into()]),
                min: None,
                max: None,
                schema: None,
                optional: false,
            },
        );
        let errors = contract.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("does not match")));
    }
}
