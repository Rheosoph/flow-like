//! The SQL nodes mint one input pin per `$placeholder` in their query literal, in an
//! `on_update` the board re-runs on every parse event. These tests pin down that the pin set
//! is a *function of the query text* — stable across repeated passes, so existing wires
//! survive — and that the tokenizer used to find placeholders agrees with the one DataFusion
//! plans with.
//!
//! The stability bar is 10 passes because that is `MAX_PASSES` in `Board::node_updates`: a
//! non-idempotent `on_update` never reaches the fixpoint and leaks a pin per pass.

use std::time::SystemTime;

use flow_like::flow::board::Board;
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::flow::pin::PinType;
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;
use flow_like_types::json::json;

/// Every node that derives parameter pins from a query literal.
const SQL_PARAM_NODES: [&str; 5] = [
    "df_sql_query",
    "df_sql_query_cached",
    "df_execute_sql",
    "df_write_delta",
    "graph_sql_query",
];

fn empty_board() -> Board {
    let mut board = Board::new_detached(Some("sql-param-test".to_string()), Path::default());
    board.name = "SQL Param Test".to_string();
    board.description.clear();
    board.viewport = (0.0, 0.0, 1.0);
    board.hash = None;
    board.created_at = SystemTime::UNIX_EPOCH;
    board.updated_at = SystemTime::UNIX_EPOCH;
    board
}

fn node_logic(node_type: &str) -> std::sync::Arc<dyn NodeLogic> {
    CatalogBuilder::new()
        .build()
        .into_iter()
        .find(|logic| logic.get_node().name == node_type)
        .unwrap_or_else(|| panic!("{node_type} is in the catalog"))
}

fn seeded_node(logic: &std::sync::Arc<dyn NodeLogic>, query: &str) -> Node {
    let mut node = logic.get_node();
    let query_pin = node
        .pins
        .values_mut()
        .find(|pin| pin.name == "query")
        .expect("query pin");
    query_pin.default_value = Some(json!(query).to_string().into_bytes());
    node
}

fn param_pins(node: &Node) -> Vec<(String, String, u16)> {
    let mut pins: Vec<(String, String, u16)> = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input && pin.name.starts_with("param_"))
        .map(|pin| (pin.name.clone(), pin.id.clone(), pin.index))
        .collect();
    pins.sort_by_key(|(name, _, _)| name.clone());
    pins
}

fn param_pin_names(node: &Node) -> Vec<String> {
    param_pins(node)
        .into_iter()
        .map(|(name, _, _)| name)
        .collect()
}

/// Runs `on_update` `passes` times and returns the node, so callers can assert on the final
/// pin set as well as on the fact that it stopped changing.
async fn settle(node_type: &str, query: &str, passes: usize) -> Node {
    let logic = node_logic(node_type);
    let board = empty_board();
    let mut node = seeded_node(&logic, query);
    for _ in 0..passes {
        logic.on_update(&mut node, &board).await;
    }
    node
}

#[flow_like_types::tokio::test]
async fn every_sql_node_derives_one_pin_per_placeholder() {
    for node_type in SQL_PARAM_NODES {
        let node = settle(
            node_type,
            "SELECT * FROM users WHERE org = $org_id AND created > $since",
            1,
        )
        .await;
        assert_eq!(
            param_pin_names(&node),
            vec!["param_org_id".to_string(), "param_since".to_string()],
            "{node_type} derived the wrong parameter pins"
        );
        assert_eq!(node.error, None, "{node_type} reported an error");
    }
}

#[flow_like_types::tokio::test]
async fn pin_identity_is_stable_across_repeated_passes() {
    // Identity has to be compared on ONE node across passes: ids are minted per node, so two
    // separately built nodes never match. A pass that re-mints a pin would silently detach
    // whatever the user had wired into it.
    let logic = node_logic("df_sql_query");
    let board = empty_board();
    let mut node = seeded_node(
        &logic,
        "SELECT * FROM users WHERE org = $org_id AND created > $since LIMIT $limit",
    );

    logic.on_update(&mut node, &board).await;
    let established = param_pins(&node);
    assert_eq!(established.len(), 3);

    for pass in 2..=10 {
        logic.on_update(&mut node, &board).await;
        assert_eq!(
            param_pins(&node),
            established,
            "pin identity changed on pass {pass}"
        );
    }
}

#[flow_like_types::tokio::test]
async fn repeated_placeholder_maps_to_a_single_pin() {
    // One placeholder used three times is still one value, bound once.
    let node = settle(
        "df_sql_query",
        "SELECT * FROM t WHERE a = $q OR b = $q OR c = $q",
        10,
    )
    .await;
    assert_eq!(param_pin_names(&node), vec!["param_q".to_string()]);
}

#[flow_like_types::tokio::test]
async fn placeholders_are_ordered_by_first_appearance() {
    let node = settle(
        "df_sql_query",
        "SELECT * FROM t WHERE b = $second AND a = $first",
        1,
    )
    .await;
    let pins = param_pins(&node);
    let second = pins
        .iter()
        .find(|(name, _, _)| name == "param_second")
        .expect("param_second");
    let first = pins
        .iter()
        .find(|(name, _, _)| name == "param_first")
        .expect("param_first");
    assert!(
        second.2 < first.2,
        "pin order must follow the query, not the alphabet"
    );
}

#[flow_like_types::tokio::test]
async fn dropping_a_placeholder_drops_its_pin() {
    let logic = node_logic("df_sql_query");
    let board = empty_board();
    let mut node = seeded_node(&logic, "SELECT * FROM t WHERE a = $a AND b = $b");
    logic.on_update(&mut node, &board).await;
    assert_eq!(param_pin_names(&node).len(), 2);

    let query_pin = node
        .pins
        .values_mut()
        .find(|pin| pin.name == "query")
        .expect("query pin");
    query_pin.default_value = Some(
        json!("SELECT * FROM t WHERE a = $a")
            .to_string()
            .into_bytes(),
    );
    logic.on_update(&mut node, &board).await;

    assert_eq!(param_pin_names(&node), vec!["param_a".to_string()]);
}

#[flow_like_types::tokio::test]
async fn dollars_that_are_not_placeholders_mint_no_pins() {
    // The node uses sqlparser's tokenizer, so string literals, quoted identifiers, comments
    // and dollar-quoted strings are data. A regex over the text would invent a pin for each.
    for query in [
        "SELECT '$5.00' AS price FROM t",
        "SELECT \"$col\" FROM t",
        "SELECT 1 -- $nope\n FROM t",
        "SELECT 1 /* $nope */ FROM t",
        "SELECT $tag$body$tag$ FROM t",
    ] {
        let node = settle("df_sql_query", query, 2).await;
        assert!(
            param_pin_names(&node).is_empty(),
            "{query} should declare no parameters, got {:?}",
            param_pin_names(&node)
        );
    }
}

#[flow_like_types::tokio::test]
async fn a_half_typed_query_keeps_the_pins_it_already_has() {
    // While the user is mid-edit the statement does not tokenize. Dropping the pins then
    // would disconnect every wire and force them to be remade once the quote is closed.
    let logic = node_logic("df_sql_query");
    let board = empty_board();
    let mut node = seeded_node(&logic, "SELECT * FROM t WHERE a = $a");
    logic.on_update(&mut node, &board).await;
    let established = param_pins(&node);
    assert_eq!(established.len(), 1);

    let query_pin = node
        .pins
        .values_mut()
        .find(|pin| pin.name == "query")
        .expect("query pin");
    query_pin.default_value = Some(
        json!("SELECT * FROM t WHERE a = $a AND b = 'unterminated")
            .to_string()
            .into_bytes(),
    );
    logic.on_update(&mut node, &board).await;

    assert_eq!(param_pins(&node), established, "pins must survive the edit");
    assert!(
        node.error.is_some(),
        "the unreadable query should be flagged"
    );
}

#[flow_like_types::tokio::test]
async fn placeholders_colliding_on_one_argument_are_rejected() {
    // `$foo_bar` and `$fooBar` both render to `paramFooBar`, so FlowScript could not tell the
    // two pins apart. Naming the clash beats binding one value to both.
    let node = settle(
        "df_sql_query",
        "SELECT * FROM t WHERE a = $foo_bar AND b = $fooBar",
        1,
    )
    .await;
    let error = node.error.expect("collision must be reported");
    assert!(error.contains("paramFooBar"), "unexpected error: {error}");
}

#[flow_like_types::tokio::test]
async fn a_wired_query_derives_no_pins() {
    // The stored literal is whatever was last typed; once the query arrives over a wire it no
    // longer describes what will run, so offering pins from it would be misleading. The
    // `params` object is the channel in that case.
    let logic = node_logic("df_sql_query");
    let board = empty_board();
    let mut node = seeded_node(&logic, "SELECT * FROM t WHERE a = $a");
    logic.on_update(&mut node, &board).await;
    assert_eq!(param_pin_names(&node).len(), 1);

    let query_pin = node
        .pins
        .values_mut()
        .find(|pin| pin.name == "query")
        .expect("query pin");
    query_pin.depends_on.insert("some-upstream-pin".to_string());
    logic.on_update(&mut node, &board).await;

    assert!(param_pin_names(&node).is_empty());
}

#[flow_like_types::tokio::test]
async fn every_sql_node_exposes_the_params_object_pin() {
    for node_type in SQL_PARAM_NODES {
        let node = node_logic(node_type).get_node();
        assert!(
            node.pins
                .values()
                .any(|pin| pin.pin_type == PinType::Input && pin.name == "params"),
            "{node_type} is missing the params pin"
        );
    }
}
