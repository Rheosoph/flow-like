//! Flow-Like Catalog - Aggregate crate for all catalog modules
//!
//! This crate re-exports all catalog sub-crates for convenience.
//! You can also depend on individual catalog crates directly.
//!
//! # Usage
//!
//! ```rust,ignore
//! use flow_like_catalog::{CatalogBuilder, CatalogPackage, get_catalog};
//!
//! // Get the full catalog
//! let catalog = get_catalog();
//!
//! // Or use the builder for customization
//! let catalog = CatalogBuilder::new()
//!     .exclude_packages(&[CatalogPackage::Onnx, CatalogPackage::Ml])
//!     .with_custom_nodes(my_custom_nodes())  // Custom nodes override existing ones by name
//!     .build();
//!
//! // Include only specific packages
//! let catalog = CatalogBuilder::new()
//!     .only_packages(&[CatalogPackage::Core, CatalogPackage::Std])
//!     .build();
//!
//! // Include/exclude specific nodes by name
//! let catalog = CatalogBuilder::new()
//!     .only_nodes(&["control_branch", "bool_or"])  // Only include these nodes
//!     .build();
//!
//! let catalog = CatalogBuilder::new()
//!     .exclude_nodes(&["control_branch"])  // Exclude specific nodes
//!     .build();
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

pub use flow_like_catalog_core::NodeLogic;
use flow_like_runtime::{flow::node::Node, state::FlowNodeRegistryInner};

#[cfg(feature = "draft-testing")]
pub mod draft_test;

#[cfg(all(test, feature = "draft-testing"))]
mod benchmark_case_tests;

// Re-export core types and utilities
#[cfg(any(feature = "portable-execute", feature = "compat"))]
pub use flow_like_catalog_core::CachedDB;
pub use flow_like_catalog_core::{
    Attachment, BoundingBox, DEFAULT_GRAPH_NEIGHBORS_DIRECTION, DEFAULT_GRAPH_OVERLAY_LIMIT,
    DEFAULT_GRAPH_QUERY_LIMIT, DEFAULT_GRAPH_SAMPLE_SIZE, FlowPath, FlowPathRuntime, FlowPathStore,
    GraphOverlay, NodeDBConnection, NodeImage, NodeImageWrapper, ObjectViewDefinition,
    OntologyActionDefinition, RemoteOntologyImport, is_reserved_table,
    ontology_action_parameter_validator, ontology_binding_nodes, register_node,
    remote_ontology_binding_nodes, validate_ontology_action_parameters,
};

// Re-export standard library
#[cfg(feature = "package-std")]
pub use flow_like_catalog_std::{control, logging, math, structs, utils, variables};

// Re-export data integrations
#[cfg(feature = "package-data")]
pub use flow_like_catalog_data::{data, events};

// Re-export web modules
#[cfg(feature = "package-web")]
pub use flow_like_catalog_web::{http, mail, web};

// Re-export Telegram and Discord modules
#[cfg(feature = "package-web")]
pub use flow_like_catalog_web::discord;
#[cfg(feature = "package-web")]
pub use flow_like_catalog_web::telegram;

// Re-export media modules
#[cfg(feature = "package-media")]
pub use flow_like_catalog_media::{bit, image};

#[cfg(feature = "package-industrial")]
pub use flow_like_catalog_industrial as industrial;

// Re-export ML module
#[cfg(feature = "package-ml")]
pub use flow_like_catalog_ml::ml;

// Re-export ONNX module
#[cfg(feature = "package-onnx")]
pub use flow_like_catalog_onnx::{onnx, teachable_machine};

// Re-export LLM/GenAI modules
#[cfg(feature = "package-llm")]
pub use flow_like_catalog_llm::generative;

// Re-export processing modules
#[cfg(feature = "package-geo")]
pub use flow_like_catalog_geo::geo;
#[cfg(feature = "package-processing")]
pub use flow_like_catalog_processing::processing;

// Re-export automation modules (desktop only — not available on iOS/Android)
#[cfg(all(
    feature = "package-automation",
    not(any(target_os = "ios", target_os = "android"))
))]
pub use flow_like_catalog_automation::{
    browser, capabilities as automation_capabilities, computer, fingerprint, llm as automation_llm,
    rpa, selector, types::screen_match as automation_screen, vision,
};

macro_rules! package_nodes {
    ($feature:literal, $get_catalog:path) => {{
        #[cfg(feature = $feature)]
        {
            $get_catalog()
        }
        #[cfg(not(feature = $feature))]
        {
            Vec::new()
        }
    }};
}

/// Available catalog packages that can be included/excluded
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatalogPackage {
    Core,
    Std,
    Data,
    Web,
    Media,
    Ml,
    Industrial,
    Onnx,
    Llm,
    Processing,
    Geo,
    Automation,
}

impl CatalogPackage {
    pub fn all() -> &'static [CatalogPackage] {
        &[
            CatalogPackage::Core,
            CatalogPackage::Std,
            CatalogPackage::Data,
            CatalogPackage::Web,
            CatalogPackage::Media,
            CatalogPackage::Ml,
            CatalogPackage::Industrial,
            CatalogPackage::Onnx,
            CatalogPackage::Llm,
            CatalogPackage::Processing,
            CatalogPackage::Geo,
            CatalogPackage::Automation,
        ]
    }

    fn get_nodes(&self) -> Vec<Arc<dyn NodeLogic>> {
        match self {
            CatalogPackage::Core => Vec::new(),
            CatalogPackage::Std => {
                package_nodes!("package-std", flow_like_catalog_std::get_catalog)
            }
            CatalogPackage::Data => {
                package_nodes!("package-data", flow_like_catalog_data::get_catalog)
            }
            CatalogPackage::Web => {
                package_nodes!("package-web", flow_like_catalog_web::get_catalog)
            }
            CatalogPackage::Media => {
                package_nodes!("package-media", flow_like_catalog_media::get_catalog)
            }
            CatalogPackage::Industrial => {
                package_nodes!(
                    "package-industrial",
                    flow_like_catalog_industrial::get_catalog
                )
            }
            CatalogPackage::Ml => {
                package_nodes!("package-ml", flow_like_catalog_ml::get_catalog)
            }
            CatalogPackage::Onnx => {
                package_nodes!("package-onnx", flow_like_catalog_onnx::get_catalog)
            }
            CatalogPackage::Llm => {
                package_nodes!("package-llm", flow_like_catalog_llm::get_catalog)
            }
            CatalogPackage::Processing => package_nodes!(
                "package-processing",
                flow_like_catalog_processing::get_catalog
            ),
            CatalogPackage::Geo => {
                package_nodes!("package-geo", flow_like_catalog_geo::get_catalog)
            }
            CatalogPackage::Automation => package_nodes!(
                "package-automation",
                flow_like_catalog_automation::get_catalog
            ),
        }
    }

    /// Stable lowercase label for this package, used as a `.flow.d` filename stem and as the
    /// `Signature.package` tag.
    pub fn label(&self) -> &'static str {
        match self {
            CatalogPackage::Core => "core",
            CatalogPackage::Std => "std",
            CatalogPackage::Data => "data",
            CatalogPackage::Web => "web",
            CatalogPackage::Media => "media",
            CatalogPackage::Ml => "ml",
            CatalogPackage::Industrial => "industrial",
            CatalogPackage::Onnx => "onnx",
            CatalogPackage::Llm => "llm",
            CatalogPackage::Processing => "processing",
            CatalogPackage::Geo => "geo",
            CatalogPackage::Automation => "automation",
        }
    }
}

/// A catalog node paired with the label of the package it ships in.
///
/// Built-in nodes carry their [`CatalogPackage`] label; third-party nodes injected at runtime
/// carry whatever label the caller assigns (defaulting to `custom`). This is the input for the
/// per-package `.flow.d` generator so any project's catalog — built-ins plus injected packages —
/// can be documented one package at a time.
pub struct LabeledNode {
    pub package: String,
    pub node: Arc<dyn NodeLogic>,
}

/// Enumerate every built-in catalog node tagged with its source package label.
///
/// Later entries win on name collisions, mirroring [`CatalogBuilder::build`] override semantics.
pub fn labeled_catalog() -> Vec<LabeledNode> {
    let mut out: Vec<LabeledNode> = Vec::new();
    for package in CatalogPackage::all() {
        let label = package.label();
        for node in package.get_nodes() {
            out.push(LabeledNode {
                package: label.to_string(),
                node,
            });
        }
    }
    out
}

impl std::str::FromStr for CatalogPackage {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "core" => Ok(CatalogPackage::Core),
            "std" | "standard" => Ok(CatalogPackage::Std),
            "data" => Ok(CatalogPackage::Data),
            "web" => Ok(CatalogPackage::Web),
            "media" => Ok(CatalogPackage::Media),
            "ml" | "machine_learning" => Ok(CatalogPackage::Ml),
            "onnx" => Ok(CatalogPackage::Onnx),
            "llm" | "genai" | "generative" => Ok(CatalogPackage::Llm),
            "processing" => Ok(CatalogPackage::Processing),
            "geo" | "geolocation" => Ok(CatalogPackage::Geo),
            "industrial" => Ok(CatalogPackage::Industrial),
            "automation" | "rpa" | "browser" | "computer" => Ok(CatalogPackage::Automation),
            _ => Err(format!("Unknown catalog package: {}", s)),
        }
    }
}

/// Predicate deciding whether an individual node stays in the built catalog.
pub type NodeFilter = Box<dyn Fn(&dyn NodeLogic) -> bool + Send + Sync>;

enum CatalogNodeFilter {
    Logic(NodeFilter),
    #[cfg(any(feature = "runtime-catalog", test))]
    Definition(fn(name: &str, category: &str) -> bool),
}

impl CatalogNodeFilter {
    fn accepts_definition(&self, _name: &str, _category: &str) -> bool {
        match self {
            Self::Logic(_) => true,
            #[cfg(any(feature = "runtime-catalog", test))]
            Self::Definition(filter) => filter(_name, _category),
        }
    }
}

/// Builder for constructing a customized catalog
#[derive(Default)]
pub struct CatalogBuilder {
    excluded_packages: HashSet<CatalogPackage>,
    included_packages: Option<HashSet<CatalogPackage>>,
    excluded_nodes: HashSet<String>,
    included_nodes: Option<HashSet<String>>,
    custom_nodes: Vec<Arc<dyn NodeLogic>>,
    node_filter: Option<CatalogNodeFilter>,
}

impl CatalogBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Include only specific packages (whitelist mode)
    pub fn only_packages(mut self, packages: &[CatalogPackage]) -> Self {
        self.included_packages = Some(packages.iter().copied().collect());
        self
    }

    /// Include only specific packages by name (whitelist mode)
    pub fn only_packages_by_name(mut self, names: &[&str]) -> Self {
        let packages: HashSet<_> = names
            .iter()
            .filter_map(|n| n.parse::<CatalogPackage>().ok())
            .collect();
        self.included_packages = Some(packages);
        self
    }

    /// Exclude specific packages
    pub fn exclude_packages(mut self, packages: &[CatalogPackage]) -> Self {
        self.excluded_packages.extend(packages.iter().copied());
        self
    }

    /// Exclude packages by name
    pub fn exclude_packages_by_name(mut self, names: &[&str]) -> Self {
        for name in names {
            if let Ok(pkg) = name.parse::<CatalogPackage>() {
                self.excluded_packages.insert(pkg);
            }
        }
        self
    }

    /// Include only specific nodes by name (whitelist mode)
    pub fn only_nodes(mut self, names: &[&str]) -> Self {
        self.included_nodes = Some(names.iter().map(|s| s.to_string()).collect());
        self
    }

    /// Exclude specific nodes by name
    pub fn exclude_nodes(mut self, names: &[&str]) -> Self {
        self.excluded_nodes
            .extend(names.iter().map(|s| s.to_string()));
        self
    }

    /// Add custom nodes to the catalog.
    /// Custom nodes will override any existing nodes with the same name.
    pub fn with_custom_nodes(mut self, nodes: Vec<Arc<dyn NodeLogic>>) -> Self {
        self.custom_nodes.extend(nodes);
        self
    }

    /// Add a single custom node.
    /// If a node with the same name already exists, it will be replaced.
    pub fn with_node(mut self, node: Arc<dyn NodeLogic>) -> Self {
        self.custom_nodes.push(node);
        self
    }

    /// Set a custom filter function for nodes.
    /// The filter receives a reference to the NodeLogic trait object.
    pub fn with_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&dyn NodeLogic) -> bool + Send + Sync + 'static,
    {
        self.node_filter = Some(CatalogNodeFilter::Logic(Box::new(filter)));
        self
    }

    #[cfg(any(feature = "runtime-catalog", test))]
    fn with_definition_filter(mut self, filter: fn(&str, &str) -> bool) -> Self {
        self.node_filter = Some(CatalogNodeFilter::Definition(filter));
        self
    }

    /// Build the catalog with all configurations applied.
    /// Custom nodes override existing nodes with the same name.
    pub fn build(self) -> Vec<Arc<dyn NodeLogic>> {
        self.build_entries(drop)
            .into_iter()
            .map(|(_, logic)| logic)
            .collect()
    }

    /// Build the registry from the definitions already read during selection.
    /// Each retained definition moves into the registry; no second metadata
    /// copy or wrapper around its `NodeLogic` is retained.
    pub fn build_registry(self) -> FlowNodeRegistryInner {
        let entries = self.build_entries(std::convert::identity);
        let mut registry = FlowNodeRegistryInner::new(entries.len());
        for (definition, logic) in entries {
            registry.insert(definition, logic);
        }
        registry
    }

    fn build_entries<T>(
        self,
        retain_definition: impl Fn(Node) -> T,
    ) -> Vec<(T, Arc<dyn NodeLogic>)> {
        let mut node_map: HashMap<String, (T, Arc<dyn NodeLogic>, bool)> = HashMap::new();

        let packages_to_include: Vec<CatalogPackage> =
            if let Some(ref included) = self.included_packages {
                included.iter().copied().collect()
            } else {
                CatalogPackage::all()
                    .iter()
                    .filter(|p| !self.excluded_packages.contains(p))
                    .copied()
                    .collect()
            };

        for package in packages_to_include {
            if self.excluded_packages.contains(&package) {
                continue;
            }
            for node in package.get_nodes() {
                let node_def = node.get_node();
                let name = node_def.name.clone();
                if self.excluded_nodes.contains(&name) {
                    continue;
                }
                if let Some(ref included) = self.included_nodes
                    && !included.contains(&name)
                {
                    continue;
                }
                let keep = self
                    .node_filter
                    .as_ref()
                    .is_none_or(|filter| filter.accepts_definition(&name, &node_def.category));
                node_map.insert(name, (retain_definition(node_def), node, keep));
            }
        }

        // Custom nodes override existing ones
        for node in self.custom_nodes {
            let node_def = node.get_node();
            let keep = self
                .node_filter
                .as_ref()
                .is_none_or(|filter| filter.accepts_definition(&node_def.name, &node_def.category));
            // A rejected override still replaces an earlier allowed node.
            node_map.insert(
                node_def.name.clone(),
                (retain_definition(node_def), node, keep),
            );
        }

        let mut catalog: Vec<(T, Arc<dyn NodeLogic>)> = node_map
            .into_values()
            .filter_map(|(definition, node, keep)| keep.then_some((definition, node)))
            .collect();

        if let Some(CatalogNodeFilter::Logic(ref filter)) = self.node_filter {
            catalog.retain(|(_, node)| filter(node.as_ref()));
        }

        catalog
    }
}

#[cfg(feature = "runtime-catalog")]
fn runtime_implementation_available(name: &str, category: &str) -> bool {
    if matches!(
        category,
        "AI/ML/Lifecycle"
            | "AI/ML/Compute"
            | "AI/ML/Auto Training"
            | "AI/ML/Continuous Learning"
            | "AI/ML/Export"
    ) && !cfg!(feature = "training-metadata")
    {
        return false;
    }
    match name {
        name if name.starts_with("industrial_ethercat_") => {
            cfg!(any(target_os = "linux", target_os = "macos"))
        }
        name if name.starts_with("industrial_cifx_") => {
            cfg!(any(target_os = "linux", target_os = "windows"))
        }
        "ml_export_student_onnx" => cfg!(feature = "training-onnx-export"),
        // Their categories also hold nodes that run without a training build, such as the
        // pure AI/ML/Inference helpers, so these are gated by name.
        "ml_prompt_sam2"
        | "ml_fine_tune_sam2"
        | "ml_extract_pretrained_features"
        | "ml_import_pretrained_vision"
        | "ml_list_pretrained_vision"
        | "ml_predict_published_model"
        | "ml_extract_published_features" => cfg!(feature = "training-metadata"),
        "ml_read_opcua_sensor" => cfg!(feature = "package-industrial"),
        "ml_capture_genicam_frame" => cfg!(feature = "sensor-genicam"),
        "ai_audio_local_text_to_speech" => cfg!(feature = "local-tts"),
        "ai_audio_local_speech_to_text" => cfg!(feature = "local-stt"),
        "df_register_oracle" | "df_register_athena" => cfg!(feature = "odbc"),
        _ => true,
    }
}

#[cfg(all(test, feature = "package-ml", feature = "runtime-catalog"))]
mod inspection_catalog_tests {
    use super::*;

    const TRAINING_METADATA_NODES: &[&str] = &[
        "ml_train_burn_model",
        "ml_student_inference",
        "ml_promote_student",
        "ml_probe_training_device",
        "ml_auto_train_tabular",
        "ml_auto_train_forecast",
        "ml_auto_train_vision",
        "ml_auto_train_anomaly",
        "ml_get_auto_training",
        "ml_predict_auto_model",
        "ml_export_auto_model",
        "ml_create_learning_project",
        "ml_step_learning_project",
        "ml_route_learning_project",
        "ml_prompt_sam2",
        "ml_fine_tune_sam2",
        "ml_extract_pretrained_features",
        "ml_import_pretrained_vision",
        "ml_list_pretrained_vision",
        "ml_predict_published_model",
        "ml_extract_published_features",
        "ml_export_model",
        "ml_import_model",
        "ml_export_training_ledger",
        "ml_import_training_ledger",
    ];

    const NODES_WITHOUT_TRAINING: &[&str] = &[
        "ml_training_readiness",
        "ml_preprocess_inspection_image",
        "ml_native_model_inference",
        "ml_apply_decision_threshold",
        "ml_student_confidence_gate",
        "ml_apply_calibration",
    ];

    #[test]
    fn inspection_nodes_follow_runtime_features() {
        let catalog = get_catalog_from(&[CatalogPackage::Ml, CatalogPackage::Industrial]);
        let has = |name: &str| catalog.iter().any(|node| node.get_node().name == name);
        for &name in TRAINING_METADATA_NODES {
            assert_eq!(has(name), cfg!(feature = "training-metadata"), "{name}");
        }
        for &name in NODES_WITHOUT_TRAINING {
            assert!(has(name), "{name} runs without a training build");
        }
        assert_eq!(
            has("ml_export_student_onnx"),
            cfg!(feature = "training-onnx-export")
        );
        assert_eq!(
            has("ml_read_opcua_sensor"),
            cfg!(feature = "package-industrial")
        );
        assert_eq!(
            has("ml_capture_genicam_frame"),
            cfg!(feature = "sensor-genicam")
        );
        assert!(has("ml_make_inspection_sample"));
    }
}

fn product_catalog_builder() -> CatalogBuilder {
    let builder = CatalogBuilder::new();
    #[cfg(feature = "runtime-catalog")]
    {
        return builder.with_definition_filter(runtime_implementation_available);
    }
    #[cfg(not(feature = "runtime-catalog"))]
    builder
}

#[cfg(test)]
mod builder_filter_tests {
    use super::*;
    use flow_like::flow::{execution::context::ExecutionContext, node::Node};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingNode {
        name: String,
        category: String,
        only_offline: bool,
        calls: Arc<AtomicUsize>,
    }

    #[flow_like_types::async_trait]
    impl NodeLogic for CountingNode {
        fn get_node(&self) -> Node {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let mut node = Node::new(&self.name, "Test", "", &self.category);
            node.only_offline = self.only_offline;
            node
        }

        async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            Ok(())
        }
    }

    fn counting_node(name: &str, category: &str) -> (Arc<dyn NodeLogic>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(CountingNode {
                name: name.to_string(),
                category: category.to_string(),
                only_offline: false,
                calls: calls.clone(),
            }),
            calls,
        )
    }

    #[test]
    fn definition_filter_materializes_each_node_once() {
        let (allowed, allowed_calls) = counting_node("allowed", "keep");
        let (denied, denied_calls) = counting_node("denied", "drop");
        let catalog = CatalogBuilder::new()
            .only_packages(&[])
            .with_definition_filter(|_, category| category == "keep")
            .with_custom_nodes(vec![allowed.clone(), denied])
            .build();

        assert_eq!(catalog.len(), 1);
        assert!(Arc::ptr_eq(&catalog[0], &allowed));
        assert_eq!(allowed_calls.load(Ordering::Relaxed), 1);
        assert_eq!(denied_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn definition_filter_applies_to_the_final_override() {
        for (first_category, last_category, kept) in
            [("keep", "drop", false), ("drop", "keep", true)]
        {
            let (first, first_calls) = counting_node("same-name", first_category);
            let (last, last_calls) = counting_node("same-name", last_category);
            let catalog = CatalogBuilder::new()
                .only_packages(&[])
                .with_definition_filter(|_, category| category == "keep")
                .with_custom_nodes(vec![first, last.clone()])
                .build();
            assert_eq!(catalog.len(), usize::from(kept));
            if kept {
                assert!(Arc::ptr_eq(&catalog[0], &last));
            }
            assert_eq!(first_calls.load(Ordering::Relaxed), 1);
            assert_eq!(last_calls.load(Ordering::Relaxed), 1);
        }
    }

    #[test]
    fn public_filter_keeps_its_logic_callback_and_replacement_semantics() {
        let (first, first_calls) = counting_node("same-name", "first");
        let (last, last_calls) = counting_node("same-name", "last");
        let filter_calls = Arc::new(AtomicUsize::new(0));
        let observed = filter_calls.clone();
        let catalog = CatalogBuilder::new()
            .only_packages(&[])
            .with_definition_filter(|_, _| false)
            .with_filter(|_| false)
            .with_filter(move |logic| {
                observed.fetch_add(1, Ordering::Relaxed);
                logic.get_node().category == "last"
            })
            .with_custom_nodes(vec![first, last.clone()])
            .build();

        assert_eq!(catalog.len(), 1);
        assert!(Arc::ptr_eq(&catalog[0], &last));
        assert_eq!(filter_calls.load(Ordering::Relaxed), 1);
        assert_eq!(first_calls.load(Ordering::Relaxed), 1);
        assert_eq!(last_calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn custom_nodes_keep_bypassing_package_node_selection() {
        let (custom, calls) = counting_node("custom", "keep");
        let catalog = CatalogBuilder::new()
            .only_packages(&[])
            .only_nodes(&["another-node"])
            .exclude_nodes(&["custom"])
            .with_definition_filter(|_, category| category == "keep")
            .with_node(custom.clone())
            .build();

        assert_eq!(catalog.len(), 1);
        assert!(Arc::ptr_eq(&catalog[0], &custom));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn direct_registry_consumes_each_definition_once_and_preserves_restrictions() {
        let (online, online_calls) = counting_node("online", "Test");
        let offline_calls = Arc::new(AtomicUsize::new(0));
        let offline: Arc<dyn NodeLogic> = Arc::new(CountingNode {
            name: "offline".into(),
            category: "Test".into(),
            only_offline: true,
            calls: offline_calls.clone(),
        });
        let registry = CatalogBuilder::new()
            .only_packages(&[])
            .with_custom_nodes(vec![online.clone(), offline.clone()])
            .build_registry();

        assert_eq!(registry.registry.len(), 2);
        let (online_definition, online_logic) = &registry.registry["online"];
        let (offline_definition, offline_logic) = &registry.registry["offline"];
        assert!(!online_definition.only_offline);
        assert!(offline_definition.only_offline);
        assert!(Arc::ptr_eq(online_logic, &online));
        assert!(Arc::ptr_eq(offline_logic, &offline));
        assert!(online_definition.namespace.is_some());
        assert!(online_definition.alias.is_some());
        assert_eq!(online_calls.load(Ordering::Relaxed), 1);
        assert_eq!(offline_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn direct_registry_matches_legacy_selection_and_fingerprint() {
        let (overridden, _) = counting_node("same-name", "drop");
        let (replacement, replacement_calls) = counting_node("same-name", "keep");
        let (denied, _) = counting_node("denied", "drop");
        let selected = || {
            CatalogBuilder::new()
                .only_packages(&[])
                .only_nodes(&["unrelated-package-node"])
                .exclude_nodes(&["same-name"])
                .with_definition_filter(|_, category| category == "keep")
                .with_custom_nodes(vec![
                    overridden.clone(),
                    replacement.clone(),
                    denied.clone(),
                ])
        };
        let legacy = FlowNodeRegistryInner::prepare(&Arc::new(selected().build()));
        let direct = selected().build_registry();

        assert_eq!(legacy.registry.len(), 1);
        assert_eq!(direct.registry.len(), 1);
        assert_eq!(direct.fingerprint(), legacy.fingerprint());
        assert!(Arc::ptr_eq(&direct.registry["same-name"].1, &replacement));
        assert_eq!(replacement_calls.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn direct_registry_keeps_public_filter_callback_semantics() {
        let (first, first_calls) = counting_node("same-name", "first");
        let (last, last_calls) = counting_node("same-name", "last");
        let filter_calls = Arc::new(AtomicUsize::new(0));
        let observed = filter_calls.clone();
        let registry = CatalogBuilder::new()
            .only_packages(&[])
            .with_filter(move |logic| {
                observed.fetch_add(1, Ordering::Relaxed);
                logic.get_node().category == "last"
            })
            .with_custom_nodes(vec![first, last.clone()])
            .build_registry();

        assert_eq!(registry.registry.len(), 1);
        assert!(Arc::ptr_eq(&registry.registry["same-name"].1, &last));
        assert_eq!(filter_calls.load(Ordering::Relaxed), 1);
        assert_eq!(first_calls.load(Ordering::Relaxed), 1);
        assert_eq!(last_calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn product_registry_fingerprint_matches_legacy_preparation() {
        let legacy = FlowNodeRegistryInner::prepare(&Arc::new(get_catalog()));
        let direct = get_catalog_registry();
        assert_eq!(direct.registry.len(), legacy.registry.len());
        assert_eq!(direct.fingerprint(), legacy.fingerprint());
        for (name, (definition, _)) in &legacy.registry {
            assert_eq!(
                direct.registry[name].0.only_offline, definition.only_offline,
                "{name}"
            );
        }
    }

    #[cfg(feature = "package-std")]
    #[test]
    fn package_selection_and_exclusions_keep_their_precedence() {
        let name = CatalogPackage::Std
            .get_nodes()
            .first()
            .unwrap()
            .get_node()
            .name;
        let selected = || {
            CatalogBuilder::new()
                .only_packages(&[CatalogPackage::Std])
                .only_nodes(&[name.as_str()])
                .with_definition_filter(|_, _| true)
        };
        assert_eq!(selected().build().len(), 1);
        assert!(
            selected()
                .exclude_nodes(&[name.as_str()])
                .build()
                .is_empty()
        );
        assert!(
            selected()
                .exclude_packages(&[CatalogPackage::Std])
                .build()
                .is_empty()
        );

        let (override_node, calls) = counting_node(&name, "override");
        let overridden = selected().with_node(override_node.clone()).build();
        assert_eq!(overridden.len(), 1);
        assert!(Arc::ptr_eq(&overridden[0], &override_node));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[cfg(feature = "runtime-catalog")]
    #[test]
    fn product_feature_filters_reuse_the_existing_definition() {
        let cases = [
            ("ordinary", "Test", true),
            (
                "training",
                "AI/ML/Compute",
                cfg!(feature = "training-metadata"),
            ),
            (
                "ml_prompt_sam2",
                "Test",
                cfg!(feature = "training-metadata"),
            ),
            (
                "ml_export_student_onnx",
                "Test",
                cfg!(feature = "training-onnx-export"),
            ),
            (
                "ml_read_opcua_sensor",
                "Test",
                cfg!(feature = "package-industrial"),
            ),
            (
                "ml_capture_genicam_frame",
                "Test",
                cfg!(feature = "sensor-genicam"),
            ),
            (
                "ai_audio_local_text_to_speech",
                "Test",
                cfg!(feature = "local-tts"),
            ),
            (
                "ai_audio_local_speech_to_text",
                "Test",
                cfg!(feature = "local-stt"),
            ),
            ("df_register_oracle", "Test", cfg!(feature = "odbc")),
            ("df_register_athena", "Test", cfg!(feature = "odbc")),
            (
                "industrial_ethercat_probe",
                "Test",
                cfg!(any(target_os = "linux", target_os = "macos")),
            ),
            (
                "industrial_cifx_probe",
                "Test",
                cfg!(any(target_os = "linux", target_os = "windows")),
            ),
        ];
        for (name, category, kept) in cases {
            let (node, calls) = counting_node(name, category);
            let catalog = product_catalog_builder()
                .only_packages(&[])
                .with_node(node.clone())
                .build();
            assert_eq!(catalog.len(), usize::from(kept), "{name}");
            if kept {
                assert!(Arc::ptr_eq(&catalog[0], &node));
            }
            assert_eq!(calls.load(Ordering::Relaxed), 1, "{name}");
            let registry = product_catalog_builder()
                .only_packages(&[])
                .with_node(node.clone())
                .build_registry();
            assert_eq!(registry.registry.len(), usize::from(kept), "{name}");
            if kept {
                assert!(Arc::ptr_eq(&registry.registry[name].1, &node));
            }
            assert_eq!(calls.load(Ordering::Relaxed), 2, "{name}");
        }
    }
}

/// Static cached product catalog, initialized once on first access.
static CATALOG: LazyLock<Vec<Arc<dyn NodeLogic>>> =
    LazyLock::new(|| product_catalog_builder().build());

/// Get the full catalog from all sub-crates (cached, initialized once)
pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    CATALOG.clone()
}

/// Build the product registry without constructing each node's definition twice.
/// This does not use a process cache; the host owns the returned registry's lifetime.
pub fn get_catalog_registry() -> FlowNodeRegistryInner {
    product_catalog_builder().build_registry()
}

/// Get catalog from specific packages only (not cached - use sparingly)
pub fn get_catalog_from(packages: &[CatalogPackage]) -> Vec<Arc<dyn NodeLogic>> {
    product_catalog_builder().only_packages(packages).build()
}

/// Get catalog excluding specific packages (not cached - use sparingly)
pub fn get_catalog_without(packages: &[CatalogPackage]) -> Vec<Arc<dyn NodeLogic>> {
    product_catalog_builder().exclude_packages(packages).build()
}

#[cfg(all(test, feature = "package-data", feature = "package-media"))]
mod catalog_surface_tests {
    use super::*;

    fn has_node(catalog: &[Arc<dyn NodeLogic>], name: &str) -> bool {
        catalog.iter().any(|logic| logic.get_node().name == name)
    }

    #[test]
    fn raw_builder_keeps_optional_nodes_for_metadata_tooling() {
        let catalog = CatalogBuilder::new().build();

        for name in [
            "ai_audio_local_text_to_speech",
            "ai_audio_local_speech_to_text",
            "df_register_oracle",
            "df_register_athena",
        ] {
            assert!(has_node(&catalog, name), "raw catalog omitted {name}");
        }
    }

    #[cfg(feature = "runtime-catalog")]
    #[test]
    fn product_catalog_omits_optional_nodes_without_their_runtime() {
        let catalog = get_catalog();

        assert_eq!(
            has_node(&catalog, "ai_audio_local_text_to_speech"),
            cfg!(feature = "local-tts")
        );
        assert_eq!(
            has_node(&catalog, "ai_audio_local_speech_to_text"),
            cfg!(feature = "local-stt")
        );
        for name in ["df_register_oracle", "df_register_athena"] {
            assert_eq!(has_node(&catalog, name), cfg!(feature = "odbc"));
        }
    }
}

/// Initialize the catalog runtime systems.
///
/// This should be called once at application startup, before any flow execution.
/// With `local-ml` enabled, it initializes ONNX Runtime with the best available
/// execution providers. Remote-only and metadata-only builds return an empty
/// status without loading an inference runtime.
///
/// # Returns
///
/// Information about the initialized execution providers.
///
/// # Example
///
/// ```rust,ignore
/// use flow_like_catalog::initialize;
///
/// fn main() {
///     let info = initialize();
///     println!("ONNX configured: {}", info.onnx_configured);
///     println!("Configured providers: {:?}", info.onnx_providers);
///     println!("Acceleration configured: {}", info.onnx_accelerated);
/// }
/// ```
pub fn initialize() -> InitInfo {
    #[cfg(feature = "local-ml")]
    {
        let onnx_info = flow_like_catalog_onnx::onnx::initialize_ort();
        if onnx_info.configured {
            tracing::info!(
                providers = ?onnx_info.active_providers,
                accelerated = onnx_info.accelerated,
                "ONNX Runtime initialized"
            );
        } else {
            tracing::warn!(
                providers = ?onnx_info.active_providers,
                warnings = ?onnx_info.warnings,
                "ONNX Runtime configuration is unavailable"
            );
        }
        return InitInfo {
            onnx_providers: onnx_info.active_providers,
            onnx_accelerated: onnx_info.accelerated,
            onnx_configured: onnx_info.configured,
            onnx_warnings: onnx_info.warnings,
        };
    }

    #[cfg(not(feature = "local-ml"))]
    {
        tracing::debug!("ONNX Runtime initialization skipped because local ML is disabled");
        InitInfo::default()
    }
}

/// Information about initialized runtime systems
#[derive(Debug, Clone, Default)]
pub struct InitInfo {
    /// Configured ONNX execution providers, including CPU fallback.
    /// Empty when local ML execution is disabled.
    pub onnx_providers: Vec<String>,
    /// Whether ONNX has a GPU/NPU provider configured
    pub onnx_accelerated: bool,
    /// Whether FlowLike configured the process-wide ONNX Runtime environment
    pub onnx_configured: bool,
    /// Any warnings during ONNX initialization
    pub onnx_warnings: Vec<String>,
}
