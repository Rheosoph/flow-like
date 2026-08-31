//! The Cast nodes exist to turn a shapeless struct into a typed one, so what they put on
//! `struct_out` is the whole point: `Break Struct` derives its field pins from it, and
//! `doPinsMatch` decides from it whether the output can be wired anywhere at all.
//!
//! Two failure modes are worth guarding. A cast that never stamps anything leaves every consumer
//! blind — the node looks wired but produces an open marker. A cast that stamps and then *keeps*
//! the stamp after its source is gone is worse: `schemas_are_compatible` rejects two differing
//! concrete schemas, so the pin becomes a contract for a shape nothing is producing and the user
//! can never re-point it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use flow_like::flow::board::Board;
use flow_like::flow::node::NodeLogic;
use flow_like::flow::pin::{PinType, is_open_object_schema};
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;
use flow_like_types::json::json;

const BREAK_STRUCT_PIN_PREFIX: &str = "__break_struct_field__";

const PERSON_SCHEMA: &str = r#"{"type":"object","properties":{"name":{"type":"string"},"age":{"type":"integer"}},"required":["name","age"]}"#;

fn catalog() -> HashMap<String, Arc<dyn NodeLogic>> {
    CatalogBuilder::new()
        .build()
        .into_iter()
        .map(|logic| (logic.get_node().name, logic))
        .collect()
}

fn empty_board() -> Board {
    let mut board = Board::new_detached(Some("struct-cast-test".to_string()), Path::default());
    board.name = "Struct Cast Test".to_string();
    board.description.clear();
    board.viewport = (0.0, 0.0, 1.0);
    board.hash = None;
    board.created_at = SystemTime::UNIX_EPOCH;
    board.updated_at = SystemTime::UNIX_EPOCH;
    board
}

fn place(board: &mut Board, logics: &HashMap<String, Arc<dyn NodeLogic>>, node_name: &str) {
    let mut node = logics
        .get(node_name)
        .unwrap_or_else(|| panic!("`{node_name}` is in the catalog"))
        .get_node();
    node.id = node_name.to_string();
    board.nodes.insert(node.id.clone(), node);
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

fn set_literal(board: &mut Board, node_id: &str, pin_name: &str, value: &str) {
    let id = pin_id(board, node_id, pin_name);
    board
        .nodes
        .get_mut(node_id)
        .unwrap()
        .pins
        .get_mut(&id)
        .unwrap()
        .set_default_value(Some(json!(value)));
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
/// `on_update`, and the sweep repeats so one node's stamp reaches its neighbours.
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

#[flow_like_types::tokio::test]
async fn a_typed_literal_becomes_the_output_shape() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "struct_cast_to_schema");
    set_literal(&mut board, "struct_cast_to_schema", "schema", PERSON_SCHEMA);

    settle(&mut board, &logics).await;

    assert_eq!(board.nodes["struct_cast_to_schema"].error, None);
    let stamped = pin_named(&board, "struct_cast_to_schema", "struct_out")
        .schema
        .clone()
        .expect("the output carries the declared shape");
    assert!(
        !is_open_object_schema(&stamped),
        "the declared shape must displace the open marker: {stamped}"
    );
    assert!(stamped.contains("\"age\""), "{stamped}");
}

/// The point of the stamp: everything downstream can now read the fields.
#[flow_like_types::tokio::test]
async fn a_cast_output_gives_break_struct_its_fields() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "struct_cast_to_schema");
    place(&mut board, &logics, "struct_break");
    set_literal(&mut board, "struct_cast_to_schema", "schema", PERSON_SCHEMA);
    connect(
        &mut board,
        ("struct_cast_to_schema", "struct_out"),
        ("struct_break", "struct_in"),
    );

    settle(&mut board, &logics).await;

    assert_eq!(board.nodes["struct_break"].error, None);
    assert_eq!(
        field_pins(&board, "struct_break"),
        vec!["age".to_string(), "name".to_string()]
    );
}

#[flow_like_types::tokio::test]
async fn an_unusable_literal_is_reported_and_leaves_the_output_open() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "struct_cast_to_schema");
    set_literal(&mut board, "struct_cast_to_schema", "schema", "{ not json");

    settle(&mut board, &logics).await;

    let error = board.nodes["struct_cast_to_schema"]
        .error
        .clone()
        .expect("a broken schema literal is named on the node");
    assert!(error.contains("not valid JSON"), "{error}");
    assert!(
        pin_named(&board, "struct_cast_to_schema", "struct_out")
            .schema
            .as_deref()
            .is_some_and(is_open_object_schema),
        "an unusable literal must leave the output open, not half-stamped"
    );
}

/// A wired Schema pin only resolves at run time, and the editor hides the stale literal behind the
/// wire — stamping it anyway would declare a shape the run will not produce.
#[flow_like_types::tokio::test]
async fn a_wired_schema_pin_leaves_the_output_open() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "struct_cast_to_schema");
    place(&mut board, &logics, "string_trim");
    set_literal(&mut board, "struct_cast_to_schema", "schema", PERSON_SCHEMA);
    connect(
        &mut board,
        ("string_trim", "trimmed_string"),
        ("struct_cast_to_schema", "schema"),
    );

    settle(&mut board, &logics).await;

    assert_eq!(board.nodes["struct_cast_to_schema"].error, None);
    assert!(
        pin_named(&board, "struct_cast_to_schema", "struct_out")
            .schema
            .as_deref()
            .is_some_and(is_open_object_schema),
        "a literal hidden behind a wire must not be stamped"
    );
}

#[flow_like_types::tokio::test]
async fn a_donor_struct_lends_its_shape_to_both_pins() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "struct_cast_to_schema");
    place(&mut board, &logics, "struct_cast_to_struct");
    set_literal(&mut board, "struct_cast_to_schema", "schema", PERSON_SCHEMA);
    connect(
        &mut board,
        ("struct_cast_to_schema", "struct_out"),
        ("struct_cast_to_struct", "struct_shape"),
    );

    settle(&mut board, &logics).await;

    assert_eq!(board.nodes["struct_cast_to_struct"].error, None);

    let donated = pin_named(&board, "struct_cast_to_schema", "struct_out")
        .schema
        .clone()
        .expect("the donor declares a shape");

    // Stamped verbatim on both: `doPinsMatch` compares schema strings, so anything but a byte copy
    // would stop the cast reaching a consumer that declares the very same type.
    assert_eq!(
        pin_named(&board, "struct_cast_to_struct", "struct_shape").schema,
        Some(donated.clone()),
        "the Shape pin must show what it is lending"
    );
    assert_eq!(
        pin_named(&board, "struct_cast_to_struct", "struct_out").schema,
        Some(donated),
    );
}

/// Keeping the last shape after the donor is gone would make the pin a contract for something
/// nothing produces, and every future donor would be refused before it could replace it.
#[flow_like_types::tokio::test]
async fn unplugging_the_donor_hands_the_open_marker_back() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "struct_cast_to_schema");
    place(&mut board, &logics, "struct_cast_to_struct");
    set_literal(&mut board, "struct_cast_to_schema", "schema", PERSON_SCHEMA);
    connect(
        &mut board,
        ("struct_cast_to_schema", "struct_out"),
        ("struct_cast_to_struct", "struct_shape"),
    );
    settle(&mut board, &logics).await;

    disconnect(
        &mut board,
        ("struct_cast_to_schema", "struct_out"),
        ("struct_cast_to_struct", "struct_shape"),
    );
    settle(&mut board, &logics).await;

    for pin_name in ["struct_shape", "struct_out"] {
        assert!(
            pin_named(&board, "struct_cast_to_struct", pin_name)
                .schema
                .as_deref()
                .is_some_and(is_open_object_schema),
            "`{pin_name}` must be open again once the donor is gone"
        );
    }
    assert_eq!(board.nodes["struct_cast_to_struct"].error, None);
}

/// A donor that declares nothing is a mistake worth naming: the cast would otherwise stamp the
/// open marker onto its output and quietly check nothing at run time.
#[flow_like_types::tokio::test]
async fn a_shapeless_donor_is_reported() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "struct_make");
    place(&mut board, &logics, "struct_cast_to_struct");
    connect(
        &mut board,
        ("struct_make", "struct"),
        ("struct_cast_to_struct", "struct_shape"),
    );

    settle(&mut board, &logics).await;

    let error = board.nodes["struct_cast_to_struct"]
        .error
        .clone()
        .expect("a donor with no shape is named on the node");
    assert!(error.contains("No target shape"), "{error}");
}

/// `struct_in` accepts anything by design; adopting a producer's schema would turn the pin into a
/// contract and stop the next producer being wired in.
#[flow_like_types::tokio::test]
async fn the_input_pin_never_adopts_a_shape() {
    let logics = catalog();
    let mut board = empty_board();

    place(&mut board, &logics, "struct_cast_to_schema");
    place(&mut board, &logics, "struct_break");
    place(&mut board, &logics, "a2ui_get_file_input_files");
    set_literal(&mut board, "struct_cast_to_schema", "schema", PERSON_SCHEMA);
    connect(
        &mut board,
        ("a2ui_get_file_input_files", "files"),
        ("struct_cast_to_schema", "struct_in"),
    );
    connect(
        &mut board,
        ("struct_cast_to_schema", "struct_out"),
        ("struct_break", "struct_in"),
    );

    settle(&mut board, &logics).await;

    assert!(
        pin_named(&board, "struct_cast_to_schema", "struct_in")
            .schema
            .as_deref()
            .is_some_and(is_open_object_schema),
        "the value being cast declares nothing about the target shape"
    );
    assert_eq!(
        field_pins(&board, "struct_break"),
        vec!["age".to_string(), "name".to_string()],
        "the cast, not the producer, decides what the fields are"
    );
}
