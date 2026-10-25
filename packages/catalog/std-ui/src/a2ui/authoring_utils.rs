use flow_like::a2ui::{
    SurfaceComponent,
    authoring::{lower_component, lower_components},
};
use flow_like_types::{
    Value,
    json::{from_value, json},
};

/// Lowers authored components (flat or nested, plain or wrapped values) to `SurfaceComponent`s,
/// so model output like `{"id":"t","type":"text","content":"Hi"}` can be rendered directly.
pub fn lower_surface_components(
    values: Vec<Value>,
) -> flow_like_types::Result<Vec<SurfaceComponent>> {
    let (lowered, errors) = lower_components(values);
    if !errors.is_empty() {
        let details = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(flow_like_types::anyhow!(
            "Invalid A2UI components: {details}"
        ));
    }

    lowered
        .into_iter()
        .enumerate()
        .map(|(index, component)| {
            let label = component
                .get("id")
                .and_then(Value::as_str)
                .map_or_else(|| format!("#{index}"), |id| format!("'{id}'"));
            from_value::<SurfaceComponent>(component).map_err(|error| {
                flow_like_types::anyhow!("Invalid A2UI component {label}: {error}")
            })
        })
        .collect()
}

/// Lowers authored component data (`{"type", ...props}`) to its canonical form: plain values
/// become bound literals and child id arrays become explicit lists.
pub fn lower_component_data(data: Value) -> flow_like_types::Result<Value> {
    let mut lowered = lower_component(json!({ "component": data }))
        .map_err(|error| flow_like_types::anyhow!("Invalid A2UI component data: {error}"))?;
    Ok(lowered
        .get_mut("component")
        .map(Value::take)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_plain_components_lower_to_surface_components() {
        let components = lower_surface_components(vec![
            json!({"id": "root", "type": "column", "gap": "12px", "children": ["title"]}),
            json!({
                "id": "title",
                "type": "text",
                "content": "Recipe Remix",
                "variant": "heading",
                "style": {"className": "p-4"}
            }),
        ])
        .unwrap();

        assert_eq!(components.len(), 2);
        assert_eq!(
            components[0].component,
            json!({
                "type": "column",
                "gap": {"literalString": "12px"},
                "children": {"explicitList": ["title"]}
            })
        );
        assert_eq!(components[1].id, "title");
        assert_eq!(components[1].get_component_type_name(), "text");
        assert_eq!(
            components[1].component["content"],
            json!({"literalString": "Recipe Remix"})
        );
        assert!(components[1].style.is_some());
        assert!(components[1].component.get("style").is_none());
    }

    #[test]
    fn canonical_components_pass_through_unchanged() {
        let canonical = json!({
            "id": "title",
            "component": {
                "type": "text",
                "content": {"path": "/title"},
                "children": {"explicitList": []}
            }
        });

        let components = lower_surface_components(vec![canonical.clone()]).unwrap();

        assert_eq!(json!(components[0]), canonical);
    }

    #[test]
    fn invalid_components_name_the_offending_component() {
        let error = lower_surface_components(vec![
            json!({"id": "ok", "type": "divider"}),
            json!({"id": "broken", "content": "Hi"}),
        ])
        .unwrap_err()
        .to_string();
        assert!(error.contains("broken"), "{error}");

        let error = lower_surface_components(vec![json!({"type": "divider"})])
            .unwrap_err()
            .to_string();
        assert!(error.contains("#0"), "{error}");
    }

    #[test]
    fn component_data_lowers_props_in_place() {
        let data = lower_component_data(json!({
            "type": "link",
            "href": "https://flow-like.com",
            "label": "Docs",
            "variant": {"literalString": "subtle"},
            "hidden": false,
            "children": ["icon"],
            "style": {"className": "p-2"}
        }))
        .unwrap();

        assert_eq!(
            data,
            json!({
                "type": "link",
                "href": {"literalString": "https://flow-like.com"},
                "label": {"literalString": "Docs"},
                "variant": "subtle",
                "hidden": {"literalBool": false},
                "children": {"explicitList": ["icon"]},
                "style": {"className": "p-2"}
            })
        );
        assert_eq!(
            lower_component_data(json!({"type": "divider"})).unwrap(),
            json!({"type": "divider"})
        );
        assert!(lower_component_data(json!({"label": "Docs"})).is_err());
    }
}
