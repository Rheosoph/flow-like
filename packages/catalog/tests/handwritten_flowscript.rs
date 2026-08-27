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
//!
//! The other direction — board -> text -> board, i.e. that the renderer's own output feeds back
//! through parse and reconcile without changing the board — lives in `render_contract_catalog.rs`.
//! The two share `flowscript_support`, so both run against the same catalog and enricher.

mod flowscript_support;

use flow_like::flow::ast::{
    MetadataEnricher, RenderOptions, apply_flowscript_to_board, board_to_flowscript, parse,
    reconcile_text_with_catalog_enriched, render,
};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{BoardCommand, NodeMetadata};
use flow_like_storage::object_store::path::Path;
use flowscript_support::{
    CATALOG, board_node_and_layer_ids, catalog, catalog_state, collect_files,
    handwritten_fixture_dir as fixture_dir,
};
use std::collections::HashSet;
use std::path::PathBuf;

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
    collect_files(&dir, "flow", &mut fixtures);
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

/// Exercise the product Apply boundary for representative gating handwritten fixtures, then copy
/// the anchored result to a second Board. The second Apply must recreate node and layer identities,
/// and both Boards must reconcile their own rendered source as a no-op.
#[tokio::test]
async fn clean_handwritten_fixtures_apply_and_copy_roundtrip() {
    let dir = fixture_dir();
    // These fixtures span positional syntax, dates, byte arrays, Function calls, multiple Events,
    // globals, caches, DataFrames, and geospatial structures. The wider corpus remains covered by
    // `handwritten_fixtures_reconcile_cleanly`; several larger fixtures document independent
    // lower/apply gaps and cannot yet satisfy this stronger lifecycle invariant.
    let fixtures = [
        "t0-positional-args.flow",
        "t1-datetime-window.flow",
        "t1-faker-seed.flow",
        "t1-hash-encode.flow",
        "t1-string-hygiene.flow",
        "t2-cron-cache.flow",
        "t2-csv-dataframe.flow",
        "t2-geo-fencing.flow",
    ]
    .map(|name| dir.join(name));

    let state = catalog_state().await;
    let render_options = RenderOptions {
        anchors: true,
        ..RenderOptions::default()
    };
    let mut failures = Vec::new();

    for path in fixtures {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        let mut source_board =
            Board::new_detached(Some(format!("handwritten-source-{name}")), Path::default());
        let source_apply = match apply_flowscript_to_board(
            &mut source_board,
            &source,
            &CATALOG.nodes,
            state.clone(),
            None,
            false,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                failures.push(format!("{name}: source Apply failed: {error:#}"));
                continue;
            }
        };
        if !source_apply.diagnostics.is_empty() {
            failures.push(format!(
                "{name}: source Apply diagnostics: {:?}",
                source_apply.diagnostics
            ));
            continue;
        }

        let anchored_source = board_to_flowscript(&source_board, &render_options);
        let source_ids = board_node_and_layer_ids(&source_board);
        let source_noop = match apply_flowscript_to_board(
            &mut source_board,
            &anchored_source,
            &CATALOG.nodes,
            state.clone(),
            None,
            false,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                failures.push(format!("{name}: source round-trip Apply failed: {error:#}"));
                continue;
            }
        };
        if !source_noop.diagnostics.is_empty() || !source_noop.board_commands.is_empty() {
            failures.push(format!(
                "{name}: source round-trip was not a no-op: diagnostics={:?}, commands={:?}",
                source_noop.diagnostics, source_noop.board_commands
            ));
            continue;
        }

        let mut copied_board =
            Board::new_detached(Some(format!("handwritten-copy-{name}")), Path::default());
        let copied_apply = match apply_flowscript_to_board(
            &mut copied_board,
            &anchored_source,
            &CATALOG.nodes,
            state.clone(),
            None,
            false,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                failures.push(format!("{name}: copied-anchor Apply failed: {error:#}"));
                continue;
            }
        };
        if !copied_apply.diagnostics.is_empty() {
            failures.push(format!(
                "{name}: copied-anchor Apply diagnostics: {:?}",
                copied_apply.diagnostics
            ));
            continue;
        }

        let copied_ids = board_node_and_layer_ids(&copied_board);
        let reused_ids = source_ids
            .intersection(&copied_ids)
            .cloned()
            .collect::<Vec<_>>();
        if !reused_ids.is_empty() {
            failures.push(format!(
                "{name}: copied Board reused source entity IDs: {reused_ids:?}"
            ));
            continue;
        }

        let anchored_copy = board_to_flowscript(&copied_board, &render_options);
        let copied_noop = match apply_flowscript_to_board(
            &mut copied_board,
            &anchored_copy,
            &CATALOG.nodes,
            state.clone(),
            None,
            false,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                failures.push(format!(
                    "{name}: copied Board round-trip Apply failed: {error:#}"
                ));
                continue;
            }
        };
        if !copied_noop.diagnostics.is_empty() || !copied_noop.board_commands.is_empty() {
            failures.push(format!(
                "{name}: copied Board round-trip was not a no-op: diagnostics={:?}, commands={:?}",
                copied_noop.diagnostics, copied_noop.board_commands
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "handwritten Apply/copy lifecycle failures:\n  {}",
        failures.join("\n  ")
    );
}

/// Rename the Function in a real handwritten program through the complete Apply path. The layer,
/// body, boundary pins, and runtime callers must retain their IDs, and the rendered result must be
/// stable on the next Apply.
#[tokio::test]
async fn handwritten_function_rename_preserves_runtime_identity() {
    let source = std::fs::read_to_string(fixture_dir().join("t0-positional-args.flow"))
        .expect("read positional-args fixture");
    let state = catalog_state().await;
    let mut board = Board::new_detached(
        Some("handwritten-function-rename".to_string()),
        Path::default(),
    );
    let initial = apply_flowscript_to_board(
        &mut board,
        &source,
        &CATALOG.nodes,
        state.clone(),
        None,
        false,
    )
    .await
    .expect("initial handwritten program applies");
    assert!(initial.diagnostics.is_empty(), "{:?}", initial.diagnostics);

    let before = board
        .layers
        .values()
        .find(|layer| layer.name == "tag")
        .expect("tag Function layer")
        .clone();
    let before_node_ids = board_node_and_layer_ids(&board);
    let before_call_ids = board
        .nodes
        .values()
        .filter(|node| node.name == "control_call_function")
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(before_call_ids.len(), 3, "three handwritten Function calls");

    let anchored = board_to_flowscript(
        &board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    let renamed_source = anchored.replace("tag(", "formatTag(");
    assert_ne!(renamed_source, anchored, "fixture rename changed no source");

    let renamed = apply_flowscript_to_board(
        &mut board,
        &renamed_source,
        &CATALOG.nodes,
        state.clone(),
        None,
        false,
    )
    .await
    .expect("renamed handwritten program applies");
    assert!(renamed.diagnostics.is_empty(), "{:?}", renamed.diagnostics);
    assert_eq!(
        renamed.board_commands.len(),
        1,
        "{:?}",
        renamed.board_commands
    );
    assert!(matches!(
        &renamed.board_commands[0],
        BoardCommand::RenameLayer { layer_id, name, .. }
            if layer_id == &before.id && name == "formatTag"
    ));

    let after = board
        .layers
        .get(&before.id)
        .expect("renamed Function keeps its layer ID");
    assert_eq!(after.name, "formatTag");
    assert_eq!(after.parent_id, before.parent_id);
    assert_eq!(after.cache, before.cache);
    assert_eq!(
        after.pins.keys().collect::<HashSet<_>>(),
        before.pins.keys().collect::<HashSet<_>>()
    );
    assert_eq!(
        after.nodes.keys().collect::<HashSet<_>>(),
        before.nodes.keys().collect::<HashSet<_>>()
    );
    assert_eq!(
        after.variables.keys().collect::<HashSet<_>>(),
        before.variables.keys().collect::<HashSet<_>>()
    );
    assert_eq!(board_node_and_layer_ids(&board), before_node_ids);

    let after_calls = board
        .nodes
        .values()
        .filter(|node| node.name == "control_call_function")
        .collect::<Vec<_>>();
    assert_eq!(
        after_calls
            .iter()
            .map(|node| node.id.clone())
            .collect::<HashSet<_>>(),
        before_call_ids
    );
    for call in after_calls {
        let target = call
            .pins
            .values()
            .find(|pin| pin.name == "function_layer_id")
            .and_then(|pin| pin.default_value.as_deref())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(bytes).ok());
        assert_eq!(target.as_deref(), Some(before.id.as_str()));
    }

    let rendered = board_to_flowscript(
        &board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    assert!(rendered.contains("function formatTag("), "{rendered}");
    assert!(!rendered.contains("function tag("), "{rendered}");
    let noop = apply_flowscript_to_board(&mut board, &rendered, &CATALOG.nodes, state, None, false)
        .await
        .expect("renamed rendering reapplies");
    assert!(noop.diagnostics.is_empty(), "{:?}", noop.diagnostics);
    assert!(noop.board_commands.is_empty(), "{:?}", noop.board_commands);
}

/// Copy a nested Event whose module, Function, body nodes, and local identities are all foreign to
/// the destination Board. Every unavailable anchor must flow through same-batch layer references
/// and produce a self-contained graph with fresh node and layer IDs.
#[tokio::test]
async fn copied_nested_event_recreates_inside_new_module_and_function() {
    let source = r#"use log::{ info }
use string::{ trim }

module tools {
    function normalize(value: string): (result: string) {
        eventsGeneric audit(message: string) {
            info({ message: message, toast: false })
        }
        const result = trim(value)
        return result
    }
}

eventsSimple copiedAnchorProbe() {
    const result = tools::normalize("  ready  ")
    info({ message: result, toast: false })
}
"#;
    let state = catalog_state().await;
    let mut source_board =
        Board::new_detached(Some("nested-anchor-source".to_string()), Path::default());
    let initial = apply_flowscript_to_board(
        &mut source_board,
        source,
        &CATALOG.nodes,
        state.clone(),
        None,
        false,
    )
    .await
    .expect("nested source applies");
    assert!(initial.diagnostics.is_empty(), "{:?}", initial.diagnostics);

    let anchored = board_to_flowscript(
        &source_board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    assert!(anchored.contains("//@l:"), "{anchored}");
    assert!(anchored.contains("//@n:"), "{anchored}");
    let source_ids = board_node_and_layer_ids(&source_board);

    let mut copied_board =
        Board::new_detached(Some("nested-anchor-copy".to_string()), Path::default());
    let copied = apply_flowscript_to_board(
        &mut copied_board,
        &anchored,
        &CATALOG.nodes,
        state.clone(),
        None,
        false,
    )
    .await
    .expect("nested copied anchors apply");
    assert!(copied.diagnostics.is_empty(), "{:?}", copied.diagnostics);
    assert!(
        source_ids.is_disjoint(&board_node_and_layer_ids(&copied_board)),
        "copied graph reused a source node or layer ID"
    );

    let function = copied_board
        .layers
        .values()
        .find(|layer| layer.name == "normalize")
        .expect("copied Function layer");
    let audit = copied_board
        .nodes
        .values()
        .find(|node| node.name == "events_generic" && node.friendly_name == "audit")
        .expect("copied nested Event entry");
    assert_eq!(audit.layer.as_deref(), Some(function.id.as_str()));

    let rendered = board_to_flowscript(
        &copied_board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    let noop = apply_flowscript_to_board(
        &mut copied_board,
        &rendered,
        &CATALOG.nodes,
        state,
        None,
        false,
    )
    .await
    .expect("copied nested graph reapplies");
    assert!(noop.diagnostics.is_empty(), "{:?}", noop.diagnostics);
    assert!(noop.board_commands.is_empty(), "{:?}", noop.board_commands);
}
