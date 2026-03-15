//! Lint tests for the entire native catalog.
//!
//! These tests load every node via `CatalogBuilder` and verify that node
//! definitions follow the project's quality rules — no execution feature
//! needed since we only inspect metadata from `get_node()`.
//!
//! Tests are split into two tiers:
//! - **Hard checks** — must pass, zero violations allowed.
//! - **Soft checks** — report violations as warnings and enforce a ceiling
//!   so the count can only go *down* over time.

use flow_like::flow::{pin::PinType, variable::VariableType};
use flow_like_catalog::CatalogBuilder;
use std::collections::HashSet;

/// Collect all nodes once — shared by every test.
fn all_nodes() -> Vec<(String, flow_like::flow::node::Node)> {
    CatalogBuilder::new()
        .build()
        .into_iter()
        .map(|logic| {
            let node = logic.get_node();
            let name = node.name.clone();
            (name, node)
        })
        .collect()
}

// ── Helpers ────────────────────────────────────────────────────────────

#[derive(Debug)]
struct LintViolation {
    node: String,
    message: String,
}

fn collect_violations(
    check: impl Fn(&flow_like::flow::node::Node) -> Vec<String>,
) -> Vec<LintViolation> {
    all_nodes()
        .iter()
        .flat_map(|(name, node)| {
            check(node)
                .into_iter()
                .map(|msg| LintViolation {
                    node: name.clone(),
                    message: msg,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn format_violations(violations: &[LintViolation]) -> String {
    violations
        .iter()
        .map(|v| format!("  [{}] {}", v.node, v.message))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Print violations as warnings and assert the count hasn't *increased*
/// beyond `ceiling`. As nodes are fixed, lower the ceiling.
fn assert_ceiling(label: &str, violations: &[LintViolation], ceiling: usize) {
    if !violations.is_empty() {
        eprintln!(
            "\n⚠ {label}: {} violation(s) (ceiling {ceiling}):\n{}",
            violations.len(),
            format_violations(violations)
        );
    }
    assert!(
        violations.len() <= ceiling,
        "{label}: violation count {} exceeds ceiling {ceiling} — \
         new code introduced violations:\n{}",
        violations.len(),
        format_violations(violations)
    );
}

// ── Hard checks (zero violations) ─────────────────────────────────────

#[test]
fn every_node_has_a_description() {
    let violations = collect_violations(|node| {
        if node.description.trim().is_empty() {
            vec!["Missing description".to_string()]
        } else {
            vec![]
        }
    });

    assert!(
        violations.is_empty(),
        "Nodes without descriptions:\n{}",
        format_violations(&violations)
    );
}

#[test]
fn every_node_has_a_category() {
    let violations = collect_violations(|node| {
        if node.category.trim().is_empty() {
            vec!["Missing category".to_string()]
        } else {
            vec![]
        }
    });

    assert!(
        violations.is_empty(),
        "Nodes without categories:\n{}",
        format_violations(&violations)
    );
}

#[test]
fn catalog_is_not_empty() {
    let nodes = all_nodes();
    assert!(
        !nodes.is_empty(),
        "Catalog returned zero nodes — something is wrong with the build"
    );
    eprintln!("\nCatalog contains {} node(s)", nodes.len());
}

#[test]
fn no_duplicate_node_names() {
    let nodes = all_nodes();
    let mut seen = HashSet::new();
    let mut dupes = Vec::new();
    for (name, _) in &nodes {
        if !seen.insert(name.as_str()) {
            dupes.push(name.clone());
        }
    }
    assert!(
        dupes.is_empty(),
        "Duplicate node names in catalog: {:?}",
        dupes
    );
}

// ── Soft checks (ceiling-guarded — lower ceilings as fixes land) ──────

/// Duplicate input/output pin names — currently 4 nodes affected.
/// Lower this ceiling as you fix the offending nodes.
#[test]
fn no_duplicate_input_output_pin_names() {
    let violations = collect_violations(|node| {
        let input_names: HashSet<&str> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .map(|p| p.name.as_str())
            .collect();

        node.pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .filter(|p| input_names.contains(p.name.as_str()))
            .map(|p| {
                format!(
                    "Input and output pin share the name \"{}\" — get/set will collide",
                    p.name
                )
            })
            .collect()
    });

    assert_ceiling("duplicate_pin_names", &violations, 0);
}

/// Impure nodes with exec pins on only one side.
/// Event/callback nodes (entry points) legitimately have only output exec pins,
/// so we skip nodes with `event_callback == Some(true)` or `start == Some(true)`.
#[test]
fn impure_nodes_have_both_exec_sides() {
    let violations = collect_violations(|node| {
        // Event / start nodes are entry points — output-only exec is expected
        if node.event_callback == Some(true) || node.start == Some(true) {
            return vec![];
        }

        let input_exec = node
            .pins
            .values()
            .any(|p| p.pin_type == PinType::Input && p.data_type == VariableType::Execution);
        let output_exec = node
            .pins
            .values()
            .any(|p| p.pin_type == PinType::Output && p.data_type == VariableType::Execution);

        let mut msgs = Vec::new();
        if input_exec && !output_exec {
            msgs.push("Has input exec pin(s) but no output exec pin".to_string());
        }
        if output_exec && !input_exec {
            msgs.push("Has output exec pin(s) but no input exec pin".to_string());
        }
        msgs
    });

    assert_ceiling("impure_exec_pins", &violations, 5);
}

/// Root-level array schemas — pin schemas should describe a single element.
/// Currently 19 pins affected. Lower as fixes land.
#[test]
fn no_root_array_schemas() {
    let violations = collect_violations(|node| {
        node.pins
            .values()
            .filter_map(|pin| {
                let schema_str = pin.schema.as_deref()?;
                let schema: serde_json::Value = serde_json::from_str(schema_str).ok()?;
                if schema.get("type").and_then(|t| t.as_str()) == Some("array") {
                    Some(format!(
                        "Pin \"{}\" has a root-level array schema — use ValueType::Array instead",
                        pin.name
                    ))
                } else {
                    None
                }
            })
            .collect()
    });

    assert_ceiling("root_array_schemas", &violations, 0);
}

/// Struct pins without a JSON schema.
#[test]
fn warn_struct_pins_without_schema() {
    let violations = collect_violations(|node| {
        node.pins
            .values()
            .filter(|p| p.data_type == VariableType::Struct)
            .filter(|p| p.schema.as_ref().map_or(true, |s| s.trim().is_empty()))
            .map(|p| {
                format!(
                    "Struct pin \"{}\" ({:?}) has no schema",
                    p.name, p.pin_type
                )
            })
            .collect()
    });

    assert_ceiling("struct_without_schema", &violations, 61);
}

// ── Info-level checks (report only, no ceiling) ───────────────────────

#[test]
fn warn_generic_pins() {
    let violations = collect_violations(|node| {
        node.pins
            .values()
            .filter(|p| p.data_type == VariableType::Generic)
            .map(|p| format!("Pin \"{}\" ({:?}) uses Generic type", p.name, p.pin_type))
            .collect()
    });

    if !violations.is_empty() {
        eprintln!(
            "\n⚠ {} Generic-typed pin(s) found (consider using specific types):\n{}",
            violations.len(),
            format_violations(&violations)
        );
    }
}

#[test]
fn warn_missing_scores() {
    let violations = collect_violations(|node| {
        if node.scores.is_none() {
            vec!["No scores defined".to_string()]
        } else {
            vec![]
        }
    });

    if !violations.is_empty() {
        eprintln!(
            "\n⚠ {} node(s) without scores:\n{}",
            violations.len(),
            format_violations(&violations)
        );
    }
}

#[test]
fn warn_pathbuf_pins() {
    let violations = collect_violations(|node| {
        node.pins
            .values()
            .filter(|p| p.data_type == VariableType::PathBuf)
            .map(|p| {
                format!(
                    "Pin \"{}\" ({:?}) uses PathBuf type — use FlowPath (Struct) instead",
                    p.name, p.pin_type
                )
            })
            .collect()
    });

    assert_ceiling("pathbuf_pins", &violations, 1);
}
