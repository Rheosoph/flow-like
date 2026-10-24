//! `@collapse` groups end to end: text applied to a board, the board rendered back, and the
//! rendered text reconciled against the board it came from.

use std::sync::Arc;

use crate::flow::ast::lower_tests::empty_board;
use crate::flow::ast::{
    ReconcileOptions, RenderOptions, apply_flowscript_to_board, board_to_flowscript,
    reconcile_text_with_catalog_opts,
};
use crate::flow::board::{Board, Layer, LayerType};
use crate::flow::copilot::{BoardCommand, NodeMetadata, node_to_metadata};
use crate::flow::node::Node;
use crate::flow::pin::PinType;
use crate::flow::variable::VariableType;
use crate::state::{FlowLikeConfig, FlowLikeState};
use crate::utils::http::HTTPClient;

fn catalog_nodes() -> Vec<Node> {
    let mut event = Node::new("events_simple", "Simple Event", "", "events");
    event.set_start(true);
    event.add_output_pin("exec_out", "Out", "", VariableType::Execution);

    let mut log = Node::new("log", "Log", "", "debug");
    log.add_input_pin("exec_in", "In", "", VariableType::Execution);
    log.add_input_pin("message", "Message", "", VariableType::String);
    log.add_output_pin("exec_out", "Out", "", VariableType::Execution);

    let mut branch = Node::new("control_branch", "Branch", "", "control");
    branch.add_input_pin("exec_in", "In", "", VariableType::Execution);
    branch.add_input_pin("condition", "Condition", "", VariableType::Boolean);
    branch.add_output_pin("true", "True", "", VariableType::Execution);
    branch.add_output_pin("false", "False", "", VariableType::Execution);

    vec![event, log, branch]
}

fn catalog() -> Vec<NodeMetadata> {
    catalog_nodes().iter().map(node_to_metadata).collect()
}

fn apply(board: &mut Board, text: &str) -> Vec<BoardCommand> {
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
            board,
            text,
            &catalog_nodes(),
            state,
            None,
            false,
        ))
        .unwrap_or_else(|error| panic!("apply failed:\n{text}\n{error}"));
    assert!(
        applied.diagnostics.is_empty(),
        "apply must succeed:\n{text}\ndiagnostics: {:?}",
        applied.diagnostics
    );
    applied.board_commands
}

fn render(board: &Board, anchors: bool) -> String {
    board_to_flowscript(
        board,
        &RenderOptions {
            anchors,
            ..RenderOptions::default()
        },
    )
}

/// The board's own anchored text reconciles to nothing.
fn assert_round_trip_is_a_no_op(board: &Board) {
    let text = render(board, true);
    let result =
        reconcile_text_with_catalog_opts(board, &text, &catalog(), &ReconcileOptions::default());
    assert!(
        result.diagnostics.is_empty() && result.commands.is_empty(),
        "the board's own text must reconcile to nothing:\n{text}\ndiagnostics: {:?}\ncommands: {:?}",
        result.diagnostics,
        result.commands
    );
}

fn node_logging<'b>(board: &'b Board, message: &str) -> &'b Node {
    board
        .nodes
        .values()
        .find(|node| {
            node.pins.values().any(|pin| {
                pin.name == "message"
                    && pin.default_value.as_deref().is_some_and(|bytes| {
                        flow_like_types::json::from_slice::<String>(bytes)
                            .is_ok_and(|value| value == message)
                    })
            })
        })
        .unwrap_or_else(|| panic!("no node logs `{message}`"))
}

fn group<'b>(board: &'b Board, name: &str) -> &'b Layer {
    let mut matches = board
        .layers
        .values()
        .filter(|layer| layer.name == name && matches!(layer.r#type, LayerType::Collapsed));
    let only = matches
        .next()
        .unwrap_or_else(|| panic!("no group `{name}`"));
    assert!(matches.next().is_none(), "one group `{name}`");
    only
}

fn layer_of(board: &Board, message: &str) -> Option<String> {
    node_logging(board, message).layer.clone()
}

/// The messages logged along the execution chain from the event, following bridge pins.
fn execution_order(board: &Board) -> Vec<String> {
    let owner = |pin_id: &str| -> Option<(&Node, bool)> {
        board.nodes.values().find_map(|node| {
            node.pins
                .get(pin_id)
                .map(|pin| (node, pin.pin_type == PinType::Input))
        })
    };
    let mut order = Vec::new();
    let entry = board
        .nodes
        .values()
        .find(|node| node.name == "events_simple")
        .expect("an entry");
    let mut current = entry;
    while let Some(out) = current
        .pins
        .values()
        .find(|pin| pin.pin_type == PinType::Output && pin.data_type == VariableType::Execution)
    {
        let mut frontier: Vec<String> = out.connected_to.iter().cloned().collect();
        let mut next = None;
        while let Some(pin_id) = frontier.pop() {
            if let Some((node, true)) = owner(&pin_id) {
                next = Some(node);
                break;
            }
            for layer in board.layers.values() {
                if let Some(bridge) = layer.pins.get(&pin_id) {
                    frontier.extend(bridge.connected_to.iter().cloned());
                }
            }
        }
        let Some(node) = next else {
            break;
        };
        if let Some(message) = node.pins.values().find(|pin| pin.name == "message") {
            let value = message
                .default_value
                .as_deref()
                .and_then(|bytes| flow_like_types::json::from_slice::<String>(bytes).ok())
                .unwrap_or_default();
            order.push(value);
        }
        current = node;
    }
    order
}

const NESTED: &str = "use debug::*

eventsSimple simpleEvent() {
    @collapse(\"Load\") {
        log(\"a\")
        @collapse(\"Parse\") {
            log(\"b\")
        }
    }
    log(\"c\")
}
";

#[test]
fn written_groups_become_layers_that_hold_their_nodes() {
    let mut board = empty_board();
    apply(&mut board, NESTED);

    let load = group(&board, "Load");
    let parse = group(&board, "Parse");
    assert_eq!(load.parent_id, None);
    assert_eq!(parse.parent_id.as_deref(), Some(load.id.as_str()));
    assert_eq!(layer_of(&board, "a"), Some(load.id.clone()));
    assert_eq!(layer_of(&board, "b"), Some(parse.id.clone()));
    assert_eq!(layer_of(&board, "c"), None);
    assert_eq!(execution_order(&board), ["a", "b", "c"]);

    assert_eq!(render(&board, false), NESTED);
    assert_round_trip_is_a_no_op(&board);
}

#[test]
fn the_shorthand_nests_one_group_per_name() {
    let mut board = empty_board();
    apply(
        &mut board,
        "eventsSimple() {\n    @collapse(\"Outer/Inner\") {\n        log(\"a\")\n    }\n}\n",
    );
    let outer = group(&board, "Outer");
    let inner = group(&board, "Inner");
    assert_eq!(inner.parent_id.as_deref(), Some(outer.id.as_str()));
    assert_eq!(layer_of(&board, "a"), Some(inner.id.clone()));
    assert_round_trip_is_a_no_op(&board);
}

#[test]
fn wrapping_anchored_statements_moves_them_into_a_new_group() {
    let mut board = empty_board();
    apply(
        &mut board,
        "eventsSimple() {\n    log(\"a\")\n    log(\"b\")\n    log(\"c\")\n}\n",
    );
    let anchored = render(&board, true);
    let b_line = anchored
        .lines()
        .find(|line| line.contains("\"b\""))
        .expect("the b statement")
        .to_string();
    let wrapped = anchored.replace(
        &b_line,
        &format!("    @collapse(\"Mid\") {{\n    {b_line}\n    }}"),
    );
    let b_before = node_logging(&board, "b").id.clone();
    apply(&mut board, &wrapped);

    let mid = group(&board, "Mid");
    assert_eq!(
        node_logging(&board, "b").id,
        b_before,
        "the node keeps its identity"
    );
    assert_eq!(layer_of(&board, "b"), Some(mid.id.clone()));
    assert_eq!(layer_of(&board, "a"), None);
    assert_eq!(layer_of(&board, "c"), None);
    assert_eq!(execution_order(&board), ["a", "b", "c"]);
    assert_round_trip_is_a_no_op(&board);
}

#[test]
fn dropping_a_wrapper_ungroups_and_removes_the_emptied_group() {
    let mut board = empty_board();
    apply(&mut board, NESTED);
    let load_id = group(&board, "Load").id.clone();
    let anchored = render(&board, true);
    let mut lines: Vec<&str> = anchored.lines().collect();
    let open = lines
        .iter()
        .position(|line| line.contains("@collapse(\"Parse\")"))
        .expect("the Parse block");
    lines.remove(open);
    let close = lines[open..]
        .iter()
        .position(|line| line.trim() == "}")
        .map(|offset| open + offset)
        .expect("the Parse block's end");
    lines.remove(close);
    let edited = format!("{}\n", lines.join("\n"));

    apply(&mut board, &edited);

    assert!(
        !board.layers.values().any(|layer| layer.name == "Parse"),
        "the emptied group is ungrouped"
    );
    assert_eq!(layer_of(&board, "b"), Some(load_id));
    assert_eq!(execution_order(&board), ["a", "b", "c"]);
    assert_round_trip_is_a_no_op(&board);
}

#[test]
fn renaming_an_anchored_group_keeps_its_layer() {
    let mut board = empty_board();
    apply(&mut board, NESTED);
    let load_id = group(&board, "Load").id.clone();
    let edited = render(&board, true).replace("@collapse(\"Load\")", "@collapse(\"Fetch\")");

    let commands = apply(&mut board, &edited);

    assert!(commands.iter().any(|command| matches!(
        command,
        BoardCommand::RenameLayer { layer_id, name, .. } if *layer_id == load_id && name == "Fetch"
    )));
    assert_eq!(group(&board, "Fetch").id, load_id);
    assert_round_trip_is_a_no_op(&board);
}

#[test]
fn a_document_without_groups_leaves_existing_groups_alone() {
    let mut board = empty_board();
    apply(&mut board, NESTED);
    let before: Vec<Option<String>> = ["a", "b", "c"]
        .iter()
        .map(|message| layer_of(&board, message))
        .collect();
    let stripped: String = render(&board, true)
        .lines()
        .filter(|line| !line.contains("@collapse(") && line.trim() != "}" || line == &"}")
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(!stripped.contains("@collapse"));

    let state = render(&board, true);
    let result = reconcile_text_with_catalog_opts(
        &board,
        &stripped,
        &catalog(),
        &ReconcileOptions::default(),
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert!(
        !result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::MoveToLayer { .. } | BoardCommand::RemoveLayer { .. }
        )),
        "a document that writes no group moves nothing:\n{stripped}\nfrom\n{state}\n{:?}",
        result.commands
    );
    let after: Vec<Option<String>> = ["a", "b", "c"]
        .iter()
        .map(|message| layer_of(&board, message))
        .collect();
    assert_eq!(before, after);
}

#[test]
fn a_branch_arm_outside_the_group_of_its_if_restarts_from_the_root() {
    let text = "eventsSimple simpleEvent() {
    @collapse(\"Check\") {
        if (true) {
            @collapse(\"/\") {
                debug::log(\"outside\")
            }
        }
    }
}
";
    let mut board = empty_board();
    apply(&mut board, text);
    let check = group(&board, "Check");
    let branch = board
        .nodes
        .values()
        .find(|node| node.name == "control_branch")
        .expect("the branch");
    assert_eq!(branch.layer.as_deref(), Some(check.id.as_str()));
    assert_eq!(layer_of(&board, "outside"), None);
    assert_eq!(render(&board, false), text);
    assert_round_trip_is_a_no_op(&board);
}

#[test]
fn a_group_whose_statements_are_apart_renders_as_two_blocks_of_one_layer() {
    let mut board = empty_board();
    apply(
        &mut board,
        "eventsSimple() {\n    @collapse(\"G\") {\n        log(\"a\")\n    }\n    log(\"b\")\n    @collapse(\"G\") {\n        log(\"c\")\n    }\n}\n",
    );
    let g = group(&board, "G").id.clone();
    assert_eq!(layer_of(&board, "a"), Some(g.clone()));
    assert_eq!(layer_of(&board, "c"), Some(g.clone()));
    let anchored = render(&board, true);
    assert_eq!(anchored.matches(&format!("//@l:{g}")).count(), 2);
    assert_round_trip_is_a_no_op(&board);
}

#[test]
fn a_group_inside_a_function_lives_in_its_layer() {
    let text = "function helper() {
    @collapse(\"Inner\") {
        log(\"x\")
    }
}

eventsSimple() {
    helper()
}
";
    let mut catalog = catalog_nodes();
    let mut call = Node::new("control_call_function", "Call Function", "", "control");
    call.add_input_pin("exec_in", "In", "", VariableType::Execution);
    call.add_input_pin("function_layer_id", "Function", "", VariableType::String);
    call.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    catalog.push(call);
    let mut board = empty_board();
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let applied = flow_like_types::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(apply_flowscript_to_board(
            &mut board, text, &catalog, state, None, false,
        ))
        .expect("apply");
    assert!(applied.diagnostics.is_empty(), "{:?}", applied.diagnostics);

    let function = board
        .layers
        .values()
        .find(|layer| matches!(layer.r#type, LayerType::Function))
        .expect("the function layer");
    let inner = group(&board, "Inner");
    assert_eq!(inner.parent_id.as_deref(), Some(function.id.as_str()));
    assert_eq!(layer_of(&board, "x"), Some(inner.id.clone()));
    let result = reconcile_text_with_catalog_opts(
        &board,
        &render(&board, true),
        &catalog.iter().map(node_to_metadata).collect::<Vec<_>>(),
        &ReconcileOptions::default(),
    );
    assert!(
        result.diagnostics.is_empty() && result.commands.is_empty(),
        "{:?} {:?}",
        result.diagnostics,
        result.commands
    );
}

/// Delete the `@collapse("<name>")` line and its closing brace, keeping the statements.
fn unwrap_group(text: &str, name: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    let open = lines
        .iter()
        .position(|line| line.contains(&format!("@collapse(\"{name}\")")))
        .unwrap_or_else(|| panic!("a `{name}` block"));
    let indent = lines[open].len() - lines[open].trim_start().len();
    lines.remove(open);
    let close = lines[open..]
        .iter()
        .position(|line| line.trim() == "}" && line.len() - line.trim_start().len() == indent)
        .map(|offset| open + offset)
        .expect("the block's closing brace");
    lines.remove(close);
    format!("{}\n", lines.join("\n"))
}

const TWO_GROUPS: &str = "use debug::*

eventsSimple simpleEvent() {
    @collapse(\"Keep\") {
        log(\"a\")
    }
    @collapse(\"Drop\") {
        log(\"b\")
    }
}
";

#[test]
fn leaving_a_group_for_the_board_root_is_not_held_by_the_deletion_gate() {
    let mut board = empty_board();
    apply(&mut board, TWO_GROUPS);
    let keep = group(&board, "Keep").id.clone();

    let edited = unwrap_group(&render(&board, true), "Drop");
    let commands = apply(&mut board, &edited);

    assert!(commands.iter().any(|command| matches!(
        command,
        BoardCommand::MoveToLayer {
            target_layer: None,
            ..
        }
    )));
    assert_eq!(layer_of(&board, "b"), None);
    assert!(!board.layers.values().any(|layer| layer.name == "Drop"));
    assert_eq!(layer_of(&board, "a"), Some(keep));
    assert_eq!(execution_order(&board), ["a", "b"]);
    assert_round_trip_is_a_no_op(&board);
}

#[test]
fn an_additive_document_only_places_what_it_writes_inside_groups() {
    let mut board = empty_board();
    apply(&mut board, TWO_GROUPS);
    let anchored = render(&board, true);
    let b_line = anchored
        .lines()
        .find(|line| line.contains("\"b\""))
        .expect("the b statement")
        .trim()
        .to_string();
    let edited = unwrap_group(&anchored, "Keep").replace(
        &b_line,
        &format!("@collapse(\"Fresh\") {{\n            {b_line}\n        }}"),
    );
    let ast = crate::flow::ast::parse(&edited).expect("the edit parses");

    let result = crate::flow::ast::reconcile_with_catalog_mode(
        &board,
        &ast,
        &catalog(),
        crate::flow::ast::ReconcileMode::Additive,
    );

    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let a = node_logging(&board, "a").id.clone();
    let b = node_logging(&board, "b").id.clone();
    let moved: Vec<&Vec<String>> = result
        .commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::MoveToLayer { ids, .. } => Some(ids),
            _ => None,
        })
        .collect();
    assert_eq!(
        moved,
        [&vec![b]],
        "only the statement written in a group moves"
    );
    assert!(!moved.iter().any(|ids| ids.contains(&a)));
    assert!(
        !result
            .commands
            .iter()
            .any(|command| matches!(command, BoardCommand::RemoveLayer { .. })),
        "an additive document removes no group"
    );
}

#[test]
fn copying_anchored_text_keeps_same_named_groups_apart() {
    let mut source = empty_board();
    apply(&mut source, TWO_GROUPS);
    let renamed = render(&source, true).replace("@collapse(\"Drop\")", "@collapse(\"Keep\")");
    apply(&mut source, &renamed);
    let named_keep = |board: &Board| {
        board
            .layers
            .values()
            .filter(|layer| layer.name == "Keep")
            .count()
    };
    assert_eq!(named_keep(&source), 2, "two groups share the name");

    let mut copy = empty_board();
    apply(&mut copy, &render(&source, true));

    assert_eq!(named_keep(&copy), 2, "the copy keeps them apart");
    assert_ne!(layer_of(&copy, "a"), layer_of(&copy, "b"));
    assert_eq!(render(&copy, false), render(&source, false));
    assert_round_trip_is_a_no_op(&copy);
}
