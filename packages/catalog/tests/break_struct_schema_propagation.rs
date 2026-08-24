//! Break Struct derives its field pins from the schema the *producer* pin declares, so anything
//! that blanks that schema on the way deletes every field pin — and with them every wire the user
//! drew from them.
//!
//! Regression: `struct_in` carries `Pin::set_open_schema()`, the marker meaning "any struct fits
//! here". Generic passthrough nodes (`Get Element`, `For Each`, …) resolve their own types by
//! copying a peer pin's schema, and an output pin reaches its *consumer* through `connected_to`.
//! `array_get` therefore inherited the marker backwards from `struct_in`, and `harmonize_type`
//! stamped it over the real element schema — so
//! `Get File Input Files -> Get Element -> Break Struct` reported "Cannot break dynamic object
//! types (e.g., HashMap)" and dropped all of its outputs. The marker declares that a shape is
//! open; it must never displace a concrete schema.
//!
//! The second half of these tests covers what makes that class of bug destructive rather than
//! merely wrong: a dropped field pin takes its half of the edge with it, `fix_pin_connections`
//! prunes the other half, and the user's wire is gone with no error anywhere. A field pin the user
//! has wired must survive any schema change and be reported instead.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use flow_like::flow::board::Board;
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::flow::pin::{PinType, is_open_object_schema};
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;

const BREAK_STRUCT_PIN_PREFIX: &str = "__break_struct_field__";

fn catalog() -> HashMap<String, Arc<dyn NodeLogic>> {
    CatalogBuilder::new()
        .build()
        .into_iter()
        .map(|logic| (logic.get_node().name, logic))
        .collect()
}

fn empty_board() -> Board {
    let mut board = Board::new_detached(Some("break-struct-test".to_string()), Path::default());
    board.name = "Break Struct Test".to_string();
    board.description.clear();
    board.viewport = (0.0, 0.0, 1.0);
    board.hash = None;
    board.created_at = SystemTime::UNIX_EPOCH;
    board.updated_at = SystemTime::UNIX_EPOCH;
    board
}

fn place(board: &mut Board, logics: &HashMap<String, Arc<dyn NodeLogic>>, node_name: &str) -> Node {
    let mut node = logics
        .get(node_name)
        .unwrap_or_else(|| panic!("`{node_name}` is in the catalog"))
        .get_node();
    node.id = node_name.to_string();
    board.nodes.insert(node.id.clone(), node.clone());
    node
}

fn pin_id(board: &Board, node_id: &str, pin_name: &str) -> String {
    board.nodes[node_id]
        .pins
        .values()
        .find(|pin| pin.name == pin_name)
        .unwrap_or_else(|| panic!("`{node_id}` has a `{pin_name}` pin"))
        .id
        .clone()
}

fn pin_named<'a>(board: &'a Board, node_id: &str, pin_name: &str) -> &'a flow_like::flow::pin::Pin {
    board.nodes[node_id]
        .pins
        .values()
        .find(|pin| pin.name == pin_name)
        .unwrap_or_else(|| panic!("`{node_id}` has a `{pin_name}` pin"))
}

fn connect(board: &mut Board, source: (&str, &str), target: (&str, &str)) {
    let source_pin = pin_id(board, source.0, source.1);
    let target_pin = pin_id(board, target.0, target.1);

    board
        .nodes
        .get_mut(source.0)
        .unwrap()
        .pins
        .get_mut(&source_pin)
        .unwrap()
        .connected_to
        .insert(target_pin.clone());

    board
        .nodes
        .get_mut(target.0)
        .unwrap()
        .pins
        .get_mut(&target_pin)
        .unwrap()
        .depends_on
        .insert(source_pin);
}

fn disconnect(board: &mut Board, source: (&str, &str), target: (&str, &str)) {
    let source_pin = pin_id(board, source.0, source.1);
    let target_pin = pin_id(board, target.0, target.1);

    board
        .nodes
        .get_mut(source.0)
        .unwrap()
        .pins
        .get_mut(&source_pin)
        .unwrap()
        .connected_to
        .remove(&target_pin);

    board
        .nodes
        .get_mut(target.0)
        .unwrap()
        .pins
        .get_mut(&target_pin)
        .unwrap()
        .depends_on
        .remove(&source_pin);
}

/// Mirrors `Board::settle_every_node`: each node is lifted out of the board for its own
/// `on_update`, and the sweep repeats so one node's retyping reaches its neighbours.
async fn settle(board: &mut Board, logics: &HashMap<String, Arc<dyn NodeLogic>>) {
    for _ in 0..4 {
        let node_ids: Vec<String> = board.nodes.keys().cloned().collect();
        for node_id in node_ids {
            let Some(mut node) = board.nodes.remove(&node_id) else {
                continue;
            };
            if let Some(logic) = logics.get(&node.name) {
                logic.on_update(&mut node, board).await;
            }
            board.nodes.insert(node_id, node);
        }
    }
}

fn field_pins(board: &Board, node_id: &str) -> Vec<String> {
    let mut fields: Vec<String> = board.nodes[node_id]
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Output)
        .filter_map(|pin| {
            pin.name
                .strip_prefix(BREAK_STRUCT_PIN_PREFIX)
                .map(str::to_string)
        })
        .collect();
    fields.sort();
    fields
}

/// `Get File Input Files -> Get Element -> Break Struct`, the shape the regression was reported on.
#[flow_like_types::tokio::test]
async fn a_typed_array_keeps_its_schema_through_get_element() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "a2ui_get_file_input_files");
    place(&mut board, &logics, "array_get");
    place(&mut board, &logics, "struct_break");

    connect(
        &mut board,
        ("a2ui_get_file_input_files", "files"),
        ("array_get", "array_in"),
    );
    connect(
        &mut board,
        ("array_get", "element"),
        ("struct_break", "struct_in"),
    );

    settle(&mut board, &logics).await;

    assert_eq!(
        board.nodes["struct_break"].error, None,
        "Break Struct must accept a typed struct array routed through Get Element"
    );
    assert_eq!(
        field_pins(&board, "struct_break"),
        vec![
            "backendUrl".to_string(),
            "flowPath".to_string(),
            "name".to_string(),
            "relativePath".to_string(),
            "size".to_string(),
            "type".to_string(),
            "url".to_string(),
        ],
    );

    let element = pin_named(&board, "array_get", "element");
    let schema = element.schema.as_deref().expect("element keeps a schema");
    assert!(
        !is_open_object_schema(schema),
        "the consumer's open marker must not displace the producer's schema: {schema}"
    );
    assert!(
        schema.contains("A2UIFileInputFile"),
        "element must carry the producer's schema: {schema}"
    );
}

/// The same chain wired straight through, without a passthrough node in between.
#[flow_like_types::tokio::test]
async fn a_typed_array_keeps_its_schema_wired_directly() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "a2ui_get_file_input_files");
    place(&mut board, &logics, "struct_break");

    connect(
        &mut board,
        ("a2ui_get_file_input_files", "files"),
        ("struct_break", "struct_in"),
    );

    settle(&mut board, &logics).await;

    assert_eq!(board.nodes["struct_break"].error, None);
    assert!(field_pins(&board, "struct_break").contains(&"flowPath".to_string()));
}

/// A producer whose pin declares the whole array (`{"type":"array","items":{…}}`) describes the
/// element one level down — Break Struct always works on a single item.
#[flow_like_types::tokio::test]
async fn an_array_schema_is_broken_as_its_item_type() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "a2ui_get_file_input_files");
    place(&mut board, &logics, "struct_break");

    let files = pin_id(&board, "a2ui_get_file_input_files", "files");
    let item_schema = board.nodes["a2ui_get_file_input_files"].pins[&files]
        .schema
        .clone()
        .expect("files declares a schema");
    board
        .nodes
        .get_mut("a2ui_get_file_input_files")
        .unwrap()
        .pins
        .get_mut(&files)
        .unwrap()
        .schema = Some(format!(r#"{{"type":"array","items":{item_schema}}}"#));

    connect(
        &mut board,
        ("a2ui_get_file_input_files", "files"),
        ("struct_break", "struct_in"),
    );

    settle(&mut board, &logics).await;

    assert_eq!(board.nodes["struct_break"].error, None);
    assert!(field_pins(&board, "struct_break").contains(&"name".to_string()));
}

/// A schema change underneath a wired field pin must not cut the wire.
///
/// Field pins are minted from the producer's schema, so any edit upstream retires the ones the new
/// schema no longer declares. Removing a wired one takes its half of the edge with it and
/// `fix_pin_connections` prunes the other half, so the connection disappears from both ends with no
/// error anywhere — the user loses work to a transient mismatch. The pin has to survive and the
/// node has to say so.
#[flow_like_types::tokio::test]
async fn a_schema_change_never_cuts_a_wired_field_pin() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "a2ui_get_file_input_files");
    place(&mut board, &logics, "struct_break");
    place(&mut board, &logics, "string_length");

    connect(
        &mut board,
        ("a2ui_get_file_input_files", "files"),
        ("struct_break", "struct_in"),
    );
    settle(&mut board, &logics).await;

    let field = format!("{BREAK_STRUCT_PIN_PREFIX}name");
    connect(
        &mut board,
        ("struct_break", &field),
        ("string_length", "string"),
    );
    let wired_pin = pin_id(&board, "struct_break", &field);

    // The producer's shape changes underneath: `name` is gone from the new schema.
    let files = pin_id(&board, "a2ui_get_file_input_files", "files");
    board
        .nodes
        .get_mut("a2ui_get_file_input_files")
        .unwrap()
        .pins
        .get_mut(&files)
        .unwrap()
        .schema = Some(
        r#"{"title":"Renamed","type":"object","properties":{"label":{"type":"string"}}}"#
            .to_string(),
    );

    settle(&mut board, &logics).await;

    let fields = field_pins(&board, "struct_break");
    assert!(
        fields.contains(&"label".to_string()),
        "the new field must be minted, got {fields:?}"
    );
    assert!(
        fields.contains(&"name".to_string()),
        "the wired field pin must survive the schema change, got {fields:?}"
    );
    assert_eq!(
        pin_named(&board, "struct_break", &field).id,
        wired_pin,
        "the surviving pin must keep its id, or the edge points at nothing"
    );
    assert!(
        board.nodes["struct_break"]
            .pins
            .get(&wired_pin)
            .is_some_and(|pin| !pin.connected_to.is_empty()),
        "the wire itself must survive"
    );

    let error = board.nodes["struct_break"]
        .error
        .as_deref()
        .expect("a kept-but-undeclared pin must be reported");
    assert!(
        error.contains("name"),
        "the error must name the mismatched pin, got {error:?}"
    );
}

/// Unplugging a producer leaves the derived pins behind only while they are wired.
#[flow_like_types::tokio::test]
async fn unplugging_a_producer_keeps_only_the_wired_field_pins() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "a2ui_get_file_input_files");
    place(&mut board, &logics, "struct_break");
    place(&mut board, &logics, "string_length");

    connect(
        &mut board,
        ("a2ui_get_file_input_files", "files"),
        ("struct_break", "struct_in"),
    );
    settle(&mut board, &logics).await;

    let field = format!("{BREAK_STRUCT_PIN_PREFIX}name");
    connect(
        &mut board,
        ("struct_break", &field),
        ("string_length", "string"),
    );

    disconnect(
        &mut board,
        ("a2ui_get_file_input_files", "files"),
        ("struct_break", "struct_in"),
    );
    settle(&mut board, &logics).await;

    assert_eq!(
        field_pins(&board, "struct_break"),
        vec!["name".to_string()],
        "only the wired field pin survives an unplugged producer"
    );
}

/// Unplugging a producer must hand `struct_in` its open marker back. A leftover concrete schema is
/// a contract, and `schemas_are_compatible` rejects two differing ones — the user would never be
/// able to plug a different struct in again.
#[flow_like_types::tokio::test]
async fn unplugging_a_producer_restores_the_open_marker() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "a2ui_get_file_input_files");
    place(&mut board, &logics, "struct_break");

    connect(
        &mut board,
        ("a2ui_get_file_input_files", "files"),
        ("struct_break", "struct_in"),
    );
    settle(&mut board, &logics).await;
    assert!(!field_pins(&board, "struct_break").is_empty());

    disconnect(
        &mut board,
        ("a2ui_get_file_input_files", "files"),
        ("struct_break", "struct_in"),
    );
    settle(&mut board, &logics).await;

    assert!(field_pins(&board, "struct_break").is_empty());
    let struct_in = pin_named(&board, "struct_break", "struct_in");
    assert!(
        struct_in
            .schema
            .as_deref()
            .is_some_and(is_open_object_schema),
        "struct_in must accept any struct again, got {:?}",
        struct_in.schema
    );
}
