//! A page's component tree reduced to component types, for a wireframe on the store page.
//! Only the `type` of each component travels; text, bindings, styles and data never do.

use super::types::ShowcaseOutlineNode;
use flow_like::a2ui::widget::{Page, PageContent};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const MAX_DEPTH: usize = 6;
const MAX_NODES: usize = 80;
const MAX_CHILDREN: usize = 12;

pub fn page_outline(page: &Page) -> ShowcaseOutlineNode {
    let components: HashMap<&str, &Value> = page
        .components
        .iter()
        .map(|component| (component.id.as_str(), &component.component))
        .collect();

    let mut roots: Vec<String> = page
        .content
        .iter()
        .filter_map(|content| match content {
            PageContent::ComponentRef(id) => Some(id.clone()),
            PageContent::Component(component) => Some(component.id.clone()),
            PageContent::Widget(_) => None,
        })
        .collect();
    if roots.is_empty() {
        let referenced: HashSet<String> = components
            .values()
            .flat_map(|component| child_ids(component))
            .collect();
        roots = page
            .components
            .iter()
            .map(|component| component.id.clone())
            .filter(|id| !referenced.contains(id))
            .collect();
    }

    let inline: HashMap<&str, &Value> = page
        .content
        .iter()
        .filter_map(|content| match content {
            PageContent::Component(component) => {
                Some((component.id.as_str(), &component.component))
            }
            _ => None,
        })
        .collect();

    let mut budget = MAX_NODES;
    let mut seen = HashSet::new();
    let children = roots
        .iter()
        .filter_map(|id| {
            let component = components
                .get(id.as_str())
                .or_else(|| inline.get(id.as_str()))?;
            build(id, component, &components, 1, &mut budget, &mut seen)
        })
        .take(MAX_CHILDREN)
        .collect();
    ShowcaseOutlineNode {
        t: "page".to_string(),
        c: children,
    }
}

fn build(
    id: &str,
    component: &Value,
    components: &HashMap<&str, &Value>,
    depth: usize,
    budget: &mut usize,
    seen: &mut HashSet<String>,
) -> Option<ShowcaseOutlineNode> {
    if *budget == 0 || !seen.insert(id.to_string()) {
        return None;
    }
    *budget -= 1;
    let t = component_type(component);
    let c = if depth >= MAX_DEPTH {
        Vec::new()
    } else {
        child_ids(component)
            .into_iter()
            .filter_map(|child| {
                let value = components.get(child.as_str())?;
                build(&child, value, components, depth + 1, budget, seen)
            })
            .take(MAX_CHILDREN)
            .collect()
    };
    Some(ShowcaseOutlineNode { t, c })
}

/// The component's type in lower case, limited to identifier characters.
fn component_type(component: &Value) -> String {
    let raw = component
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(40)
        .collect::<String>()
        .to_ascii_lowercase();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

/// Ids a component places inside itself: `explicitList` arrays and single `child` refs, at any
/// depth of the component object (slots such as tabs or card footers nest them).
fn child_ids(component: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    collect_child_ids(component, &mut ids, 0);
    ids
}

fn collect_child_ids(value: &Value, ids: &mut Vec<String>, depth: usize) {
    if depth > 6 {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, entry) in map {
                match (key.as_str(), entry) {
                    ("explicitList", Value::Array(list)) => {
                        ids.extend(list.iter().filter_map(Value::as_str).map(str::to_string));
                    }
                    ("child", Value::String(id)) => ids.push(id.clone()),
                    _ => collect_child_ids(entry, ids, depth + 1),
                }
            }
        }
        Value::Array(list) => {
            for entry in list {
                collect_child_ids(entry, ids, depth + 1);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_types_and_nesting_only() {
        let root = json!({
            "type": "Column",
            "children": {"explicitList": ["title", "grid"]},
            "gap": {"literalString": "8px"}
        });
        let grid = json!({"type": "grid", "children": {"explicitList": ["chart", "table"]}});
        let components: HashMap<&str, &Value> = [
            ("root", &root),
            (
                "title",
                &json!({"type": "text", "text": {"literalString": "Secret"}}),
            ),
            ("grid", &grid),
            ("chart", &json!({"type": "chart"})),
            ("table", &json!({"type": "table"})),
        ]
        .into_iter()
        .map(|(id, value)| (id, Box::leak(Box::new(value.clone())) as &Value))
        .collect();

        let mut budget = MAX_NODES;
        let mut seen = HashSet::new();
        let node = build("root", &root, &components, 1, &mut budget, &mut seen).unwrap();
        assert_eq!(node.t, "column");
        assert_eq!(node.c.len(), 2);
        assert_eq!(
            node.c[0],
            ShowcaseOutlineNode {
                t: "text".into(),
                c: vec![]
            }
        );
        assert_eq!(
            node.c[1].c.iter().map(|n| n.t.as_str()).collect::<Vec<_>>(),
            vec!["chart", "table"]
        );
        assert!(!serde_json::to_string(&node).unwrap().contains("Secret"));
    }

    #[test]
    fn cycles_and_unknown_types_are_safe() {
        let a = json!({"type": "row", "child": "b"});
        let b = json!({"type": "<script>", "child": "a"});
        let components: HashMap<&str, &Value> = [("a", &a), ("b", &b)].into_iter().collect();
        let mut budget = MAX_NODES;
        let mut seen = HashSet::new();
        let node = build("a", &a, &components, 1, &mut budget, &mut seen).unwrap();
        assert_eq!(node.c[0].t, "script");
        assert!(node.c[0].c.is_empty());
    }
}
