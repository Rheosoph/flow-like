use flow_like::flow::node::Node;
use flow_like_types::Value;

pub fn get_pin_string_value(node: &Node, name: &str) -> String {
    node.get_pin_by_name(name)
        .and_then(|pin| pin.default_value.clone())
        .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}
