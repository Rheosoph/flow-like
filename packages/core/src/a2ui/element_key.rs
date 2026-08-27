use flow_like_types::Value;
use flow_like_types::json::Map;

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
    let host_suffix = format!("/{prefix}");
    elements.iter().any(|(key, value)| {
        (key == prefix || key.ends_with(&host_suffix))
            && value
                .get("component")
                .and_then(|c| c.get("type"))
                .and_then(|t| t.as_str())
                == Some("widgetInstance")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

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
}
