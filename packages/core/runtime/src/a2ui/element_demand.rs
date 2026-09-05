use crate::flow::{
    board::Board,
    node::Node,
    pin::{Pin, PinType},
};
use flow_like_types::{Value, json::from_slice};
use std::collections::HashSet;

/// Which page elements a board reads, derived statically from literal element refs on the read
/// pins of the catalog's UI reader nodes.
///
/// `selectors` use the frontend materializer grammar (`pageId/elementId`, `type:X`,
/// `glob:PATTERN`, `children:KEY`, `parent:KEY`, `values:instanceId`, ...). `dynamic` is set when
/// at least one read pin is wired instead of literal, so the set of elements the run needs cannot
/// be known before it executes and the client must keep on-demand reads available.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ElementDemand {
    pub selectors: Vec<String>,
    pub dynamic: bool,
}

enum ReadKind {
    Key(&'static str),
    Children(&'static str),
    Parent(&'static str),
    ByType,
    ByIdPattern,
    WidgetValues,
    Selectors,
}

const ELEMENT_REF_READERS: &[&str] = &[
    "a2ui_get_element",
    "a2ui_get_element_value",
    "a2ui_get_element_text",
    "a2ui_get_button_label",
    "a2ui_get_button_disabled",
    "a2ui_get_button_loading",
    "a2ui_get_input_placeholder",
    "a2ui_get_select_value",
    "a2ui_get_tooltip_content",
    "a2ui_get_iframe_src",
    "a2ui_get_file_input_files",
    "a2ui_update_toggle",
    "a2ui_update_table",
    "a2ui_update_hotspot",
    "a2ui_update_labeler",
    "a2ui_update_gantt",
    "a2ui_update_calendar",
];

fn read_kind(node_name: &str) -> Option<ReadKind> {
    if ELEMENT_REF_READERS.contains(&node_name) {
        return Some(ReadKind::Key("element_ref"));
    }
    match node_name {
        "a2ui_clone_element" => Some(ReadKind::Key("source_element")),
        "a2ui_query_children" => Some(ReadKind::Children("element_ref")),
        "a2ui_get_child_at_index" => Some(ReadKind::Children("container_ref")),
        "a2ui_query_parent" => Some(ReadKind::Parent("element_ref")),
        "a2ui_query_elements_by_type" => Some(ReadKind::ByType),
        "a2ui_query_elements_by_id" => Some(ReadKind::ByIdPattern),
        "a2ui_widget_query" => Some(ReadKind::WidgetValues),
        "a2ui_request_elements" => Some(ReadKind::Selectors),
        _ => None,
    }
}

enum PinState {
    Connected,
    Literal(Value),
    Empty,
}

fn pin_state(pin: &Pin) -> PinState {
    if !pin.depends_on.is_empty() {
        return PinState::Connected;
    }
    match pin
        .default_value
        .as_deref()
        .and_then(|bytes| from_slice::<Value>(bytes).ok())
    {
        Some(Value::Null) | None => PinState::Empty,
        Some(value) => PinState::Literal(value),
    }
}

fn non_empty_str(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.is_empty())
}

/// Mirrors the catalog's `extract_element_id`: a bare key, or the Get Element output shape.
fn element_key(value: &Value) -> Option<String> {
    match value {
        Value::String(_) => non_empty_str(value).map(str::to_string),
        Value::Object(obj) => obj
            .get("__element_id")
            .and_then(non_empty_str)
            .or_else(|| obj.get("id").and_then(non_empty_str))
            .map(str::to_string),
        _ => None,
    }
}

/// Mirrors the catalog's `extract_widget_instance_id`: Instantiate Widget refs carry `instanceId`
/// at the top level, Get Element nests it under `component`.
fn widget_instance_id(value: &Value) -> Option<String> {
    match value {
        Value::String(_) => non_empty_str(value).map(str::to_string),
        Value::Object(obj) => obj
            .get("instanceId")
            .and_then(non_empty_str)
            .or_else(|| {
                obj.get("component")
                    .and_then(|component| component.get("instanceId"))
                    .and_then(non_empty_str)
            })
            .map(str::to_string),
        _ => None,
    }
}

/// The selector `a2ui_query_elements_by_id` reads for a pattern + match type; shared with the
/// node so the prefetch and the static scan never disagree.
pub fn id_pattern_selector(pattern: &str, match_type: &str) -> String {
    match match_type.to_lowercase().as_str() {
        "starts_with" | "startswith" => format!("glob:{pattern}*"),
        "ends_with" | "endswith" => format!("glob:*{pattern}"),
        "exact" => pattern.to_string(),
        _ => format!("glob:*{pattern}*"),
    }
}

#[derive(Default)]
struct Collector {
    selectors: Vec<String>,
    seen: HashSet<String>,
    dynamic: bool,
}

impl Collector {
    fn push(&mut self, selector: String) {
        if self.seen.insert(selector.clone()) {
            self.selectors.push(selector);
        }
    }

    fn finish(self) -> ElementDemand {
        ElementDemand {
            selectors: self.selectors,
            dynamic: self.dynamic,
        }
    }
}

fn input_pins<'a>(node: &'a Node, name: &'a str) -> impl Iterator<Item = &'a Pin> + 'a {
    let mut pins: Vec<&Pin> = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input && pin.name == name)
        .collect();
    pins.sort_by(|a, b| (a.index, &a.id).cmp(&(b.index, &b.id)));
    pins.into_iter()
}

fn scan_keyed(node: &Node, pin_name: &str, wrap: impl Fn(&str) -> String, out: &mut Collector) {
    for pin in input_pins(node, pin_name) {
        match pin_state(pin) {
            PinState::Connected => out.dynamic = true,
            PinState::Literal(value) => {
                if let Some(key) = element_key(&value) {
                    out.push(wrap(&key));
                }
            }
            PinState::Empty => {}
        }
    }
}

fn scan_by_type(node: &Node, out: &mut Collector) {
    for pin in input_pins(node, "component_type") {
        match pin_state(pin) {
            PinState::Connected => out.dynamic = true,
            PinState::Literal(value) => {
                if let Some(component_type) = non_empty_str(&value) {
                    out.push(format!("type:{component_type}"));
                }
            }
            PinState::Empty => {}
        }
    }
}

fn scan_by_id_pattern(node: &Node, out: &mut Collector) {
    let match_type = match input_pins(node, "match_type").next().map(pin_state) {
        Some(PinState::Connected) => {
            out.dynamic = true;
            return;
        }
        Some(PinState::Literal(value)) => value.as_str().unwrap_or("contains").to_string(),
        Some(PinState::Empty) | None => "contains".to_string(),
    };
    for pin in input_pins(node, "pattern") {
        match pin_state(pin) {
            PinState::Connected => out.dynamic = true,
            PinState::Literal(value) => {
                if let Some(pattern) = non_empty_str(&value) {
                    out.push(id_pattern_selector(pattern, &match_type));
                }
            }
            PinState::Empty => {}
        }
    }
}

fn scan_widget_values(node: &Node, out: &mut Collector) {
    for pin in input_pins(node, "element_ref") {
        match pin_state(pin) {
            PinState::Connected => out.dynamic = true,
            PinState::Literal(value) => match widget_instance_id(&value) {
                Some(instance_id) => out.push(format!("values:{instance_id}")),
                None => out.dynamic = true,
            },
            PinState::Empty => {}
        }
    }
}

fn scan_selectors(node: &Node, out: &mut Collector) {
    for pin in input_pins(node, "element_ids") {
        match pin_state(pin) {
            PinState::Connected => out.dynamic = true,
            PinState::Literal(Value::Array(items)) => {
                for selector in items.iter().filter_map(non_empty_str) {
                    out.push(selector.to_string());
                }
            }
            PinState::Literal(value) => {
                if let Some(selector) = non_empty_str(&value) {
                    out.push(selector.to_string());
                }
            }
            PinState::Empty => {}
        }
    }
}

fn scan_node(node: &Node, out: &mut Collector) {
    let Some(kind) = read_kind(&node.name) else {
        return;
    };
    match kind {
        ReadKind::Key(pin) => scan_keyed(node, pin, str::to_string, out),
        ReadKind::Children(pin) => scan_keyed(node, pin, |key| format!("children:{key}"), out),
        ReadKind::Parent(pin) => scan_keyed(node, pin, |key| format!("parent:{key}"), out),
        ReadKind::ByType => scan_by_type(node, out),
        ReadKind::ByIdPattern => scan_by_id_pattern(node, out),
        ReadKind::WidgetValues => scan_widget_values(node, out),
        ReadKind::Selectors => scan_selectors(node, out),
    }
}

fn sorted_nodes<'a>(nodes: impl Iterator<Item = &'a Node>) -> Vec<&'a Node> {
    let mut nodes: Vec<&Node> = nodes.collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    nodes
}

/// Static element demand of a board: every literal element ref on a reader node's read pin, over
/// the top-level nodes and every layer's nodes.
///
/// The walk is ordered by node id (layers by layer id) so the result is stable across runs and
/// machines; the manifest signature depends on it.
pub fn element_demand(board: &Board) -> ElementDemand {
    let mut out = Collector::default();
    for node in sorted_nodes(board.nodes.values()) {
        scan_node(node, &mut out);
    }
    let mut layers: Vec<_> = board.layers.values().collect();
    layers.sort_by(|a, b| a.id.cmp(&b.id));
    for layer in layers {
        for node in sorted_nodes(layer.nodes.values()) {
            scan_node(node, &mut out);
        }
    }
    out.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::{Layer, LayerType};
    use crate::flow::variable::VariableType;
    use flow_like_storage::Path;
    use flow_like_types::json::json;

    fn board() -> Board {
        Board::new_detached(Some("b".into()), Path::default())
    }

    fn node_with_literal(name: &str, pin: &str, value: Value) -> Node {
        let mut node = Node::new(name, name, "", "UI");
        node.add_input_pin(pin, pin, "", VariableType::String)
            .set_default_value(Some(value));
        node
    }

    fn node_with_connected(name: &str, pin: &str) -> Node {
        let mut node = Node::new(name, name, "", "UI");
        node.add_input_pin(pin, pin, "", VariableType::String)
            .depends_on
            .insert("upstream-pin".into());
        node
    }

    fn insert(board: &mut Board, node: Node) {
        board.nodes.insert(node.id.clone(), node);
    }

    #[test]
    fn literal_element_ref_yields_its_key() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal("a2ui_get_element_text", "element_ref", json!("main/title")),
        );

        let demand = element_demand(&board);
        assert_eq!(demand.selectors, vec!["main/title"]);
        assert!(!demand.dynamic);
    }

    #[test]
    fn get_element_output_shape_is_a_literal_key() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal(
                "a2ui_get_element_value",
                "element_ref",
                json!({ "__element_id": "main/input", "component": { "type": "textField" } }),
            ),
        );
        insert(
            &mut board,
            node_with_literal(
                "a2ui_update_toggle",
                "element_ref",
                json!({ "id": "main/switch" }),
            ),
        );

        let demand = element_demand(&board);
        assert!(demand.selectors.contains(&"main/input".to_string()));
        assert!(demand.selectors.contains(&"main/switch".to_string()));
        assert_eq!(demand.selectors.len(), 2);
        assert!(!demand.dynamic);
    }

    #[test]
    fn connected_element_ref_is_dynamic_and_contributes_nothing() {
        let mut board = board();
        insert(
            &mut board,
            node_with_connected("a2ui_get_element", "element_ref"),
        );

        let demand = element_demand(&board);
        assert!(demand.selectors.is_empty());
        assert!(demand.dynamic);
    }

    #[test]
    fn empty_unconnected_ref_is_neither_literal_nor_dynamic() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal("a2ui_get_element", "element_ref", json!("")),
        );
        let mut bare = Node::new("a2ui_get_button_label", "", "", "UI");
        bare.add_input_pin("element_ref", "", "", VariableType::String);
        insert(&mut board, bare);

        let demand = element_demand(&board);
        assert!(demand.selectors.is_empty());
        assert!(!demand.dynamic);
    }

    #[test]
    fn query_by_type_literal_becomes_type_selector() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal(
                "a2ui_query_elements_by_type",
                "component_type",
                json!("button"),
            ),
        );

        let demand = element_demand(&board);
        assert_eq!(demand.selectors, vec!["type:button"]);
        assert!(!demand.dynamic);
    }

    #[test]
    fn query_by_id_maps_match_type_to_glob() {
        let cases = [
            (Some("starts_with"), "glob:row-*"),
            (Some("StartsWith"), "glob:row-*"),
            (Some("ends_with"), "glob:*row-"),
            (Some("exact"), "row-"),
            (Some("contains"), "glob:*row-*"),
            (None, "glob:*row-*"),
        ];
        for (match_type, expected) in cases {
            let mut board = board();
            let mut node = node_with_literal("a2ui_query_elements_by_id", "pattern", json!("row-"));
            let pin = node.add_input_pin("match_type", "", "", VariableType::String);
            if let Some(match_type) = match_type {
                pin.set_default_value(Some(json!(match_type)));
            }
            insert(&mut board, node);

            let demand = element_demand(&board);
            assert_eq!(
                demand.selectors,
                vec![expected],
                "match_type {match_type:?}"
            );
            assert!(!demand.dynamic);
        }
    }

    #[test]
    fn query_by_id_with_connected_match_type_is_dynamic() {
        let mut board = board();
        let mut node = node_with_literal("a2ui_query_elements_by_id", "pattern", json!("row-"));
        node.add_input_pin("match_type", "", "", VariableType::String)
            .depends_on
            .insert("upstream-pin".into());
        insert(&mut board, node);

        let demand = element_demand(&board);
        assert!(demand.selectors.is_empty());
        assert!(demand.dynamic);
    }

    #[test]
    fn children_and_parent_readers_wrap_their_key() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal("a2ui_query_children", "element_ref", json!("main/list")),
        );
        insert(
            &mut board,
            node_with_literal(
                "a2ui_get_child_at_index",
                "container_ref",
                json!("main/grid"),
            ),
        );
        insert(
            &mut board,
            node_with_literal("a2ui_query_parent", "element_ref", json!("main/item")),
        );

        let demand = element_demand(&board);
        let mut selectors = demand.selectors.clone();
        selectors.sort();
        assert_eq!(
            selectors,
            vec![
                "children:main/grid",
                "children:main/list",
                "parent:main/item"
            ]
        );
        assert!(!demand.dynamic);
    }

    #[test]
    fn clone_element_reads_its_source() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal("a2ui_clone_element", "source_element", json!("main/card")),
        );

        let demand = element_demand(&board);
        assert_eq!(demand.selectors, vec!["main/card"]);
    }

    #[test]
    fn request_elements_array_passes_selectors_through() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal(
                "a2ui_request_elements",
                "element_ids",
                json!([
                    "main/input",
                    "type:switch",
                    "",
                    42,
                    "glob:feed-row-*/subscribed"
                ]),
            ),
        );

        let demand = element_demand(&board);
        assert_eq!(
            demand.selectors,
            vec!["main/input", "type:switch", "glob:feed-row-*/subscribed"]
        );
        assert!(!demand.dynamic);
    }

    #[test]
    fn widget_query_literal_yields_values_selector() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal(
                "a2ui_widget_query",
                "element_ref",
                json!({ "instanceId": "instantiated-1" }),
            ),
        );
        insert(
            &mut board,
            node_with_literal(
                "a2ui_widget_query",
                "element_ref",
                json!({ "component": { "instanceId": "micro-sales-chart-1" } }),
            ),
        );

        let demand = element_demand(&board);
        let mut selectors = demand.selectors.clone();
        selectors.sort();
        assert_eq!(
            selectors,
            vec!["values:instantiated-1", "values:micro-sales-chart-1"]
        );
        assert!(!demand.dynamic);
    }

    #[test]
    fn widget_query_without_instance_id_is_dynamic() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal(
                "a2ui_widget_query",
                "element_ref",
                json!({ "id": "main/host" }),
            ),
        );

        let demand = element_demand(&board);
        assert!(demand.selectors.is_empty());
        assert!(demand.dynamic);
    }

    #[test]
    fn write_nodes_contribute_nothing() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal("a2ui_set_element_text", "element_ref", json!("main/title")),
        );
        insert(
            &mut board,
            node_with_connected("a2ui_set_element_value", "element_ref"),
        );

        let demand = element_demand(&board);
        assert!(demand.selectors.is_empty());
        assert!(!demand.dynamic);
    }

    #[test]
    fn layer_nodes_are_scanned() {
        let mut board = board();
        let mut layer = Layer::new("f".into(), "Fn".into(), LayerType::Function);
        let node = node_with_literal("a2ui_get_select_value", "element_ref", json!("main/select"));
        layer.nodes.insert(node.id.clone(), node);
        let connected = node_with_connected("a2ui_get_iframe_src", "element_ref");
        layer.nodes.insert(connected.id.clone(), connected);
        board.layers.insert(layer.id.clone(), layer);

        let demand = element_demand(&board);
        assert_eq!(demand.selectors, vec!["main/select"]);
        assert!(demand.dynamic);
    }

    #[test]
    fn selectors_are_deduplicated() {
        let mut board = board();
        insert(
            &mut board,
            node_with_literal("a2ui_get_element_text", "element_ref", json!("main/title")),
        );
        insert(
            &mut board,
            node_with_literal("a2ui_get_element_value", "element_ref", json!("main/title")),
        );
        insert(
            &mut board,
            node_with_literal(
                "a2ui_request_elements",
                "element_ids",
                json!(["main/title", "main/body"]),
            ),
        );

        let demand = element_demand(&board);
        assert_eq!(demand.selectors.len(), 2);
        assert!(demand.selectors.contains(&"main/title".to_string()));
        assert!(demand.selectors.contains(&"main/body".to_string()));
    }

    #[test]
    fn walk_order_is_stable_across_hashmap_layouts() {
        let mut board = board();
        let mut a = node_with_literal("a2ui_get_element_text", "element_ref", json!("main/second"));
        a.id = "node-b".into();
        let mut b = node_with_literal("a2ui_get_element_text", "element_ref", json!("main/first"));
        b.id = "node-a".into();
        insert(&mut board, a);
        insert(&mut board, b);

        let demand = element_demand(&board);
        assert_eq!(demand.selectors, vec!["main/first", "main/second"]);
    }
}
