use super::PackagePermissions;
use crate::widget::WidgetContract;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current manifest version (v2 adds micro-frontend widget support).
pub const MANIFEST_VERSION: u32 = 2;

/// Widget entry embedded in a package manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackageWidgetEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    pub contract: WidgetContract,
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// Domain category for WASM packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WasmPackageCategory {
    DocumentProcessing,
    DataTransformation,
    WorkflowAutomation,
    Communication,
    AnalyticsReporting,
    FinanceBilling,
    ComplianceRegulatory,
    HrPeople,
    AiMl,
    IntegrationConnectors,
    SecurityIdentity,
    Devops,
    IotIndustrial,
    RoboticsPhysicalAi,
    GamingSimulation,
    Healthcare,
    Veterinary,
    Legal,
    Manufacturing,
    Agriculture,
    RealEstate,
    Logistics,
    Energy,
    ConstructionTrades,
    Education,
    GovernmentDefense,
    Ecommerce,
    Insurance,
    Telecom,
    ScientificEngineering,
    Geospatial,
    MediaContent,
    Other,
}

impl std::fmt::Display for WasmPackageCategory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}",
            serde_json::to_value(self)
                .unwrap_or_default()
                .as_str()
                .unwrap_or("OTHER")
        )
    }
}

impl WasmPackageCategory {
    pub fn from_str_opt(value: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(value.to_string())).ok()
    }
}

/// Package author information.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackageAuthor {
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// Package manifest for WASM nodes and micro-frontend widgets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackageManifest {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub authors: Vec<PackageAuthor>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub permissions: PackagePermissions,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_category: Option<WasmPackageCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_category: Option<WasmPackageCategory>,
    #[serde(default)]
    pub min_flow_like_version: Option<String>,
    #[serde(default)]
    pub wasm_path: Option<String>,
    #[serde(default)]
    pub wasm_hash: Option<String>,
    #[serde(default)]
    pub widgets: Vec<PackageWidgetEntry>,
    #[serde(default)]
    pub widget_bundle_path: Option<String>,
    #[serde(default)]
    pub widget_bundle_hash: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl PackageManifest {
    pub fn new(id: &str, name: &str, version: &str, description: &str) -> Self {
        Self {
            manifest_version: MANIFEST_VERSION,
            id: id.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            description: description.to_string(),
            authors: Vec::new(),
            license: None,
            repository: None,
            homepage: None,
            permissions: PackagePermissions::default(),
            keywords: Vec::new(),
            primary_category: None,
            secondary_category: None,
            min_flow_like_version: None,
            wasm_path: None,
            wasm_hash: None,
            widgets: Vec::new(),
            widget_bundle_path: None,
            widget_bundle_hash: None,
            metadata: HashMap::new(),
        }
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.id.is_empty() {
            errors.push("Package ID is required".to_string());
        }
        if self.name.is_empty() {
            errors.push("Package name is required".to_string());
        }
        if self.version.is_empty() {
            errors.push("Package version is required".to_string());
        }

        let mut seen_widget_ids = std::collections::HashSet::new();
        for widget in &self.widgets {
            if !seen_widget_ids.insert(widget.id.clone()) {
                errors.push(format!("Duplicate widget id in manifest: {}", widget.id));
            }
            if widget.name.is_empty() {
                errors.push(format!("Widget '{}' is missing a name", widget.id));
            }
            if widget.contract.id != widget.id {
                errors.push(format!(
                    "Widget '{}' contract id '{}' does not match its entry id",
                    widget.id, widget.contract.id
                ));
            }
            if let Err(contract_errors) = widget.contract.validate() {
                errors.extend(contract_errors);
            }
        }
        if !self.widgets.is_empty()
            && self.widget_bundle_path.is_none()
            && self.widget_bundle_hash.is_none()
        {
            errors.push(
                "Manifest declares widgets but neither widget_bundle_path nor widget_bundle_hash"
                    .to_string(),
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }

    pub fn from_json(content: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(content)
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_manifest_parses_without_widget_fields() {
        let manifest = PackageManifest::from_json(
            r#"{
                "manifest_version": 1,
                "id": "com.example.legacy",
                "name": "Legacy",
                "version": "0.1.0",
                "description": "no widgets"
            }"#,
        )
        .unwrap();
        assert!(manifest.widgets.is_empty());
        assert!(manifest.widget_bundle_path.is_none());
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn widgets_require_a_bundle_reference_and_matching_contract() {
        let mut manifest = PackageManifest::new(
            "com.example.widgets",
            "Widget Package",
            "1.0.0",
            "ships widgets",
        );
        manifest.widgets.push(PackageWidgetEntry {
            id: "sales-chart".into(),
            name: "Sales Chart".into(),
            description: "chart".into(),
            icon: None,
            thumbnail: None,
            contract: WidgetContract::new("sales-chart"),
            keywords: Vec::new(),
        });
        assert!(
            manifest
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.contains("widget_bundle_path"))
        );

        manifest.widget_bundle_path = Some("widgets.flwb".into());
        assert!(manifest.validate().is_ok());
        manifest.widgets[0].contract.id = "other-id".into();
        assert!(
            manifest
                .validate()
                .unwrap_err()
                .iter()
                .any(|error| error.contains("does not match"))
        );
    }
}
