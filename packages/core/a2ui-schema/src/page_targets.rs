//! Keeping a page's workflow targets inside the app that owns the page.
//!
//! A `workflow_event` action is the one action whose context can redirect the
//! run away from the surface it was fired on: the runtime prefers
//! `context.appId` / `context.boardId` over the identity of the page being
//! rendered. A page copied between apps — a fork, a template instantiation, an
//! agent duplicating a layout — therefore keeps firing the source app's nodes,
//! and a viewer who happens to hold execute rights on that source app runs it
//! as themselves. Normalizing on write is what makes the stored page match the
//! app it lives in.
//!
//! The rewrite is deliberately narrow: only the direct keys of a
//! `workflow_event` action's own `context`, and only when its `appId` names an
//! app that is not the one being written. Cross-app ids are a supported feature
//! elsewhere in the same JSON — `WidgetRef.app_id`,
//! `WidgetInstanceProps.appId`, `AppLinkProps.appId`, `OntologyGraphProps.appId`
//! and `submit_feedback` contexts all name a foreign app on purpose — so a
//! blanket "no foreign appId" sweep would break them.
//!
//! A foreign `appId` drags its `boardId` with it: the board named alongside it
//! belongs to that other app, so it is dropped and the action falls back to the
//! surface it is rendered on. Deciding this from the *app* rather than from a
//! board-membership list is what keeps it safe — `manifest.app` is a
//! last-write-wins document that several endpoints rewrite without a shared
//! lock, so a board can be briefly absent from it, and a page must never be
//! rewritten (or rejected) on the strength of that. It also means the desktop
//! and the API reach the same answer, so a page does not flip back and forth
//! depending on which one saved it.
//!
//! This is why the module walks the tree itself instead of reusing
//! [`super::id_refs`]: that walker recognizes ids by field name anywhere in the
//! tree and can only *replace* a value, while retargeting needs an enclosing
//! `name == "workflow_event"` predicate and has to *delete* the board key that
//! travelled with a foreign app.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

use super::BoundValue;
use super::widget::{ActionBinding, Page, PageContent, WidgetInstance};

const WORKFLOW_EVENT_ACTION: &str = "workflow_event";
const APP_ID_KEYS: [&str; 2] = ["appId", "app_id"];
const BOARD_ID_KEYS: [&str; 2] = ["boardId", "board_id"];

/// One id that did not belong to the owning app. `to` is `None` when the key
/// was dropped so the action falls back to the surface's own board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetargetedAction {
    pub component_id: String,
    pub field: &'static str,
    pub from: String,
    pub to: Option<String>,
}

/// Rewrite every `workflow_event` action context on `page` that names another
/// app so it targets `app_id`, dropping the board reference that came with it.
///
/// Returns what was changed; an empty result means the page was already
/// consistent, which is the steady state after one save.
pub fn retarget_page_workflow_actions(page: &mut Page, app_id: &str) -> Vec<RetargetedAction> {
    let mut retarget = Retarget {
        app_id,
        changes: Vec::new(),
    };

    for component in page.components.iter_mut() {
        let component_id = component.id.clone();
        retarget.walk(&component_id, &mut component.component);
    }

    for content in page.content.iter_mut() {
        match content {
            PageContent::Component(component) => {
                let component_id = component.id.clone();
                retarget.walk(&component_id, &mut component.component);
            }
            PageContent::Widget(instance) => retarget.walk_widget_instance(instance),
            PageContent::ComponentRef(_) => {}
        }
    }

    // Inlined widget definitions are page data, wherever the definition came
    // from: `WidgetSelector` lists widgets across all of the user's apps and
    // `insertWidgetInstance` inlines whatever was dragged in. A `workflow_event`
    // inside one still runs on the surface that renders it, so it belongs to
    // the app that owns the page like any other component.
    for widget in page.widget_refs.values_mut() {
        for component in widget.components.iter_mut() {
            let component_id = component.id.clone();
            retarget.walk(&component_id, &mut component.component);
        }
        for entry in widget.data_model.iter_mut() {
            let component_id = format!("{}:{}", widget.id, entry.key);
            retarget.walk(&component_id, &mut entry.value);
        }
        // An exposed prop can target `actions` / `eventHandlers`, so its default
        // is applied onto a component at render and can carry a live
        // `workflow_event`. The fork's twin walks these too.
        for prop in widget.exposed_props.iter_mut() {
            let component_id = format!("{}:{}", widget.id, prop.id);
            if let Some(default_value) = prop.default_value.as_mut() {
                retarget.walk_json_blob(&component_id, default_value);
            }
        }
        for option in widget.customization_options.iter_mut() {
            let component_id = format!("{}:{}", widget.id, option.id);
            if let Some(default_value) = option.default_value.as_mut() {
                retarget.walk_json_blob(&component_id, default_value);
            }
        }
    }

    retarget.changes
}

struct Retarget<'a> {
    app_id: &'a str,
    changes: Vec<RetargetedAction>,
}

impl Retarget<'_> {
    fn walk(&mut self, component_id: &str, value: &mut Value) {
        match value {
            Value::Object(map) => {
                if is_workflow_event_action(map)
                    && let Some(context) = map.get_mut("context")
                {
                    self.retarget_action_context(component_id, context);
                }
                for (_, child) in map.iter_mut() {
                    self.walk(component_id, child);
                }
            }
            Value::Array(items) => {
                for item in items.iter_mut() {
                    self.walk(component_id, item);
                }
            }
            Value::String(raw) => self.walk_embedded_json(component_id, raw),
            _ => {}
        }
    }

    /// A `literalJson` payload carries a whole document as a string, so an
    /// action list can hide inside one. Re-serialization only happens when the
    /// walk actually changed something, which keeps untouched payloads
    /// byte-identical.
    fn walk_embedded_json(&mut self, component_id: &str, raw: &mut String) {
        let trimmed = raw.trim_start();
        if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
            return;
        }
        let Ok(mut parsed) = serde_json::from_str::<Value>(raw) else {
            return;
        };
        let before = self.changes.len();
        self.walk(component_id, &mut parsed);
        if self.changes.len() == before {
            return;
        }
        match serde_json::to_string(&parsed) {
            Ok(encoded) => *raw = encoded,
            Err(err) => tracing::warn!(
                component_id = %component_id,
                error = %err,
                "re-encode embedded json after workflow retarget failed; leaving payload untouched"
            ),
        }
    }

    fn walk_json_blob(&mut self, component_id: &str, blob: &mut Vec<u8>) {
        let Ok(mut parsed) = serde_json::from_slice::<Value>(blob) else {
            return;
        };
        let before = self.changes.len();
        self.walk(component_id, &mut parsed);
        if self.changes.len() == before {
            return;
        }
        match serde_json::to_vec(&parsed) {
            Ok(encoded) => *blob = encoded,
            Err(err) => tracing::warn!(
                component_id = %component_id,
                error = %err,
                "re-encode widget value after workflow retarget failed; leaving payload untouched"
            ),
        }
    }

    fn walk_widget_instance(&mut self, instance: &mut WidgetInstance) {
        let instance_id = instance.instance_id.clone();
        for (action_id, binding) in instance.action_bindings.iter_mut() {
            let ActionBinding::WorkflowEvent {
                context_mapping, ..
            } = binding
            else {
                continue;
            };
            let component_id = format!("{instance_id}:{action_id}");
            self.retarget_context_mapping(&component_id, context_mapping);
        }
        for value in instance.exposed_prop_values.values_mut() {
            self.walk_json_blob(&instance_id, value);
        }
        for value in instance.customization_values.values_mut() {
            self.walk_json_blob(&instance_id, value);
        }
    }

    /// Only the context's own keys are considered. Everything below them is
    /// user-authored payload handed to the node, where an `appId` is data, not
    /// a target.
    fn retarget_action_context(&mut self, component_id: &str, context: &mut Value) {
        let Value::Object(map) = context else {
            return;
        };

        let mut had_foreign_app = false;
        for key in APP_ID_KEYS {
            let Some(current) = map.get(key).and_then(bound_string).map(str::to_string) else {
                continue;
            };
            if current.is_empty() || current == self.app_id {
                continue;
            }
            let Some(slot) = map.get_mut(key) else {
                continue;
            };
            if set_bound_string(slot, self.app_id) {
                had_foreign_app = true;
                self.record(
                    component_id,
                    "appId",
                    current,
                    Some(self.app_id.to_string()),
                );
            }
        }

        if !had_foreign_app {
            return;
        }
        for key in BOARD_ID_KEYS {
            let Some(current) = map.get(key).and_then(bound_string).map(str::to_string) else {
                continue;
            };
            if current.is_empty() {
                continue;
            }
            map.remove(key);
            self.record(component_id, "boardId", current, None);
        }
    }

    /// The typed twin of [`Self::retarget_action_context`]: a widget instance
    /// stores the same context as `ActionBinding::WorkflowEvent.context_mapping`
    /// instead of opaque JSON.
    fn retarget_context_mapping(
        &mut self,
        component_id: &str,
        mapping: &mut HashMap<String, BoundValue>,
    ) {
        let mut had_foreign_app = false;
        for key in APP_ID_KEYS {
            let Some(BoundValue::LiteralString { value }) = mapping.get_mut(key) else {
                continue;
            };
            if value.is_empty() || value.as_str() == self.app_id {
                continue;
            }
            let from = std::mem::replace(value, self.app_id.to_string());
            had_foreign_app = true;
            self.record(component_id, "appId", from, Some(self.app_id.to_string()));
        }

        if !had_foreign_app {
            return;
        }
        for key in BOARD_ID_KEYS {
            let Some(BoundValue::LiteralString { value }) = mapping.get(key) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            let from = value.clone();
            mapping.remove(key);
            self.record(component_id, "boardId", from, None);
        }
    }

    fn record(
        &mut self,
        component_id: &str,
        field: &'static str,
        from: String,
        to: Option<String>,
    ) {
        self.changes.push(RetargetedAction {
            component_id: component_id.to_string(),
            field,
            from,
            to,
        });
    }
}

/// The action's `name` is a plain string in every stored shape — `Action.name`
/// is not a `BoundValue` — so an exact match is enough to separate a workflow
/// trigger from `submit_feedback`, `navigate_app_config` and every other action
/// that names a foreign app on purpose.
fn is_workflow_event_action(map: &serde_json::Map<String, Value>) -> bool {
    matches!(map.get("name"), Some(Value::String(name)) if name == WORKFLOW_EVENT_ACTION)
}

fn bound_string(value: &Value) -> Option<&str> {
    match value {
        Value::String(raw) => Some(raw.as_str()),
        Value::Object(obj) => match obj.get("literalString") {
            Some(Value::String(raw)) => Some(raw.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn set_bound_string(target: &mut Value, new_id: &str) -> bool {
    match target {
        Value::String(raw) => {
            *raw = new_id.to_string();
            true
        }
        Value::Object(obj) => match obj.get_mut("literalString") {
            Some(Value::String(raw)) => {
                *raw = new_id.to_string();
                true
            }
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SurfaceComponent;
    use serde_json::json;

    fn page_with_component(component: Value) -> Page {
        let mut page = Page::new("page_1", "Page", "/");
        page.components
            .push(SurfaceComponent::new("button_1", component));
        page
    }

    #[test]
    fn foreign_app_and_board_are_normalized() {
        let mut page = page_with_component(json!({
            "actions": [{
                "name": "workflow_event",
                "context": {
                    "nodeId": "node_1",
                    "appId": "other_app",
                    "boardId": "other_board"
                }
            }]
        }));

        let changes = retarget_page_workflow_actions(&mut page, "own_app");

        let context = &page.components[0].component["actions"][0]["context"];
        assert_eq!(context["appId"], "own_app");
        assert_eq!(context["nodeId"], "node_1");
        assert!(context.get("boardId").is_none());
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().any(|change| change.field == "appId"
            && change.from == "other_app"
            && change.to.as_deref() == Some("own_app")));
        assert!(
            changes
                .iter()
                .any(|change| change.field == "boardId" && change.to.is_none())
        );
    }

    #[test]
    fn a_board_that_travelled_with_a_foreign_app_goes_with_it() {
        let mut page = page_with_component(json!({
            "eventHandlers": {
                "onClick": [{
                    "name": "workflow_event",
                    "context": {
                        "appId": { "literalString": "other_app" },
                        "boardId": { "literalString": "other_board" }
                    }
                }]
            }
        }));

        let changes = retarget_page_workflow_actions(&mut page, "own_app");

        let context = &page.components[0].component["eventHandlers"]["onClick"][0]["context"];
        assert_eq!(context["appId"]["literalString"], "own_app");
        assert!(context.get("boardId").is_none());
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn an_action_already_on_this_app_is_left_entirely_alone() {
        // The board list is deliberately not consulted: `manifest.app` is
        // last-write-wins across several endpoints, so a board can be missing
        // from it for reasons that have nothing to do with this page.
        let mut page = page_with_component(json!({
            "actions": [{
                "name": "workflow_event",
                "context": {
                    "appId": "own_app",
                    "boardId": "a_board_the_manifest_may_not_list_yet",
                    "nodeId": "node_1"
                }
            }]
        }));

        let before = page.components[0].component.clone();
        let changes = retarget_page_workflow_actions(&mut page, "own_app");

        assert_eq!(page.components[0].component, before);
        assert!(changes.is_empty());
    }

    #[test]
    fn other_actions_keep_their_foreign_app() {
        let mut page = page_with_component(json!({
            "appId": "other_app",
            "widgetRef": { "appId": "other_app", "widgetId": "w1" },
            "actions": [
                { "name": "submit_feedback", "context": { "appId": "other_app" } },
                { "name": "navigate_app_config", "context": { "appId": "other_app" } }
            ]
        }));

        let before = page.components[0].component.clone();
        let changes = retarget_page_workflow_actions(&mut page, "own_app");

        assert_eq!(page.components[0].component, before);
        assert!(changes.is_empty());
    }

    #[test]
    fn embedded_json_payloads_are_retargeted() {
        let mut page = page_with_component(json!({
            "data": {
                "literalJson": "{\"actions\":[{\"name\":\"workflow_event\",\"context\":{\"appId\":\"other_app\"}}]}"
            }
        }));

        let changes = retarget_page_workflow_actions(&mut page, "own_app");

        let encoded = page.components[0].component["data"]["literalJson"]
            .as_str()
            .expect("literalJson stays a string");
        let inner: Value = serde_json::from_str(encoded).expect("still valid json");
        assert_eq!(inner["actions"][0]["context"]["appId"], "own_app");
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn typed_action_bindings_are_retargeted() {
        let mut instance = WidgetInstance::new("widget_1", "instance_1");
        let mut context_mapping = HashMap::new();
        context_mapping.insert("appId".to_string(), BoundValue::literal_string("other_app"));
        context_mapping.insert(
            "boardId".to_string(),
            BoundValue::literal_string("other_board"),
        );
        instance.action_bindings.insert(
            "clicked".to_string(),
            ActionBinding::WorkflowEvent {
                event_id: "node_1".to_string(),
                context_mapping,
            },
        );

        let mut page = Page::new("page_1", "Page", "/");
        page.content.push(PageContent::Widget(instance));

        let changes = retarget_page_workflow_actions(&mut page, "own_app");

        let PageContent::Widget(instance) = &page.content[0] else {
            panic!("widget content survives retargeting");
        };
        let Some(ActionBinding::WorkflowEvent {
            context_mapping, ..
        }) = instance.action_bindings.get("clicked")
        else {
            panic!("workflow binding survives retargeting");
        };
        assert!(matches!(
            context_mapping.get("appId"),
            Some(BoundValue::LiteralString { value }) if value == "own_app"
        ));
        assert!(context_mapping.get("boardId").is_none());
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].component_id, "instance_1:clicked");
    }
}
