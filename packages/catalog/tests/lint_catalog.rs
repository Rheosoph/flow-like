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
    board::{Board, ExecutionMode, ExecutionStage},
    execution::LogLevel,
    node::{Node, NodeLogic},
    pin::PinType,
    variable::VariableType,
};
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;
use flow_like_types::json::json;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
    Board {
        id: "lint-board".to_string(),
        name: "Lint Board".to_string(),
        description: String::new(),
        nodes: HashMap::new(),
        variables: HashMap::new(),
        comments: HashMap::new(),
        viewport: (0.0, 0.0, 0.0),
        version: (0, 0, 1),
        stage: ExecutionStage::Dev,
        log_level: LogLevel::Info,
        execution_mode: ExecutionMode::Hybrid,
        refs: HashMap::new(),
        layers: HashMap::new(),
        page_ids: Vec::new(),
        hash: None,
        created_at: SystemTime::UNIX_EPOCH,
        updated_at: SystemTime::UNIX_EPOCH,
        parent: None,
        board_dir: Path::default(),
        logic_nodes: HashMap::new(),
        app_state: None,
    }
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
#[test]
fn warn_struct_pins_without_schema() {
    let violations = collect_violations(|node| {
        node.pins
            .values()
            .filter(|p| p.data_type == VariableType::Struct)
            .filter(|p| p.schema.as_ref().map_or(true, |s| s.trim().is_empty()))
            .map(|p| format!("Struct pin \"{}\" ({:?}) has no schema", p.name, p.pin_type))
            .collect()
    });

    assert_ceiling("struct_without_schema", &violations, 111);
}

/// `on_update()` must settle when the node settings and board are unchanged.
/// Recreating identical pins on every pass changes generated pin IDs and keeps
/// the board update loop dirty.
#[tokio::test]
async fn covered_on_update_nodes_are_hash_stable_after_second_run() {
    // Start with fixed regressions and grow this list as dynamic nodes are audited.
    const COVERED_NODES: &[&str] = &["a2ui_update_overlay"];
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
