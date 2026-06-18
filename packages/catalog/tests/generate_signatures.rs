//! Generates the FlowScript node-signature registry from the native catalog.
//!
//! The standalone `flow-like-ast` crate parses FlowScript without depending on the (heavy)
//! catalog. To recover pin names from positional args and validate calls, it needs every
//! node's signature. This test dumps `get_node()` metadata for the whole catalog into a
//! checked-in `signatures.json` next to the AST crate.
//!
//! Run `cargo test -p flow-like-catalog --test generate_signatures` to regenerate after
//! changing node pins; the JSON diff then surfaces exactly which signatures moved.

use std::path::PathBuf;

use flow_like::flow::ast::{
    SignatureSet, declarations_by_category, declarations_by_package, node_to_signature,
    node_to_signature_in, schema_sidecar,
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
    std::fs::write(&path, format!("{json}\n")).unwrap_or_else(|e| panic!("write {path:?}: {e}"));

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
        std::fs::write(&out, &file.content).unwrap_or_else(|e| panic!("write {out:?}: {e}"));
        index.push_str(&format!("// {} -> {}.flow.d\n", file.category, file.stem));
    }
    let index_path = dir.join("index.flow.d");
    std::fs::write(&index_path, index).unwrap_or_else(|e| panic!("write {index_path:?}: {e}"));

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
    std::fs::write(&sidecar_path, format!("{sidecar_json}\n"))
        .unwrap_or_else(|e| panic!("write {sidecar_path:?}: {e}"));

    eprintln!(
        "wrote schema sidecar for {} nodes to {}",
        sidecar.len(),
        sidecar_path.display()
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
        std::fs::write(&out, &file.content).unwrap_or_else(|e| panic!("write {out:?}: {e}"));
        pkg_index.push_str(&format!(
            "// {} -> packages/{}.flow.d\n",
            file.category, file.stem
        ));
    }
    let pkg_index_path = pkg_dir.join("index.flow.d");
    std::fs::write(&pkg_index_path, pkg_index)
        .unwrap_or_else(|e| panic!("write {pkg_index_path:?}: {e}"));

    eprintln!(
        "wrote {} per-package declaration files to {}",
        pkg_files.len(),
        pkg_dir.display()
    );

    let pkg_sidecar = schema_sidecar(&pkg_set.signatures);
    let pkg_sidecar_json =
        flow_like_types::json::to_string_pretty(&pkg_sidecar).expect("serialize package sidecar");
    let pkg_sidecar_path = pkg_dir.join("node.flow.schemas.json");
    std::fs::write(&pkg_sidecar_path, format!("{pkg_sidecar_json}\n"))
        .unwrap_or_else(|e| panic!("write {pkg_sidecar_path:?}: {e}"));
}
