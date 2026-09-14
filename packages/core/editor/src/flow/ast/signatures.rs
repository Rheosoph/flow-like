//! Node → signature stub (`flow_like_ast::Signature`) from catalog node metadata.
//!
//! Core builds `Signature` values; the AST crate owns their shape and TS-flavoured rendering
//! plus the serializable [`SignatureSet`](flow_like_ast::SignatureSet) registry. A generator
//! in `flow-like-catalog` dumps every `get_node()` into a `SignatureSet`, which the parser
//! later consumes to recover pin names from positional args without depending on the catalog.

use flow_like_ast::{NameEntry, NodeNames, SigParam, Signature, to_camel_case};

use crate::flow::node::Node;
use crate::flow::pin::{Pin, PinType};
use crate::flow::variable::VariableType;

fn is_exec(pin: &Pin) -> bool {
    pin.data_type == VariableType::Execution
}

fn sig_param(pin: &Pin) -> SigParam {
    let doc = {
        let d = pin.description.trim();
        if d.is_empty() {
            None
        } else {
            Some(d.to_string())
        }
    };
    SigParam {
        name: pin.name.clone(),
        ty: super::types::type_ref_with_schema(
            &pin.data_type,
            &pin.value_type,
            pin.schema.as_deref(),
        ),
        optional: pin.is_optional()
            || pin
                .default_value
                .as_ref()
                .map(|b| !b.is_empty())
                .unwrap_or(false),
        doc,
        schema: pin.schema.clone(),
    }
}

/// Build the FlowScript signature for a catalog node's static metadata.
///
/// Only statically-declared, non-execution pins become signature params; exec pins carry
/// control flow (block structure), not data. Pins are ordered by `index` for stable output.
pub fn node_to_signature(node: &Node) -> Signature {
    let mut inputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Input && !is_exec(p))
        .collect();
    inputs.sort_by_key(|p| p.index);

    let mut outputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output && !is_exec(p))
        .collect();
    outputs.sort_by_key(|p| p.index);

    let impure = node.pins.values().any(is_exec);

    let doc = {
        let doc = node.description.trim();
        if doc.is_empty() {
            None
        } else {
            Some(doc.to_string())
        }
    };

    let friendly = {
        let f = node.friendly_name.trim();
        if f.is_empty() {
            None
        } else {
            Some(f.to_string())
        }
    };

    let category = {
        let c = node.category.trim();
        if c.is_empty() {
            None
        } else {
            Some(c.to_string())
        }
    };

    Signature {
        node_type: node.name.clone(),
        display: to_camel_case(&node.name),
        friendly,
        category,
        package: None,
        inputs: inputs.into_iter().map(sig_param).collect(),
        outputs: outputs.into_iter().map(sig_param).collect(),
        impure,
        doc,
        namespace: Some(node.flowscript_namespace()),
        alias: Some(node.flowscript_alias()),
        receiver: node.flowscript_receiver(),
    }
}

/// Like [`node_to_signature`], but tags the signature with the catalog package it ships in.
///
/// Used by the per-package `.flow.d` generator so a project (including any third-party packages
/// injected into its catalog at runtime) can be documented one package at a time.
pub fn node_to_signature_in(node: &Node, package: impl Into<String>) -> Signature {
    let mut sig = node_to_signature(node);
    let package = package.into();
    sig.package = if package.trim().is_empty() {
        None
    } else {
        Some(package)
    };
    sig
}

/// The effective FlowScript names of a catalog node (explicit fields or derived defaults).
pub fn node_names(node: &Node) -> NodeNames {
    let namespace = node.flowscript_namespace();
    let alias = node.flowscript_alias();
    NodeNames {
        qualified: flow_like_ast::qualified_name(&namespace, &alias),
        namespace,
        alias,
        flat: flow_like_ast::legacy_display(&node.name),
        receiver: node.flowscript_receiver(),
        class: node.flowscript_receiver_class(),
        category: node.category.clone(),
    }
}

/// The collision-checker view of a catalog node's effective names.
pub fn node_name_entry(node: &Node) -> NameEntry {
    node_names(node).entry(&node.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::pin::PinOptions;

    #[test]
    fn signatures_honor_optional_geometry_and_legacy_pins_without_defaults() {
        let mut node = Node::new("geo_test", "Geometry", "", "Web/Geo");
        for (name, data_type) in [
            ("geometry", VariableType::Geometry),
            ("coordinate", VariableType::Struct),
        ] {
            node.add_input_pin(name, name, "", data_type)
                .set_options(PinOptions::new().set_optional(true).build());
        }
        node.add_input_pin("required", "Required", "", VariableType::Geometry);

        let signature = node_to_signature(&node);
        assert_eq!(
            signature
                .inputs
                .iter()
                .map(|pin| (pin.name.as_str(), pin.optional))
                .collect::<Vec<_>>(),
            vec![
                ("geometry", true),
                ("coordinate", true),
                ("required", false)
            ],
        );
    }

    #[test]
    fn signatures_preserve_default_based_optionality() {
        let mut node = Node::new("defaults_test", "Defaults", "", "Test");
        node.add_input_pin("default", "Default", "", VariableType::String)
            .set_default_value(Some(flow_like_types::json::json!("")));
        node.add_input_pin("empty_bytes", "Empty bytes", "", VariableType::String)
            .default_value = Some(Vec::new());

        let signature = node_to_signature(&node);
        assert!(signature.inputs[0].optional);
        assert!(!signature.inputs[1].optional);
    }
}
