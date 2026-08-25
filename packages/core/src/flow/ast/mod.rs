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
    apply_flowscript_to_board_file, apply_flowscript_to_board_scoped,
    blocked_destructive_flowscript_message, destructive_flowscript_command_summaries,
    ensure_module_layer, validate_module_apply_params,
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
pub use lower::{
    FlowScriptFile, ScopedBoardAst, binary_operator_node_types, lower_board, lower_board_file,
    lower_board_scoped, pin_is_untouched_default,
};
pub use reconcile::{
    MAX_NODES_PER_LAYER, MetadataEnricher, ReconcileMode, ReconcileOptions, ReconcileResult,
    reconcile, reconcile_text, reconcile_text_with_catalog, reconcile_text_with_catalog_enriched,
    reconcile_text_with_catalog_enriched_opts, reconcile_text_with_catalog_enriched_scoped,
    reconcile_text_with_catalog_opts, reconcile_text_with_catalog_scoped, reconcile_with_catalog,
    reconcile_with_catalog_mode, reconcile_with_catalog_scoped,
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

/// A selection-scoped FlowScript render: the text of the kept sections plus the anchors a later
/// scoped apply/reconcile of that text must be limited to.
#[derive(Clone, serde::Serialize)]
pub struct ScopedFlowScript {
    pub text: String,
    /// Anchors (event entry node id / function layer id) of the rendered events/functions.
    pub scope_anchors: Vec<String>,
}

/// Render only the slice of `board` containing `node_ids`: every top-level event/function whose
/// body (nested handlers included) contains a selected node, every function such a section
/// references (transitively), and the full variable/interface context. Apply the edited text back
/// with `scope_anchors` so the reconciler never treats the unrendered rest as deleted.
pub fn board_to_flowscript_scoped(
    board: &Board,
    node_ids: &[String],
    opts: &RenderOptions,
) -> ScopedFlowScript {
    let scoped = lower::lower_board_scoped(board, node_ids);
    ScopedFlowScript {
        text: render(&scoped.ast, opts),
        scope_anchors: scoped.scope_anchors,
    }
}

/// Render one virtual FlowScript file of `board`: [`FlowScriptFile::Main`] for the sections owning
/// no module (plus the board's variables), or [`FlowScriptFile::Module`] for one module layer's own
/// file — its functions and events unwrapped, with no `module` block around them and none of its
/// nested modules, which are files of their own. Interfaces are kept in every file as type context.
/// Apply the edited text back with `scope_anchors` so the reconciler never treats the other files
/// as deleted.
pub fn board_to_flowscript_file(
    board: &Board,
    file: &FlowScriptFile,
    opts: &RenderOptions,
) -> flow_like_types::Result<ScopedFlowScript> {
    let scoped = lower::lower_board_file(board, file)?;
    Ok(ScopedFlowScript {
        text: render(&scoped.ast, opts),
        scope_anchors: scoped.scope_anchors,
    })
}

/// Canonically format FlowScript text: parse it, then re-render the AST. Pure text-domain — no
/// board or catalog involved — and stable because render(parse(render(x))) == render(x) (the
/// round-trip invariant). Anchors present in `text` survive the parse; `anchors: true` re-emits
/// them, `false` strips them from the output.
pub fn format_flowscript(text: &str, anchors: bool) -> Result<String, ParseError> {
    let ast = parse(text)?;
    Ok(render(
        &ast,
        &RenderOptions {
            anchors,
            ..RenderOptions::default()
        },
    ))
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
    pub(super) fn fixture_board(proto: flow_like_types::proto::Board) -> Board {
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

    pub(super) fn empty_board() -> Board {
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

    pub(super) fn connect(
        board: &mut Board,
        from_node: &str,
        from_pin: &str,
        to_node: &str,
        to_pin: &str,
    ) {
        crate::flow::board::commands::pins::connect_pins::connect_pins(
            board, from_node, from_pin, to_node, to_pin,
        )
        .expect("connect pins");
    }

    pub(super) fn exec_log(id: &str, layer: Option<&str>, message: &str) -> (Node, String, String) {
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
            Stmt::Let {
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
        let branch_end = text.find("    const rows = ").expect("top-level query");
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

    fn layer(id: &str, name: &str, kind: LayerType, parent: Option<&str>) -> Layer {
        let mut layer = Layer::new(id.to_string(), name.to_string(), kind);
        layer.parent_id = parent.map(str::to_string);
        layer
    }

    pub(super) fn add_layer(
        board: &mut Board,
        id: &str,
        name: &str,
        kind: LayerType,
        parent: Option<&str>,
    ) {
        let layer = layer(id, name, kind, parent);
        board.layers.insert(layer.id.clone(), layer);
    }

    pub(super) fn start_event(id: &str, layer: Option<&str>) -> (Node, String) {
        let mut event = Node::new("events_simple", "Run", "", "events");
        event.id = id.to_string();
        event.layer = layer.map(str::to_string);
        event.set_start(true);
        let exec_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        (event, exec_out)
    }

    /// A `control_call_function` node targeting `target_layer` through its stored pin default.
    pub(super) fn call_function(
        id: &str,
        layer: Option<&str>,
        target_layer: &str,
    ) -> (Node, String, String) {
        let mut node = Node::new("control_call_function", "Call Function", "", "control");
        node.id = id.to_string();
        node.layer = layer.map(str::to_string);
        let exec_in = node
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        node.add_input_pin("function_layer_id", "Function", "", VariableType::String)
            .set_default_value(Some(Value::String(target_layer.to_string())));
        let exec_out = node
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        (node, exec_in, exec_out)
    }

    pub(super) fn add_node(board: &mut Board, node: Node) {
        board.nodes.insert(node.id.clone(), node);
    }

    /// Event → one log, so the event owns a body statement. Returns the log's exec output pin id,
    /// the tail a test can chain further statements onto.
    pub(super) fn event_with_log(
        board: &mut Board,
        id: &str,
        layer: Option<&str>,
        message: &str,
    ) -> String {
        let (event, exec_out) = start_event(id, layer);
        add_node(board, event);
        let log_id = format!("{id}-log");
        let (log, log_in, log_out) = exec_log(&log_id, layer, message);
        add_node(board, log);
        connect(board, id, &exec_out, &log_id, &log_in);
        log_out
    }

    fn anchored(board: &Board) -> String {
        board_to_flowscript(
            board,
            &RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
    }

    /// Global function + global event, module `checkout` (event + local function) and the module
    /// `checkout::payments` nested inside it (function). Returns the board plus the exec tails of
    /// the root and module event bodies.
    pub(super) fn module_board() -> (Board, String, String) {
        let mut board = empty_board();
        add_layer(&mut board, "mod-m", "Checkout", LayerType::Module, None);
        add_layer(
            &mut board,
            "mod-p",
            "Payments",
            LayerType::Module,
            Some("mod-m"),
        );
        add_layer(
            &mut board,
            "fn-global",
            "Global Helper",
            LayerType::Function,
            None,
        );
        add_layer(
            &mut board,
            "fn-local",
            "Local Helper",
            LayerType::Function,
            Some("mod-m"),
        );
        add_layer(
            &mut board,
            "fn-nested",
            "Nested Helper",
            LayerType::Function,
            Some("mod-p"),
        );

        for (id, layer) in [
            ("global-body", "fn-global"),
            ("local-body", "fn-local"),
            ("nested-body", "fn-nested"),
        ] {
            let (log, ..) = exec_log(id, Some(layer), id);
            add_node(&mut board, log);
        }

        let root_tail = event_with_log(&mut board, "root-event", None, "root");
        let module_tail = event_with_log(&mut board, "module-event", Some("mod-m"), "in module");
        (board, root_tail, module_tail)
    }

    #[test]
    fn module_layers_partition_sections_into_nested_blocks() {
        let (board, ..) = module_board();
        let ast = lower_to_ast(&board);

        assert_eq!(
            ast.functions
                .iter()
                .map(|function| function.name.as_str())
                .collect::<Vec<_>>(),
            vec!["globalHelper"],
            "only a function with no module ancestor stays top-level"
        );
        assert_eq!(
            ast.events
                .iter()
                .filter_map(|event| event.anchor.as_deref())
                .collect::<Vec<_>>(),
            vec!["root-event"]
        );

        assert_eq!(ast.modules.len(), 1);
        let checkout = &ast.modules[0];
        assert_eq!(checkout.name, "checkout");
        assert_eq!(checkout.anchor.as_deref(), Some("mod-m"));
        assert_eq!(
            checkout
                .functions
                .iter()
                .map(|function| function.name.as_str())
                .collect::<Vec<_>>(),
            vec!["localHelper"]
        );
        assert_eq!(
            checkout
                .events
                .iter()
                .filter_map(|event| event.anchor.as_deref())
                .collect::<Vec<_>>(),
            vec!["module-event"]
        );

        assert_eq!(checkout.modules.len(), 1);
        let payments = &checkout.modules[0];
        assert_eq!(payments.name, "payments");
        assert_eq!(payments.anchor.as_deref(), Some("mod-p"));
        assert_eq!(
            payments
                .functions
                .iter()
                .map(|function| function.name.as_str())
                .collect::<Vec<_>>(),
            vec!["nestedHelper"]
        );
        assert!(payments.events.is_empty());
        assert!(payments.modules.is_empty());

        let text = anchored(&board);
        assert!(text.contains("function globalHelper()"), "{text}");
        assert!(text.contains("module checkout {   //@l:mod-m\n"), "{text}");
        assert!(
            text.contains("    module payments {   //@l:mod-p\n"),
            "nested module blocks indent one level:\n{text}"
        );
        assert!(
            text.contains("        function nestedHelper()"),
            "a function inside a nested module indents twice:\n{text}"
        );
        // The module blocks come after every top-level section.
        let root_function = text.find("function globalHelper").expect("global function");
        let module_block = text.find("module checkout").expect("module block");
        assert!(root_function < module_block, "{text}");

        assert_eq!(
            format_flowscript(&text, true).expect("a rendered module document must parse"),
            text,
            "module blocks must round-trip through parse/render unchanged"
        );
    }

    /// A `detached` chain is filed under the module its root node lives in, not hoisted to the
    /// root — the block carries no anchor of its own, so ownership travels with the chain.
    #[test]
    fn detached_chains_stay_in_their_own_module_block() {
        let (mut board, ..) = module_board();
        let (orphan, ..) = exec_log("module-orphan", Some("mod-m"), "unreachable");
        add_node(&mut board, orphan);

        let ast = lower_to_ast(&board);
        assert!(ast.detached.is_empty(), "the chain belongs to the module");
        assert_eq!(ast.modules[0].detached.len(), 1);
        assert_eq!(
            ast.modules[0].detached[0].root_anchor(),
            Some("module-orphan")
        );

        let text = anchored(&board);
        assert!(
            text.contains("    detached {\n"),
            "a module's detached block indents one level:\n{text}"
        );
        assert_eq!(
            format_flowscript(&text, true).expect("a rendered detached block must parse"),
            text,
            "detached blocks must round-trip through parse/render unchanged"
        );

        let catalog: Vec<_> = board
            .nodes
            .values()
            .map(crate::flow::copilot::node_to_metadata)
            .collect();
        let result = reconcile_text_with_catalog(&board, &text, &catalog);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.is_empty(), "{:?}", result.commands);
    }

    #[test]
    fn cross_module_calls_render_the_full_path_from_the_root_module() {
        let (mut board, root_tail, module_tail) = module_board();

        // The module's own event calls into the module NESTED below it: a parent→child call is
        // still cross-module, so it spells the whole path rather than the relative `payments::`.
        let (call, call_in, _) = call_function("call-down", Some("mod-m"), "fn-nested");
        add_node(&mut board, call);
        connect(
            &mut board,
            "module-event-log",
            &module_tail,
            "call-down",
            &call_in,
        );

        // The root event calls the same nested function.
        let (root_call, root_call_in, _) = call_function("call-root", None, "fn-nested");
        add_node(&mut board, root_call);
        connect(
            &mut board,
            "root-event-log",
            &root_tail,
            "call-root",
            &root_call_in,
        );

        let text = anchored(&board);
        assert_eq!(
            text.matches("checkout::payments::nestedHelper()").count(),
            2,
            "a cross-module call always spells the full path from the root module:\n{text}"
        );
        assert_eq!(
            format_flowscript(&text, true).expect("a qualified call must parse"),
            text,
            "qualified cross-module calls must round-trip through parse/render unchanged"
        );
    }

    #[test]
    fn same_module_and_global_targets_stay_bare() {
        let (mut board, _, module_tail) = module_board();

        // Module event → local function of the same module, then the global function.
        let (same, same_in, same_out) = call_function("call-same", Some("mod-m"), "fn-local");
        add_node(&mut board, same);
        let (global, global_in, _) = call_function("call-global", Some("mod-m"), "fn-global");
        add_node(&mut board, global);
        connect(
            &mut board,
            "module-event-log",
            &module_tail,
            "call-same",
            &same_in,
        );
        connect(
            &mut board,
            "call-same",
            &same_out,
            "call-global",
            &global_in,
        );

        let text = anchored(&board);
        assert!(
            text.contains("localHelper()") && !text.contains("checkout::localHelper()"),
            "a call inside the owning module stays bare:\n{text}"
        );
        assert!(
            text.contains("globalHelper()") && !text.contains("::globalHelper()"),
            "a global function is bare from anywhere:\n{text}"
        );
    }

    #[test]
    fn a_collapsed_layer_under_a_module_lowers_into_that_module() {
        let mut board = empty_board();
        add_layer(&mut board, "mod-m", "Checkout", LayerType::Module, None);
        add_layer(
            &mut board,
            "collapsed",
            "Grouped",
            LayerType::Collapsed,
            Some("mod-m"),
        );
        let _ = event_with_log(&mut board, "grouped-event", Some("collapsed"), "grouped");

        let ast = lower_to_ast(&board);
        assert!(
            ast.events.is_empty(),
            "a collapsed layer is transparent: its event belongs to the module"
        );
        assert_eq!(ast.modules.len(), 1);
        assert_eq!(
            ast.modules[0]
                .events
                .iter()
                .filter_map(|event| event.anchor.as_deref())
                .collect::<Vec<_>>(),
            vec!["grouped-event"]
        );
    }

    #[test]
    fn use_derivation_counts_call_sites_inside_module_blocks() {
        let mut board = empty_board();
        add_layer(&mut board, "mod-m", "Checkout", LayerType::Module, None);
        let (event, exec_out) = start_event("module-event", Some("mod-m"));
        add_node(&mut board, event);
        let (first, first_in, first_out) = exec_log("first", Some("mod-m"), "one");
        add_node(&mut board, first);
        let (second, second_in, _) = exec_log("second", Some("mod-m"), "two");
        add_node(&mut board, second);
        connect(&mut board, "module-event", &exec_out, "first", &first_in);
        connect(&mut board, "first", &first_out, "second", &second_in);

        let text = board_to_flowscript(&board, &RenderOptions::default());
        assert!(
            text.starts_with("use debug::*\n"),
            "two static sites inside a module block still open the namespace:\n{text}"
        );
        assert!(
            text.contains("        logInfo({ message: \"one\" })"),
            "the opened members render bare inside the module block:\n{text}"
        );
    }

    #[test]
    fn an_empty_module_still_renders_its_block() {
        let mut board = empty_board();
        add_layer(&mut board, "mod-m", "Checkout", LayerType::Module, None);
        add_layer(
            &mut board,
            "mod-p",
            "Payments",
            LayerType::Module,
            Some("mod-m"),
        );

        let ast = lower_to_ast(&board);
        assert_eq!(ast.modules.len(), 1);
        assert_eq!(ast.modules[0].modules.len(), 1);

        let text = anchored(&board);
        assert_eq!(
            text, "module checkout {   //@l:mod-m\n    module payments {   //@l:mod-p\n    }\n}\n",
            "an empty module keeps its block so the board's organization survives:\n{text}"
        );
    }

    #[test]
    fn a_scoped_render_keeps_only_the_selected_module_section() {
        let (board, ..) = module_board();
        let scoped = board_to_flowscript_scoped(
            &board,
            &["module-event-log".to_string()],
            &RenderOptions::default(),
        );

        assert!(scoped.text.contains("module checkout {"), "{}", scoped.text);
        assert!(
            !scoped.text.contains("module payments"),
            "a module left with nothing selected is dropped:\n{}",
            scoped.text
        );
        assert!(
            !scoped.text.contains("globalHelper") && !scoped.text.contains("localHelper"),
            "{}",
            scoped.text
        );
        assert_eq!(scoped.scope_anchors, vec!["module-event".to_string()]);
    }

    #[test]
    fn a_scoped_render_follows_a_qualified_cross_module_reference() {
        let (mut board, root_tail, _) = module_board();
        let (root_call, root_call_in, _) = call_function("call-root", None, "fn-nested");
        add_node(&mut board, root_call);
        connect(
            &mut board,
            "root-event-log",
            &root_tail,
            "call-root",
            &root_call_in,
        );

        let scoped = board_to_flowscript_scoped(
            &board,
            &["call-root".to_string()],
            &RenderOptions::default(),
        );

        assert!(
            scoped.text.contains("checkout::payments::nestedHelper()"),
            "{}",
            scoped.text
        );
        assert!(
            scoped.text.contains("function nestedHelper()"),
            "a qualified reference must keep its target declared:\n{}",
            scoped.text
        );
        assert!(
            scoped.scope_anchors.contains(&"fn-nested".to_string())
                && scoped.scope_anchors.contains(&"root-event".to_string()),
            "{:?}",
            scoped.scope_anchors
        );
        assert!(!scoped.text.contains("localHelper"), "{}", scoped.text);
    }

    /// Render one virtual file with anchors on, so the assertions can name the kept sections.
    fn file_text(board: &Board, file: FlowScriptFile) -> ScopedFlowScript {
        board_to_flowscript_file(
            board,
            &file,
            &RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
        .expect("a valid file must render")
    }

    /// A struct variable: a `const` in `main.flow` plus an `interface` every file keeps as the
    /// shared type context.
    fn add_record_variable(board: &mut Board) {
        use crate::flow::pin::ValueType;
        use crate::flow::variable::Variable;
        let mut variable = Variable::new("record", VariableType::Struct, ValueType::Normal);
        variable.id = "var-record".to_string();
        variable.schema = Some(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"Record","type":"object","properties":{"name":{"type":"string"}},"required":["name"]}"#
                .to_string(),
        );
        variable.set_default_value(Value::Object(Default::default()));
        board.variables.insert(variable.id.clone(), variable);
    }

    #[test]
    fn the_main_file_holds_only_the_sections_owning_no_module() {
        let (mut board, ..) = module_board();
        add_record_variable(&mut board);

        let main = file_text(&board, FlowScriptFile::Main);

        assert!(
            main.text.contains("function globalHelper()") && main.text.contains("\"root\""),
            "the root function and event stay in main.flow:\n{}",
            main.text
        );
        assert!(
            !main.text.contains("module "),
            "modules are files of their own, never blocks of main.flow:\n{}",
            main.text
        );
        assert!(
            !main.text.contains("localHelper")
                && !main.text.contains("nestedHelper")
                && !main.text.contains("in module"),
            "no module content leaks into main.flow:\n{}",
            main.text
        );
        assert!(
            main.text.contains("interface Record") && main.text.contains("const record"),
            "main.flow owns the board's variables and the shared interfaces:\n{}",
            main.text
        );
        assert_eq!(
            main.scope_anchors,
            vec!["fn-global".to_string(), "root-event".to_string()]
        );
    }

    #[test]
    fn a_module_file_is_the_module_unwrapped() {
        let (mut board, ..) = module_board();
        add_record_variable(&mut board);

        let module = file_text(&board, FlowScriptFile::Module("mod-m".to_string()));

        assert!(
            module.text.contains("function localHelper()") && module.text.contains("\"in module\""),
            "the module's own sections sit at the top level:\n{}",
            module.text
        );
        assert!(
            !module.text.contains("module "),
            "the file IS the module — no wrapper, and no block for the nested file:\n{}",
            module.text
        );
        assert!(
            !module.text.contains("nestedHelper") && !module.text.contains("globalHelper"),
            "neither the nested module's nor the root's sections belong here:\n{}",
            module.text
        );
        assert!(
            !module.text.contains("const record"),
            "board variables are globals declared once, in main.flow:\n{}",
            module.text
        );
        assert!(
            module.text.contains("interface Record"),
            "interfaces are pure type context and stay in every file:\n{}",
            module.text
        );
        assert_eq!(
            module.scope_anchors,
            vec!["fn-local".to_string(), "module-event".to_string()]
        );
        assert_eq!(
            format_flowscript(&module.text, true).expect("a module file must parse"),
            module.text,
            "a hoisted module file must round-trip through parse/render unchanged"
        );
    }

    #[test]
    fn a_module_file_keeps_call_paths_of_its_own_scope_without_declaring_the_targets() {
        let (mut board, _, module_tail) = module_board();

        // The module event calls the global function, then the nested module's function.
        let (global, global_in, global_out) =
            call_function("call-global", Some("mod-m"), "fn-global");
        add_node(&mut board, global);
        let (down, down_in, _) = call_function("call-down", Some("mod-m"), "fn-nested");
        add_node(&mut board, down);
        connect(
            &mut board,
            "module-event-log",
            &module_tail,
            "call-global",
            &global_in,
        );
        connect(
            &mut board,
            "call-global",
            &global_out,
            "call-down",
            &down_in,
        );

        let module = file_text(&board, FlowScriptFile::Module("mod-m".to_string()));

        assert!(
            module.text.contains("globalHelper()") && !module.text.contains("::globalHelper()"),
            "a global target is bare from the module file:\n{}",
            module.text
        );
        assert!(
            module.text.contains("checkout::payments::nestedHelper()"),
            "a cross-module target keeps the full path from the root module:\n{}",
            module.text
        );
        assert!(
            !module.text.contains("function globalHelper")
                && !module.text.contains("function nestedHelper"),
            "a call never pulls another file's declaration in:\n{}",
            module.text
        );
    }

    #[test]
    fn a_nested_module_is_its_own_file() {
        let (board, ..) = module_board();

        let nested = file_text(&board, FlowScriptFile::Module("mod-p".to_string()));

        assert!(
            nested.text.contains("function nestedHelper()") && !nested.text.contains("module "),
            "the nested module's file is just its own sections:\n{}",
            nested.text
        );
        assert_eq!(nested.scope_anchors, vec!["fn-nested".to_string()]);
    }

    #[test]
    fn an_empty_module_file_is_empty() {
        let mut board = empty_board();
        add_layer(&mut board, "mod-m", "Checkout", LayerType::Module, None);
        add_layer(
            &mut board,
            "mod-p",
            "Payments",
            LayerType::Module,
            Some("mod-m"),
        );

        let module = file_text(&board, FlowScriptFile::Module("mod-m".to_string()));
        assert_eq!(
            module.text, "",
            "an empty module's file has nothing in it — its child module is a file of its own"
        );
        assert!(module.scope_anchors.is_empty());
    }

    #[test]
    fn only_a_module_layer_names_a_file() {
        let (board, ..) = module_board();

        assert!(
            board_to_flowscript_file(
                &board,
                &FlowScriptFile::Module("nope".to_string()),
                &RenderOptions::default(),
            )
            .is_err(),
            "an unknown layer id is not a file"
        );
        assert!(
            board_to_flowscript_file(
                &board,
                &FlowScriptFile::Module("fn-global".to_string()),
                &RenderOptions::default(),
            )
            .is_err(),
            "a function layer is not a file"
        );
    }

    #[test]
    fn a_board_without_modules_renders_its_whole_document_as_main() {
        let board = fetch_body_board(None);
        let main =
            board_to_flowscript_file(&board, &FlowScriptFile::Main, &RenderOptions::default())
                .expect("main always renders");

        assert_eq!(
            main.text,
            board_to_flowscript(&board, &RenderOptions::default()),
            "with no module layer, main.flow IS the full document"
        );
        assert_eq!(main.scope_anchors, vec!["event".to_string()]);
    }

    /// event → fetch → log, with `fetch.body` feeding the log message: lowering mints a local
    /// named after the `body` output pin. `module_name` adds a root module of that name.
    fn fetch_body_board(module_name: Option<&str>) -> Board {
        let mut board = empty_board();
        if let Some(name) = module_name {
            add_layer(&mut board, "mod-m", name, LayerType::Module, None);
        }

        let (event, exec_out) = start_event("event", None);
        add_node(&mut board, event);

        let mut fetch = Node::new("http_fetch", "Fetch", "", "web");
        fetch.id = "fetch".to_string();
        let fetch_in = fetch
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let fetch_out = fetch
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let body = fetch
            .add_output_pin("body", "Body", "", VariableType::String)
            .id
            .clone();
        add_node(&mut board, fetch);

        let (log, log_in, _) = exec_log("log", None, "hello");
        add_node(&mut board, log);
        let message = board.nodes["log"]
            .pins
            .values()
            .find(|pin| pin.name == "message")
            .expect("message pin")
            .id
            .clone();

        connect(&mut board, "event", &exec_out, "fetch", &fetch_in);
        connect(&mut board, "fetch", &fetch_out, "log", &log_in);
        connect(&mut board, "fetch", &body, "log", &message);
        board
    }

    #[test]
    fn module_roots_are_never_minted_as_binding_names() {
        let baseline = board_to_flowscript(&fetch_body_board(None), &RenderOptions::default());
        assert!(
            baseline.contains("const body ="),
            "control: without the module the pin name is free:\n{baseline}"
        );

        // A module named exactly like the local lowering would otherwise mint for `fetch.body`.
        let text = board_to_flowscript(&fetch_body_board(Some("Body")), &RenderOptions::default());
        assert!(
            text.contains("const body2 ="),
            "a minted binding must not shadow a module namespace root:\n{text}"
        );
    }
}

#[cfg(test)]
mod module_reconcile_tests {
    use super::lower_tests::{
        add_layer, add_node, call_function, connect, empty_board, event_with_log, exec_log,
        start_event,
    };
    use super::*;
    use crate::flow::board::{Board, LayerType};
    use crate::flow::copilot::{BoardCommand, NodeMetadata, node_to_metadata};
    use crate::flow::node::Node;
    use crate::flow::variable::VariableType;
    use flow_like_types::Value;

    /// A Function layer shaped the way an applied `function` is: an execution boundary whose
    /// `exec_in` drives one impure body statement and whose `exec_out` is driven by its tail.
    fn add_function(board: &mut Board, id: &str, name: &str, parent: Option<&str>, message: &str) {
        use crate::flow::board::Layer;
        let mut layer = Layer::new(id.to_string(), name.to_string(), LayerType::Function);
        layer.parent_id = parent.map(str::to_string);
        let mut template = Node::new("boundary", "Boundary", "", "test");
        let exec_in = template
            .add_input_pin("exec_in", "Exec In", "", VariableType::Execution)
            .clone();
        let exec_out = template
            .add_output_pin("exec_out", "Exec Out", "", VariableType::Execution)
            .clone();
        layer.pins.insert(exec_in.id.clone(), exec_in.clone());
        layer.pins.insert(exec_out.id.clone(), exec_out.clone());
        board.layers.insert(layer.id.clone(), layer);

        let body = format!("{id}-body");
        let (log, log_in, log_out) = exec_log(&body, Some(id), message);
        add_node(board, log);
        connect(board, id, &exec_in.id, &body, &log_in);
        connect(board, &body, &log_out, id, &exec_out.id);
    }

    /// `module_board`'s shape with reconcile-valid function layers: a global function and event,
    /// module `checkout` (event + local function) and `checkout::payments` nested in it.
    fn module_test_board() -> Board {
        let mut board = empty_board();
        add_layer(&mut board, "mod-m", "Checkout", LayerType::Module, None);
        add_layer(
            &mut board,
            "mod-p",
            "Payments",
            LayerType::Module,
            Some("mod-m"),
        );
        add_function(&mut board, "fn-global", "Global Helper", None, "global");
        add_function(
            &mut board,
            "fn-local",
            "Local Helper",
            Some("mod-m"),
            "local",
        );
        add_function(
            &mut board,
            "fn-nested",
            "Nested Helper",
            Some("mod-p"),
            "nested",
        );
        event_with_log(&mut board, "root-event", None, "root");
        event_with_log(&mut board, "module-event", Some("mod-m"), "in module");
        board
    }

    fn anchored() -> RenderOptions {
        RenderOptions {
            anchors: true,
            ..Default::default()
        }
    }

    /// One catalog entry per node type the board places, plus the `control_call_function` node a
    /// new call needs. Instance defaults are dropped so the entries are pure type declarations.
    fn board_catalog(board: &Board) -> Vec<NodeMetadata> {
        let (call, ..) = call_function("catalog-call", None, "");
        let mut seen = std::collections::HashSet::new();
        let mut catalog = board
            .nodes
            .values()
            .chain(std::iter::once(&call))
            .filter(|node| seen.insert(node.name.clone()))
            .map(|node| {
                let mut meta = node_to_metadata(node);
                for pin in &mut meta.inputs {
                    pin.default_value = None;
                }
                meta
            })
            .collect::<Vec<_>>();
        catalog.sort_by(|left, right| left.name.cmp(&right.name));
        catalog
    }

    fn file(board: &Board, file: FlowScriptFile) -> ScopedFlowScript {
        board_to_flowscript_file(board, &file, &anchored()).expect("a valid file must render")
    }

    fn reconcile_file(
        board: &Board,
        text: &str,
        scope_anchors: &[String],
        file: FlowScriptFile,
    ) -> ReconcileResult {
        reconcile_text_with_catalog_opts(
            board,
            text,
            &board_catalog(board),
            &ReconcileOptions {
                scope_anchors: Some(scope_anchors),
                file: Some(file),
                ..Default::default()
            },
        )
    }

    /// Append a statement to the LAST block of a rendered document.
    fn append_to_last_block(text: &str, statement: &str) -> String {
        let index = text.rfind("\n}").expect("a block to extend");
        format!("{}\n    {statement}{}", &text[..index], &text[index..])
    }

    fn added_node_types(result: &ReconcileResult) -> Vec<&str> {
        result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::AddNode { node_type, .. } => Some(node_type.as_str()),
                _ => None,
            })
            .collect()
    }

    fn created_layers(result: &ReconcileResult) -> Vec<(&str, Option<&str>, Option<&str>)> {
        result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::CreateLayer {
                    name,
                    layer_type,
                    target_layer,
                    ..
                } => Some((
                    name.as_str(),
                    layer_type.as_deref(),
                    target_layer.as_deref(),
                )),
                _ => None,
            })
            .collect()
    }

    fn assert_noop(result: &ReconcileResult, what: &str) {
        assert!(
            result.diagnostics.is_empty(),
            "{what} must not diagnose: {:?}",
            result.diagnostics
        );
        assert!(
            result.commands.is_empty(),
            "{what} must be a no-op: {:?}",
            result.commands
        );
    }

    #[test]
    fn a_full_document_of_a_module_board_round_trips_without_commands() {
        let board = module_test_board();
        let text = board_to_flowscript(&board, &anchored());
        let result = reconcile_text_with_catalog(&board, &text, &board_catalog(&board));
        assert_noop(&result, "a full document with nested modules");
    }

    #[test]
    fn every_virtual_file_of_a_module_board_round_trips_without_commands() {
        let board = module_test_board();
        for target in [
            FlowScriptFile::Main,
            FlowScriptFile::Module("mod-m".to_string()),
            FlowScriptFile::Module("mod-p".to_string()),
        ] {
            let rendered = file(&board, target.clone());
            let result = reconcile_file(&board, &rendered.text, &rendered.scope_anchors, target);
            assert_noop(&result, &format!("file {:?}", rendered.scope_anchors));
        }
    }

    #[test]
    fn a_selection_scoped_render_of_a_module_board_round_trips_without_commands() {
        let board = module_test_board();
        // The selection path renders module blocks inline and carries no file, so the blocks
        // themselves have to map back onto the board's module tree.
        let scoped =
            board_to_flowscript_scoped(&board, &["module-event-log".to_string()], &anchored());
        assert!(scoped.text.contains("module checkout {"), "{}", scoped.text);

        let result = reconcile_text_with_catalog_scoped(
            &board,
            &scoped.text,
            &board_catalog(&board),
            Some(&scoped.scope_anchors),
        );
        assert_noop(&result, "a selection-scoped render of a module board");
    }

    #[test]
    fn an_empty_module_file_is_never_a_delete_everything() {
        let mut board = module_test_board();
        add_layer(&mut board, "mod-empty", "Empty", LayerType::Module, None);

        let rendered = file(&board, FlowScriptFile::Module("mod-empty".to_string()));
        assert_eq!(rendered.text, "");
        let result = reconcile_file(
            &board,
            &rendered.text,
            &rendered.scope_anchors,
            FlowScriptFile::Module("mod-empty".to_string()),
        );
        assert_noop(&result, "an empty module file");
    }

    #[test]
    fn a_module_file_claims_its_own_events_entry_and_never_mains() {
        let mut board = module_test_board();
        // Both events drive the module's first body node, so ONLY the module context tells the
        // two candidate entries apart.
        let root_out = board.nodes["root-event"]
            .pins
            .values()
            .find(|pin| pin.name == "exec_out")
            .expect("event output")
            .id
            .clone();
        let log_in = board.nodes["module-event-log"]
            .pins
            .values()
            .find(|pin| pin.name == "exec_in")
            .expect("log input")
            .id
            .clone();
        connect(
            &mut board,
            "root-event",
            &root_out,
            "module-event-log",
            &log_in,
        );

        let rendered = file(&board, FlowScriptFile::Module("mod-m".to_string()));
        let text = rendered.text.replace("   //@n:module-event\n", "\n");
        assert!(!text.contains("//@n:module-event\n"), "{text}");

        let result = reconcile_file(
            &board,
            &text,
            &rendered.scope_anchors,
            FlowScriptFile::Module("mod-m".to_string()),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            added_node_types(&result).is_empty(),
            "the module's own live entry is rebound instead of duplicated: {:?}",
            result.commands
        );
        assert!(
            result
                .corrections
                .iter()
                .any(|correction| correction.contains("module-event")),
            "{:?}",
            result.corrections
        );
    }

    #[test]
    fn a_module_file_calls_an_undeclared_global_function() {
        let board = module_test_board();
        let rendered = file(&board, FlowScriptFile::Module("mod-m".to_string()));
        let text = append_to_last_block(&rendered.text, "globalHelper()");

        let result = reconcile_file(
            &board,
            &text,
            &rendered.scope_anchors,
            FlowScriptFile::Module("mod-m".to_string()),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(added_node_types(&result), vec!["control_call_function"]);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { pin_id, value, .. }
                    if pin_id == "function_layer_id"
                        && value == &Value::String("fn-global".to_string())
            )),
            "the call must target the existing global layer: {:?}",
            result.commands
        );
    }

    #[test]
    fn main_reaches_a_nested_module_function_through_its_absolute_path() {
        let board = module_test_board();
        let rendered = file(&board, FlowScriptFile::Main);
        let text = append_to_last_block(&rendered.text, "checkout::payments::nestedHelper()");

        let result = reconcile_file(&board, &text, &rendered.scope_anchors, FlowScriptFile::Main);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(added_node_types(&result), vec!["control_call_function"]);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { pin_id, value, .. }
                    if pin_id == "function_layer_id"
                        && value == &Value::String("fn-nested".to_string())
            )),
            "{:?}",
            result.commands
        );
    }

    #[test]
    fn a_bare_name_never_crosses_a_module_boundary() {
        let board = module_test_board();
        let rendered = file(&board, FlowScriptFile::Main);
        let text = append_to_last_block(&rendered.text, "nestedHelper()");

        let result = reconcile_file(&board, &text, &rendered.scope_anchors, FlowScriptFile::Main);

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("checkout::payments::nestedHelper")),
            "the diagnostic must name the qualified spelling: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn a_path_that_is_both_a_module_and_a_catalog_namespace_fails_closed() {
        let mut board = empty_board();
        add_layer(&mut board, "mod-string", "String", LayerType::Module, None);
        add_function(&mut board, "fn-trim", "Trim", Some("mod-string"), "trim");
        let (event, exec_out) = start_event("root-event", None);
        add_node(&mut board, event);
        let (log, log_in, _) = exec_log("root-log", None, "root");
        add_node(&mut board, log);
        connect(&mut board, "root-event", &exec_out, "root-log", &log_in);

        let mut trim = Node::new("string_trim", "Trim String", "", "Utils/String");
        trim.set_flowscript_name("string", "trim");
        trim.add_input_pin("string", "String", "", VariableType::String);
        trim.add_output_pin("result", "Result", "", VariableType::String);
        let mut catalog = board_catalog(&board);
        catalog.push(node_to_metadata(&trim));

        let rendered = file(&board, FlowScriptFile::Main);
        let text = append_to_last_block(&rendered.text, "string::trim({ string: \"a\" })");
        let result = reconcile_text_with_catalog_opts(
            &board,
            &text,
            &catalog,
            &ReconcileOptions {
                scope_anchors: Some(&rendered.scope_anchors),
                file: Some(FlowScriptFile::Main),
                ..Default::default()
            },
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("ambiguous")
                    && diagnostic.contains("module `string`")
                    && diagnostic.contains("string_trim")
            }),
            "a path that names both a module and a catalog namespace must fail closed: {:?}",
            result.diagnostics
        );
        assert!(result.commands.is_empty(), "{:?}", result.commands);
    }

    #[test]
    fn a_new_function_in_a_module_file_is_created_inside_that_module() {
        let board = module_test_board();
        let rendered = file(&board, FlowScriptFile::Module("mod-m".to_string()));
        let text = format!(
            "{}\nfunction newHelper() {{\n    logInfo({{ message: \"fresh\" }})\n}}\n",
            rendered.text
        );

        let result = reconcile_file(
            &board,
            &text,
            &rendered.scope_anchors,
            FlowScriptFile::Module("mod-m".to_string()),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            created_layers(&result),
            vec![("newHelper", Some("Function"), Some("mod-m"))]
        );
    }

    #[test]
    fn a_nested_module_block_in_a_module_file_lands_one_level_down() {
        let board = module_test_board();
        let rendered = file(&board, FlowScriptFile::Module("mod-m".to_string()));
        let text = format!(
            "{}\nmodule inner {{\n    function innerHelper() {{\n        logInfo({{ message: \"inner\" }})\n    }}\n}}\n",
            rendered.text
        );

        let result = reconcile_file(
            &board,
            &text,
            &rendered.scope_anchors,
            FlowScriptFile::Module("mod-m".to_string()),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let layers = created_layers(&result);
        assert_eq!(layers.len(), 2, "{layers:?}");
        assert_eq!(layers[0], ("inner", Some("Module"), Some("mod-m")));
        assert_eq!(layers[1].0, "innerHelper");
        assert_eq!(layers[1].1, Some("Function"));
        assert_eq!(
            layers[1].2,
            Some("$0"),
            "the function is parented to the module created in this same batch"
        );
    }

    #[test]
    fn introducing_a_module_local_shadow_of_a_global_function_is_rejected() {
        let board = module_test_board();
        let rendered = file(&board, FlowScriptFile::Module("mod-m".to_string()));
        let text = format!(
            "{}\nfunction globalHelper() {{\n    logInfo({{ message: \"shadow\" }})\n}}\n",
            rendered.text
        );

        let result = reconcile_file(
            &board,
            &text,
            &rendered.scope_anchors,
            FlowScriptFile::Module("mod-m".to_string()),
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("shadows")),
            "{:?}",
            result.diagnostics
        );
    }

    #[test]
    fn a_shadow_the_board_already_has_still_round_trips() {
        let mut board = module_test_board();
        add_function(
            &mut board,
            "fn-shadow",
            "Global Helper",
            Some("mod-m"),
            "shadow",
        );

        let text = board_to_flowscript(&board, &anchored());
        let result = reconcile_text_with_catalog(&board, &text, &board_catalog(&board));
        assert_noop(&result, "a pre-existing shadow");
    }

    #[test]
    fn two_modules_may_each_own_a_function_of_the_same_name() {
        let mut board = empty_board();
        add_layer(&mut board, "mod-a", "Alpha", LayerType::Module, None);
        add_layer(&mut board, "mod-b", "Beta", LayerType::Module, None);
        for (module, layer, message) in [
            ("mod-a", "fn-a", "alpha helper"),
            ("mod-b", "fn-b", "beta helper"),
        ] {
            add_function(&mut board, layer, "Helper", Some(module), message);
        }
        let (event, exec_out) = start_event("root-event", None);
        add_node(&mut board, event);
        let (log, log_in, _) = exec_log("root-log", None, "root");
        add_node(&mut board, log);
        connect(&mut board, "root-event", &exec_out, "root-log", &log_in);

        assert_noop(
            &reconcile_text_with_catalog(
                &board,
                &board_to_flowscript(&board, &anchored()),
                &board_catalog(&board),
            ),
            "same-named functions in two modules",
        );

        // Module blocks render last, so exercise the call sites from `main.flow` — the file whose
        // last block IS the root event.
        let main = file(&board, FlowScriptFile::Main);
        for (path, layer) in [("alpha::helper", "fn-a"), ("beta::helper", "fn-b")] {
            let called = append_to_last_block(&main.text, &format!("{path}()"));
            let result = reconcile_text_with_catalog(&board, &called, &board_catalog(&board));
            assert!(
                result.diagnostics.is_empty(),
                "{path}: {:?}",
                result.diagnostics
            );
            assert!(
                result.commands.iter().any(|command| matches!(
                    command,
                    BoardCommand::UpdateNodePin { pin_id, value, .. }
                        if pin_id == "function_layer_id"
                            && value == &Value::String(layer.to_string())
                )),
                "{path} must resolve to {layer}: {:?}",
                result.commands
            );
        }
    }

    #[test]
    fn a_module_file_never_removes_a_board_variable_and_may_declare_one() {
        use crate::flow::pin::ValueType;
        use crate::flow::variable::Variable;
        let mut board = module_test_board();
        let mut kept = Variable::new("kept", VariableType::String, ValueType::Normal);
        kept.id = "var-kept".to_string();
        kept.set_default_value(Value::String("keep me".to_string()));
        board.variables.insert(kept.id.clone(), kept);

        let rendered = file(&board, FlowScriptFile::Module("mod-m".to_string()));
        assert!(!rendered.text.contains("kept"), "{}", rendered.text);
        let text = format!("const declaredHere: string = \"a\"\n\n{}", rendered.text);

        let result = reconcile_file(
            &board,
            &text,
            &rendered.scope_anchors,
            FlowScriptFile::Module("mod-m".to_string()),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            !result
                .commands
                .iter()
                .any(|command| matches!(command, BoardCommand::RemoveVariable { .. })),
            "a module file declares no variables, so it can never delete one: {:?}",
            result.commands
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::CreateVariable { name, .. } if name == "declaredHere"
            )),
            "{:?}",
            result.commands
        );
    }

    #[test]
    fn a_variable_declared_in_a_module_file_renders_in_main_afterwards() {
        use crate::flow::pin::ValueType;
        use crate::flow::variable::Variable;
        let mut board = module_test_board();
        let mut declared = Variable::new("declaredHere", VariableType::String, ValueType::Normal);
        declared.id = "var-declared".to_string();
        declared.set_default_value(Value::String("a".to_string()));
        board.variables.insert(declared.id.clone(), declared);

        let main = file(&board, FlowScriptFile::Main);
        assert!(
            main.text.contains("declaredHere"),
            "a board global always renders in main.flow:\n{}",
            main.text
        );
        let module = file(&board, FlowScriptFile::Module("mod-m".to_string()));
        assert!(!module.text.contains("declaredHere"), "{}", module.text);
    }

    #[test]
    fn a_module_is_a_file_and_therefore_exempt_from_the_layer_node_cap() {
        let mut board = module_test_board();
        let mut tail_node = "module-event-log".to_string();
        let mut tail_pin = board.nodes["module-event-log"]
            .pins
            .values()
            .find(|pin| pin.name == "exec_out")
            .expect("log tail")
            .id
            .clone();
        for index in 0..MAX_NODES_PER_LAYER + 5 {
            let id = format!("filler-{index}");
            let (log, log_in, log_out) = exec_log(&id, Some("mod-m"), &id);
            add_node(&mut board, log);
            connect(&mut board, &tail_node, &tail_pin, &id, &log_in);
            tail_node = id;
            tail_pin = log_out;
        }

        let rendered = file(&board, FlowScriptFile::Module("mod-m".to_string()));
        let text = append_to_last_block(&rendered.text, "globalHelper()");
        let result = reconcile_file(
            &board,
            &text,
            &rendered.scope_anchors,
            FlowScriptFile::Module("mod-m".to_string()),
        );

        assert!(
            !result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("nodes (max")),
            "a module holds a whole file's worth of nodes: {:?}",
            result.diagnostics
        );
    }
}

#[cfg(test)]
mod format_flowscript_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn format_canonicalizes_sugar_and_preserves_comments_and_anchors() {
        let source = "eventsGeneric run(name: string) {   //@n:entry\n    // keep this comment\n    let x: int = 1\n    x += 1\n    log({ message: 'a' })   //@n:log-node\n    log({ message: `Hello ${name}` })   //@n:tpl-node\n}\n";
        let formatted = format_flowscript(source, true).expect("format");
        assert!(
            formatted.contains("log({ message: \"a\" })"),
            "single quotes must canonicalize to double quotes:\n{formatted}"
        );
        assert!(
            formatted.contains("x = x + 1"),
            "compound assignment must canonicalize:\n{formatted}"
        );
        assert!(
            formatted.contains("// keep this comment"),
            "comments must survive:\n{formatted}"
        );
        assert!(
            formatted.contains("`Hello ${name}`"),
            "template literals must survive:\n{formatted}"
        );
        assert!(
            formatted.contains("//@n:log-node") && formatted.contains("//@n:entry"),
            "anchors must be preserved and re-emitted:\n{formatted}"
        );

        let stripped = format_flowscript(source, false).expect("format without anchors");
        assert!(
            !stripped.contains("//@n:"),
            "anchors: false must strip anchor comments:\n{stripped}"
        );
    }

    /// Formatting already-canonical text is the identity — checked against a committed fixture
    /// snapshot in both the anchored and plain forms.
    #[test]
    fn format_is_identity_on_canonical_fixture_text() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/ast");
        for (file, anchors) in [
            ("ttwctnp08u18sg2z6nmcqqak.anchored.flow", true),
            ("ttwctnp08u18sg2z6nmcqqak.flow", false),
            ("bypaw6n2ksuvrw0kcaj14omz.anchored.flow", true),
            ("bypaw6n2ksuvrw0kcaj14omz.flow", false),
        ] {
            let path = dir.join(file);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let formatted = format_flowscript(&text, anchors)
                .unwrap_or_else(|e| panic!("{file} must parse: {e:?}"));
            assert_eq!(
                formatted, text,
                "{file}: formatting canonical text must be identity"
            );
        }
    }

    #[test]
    fn format_surfaces_parse_errors() {
        let error = format_flowscript("eventsSimple run() {", true).expect_err("unclosed block");
        assert!(error.line >= 1);
    }
}

#[cfg(test)]
mod scoped_flowscript {
    use super::*;
    use crate::flow::copilot::{BoardCommand, NodeMetadata, node_to_metadata};
    use flow_like_types::tokio;
    use std::path::PathBuf;
    use std::sync::Arc;

    const LOAD_VARIABLES_EVENT: &str = "slde8unylsfksbdl72a0bfce";
    const OPEN_MEMORY_NODE: &str = "o20ngu02bpt0hlm16ckg6cd0";
    const FETCH_PAGE_EVENT: &str = "jifmm59liln9cnwc7ec83rf5";
    const PUSH_STEP_NODE: &str = "oyomq4tgddsswio67wqomq25";
    const CALL_LIBRARIAN_EVENT: &str = "v36kd2hgbjdmg5m9xy0b2xg6";
    const CONSTRUCT_PROMPT_CALL_NODE: &str = "o336xwkn4lhf9s70qdyzzqk4";
    const CONSTRUCT_PROMPT_FUNCTION: &str = "olhizg2b8s6seeuntnn1ni4o";

    async fn load_fixture(file_name: &str) -> Board {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/ast")
            .canonicalize()
            .expect("tests/ast directory should exist");
        let store: Arc<dyn flow_like_storage::object_store::ObjectStore> = Arc::new(
            flow_like_storage::object_store::local::LocalFileSystem::new_with_prefix(&dir)
                .expect("local object store"),
        );
        let proto: flow_like_types::proto::Board = crate::utils::compression::from_compressed(
            store,
            flow_like_storage::Path::from(file_name),
        )
        .await
        .unwrap_or_else(|e| panic!("decode {file_name}: {e}"));
        super::generate_flowscript::fixture_board(proto)
    }

    fn board_catalog(board: &Board) -> Vec<NodeMetadata> {
        board.nodes.values().map(node_to_metadata).collect()
    }

    fn anchored() -> RenderOptions {
        RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        }
    }

    fn removal_ids(commands: &[BoardCommand]) -> Vec<String> {
        commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::RemoveNode { node_id, .. } => Some(node_id.clone()),
                BoardCommand::RemoveVariable { variable_id, .. } => Some(variable_id.clone()),
                BoardCommand::RemoveLayer { layer_id, .. } => Some(layer_id.clone()),
                BoardCommand::RemoveComment { comment_id, .. } => Some(comment_id.clone()),
                _ => None,
            })
            .collect()
    }

    /// (a) A scoped render of one event keeps that event plus all variables/interfaces and
    /// nothing else; a section that references a function pulls that function in.
    #[tokio::test]
    async fn scoped_render_keeps_selection_references_and_variables() {
        let board = load_fixture("ttwctnp08u18sg2z6nmcqqak.board").await;

        let scoped =
            board_to_flowscript_scoped(&board, &[OPEN_MEMORY_NODE.to_string()], &anchored());
        assert_eq!(
            scoped.scope_anchors,
            vec![LOAD_VARIABLES_EVENT.to_string()],
            "selecting a body node must scope to its owning event"
        );
        assert!(scoped.text.contains("loadVariables() {"), "{}", scoped.text);
        assert!(
            scoped.text.contains("const librarian"),
            "all variables must render:\n{}",
            &scoped.text[..500]
        );
        assert!(
            scoped.text.contains("interface Bit "),
            "interfaces must render"
        );
        assert!(
            !scoped.text.contains("function constructPrompt"),
            "unreferenced functions must not render"
        );
        assert!(
            !scoped.text.contains("upsertDatabaseItem") && !scoped.text.contains("callLibrarian"),
            "other events must not render:\n{}",
            scoped.text
        );

        let scoped = board_to_flowscript_scoped(
            &board,
            &[CONSTRUCT_PROMPT_CALL_NODE.to_string()],
            &anchored(),
        );
        assert!(
            scoped.text.contains("function constructPrompt"),
            "a function referenced by the kept event must be declared:\n{}",
            scoped.text
        );
        assert!(scoped.text.contains("callLibrarian("), "{}", scoped.text);
        assert!(
            scoped
                .scope_anchors
                .contains(&CONSTRUCT_PROMPT_FUNCTION.to_string())
                && scoped
                    .scope_anchors
                    .contains(&CALL_LIBRARIAN_EVENT.to_string()),
            "scope anchors must list the kept event and the referenced function: {:?}",
            scoped.scope_anchors
        );
    }

    /// (b) Reconciling a scoped render back with its scope anchors never plans a removal of
    /// anything (the slice is unchanged), and any residual command/diagnostic it produces is one
    /// the full-document round-trip produces too — the scope can only shrink the residual class.
    #[tokio::test]
    async fn scoped_roundtrip_never_removes_out_of_scope_content() {
        for file_name in [
            "ttwctnp08u18sg2z6nmcqqak.board",
            "bypaw6n2ksuvrw0kcaj14omz.board",
        ] {
            let board = load_fixture(file_name).await;
            let catalog = board_catalog(&board);

            let full_text = board_to_flowscript(&board, &anchored());
            let full = reconcile_text_with_catalog(&board, &full_text, &catalog);
            let full_commands: Vec<String> = full
                .commands
                .iter()
                .map(|command| format!("{command:?}"))
                .collect();

            let ast = lower_to_ast(&board);
            let section_anchors: Vec<String> = ast
                .events
                .iter()
                .filter_map(|event| event.anchor.clone())
                .chain(ast.functions.iter().filter_map(|f| f.anchor.clone()))
                .collect();
            assert!(!section_anchors.is_empty(), "{file_name}: no sections");

            for anchor in section_anchors {
                let scoped = board_to_flowscript_scoped(&board, &[anchor.clone()], &anchored());
                assert!(
                    scoped.scope_anchors.contains(&anchor),
                    "{file_name}: {anchor} missing from its own scope"
                );
                let result = reconcile_text_with_catalog_scoped(
                    &board,
                    &scoped.text,
                    &catalog,
                    Some(&scoped.scope_anchors),
                );
                let removals = removal_ids(&result.commands);
                assert!(
                    removals.is_empty(),
                    "{file_name}: scoped no-op for `{anchor}` planned removals {removals:?}\n{}",
                    scoped.text
                );
                for command in &result.commands {
                    let rendered = format!("{command:?}");
                    assert!(
                        full_commands.contains(&rendered),
                        "{file_name}: scoped residual for `{anchor}` exceeds the full round-trip residual: {rendered}"
                    );
                }
                for diagnostic in &result.diagnostics {
                    assert!(
                        full.diagnostics.contains(diagnostic),
                        "{file_name}: scoped diagnostic for `{anchor}` exceeds the full round-trip: {diagnostic}"
                    );
                }
            }
        }
    }

    /// (c) A literal edit inside the slice reconciles to exactly that pin update.
    #[tokio::test]
    async fn scoped_literal_edit_yields_exactly_one_pin_update() {
        let board = load_fixture("ttwctnp08u18sg2z6nmcqqak.board").await;
        let catalog = board_catalog(&board);

        let scoped =
            board_to_flowscript_scoped(&board, &[LOAD_VARIABLES_EVENT.to_string()], &anchored());
        let baseline = reconcile_text_with_catalog_scoped(
            &board,
            &scoped.text,
            &catalog,
            Some(&scoped.scope_anchors),
        );
        assert!(
            baseline.commands.is_empty() && baseline.diagnostics.is_empty(),
            "the unchanged slice must be a clean no-op: {:?} {:?}",
            baseline.commands,
            baseline.diagnostics
        );

        let needle = "name: \"memory\", userScoped: true, batchSize: 1000";
        let edited = scoped
            .text
            .replace(needle, "name: \"memory\", userScoped: true, batchSize: 500");
        assert_ne!(
            edited, scoped.text,
            "literal edit must apply:\n{}",
            scoped.text
        );

        let result = reconcile_text_with_catalog_scoped(
            &board,
            &edited,
            &catalog,
            Some(&scoped.scope_anchors),
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        match result.commands.as_slice() {
            [BoardCommand::UpdateNodePin { node_id, value, .. }] => {
                assert_eq!(node_id, OPEN_MEMORY_NODE);
                assert_eq!(value, &flow_like_types::Value::from(500));
            }
            other => panic!("expected exactly one UpdateNodePin, got {other:?}"),
        }
    }

    /// (d) Omitting a rendered statement from the slice still deletes it — deletion works INSIDE
    /// the scope; only out-of-scope content is invisible.
    #[tokio::test]
    async fn scoped_apply_still_deletes_within_scope() {
        let board = load_fixture("ttwctnp08u18sg2z6nmcqqak.board").await;
        let catalog = board_catalog(&board);

        let scoped =
            board_to_flowscript_scoped(&board, &[FETCH_PAGE_EVENT.to_string()], &anchored());
        let edited: String = scoped
            .text
            .lines()
            .filter(|line| !line.contains(PUSH_STEP_NODE))
            .collect::<Vec<_>>()
            .join("\n");
        assert_ne!(edited, scoped.text, "the pushStep line must exist");

        let result = reconcile_text_with_catalog_scoped(
            &board,
            &edited,
            &catalog,
            Some(&scoped.scope_anchors),
        );
        let removals = removal_ids(&result.commands);
        assert!(
            removals.contains(&PUSH_STEP_NODE.to_string()),
            "the omitted in-scope statement must be removed: {removals:?}\ndiagnostics: {:?}",
            result.diagnostics
        );
        assert!(
            removals.iter().all(|node_id| node_id == PUSH_STEP_NODE),
            "nothing outside the omitted statement may be removed: {removals:?}"
        );
    }

    /// (e) Deletion gating is unchanged for full applies: an unscoped apply of a partial document
    /// still blocks with `blocked_destructive_flowscript_message`, while the SAME partial text
    /// applied with its scope anchors is a clean no-op — the scope flag alone flips the semantics.
    #[test]
    fn full_apply_deletion_gate_unchanged_and_scoped_apply_bypasses_nothing_in_scope() {
        use crate::flow::node::Node;
        use crate::flow::variable::VariableType;
        use crate::state::{FlowLikeConfig, FlowLikeState};
        use crate::utils::http::HTTPClient;

        fn empty_board() -> Board {
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

        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let mut log = Node::new("log", "Log", "", "debug");
        log.add_input_pin("exec_in", "In", "", VariableType::Execution);
        log.add_input_pin("message", "Message", "", VariableType::String);
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let catalog_nodes = vec![event, log];

        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let mut board = empty_board();
        let source = "onMessage() {\n    log({ message: \"one\" })\n}\n\nonTick() {\n    log({ message: \"two\" })\n}\n";
        let applied = runtime
            .block_on(apply_flowscript_to_board(
                &mut board,
                source,
                &catalog_nodes,
                state.clone(),
                None,
                false,
            ))
            .expect("seed apply");
        assert!(applied.diagnostics.is_empty(), "{:?}", applied.diagnostics);

        let ast = lower_to_ast(&board);
        assert_eq!(ast.events.len(), 2, "two events seeded");
        let on_message = ast
            .events
            .iter()
            .find(|event| event.event_name.as_deref() == Some("onMessage"))
            .and_then(|event| event.anchor.clone())
            .expect("onMessage anchor");

        let scoped = board_to_flowscript_scoped(&board, &[on_message.clone()], &anchored());
        assert!(
            scoped.text.contains("one") && !scoped.text.contains("two"),
            "scoped render must keep only onMessage:\n{}",
            scoped.text
        );
        assert_eq!(scoped.scope_anchors, vec![on_message]);

        // Unscoped apply of the partial document: the omitted event's body reads as a deletion
        // and the gate must block it with the canonical message.
        let mut unscoped_board = board.clone();
        let blocked = runtime
            .block_on(apply_flowscript_to_board(
                &mut unscoped_board,
                &scoped.text,
                &catalog_nodes,
                state.clone(),
                None,
                false,
            ))
            .expect("unscoped apply");
        assert!(
            blocked.commands.is_empty(),
            "blocked apply must execute nothing"
        );
        let first = blocked
            .diagnostics
            .first()
            .expect("unscoped partial apply must be blocked");
        assert!(
            first.starts_with("FlowScript edit would delete"),
            "blocked message must be unchanged: {first}"
        );
        let summaries = destructive_flowscript_command_summaries(&blocked.board_commands);
        assert_eq!(first, &blocked_destructive_flowscript_message(&summaries));

        // The same text with its scope anchors is a clean no-op: nothing deleted, nothing blocked.
        let mut scoped_board = board.clone();
        let scoped_apply = runtime
            .block_on(apply_flowscript_to_board_scoped(
                &mut scoped_board,
                &scoped.text,
                &catalog_nodes,
                state,
                None,
                false,
                Some(&scoped.scope_anchors),
            ))
            .expect("scoped apply");
        assert!(
            scoped_apply.commands.is_empty()
                && scoped_apply.board_commands.is_empty()
                && scoped_apply.diagnostics.is_empty(),
            "scoped apply must be a clean no-op: {:?} {:?}",
            scoped_apply.board_commands,
            scoped_apply.diagnostics
        );
    }
}
