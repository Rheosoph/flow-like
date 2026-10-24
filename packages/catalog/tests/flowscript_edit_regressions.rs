//! Edits people made in the FlowScript editor that production apply-failure telemetry showed the
//! pipeline rejecting or mishandling. Each test builds a board through the product apply path,
//! renders it the way the editor does (anchors on), makes the edit, and checks both the apply and
//! that the result renders back as a no-op.
//!
//! The text-only spellings from the same captures live in
//! `tests/ast/handwritten/t1-editor-telemetry-spellings.flow`.

mod flowscript_support;

use flow_like::flow::ast::{
    ApplyFlowScriptResult, ReconcileMode, ReconcileOptions, RenderOptions,
    apply_board_commands_to_board, apply_flowscript_to_board, board_to_flowscript,
    reconcile_text_with_catalog_enriched, reconcile_text_with_catalog_enriched_opts,
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

async fn bend_execution_wire(board: &mut Board, from_id: &str, to_id: &str, count: usize) {
    let nodes = all_nodes(board);
    let from = nodes.iter().find(|node| node.id == from_id).unwrap();
    let to = nodes.iter().find(|node| node.id == to_id).unwrap();
    let input = exec_pin(to, PinType::Input);
    let output = from
        .pins
        .values()
        .find(|pin| {
            pin.pin_type == PinType::Output
                && pin.data_type == VariableType::Execution
                && pin.connected_to.contains(&input.id)
        })
        .expect("the execution wire exists");
    let mut commands = vec![BoardCommand::DisconnectPins {
        from_node: from_id.into(),
        from_pin: output.name.clone(),
        to_node: to_id.into(),
        to_pin: input.name.clone(),
        summary: None,
    }];
    let (mut previous_node, mut previous_pin) = (from_id.to_string(), output.name.clone());
    for index in 0..count {
        let reroute = format!("$bend{index}");
        commands.push(BoardCommand::AddNode {
            node_type: "reroute".into(),
            ref_id: Some(reroute.clone()),
            position: Some(NodePosition { x: 0.0, y: 0.0 }),
            friendly_name: None,
            additional_pins: None,
            target_layer: None,
            summary: None,
        });
        commands.push(BoardCommand::ConnectPins {
            from_node: previous_node,
            from_pin: previous_pin,
            to_node: reroute.clone(),
            to_pin: "route_in".into(),
            summary: None,
        });
        previous_node = reroute;
        previous_pin = "route_out".into();
    }
    commands.push(BoardCommand::ConnectPins {
        from_node: previous_node,
        from_pin: previous_pin,
        to_node: to_id.into(),
        to_pin: input.name.clone(),
        summary: None,
    });
    apply_commands(board, commands).await;
}

/// Reroutes on both sides of a removed statement must resolve to the surviving statements.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_or_replacing_a_statement_splices_through_execution_reroutes() {
    for replace in [false, true] {
        let mut board = board_from(
            r#"eventsSimple bent() {
    log::info({ message: "before", toast: false })
    log::info({ message: "removed", toast: false })
    log::info({ message: "after", toast: false })
}
"#,
        )
        .await;
        let before = info(&board, "before").id.clone();
        let removed = info(&board, "removed").id.clone();
        let after = info(&board, "after").id.clone();
        bend_execution_wire(&mut board, &before, &removed, 2).await;
        bend_execution_wire(&mut board, &removed, &after, 2).await;
        let text = anchored(&board);
        assert!(!text.contains("reroute"), "{text}");
        assert_round_trips(&board);
        let edited = if replace {
            text.replace(
                line_with(&text, "\"removed\""),
                "    log::info({ message: \"replacement\", toast: false })",
            )
        } else {
            without_line(&text, "\"removed\"")
        };

        apply_edit(&mut board, &edited).await;

        assert!(!all_nodes(&board).iter().any(|node| node.id == removed));
        assert!(!all_nodes(&board).iter().any(|node| node.name == "reroute"));
        let next = if replace { "replacement" } else { "after" };
        assert!(runs_before(
            &board,
            info(&board, "before"),
            info(&board, next)
        ));
        if replace {
            assert!(runs_before(
                &board,
                info(&board, "replacement"),
                info(&board, "after")
            ));
        }
        assert_round_trips(&board);
    }
}

/// Branch-arm reroutes must not become extra successors when the branch is unwrapped.
#[tokio::test(flavor = "multi_thread")]
async fn unwrapping_an_if_else_splices_through_execution_reroutes() {
    let mut board = board_from(
        r#"eventsSimple bentBranch() {
    log::info({ message: "before", toast: false })
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
    let before = info(&board, "before").id.clone();
    let yes = info(&board, "yes").id.clone();
    let no = info(&board, "no").id.clone();
    let branch = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "control_branch")
        .expect("the if is a branch")
        .id
        .clone();
    bend_execution_wire(&mut board, &before, &branch, 2).await;
    bend_execution_wire(&mut board, &branch, &yes, 2).await;
    bend_execution_wire(&mut board, &branch, &no, 2).await;
    assert_round_trips(&board);
    let edited = unwrap_block(&anchored(&board), "if (");

    apply_edit(&mut board, &edited).await;

    assert!(!all_nodes(&board).iter().any(|node| node.id == branch));
    assert!(!all_nodes(&board).iter().any(|node| node.name == "reroute"));
    for (from, to) in [("before", "yes"), ("yes", "no"), ("no", "after")] {
        assert!(runs_before(&board, info(&board, from), info(&board, to)));
    }
    assert_round_trips(&board);
}

/// Replacing a body removes its inline data producers, including unused reads of deleted variables.
#[tokio::test(flavor = "multi_thread")]
async fn replacing_an_event_body_removes_orphaned_expression_chains() {
    let mut board = board_from(
        r#"let oldValue: string = "old"

eventsSimple replaceBody() {
    log::info({ message: oldValue.toUpper().toLower(), toast: false })
    log::info({ message: json::stringify({ value: struct::make(), pretty: false }), toast: false })
}
"#,
    )
    .await;
    let variable = variables_named(&board, "oldValue").pop().unwrap();
    apply_commands(
        &mut board,
        vec![
            BoardCommand::AddNode {
                node_type: "variable_get".into(),
                ref_id: Some("$unusedRead".into()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::UpdateNodePin {
                node_id: "$unusedRead".into(),
                pin_id: "var_ref".into(),
                value: serde_json::json!(variable),
                summary: None,
            },
        ],
    )
    .await;
    let expressions: Vec<String> = all_nodes(&board)
        .into_iter()
        .filter(|node| {
            matches!(
                node.name.as_str(),
                "variable_get"
                    | "string_to_upper"
                    | "string_to_lower"
                    | "val_to_string"
                    | "struct_make"
            )
        })
        .map(|node| node.id.clone())
        .collect();
    assert_eq!(expressions.len(), 6, "both reads and four pure helpers");
    let text = anchored(&board);
    for id in &expressions {
        assert!(!text.contains(&format!("//@n:{id}")), "{text}");
    }
    let edited = without_line(
        &replace_body(
            &text,
            "eventsSimple replaceBody(",
            &["    log::info({ message: \"replacement\", toast: false })".into()],
        ),
        &format!("//@v:{variable}"),
    );

    apply_edit(&mut board, &edited).await;

    assert!(!board.variables.contains_key(&variable));
    for id in expressions {
        assert!(
            !all_nodes(&board).iter().any(|node| node.id == id),
            "orphaned expression {id} survived"
        );
    }
    info(&board, "replacement");
    assert_round_trips(&board);
}

/// An inline producer shared with another event stays wired there; unrelated canvas work stays too.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_consumer_preserves_shared_and_unrelated_expressions() {
    let mut board = board_from(
        r#"let sharedValue: string = "shared"

eventsSimple replaceBody() {
    log::info({ message: sharedValue.toUpper(), toast: false })
}

eventsSimple keepBody() {
    log::info({ message: "keep", toast: false })
}
"#,
    )
    .await;
    let upper = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "string_to_upper")
        .unwrap()
        .id
        .clone();
    let getter = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "variable_get")
        .unwrap()
        .id
        .clone();
    let kept_consumer = info(&board, "keep").id.clone();
    apply_commands(
        &mut board,
        vec![
            BoardCommand::ConnectPins {
                from_node: upper.clone(),
                from_pin: "uppercase_string".into(),
                to_node: kept_consumer.clone(),
                to_pin: "message".into(),
                summary: None,
            },
            BoardCommand::AddNode {
                node_type: "struct_make".into(),
                ref_id: Some("$unrelated".into()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
        ],
    )
    .await;
    let unrelated = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "struct_make")
        .unwrap()
        .id
        .clone();
    assert_round_trips(&board);
    let edited = replace_body(
        &anchored(&board),
        "eventsSimple replaceBody(",
        &["    log::info({ message: \"replacement\", toast: false })".into()],
    );

    apply_edit(&mut board, &edited).await;

    for id in [&upper, &getter, &unrelated, &kept_consumer] {
        assert!(
            all_nodes(&board).iter().any(|node| &node.id == id),
            "unaffected node {id} lost its identity"
        );
    }
    let nodes = all_nodes(&board);
    let producer = nodes.iter().find(|node| node.id == upper).unwrap();
    let consumer = nodes.iter().find(|node| node.id == kept_consumer).unwrap();
    let message = consumer
        .pins
        .values()
        .find(|pin| pin.name == "message")
        .unwrap();
    assert!(
        producer
            .pins
            .values()
            .any(|pin| { pin.name == "uppercase_string" && message.depends_on.contains(&pin.id) })
    );
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

/// Rename a Function layer's boundary pin in place, the way a canvas edit leaves it: FlowScript
/// cannot author two same-named boundary pins.
fn rename_function_pin(board: &mut Board, function: &str, from: &str, to: &str) {
    let pin = board
        .layers
        .values_mut()
        .find(|layer| layer.name == function)
        .and_then(|layer| layer.pins.values_mut().find(|pin| pin.name == from))
        .unwrap_or_else(|| panic!("function {function} has no pin {from}"));
    pin.name = to.to_string();
    pin.friendly_name = to.to_string();
}

/// "duplicate FlowScript function return `samples`": a Function layer with two outputs named alike
/// rendered both as `samples`, and the preflight refused every edit of the board.
#[tokio::test(flavor = "multi_thread")]
async fn same_named_function_returns_render_distinct_names() {
    let mut board = board_from(
        r#"function pair(first: string, second: string): (samples: string, picks: string) {
    return first.toUpper(), second.trim()
}

eventsSimple tick() {
    log::info({ message: "before", toast: false })
}
"#,
    )
    .await;
    rename_function_pin(&mut board, "pair", "picks", "samples");
    let text = anchored(&board);
    assert!(
        text.contains("(samples: string, samples2: string)"),
        "{text}"
    );
    assert_round_trips(&board);
    apply_edit(&mut board, &text.replace("\"before\"", "\"after\"")).await;
    info(&board, "after");
    assert_round_trips(&board);
}

/// "source `child.path` is `Struct/Normal`, but input `p` requires `String/Normal`": the FlowPath
/// field read rendered as `child(...).path`, which reconcile reads as the `path` OUTPUT PIN. The
/// board's own round trip survived on its existing wiring; the same text anywhere else did not.
#[tokio::test(flavor = "multi_thread")]
async fn a_field_read_named_like_an_output_pin_renders_the_pin_explicitly() {
    let board = board_from(
        r#"function storagePath(): (p: string) {
    return files::uploadDir().child("Rechnungen").path.path
}

eventsSimple go() {
    const p = storagePath()
    log::info({ message: p, toast: false })
}
"#,
    )
    .await;
    let text = anchored(&board);
    assert!(
        text.contains(r#"files::uploadDir().child("Rechnungen").path.path"#),
        "{text}"
    );
    assert_round_trips(&board);
    assert_round_trips(&board_from(&text).await);

    let empty = Board::new_detached(Some("edit-regressions".into()), Path::default());
    let pin_read = text.replace(".path.path", ".path");
    let diagnostics = reconcile_diagnostics(&empty, &pin_read);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("read the field with `.path.path`")),
        "{diagnostics:?}"
    );
}

/// "assignment to `changed` carries node anchor …, but its value is not an anchored call or
/// variable-set operation": a `let` in a module's event lives on the module layer, and neither the
/// board document nor the module's own file declared it again. Written by two events, it is
/// declared once; the other event's anchored assignment resolves through the module scope.
#[tokio::test(flavor = "multi_thread")]
async fn a_module_layer_variable_is_declared_and_seen_by_every_section_of_its_module() {
    let mut board = board_from(
        r#"module lea {
    eventsSimple leaToggle() {
        let changed: string = ""
        changed = "a"
        log::info({ message: changed, toast: false })
    }
}
"#,
    )
    .await;
    let (module_id, variable_id) = board
        .layers
        .values()
        .find(|layer| layer.name == "lea" && layer.variables.len() == 1)
        .and_then(|layer| Some((layer.id.clone(), layer.variables.keys().next()?.clone())))
        .expect("the event `let` lives on module `lea`");
    let text = anchored(&board);
    assert!(text.contains("let changed: string"), "{text}");
    assert_round_trips(&board);

    let close = text.rfind("\n}").expect("the module block closes");
    let edited = format!(
        "{}\n    eventsSimple leaReset() {{\n        let changed: string = \"\"\n        changed = \"reset\"\n        log::info({{ message: changed, toast: false }})\n    }}{}",
        &text[..close],
        &text[close..]
    );
    apply_edit(&mut board, &edited).await;
    assert_eq!(writers_of(&board, &variable_id), 2, "both events write it");
    let text = anchored(&board);
    assert_eq!(text.matches("let changed: string").count(), 1, "{text}");
    assert_round_trips(&board);

    let file = apply_module_file_edit(&mut board, &module_id, "\"reset\"", "\"cleared\"").await;
    assert_eq!(file.matches("let changed: string").count(), 1, "{file}");
    assert_eq!(writers_of(&board, &variable_id), 2, "both events write it");
    assert_round_trips(&board);
}

/// A board variable used in a template must feed the format node's dynamic placeholder pin.
#[tokio::test(flavor = "multi_thread")]
async fn a_template_placeholder_reads_its_board_variable() {
    fn assert_greeting_wired(board: &Board) {
        let variable = board
            .variables
            .values()
            .find(|variable| variable.name == "greeting")
            .expect("greeting is a board variable");
        let nodes = all_nodes(board);
        let getters: Vec<_> = nodes
            .iter()
            .filter(|node| node.name == "variable_get")
            .collect();
        assert_eq!(getters.len(), 1, "the template needs one variable getter");
        let getter = getters[0];
        let var_ref = getter
            .get_pin_by_name("var_ref")
            .expect("variable reference");
        let expected_ref = format!("\"{}\"", variable.id);
        assert_eq!(
            var_ref.default_value.as_deref(),
            Some(expected_ref.as_bytes())
        );
        let output = getter.get_pin_by_name("value_ref").expect("variable value");
        let formats: Vec<_> = nodes
            .iter()
            .filter(|node| node.name == "string_format")
            .collect();
        assert_eq!(formats.len(), 1, "the template needs one format node");
        let placeholder = formats[0]
            .get_pin_by_name("greeting")
            .expect("the format node has a greeting placeholder");
        assert_eq!(placeholder.depends_on.len(), 1);
        assert!(placeholder.depends_on.contains(&output.id));
        assert_eq!(output.connected_to.len(), 1);
        assert!(output.connected_to.contains(&placeholder.id));
    }

    for (existing_variable, body) in [
        (
            false,
            r#"log::info({ message: `${greeting} x`, toast: false })"#,
        ),
        (
            true,
            r#"const message = `${greeting} x ${greeting}`
    log::info({ message: message, toast: false })"#,
        ),
    ] {
        let source =
            format!("const greeting = \"hi\"\n\neventsSimple greet() {{\n    {body}\n}}\n");
        let mut board = if existing_variable {
            board_from("const greeting = \"hi\"\n").await
        } else {
            Board::new_detached(Some("template-variable".into()), Path::default())
        };
        apply_edit(&mut board, &source).await;
        assert_greeting_wired(&board);
        assert_round_trips(&board);

        let text = anchored(&board);
        apply_edit(&mut board, &text).await;
        assert_greeting_wired(&board);
        assert_round_trips(&board);

        let copy = board_from(&anchored(&board)).await;
        assert_greeting_wired(&copy);
        assert_round_trips(&copy);
    }
}

/// A missing format placeholder must reject Apply without publishing the staged board.
#[tokio::test(flavor = "multi_thread")]
async fn a_format_placeholder_without_a_value_rejects_apply() {
    const VALID_SOURCE: &str = r#"const greeting = "hi"

eventsSimple greet() {
    log::info({ message: `${greeting} x`, toast: false })
}
"#;
    const INCOMPLETE_FORMAT: &str = r#"string::format({ formatString: "{greeting} x" })"#;
    const INCOMPLETE_EDIT: &str =
        r#"string::format({ formatString: "{greeting} x {missing}", greeting: greeting })"#;

    for existing_template in [false, true] {
        let mut board = if existing_template {
            board_from(VALID_SOURCE).await
        } else {
            Board::new_detached(Some("incomplete-template".into()), Path::default())
        };
        let source = if existing_template {
            anchored(&board)
        } else {
            VALID_SOURCE.to_string()
        };
        let (replacement, missing_placeholder) = if existing_template {
            (INCOMPLETE_EDIT, "missing")
        } else {
            (INCOMPLETE_FORMAT, "greeting")
        };
        let incomplete = source.replace("`${greeting} x`", replacement);
        assert_ne!(
            source, incomplete,
            "the edit introduces a missing placeholder value"
        );
        let original = flow_like_types::json::to_value(&board).expect("serialize original board");
        let applied = apply_flowscript_to_board(
            &mut board,
            &incomplete,
            &CATALOG.nodes,
            catalog_state().await,
            None,
            true,
        )
        .await;
        let rejection = match applied {
            Err(error) => error.to_string(),
            Ok(result) => {
                assert!(result.commands.is_empty(), "invalid formats must not apply");
                assert!(
                    !result.diagnostics.is_empty(),
                    "invalid formats must diagnose"
                );
                result.diagnostics.join("\n")
            }
        };
        assert!(rejection.contains(missing_placeholder), "{rejection}");
        assert!(
            rejection.contains("missing required inputs")
                || rejection.contains("no value or connection"),
            "{rejection}"
        );
        assert_eq!(
            flow_like_types::json::to_value(&board).expect("serialize board after rejection"),
            original,
            "rejected Apply must preserve the complete original board"
        );
    }

    let mut board = board_from(VALID_SOURCE).await;
    let original = flow_like_types::json::to_value(&board).expect("serialize original board");
    let nodes = all_nodes(&board);
    let getter = nodes
        .iter()
        .find(|node| node.name == "variable_get")
        .expect("the valid template has a variable getter");
    let format = nodes
        .iter()
        .find(|node| node.name == "string_format")
        .expect("the valid template has a format node");
    let disconnect = BoardCommand::DisconnectPins {
        from_node: getter.id.clone(),
        from_pin: "value_ref".into(),
        to_node: format.id.clone(),
        to_pin: "greeting".into(),
        summary: None,
    };
    let rejection = apply_board_commands_to_board(
        &mut board,
        vec![disconnect],
        &CATALOG.nodes,
        catalog_state().await,
        None,
    )
    .await
    .err()
    .expect("disconnecting a template placeholder must reject Apply")
    .to_string();
    assert!(rejection.contains("greeting"), "{rejection}");
    assert!(rejection.contains("no value or connection"), "{rejection}");
    assert_eq!(
        flow_like_types::json::to_value(&board).expect("serialize board after rejection"),
        original,
        "a rejected disconnection must preserve the original placeholder wire"
    );
}

/// How many `variable_set` nodes write variable `variable_id`.
fn writers_of(board: &Board, variable_id: &str) -> usize {
    let expected = format!("\"{variable_id}\"");
    all_nodes(board)
        .into_iter()
        .filter(|node| {
            node.name == "variable_set"
                && node.pins.values().any(|pin| {
                    pin.name == "var_ref"
                        && pin.default_value.as_deref() == Some(expected.as_bytes())
                })
        })
        .count()
}

/// Render module `module_id`'s own file, replace `from` with `to` in it, and apply it the way the
/// editor applies a module file. Returns the file as rendered before the edit.
async fn apply_module_file_edit(
    board: &mut Board,
    module_id: &str,
    from: &str,
    to: &str,
) -> String {
    let file = flow_like::flow::ast::FlowScriptFile::Module(module_id.to_string());
    let rendered = flow_like::flow::ast::board_to_flowscript_file(
        board,
        &file,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    )
    .expect("the module file renders");
    let result = flow_like::flow::ast::apply_flowscript_to_board_file(
        board,
        &rendered.text.replace(from, to),
        &CATALOG.nodes,
        catalog_state().await,
        Some(module_id.to_string()),
        false,
        Some(&rendered.scope_anchors),
        Some(file),
    )
    .await
    .expect("the module file applies");
    assert!(
        result.diagnostics.is_empty(),
        "{:?}\n{}",
        result.diagnostics,
        rendered.text
    );
    rendered.text
}

/// A board variable with a CUID-style id, as the variables panel creates it.
fn create_variable(id: &str, name: &str, data_type: &str, value_type: &str) -> BoardCommand {
    BoardCommand::CreateVariable {
        variable_id: Some(id.into()),
        name: name.into(),
        data_type: data_type.into(),
        value_type: value_type.into(),
        default_value: None,
        description: None,
        category: None,
        schema: None,
        exposed: None,
        secret: None,
        editable: None,
        runtime_configured: None,
        target_layer: None,
        summary: None,
    }
}

fn assert_text_reconciles_to_nothing(board: &Board, text: &str) {
    let (catalog, enricher) = catalog();
    let result = reconcile_text_with_catalog_enriched(board, text, &catalog, &enricher);
    assert!(
        result.commands.is_empty() && result.diagnostics.is_empty(),
        "{:?}\n{:?}\n{text}",
        result.commands,
        result.diagnostics
    );
}

/// "variable reference `m5fr…` does not resolve to a board or FlowScript variable":
/// `array::pushRef` spelled its variable by id, and an id means nothing on the board the text is
/// pasted into, although the text declares the variable.
#[tokio::test(flavor = "multi_thread")]
async fn a_var_ref_literal_renders_the_variable_name_and_survives_a_copy() {
    let mut board = Board::new_detached(Some("edit-regressions".into()), Path::default());
    apply_commands(
        &mut board,
        vec![create_variable(
            "m5fr5na9m1poac41clty7rk6",
            "committedPaths",
            "String",
            "Array",
        )],
    )
    .await;
    apply_edit(
        &mut board,
        r#"const committedPaths: string[]   //@v:m5fr5na9m1poac41clty7rk6

eventsSimple collect() {
    array::pushRef({ varRef: "committedPaths", value: "x" })
}
"#,
    )
    .await;

    let text = anchored(&board);
    assert!(text.contains(r#"varRef: "committedPaths""#), "{text}");
    assert_round_trips(&board);
    let by_id = text.replace(
        r#"varRef: "committedPaths""#,
        r#"varRef: "m5fr5na9m1poac41clty7rk6""#,
    );
    assert_text_reconciles_to_nothing(&board, &by_id);

    let copy = board_from(&text).await;
    let copied = copy
        .variables
        .values()
        .find(|variable| variable.name == "committedPaths")
        .expect("the copy declares committedPaths");
    let push = all_nodes(&copy)
        .into_iter()
        .find(|node| node.name == "array_push_ref")
        .expect("pushRef is copied");
    let expected = format!("\"{}\"", copied.id);
    assert!(push.pins.values().any(|pin| {
        pin.name == "var_ref" && pin.default_value.as_deref() == Some(expected.as_bytes())
    }));
    assert_round_trips(&copy);
}

/// "variable reference `m8db…` does not resolve to a board or FlowScript variable": three
/// `variable::set` nodes still held the ids of deleted variables, and those unchanged lines blocked
/// every edit of the board.
#[tokio::test(flavor = "multi_thread")]
async fn an_unchanged_set_of_a_deleted_variable_does_not_block_edits() {
    let mut board = board_from(
        r#"let note: string = ""

eventsSimple tick() {
    note = "a"
    log::info({ message: "before", toast: false })
}
"#,
    )
    .await;
    let variable_id = board
        .variables
        .keys()
        .next()
        .cloned()
        .expect("note is declared");
    apply_commands(
        &mut board,
        vec![BoardCommand::RemoveVariable {
            variable_id: variable_id.clone(),
            summary: None,
        }],
    )
    .await;
    let text = anchored(&board);
    assert!(
        text.contains(&format!("varRef: \"{variable_id}\"")),
        "{text}"
    );
    assert_round_trips(&board);
    let edited = text.replace("\"before\"", "\"after\"");
    apply_edit(&mut board, &edited).await;
    info(&board, "after");
    assert_round_trips(&board);

    let changed = anchored(&board).replace(&variable_id, "gone");
    let diagnostics = reconcile_diagnostics(&board, &changed);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("variable reference `gone` does not resolve")),
        "{diagnostics:?}"
    );
}

/// A `struct::get` whose field is a path rendered as `alert.asset.hostname`, which reads back as
/// one accessor per segment: the board's own text re-planned two nodes on every apply.
#[tokio::test(flavor = "multi_thread")]
async fn a_struct_get_of_a_field_path_keeps_its_call_form() {
    let board = board_from(
        r#"function host(alert: Struct): (hostname: string) {
    return struct::get({ struct: alert, field: "asset.hostname" }).value
}

eventsGeneric go(payload: Struct) {
    const hostname = host(payload)
    log::info({ message: hostname, toast: false })
}
"#,
    )
    .await;
    let text = anchored(&board);
    assert!(
        text.contains(r#""asset.hostname""#) && !text.contains(".asset.hostname"),
        "{text}"
    );
    assert_round_trips(&board);
}

fn reconcile_on_empty_board(source: &str) -> Vec<String> {
    let board = Board::new_detached(Some("edit-regressions".into()), Path::default());
    let (catalog, enricher) = catalog();
    reconcile_text_with_catalog_enriched(&board, source, &catalog, &enricher).diagnostics
}

/// "binary operator `==` has ambiguous operand type … Make one side concrete": the author typed
/// the binding as the hint asked (`const requestedDraftId: string = actionContext.draftId`), but
/// the parser dropped the annotation and the same error came back.
#[tokio::test(flavor = "multi_thread")]
async fn a_typed_local_const_makes_an_untyped_field_concrete() {
    let board = board_from(
        r#"function f(p: Struct) {
    const a: string = p.x
    if (a == p.y) {
        log::info({ message: "same", toast: false })
    }
}

eventsGeneric go(payload: Struct) {
    f(payload)
}
"#,
    )
    .await;
    assert!(
        all_nodes(&board)
            .iter()
            .any(|node| node.name == "equal_string"),
        "the declared `string` selects the string comparison"
    );
    assert_round_trips(&board);

    let diagnostics = reconcile_on_empty_board(
        "eventsSimple tick() {\n    const label: int = string::trim({ string: \" x \" })\n    log::info({ message: label, toast: false })\n}\n",
    );
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic.contains(
            "`const label: int` does not match its value: `string::trim(...)` is `String`"
        )),
        "{diagnostics:?}"
    );
}

/// "none of the outputs of `db::filter` (`values` (Struct)) fits `data` (Struct)": Update Table's
/// `data` takes the rows array, but its catalog declaration said one Struct, and the hint printed
/// the array as a single Struct.
#[tokio::test(flavor = "multi_thread")]
async fn update_table_takes_the_rows_a_query_returns() {
    let rows = r#"    const { database } = db::open({ name: "t", userScoped: false, batchSize: 0 })
    const rows = database.filter({ limit: 200, offset: 0 })
"#;
    let board = board_from(&format!(
        "eventsGeneric go(payload: Struct) {{\n{rows}    ui::updateTable({{ elementRef: ui::getElement({{ elementRef: \"episode_table\" }}).element, operation: \"Set Data\", data: rows }})\n}}\n"
    ))
    .await;
    let table = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "a2ui_update_table")
        .expect("the update table node is planned");
    let data = table
        .pins
        .values()
        .find(|pin| pin.name == "data" && pin.pin_type == PinType::Input)
        .expect("Set Data has a data input");
    assert!(
        !data.depends_on.is_empty(),
        "the rows are wired into `data`"
    );
    assert_round_trips(&board);

    let diagnostics = reconcile_on_empty_board(&format!(
        "eventsGeneric go(payload: Struct) {{\n{rows}    ui::writeCsvToTable({{ elementRef: \"t\", csv: \"\", table: rows, delimiter: \",\" }})\n}}\n"
    ));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("(`values` (Struct[])) fits `table` (Struct)")),
        "{diagnostics:?}"
    );
}

/// "source `string_concat.concatenated` is `String/Normal`, but input `element_ref` requires
/// `Struct/Normal`": the element pins take an id string or an element object, yet only a typed-in
/// id was accepted.
#[tokio::test(flavor = "multi_thread")]
async fn element_nodes_accept_a_computed_element_id() {
    let board = board_from(
        r#"function f(pageId: string) {
    ui::setElementText({ elementRef: pageId + "/front", text: "x" })
    ui::setElementLoading({ elementRef: pageId + "/save", loading: true })
    ui::setButtonLabel({ elementRef: pageId + "/save", label: "Save" })
    ui::setElementDisabled({ elementRef: pageId + "/save", disabled: false })
    ui::setElementValue({ elementRef: pageId + "/name", value: "x" })
    ui::setInputPlaceholder({ elementRef: pageId + "/name", placeholder: "Name" })
    ui::setSelectValue({ elementRef: pageId + "/kind", value: "a" })
    ui::setChartLayout({ elementRef: pageId + "/chart", layout: { title: "t" } })
    ui::setElementAction({ elementRef: pageId + "/link", route: "/next", queryParams: "", url: "", nodeId: "" })
    if (ui::getFileInputFiles({ elementRef: pageId + "/upload" }).exists) {
        log::info({ message: "uploaded", toast: false })
    }
}

eventsGeneric go(payload: Struct) {
    f("abc")
}
"#,
    )
    .await;
    for node_type in [
        "a2ui_set_element_text",
        "a2ui_set_element_loading",
        "a2ui_set_button_label",
        "a2ui_set_element_disabled",
        "a2ui_set_element_value",
        "a2ui_set_input_placeholder",
        "a2ui_set_select_value",
        "a2ui_set_chart_layout",
        "a2ui_set_element_action",
        "a2ui_get_file_input_files",
    ] {
        let node = all_nodes(&board)
            .into_iter()
            .find(|node| node.name == node_type)
            .unwrap_or_else(|| panic!("{node_type} is planned"));
        let element = node
            .pins
            .values()
            .find(|pin| pin.name == "element_ref")
            .expect("element_ref pin");
        assert!(
            !element.depends_on.is_empty(),
            "{node_type} takes the computed id"
        );
    }
    assert_round_trips(&board);
}

fn node_types_on(board: &Board) -> Vec<&str> {
    all_nodes(board)
        .into_iter()
        .map(|node| node.name.as_str())
        .collect()
}

/// "`session5.sqlQuery(...)` is ambiguous between `db::graph::sqlQuery`, `df::sqlQuery`": the
/// database, graph and DataFusion session handles are all `{cache_key}` and differ only by title.
/// A titled receiver dispatches on its own class whatever `use` opens; `use db::*` alone used to
/// wire a DataFusion session into `db::graph::sqlQuery` without a diagnostic.
#[tokio::test(flavor = "multi_thread")]
async fn equal_shaped_handles_dispatch_on_their_own_class() {
    let cases = [
        (
            "df_sql_query",
            "    const session = df::createSession({ sessionName: \"default\" })\n    const { rows } = session.sqlQuery({ query: \"SELECT 1\", params: {} })\n    log::info({ message: rows, toast: false })",
        ),
        (
            "schema_local_db",
            "    const { database } = db::open({ name: \"x\", userScoped: false })\n    const tableSchema = database.schema()\n    log::info({ message: tableSchema, toast: false })",
        ),
        (
            "graph_schema",
            "    const { graph } = db::graph::openOverlay({ overlayId: \"g\" })\n    const graphSchema = graph.schema()\n    log::info({ message: graphSchema, toast: false })",
        ),
    ];
    for (node_type, body) in cases {
        for uses in ["", "use db::*\n\n", "use db::*\nuse df::*\n\n"] {
            let board = board_from(&format!("{uses}eventsSimple query() {{\n{body}\n}}\n")).await;
            let types = node_types_on(&board);
            assert!(
                types.contains(&node_type),
                "{node_type} with {uses:?}: {types:?}"
            );
            assert_round_trips(&board);
        }
    }
}

/// The headline example of `studio/flowscript.md`: with only `use db::*` open, a DataFusion
/// session's `sqlQuery` is `df::sqlQuery`.
#[tokio::test(flavor = "multi_thread")]
async fn the_documented_session_query_runs_on_datafusion() {
    let board = board_from(
        r#"use db::*

eventsSimple clickLatestOverview() {
    const database = open({ name: "report_overview", userScoped: true, batchSize: 1000 })
    const session = df::createSession({ sessionName: "default" })
    session.registerLance({ database: database, tableName: "reports" })
    const { rows } = session.sqlQuery({ query: "SELECT report_id FROM reports ORDER BY created DESC LIMIT 1;" })
    log::info({ message: rows, toast: false })
}
"#,
    )
    .await;
    let types = node_types_on(&board);
    assert!(types.contains(&"df_sql_query"), "{types:?}");
    assert!(!types.contains(&"graph_sql_query"), "{types:?}");
    assert_round_trips(&board);
}

/// "argument `string` on `equalString` … source `f.n` is `Integer/Normal`": a comparison whose
/// operands change type needs a comparator of the new type instead of the live one rewired.
#[tokio::test(flavor = "multi_thread")]
async fn a_comparison_whose_operands_change_type_gets_a_matching_comparator() {
    for (seeded, edited) in [
        (r#"s == "1""#, "n == 1"),
        (r#"(s == "a") && (s == "b")"#, r#"(s == "a") && (n == 1)"#),
    ] {
        let mut board = board_from(&format!(
            "function f(s: string, n: int): (ok: bool) {{\n    return {seeded}\n}}\n"
        ))
        .await;
        let text = replace_body(
            &anchored(&board),
            "function f(",
            &[format!("    return {edited}")],
        );
        apply_edit(&mut board, &text).await;
        let comparator = all_nodes(&board)
            .into_iter()
            .find(|node| node.name == "int_equal")
            .unwrap_or_else(|| panic!("{edited}: {:?}", node_types_on(&board)));
        assert!(
            comparator
                .pins
                .values()
                .any(|pin| pin.pin_type == PinType::Output && !pin.connected_to.is_empty()),
            "{edited}: the Integer comparison feeds the return"
        );
        assert_round_trips(&board);
    }
}

async fn board_with_chat_event() -> Board {
    let mut board = board_from(
        r#"eventsSimple seed() {
    log::info({ message: "seed", toast: false })
}
"#,
    )
    .await;
    apply_commands(
        &mut board,
        vec![BoardCommand::AddNode {
            node_type: "events_chat".into(),
            ref_id: None,
            position: None,
            friendly_name: None,
            additional_pins: None,
            target_layer: None,
            summary: None,
        }],
    )
    .await;
    board
}

/// "event `eventsChat` changes the parameter contract of anchored entry": a variable whose
/// interface is named `ToolCall` makes the render suffix the chat `History` family's `ToolCall2`.
/// `History` is recursive, and the board's own text must still match the live event.
#[tokio::test(flavor = "multi_thread")]
async fn a_chat_history_with_renamed_definitions_round_trips() {
    let mut board = board_with_chat_event().await;
    let call = r#"{"type":"object","properties":{"id":{"type":"string"},"extra":{"type":"integer"}},"required":["id","extra"]}"#;
    apply_commands(
        &mut board,
        vec![struct_variable("var_toolCall", "toolCall", call.into())],
    )
    .await;
    let text = anchored(&board);
    assert!(text.contains("interface ToolCall2"), "{text}");
    assert_round_trips(&board);
}

/// "argument `history` on `prep` … the source schema does not cover every field the input
/// declares": a function typed with the rendered `History` takes the chat event's own `history`,
/// whose `thinking` (and its tools' schema `type`) is an `Option<enum>`.
#[tokio::test(flavor = "multi_thread")]
async fn a_function_typed_with_the_rendered_history_takes_the_chat_history() {
    let mut board = board_with_chat_event().await;
    let text = anchored(&board);
    let chat = text
        .lines()
        .find(|line| line.starts_with("eventsChat "))
        .expect("the chat event renders")
        .to_string();
    let edited = text.replace(
        &chat,
        &format!(
            "{chat}\n    const ready = prep(history)\n    log::info({{ message: ready, toast: false }})"
        ),
    );
    let edited = format!(
        "function prep(history: History): (ready: bool) {{\n    return true\n}}\n\n{edited}"
    );
    apply_edit(&mut board, &edited).await;
    assert_round_trips(&board);
}

fn struct_variable(id: &str, name: &str, schema: String) -> BoardCommand {
    let mut command = create_variable(id, name, "Struct", "Normal");
    if let BoardCommand::CreateVariable { schema: slot, .. } = &mut command {
        *slot = Some(schema);
    }
    command
}

fn without_enum_values(schema: &mut flow_like_types::Value, removed: &[&str]) {
    use flow_like_types::Value;
    match schema {
        Value::Object(fields) => {
            if let Some(Value::Array(values)) = fields.get_mut("enum") {
                values.retain(|value| value.as_str().is_none_or(|value| !removed.contains(&value)));
            }
            fields
                .values_mut()
                .for_each(|field| without_enum_values(field, removed));
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| without_enum_values(item, removed)),
        _ => {}
    }
}

/// "assignment to variable … the source declares `Bit`, the input requires an untitled schema":
/// a `Bit` variable created before `BitTypes` gained `SystemOne` and `Forecasting` holds the old
/// enum. The board refreshes that copy, so `ai::findModel`'s current `Bit` assigns to it.
#[tokio::test(flavor = "multi_thread")]
async fn a_bit_variable_from_before_new_model_types_takes_find_model() {
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
    let mut stale: flow_like_types::Value =
        flow_like_types::json::from_str(&bit_schema).expect("Bit schema parses");
    without_enum_values(&mut stale, &["SystemOne", "Forecasting"]);
    let mut board = board_from(
        r#"eventsSimple pick() {
    log::info({ message: "start", toast: false })
}
"#,
    )
    .await;
    apply_commands(
        &mut board,
        vec![struct_variable("var_model", "model", stale.to_string())],
    )
    .await;
    let text = anchored(&board);
    assert!(text.contains("\"Forecasting\""), "{text}");
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

const HELPER_AND_CALLER: &str = r#"function helper() {
    log::info({ message: "inside", toast: false })
    log::info({ message: "inside2", toast: false })
}

eventsSimple run() {
    helper()
    log::info({ message: "after", toast: false })
}
"#;

/// Drop the line containing `header` through the `}` that closes it: the declaration deleted.
fn without_block(text: &str, header: &str) -> String {
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
    lines[..start]
        .iter()
        .chain(&lines[end + 1..])
        .map(|line| format!("{line}\n"))
        .collect()
}

/// "Cannot connect a node to itself": once a Function's whole body went (its statements deleted,
/// or the declaration together with its call), the re-join around the removed nodes wired the
/// layer's entry to its own exit and the apply rolled back.
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_whole_function_body_never_wires_the_layer_to_itself() {
    let log_count = |board: &Board| {
        node_types_on(board)
            .into_iter()
            .filter(|node_type| *node_type == "log_info")
            .count()
    };

    let mut board = board_from(HELPER_AND_CALLER).await;
    let emptied = replace_body(&anchored(&board), "function helper()", &[]);
    apply_edit(&mut board, &emptied).await;
    assert_eq!(log_count(&board), 1);
    assert!(node_types_on(&board).contains(&"control_call_function"));
    assert_round_trips(&board);

    let mut board = board_from(HELPER_AND_CALLER).await;
    let deleted = without_line(
        &without_block(&anchored(&board), "function helper()"),
        "helper()",
    );
    apply_edit(&mut board, &deleted).await;
    assert_eq!(log_count(&board), 1);
    assert!(!node_types_on(&board).contains(&"control_call_function"));
    let entry = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "events_simple")
        .expect("the event keeps its entry");
    assert!(runs_before(&board, entry, info(&board, "after")));
    assert_round_trips(&board);
}

/// Deleting a `function` the text still calls applied silently and failed only at run time
/// ("Function 'helper' has no node connected to its exec input"). A selection or a module file
/// that does not render the function is not deleting it.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_function_that_is_still_called_is_refused() {
    let board = board_from(HELPER_AND_CALLER).await;
    let diagnostics = reconcile_diagnostics(
        &board,
        &without_block(&anchored(&board), "function helper()"),
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("function `helper` is no longer declared")
                && diagnostic.contains("in event `run`")
        }),
        "{diagnostics:?}"
    );

    let board = board_from(&format!(
        "{HELPER_AND_CALLER}\neventsSimple other() {{\n    log::info({{ message: \"other\", toast: false }})\n}}\n"
    ))
    .await;
    let scoped = flow_like::flow::ast::board_to_flowscript_scoped(
        &board,
        &[info(&board, "other").id.clone()],
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    assert!(!scoped.text.contains("function helper"), "{}", scoped.text);
    let (catalog, enricher) = catalog();
    let diagnostics = reconcile_text_with_catalog_enriched_opts(
        &board,
        &scoped.text,
        &catalog,
        &enricher,
        &ReconcileOptions {
            scope_anchors: Some(&scoped.scope_anchors),
            ..ReconcileOptions::default()
        },
    )
    .diagnostics;
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let mut board = board_from(
        r#"function helper() {
    log::info({ message: "inside", toast: false })
}

module lea {
    eventsSimple leaToggle() {
        helper()
        log::info({ message: "lea", toast: false })
    }
}
"#,
    )
    .await;
    let module_id = board
        .layers
        .values()
        .find(|layer| layer.name == "lea")
        .map(|layer| layer.id.clone())
        .expect("module `lea` exists");
    apply_module_file_edit(&mut board, &module_id, "\"lea\"", "\"lea2\"").await;
}

const ROBOT_SHOWROOM: &str = r#"use log::*

eventsGeneric robotChat(payload: Struct, message: string) {
    info({ message: message, toast: false })
}

eventsGeneric robotHealth(payload: Struct) {
    info({ message: "ok", toast: false })
}

eventsSimple startRobotShowroom() {
    const config = rest::serverConfig({ host: "127.0.0.1", port: 38477, tls: {} })
    const chat = rest::registerFunction({ configIn: config, path: "/chat", method: "POST", fnRefs: [robotChat] })
    const health = rest::registerFunction({ configIn: chat, path: "/health", method: "GET", fnRefs: [robotHealth] })
    rest::server({ config: health })
}
"#;

fn event_entry_ids(board: &Board) -> Vec<String> {
    let mut ids: Vec<String> = all_nodes(board)
        .into_iter()
        .filter(|node| node.name.starts_with("events_"))
        .map(|node| node.id.clone())
        .collect();
    ids.sort();
    ids
}

/// "Node reference `robotChat` is ambiguous": re-applying a program written without anchors added
/// a second entry beside every live event, so `fnRefs: [robotChat]` named two entries and the
/// apply rolled back. Without references the duplicates simply stayed (`robotChat`, emptied, and
/// `robotChat2`).
#[tokio::test(flavor = "multi_thread")]
async fn reapplying_an_unanchored_program_keeps_its_event_entries() {
    let mut board = board_from(ROBOT_SHOWROOM).await;
    let entries = event_entry_ids(&board);
    apply_edit(&mut board, ROBOT_SHOWROOM).await;
    assert_eq!(event_entry_ids(&board), entries);
    assert_round_trips(&board);

    let changed_contract = ROBOT_SHOWROOM.replace(
        "robotChat(payload: Struct, message: string)",
        "robotChat(payload: Struct, message: string, topic: string)",
    );
    let diagnostics = reconcile_diagnostics(&board, &changed_contract);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("changes that entry's parameter contract")),
        "{diagnostics:?}"
    );
}

const POST_HELPER: &str = r#"use http::*
use log::*
@secret
const engineeringKey = ""
function post(path: string, body: Struct): (result: Struct) {
 const request = http::request({method:"POST",url:"http://127.0.0.1:38551/api"}).setHeader({name:"X-Rheosoph-Key",value:engineeringKey}).setStructBody({body:body})
 return request.fetch().toJson()
}
eventsSimple go() {
 info({message: post({path:"a", body:{}}), toast:false})
}
"#;

/// "Entity (…) not found; recovery errors: failed to rollback current command": re-applying an
/// unanchored function reused its old `fetch` and `toJson` for `request.fetch().toJson()`, nodes
/// the same apply removes, so the batch connected into a deleted node.
#[tokio::test(flavor = "multi_thread")]
async fn reapplying_an_unanchored_function_never_reuses_the_nodes_it_removes() {
    let mut board = board_from(POST_HELPER).await;
    apply_edit(&mut board, POST_HELPER).await;
    let fetches = node_types_on(&board)
        .into_iter()
        .filter(|node_type| *node_type == "http_fetch")
        .count();
    assert_eq!(fetches, 1);
    assert_round_trips(&board);
}

const EVENT_LOCAL_A: &str = r#"use log::*

eventsSimple a() {
    let inputId: string = ""
    inputId = "a"
    info({ message: inputId, toast: false })
}
"#;

const EVENT_LOCAL_B: &str = r#"
eventsSimple b() {
    let inputId: string = ""
    inputId = "b"
    info({ message: inputId, toast: false })
}
"#;

const EVENT_LOCAL_C: &str = r#"
eventsSimple c() {
    let inputId: string = ""
    inputId = "c"
    info({ message: inputId, toast: false })
}
"#;

fn variables_named(board: &Board, name: &str) -> Vec<String> {
    let mut ids: Vec<String> = board
        .variables
        .values()
        .filter(|variable| variable.name == name)
        .map(|variable| variable.id.clone())
        .collect();
    ids.sort();
    ids
}

/// The variable the `variable_set` in event `event` writes.
fn variable_written_by(board: &Board, event: &str) -> String {
    let entry = all_nodes(board)
        .into_iter()
        .find(|node| node.name == "events_simple" && node.friendly_name == event)
        .unwrap_or_else(|| panic!("no event {event}"));
    let set = all_nodes(board)
        .into_iter()
        .find(|node| node.name == "variable_set" && runs_before(board, entry, node))
        .unwrap_or_else(|| panic!("event {event} sets no variable"));
    let var_ref = set
        .pins
        .values()
        .find(|pin| pin.name == "var_ref")
        .and_then(|pin| pin.default_value.as_deref())
        .expect("the set names its variable");
    serde_json::from_slice(var_ref).expect("a variable id")
}

/// "unanchored local variable `inputId` matches 2 live variables": each root event's own
/// `let inputId` minted another board global (`var_inputId_2`, …), after which no unanchored
/// `let inputId` could bind.
#[tokio::test(flavor = "multi_thread")]
async fn event_locals_of_one_name_share_one_board_variable() {
    let mut board = board_from(&format!("{EVENT_LOCAL_A}{EVENT_LOCAL_B}")).await;
    assert_eq!(variables_named(&board, "inputId").len(), 1);
    assert_round_trips(&board);

    apply_edit(
        &mut board,
        &format!("{EVENT_LOCAL_A}{EVENT_LOCAL_B}{EVENT_LOCAL_C}"),
    )
    .await;
    let ids = variables_named(&board, "inputId");
    assert_eq!(ids.len(), 1, "{ids:?}");
    for event in ["a", "b", "c"] {
        assert_eq!(variable_written_by(&board, event), ids[0]);
    }
    assert_round_trips(&board);
}

/// Boards the old behaviour left with `var_inputId_2` keep working: an event that already writes
/// the duplicate stays on it, and a new event's `let inputId` takes the first variable
/// FlowScript created with that name and initial value.
#[tokio::test(flavor = "multi_thread")]
async fn event_locals_stay_on_the_duplicate_variables_they_already_use() {
    let mut board = board_from(EVENT_LOCAL_A).await;
    apply_commands(
        &mut board,
        vec![create_variable(
            "var_inputId_2",
            "inputId",
            "String",
            "Normal",
        )],
    )
    .await;
    let text = anchored(&board);
    let duplicate = text
        .lines()
        .find(|line| line.contains("//@v:var_inputId_2"))
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_else(|| panic!("the duplicate renders:\n{text}"))
        .trim_end_matches(':')
        .to_string();
    apply_edit(
        &mut board,
        &format!(
            "{text}\neventsSimple b() {{\n    {duplicate} = \"b\"\n    log::info({{ message: {duplicate}, toast: false }})\n}}\n"
        ),
    )
    .await;
    assert_eq!(variable_written_by(&board, "b"), "var_inputId_2");
    assert_round_trips(&board);

    apply_edit(
        &mut board,
        &format!("{EVENT_LOCAL_A}{EVENT_LOCAL_B}{EVENT_LOCAL_C}"),
    )
    .await;
    assert_eq!(variable_written_by(&board, "a"), "var_inputId");
    assert_eq!(variable_written_by(&board, "b"), "var_inputId_2");
    assert_eq!(variable_written_by(&board, "c"), "var_inputId");
    assert_round_trips(&board);
}

fn line_with<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line with {needle}:\n{text}"))
}

fn anchor_of(line: &str) -> String {
    line.split("//@n:")
        .nth(1)
        .unwrap_or_else(|| panic!("{line} carries no node anchor"))
        .trim()
        .to_string()
}

fn layer_id(board: &Board, name: &str) -> String {
    board
        .layers
        .values()
        .find(|layer| layer.name == name)
        .map(|layer| layer.id.clone())
        .unwrap_or_else(|| panic!("no layer {name}"))
}

/// The function's exit is driven by `tail`'s execution output.
fn exits_after(board: &Board, function: &str, tail: &Node) -> bool {
    function_exit(board, function)
        .depends_on
        .contains(&exec_pin(tail, PinType::Output).id)
}

/// "call `info` keeps anchor …, but that node belongs to the board root while this statement is in
/// layer `$0`" (I062, I101, I113): cutting anchored lines out of an event and pasting them into a
/// new `function` is an extract-function refactor. The nodes keep their identity and move, and so
/// does a pure helper that only fed them.
#[tokio::test(flavor = "multi_thread")]
async fn anchored_lines_pasted_into_a_new_function_move_there() {
    let mut board = board_from(
        r#"eventsSimple extract() {
    log::info({ message: "before", toast: false })
    log::info({ message: "hello".toUpper(), toast: false })
    log::info({ message: "second", toast: false })
    log::info({ message: "after", toast: false })
}
"#,
    )
    .await;
    let text = anchored(&board);
    let first = line_with(&text, "toUpper");
    let second = line_with(&text, "\"second\"");
    let (first_id, second_id) = (anchor_of(first), anchor_of(second));
    let upper_id = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "string_to_upper")
        .map(|node| node.id.clone())
        .expect("the inline toUpper is a node");
    let edited = format!(
        "{}\nfunction helper() {{\n{first}\n{second}\n}}\n",
        text.replace(&format!("{first}\n{second}\n"), "    helper()\n")
    );

    let result = apply_edit(&mut board, &edited).await;

    assert!(
        !result
            .board_commands
            .iter()
            .any(|command| matches!(command, BoardCommand::RemoveNode { .. })),
        "{:?}",
        result.board_commands
    );
    let helper = layer_id(&board, "helper");
    for id in [&first_id, &second_id, &upper_id] {
        let node = all_nodes(&board)
            .into_iter()
            .find(|node| &node.id == id)
            .unwrap_or_else(|| panic!("{id} kept its identity"));
        assert_eq!(
            node.layer.as_deref(),
            Some(helper.as_str()),
            "{}",
            node.name
        );
    }
    let second = all_nodes(&board)
        .into_iter()
        .find(|node| node.id == second_id)
        .unwrap();
    assert!(exits_after(&board, "helper", second));
    assert!(!runs_before(&board, info(&board, "before"), second));
    assert_round_trips(&board);
}

/// Moving the last statement of one function into another keeps both functions whole: the
/// shortened one now exits after its new tail.
#[tokio::test(flavor = "multi_thread")]
async fn a_statement_moved_between_functions_rewires_both() {
    let mut board = board_from(
        r#"function report() {
    log::info({ message: "x", toast: false })
}

function other() {
    log::info({ message: "o1", toast: false })
    log::info({ message: "o2", toast: false })
}

eventsSimple run() {
    report()
    other()
}
"#,
    )
    .await;
    let text = anchored(&board);
    let x = line_with(&text, "\"x\"");
    let o2 = line_with(&text, "\"o2\"");
    let edited = text
        .replace(&format!("{o2}\n"), "")
        .replace(x, &format!("{x}\n{o2}"));

    apply_edit(&mut board, &edited).await;

    let moved = info(&board, "o2");
    assert_eq!(moved.layer, Some(layer_id(&board, "report")));
    assert!(runs_before(&board, info(&board, "x"), moved));
    assert!(exits_after(&board, "report", moved));
    assert!(exits_after(&board, "other", info(&board, "o1")));
    assert_round_trips(&board);
}

/// A document that moves lines into a function but leaves out the event they came from cannot
/// say what runs there now; refusing beats silently cutting the event's chain.
#[tokio::test(flavor = "multi_thread")]
async fn moving_lines_out_of_an_unwritten_event_is_refused() {
    let board = board_from(
        r#"eventsSimple run() {
    log::info({ message: "a", toast: false })
    log::info({ message: "b", toast: false })
    log::info({ message: "c", toast: false })
}
"#,
    )
    .await;
    let text = anchored(&board);
    let partial = format!(
        "use log::*\n\nfunction helper() {{\n{}\n}}\n",
        line_with(&text, "\"b\"")
    );
    let (catalog, enricher) = catalog();
    let diagnostics = reconcile_text_with_catalog_enriched_opts(
        &board,
        &partial,
        &catalog,
        &enricher,
        &ReconcileOptions {
            mode: ReconcileMode::Additive,
            ..ReconcileOptions::default()
        },
    )
    .diagnostics;
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("with a `helper(…)` call where they were")),
        "{diagnostics:?}"
    );
}

/// Whether execution can flow from `from` to `to` along any chain of execution wires.
fn exec_reaches(board: &Board, from: &Node, to: &Node) -> bool {
    let nodes = all_nodes(board);
    let owners: std::collections::HashMap<&str, &Node> = nodes
        .iter()
        .flat_map(|node| node.pins.keys().map(move |id| (id.as_str(), *node)))
        .collect();
    let mut stack = vec![from];
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node.id.as_str()) {
            continue;
        }
        let exec_outputs = node.pins.values().filter(|pin| {
            pin.pin_type == PinType::Output && pin.data_type == VariableType::Execution
        });
        for target in exec_outputs.flat_map(|pin| pin.connected_to.iter()) {
            let Some(next) = owners.get(target.as_str()) else {
                continue;
            };
            if next.id == to.id {
                return true;
            }
            stack.push(next);
        }
    }
    false
}

/// A literal `let` inside a loop must reset the variable before each accumulation. Typed and
/// inferred declarations take different compiler paths, in both events and functions.
#[tokio::test(flavor = "multi_thread")]
async fn a_let_inside_a_loop_resets_its_variable_on_every_iteration() {
    for header in ["eventsSimple countRows()", "function countRows()"] {
        for declaration in ["let total = 0", "let total: int = 0"] {
            let source = format!(
                r#"{header} {{
    for (const row of ["a", "b"]) {{
        {declaration}
        for (const x of [1, 2]) {{
            total = total + x
        }}
        log::info({{ message: "row", toast: false }})
    }}
    log::info({{ message: "done", toast: false }})
}}
"#,
            );
            let board = board_from(&source).await;
            let variable_id = board
                .variables
                .values()
                .chain(
                    board
                        .layers
                        .values()
                        .flat_map(|layer| layer.variables.values()),
                )
                .find(|variable| variable.name == "total")
                .map(|variable| variable.id.clone())
                .expect("`total` becomes a variable");
            let expected_ref = format!("\"{variable_id}\"");
            let writers: Vec<&Node> = all_nodes(&board)
                .into_iter()
                .filter(|node| {
                    node.name == "variable_set"
                        && node.pins.values().any(|pin| {
                            pin.name == "var_ref"
                                && pin.default_value.as_deref() == Some(expected_ref.as_bytes())
                        })
                })
                .collect();
            assert_eq!(writers.len(), 2, "a reset and the accumulation:\n{source}");
            let (resets, accumulations): (Vec<&Node>, Vec<&Node>) =
                writers.into_iter().partition(|node| {
                    node.pins.values().any(|pin| {
                        pin.name == "value_in"
                            && pin.depends_on.is_empty()
                            && pin.default_value.as_deref() == Some(b"0".as_slice())
                    })
                });
            assert_eq!(resets.len(), 1, "one literal reset:\n{source}");
            assert_eq!(accumulations.len(), 1, "one accumulation:\n{source}");
            let (reset, accumulation) = (resets[0], accumulations[0]);
            let loops: Vec<&Node> = all_nodes(&board)
                .into_iter()
                .filter(|node| node.name == "control_for_each")
                .collect();
            assert_eq!(loops.len(), 2, "outer and inner loops:\n{source}");
            let inner = loops
                .iter()
                .copied()
                .find(|node| runs_before(&board, reset, node))
                .expect("the reset runs immediately before the inner loop");
            let outer = loops
                .iter()
                .copied()
                .find(|node| node.id != inner.id)
                .expect("the outer loop");
            let output_connects_to = |node: &Node, output: &str, next: &Node| {
                node.pins.values().any(|pin| {
                    pin.pin_type == PinType::Output
                        && pin.name == output
                        && pin
                            .connected_to
                            .contains(&exec_pin(next, PinType::Input).id)
                })
            };
            assert!(
                output_connects_to(outer, "exec_out", reset),
                "the outer loop body starts with the reset:\n{source}"
            );
            assert!(output_connects_to(inner, "exec_out", accumulation));
            assert!(output_connects_to(inner, "done", info(&board, "row")));
            assert!(output_connects_to(outer, "done", info(&board, "done")));
            assert!(exec_reaches(&board, reset, accumulation));
            assert_round_trips(&board);
        }
    }
}

/// The `control_call_function` nodes that call function `name`.
fn calls_of<'a>(board: &'a Board, name: &str) -> Vec<&'a Node> {
    let expected = format!("\"{}\"", layer_id(board, name));
    all_nodes(board)
        .into_iter()
        .filter(|node| {
            node.name == "control_call_function"
                && node.pins.values().any(|pin| {
                    pin.name == "function_layer_id"
                        && pin.default_value.as_deref() == Some(expected.as_bytes())
                })
        })
        .collect()
}

fn input_named<'a>(node: &'a Node, name: &str) -> &'a Pin {
    node.pins
        .values()
        .find(|pin| pin.pin_type == PinType::Input && pin.name == name)
        .unwrap_or_else(|| panic!("`{}` has no input `{name}`", node.friendly_name))
}

fn function_param<'a>(board: &'a Board, function: &str, name: &str) -> &'a Pin {
    board.layers[&layer_id(board, function)]
        .pins
        .values()
        .find(|pin| pin.pin_type == PinType::Input && pin.name == name)
        .unwrap_or_else(|| panic!("function `{function}` has no parameter `{name}`"))
}

const GREETER: &str = r#"function greet(name: string) {
    log::info({ message: name, toast: false })
}

eventsSimple run() {
    greet({ name: "a" })
}
"#;

/// Relay: changing a function's signature together with its calls was refused with "nothing was
/// applied". Adding a parameter now migrates the layer and its calls in one apply.
#[tokio::test(flavor = "multi_thread")]
async fn adding_a_parameter_migrates_the_function_and_its_calls() {
    let mut board = board_from(GREETER).await;
    let kept_param = function_param(&board, "greet", "name").id.clone();
    let edited = anchored(&board)
        .replace("greet(name: string)", "greet(name: string, loud: bool)")
        .replace(
            "greet({ name: \"a\" })",
            "greet({ name: \"a\", loud: true })",
        );
    let result = apply_edit(&mut board, &edited).await;

    assert_eq!(function_param(&board, "greet", "name").id, kept_param);
    function_param(&board, "greet", "loud");
    let calls = calls_of(&board, "greet");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        input_named(calls[0], "loud").default_value.as_deref(),
        Some(b"true".as_slice())
    );
    assert!(
        result
            .changes
            .layers_changed
            .iter()
            .any(|layer| layer.name == "greet"),
        "{:?}",
        result.changes
    );
    assert_round_trips(&board);
}

/// A renamed parameter is a new pin: the body and the call are wired to it again from the text.
#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_parameter_rewires_the_body_and_the_call() {
    let mut board = board_from(GREETER).await;
    let edited = anchored(&board)
        .replace("greet(name: string)", "greet(who: string)")
        .replace("info(name,", "info(who,")
        .replace("greet({ name: \"a\" })", "greet({ who: \"a\" })");
    apply_edit(&mut board, &edited).await;

    let who = function_param(&board, "greet", "who");
    assert!(
        !board.layers[&layer_id(&board, "greet")]
            .pins
            .values()
            .any(|pin| pin.name == "name")
    );
    let message = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "log_info")
        .map(|node| input_named(node, "message"))
        .expect("the body keeps its log");
    assert!(message.depends_on.contains(&who.id), "{message:?}");
    let calls = calls_of(&board, "greet");
    assert_eq!(
        input_named(calls[0], "who").default_value.as_deref(),
        Some(b"\"a\"".as_slice())
    );
    assert_round_trips(&board);
}

/// Dropping a parameter while a call the document does not write keeps using the function would
/// silently cut that call's wire, so it is refused.
#[tokio::test(flavor = "multi_thread")]
async fn dropping_a_parameter_with_an_unwritten_call_is_refused() {
    let board = board_from(GREETER).await;
    let text = anchored(&board);
    let mut ast = flow_like_ast::parse(&text).expect("the board's source parses");
    ast.events.clear();
    let function = flow_like_ast::render(
        &ast,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    )
    .replace("greet(name: string)", "greet()")
    .replace("info(name,", "info(\"hi\",");
    let (catalog, enricher) = catalog();
    let diagnostics = reconcile_text_with_catalog_enriched_opts(
        &board,
        &function,
        &catalog,
        &enricher,
        &ReconcileOptions {
            mode: ReconcileMode::Additive,
            ..ReconcileOptions::default()
        },
    )
    .diagnostics;
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("are not part of this document")),
        "{diagnostics:?}\n{function}"
    );
}

/// "Gleicher Graph, gleicher Export": a name the text gives a value is stored on its node, so the
/// next render keeps it, and a node added elsewhere renames nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_renamed_binding_is_stored_and_survives_unrelated_edits() {
    let mut board = board_from(
        r#"function stamp(value: string): (out: string) {
    log::info({ message: value, toast: false })
    return value
}

eventsSimple run() {
    const out = stamp("x")
    log::info({ message: out, toast: false })
}
"#,
    )
    .await;
    let edited = anchored(&board)
        .replace("out = stamp", "selectedPerson = stamp")
        .replace("info(out,", "info(selectedPerson,");
    let result = apply_edit(&mut board, &edited).await;
    assert!(
        result
            .board_commands
            .iter()
            .all(|command| matches!(command, BoardCommand::SetNodeCodeNames { .. })),
        "a rename only stores the name: {:?}",
        result.board_commands
    );
    let text = anchored(&board);
    assert!(text.contains("const selectedPerson = stamp("), "{text}");

    let reader = line_with(&text, "info(selectedPerson,");
    let extended = text.replace(
        reader,
        &format!(
            "    const second = stamp({{ value: \"y\" }})\n{reader}\n    log::info({{ message: second, toast: false }})"
        ),
    );
    apply_edit(&mut board, &extended).await;
    let text = anchored(&board);
    assert!(
        text.contains("const selectedPerson = stamp({ value: \"x\" })"),
        "{text}"
    );
    assert!(
        text.contains("const second = stamp({ value: \"y\" })"),
        "{text}"
    );
    assert_round_trips(&board);
}

/// Loop element names written in the text survive too, instead of falling back to `item`.
#[tokio::test(flavor = "multi_thread")]
async fn a_loop_element_keeps_its_written_name() {
    let board = board_from(
        r#"eventsSimple run() {
    for (const row of ["a", "b"]) {
        log::info({ message: row, toast: false })
    }
}
"#,
    )
    .await;
    let text = anchored(&board);
    assert!(text.contains("for (const row of"), "{text}");
    assert_round_trips(&board);
}

fn forget_code_names(board: &mut Board) {
    for node in board.nodes.values_mut() {
        node.code_names = None;
    }
    for layer in board.layers.values_mut() {
        for node in layer.nodes.values_mut() {
            node.code_names = None;
        }
    }
}

fn function_block(text: &str, header: &str) -> String {
    let start = text
        .find(header)
        .unwrap_or_else(|| panic!("no {header}:\n{text}"));
    let end = text[start..]
        .find("\n}\n")
        .map(|end| start + end)
        .unwrap_or(text.len());
    text[start..end].to_string()
}

/// Relay: an unchanged call was renumbered `saved7` → `saved9` because a node elsewhere joined one
/// board-wide numbering. Generated names are numbered per function, so a node added to an event
/// renames nothing inside a function, even on a board that stores no names.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_added_to_an_event_renames_nothing_in_a_function() {
    let mut board = board_from(
        r#"function stamp(value: string): (out: string) {
    log::info({ message: value, toast: false })
    return value
}

function pair(): (a: string, b: string) {
    const first = stamp("x")
    const second = stamp("y")
    return first, second
}

eventsSimple run() {
    const { a, b } = pair()
    log::info({ message: a, toast: false })
}
"#,
    )
    .await;
    forget_code_names(&mut board);
    let before = function_block(&anchored(&board), "function pair");

    let text = anchored(&board);
    let reader = line_with(&text, "info(a,");
    let edited = text.replace(
        reader,
        &format!(
            "    const extra = stamp({{ value: \"z\" }})\n{reader}\n    log::info({{ message: extra, toast: false }})"
        ),
    );
    assert_ne!(edited, text, "the edit adds a call to the event");
    apply_edit(&mut board, &edited).await;
    forget_code_names(&mut board);
    let after = function_block(&anchored(&board), "function pair");
    assert!(
        before.contains("const "),
        "the function binds its calls:\n{before}"
    );
    assert_eq!(before, after);
}

/// Sentinel / t4-pr-review-bot: a statement after a branch rendered inside one arm when another
/// arm held a call with an unhandled error output, so the next apply ran it on that path only.
#[tokio::test(flavor = "multi_thread")]
async fn a_statement_after_a_branch_stays_after_it_when_an_arm_leaves_an_error_unhandled() {
    let board = board_from(
        r#"eventsSimple run() {
    if (1 > 0) {
        http::fetch({ request: http::request({ method: "GET", url: "https://example.com" }) })
        log::info({ message: "fetched", toast: false })
    } else {
        log::info({ message: "skipped", toast: false })
    }
    log::info({ message: "after", toast: false })
}
"#,
    )
    .await;
    let after = info(&board, "after");
    assert!(exec_reaches(&board, info(&board, "fetched"), after));
    assert!(exec_reaches(&board, info(&board, "skipped"), after));

    let text = anchored(&board);
    let line = line_with(&text, "\"after\"");
    assert!(
        line.starts_with("    ") && !line.starts_with("     "),
        "the statement after the branch renders after it, not inside an arm:\n{text}"
    );
    assert_round_trips(&board);
}

/// Beleg / t1-faker-seed: `if (hasLead)` over `struct::get(…).found` rendered as the member read
/// `lead.company`, which means the field's VALUE, so the next apply branched on the value.
#[tokio::test(flavor = "multi_thread")]
async fn a_struct_get_found_output_keeps_its_meaning_on_render() {
    let board = board_from(
        r#"eventsSimple run() {
    const lead = struct::make().set("company", "acme")
    const { value: org, found: hasLead } = struct::get({ struct: lead, field: "company" })
    if (hasLead) {
        log::info({ message: "lead", toast: false })
    }
}
"#,
    )
    .await;
    let text = anchored(&board);
    assert!(text.contains(".found"), "{text}");
    assert_round_trips(&board);
}

/// t3-jira-confluence: `array::length({ array: issue.labels })` rendered as `issue.labels.length`,
/// and a member read of an untyped field reads `.length` back as another field.
#[tokio::test(flavor = "multi_thread")]
async fn the_length_of_a_field_read_keeps_its_call_form() {
    let board = board_from(
        r#"function labelCount(issue: Struct): (count: int) {
    return array::length({ array: issue.labels })
}

eventsSimple run() {
    const count = labelCount(struct::make())
    log::info({ message: count, toast: false })
}
"#,
    )
    .await;
    let text = anchored(&board);
    assert!(!text.contains("labels.length"), "{text}");
    assert_round_trips(&board);
}

#[tokio::test(flavor = "multi_thread")]
async fn event_descriptions_apply_edit_clear_and_resolve_stored_refs() {
    let mut board = board_from(
        r#"@description("Fetch a web page and return its text for the agent to read.")
eventsGeneric fetchPage(url: string, payload: Struct) {
    const response = http::fetch({ request: http::request({ method: "GET", url: url }) })
    return http::toText({ response: response })
}
"#,
    )
    .await;
    let entry_id = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "events_generic" && node.friendly_name == "fetchPage")
        .expect("the described event was created")
        .id
        .clone();
    let assert_description = |board: &Board, expected: &str| {
        let entry = all_nodes(board)
            .into_iter()
            .find(|node| node.id == entry_id)
            .expect("the event keeps its identity");
        assert_eq!(
            flow_like::flow::pin::resolve_schema(&entry.description, &board.refs).unwrap(),
            expected
        );
        if !expected.is_empty() {
            assert_eq!(
                board.refs.get(&entry.description).map(String::as_str),
                Some(expected)
            );
        }
    };
    let initial = "Fetch a web page and return its text for the agent to read.";
    assert_description(&board, initial);
    assert_round_trips(&board);

    let updated = "Fetch the page.\nReturn its \"text\" to the agent.";
    let edited = anchored(&board).replace(
        &flow_like_ast::quote_string(initial),
        &flow_like_ast::quote_string(updated),
    );
    apply_edit(&mut board, &edited).await;
    assert_description(&board, updated);
    assert_round_trips(&board);

    let omitted = without_line(&anchored(&board), "@description(");
    let result = apply_edit(&mut board, &omitted).await;
    assert!(
        result.commands.is_empty(),
        "omitting the description is a no-op"
    );
    assert_description(&board, updated);

    let cleared = anchored(&board).replace(&flow_like_ast::quote_string(updated), "\"\"");
    apply_edit(&mut board, &cleared).await;
    assert_description(&board, "");
    assert!(!anchored(&board).contains("@description("));
    assert_round_trips(&board);
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_descriptions_in_module_functions_keep_agent_references() {
    let mut board = board_from(
        r#"module assistants {
    function configureTools() {
        const agent = agent::fromModel({ model: ai::findModel({}) }).registerFunctionTools({ tools: [fetchPage] })
        log::info({ message: json::stringify({ value: agent, pretty: false }), toast: false })
        @description("Fetch a web page and return its text for the agent to read.")
        eventsGeneric fetchPage(url: string, payload: Struct) {
            const response = http::fetch({ request: http::request({ method: "GET", url: url }) })
            return http::toText({ response: response })
        }
    }

    @description("Prepare the assistant tools.")
    eventsSimple prepare() {
        configureTools()
    }
}
"#,
    )
    .await;
    let module_id = board
        .layers
        .values()
        .find(|layer| layer.name == "assistants")
        .expect("the module exists")
        .id
        .clone();
    let tool = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "events_generic" && node.friendly_name == "fetchPage")
        .expect("the nested tool exists");
    let tool_id = tool.id.clone();
    assert_eq!(
        flow_like::flow::pin::resolve_schema(&tool.description, &board.refs).unwrap(),
        "Fetch a web page and return its text for the agent to read."
    );
    let registration_id = all_nodes(&board)
        .into_iter()
        .find(|node| node.name == "agent_register_function_tools")
        .expect("the tool registration exists")
        .id
        .clone();
    let assert_reference = |board: &Board| {
        let registration = all_nodes(board)
            .into_iter()
            .find(|node| node.id == registration_id)
            .expect("the tool registration keeps its identity");
        assert_eq!(
            registration.fn_refs.as_ref().unwrap().fn_refs,
            vec![tool_id.clone()]
        );
    };
    assert_reference(&board);
    assert_round_trips(&board);

    let text = apply_module_file_edit(
        &mut board,
        &module_id,
        "Fetch a web page and return its text for the agent to read.",
        "Retrieve the requested page as plain text.",
    )
    .await;
    assert!(text.contains("@description("), "{text}");
    let tool = all_nodes(&board)
        .into_iter()
        .find(|node| node.id == tool_id)
        .expect("the nested tool keeps its identity");
    assert_eq!(
        flow_like::flow::pin::resolve_schema(&tool.description, &board.refs).unwrap(),
        "Retrieve the requested page as plain text."
    );
    assert_reference(&board);
    assert_round_trips(&board);
}
