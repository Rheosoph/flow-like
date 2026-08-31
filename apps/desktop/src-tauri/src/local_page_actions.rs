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

    pub(crate) fn seal_payload(&self, payload: &mut Value) -> LocalPageActionSealingReport {
        let mut report = LocalPageActionSealingReport::default();
        let mut path = Vec::new();
        self.walk(payload, &mut path, &mut report);
        report
    }

    fn walk(
        &self,
        value: &mut Value,
        path: &mut Vec<String>,
        report: &mut LocalPageActionSealingReport,
    ) {
        match value {
            Value::Object(map) => {
                if is_action_slot(path) && is_workflow_action(map) {
                    self.seal_action(map, report);
                } else if is_binding_slot(path) && is_workflow_binding(map) {
                    self.seal_binding(map, report);
                }

                for (key, child) in map.iter_mut() {
                    if matches!(key.as_str(), "literalJson" | "literal_json")
                        && let Value::String(raw) = child
                    {
                        self.walk_embedded_json(raw, path, report);
                        continue;
                    }
                    path.push(key.clone());
                    self.walk(child, path, report);
                    path.pop();
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter_mut().enumerate() {
                    path.push(index.to_string());
                    self.walk(child, path, report);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    fn walk_embedded_json(
        &self,
        raw: &mut String,
        path: &mut Vec<String>,
        report: &mut LocalPageActionSealingReport,
    ) {
        let trimmed = raw.trim_start();
        if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
            return;
        }
        let Ok(mut parsed) = serde_json::from_str::<Value>(raw) else {
            return;
        };
        path.push("$literalJson".to_string());
        self.walk(&mut parsed, path, report);
        path.pop();
        if let Ok(encoded) = serde_json::to_string(&parsed) {
            *raw = encoded;
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
            "component": {
                "eventHandlers": {
                    "click": [
                        {"name": "workflow_event", "context": {"nodeId": "entry-1", "input": "kept"}},
                        {"name": "navigate_page", "context": {"route": "/next"}},
                        {"name": "workflow_event", "context": {"nodeId": "entry-2"}}
                    ]
                }
            }
        });

        let report = context.seal_payload(&mut payload);
        assert_eq!(report.sealed, 2);
        assert_eq!(report.rejected, 0);
        let actions = payload["component"]["eventHandlers"]["click"]
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
    fn seals_widget_bindings_and_literal_json() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "component": {
                "actionBindings": {
                    "approve": {
                        "workflow": {"flowId": "entry-1", "inputMappings": {"value": "kept"}}
                    }
                },
                "data": {
                    "literalJson": "{\"actions\":[{\"name\":\"workflow_event\",\"context\":{\"nodeId\":\"entry-2\"}}]}"
                }
            }
        });

        let report = context.seal_payload(&mut payload);
        assert_eq!(report.sealed, 2);
        let binding = &payload["component"]["actionBindings"]["approve"];
        assert!(binding["workflow"].get("flowId").is_none());
        assert_eq!(binding["workflow"]["inputMappings"]["value"], "kept");
        assert!(binding["pageAction"]["actionId"].is_string());

        let embedded: Value = serde_json::from_str(
            payload["component"]["data"]["literalJson"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert!(embedded["actions"][0]["pageAction"]["actionId"].is_string());
    }

    #[test]
    fn seals_actions_inside_snake_case_literal_json() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "value": {
                "literal_json": "{\"actions\":[{\"name\":\"workflow_event\",\"context\":{\"nodeId\":\"entry-1\"}}]}"
            }
        });

        assert_eq!(context.seal_payload(&mut payload).sealed, 1);
        let embedded: Value =
            serde_json::from_str(payload["value"]["literal_json"].as_str().unwrap()).unwrap();
        assert!(embedded["actions"][0]["pageAction"]["actionId"].is_string());
        assert!(embedded["actions"][0]["context"].get("nodeId").is_none());
    }

    #[test]
    fn rejects_foreign_and_non_entry_targets_after_stripping_routes() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "actions": [
                {"name": "workflow_event", "context": {"nodeId": "entry-1", "appId": "other-app"}},
                {"name": "workflow_event", "context": {"nodeId": "not-entry", "boardId": "board-1"}}
            ]
        });

        let report = context.seal_payload(&mut payload);
        assert_eq!(report.sealed, 0);
        assert_eq!(report.rejected, 2);
        for action in payload["actions"].as_array().unwrap() {
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
    fn workflow_shaped_model_data_is_not_treated_as_an_action() {
        let (context, _) = context();
        let mut payload = serde_json::json!({
            "model": {
                "action": {"name": "workflow_event", "context": {"nodeId": "entry-1"}},
                "binding": {"workflow": {"flowId": "entry-2"}}
            }
        });
        let original = payload.clone();

        assert_eq!(
            context.seal_payload(&mut payload),
            LocalPageActionSealingReport::default()
        );
        assert_eq!(payload, original);
    }
}
