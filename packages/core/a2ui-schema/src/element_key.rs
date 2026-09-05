use serde_json::Map;
use serde_json::Value;

/// Resolves an element reference against an `_elements` payload map whose keys are
/// `surfaceId/componentId`.
///
/// Resolution order:
/// 1. Exact key match — `pageId/componentId` on its own page.
/// 2. Suffix match — a bare `componentId` (with or without slashes of its own) finds the
///    current surface's `*/componentId`.
/// 3. Page retarget — a reference prefixed with *another* page's id (`id1/progress`) falls
///    back to the current surface's component of the same name (`id2/progress`), so flows
///    written against one page work on every page that has the element.
///
/// A prefix that names a widget instance of the current surface is never retargeted:
/// `instance/child` addresses into that widget and must keep its widget semantics.
pub fn resolve_element_key<'a>(
    elements: &'a Map<String, Value>,
    element_id: &str,
) -> Option<&'a String> {
    if let Some(key) = elements.keys().find(|k| k.as_str() == element_id) {
        return Some(key);
    }

    let suffix = format!("/{element_id}");
    if let Some(key) = elements.keys().find(|k| k.ends_with(&suffix)) {
        return Some(key);
    }

    let (prefix, rest) = element_id.split_once('/')?;
    if prefix.is_empty() || rest.is_empty() || is_widget_host(elements, prefix) {
        return None;
    }

    let rest_suffix = format!("/{rest}");
    elements.keys().find(|k| k.ends_with(&rest_suffix))
}

fn is_widget_host(elements: &Map<String, Value>, prefix: &str) -> bool {
    host_for_prefix(elements, prefix).is_some()
}

fn is_widget_instance(value: &Value) -> bool {
    value
        .get("component")
        .and_then(|component| component.get("type"))
        .and_then(Value::as_str)
        == Some("widgetInstance")
}

/// The widget host a child prefix names: its instance id, or its own component id.
fn host_for_prefix<'a>(elements: &'a Map<String, Value>, prefix: &str) -> Option<&'a Value> {
    let host_suffix = format!("/{prefix}");
    elements
        .iter()
        .find(|(key, value)| {
            is_widget_instance(value)
                && (key.as_str() == prefix
                    || key.ends_with(&host_suffix)
                    || value
                        .get("component")
                        .and_then(|component| component.get("instanceId"))
                        .and_then(Value::as_str)
                        == Some(prefix))
        })
        .map(|(_, value)| value)
}

/// A widget child addressed as `instanceId/childId` that the payload carries only inside its
/// host's inline definition, returned as the host stores it (`{id, component, style}`).
pub fn resolve_in_host_defs(
    elements: &Map<String, Value>,
    element_id: &str,
) -> Option<(String, Value)> {
    let (prefix, child_id) = element_id.split_once('/')?;
    if prefix.is_empty() || child_id.is_empty() {
        return None;
    }
    let child = host_for_prefix(elements, prefix)?
        .get("component")?
        .get("inlineWidgetDef")?
        .get("components")?
        .as_array()?
        .iter()
        .find(|child| child.get("id").and_then(Value::as_str) == Some(child_id))?;
    Some((element_id.to_string(), child.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn elements(entries: &[(&str, &str)]) -> Map<String, Value> {
        let mut map = Map::new();
        for (key, component_type) in entries {
            map.insert(
                key.to_string(),
                json!({ "component": { "type": component_type } }),
            );
        }
        map
    }

    #[test]
    fn exact_key_wins() {
        let map = elements(&[
            ("page-a/progress", "progress"),
            ("page-b/progress", "progress"),
        ]);
        assert_eq!(
            resolve_element_key(&map, "page-b/progress").map(String::as_str),
            Some("page-b/progress")
        );
    }

    #[test]
    fn bare_id_matches_current_surface() {
        let map = elements(&[("page-a/progress", "progress")]);
        assert_eq!(
            resolve_element_key(&map, "progress").map(String::as_str),
            Some("page-a/progress")
        );
    }

    #[test]
    fn foreign_page_prefix_retargets_to_current_surface() {
        let map = elements(&[("page-b/progress", "progress")]);
        assert_eq!(
            resolve_element_key(&map, "page-a/progress").map(String::as_str),
            Some("page-b/progress")
        );
    }

    #[test]
    fn widget_host_prefix_is_never_retargeted() {
        let map = elements(&[
            ("page-a/my-widget", "widgetInstance"),
            ("page-a/chart-1", "chart"),
        ]);
        assert_eq!(resolve_element_key(&map, "my-widget/chart-1"), None);
    }

    #[test]
    fn slash_bearing_component_id_resolves_before_retargeting() {
        let map = elements(&[("page-a/group/field", "textField")]);
        assert_eq!(
            resolve_element_key(&map, "group/field").map(String::as_str),
            Some("page-a/group/field")
        );
    }

    #[test]
    fn missing_component_stays_unresolved() {
        let map = elements(&[("page-a/progress", "progress")]);
        assert_eq!(resolve_element_key(&map, "page-b/banner"), None);
        assert_eq!(resolve_element_key(&map, "banner"), None);
    }

    fn host(instance_id: &str, children: &[&str]) -> Value {
        let components: Vec<Value> = children
            .iter()
            .map(|id| json!({ "id": id, "component": { "type": "textField" }, "style": null }))
            .collect();
        json!({
            "id": "host",
            "component": {
                "type": "widgetInstance",
                "instanceId": instance_id,
                "inlineWidgetDef": { "components": components }
            }
        })
    }

    #[test]
    fn widget_child_resolves_from_its_host_definition() {
        let mut map = Map::new();
        map.insert("page-a/host".to_string(), host("inst-1", &["field"]));

        let (key, value) = resolve_in_host_defs(&map, "inst-1/field").unwrap();
        assert_eq!(key, "inst-1/field");
        assert_eq!(value["component"]["type"], "textField");
        assert!(resolve_in_host_defs(&map, "inst-1/missing").is_none());
        assert!(resolve_in_host_defs(&map, "inst-2/field").is_none());
        assert!(resolve_in_host_defs(&map, "field").is_none());
    }

    #[test]
    fn instance_prefix_is_never_retargeted_to_the_page() {
        let mut map = Map::new();
        map.insert("page-a/host".to_string(), host("inst-1", &["field"]));
        map.insert(
            "page-a/field".to_string(),
            json!({ "component": { "type": "text" } }),
        );

        assert_eq!(resolve_element_key(&map, "inst-1/field"), None);
    }
}
