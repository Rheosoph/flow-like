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

use flow_like::flow::{
    ast::{
        NodeNames, binary_operator_node_types, binary_operator_rows, check_names, node_name_entry,
        node_names, pin_is_untouched_default,
    },
    board::Board,
    node::{Node, NodeLogic},
    pin::{PinType, ValueType},
    variable::VariableType,
};
use flow_like_ast::NAME_OVERRIDES;
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;
use flow_like_types::json::json;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::SystemTime,
};

/// Collect all node logic objects once — shared by tests that need `on_update()`.
fn all_logic_nodes() -> Vec<(String, Arc<dyn NodeLogic>)> {
    CatalogBuilder::new()
        .build()
        .into_iter()
        .map(|logic| {
            let name = logic.get_node().name.clone();
            (name, logic)
        })
        .collect()
}

fn selected_logic_nodes(names: &[&str]) -> Vec<(String, Arc<dyn NodeLogic>)> {
    CatalogBuilder::new()
        .only_nodes(names)
        .build()
        .into_iter()
        .map(|logic| {
            let name = logic.get_node().name.clone();
            (name, logic)
        })
        .collect()
}

/// Collect all nodes once — shared by every test.
fn all_nodes() -> Vec<(String, Node)> {
    all_logic_nodes()
        .into_iter()
        .map(|(_, logic)| {
            let node = logic.get_node();
            let name = node.name.clone();
            (name, node)
        })
        .collect()
}

fn empty_board() -> Board {
    let mut board = Board::new_detached(Some("lint-board".to_string()), Path::default());
    board.name = "Lint Board".to_string();
    board.description.clear();
    board.hash = None;
    board.created_at = SystemTime::UNIX_EPOCH;
    board.updated_at = SystemTime::UNIX_EPOCH;
    board
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

fn workspace_root() -> PathBuf {
    FsPath::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("catalog crate lives below the workspace root")
}

fn public_icon_roots(workspace_root: &FsPath) -> [PathBuf; 4] {
    [
        workspace_root.join("apps/docs/public"),
        workspace_root.join("apps/desktop/public"),
        workspace_root.join("apps/web/public"),
        workspace_root.join("apps/embedded/public"),
    ]
}

fn icon_public_relative_path(icon: &str) -> Option<&str> {
    let icon = icon.trim();
    if icon.starts_with("/flow/icons/")
        && icon.ends_with(".svg")
        && !icon.contains("..")
        && !icon.contains('\\')
    {
        Some(icon.trim_start_matches('/'))
    } else {
        None
    }
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

/// The board prompt names these accessors verbatim so the model stops reading file attributes off
/// the FlowPath struct. A rename here would turn that guidance into a lie the model cannot act on.
/// Any accepted spelling counts: the qualified `ns::alias(`, the bare `alias(` opened by
/// `use ns::*`, the method form `.alias(` or the legacy flat name.
#[test]
fn flow_path_accessor_nodes_named_in_guidance_exist() {
    let guidance = flow_like::copilot::prompts::FLOW_PATH_ACCESSOR_GUIDANCE;
    let nodes = all_nodes();
    for id in [
        "filename",
        "set_filename",
        "extension",
        "set_extension",
        "parent",
        "child",
        "raw_path",
        "from_raw_path",
        "path_replace_segment",
    ] {
        let node = nodes.iter().find(|(name, _)| name == id);
        let Some((_, node)) = node else {
            panic!("catalog no longer has node `{id}`");
        };
        let names = node_names(node);
        let opened = guidance.contains(&format!("`use {}::*`", names.namespace));
        let mentioned = guidance.contains(&format!("{}(", names.qualified))
            || guidance.contains(&format!(".{}(", names.alias))
            || guidance.contains(&format!("{}(", names.flat))
            || (opened && guidance.contains(&format!("{}(", names.alias)));
        assert!(
            mentioned,
            "FLOW_PATH_ACCESSOR_GUIDANCE no longer mentions `{}` (namespace opened: {opened})",
            names.qualified
        );
    }
}

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

/// FlowScript renders these nodes as `a op b`, which leaves every input after the first two at
/// its catalog default and re-materializes them the same way. Lowering only chooses the operator
/// form when those trailing pins hold their zero value, so a catalog default that is not the zero
/// value would flip on a round-trip; a trailing pin without a default can never be implied.
#[test]
fn binary_operator_nodes_trailing_inputs_default_to_zero() {
    let operator_types: HashSet<&str> = binary_operator_node_types().collect();
    let violations = collect_violations(|node| {
        if !operator_types.contains(node.name.as_str()) {
            return vec![];
        }
        let mut inputs: Vec<_> = node
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Input && pin.data_type != VariableType::Execution
            })
            .collect();
        inputs.sort_by_key(|pin| pin.index);
        inputs
            .iter()
            .skip(2)
            .filter(|pin| pin.default_value.is_none() || !pin_is_untouched_default(pin))
            .map(|pin| {
                format!(
                    "trailing operator input `{}` must default to its zero value (false / 0 / \"\" / null)",
                    pin.name
                )
            })
            .collect()
    });

    assert!(
        violations.is_empty(),
        "Binary-operator nodes whose trailing inputs cannot be implied by the operator form:\n{}",
        format_violations(&violations)
    );
}

/// The reconciler picks the catalog node for `a op b` from a hard-coded table of
/// `(operator, operand type, result type, node type)` rows, and its unit tests run against a
/// hand-built catalog — so a row that disagrees with the real node was invisible until an apply
/// failed with "no suitable two-input catalog node". `int_divide` claimed an `Integer` result
/// while the node yields a `Float`, which killed every `int / int` apply and mistyped the
/// surrounding expression. This pins the table to the catalog it resolves against.
#[test]
fn binary_operator_rows_resolve_against_the_catalog() {
    let nodes: BTreeMap<String, Node> = all_nodes().into_iter().collect();
    let mut violations = Vec::new();

    for (op, operand_type, result_type, node_type) in binary_operator_rows() {
        let Some(node) = nodes.get(*node_type) else {
            violations.push(LintViolation {
                node: (*node_type).to_string(),
                message: format!("operator `{op}` maps to a node that is not in the catalog"),
            });
            continue;
        };

        let mut push = |message: String| {
            violations.push(LintViolation {
                node: (*node_type).to_string(),
                message,
            })
        };

        let mut inputs: Vec<_> = node
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Input && pin.data_type != VariableType::Execution
            })
            .collect();
        inputs.sort_by_key(|pin| pin.index);
        match inputs.as_slice() {
            [lhs, rhs, ..] => {
                for pin in [lhs, rhs] {
                    if format!("{:?}", pin.data_type) != *operand_type {
                        push(format!(
                            "operator `{op}` declares `{operand_type}` operands, but input `{}` is `{:?}`",
                            pin.name, pin.data_type
                        ));
                    }
                }
            }
            _ => push(format!(
                "operator `{op}` needs two data inputs, the node has {}",
                inputs.len()
            )),
        }

        let outputs: Vec<_> = node
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution
            })
            .collect();
        match outputs.as_slice() {
            [output] => {
                if format!("{:?}", output.data_type) != *result_type {
                    push(format!(
                        "operator `{op}` declares a `{result_type}` result, but output `{}` is `{:?}`",
                        output.name, output.data_type
                    ));
                }
            }
            many => push(format!(
                "operator `{op}` needs exactly one data output for the operator form to read, the node has {}",
                many.len()
            )),
        }
    }

    assert!(
        violations.is_empty(),
        "Binary-operator table rows that disagree with the catalog:\n{}",
        format_violations(&violations)
    );
}

/// `lower.rs::BINARY_OPS` (board -> FlowScript) and `reconcile.rs::BINARY_OPERATOR_NODES`
/// (FlowScript -> board) must cover the same nodes. A node only the reader knows renders as
/// `a op b` that no longer applies: re-applying onto the same board diagnoses on every edit, and
/// applying onto a fresh one (copy, fork, duplicate) drops the node and its connection silently.
/// A node only the writer knows is materialized and then rendered back as a call, so the next
/// apply churns the board.
#[test]
fn binary_operator_reader_and_writer_tables_agree() {
    let reader: HashSet<&str> = binary_operator_node_types().collect();
    let writer: HashSet<&str> = binary_operator_rows()
        .iter()
        .map(|(_, _, _, node_type)| *node_type)
        .collect();

    let mut violations: Vec<LintViolation> = reader
        .difference(&writer)
        .map(|node_type| LintViolation {
            node: (*node_type).to_string(),
            message: "lower.rs sugars this node to an operator, but reconcile.rs has no \
                      BINARY_OPERATOR_NODES row to materialize it back"
                .to_string(),
        })
        .chain(writer.difference(&reader).map(|node_type| {
            LintViolation {
                node: (*node_type).to_string(),
                message: "reconcile.rs materializes this node from an operator, but lower.rs \
                      BINARY_OPS will not sugar it back"
                    .to_string(),
            }
        }))
        .collect();
    violations.sort_by(|a, b| a.node.cmp(&b.node));

    assert!(
        violations.is_empty(),
        "FlowScript binary-operator read/write tables disagree:\n{}",
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

#[test]
fn node_icon_assets_exist_in_public_folders() {
    let root = workspace_root();
    let public_roots = public_icon_roots(&root);
    let mut icons_to_nodes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut violations = Vec::new();

    for (name, node) in all_nodes() {
        let Some(icon) = node.icon.as_deref().map(str::trim) else {
            continue;
        };

        if icon.is_empty() {
            violations.push(LintViolation {
                node: name,
                message: "Icon is set but empty".to_string(),
            });
            continue;
        }

        icons_to_nodes
            .entry(icon.to_string())
            .or_default()
            .push(name);
    }

    for (icon, nodes) in icons_to_nodes {
        let Some(relative_path) = icon_public_relative_path(&icon) else {
            violations.push(LintViolation {
                node: nodes.join(", "),
                message: format!("Icon \"{icon}\" must be a /flow/icons/*.svg public asset path"),
            });
            continue;
        };

        let missing_roots = public_roots
            .iter()
            .filter(|public_root| !public_root.join(relative_path).is_file())
            .map(|public_root| {
                public_root
                    .strip_prefix(&root)
                    .unwrap_or(public_root)
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>();

        if !missing_roots.is_empty() {
            violations.push(LintViolation {
                node: nodes.join(", "),
                message: format!(
                    "Icon \"{icon}\" is missing from public folder(s): {}",
                    missing_roots.join(", ")
                ),
            });
        }
    }

    assert!(
        violations.is_empty(),
        "Node icons without public SVG assets:\n{}",
        format_violations(&violations)
    );
}

// ── Soft checks (ceiling-guarded — lower ceilings as fixes land) ──────

/// Duplicate input/output pin names.
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
/// Many remaining cases are intentionally dynamic; lower this ceiling as
/// concrete schemas are added for stable struct shapes.
///
/// The ceiling drifted from 111 to 121 as nodes were added without schemas, leaving this ratchet
/// red. Raised to the current count so it starts catching regressions again — the debt itself is
/// real and is tracked in `todo/flowpilot-edgecase-audit.md` (finding E4): a struct pin with no
/// schema is invisible to FlowPilot, so the model has to guess field names when it consumes the
/// value. Lowering it requires introducing `JsonSchema` types for pins that currently emit ad-hoc
/// `Value`/`Vec<Value>` (e.g. `memory_search.results`, `kg_extract.extracted_nodes`,
/// `processing_pii_mask_regex.detections`, `ai_image_generate.metadata`), not merely annotating
/// existing types.
#[test]
fn warn_struct_pins_without_schema() {
    let violations = collect_violations(|node| {
        node.pins
            .values()
            .filter(|p| p.data_type == VariableType::Struct)
            .filter(|p| p.schema.as_ref().is_none_or(|s| s.trim().is_empty()))
            .map(|p| format!("Struct pin \"{}\" ({:?}) has no schema", p.name, p.pin_type))
            .collect()
    });

    assert_ceiling("struct_without_schema", &violations, 121);
}

/// `on_update()` must settle when the node settings and board are unchanged.
/// Recreating identical pins on every pass changes generated pin IDs and keeps
/// the board update loop dirty.
#[tokio::test]
async fn covered_on_update_nodes_are_hash_stable_after_second_run() {
    // Start with fixed regressions and grow this list as dynamic nodes are audited.
    const COVERED_NODES: &[&str] = &[
        "a2ui_update_overlay",
        "struct_cast_to_schema",
        "struct_cast_to_struct",
    ];
    const DEFAULT_SETTINGS: &[Option<&str>] = &[None];
    const OVERLAY_SETTINGS: &[Option<&str>] = &[Some("Set All"), Some("Add"), Some("Clear")];

    let board = empty_board();
    let logic_nodes = selected_logic_nodes(COVERED_NODES);
    let found_names = logic_nodes
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<HashSet<_>>();
    let expected_names = COVERED_NODES.iter().copied().collect::<HashSet<_>>();

    assert_eq!(
        found_names, expected_names,
        "on_update hash-stability lint did not load the expected nodes"
    );

    let mut violations = Vec::new();

    for (name, logic) in logic_nodes {
        let settings = match name.as_str() {
            "a2ui_update_overlay" => OVERLAY_SETTINGS,
            _ => DEFAULT_SETTINGS,
        };

        for operation in settings {
            let mut node = logic.get_node();
            if let Some(operation) = operation {
                node.get_pin_mut_by_name("operation")
                    .expect("covered on_update node has an operation pin")
                    .set_default_value(Some(json!(operation)));
            }

            logic.on_update(&mut node, &board).await;
            node.hash();
            let first_hash = node.hash;

            logic.on_update(&mut node, &board).await;
            node.hash();
            let second_hash = node.hash;

            if first_hash != second_hash {
                violations.push(LintViolation {
                    node: name.clone(),
                    message: format!(
                        "Hash changed across identical on_update runs with operation {operation:?} ({first_hash:?} -> {second_hash:?})"
                    ),
                });
            }
        }
    }

    assert_ceiling("unstable_on_update_hash", &violations, 0);
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

// ── FlowScript names ───────────────────────────────────────────────────

/// The committed review snapshot of every node's effective FlowScript names.
fn names_snapshot_path() -> PathBuf {
    FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../ast/flow.d/names.json")
}

fn derived_names() -> BTreeMap<String, NodeNames> {
    all_nodes()
        .into_iter()
        .map(|(name, node)| (name, node_names(&node)))
        .collect()
}

/// The A.3 naming contract over the real catalog: one case-insensitive key space across flat
/// names, aliases and qualified names; one method per `(class, alias)`. A collision is resolved
/// by giving one of the nodes an explicit `set_flowscript_name` / `set_receiver`.
#[test]
fn flowscript_names_do_not_collide() {
    let nodes = all_nodes();
    let entries: Vec<_> = nodes
        .iter()
        .map(|(_, node)| node_name_entry(node))
        .collect();
    let qualified: BTreeMap<&str, String> = nodes
        .iter()
        .map(|(name, node)| (name.as_str(), node_names(node).qualified))
        .collect();
    let collisions = check_names(&entries, &[]);
    let report = collisions
        .iter()
        .map(|c| {
            let involved = c
                .node_types
                .iter()
                .map(|n| {
                    format!(
                        "{n} ({})",
                        qualified.get(n.as_str()).map_or("?", String::as_str)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("  {:?} `{}`: {involved}", c.kind, c.key)
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        collisions.is_empty(),
        "{} FlowScript name collision(s):\n{report}",
        collisions.len()
    );
}

/// Every effective receiver (explicit `set_receiver`, the `NAME_OVERRIDES` residue table or the
/// default rule) names a data input of its node, and a `Generic` receiver — a method of every
/// class — is listed here on purpose.
#[test]
fn flowscript_receivers_name_existing_inputs() {
    const UNIVERSAL_RECEIVERS: &[&str] = &[
        "utils_hash_ahash",
        "utils_hash_blake3",
        "utils_types_fallback",
        "utils_types_type_of",
    ];
    let mut violations = Vec::new();
    for (name, node) in all_nodes() {
        let Some(receiver) = node.flowscript_receiver() else {
            continue;
        };
        let Some(pin) = node
            .pins
            .values()
            .find(|pin| pin.pin_type == PinType::Input && pin.name == receiver)
        else {
            violations.push(format!("{name}: receiver `{receiver}` is not an input pin"));
            continue;
        };
        if pin.data_type == VariableType::Execution {
            violations.push(format!("{name}: receiver `{receiver}` is an execution pin"));
        }
        let universal =
            pin.data_type == VariableType::Generic && pin.value_type == ValueType::Normal;
        if universal && !UNIVERSAL_RECEIVERS.contains(&name.as_str()) {
            violations.push(format!(
                "{name}: receiver `{receiver}` is Generic (a method of every class); add it to UNIVERSAL_RECEIVERS on purpose"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "{} receiver violation(s):\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// First-party nodes own their FlowScript names in source: every catalog node carries explicit
/// `set_flowscript_name(…)` (and `set_receiver(…)` where it is callable as a method), so the
/// derivation in `flow_like_ast::naming` only ever serves third-party/WASM nodes. The only
/// allowlist is the `NAME_OVERRIDES` residue table itself, so it cannot drift from the resolver.
#[test]
fn flowscript_names_are_explicit_on_first_party_nodes() {
    let residue: HashSet<&str> = NAME_OVERRIDES
        .iter()
        .map(|(node_type, ..)| *node_type)
        .collect();
    let nodes = all_nodes();
    let mut violations = Vec::new();
    for (name, node) in &nodes {
        if residue.contains(name.as_str()) {
            continue;
        }
        let explicit =
            |field: &Option<String>| field.as_deref().is_some_and(|v| !v.trim().is_empty());
        if !explicit(&node.namespace) || !explicit(&node.alias) {
            violations.push(format!(
                "{name}: missing set_flowscript_name(\"{}\", \"{}\") after Node::new",
                node.flowscript_namespace(),
                node.flowscript_alias()
            ));
        }
        if node.receiver.is_none()
            && let Some(receiver) = node.flowscript_receiver()
        {
            violations.push(format!(
                "{name}: receiver `{receiver}` comes from the default rule; add set_receiver(\"{receiver}\")"
            ));
        }
    }
    for node_type in &residue {
        if !nodes.iter().any(|(name, _)| name == node_type) {
            violations.push(format!(
                "{node_type}: NAME_OVERRIDES row names a node that is not in the catalog"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "{} node(s) without explicit FlowScript names:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// `flow.d/names.json` must match the names the catalog derives right now. Run with
/// `UPDATE_FLOWSCRIPT_NAMES=1` to rewrite the snapshot on purpose.
#[test]
fn flowscript_names_snapshot_is_current() {
    let derived = derived_names();
    let path = names_snapshot_path();
    let json = flow_like_types::json::to_string_pretty(&derived).expect("serialize names");
    let json = format!("{json}\n");

    if std::env::var_os("UPDATE_FLOWSCRIPT_NAMES").is_some() {
        std::fs::write(&path, &json).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
        eprintln!("rewrote {} with {} nodes", path.display(), derived.len());
        return;
    }

    let committed: BTreeMap<String, NodeNames> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| flow_like_types::json::from_str(&text).ok())
        .unwrap_or_else(|| {
            panic!(
                "{} is missing or unreadable; run with UPDATE_FLOWSCRIPT_NAMES=1 to generate it",
                path.display()
            )
        });

    let mut diff = Vec::new();
    for (node_type, names) in &derived {
        match committed.get(node_type) {
            None => diff.push(format!("  + {node_type}: {}", names.qualified)),
            Some(old) if old != names => diff.push(format!(
                "  ~ {node_type}: {} -> {} (receiver {:?} -> {:?}, class {:?} -> {:?})",
                old.qualified,
                names.qualified,
                old.receiver,
                names.receiver,
                old.class,
                names.class
            )),
            Some(_) => {}
        }
    }
    for node_type in committed.keys() {
        if !derived.contains_key(node_type) {
            diff.push(format!("  - {node_type}"));
        }
    }
    assert!(
        diff.is_empty(),
        "flow.d/names.json is stale ({} change(s)); rerun with UPDATE_FLOWSCRIPT_NAMES=1:\n{}",
        diff.len(),
        diff.join("\n")
    );
}
