//! Edits people made in the FlowScript editor that production apply-failure telemetry showed the
//! pipeline rejecting or mishandling. Each test builds a board through the product apply path,
//! renders it the way the editor does (anchors on), makes the edit, and checks both the apply and
//! that the result renders back as a no-op.
//!
//! The text-only spellings from the same captures live in
//! `tests/ast/handwritten/t1-editor-telemetry-spellings.flow`.

mod flowscript_support;

use flow_like::flow::ast::{
    ApplyFlowScriptResult, RenderOptions, apply_board_commands_to_board, apply_flowscript_to_board,
    board_to_flowscript, reconcile_text_with_catalog_enriched,
};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{BoardCommand, NodePosition};
use flow_like::flow::node::Node;
use flow_like::flow::pin::{Pin, PinType};
use flow_like::flow::variable::VariableType;
use flow_like_storage::object_store::path::Path;
use flowscript_support::{CATALOG, all_nodes, catalog, catalog_state};

async fn board_from(source: &str) -> Board {
    let mut board = Board::new_detached(Some("edit-regressions".into()), Path::default());
    let result = apply_flowscript_to_board(
        &mut board,
        source,
        &CATALOG.nodes,
        catalog_state().await,
        None,
        false,
    )
    .await
    .expect("seed source applies");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    board
}

fn anchored(board: &Board) -> String {
    board_to_flowscript(
        board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    )
}

async fn apply_edit(board: &mut Board, source: &str) -> ApplyFlowScriptResult {
    let result = apply_flowscript_to_board(
        board,
        source,
        &CATALOG.nodes,
        catalog_state().await,
        None,
        true,
    )
    .await
    .expect("edit applies without rolling back");
    assert!(
        result.diagnostics.is_empty(),
        "{:?}\n{source}",
        result.diagnostics
    );
    result
}

async fn apply_commands(board: &mut Board, commands: Vec<BoardCommand>) {
    apply_board_commands_to_board(board, commands, &CATALOG.nodes, catalog_state().await, None)
        .await
        .expect("board commands apply");
}

fn assert_round_trips(board: &Board) {
    let (catalog, enricher) = catalog();
    let text = anchored(board);
    let result = reconcile_text_with_catalog_enriched(board, &text, &catalog, &enricher);
    assert!(
        result.commands.is_empty() && result.diagnostics.is_empty(),
        "the board's own text must reconcile to nothing:\n{:?}\n{:?}\n{text}",
        result.commands,
        result.diagnostics
    );
}

/// The `log::info` node whose message is `message`.
fn info<'a>(board: &'a Board, message: &str) -> &'a Node {
    let expected = format!("\"{message}\"");
    all_nodes(board)
        .into_iter()
        .find(|node| {
            node.name == "log_info"
                && node.pins.values().any(|pin| {
                    pin.name == "message"
                        && pin
                            .default_value
                            .as_deref()
                            .is_some_and(|value| value == expected.as_bytes())
                })
        })
        .unwrap_or_else(|| panic!("no log::info with message {message:?}"))
}

fn exec_pin<'a>(node: &'a Node, pin_type: PinType) -> &'a Pin {
    node.pins
        .values()
        .find(|pin| pin.pin_type == pin_type && pin.data_type == VariableType::Execution)
        .unwrap_or_else(|| panic!("`{}` has no {pin_type:?} exec pin", node.friendly_name))
}

/// Whether `from`'s execution output reaches `to`'s input, directly or across bridge pins.
fn runs_before(board: &Board, from: &Node, to: &Node) -> bool {
    let target = exec_pin(to, PinType::Input).id.clone();
    let mut stack: Vec<String> = exec_pin(from, PinType::Output)
        .connected_to
        .iter()
        .cloned()
        .collect();
    let mut seen = std::collections::HashSet::new();
    while let Some(pin_id) = stack.pop() {
        if pin_id == target {
            return true;
        }
        if !seen.insert(pin_id.clone()) {
            continue;
        }
        if let Some(bridge) = board
            .layers
            .values()
            .find_map(|layer| layer.pins.get(&pin_id))
        {
            stack.extend(bridge.connected_to.iter().cloned());
        }
    }
    false
}

/// Drop the block header line containing `header`, the `} else {` and `}` lines that close it
/// at the same indentation, and dedent what was inside: the "unwrap" edit.
fn unwrap_block(text: &str, header: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.contains(header))
        .unwrap_or_else(|| panic!("no line contains {header:?}:\n{text}"));
    let indent = lines[start].len() - lines[start].trim_start().len();
    let closing = |line: &str| {
        line.len() - line.trim_start().len() == indent
            && (line.trim() == "}" || line.trim().starts_with("} else"))
    };
    let mut out = Vec::new();
    let mut open = true;
    for (index, line) in lines.iter().enumerate() {
        if index == start {
            continue;
        }
        if open && index > start && closing(line) {
            if line.trim() == "}" {
                open = false;
            }
            continue;
        }
        if open && index > start {
            out.push(line.strip_prefix("    ").unwrap_or(line).to_string());
        } else {
            out.push(line.to_string());
        }
    }
    out.join("\n") + "\n"
}

/// The rendered statement line carrying `message`, dedented to a Function body's indent.
fn body_line(text: &str, message: &str) -> String {
    let needle = format!("\"{message}\"");
    let line = text
        .lines()
        .find(|line| line.contains(&needle))
        .unwrap_or_else(|| panic!("no line renders {message:?}:\n{text}"));
    format!("    {}", line.trim_start())
}

/// Replace everything between the line containing `header` and its closing `}` with `body`.
fn replace_body(text: &str, header: &str, body: &[String]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.contains(header))
        .unwrap_or_else(|| panic!("no line contains {header:?}:\n{text}"));
    let end = start
        + lines[start..]
            .iter()
            .position(|line| *line == "}")
            .expect("the block closes");
    let mut out: Vec<String> = lines[..=start]
        .iter()
        .map(|line| line.to_string())
        .collect();
    out.extend(body.iter().cloned());
    out.extend(lines[end..].iter().map(|line| line.to_string()));
    out.join("\n") + "\n"
}

fn reconcile_diagnostics(board: &Board, text: &str) -> Vec<String> {
    let (catalog, enricher) = catalog();
    reconcile_text_with_catalog_enriched(board, text, &catalog, &enricher).diagnostics
}

fn function_exit<'a>(board: &'a Board, name: &str) -> &'a Pin {
    board
        .layers
        .values()
        .find(|layer| layer.name == name)
        .and_then(|layer| {
            layer.pins.values().find(|pin| {
                pin.pin_type == PinType::Output && pin.data_type == VariableType::Execution
            })
        })
        .unwrap_or_else(|| panic!("function {name} has an exit"))
}

fn without_line(text: &str, needle: &str) -> String {
    text.lines()
        .filter(|line| !line.contains(needle))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// "removing `For Each` leaves multiple execution successors": dropping a loop but keeping its
/// body must follow the order the text writes.
#[tokio::test(flavor = "multi_thread")]
async fn unwrapping_a_loop_runs_its_body_in_written_order() {
    let mut board = board_from(
        r#"eventsSimple unwrapLoop() {
    log::info({ message: "before", toast: false })
    for (const _ of ["a", "b"]) {
        log::info({ message: "inside", toast: false })
    }
    log::info({ message: "after", toast: false })
}
"#,
    )
    .await;
    let edited = unwrap_block(&anchored(&board), "for (");
    apply_edit(&mut board, &edited).await;

    assert!(
        !all_nodes(&board)
            .iter()
            .any(|node| node.name == "control_for_each")
    );
    assert!(runs_before(
        &board,
        info(&board, "before"),
        info(&board, "inside")
    ));
    assert!(runs_before(
        &board,
        info(&board, "inside"),
        info(&board, "after")
    ));
    assert_round_trips(&board);
}

/// Unwrapping an if/else keeps BOTH arms: the old `yes -> after` edge is superseded by
/// `yes -> no`, which the text writes, instead of `no` being cut off.
#[tokio::test(flavor = "multi_thread")]
async fn unwrapping_an_if_else_runs_both_arms_in_written_order() {
    let mut board = board_from(
        r#"eventsSimple unwrapBranch() {
    if (1 > 0) {
        log::info({ message: "yes", toast: false })
    } else {
        log::info({ message: "no", toast: false })
    }
    log::info({ message: "after", toast: false })
}
"#,
    )
    .await;
    let edited = unwrap_block(&anchored(&board), "if (");
    apply_edit(&mut board, &edited).await;

    assert!(runs_before(&board, info(&board, "yes"), info(&board, "no")));
    assert!(runs_before(
        &board,
        info(&board, "no"),
        info(&board, "after")
    ));
    assert_round_trips(&board);
}

/// "Pin `exec_out` not found on `Collapsed`": the re-join around a deleted or replaced statement
/// must name the nodes on both sides of a collapsed frame, never the frame itself.
#[tokio::test(flavor = "multi_thread")]
async fn editing_next_to_a_collapsed_group_rejoins_the_real_nodes() {
    for replace in [false, true] {
        let mut board = board_from(
            r#"eventsSimple grouped() {
    log::info({ message: "a", toast: false })
    log::info({ message: "b", toast: false })
    log::info({ message: "c", toast: false })
}
"#,
        )
        .await;
        let a = info(&board, "a").id.clone();
        apply_commands(
            &mut board,
            vec![BoardCommand::CreateLayer {
                name: "Collapsed".into(),
                ref_id: None,
                layer_type: Some("Collapsed".into()),
                node_ids: vec![a],
                pins: None,
                position: None,
                color: None,
                target_layer: None,
                cache: None,
                summary: None,
            }],
        )
        .await;
        let text = anchored(&board);
        let b_line = text
            .lines()
            .find(|line| line.contains("\"b\""))
            .expect("b renders")
            .to_string();
        let edited = if replace {
            text.replace(&b_line, "    log::info({ message: \"x\", toast: false })")
        } else {
            without_line(&text, "\"b\"")
        };
        let result = apply_edit(&mut board, &edited).await;
        let frame = board
            .layers
            .values()
            .find(|layer| layer.name == "Collapsed")
            .map(|layer| layer.id.clone())
            .expect("the frame survives");
        assert!(!result.board_commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, to_node, .. }
                if *from_node == frame || *to_node == frame
        )));
        let next = if replace { "x" } else { "c" };
        assert!(runs_before(&board, info(&board, "a"), info(&board, next)));
        assert_round_trips(&board);
    }
}

/// Auto-layout drops reroutes on execution wires. They must not show up in the text, must not
/// block Apply, and must go away with the wire they bent.
#[tokio::test(flavor = "multi_thread")]
async fn execution_reroutes_are_invisible_and_leave_with_their_wire() {
    let mut board = board_from(
        r#"eventsSimple bent() {
    log::info({ message: "before", toast: false })
    log::info({ message: "after", toast: false })
}
"#,
    )
    .await;
    let before = info(&board, "before");
    let after = info(&board, "after");
    let (before_id, after_id) = (before.id.clone(), after.id.clone());
    let (out_pin, in_pin) = (
        exec_pin(before, PinType::Output).name.clone(),
        exec_pin(after, PinType::Input).name.clone(),
    );
    apply_commands(
        &mut board,
        vec![
            BoardCommand::AddNode {
                node_type: "reroute".into(),
                ref_id: Some("$bend".into()),
                position: Some(NodePosition { x: 0.0, y: 0.0 }),
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::DisconnectPins {
                from_node: before_id.clone(),
                from_pin: out_pin.clone(),
                to_node: after_id.clone(),
                to_pin: in_pin.clone(),
                summary: None,
            },
            BoardCommand::ConnectPins {
                from_node: before_id,
                from_pin: out_pin,
                to_node: "$bend".into(),
                to_pin: "route_in".into(),
                summary: None,
            },
            BoardCommand::ConnectPins {
                from_node: "$bend".into(),
                from_pin: "route_out".into(),
                to_node: after_id,
                to_pin: in_pin,
                summary: None,
            },
        ],
    )
    .await;
    assert!(all_nodes(&board).iter().any(|node| node.name == "reroute"));

    let text = anchored(&board);
    assert!(!text.contains("reroute"), "{text}");
    assert_round_trips(&board);

    let after_line = text
        .lines()
        .find(|line| line.contains("\"after\""))
        .expect("after renders")
        .to_string();
    let edited = text.replace(
        &after_line,
        &format!("    log::info({{ message: \"middle\", toast: false }})\n{after_line}"),
    );
    apply_edit(&mut board, &edited).await;
    assert!(
        !all_nodes(&board).iter().any(|node| node.name == "reroute"),
        "the reroute bent a wire that no longer exists"
    );
    assert!(runs_before(
        &board,
        info(&board, "before"),
        info(&board, "middle")
    ));
    assert!(runs_before(
        &board,
        info(&board, "middle"),
        info(&board, "after")
    ));
    assert_round_trips(&board);
}

/// "assignment to variable … incompatible pin types or schemas": a variable typed with the
/// interface the editor renders for `Bit` must accept `ai::findModel`'s `Bit`.
#[tokio::test(flavor = "multi_thread")]
async fn a_variable_typed_with_its_rendered_interface_accepts_its_own_type() {
    let bit_schema = CATALOG
        .nodes
        .iter()
        .find(|node| node.name == "ai_generative_find_model")
        .and_then(|node| {
            node.pins
                .values()
                .find(|pin| pin.pin_type == PinType::Output && pin.name == "model")
        })
        .and_then(|pin| pin.schema.clone())
        .expect("findModel declares its Bit schema");
    let mut board = board_from(
        r#"eventsSimple pick() {
    log::info({ message: "start", toast: false })
}
"#,
    )
    .await;
    apply_commands(
        &mut board,
        vec![BoardCommand::CreateVariable {
            variable_id: Some("var_model".into()),
            name: "model".into(),
            data_type: "Struct".into(),
            value_type: "Normal".into(),
            default_value: None,
            description: None,
            category: None,
            schema: Some(bit_schema),
            exposed: None,
            secret: None,
            editable: None,
            runtime_configured: None,
            target_layer: None,
            summary: None,
        }],
    )
    .await;
    let text = anchored(&board);
    let start_line = text
        .lines()
        .find(|line| line.contains("\"start\""))
        .expect("start renders")
        .to_string();
    let edited = text.replace(
        &start_line,
        &format!("{start_line}\n    model = ai::findModel({{}})"),
    );
    apply_edit(&mut board, &edited).await;
    assert_round_trips(&board);
}

/// Returning a parameter is carried by a reroute inside the Function layer (a layer cannot wire
/// its own boundary to itself). Splicing that reroute out of the text must keep the `return`.
#[tokio::test(flavor = "multi_thread")]
async fn a_function_returning_its_parameter_keeps_its_return() {
    let board = board_from(
        r#"function passThrough(value: string): (out: string) {
    return value
}

eventsSimple usePassThrough() {
    const out = passThrough("x")
    log::info({ message: out, toast: false })
}
"#,
    )
    .await;
    let text = anchored(&board);
    assert!(text.contains("return value"), "{text}");
    assert_round_trips(&board);
}

/// Reordering statements across a deleted one runs them in the written order, orphans nothing
/// and closes no loop.
#[tokio::test(flavor = "multi_thread")]
async fn reordering_across_a_removed_statement_follows_the_text() {
    let mut board = board_from(
        r#"eventsSimple reorder() {
    log::info({ message: "y", toast: false })
    log::info({ message: "r", toast: false })
    log::info({ message: "x", toast: false })
}
"#,
    )
    .await;
    let text = anchored(&board);
    let line = |message: &str| {
        text.lines()
            .find(|line| line.contains(&format!("\"{message}\"")))
            .expect("statement renders")
            .to_string()
    };
    let (y, x) = (line("y"), line("x"));
    let edited = text
        .replace(&line("r"), "")
        .replace(&y, "@@Y@@")
        .replace(&x, &y)
        .replace("@@Y@@", &x);
    apply_edit(&mut board, &edited).await;

    assert!(runs_before(&board, info(&board, "x"), info(&board, "y")));
    assert!(!runs_before(&board, info(&board, "y"), info(&board, "x")));
    assert_round_trips(&board);
}

/// Unwrapping an `if` that opens a Function body: the entry is the layer's own boundary.
#[tokio::test(flavor = "multi_thread")]
async fn unwrapping_an_if_at_the_start_of_a_function() {
    let mut board = board_from(
        r#"function report(): (done: bool) {
    if (1 > 0) {
        log::info({ message: "inside", toast: false })
    }
    log::info({ message: "after", toast: false })
    return true
}

eventsSimple callReport() {
    const done = report()
    log::info({ message: "called", toast: false })
}
"#,
    )
    .await;
    let edited = unwrap_block(&anchored(&board), "if (");
    apply_edit(&mut board, &edited).await;
    assert!(runs_before(
        &board,
        info(&board, "inside"),
        info(&board, "after")
    ));
    assert_round_trips(&board);
}

/// Unwrapping an if/else that closes a Function body: both arms fed the layer's `exec_out`, and
/// the text now runs them in sequence before it.
#[tokio::test(flavor = "multi_thread")]
async fn unwrapping_an_if_else_at_the_end_of_a_function() {
    let mut board = board_from(
        r#"function report() {
    log::info({ message: "before", toast: false })
    if (1 > 0) {
        log::info({ message: "yes", toast: false })
    } else {
        log::info({ message: "no", toast: false })
    }
}

eventsSimple callReport() {
    report()
    log::info({ message: "called", toast: false })
}
"#,
    )
    .await;
    let edited = unwrap_block(&anchored(&board), "if (");
    apply_edit(&mut board, &edited).await;
    assert!(runs_before(
        &board,
        info(&board, "before"),
        info(&board, "yes")
    ));
    assert!(runs_before(&board, info(&board, "yes"), info(&board, "no")));
    let exit = board
        .layers
        .values()
        .find(|layer| layer.name == "report")
        .and_then(|layer| {
            layer.pins.values().find(|pin| {
                pin.pin_type == PinType::Output && pin.data_type == VariableType::Execution
            })
        })
        .expect("report keeps its exit");
    assert!(
        exec_pin(info(&board, "no"), PinType::Output)
            .connected_to
            .contains(&exit.id),
        "the last statement still leads out of the function"
    );
    assert_round_trips(&board);
}

/// Moving a statement behind an unwrapped if/else at the end of a Function makes it the new last
/// statement: it, not the arms, leads out of the function.
#[tokio::test(flavor = "multi_thread")]
async fn a_statement_moved_last_in_a_function_leads_out_of_it() {
    let mut board = board_from(
        r#"function report() {
    log::info({ message: "before", toast: false })
    if (1 > 0) {
        log::info({ message: "yes", toast: false })
    } else {
        log::info({ message: "no", toast: false })
    }
}

eventsSimple callReport() {
    report()
    log::info({ message: "called", toast: false })
}
"#,
    )
    .await;
    let text = unwrap_block(&anchored(&board), "if (");
    let before = text
        .lines()
        .find(|line| line.contains("\"before\""))
        .expect("before is rendered")
        .to_string();
    let no = text
        .lines()
        .find(|line| line.contains("\"no\""))
        .expect("no is rendered")
        .to_string();
    let edited = without_line(&text, "\"before\"").replace(&no, &format!("{no}\n{before}"));
    apply_edit(&mut board, &edited).await;
    assert!(runs_before(&board, info(&board, "yes"), info(&board, "no")));
    assert!(runs_before(
        &board,
        info(&board, "no"),
        info(&board, "before")
    ));
    let exit = board
        .layers
        .values()
        .find(|layer| layer.name == "report")
        .and_then(|layer| {
            layer.pins.values().find(|pin| {
                pin.pin_type == PinType::Output && pin.data_type == VariableType::Execution
            })
        })
        .expect("report keeps its exit");
    assert!(
        exec_pin(info(&board, "before"), PinType::Output)
            .connected_to
            .contains(&exit.id),
        "the statement written last leads out of the function"
    );
    assert_round_trips(&board);
}

/// A loop body's tail has no execution successor on the board. Unwrapping the loop and writing that
/// statement last must still lead it out of the function.
#[tokio::test(flavor = "multi_thread")]
async fn a_former_loop_tail_written_last_leads_out_of_the_function() {
    let mut board = board_from(
        r#"function report() {
    for (const _ of ["a", "b"]) {
        log::info({ message: "inside", toast: false })
    }
    log::info({ message: "after", toast: false })
}

eventsSimple callReport() {
    report()
    log::info({ message: "called", toast: false })
}
"#,
    )
    .await;
    let text = anchored(&board);
    let edited = replace_body(
        &text,
        "function report()",
        &[body_line(&text, "after"), body_line(&text, "inside")],
    );
    apply_edit(&mut board, &edited).await;
    assert!(runs_before(
        &board,
        info(&board, "after"),
        info(&board, "inside")
    ));
    assert!(
        exec_pin(info(&board, "inside"), PinType::Output)
            .connected_to
            .contains(&function_exit(&board, "report").id)
    );
    assert_round_trips(&board);
}

/// A re-join supersedes the exit's old tail, and the plan re-targets the only other statement that
/// fed it: the exit is left with nothing, which must be refused rather than applied as a loop.
#[tokio::test(flavor = "multi_thread")]
async fn a_reorder_that_leaves_the_function_exit_unfed_is_refused() {
    let board = board_from(
        r#"function report() {
    log::info({ message: "first", toast: false })
    log::info({ message: "second", toast: false })
    if (1 > 0) {
        log::info({ message: "yes", toast: false })
    } else {
        log::info({ message: "no", toast: false })
    }
}

eventsSimple callReport() {
    report()
    log::info({ message: "called", toast: false })
}
"#,
    )
    .await;
    let text = anchored(&board);
    let edited = replace_body(
        &text,
        "function report()",
        &[
            body_line(&text, "second"),
            body_line(&text, "yes"),
            body_line(&text, "no"),
            "    info({ message: \"new\", toast: false })".to_string(),
            body_line(&text, "first"),
        ],
    );
    let diagnostics = reconcile_diagnostics(&board, &edited);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("exit of Function `report`")),
        "{diagnostics:?}\n{edited}"
    );
}

/// The removed node's surviving successor was also fed by a statement the plan now re-targets:
/// counting that stale edge as "still fed" silently orphaned it.
#[tokio::test(flavor = "multi_thread")]
async fn a_successor_fed_only_by_a_retargeted_statement_is_not_orphaned_silently() {
    let board = board_from(
        r#"function report() {
    if (1 > 0) {
        log::info({ message: "then", toast: false })
        log::info({ message: "removed", toast: false })
    } else {
        log::info({ message: "otherwise", toast: false })
    }
    log::info({ message: "joined", toast: false })
}

eventsSimple callReport() {
    report()
    log::info({ message: "called", toast: false })
}
"#,
    )
    .await;
    let text = anchored(&board);
    let edited = replace_body(
        &text,
        "function report()",
        &[
            body_line(&text, "joined"),
            body_line(&text, "otherwise"),
            "    info({ message: \"new\", toast: false })".to_string(),
            body_line(&text, "then"),
        ],
    );
    let diagnostics = reconcile_diagnostics(&board, &edited);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("no execution predecessor")),
        "{diagnostics:?}\n{edited}"
    );
}

/// `tools:` is only a function-reference list when the callee has no `tools` input. On a node that
/// has one, a computed `[tool]` is an `array_push` that needs the function's execution chain.
#[tokio::test(flavor = "multi_thread")]
async fn a_computed_tools_array_on_a_real_tools_input_keeps_its_function_impure() {
    let board = board_from(
        r#"function mcpFor(tool: string): (serverConfig: Struct) {
    const serverConfig = github::copilot::mcpLocalServer({ command: "npx", tools: [tool] })
    return serverConfig
}

eventsSimple useMcp() {
    const serverConfig = mcpFor("search")
    log::info({ message: "configured", toast: false })
}
"#,
    )
    .await;
    let push = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "array_push")
        .expect("the computed singleton plans an array_push");
    assert!(
        !exec_pin(push, PinType::Input).depends_on.is_empty(),
        "the array_push runs inside the function's execution chain"
    );
    assert_round_trips(&board);
}
