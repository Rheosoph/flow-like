//! Cross-app id references embedded in a2ui payloads.
//!
//! Page and widget bodies are stored as opaque JSON (`Component.component_json`,
//! exposed-prop defaults, customization values). Inside that JSON live ids that
//! belong to the surrounding app: the node an `on_click` starts, the board it
//! runs on, the page a navigation targets, the widget an instance renders.
//! Every operation that moves a payload into a different id space — forking an
//! app, instantiating a board from a template — has to rewrite them or the
//! copy keeps firing the source's nodes.
//!
//! The walker recognizes references **by field name**, at any depth: pattern
//! matching individual component shapes was lossy, because ids show up in image
//! hotspots, dialogue choices, modal triggers and user-authored action contexts
//! alike. Translation is opt-in per value: the caller's closure returns `None`
//! to leave a value alone, so a field that merely shares a name with a
//! reference (a `nodeId` in unrelated game state) survives untouched unless the
//! caller recognizes the id.

use flow_like_types::Value;

/// What a recognized field refers to. `Node` covers the `events_simple` node
/// ids that page hooks and `workflow_event` actions point at — despite some of
/// those fields being spelled `eventId`, they are node ids, not event-row ids.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdRef {
    Node,
    Board,
    Page,
    Widget,
    Event,
    App,
}

/// Where in the tree the walker currently is. `workflow`/`workflowEvent`
/// objects name their node id `eventId`, which everywhere else means an
/// event-row id.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RefContext {
    General,
    Workflow,
}

/// Rewrite every id reference in a JSON tree.
///
/// `translate` is called once per recognized value with the kind of reference
/// and the current id; returning `Some(new_id)` replaces it. Values are read
/// out of either a bare string or a `BoundValue` literal
/// (`{"literalString": "..."}`), and `literalJson` payloads are re-parsed so
/// references nested inside them are covered too.
pub fn rewrite_json_ids(
    value: &mut Value,
    translate: &mut dyn FnMut(IdRef, &str) -> Option<String>,
) {
    rewrite_in(value, translate, RefContext::General);
}

/// Rewrite ids in a payload that carries no enclosing field name — an exposed
/// prop's default value, a customization value. Those decode to a bare literal
/// (`{"literalString": "<page id>"}` or just `"<page id>"`), so there is no key
/// to key off; the caller instead decides per string whether it recognizes the
/// id at all. A widget's `data_model` is deliberately *not* in this list: it is
/// app state, and a string there must not be rewritten just because it happens
/// to equal an id.
///
/// Named references nested inside such a payload are still handled by name, so
/// callers pass both closures.
pub fn rewrite_unkeyed_json_ids(
    value: &mut Value,
    translate: &mut dyn FnMut(IdRef, &str) -> Option<String>,
    translate_literal: &mut dyn FnMut(&str) -> Option<String>,
) {
    rewrite_in(value, translate, RefContext::General);
    rewrite_literals_in(value, translate_literal);
}

fn rewrite_in(
    value: &mut Value,
    translate: &mut dyn FnMut(IdRef, &str) -> Option<String>,
    context: RefContext,
) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if let Some(kind) = ref_kind(key, context) {
                    translate_bound_value(val, &mut |id| translate(kind, id));
                }
                let child_context = match key.as_str() {
                    "workflow" | "workflowEvent" | "workflow_event" => RefContext::Workflow,
                    _ => RefContext::General,
                };
                rewrite_in(val, translate, child_context);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                rewrite_in(item, translate, context);
            }
        }
        Value::String(raw) => {
            rewrite_embedded_json(raw, &mut |inner| rewrite_in(inner, translate, context))
        }
        _ => {}
    }
}

/// The maps a component carries whose keys are prop ids rather than reference
/// names. Their *values* can still be references, so they take the by-value
/// pass even when the surrounding document only takes the by-name one.
pub const PROP_VALUE_CONTAINERS: [&str; 4] = [
    "exposedPropValues",
    "exposed_prop_values",
    "customizationValues",
    "customization_values",
];

/// Apply `translate_literal` to the contents of every [`PROP_VALUE_CONTAINERS`]
/// map found in the tree, leaving the rest of the document to the by-name pass.
pub fn rewrite_prop_value_containers(
    value: &mut Value,
    translate_literal: &mut dyn FnMut(&str) -> Option<String>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if PROP_VALUE_CONTAINERS.contains(&key.as_str()) {
                    rewrite_literals_in(child, translate_literal);
                } else {
                    rewrite_prop_value_containers(child, translate_literal);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                rewrite_prop_value_containers(item, translate_literal);
            }
        }
        _ => {}
    }
}

fn rewrite_literals_in(
    value: &mut Value,
    translate_literal: &mut dyn FnMut(&str) -> Option<String>,
) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if key == "literalString"
                    && let Value::String(current) = val
                    && let Some(new_id) = translate_literal(current.as_str())
                {
                    *current = new_id;
                    continue;
                }
                rewrite_literals_in(val, translate_literal);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                rewrite_literals_in(item, translate_literal);
            }
        }
        Value::String(raw) => {
            if let Some(new_id) = translate_literal(raw.as_str()) {
                *raw = new_id;
            }
        }
        _ => {}
    }
}

fn ref_kind(key: &str, context: RefContext) -> Option<IdRef> {
    match key {
        "nodeId" | "node_id" | "flowId" | "flow_id" | "workflowId" | "workflow_id"
        | "workflowEventId" | "workflow_event_id" => Some(IdRef::Node),
        "boardId" | "board_id" => Some(IdRef::Board),
        "pageId" | "page_id" => Some(IdRef::Page),
        "widgetId" | "widget_id" => Some(IdRef::Widget),
        "eventId" | "event_id" => Some(if context == RefContext::Workflow {
            IdRef::Node
        } else {
            IdRef::Event
        }),
        "appId" | "app_id" => Some(IdRef::App),
        _ => None,
    }
}

/// A reference is stored either bare or inside a `BoundValue`. The literal
/// string is the obvious carrier; a `path` binding resolves against the data
/// model at render time and is not a reference, but the `defaultValue` it falls
/// back to stands in for the same id and has to travel with it.
fn translate_bound_value(target: &mut Value, translate: &mut dyn FnMut(&str) -> Option<String>) {
    match target {
        Value::String(current) => {
            if let Some(new_id) = translate(current.as_str()) {
                *current = new_id;
            }
        }
        Value::Object(obj) => {
            for slot in ["literalString", "defaultValue", "default_value"] {
                if let Some(Value::String(current)) = obj.get_mut(slot)
                    && let Some(new_id) = translate(current.as_str())
                {
                    *current = new_id;
                }
            }
        }
        _ => {}
    }
}

/// `literalJson` carries a JSON document as a *string*, which the renderer
/// re-parses on every read. Ids inside it are invisible to a walker that only
/// looks at object keys, so parse, rewrite, and re-serialize. A payload that is
/// not JSON, or that re-encodes to something different in shape, is left as it
/// was.
fn rewrite_embedded_json(raw: &mut String, rewrite: &mut dyn FnMut(&mut Value)) {
    let trimmed = raw.trim_start();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return;
    }
    let Ok(mut parsed) = flow_like_types::json::from_str::<Value>(raw) else {
        return;
    };
    let before = parsed.clone();
    rewrite(&mut parsed);
    if parsed == before {
        return;
    }
    if let Ok(encoded) = flow_like_types::json::to_string(&parsed) {
        *raw = encoded;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;
    use std::collections::HashMap;

    fn map_translator(nodes: HashMap<String, String>) -> impl FnMut(IdRef, &str) -> Option<String> {
        move |kind, id| match kind {
            IdRef::Node => nodes.get(id).cloned(),
            _ => None,
        }
    }

    #[test]
    fn translates_bare_and_bound_references() {
        let mut nodes = HashMap::new();
        nodes.insert("src_node".to_string(), "dst_node".to_string());
        let mut translate = map_translator(nodes);

        let mut value = json!({
            "actions": [
                { "name": "workflow_event", "context": { "nodeId": "src_node" } },
                { "name": "workflow_event", "context": { "nodeId": { "literalString": "src_node" } } },
                { "name": "workflow_event", "context": { "nodeId": "unknown_node" } }
            ]
        });
        rewrite_json_ids(&mut value, &mut translate);

        assert_eq!(value["actions"][0]["context"]["nodeId"], "dst_node");
        assert_eq!(
            value["actions"][1]["context"]["nodeId"]["literalString"],
            "dst_node"
        );
        assert_eq!(value["actions"][2]["context"]["nodeId"], "unknown_node");
    }

    #[test]
    fn a_path_bindings_fallback_travels_with_the_reference() {
        let mut nodes = HashMap::new();
        nodes.insert("src_node".to_string(), "dst_node".to_string());
        let mut translate = map_translator(nodes);

        let mut value = json!({
            "context": { "nodeId": { "path": "$.selected", "defaultValue": "src_node" } }
        });
        rewrite_json_ids(&mut value, &mut translate);

        assert_eq!(value["context"]["nodeId"]["defaultValue"], "dst_node");
        assert_eq!(value["context"]["nodeId"]["path"], "$.selected");
    }

    #[test]
    fn event_id_is_a_node_id_only_under_workflow() {
        let mut translate = |kind: IdRef, id: &str| match (kind, id) {
            (IdRef::Node, "src") => Some("dst_node".to_string()),
            (IdRef::Event, "src") => Some("dst_event".to_string()),
            _ => None,
        };

        let mut value = json!({
            "workflow": { "eventId": "src" },
            "eventId": "src"
        });
        rewrite_json_ids(&mut value, &mut translate);

        assert_eq!(value["workflow"]["eventId"], "dst_node");
        assert_eq!(value["eventId"], "dst_event");
    }

    #[test]
    fn literal_json_payloads_are_rewritten_in_place() {
        let mut translate = |kind: IdRef, id: &str| match (kind, id) {
            (IdRef::Page, "src_page") => Some("dst_page".to_string()),
            _ => None,
        };

        let mut value = json!({
            "data": { "literalJson": "{\"pageId\":\"src_page\",\"label\":\"Open\"}" }
        });
        rewrite_json_ids(&mut value, &mut translate);

        let encoded = value["data"]["literalJson"]
            .as_str()
            .expect("literalJson stays a string");
        let inner: Value = flow_like_types::json::from_str(encoded).expect("still valid json");
        assert_eq!(inner["pageId"], "dst_page");
        assert_eq!(inner["label"], "Open");
    }

    #[test]
    fn non_json_strings_are_left_alone() {
        let mut translate = |_: IdRef, _: &str| Some("rewritten".to_string());
        let mut value = json!({ "content": "not json {at all", "route": "/dashboard" });
        let before = value.clone();
        rewrite_json_ids(&mut value, &mut translate);
        assert_eq!(value, before);
    }

    #[test]
    fn unkeyed_literals_translate_only_recognized_ids() {
        let mut translate = |_: IdRef, _: &str| None;
        let mut translate_literal = |id: &str| (id == "src_page").then(|| "dst_page".to_string());

        let mut bound = json!({ "literalString": "src_page" });
        rewrite_unkeyed_json_ids(&mut bound, &mut translate, &mut translate_literal);
        assert_eq!(bound["literalString"], "dst_page");

        let mut bare = Value::String("src_page".to_string());
        rewrite_unkeyed_json_ids(&mut bare, &mut translate, &mut translate_literal);
        assert_eq!(bare, Value::String("dst_page".to_string()));

        let mut untouched = Value::String("Ingest files".to_string());
        rewrite_unkeyed_json_ids(&mut untouched, &mut translate, &mut translate_literal);
        assert_eq!(untouched, Value::String("Ingest files".to_string()));
    }
}
