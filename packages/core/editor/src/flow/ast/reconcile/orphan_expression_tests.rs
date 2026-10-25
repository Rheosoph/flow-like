use super::*;

fn add_expression(board: &mut Board, id: &str, node_type: &str) {
    let mut node = Node::new(node_type, id, "", "test");
    node.id = id.into();
    node.add_input_pin("value", "Value", "", VariableType::String);
    node.add_output_pin("result", "Result", "", VariableType::String);
    board.nodes.insert(id.into(), node);
}

fn wire_data(board: &mut Board, source: &str, target: &str) {
    let output = find_output_pin(&board.nodes[source], "result")
        .unwrap()
        .id
        .clone();
    let input = find_input_pin(&board.nodes[target], "value")
        .unwrap()
        .id
        .clone();
    connect(board, source, &output, target, &input);
}

fn remove(id: &str) -> BoardCommand {
    BoardCommand::RemoveNode {
        node_id: id.into(),
        summary: None,
    }
}

fn removed(commands: &[BoardCommand]) -> BTreeSet<&str> {
    commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::RemoveNode { node_id, .. } => Some(node_id.as_str()),
            _ => None,
        })
        .collect()
}

fn chain() -> Board {
    let mut board = empty_board();
    for id in ["source", "helper", "consumer", "unrelated"] {
        add_expression(&mut board, id, "test_pure");
    }
    wire_data(&mut board, "source", "helper");
    wire_data(&mut board, "helper", "consumer");
    board
}

#[test]
fn removes_only_the_expression_chain_that_lost_its_consumer() {
    let board = chain();
    let mut commands = vec![remove("consumer")];
    orphan_expressions::remove_orphans(&board, &HashSet::new(), &mut commands);
    assert_eq!(
        removed(&commands),
        BTreeSet::from(["consumer", "helper", "source"])
    );
}

#[test]
fn preserves_shared_expressions_and_explicit_anchors() {
    for anchored in [false, true] {
        let mut board = chain();
        let mut anchors = HashSet::new();
        if anchored {
            anchors.insert("helper");
        } else {
            wire_data(&mut board, "helper", "unrelated");
        }
        let mut commands = vec![remove("consumer")];
        orphan_expressions::remove_orphans(&board, &anchors, &mut commands);
        assert_eq!(removed(&commands), BTreeSet::from(["consumer"]));
    }
}

#[test]
fn a_new_consumer_preserves_the_existing_expression_chain() {
    let board = chain();
    let mut commands = vec![
        remove("consumer"),
        BoardCommand::ConnectPins {
            from_node: "helper".into(),
            from_pin: "result".into(),
            to_node: "$new".into(),
            to_pin: "value".into(),
            summary: None,
        },
    ];
    orphan_expressions::remove_orphans(&board, &HashSet::new(), &mut commands);
    assert_eq!(removed(&commands), BTreeSet::from(["consumer"]));
}

#[test]
fn deleting_and_rewiring_consumers_cleans_helpers_and_redundant_disconnects() {
    for reconnect in [false, true] {
        let mut board = chain();
        add_expression(&mut board, "survivor", "test_pure");
        wire_data(&mut board, "helper", "survivor");
        let mut commands = vec![BoardCommand::DisconnectPins {
            from_node: "helper".into(),
            from_pin: "result".into(),
            to_node: "survivor".into(),
            to_pin: "value".into(),
            summary: None,
        }];
        if reconnect {
            commands.clear();
            commands.push(BoardCommand::ConnectPins {
                from_node: "unrelated".into(),
                from_pin: "result".into(),
                to_node: "survivor".into(),
                to_pin: "value".into(),
                summary: None,
            });
        }
        commands.push(remove("consumer"));
        orphan_expressions::remove_orphans(&board, &HashSet::new(), &mut commands);
        assert_eq!(
            removed(&commands),
            BTreeSet::from(["consumer", "helper", "source"])
        );
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, BoardCommand::DisconnectPins { .. }))
        );
    }
}

#[test]
fn a_deleted_variable_removes_only_its_unused_readers() {
    let mut board = chain();
    board.nodes.get_mut("unrelated").unwrap().add_input_pin(
        "exec_in",
        "In",
        "",
        VariableType::Execution,
    );
    for (id, variable) in [
        ("unused", "old"),
        ("chain", "old"),
        ("shared", "old"),
        ("other", "kept"),
    ] {
        add_expression(&mut board, id, "variable_get");
        board
            .nodes
            .get_mut(id)
            .unwrap()
            .add_input_pin("var_ref", "Variable", "", VariableType::String)
            .default_value = Some(flow_like_types::json::to_vec(variable).unwrap());
    }
    add_expression(&mut board, "orphan_helper", "test_pure");
    wire_data(&mut board, "chain", "orphan_helper");
    wire_data(&mut board, "shared", "unrelated");
    let mut commands = vec![BoardCommand::RemoveVariable {
        variable_id: "old".into(),
        summary: None,
    }];
    orphan_expressions::remove_orphans(&board, &HashSet::new(), &mut commands);
    assert_eq!(
        removed(&commands),
        BTreeSet::from(["chain", "orphan_helper", "unused"])
    );
}

#[test]
fn affected_expression_cycles_are_removed_without_a_live_consumer() {
    let mut board = chain();
    wire_data(&mut board, "helper", "source");
    let mut commands = vec![remove("consumer")];
    orphan_expressions::remove_orphans(&board, &HashSet::new(), &mut commands);
    assert_eq!(
        removed(&commands),
        BTreeSet::from(["consumer", "helper", "source"])
    );
}

#[test]
fn data_reroutes_do_not_keep_an_unused_expression_alive() {
    let mut board = chain();
    let output = find_output_pin(&board.nodes["helper"], "result")
        .unwrap()
        .id
        .clone();
    let input = find_input_pin(&board.nodes["consumer"], "value")
        .unwrap()
        .id
        .clone();
    board
        .nodes
        .get_mut("helper")
        .unwrap()
        .pins
        .get_mut(&output)
        .unwrap()
        .connected_to
        .clear();
    board
        .nodes
        .get_mut("consumer")
        .unwrap()
        .pins
        .get_mut(&input)
        .unwrap()
        .depends_on
        .clear();
    let mut reroute = Node::new("reroute", "Reroute", "", "test");
    reroute.id = "bend".into();
    let route_in = reroute
        .add_input_pin("route_in", "In", "", VariableType::String)
        .id
        .clone();
    let route_out = reroute
        .add_output_pin("route_out", "Out", "", VariableType::String)
        .id
        .clone();
    board.nodes.insert(reroute.id.clone(), reroute);
    connect(&mut board, "helper", &output, "bend", &route_in);
    connect(&mut board, "bend", &route_out, "consumer", &input);
    let mut commands = vec![remove("consumer")];
    orphan_expressions::remove_orphans(&board, &HashSet::new(), &mut commands);
    assert_eq!(
        removed(&commands),
        BTreeSet::from(["consumer", "helper", "source"])
    );
}

#[test]
fn function_returns_preserve_shared_expressions() {
    let mut board = chain();
    let mut layer = Layer::new("function".into(), "Function".into(), LayerType::Function);
    let mut boundary = Node::new("boundary", "Boundary", "", "test");
    let mut output = boundary
        .add_output_pin("result", "Result", "", VariableType::String)
        .clone();
    output.depends_on.insert(
        find_output_pin(&board.nodes["helper"], "result")
            .unwrap()
            .id
            .clone(),
    );
    layer.pins.insert(output.id.clone(), output);
    board.layers.insert(layer.id.clone(), layer);
    let mut commands = vec![remove("consumer")];
    orphan_expressions::remove_orphans(&board, &HashSet::new(), &mut commands);
    assert_eq!(removed(&commands), BTreeSet::from(["consumer"]));
}
