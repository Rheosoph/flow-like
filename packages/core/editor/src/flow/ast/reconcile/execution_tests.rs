use super::*;

fn swap_anchored_lines(text: &str, left: &str, right: &str) -> String {
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    let left = lines
        .iter()
        .position(|line| line.contains(&format!("//@n:{left}")))
        .expect("left statement is rendered");
    let right = lines
        .iter()
        .position(|line| line.contains(&format!("//@n:{right}")))
        .expect("right statement is rendered");
    lines.swap(left, right);
    lines.join("\n")
}

fn connection_pin_id(board: &Board, owner: &str, reference: &str) -> String {
    let pins = board
        .nodes
        .get(owner)
        .map(|node| &node.pins)
        .or_else(|| board.layers.get(owner).map(|layer| &layer.pins))
        .expect("connection owner exists");
    if let Some(pin) = pins.get(reference) {
        return pin.id.clone();
    }
    let (name, occurrence) = parse_pin_occurrence_ref(reference).unwrap_or((reference, 0));
    let mut matching = pins
        .values()
        .filter(|pin| pin.name == name)
        .collect::<Vec<_>>();
    matching.sort_by_key(|pin| (pin.index, pin.id.as_str()));
    matching
        .get(occurrence)
        .unwrap_or_else(|| panic!("missing pin {owner}.{reference}"))
        .id
        .clone()
}

fn apply_existing_connections(board: &mut Board, result: &ReconcileResult) {
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    for command in &result.commands {
        let (from_node, from_pin, to_node, to_pin, disconnect) = match command {
            BoardCommand::ConnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => (from_node, from_pin, to_node, to_pin, false),
            BoardCommand::DisconnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => (from_node, from_pin, to_node, to_pin, true),
            BoardCommand::AddNode { .. } | BoardCommand::RemoveNode { .. } => {
                panic!("rewiring existing statements must retain their nodes: {command:?}");
            }
            _ => continue,
        };
        let from_pin = connection_pin_id(board, from_node, from_pin);
        let to_pin = connection_pin_id(board, to_node, to_pin);
        if disconnect {
            crate::flow::board::commands::pins::connect_pins::disconnect_pins(
                board, from_node, &from_pin, to_node, &to_pin,
            )
            .expect("disconnect existing pins");
        } else {
            connect(board, from_node, &from_pin, to_node, &to_pin);
        }
    }
}

fn assert_only_exec_target(board: &Board, source: &str, output: &str, target_pin: &str) {
    let output = connection_pin_id(board, source, output);
    let pin = board
        .nodes
        .get(source)
        .and_then(|node| node.pins.get(&output))
        .or_else(|| {
            board
                .layers
                .get(source)
                .and_then(|layer| layer.pins.get(&output))
        })
        .expect("execution output exists");
    assert_eq!(
        pin.connected_to,
        BTreeSet::from([target_pin.to_string()]),
        "unexpected execution targets for {source}.{}",
        pin.name
    );
}

#[test]
fn reordering_existing_function_statements_rewires_entry_and_exit() {
    let mut board = empty_board();
    let mut boundary = Node::new("boundary", "Boundary", "", "test");
    let entry = boundary
        .add_input_pin("exec_in", "Exec In", "", VariableType::Execution)
        .clone();
    let exit = boundary
        .add_output_pin("exec_out", "Exec Out", "", VariableType::Execution)
        .clone();
    let mut layer = Layer::new(
        "function".to_string(),
        "runLogs".to_string(),
        LayerType::Function,
    );
    layer.pins.insert(entry.id.clone(), entry.clone());
    layer.pins.insert(exit.id.clone(), exit.clone());
    board.layers.insert(layer.id.clone(), layer);

    let (first_in, first_out) = log_node(&mut board, "first");
    let (second_in, second_out) = log_node(&mut board, "second");
    for id in ["first", "second"] {
        board.nodes.get_mut(id).expect("body node").layer = Some("function".to_string());
    }
    connect(&mut board, "function", &entry.id, "first", &first_in);
    connect(&mut board, "first", &first_out, "second", &second_in);
    connect(&mut board, "second", &second_out, "function", &exit.id);

    let text = anchored_text(&board);
    let catalog = board_derived_catalog(&board);
    let unchanged = reconcile_text_with_catalog(&board, &text, &catalog);
    assert!(
        unchanged.diagnostics.is_empty(),
        "{:?}\n{text}",
        unchanged.diagnostics
    );
    assert!(unchanged.commands.is_empty(), "{:?}", unchanged.commands);

    let edited = swap_anchored_lines(&text, "first", "second");
    let result = reconcile_text_with_catalog(&board, &edited, &catalog);
    apply_existing_connections(&mut board, &result);

    assert_only_exec_target(&board, "function", "exec_in", &second_in);
    assert_only_exec_target(&board, "second", "exec_out", &first_in);
    assert_only_exec_target(&board, "first", "exec_out", &exit.id);
    assert_eq!(
        board.nodes["second"].pins[&second_in].depends_on,
        BTreeSet::from([entry.id])
    );
    assert_eq!(
        board.layers["function"].pins[&exit.id].depends_on,
        BTreeSet::from([first_out])
    );
}

#[test]
fn swapping_existing_repeated_exec_arms_uses_each_output_occurrence() {
    let mut board = empty_board();
    let event_out = start_event(&mut board);
    let mut parallel = Node::new("control_par_execution", "Parallel Execution", "", "control");
    parallel.id = "parallel".to_string();
    let parallel_in = parallel
        .add_input_pin("exec_in", "In", "", VariableType::Execution)
        .id
        .clone();
    let first_output = parallel
        .add_output_pin("exec_out", "Out", "", VariableType::Execution)
        .id
        .clone();
    let second_output = parallel
        .add_output_pin("exec_out", "Out", "", VariableType::Execution)
        .id
        .clone();
    board.nodes.insert(parallel.id.clone(), parallel);
    let (first_in, _) = log_node(&mut board, "first");
    let (second_in, _) = log_node(&mut board, "second");
    connect(&mut board, "event", &event_out, "parallel", &parallel_in);
    connect(&mut board, "parallel", &first_output, "first", &first_in);
    connect(&mut board, "parallel", &second_output, "second", &second_in);

    let text = anchored_text(&board);
    let catalog = board_derived_catalog(&board);
    let unchanged = reconcile_text_with_catalog(&board, &text, &catalog);
    assert!(
        unchanged.diagnostics.is_empty(),
        "{:?}\n{text}",
        unchanged.diagnostics
    );
    assert!(unchanged.commands.is_empty(), "{:?}", unchanged.commands);

    let edited = swap_anchored_lines(&text, "first", "second");
    let result = reconcile_text_with_catalog(&board, &edited, &catalog);
    apply_existing_connections(&mut board, &result);

    assert_only_exec_target(&board, "parallel", &first_output, &second_in);
    assert_only_exec_target(&board, "parallel", &second_output, &first_in);
    assert_eq!(
        board.nodes["first"].pins[&first_in].depends_on,
        BTreeSet::from([second_output])
    );
    assert_eq!(
        board.nodes["second"].pins[&second_in].depends_on,
        BTreeSet::from([first_output])
    );
}

#[test]
fn scoped_existing_reorder_preserves_another_events_fan_in() {
    let mut board = empty_board();
    let event_out = start_event(&mut board);
    board
        .nodes
        .get_mut("event")
        .expect("edited event")
        .coordinates = Some((0.0, 0.0, 0.0));
    let mut outside = Node::new("events_simple", "Outside", "", "events");
    outside.id = "outside".to_string();
    outside.set_start(true);
    outside.coordinates = Some((0.0, 500.0, 0.0));
    let outside_out = outside
        .add_output_pin("exec_out", "Out", "", VariableType::Execution)
        .id
        .clone();
    board.nodes.insert(outside.id.clone(), outside);
    let (first_in, first_out) = log_node(&mut board, "first");
    let (second_in, second_out) = log_node(&mut board, "second");
    let (tail_in, _) = log_node(&mut board, "tail");
    connect(&mut board, "event", &event_out, "first", &first_in);
    connect(&mut board, "first", &first_out, "second", &second_in);
    connect(&mut board, "second", &second_out, "tail", &tail_in);
    connect(&mut board, "outside", &outside_out, "second", &second_in);

    let catalog = board_derived_catalog(&board);
    let mut ast = flow_like_ast::parse(&anchored_text(&board)).expect("rendered source parses");
    ast.events
        .retain(|event| event.anchor.as_deref() == Some("event"));
    assert_eq!(ast.events.len(), 1);
    assert_eq!(ast.events[0].body.stmts.len(), 3);
    let scope = ["event".to_string()];
    let unchanged = reconcile_with_catalog_scoped(&board, &ast, &catalog, Some(&scope));
    assert!(
        unchanged.diagnostics.is_empty(),
        "{:?}",
        unchanged.diagnostics
    );
    assert!(unchanged.commands.is_empty(), "{:?}", unchanged.commands);

    ast.events[0].body.stmts.swap(0, 1);
    let result = reconcile_with_catalog_scoped(&board, &ast, &catalog, Some(&scope));
    assert!(
        result.commands.iter().all(|command| match command {
            BoardCommand::ConnectPins { from_node, .. }
            | BoardCommand::DisconnectPins { from_node, .. } => from_node != "outside",
            _ => true,
        }),
        "out-of-scope predecessor must keep its edge: {:?}",
        result.commands
    );
    apply_existing_connections(&mut board, &result);

    assert_only_exec_target(&board, "event", "exec_out", &second_in);
    assert_only_exec_target(&board, "second", "exec_out", &first_in);
    assert_only_exec_target(&board, "first", "exec_out", &tail_in);
    assert_only_exec_target(&board, "outside", "exec_out", &second_in);
    assert_eq!(
        board.nodes["second"].pins[&second_in].depends_on,
        BTreeSet::from([event_out, outside_out]),
        "the edited event and external event must both reach the shared statement"
    );
}

#[test]
fn moving_a_shared_statement_rejects_a_cycle_through_an_unrendered_edge() {
    let mut board = empty_board();
    let first_event_out = start_event(&mut board);
    board
        .nodes
        .get_mut("event")
        .expect("first event")
        .coordinates = Some((0.0, 0.0, 0.0));
    let mut second_event = Node::new("events_simple", "Later", "", "events");
    second_event.id = "later_event".to_string();
    second_event.set_start(true);
    second_event.coordinates = Some((0.0, 500.0, 0.0));
    let second_event_out = second_event
        .add_output_pin("exec_out", "Out", "", VariableType::Execution)
        .id
        .clone();
    board.nodes.insert(second_event.id.clone(), second_event);
    let (first_in, first_out) = log_node(&mut board, "first");
    let (shared_in, _) = log_node(&mut board, "shared");
    connect(&mut board, "event", &first_event_out, "shared", &shared_in);
    connect(
        &mut board,
        "later_event",
        &second_event_out,
        "first",
        &first_in,
    );
    connect(&mut board, "first", &first_out, "shared", &shared_in);

    let catalog = board_derived_catalog(&board);
    let text = anchored_text(&board);
    let unchanged = reconcile_text_with_catalog(&board, &text, &catalog);
    assert!(
        unchanged.diagnostics.is_empty(),
        "{:?}\n{text}",
        unchanged.diagnostics
    );
    assert!(unchanged.commands.is_empty(), "{:?}", unchanged.commands);

    let mut ast = flow_like_ast::parse(&text).expect("rendered source parses");
    let first_event = ast
        .events
        .iter_mut()
        .find(|event| event.anchor.as_deref() == Some("event"))
        .expect("first event is rendered");
    assert_eq!(first_event.body.stmts.len(), 1);
    let shared = first_event.body.stmts.remove(0);
    assert_eq!(shared.anchor(), Some("shared"));
    let second_event = ast
        .events
        .iter_mut()
        .find(|event| event.anchor.as_deref() == Some("later_event"))
        .expect("second event is rendered");
    assert_eq!(second_event.body.stmts.len(), 1);
    assert_eq!(second_event.body.stmts[0].anchor(), Some("first"));
    second_event.body.stmts.insert(0, shared);

    // Lowering shows the shared statement only under the first event. Moving it before
    // `first` in the second event must account for the surviving, unrendered first -> shared edge.
    let result = reconcile_with_catalog(&board, &ast, &catalog);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("cycle")),
        "moving a shared statement must reject the hidden execution cycle: {:?}; commands: {:?}",
        result.diagnostics,
        result.commands
    );
}
