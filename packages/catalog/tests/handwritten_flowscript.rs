//! Applies every hand-written FlowScript fixture under `tests/ast/handwritten/` to an EMPTY board
//! against the real catalog.
//!
//! The committed `tests/ast/*.board` snapshots only ever check board -> text. This is the
//! text -> board direction, which is where the reconciler's resolution, typing and lowering bugs
//! live. Each fixture is checked three ways:
//!
//! 1. it parses;
//! 2. rendering is idempotent -- `render(parse(render(parse(src))))` equals `render(parse(src))`.
//!    The authored source itself is allowed to differ from canonical form, because a fixture may
//!    deliberately use positional arguments or another accepted-but-not-emitted spelling; that
//!    drift is reported as information, not failure;
//! 3. reconciling it onto an empty board yields commands and no diagnostics.
//!
//! A fixture named `*.wip.flow` is known-incomplete: it is reported but does not fail the run.
//! `FLOWSCRIPT_ONLY=<substring>` restricts the run to matching fixtures.
//! `FLOWSCRIPT_REPORT=1` prints the full report and does not fail, for triage runs.

use flow_like::flow::ast::{
    MetadataEnricher, RenderOptions, parse, reconcile_text_with_catalog_enriched, render,
};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{NodeMetadata, node_to_metadata};
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::flow::pin::PinType;
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Node types whose `on_update` derives pins from their own literal arguments. Mirrors
/// `ENRICH_ALLOWLIST` in `packages/core/src/flow/ast/apply.rs`: the product apply path enriches
/// through it, so a harness that skipped it would under-test every dynamic-pin node.
fn enrich_allowlist() -> Vec<&'static str> {
    let mut list = vec![
        "string_format",
        "string_render_template",
        "a2ui_push_csv_to_chart",
        "df_sql_query",
        "df_sql_query_cached",
        "df_execute_sql",
        "df_write_delta",
        "graph_sql_query",
        "control_switch",
        "struct_break",
        "struct_make_from_schema",
        "ml_apply_transform",
        "ml_predict",
    ];
    list.extend(ML_FIT_NODES);
    list
}

const ML_FIT_NODES: &[&str] = &[
    "fit_adaboost",
    "fit_dbscan",
    "fit_decision_tree",
    "fit_elastic_net",
    "fit_feature_scaler",
    "fit_gaussian_mixture",
    "fit_glm",
    "fit_kmeans",
    "fit_knn_classifier",
    "fit_knn_regressor",
    "fit_linear_regression",
    "fit_logistic_regression",
    "fit_multinomial_naive_bayes",
    "fit_naive_bayes",
    "fit_one_class_svm",
    "fit_pca",
    "fit_random_forest",
    "fit_svm_multi_class",
    "fit_svm_regression",
    "fit_tfidf_vectorizer",
    "fit_tsne",
];

/// `pin_name_matches` is crate-private, so this mirrors it: compare ignoring case and any
/// `_`/space separators, which makes `output_col` match `outputCol` and `Input Col`.
fn loose_pin_match(left: &str, right: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    norm(left) == norm(right)
}

/// Build the same enricher the product apply path builds: seed a scratch node with the call's
/// literal arguments, run its `on_update`, and read the pins back.
fn build_enricher(logic: &[Arc<dyn NodeLogic>]) -> MetadataEnricher {
    let allow = enrich_allowlist();
    let logic_by_type: HashMap<String, Arc<dyn NodeLogic>> = logic
        .iter()
        .map(|logic| (logic.get_node().name, logic.clone()))
        .filter(|(name, _)| allow.contains(&name.as_str()))
        .collect();
    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime for on_update"),
    );
    Box::new(
        move |meta: &NodeMetadata, args: &[(String, flow_like_types::Value)], board: &Board| {
            let logic = logic_by_type.get(&meta.name)?;
            let mut scratch = logic.get_node();
            let mut seeded = false;
            for (arg_name, value) in args {
                let pin_id = scratch
                    .pins
                    .iter()
                    .find(|(_, pin)| {
                        pin.pin_type == PinType::Input
                            && (loose_pin_match(&pin.name, arg_name)
                                || loose_pin_match(&pin.friendly_name, arg_name))
                    })
                    .map(|(id, _)| id.clone());
                if let Some(pin_id) = pin_id
                    && let Some(pin) = scratch.pins.get_mut(&pin_id)
                    && let Ok(bytes) = flow_like_types::json::to_vec(value)
                {
                    pin.default_value = Some(bytes);
                    seeded = true;
                }
            }
            if !seeded {
                return None;
            }
            runtime.block_on(logic.on_update(&mut scratch, board));
            Some(node_to_metadata(&scratch))
        },
    )
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/ast/handwritten")
}

fn catalog() -> (Vec<NodeMetadata>, MetadataEnricher) {
    let logic: Vec<Arc<dyn NodeLogic>> = CatalogBuilder::new().build();
    let nodes: Vec<Node> = logic.iter().map(|logic| logic.get_node()).collect();
    let metadata = nodes.iter().map(node_to_metadata).collect();
    (metadata, build_enricher(&logic))
}

fn collect_fixtures(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_fixtures(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("flow") {
            out.push(path);
        }
    }
}

#[derive(Default)]
struct Report {
    name: String,
    lines: usize,
    parse_error: Option<String>,
    reparse_error: Option<String>,
    not_idempotent: Option<String>,
    canonical_drift: Option<String>,
    diagnostics: Vec<String>,
    commands: usize,
}

impl Report {
    fn failed(&self) -> bool {
        self.parse_error.is_some()
            || self.reparse_error.is_some()
            || self.not_idempotent.is_some()
            || !self.diagnostics.is_empty()
            || self.commands == 0
    }
}

fn first_difference(left: &str, right: &str) -> String {
    let left: Vec<&str> = left.lines().collect();
    let right: Vec<&str> = right.lines().collect();
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or("<missing>");
        let b = right.get(index).copied().unwrap_or("<missing>");
        if a != b {
            return format!("line {}:\n    authored: {a}\n    rendered: {b}", index + 1);
        }
    }
    "no line differs (trailing whitespace?)".to_string()
}

fn check(path: &PathBuf, catalog: &[NodeMetadata], enricher: &MetadataEnricher) -> Report {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let source = std::fs::read_to_string(path).expect("read fixture");
    let mut report = Report {
        name,
        lines: source.lines().count(),
        ..Default::default()
    };

    let ast = match parse(&source) {
        Ok(ast) => ast,
        Err(error) => {
            report.parse_error = Some(format!("{error:?}"));
            return report;
        }
    };

    let rendered = render(&ast, &RenderOptions::default());
    if rendered != source {
        report.canonical_drift = Some(first_difference(&source, &rendered));
    }
    match parse(&rendered) {
        Ok(reparsed) => {
            let again = render(&reparsed, &RenderOptions::default());
            if again != rendered {
                report.not_idempotent = Some(first_difference(&rendered, &again));
            }
        }
        Err(error) => report.reparse_error = Some(format!("{error:?}")),
    }

    let board = Board::new_detached(Some("handwritten".into()), Path::default());
    let result = reconcile_text_with_catalog_enriched(&board, &source, catalog, enricher);
    report.diagnostics = result.diagnostics;
    report.commands = result.commands.len();
    report
}

#[test]
fn handwritten_fixtures_reconcile_cleanly() {
    let dir = fixture_dir();
    let mut fixtures: Vec<PathBuf> = Vec::new();
    collect_fixtures(&dir, &mut fixtures);
    fixtures.sort();

    if let Ok(filter) = std::env::var("FLOWSCRIPT_ONLY")
        && !filter.is_empty()
    {
        fixtures.retain(|path| path.to_string_lossy().contains(&filter));
    }

    assert!(
        !fixtures.is_empty(),
        "no .flow fixtures found under {dir:?} (FLOWSCRIPT_ONLY may not match)"
    );

    let (catalog, enricher) = catalog();
    let reports: Vec<Report> = fixtures
        .iter()
        .map(|path| check(path, &catalog, &enricher))
        .collect();

    let mut out = String::new();
    for report in &reports {
        let status = if report.failed() { "FAIL" } else { "ok  " };
        out.push_str(&format!(
            "\n{status} {} ({} lines, {} commands)\n",
            report.name, report.lines, report.commands
        ));
        if let Some(error) = &report.parse_error {
            out.push_str(&format!("     parse: {error}\n"));
        }
        if let Some(error) = &report.reparse_error {
            out.push_str(&format!("     RENDERED OUTPUT DOES NOT PARSE: {error}\n"));
        }
        if let Some(diff) = &report.not_idempotent {
            out.push_str(&format!("     render is not idempotent -> {diff}\n"));
        }
        if let Some(diff) = &report.canonical_drift {
            out.push_str(&format!(
                "     note: authored form is not canonical -> {diff}\n"
            ));
        }
        for diagnostic in &report.diagnostics {
            out.push_str(&format!("     diag: {diagnostic}\n"));
        }
        if report.parse_error.is_none() && report.commands == 0 {
            out.push_str("     produced no commands\n");
        }
    }
    println!("{out}");

    // `bug-*.flow` files are minimal repros of engine defects: they are EXPECTED to fail.
    // One that starts passing means the underlying bug was fixed and the repro should be
    // promoted into a real fixture, so that is reported too.
    let (repros, rest): (Vec<&Report>, Vec<&Report>) =
        reports.iter().partition(|r| r.name.starts_with("bug-"));
    // `*.wip.flow` is a fixture that is known not to apply yet. It stays in the corpus for the
    // constructs it documents, but it does not gate the suite.
    let (wip, fixtures): (Vec<&Report>, Vec<&Report>) = rest
        .into_iter()
        .partition(|r| r.name.ends_with(".wip.flow"));
    if !wip.is_empty() {
        println!(
            "\n{} work-in-progress fixture(s), not gating:\n  {}",
            wip.len(),
            wip.iter()
                .map(|r| format!("{} ({} diagnostics)", r.name, r.diagnostics.len()))
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
    let fixed: Vec<&&Report> = repros.iter().filter(|r| !r.failed()).collect();
    if !fixed.is_empty() {
        println!(
            "\n{} bug repro(s) now reconcile cleanly -- the defect looks fixed, promote them:\n  {}",
            fixed.len(),
            fixed
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
    println!(
        "\n{} fixtures, {} clean; {} bug repros, {} still reproducing",
        fixtures.len(),
        fixtures.iter().filter(|r| !r.failed()).count(),
        repros.len(),
        repros.iter().filter(|r| r.failed()).count()
    );

    let failed: Vec<&Report> = fixtures.iter().copied().filter(|r| r.failed()).collect();
    let triage = std::env::var("FLOWSCRIPT_REPORT").is_ok_and(|v| !v.is_empty());
    if triage {
        return;
    }
    assert!(
        failed.is_empty(),
        "{} of {} hand-written FlowScript fixtures failed:{out}",
        failed.len(),
        fixtures.len()
    );
}
