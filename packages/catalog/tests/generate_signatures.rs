//! Generates the FlowScript node-signature registry from the native catalog.
//!
//! The standalone `flow-like-ast` crate parses FlowScript without depending on the (heavy)
//! catalog. To recover pin names from positional args and validate calls, it needs every
//! node's signature. This test dumps `get_node()` metadata for the whole catalog into a
//! checked-in `signatures.json` next to the AST crate.
//!
//! Run `cargo test -p flow-like-catalog --test generate_signatures` to regenerate after
//! changing node pins; the JSON diff then surfaces exactly which signatures moved.

use std::{collections::BTreeMap, path::PathBuf};

use flow_like::flow::ast::{
    NodeNames, SignatureSet, declarations_by_category, declarations_by_package, node_names,
    node_to_signature, node_to_signature_in, schema_sidecar,
};
use flow_like_catalog::{CatalogBuilder, labeled_catalog};

/// Path to the checked-in registry consumed by `flow-like-ast`.
fn signatures_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ast/signatures.json")
}

/// Directory holding the generated per-category `.flow.d` declaration files.
fn declarations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ast/flow.d")
}

/// Write `content` only when it differs. The `.flow.d` files are `include_str!`-embedded by
/// `flow-like`, so an unchanged regeneration must not touch their mtimes and force a rebuild of
/// every downstream crate.
fn write_if_changed(path: &std::path::Path, content: &str) {
    if std::fs::read_to_string(path).is_ok_and(|current| current == content) {
        return;
    }
    std::fs::write(path, content).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
}

/// Effective FlowScript names of every catalog node, `node_type` → record, sorted.
///
/// This is the review artifact for `namespace::alias` naming (and the method-form receiver per
/// node). `lint_catalog::flowscript_names_snapshot_is_current` keeps it in sync with the catalog.
fn names_snapshot() -> BTreeMap<String, NodeNames> {
    CatalogBuilder::new()
        .build()
        .into_iter()
        .map(|logic| {
            let node = logic.get_node();
            (node.name.clone(), node_names(&node))
        })
        .collect()
}

#[test]
fn generate_signatures_json() {
    let signatures = CatalogBuilder::new()
        .build()
        .into_iter()
        .map(|logic| node_to_signature(&logic.get_node()))
        .collect::<Vec<_>>();

    let set = SignatureSet::new(signatures);
    assert!(
        !set.signatures.is_empty(),
        "catalog produced zero signatures"
    );

    let json = flow_like_types::json::to_string_pretty(&set).expect("serialize signature set");
    let path = signatures_path();
    write_if_changed(&path, &format!("{json}\n"));

    eprintln!(
        "wrote {} signatures to {}",
        set.signatures.len(),
        path.display()
    );

    // Emit human/agent-facing `.flow.d` declaration files, one per top-level category. These
    // give a coding agent a documented, browsable index of every callable node by domain.
    let dir = declarations_dir();
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("create {dir:?}: {e}"));

    let files = declarations_by_category(&set.signatures);
    let mut index = String::from(
        "// FlowScript node declarations (generated, do not edit).\n\
         // Each file documents one top-level catalog category.\n\n",
    );
    for file in &files {
        let out = dir.join(format!("{}.flow.d", file.stem));
        write_if_changed(&out, &file.content);
        index.push_str(&format!("// {} -> {}.flow.d\n", file.category, file.stem));
    }
    write_if_changed(&dir.join("index.flow.d"), &index);

    eprintln!(
        "wrote {} declaration files to {}",
        files.len(),
        dir.display()
    );

    // Emit the schema sidecar: per-node JSON Schema strings for struct-typed pins. Editor tooling
    // loads this to resolve node-call result structs deeply, without bloating the `.flow.d` text.
    let sidecar = schema_sidecar(&set.signatures);
    let sidecar_json =
        flow_like_types::json::to_string_pretty(&sidecar).expect("serialize schema sidecar");
    let sidecar_path = dir.join("node.flow.schemas.json");
    write_if_changed(&sidecar_path, &format!("{sidecar_json}\n"));

    eprintln!(
        "wrote schema sidecar for {} nodes to {}",
        sidecar.len(),
        sidecar_path.display()
    );

    let names = names_snapshot();
    let names_json =
        flow_like_types::json::to_string_pretty(&names).expect("serialize names snapshot");
    let names_path = dir.join("names.json");
    write_if_changed(&names_path, &format!("{names_json}\n"));

    eprintln!(
        "wrote FlowScript names for {} nodes to {}",
        names.len(),
        names_path.display()
    );

    // Emit per-package declaration files under `flow.d/packages/`. FlowPilot loads these so a
    // project is documented by the packages it actually uses (built-ins plus any third-party
    // packages injected into the catalog at runtime).
    let pkg_signatures = labeled_catalog()
        .into_iter()
        .map(|labeled| node_to_signature_in(&labeled.node.get_node(), labeled.package))
        .collect::<Vec<_>>();
    let pkg_set = SignatureSet::new(pkg_signatures);

    let pkg_dir = dir.join("packages");
    std::fs::create_dir_all(&pkg_dir).unwrap_or_else(|e| panic!("create {pkg_dir:?}: {e}"));

    let pkg_files = declarations_by_package(&pkg_set.signatures);
    let mut pkg_index = String::from(
        "// FlowScript node declarations (generated, do not edit).\n\
         // Each file documents one catalog package.\n\n",
    );
    for file in &pkg_files {
        let out = pkg_dir.join(format!("{}.flow.d", file.stem));
        write_if_changed(&out, &file.content);
        pkg_index.push_str(&format!(
            "// {} -> packages/{}.flow.d\n",
            file.category, file.stem
        ));
    }
    write_if_changed(&pkg_dir.join("index.flow.d"), &pkg_index);

    eprintln!(
        "wrote {} per-package declaration files to {}",
        pkg_files.len(),
        pkg_dir.display()
    );

    let pkg_sidecar = schema_sidecar(&pkg_set.signatures);
    let pkg_sidecar_json =
        flow_like_types::json::to_string_pretty(&pkg_sidecar).expect("serialize package sidecar");
    write_if_changed(
        &pkg_dir.join("node.flow.schemas.json"),
        &format!("{pkg_sidecar_json}\n"),
    );
}
