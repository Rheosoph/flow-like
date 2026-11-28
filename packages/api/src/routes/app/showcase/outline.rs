//! A page's component tree reduced to component types, for a wireframe on the store page.
//! Only the `type` of each component travels; text, bindings, styles and data never do.

use super::types::ShowcaseOutlineNode;
use flow_like::a2ui::SurfaceComponent;
use flow_like::a2ui::widget::{Page, PageContent};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const MAX_DEPTH: usize = 6;
const MAX_NODES: usize = 80;
const MAX_CHILDREN: usize = 12;

type ComponentMap<'a> = HashMap<&'a str, &'a Value>;

struct Scope<'a> {
    name: String,
    components: ComponentMap<'a>,
}

enum Root<'a> {
    Id(&'a str),
    Widget(&'a str),
}

struct Walker<'a> {
    page: &'a Page,
    budget: usize,
    seen: HashSet<String>,
}

pub fn page_outline(page: &Page) -> ShowcaseOutlineNode {
    let page_scope = Scope {
        name: String::new(),
        components: page
            .components
            .iter()
            .map(|component| (component.id.as_str(), &component.component))
            .collect(),
    };

    let mut roots: Vec<Root> = page
        .content
        .iter()
        .map(|content| match content {
            PageContent::ComponentRef(id) => Root::Id(id.as_str()),
            PageContent::Component(component) => Root::Id(component.id.as_str()),
            PageContent::Widget(instance) => Root::Widget(instance.instance_id.as_str()),
        })
        .collect();
    if roots.is_empty() {
        let referenced: HashSet<String> = page_scope
            .components
            .values()
            .flat_map(|component| child_ids(component))
            .collect();
        roots = page
            .components
            .iter()
            .map(|component| component.id.as_str())
            .filter(|id| !referenced.contains(*id))
            .map(Root::Id)
            .collect();
    }

    let inline: ComponentMap = page
        .content
        .iter()
        .filter_map(|content| match content {
            PageContent::Component(component) => {
                Some((component.id.as_str(), &component.component))
            }
            _ => None,
        })
        .collect();

    let mut walker = Walker {
        page,
        budget: MAX_NODES,
        seen: HashSet::new(),
    };
    let children = roots
        .iter()
        .filter_map(|root| match root {
            Root::Id(id) => {
                let component = page_scope.components.get(id).or_else(|| inline.get(id))?;
                walker.build(id, component, &page_scope, 1)
            }
            Root::Widget(instance_id) => {
                let widget = page.widget_refs.get(*instance_id)?;
                let scope = widget_scope(instance_id, &widget.components);
                walker.build_widget_root(&widget.root_component_id, &scope, 1)
            }
        })
        .take(MAX_CHILDREN)
        .collect();
    ShowcaseOutlineNode {
        t: "page".to_string(),
        c: children,
    }
}

fn widget_scope<'a>(instance_id: &str, components: &'a [SurfaceComponent]) -> Scope<'a> {
    Scope {
        name: instance_id.to_string(),
        components: components
            .iter()
            .map(|component| (component.id.as_str(), &component.component))
            .collect(),
    }
}

fn inline_widget_scope<'a>(
    instance_id: &str,
    definition: &'a Value,
) -> Option<(Scope<'a>, &'a str)> {
    let root = definition.get("rootComponentId")?.as_str()?;
    let components = definition
        .get("components")?
        .as_array()?
        .iter()
        .filter_map(|entry| Some((entry.get("id")?.as_str()?, entry.get("component")?)))
        .collect();
    Some((
        Scope {
            name: instance_id.to_string(),
            components,
        },
        root,
    ))
}

impl<'a> Walker<'a> {
    fn resolve_instance(&self, component: &'a Value) -> Option<(Scope<'a>, &'a str)> {
        let instance_id = component.get("instanceId").and_then(Value::as_str);
        if let Some(widget) = instance_id.and_then(|id| self.page.widget_refs.get(id)) {
            let scope = widget_scope(instance_id?, &widget.components);
            return Some((scope, widget.root_component_id.as_str()));
        }
        inline_widget_scope(
            instance_id.unwrap_or("inline"),
            component.get("inlineWidgetDef")?,
        )
    }

    fn build_widget_root(
        &mut self,
        root: &str,
        scope: &Scope<'a>,
        depth: usize,
    ) -> Option<ShowcaseOutlineNode> {
        let component = scope.components.get(root)?;
        self.build(root, component, scope, depth)
    }

    fn build(
        &mut self,
        id: &str,
        component: &'a Value,
        scope: &Scope<'a>,
        depth: usize,
    ) -> Option<ShowcaseOutlineNode> {
        if self.budget == 0 || !self.seen.insert(format!("{}/{id}", scope.name)) {
            return None;
        }
        let t = component_type(component);
        match t.as_str() {
            "widgetinstance" => {
                return match self.resolve_instance(component) {
                    Some((inner, root)) => self.build_widget_root(root, &inner, depth),
                    None => Some(ShowcaseOutlineNode { t, c: Vec::new() }),
                };
            }
            "microwidgetinstance" => return Some(ShowcaseOutlineNode { t, c: Vec::new() }),
            _ => {}
        }
        self.budget -= 1;
        let c = if depth >= MAX_DEPTH {
            Vec::new()
        } else {
            child_ids(component)
                .into_iter()
                .filter_map(|child| {
                    let value = scope.components.get(child.as_str())?;
                    self.build(&child, value, scope, depth + 1)
                })
                .take(MAX_CHILDREN)
                .collect()
        };
        Some(ShowcaseOutlineNode { t, c })
    }
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
    use flow_like::a2ui::widget::{Widget, WidgetInstance};
    use serde_json::json;

    fn types(node: &ShowcaseOutlineNode) -> Vec<&str> {
        node.c.iter().map(|n| n.t.as_str()).collect()
    }

    fn column_widget(id: &str) -> Widget {
        let mut widget = Widget::new("w", "W", "root");
        widget.components = vec![
            SurfaceComponent::new(
                "root",
                json!({"type": "Column", "children": {"explicitList": ["a", "b"]}}),
            ),
            SurfaceComponent::new(
                "a",
                json!({"type": "text", "text": {"literalString": "Secret"}}),
            ),
            SurfaceComponent::new("b", json!({"type": "chart"})),
        ];
        widget.id = id.to_string();
        widget
    }

    fn page() -> Page {
        Page::new("p", "P", "/p")
    }

    #[test]
    fn keeps_types_and_nesting_only() {
        let mut page = page();
        page.components = vec![
            SurfaceComponent::new(
                "root",
                json!({
                    "type": "Column",
                    "children": {"explicitList": ["title", "grid"]},
                    "gap": {"literalString": "8px"}
                }),
            ),
            SurfaceComponent::new(
                "title",
                json!({"type": "text", "text": {"literalString": "Secret"}}),
            ),
            SurfaceComponent::new(
                "grid",
                json!({"type": "grid", "children": {"explicitList": ["chart", "table"]}}),
            ),
            SurfaceComponent::new("chart", json!({"type": "chart"})),
            SurfaceComponent::new("table", json!({"type": "table"})),
        ];
        let outline = page_outline(&page);
        let node = &outline.c[0];
        assert_eq!(node.t, "column");
        assert_eq!(node.c.len(), 2);
        assert_eq!(
            node.c[0],
            ShowcaseOutlineNode {
                t: "text".into(),
                c: vec![]
            }
        );
        assert_eq!(types(&node.c[1]), vec!["chart", "table"]);
        assert!(!serde_json::to_string(&outline).unwrap().contains("Secret"));
    }

    #[test]
    fn cycles_and_unknown_types_are_safe() {
        let mut page = page();
        page.content = vec![PageContent::ComponentRef("a".into())];
        page.components = vec![
            SurfaceComponent::new("a", json!({"type": "row", "child": "b"})),
            SurfaceComponent::new("b", json!({"type": "<script>", "child": "a"})),
        ];
        let outline = page_outline(&page);
        assert_eq!(outline.c[0].c[0].t, "script");
        assert!(outline.c[0].c[0].c.is_empty());
    }

    #[test]
    fn widget_content_resolves_through_widget_refs() {
        let mut page = page();
        page.content = vec![PageContent::Widget(WidgetInstance::new("w", "inst1"))];
        page.widget_refs.insert("inst1".into(), column_widget("w"));
        let outline = page_outline(&page);
        assert_eq!(outline.c.len(), 1);
        assert_eq!(outline.c[0].t, "column");
        assert_eq!(types(&outline.c[0]), vec!["text", "chart"]);
        assert!(!serde_json::to_string(&outline).unwrap().contains("Secret"));
    }

    #[test]
    fn widget_instance_component_resolves_through_widget_refs() {
        let mut page = page();
        page.components = vec![
            SurfaceComponent::new(
                "root",
                json!({"type": "row", "children": {"explicitList": ["wi"]}}),
            ),
            SurfaceComponent::new(
                "wi",
                json!({"type": "widgetInstance", "instanceId": "inst1", "widgetId": "w"}),
            ),
        ];
        page.widget_refs.insert("inst1".into(), column_widget("w"));
        let outline = page_outline(&page);
        let row = &outline.c[0];
        assert_eq!(row.t, "row");
        assert_eq!(row.c.len(), 1);
        assert_eq!(row.c[0].t, "column");
        assert_eq!(types(&row.c[0]), vec!["text", "chart"]);
    }

    #[test]
    fn widget_instance_component_resolves_through_inline_definition() {
        let mut page = page();
        page.components = vec![SurfaceComponent::new(
            "wi",
            json!({
                "type": "widgetInstance",
                "instanceId": "inst1",
                "widgetId": "w",
                "inlineWidgetDef": {
                    "rootComponentId": "root",
                    "components": [
                        {"id": "root", "component": {"type": "card", "child": "x"}},
                        {"id": "x", "component": {"type": "table"}}
                    ]
                }
            }),
        )];
        let outline = page_outline(&page);
        assert_eq!(outline.c[0].t, "card");
        assert_eq!(types(&outline.c[0]), vec!["table"]);
    }

    #[test]
    fn unresolved_and_micro_instances_are_leaves() {
        let mut page = page();
        page.components = vec![
            SurfaceComponent::new(
                "root",
                json!({"type": "row", "children": {"explicitList": ["a", "b"]}}),
            ),
            SurfaceComponent::new(
                "a",
                json!({"type": "widgetInstance", "instanceId": "nope", "widgetId": "w"}),
            ),
            SurfaceComponent::new("b", json!({"type": "microWidgetInstance", "child": "a"})),
        ];
        let outline = page_outline(&page);
        assert_eq!(
            types(&outline.c[0]),
            vec!["widgetinstance", "microwidgetinstance"]
        );
        assert!(outline.c[0].c.iter().all(|n| n.c.is_empty()));
    }

    #[test]
    fn two_instances_of_one_widget_both_expand() {
        let mut page = page();
        page.components = vec![
            SurfaceComponent::new(
                "root",
                json!({"type": "row", "children": {"explicitList": ["w1", "w2"]}}),
            ),
            SurfaceComponent::new(
                "w1",
                json!({"type": "widgetInstance", "instanceId": "i1", "widgetId": "w"}),
            ),
            SurfaceComponent::new(
                "w2",
                json!({"type": "widgetInstance", "instanceId": "i2", "widgetId": "w"}),
            ),
        ];
        page.widget_refs.insert("i1".into(), column_widget("w"));
        page.widget_refs.insert("i2".into(), column_widget("w"));
        let outline = page_outline(&page);
        let row = &outline.c[0];
        assert_eq!(types(row), vec!["column", "column"]);
        assert_eq!(types(&row.c[1]), vec!["text", "chart"]);
    }

    #[test]
    fn self_referencing_widget_terminates() {
        let mut page = page();
        page.content = vec![PageContent::Widget(WidgetInstance::new("w", "i"))];
        let mut widget = Widget::new("w", "W", "root");
        widget.components = vec![SurfaceComponent::new(
            "root",
            json!({"type": "widgetInstance", "instanceId": "i", "widgetId": "w"}),
        )];
        page.widget_refs.insert("i".into(), widget);
        assert!(page_outline(&page).c.is_empty());
    }
}
