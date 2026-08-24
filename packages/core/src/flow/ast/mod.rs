//! Board ⇄ FlowScript glue (the *board* half of the pipeline).
//!
//! The language half — the [`flow_like_ast`] crate — owns the IR plus pure render/parse/lint
//! operations. This module owns everything that touches the core [`Board`] graph model:
//! lowering a board into the AST today, and reconcile/placement in later phases.
//!
//! See `todo/ast.md`.

mod apply;
mod diagnostics;
mod lower;
mod reconcile;
mod signatures;
mod template;
mod types;

pub use apply::{
    ApplyFlowScriptResult, apply_board_commands_to_board, apply_flowscript_to_board,
    blocked_destructive_flowscript_message, destructive_flowscript_command_summaries,
};
pub use diagnostics::{
    FlowScriptDiagnostic, FlowScriptDiagnosticCode, FlowScriptDiagnosticFix,
    FlowScriptDiagnosticPhase, FlowScriptSourcePosition, FlowScriptSourceSpan,
    structure_reconcile_diagnostics,
};
pub use flow_like_ast::{
    BoardAst, DeclarationFile, NameCollision, NameEntry, NodeNames, NodeSchemas, ParseError,
    RedactedFlowScript, RenderOptions, Signature, SignatureSet, check_names,
    declarations_by_category, declarations_by_package, is_signature_line, parse, redact_flowscript,
    render, schema_sidecar,
};
pub use lower::{binary_operator_node_types, lower_board, pin_is_untouched_default};
pub use reconcile::{
    MAX_NODES_PER_LAYER, MetadataEnricher, ReconcileMode, ReconcileResult, reconcile,
    reconcile_text, reconcile_text_with_catalog, reconcile_text_with_catalog_enriched,
    reconcile_with_catalog, reconcile_with_catalog_mode,
};
pub(crate) use reconcile::{catalog_names, parse_pin_occurrence_ref, pin_occurrence_ref};
pub(crate) use reconcile::{
    dynamic_placeholder_config_pin, synthesize_dynamic_input_pin_from_template,
};
pub use signatures::{node_name_entry, node_names, node_to_signature, node_to_signature_in};
pub(crate) use template::template_format_call;

use crate::flow::board::Board;

/// Lower a board into the FlowScript AST.
pub fn lower_to_ast(board: &Board) -> BoardAst {
    lower::lower_board(board)
}

/// Lower a board and render it to FlowScript text in one step.
pub fn board_to_flowscript(board: &Board, opts: &RenderOptions) -> String {
    render(&lower::lower_board(board), opts)
}

#[cfg(test)]
mod generate_flowscript {
    use super::*;
    use flow_like_types::{FromProto, tokio};
    use std::path::{Path as FsPath, PathBuf};
    use std::sync::Arc;

    /// Set this env var (to any non-empty value) to regenerate the committed `.flow` snapshots
    /// instead of asserting against them. Example:
    /// `UPDATE_AST_SNAPSHOTS=1 cargo test -p flow-like --lib flow::ast::generate_flowscript`.
    const UPDATE_ENV: &str = "UPDATE_AST_SNAPSHOTS";

    /// Compare `rendered` against the committed snapshot at `path`. In update mode the snapshot is
    /// (re)written; otherwise a mismatch (or missing file) is recorded for a batched failure.
    fn check_snapshot(path: &FsPath, rendered: &str, update: bool, mismatches: &mut Vec<String>) {
        if update {
            std::fs::write(path, rendered).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(expected) if expected == rendered => {}
            Ok(_) => mismatches.push(format!("{}: content differs", path.display())),
            Err(_) => mismatches.push(format!("{}: snapshot missing", path.display())),
        }
    }

    /// The fixture boards predate explicit FlowScript names on placed nodes. A loaded board
    /// gets them (and a repaired `category`) from the catalog — `sync_board_node_schemas` runs
    /// on every load — but the catalog is not available in this crate, so stamp its committed
    /// snapshot (`flow.d/names.json`) the same way and the fixtures lower exactly as a loaded
    /// board does.
    fn fixture_board(proto: flow_like_types::proto::Board) -> Board {
        use std::{collections::BTreeMap, sync::OnceLock};
        static NAMES: OnceLock<BTreeMap<String, NodeNames>> = OnceLock::new();
        let names = NAMES.get_or_init(|| {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ast/flow.d/names.json");
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            flow_like_types::json::from_str(&text).expect("parse flow.d/names.json")
        });
        let stamp = |node: &mut crate::flow::node::Node| {
            if let Some(names) = names.get(&node.name) {
                node.set_flowscript_name(&names.namespace, &names.alias);
                node.set_receiver(names.receiver.as_deref().unwrap_or(""));
                node.category.clone_from(&names.category);
            }
        };
        let mut board = Board::from_proto(proto);
        board.nodes.values_mut().for_each(stamp);
        for layer in board.layers.values_mut() {
            layer.nodes.values_mut().for_each(stamp);
        }
        board
    }

    /// Recursively collect every `.board` fixture under `dir`, descending into subdirectories
    /// (e.g. `widgets-pages/`) so dashboard/widget-driven boards are exercised too.
    fn collect_boards(dir: &FsPath, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read tests/ast").flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_boards(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("board") {
                out.push(path);
            }
        }
    }

    /// Decode every `tests/ast/**/*.board` fixture, lower it to FlowScript, and assert the rendered
    /// text matches the committed `<name>.flow` / `<name>.anchored.flow` snapshots placed next to
    /// each board. Run with `UPDATE_AST_SNAPSHOTS=1` to regenerate the snapshots after an
    /// intentional change.
    #[tokio::test]
    async fn matches_flowscript_snapshots() {
        let update = std::env::var(UPDATE_ENV).is_ok_and(|v| !v.is_empty());

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/ast")
            .canonicalize()
            .expect("tests/ast directory should exist");

        let store: Arc<dyn flow_like_storage::object_store::ObjectStore> = Arc::new(
            flow_like_storage::object_store::local::LocalFileSystem::new_with_prefix(&dir)
                .expect("local object store"),
        );

        let mut boards: Vec<PathBuf> = Vec::new();
        collect_boards(&dir, &mut boards);
        boards.sort();
        assert!(!boards.is_empty(), "no .board fixtures found in {dir:?}");

        let mut mismatches: Vec<String> = Vec::new();

        for board_path in boards {
            let rel = board_path
                .strip_prefix(&dir)
                .expect("board path under tests/ast")
                .to_string_lossy()
                .to_string();
            let store_path = flow_like_storage::Path::from(rel.clone());
            let proto: flow_like_types::proto::Board =
                crate::utils::compression::from_compressed(store.clone(), store_path)
                    .await
                    .unwrap_or_else(|e| panic!("decode {rel}: {e}"));
            let board = fixture_board(proto);

            let plain = board_to_flowscript(&board, &RenderOptions::default());
            let annotated = board_to_flowscript(
                &board,
                &RenderOptions {
                    anchors: true,
                    ..RenderOptions::default()
                },
            );

            let stem = board_path
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let parent = board_path.parent().unwrap_or(&dir);
            check_snapshot(
                &parent.join(format!("{stem}.flow")),
                &plain,
                update,
                &mut mismatches,
            );
            check_snapshot(
                &parent.join(format!("{stem}.anchored.flow")),
                &annotated,
                update,
                &mut mismatches,
            );
        }

        assert!(
            mismatches.is_empty(),
            "FlowScript snapshots are out of date:\n  {}\nRe-run with `{UPDATE_ENV}=1` to regenerate after verifying the diff.",
            mismatches.join("\n  ")
        );
    }

    /// Decode every fixture board, render it as anchored FlowScript, and reconcile that document
    /// back against the SAME board (catalog = the board's own nodes): an unchanged document must
    /// be a no-op — no commands, no diagnostics. This exercises the full lower→parse→reconcile
    /// pipeline on real boards, including functions/layers, loops, and streaming handlers.
    ///
    /// Known remaining gap (2026-07-16, run manually with `--ignored` while closing it). The
    /// 2026-07-05 list (anchored Assign ConnectPins re-emission, variable.field reader reuse,
    /// event-level `return`, conflicting board-derived declarations, boundary/variable schema
    /// projection drift, duplicate multi-exec arm labels, composite-literal pin writes, node
    /// budget on pre-existing overfull layers) is fixed with targeted regression tests. What is
    /// left is one lowering-expressiveness class:
    /// - boards with DUPLICATED tool-handler subgraphs / cross-handler reads render a bare local
    ///   name (e.g. the current handler's `url` param) for an edge that actually originates from
    ///   a DIFFERENT same-named entry or sibling subtree, so reconcile re-wires the consumer to
    ///   the local producer (ConnectPins churn) and expression calls inside those subtrees can
    ///   still hit "matched conflicting catalog declarations".
    #[ignore = "documents the remaining lower→reconcile roundtrip gap: cross-handler/duplicated-subgraph name collapse"]
    #[tokio::test]
    async fn anchored_roundtrip_is_noop() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/ast")
            .canonicalize()
            .expect("tests/ast directory should exist");

        let store: Arc<dyn flow_like_storage::object_store::ObjectStore> = Arc::new(
            flow_like_storage::object_store::local::LocalFileSystem::new_with_prefix(&dir)
                .expect("local object store"),
        );

        let mut boards: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("read tests/ast")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("board"))
            .collect();
        boards.sort();
        assert!(!boards.is_empty(), "no .board fixtures found in {dir:?}");

        let mut failures: Vec<String> = Vec::new();

        for board_path in boards {
            let file_name = board_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let store_path = flow_like_storage::Path::from(file_name.clone());
            let proto: flow_like_types::proto::Board =
                crate::utils::compression::from_compressed(store.clone(), store_path)
                    .await
                    .unwrap_or_else(|e| panic!("decode {file_name}: {e}"));
            let board = fixture_board(proto);

            let annotated = board_to_flowscript(
                &board,
                &RenderOptions {
                    anchors: true,
                    ..RenderOptions::default()
                },
            );
            let catalog: Vec<crate::flow::copilot::NodeMetadata> = board
                .nodes
                .values()
                .map(crate::flow::copilot::node_to_metadata)
                .collect();

            let result = reconcile_text_with_catalog(&board, &annotated, &catalog);
            if !result.diagnostics.is_empty() {
                failures.push(format!(
                    "{file_name}: roundtrip diagnostics:\n    {}",
                    result.diagnostics.join("\n    ")
                ));
            }
            if !result.commands.is_empty() {
                let summaries: Vec<String> = result
                    .commands
                    .iter()
                    .map(|command| format!("{command:?}"))
                    .collect();
                failures.push(format!(
                    "{file_name}: roundtrip produced {} commands (expected none):\n    {}",
                    summaries.len(),
                    summaries.join("\n    ")
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "anchored FlowScript roundtrip is not a no-op:\n  {}",
            failures.join("\n  ")
        );
    }
}

/// Property tests over the full FlowScript pipeline: generated programs are applied to an empty
/// board through `apply_flowscript_to_board`, the resulting board is lowered back to anchored
/// text, and re-reconciling that text against the board must be a perfect no-op (idempotency).
/// Random single-token mutations of the lowered text must never panic parse/reconcile and must
/// never yield destructive commands that escape both the diagnostics gate and the deletion gate.
#[cfg(test)]
mod roundtrip_properties {
    use super::*;
    use crate::flow::copilot::{NodeMetadata, node_to_metadata};
    use crate::flow::node::Node;
    use crate::flow::variable::VariableType;
    use crate::state::{FlowLikeConfig, FlowLikeState};
    use crate::utils::http::HTTPClient;
    use proptest::prelude::*;
    use proptest::test_runner::{Config as PropConfig, RngAlgorithm, TestRng, TestRunner};
    use std::fmt::Write as _;
    use std::sync::Arc;

    fn empty_board() -> crate::flow::board::Board {
        use crate::flow::board::{Board, ExecutionMode, ExecutionStage};
        use crate::flow::execution::LogLevel;
        use std::collections::HashMap;
        use std::time::SystemTime;
        Board {
            id: "board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: flow_like_storage::Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
            pin_index: None,
        }
    }

    /// Tiny fixed vocabulary of catalog-shaped nodes mirroring the real catalog contracts the
    /// reconciler depends on (events, logging, a pure string helper, variable accessors, branch).
    fn vocabulary_nodes() -> Vec<Node> {
        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        let mut generic_event = Node::new("events_generic", "Generic Event", "", "events");
        generic_event.set_start(true);
        generic_event.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        let mut ret = Node::new(
            "events_generic_return_result",
            "Return Result",
            "",
            "events",
        );
        ret.set_event_callback(true);
        ret.add_input_pin("exec_in", "In", "", VariableType::Execution);
        ret.add_input_pin("response", "Response", "", VariableType::Generic);

        let mut log = Node::new("log", "Log", "", "debug");
        log.add_input_pin("exec_in", "In", "", VariableType::Execution);
        log.add_input_pin("message", "Message", "", VariableType::String);
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        let mut trim = Node::new("string_trim", "String Trim", "", "strings");
        trim.add_input_pin("string", "String", "", VariableType::String);
        trim.add_output_pin("trimmed", "Trimmed", "", VariableType::String);

        let mut variable_get = Node::new("variable_get", "Get Variable", "", "variables");
        variable_get.add_input_pin("var_ref", "Variable", "", VariableType::String);
        variable_get.add_output_pin("value_ref", "Value", "", VariableType::Generic);

        let mut variable_set = Node::new("variable_set", "Set Variable", "", "variables");
        variable_set.add_input_pin("exec_in", "In", "", VariableType::Execution);
        variable_set.add_input_pin("var_ref", "Variable", "", VariableType::String);
        variable_set.add_input_pin("value_in", "Value", "", VariableType::Generic);
        variable_set.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        variable_set.add_output_pin("value_ref", "Value", "", VariableType::Generic);

        let mut branch = Node::new("control_branch", "Branch", "", "control");
        branch.add_input_pin("exec_in", "In", "", VariableType::Execution);
        branch.add_input_pin("condition", "Condition", "", VariableType::Boolean);
        branch.add_output_pin("true", "True", "", VariableType::Execution);
        branch.add_output_pin("false", "False", "", VariableType::Execution);

        vec![
            event,
            generic_event,
            ret,
            log,
            trim,
            variable_get,
            variable_set,
            branch,
        ]
    }

    #[derive(Debug, Clone)]
    enum PExpr {
        Lit(String),
        Trim(String),
        Local(usize),
    }

    #[derive(Debug, Clone)]
    enum PStmt {
        Log(PExpr),
        Assign(usize, String),
        If {
            condition: bool,
            then: Vec<PStmt>,
            otherwise: Vec<PStmt>,
        },
    }

    #[derive(Debug, Clone)]
    struct PFn {
        name: String,
        locals: usize,
        stmts: Vec<PStmt>,
        ret: PExpr,
    }

    #[derive(Debug, Clone)]
    struct PProgram {
        event_name: String,
        event_locals: usize,
        event_stmts: Vec<PStmt>,
        functions: Vec<PFn>,
    }

    fn lit_strategy() -> impl Strategy<Value = String> {
        prop::sample::select(vec![
            "alpha".to_string(),
            "beta".to_string(),
            "gamma delta".to_string(),
            "x1".to_string(),
            String::new(),
        ])
    }

    fn expr_strategy(locals: usize) -> BoxedStrategy<PExpr> {
        let mut options = vec![
            lit_strategy().prop_map(PExpr::Lit).boxed(),
            lit_strategy().prop_map(PExpr::Trim).boxed(),
        ];
        if locals > 0 {
            options.push((0..locals).prop_map(PExpr::Local).boxed());
        }
        proptest::strategy::Union::new(options).boxed()
    }

    fn simple_stmt_strategy(locals: usize) -> BoxedStrategy<PStmt> {
        let mut options = vec![expr_strategy(locals).prop_map(PStmt::Log).boxed()];
        if locals > 0 {
            options.push(
                ((0..locals), lit_strategy())
                    .prop_map(|(local, value)| PStmt::Assign(local, value))
                    .boxed(),
            );
        }
        proptest::strategy::Union::new(options).boxed()
    }

    fn stmt_strategy(locals: usize) -> BoxedStrategy<PStmt> {
        let leaf = simple_stmt_strategy(locals);
        let arm = prop::collection::vec(simple_stmt_strategy(locals), 0..=2);
        let branch =
            (any::<bool>(), arm.clone(), arm).prop_map(|(condition, then, otherwise)| PStmt::If {
                condition,
                then,
                otherwise,
            });
        proptest::strategy::Union::new(vec![leaf, branch.boxed()]).boxed()
    }

    fn body_strategy() -> impl Strategy<Value = (usize, Vec<PStmt>)> {
        (0usize..=2).prop_flat_map(|locals| {
            prop::collection::vec(stmt_strategy(locals), 0..=3)
                .prop_map(move |stmts| (locals, stmts))
        })
    }

    fn function_strategy(index: usize) -> impl Strategy<Value = PFn> {
        (body_strategy(), lit_strategy(), any::<u8>()).prop_map(
            move |((locals, stmts), lit, pick)| {
                let ret = match pick % 3 {
                    0 => PExpr::Lit(lit),
                    1 => PExpr::Trim(lit),
                    _ if locals > 0 => PExpr::Local(pick as usize % locals),
                    _ => PExpr::Lit(lit),
                };
                PFn {
                    name: format!("helper{index}"),
                    locals,
                    stmts,
                    ret,
                }
            },
        )
    }

    fn program_strategy() -> impl Strategy<Value = PProgram> {
        (
            prop::sample::select(vec![
                "onTick".to_string(),
                "onMessage".to_string(),
                "onSubmit".to_string(),
            ]),
            body_strategy(),
            prop::collection::vec(any::<u8>(), 0..=2),
        )
            .prop_flat_map(|(event_name, (event_locals, event_stmts), fn_seeds)| {
                let functions: Vec<BoxedStrategy<PFn>> = fn_seeds
                    .iter()
                    .enumerate()
                    .map(|(index, _)| function_strategy(index).boxed())
                    .collect();
                (
                    Just(event_name),
                    Just(event_locals),
                    Just(event_stmts),
                    functions,
                )
                    .prop_map(
                        |(event_name, event_locals, event_stmts, functions)| PProgram {
                            event_name,
                            event_locals,
                            event_stmts,
                            functions,
                        },
                    )
            })
    }

    fn render_expr(expr: &PExpr) -> String {
        match expr {
            PExpr::Lit(value) => format!("{value:?}"),
            PExpr::Trim(value) => format!("stringTrim({{ string: {value:?} }})"),
            PExpr::Local(index) => format!("l{index}"),
        }
    }

    fn render_stmt(stmt: &PStmt, indent: usize, out: &mut String) {
        let pad = "    ".repeat(indent);
        match stmt {
            PStmt::Log(expr) => {
                let _ = writeln!(out, "{pad}log({{ message: {} }})", render_expr(expr));
            }
            PStmt::Assign(local, value) => {
                let _ = writeln!(out, "{pad}l{local} = {value:?}");
            }
            PStmt::If {
                condition,
                then,
                otherwise,
            } => {
                let _ = writeln!(out, "{pad}if ({condition}) {{");
                for stmt in then {
                    render_stmt(stmt, indent + 1, out);
                }
                if otherwise.is_empty() {
                    let _ = writeln!(out, "{pad}}}");
                } else {
                    let _ = writeln!(out, "{pad}}} else {{");
                    for stmt in otherwise {
                        render_stmt(stmt, indent + 1, out);
                    }
                    let _ = writeln!(out, "{pad}}}");
                }
            }
        }
    }

    fn render_body(locals: usize, stmts: &[PStmt], indent: usize, out: &mut String) {
        let pad = "    ".repeat(indent);
        let _ = writeln!(out, "{pad}log({{ message: \"entered\" }})");
        for local in 0..locals {
            let _ = writeln!(out, "{pad}let l{local} = \"seed{local}\"");
        }
        for stmt in stmts {
            render_stmt(stmt, indent, out);
        }
    }

    fn render_program(program: &PProgram) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{}() {{", program.event_name);
        render_body(program.event_locals, &program.event_stmts, 1, &mut out);
        let _ = writeln!(out, "}}");
        for function in &program.functions {
            let _ = writeln!(out, "\nfunction {}(): (result: string) {{", function.name);
            render_body(function.locals, &function.stmts, 1, &mut out);
            let _ = writeln!(out, "    return {}", render_expr(&function.ret));
            let _ = writeln!(out, "}}");
        }
        out
    }

    /// Apply `program` to an empty board; panics (failing the property) on apply diagnostics —
    /// every generated program is valid by construction.
    fn apply_program(program: &PProgram) -> (crate::flow::board::Board, Vec<NodeMetadata>, String) {
        let catalog_nodes = vocabulary_nodes();
        let source = render_program(program);
        let mut board = empty_board();
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let runtime = flow_like_types::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let applied = runtime
            .block_on(apply_flowscript_to_board(
                &mut board,
                &source,
                &catalog_nodes,
                state,
                None,
                false,
            ))
            .unwrap_or_else(|error| panic!("apply failed for program:\n{source}\n{error}"));
        assert!(
            applied.diagnostics.is_empty(),
            "generated program must apply cleanly:\n{source}\ndiagnostics: {:?}",
            applied.diagnostics
        );
        let catalog: Vec<NodeMetadata> = catalog_nodes.iter().map(node_to_metadata).collect();
        (board, catalog, source)
    }

    fn assert_roundtrip_idempotent(program: &PProgram) {
        let (board, catalog, source) = apply_program(program);

        let anchored = board_to_flowscript(
            &board,
            &RenderOptions {
                anchors: true,
                ..RenderOptions::default()
            },
        );
        assert!(
            anchored.contains("log(") && anchored.contains("//@n:"),
            "the applied board must lower back to real anchored statements:\n{anchored}"
        );
        let reparsed = parse(&anchored)
            .unwrap_or_else(|error| panic!("lowered text must parse:\n{anchored}\n{error:?}"));
        let result = reconcile_with_catalog(&board, &reparsed, &catalog);

        assert!(
            result.diagnostics.is_empty(),
            "re-reconcile must not diagnose.\nprogram:\n{source}\nlowered:\n{anchored}\ndiagnostics: {:?}",
            result.diagnostics
        );
        assert!(
            result.commands.is_empty(),
            "re-reconcile must be a no-op.\nprogram:\n{source}\nlowered:\n{anchored}\ncommands: {:?}",
            result.commands
        );
    }

    /// One deterministic single-token mutation of `text` (delete / duplicate / replace / break a
    /// token), chosen by `token_pick`/`kind`.
    fn mutate_single_token(text: &str, token_pick: usize, kind: u8) -> Option<String> {
        let tokens: Vec<(usize, &str)> = text
            .split_whitespace()
            .map(|token| {
                let offset = token.as_ptr() as usize - text.as_ptr() as usize;
                (offset, token)
            })
            .collect();
        if tokens.is_empty() {
            return None;
        }
        let (offset, token) = tokens[token_pick % tokens.len()];
        let (before, rest) = text.split_at(offset);
        let after = &rest[token.len()..];
        let mutated = match kind % 4 {
            0 => format!("{before}{after}"),
            1 => format!("{before}{token} {token}{after}"),
            2 => format!("{before}qqq{after}"),
            _ => format!("{before}]{after}"),
        };
        (mutated != text).then_some(mutated)
    }

    fn assert_mutation_never_destroys_silently(program: &PProgram, token_pick: usize, kind: u8) {
        let (board, catalog, _) = apply_program(program);
        let anchored = board_to_flowscript(
            &board,
            &RenderOptions {
                anchors: true,
                ..RenderOptions::default()
            },
        );
        let Some(mutated) = mutate_single_token(&anchored, token_pick, kind) else {
            return;
        };

        // Parse must never panic; a parse error is a legitimate outcome.
        let _ = parse(&mutated);

        // Reconcile must never panic, and destructive commands must never escape BOTH gates:
        // either diagnostics block the apply, or the deletion gate enumerates every removal.
        let result = reconcile_text_with_catalog(&board, &mutated, &catalog);
        let removals = result
            .commands
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    crate::flow::copilot::BoardCommand::RemoveNode { .. }
                        | crate::flow::copilot::BoardCommand::RemoveVariable { .. }
                        | crate::flow::copilot::BoardCommand::RemoveLayer { .. }
                        | crate::flow::copilot::BoardCommand::RemoveComment { .. }
                )
            })
            .count();
        let gated = destructive_flowscript_command_summaries(&result.commands).len();
        assert_eq!(
            removals, gated,
            "every removal must be visible to the deletion gate.\nmutated:\n{mutated}\ncommands: {:?}",
            result.commands
        );
        assert!(
            removals == 0 || !result.diagnostics.is_empty() || gated > 0,
            "destructive commands must be accompanied by a blocking signal.\nmutated:\n{mutated}\ncommands: {:?}",
            result.commands
        );
    }

    fn run_property<S, F>(cases: u32, strategy: S, check: F)
    where
        S: Strategy,
        F: Fn(&S::Value),
    {
        let config = PropConfig {
            cases,
            failure_persistence: None,
            ..PropConfig::default()
        };
        // Fixed seed: CI runs are reproducible; the high-iteration variant widens coverage.
        let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
        let mut runner = TestRunner::new_with_rng(config, rng);
        runner
            .run(&strategy, |value| {
                check(&value);
                Ok(())
            })
            .unwrap_or_else(|error| panic!("property failed: {error}"));
    }

    #[test]
    fn generated_programs_roundtrip_idempotently() {
        run_property(64, program_strategy(), assert_roundtrip_idempotent);
    }

    #[test]
    #[ignore = "high-iteration variant of the roundtrip property; run manually"]
    fn generated_programs_roundtrip_idempotently_high_iteration() {
        run_property(512, program_strategy(), assert_roundtrip_idempotent);
    }

    #[test]
    fn single_token_mutations_never_panic_or_destroy_silently() {
        run_property(
            64,
            (
                program_strategy(),
                any::<prop::sample::Index>(),
                any::<u8>(),
            ),
            |(program, index, kind)| {
                assert_mutation_never_destroys_silently(program, index.index(usize::MAX - 1), *kind)
            },
        );
    }

    #[test]
    #[ignore = "high-iteration variant of the mutation property; run manually"]
    fn single_token_mutations_never_panic_or_destroy_silently_high_iteration() {
        run_property(
            512,
            (
                program_strategy(),
                any::<prop::sample::Index>(),
                any::<u8>(),
            ),
            |(program, index, kind)| {
                assert_mutation_never_destroys_silently(program, index.index(usize::MAX - 1), *kind)
            },
        );
    }
}

#[cfg(test)]
mod lower_tests {
    use super::*;
    use crate::flow::board::{Board, ExecutionMode, ExecutionStage, Layer, LayerType};
    use crate::flow::execution::LogLevel;
    use crate::flow::node::Node;
    use crate::flow::variable::{VariableType, infer_schema_from_json};
    use flow_like_ast::model::Stmt;
    use flow_like_storage::Path;
    use flow_like_types::Value;
    use std::collections::HashMap;
    use std::time::SystemTime;

    fn empty_board() -> Board {
        Board {
            id: "board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
            pin_index: None,
        }
    }

    fn connect(board: &mut Board, from_node: &str, from_pin: &str, to_node: &str, to_pin: &str) {
        crate::flow::board::commands::pins::connect_pins::connect_pins(
            board, from_node, from_pin, to_node, to_pin,
        )
        .expect("connect pins");
    }

    fn exec_log(id: &str, layer: Option<&str>, message: &str) -> (Node, String, String) {
        let mut log = Node::new("log_info", "Log Info", "", "debug");
        log.id = id.to_string();
        log.layer = layer.map(str::to_string);
        let exec_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_input_pin("message", "Message", "", VariableType::String)
            .set_default_value(Some(Value::String(message.to_string())));
        let exec_out = log
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        (log, exec_in, exec_out)
    }

    #[test]
    fn if_else_shared_query_join_stays_at_function_scope_before_return() {
        let mut board = empty_board();
        let layer_id = "read-threads-layer";
        let mut layer = Layer::new(
            layer_id.to_string(),
            "readThreads".to_string(),
            LayerType::Function,
        );
        let mut boundary = Node::new("boundary", "Boundary", "", "test");
        let rows_return = boundary.add_output_pin("rows", "Rows", "", VariableType::Struct);
        rows_return.set_value_type(crate::flow::pin::ValueType::Array);
        let rows_return = rows_return.clone();
        layer
            .pins
            .insert(rows_return.id.clone(), rows_return.clone());
        board.layers.insert(layer.id.clone(), layer);

        let mut branch = Node::new("control_branch", "Branch", "", "control");
        branch.id = "branch".to_string();
        branch.layer = Some(layer_id.to_string());
        branch.add_input_pin("exec_in", "In", "", VariableType::Execution);
        branch
            .add_input_pin("condition", "Condition", "", VariableType::Boolean)
            .set_default_value(Some(Value::Bool(true)));
        let branch_true = branch
            .add_output_pin("true", "True", "", VariableType::Execution)
            .id
            .clone();
        let branch_false = branch
            .add_output_pin("false", "False", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(branch.id.clone(), branch);

        let (then_log, then_in, then_out) = exec_log("then-log", Some(layer_id), "then branch");
        board.nodes.insert(then_log.id.clone(), then_log);
        let (else_log, else_in, else_out) = exec_log("else-log", Some(layer_id), "else branch");
        board.nodes.insert(else_log.id.clone(), else_log);

        let mut query = Node::new("df_sql_query", "SQL Query", "", "data");
        query.id = "query".to_string();
        query.layer = Some(layer_id.to_string());
        let query_in = query
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        query
            .add_input_pin("query", "Query", "", VariableType::String)
            .set_default_value(Some(Value::String("SELECT * FROM threads".to_string())));
        query.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let query_rows = query.add_output_pin("rows", "Rows", "", VariableType::Struct);
        query_rows.set_value_type(crate::flow::pin::ValueType::Array);
        let query_rows = query_rows.id.clone();
        board.nodes.insert(query.id.clone(), query);

        connect(&mut board, "branch", &branch_true, "then-log", &then_in);
        connect(&mut board, "branch", &branch_false, "else-log", &else_in);
        connect(&mut board, "then-log", &then_out, "query", &query_in);
        connect(&mut board, "else-log", &else_out, "query", &query_in);
        connect(&mut board, "query", &query_rows, layer_id, &rows_return.id);

        let ast = lower_to_ast(&board);
        let function = ast
            .functions
            .iter()
            .find(|function| function.name == "readThreads")
            .expect("readThreads function");
        assert_eq!(
            function.body.stmts.len(),
            3,
            "branch, shared query, and return must be siblings: {:?}",
            function.body.stmts
        );
        let Stmt::Branch { arms, .. } = &function.body.stmts[0] else {
            panic!("first statement must be the if/else");
        };
        assert_eq!(arms.len(), 2);
        assert!(
            arms.iter().all(|arm| arm.body.stmts.len() == 1),
            "each arm must contain only its own log: {arms:?}"
        );
        assert!(matches!(
            &function.body.stmts[1],
            Stmt::Destructure {
                anchor: Some(anchor),
                ..
            } if anchor == "query"
        ));
        assert!(matches!(&function.body.stmts[2], Stmt::Return { .. }));

        let text = board_to_flowscript(
            &board,
            &RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );
        let branch_end = text.find("    const { rows } = ").expect("top-level query");
        let return_start = text.find("    return rows").expect("return query rows");
        assert!(
            branch_end < return_start,
            "rendered query and return must remain top-level siblings:\n{text}"
        );
        let roundtrip = reconcile_text(&board, &text);
        assert!(
            roundtrip.diagnostics.is_empty(),
            "canonical readback must resolve the post-branch query return:\n{text}\n{:?}",
            roundtrip.diagnostics
        );
        assert!(
            roundtrip.commands.is_empty(),
            "canonical readback must remain a no-op:\n{text}\n{:?}",
            roundtrip.commands
        );
    }

    #[test]
    fn lone_if_shared_continuation_stays_after_the_branch() {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Run", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut branch = Node::new("control_branch", "Branch", "", "control");
        branch.id = "branch".to_string();
        let branch_in = branch
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        branch
            .add_input_pin("condition", "Condition", "", VariableType::Boolean)
            .set_default_value(Some(Value::Bool(true)));
        let branch_true = branch
            .add_output_pin("true", "True", "", VariableType::Execution)
            .id
            .clone();
        let branch_false = branch
            .add_output_pin("false", "False", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(branch.id.clone(), branch);

        let (then_log, then_in, then_out) = exec_log("then-log", None, "yes");
        board.nodes.insert(then_log.id.clone(), then_log);
        let (after_log, after_in, _) = exec_log("after-log", None, "after");
        board.nodes.insert(after_log.id.clone(), after_log);

        connect(&mut board, "event", &event_out, "branch", &branch_in);
        connect(&mut board, "branch", &branch_true, "then-log", &then_in);
        connect(&mut board, "then-log", &then_out, "after-log", &after_in);
        connect(&mut board, "branch", &branch_false, "after-log", &after_in);

        let ast = lower_to_ast(&board);
        let body = &ast.events[0].body.stmts;
        assert_eq!(
            body.len(),
            2,
            "the post-if log must be an event-body sibling: {body:?}"
        );
        let Stmt::Branch { arms, .. } = &body[0] else {
            panic!("first event statement must be the if");
        };
        assert_eq!(arms.len(), 2);
        assert_eq!(arms[0].body.stmts.len(), 1);
        assert!(arms[1].body.stmts.is_empty());
        assert!(matches!(
            &body[1],
            Stmt::Call {
                anchor: Some(anchor),
                ..
            } if anchor == "after-log"
        ));
    }

    #[test]
    fn nested_if_arms_leave_the_outer_shared_continuation_unconsumed() {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Run", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut outer = Node::new("control_branch", "Outer Branch", "", "control");
        outer.id = "outer".to_string();
        let outer_in = outer
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        outer
            .add_input_pin("condition", "Condition", "", VariableType::Boolean)
            .set_default_value(Some(Value::Bool(true)));
        let outer_true = outer
            .add_output_pin("true", "True", "", VariableType::Execution)
            .id
            .clone();
        let outer_false = outer
            .add_output_pin("false", "False", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(outer.id.clone(), outer);

        let mut inner = Node::new("control_branch", "Inner Branch", "", "control");
        inner.id = "inner".to_string();
        let inner_in = inner
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        inner
            .add_input_pin("condition", "Condition", "", VariableType::Boolean)
            .set_default_value(Some(Value::Bool(false)));
        let inner_true = inner
            .add_output_pin("true", "True", "", VariableType::Execution)
            .id
            .clone();
        let inner_false = inner
            .add_output_pin("false", "False", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(inner.id.clone(), inner);

        let (inner_true_log, inner_true_in, inner_true_out) =
            exec_log("inner-true-log", None, "inner true");
        board
            .nodes
            .insert(inner_true_log.id.clone(), inner_true_log);
        let (inner_false_log, inner_false_in, inner_false_out) =
            exec_log("inner-false-log", None, "inner false");
        board
            .nodes
            .insert(inner_false_log.id.clone(), inner_false_log);
        let (outer_false_log, outer_false_in, outer_false_out) =
            exec_log("outer-false-log", None, "outer false");
        board
            .nodes
            .insert(outer_false_log.id.clone(), outer_false_log);
        let (after_log, after_in, _) = exec_log("after-log", None, "after");
        board.nodes.insert(after_log.id.clone(), after_log);

        connect(&mut board, "event", &event_out, "outer", &outer_in);
        connect(&mut board, "outer", &outer_true, "inner", &inner_in);
        connect(
            &mut board,
            "outer",
            &outer_false,
            "outer-false-log",
            &outer_false_in,
        );
        connect(
            &mut board,
            "inner",
            &inner_true,
            "inner-true-log",
            &inner_true_in,
        );
        connect(
            &mut board,
            "inner",
            &inner_false,
            "inner-false-log",
            &inner_false_in,
        );
        connect(
            &mut board,
            "inner-true-log",
            &inner_true_out,
            "after-log",
            &after_in,
        );
        connect(
            &mut board,
            "inner-false-log",
            &inner_false_out,
            "after-log",
            &after_in,
        );
        connect(
            &mut board,
            "outer-false-log",
            &outer_false_out,
            "after-log",
            &after_in,
        );

        let ast = lower_to_ast(&board);
        let body = &ast.events[0].body.stmts;
        assert_eq!(body.len(), 2, "outer join must remain top-level: {body:?}");
        let Stmt::Branch {
            arms: outer_arms, ..
        } = &body[0]
        else {
            panic!("outer branch");
        };
        let Stmt::Branch {
            arms: inner_arms, ..
        } = &outer_arms[0].body.stmts[0]
        else {
            panic!("inner branch must stay inside the outer true arm");
        };
        assert!(inner_arms.iter().all(|arm| arm.body.stmts.len() == 1));
        assert_eq!(outer_arms[1].body.stmts.len(), 1);
        assert!(matches!(
            &body[1],
            Stmt::Call {
                anchor: Some(anchor),
                ..
            } if anchor == "after-log"
        ));
    }

    #[test]
    fn generic_multi_exec_shared_continuation_stays_after_its_arms() {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Run", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut choice = Node::new("http_fetch", "Fetch", "", "http");
        choice.id = "choice".to_string();
        let choice_in = choice
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let success = choice
            .add_output_pin("success", "Success", "", VariableType::Execution)
            .id
            .clone();
        let error = choice
            .add_output_pin("error", "Error", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(choice.id.clone(), choice);

        let (success_log, success_in, success_out) = exec_log("success-log", None, "success");
        board.nodes.insert(success_log.id.clone(), success_log);
        let (error_log, error_in, error_out) = exec_log("error-log", None, "error");
        board.nodes.insert(error_log.id.clone(), error_log);
        let (after_log, after_in, _) = exec_log("after-log", None, "after");
        board.nodes.insert(after_log.id.clone(), after_log);

        connect(&mut board, "event", &event_out, "choice", &choice_in);
        connect(&mut board, "choice", &success, "success-log", &success_in);
        connect(&mut board, "choice", &error, "error-log", &error_in);
        connect(
            &mut board,
            "success-log",
            &success_out,
            "after-log",
            &after_in,
        );
        connect(&mut board, "error-log", &error_out, "after-log", &after_in);

        let ast = lower_to_ast(&board);
        let body = &ast.events[0].body.stmts;
        assert_eq!(
            body.len(),
            2,
            "the shared continuation must follow the generic arm block: {body:?}"
        );
        let Stmt::Branch { arms, anchor, .. } = &body[0] else {
            panic!("generic multi-exec node must render as an arm block");
        };
        assert_eq!(anchor.as_deref(), Some("choice"));
        assert!(arms.iter().all(|arm| arm.body.stmts.len() == 1));
        assert!(matches!(
            &body[1],
            Stmt::Call {
                anchor: Some(anchor),
                ..
            } if anchor == "after-log"
        ));
    }

    #[test]
    fn multi_exec_entry_shared_continuation_stays_after_its_arms() {
        let mut board = empty_board();

        let mut event = Node::new("events_routed", "Run", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let accepted = event
            .add_output_pin("accepted", "Accepted", "", VariableType::Execution)
            .id
            .clone();
        let rejected = event
            .add_output_pin("rejected", "Rejected", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let (accepted_log, accepted_in, accepted_out) = exec_log("accepted-log", None, "accepted");
        board.nodes.insert(accepted_log.id.clone(), accepted_log);
        let (rejected_log, rejected_in, rejected_out) = exec_log("rejected-log", None, "rejected");
        board.nodes.insert(rejected_log.id.clone(), rejected_log);
        let (after_log, after_in, _) = exec_log("after-log", None, "after");
        board.nodes.insert(after_log.id.clone(), after_log);

        connect(&mut board, "event", &accepted, "accepted-log", &accepted_in);
        connect(&mut board, "event", &rejected, "rejected-log", &rejected_in);
        connect(
            &mut board,
            "accepted-log",
            &accepted_out,
            "after-log",
            &after_in,
        );
        connect(
            &mut board,
            "rejected-log",
            &rejected_out,
            "after-log",
            &after_in,
        );

        let ast = lower_to_ast(&board);
        let body = &ast.events[0].body.stmts;
        assert_eq!(
            body.len(),
            2,
            "the entry's common continuation must follow its arm block: {body:?}"
        );
        let Stmt::Branch { arms, anchor, .. } = &body[0] else {
            panic!("multi-exec entry must render as an arm block");
        };
        assert_eq!(anchor.as_deref(), Some("event"));
        assert!(arms.iter().all(|arm| arm.body.stmts.len() == 1));
        assert!(matches!(
            &body[1],
            Stmt::Call {
                anchor: Some(anchor),
                ..
            } if anchor == "after-log"
        ));
    }

    #[test]
    fn interface_schema_uses_board_schema_normalization() {
        let ast = flow_like_ast::parse(
            "interface ReportEntry {\n    title: string;\n    uri: string;\n    summary?: string | null = null;\n    tags?: string[] = [];\n}\n\nconst reportEntry: ReportEntry = {}\n",
        )
        .expect("interface form should parse");
        let generated = ast.variables[0]
            .schema
            .as_deref()
            .expect("interface variable should carry generated schema");

        let board_normalized =
            infer_schema_from_json(generated).expect("board schema path should accept schema");
        let generated_json: Value =
            flow_like_types::json::from_str(generated).expect("generated schema json");
        let board_json: Value =
            flow_like_types::json::from_str(&board_normalized).expect("board schema json");

        assert_eq!(generated_json, board_json);
    }

    /// An event entry node's non-exec data outputs become the event's typed parameter list,
    /// ordered by pin index and camelCased.
    #[test]
    fn event_outputs_become_typed_params() {
        let mut board = empty_board();

        let mut event = Node::new("events_generic", "Now", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        event.add_output_pin("payload", "Payload", "", VariableType::Struct);
        event.add_output_pin("title", "Title", "", VariableType::String);
        board.nodes.insert(event.id.clone(), event);

        let ast = lower_to_ast(&board);

        assert_eq!(ast.events.len(), 1, "one event lowered");
        let ev = &ast.events[0];
        assert_eq!(ev.name, "eventsGeneric");
        assert_eq!(ev.node_type, "events_generic");
        assert_eq!(ev.event_name.as_deref(), Some("now"));
        let params: Vec<(&str, &str)> = ev
            .params
            .iter()
            .map(|p| (p.name.as_str(), p.ty.base.as_str()))
            .collect();
        assert_eq!(params, vec![("payload", "Struct"), ("title", "string")]);
    }

    /// A body node that reads an event payload pin resolves to the bare parameter name (not the
    /// `eventName.field` form), mirroring how function parameters resolve.
    #[test]
    fn event_param_resolves_to_bare_reference_in_body() {
        let mut board = empty_board();

        let mut event = Node::new("events_generic", "Now", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let exec_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let title = event
            .add_output_pin("title", "Title", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let exec_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let text = log
            .add_input_pin("text", "Text", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(log.id.clone(), log);

        connect(&mut board, "event", &exec_out, "log", &exec_in);
        connect(&mut board, "event", &title, "log", &text);

        let text_out = board_to_flowscript(&board, &RenderOptions::default());
        assert!(
            text_out.contains("eventsGeneric now(title: string)"),
            "event preserves its catalog type, alias, and payload param:\n{text_out}"
        );
        assert!(
            text_out.contains("log({ text: title })"),
            "body reads the bare param name:\n{text_out}"
        );
        assert!(
            !text_out.contains("now.title"),
            "body must not leak qualified payload ref:\n{text_out}"
        );
    }

    /// An unnamed entry still renders its exact catalog selector without inventing an alias.
    #[test]
    fn unnamed_event_renders_canonical_catalog_type() {
        let mut board = empty_board();

        let mut event = Node::new("events_widget_action", "", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let ast = lower_to_ast(&board);
        assert_eq!(ast.events[0].name, "eventsWidgetAction");
        assert_eq!(ast.events[0].node_type, "events_widget_action");
        assert_eq!(ast.events[0].event_name, None);

        let text = board_to_flowscript(&board, &RenderOptions::default());
        assert!(text.starts_with("eventsWidgetAction() {"), "{text}");
    }

    /// A friendly name occupies the optional alias slot instead of replacing catalog identity.
    #[test]
    fn named_event_renders_catalog_type_before_alias() {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Dashboard Load", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let text = board_to_flowscript(&board, &RenderOptions::default());
        assert!(text.starts_with("eventsSimple dashboardLoad() {"), "{text}");
    }

    /// `events_generic_return_result` sugars into a bare `return <response>` statement.
    #[test]
    fn return_result_node_sugars_to_return() {
        let mut board = empty_board();

        let mut event = Node::new("events_generic", "Now", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let exec_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut ret = Node::new(
            "events_generic_return_result",
            "Return Result",
            "",
            "events",
        );
        ret.id = "ret".to_string();
        ret.set_event_callback(true);
        let exec_in = ret
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        ret.add_input_pin("response", "Response", "", VariableType::String);
        board.nodes.insert(ret.id.clone(), ret);

        connect(&mut board, "event", &exec_out, "ret", &exec_in);

        let text_out = board_to_flowscript(&board, &RenderOptions::default());
        assert!(
            text_out.contains("return"),
            "return-result node renders as a return statement:\n{text_out}"
        );
        assert!(
            !text_out.contains("eventsGenericReturnResult"),
            "the raw call must not leak:\n{text_out}"
        );
    }

    #[test]
    fn legacy_layer_local_only_function_nodes_are_lowered() {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "function-layer".to_string(),
            "Legacy Helper".to_string(),
            LayerType::Function,
        );
        let mut body = Node::new("log", "Log", "", "debug");
        body.id = "legacy-body".to_string();
        body.add_input_pin("exec_in", "In", "", VariableType::Execution);
        body.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        layer.nodes.insert(body.id.clone(), body);
        board.layers.insert(layer.id.clone(), layer);

        let text = board_to_flowscript(
            &board,
            &RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );

        assert!(text.contains("function legacyHelper()"), "{text}");
        assert!(
            text.contains("log()"),
            "legacy function body was hidden:\n{text}"
        );
        assert!(text.contains("//@n:legacy-body"), "{text}");
    }

    #[test]
    fn canonical_flat_node_wins_over_stale_layer_local_clone() {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "function-layer".to_string(),
            "Helper".to_string(),
            LayerType::Function,
        );

        let mut canonical = Node::new("canonical_log", "Canonical", "", "debug");
        canonical.id = "shared-body".to_string();
        canonical.layer = Some(layer.id.clone());
        canonical.add_input_pin("exec_in", "In", "", VariableType::Execution);
        canonical.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        let mut stale = Node::new("stale_log", "Stale", "", "debug");
        stale.id = canonical.id.clone();
        stale.add_input_pin("exec_in", "In", "", VariableType::Execution);
        stale.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        board.nodes.insert(canonical.id.clone(), canonical);
        layer.nodes.insert(stale.id.clone(), stale);
        board.layers.insert(layer.id.clone(), layer);

        let text = board_to_flowscript(
            &board,
            &RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );

        assert!(text.contains("canonicalLog()"), "{text}");
        assert!(
            !text.contains("staleLog()"),
            "stale compatibility clone won:\n{text}"
        );
        assert_eq!(text.matches("//@n:shared-body").count(), 1, "{text}");
    }

    #[test]
    fn duplicate_legacy_identity_uses_the_first_layer_in_stable_id_order() {
        let mut board = empty_board();
        let mut first = Layer::new(
            "a-function".to_string(),
            "First Helper".to_string(),
            LayerType::Function,
        );
        let mut last = Layer::new(
            "z-function".to_string(),
            "Last Helper".to_string(),
            LayerType::Function,
        );

        let mut preferred = Node::new("preferred_log", "Preferred", "", "debug");
        preferred.id = "legacy-shared".to_string();
        preferred.add_input_pin("exec_in", "In", "", VariableType::Execution);
        preferred.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        let mut stale = Node::new("stale_log", "Stale", "", "debug");
        stale.id = preferred.id.clone();
        stale.add_input_pin("exec_in", "In", "", VariableType::Execution);
        stale.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        first.nodes.insert(preferred.id.clone(), preferred);
        last.nodes.insert(stale.id.clone(), stale);
        // Insert in reverse lexical order: selection must follow semantic ids, not insertion or
        // randomized HashMap iteration order.
        board.layers.insert(last.id.clone(), last);
        board.layers.insert(first.id.clone(), first);

        let text = board_to_flowscript(
            &board,
            &RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );

        assert!(text.contains("preferredLog()"), "{text}");
        assert!(
            !text.contains("staleLog()"),
            "unstable legacy fallback won:\n{text}"
        );
        assert_eq!(text.matches("//@n:legacy-shared").count(), 1, "{text}");
    }

    #[test]
    fn cyclic_presentational_layer_parents_do_not_hang_or_hide_root_events() {
        let mut board = empty_board();
        let mut first = Layer::new(
            "cycle-a".to_string(),
            "Cycle A".to_string(),
            LayerType::Macro,
        );
        let mut second = Layer::new(
            "cycle-b".to_string(),
            "Cycle B".to_string(),
            LayerType::Collapsed,
        );
        first.parent_id = Some(second.id.clone());
        second.parent_id = Some(first.id.clone());
        board.layers.insert(first.id.clone(), first);
        board.layers.insert(second.id.clone(), second);

        let mut event = Node::new("events_generic", "Cycle Event", "", "events");
        event.id = "cycle-event".to_string();
        event.layer = Some("cycle-a".to_string());
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let ast = lower_to_ast(&board);

        assert_eq!(ast.events.len(), 1);
        assert_eq!(ast.events[0].anchor.as_deref(), Some("cycle-event"));
        assert!(ast.functions.is_empty());
    }
}
