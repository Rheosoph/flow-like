use super::*;
use crate::state::{FlowLikeConfig, FlowLikeState};
use crate::utils::http::HTTPClient;
use std::sync::Arc;

fn return_catalog() -> Vec<Node> {
    let mut response = Node::new("read_response", "Read Response", "", "test");
    response.add_input_pin("exec_in", "In", "", VariableType::Execution);
    response.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    response
        .add_output_pin("response", "Response", "", VariableType::Struct)
        .schema = Some(r#"{"type":"object","properties":{"title":{"type":"string"}}}"#.to_string());
    let mut rows = Node::new("read_rows", "Read Rows", "", "test");
    rows.add_input_pin("exec_in", "In", "", VariableType::Execution);
    rows.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    rows.add_output_pin("rows", "Rows", "", VariableType::String)
        .value_type = ValueType::Array;
    rows.add_output_pin("index", "Index", "", VariableType::Integer);
    rows.add_output_pin("ready", "Ready", "", VariableType::Boolean);
    let mut member = Node::new("struct_get", "Get Field", "", "structs");
    member.add_input_pin("struct", "Struct", "", VariableType::Struct);
    member.add_input_pin("field", "Field", "", VariableType::String);
    member.add_output_pin("value", "Value", "", VariableType::Generic);
    let mut index = Node::new("array_get", "Get Element", "", "array");
    index
        .add_input_pin("array_in", "Array", "", VariableType::Generic)
        .value_type = ValueType::Array;
    index.add_input_pin("index", "Index", "", VariableType::Integer);
    index.add_output_pin("element", "Element", "", VariableType::Generic);
    let mut make = Node::new("struct_make", "Make Struct", "", "structs");
    make.add_output_pin("struct", "Struct", "", VariableType::Struct);
    let mut set = Node::new("struct_set", "Set Field", "", "structs");
    set.set_flowscript_name("struct", "set");
    set.set_receiver("struct_in");
    set.add_input_pin("exec_in", "In", "", VariableType::Execution);
    set.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    set.add_input_pin("struct_in", "Struct", "", VariableType::Struct);
    set.add_input_pin("field", "Field", "", VariableType::String);
    set.add_input_pin("value", "Value", "", VariableType::Generic);
    set.add_output_pin("struct_out", "Struct", "", VariableType::Struct);
    let mut select = Node::new(TYPES_SELECT_NODE_TYPE, "Select", "", "test");
    select.add_input_pin("condition", "Condition", "", VariableType::Boolean);
    select.add_input_pin("a", "A", "", VariableType::Generic);
    select.add_input_pin("b", "B", "", VariableType::Generic);
    select.add_output_pin("result", "Result", "", VariableType::Generic);
    vec![response, rows, member, index, make, set, select]
}

async fn apply_return_script(board: &mut Board, script: &str, catalog: &[Node]) {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let result =
        crate::flow::ast::apply_flowscript_to_board(board, script, catalog, state, None, true)
            .await
            .expect("return source script applies");
    assert!(
        result.diagnostics.is_empty(),
        "{:?}\n{script}",
        result.diagnostics
    );
}

fn without_statement_anchors(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            line.split_once("//@")
                .map_or(line, |(source, _)| source)
                .trim_end()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn node_pin<'a>(node: &'a Node, name: &str) -> &'a crate::flow::pin::Pin {
    node.pins
        .values()
        .find(|pin| pin.name == name)
        .unwrap_or_else(|| panic!("missing {}.{name}", node.name))
}

fn assert_live_inputs(board: &Board) {
    let pin_ids: HashSet<_> = board
        .nodes
        .values()
        .flat_map(|node| node.pins.keys())
        .chain(board.layers.values().flat_map(|layer| layer.pins.keys()))
        .collect();
    for pin in board
        .nodes
        .values()
        .flat_map(|node| node.pins.values())
        .chain(board.layers.values().flat_map(|layer| layer.pins.values()))
    {
        for source in &pin.depends_on {
            assert!(
                pin_ids.contains(source),
                "{} has deleted source {source}",
                pin.name
            );
        }
    }
    for node in board.nodes.values() {
        let inputs: &[&str] = match node.name.as_str() {
            "struct_get" => &["struct"],
            "array_get" => &["array_in", "index"],
            "struct_set" => &["struct_in", "value"],
            TYPES_SELECT_NODE_TYPE => &["condition", "a", "b"],
            _ => continue,
        };
        for name in inputs {
            let pin = node_pin(node, name);
            assert!(
                !pin.depends_on.is_empty() || pin.default_value.is_some(),
                "{}.{} lost its source",
                node.name,
                name
            );
        }
    }
}

fn assert_direct_source(board: &Board, target: &str, input: &str, source: &str, output: &str) {
    let targets = board
        .nodes
        .values()
        .filter(|node| node.name == target)
        .collect::<Vec<_>>();
    assert!(!targets.is_empty(), "missing {target} accessor");
    for target in targets {
        let source = board
            .nodes
            .values()
            .find(|node| node.name == source && node.layer == target.layer)
            .expect("current producer exists");
        assert_eq!(
            node_pin(target, input).depends_on,
            BTreeSet::from([node_pin(source, output).id.clone()]),
            "{}.{} must read the current producer",
            target.name,
            input
        );
    }
}

#[tokio::test]
async fn anchorless_function_returns_rewire_rebuilt_action_outputs_on_every_apply() {
    let catalog = return_catalog();
    let authored = r#"function returnTitle(): (title: Generic) {
    const response = readResponse()
    return response.title
}
function returnObject(): (result: Struct) {
    const response = readResponse()
    return { title: response.title }
}
function returnFirst(): (row: Generic) {
    const result = readRows()
    return result.rows[0]
}
function returnIndexed(): (row: Generic) {
    const result = readRows()
    return result.rows[result.index]
}
function returnSelected(): (row: Generic) {
    const result = readRows()
    return result.ready ? result.rows[0] : result.rows[1]
}
"#;
    for use_canonical in [false, true] {
        let mut board = empty_board();
        apply_return_script(&mut board, authored, &catalog).await;
        let source = if use_canonical {
            without_statement_anchors(&anchored_text(&board))
        } else {
            authored.to_string()
        };
        // Canonical struct_set syntax stores its empty input as a pin default when rebuilt.
        let expected_count = board.nodes.len() - usize::from(use_canonical);
        for _ in 0..3 {
            let previous_producers = board
                .nodes
                .values()
                .filter(|node| matches!(node.name.as_str(), "read_response" | "read_rows"))
                .map(|node| node.id.clone())
                .collect::<Vec<_>>();
            assert!(!previous_producers.is_empty());
            apply_return_script(&mut board, &source, &catalog).await;
            assert!(
                previous_producers
                    .iter()
                    .all(|id| !board.nodes.contains_key(id)),
                "unanchored actions must be replaced to exercise return source reuse"
            );
            assert_live_inputs(&board);
            assert_direct_source(&board, "struct_get", "struct", "read_response", "response");
            assert_direct_source(&board, "array_get", "array_in", "read_rows", "rows");
            assert_direct_source(
                &board,
                TYPES_SELECT_NODE_TYPE,
                "condition",
                "read_rows",
                "ready",
            );
            let indexed_layer = board
                .layers
                .values()
                .find(|layer| layer.name == "returnIndexed")
                .unwrap();
            let accessor = board
                .nodes
                .values()
                .find(|node| {
                    node.name == "array_get"
                        && node.layer.as_deref() == Some(indexed_layer.id.as_str())
                })
                .unwrap();
            let producer = board
                .nodes
                .values()
                .find(|node| node.name == "read_rows" && node.layer == accessor.layer)
                .unwrap();
            assert_eq!(
                node_pin(accessor, "index").depends_on,
                BTreeSet::from([node_pin(producer, "index").id.clone()])
            );
            assert_eq!(
                board.nodes.len(),
                expected_count,
                "repeated Apply must retain the return graph without accumulating helpers"
            );
        }
        let canonical = anchored_text(&board);
        let metadata = catalog.iter().map(node_to_metadata).collect::<Vec<_>>();
        let result = reconcile_text_with_catalog(&board, &canonical, &metadata);
        assert!(
            result.diagnostics.is_empty(),
            "{:?}\n{canonical}",
            result.diagnostics
        );
        assert!(
            result.commands.is_empty(),
            "{:?}\n{canonical}",
            result.commands
        );
    }
}

#[tokio::test]
async fn applying_original_return_text_repairs_missing_accessor_inputs() {
    let catalog = return_catalog();
    let mut board = empty_board();
    let script = "function title(): (title: Generic) {\n    const response = readResponse()\n    return response.title\n}\nfunction first(): (row: Generic) {\n    const result = readRows()\n    return result.rows[0]\n}";
    apply_return_script(&mut board, script, &catalog).await;
    let source = without_statement_anchors(&anchored_text(&board));
    let broken = board
        .nodes
        .values()
        .filter_map(|node| {
            let input = match node.name.as_str() {
                "struct_get" => "struct",
                "array_get" => "array_in",
                _ => return None,
            };
            Some((node.id.clone(), node_pin(node, input).id.clone()))
        })
        .collect::<Vec<_>>();
    for (node_id, pin_id) in &broken {
        board
            .nodes
            .get_mut(node_id)
            .unwrap()
            .pins
            .get_mut(pin_id)
            .unwrap()
            .depends_on
            .clear();
        for node in board.nodes.values_mut() {
            for pin in node.pins.values_mut() {
                pin.connected_to.remove(pin_id);
            }
        }
    }
    apply_return_script(&mut board, &source, &catalog).await;
    assert_live_inputs(&board);
    assert_direct_source(&board, "struct_get", "struct", "read_response", "response");
    assert_direct_source(&board, "array_get", "array_in", "read_rows", "rows");
}

#[tokio::test]
async fn schema_object_return_rewires_rebuilt_and_missing_field_sources() {
    for missing_input in [false, true] {
        let mut board = empty_board();
        let mut boundary = Node::new("boundary", "Boundary", "", "test");
        let entry = boundary
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .clone();
        let exit = boundary
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .clone();
        let result = boundary
            .add_output_pin("result", "Result", "", VariableType::Struct)
            .clone();
        let mut layer = Layer::new(
            "build".to_string(),
            "build".to_string(),
            LayerType::Function,
        );
        for pin in [&entry, &exit, &result] {
            layer.pins.insert(pin.id.clone(), pin.clone());
        }
        board.layers.insert(layer.id.clone(), layer);
        let mut producer = Node::new("read_title", "Read Title", "", "test");
        producer.id = "producer".to_string();
        producer.layer = Some("build".to_string());
        let producer_in = producer
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let producer_out = producer
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let title = producer
            .add_output_pin("title", "Title", "", VariableType::String)
            .id
            .clone();
        let mut composite = Node::new("struct_make_from_schema", "Make Struct", "", "structs");
        composite.id = "composite".to_string();
        composite.layer = Some("build".to_string());
        let field_name = format!("{}title", super::super::super::lower::MAKE_STRUCT_PREFIX);
        let field = composite
            .add_input_pin(&field_name, "Title", "", VariableType::String)
            .id
            .clone();
        let object = composite
            .add_output_pin("struct", "Struct", "", VariableType::Struct)
            .id
            .clone();
        let catalog = vec![producer.clone(), composite.clone()];
        board.nodes.insert(producer.id.clone(), producer);
        board.nodes.insert(composite.id.clone(), composite);
        connect(&mut board, "build", &entry.id, "producer", &producer_in);
        connect(&mut board, "producer", &producer_out, "build", &exit.id);
        connect(&mut board, "composite", &object, "build", &result.id);
        if !missing_input {
            connect(&mut board, "producer", &title, "composite", &field);
        }
        let source = "function build(): (result: Struct) {\n    const response = readTitle()\n    return { title: response }\n}";
        for _ in 0..3 {
            apply_return_script(&mut board, source, &catalog).await;
            assert_live_inputs(&board);
            assert!(
                board.nodes.contains_key("composite"),
                "the schema constructor should remain reusable"
            );
            assert_direct_source(
                &board,
                "struct_make_from_schema",
                &field_name,
                "read_title",
                "title",
            );
            assert_eq!(board.nodes.len(), 2);
        }
    }
}

#[tokio::test]
async fn anchored_event_return_rewires_accessor_after_replacing_its_action() {
    let mut catalog = return_catalog();
    let mut event = Node::new("events_generic", "Generic Event", "", "events");
    event.set_start(true);
    event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    let mut result = Node::new(
        "events_generic_return_result",
        "Return Result",
        "",
        "events",
    );
    result.set_event_callback(true);
    result.add_input_pin("exec_in", "In", "", VariableType::Execution);
    result.add_input_pin("response", "Response", "", VariableType::Generic);
    catalog.extend([event, result]);
    let mut board = empty_board();
    apply_return_script(
        &mut board,
        "eventsGeneric() {\n    const response = readResponse()\n    return response.title\n}",
        &catalog,
    )
    .await;
    let producer_id = board
        .nodes
        .values()
        .find(|node| node.name == "read_response")
        .unwrap()
        .id
        .clone();
    let source = anchored_text(&board)
        .lines()
        .map(|line| {
            if line.contains(&format!("//@n:{producer_id}")) {
                line.split_once("//@").unwrap().0.trim_end()
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let count = board.nodes.len();
    for _ in 0..3 {
        let previous_producers = board
            .nodes
            .values()
            .filter(|node| matches!(node.name.as_str(), "read_response" | "read_rows"))
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        assert!(!previous_producers.is_empty());
        apply_return_script(&mut board, &source, &catalog).await;
        assert!(
            previous_producers
                .iter()
                .all(|id| !board.nodes.contains_key(id)),
            "unanchored actions must be replaced to exercise return source reuse"
        );
        assert_live_inputs(&board);
        assert_direct_source(&board, "struct_get", "struct", "read_response", "response");
        assert_eq!(board.nodes.len(), count);
    }
}
