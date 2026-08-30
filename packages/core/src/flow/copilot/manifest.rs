//! Deterministic, provider-neutral context for a FlowPilot workflow session.
//!
//! A model adapter should not discover the same board, catalog, database, storage, and UI facts
//! independently. This module turns those host-owned facts into one immutable manifest that can be
//! cached, fingerprinted, rendered into a prompt, and replayed through any provider transport.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::flow::board::Board;

use super::{
    context::{GraphContext, prepare_context},
    types::{NodeMetadata, PinMetadata},
};

pub const BOARD_CONTEXT_MANIFEST_VERSION: &str = "flowpilot.context-manifest/v1";
const MANIFEST_FINGERPRINT_DOMAIN: &[u8] = b"flowpilot.context-manifest.fingerprint/v1\0";
const AUGMENTATION_FINGERPRINT_DOMAIN: &[u8] = b"flowpilot.context-manifest.augmentation/v1\0";

/// Semantic board state needed while authoring a workflow. Volatile host fields such as local
/// paths, timestamps, and in-memory logic implementations are deliberately excluded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestBoard {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: (u32, u32, u32),
    pub stage: String,
    pub execution_mode: String,
    pub refs: BTreeMap<String, String>,
    pub page_ids: Vec<String>,
    pub graph: GraphContext,
}

impl ManifestBoard {
    pub fn from_board(board: &Board, selected_node_ids: &[String]) -> Result<Self, ManifestError> {
        let graph = prepare_context(board, selected_node_ids)
            .map_err(|error| ManifestError::BoardContext(error.to_string()))?;
        Ok(Self {
            id: board.id.clone(),
            name: board.name.clone(),
            description: board.description.clone(),
            version: board.version,
            stage: serialized_label(&board.stage)?,
            execution_mode: serialized_label(&board.execution_mode)?,
            refs: board
                .refs
                .iter()
                .filter(|(key, _)| !crate::flow::board::is_internal_board_ref(key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            page_ids: board.page_ids.clone(),
            graph,
        }
        .normalized())
    }

    pub fn normalized(mut self) -> Self {
        normalize_graph_context(&mut self.graph);
        self.page_ids.sort_unstable();
        self.page_ids.dedup();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestCatalogPin {
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub data_type: String,
    pub value_type: String,
    pub default_value: Option<String>,
    pub schema: Option<String>,
    pub is_generic: bool,
    pub valid_values: Vec<String>,
    pub enforce_schema: bool,
}

impl From<&PinMetadata> for ManifestCatalogPin {
    fn from(pin: &PinMetadata) -> Self {
        let mut valid_values = pin.valid_values.clone().unwrap_or_default();
        valid_values.sort_unstable();
        valid_values.dedup();
        Self {
            name: pin.name.clone(),
            friendly_name: pin.friendly_name.clone(),
            description: pin.description.clone(),
            data_type: pin.data_type.clone(),
            value_type: pin.value_type.clone(),
            default_value: pin.default_value.clone(),
            schema: pin.schema.clone(),
            is_generic: pin.is_generic,
            valid_values,
            enforce_schema: pin.enforce_schema,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestCatalogNode {
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub category: Option<String>,
    pub inputs: Vec<ManifestCatalogPin>,
    pub outputs: Vec<ManifestCatalogPin>,
    pub required_inputs: Vec<String>,
    pub companion_nodes: Vec<String>,
    pub capability_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
}

impl From<&NodeMetadata> for ManifestCatalogNode {
    fn from(node: &NodeMetadata) -> Self {
        let mut inputs = node
            .inputs
            .iter()
            .map(ManifestCatalogPin::from)
            .collect::<Vec<_>>();
        let mut outputs = node
            .outputs
            .iter()
            .map(ManifestCatalogPin::from)
            .collect::<Vec<_>>();
        inputs.sort_by(catalog_pin_order);
        outputs.sort_by(catalog_pin_order);

        let mut required_inputs = node.required_inputs.clone();
        let mut companion_nodes = node.companion_nodes.clone();
        let mut capability_tags = node.capability_tags.clone();
        sort_and_dedup(&mut required_inputs);
        sort_and_dedup(&mut companion_nodes);
        sort_and_dedup(&mut capability_tags);

        Self {
            name: node.name.clone(),
            friendly_name: node.friendly_name.clone(),
            description: node.description.clone(),
            category: node.category.clone(),
            inputs,
            outputs,
            required_inputs,
            companion_nodes,
            capability_tags,
            namespace: node.namespace.clone(),
            alias: node.alias.clone(),
            receiver: node.receiver.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestCatalog {
    pub fingerprint: String,
    pub nodes: Vec<ManifestCatalogNode>,
}

impl ManifestCatalog {
    pub fn from_metadata(nodes: &[NodeMetadata]) -> Result<Self, ManifestError> {
        let mut nodes = nodes
            .iter()
            .map(ManifestCatalogNode::from)
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| {
            (&left.name, &left.friendly_name, &left.description).cmp(&(
                &right.name,
                &right.friendly_name,
                &right.description,
            ))
        });
        let bytes = serde_json::to_vec(&nodes)?;
        Ok(Self {
            fingerprint: domain_hash(MANIFEST_FINGERPRINT_DOMAIN, &bytes),
            nodes,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ManifestSourceStatus {
    Absent,
    Existing,
    Retained,
    ValidationErrors,
    Valid,
    Prepared,
}

/// Current source artifact and compiler-owned revision data. Line endings are normalized to LF so
/// the same source loaded on different hosts produces one digest; all other source bytes survive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestSource {
    pub status: ManifestSourceStatus,
    pub revision: Option<u64>,
    pub digest: Option<String>,
    pub flowscript: Option<String>,
    pub diagnostic_fingerprint: Option<String>,
}

impl ManifestSource {
    pub fn new(
        status: ManifestSourceStatus,
        revision: Option<u64>,
        flowscript: Option<String>,
        diagnostic_fingerprint: Option<String>,
    ) -> Self {
        let flowscript = flowscript.map(|source| normalize_line_endings(&source));
        let digest = flowscript
            .as_deref()
            .map(|source| domain_hash(MANIFEST_FINGERPRINT_DOMAIN, source.as_bytes()));
        Self {
            status,
            revision,
            digest,
            flowscript,
            diagnostic_fingerprint,
        }
    }

    pub fn absent() -> Self {
        Self::new(ManifestSourceStatus::Absent, None, None, None)
    }
}

/// Immutable request and build provenance. Callers can add stable host-specific fields through
/// `attributes`; secrets and volatile timestamps should never be placed here.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestAudit {
    pub request_identity: String,
    pub base_fingerprint: String,
    pub acceptance_contract_fingerprint: Option<String>,
    pub build_id: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestAugmentation {
    pub schema: String,
    pub revision: Option<String>,
    pub fingerprint: String,
    pub payload: Value,
}

impl ManifestAugmentation {
    pub fn new(schema: impl Into<String>, revision: Option<String>, payload: Value) -> Self {
        let payload = canonicalize_json(payload);
        let bytes = serde_json::to_vec(&payload).unwrap_or_default();
        Self {
            schema: schema.into(),
            revision,
            fingerprint: domain_hash(AUGMENTATION_FINGERPRINT_DOMAIN, &bytes),
            payload,
        }
    }
}

/// The fixed slots make cross-provider feature parity inspectable. `extensions` remains available
/// for future host context without changing the manifest schema for every new read-only domain.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestAugmentations {
    pub database: Option<ManifestAugmentation>,
    pub ui: Option<ManifestAugmentation>,
    pub storage: Option<ManifestAugmentation>,
    #[serde(default)]
    pub extensions: BTreeMap<String, ManifestAugmentation>,
}

impl ManifestAugmentations {
    /// Convert the frontend-owned `flowpilot.board-context-augmentation/v1` envelope into the
    /// provider-neutral slots. `generated_at_ms` is intentionally never copied: it is useful for
    /// cache expiry at the host boundary but must not invalidate an otherwise identical manifest.
    pub fn from_host_value(value: Option<&Value>) -> Self {
        let Some(root) = value.and_then(Value::as_object) else {
            return Self::default();
        };
        let schema = root
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or("flowpilot.board-context-augmentation/v1");
        let host_truncated = root
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let slot = |name: &str| {
            root.get(name).cloned().map(|mut payload| {
                if host_truncated && let Some(payload) = payload.as_object_mut() {
                    // The frontend can compact several domains to stay within one byte ceiling.
                    // Conservatively expose every resulting slot as incomplete so the agent may
                    // perform one diagnostic-driven read instead of trusting an omitted item.
                    payload.insert("complete".to_string(), Value::Bool(false));
                    payload.insert("host_manifest_truncated".to_string(), Value::Bool(true));
                }
                ManifestAugmentation::new(
                    format!("{schema}#{name}"),
                    None,
                    remove_volatile_manifest_fields(payload),
                )
            })
        };
        Self {
            database: slot("data"),
            ui: slot("ui"),
            storage: slot("storage"),
            extensions: BTreeMap::new(),
        }
    }
}

/// A reusable source-building unit. These are planning descriptors, not separately committed
/// artifacts: adapters compose every module into the same retained FlowScript workspace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowScriptModuleTemplate {
    pub id: String,
    pub order: u16,
    pub title: String,
    pub purpose: String,
    pub entrypoint_hint: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub acceptance_checks: Vec<String>,
}

impl FlowScriptModuleTemplate {
    pub fn normalized(mut self) -> Self {
        sort_and_dedup(&mut self.depends_on);
        sort_and_dedup(&mut self.required_capabilities);
        sort_and_dedup(&mut self.acceptance_checks);
        self
    }
}

/// Generic module sequence suitable for both small and cross-domain workflows. Callers can
/// replace it with request-specific descriptors while retaining the same manifest contract.
pub fn default_flowscript_module_templates() -> Vec<FlowScriptModuleTemplate> {
    vec![
        FlowScriptModuleTemplate {
            id: "foundation".to_string(),
            order: 10,
            title: "Foundation and entrypoints".to_string(),
            purpose: "Define Events, typed boundaries, constants, and stable board anchors."
                .to_string(),
            entrypoint_hint: Some("eventsSimple()".to_string()),
            depends_on: vec![],
            required_capabilities: vec![],
            acceptance_checks: vec!["Every requested trigger has an explicit Event.".to_string()],
        },
        FlowScriptModuleTemplate {
            id: "inputs_and_access".to_string(),
            order: 20,
            title: "Inputs and access".to_string(),
            purpose: "Load inputs, credentials, roles, and authorization gates.".to_string(),
            entrypoint_hint: None,
            depends_on: vec!["foundation".to_string()],
            required_capabilities: vec![],
            acceptance_checks: vec![
                "Secrets use host-owned secret references, never prompt-authored plaintext."
                    .to_string(),
            ],
        },
        FlowScriptModuleTemplate {
            id: "domain_logic".to_string(),
            order: 30,
            title: "Domain logic".to_string(),
            purpose: "Implement retrieval, transformation, decisions, and state changes."
                .to_string(),
            entrypoint_hint: None,
            depends_on: vec!["inputs_and_access".to_string()],
            required_capabilities: vec![],
            acceptance_checks: vec![
                "Every requested capability has an executable path.".to_string(),
            ],
        },
        FlowScriptModuleTemplate {
            id: "outputs_and_review".to_string(),
            order: 40,
            title: "Outputs and review".to_string(),
            purpose: "Deliver results, approval branches, and explicit failure behavior."
                .to_string(),
            entrypoint_hint: None,
            depends_on: vec!["domain_logic".to_string()],
            required_capabilities: vec![],
            acceptance_checks: vec![
                "Success, rejection, and failure branches terminate deliberately.".to_string(),
            ],
        },
        FlowScriptModuleTemplate {
            id: "observability".to_string(),
            order: 50,
            title: "Observability and reconciliation".to_string(),
            purpose: "Add useful logs and reconcile the composed source against acceptance."
                .to_string(),
            entrypoint_hint: None,
            depends_on: vec!["outputs_and_review".to_string()],
            required_capabilities: vec![],
            acceptance_checks: vec![
                "The complete composed source compiles before review is prepared.".to_string(),
            ],
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoardContextManifest {
    pub schema: String,
    pub fingerprint: String,
    pub board: ManifestBoard,
    pub catalog: ManifestCatalog,
    pub source: ManifestSource,
    pub audit: ManifestAudit,
    pub augmentations: ManifestAugmentations,
    pub module_templates: Vec<FlowScriptModuleTemplate>,
}

impl BoardContextManifest {
    pub fn build(
        board: ManifestBoard,
        catalog: &[NodeMetadata],
        source: ManifestSource,
        audit: ManifestAudit,
        augmentations: ManifestAugmentations,
        module_templates: Vec<FlowScriptModuleTemplate>,
    ) -> Result<Self, ManifestError> {
        let mut module_templates = module_templates
            .into_iter()
            .map(FlowScriptModuleTemplate::normalized)
            .collect::<Vec<_>>();
        module_templates.sort_by(|left, right| {
            (left.order, &left.id, &left.title).cmp(&(right.order, &right.id, &right.title))
        });
        let mut manifest = Self {
            schema: BOARD_CONTEXT_MANIFEST_VERSION.to_string(),
            fingerprint: String::new(),
            board: board.normalized(),
            catalog: ManifestCatalog::from_metadata(catalog)?,
            source,
            audit,
            augmentations,
            module_templates,
        };
        manifest.fingerprint = manifest.compute_fingerprint()?;
        Ok(manifest)
    }

    pub fn from_board(
        board: &Board,
        selected_node_ids: &[String],
        catalog: &[NodeMetadata],
        source: ManifestSource,
        audit: ManifestAudit,
        augmentations: ManifestAugmentations,
        module_templates: Vec<FlowScriptModuleTemplate>,
    ) -> Result<Self, ManifestError> {
        Self::build(
            ManifestBoard::from_board(board, selected_node_ids)?,
            catalog,
            source,
            audit,
            augmentations,
            module_templates,
        )
    }

    pub fn verify_fingerprint(&self) -> Result<bool, ManifestError> {
        Ok(self.fingerprint == self.compute_fingerprint()?)
    }

    /// Render one byte-stable prompt block. Adapters may prepend provider-specific mechanics, but
    /// the factual context remains identical and independently fingerprinted.
    pub fn render_prompt(&self) -> Result<String, ManifestError> {
        let json = serde_json::to_string_pretty(self)?;
        Ok(format!(
            "FLOWPILOT BOARD CONTEXT (immutable; {BOARD_CONTEXT_MANIFEST_VERSION})\n\
             Use this manifest as the authoritative cached board/catalog/context snapshot.\n\
             Do not repeat a context read unless a compiler diagnostic identifies a missing fact.\n\
             ```json\n{json}\n```"
        ))
    }

    /// Render only authoring facts that are not already present in FlowPilot's existing board and
    /// catalog system prompt. This is the preferred cross-provider injection during migration: it
    /// carries augmentation payloads, completeness, module boundaries, and stable identities while
    /// avoiding a second copy of the full graph, catalog contracts, or FlowScript source.
    pub fn render_authoring_prompt(&self) -> Result<String, ManifestError> {
        #[derive(Serialize)]
        struct AugmentationView<'a> {
            available: bool,
            complete: bool,
            schema: Option<&'a str>,
            fingerprint: Option<&'a str>,
            payload: Option<&'a Value>,
        }

        impl<'a> From<Option<&'a ManifestAugmentation>> for AugmentationView<'a> {
            fn from(slot: Option<&'a ManifestAugmentation>) -> Self {
                Self {
                    available: slot.is_some(),
                    complete: slot
                        .and_then(|slot| slot.payload.get("complete"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    schema: slot.map(|slot| slot.schema.as_str()),
                    fingerprint: slot.map(|slot| slot.fingerprint.as_str()),
                    payload: slot.map(|slot| &slot.payload),
                }
            }
        }

        #[derive(Serialize)]
        struct SourceView<'a> {
            status: ManifestSourceStatus,
            revision: Option<u64>,
            digest: Option<&'a str>,
            diagnostic_fingerprint: Option<&'a str>,
        }

        #[derive(Serialize)]
        struct AuthoringView<'a> {
            schema: &'a str,
            manifest_fingerprint: &'a str,
            catalog_fingerprint: &'a str,
            board_id: &'a str,
            source: SourceView<'a>,
            database: AugmentationView<'a>,
            ui: AugmentationView<'a>,
            storage: AugmentationView<'a>,
            extensions: BTreeMap<&'a str, AugmentationView<'a>>,
            module_templates: &'a [FlowScriptModuleTemplate],
        }

        let view = AuthoringView {
            schema: &self.schema,
            manifest_fingerprint: &self.fingerprint,
            catalog_fingerprint: &self.catalog.fingerprint,
            board_id: &self.board.id,
            source: SourceView {
                status: self.source.status,
                revision: self.source.revision,
                digest: self.source.digest.as_deref(),
                diagnostic_fingerprint: self.source.diagnostic_fingerprint.as_deref(),
            },
            database: self.augmentations.database.as_ref().into(),
            ui: self.augmentations.ui.as_ref().into(),
            storage: self.augmentations.storage.as_ref().into(),
            extensions: self
                .augmentations
                .extensions
                .iter()
                .map(|(name, slot)| (name.as_str(), Some(slot).into()))
                .collect(),
            module_templates: &self.module_templates,
        };
        let json = serde_json::to_string_pretty(&view)?;
        Ok(format!(
            "FLOWPILOT AUTHORING MANIFEST (immutable; shared by every model backend)\n\
             Reuse these cached host facts. Missing or incomplete slots may justify one focused read; complete slots must not be inventoried again.\n\
             Build the reusable modules into one retained FlowScript artifact and reconcile the complete source before review.\n\
             ```json\n{json}\n```"
        ))
    }

    fn compute_fingerprint(&self) -> Result<String, ManifestError> {
        #[derive(Serialize)]
        struct FingerprintMaterial<'a> {
            schema: &'a str,
            board: &'a ManifestBoard,
            catalog: &'a ManifestCatalog,
            source: &'a ManifestSource,
            audit: &'a ManifestAudit,
            augmentations: &'a ManifestAugmentations,
            module_templates: &'a [FlowScriptModuleTemplate],
        }
        let material = FingerprintMaterial {
            schema: &self.schema,
            board: &self.board,
            catalog: &self.catalog,
            source: &self.source,
            audit: &self.audit,
            augmentations: &self.augmentations,
            module_templates: &self.module_templates,
        };
        Ok(domain_hash(
            MANIFEST_FINGERPRINT_DOMAIN,
            &serde_json::to_vec(&material)?,
        ))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("could not prepare board context: {0}")]
    BoardContext(String),
    #[error("could not serialize FlowPilot context manifest: {0}")]
    Serialization(#[from] serde_json::Error),
}

fn serialized_label<T: Serialize>(value: &T) -> Result<String, ManifestError> {
    let value = serde_json::to_value(value)?;
    Ok(match value {
        Value::String(value) => value,
        value => serde_json::to_string(&canonicalize_json(value))?,
    })
}

fn normalize_graph_context(graph: &mut GraphContext) {
    for node in &mut graph.nodes {
        node.inputs.sort_by(|left, right| {
            (&left.name, &left.type_name, &left.default_value).cmp(&(
                &right.name,
                &right.type_name,
                &right.default_value,
            ))
        });
        node.outputs.sort_by(|left, right| {
            (&left.name, &left.type_name, &left.default_value).cmp(&(
                &right.name,
                &right.type_name,
                &right.default_value,
            ))
        });
    }
    graph.nodes.sort_by(|left, right| {
        (&left.id, &left.node_type, &left.friendly_name).cmp(&(
            &right.id,
            &right.node_type,
            &right.friendly_name,
        ))
    });
    graph.edges.sort_by(|left, right| {
        (
            &left.from_node_id,
            &left.from_pin_name,
            &left.to_node_id,
            &left.to_pin_name,
        )
            .cmp(&(
                &right.from_node_id,
                &right.from_pin_name,
                &right.to_node_id,
                &right.to_pin_name,
            ))
    });
    for layer in &mut graph.layers {
        sort_and_dedup(&mut layer.node_ids);
        layer.inputs.sort_by(|left, right| {
            (&left.name, &left.type_name, &left.default_value).cmp(&(
                &right.name,
                &right.type_name,
                &right.default_value,
            ))
        });
        layer.outputs.sort_by(|left, right| {
            (&left.name, &left.type_name, &left.default_value).cmp(&(
                &right.name,
                &right.type_name,
                &right.default_value,
            ))
        });
    }
    graph
        .layers
        .sort_by(|left, right| (&left.id, &left.name).cmp(&(&right.id, &right.name)));
    graph.variables.sort_by(|left, right| {
        (&left.id, &left.name, &left.data_type).cmp(&(&right.id, &right.name, &right.data_type))
    });
    sort_and_dedup(&mut graph.selected_nodes);
}

fn catalog_pin_order(left: &ManifestCatalogPin, right: &ManifestCatalogPin) -> std::cmp::Ordering {
    (&left.name, &left.data_type, &left.value_type).cmp(&(
        &right.name,
        &right.data_type,
        &right.value_type,
    ))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(canonicalize_json)
                .collect::<Vec<_>>(),
        ),
        Value::Object(values) => {
            let sorted = values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        value => value,
    }
}

fn remove_volatile_manifest_fields(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(remove_volatile_manifest_fields)
                .collect(),
        ),
        Value::Object(values) => {
            let sorted = values
                .into_iter()
                .filter(|(key, _)| key != "generated_at_ms")
                .map(|(key, value)| (key, remove_volatile_manifest_fields(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        value => value,
    }
}

fn normalize_line_endings(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn sort_and_dedup(values: &mut Vec<String>) {
    values.sort_unstable();
    values.dedup();
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(bytes);
    format!("b3:{}", hasher.finalize().to_hex())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::flow::copilot::{
        EdgeContext, LayerCacheContext, LayerContext, NodeContext, PinContext, VariableContext,
    };

    fn pin(name: &str) -> PinMetadata {
        PinMetadata {
            name: name.to_string(),
            friendly_name: name.to_uppercase(),
            description: format!("{name} pin"),
            data_type: "String".to_string(),
            value_type: "Normal".to_string(),
            default_value: None,
            schema: None,
            is_generic: false,
            valid_values: Some(vec!["z".to_string(), "a".to_string()]),
            enforce_schema: false,
        }
    }

    fn node(name: &str, input_name: &str) -> NodeMetadata {
        NodeMetadata {
            name: name.to_string(),
            friendly_name: format!("Friendly {name}"),
            description: format!("Description {name}"),
            inputs: vec![pin(input_name)],
            outputs: vec![pin("result")],
            category: Some("test".to_string()),
            required_inputs: vec![input_name.to_string()],
            companion_nodes: vec![],
            capability_tags: vec!["write".to_string(), "read".to_string()],
            namespace: None,
            alias: None,
            receiver: None,
        }
    }

    fn graph(reverse: bool) -> GraphContext {
        let mut nodes = vec![
            NodeContext {
                id: "b".to_string(),
                node_type: "beta".to_string(),
                friendly_name: "Beta".to_string(),
                inputs: vec![],
                outputs: vec![],
                position: (2, 2),
                estimated_size: (200, 32),
            },
            NodeContext {
                id: "a".to_string(),
                node_type: "alpha".to_string(),
                friendly_name: "Alpha".to_string(),
                inputs: vec![PinContext {
                    name: "value".to_string(),
                    type_name: "String".to_string(),
                    default_value: None,
                }],
                outputs: vec![],
                position: (1, 1),
                estimated_size: (200, 52),
            },
        ];
        if reverse {
            nodes.reverse();
        }
        GraphContext {
            nodes,
            edges: vec![EdgeContext {
                from_node_id: "a".to_string(),
                from_pin_name: "result".to_string(),
                to_node_id: "b".to_string(),
                to_pin_name: "value".to_string(),
            }],
            layers: Vec::<LayerContext>::new(),
            variables: Vec::<VariableContext>::new(),
            selected_nodes: if reverse {
                vec!["b".to_string(), "a".to_string()]
            } else {
                vec!["a".to_string(), "b".to_string()]
            },
        }
    }

    fn board(graph: GraphContext) -> ManifestBoard {
        ManifestBoard {
            id: "board-1".to_string(),
            name: "Board".to_string(),
            description: "Test board".to_string(),
            version: (1, 2, 3),
            stage: "dev".to_string(),
            execution_mode: "hybrid".to_string(),
            refs: BTreeMap::from([
                ("z".to_string(), "last".to_string()),
                ("a".to_string(), "first".to_string()),
            ]),
            page_ids: vec!["page-b".to_string(), "page-a".to_string()],
            graph,
        }
    }

    fn audit() -> ManifestAudit {
        ManifestAudit {
            request_identity: "request-1".to_string(),
            base_fingerprint: "base-1".to_string(),
            acceptance_contract_fingerprint: Some("acceptance-1".to_string()),
            build_id: Some("test-build".to_string()),
            attributes: BTreeMap::new(),
        }
    }

    #[test]
    fn manifest_and_prompt_are_independent_of_input_order() {
        let mut catalog_left = vec![node("zeta", "z"), node("alpha", "a")];
        let mut catalog_right = catalog_left.clone();
        catalog_right.reverse();
        catalog_left[0].capability_tags.reverse();

        let left = BoardContextManifest::build(
            board(graph(false)),
            &catalog_left,
            ManifestSource::new(
                ManifestSourceStatus::Existing,
                Some(3),
                Some("Event start() {\r\n  return\r\n}".to_string()),
                None,
            ),
            audit(),
            ManifestAugmentations {
                database: Some(ManifestAugmentation::new(
                    "database/v1",
                    Some("7".to_string()),
                    json!({"tables": {"z": 2, "a": 1}}),
                )),
                ..ManifestAugmentations::default()
            },
            default_flowscript_module_templates(),
        )
        .expect("left manifest");
        let right = BoardContextManifest::build(
            board(graph(true)),
            &catalog_right,
            ManifestSource::new(
                ManifestSourceStatus::Existing,
                Some(3),
                Some("Event start() {\n  return\n}".to_string()),
                None,
            ),
            audit(),
            ManifestAugmentations {
                database: Some(ManifestAugmentation::new(
                    "database/v1",
                    Some("7".to_string()),
                    json!({"tables": {"a": 1, "z": 2}}),
                )),
                ..ManifestAugmentations::default()
            },
            default_flowscript_module_templates(),
        )
        .expect("right manifest");

        assert_eq!(left, right);
        assert_eq!(
            left.render_prompt().expect("left prompt"),
            right.render_prompt().expect("right prompt")
        );
        assert!(left.verify_fingerprint().expect("verify fingerprint"));
    }

    #[test]
    fn semantic_source_change_changes_manifest_fingerprint() {
        let build = |source: &str| {
            BoardContextManifest::build(
                board(graph(false)),
                &[node("alpha", "value")],
                ManifestSource::new(
                    ManifestSourceStatus::Existing,
                    Some(1),
                    Some(source.to_string()),
                    None,
                ),
                audit(),
                ManifestAugmentations::default(),
                default_flowscript_module_templates(),
            )
            .expect("manifest")
        };
        assert_ne!(
            build("Event a() {}\n").fingerprint,
            build("Event b() {}\n").fingerprint
        );
    }

    #[test]
    fn function_cache_is_part_of_the_manifest_and_its_fingerprint() {
        let build = |cache: Option<LayerCacheContext>| {
            let mut graph = graph(false);
            graph.layers.push(LayerContext {
                id: "pricing-layer".to_string(),
                name: "calculatePricing".to_string(),
                layer_type: "Function".to_string(),
                parent_id: None,
                node_ids: vec![],
                position: (0, 0),
                inputs: vec![],
                outputs: vec![],
                cache,
            });
            BoardContextManifest::build(
                board(graph),
                &[],
                ManifestSource::absent(),
                audit(),
                ManifestAugmentations::default(),
                default_flowscript_module_templates(),
            )
            .expect("manifest")
        };

        let uncached = build(None);
        let cached = build(Some(LayerCacheContext {
            enabled: true,
            namespace: "pricing".to_string(),
            ttl_seconds: Some(3_600),
            scope: "user".to_string(),
        }));

        assert_ne!(uncached.fingerprint, cached.fingerprint);
        assert!(cached.verify_fingerprint().expect("verify fingerprint"));
        let prompt = cached.render_prompt().expect("full manifest prompt");
        assert!(prompt.contains("\"namespace\": \"pricing\""));
        assert!(prompt.contains("\"ttl_seconds\": 3600"));
        assert!(prompt.contains("\"scope\": \"user\""));
    }

    #[test]
    fn host_augmentation_splits_slots_and_ignores_generated_timestamp() {
        let host = |generated_at_ms| {
            json!({
                "schema": "flowpilot.board-context-augmentation/v1",
                "app_id": "app-1",
                "board_id": "board-1",
                "generated_at_ms": generated_at_ms,
                "data": {"complete": true, "tables": [{"table_name": "users"}]},
                "ui": {"complete": false, "pages": [], "errors": ["unavailable"]},
                "storage": {"complete": true, "project_items": []},
                "truncated": false,
            })
        };
        let early = host(10);
        let late = host(99_999);
        let early = ManifestAugmentations::from_host_value(Some(&early));
        let late = ManifestAugmentations::from_host_value(Some(&late));

        assert_eq!(early, late);
        assert_eq!(
            early
                .database
                .as_ref()
                .and_then(|slot| slot.payload.get("complete")),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            early
                .ui
                .as_ref()
                .and_then(|slot| slot.payload.get("complete")),
            Some(&Value::Bool(false))
        );
        assert!(early.storage.is_some());
    }

    #[test]
    fn truncated_host_augmentation_never_claims_complete_slots() {
        let host = json!({
            "schema": "flowpilot.board-context-augmentation/v1",
            "generated_at_ms": 42,
            "data": {"complete": true, "tables": []},
            "ui": {"complete": true, "pages": []},
            "storage": {"complete": true, "project_items": []},
            "truncated": true,
        });
        let slots = ManifestAugmentations::from_host_value(Some(&host));
        for slot in [slots.database, slots.ui, slots.storage] {
            let payload = &slot.expect("slot").payload;
            assert_eq!(payload.get("complete"), Some(&Value::Bool(false)));
            assert_eq!(
                payload.get("host_manifest_truncated"),
                Some(&Value::Bool(true))
            );
        }
    }

    #[test]
    fn compact_authoring_prompt_omits_full_graph_catalog_and_source() {
        let host = json!({
            "schema": "flowpilot.board-context-augmentation/v1",
            "generated_at_ms": 42,
            "data": {"complete": true, "tables": [{"table_name": "orders"}]},
            "ui": {"complete": true, "pages": [{"name": "Dashboard"}]},
            "storage": {"complete": true, "project_items": []},
        });
        let manifest = BoardContextManifest::build(
            board(graph(false)),
            &[node("catalog_node_that_must_not_be_duplicated", "value")],
            ManifestSource::new(
                ManifestSourceStatus::Existing,
                Some(2),
                Some("Event secret_full_source() {}".to_string()),
                None,
            ),
            audit(),
            ManifestAugmentations::from_host_value(Some(&host)),
            default_flowscript_module_templates(),
        )
        .expect("manifest");
        let prompt = manifest
            .render_authoring_prompt()
            .expect("authoring prompt");

        assert!(prompt.contains(&manifest.fingerprint));
        assert!(prompt.contains(&manifest.catalog.fingerprint));
        assert!(prompt.contains("orders"));
        assert!(prompt.contains("Dashboard"));
        assert!(prompt.contains("\"complete\": true"));
        assert!(prompt.contains("foundation"));
        assert!(!prompt.contains("catalog_node_that_must_not_be_duplicated"));
        assert!(!prompt.contains("secret_full_source"));
        assert!(!prompt.contains("generated_at_ms"));
        assert!(!prompt.contains("selected_nodes"));
    }

    #[test]
    fn module_template_sets_are_normalized_without_changing_module_order() {
        let mut templates = default_flowscript_module_templates();
        templates.reverse();
        templates[0]
            .depends_on
            .push("outputs_and_review".to_string());
        let manifest = BoardContextManifest::build(
            board(graph(false)),
            &[],
            ManifestSource::absent(),
            audit(),
            ManifestAugmentations::default(),
            templates,
        )
        .expect("manifest");

        assert_eq!(manifest.module_templates[0].id, "foundation");
        assert_eq!(
            manifest.module_templates.last().unwrap().id,
            "observability"
        );
        assert_eq!(
            manifest.module_templates.last().unwrap().depends_on,
            vec!["outputs_and_review".to_string()]
        );
    }
}
