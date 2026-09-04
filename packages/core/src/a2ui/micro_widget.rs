//! Micro widget (package widget) contracts and the unified widget provider.
//!
//! The contract types below are a consumer-side mirror of
//! `packages/wasm/schema/src/widget.rs` (`WidgetContract` et al.), which is the
//! source of truth for the `contract.json` interchange format. `flow-like-wasm`
//! depends on this crate, so the types cannot be imported here without a
//! dependency cycle; the serde shape (camelCase) must stay in sync with
//! `widget.rs`.
//!
//! [`WidgetProvider`] unifies the two widget sources a board can instantiate
//! from: the project's declarative A2UI widgets and micro widgets shipped by
//! packages added to the app (`App.packages`). Package widgets are resolved
//! through a host-registered [`PackageWidgetSource`] on [`FlowLikeState`],
//! since installed package manifests live outside this crate.

use crate::app::App;
use crate::flow::board::Board;
use crate::state::FlowLikeState;
use flow_like_types::{Value, async_trait};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::widget::Widget;

/// Selector prefix marking a package widget reference: `pkg:{package_id}/{widget_id}`
pub const PACKAGE_WIDGET_REF_PREFIX: &str = "pkg:";

/// Component type of a package widget instance rendered in a sandboxed iframe.
pub const MICRO_WIDGET_COMPONENT_TYPE: &str = "microWidgetInstance";

/// Encode a package widget reference for the `widget_selector` dropdown.
pub fn encode_package_widget_ref(package_id: &str, widget_id: &str) -> String {
    format!("{PACKAGE_WIDGET_REF_PREFIX}{package_id}/{widget_id}")
}

/// Decode a `pkg:{package_id}/{widget_id}` selector into `(package_id, widget_id)`.
/// Returns `None` for declarative selectors or malformed refs.
pub fn decode_package_widget_ref(selector: &str) -> Option<(&str, &str)> {
    let rest = selector.strip_prefix(PACKAGE_WIDGET_REF_PREFIX)?;
    let (package_id, widget_id) = rest.split_once('/')?;
    if package_id.is_empty() || widget_id.is_empty() {
        return None;
    }
    Some((package_id, widget_id))
}

/// Display label for a package widget entry: `"{widget name} · {package_id}"`.
pub fn package_widget_display_label(name: &str, package_id: &str) -> String {
    format!("{name} · {package_id}")
}

/// Simple pin-type tag derived from a contract input's top-level `type`/`enum`.
/// Mirror of `ContractInputType` in `packages/wasm/schema/src/widget.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContractInputType {
    String,
    Number,
    Integer,
    Boolean,
    Enum,
    Json,
}

/// A single typed widget input. Mirror of `ContractInput` in `packages/wasm/schema/src/widget.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractInput {
    #[serde(rename = "type")]
    pub input_type: ContractInputType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

/// A widget event workflow event nodes can bind to.
/// Mirror of `ContractEvent` in `packages/wasm/schema/src/widget.rs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractEvent {
    #[serde(default)]
    pub payload_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A request/response query on a widget instance.
/// Mirror of `ContractQuery` in `packages/wasm/schema/src/widget.rs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractQuery {
    #[serde(default)]
    pub args_schema: Option<Value>,
    #[serde(default)]
    pub result_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Sizing hints for the host iframe. Mirror of `WidgetSizing` in `packages/wasm/schema/src/widget.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetSizing {
    #[serde(default = "default_height")]
    pub default_height: u32,
    #[serde(default = "default_resizable")]
    pub resizable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
}

fn default_height() -> u32 {
    320
}

fn default_resizable() -> bool {
    true
}

fn default_contract_version() -> u32 {
    1
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

/// Typed contract of a package widget (`contract.json`).
/// Mirror of `WidgetContract` in `packages/wasm/schema/src/widget.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetContract {
    #[serde(default = "default_contract_version")]
    pub contract_version: u32,
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

/// A resolvable package widget entry, sourced from an installed package manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageWidgetRef {
    pub package_id: String,
    /// Version of the providing package (manifest version)
    pub package_version: String,
    /// Widget id, unique within the package
    pub widget_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The manifest's `widget_bundle_hash`, when present
    #[serde(default)]
    pub bundle_hash: Option<String>,
    /// Raw contract JSON exactly as stored in the manifest (camelCase). Kept
    /// verbatim so instances embed the contract byte-for-byte.
    pub contract: Value,
}

impl PackageWidgetRef {
    /// Parse the raw contract into its typed representation.
    pub fn parsed_contract(&self) -> flow_like_types::Result<WidgetContract> {
        flow_like_types::json::from_value(self.contract.clone()).map_err(|e| {
            flow_like_types::anyhow!(
                "Invalid widget contract for '{}/{}': {}",
                self.package_id,
                self.widget_id,
                e
            )
        })
    }

    /// Encoded `pkg:{package_id}/{widget_id}` selector value.
    pub fn selector(&self) -> String {
        encode_package_widget_ref(&self.package_id, &self.widget_id)
    }

    /// Display label for dropdowns / lists.
    pub fn display_label(&self) -> String {
        package_widget_display_label(&self.name, &self.package_id)
    }
}

/// Host-registered source of package widget entries.
///
/// Implementations (desktop registry client, server-side registry) resolve the
/// installed manifests of the given packages. Only the requested packages may
/// be considered — never everything installed locally.
#[async_trait]
pub trait PackageWidgetSource: Send + Sync {
    /// List widget entries for an app's package map (`package_id -> version`).
    async fn list_widgets(
        &self,
        packages: &HashMap<String, String>,
    ) -> flow_like_types::Result<Vec<PackageWidgetRef>>;
}

/// Host-registered supplier of an app's declarative widgets.
///
/// Server executors implement this over the hub API so a run never needs a
/// meta-store credential to instantiate a widget; the desktop leaves it
/// unregistered and [`load_app_widgets`] falls back to the local store.
/// Implementations cache per instance, so after the first call every further
/// instantiation in the same run costs no I/O.
#[async_trait]
pub trait AppWidgetSource: Send + Sync {
    async fn list_app_widgets(&self, app_id: &str) -> flow_like_types::Result<Arc<Vec<Widget>>>;
}

/// An app's declarative widgets, through the registered [`AppWidgetSource`]
/// when there is one and from the meta store otherwise.
pub async fn load_app_widgets(
    app_id: &str,
    state: Arc<FlowLikeState>,
) -> flow_like_types::Result<Arc<Vec<Widget>>> {
    if let Some(source) = state.app_widget_source().await {
        return source.list_app_widgets(app_id).await;
    }
    let app = App::load(app_id.to_string(), state).await?;
    Ok(Arc::new(app.get_widgets().await?))
}

/// A widget resolved through the unified provider.
pub enum ResolvedWidget<'a> {
    Declarative(&'a Widget),
    Package(&'a PackageWidgetRef),
}

/// Unified resolution layer over project declarative widgets and package
/// widgets of the packages added to the app.
#[derive(Default)]
pub struct WidgetProvider {
    declarative: Vec<Widget>,
    package_widgets: Vec<PackageWidgetRef>,
}

impl WidgetProvider {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn new(declarative: Vec<Widget>, package_widgets: Vec<PackageWidgetRef>) -> Self {
        Self {
            declarative,
            package_widgets,
        }
    }

    /// Load both widget sources for an app. Declarative widget failures are
    /// tolerated (empty list, matching existing behavior); package source
    /// errors propagate so callers can surface them.
    pub async fn load(app_id: &str, state: Arc<FlowLikeState>) -> flow_like_types::Result<Self> {
        let declarative = load_app_widgets(app_id, state.clone())
            .await
            .map(|widgets| widgets.as_ref().clone())
            .unwrap_or_default();
        let package_widgets = match state.package_widget_source().await {
            Some(source) => {
                let app = App::load(app_id.to_string(), state.clone()).await?;
                if app.packages.is_empty() {
                    Vec::new()
                } else {
                    source.list_widgets(&app.packages).await?
                }
            }
            None => Vec::new(),
        };
        Ok(Self {
            declarative,
            package_widgets,
        })
    }

    /// Tolerant variant for `on_update`: any failure yields an empty provider.
    pub async fn from_board(board: &Board) -> Self {
        let Some(state) = board.app_state.clone() else {
            return Self::empty();
        };
        let app_id = match board.board_dir.filename() {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return Self::empty(),
        };
        match Self::load(&app_id, state).await {
            Ok(provider) => provider,
            Err(e) => {
                tracing::warn!(app_id = %app_id, error = %e, "Failed to load widget provider");
                Self::empty()
            }
        }
    }

    pub fn declarative_widgets(&self) -> &[Widget] {
        &self.declarative
    }

    pub fn package_widgets(&self) -> &[PackageWidgetRef] {
        &self.package_widgets
    }

    /// All selector values for the `widget_selector` dropdown: declarative
    /// widget names (today's encoding) followed by encoded package refs.
    pub fn selector_values(&self) -> Vec<String> {
        let mut values: Vec<String> = self.declarative.iter().map(|w| w.name.clone()).collect();
        values.extend(self.package_widgets.iter().map(|w| w.selector()));
        values
    }

    /// Resolve a selector: `pkg:` refs resolve to package widgets, everything
    /// else to declarative widgets by id or name.
    pub fn resolve(&self, selector: &str) -> Option<ResolvedWidget<'_>> {
        if let Some((package_id, widget_id)) = decode_package_widget_ref(selector) {
            return self
                .resolve_package(package_id, widget_id)
                .map(ResolvedWidget::Package);
        }
        self.resolve_declarative(selector)
            .map(ResolvedWidget::Declarative)
    }

    pub fn resolve_declarative(&self, selector: &str) -> Option<&Widget> {
        self.declarative
            .iter()
            .find(|w| w.id == selector)
            .or_else(|| self.declarative.iter().find(|w| w.name == selector))
    }

    pub fn resolve_package(&self, package_id: &str, widget_id: &str) -> Option<&PackageWidgetRef> {
        self.package_widgets
            .iter()
            .find(|w| w.package_id == package_id && w.widget_id == widget_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    #[test]
    fn test_package_ref_roundtrip() {
        let encoded = encode_package_widget_ref("com.example.sales", "sales-chart");
        assert_eq!(encoded, "pkg:com.example.sales/sales-chart");
        assert_eq!(
            decode_package_widget_ref(&encoded),
            Some(("com.example.sales", "sales-chart"))
        );
    }

    #[test]
    fn test_decode_rejects_invalid_refs() {
        assert_eq!(decode_package_widget_ref("My Widget"), None);
        assert_eq!(decode_package_widget_ref("pkg:"), None);
        assert_eq!(decode_package_widget_ref("pkg:no-slash"), None);
        assert_eq!(decode_package_widget_ref("pkg:/widget"), None);
        assert_eq!(decode_package_widget_ref("pkg:package/"), None);
    }

    #[test]
    fn test_display_label() {
        assert_eq!(
            package_widget_display_label("Sales Chart", "com.example.sales"),
            "Sales Chart · com.example.sales"
        );
    }

    #[test]
    fn test_contract_parses_camel_case_manifest_json() {
        let raw = json!({
            "contractVersion": 1,
            "id": "sales-chart",
            "inputs": {
                "title": { "type": "string", "default": "Sales", "description": "Chart headline" },
                "variant": { "type": "enum", "choices": ["bar", "line"], "default": "bar" },
                "limit": { "type": "integer", "min": 1, "max": 500, "default": 50 },
                "rows": { "type": "json", "schema": { "type": "array" }, "optional": true }
            },
            "events": {
                "pointSelected": { "payloadSchema": { "type": "object" } },
                "refreshRequested": { "payloadSchema": null }
            },
            "queries": {
                "getValue": { "argsSchema": null, "resultSchema": { "type": "string" } }
            },
            "sizing": { "defaultHeight": 320, "resizable": true }
        });

        let entry = PackageWidgetRef {
            package_id: "com.example.sales".into(),
            package_version: "1.2.0".into(),
            widget_id: "sales-chart".into(),
            name: "Sales Chart".into(),
            description: "Interactive chart".into(),
            bundle_hash: Some("abc123".into()),
            contract: raw,
        };

        let contract = entry.parsed_contract().unwrap();
        assert_eq!(contract.id, "sales-chart");
        assert_eq!(contract.contract_version, 1);
        assert_eq!(contract.inputs.len(), 4);
        assert_eq!(
            contract.inputs["variant"].input_type,
            ContractInputType::Enum
        );
        assert!(contract.inputs["rows"].optional);
        assert!(!contract.inputs["title"].optional);
        assert!(contract.events["pointSelected"].payload_schema.is_some());
        assert!(contract.events["refreshRequested"].payload_schema.is_none());
        assert!(contract.queries["getValue"].result_schema.is_some());
        assert_eq!(contract.sizing.default_height, 320);
    }

    #[test]
    fn test_provider_resolution() {
        let widget = Widget::new("widget-1", "My Widget", "root");

        let pkg = PackageWidgetRef {
            package_id: "com.example.sales".into(),
            package_version: "1.0.0".into(),
            widget_id: "sales-chart".into(),
            name: "Sales Chart".into(),
            description: String::new(),
            bundle_hash: None,
            contract: json!({ "contractVersion": 1, "id": "sales-chart" }),
        };

        let provider = WidgetProvider::new(vec![widget], vec![pkg]);

        assert_eq!(
            provider.selector_values(),
            vec![
                "My Widget".to_string(),
                "pkg:com.example.sales/sales-chart".to_string()
            ]
        );

        assert!(matches!(
            provider.resolve("My Widget"),
            Some(ResolvedWidget::Declarative(_))
        ));
        assert!(matches!(
            provider.resolve("widget-1"),
            Some(ResolvedWidget::Declarative(_))
        ));
        assert!(matches!(
            provider.resolve("pkg:com.example.sales/sales-chart"),
            Some(ResolvedWidget::Package(_))
        ));
        assert!(provider.resolve("pkg:unknown/none").is_none());
        assert!(provider.resolve("missing").is_none());
    }
}

#[cfg(test)]
mod app_widget_source_tests {
    use super::*;
    use crate::state::FlowLikeConfig;
    use flow_like_storage::files::store::FlowLikeStore;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSource {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl AppWidgetSource for CountingSource {
        async fn list_app_widgets(
            &self,
            _app_id: &str,
        ) -> flow_like_types::Result<Arc<Vec<Widget>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Vec::new()))
        }
    }

    fn state() -> Arc<FlowLikeState> {
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ))
    }

    #[tokio::test]
    async fn a_registered_source_replaces_the_store_read() {
        let state = state();
        let source = Arc::new(CountingSource {
            calls: AtomicUsize::new(0),
        });
        state.register_app_widget_source(source.clone()).await;

        load_app_widgets("app-1", state.clone())
            .await
            .expect("served by the source");
        load_app_widgets("app-1", state.clone())
            .await
            .expect("served by the source");
        assert_eq!(
            source.calls.load(Ordering::SeqCst),
            2,
            "the shared helper delegates every call; per-run caching is the source's job"
        );
    }

    #[tokio::test]
    async fn without_a_source_the_store_is_consulted() {
        // Nothing registered and an empty store: the fallback path runs and fails
        // on the missing manifest instead of pretending the app has no widgets.
        let state = state();
        assert!(load_app_widgets("app-1", state).await.is_err());
    }
}
