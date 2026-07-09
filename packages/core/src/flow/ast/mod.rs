//! Board ⇄ FlowScript glue (the *board* half of the pipeline).
//!
//! The language half — the [`flow_like_ast`] crate — owns the IR plus pure render/parse/lint
//! operations. This module owns everything that touches the core [`Board`] graph model:
//! lowering a board into the AST today, and reconcile/placement in later phases.
//!
//! See `todo/ast.md`.

mod apply;
mod lower;
mod reconcile;
mod signatures;
mod types;

pub use apply::{
    ApplyFlowScriptResult, apply_flowscript_to_board, blocked_destructive_flowscript_message,
    destructive_flowscript_command_summaries,
};
pub use flow_like_ast::{
    BoardAst, DeclarationFile, NodeSchemas, ParseError, RenderOptions, Signature, SignatureSet,
    declarations_by_category, declarations_by_package, parse, render, schema_sidecar,
};
pub use lower::lower_board;
pub use reconcile::{
    MetadataEnricher, ReconcileResult, reconcile, reconcile_text, reconcile_text_with_catalog,
    reconcile_text_with_catalog_enriched, reconcile_with_catalog,
};
pub use signatures::{node_to_signature, node_to_signature_in};

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
            let board = Board::from_proto(proto);

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
    /// Known remaining gaps (2026-07-05, run manually with `--ignored` while closing them):
    /// - anchored Assign / variable_set paths re-emit ConnectPins for edges that already exist
    ///   (needs the same already-wired guard `plan_call_arguments` has),
    /// - `variable.field` sugar (Field on a variable ref) re-creates variable_get + struct_get
    ///   chains instead of reusing the existing readers,
    /// - event-level `return` (return_result sugar) reports "only supported inside functions",
    /// - multi-pin array args (`tools: [...]`, `fnRefs: [...]`) don't map to repeated pins.
    #[ignore = "documents remaining lower→reconcile roundtrip gaps; the destructive classes are fixed"]
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
            let board = Board::from_proto(proto);

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

#[cfg(test)]
mod lower_tests {
    use super::*;
    use crate::flow::board::{Board, ExecutionMode, ExecutionStage};
    use crate::flow::execution::LogLevel;
    use crate::flow::node::Node;
    use crate::flow::variable::{VariableType, infer_schema_from_json};
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
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
        }
    }

    fn connect(board: &mut Board, from_node: &str, from_pin: &str, to_node: &str, to_pin: &str) {
        crate::flow::board::commands::pins::connect_pins::connect_pins(
            board, from_node, from_pin, to_node, to_pin,
        )
        .expect("connect pins");
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
        assert_eq!(ev.name, "now");
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
            text_out.contains("now(title: string)"),
            "event declares its payload param:\n{text_out}"
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
}
