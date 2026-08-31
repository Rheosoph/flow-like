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
    /// Exact Page board version. An unpinned Page may stream UI, but its raw
    /// dynamic routes are stripped without minting a reusable capability.
    pub target_board_version: Option<(u32, u32, u32)>,
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
    /// Decorate every recognized executable action in one InterCom payload.
    ///
    /// `message_id` is the stable executor event id when one exists. The
    /// structural path keeps multiple workflow actions on the same trigger
    /// independent and preserves their array order.
    pub fn seal_payload(&self, message_id: &str, payload: &mut Value) -> PageActionSealingReport {
        let mut report = PageActionSealingReport::default();
        let mut path = Vec::new();
        self.walk(message_id, payload, &mut path, &mut report);
        report
    }

    fn walk(
        &self,
        message_id: &str,
        value: &mut Value,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
        match value {
            Value::Object(map) => {
                if is_action_slot(path) && is_workflow_action(map) {
                    self.seal_action(message_id, map, path, report);
                } else if is_binding_slot(path) && is_workflow_binding(map) {
                    self.seal_binding(message_id, map, path, report);
                }

                for (key, child) in map.iter_mut() {
                    // A literalJson value can itself contain a component or an
                    // action list which the frontend decodes later.
                    if matches!(key.as_str(), "literalJson" | "literal_json")
                        && let Value::String(raw) = child
                    {
                        self.walk_embedded_json(message_id, raw, path, report);
                        continue;
                    }
                    path.push(key.clone());
                    self.walk(message_id, child, path, report);
                    path.pop();
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter_mut().enumerate() {
                    path.push(index.to_string());
                    self.walk(message_id, child, path, report);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    fn walk_embedded_json(
        &self,
        message_id: &str,
        raw: &mut String,
        path: &mut Vec<String>,
        report: &mut PageActionSealingReport,
    ) {
        let trimmed = raw.trim_start();
        if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
            return;
        }
        let Ok(mut parsed) = serde_json::from_str::<Value>(raw) else {
            return;
        };
        path.push("$literalJson".to_string());
        self.walk(message_id, &mut parsed, path, report);
        path.pop();
        if let Ok(encoded) = serde_json::to_string(&parsed) {
            *raw = encoded;
        }
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
        if let Some(workflow) = workflow_binding_mut(binding) {
            for key in [
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
                workflow.remove(key);
            }
        }

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
        let Some(target_board_version) = self.target_board_version else {
            return Ok(None);
        };
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
            target_board_version,
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

/// Only mutate values in A2UI execution slots. Workflow-shaped application
/// data may appear anywhere in a model and must remain ordinary data.
fn is_action_slot(path: &[String]) -> bool {
    let Some(index) = path.last() else {
        return false;
    };
    if index.parse::<usize>().is_err() {
        return false;
    }

    matches!(
        path.get(path.len().saturating_sub(2)).map(String::as_str),
        Some("actions")
    ) || matches!(
        path.get(path.len().saturating_sub(3)).map(String::as_str),
        Some("eventHandlers" | "event_handlers")
    )
}

fn is_binding_slot(path: &[String]) -> bool {
    matches!(
        path.get(path.len().saturating_sub(2)).map(String::as_str),
        Some("actionBindings")
    )
}

fn is_workflow_action(map: &Map<String, Value>) -> bool {
    map.get("name").and_then(Value::as_str) == Some("workflow_event")
        && map.get("context").is_some_and(Value::is_object)
}

fn is_workflow_binding(map: &Map<String, Value>) -> bool {
    workflow_binding(map).is_some_and(|workflow| {
        [
            "flowId", "flow_id", "eventId", "event_id", "nodeId", "node_id",
        ]
        .iter()
        .any(|key| workflow.contains_key(*key))
    })
}

fn workflow_binding(map: &Map<String, Value>) -> Option<&Map<String, Value>> {
    ["workflow", "workflowEvent", "workflow_event"]
        .iter()
        .find_map(|key| map.get(*key).and_then(Value::as_object))
}

fn workflow_binding_mut(map: &mut Map<String, Value>) -> Option<&mut Map<String, Value>> {
    for key in ["workflow", "workflowEvent", "workflow_event"] {
        if map.get(key).is_some_and(Value::is_object) {
            return map.get_mut(key).and_then(Value::as_object_mut);
        }
    }
    None
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

        let report = context().seal_payload("message-1", &mut payload);
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
    fn seals_widget_and_micro_widget_bindings_without_exposing_flow_id() {
        let mut payload = serde_json::json!({
            "component": {
                "type": "microWidgetInstance",
                "actionBindings": {
                    "approve": {
                        "workflow": {"flowId": "entry-1", "inputMappings": {}}
                    }
                }
            }
        });

        let report = context().seal_payload("message-2", &mut payload);
        assert_eq!(report.sealed, 1);
        let binding = &payload["component"]["actionBindings"]["approve"];
        assert!(binding["workflow"].get("flowId").is_none());
        assert!(binding["pageAction"]["capabilityJwt"].is_string());
    }

    #[test]
    fn rejects_foreign_or_non_entry_targets_fail_closed() {
        let mut payload = serde_json::json!({
            "actions": [
                {"name": "workflow_event", "context": {
                    "nodeId": "entry-1", "boardId": "other-board"
                }, "pageAction": {"capabilityJwt": "attacker"}},
                {"name": "workflow_event", "context": {"nodeId": "not-an-entry"},
                    "page_action": {"capability_jwt": "attacker"}}
            ]
        });

        let report = context().seal_payload("message-3", &mut payload);
        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 2);
        for action in payload["actions"].as_array().unwrap() {
            assert!(action.get("pageAction").is_none());
            assert!(action.get("page_action").is_none());
            assert!(action["context"].get("nodeId").is_none());
            assert!(action["context"].get("boardId").is_none());
        }
    }

    #[test]
    fn rejects_foreign_widget_binding_and_strips_every_routing_alias() {
        let mut payload = serde_json::json!({
            "actionBindings": {
                "submit": {
                    "workflowEvent": {
                        "eventId": "entry-1",
                        "appId": "other-app",
                        "boardId": "other-board",
                        "contextMapping": {"value": {"literalString": "kept"}}
                    }
                }
            }
        });

        let report = context().seal_payload("message-widget-foreign", &mut payload);
        let binding = &payload["actionBindings"]["submit"];
        let workflow = &binding["workflowEvent"];

        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 1);
        assert!(binding.get("pageAction").is_none());
        assert!(workflow.get("eventId").is_none());
        assert!(workflow.get("appId").is_none());
        assert!(workflow.get("boardId").is_none());
        assert_eq!(workflow["contextMapping"]["value"]["literalString"], "kept");
    }

    #[test]
    fn unpinned_page_strips_allowed_target_without_minting_capability() {
        let mut context = context();
        context.target_board_version = None;
        let mut payload = serde_json::json!({
            "actions": [{
                "name": "workflow_event",
                "context": {"nodeId": "entry-1", "input": "kept"}
            }]
        });

        let report = context.seal_payload("message-unpinned", &mut payload);
        let action = &payload["actions"][0];

        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 1);
        assert!(action.get("pageAction").is_none());
        assert!(action["context"].get("nodeId").is_none());
        assert_eq!(action["context"]["input"], "kept");
    }

    #[test]
    fn seals_actions_inside_literal_json() {
        let mut payload = serde_json::json!({
            "value": {"literalJson": "{\"actions\":[{\"name\":\"workflow_event\",\"context\":{\"nodeId\":\"entry-1\"}}]}"}
        });
        let report = context().seal_payload("message-4", &mut payload);
        assert_eq!(report.sealed, 1);
        let inner: Value =
            serde_json::from_str(payload["value"]["literalJson"].as_str().unwrap()).unwrap();
        assert!(inner["actions"][0]["pageAction"]["capabilityJwt"].is_string());
    }

    #[test]
    fn seals_actions_inside_snake_case_literal_json() {
        let mut payload = serde_json::json!({
            "value": {"literal_json": "{\"actions\":[{\"name\":\"workflow_event\",\"context\":{\"node_id\":\"entry-1\"}}]}"}
        });

        let report = context().seal_payload("message-1", &mut payload);
        let embedded: Value =
            serde_json::from_str(payload["value"]["literal_json"].as_str().unwrap()).unwrap();

        assert_eq!(report.sealed, 1);
        assert!(embedded["actions"][0]["context"].get("node_id").is_none());
        assert!(embedded["actions"][0]["pageAction"]["actionId"]
            .as_str()
            .is_some_and(|id| id.starts_with(DYNAMIC_PAGE_ACTION_ID_PREFIX)));
        assert!(embedded["actions"][0]["pageAction"]["capabilityJwt"]
            .as_str()
            .is_some());
    }

    #[test]
    fn leaves_workflow_shaped_model_data_untouched() {
        let mut payload = serde_json::json!({
            "model": {
                "example": {
                    "name": "workflow_event",
                    "context": {"nodeId": "entry-1"}
                },
                "record": {
                    "workflow": {"flowId": "entry-2"}
                }
            }
        });
        let original = payload.clone();

        let report = context().seal_payload("message-data", &mut payload);

        assert_eq!(report, PageActionSealingReport::default());
        assert_eq!(payload, original);
    }
}
