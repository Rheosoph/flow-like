use super::*;

fn add_node(
    board: &mut Board,
    id: &str,
    node_type: &str,
    inputs: &[(&str, VariableType)],
    outputs: &[(&str, VariableType)],
) {
    let mut node = Node::new(node_type, node_type, "", "test");
    node.id = id.to_string();
    for (name, ty) in inputs {
        node.add_input_pin(name, name, "", ty.clone());
    }
    for (name, ty) in outputs {
        node.add_output_pin(name, name, "", ty.clone());
    }
    board.nodes.insert(id.to_string(), node);
}

fn wire(board: &mut Board, source: &str, output: &str, target: &str, input: &str) {
    let from = board.nodes[source]
        .pins
        .values()
        .find(|pin| pin.name == output)
        .unwrap()
        .id
        .clone();
    let to = board.nodes[target]
        .pins
        .values()
        .find(|pin| pin.name == input)
        .unwrap()
        .id
        .clone();
    connect(board, source, &from, target, &to);
}

fn literal(board: &mut Board, node: &str, input: &str, value: flow_like_types::Value) {
    board
        .nodes
        .get_mut(node)
        .unwrap()
        .pins
        .values_mut()
        .find(|pin| pin.name == input)
        .unwrap()
        .default_value = Some(flow_like_types::json::to_vec(&value).unwrap());
}

fn event_board() -> Board {
    let mut board = empty_board();
    add_node(
        &mut board,
        "event",
        "events_generic",
        &[],
        &[
            ("exec_out", VariableType::Execution),
            ("payload", VariableType::String),
        ],
    );
    board.nodes.get_mut("event").unwrap().set_start(true);
    board
}

fn assert_retains_nodes(board: &Board, source: &str) -> ReconcileResult {
    let result = reconcile_text_with_catalog(board, source, &board_derived_catalog(board));
    assert!(
        result.diagnostics.is_empty(),
        "{:?}\n{source}",
        result.diagnostics
    );
    assert!(
        result.commands.iter().all(|command| !matches!(
            command,
            BoardCommand::AddNode { .. } | BoardCommand::RemoveNode { .. }
        )),
        "repeated source must retain its expression helpers: {:?}",
        result.commands
    );
    result
}

fn computed_object_board() -> Board {
    let mut board = event_board();
    add_node(
        &mut board,
        "make",
        "struct_make",
        &[],
        &[("struct", VariableType::Struct)],
    );
    for id in ["first", "second"] {
        add_node(
            &mut board,
            id,
            "struct_set",
            &[
                ("exec_in", VariableType::Execution),
                ("struct_in", VariableType::Struct),
                ("field", VariableType::String),
                ("value", VariableType::Generic),
            ],
            &[
                ("exec_out", VariableType::Execution),
                ("struct_out", VariableType::Struct),
            ],
        );
    }
    literal(
        &mut board,
        "first",
        "field",
        flow_like_types::json::json!("a"),
    );
    literal(
        &mut board,
        "second",
        "field",
        flow_like_types::json::json!("b"),
    );
    literal(
        &mut board,
        "second",
        "value",
        flow_like_types::json::json!("fixed"),
    );
    add_node(
        &mut board,
        "sink",
        "consume_struct",
        &[
            ("exec_in", VariableType::Execution),
            ("value", VariableType::Struct),
        ],
        &[("exec_out", VariableType::Execution)],
    );
    for (from, output, to, input) in [
        ("event", "exec_out", "first", "exec_in"),
        ("first", "exec_out", "second", "exec_in"),
        ("second", "exec_out", "sink", "exec_in"),
        ("event", "payload", "first", "value"),
        ("make", "struct", "first", "struct_in"),
        ("first", "struct_out", "second", "struct_in"),
        ("second", "struct_out", "sink", "value"),
    ] {
        wire(&mut board, from, output, to, input);
    }
    board
}

#[test]
fn computed_object_reuses_generated_setters_on_repeated_apply() {
    let board = computed_object_board();
    let source = "eventsGeneric(payload: string) { //@n:event\n    consumeStruct({ value: { a: payload, b: \"fixed\" } }) //@n:sink\n}";
    for _ in 0..4 {
        let result = assert_retains_nodes(&board, source);
        assert!(result.commands.is_empty(), "{:?}", result.commands);
    }
    let edited = assert_retains_nodes(&board, &source.replace("fixed", "edited"));
    assert!(edited.commands.iter().any(|command| matches!(command,
        BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
            if node_id == "second" && pin_id == "value" && value == &flow_like_types::json::json!("edited")
    )));
}

#[test]
fn computed_singleton_array_reuses_generated_push_on_repeated_apply() {
    let mut board = event_board();
    add_node(
        &mut board,
        "push",
        "array_push",
        &[
            ("exec_in", VariableType::Execution),
            ("array_in", VariableType::String),
            ("value", VariableType::String),
        ],
        &[
            ("exec_out", VariableType::Execution),
            ("array_out", VariableType::String),
        ],
    );
    add_node(
        &mut board,
        "sink",
        "consume_array",
        &[
            ("exec_in", VariableType::Execution),
            ("value", VariableType::String),
        ],
        &[("exec_out", VariableType::Execution)],
    );
    for (id, pin) in [
        ("push", "array_in"),
        ("push", "array_out"),
        ("sink", "value"),
    ] {
        board
            .nodes
            .get_mut(id)
            .unwrap()
            .pins
            .values_mut()
            .find(|p| p.name == pin)
            .unwrap()
            .value_type = ValueType::Array;
    }
    literal(
        &mut board,
        "push",
        "array_in",
        flow_like_types::json::json!([]),
    );
    for (from, output, to, input) in [
        ("event", "exec_out", "push", "exec_in"),
        ("push", "exec_out", "sink", "exec_in"),
        ("event", "payload", "push", "value"),
        ("push", "array_out", "sink", "value"),
    ] {
        wire(&mut board, from, output, to, input);
    }
    let result = assert_retains_nodes(
        &board,
        "eventsGeneric(payload: string) { //@n:event\n    consumeArray({ value: [payload] }) //@n:sink\n}",
    );
    assert!(result.commands.is_empty(), "{:?}", result.commands);
}

#[test]
fn null_comparison_reuses_its_inspection_and_negation() {
    for negated in [false, true] {
        let mut board = event_board();
        add_node(
            &mut board,
            "inspect",
            "utils_types_type_of",
            &[("value", VariableType::Generic)],
            &[
                ("type", VariableType::String),
                ("is_null", VariableType::Boolean),
            ],
        );
        add_node(
            &mut board,
            "sink",
            "log_bool",
            &[
                ("exec_in", VariableType::Execution),
                ("value", VariableType::Boolean),
            ],
            &[("exec_out", VariableType::Execution)],
        );
        wire(&mut board, "event", "exec_out", "sink", "exec_in");
        wire(&mut board, "event", "payload", "inspect", "value");
        if negated {
            add_node(
                &mut board,
                "negate",
                "bool_not",
                &[("boolean", VariableType::Boolean)],
                &[("result", VariableType::Boolean)],
            );
            wire(&mut board, "inspect", "is_null", "negate", "boolean");
            wire(&mut board, "negate", "result", "sink", "value");
        } else {
            wire(&mut board, "inspect", "is_null", "sink", "value");
        }
        let op = if negated { "!=" } else { "==" };
        let result = assert_retains_nodes(
            &board,
            &format!(
                "eventsGeneric(payload: string) {{ //@n:event\n    logBool({{ value: payload {op} null }}) //@n:sink\n}}"
            ),
        );
        assert!(result.commands.is_empty(), "{:?}", result.commands);
    }
}

#[test]
fn deletion_preview_ids_are_stable_across_identical_attempts() {
    let mut board = empty_board();
    for index in (0..64).rev() {
        log_node(&mut board, &format!("log-{index:02}"));
    }
    let expected = (0..64)
        .map(|index| format!("log-{index:02}"))
        .collect::<Vec<_>>();
    for _ in 0..8 {
        let result = reconcile_text(&board, "");
        let removed = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::RemoveNode { node_id, .. } => Some(node_id.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(removed, expected);
    }
}

#[test]
fn a_bare_variable_reference_does_not_revive_its_deleted_assignment() {
    let mut board = event_board();
    let mut variable = Variable::new("config", VariableType::String, ValueType::Normal);
    variable.id = "config-variable".to_string();
    board.variables.insert(variable.id.clone(), variable);
    add_node(
        &mut board,
        "setter",
        "variable_set",
        &[
            ("exec_in", VariableType::Execution),
            ("var_ref", VariableType::String),
            ("value_in", VariableType::String),
        ],
        &[
            ("exec_out", VariableType::Execution),
            ("value_ref", VariableType::String),
        ],
    );
    literal(
        &mut board,
        "setter",
        "var_ref",
        flow_like_types::json::json!("config-variable"),
    );
    literal(
        &mut board,
        "setter",
        "value_in",
        flow_like_types::json::json!("configured"),
    );
    add_node(
        &mut board,
        "sink",
        "log",
        &[
            ("exec_in", VariableType::Execution),
            ("message", VariableType::String),
        ],
        &[("exec_out", VariableType::Execution)],
    );
    wire(&mut board, "event", "exec_out", "setter", "exec_in");
    wire(&mut board, "setter", "exec_out", "sink", "exec_in");
    wire(&mut board, "setter", "value_ref", "sink", "message");
    let source = anchored_text(&board)
        .lines()
        .filter(|line| !line.contains("//@n:setter"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut catalog = board_derived_catalog(&board);
    catalog.push(catalog_meta(
        "variable_get",
        "Get Variable",
        vec![pin_meta("var_ref", "String", PinType::Input)],
        vec![pin_meta("value_ref", "String", PinType::Output)],
    ));
    let result = reconcile_text_with_catalog(&board, &source, &catalog);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert!(result.commands.iter().any(|command| matches!(command,
        BoardCommand::RemoveNode { node_id, .. } if node_id == "setter")));
    assert!(result.commands.iter().any(|command| matches!(command,
        BoardCommand::AddNode { node_type, .. } if node_type == "variable_get")));
}

#[tokio::test]
async fn applied_computed_object_keeps_helper_ids_on_repeated_apply() {
    use crate::state::{FlowLikeConfig, FlowLikeState};
    use crate::utils::http::HTTPClient;
    use std::sync::Arc;

    let mut templates = event_board();
    add_node(
        &mut templates,
        "make",
        "struct_make",
        &[],
        &[("struct", VariableType::Struct)],
    );
    add_node(
        &mut templates,
        "setter",
        "struct_set",
        &[
            ("exec_in", VariableType::Execution),
            ("struct_in", VariableType::Struct),
            ("field", VariableType::String),
            ("value", VariableType::Generic),
        ],
        &[
            ("exec_out", VariableType::Execution),
            ("struct_out", VariableType::Struct),
        ],
    );
    add_node(
        &mut templates,
        "sink",
        "consume_struct",
        &[
            ("exec_in", VariableType::Execution),
            ("value", VariableType::Struct),
        ],
        &[("exec_out", VariableType::Execution)],
    );
    let catalog = templates.nodes.into_values().collect::<Vec<_>>();
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let mut board = empty_board();
    let source = "eventsGeneric(payload: string) {\n    consumeStruct({ value: { a: payload, b: \"fixed\" } })\n}";
    let first = super::super::super::apply_flowscript_to_board(
        &mut board,
        source,
        &catalog,
        state.clone(),
        None,
        false,
    )
    .await
    .expect("computed object applies from scratch");
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
    let event = board
        .nodes
        .values()
        .find(|node| node.name == "events_generic")
        .unwrap()
        .id
        .clone();
    let sink = board
        .nodes
        .values()
        .find(|node| node.name == "consume_struct")
        .unwrap()
        .id
        .clone();
    let source = format!(
        "eventsGeneric(payload: string) {{ //@n:{event}\n    consumeStruct({{ value: {{ a: payload, b: \"fixed\" }} }}) //@n:{sink}\n}}"
    );
    let identities = board.nodes.keys().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        identities.len(),
        5,
        "event, consumer and three object helpers"
    );
    for _ in 0..3 {
        let repeated = super::super::super::apply_flowscript_to_board(
            &mut board,
            &source,
            &catalog,
            state.clone(),
            None,
            false,
        )
        .await
        .expect("identical source requires no deletion approval");
        assert!(
            repeated.diagnostics.is_empty(),
            "{:?}",
            repeated.diagnostics
        );
        assert!(
            repeated.commands.is_empty(),
            "{} unexpected commands",
            repeated.commands.len()
        );
        assert_eq!(
            board.nodes.keys().cloned().collect::<BTreeSet<_>>(),
            identities
        );
    }
}

#[test]
fn shared_generated_helpers_are_claimed_by_only_one_inline_evaluation() {
    for value in ["payload", "\"changed\""] {
        let mut board = computed_object_board();
        add_node(
            &mut board,
            "other",
            "consume_struct",
            &[
                ("exec_in", VariableType::Execution),
                ("value", VariableType::Struct),
            ],
            &[("exec_out", VariableType::Execution)],
        );
        wire(&mut board, "sink", "exec_out", "other", "exec_in");
        wire(&mut board, "second", "struct_out", "other", "value");
        let source = format!(
            "eventsGeneric(payload: string) {{ //@n:event\n    consumeStruct({{ value: {{ a: payload, b: \"fixed\" }} }}) //@n:sink\n    consumeStruct({{ value: {{ a: {value}, b: \"fixed\" }} }}) //@n:other\n}}"
        );
        let catalog = all_board_nodes(&board)
            .into_iter()
            .fold(BTreeMap::new(), |mut types, node| {
                types
                    .entry(node.name.clone())
                    .or_insert_with(|| node_to_metadata(node));
                types
            })
            .into_values()
            .collect::<Vec<_>>();
        let result = reconcile_text_with_catalog(&board, &source, &catalog);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            !result.commands.iter().any(|command| matches!(command,
            BoardCommand::RemoveNode { node_id, .. } if node_id == "first" || node_id == "second"))
        );
        assert!(!result.commands.iter().any(|command| matches!(command,
            BoardCommand::UpdateNodePin { node_id, pin_id, .. } if node_id == "first" && pin_id == "value")),
            "the second expression must not change the first consumer's helper: {:?}", result.commands);
        if value == "payload" {
            assert!(
                result.commands.iter().any(|command| matches!(command,
                BoardCommand::AddNode { node_type, .. } if node_type == "struct_set")),
                "two inline evaluations need independent impure helpers: {:?}",
                result.commands
            );
        } else {
            assert!(result.commands.iter().any(|command| matches!(command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "other" && pin_id == "value" && value == &flow_like_types::json::json!({"a": "changed", "b": "fixed"}))),
                "a constant second object belongs on its own consumer: {:?}", result.commands);
        }
    }
}

#[test]
fn retained_helpers_move_to_their_inline_expression_position() {
    let mut board = computed_object_board();
    log_node(&mut board, "middle");
    wire(&mut board, "second", "exec_out", "middle", "exec_in");
    wire(&mut board, "middle", "exec_out", "sink", "exec_in");
    let result = assert_retains_nodes(
        &board,
        "eventsGeneric(payload: string) { //@n:event\n    log({ message: \"after\" }) //@n:middle\n    consumeStruct({ value: { a: payload, b: \"fixed\" } }) //@n:sink\n}",
    );
    for (source, target) in [("event", "middle"), ("middle", "first"), ("second", "sink")] {
        assert!(result.commands.iter().any(|command| matches!(command,
            BoardCommand::ConnectPins { from_node, to_node, .. } if from_node == source && to_node == target)),
            "missing execution edge {source} -> {target}: {:?}", result.commands);
    }
}

#[test]
fn shared_array_helper_splits_when_authored_elements_diverge() {
    for value in ["fixed", "changed"] {
        let mut board = event_board();
        add_node(
            &mut board,
            "array",
            "construct_array",
            &[
                ("element", VariableType::String),
                ("element", VariableType::String),
            ],
            &[("array_out", VariableType::String)],
        );
        let array = board.nodes.get_mut("array").unwrap();
        array
            .pins
            .values_mut()
            .find(|pin| pin.name == "array_out")
            .unwrap()
            .value_type = ValueType::Array;
        let mut elements = array
            .pins
            .values_mut()
            .filter(|pin| pin.name == "element")
            .collect::<Vec<_>>();
        elements.sort_by_key(|pin| pin.index);
        elements[1].default_value = Some(b"\"fixed\"".to_vec());
        let first_element = elements[0].id.clone();
        let payload = board.nodes["event"]
            .pins
            .values()
            .find(|pin| pin.name == "payload")
            .unwrap()
            .id
            .clone();
        connect(&mut board, "event", &payload, "array", &first_element);
        for id in ["sink", "other"] {
            add_node(
                &mut board,
                id,
                "consume_array",
                &[
                    ("exec_in", VariableType::Execution),
                    ("value", VariableType::String),
                ],
                &[("exec_out", VariableType::Execution)],
            );
            board
                .nodes
                .get_mut(id)
                .unwrap()
                .pins
                .values_mut()
                .find(|pin| pin.name == "value")
                .unwrap()
                .value_type = ValueType::Array;
            wire(&mut board, "array", "array_out", id, "value");
        }
        wire(&mut board, "event", "exec_out", "sink", "exec_in");
        wire(&mut board, "sink", "exec_out", "other", "exec_in");
        let source = format!(
            "eventsGeneric(payload: string) {{ //@n:event\n    consumeArray({{ value: [payload, \"fixed\"] }}) //@n:sink\n    consumeArray({{ value: [payload, \"{value}\"] }}) //@n:other\n}}"
        );
        let result = reconcile_text_with_catalog(&board, &source, &board_derived_catalog(&board));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        if value == "fixed" {
            assert!(result.commands.is_empty(), "{:?}", result.commands);
        } else {
            assert!(result.commands.iter().any(|command| matches!(command,
                BoardCommand::AddNode { node_type, .. } if node_type == "construct_array")));
            assert!(!result.commands.iter().any(|command| matches!(command,
                BoardCommand::UpdateNodePin { node_id, .. } if node_id == "array")));
        }
    }
}

#[test]
fn shared_null_inspection_splits_when_authored_operands_diverge() {
    for operand in ["payload", "\"changed\""] {
        let mut board = event_board();
        add_node(
            &mut board,
            "inspect",
            "utils_types_type_of",
            &[("value", VariableType::Generic)],
            &[
                ("type", VariableType::String),
                ("is_null", VariableType::Boolean),
            ],
        );
        wire(&mut board, "event", "payload", "inspect", "value");
        for id in ["sink", "other"] {
            add_node(
                &mut board,
                id,
                "log_bool",
                &[
                    ("exec_in", VariableType::Execution),
                    ("value", VariableType::Boolean),
                ],
                &[("exec_out", VariableType::Execution)],
            );
            wire(&mut board, "inspect", "is_null", id, "value");
        }
        wire(&mut board, "event", "exec_out", "sink", "exec_in");
        wire(&mut board, "sink", "exec_out", "other", "exec_in");
        let source = format!(
            "eventsGeneric(payload: string) {{ //@n:event\n    logBool({{ value: payload == null }}) //@n:sink\n    logBool({{ value: {operand} == null }}) //@n:other\n}}"
        );
        let result = reconcile_text_with_catalog(&board, &source, &board_derived_catalog(&board));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        if operand == "payload" {
            assert!(result.commands.is_empty(), "{:?}", result.commands);
        } else {
            assert!(result.commands.iter().any(|command| matches!(command,
                BoardCommand::AddNode { node_type, .. } if node_type == "utils_types_type_of")));
            assert!(!result.commands.iter().any(|command| matches!(command,
                BoardCommand::UpdateNodePin { node_id, .. } if node_id == "inspect")));
        }
    }
}
