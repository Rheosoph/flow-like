//! A widget a run pushed to the chat, with the a2ui updates that target it, so a reader that
//! shows the widget later replays it to exactly the state the run left it in.

use std::collections::BTreeSet;

use flow_like_types_contracts::app_chat::ChatFold;
use serde_json::Value;

use crate::A2UIServerMessage as Msg;

fn inline_child_ids(component: &Value) -> Vec<String> {
    component
        .get("inlineWidgetDef")
        .and_then(|def| def.get("components"))
        .and_then(Value::as_array)
        .map(|components| {
            components
                .iter()
                .filter_map(|component| component.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The data-model paths of every `{path, defaultValue?}` binding in a component tree.
fn collect_bound_paths(value: &Value, paths: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(path) = map.get("path").and_then(Value::as_str)
                && map.keys().all(|key| key == "path" || key == "defaultValue")
            {
                paths.insert(path.to_string());
            }
            for value in map.values() {
                collect_bound_paths(value, paths);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_bound_paths(value, paths);
            }
        }
        _ => {}
    }
}

/// A data update of `updated` touches a binding on `bound` when either path is the other or
/// one of its parents.
fn paths_overlap(updated: &str, bound: &str) -> bool {
    updated == bound
        || bound
            .strip_prefix(updated)
            .is_some_and(|rest| rest.starts_with('/'))
        || updated
            .strip_prefix(bound)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The messages of `log` that target the widget instance, in order, as replayable JSON.
///
/// Relevance is a fixpoint: pushing another instance into a container inside this one makes
/// that instance, its children and its pushes part of the replay. Data-model updates match by
/// path against the widget's bindings, because a board's Data Update targets a surface id that
/// never equals the chat instance id, and are rewritten to the widget's surface id. Every
/// matching message is kept, re-registrations included, so the replay ends where the run did.
pub fn widget_updates(
    instance_id: &str,
    surface_id: &str,
    component: &Value,
    log: &[Msg],
) -> Vec<Value> {
    let mut instance_ids = BTreeSet::from([instance_id.to_string()]);
    let mut child_ids: BTreeSet<String> = inline_child_ids(component).into_iter().collect();
    let mut bound_paths = BTreeSet::new();
    collect_bound_paths(component, &mut bound_paths);

    let element_matches =
        |element_id: &str, instance_ids: &BTreeSet<String>, child_ids: &BTreeSet<String>| {
            if let Some((scope, _)) = element_id.split_once('/') {
                return instance_ids.contains(scope);
            }
            instance_ids.contains(element_id) || {
                let suffix = format!("-{element_id}");
                child_ids
                    .iter()
                    .any(|child| child == element_id || child.ends_with(&suffix))
            }
        };

    loop {
        let mut changed = false;
        for message in log {
            let Msg::UpsertElement { element_id, value } = message else {
                continue;
            };
            if !element_matches(element_id, &instance_ids, &child_ids) {
                continue;
            }
            match value.get("type").and_then(Value::as_str) {
                Some("pushChild") | Some("insertChildAt") => {
                    if let Some(child) = value.get("childId").and_then(Value::as_str)
                        && !child_ids.contains(child)
                    {
                        changed |= instance_ids.insert(child.to_string());
                    }
                }
                Some("createComponent") if instance_ids.contains(element_id) => {
                    if let Some(component) = value.get("component") {
                        for id in inline_child_ids(component) {
                            changed |= child_ids.insert(id);
                        }
                        collect_bound_paths(component, &mut bound_paths);
                    }
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }

    let mut updates = Vec::new();
    for message in log {
        let kept = match message {
            Msg::UpsertElement { element_id, .. } => {
                element_matches(element_id, &instance_ids, &child_ids).then(|| message.clone())
            }
            Msg::CreateElement {
                surface_id: target, ..
            }
            | Msg::RemoveElement {
                surface_id: target, ..
            } => (target == surface_id || instance_ids.contains(target)).then(|| message.clone()),
            Msg::DataModelUpdate {
                surface_id: target,
                path,
                contents,
            } => {
                let targeted = target == surface_id || instance_ids.contains(target);
                let bound = !targeted
                    && (path.as_deref().is_some_and(|path| {
                        bound_paths.iter().any(|bound| paths_overlap(path, bound))
                    }) || (path.is_none()
                        && contents.iter().any(|entry| {
                            bound_paths
                                .iter()
                                .any(|bound| paths_overlap(&entry.key, bound))
                        })));
                (targeted || bound).then(|| Msg::DataModelUpdate {
                    surface_id: surface_id.to_string(),
                    path: path.clone(),
                    contents: contents.clone(),
                })
            }
            _ => None,
        };
        if let Some(message) = kept
            && let Ok(value) = serde_json::to_value(&message)
        {
            updates.push(value);
        }
    }
    updates
}

/// The widgets a run pushed, each with the a2ui updates the run sent after the push appended
/// to its `updates`. A widget that is no object is left out.
pub fn finished_widgets(fold: &ChatFold) -> Vec<Value> {
    let parsed: Vec<Option<Msg>> = fold
        .a2ui
        .iter()
        .map(|payload| serde_json::from_value(payload.clone()).ok())
        .collect();
    fold.widgets()
        .filter_map(|(widget, pushed_at)| {
            let mut widget = widget.clone();
            let object = widget.as_object_mut()?;
            let later: Vec<Msg> = parsed[pushed_at.min(parsed.len())..]
                .iter()
                .flatten()
                .cloned()
                .collect();
            if later.is_empty() {
                return Some(widget);
            }
            let instance_id = object.get("instance_id")?.as_str()?.to_string();
            let surface_id = object
                .get("surface_id")
                .and_then(Value::as_str)
                .unwrap_or(&instance_id)
                .to_string();
            let component = object.get("component").cloned().unwrap_or(Value::Null);
            let updates = widget_updates(&instance_id, &surface_id, &component, &later);
            let kept = object
                .entry("updates")
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Some(kept) = kept.as_array_mut() {
                kept.extend(updates);
            }
            Some(widget)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn upsert(element_id: &str, value: Value) -> Msg {
        Msg::UpsertElement {
            element_id: element_id.to_string(),
            value,
        }
    }

    #[test]
    fn nested_widgets_and_bound_paths_are_followed() {
        let component = json!({
            "type": "widgetInstance",
            "inlineWidgetDef": { "components": [
                { "id": "a-box", "component": { "type": "column" } },
                { "id": "a-title", "component": { "type": "text", "text": { "path": "/title" } } },
            ]},
        });
        let log = vec![
            upsert("box", json!({ "type": "pushChild", "childId": "b" })),
            upsert("b", json!({ "type": "setText", "text": "inner" })),
            upsert("unrelated", json!({ "type": "setText", "text": "x" })),
            Msg::DataModelUpdate {
                surface_id: "board-surface".into(),
                path: Some("/title".into()),
                contents: vec![],
            },
            Msg::DataModelUpdate {
                surface_id: "board-surface".into(),
                path: Some("/other".into()),
                contents: vec![],
            },
        ];
        let updates = widget_updates("a", "a", &component, &log);
        assert_eq!(updates.len(), 3);
        assert_eq!(updates[1]["element_id"], "b");
        assert_eq!(updates[2]["surface_id"], "a");
    }

    #[test]
    fn a_widget_gets_only_the_updates_sent_after_its_push() {
        let mut fold = ChatFold::default();
        let set = |text: &str| {
            json!({
                "event_type": "a2ui",
                "payload": { "type": "upsertElement", "element_id": "w1", "value": { "type": "setText", "text": text } },
            })
        };
        fold.push(&set("before"));
        fold.push(&json!({
            "event_type": "chat_stream_partial",
            "payload": { "widgets": [{ "instance_id": "w1", "surface_id": "w1", "component": {}, "updates": [{ "pre": true }] }] },
        }));
        fold.push(&set("after"));
        let widgets = finished_widgets(&fold);
        assert_eq!(widgets.len(), 1);
        let updates = widgets[0]["updates"].as_array().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0], json!({ "pre": true }));
        assert_eq!(updates[1]["value"]["text"], "after");
    }
}
