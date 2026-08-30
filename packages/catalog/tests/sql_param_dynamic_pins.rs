//! The parameterized nodes mint one input pin per `$placeholder` in their SQL literal, in an
//! `on_update` the board re-runs on every parse event. These tests pin down that the pin set
//! is a *function of the text* — stable across repeated passes, so existing wires survive —
//! and that the tokenizer used to find placeholders agrees with the one the engine parses
//! with.
//!
//! Two families share the machinery and disagree on the dialect. A DataFusion node
//! parameterizes its `query` and its values are bound by the planner; a LanceDB node
//! parameterizes the `filter` it hands to `only_if`, where `"col"` is a string literal rather
//! than a quoted identifier and backticks are the only way to delimit a name. Every dialect
//! assertion is therefore made per family, against the module that node actually calls.
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

/// Every node that derives parameter pins from a literal, with the pin that carries it.
const PARAM_NODES: [(&str, &str); 11] = [
    ("df_sql_query", "query"),
    ("df_sql_query_cached", "query"),
    ("df_execute_sql", "query"),
    ("df_write_delta", "query"),
    ("graph_sql_query", "query"),
    ("filter_local_db", "filter"),
    ("count_local_db", "filter"),
    ("filter_delete_local_db", "filter"),
    ("vector_search_local_db", "filter"),
    ("fts_search_local_db", "filter"),
    ("hybrid_search_local_db", "filter"),
];

/// One node type standing in for its dialect, with the texts each behaviour needs written in
/// that dialect's syntax.
struct Family {
    node_type: &'static str,
    config_pin: &'static str,
    /// Declares `$a` then `$b`.
    two_params: &'static str,
    /// Declares `$a` alone.
    one_param: &'static str,
    /// Declares `$q` three times.
    repeated_param: &'static str,
    /// Declares `$second` before `$first`.
    reordered_params: &'static str,
    /// Declares `$org_id`, `$since`, `$limit`.
    three_params: &'static str,
    /// `$a` plus an unterminated literal, i.e. mid-edit.
    half_typed: &'static str,
    /// `$foo_bar` and `$fooBar`, which render to the same FlowScript argument.
    colliding_params: &'static str,
    /// Texts holding a `$` that is data rather than a placeholder.
    inert_dollars: &'static [&'static str],
}

const DATAFUSION: Family = Family {
    node_type: "df_sql_query",
    config_pin: "query",
    two_params: "SELECT * FROM t WHERE a = $a AND b = $b",
    one_param: "SELECT * FROM t WHERE a = $a",
    repeated_param: "SELECT * FROM t WHERE a = $q OR b = $q OR c = $q",
    reordered_params: "SELECT * FROM t WHERE b = $second AND a = $first",
    three_params: "SELECT * FROM users WHERE org = $org_id AND created > $since LIMIT $limit",
    half_typed: "SELECT * FROM t WHERE a = $a AND b = 'unterminated",
    colliding_params: "SELECT * FROM t WHERE a = $foo_bar AND b = $fooBar",
    inert_dollars: &[
        "SELECT '$5.00' AS price FROM t",
        "SELECT \"$col\" FROM t",
        "SELECT 1 -- $nope\n FROM t",
        "SELECT 1 /* $nope */ FROM t",
        "SELECT $tag$body$tag$ FROM t",
    ],
};

const LANCE: Family = Family {
    node_type: "filter_local_db",
    config_pin: "filter",
    two_params: "a = $a AND b = $b",
    one_param: "a = $a",
    repeated_param: "a = $q OR b = $q OR c = $q",
    reordered_params: "b = $second AND a = $first",
    three_params: "org = $org_id AND created > $since AND rank < $limit",
    half_typed: "a = $a AND b = 'unterminated",
    colliding_params: "a = $foo_bar AND b = $fooBar",
    inert_dollars: &[
        "price = '$5.00'",
        // A `$` is an identifier part, so this is one column name, not a column and a
        // placeholder.
        "col$id = 1",
        "`weird $col` = 1",
        "a = 1 -- $nope\n",
        "a = 1 /* $nope */",
        "note = $tag$body$tag$",
    ],
};

const FAMILIES: [Family; 2] = [DATAFUSION, LANCE];

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

fn seeded_node(logic: &std::sync::Arc<dyn NodeLogic>, config_pin: &str, text: &str) -> Node {
    let mut node = logic.get_node();
    set_config(&mut node, config_pin, text);
    node
}

fn set_config(node: &mut Node, config_pin: &str, text: &str) {
    let pin = node
        .pins
        .values_mut()
        .find(|pin| pin.name == config_pin)
        .unwrap_or_else(|| panic!("{config_pin} pin"));
    pin.default_value = Some(json!(text).to_string().into_bytes());
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
async fn settle(node_type: &str, config_pin: &str, text: &str, passes: usize) -> Node {
    let logic = node_logic(node_type);
    let board = empty_board();
    let mut node = seeded_node(&logic, config_pin, text);
    for _ in 0..passes {
        logic.on_update(&mut node, &board).await;
    }
    node
}

#[flow_like_types::tokio::test]
async fn every_parameterized_node_derives_one_pin_per_placeholder() {
    for (node_type, config_pin) in PARAM_NODES {
        let text = if config_pin == "query" {
            "SELECT * FROM users WHERE org = $org_id AND created > $since"
        } else {
            "org = $org_id AND created > $since"
        };
        let node = settle(node_type, config_pin, text, 1).await;
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
    for family in FAMILIES {
        let logic = node_logic(family.node_type);
        let board = empty_board();
        let mut node = seeded_node(&logic, family.config_pin, family.three_params);

        logic.on_update(&mut node, &board).await;
        let established = param_pins(&node);
        assert_eq!(established.len(), 3, "{}", family.node_type);

        for pass in 2..=10 {
            logic.on_update(&mut node, &board).await;
            assert_eq!(
                param_pins(&node),
                established,
                "{} changed pin identity on pass {pass}",
                family.node_type
            );
        }
    }
}

#[flow_like_types::tokio::test]
async fn repeated_placeholder_maps_to_a_single_pin() {
    // One placeholder used three times is still one value, bound once.
    for family in FAMILIES {
        let node = settle(
            family.node_type,
            family.config_pin,
            family.repeated_param,
            10,
        )
        .await;
        assert_eq!(
            param_pin_names(&node),
            vec!["param_q".to_string()],
            "{}",
            family.node_type
        );
    }
}

#[flow_like_types::tokio::test]
async fn placeholders_are_ordered_by_first_appearance() {
    for family in FAMILIES {
        let node = settle(
            family.node_type,
            family.config_pin,
            family.reordered_params,
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
            "{} ordered pins alphabetically rather than by appearance",
            family.node_type
        );
    }
}

#[flow_like_types::tokio::test]
async fn dropping_a_placeholder_drops_its_pin() {
    for family in FAMILIES {
        let logic = node_logic(family.node_type);
        let board = empty_board();
        let mut node = seeded_node(&logic, family.config_pin, family.two_params);
        logic.on_update(&mut node, &board).await;
        assert_eq!(param_pin_names(&node).len(), 2, "{}", family.node_type);

        set_config(&mut node, family.config_pin, family.one_param);
        logic.on_update(&mut node, &board).await;

        assert_eq!(
            param_pin_names(&node),
            vec!["param_a".to_string()],
            "{}",
            family.node_type
        );
    }
}

#[flow_like_types::tokio::test]
async fn dollars_that_are_not_placeholders_mint_no_pins() {
    // Both nodes tokenize with sqlparser, so string literals, quoted identifiers, comments and
    // dollar-quoted strings are data. A regex over the text would invent a pin for each — and
    // each family is checked in its own dialect, since they do not delimit identifiers alike.
    for family in FAMILIES {
        for text in family.inert_dollars {
            let node = settle(family.node_type, family.config_pin, text, 2).await;
            assert!(
                param_pin_names(&node).is_empty(),
                "{} declared parameters for {text:?}: {:?}",
                family.node_type,
                param_pin_names(&node)
            );
        }
    }
}

#[flow_like_types::tokio::test]
async fn a_half_typed_literal_keeps_the_pins_it_already_has() {
    // While the user is mid-edit the text does not tokenize. Dropping the pins then would
    // disconnect every wire and force them to be remade once the quote is closed.
    for family in FAMILIES {
        let logic = node_logic(family.node_type);
        let board = empty_board();
        let mut node = seeded_node(&logic, family.config_pin, family.one_param);
        logic.on_update(&mut node, &board).await;
        let established = param_pins(&node);
        assert_eq!(established.len(), 1, "{}", family.node_type);

        set_config(&mut node, family.config_pin, family.half_typed);
        logic.on_update(&mut node, &board).await;

        assert_eq!(
            param_pins(&node),
            established,
            "{} dropped pins mid-edit",
            family.node_type
        );
        assert!(
            node.error.is_some(),
            "{} did not flag the unreadable text",
            family.node_type
        );
    }
}

#[flow_like_types::tokio::test]
async fn placeholders_colliding_on_one_argument_are_rejected() {
    // `$foo_bar` and `$fooBar` both render to `paramFooBar`, so FlowScript could not tell the
    // two pins apart. Naming the clash beats binding one value to both.
    for family in FAMILIES {
        let node = settle(
            family.node_type,
            family.config_pin,
            family.colliding_params,
            1,
        )
        .await;
        let error = node
            .error
            .unwrap_or_else(|| panic!("{} must report the collision", family.node_type));
        assert!(
            error.contains("paramFooBar"),
            "{}: unexpected error: {error}",
            family.node_type
        );
    }
}

#[flow_like_types::tokio::test]
async fn a_wired_literal_derives_no_pins() {
    // The stored literal is whatever was last typed; once the text arrives over a wire it no
    // longer describes what will run, so offering pins from it would be misleading. The
    // `params` object is the channel in that case.
    for family in FAMILIES {
        let logic = node_logic(family.node_type);
        let board = empty_board();
        let mut node = seeded_node(&logic, family.config_pin, family.one_param);
        logic.on_update(&mut node, &board).await;
        assert_eq!(param_pin_names(&node).len(), 1, "{}", family.node_type);

        node.pins
            .values_mut()
            .find(|pin| pin.name == family.config_pin)
            .expect("config pin")
            .depends_on
            .insert("some-upstream-pin".to_string());
        logic.on_update(&mut node, &board).await;

        assert!(
            param_pin_names(&node).is_empty(),
            "{} kept pins for a wired literal",
            family.node_type
        );
    }
}

#[flow_like_types::tokio::test]
async fn every_parameterized_node_exposes_the_params_object_pin() {
    for (node_type, _) in PARAM_NODES {
        let node = node_logic(node_type).get_node();
        assert!(
            node.pins
                .values()
                .any(|pin| pin.pin_type == PinType::Input && pin.name == "params"),
            "{node_type} is missing the params pin"
        );
    }
}

/// Retiring the derived pins of a now-wired literal must not cut wires that were already made. The
/// deleted pin takes its half of the edge, `Board::cleanup` prunes the producer's surviving half,
/// and the connection is gone from both ends with nothing reported — leaving the producer stranded.
#[flow_like_types::tokio::test]
async fn wiring_the_literal_keeps_wired_param_pins() {
    for family in FAMILIES {
        let logic = node_logic(family.node_type);
        let board = empty_board();
        let mut node = seeded_node(&logic, family.config_pin, family.two_params);
        logic.on_update(&mut node, &board).await;
        assert_eq!(param_pin_names(&node).len(), 2, "{}", family.node_type);

        node.pins
            .values_mut()
            .find(|pin| pin.name == "param_a")
            .expect("param_a pin")
            .depends_on
            .insert("source-of-a".to_string());
        node.pins
            .values_mut()
            .find(|pin| pin.name == family.config_pin)
            .expect("config pin")
            .depends_on
            .insert("some-upstream-pin".to_string());

        logic.on_update(&mut node, &board).await;

        assert_eq!(
            param_pin_names(&node),
            vec!["param_a".to_string()],
            "{}: the connected parameter pin must survive; the unconnected one is retired",
            family.node_type
        );
        assert!(
            node.error
                .as_deref()
                .is_some_and(|error| error.contains("param_a")),
            "{}: the kept pin must be reported: {:?}",
            family.node_type,
            node.error
        );
    }
}

/// A list parameter is the reason `IN (...)` support exists: without it a set filter has to be
/// pasted together from user input, which is the string building parameters replace.
#[flow_like_types::tokio::test]
async fn a_lance_filter_derives_a_pin_for_an_in_list() {
    let node = settle(
        "filter_local_db",
        "filter",
        "id IN ($ids) AND rank > $min",
        2,
    )
    .await;
    assert_eq!(
        param_pin_names(&node),
        vec!["param_ids".to_string(), "param_min".to_string()]
    );
    assert_eq!(node.error, None);
}

/// A LanceDB filter reads `"col"` as a string literal, so a `$` inside one is data — whereas the
/// same text is a quoted identifier to DataFusion. Neither declares a parameter, but they get
/// there for opposite reasons, and only the Lance-dialect tokenizer can be trusted to say so for
/// a filter.
#[flow_like_types::tokio::test]
async fn a_lance_filter_reads_double_quotes_as_data() {
    let node = settle("filter_local_db", "filter", "note = \"$id\"", 2).await;
    assert!(param_pin_names(&node).is_empty(), "{:?}", node.error);
}
