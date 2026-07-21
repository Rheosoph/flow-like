//! `string_format` mints one input pin per `{placeholder}` in its `on_update`, which the board
//! re-runs on every parse event. These tests pin down that the pin set is a *function of the
//! format string* — stable no matter how many times `on_update` runs, and independent of how
//! often a placeholder is repeated.
//!
//! Regression: `captures_iter` yields every occurrence, so a repeated `{page}` used to re-add a
//! pin whose name had already been claimed. `add_input_pin` mints a fresh id rather than deduping
//! by name, so each parse leaked another `page` pin and the board grew without bound.

use std::collections::HashMap;
use std::time::SystemTime;

use flow_like::flow::board::{Board, ExecutionMode, ExecutionStage};
use flow_like::flow::execution::LogLevel;
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::flow::pin::PinType;
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;
use flow_like_types::json::json;

fn empty_board() -> Board {
    Board {
        id: "string-format-test".to_string(),
        name: "String Format Test".to_string(),
        description: String::new(),
        nodes: HashMap::new(),
        variables: HashMap::new(),
        comments: HashMap::new(),
        viewport: (0.0, 0.0, 1.0),
        version: (0, 0, 1),
        stage: ExecutionStage::Dev,
        log_level: LogLevel::Info,
        execution_mode: ExecutionMode::Hybrid,
        refs: HashMap::new(),
        layers: HashMap::new(),
        page_ids: Vec::new(),
        hash: None,
        created_at: SystemTime::UNIX_EPOCH,
        updated_at: SystemTime::UNIX_EPOCH,
        parent: None,
        board_dir: Path::default(),
        logic_nodes: HashMap::new(),
        app_state: None,
    }
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
