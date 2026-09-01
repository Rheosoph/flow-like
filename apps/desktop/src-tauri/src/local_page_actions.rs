//! Native capabilities for workflow actions created by a locally run Page.
//!
//! JavaScript receives only an opaque lookup id. The target node and every
//! part of the Page execution scope stay in this process, expire, and are
//! checked again after the caller's ordinary authorization is refreshed.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use flow_like::flow::execution::{ExecutionPrincipal, UserExecutionContext};
use flow_like_types::create_id;
use serde_json::{Map, Value};

pub(crate) const LOCAL_DYNAMIC_PAGE_ACTION_ID_PREFIX: &str = "lda1_";

const LOCAL_DYNAMIC_PAGE_ACTION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

static LOCAL_DYNAMIC_PAGE_ACTIONS: LazyLock<Arc<LocalDynamicPageActionRegistry>> =
    LazyLock::new(|| {
        Arc::new(LocalDynamicPageActionRegistry::new(
            LOCAL_DYNAMIC_PAGE_ACTION_TTL,
        ))
    });

/// Stable caller binding used by a local Page run and its later callbacks.
///
/// Hosted callers use a one-way token fingerprint. The token is still checked
/// with the hub on every invocation, and neither the token nor its digest is
/// exposed to JavaScript. Offline apps bind to their device-owned app id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalPagePrincipalBinding(String);

impl LocalPagePrincipalBinding {
    pub(crate) fn authenticated(
        context: &UserExecutionContext,
        fallback_token: &str,
        authority: &str,
    ) -> Self {
        let identity = match context.principal {
            ExecutionPrincipal::User if !context.sub.is_empty() => {
                Some(("user", context.sub.as_str()))
            }
            ExecutionPrincipal::ApiKey => context.key_id.as_deref().map(|id| ("api-key", id)),
            ExecutionPrincipal::ConnectedApp => context
                .origin_app_id
                .as_deref()
                .map(|id| ("connected-app", id)),
            _ => None,
        };
        match identity {
            Some((kind, id)) => Self(fingerprint(&["authenticated", authority, kind, id])),
            None => Self::hosted(fallback_token, authority),
        }
    }

    pub(crate) fn hosted(token: &str, authority: &str) -> Self {
        Self(fingerprint(&["hosted", authority, token]))
    }

    pub(crate) fn offline_owner(app_id: &str) -> Self {
        Self(fingerprint(&["offline-owner", app_id]))
    }
}

/// Exact authority scope for one locally rendered Page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalPageActionScope {
    pub app_id: String,
    pub event_id: String,
    pub page_id: String,
    pub board_id: String,
    pub board_version: (u32, u32, u32),
    pub manifest_revision: String,
    pub principal: LocalPagePrincipalBinding,
}

#[derive(Clone, Debug)]
struct LocalDynamicPageActionGrant {
    scope: LocalPageActionScope,
    target_node_id: String,
    expires_at: Instant,
}

#[derive(Debug)]
struct LocalDynamicPageActionRegistry {
    grants: Mutex<HashMap<String, LocalDynamicPageActionGrant>>,
    ttl: Duration,
}

impl LocalDynamicPageActionRegistry {
    fn new(ttl: Duration) -> Self {
        Self {
            grants: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    fn insert(
        &self,
        scope: &LocalPageActionScope,
        target_node_id: &str,
    ) -> flow_like_types::Result<String> {
        let expires_at = Instant::now()
            .checked_add(self.ttl)
            .ok_or_else(|| flow_like_types::anyhow!("Local Page action expiry overflowed"))?;
        self.insert_with_expiry(scope, target_node_id, expires_at)
    }

    fn insert_with_expiry(
        &self,
        scope: &LocalPageActionScope,
        target_node_id: &str,
        expires_at: Instant,
    ) -> flow_like_types::Result<String> {
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| flow_like_types::anyhow!("Local Page action registry is unavailable"))?;
        let now = Instant::now();
        grants.retain(|_, grant| grant.expires_at > now);

        loop {
            let action_id = format!("{LOCAL_DYNAMIC_PAGE_ACTION_ID_PREFIX}{}", create_id());
            if grants.contains_key(&action_id) {
                continue;
            }
            grants.insert(
                action_id.clone(),
                LocalDynamicPageActionGrant {
                    scope: scope.clone(),
                    target_node_id: target_node_id.to_string(),
                    expires_at,
                },
            );
            return Ok(action_id);
        }
    }

    fn resolve(
        &self,
        action_id: &str,
        expected_scope: &LocalPageActionScope,
    ) -> flow_like_types::Result<String> {
        if !action_id.starts_with(LOCAL_DYNAMIC_PAGE_ACTION_ID_PREFIX) {
            return Err(flow_like_types::anyhow!(
                "The local Page action id is invalid"
            ));
        }

        let mut grants = self
            .grants
            .lock()
            .map_err(|_| flow_like_types::anyhow!("Local Page action registry is unavailable"))?;
        let now = Instant::now();
        let expired = grants
            .get(action_id)
            .is_some_and(|grant| grant.expires_at <= now);
        if expired {
            grants.remove(action_id);
            return Err(flow_like_types::anyhow!(
                "The local Page action expired; reload the Page"
            ));
        }

        let grant = grants.get(action_id).ok_or_else(|| {
            flow_like_types::anyhow!("The local Page action is unknown; reload the Page")
        })?;
        if grant.scope != *expected_scope {
            return Err(flow_like_types::anyhow!(
                "The local Page action does not belong to this Page execution"
            ));
        }
        Ok(grant.target_node_id.clone())
    }
}

pub(crate) fn resolve_local_dynamic_page_action(
    action_id: &str,
    expected_scope: &LocalPageActionScope,
) -> flow_like_types::Result<String> {
    LOCAL_DYNAMIC_PAGE_ACTIONS.resolve(action_id, expected_scope)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LocalPageActionSealingReport {
    pub sealed: usize,
    pub rejected: usize,
}

/// Rewrites executable A2UI slots before a local executor event reaches JS.
#[derive(Clone, Debug)]
pub(crate) struct LocalPageActionSealingContext {
    scope: LocalPageActionScope,
    allowed_entry_nodes: HashSet<String>,
    registry: Arc<LocalDynamicPageActionRegistry>,
}

impl LocalPageActionSealingContext {
    pub(crate) fn new(scope: LocalPageActionScope, allowed_entry_nodes: HashSet<String>) -> Self {
        Self {
            scope,
            allowed_entry_nodes,
            registry: LOCAL_DYNAMIC_PAGE_ACTIONS.clone(),
        }
    }

    #[cfg(test)]
    fn with_registry(
        scope: LocalPageActionScope,
        allowed_entry_nodes: HashSet<String>,
        registry: Arc<LocalDynamicPageActionRegistry>,
    ) -> Self {
        Self {
            scope,
            allowed_entry_nodes,
            registry,
        }
    }

    pub(crate) fn seal_payload(
        &self,
        event_type: &str,
        payload: &mut Value,
    ) -> LocalPageActionSealingReport {
        let mut report = LocalPageActionSealingReport::default();
        match event_type {
            "a2ui" => self.seal_a2ui_message(payload, &mut report),
            "chat_stream_partial" | "chat_stream" | "chat_out" => {
                self.seal_chat_widgets(payload, &mut report);
            }
            _ => {}
        }
        report
    }

    fn seal_chat_widgets(&self, payload: &mut Value, report: &mut LocalPageActionSealingReport) {
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

    fn seal_a2ui_message(&self, payload: &mut Value, report: &mut LocalPageActionSealingReport) {
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
        report: &mut LocalPageActionSealingReport,
    ) {
        let Some(components) = owner.get_mut(field).and_then(Value::as_array_mut) else {
            return;
        };
        for component in components {
            self.seal_surface_component(component, report);
        }
    }

    fn seal_surface_component(&self, value: &mut Value, report: &mut LocalPageActionSealingReport) {
        let Some(surface_component) = value.as_object_mut() else {
            return;
        };
        if let Some(component) = surface_component.get_mut("component") {
            self.seal_component(component, report);
        }
    }

    fn seal_component(&self, value: &mut Value, report: &mut LocalPageActionSealingReport) {
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

    fn seal_element_update(&self, value: &mut Value, report: &mut LocalPageActionSealingReport) {
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

    fn seal_action_array(
        &self,
        owner: &mut Map<String, Value>,
        field: &str,
        report: &mut LocalPageActionSealingReport,
    ) {
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

    fn seal_legacy_actions(
        &self,
        component: &mut Map<String, Value>,
        report: &mut LocalPageActionSealingReport,
    ) {
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
        report: &mut LocalPageActionSealingReport,
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
        report: &mut LocalPageActionSealingReport,
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

    fn seal_action(
        &self,
        action: &mut Map<String, Value>,
        report: &mut LocalPageActionSealingReport,
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

        match self.registry.insert(&self.scope, target_node_id) {
            Ok(action_id) => {
                action.insert(
                    "pageAction".to_string(),
                    serde_json::json!({
                        "actionId": action_id,
                        "manifestRevision": self.scope.manifest_revision,
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

    fn seal_binding(
        &self,
        binding: &mut Map<String, Value>,
        report: &mut LocalPageActionSealingReport,
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

        let Some(target_node_id) = self.allowed_target(
            target_node_id.as_deref(),
            target_app_id.as_deref(),
            target_board_id.as_deref(),
        ) else {
            report.rejected += 1;
            return;
        };

        match self.registry.insert(&self.scope, target_node_id) {
            Ok(action_id) => {
                binding.insert(
                    "pageAction".to_string(),
                    serde_json::json!({
                        "actionId": action_id,
                        "manifestRevision": self.scope.manifest_revision,
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
        let node_id = node_id.filter(|id| !id.trim().is_empty())?;
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

fn fingerprint(parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
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

fn strip_unreachable_action(
    action: &mut Map<String, Value>,
    report: &mut LocalPageActionSealingReport,
) {
    action.remove("pageAction");
    action.remove("page_action");
    strip_action_routing(action);
    report.rejected += 1;
}

fn strip_unknown_update_actions(
    update: &mut Map<String, Value>,
    report: &mut LocalPageActionSealingReport,
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
    report: &mut LocalPageActionSealingReport,
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

    fn scope() -> LocalPageActionScope {
        LocalPageActionScope {
            app_id: "app-1".into(),
            event_id: "event-1".into(),
            page_id: "page-1".into(),
            board_id: "board-1".into(),
            board_version: (1, 2, 3),
            manifest_revision: "per2-current".into(),
            principal: LocalPagePrincipalBinding::authenticated(
                &UserExecutionContext::new("user-1"),
                "token-1",
                "hub-1",
            ),
        }
    }

    fn context() -> (
        LocalPageActionSealingContext,
        Arc<LocalDynamicPageActionRegistry>,
    ) {
        let registry = Arc::new(LocalDynamicPageActionRegistry::new(Duration::from_secs(60)));
        (
            LocalPageActionSealingContext::with_registry(
                scope(),
                HashSet::from(["entry-1".into(), "entry-2".into()]),
                registry.clone(),
            ),
            registry,
        )
    }

    #[test]
    fn seals_ordered_actions_and_resolves_only_from_the_registry() {
        let (context, registry) = context();
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{"id": "button", "component": {
                "eventHandlers": {
                    "click": [
                        {"name": "workflow_event", "context": {"nodeId": "entry-1", "input": "kept"}},
                        {"name": "navigate_page", "context": {"route": "/next"}},
                        {"name": "workflow_event", "context": {"nodeId": "entry-2"}}
                    ]
                }
            }}]
        });

        let report = context.seal_payload("a2ui", &mut payload);
        assert_eq!(report.sealed, 2);
        assert_eq!(report.rejected, 0);
        let actions = payload["components"][0]["component"]["eventHandlers"]["click"]
            .as_array()
            .unwrap();
        assert!(actions[0]["context"].get("nodeId").is_none());
        assert_eq!(actions[0]["context"]["input"], "kept");
        assert!(actions[0]["pageAction"].get("capabilityJwt").is_none());
        assert_ne!(
            actions[0]["pageAction"]["actionId"],
            actions[2]["pageAction"]["actionId"]
        );

        let first_id = actions[0]["pageAction"]["actionId"].as_str().unwrap();
        assert!(first_id.starts_with(LOCAL_DYNAMIC_PAGE_ACTION_ID_PREFIX));
        assert_eq!(registry.resolve(first_id, &scope()).unwrap(), "entry-1");
    }

    #[test]
    fn legacy_actions_mint_only_the_renderer_executable_first_item() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{"id": "button", "component": {
                "actions": [
                    {"name": "workflow_event", "context": {"nodeId": "entry-1"}},
                    {"name": "workflow_event", "context": {"nodeId": "entry-2"},
                        "pageAction": {"actionId": "attacker"}}
                ]
            }}]
        });

        let report = context.seal_payload("a2ui", &mut payload);
        let actions = payload["components"][0]["component"]["actions"]
            .as_array()
            .unwrap();

        assert_eq!(report.sealed, 1);
        assert_eq!(report.rejected, 1);
        assert!(actions[0]["pageAction"]["actionId"].is_string());
        assert!(actions[1].get("pageAction").is_none());
        assert!(actions[1]["context"].get("nodeId").is_none());
    }

    #[test]
    fn seals_widget_bindings_but_not_literal_json_application_data() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{"id": "widget", "component": {
                "type": "widgetInstance",
                "eventHandlers": {"blocked": []},
                "actionBindings": {
                    "approve": {
                        "workflow": {"flowId": "entry-1", "inputMappings": {"value": "kept"}},
                        "workflowEvent": {"eventId": "entry-2", "boardId": "board-1"},
                        "workflow_event": {"node_id": "entry-2", "board_version": [1, 2, 3]}
                    },
                    "blocked": {
                        "workflow": {"flowId": "entry-2", "inputMappings": {}}
                    }
                },
                "data": {
                    "literalJson": "{\"actions\":[{\"name\":\"workflow_event\",\"context\":{\"nodeId\":\"entry-2\"}}]}"
                }
            }}]
        });

        let report = context.seal_payload("a2ui", &mut payload);
        assert_eq!(report.sealed, 1);
        assert_eq!(report.rejected, 1);
        let binding = &payload["components"][0]["component"]["actionBindings"]["approve"];
        assert!(binding["workflow"].get("flowId").is_none());
        assert_eq!(binding["workflow"]["inputMappings"]["value"], "kept");
        assert!(binding["workflowEvent"].get("eventId").is_none());
        assert!(binding["workflowEvent"].get("boardId").is_none());
        assert!(binding["workflow_event"].get("node_id").is_none());
        assert!(binding["workflow_event"].get("board_version").is_none());
        assert!(binding["pageAction"]["actionId"].is_string());
        let blocked = &payload["components"][0]["component"]["actionBindings"]["blocked"];
        assert!(blocked["workflow"].get("flowId").is_none());
        assert!(blocked.get("pageAction").is_none());

        let embedded: Value = serde_json::from_str(
            payload["components"][0]["component"]["data"]["literalJson"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert!(embedded["actions"][0].get("pageAction").is_none());
        assert_eq!(embedded["actions"][0]["context"]["nodeId"], "entry-2");
    }

    #[test]
    fn seals_chat_widget_components_and_replay_updates() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "widgets": [{
                "instance_id": "widget-1",
                "component": {
                    "type": "microWidgetInstance",
                    "actionBindings": {
                        "submit": {"workflow": {"flowId": "entry-1"}}
                    }
                },
                "updates": [{
                    "type": "upsertElement",
                    "element_id": "widget-1/button",
                    "value": {
                        "type": "setAction",
                        "action": {
                            "name": "workflow_event",
                            "context": {"nodeId": "entry-2"}
                        }
                    }
                }]
            }]
        });

        let report = context.seal_payload("chat_stream_partial", &mut payload);

        assert_eq!(report.sealed, 2);
        assert!(
            payload["widgets"][0]["component"]["actionBindings"]["submit"]["pageAction"]
                ["actionId"]
                .is_string()
        );
        assert!(
            payload["widgets"][0]["updates"][0]["value"]["action"]["pageAction"]["actionId"]
                .is_string()
        );
    }

    #[test]
    fn seals_set_action_and_named_event_actions() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "type": "upsertElement",
            "element_id": "page/button",
            "value": {
                "type": "setAction",
                "action": {"name": "workflow_event", "context": {"nodeId": "entry-1"}}
            }
        });

        assert_eq!(context.seal_payload("a2ui", &mut payload).sealed, 1);
        assert!(payload["value"]["action"]["pageAction"]["actionId"].is_string());
        assert!(
            payload["value"]["action"]["context"]
                .get("nodeId")
                .is_none()
        );

        payload["value"] = serde_json::json!({
            "type": "setEventActions",
            "eventName": "click",
            "actions": [
                {"name": "workflow_event", "context": {"nodeId": "entry-1"}},
                {"name": "workflow_event", "context": {"nodeId": "entry-2"}}
            ]
        });
        assert_eq!(context.seal_payload("a2ui", &mut payload).sealed, 2);
        assert!(payload["value"]["actions"][0]["pageAction"]["actionId"].is_string());
        assert!(payload["value"]["actions"][1]["pageAction"]["actionId"].is_string());

        payload["value"] = serde_json::json!({
            "type": "setEventActions",
            "eventName": " ",
            "actions": [
                {"name": "workflow_event", "context": {"nodeId": "entry-1"}}
            ]
        });
        let original = payload.clone();
        assert_eq!(
            context.seal_payload("a2ui", &mut payload),
            LocalPageActionSealingReport::default()
        );
        assert_eq!(payload, original);
    }

    #[test]
    fn set_props_content_is_sealed_like_a_component_body() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "type": "upsertElement",
            "element_id": "page/button",
            "value": {
                "type": "setProps",
                "props": {
                    "pageAction": {"actionId": "attacker"},
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

        let report = context.seal_payload("a2ui", &mut payload);
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
                .starts_with(LOCAL_DYNAMIC_PAGE_ACTION_ID_PREFIX)
        );
        assert!(actions[0]["context"].get("nodeId").is_none());
        assert_eq!(actions[0]["context"]["input"], "kept");
        assert!(actions[1].get("pageAction").is_none());
        assert!(actions[1]["context"].get("nodeId").is_none());
        assert!(actions[1]["context"].get("boardId").is_none());
    }

    #[test]
    fn unknown_update_ops_never_pass_executable_fields_through() {
        let (context, _) = context();
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
                "pageAction": {"actionId": "attacker"}
            }
        });

        let report = context.seal_payload("a2ui", &mut payload);
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
        let report = context.seal_payload("a2ui", &mut payload);
        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 1);
        assert!(payload["value"].get("event_handlers").is_none());
        assert_eq!(payload["value"]["props"]["literalString"], "kept");
    }

    #[test]
    fn non_widget_components_never_carry_action_bindings() {
        let (context, _) = context();
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

        let report = context.seal_payload("a2ui", &mut payload);
        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 2);
        let button = &payload["components"][0]["component"];
        assert!(button.get("actionBindings").is_none());
        assert_eq!(button["label"]["literalString"], "kept");
        let record = &payload["components"][1]["component"];
        assert!(record.get("action_bindings").is_none());
    }

    #[test]
    fn rejects_foreign_and_non_entry_targets_after_stripping_routes() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{"id": "button", "component": {
                "actions": [
                    {"name": "workflow_event", "context": {"nodeId": "entry-1", "appId": "other-app"}},
                    {"name": "workflow_event", "context": {"nodeId": "not-entry", "boardId": "board-1"}}
                ]
            }}]
        });

        let report = context.seal_payload("a2ui", &mut payload);
        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 2);
        for action in payload["components"][0]["component"]["actions"]
            .as_array()
            .unwrap()
        {
            assert!(action.get("pageAction").is_none());
            assert!(action["context"].get("nodeId").is_none());
            assert!(action["context"].get("appId").is_none());
            assert!(action["context"].get("boardId").is_none());
        }
    }

    #[test]
    fn grant_rejects_scope_and_principal_mismatches() {
        let registry = LocalDynamicPageActionRegistry::new(Duration::from_secs(60));
        assert!(registry.resolve("lda1_missing", &scope()).is_err());
        let action_id = registry.insert(&scope(), "entry-1").unwrap();

        let mut wrong_app = scope();
        wrong_app.app_id = "app-2".into();
        assert!(registry.resolve(&action_id, &wrong_app).is_err());

        let mut wrong_event = scope();
        wrong_event.event_id = "event-2".into();
        assert!(registry.resolve(&action_id, &wrong_event).is_err());

        let mut wrong_page = scope();
        wrong_page.page_id = "page-2".into();
        assert!(registry.resolve(&action_id, &wrong_page).is_err());

        let mut wrong_board = scope();
        wrong_board.board_id = "board-2".into();
        assert!(registry.resolve(&action_id, &wrong_board).is_err());

        let mut wrong_version = scope();
        wrong_version.board_version = (1, 2, 4);
        assert!(registry.resolve(&action_id, &wrong_version).is_err());

        let mut wrong_revision = scope();
        wrong_revision.manifest_revision = "per2-other".into();
        assert!(registry.resolve(&action_id, &wrong_revision).is_err());

        let mut wrong_principal = scope();
        wrong_principal.principal = LocalPagePrincipalBinding::authenticated(
            &UserExecutionContext::new("user-2"),
            "token-2",
            "hub-1",
        );
        assert!(registry.resolve(&action_id, &wrong_principal).is_err());
        assert_eq!(registry.resolve(&action_id, &scope()).unwrap(), "entry-1");
    }

    #[test]
    fn authenticated_binding_tracks_the_principal_across_token_refreshes() {
        let user_one = UserExecutionContext::new("user-1");
        let user_two = UserExecutionContext::new("user-2");

        assert_eq!(
            LocalPagePrincipalBinding::authenticated(&user_one, "token-old", "hub-1"),
            LocalPagePrincipalBinding::authenticated(&user_one, "token-new", "hub-1")
        );
        assert_ne!(
            LocalPagePrincipalBinding::authenticated(&user_one, "token", "hub-1"),
            LocalPagePrincipalBinding::authenticated(&user_two, "token", "hub-1")
        );
        assert_ne!(
            LocalPagePrincipalBinding::authenticated(&user_one, "token", "hub-1"),
            LocalPagePrincipalBinding::authenticated(&user_one, "token", "hub-2")
        );
    }

    #[test]
    fn expired_grant_is_removed_and_fails_closed() {
        let registry = LocalDynamicPageActionRegistry::new(Duration::from_secs(60));
        let action_id = registry
            .insert_with_expiry(
                &scope(),
                "entry-1",
                Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
            )
            .unwrap();

        assert!(registry.resolve(&action_id, &scope()).is_err());
        assert!(!registry.grants.lock().unwrap().contains_key(&action_id));
    }

    #[test]
    fn generic_results_and_data_model_updates_are_not_capability_sources() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "type": "surfaceUpdate",
            "components": [{"id": "data", "component": {
                "actions": [{"name": "workflow_event", "context": {"nodeId": "entry-1"}}]
            }}],
            "widgets": [{
                "component": {
                    "type": "widgetInstance",
                    "actionBindings": {
                        "submit": {"workflow": {"flowId": "entry-2"}}
                    }
                }
            }]
        });
        let original = payload.clone();

        assert_eq!(
            context.seal_payload("generic_result", &mut payload),
            LocalPageActionSealingReport::default()
        );
        assert_eq!(payload, original);

        payload = serde_json::json!({
            "type": "dataModelUpdate",
            "surface_id": "page",
            "contents": [{"key": "rows", "value": {"actions": [{
                "name": "workflow_event", "context": {"nodeId": "entry-1"}
            }]}}]
        });
        let original = payload.clone();
        assert_eq!(
            context.seal_payload("a2ui", &mut payload),
            LocalPageActionSealingReport::default()
        );
        assert_eq!(payload, original);
    }
}
