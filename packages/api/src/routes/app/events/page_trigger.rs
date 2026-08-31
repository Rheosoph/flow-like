//! Resolve Page-owned triggers to one server-authorized workflow entry.
//!
//! `PageTrigger` is request data carried beside the caller's normal
//! authentication. A dynamic action capability is deliberately verified here
//! and never enters the authentication middleware.

use std::collections::HashSet;

use flow_like::{
    a2ui::Page,
    flow::{
        board::Board,
        compiled::prerun::{
            PAGE_ACTION_ID_PREFIX, PrerunManifest, PrerunPageExecution, page_execution_revision,
        },
        event::Event,
    },
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    error::ApiError,
    execution::{DYNAMIC_PAGE_ACTION_ID_PREFIX, PageActionClaims, verify_page_action_capability},
    middleware::jwt::AppPermissionResponse,
    permission::role_permission::RolePermissions,
    routes::app::{
        page::get_page::load_event_bound_page,
        prerun_shared::{PrerunPayload, load_prerun_manifest},
    },
    state::AppState,
};

/// A Page lifecycle hook. These names live outside the user-authored action
/// namespace, so a component action cannot replace a lifecycle target.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageSpecialEvent {
    Load,
    Unload,
    Interval,
}

/// Trigger data accepted only when invoking an Event that owns a Page.
///
/// `capability_jwt` is a secondary body value. The caller must still send its
/// ordinary bearer/API-key/PAT authentication through the normal middleware.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PageTrigger {
    Action {
        action_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capability_jwt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        manifest_revision: Option<String>,
    },
    Special {
        special_event: PageSpecialEvent,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        manifest_revision: Option<String>,
    },
}

/// Exact execution target resolved from a Page contract.
///
/// Callers must use these fields for the run record and dispatch request. In
/// particular, they must not fall back to `Event.node_id` for a Page action.
#[derive(Debug, Clone)]
pub struct ResolvedPageTrigger {
    pub board_id: String,
    pub board_version: Option<(u32, u32, u32)>,
    pub node_id: String,
    pub page_id: String,
    pub manifest_revision: String,
    pub prerun: PrerunPayload,
    /// Entry nodes of the exact Page board. Dynamic A2UI output sealing uses
    /// this same allow-list before minting a capability.
    pub entry_node_ids: HashSet<String>,
}

/// Compile the user-independent contract for one Event-bound Page.
///
/// The board-only prerun signature remains the cacheable artifact identity.
/// The Page revision extends it with the exact Page execution map, so changing
/// a handler, widget binding, lifecycle hook, or action order invalidates old
/// static references and dynamic capabilities.
pub fn compile_page_contract(
    board: &Board,
    page: &Page,
) -> Result<(PrerunPageExecution, PrerunPayload, String), ApiError> {
    let page_execution = PrerunPageExecution::from_page(board, page).map_err(|error| {
        tracing::warn!(
            board_id = %board.id,
            page_id = %page.id,
            error = %error,
            "Page execution contract is invalid"
        );
        ApiError::bad_request("The Page execution contract is invalid")
    })?;
    let manifest = PrerunManifest::from_board(board);
    let revision = page_execution_revision(board, &page_execution).map_err(|error| {
        ApiError::internal_error(flow_like_types::anyhow!(
            "failed to derive Page execution revision: {error}"
        ))
    })?;
    let mut prerun = PrerunPayload::from(&manifest);
    prerun.signature = revision.clone();
    Ok((page_execution, prerun, revision))
}

/// Resolve a Page trigger after the route has evaluated normal Event runtime
/// permission and loaded the Event from the requested app.
pub async fn resolve_page_trigger(
    state: &AppState,
    permission: &AppPermissionResponse,
    app_id: &str,
    event: &Event,
    trigger: &PageTrigger,
) -> Result<ResolvedPageTrigger, ApiError> {
    if !permission.has_permission(RolePermissions::ExecuteEvents) {
        return Err(ApiError::forbidden(
            "Page execution requires Event runtime permission",
        ));
    }
    if !event.active {
        return Err(ApiError::forbidden("The Page Event is not active"));
    }

    let configured_page_id = event.default_page_id.as_deref().ok_or_else(|| {
        ApiError::bad_request("Page triggers can only invoke Events that own a Page")
    })?;
    let pinned_version = event.board_version.ok_or_else(|| {
        ApiError::bad_request("Governed Page Events must pin an immutable board version")
    })?;

    let app = state
        .master_app(&permission.identifier(), app_id, state)
        .await
        .map_err(ApiError::internal_error)?;
    let page = load_event_bound_page(&app, event)
        .await
        .map_err(|_| ApiError::bad_request("The Page Event configuration is invalid"))?;
    if page.id != configured_page_id {
        return Err(ApiError::bad_request(
            "The Page Event configuration is invalid",
        ));
    }

    let persisted_manifest =
        load_prerun_manifest(state, app_id, &event.board_id, Some(pinned_version)).await?;

    let board = app
        .open_board(event.board_id.clone(), None, Some(pinned_version))
        .await
        .map_err(|error| {
            tracing::warn!(
                app_id,
                event_id = %event.id,
                board_id = %event.board_id,
                error = %error,
                "Page Event board could not be loaded"
            );
            ApiError::bad_request("The Page Event configuration is invalid")
        })?;
    let board = board.lock().await;
    if board.id != event.board_id || board.version != pinned_version {
        return Err(ApiError::bad_request(
            "The Page Event configuration is invalid",
        ));
    }

    // Versioned manifests carry the Page map produced at publication. A v1
    // artifact or a publication that could not load its Pages has no map, so
    // it safely falls back to compiling this exact immutable Page snapshot.
    let page_execution = persisted_manifest
        .page_events
        .iter()
        .find(|candidate| candidate.page_id == configured_page_id)
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| PrerunPageExecution::from_page(&board, &page))
        .map_err(|error| {
            tracing::warn!(
                board_id = %board.id,
                page_id = %page.id,
                error = %error,
                "Page execution contract is invalid"
            );
            ApiError::bad_request("The Page execution contract is invalid")
        })?;
    let manifest_revision = page_execution_revision(&board, &page_execution).map_err(|error| {
        ApiError::internal_error(flow_like_types::anyhow!(
            "failed to derive Page execution revision: {error}"
        ))
    })?;
    let mut prerun = PrerunPayload::from(&*persisted_manifest);
    prerun.signature = manifest_revision.clone();
    let entry_node_ids = entry_node_ids(&board);
    let node_id = match trigger {
        PageTrigger::Action {
            action_id,
            capability_jwt,
            manifest_revision: requested_revision,
        } => {
            require_current_revision(requested_revision.as_deref(), &manifest_revision)?;
            resolve_action(
                permission,
                app_id,
                event,
                configured_page_id,
                &manifest_revision,
                &page_execution,
                &entry_node_ids,
                action_id,
                capability_jwt.as_deref(),
            )?
        }
        PageTrigger::Special {
            special_event,
            manifest_revision: requested_revision,
        } => {
            require_current_revision(requested_revision.as_deref(), &manifest_revision)?;
            resolve_special(&page_execution, *special_event)?
        }
    };

    if !entry_node_ids.contains(&node_id) {
        return Err(ApiError::bad_request(
            "The Page action does not resolve to an executable entry",
        ));
    }

    Ok(ResolvedPageTrigger {
        board_id: event.board_id.clone(),
        board_version: Some(pinned_version),
        node_id,
        page_id: configured_page_id.to_string(),
        manifest_revision,
        prerun,
        entry_node_ids,
    })
}

#[allow(clippy::too_many_arguments)]
fn resolve_action(
    permission: &AppPermissionResponse,
    app_id: &str,
    event: &Event,
    page_id: &str,
    manifest_revision: &str,
    page_execution: &PrerunPageExecution,
    entry_node_ids: &HashSet<String>,
    action_id: &str,
    capability_jwt: Option<&str>,
) -> Result<String, ApiError> {
    if action_id.starts_with(PAGE_ACTION_ID_PREFIX) {
        if capability_jwt.is_some() {
            return Err(ApiError::bad_request(
                "Static Page actions do not accept a capability token",
            ));
        }
        return page_execution
            .action_events
            .iter()
            .find(|action| action.action_id == action_id)
            .map(|action| action.node_id.clone())
            .ok_or_else(|| ApiError::bad_request("The Page action is stale or invalid"));
    }

    if !action_id.starts_with(DYNAMIC_PAGE_ACTION_ID_PREFIX) {
        return Err(ApiError::bad_request("The Page action id is invalid"));
    }
    let Some(pinned_version) = event.board_version else {
        return Err(ApiError::bad_request(
            "Dynamic Page actions require the Page Event to pin a board version",
        ));
    };
    let token = capability_jwt.ok_or_else(|| {
        ApiError::forbidden("A dynamic Page action requires its capability token")
    })?;
    let claims = verify_page_action_capability(token)
        .map_err(|_| ApiError::forbidden("The Page action capability is invalid"))?;
    let caller_sub = permission.effective_user_id().map_err(|_| {
        ApiError::forbidden("Page execution requires a caller linked to a user account")
    })?;
    let caller_technical_user = permission.technical_user_id();

    let binding = DynamicCapabilityBinding {
        caller_sub: &caller_sub,
        caller_technical_user,
        app_id,
        event_id: &event.id,
        page_id,
        manifest_revision,
        board_id: &event.board_id,
        board_version: pinned_version,
        action_id,
        entry_node_ids,
    };
    if !dynamic_capability_matches(&claims, &binding) {
        return Err(ApiError::forbidden(
            "The Page action capability is invalid for this request",
        ));
    }

    Ok(claims.target_node_id)
}

struct DynamicCapabilityBinding<'a> {
    caller_sub: &'a str,
    caller_technical_user: Option<&'a str>,
    app_id: &'a str,
    event_id: &'a str,
    page_id: &'a str,
    manifest_revision: &'a str,
    board_id: &'a str,
    board_version: (u32, u32, u32),
    action_id: &'a str,
    entry_node_ids: &'a HashSet<String>,
}

fn dynamic_capability_matches(
    claims: &PageActionClaims,
    binding: &DynamicCapabilityBinding<'_>,
) -> bool {
    claims.sub == binding.caller_sub
        && claims.technical_user_id.as_deref() == binding.caller_technical_user
        && claims.source_app_id == binding.app_id
        && claims.source_event_id == binding.event_id
        && claims.source_page_id == binding.page_id
        && claims.source_manifest_revision == binding.manifest_revision
        && claims.target_app_id == binding.app_id
        && claims.target_board_id == binding.board_id
        && claims.target_board_version == binding.board_version
        && claims.action_id == binding.action_id
        && binding.entry_node_ids.contains(&claims.target_node_id)
}

fn resolve_special(
    page_execution: &PrerunPageExecution,
    special: PageSpecialEvent,
) -> Result<String, ApiError> {
    let node_id = match special {
        PageSpecialEvent::Load => page_execution
            .special_events
            .load
            .as_ref()
            .map(|target| target.node_id.clone()),
        PageSpecialEvent::Unload => page_execution
            .special_events
            .unload
            .as_ref()
            .map(|target| target.node_id.clone()),
        PageSpecialEvent::Interval => page_execution
            .special_events
            .interval
            .as_ref()
            .map(|target| target.node_id.clone()),
    };
    node_id.ok_or_else(|| ApiError::bad_request("The Page lifecycle event is not configured"))
}

fn require_current_revision(requested: Option<&str>, current: &str) -> Result<(), ApiError> {
    let requested = requested
        .filter(|revision| !revision.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("The Page manifest revision is required"))?;
    if requested != current {
        return Err(ApiError::bad_request(
            "The Page manifest is stale; reload the Page",
        ));
    }
    Ok(())
}

fn entry_node_ids(board: &Board) -> HashSet<String> {
    board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        .filter(|node| node.start == Some(true))
        .map(|node| node.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend_jwt::TokenType, execution::PAGE_ACTION_CAPABILITY_VERSION};

    fn matching_dynamic_claims() -> PageActionClaims {
        PageActionClaims {
            capability_version: PAGE_ACTION_CAPABILITY_VERSION,
            sub: "user-1".into(),
            technical_user_id: Some("technical-1".into()),
            source_app_id: "app-1".into(),
            source_event_id: "event-1".into(),
            source_page_id: "page-1".into(),
            source_manifest_revision: "manifest-1".into(),
            target_app_id: "app-1".into(),
            target_board_id: "board-1".into(),
            target_board_version: (1, 2, 3),
            target_node_id: "entry-1".into(),
            action_id: "da1_action-1".into(),
            origin_run_id: "run-1".into(),
            origin_locator: "page/button/click/0".into(),
            token_type: TokenType::PageAction,
            iss: "issuer".into(),
            aud: "audience".into(),
            iat: 1,
            nbf: 1,
            exp: 2,
            jti: "jti-1".into(),
        }
    }

    fn matching_dynamic_binding(entry_node_ids: &HashSet<String>) -> DynamicCapabilityBinding<'_> {
        DynamicCapabilityBinding {
            caller_sub: "user-1",
            caller_technical_user: Some("technical-1"),
            app_id: "app-1",
            event_id: "event-1",
            page_id: "page-1",
            manifest_revision: "manifest-1",
            board_id: "board-1",
            board_version: (1, 2, 3),
            action_id: "da1_action-1",
            entry_node_ids,
        }
    }

    #[test]
    fn dynamic_capability_accepts_an_exact_claim_binding() {
        let entry_node_ids = HashSet::from(["entry-1".to_string()]);

        assert!(dynamic_capability_matches(
            &matching_dynamic_claims(),
            &matching_dynamic_binding(&entry_node_ids),
        ));
    }

    #[test]
    fn dynamic_capability_rejects_every_mismatched_claim_binding() {
        let entry_node_ids = HashSet::from(["entry-1".to_string()]);
        let binding = matching_dynamic_binding(&entry_node_ids);
        let matching = matching_dynamic_claims();
        let cases = [
            ("user", {
                let mut claims = matching.clone();
                claims.sub = "user-2".into();
                claims
            }),
            ("technical principal", {
                let mut claims = matching.clone();
                claims.technical_user_id = Some("technical-2".into());
                claims
            }),
            ("source app", {
                let mut claims = matching.clone();
                claims.source_app_id = "app-2".into();
                claims
            }),
            ("Event", {
                let mut claims = matching.clone();
                claims.source_event_id = "event-2".into();
                claims
            }),
            ("Page", {
                let mut claims = matching.clone();
                claims.source_page_id = "page-2".into();
                claims
            }),
            ("manifest", {
                let mut claims = matching.clone();
                claims.source_manifest_revision = "manifest-2".into();
                claims
            }),
            ("target app", {
                let mut claims = matching.clone();
                claims.target_app_id = "app-2".into();
                claims
            }),
            ("board", {
                let mut claims = matching.clone();
                claims.target_board_id = "board-2".into();
                claims
            }),
            ("board version", {
                let mut claims = matching.clone();
                claims.target_board_version = (1, 2, 4);
                claims
            }),
            ("action", {
                let mut claims = matching.clone();
                claims.action_id = "da1_action-2".into();
                claims
            }),
            ("non-entry target", {
                let mut claims = matching;
                claims.target_node_id = "non-entry".into();
                claims
            }),
        ];

        for (field, claims) in cases {
            assert!(
                !dynamic_capability_matches(&claims, &binding),
                "mismatched {field} must fail closed"
            );
        }
    }

    #[test]
    fn page_trigger_keeps_capability_in_the_request_body() {
        let trigger: PageTrigger = serde_json::from_value(serde_json::json!({
            "kind": "action",
            "action_id": "da1_runtime-action",
            "capability_jwt": "secondary-token",
            "manifest_revision": "revision-1"
        }))
        .unwrap();

        assert_eq!(
            trigger,
            PageTrigger::Action {
                action_id: "da1_runtime-action".into(),
                capability_jwt: Some("secondary-token".into()),
                manifest_revision: Some("revision-1".into()),
            }
        );
    }

    #[test]
    fn special_events_have_a_reserved_namespace() {
        let trigger: PageTrigger = serde_json::from_value(serde_json::json!({
            "kind": "special",
            "special_event": "interval",
            "manifest_revision": "revision-1"
        }))
        .unwrap();

        assert_eq!(
            trigger,
            PageTrigger::Special {
                special_event: PageSpecialEvent::Interval,
                manifest_revision: Some("revision-1".into()),
            }
        );
    }

    #[test]
    fn special_events_reject_capability_fields() {
        let trigger = serde_json::from_value::<PageTrigger>(serde_json::json!({
            "kind": "special",
            "special_event": "load",
            "manifest_revision": "revision-1",
            "capability_jwt": "must-not-be-accepted"
        }));
        assert!(trigger.is_err());
    }

    #[test]
    fn missing_and_stale_revisions_fail_closed() {
        assert!(require_current_revision(None, "current").is_err());
        assert!(require_current_revision(Some(""), "current").is_err());
        assert!(require_current_revision(Some("old"), "current").is_err());
        assert!(require_current_revision(Some("current"), "current").is_ok());
    }
}
