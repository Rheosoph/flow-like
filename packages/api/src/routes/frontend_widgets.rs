//! Read only widget definitions reachable from the published Page.

use std::collections::{HashSet, VecDeque};

use flow_like::a2ui::{
    SurfaceComponent,
    widget::{Page, PageContent, Widget},
};

use crate::{credentials::validate_path_component, error::ApiError, state::AppState};

type WidgetRef = (String, Option<(u32, u32, u32)>);

fn component_refs(
    app_id: &str,
    components: &[SurfaceComponent],
    refs: &mut VecDeque<WidgetRef>,
    page: Option<&Page>,
) {
    for component in components {
        append_component_ref(app_id, &component.component, refs, 0, page);
    }
}

fn append_component_ref(
    app_id: &str,
    component: &serde_json::Value,
    refs: &mut VecDeque<WidgetRef>,
    depth: usize,
    page: Option<&Page>,
) {
    if depth > 32
        || component.get("type").and_then(|value| value.as_str()) != Some("widgetInstance")
    {
        return;
    }
    let owner = component
        .get("appId")
        .and_then(|value| value.as_str())
        .unwrap_or(app_id);
    if owner != app_id {
        return;
    }
    if let Some(snapshot) = component
        .get("instanceId")
        .and_then(|value| value.as_str())
        .and_then(|id| page.and_then(|page| page.widget_refs.get(id)))
    {
        for nested in &snapshot.components {
            append_component_ref(app_id, &nested.component, refs, depth + 1, page);
        }
    } else if let Some(inline) = component
        .get("inlineWidgetDef")
        .and_then(|value| value.get("components"))
        .and_then(|value| value.as_array())
    {
        for nested in inline {
            if let Some(value) = nested.get("component") {
                append_component_ref(app_id, value, refs, depth + 1, page);
            }
        }
    } else if let Some(widget_id) = component.get("widgetId").and_then(|value| value.as_str()) {
        refs.push_back((widget_id.to_string(), None));
    }
}

pub(crate) async fn read_widget(
    state: &AppState,
    app_id: &str,
    subject: &str,
    page: &Page,
    widget_id: &str,
    requested_version: Option<(u32, u32, u32)>,
) -> Result<Widget, ApiError> {
    validate_path_component(widget_id, "widget_id")
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let mut refs = VecDeque::new();
    component_refs(app_id, &page.components, &mut refs, Some(page));
    for content in &page.content {
        match content {
            PageContent::Widget(instance) => {
                if instance
                    .widget_ref
                    .as_ref()
                    .is_some_and(|reference| reference.app_id != app_id)
                {
                    continue;
                }
                if let Some(snapshot) = page.widget_refs.get(&instance.instance_id) {
                    component_refs(app_id, &snapshot.components, &mut refs, Some(page));
                    continue;
                }
                if let Some(reference) = &instance.widget_ref {
                    if reference.app_id == app_id {
                        refs.push_back((reference.widget_id.clone(), reference.version));
                    }
                } else {
                    refs.push_back((instance.widget_id.clone(), None));
                }
            }
            PageContent::Component(component) => component_refs(
                app_id,
                std::slice::from_ref(component),
                &mut refs,
                Some(page),
            ),
            PageContent::ComponentRef(_) => {}
        }
    }
    // Bootstrap already decorates and redacts these exact inlined definitions.
    for widget in page.widget_refs.values() {
        if widget.id == widget_id
            && requested_version.is_none_or(|version| widget.version == Some(version))
        {
            return Ok(widget.clone());
        }
    }
    let app = state.master_app(subject, app_id, state).await?;
    let mut visited = HashSet::new();
    while let Some((id, version)) = refs.pop_front() {
        if !visited.insert((id.clone(), version)) {
            continue;
        }
        if visited.len() > 128 {
            return Err(ApiError::bad_request(
                "The hosted Page has too many widget dependencies",
            ));
        }
        validate_path_component(&id, "widget_id")
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        let widget = app.open_widget(id.clone(), version).await?;
        if id == widget_id && requested_version.is_none_or(|requested| version == Some(requested)) {
            return Ok(widget);
        }
        component_refs(app_id, &widget.components, &mut refs, None);
    }
    Err(ApiError::NOT_FOUND)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_same_app_widget_components_create_read_authority() {
        let mut refs = VecDeque::new();
        for component in [
            json!({"type":"widgetInstance", "widgetId":"allowed"}),
            json!({"type":"widgetInstance", "widgetId":"private", "appId":"other-app"}),
            json!({"type":"text", "widgetId":"private"}),
            json!({"type":"microWidgetInstance", "widgetId":"package-widget"}),
        ] {
            append_component_ref("app", &component, &mut refs, 0, None);
        }
        assert_eq!(
            refs.into_iter().collect::<Vec<_>>(),
            vec![("allowed".to_string(), None)]
        );
    }
}
