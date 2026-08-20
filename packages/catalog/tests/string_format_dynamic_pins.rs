//! `string_format` mints one input pin per `{placeholder}` in its `on_update`, which the board
//! re-runs on every parse event. These tests pin down that the pin set is a *function of the
//! format string* — stable no matter how many times `on_update` runs, and independent of how
//! often a placeholder is repeated.
//!
//! Regression: `captures_iter` yields every occurrence, so a repeated `{page}` used to re-add a
//! pin whose name had already been claimed. `add_input_pin` mints a fresh id rather than deduping
//! by name, so each parse leaked another `page` pin and the board grew without bound.

use std::time::SystemTime;

use flow_like::flow::board::Board;
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::flow::pin::PinType;
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;
use flow_like_types::json::json;

fn empty_board() -> Board {
    let mut board = Board::new_detached(Some("string-format-test".to_string()), Path::default());
    board.name = "String Format Test".to_string();
    board.description.clear();
    board.viewport = (0.0, 0.0, 1.0);
    board.hash = None;
    board.created_at = SystemTime::UNIX_EPOCH;
    board.updated_at = SystemTime::UNIX_EPOCH;
    board
}

fn string_format_logic() -> std::sync::Arc<dyn NodeLogic> {
    CatalogBuilder::new()
        .build()
        .into_iter()
        .find(|logic| logic.get_node().name == "string_format")
        .expect("string_format is in the catalog")
}

fn input_pins_named(node: &Node, name: &str) -> usize {
    node.pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input && pin.name == name)
        .count()
}

fn input_pin_identity(node: &Node, name: &str) -> (String, u16) {
    let mut pins = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input && pin.name == name);
    let pin = pins.next().expect("placeholder input pin");
    assert!(pins.next().is_none(), "placeholder `{name}` must be unique");
    (pin.id.clone(), pin.index)
}

/// Run `on_update` `passes` times, reporting the placeholder pin counts after each pass.
async fn pin_counts_per_pass(format_string: &str, placeholder: &str, passes: usize) -> Vec<usize> {
    let logic = string_format_logic();
    let board = empty_board();
    let mut node = logic.get_node();
    let format_pin = node
        .pins
        .values_mut()
        .find(|pin| pin.name == "format_string")
        .expect("format_string pin");
    format_pin.default_value = Some(json!(format_string).to_string().into_bytes());

    let mut counts = Vec::with_capacity(passes);
    for _ in 0..passes {
        logic.on_update(&mut node, &board).await;
        counts.push(input_pins_named(&node, placeholder));
    }
    counts
}

#[flow_like_types::tokio::test]
async fn repeated_placeholder_maps_to_a_single_stable_pin() {
    // Shape taken from a generated recursive-CTE query, where `{page}` appears three times.
    let counts = pin_counts_per_pass(
        "WITH RECURSIVE d(id) AS (SELECT id FROM p WHERE id = '{page}' UNION ALL \
         SELECT p.id FROM p JOIN d ON p.parent_id = d.id) SELECT '{page}', COUNT(*) FROM d \
         WHERE id <> '{page}'",
        "page",
        5,
    )
    .await;
    assert_eq!(
        counts,
        vec![1; 5],
        "a repeated placeholder must not leak a pin per on_update pass"
    );
}

#[flow_like_types::tokio::test]
async fn repeated_sql_parameter_maps_to_one_pin() {
    let query = "SELECT id, parent_id, title, path, updated_at FROM wiki_pages \
        WHERE lower(title) LIKE lower('%{query}%') \
        OR lower(path) LIKE lower('%{query}%') \
        ORDER BY path LIMIT 50 OFFSET {offset};";
    let logic = string_format_logic();
    let board = empty_board();
    let mut node = logic.get_node();
    node.pins
        .values_mut()
        .find(|pin| pin.name == "format_string")
        .expect("format_string pin")
        .default_value = Some(json!(query).to_string().into_bytes());

    // Board stabilization may call `on_update` up to ten times. Neither parameter may grow or be
    // replaced/reordered between passes, since doing so would invalidate existing connections.
    let mut expected_identity = None;
    for _ in 0..10 {
        logic.on_update(&mut node, &board).await;
        assert_eq!(input_pins_named(&node, "query"), 1);
        assert_eq!(input_pins_named(&node, "offset"), 1);

        let query_identity = input_pin_identity(&node, "query");
        let offset_identity = input_pin_identity(&node, "offset");
        assert!(
            query_identity.1 < offset_identity.1,
            "pin order must follow first placeholder appearance"
        );
        let identity = vec![query_identity, offset_identity];
        if let Some(expected) = &expected_identity {
            assert_eq!(&identity, expected, "placeholder pins must remain stable");
        } else {
            expected_identity = Some(identity);
        }
    }
}

#[flow_like_types::tokio::test]
async fn distinct_placeholders_are_stable_across_updates() {
    assert_eq!(
        pin_counts_per_pass("/{parent}/{page}", "page", 5).await,
        vec![1; 5]
    );
    assert_eq!(
        pin_counts_per_pass("/{parent}/{page}", "parent", 5).await,
        vec![1; 5]
    );
}

#[flow_like_types::tokio::test]
async fn already_leaked_duplicate_pins_are_healed() {
    let logic = string_format_logic();
    let board = empty_board();
    let mut node = logic.get_node();
    let format_pin = node
        .pins
        .values_mut()
        .find(|pin| pin.name == "format_string")
        .expect("format_string pin");
    format_pin.default_value = Some(json!("{page}").to_string().into_bytes());

    // Simulate a board persisted by the buggy version: several pins sharing one placeholder name.
    for _ in 0..4 {
        node.add_input_pin(
            "page",
            "page",
            "",
            flow_like::flow::variable::VariableType::Generic,
        );
    }
    assert_eq!(
        input_pins_named(&node, "page"),
        4,
        "precondition: leaked pins present"
    );

    logic.on_update(&mut node, &board).await;
    assert_eq!(
        input_pins_named(&node, "page"),
        1,
        "on_update must collapse duplicate placeholder pins, not preserve them"
    );
}

#[flow_like_types::tokio::test]
async fn placeholders_removed_from_the_format_string_drop_their_pins() {
    let logic = string_format_logic();
    let board = empty_board();
    let mut node = logic.get_node();

    let set_format = |node: &mut Node, value: &str| {
        let pin = node
            .pins
            .values_mut()
            .find(|pin| pin.name == "format_string")
            .expect("format_string pin");
        pin.default_value = Some(json!(value).to_string().into_bytes());
    };

    set_format(&mut node, "{parent}/{page}");
    logic.on_update(&mut node, &board).await;
    assert_eq!(input_pins_named(&node, "parent"), 1);
    assert_eq!(input_pins_named(&node, "page"), 1);

    set_format(&mut node, "{page}");
    logic.on_update(&mut node, &board).await;
    assert_eq!(
        input_pins_named(&node, "parent"),
        0,
        "stale placeholder pin must be removed"
    );
    assert_eq!(input_pins_named(&node, "page"), 1);
}

/// Removing a placeholder must not silently cut a wire. `on_update` deleting a connected pin takes
/// its half of the edge with it, and `Board::cleanup`'s `fix_pin_connections` then prunes the
/// producer's surviving half — the connection vanishes from both ends with no error, leaving the
/// producer dead on the canvas next to an empty input.
#[flow_like_types::tokio::test]
async fn a_wired_placeholder_pin_survives_its_placeholder_being_removed() {
    let logic = string_format_logic();
    let board = empty_board();
    let mut node = logic.get_node();

    set_format_string(&mut node, "{parent}/{page}");
    logic.on_update(&mut node, &board).await;
    wire_input_pin(&mut node, "parent");

    set_format_string(&mut node, "{page}");
    logic.on_update(&mut node, &board).await;

    assert_eq!(
        input_pins_named(&node, "parent"),
        1,
        "a connected placeholder pin must be kept, not deleted with its wire"
    );
    assert!(
        node.error
            .as_deref()
            .is_some_and(|error| error.contains("parent")),
        "the node must report the stale-but-connected pin: {:?}",
        node.error
    );
    assert_eq!(input_pins_named(&node, "page"), 1);
}

/// A wired `format_string` says nothing about which placeholders the node will see at runtime, so
/// the stale literal must not be read as "declares nothing" — that wiped every placeholder pin.
#[flow_like_types::tokio::test]
async fn a_wired_format_string_keeps_the_placeholder_pins_it_already_has() {
    let logic = string_format_logic();
    let board = empty_board();
    let mut node = logic.get_node();

    set_format_string(&mut node, "{parent}/{page}");
    logic.on_update(&mut node, &board).await;

    node.pins
        .values_mut()
        .find(|pin| pin.name == "format_string")
        .expect("format_string pin")
        .depends_on
        .insert("upstream-pin".to_string());

    logic.on_update(&mut node, &board).await;

    assert_eq!(input_pins_named(&node, "parent"), 1);
    assert_eq!(input_pins_named(&node, "page"), 1);
}

/// A `{format_string}` token would mint a second pin with the config pin's own name, and every
/// later lookup of that name would pick whichever the pin map yielded first.
#[flow_like_types::tokio::test]
async fn a_placeholder_named_like_the_config_pin_is_rejected() {
    let logic = string_format_logic();
    let board = empty_board();
    let mut node = logic.get_node();

    set_format_string(&mut node, "{format_string} and {page}");
    logic.on_update(&mut node, &board).await;

    assert_eq!(
        input_pins_named(&node, "format_string"),
        1,
        "the node's own input must stay unique"
    );
    assert_eq!(input_pins_named(&node, "page"), 1);
    assert!(
        node.error.is_some(),
        "the colliding placeholder must be reported"
    );
}

fn set_format_string(node: &mut Node, value: &str) {
    node.pins
        .values_mut()
        .find(|pin| pin.name == "format_string")
        .expect("format_string pin")
        .default_value = Some(json!(value).to_string().into_bytes());
}

fn wire_input_pin(node: &mut Node, name: &str) {
    node.pins
        .values_mut()
        .find(|pin| pin.pin_type == PinType::Input && pin.name == name)
        .unwrap_or_else(|| panic!("input pin `{name}`"))
        .depends_on
        .insert(format!("source-of-{name}"));
}
