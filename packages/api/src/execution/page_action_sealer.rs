//! Authorize workflow actions before a Page run exposes them to a client.
//!
//! Workflows can create or retarget A2UI actions at runtime. Those messages
//! still contain a raw node id when they leave the executor. The API owns the
//! signing key, so the last trusted delivery boundary replaces every raw
//! workflow target with a short-lived Page-action capability. The capability
//! remains request data; it is never installed as the caller's identity.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::{PageActionJwtParams, sign_page_action_capability};

pub const DYNAMIC_PAGE_ACTION_ID_PREFIX: &str = "da1_";

#[derive(Debug, Clone)]
pub struct PageActionSealingContext {
    pub sub: String,
    pub technical_user_id: Option<String>,
    pub source_app_id: String,
    pub source_event_id: String,
    pub source_page_id: String,
    pub source_manifest_revision: String,
    pub target_app_id: String,
    pub target_board_id: String,
    /// Exact Page board version for a pinned Event.
    pub target_board_version: Option<(u32, u32, u32)>,
    /// Exact source object identity for a floating Latest Event.
    pub target_board_etag: Option<String>,
    pub wasm_authority_revision: Option<String>,
    pub origin_run_id: String,
    /// Only entry nodes from the immutable Page board may become callbacks.
    pub allowed_entry_nodes: HashSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageActionSealingReport {
    pub sealed: usize,
    pub rejected: usize,
}

impl PageActionSealingContext {
    /// Decorate executable actions in one typed A2UI or chat-widget payload.
    ///
    /// `message_id` is the stable executor event id when one exists. The
    /// structural path keeps multiple workflow actions on the same trigger
    /// independent and preserves their array order.
    pub fn seal_payload(
        &self,
        event_type: &str,
        message_id: &str,
        payload: &mut Value,
    ) -> PageActionSealingReport {
        let mut report = PageActionSealingReport::default();
        let mut path = Vec::new();
        match event_type {
            "a2ui" => self.seal_a2ui_message(message_id, payload, &mut path, &mut report),
            "chat_stream_partial" | "chat_stream" | "chat_out" => {
                self.seal_chat_widgets(message_id, payload, &mut path, &mut report);
            }
            _ => {}
        }
        report
    }

    /// Chat events embed self-contained A2UI widget instances without using
    /// the A2UI event channel. Only their declared component and replay-update
    /// fields are executable; the rest of the chat response is application
    /// data.
    fn seal_chat_widgets(
        &self,
        message_id: &str,
        payload: &mut Value,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
        let Some(widgets) = payload
            .as_object_mut()
            .and_then(|payload| payload.get_mut("widgets"))
            .and_then(Value::as_array_mut)
        else {
            return;
        };

        path.push("widgets".to_string());
        for (widget_index, widget) in widgets.iter_mut().enumerate() {
            let Some(widget) = widget.as_object_mut() else {
                continue;
            };
            path.push(widget_index.to_string());
            if let Some(component) = widget.get_mut("component") {
                path.push("component".to_string());
                self.seal_component(message_id, component, path, report);
                path.pop();
            }
            if let Some(updates) = widget.get_mut("updates").and_then(Value::as_array_mut) {
                path.push("updates".to_string());
                for (update_index, update) in updates.iter_mut().enumerate() {
                    path.push(update_index.to_string());
                    self.seal_a2ui_message(message_id, update, path, report);
                    path.pop();
                }
                path.pop();
            }
            path.pop();
        }
        path.pop();
    }

    fn seal_a2ui_message(
        &self,
        message_id: &str,
        payload: &mut Value,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
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
                self.seal_surface_components(message_id, message, "components", path, report);
            }
            "createElement" => {
                if let Some(component) = message.get_mut("component") {
                    path.push("component".to_string());
                    self.seal_surface_component(message_id, component, path, report);
                    path.pop();
                }
            }
            "upsertElement" => {
                if let Some(update) = message.get_mut("value") {
                    path.push("value".to_string());
                    self.seal_element_update(message_id, update, path, report);
                    path.pop();
                }
            }
            // DataModelUpdate, state updates, widget-query args, and every
            // other message variant carry application data, not actions.
            _ => {}
        }
    }

    fn seal_surface_components(
        &self,
        message_id: &str,
        owner: &mut Map<String, Value>,
        field: &str,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
        let Some(components) = owner.get_mut(field).and_then(Value::as_array_mut) else {
            return;
        };
        path.push(field.to_string());
        for (index, component) in components.iter_mut().enumerate() {
            path.push(index.to_string());
            self.seal_surface_component(message_id, component, path, report);
            path.pop();
        }
        path.pop();
    }

    fn seal_surface_component(
        &self,
        message_id: &str,
        value: &mut Value,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
        let Some(surface_component) = value.as_object_mut() else {
            return;
        };
        if let Some(component) = surface_component.get_mut("component") {
            path.push("component".to_string());
            self.seal_component(message_id, component, path, report);
            path.pop();
        }
    }

    fn seal_component(
        &self,
        message_id: &str,
        value: &mut Value,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
        let Some(component) = value.as_object_mut() else {
            return;
        };

        self.seal_legacy_actions(message_id, component, path, report);
        for field in ["eventHandlers", "event_handlers"] {
            self.seal_event_handlers(message_id, component, field, path, report);
        }
        let is_widget = component
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "widgetInstance" | "microWidgetInstance"));
        if is_widget {
            for field in ["actionBindings", "action_bindings"] {
                self.seal_action_bindings(message_id, component, field, path, report);
            }

            for field in ["inlineWidgetDef", "inline_widget_def"] {
                if let Some(definition) = component.get_mut(field).and_then(Value::as_object_mut) {
                    path.push(field.to_string());
                    self.seal_surface_components(
                        message_id,
                        definition,
                        "components",
                        path,
                        report,
                    );
                    path.pop();
                }
            }

            for field in ["runtimeChildUpdates", "runtime_child_updates"] {
                let Some(updates) = component.get_mut(field).and_then(Value::as_object_mut) else {
                    continue;
                };
                path.push(field.to_string());
                for (component_id, operations) in updates.iter_mut() {
                    let Some(operations) = operations.as_array_mut() else {
                        continue;
                    };
                    path.push(component_id.clone());
                    for (index, operation) in operations.iter_mut().enumerate() {
                        path.push(index.to_string());
                        self.seal_element_update(message_id, operation, path, report);
                        path.pop();
                    }
                    path.pop();
                }
                path.pop();
            }
        } else {
            // Only widget instances route bindings; on any other component
            // they are unreachable and must not survive the boundary.
            strip_non_widget_bindings(component, report);
        }
    }

    fn seal_element_update(
        &self,
        message_id: &str,
        value: &mut Value,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
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
                    path.push("action".to_string());
                    self.seal_action(message_id, action, path, report);
                    path.pop();
                }
            }
            Some("setEventActions") => {
                if update
                    .get("eventName")
                    .and_then(Value::as_str)
                    .is_some_and(|event_name| !event_name.trim().is_empty())
                {
                    self.seal_action_array(message_id, update, "actions", path, report);
                }
            }
            Some("createComponent") => {
                if let Some(component) = update.get_mut("component") {
                    path.push("component".to_string());
                    self.seal_component(message_id, component, path, report);
                    path.pop();
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
                    path.push("props".to_string());
                    self.seal_component(message_id, props, path, report);
                    path.pop();
                }
            }
            // The renderer's fallback spreads every field of an unrecognized
            // update into component data. Strip anything executable; an
            // unknown op never mints.
            _ => strip_unknown_update_actions(update, report),
        }
    }

    fn seal_action_array(
        &self,
        message_id: &str,
        owner: &mut Map<String, Value>,
        field: &str,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
        let Some(actions) = owner.get_mut(field).and_then(Value::as_array_mut) else {
            return;
        };
        path.push(field.to_string());
        for (index, action) in actions.iter_mut().enumerate() {
            let Some(action) = action.as_object_mut() else {
                continue;
            };
            if !is_workflow_action(action) {
                continue;
            }
            path.push(index.to_string());
            self.seal_action(message_id, action, path, report);
            path.pop();
        }
        path.pop();
    }

    fn seal_legacy_actions(
        &self,
        message_id: &str,
        component: &mut Map<String, Value>,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
        let Some(actions) = component.get_mut("actions").and_then(Value::as_array_mut) else {
            return;
        };
        path.push("actions".to_string());
        for (index, action) in actions.iter_mut().enumerate() {
            let Some(action) = action.as_object_mut() else {
                continue;
            };
            if !is_workflow_action(action) {
                continue;
            }
            path.push(index.to_string());
            // The renderer's legacy fallback executes only actions[0]. Named
            // eventHandlers and setEventActions retain ordered multi-action
            // semantics through seal_action_array.
            if index == 0 {
                self.seal_action(message_id, action, path, report);
            } else {
                strip_unreachable_action(action, report);
            }
            path.pop();
        }
        path.pop();
    }

    fn seal_event_handlers(
        &self,
        message_id: &str,
        component: &mut Map<String, Value>,
        field: &str,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
        let Some(handlers) = component.get_mut(field).and_then(Value::as_object_mut) else {
            return;
        };
        path.push(field.to_string());
        for (event_name, actions) in handlers.iter_mut() {
            let Some(actions) = actions.as_array_mut() else {
                continue;
            };
            path.push(event_name.clone());
            for (index, action) in actions.iter_mut().enumerate() {
                let Some(action) = action.as_object_mut() else {
                    continue;
                };
                if !is_workflow_action(action) {
                    continue;
                }
                path.push(index.to_string());
                self.seal_action(message_id, action, path, report);
                path.pop();
            }
            path.pop();
        }
        path.pop();
    }

    fn seal_action_bindings(
        &self,
        message_id: &str,
        component: &mut Map<String, Value>,
        field: &str,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
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
        path.push(field.to_string());
        for (binding_id, binding) in bindings.iter_mut() {
            let Some(binding) = binding.as_object_mut() else {
                continue;
            };
            if workflow_binding(binding).is_none() {
                continue;
            }
            path.push(binding_id.clone());
            if wildcard_handler || shadowed_bindings.contains(binding_id) {
                binding.remove("pageAction");
                binding.remove("page_action");
                strip_binding_routing(binding);
                report.rejected += 1;
            } else {
                self.seal_binding(message_id, binding, path, report);
            }
            path.pop();
        }
        path.pop();
    }

    fn seal_action(
        &self,
        message_id: &str,
        action: &mut Map<String, Value>,
        path: &[String],
        report: &mut PageActionSealingReport,
    ) {
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

        // An action-provided invocation is never trusted. It is replaced only
        // after the raw target has passed this Page run's exact scope checks.
        action.remove("pageAction");
        action.remove("page_action");
        strip_action_routing(action);

        if !self.target_is_allowed(
            target_node_id.as_deref(),
            target_app_id.as_deref(),
            target_board_id.as_deref(),
        ) {
            report.rejected += 1;
            return;
        }

        let target_node_id = target_node_id.expect("allowed target has a node id");
        match self.invocation(message_id, path, &target_node_id) {
            Ok(Some(invocation)) => {
                action.insert("pageAction".to_string(), invocation);
                report.sealed += 1;
            }
            Ok(None) => {
                report.rejected += 1;
            }
            Err(error) => {
                tracing::error!(error = %error, "failed to sign dynamic Page action");
                report.rejected += 1;
            }
        }
    }

    fn seal_binding(
        &self,
        message_id: &str,
        binding: &mut Map<String, Value>,
        path: &[String],
        report: &mut PageActionSealingReport,
    ) {
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

        if !self.target_is_allowed(
            target_node_id.as_deref(),
            target_app_id.as_deref(),
            target_board_id.as_deref(),
        ) {
            report.rejected += 1;
            return;
        }

        let target_node_id = target_node_id.expect("allowed binding has a node id");
        match self.invocation(message_id, path, &target_node_id) {
            Ok(Some(invocation)) => {
                binding.insert("pageAction".to_string(), invocation);
                report.sealed += 1;
            }
            Ok(None) => {
                report.rejected += 1;
            }
            Err(error) => {
                tracing::error!(error = %error, "failed to sign dynamic Page widget binding");
                report.rejected += 1;
            }
        }
    }

    fn target_is_allowed(
        &self,
        node_id: Option<&str>,
        app_id: Option<&str>,
        board_id: Option<&str>,
    ) -> bool {
        let Some(node_id) = node_id.filter(|id| !id.trim().is_empty()) else {
            return false;
        };
        if app_id.is_some_and(|id| id != self.target_app_id) {
            return false;
        }
        if board_id.is_some_and(|id| id != self.target_board_id) {
            return false;
        }
        self.allowed_entry_nodes.contains(node_id)
    }

    fn invocation(
        &self,
        message_id: &str,
        path: &[String],
        target_node_id: &str,
    ) -> Result<Option<Value>, super::PageActionJwtError> {
        let has_board_etag = self
            .target_board_etag
            .as_deref()
            .is_some_and(|etag| !etag.trim().is_empty());
        match (self.target_board_version.is_some(), has_board_etag) {
            (true, false) | (false, true) => {}
            (true, true) | (false, false) => return Ok(None),
        }
        let origin_locator = if path.is_empty() {
            "$".to_string()
        } else {
            path.join("/")
        };
        let mut hasher = blake3::Hasher::new();
        for part in [
            self.origin_run_id.as_str(),
            message_id,
            origin_locator.as_str(),
            target_node_id,
        ] {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
        let action_id = format!(
            "{DYNAMIC_PAGE_ACTION_ID_PREFIX}{}",
            hasher.finalize().to_hex()
        );
        let capability_jwt = sign_page_action_capability(PageActionJwtParams {
            sub: self.sub.clone(),
            technical_user_id: self.technical_user_id.clone(),
            source_app_id: self.source_app_id.clone(),
            source_event_id: self.source_event_id.clone(),
            source_page_id: self.source_page_id.clone(),
            source_manifest_revision: self.source_manifest_revision.clone(),
            target_app_id: self.target_app_id.clone(),
            target_board_id: self.target_board_id.clone(),
            target_board_version: self.target_board_version,
            target_board_etag: self.target_board_etag.clone(),
            target_wasm_authority_revision: self.wasm_authority_revision.clone(),
            target_node_id: target_node_id.to_string(),
            action_id: action_id.clone(),
            origin_run_id: self.origin_run_id.clone(),
            origin_locator,
            ttl_seconds: None,
        })?;

        Ok(Some(serde_json::json!({
            "actionId": action_id,
            "capabilityJwt": capability_jwt,
            "manifestRevision": self.source_manifest_revision,
        })))
    }
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

fn strip_unreachable_action(action: &mut Map<String, Value>, report: &mut PageActionSealingReport) {
    action.remove("pageAction");
    action.remove("page_action");
    strip_action_routing(action);
    report.rejected += 1;
}

fn strip_unknown_update_actions(
    update: &mut Map<String, Value>,
    report: &mut PageActionSealingReport,
) {
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

fn strip_non_widget_bindings(
    component: &mut Map<String, Value>,
    report: &mut PageActionSealingReport,
) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> PageActionSealingContext {
        crate::backend_jwt::init_for_tests();
        PageActionSealingContext {
            sub: "user-1".into(),
            technical_user_id: None,
            source_app_id: "app-1".into(),
            source_event_id: "event-1".into(),
            source_page_id: "page-1".into(),
            source_manifest_revision: "revision-1".into(),
            target_app_id: "app-1".into(),
            target_board_id: "board-1".into(),
            target_board_version: Some((1, 2, 3)),
            target_board_etag: None,
            wasm_authority_revision: Some("wasm-revision-1".into()),
            origin_run_id: "run-1".into(),
            allowed_entry_nodes: HashSet::from(["entry-1".into(), "entry-2".into()]),
        }
    }

    #[test]
    fn seals_each_ordered_action_and_strips_raw_routing() {
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{
                "id": "button",
                "component": {
                    "eventHandlers": {
                        "click": [
                            {"name": "workflow_event", "context": {"nodeId": "entry-1"}},
                            {"name": "navigate_page", "context": {"route": "/next"}},
                            {"name": "workflow_event", "context": {"nodeId": "entry-2"}}
                        ]
                    }
                }
            }]
        });

        let report = context().seal_payload("a2ui", "message-1", &mut payload);
        assert_eq!(report.sealed, 2);
        assert_eq!(report.rejected, 0);
        let actions = payload["components"][0]["component"]["eventHandlers"]["click"]
            .as_array()
            .unwrap();
        assert!(actions[0]["context"].get("nodeId").is_none());
        assert!(
            actions[0]["pageAction"]["actionId"]
                .as_str()
                .unwrap()
                .starts_with(DYNAMIC_PAGE_ACTION_ID_PREFIX)
        );
        assert!(actions[0]["pageAction"]["capabilityJwt"].is_string());
        assert!(actions[1].get("pageAction").is_none());
        assert_ne!(
            actions[0]["pageAction"]["actionId"],
            actions[2]["pageAction"]["actionId"]
        );
    }

    #[test]
    fn legacy_actions_mint_only_the_renderer_executable_first_item() {
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{"id": "button", "component": {
                "actions": [
                    {"name": "workflow_event", "context": {"nodeId": "entry-1"}},
                    {"name": "workflow_event", "context": {"nodeId": "entry-2"},
                        "pageAction": {"capabilityJwt": "attacker"}}
                ]
            }}]
        });

        let report = context().seal_payload("a2ui", "message-legacy", &mut payload);
        let actions = payload["components"][0]["component"]["actions"]
            .as_array()
            .unwrap();

        assert_eq!(report.sealed, 1);
        assert_eq!(report.rejected, 1);
        assert!(actions[0]["pageAction"]["capabilityJwt"].is_string());
        assert!(actions[1].get("pageAction").is_none());
        assert!(actions[1]["context"].get("nodeId").is_none());
    }

    #[test]
    fn seals_widget_and_micro_widget_bindings_without_exposing_flow_id() {
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{
                "id": "widget",
                "component": {
                    "type": "microWidgetInstance",
                    "eventHandlers": {"blocked": []},
                    "actionBindings": {
                        "approve": {
                            "workflow": {"flowId": "entry-1", "inputMappings": {}}
                        },
                        "blocked": {
                            "workflow": {"flowId": "entry-2", "inputMappings": {}}
                        }
                    }
                }
            }]
        });

        let report = context().seal_payload("a2ui", "message-2", &mut payload);
        assert_eq!(report.sealed, 1);
        assert_eq!(report.rejected, 1);
        let binding = &payload["components"][0]["component"]["actionBindings"]["approve"];
        assert!(binding["workflow"].get("flowId").is_none());
        assert!(binding["pageAction"]["capabilityJwt"].is_string());
        let blocked = &payload["components"][0]["component"]["actionBindings"]["blocked"];
        assert!(blocked["workflow"].get("flowId").is_none());
        assert!(blocked.get("pageAction").is_none());
    }

    #[test]
    fn seals_widgets_embedded_in_each_chat_delivery_shape() {
        for event_type in ["chat_stream_partial", "chat_stream", "chat_out"] {
            let mut payload = serde_json::json!({
                "widgets": [{
                    "instance_id": "widget-1",
                    "component": {
                        "type": "widgetInstance",
                        "actionBindings": {
                            "approve": {"workflow": {"flowId": "entry-1"}}
                        },
                        "inlineWidgetDef": {
                            "components": [{
                                "id": "button",
                                "component": {
                                    "type": "button",
                                    "eventHandlers": {
                                        "click": [{
                                            "name": "workflow_event",
                                            "context": {"nodeId": "entry-2"}
                                        }]
                                    }
                                }
                            }]
                        }
                    },
                    "updates": [{
                        "type": "upsertElement",
                        "element_id": "widget-1/button",
                        "value": {
                            "type": "setEventActions",
                            "eventName": "click",
                            "actions": [{
                                "name": "workflow_event",
                                "context": {"nodeId": "entry-1"}
                            }]
                        }
                    }]
                }]
            });

            let report = context().seal_payload(event_type, "message-chat", &mut payload);

            assert_eq!(report.sealed, 3, "{event_type}");
            assert!(
                payload["widgets"][0]["component"]["actionBindings"]["approve"]
                    ["pageAction"]["capabilityJwt"]
                    .is_string()
            );
            assert!(
                payload["widgets"][0]["component"]["inlineWidgetDef"]["components"][0]["component"]
                    ["eventHandlers"]["click"][0]["pageAction"]["capabilityJwt"]
                    .is_string()
            );
            assert!(
                payload["widgets"][0]["updates"][0]["value"]["actions"][0]["pageAction"]
                    ["capabilityJwt"]
                    .is_string()
            );
        }
    }

    #[test]
    fn rejects_foreign_or_non_entry_targets_fail_closed() {
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{"id": "button", "component": {
                "actions": [
                    {"name": "workflow_event", "context": {
                        "nodeId": "entry-1", "boardId": "other-board"
                    }, "pageAction": {"capabilityJwt": "attacker"}},
                    {"name": "workflow_event", "context": {"nodeId": "not-an-entry"},
                        "page_action": {"capability_jwt": "attacker"}}
                ]
            }}]
        });

        let report = context().seal_payload("a2ui", "message-3", &mut payload);
        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 2);
        for action in payload["components"][0]["component"]["actions"]
            .as_array()
            .unwrap()
        {
            assert!(action.get("pageAction").is_none());
            assert!(action.get("page_action").is_none());
            assert!(action["context"].get("nodeId").is_none());
            assert!(action["context"].get("boardId").is_none());
        }
    }

    #[test]
    fn rejects_foreign_widget_binding_and_strips_every_routing_alias() {
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{"id": "widget", "component": {
                "type": "widgetInstance",
                "actionBindings": {
                    "submit": {
                        "workflow": {
                            "flowId": "entry-1",
                            "appId": "other-app",
                            "inputMappings": {"kept": true}
                        },
                        "workflowEvent": {
                            "eventId": "entry-1",
                            "appId": "other-app",
                            "boardId": "other-board",
                            "contextMapping": {"value": {"literalString": "kept"}}
                        },
                        "workflow_event": {
                            "node_id": "entry-2",
                            "board_version": [9, 9, 9]
                        }
                    }
                }
            }}]
        });

        let report = context().seal_payload("a2ui", "message-widget-foreign", &mut payload);
        let binding = &payload["components"][0]["component"]["actionBindings"]["submit"];

        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 1);
        assert!(binding.get("pageAction").is_none());
        assert!(binding["workflow"].get("flowId").is_none());
        assert!(binding["workflow"].get("appId").is_none());
        assert_eq!(binding["workflow"]["inputMappings"]["kept"], true);
        assert!(binding["workflowEvent"].get("eventId").is_none());
        assert!(binding["workflowEvent"].get("appId").is_none());
        assert!(binding["workflowEvent"].get("boardId").is_none());
        assert_eq!(
            binding["workflowEvent"]["contextMapping"]["value"]["literalString"],
            "kept"
        );
        assert!(binding["workflow_event"].get("node_id").is_none());
        assert!(binding["workflow_event"].get("board_version").is_none());
    }

    #[test]
    fn page_without_an_exact_board_selector_strips_target_without_minting() {
        let mut context = context();
        context.target_board_version = None;
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{"id": "button", "component": {
                "actions": [{
                    "name": "workflow_event",
                    "context": {"nodeId": "entry-1", "input": "kept"}
                }]
            }}]
        });

        let report = context.seal_payload("a2ui", "message-unpinned", &mut payload);
        let action = &payload["components"][0]["component"]["actions"][0];

        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 1);
        assert!(action.get("pageAction").is_none());
        assert!(action["context"].get("nodeId").is_none());
        assert_eq!(action["context"]["input"], "kept");
    }

    #[test]
    fn latest_page_etag_mints_an_etag_bound_capability() {
        let mut context = context();
        context.target_board_version = None;
        context.target_board_etag = Some("etag-latest-1".into());
        let mut payload = serde_json::json!({
            "type": "upsertElement",
            "element_id": "page/button",
            "value": {
                "type": "setAction",
                "action": {
                    "name": "workflow_event",
                    "context": {"nodeId": "entry-1"}
                }
            }
        });

        let report = context.seal_payload("a2ui", "message-latest", &mut payload);
        let capability = payload["value"]["action"]["pageAction"]["capabilityJwt"]
            .as_str()
            .unwrap();
        let claims = crate::execution::verify_page_action_capability(capability).unwrap();

        assert_eq!(report.sealed, 1);
        assert_eq!(claims.target_board_version, None);
        assert_eq!(claims.target_board_etag.as_deref(), Some("etag-latest-1"));
    }

    #[test]
    fn seals_set_action_and_named_set_event_actions() {
        let mut payload = serde_json::json!({
            "type": "upsertElement",
            "element_id": "page/button",
            "value": {
                "type": "setAction",
                "action": {
                    "name": "workflow_event",
                    "context": {"nodeId": "entry-1", "input": "kept"}
                }
            }
        });

        let report = context().seal_payload("a2ui", "message-set-action", &mut payload);
        assert_eq!(report.sealed, 1);
        let action = &payload["value"]["action"];
        assert!(action["context"].get("nodeId").is_none());
        assert_eq!(action["context"]["input"], "kept");
        assert!(action["pageAction"]["capabilityJwt"].is_string());

        payload["value"] = serde_json::json!({
            "type": "setEventActions",
            "eventName": "click",
            "actions": [
                {"name": "workflow_event", "context": {"nodeId": "entry-1"}},
                {"name": "workflow_event", "context": {"nodeId": "entry-2"}}
            ]
        });
        let report = context().seal_payload("a2ui", "message-set-event-actions", &mut payload);
        assert_eq!(report.sealed, 2);
        assert_ne!(
            payload["value"]["actions"][0]["pageAction"]["actionId"],
            payload["value"]["actions"][1]["pageAction"]["actionId"]
        );

        payload["value"] = serde_json::json!({
            "type": "setEventActions",
            "eventName": " ",
            "actions": [
                {"name": "workflow_event", "context": {"nodeId": "entry-1"}}
            ]
        });
        let original = payload.clone();
        assert_eq!(
            context().seal_payload("a2ui", "message-unnamed-event", &mut payload),
            PageActionSealingReport::default()
        );
        assert_eq!(payload, original);
    }

    #[test]
    fn set_props_content_is_sealed_like_a_component_body() {
        let mut payload = serde_json::json!({
            "type": "upsertElement",
            "element_id": "page/button",
            "value": {
                "type": "setProps",
                "props": {
                    "pageAction": {"capabilityJwt": "attacker"},
                    "label": {"literalString": "kept"},
                    "eventHandlers": {
                        "click": [
                            {"name": "workflow_event",
                                "context": {"nodeId": "entry-1", "input": "kept"}},
                            {"name": "workflow_event",
                                "context": {"nodeId": "not-an-entry", "boardId": "other-board"}}
                        ]
                    },
                    "actionBindings": {
                        "approve": {"workflow": {"flowId": "entry-1"}}
                    }
                }
            }
        });

        let report = context().seal_payload("a2ui", "message-set-props", &mut payload);
        assert_eq!(report.sealed, 1);
        assert_eq!(report.rejected, 2);
        let props = &payload["value"]["props"];
        assert!(props.get("pageAction").is_none());
        assert_eq!(props["label"]["literalString"], "kept");
        assert!(props.get("actionBindings").is_none());
        let actions = props["eventHandlers"]["click"].as_array().unwrap();
        assert!(
            actions[0]["pageAction"]["actionId"]
                .as_str()
                .unwrap()
                .starts_with(DYNAMIC_PAGE_ACTION_ID_PREFIX)
        );
        assert!(actions[0]["pageAction"]["capabilityJwt"].is_string());
        assert!(actions[0]["context"].get("nodeId").is_none());
        assert_eq!(actions[0]["context"]["input"], "kept");
        assert!(actions[1].get("pageAction").is_none());
        assert!(actions[1]["context"].get("nodeId").is_none());
        assert!(actions[1]["context"].get("boardId").is_none());
    }

    #[test]
    fn unknown_update_ops_never_pass_executable_fields_through() {
        let mut payload = serde_json::json!({
            "type": "upsertElement",
            "element_id": "page/panel",
            "value": {
                "type": "setCustomState",
                "state": {"literalString": "kept"},
                "actions": [{"name": "workflow_event", "context": {"nodeId": "entry-1"}}],
                "eventHandlers": {"click": [
                    {"name": "workflow_event", "context": {"nodeId": "entry-1"}}
                ]},
                "actionBindings": {"approve": {"workflow": {"flowId": "entry-1"}}},
                "pageAction": {"capabilityJwt": "attacker"}
            }
        });

        let report = context().seal_payload("a2ui", "message-unknown-op", &mut payload);
        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 4);
        let update = payload["value"].as_object().unwrap();
        for key in ["actions", "eventHandlers", "actionBindings", "pageAction"] {
            assert!(update.get(key).is_none(), "{key} must be stripped");
        }
        assert_eq!(update["type"], "setCustomState");
        assert_eq!(update["state"]["literalString"], "kept");

        payload["value"] = serde_json::json!({
            "props": {"literalString": "kept"},
            "event_handlers": {"click": [
                {"name": "workflow_event", "context": {"nodeId": "entry-1"}}
            ]}
        });
        let report = context().seal_payload("a2ui", "message-untyped-op", &mut payload);
        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 1);
        assert!(payload["value"].get("event_handlers").is_none());
        assert_eq!(payload["value"]["props"]["literalString"], "kept");
    }

    #[test]
    fn non_widget_components_never_carry_action_bindings() {
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [
                {"id": "button", "component": {
                    "type": "button",
                    "actionBindings": {
                        "approve": {"workflow": {"flowId": "entry-1"}}
                    },
                    "label": {"literalString": "kept"}
                }},
                {"id": "record", "component": {
                    "action_bindings": {
                        "submit": {"workflow": {"flowId": "not-an-entry"}},
                        "noise": {"config": {"literalString": "kept"}}
                    }
                }}
            ]
        });

        let report = context().seal_payload("a2ui", "message-non-widget", &mut payload);
        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 2);
        let button = &payload["components"][0]["component"];
        assert!(button.get("actionBindings").is_none());
        assert_eq!(button["label"]["literalString"], "kept");
        let record = &payload["components"][1]["component"];
        assert!(record.get("action_bindings").is_none());
    }

    #[test]
    fn generic_results_and_literal_json_are_not_capability_sources() {
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{"id": "record", "component": {
                "value": {
                    "literalJson": "{\"actions\":[{\"name\":\"workflow_event\",\"context\":{\"nodeId\":\"entry-1\"}}]}"
                }
            }}],
            "widgets": [{
                "component": {
                    "type": "microWidgetInstance",
                    "actionBindings": {
                        "submit": {"workflow": {"flowId": "entry-2"}}
                    }
                }
            }]
        });
        let original = payload.clone();

        let report = context().seal_payload("generic_result", "message-generic", &mut payload);

        assert_eq!(report, PageActionSealingReport::default());
        assert_eq!(payload, original);

        let report = context().seal_payload("a2ui", "message-literal", &mut payload);
        assert_eq!(report, PageActionSealingReport::default());
        assert_eq!(payload, original);
    }

    #[test]
    fn data_model_update_application_values_are_untouched() {
        let mut payload = serde_json::json!({
            "type": "dataModelUpdate",
            "surface_id": "page",
            "contents": [{
                "key": "records",
                "value": {"actions": [{
                    "name": "workflow_event",
                    "context": {"nodeId": "entry-1"}
                }]}
            }]
        });
        let original = payload.clone();

        let report = context().seal_payload("a2ui", "message-data", &mut payload);

        assert_eq!(report, PageActionSealingReport::default());
        assert_eq!(payload, original);
    }
}
