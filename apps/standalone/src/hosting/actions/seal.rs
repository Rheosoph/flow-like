use super::{Registry, RunGrant, Scope};
use serde_json::{Map, Value};
use std::{collections::HashSet, sync::Arc};

#[derive(Default)]
pub(crate) struct Report {
    pub sealed: usize,
    pub rejected: usize,
}

pub(crate) struct Sealer {
    scope: Scope,
    allowed_entry_nodes: HashSet<String>,
    registry: Arc<Registry>,
    run: Arc<RunGrant>,
}
impl Sealer {
    pub fn new(
        registry: Arc<Registry>,
        run: Arc<RunGrant>,
        allowed_entry_nodes: HashSet<String>,
    ) -> Self {
        Self {
            scope: run.scope.clone(),
            allowed_entry_nodes,
            registry,
            run,
        }
    }
    pub(crate) fn seal_payload(&self, event_type: &str, payload: &mut Value) -> Report {
        let mut report = Report::default();
        if matches!(
            event_type,
            "a2ui" | "chat_stream_partial" | "chat_stream" | "chat_out"
        ) && !bounded(payload)
        {
            *payload = Value::Null;
            report.rejected = 1;
            return report;
        }
        match event_type {
            "a2ui" => self.seal_a2ui_message(payload, &mut report),
            "chat_stream_partial" | "chat_stream" | "chat_out" => {
                self.seal_chat_widgets(payload, &mut report);
            }
            _ => {}
        }
        report
    }

    fn seal_chat_widgets(&self, payload: &mut Value, report: &mut Report) {
        let Some(widgets) = payload
            .as_object_mut()
            .and_then(|payload| payload.get_mut("widgets"))
            .and_then(Value::as_array_mut)
        else {
            return;
        };

        for widget in widgets {
            let Some(widget) = widget.as_object_mut() else {
                continue;
            };
            if let Some(component) = widget.get_mut("component") {
                self.seal_component(component, report);
            }
            if let Some(updates) = widget.get_mut("updates").and_then(Value::as_array_mut) {
                for update in updates {
                    self.seal_a2ui_message(update, report);
                }
            }
        }
    }

    fn seal_a2ui_message(&self, payload: &mut Value, report: &mut Report) {
        let Some(message) = payload.as_object_mut() else {
            return;
        };
        let Some(message_type) = message
            .get("type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            return;
        };

        match message_type.as_str() {
            "beginRendering" | "surfaceUpdate" => {
                self.seal_surface_components(message, "components", report);
            }
            "createElement" => {
                if let Some(component) = message.get_mut("component") {
                    self.seal_surface_component(component, report);
                }
            }
            "upsertElement" => {
                if let Some(update) = message.get_mut("value") {
                    self.seal_element_update(update, report);
                }
            }
            _ => {}
        }
    }

    fn seal_surface_components(
        &self,
        owner: &mut Map<String, Value>,
        field: &str,
        report: &mut Report,
    ) {
        let Some(components) = owner.get_mut(field).and_then(Value::as_array_mut) else {
            return;
        };
        for component in components {
            self.seal_surface_component(component, report);
        }
    }

    fn seal_surface_component(&self, value: &mut Value, report: &mut Report) {
        let Some(surface_component) = value.as_object_mut() else {
            return;
        };
        if let Some(component) = surface_component.get_mut("component") {
            self.seal_component(component, report);
        }
    }

    fn seal_component(&self, value: &mut Value, report: &mut Report) {
        let Some(component) = value.as_object_mut() else {
            return;
        };

        self.seal_legacy_actions(component, report);
        for field in ["eventHandlers", "event_handlers"] {
            self.seal_event_handlers(component, field, report);
        }
        let is_widget = component
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "widgetInstance" | "microWidgetInstance"));
        if is_widget {
            for field in ["actionBindings", "action_bindings"] {
                self.seal_action_bindings(component, field, report);
            }

            for field in ["inlineWidgetDef", "inline_widget_def"] {
                if let Some(definition) = component.get_mut(field).and_then(Value::as_object_mut) {
                    self.seal_surface_components(definition, "components", report);
                }
            }

            for field in ["runtimeChildUpdates", "runtime_child_updates"] {
                let Some(updates) = component.get_mut(field).and_then(Value::as_object_mut) else {
                    continue;
                };
                for operations in updates.values_mut() {
                    let Some(operations) = operations.as_array_mut() else {
                        continue;
                    };
                    for operation in operations {
                        self.seal_element_update(operation, report);
                    }
                }
            }
        } else {
            // Only widget instances route bindings; on any other component
            // they are unreachable and must not survive the boundary.
            strip_non_widget_bindings(component, report);
        }
    }

    fn seal_element_update(&self, value: &mut Value, report: &mut Report) {
        let Some(update) = value.as_object_mut() else {
            return;
        };
        let update_type = update
            .get("type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        match update_type.as_deref() {
            Some("setAction") => {
                if let Some(action) = update.get_mut("action").and_then(Value::as_object_mut)
                    && is_workflow_action(action)
                {
                    self.seal_action(action, report);
                }
            }
            Some("setEventActions") => {
                if update
                    .get("eventName")
                    .and_then(Value::as_str)
                    .is_some_and(|event_name| !event_name.trim().is_empty())
                {
                    self.seal_action_array(update, "actions", report);
                }
            }
            Some("createComponent") => {
                if let Some(component) = update.get_mut("component") {
                    self.seal_component(component, report);
                }
            }
            Some("setProps") => {
                // The renderer spreads `props` into the live component data,
                // so it is an executable channel like a component body.
                if let Some(props) = update.get_mut("props") {
                    if let Some(props) = props.as_object_mut() {
                        props.remove("pageAction");
                        props.remove("page_action");
                    }
                    self.seal_component(props, report);
                }
            }
            // The renderer's fallback spreads every field of an unrecognized
            // update into component data. Strip anything executable; an
            // unknown op never mints.
            _ => strip_unknown_update_actions(update, report),
        }
    }

    fn seal_action_array(&self, owner: &mut Map<String, Value>, field: &str, report: &mut Report) {
        let Some(actions) = owner.get_mut(field).and_then(Value::as_array_mut) else {
            return;
        };
        for action in actions {
            let Some(action) = action.as_object_mut() else {
                continue;
            };
            if is_workflow_action(action) {
                self.seal_action(action, report);
            }
        }
    }

    fn seal_legacy_actions(&self, component: &mut Map<String, Value>, report: &mut Report) {
        let Some(actions) = component.get_mut("actions").and_then(Value::as_array_mut) else {
            return;
        };
        for (index, action) in actions.iter_mut().enumerate() {
            let Some(action) = action.as_object_mut() else {
                continue;
            };
            if !is_workflow_action(action) {
                continue;
            }
            // The renderer's legacy fallback executes only actions[0]. Named
            // handlers keep ordered multi-action semantics above.
            if index == 0 {
                self.seal_action(action, report);
            } else {
                strip_unreachable_action(action, report);
            }
        }
    }

    fn seal_event_handlers(
        &self,
        component: &mut Map<String, Value>,
        field: &str,
        report: &mut Report,
    ) {
        let Some(handlers) = component.get_mut(field).and_then(Value::as_object_mut) else {
            return;
        };
        for actions in handlers.values_mut() {
            let Some(actions) = actions.as_array_mut() else {
                continue;
            };
            for action in actions {
                let Some(action) = action.as_object_mut() else {
                    continue;
                };
                if is_workflow_action(action) {
                    self.seal_action(action, report);
                }
            }
        }
    }

    fn seal_action_bindings(
        &self,
        component: &mut Map<String, Value>,
        field: &str,
        report: &mut Report,
    ) {
        let mut shadowed_bindings = HashSet::new();
        let mut wildcard_handler = false;
        for handler_field in ["eventHandlers", "event_handlers"] {
            let Some(handlers) = component.get(handler_field).and_then(Value::as_object) else {
                continue;
            };
            wildcard_handler |= handlers.contains_key("*");
            shadowed_bindings.extend(handlers.keys().cloned());
        }
        let Some(bindings) = component.get_mut(field).and_then(Value::as_object_mut) else {
            return;
        };
        for (binding_id, binding) in bindings.iter_mut() {
            let Some(binding) = binding.as_object_mut() else {
                continue;
            };
            if workflow_binding(binding).is_none() {
                continue;
            }
            if wildcard_handler || shadowed_bindings.contains(binding_id) {
                binding.remove("pageAction");
                binding.remove("page_action");
                strip_binding_routing(binding);
                report.rejected += 1;
            } else {
                self.seal_binding(binding, report);
            }
        }
    }

    fn seal_action(&self, action: &mut Map<String, Value>, report: &mut Report) {
        let (target_node_id, target_app_id, target_board_id) = action
            .get("context")
            .and_then(Value::as_object)
            .map(|context| {
                (
                    bound_string(
                        context
                            .get("nodeId")
                            .or_else(|| context.get("node_id"))
                            .or_else(|| context.get("flowId"))
                            .or_else(|| context.get("flow_id"))
                            .or_else(|| context.get("eventId"))
                            .or_else(|| context.get("event_id")),
                    ),
                    bound_string(context.get("appId").or_else(|| context.get("app_id"))),
                    bound_string(context.get("boardId").or_else(|| context.get("board_id"))),
                )
            })
            .unwrap_or_default();

        action.remove("pageAction");
        action.remove("page_action");
        strip_action_routing(action);

        let Some(target_node_id) = self.allowed_target(
            target_node_id.as_deref(),
            target_app_id.as_deref(),
            target_board_id.as_deref(),
        ) else {
            report.rejected += 1;
            return;
        };

        match self.registry.issue(&self.run, target_node_id) {
            Ok((action_id, capability)) => {
                action.insert(
                    "pageAction".to_string(),
                    serde_json::json!({
                        "actionId": action_id,
                        "manifestRevision": self.scope.manifest_revision,
                        "capabilityJwt": capability,
                    }),
                );
                report.sealed += 1;
            }
            Err(error) => {
                tracing::error!(error = %error, "failed to register local dynamic Page action");
                report.rejected += 1;
            }
        }
    }

    fn seal_binding(&self, binding: &mut Map<String, Value>, report: &mut Report) {
        let (target_node_id, target_app_id, target_board_id) = workflow_binding(binding)
            .map(|workflow| {
                (
                    bound_string(
                        workflow
                            .get("flowId")
                            .or_else(|| workflow.get("flow_id"))
                            .or_else(|| workflow.get("eventId"))
                            .or_else(|| workflow.get("event_id"))
                            .or_else(|| workflow.get("nodeId"))
                            .or_else(|| workflow.get("node_id")),
                    ),
                    bound_string(workflow.get("appId").or_else(|| workflow.get("app_id"))),
                    bound_string(workflow.get("boardId").or_else(|| workflow.get("board_id"))),
                )
            })
            .unwrap_or_default();

        binding.remove("pageAction");
        binding.remove("page_action");
        strip_binding_routing(binding);

        let Some(target_node_id) = self.allowed_target(
            target_node_id.as_deref(),
            target_app_id.as_deref(),
            target_board_id.as_deref(),
        ) else {
            report.rejected += 1;
            return;
        };

        match self.registry.issue(&self.run, target_node_id) {
            Ok((action_id, capability)) => {
                binding.insert(
                    "pageAction".to_string(),
                    serde_json::json!({
                        "actionId": action_id,
                        "manifestRevision": self.scope.manifest_revision,
                        "capabilityJwt": capability,
                    }),
                );
                report.sealed += 1;
            }
            Err(error) => {
                tracing::error!(error = %error, "failed to register local Page widget action");
                report.rejected += 1;
            }
        }
    }

    fn allowed_target<'a>(
        &self,
        node_id: Option<&'a str>,
        app_id: Option<&str>,
        board_id: Option<&str>,
    ) -> Option<&'a str> {
        let node_id = node_id.filter(|id| !id.trim().is_empty() && id.len() <= 128)?;
        if app_id.is_some_and(|id| id != self.scope.app_id) {
            return None;
        }
        if board_id.is_some_and(|id| id != self.scope.board_id) {
            return None;
        }
        self.allowed_entry_nodes
            .contains(node_id)
            .then_some(node_id)
    }
}

fn bounded(value: &Value) -> bool {
    let mut pending = vec![(value, 0)];
    let mut visited = 0;
    while let Some((value, depth)) = pending.pop() {
        visited += 1;
        if depth > 64 || visited > 65_536 {
            return false;
        }
        match value {
            Value::Array(values) => pending.extend(values.iter().map(|value| (value, depth + 1))),
            Value::Object(values) => {
                pending.extend(values.values().map(|value| (value, depth + 1)))
            }
            _ => {}
        }
        if pending.len() > 65_536 {
            return false;
        }
    }
    true
}
fn is_workflow_action(map: &Map<String, Value>) -> bool {
    map.get("name").and_then(Value::as_str) == Some("workflow_event")
}

fn workflow_binding(map: &Map<String, Value>) -> Option<&Map<String, Value>> {
    ["workflow", "workflowEvent", "workflow_event"]
        .iter()
        .find_map(|key| map.get(*key).and_then(Value::as_object))
}

fn bound_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Object(value) => value
            .get("literalString")
            .or_else(|| value.get("literal_string"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        _ => None,
    }
}

fn strip_binding_routing(binding: &mut Map<String, Value>) {
    for binding_key in ["workflow", "workflowEvent", "workflow_event"] {
        let Some(workflow) = binding.get_mut(binding_key).and_then(Value::as_object_mut) else {
            continue;
        };
        for route_key in [
            "flowId",
            "flow_id",
            "eventId",
            "event_id",
            "nodeId",
            "node_id",
            "appId",
            "app_id",
            "boardId",
            "board_id",
            "boardVersion",
            "board_version",
        ] {
            workflow.remove(route_key);
        }
    }
}

fn strip_action_routing(action: &mut Map<String, Value>) {
    let Some(context) = action.get_mut("context").and_then(Value::as_object_mut) else {
        return;
    };
    for key in [
        "nodeId",
        "node_id",
        "flowId",
        "flow_id",
        "eventId",
        "event_id",
        "appId",
        "app_id",
        "boardId",
        "board_id",
        "boardVersion",
        "board_version",
    ] {
        context.remove(key);
    }
}

fn strip_unreachable_action(action: &mut Map<String, Value>, report: &mut Report) {
    action.remove("pageAction");
    action.remove("page_action");
    strip_action_routing(action);
    report.rejected += 1;
}

fn strip_unknown_update_actions(update: &mut Map<String, Value>, report: &mut Report) {
    for key in [
        "actions",
        "eventHandlers",
        "event_handlers",
        "actionBindings",
        "action_bindings",
        "pageAction",
        "page_action",
    ] {
        if update.remove(key).is_some() {
            report.rejected += 1;
        }
    }
}

fn strip_non_widget_bindings(component: &mut Map<String, Value>, report: &mut Report) {
    for field in ["actionBindings", "action_bindings"] {
        let Some(bindings) = component.remove(field) else {
            continue;
        };
        if let Some(bindings) = bindings.as_object() {
            report.rejected += bindings
                .values()
                .filter(|binding| {
                    binding
                        .as_object()
                        .is_some_and(|binding| workflow_binding(binding).is_some())
                })
                .count();
        }
    }
}
