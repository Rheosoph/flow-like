use async_trait::async_trait;
use flow_like_ast::model::{Container, TypeRef};
use flow_like_ast::{SigParam, Signature, to_camel_case};
use std::collections::HashSet;

use super::declarations::{render_declaration_matches, search_declarations};
use super::search::score_catalog_metadata;
use super::types::{NodeMetadata, PinMetadata};
use crate::flow::node::Node;
use crate::flow::pin::{Pin, PinType};
use crate::flow::variable::VariableType;

/// Trait for providing catalog search functionality
#[async_trait]
pub trait CatalogProvider: Send + Sync {
    async fn search(&self, query: &str) -> Vec<NodeMetadata>;
    async fn search_by_pin_type(&self, pin_type: &str, is_input: bool) -> Vec<NodeMetadata>;
    async fn filter_by_category(&self, category_prefix: &str) -> Vec<NodeMetadata>;
    async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata>;
    async fn get_all_nodes(&self) -> Vec<String>;

    /// Return metadata for the full catalog. FlowScript reconciliation uses this to resolve
    /// parsed camelCase calls back to catalog node types without asking the model to manually
    /// emit command JSON.
    async fn get_all_metadata(&self) -> Vec<NodeMetadata> {
        let node_types = self.get_all_nodes().await;
        let mut metadata = Vec::with_capacity(node_types.len());
        for node_type in node_types {
            if let Some(meta) = self.get_node_metadata(&node_type).await {
                metadata.push(meta);
            }
        }
        metadata
    }

    /// Render `.flow.d`-style FlowScript declarations for nodes matching `query`.
    ///
    /// This is FlowPilot's type-reference lookup: instead of inspecting nodes pin-by-pin, the
    /// agent retrieves the exact `declare function …` signatures (camelCase node type, typed
    /// params, `@impure` marker) for the nodes it wants to write in FlowScript. The default
    /// implementation derives signatures from the same metadata `search` returns, so every
    /// provider — including ones that inject third-party packages into the catalog — supports it
    /// without extra wiring.
    async fn get_declarations(&self, query: &str) -> String {
        let declaration_matches = search_declarations(query);
        if query.trim().is_empty() {
            return render_declaration_matches(query, &declaration_matches);
        }

        let embedded_function_names: HashSet<String> = declaration_matches
            .iter()
            .map(|matched| matched.function_name.clone())
            .collect();
        let mut live_matches: Vec<(i32, NodeMetadata)> = self
            .get_all_metadata()
            .await
            .into_iter()
            .filter_map(|meta| {
                let signature = metadata_to_signature(&meta);
                if embedded_function_names.contains(&signature.display) {
                    return None;
                }
                let score = score_catalog_metadata(&meta, query);
                (score > 0).then_some((score, meta))
            })
            .collect();
        live_matches.sort_by(|left, right| right.0.cmp(&left.0));
        let live_matches: Vec<NodeMetadata> = live_matches
            .into_iter()
            .take(12)
            .map(|(_, meta)| meta)
            .collect();

        if declaration_matches.is_empty() && live_matches.is_empty() {
            return render_declaration_matches(query, &[]);
        }

        let mut out = if declaration_matches.is_empty() {
            format!(
                "// FlowScript declarations matched {query:?} from the live app catalog provider.\n\
                 // The embedded .flow.d index had no direct hit, so these compact signatures were rendered from metadata.\n\n",
            )
        } else {
            render_declaration_matches(query, &declaration_matches)
        };

        if !live_matches.is_empty() && !declaration_matches.is_empty() {
            out.push_str("\n// Additional live app catalog declarations, including installed package nodes:\n\n");
        }

        let start_idx = if declaration_matches.is_empty() {
            0
        } else {
            declaration_matches.len()
        };
        for (idx, meta) in live_matches.iter().enumerate() {
            let signature = metadata_to_signature(meta);
            out.push_str(&format!(
                "{}. {} — {} [{}]\n",
                start_idx + idx + 1,
                signature.display,
                signature
                    .doc
                    .as_deref()
                    .map(compact_doc_line)
                    .unwrap_or_else(|| signature
                        .friendly
                        .clone()
                        .unwrap_or_else(|| signature.display.clone())),
                meta.category
                    .clone()
                    .unwrap_or_else(|| "catalog".to_string())
            ));
            out.push_str("   ");
            out.push_str(&metadata_signature_line(&signature));
            out.push_str("\n\n");
        }
        out
    }
}

fn compact_doc_line(doc: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 120;
    let doc = doc.replace('\n', " ");
    let mut out = String::with_capacity(doc.len().min(MAX_SUMMARY_CHARS));
    for (idx, ch) in doc.trim().chars().enumerate() {
        if idx >= MAX_SUMMARY_CHARS {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

fn metadata_signature_line(signature: &crate::flow::ast::Signature) -> String {
    signature
        .render_declaration()
        .lines()
        .find(|line| line.trim_start().starts_with("declare function "))
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

/// FlowScript base type for a metadata pin's `data_type` string (the `Debug` spelling of the
/// core `VariableType`). Unknown / generic types collapse to `any`.
fn base_type(data_type: &str) -> &'static str {
    match data_type {
        "String" => "string",
        "Integer" => "int",
        "Float" => "float",
        "Boolean" => "bool",
        "Date" => "Date",
        "PathBuf" => "Path",
        "Struct" => "Struct",
        "Byte" => "bytes",
        "Execution" => "exec",
        _ => "any",
    }
}

/// FlowScript container shape for a metadata pin's `value_type` string.
fn container(value_type: &str) -> Container {
    match value_type {
        "Array" => Container::Array,
        "HashMap" => Container::Map,
        "HashSet" => Container::Set,
        _ => Container::Normal,
    }
}

fn pin_to_sig_param(pin: &PinMetadata) -> SigParam {
    let doc = {
        let d = pin.description.trim();
        (!d.is_empty()).then(|| d.to_string())
    };
    SigParam {
        name: pin.name.clone(),
        ty: TypeRef::new(base_type(&pin.data_type), container(&pin.value_type)),
        optional: pin
            .default_value
            .as_ref()
            .is_some_and(|v| !v.trim().is_empty()),
        doc,
        schema: pin.schema.clone(),
    }
}

/// Convert a board/catalog pin into the metadata shape FlowPilot and FlowScript use.
pub fn pin_to_metadata(pin: &Pin) -> PinMetadata {
    let is_generic = pin.data_type == VariableType::Generic;
    let enforce_schema = pin
        .options
        .as_ref()
        .and_then(|options| options.enforce_schema)
        .unwrap_or(false);
    let valid_values = pin
        .options
        .as_ref()
        .and_then(|options| options.valid_values.clone());

    PinMetadata {
        name: pin.name.clone(),
        friendly_name: pin.friendly_name.clone(),
        description: pin.description.clone(),
        data_type: format!("{:?}", pin.data_type),
        value_type: format!("{:?}", pin.value_type),
        default_value: pin
            .default_value
            .as_ref()
            .map(|value| String::from_utf8_lossy(value).to_string())
            .filter(|value| !value.is_empty() && value != "null"),
        schema: pin.schema.clone(),
        is_generic,
        valid_values,
        enforce_schema,
    }
}

/// Convert a board/catalog node into the metadata shape FlowPilot and FlowScript use.
pub fn node_to_metadata(node: &Node) -> NodeMetadata {
    let derived_category = node
        .name
        .to_lowercase()
        .split("::")
        .nth(1)
        .unwrap_or("")
        .to_string();
    let category = if derived_category.is_empty() {
        node.category.clone()
    } else {
        derived_category
    };

    let mut inputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input)
        .collect();
    inputs.sort_by_key(|pin| (pin.index, pin.name.clone()));

    let mut outputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Output)
        .collect();
    outputs.sort_by_key(|pin| (pin.index, pin.name.clone()));

    super::search::enrich_node_metadata(NodeMetadata {
        name: node.name.clone(),
        friendly_name: node.friendly_name.clone(),
        description: node.description.clone(),
        inputs: inputs.into_iter().map(pin_to_metadata).collect(),
        outputs: outputs.into_iter().map(pin_to_metadata).collect(),
        category: Some(category),
        required_inputs: Vec::new(),
        companion_nodes: Vec::new(),
        capability_tags: Vec::new(),
    })
}

/// Build a FlowScript [`Signature`] from catalog [`NodeMetadata`].
///
/// Mirrors `flow::ast::node_to_signature` but works off the already-flattened metadata the
/// providers expose, so the copilot can render declarations without re-reading the catalog.
/// Execution pins carry control flow (not data) so they are excluded from params and instead set
/// the `impure` flag.
pub fn metadata_to_signature(meta: &NodeMetadata) -> Signature {
    let impure = meta
        .inputs
        .iter()
        .chain(meta.outputs.iter())
        .any(|p| p.data_type == "Execution");

    let inputs = meta
        .inputs
        .iter()
        .filter(|p| p.data_type != "Execution")
        .map(pin_to_sig_param)
        .collect();
    let outputs = meta
        .outputs
        .iter()
        .filter(|p| p.data_type != "Execution")
        .map(pin_to_sig_param)
        .collect();

    let friendly = {
        let f = meta.friendly_name.trim();
        (!f.is_empty()).then(|| f.to_string())
    };
    let category = meta
        .category
        .as_ref()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .map(|c| c.to_string());
    let doc = {
        let d = meta.description.trim();
        (!d.is_empty()).then(|| d.to_string())
    };

    Signature {
        node_type: meta.name.clone(),
        display: to_camel_case(&meta.name),
        friendly,
        category,
        package: None,
        inputs,
        outputs,
        impure,
        doc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::tokio;

    struct LiveOnlyProvider {
        nodes: Vec<NodeMetadata>,
    }

    #[async_trait]
    impl CatalogProvider for LiveOnlyProvider {
        async fn search(&self, _query: &str) -> Vec<NodeMetadata> {
            self.nodes.clone()
        }

        async fn search_by_pin_type(&self, _pin_type: &str, _is_input: bool) -> Vec<NodeMetadata> {
            Vec::new()
        }

        async fn filter_by_category(&self, _category_prefix: &str) -> Vec<NodeMetadata> {
            Vec::new()
        }

        async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata> {
            self.nodes
                .iter()
                .find(|node| node.name == node_type)
                .cloned()
        }

        async fn get_all_nodes(&self) -> Vec<String> {
            self.nodes.iter().map(|node| node.name.clone()).collect()
        }

        async fn get_all_metadata(&self) -> Vec<NodeMetadata> {
            self.nodes.clone()
        }
    }

    #[tokio::test]
    async fn declarations_append_live_catalog_nodes_when_embedded_index_matches() {
        let provider = LiveOnlyProvider {
            nodes: vec![NodeMetadata {
                name: "custom_package_database_export".to_string(),
                friendly_name: "Package Database Export".to_string(),
                description: "Exports database rows through an installed package node.".to_string(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                category: Some("packages/database".to_string()),
                required_inputs: Vec::new(),
                companion_nodes: Vec::new(),
                capability_tags: Vec::new(),
            }],
        };

        let declarations = provider.get_declarations("database").await;

        assert!(declarations.contains("embedded .flow.d index"));
        assert!(declarations.contains("Additional live app catalog declarations"));
        assert!(declarations.contains("customPackageDatabaseExport"));
    }
}
