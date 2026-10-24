use super::*;
use crate::{
    flow::{ast::board_to_flowscript, board::commands::pins::connect_pins::connect_pins},
    state::FlowLikeConfig,
    utils::http::HTTPClient,
};
use flow_like_types::{json::json, tokio};

fn state() -> Arc<FlowLikeState> {
    Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ))
}

fn catalog() -> Vec<Node> {
    let mut event = Node::new("events_simple", "Simple Event", "", "events");
    event.set_start(true);
    event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    let mut producer = Node::new("make_value", "Make Value", "", "test");
    producer.add_input_pin("exec_in", "In", "", VariableType::Execution);
    producer.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    producer.add_output_pin("value", "Value", "", VariableType::String);
    let mut sink = Node::new("payload_sink", "Payload Sink", "", "test");
    sink.add_input_pin("exec_in", "In", "", VariableType::Execution);
    sink.add_input_pin("payload", "Payload", "", VariableType::String);
    sink.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    vec![event, producer, sink]
}

fn place(board: &mut Board, template: &Node, id: &str) {
    let mut node = AddNodeCommand::new(template.clone()).node;
    node.id = id.to_string();
    board.nodes.insert(id.to_string(), node);
}

fn wire(board: &mut Board, from: &str, output: &str, to: &str, input: &str) {
    let output = board.nodes[from]
        .get_pin_by_name(output)
        .unwrap()
        .id
        .clone();
    let input = board.nodes[to].get_pin_by_name(input).unwrap().id.clone();
    connect_pins(board, from, &output, to, &input).unwrap();
}

fn remove_node(id: &str) -> BoardCommand {
    BoardCommand::RemoveNode {
        node_id: id.to_string(),
        summary: None,
    }
}

fn disconnect(from: &str, output: &str, to: &str, input: &str) -> BoardCommand {
    BoardCommand::DisconnectPins {
        from_node: from.to_string(),
        from_pin: output.to_string(),
        to_node: to.to_string(),
        to_pin: input.to_string(),
        summary: None,
    }
}

fn graph(board: &Board) -> flow_like_types::Value {
    let mut nodes = board.nodes.clone();
    let mut layers = board.layers.clone();
    for node in nodes.values_mut() {
        node.hash = None;
    }
    for layer in layers.values_mut() {
        layer.hash = None;
    }
    json!({ "nodes": nodes, "layers": layers })
}

fn assert_disconnects_precede_removals(commands: &[GenericCommand]) {
    let first_removal = commands
        .iter()
        .position(|command| {
            matches!(
                command,
                GenericCommand::RemoveNode(_) | GenericCommand::RemoveLayer(_)
            )
        })
        .expect("the batch removes an endpoint");
    let last_disconnect = commands
        .iter()
        .rposition(|command| matches!(command, GenericCommand::DisconnectPin(_)))
        .expect("the batch explicitly disconnects an edge");
    assert!(last_disconnect < first_removal);
}

#[tokio::test]
async fn confirmed_text_deletions_rewire_removed_producers_and_restore_undo() {
    for count in [1, 190] {
        let catalog = catalog();
        let state = state();
        let mut board = Board::new_detached(None, flow_like_storage::Path::default());
        for index in 0..count {
            let event = format!("event-{index:03}");
            let producer = format!("remove-{index:03}");
            let sink = format!("sink-{index:03}");
            place(&mut board, &catalog[0], &event);
            board.nodes.get_mut(&event).unwrap().friendly_name = format!("Event {index}");
            place(&mut board, &catalog[1], &producer);
            place(&mut board, &catalog[2], &sink);
            wire(&mut board, &event, "exec_out", &producer, "exec_in");
            wire(&mut board, &producer, "exec_out", &sink, "exec_in");
            wire(&mut board, &producer, "value", &sink, "payload");
        }
        board.cleanup();
        let original = board.clone();
        let text = board_to_flowscript(
            &board,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );
        assert_eq!(text.matches("//@n:remove-").count(), count);
        let edited = text
            .lines()
            .filter(|line| !line.contains("//@n:remove-"))
            .map(|line| {
                if !line.contains("//@n:sink-") {
                    return line.to_string();
                }
                let start =
                    line.find("payloadSink(").expect("rendered sink call") + "payloadSink(".len();
                let end = line.rfind(')').expect("rendered call ending");
                format!("{}\"literal\"{}", &line[..start], &line[end..])
            })
            .collect::<Vec<_>>()
            .join("\n");
        let preview =
            apply_flowscript_to_board(&mut board, &edited, &catalog, state.clone(), None, false)
                .await
                .expect("deletion preview");
        assert!(
            is_destructive_block(preview.commands.len(), &preview.diagnostics),
            "{:?}",
            preview.diagnostics
        );
        assert_eq!(
            destructive_flowscript_command_summaries(&preview.board_commands).len(),
            count
        );
        assert_eq!(
            serde_json::to_value(&board).unwrap(),
            serde_json::to_value(&original).unwrap()
        );

        let applied =
            apply_flowscript_to_board(&mut board, &edited, &catalog, state.clone(), None, true)
                .await
                .expect("confirmed deletion apply");
        assert!(applied.diagnostics.is_empty(), "{:?}", applied.diagnostics);
        assert_eq!(
            applied
                .commands
                .iter()
                .filter(|command| matches!(command, GenericCommand::RemoveNode(_)))
                .count(),
            count
        );
        assert_eq!(board.nodes.len(), count * 2);
        for index in 0..count {
            let event = &board.nodes[&format!("event-{index:03}")];
            let sink = &board.nodes[&format!("sink-{index:03}")];
            let output = event.get_pin_by_name("exec_out").unwrap();
            let input = sink.get_pin_by_name("exec_in").unwrap();
            assert_eq!(output.connected_to, BTreeSet::from([input.id.clone()]));
            assert_eq!(input.depends_on, BTreeSet::from([output.id.clone()]));
            let payload = sink.get_pin_by_name("payload").unwrap();
            assert!(payload.depends_on.is_empty());
            assert_eq!(
                payload.default_value,
                Some(serde_json::to_vec(&json!("literal")).unwrap())
            );
        }
        let confirmed = graph(&board);
        let mut replay = original.clone();
        replay
            .execute_commands(applied.commands.clone(), state.clone())
            .await
            .expect("replay deletion receipt");
        assert_eq!(graph(&replay), confirmed);
        board
            .undo(applied.commands, state.clone())
            .await
            .expect("undo deletions");
        assert_eq!(graph(&board), graph(&original));
        let repeated =
            apply_flowscript_to_board(&mut replay, &edited, &catalog, state, None, false)
                .await
                .expect("repeat confirmed text");
        assert!(
            repeated.diagnostics.is_empty(),
            "{:?}",
            repeated.diagnostics
        );
        assert!(repeated.commands.is_empty());
        assert_eq!(graph(&replay), confirmed);
    }
}

#[tokio::test]
async fn deleting_a_middle_statement_preserves_bridge_undo() {
    let catalog = catalog();
    let state = state();
    let mut board = Board::new_detached(None, flow_like_storage::Path::default());
    place(&mut board, &catalog[0], "event");
    place(&mut board, &catalog[1], "middle");
    place(&mut board, &catalog[2], "sink");
    board
        .nodes
        .get_mut("sink")
        .unwrap()
        .get_pin_mut_by_name("payload")
        .unwrap()
        .set_default_value(Some(json!("retained")));
    wire(&mut board, "event", "exec_out", "middle", "exec_in");
    wire(&mut board, "middle", "exec_out", "sink", "exec_in");
    board.cleanup();
    let original = board.clone();
    let text = board_to_flowscript(
        &board,
        &flow_like_ast::RenderOptions {
            anchors: true,
            ..Default::default()
        },
    );
    let edited = text
        .lines()
        .filter(|line| !line.contains("//@n:middle"))
        .collect::<Vec<_>>()
        .join("\n");
    let applied =
        apply_flowscript_to_board(&mut board, &edited, &catalog, state.clone(), None, true)
            .await
            .expect("delete middle statement");
    assert!(applied.diagnostics.is_empty(), "{:?}", applied.diagnostics);
    assert!(!board.nodes.contains_key("middle"));
    assert!(
        !applied
            .commands
            .iter()
            .any(|command| matches!(command, GenericCommand::DisconnectPin(_))),
        "exercise the bridge without an explicit disconnect receipt"
    );
    let output = board.nodes["event"].get_pin_by_name("exec_out").unwrap();
    let input = board.nodes["sink"].get_pin_by_name("exec_in").unwrap();
    assert_eq!(output.connected_to, BTreeSet::from([input.id.clone()]));
    assert_eq!(input.depends_on, BTreeSet::from([output.id.clone()]));
    board
        .undo(applied.commands, state)
        .await
        .expect("undo bridge deletion");
    assert_eq!(graph(&board), graph(&original));
}

#[tokio::test]
async fn direct_deletion_batches_disconnect_either_or_both_node_endpoints() {
    for (count, removed) in [
        (1, vec!["source"]),
        (1, vec!["sink"]),
        (1, vec!["source", "sink"]),
        (190, vec!["source"]),
    ] {
        let catalog = catalog();
        let state = state();
        let mut board = Board::new_detached(None, flow_like_storage::Path::default());
        let mut commands = Vec::new();
        let mut disconnects = Vec::new();
        let mut removed_ids = Vec::new();
        for index in 0..count {
            let source = format!("source-{index:03}");
            let sink = format!("sink-{index:03}");
            place(&mut board, &catalog[1], &source);
            place(&mut board, &catalog[2], &sink);
            wire(&mut board, &source, "value", &sink, "payload");
            for id in &removed {
                let id = format!("{id}-{index:03}");
                commands.push(remove_node(&id));
                removed_ids.push(id);
            }
            disconnects.push(disconnect(&source, "value", &sink, "payload"));
        }
        commands.extend(disconnects);
        board.cleanup();
        let original = board.clone();
        let applied =
            apply_board_commands_to_board(&mut board, commands, &catalog, state.clone(), None)
                .await
                .expect("delete edge endpoints");
        assert_disconnects_precede_removals(&applied.commands);
        assert!(removed_ids.iter().all(|id| !board.nodes.contains_key(id)));
        assert_eq!(board.nodes.len(), count * (2 - removed.len()));
        assert!(
            board
                .nodes
                .values()
                .flat_map(|node| node.pins.values())
                .all(|pin| pin.connected_to.is_empty() && pin.depends_on.is_empty())
        );
        let mut replay = original.clone();
        replay
            .execute_commands(applied.commands.clone(), state.clone())
            .await
            .expect("replay endpoint deletions");
        assert_eq!(graph(&replay), graph(&board));
        board
            .undo(applied.commands, state)
            .await
            .expect("undo endpoint deletions");
        assert_eq!(graph(&board), graph(&original));
    }
}

#[tokio::test]
async fn direct_layer_deletion_disconnects_its_boundary_before_removal() {
    let catalog = catalog();
    let state = state();
    let mut board = Board::new_detached(None, flow_like_storage::Path::default());
    place(&mut board, &catalog[2], "sink");
    board.nodes.get_mut("sink").unwrap().layer = Some("function".into());
    let mut layer = Layer::new("function".into(), "Function".into(), LayerType::Function);
    let mut output = catalog[1].get_pin_by_name("value").unwrap().clone();
    output.pin_type = PinType::Input;
    layer.pins.insert(output.id.clone(), output.clone());
    board.layers.insert(layer.id.clone(), layer);
    let input = board.nodes["sink"]
        .get_pin_by_name("payload")
        .unwrap()
        .id
        .clone();
    connect_pins(&mut board, "function", &output.id, "sink", &input).unwrap();
    board.cleanup();
    let original = board.clone();
    let commands = vec![
        BoardCommand::RemoveLayer {
            layer_id: "function".into(),
            summary: None,
        },
        disconnect("function", "value", "sink", "payload"),
    ];
    let applied =
        apply_board_commands_to_board(&mut board, commands, &catalog, state.clone(), None)
            .await
            .expect("delete function boundary");
    assert_disconnects_precede_removals(&applied.commands);
    assert!(board.layers.is_empty());
    assert!(board.nodes["sink"].layer.is_none());
    assert!(
        board.nodes["sink"]
            .get_pin_by_name("payload")
            .unwrap()
            .depends_on
            .is_empty()
    );
    let mut replay = original.clone();
    replay
        .execute_commands(applied.commands.clone(), state.clone())
        .await
        .expect("replay layer deletion");
    assert_eq!(graph(&replay), graph(&board));
    board
        .undo(applied.commands, state)
        .await
        .expect("undo layer deletion");
    assert_eq!(graph(&board), graph(&original));
}

#[tokio::test]
async fn unrelated_deletions_preserve_the_order_of_surviving_edge_edits() {
    for connect_last in [false, true] {
        let catalog = catalog();
        let mut board = Board::new_detached(None, flow_like_storage::Path::default());
        place(&mut board, &catalog[1], "source");
        place(&mut board, &catalog[2], "sink");
        place(&mut board, &catalog[1], "unrelated");
        let connect = BoardCommand::ConnectPins {
            from_node: "source".into(),
            from_pin: "value".into(),
            to_node: "sink".into(),
            to_pin: "payload".into(),
            summary: None,
        };
        let disconnect = disconnect("source", "value", "sink", "payload");
        let mut commands = vec![remove_node("unrelated")];
        commands.extend(if connect_last {
            [disconnect, connect]
        } else {
            [connect, disconnect]
        });
        let applied = apply_board_commands_to_board(&mut board, commands, &catalog, state(), None)
            .await
            .expect("apply ordered edge edits with an unrelated deletion");
        assert!(applied.diagnostics.is_empty());
        assert!(!board.nodes.contains_key("unrelated"));
        let output = board.nodes["source"].get_pin_by_name("value").unwrap();
        let input = board.nodes["sink"].get_pin_by_name("payload").unwrap();
        assert_eq!(output.connected_to.contains(&input.id), connect_last);
        assert_eq!(input.depends_on.contains(&output.id), connect_last);
    }
}

#[tokio::test]
async fn a_missing_disconnect_endpoint_still_rejects_the_atomic_batch() {
    let catalog = catalog();
    let mut board = Board::new_detached(None, flow_like_storage::Path::default());
    place(&mut board, &catalog[1], "source");
    place(&mut board, &catalog[2], "sink");
    wire(&mut board, "source", "value", "sink", "payload");
    let original = serde_json::to_value(&board).unwrap();
    let commands = vec![
        remove_node("source"),
        disconnect("missing", "value", "sink", "payload"),
    ];
    let error = apply_board_commands_to_board(&mut board, commands, &catalog, state(), None)
        .await
        .err()
        .expect("unknown endpoint must fail");
    assert!(error.to_string().contains("missing"), "{error}");
    assert_eq!(serde_json::to_value(&board).unwrap(), original);
}
