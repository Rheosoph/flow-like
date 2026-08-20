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

// Re-export core types and utilities
pub use flow_like_catalog_core::{
    Attachment, BoundingBox, CachedDB, DEFAULT_GRAPH_NEIGHBORS_DIRECTION,
    DEFAULT_GRAPH_OVERLAY_LIMIT, DEFAULT_GRAPH_QUERY_LIMIT, DEFAULT_GRAPH_SAMPLE_SIZE, FlowPath,
    FlowPathRuntime, FlowPathStore, GraphOverlay, NodeDBConnection, NodeImage, NodeImageWrapper,
    ObjectViewDefinition, OntologyActionDefinition, RemoteOntologyImport, is_reserved_table,
    ontology_action_parameter_validator, ontology_binding_nodes, register_node,
    remote_ontology_binding_nodes, validate_ontology_action_parameters,
};

// Re-export standard library
pub use flow_like_catalog_std::{control, logging, math, structs, utils, variables};

// Re-export data integrations
pub use flow_like_catalog_data::{data, events};

// Re-export web modules
pub use flow_like_catalog_web::{http, mail, web};

// Re-export Telegram and Discord modules
pub use flow_like_catalog_web::discord;
pub use flow_like_catalog_web::telegram;

// Re-export media modules
pub use flow_like_catalog_media::{bit, image};

// Re-export ML module
pub use flow_like_catalog_ml::ml;

// Re-export ONNX module
pub use flow_like_catalog_onnx::{onnx, teachable_machine};

// Re-export LLM/GenAI modules
pub use flow_like_catalog_llm::generative;

// Re-export processing modules
pub use flow_like_catalog_geo::geo;
pub use flow_like_catalog_processing::processing;

// Re-export automation modules (desktop only — not available on iOS/Android)
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub use flow_like_catalog_automation::{
    browser, computer, fingerprint, llm as automation_llm, rpa, selector, vision,
};

/// Available catalog packages that can be included/excluded
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatalogPackage {
    Core,
    Std,
    Data,
    Web,
    Media,
    Ml,
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
            CatalogPackage::Std => flow_like_catalog_std::get_catalog(),
            CatalogPackage::Data => flow_like_catalog_data::get_catalog(),
            CatalogPackage::Web => flow_like_catalog_web::get_catalog(),
            CatalogPackage::Media => flow_like_catalog_media::get_catalog(),
            CatalogPackage::Ml => flow_like_catalog_ml::get_catalog(),
            CatalogPackage::Onnx => flow_like_catalog_onnx::get_catalog(),
            CatalogPackage::Llm => flow_like_catalog_llm::get_catalog(),
            CatalogPackage::Processing => flow_like_catalog_processing::get_catalog(),
            CatalogPackage::Geo => flow_like_catalog_geo::get_catalog(),
            CatalogPackage::Automation => flow_like_catalog_automation::get_catalog(),
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
            "automation" | "rpa" | "browser" | "computer" => Ok(CatalogPackage::Automation),
            _ => Err(format!("Unknown catalog package: {}", s)),
        }
    }
}

/// Predicate deciding whether an individual node stays in the built catalog.
pub type NodeFilter = Box<dyn Fn(&dyn NodeLogic) -> bool + Send + Sync>;

/// Builder for constructing a customized catalog
#[derive(Default)]
pub struct CatalogBuilder {
    excluded_packages: HashSet<CatalogPackage>,
    included_packages: Option<HashSet<CatalogPackage>>,
    excluded_nodes: HashSet<String>,
    included_nodes: Option<HashSet<String>>,
    custom_nodes: Vec<Arc<dyn NodeLogic>>,
    node_filter: Option<NodeFilter>,
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
        self.node_filter = Some(Box::new(filter));
        self
    }

    /// Build the catalog with all configurations applied.
    /// Custom nodes override existing nodes with the same name.
    pub fn build(self) -> Vec<Arc<dyn NodeLogic>> {
        let mut node_map: HashMap<String, Arc<dyn NodeLogic>> = HashMap::new();

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
                node_map.insert(name, node);
            }
        }

        // Custom nodes override existing ones
        for node in self.custom_nodes {
            let node_def = node.get_node();
            node_map.insert(node_def.name.clone(), node);
        }

        let mut catalog: Vec<Arc<dyn NodeLogic>> = node_map.into_values().collect();

        if let Some(ref filter) = self.node_filter {
            catalog.retain(|node| filter(node.as_ref()));
        }

        catalog
    }
}

/// Static cached catalog - initialized once on first access
static CATALOG: LazyLock<Vec<Arc<dyn NodeLogic>>> = LazyLock::new(|| CatalogBuilder::new().build());

/// Get the full catalog from all sub-crates (cached, initialized once)
pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    CATALOG.clone()
}

/// Get catalog from specific packages only (not cached - use sparingly)
pub fn get_catalog_from(packages: &[CatalogPackage]) -> Vec<Arc<dyn NodeLogic>> {
    CatalogBuilder::new().only_packages(packages).build()
}

/// Get catalog excluding specific packages (not cached - use sparingly)
pub fn get_catalog_without(packages: &[CatalogPackage]) -> Vec<Arc<dyn NodeLogic>> {
    CatalogBuilder::new().exclude_packages(packages).build()
}

/// Initialize the catalog runtime systems.
///
/// This should be called once at application startup, before any flow execution.
/// It initializes:
/// - ONNX Runtime with the best available execution providers (GPU/NPU acceleration)
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
///     println!("Configured providers: {:?}", info.onnx_providers);
///     println!("Acceleration configured: {}", info.onnx_accelerated);
/// }
/// ```
#[cfg(feature = "execute")]
pub fn initialize() -> InitInfo {
    let onnx_info = flow_like_catalog_onnx::onnx::initialize_ort();
    tracing::info!(
        providers = ?onnx_info.active_providers,
        accelerated = onnx_info.accelerated,
        "ONNX Runtime initialized"
    );
    InitInfo {
        onnx_providers: onnx_info.active_providers,
        onnx_accelerated: onnx_info.accelerated,
        onnx_warnings: onnx_info.warnings,
    }
}

/// Information about initialized runtime systems
#[cfg(feature = "execute")]
#[derive(Debug, Clone, Default)]
pub struct InitInfo {
    /// Configured ONNX execution providers, including CPU fallback
    pub onnx_providers: Vec<String>,
    /// Whether ONNX has a GPU/NPU provider configured
    pub onnx_accelerated: bool,
    /// Any warnings during ONNX initialization
    pub onnx_warnings: Vec<String>,
}

#[cfg(not(feature = "execute"))]
pub fn initialize() -> () {
    // No-op when execute feature is not enabled
}
