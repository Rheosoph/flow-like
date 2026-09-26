//! Typed widget contracts for micro-frontend package widgets
//!
//! A contract declares a widget's inputs, events, and queries so catalog
//! nodes can generate exact pins without string matching. Contracts are
//! authored as plain TypeScript types and compiled to `contract.json` by
//! `@flow-like/widget-bundler`; this module is the Rust representation used
//! for manifest validation, publish validation, and pin generation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::widget_policy::{
    WidgetCsp, WidgetCspPurpose, flatten_csp_purposes, validate_csp_purposes,
};

/// Current widget contract version; contracts declaring `csp` must use it
pub const CONTRACT_VERSION: u32 = 2;

/// Version of contracts without `csp`, readable by hosts that predate `csp`
pub const BASE_CONTRACT_VERSION: u32 = 1;

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
    /// A mutation may change widget state and must be acknowledged by a live instance.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mutation: bool,
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

/// Browser features explicitly requested by a widget contract.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct WidgetCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workers: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<bool>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<WidgetCapabilities>,
    /// CSP extensions as purpose groups; present only with `contractVersion` 2
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csp: Option<Vec<WidgetCspPurpose>>,
}

impl WidgetContract {
    pub fn new(id: &str) -> Self {
        Self {
            contract_version: BASE_CONTRACT_VERSION,
            id: id.to_string(),
            inputs: BTreeMap::new(),
            events: BTreeMap::new(),
            queries: BTreeMap::new(),
            sizing: WidgetSizing::default(),
            capabilities: None,
            csp: None,
        }
    }

    /// Sets `csp` in canonical form (absent without purposes) and the
    /// contract version it requires. Group order is kept.
    pub fn with_csp(mut self, mut purposes: Vec<WidgetCspPurpose>) -> Self {
        purposes.iter_mut().for_each(WidgetCspPurpose::canonicalize);
        if purposes.is_empty() {
            self.csp = None;
            self.contract_version = BASE_CONTRACT_VERSION;
        } else {
            self.csp = Some(purposes);
            self.contract_version = CONTRACT_VERSION;
        }
        self
    }

    /// Per-directive union of every purpose's sources, sorted and deduplicated.
    pub fn declared_csp(&self) -> WidgetCsp {
        self.csp
            .as_deref()
            .map(flatten_csp_purposes)
            .unwrap_or_default()
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

        match (self.contract_version, &self.csp) {
            (CONTRACT_VERSION, Some(_)) | (BASE_CONTRACT_VERSION, None) => {}
            (BASE_CONTRACT_VERSION, Some(_)) => errors.push(format!(
                "Widget '{}' declares csp and must use contractVersion {}",
                self.id, CONTRACT_VERSION
            )),
            (CONTRACT_VERSION, None) => errors.push(format!(
                "Widget '{}' uses contractVersion {} without csp; contracts without csp must use contractVersion {}",
                self.id, CONTRACT_VERSION, BASE_CONTRACT_VERSION
            )),
            (version, _) => errors.push(format!(
                "Unsupported contractVersion {} for widget '{}' (supported: {}, or {} with csp)",
                version, self.id, BASE_CONTRACT_VERSION, CONTRACT_VERSION
            )),
        }

        if let Some(purposes) = &self.csp {
            errors.extend(validate_csp_purposes(&self.id, purposes, &self.inputs));
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

/// Contract widget ids: non-empty lowercase kebab-case (`[a-z0-9-]`).
pub fn is_valid_widget_id(id: &str) -> bool {
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

    #[test]
    fn widget_capabilities_roundtrip_and_reject_unknown_fields() {
        let value = serde_json::json!({"contractVersion":1,"id":"globe","capabilities":{"workers":true,"media":true,"microphone":true,"wasm":true,"downloads":true}});
        let contract: WidgetContract = serde_json::from_value(value).unwrap();
        let serialized = serde_json::to_value(contract).unwrap();
        assert_eq!(serialized["capabilities"]["downloads"], true);
        assert!(serde_json::from_value::<WidgetContract>(serde_json::json!({"contractVersion":1,"id":"globe","capabilities":{"sameOrigin":true}})).is_err());
        assert!(serde_json::from_value::<WidgetContract>(serde_json::json!({"contractVersion":1,"id":"globe","capabilities":{"resources":true}})).is_err());
    }

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
                mutation: false,
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
    fn mutation_flag_defaults_false_and_only_serializes_when_true() {
        let read: ContractQuery = serde_json::from_value(json!({
            "argsSchema": null,
            "resultSchema": { "type": "string" }
        }))
        .unwrap();
        assert!(!read.mutation);
        assert!(
            serde_json::to_value(&read)
                .unwrap()
                .get("mutation")
                .is_none()
        );

        let mutation: ContractQuery = serde_json::from_value(json!({
            "argsSchema": { "type": "object" },
            "resultSchema": { "type": "object" },
            "mutation": true
        }))
        .unwrap();
        assert!(mutation.mutation);
        assert_eq!(serde_json::to_value(mutation).unwrap()["mutation"], true);
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

    fn map_csp() -> Vec<WidgetCspPurpose> {
        vec![WidgetCspPurpose {
            reason: "Loads vector tiles and live positions".into(),
            connect_src: vec![
                "https://api.maptiler.com".into(),
                "wss://live.example.com".into(),
            ],
            img_src: vec!["https://a.tile.openstreetmap.org".into()],
            ..WidgetCspPurpose::default()
        }]
    }

    #[test]
    fn contract_version_is_two_exactly_when_csp_is_present() {
        let plain = WidgetContract::new("live-map");
        assert_eq!(plain.contract_version, BASE_CONTRACT_VERSION);
        assert!(plain.validate().is_ok());

        let with_csp = WidgetContract::new("live-map").with_csp(map_csp());
        assert_eq!(with_csp.contract_version, CONTRACT_VERSION);
        assert!(with_csp.validate().is_ok());

        let mut csp_on_v1 = with_csp.clone();
        csp_on_v1.contract_version = BASE_CONTRACT_VERSION;
        assert!(
            csp_on_v1
                .validate()
                .unwrap_err()
                .iter()
                .any(|e| e.contains("must use contractVersion 2"))
        );

        let mut v2_without_csp = plain.clone();
        v2_without_csp.contract_version = CONTRACT_VERSION;
        assert!(
            v2_without_csp
                .validate()
                .unwrap_err()
                .iter()
                .any(|e| e.contains("without csp"))
        );

        for version in [0, 3, 99] {
            let mut unsupported = with_csp.clone();
            unsupported.contract_version = version;
            assert!(
                unsupported
                    .validate()
                    .unwrap_err()
                    .iter()
                    .any(|e| e.contains("Unsupported contractVersion"))
            );
        }
    }

    #[test]
    fn with_csp_omits_empty_declarations_and_validate_rejects_them() {
        let emptied = WidgetContract::new("live-map")
            .with_csp(map_csp())
            .with_csp(Vec::new());
        assert!(emptied.csp.is_none());
        assert_eq!(emptied.contract_version, BASE_CONTRACT_VERSION);
        let json = serde_json::to_value(&emptied).unwrap();
        assert!(json.get("csp").is_none());
        assert!(emptied.declared_csp().is_empty());

        let mut explicit_empty = WidgetContract::new("live-map");
        explicit_empty.contract_version = CONTRACT_VERSION;
        explicit_empty.csp = Some(Vec::new());
        assert!(
            explicit_empty
                .validate()
                .unwrap_err()
                .iter()
                .any(|e| e.contains("empty csp"))
        );
    }

    #[test]
    fn csp_parses_from_contract_json_and_is_validated() {
        let contract = WidgetContract::from_json(
            r#"{"contractVersion":2,"id":"live-map","csp":[{"reason":"Loads map styles and tiles","connectSrc":["https://api.maptiler.com"],"styleSrc":["https://fonts.googleapis.com"]}]}"#,
        )
        .unwrap();
        assert!(contract.validate().is_ok());
        let serialized = serde_json::to_value(&contract).unwrap();
        assert_eq!(
            serialized["csp"],
            json!([{"reason":"Loads map styles and tiles","connectSrc":["https://api.maptiler.com"],"styleSrc":["https://fonts.googleapis.com"]}])
        );
        assert_eq!(
            contract.declared_csp(),
            WidgetCsp {
                connect_src: vec!["https://api.maptiler.com".into()],
                style_src: vec!["https://fonts.googleapis.com".into()],
                ..WidgetCsp::default()
            }
        );

        for rejected in [
            r#"{"contractVersion":2,"id":"live-map","csp":{"connectSrc":["https://api.maptiler.com"]}}"#,
            r#"{"contractVersion":2,"id":"live-map","csp":[{"reason":"Loads map tiles","scriptSrc":["https://cdn.example.org"]}]}"#,
            r#"{"contractVersion":2,"id":"live-map","csp":[{"connectSrc":["https://api.maptiler.com"]}]}"#,
        ] {
            assert!(WidgetContract::from_json(rejected).is_err(), "{rejected}");
        }

        let invalid = WidgetContract::from_json(
            r#"{"contractVersion":2,"id":"live-map","csp":[{"reason":"Loads map tiles","connectSrc":["https://b.example.org","http://a.example.org"]}]}"#,
        )
        .unwrap();
        let errors = invalid.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("http://a.example.org")));
        assert!(errors.iter().any(|e| e.contains("sorted ascending")));
        assert!(
            errors
                .iter()
                .all(|e| e.starts_with("Widget 'live-map': csp purpose 0: "))
        );
    }

    #[test]
    fn csp_serializes_after_capabilities_in_group_and_field_order() {
        let mut contract = WidgetContract::new("live-map").with_csp(vec![
            WidgetCspPurpose {
                reason: "Loads map styles and fonts".into(),
                style_src: vec!["https://fonts.googleapis.com".into()],
                connect_src: vec!["https://api.maptiler.com".into()],
                font_src: vec!["https://fonts.gstatic.com".into()],
                ..WidgetCspPurpose::default()
            },
            WidgetCspPurpose {
                reason: "Loads tiles given at runtime".into(),
                inputs: vec![crate::widget_policy::WidgetNetworkInput {
                    path: "tileUrl".into(),
                    directives: vec![crate::widget_policy::CspDirective::ImgSrc],
                    template: None,
                }],
                ..WidgetCspPurpose::default()
            },
        ]);
        contract.capabilities = Some(WidgetCapabilities {
            workers: Some(true),
            ..WidgetCapabilities::default()
        });
        let text = serde_json::to_string(&contract).unwrap();
        let capabilities = text.find("\"capabilities\"").unwrap();
        let csp = text.find("\"csp\"").unwrap();
        assert!(capabilities < csp);
        assert!(text.ends_with(
            r#""csp":[{"reason":"Loads map styles and fonts","connectSrc":["https://api.maptiler.com"],"fontSrc":["https://fonts.gstatic.com"],"styleSrc":["https://fonts.googleapis.com"]},{"reason":"Loads tiles given at runtime","inputs":[{"path":"tileUrl","directives":["imgSrc"]}]}]}"#
        ));
        assert!(
            contract
                .validate()
                .unwrap_err()
                .iter()
                .any(|e| e.contains("root \"tileUrl\" is not a contract input"))
        );
    }
}
