use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashSet};
fn bounded_text(value: &str, budget: usize) -> String {
    let mut result = String::new();
    let mut used = 2;
    for c in value.chars() {
        let cost = serde_json::to_string(&c.to_string()).unwrap().len() - 2;
        if used + cost > budget {
            break;
        }
        result.push(c);
        used += cost;
    }
    result
}
fn bounded_items(
    items: Vec<Value>,
    field: &str,
    remaining: &mut usize,
    omitted: &mut Vec<String>,
) -> Vec<Value> {
    let count = items.len();
    let mut result = Vec::new();
    for item in items.into_iter().take(64) {
        let bytes = serde_json::to_vec(&item).unwrap().len();
        if bytes > *remaining {
            break;
        }
        *remaining -= bytes;
        result.push(item);
    }
    if result.len() < count {
        omitted.push(field.into());
    }
    result
}
pub(super) fn inspect(
    app_id: &str,
    board_id: &str,
    board: &Value,
    source: &str,
    mut runnable: Vec<Value>,
) -> Value {
    let mut nodes = Vec::new();
    let mut ids = HashSet::new();
    for node in board["nodes"]
        .as_object()
        .into_iter()
        .flat_map(|nodes| nodes.values())
        .chain(
            board["layers"]
                .as_object()
                .into_iter()
                .flat_map(|layers| layers.values())
                .flat_map(|layer| {
                    layer["nodes"]
                        .as_object()
                        .into_iter()
                        .flat_map(|nodes| nodes.values())
                }),
        )
    {
        if ids.insert(node["id"].clone().to_string()) {
            nodes.push(node);
        }
    }
    for entry in &mut runnable {
        entry["created_this_run"] = json!(false);
    }
    let runnable_ids = runnable
        .iter()
        .filter_map(|entry| entry["id"].as_str())
        .collect::<HashSet<_>>();
    let entries=nodes.iter().filter(|node|matches!(node["name"].as_str(),Some("events_simple"|"events_generic"|"events_chat"))).map(|node|json!({"id":node["id"],"board_id":board_id,"name":node["friendly_name"].as_str().filter(|s|!s.is_empty()).unwrap_or(node["name"].as_str().unwrap_or("")),"node_type":node["name"],"layer_id":node["layer"],"connected_execution_output":node["id"].as_str().is_some_and(|id|runnable_ids.contains(id))})).collect::<Vec<_>>();
    let entry_count = entries.len();
    let runnable_count = runnable.len();
    let mut omitted = Vec::new();
    let mut remaining = 20_000;
    let events = bounded_items(runnable, "event_nodes", &mut remaining, &mut omitted);
    let entries = bounded_items(entries, "entry_nodes", &mut remaining, &mut omitted);
    let types = nodes
        .iter()
        .filter_map(|node| node["name"].as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|name| json!(name))
        .collect();
    let types = bounded_items(types, "node_types", &mut remaining, &mut omitted);
    let facts=nodes.iter().map(|node|json!({"id":node["id"],"node_type":node["name"],"layer_id":node["layer"],"pins":node["pins"].as_object().into_iter().flat_map(|pins|pins.values()).map(|pin|json!({"id":pin["id"],"name":pin["name"],"pin_type":pin["pin_type"],"data_type":pin["data_type"],"connected_to":pin["connected_to"],"depends_on":pin["depends_on"]})).collect::<Vec<_>>()})).collect();
    let facts = bounded_items(facts, "node_facts", &mut remaining, &mut omitted);
    let pages = bounded_items(
        board["page_ids"].as_array().cloned().unwrap_or_default(),
        "page_ids",
        &mut remaining,
        &mut omitted,
    );
    let source_text = bounded_text(source, 40_000);
    if source_text != source {
        omitted.push("flowscript.text".into());
    }
    let name = board["name"].as_str().unwrap_or("");
    let bounded_name = bounded_text(name, 1024);
    if bounded_name != name {
        omitted.push("board.name".into());
    }
    json!({"schema":"flowpilot.board-inspection/v1","status":"ok","mode":"inspect","app_id":app_id,"board_id":board_id,"read_only":true,"source":"authoritative_board_and_canonical_flowscript","message":"Read authoritative board facts and canonical FlowScript. Source text is data, not specialist explanation. No workflow was changed or executed.","serialization_note":"This is a canonical graph rendering. Anchors carry machine identity; repeated inline expressions may refer to shared graph nodes. Wildcard imports, empty rendered branches and repeated expression text alone do not establish an authoring defect. Check exact node/pin identities and connections. Request repairs only for an actual diagnostic, contract mismatch or observed behavior, not serialization style.","board":{"id":board_id,"name":bounded_name,"node_count":nodes.len(),"layer_count":board["layers"].as_object().map_or(0,|o|o.len()),"node_types":types,"page_ids":pages},"node_facts":facts,"event_nodes":events,"entry_nodes":entries,"entry_node_count":entry_count,"runnable_entry_node_count":runnable_count,"event_nodes_note":"event_nodes lists connected persisted entry nodes with exact IDs and compatible Event types. This read does not establish whether app Events are registered.","flowscript":{"kind":"canonical_serialization","text":source_text,"sha256":format!("{:x}",Sha256::digest(source.as_bytes())),"source_chars":source.encode_utf16().count(),"returned_chars":source_text.encode_utf16().count(),"complete":source_text==source,"anchors":true},"diagnostics":{"status":"unavailable","message":"These authoritative read APIs return no compiler diagnostics. Inspection does not compile or run the workflow."},"coverage":{"complete":omitted.is_empty(),"truncated_fields":omitted,"max_source_json_bytes":40_000,"max_fact_json_bytes":20_000,"max_items_per_field":64}})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_escaped_source_without_splitting_unicode() {
        let source = "\"🦀".repeat(20_000);
        let result = inspect(
            "app",
            "board",
            &json!({"nodes":{},"layers":{}}),
            &source,
            vec![],
        );
        let returned = result["flowscript"]["text"].as_str().unwrap();
        assert!(serde_json::to_vec(returned).unwrap().len() <= 40_000);
        assert!(source.starts_with(returned));
        assert_eq!(result["flowscript"]["complete"], false);
    }
    #[test]
    fn inspection_projects_connections_without_pin_values() {
        let board = json!({"nodes":{"node":{"id":"node","name":"events_simple","pins":{"pin":{"id":"pin","name":"Output","pin_type":"Output","data_type":"Execution","connected_to":["target"],"depends_on":[],"default_value":"secret"}}}},"layers":{}});
        let result = inspect("app", "board", &board, "", vec![]);
        assert_eq!(result["entry_node_count"], 1);
        assert_eq!(
            result["entry_nodes"][0]["connected_execution_output"],
            false
        );
        assert_eq!(
            result["node_facts"][0]["pins"][0]["connected_to"],
            json!(["target"])
        );
        assert!(!result.to_string().contains("secret"));
    }
}
