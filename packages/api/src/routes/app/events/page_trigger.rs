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
    routes::app::prerun_shared::{
        PrerunPayload, ensure_draft_board_snapshot, ensure_draft_prerun_manifest,
        ensure_versioned_page_prerun_manifest, load_exact_prerun_manifest, load_prerun_manifest,
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
    /// Source object identity for a floating `Latest` board. Pinned versions
    /// are already immutable and leave this empty.
    pub board_etag: Option<String>,
    pub node_id: String,
    pub page_id: String,
    pub manifest_revision: String,
    pub prerun: PrerunPayload,
    /// Signature of the exact board-only prerun manifest used as compact
    /// callback authority. Legacy pinned manifests leave this empty.
    pub entry_authority_revision: Option<String>,
    /// WASM package-set revision required by a previously minted dynamic
    /// capability. Static/current actions leave this empty.
    pub wasm_authority_revision: Option<String>,
    /// Entry nodes of the exact Page board. Dynamic A2UI output sealing uses
    /// this same allow-list before minting a capability.
    pub entry_node_ids: HashSet<String>,
}

/// Exact Page authority resolved from one Event selector.
///
/// A floating Event keeps `board_version=None`; `board_etag` identifies the
/// draft object used to compile this contract and later dispatches the matching
/// content-addressed compiled artifact.
#[derive(Clone)]
pub struct ResolvedPageContract {
    pub board_etag: Option<String>,
    pub page: Page,
    pub page_execution: PrerunPageExecution,
    pub manifest_revision: String,
    pub prerun: PrerunPayload,
    pub entry_node_ids: HashSet<String>,
    pub entry_authority_revision: Option<String>,
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

/// Load the exact Board and Page selected by an Event and compile their
/// governed execution contract.
///
/// `master_board_shared` performs an ETag-conditional read for Latest. The
/// returned ETag therefore names the same Board value held in `board`; callers
/// can pass it through dispatch so an executor never substitutes a newer draft
/// after action authorization.
pub async fn resolve_page_contract(
    state: &AppState,
    app_id: &str,
    event: &Event,
) -> Result<ResolvedPageContract, ApiError> {
    resolve_page_contract_inner(state, app_id, event, false).await
}

/// Bootstrap keeps its established not-found response when a configured Page
/// snapshot is unavailable, while invocation reports the same condition as an
/// invalid Event contract.
pub async fn resolve_page_contract_for_bootstrap(
    state: &AppState,
    app_id: &str,
    event: &Event,
) -> Result<ResolvedPageContract, ApiError> {
    resolve_page_contract_inner(state, app_id, event, true).await
}

async fn resolve_page_contract_inner(
    state: &AppState,
    app_id: &str,
    event: &Event,
    missing_as_not_found: bool,
) -> Result<ResolvedPageContract, ApiError> {
    let configured_page_id = event.default_page_id.as_deref().ok_or_else(|| {
        ApiError::bad_request("Page triggers can only invoke Events that own a Page")
    })?;

    let cached = state
        .master_board_shared(app_id, &event.board_id, state, event.board_version)
        .await
        .map_err(|error| {
            tracing::warn!(
                app_id,
                event_id = %event.id,
                board_id = %event.board_id,
                error = %error,
                "Page Event board could not be loaded"
            );
            if missing_as_not_found {
                ApiError::NOT_FOUND
            } else {
                ApiError::bad_request("The Page Event configuration is invalid")
            }
        })?;
    let board = cached.board.clone();
    if board.id != event.board_id
        || event
            .board_version
            .is_some_and(|expected| board.version != expected)
    {
        return Err(ApiError::bad_request(
            "The Page Event configuration is invalid",
        ));
    }

    let board_etag = if event.board_version.is_none() {
        Some(cached.e_tag.trim().to_string()).filter(|etag| !etag.is_empty())
    } else {
        None
    };
    if event.board_version.is_none() && board_etag.is_none() {
        return Err(ApiError::internal_error(flow_like_types::anyhow!(
            "Latest Page board '{}' has no storage ETag",
            event.board_id
        )));
    }

    let page = match event.board_version {
        Some(version) => {
            board
                .load_versioned_page(configured_page_id, version, None)
                .await
        }
        None => board.load_page(configured_page_id, None).await,
    }
    .map_err(|_| {
        if missing_as_not_found {
            ApiError::NOT_FOUND
        } else {
            ApiError::bad_request("The Page Event configuration is invalid")
        }
    })?;
    if page.id != configured_page_id
        || page
            .board_id
            .as_deref()
            .is_some_and(|board_id| board_id != board.id)
    {
        return Err(ApiError::bad_request(
            "The Page Event configuration is invalid",
        ));
    }

    // Published manifests carry the Page map produced at publication. Latest
    // uses the same prerun artifact shape, cached by the exact Board ETag and
    // canonical Page revision, so unchanged drafts do not rebuild their map.
    let (manifest, entry_node_ids, entry_authority_revision) = match event.board_version {
        Some(version) => {
            let authority =
                load_prerun_manifest(state, app_id, &event.board_id, Some(version)).await?;
            let manifest = ensure_versioned_page_prerun_manifest(
                state,
                app_id,
                &event.board_id,
                version,
                &board,
                &page,
                authority.clone(),
            )
            .await?;
            let entry_node_ids = authority
                .entry_node_ids
                .iter()
                .cloned()
                .collect::<HashSet<_>>();
            let entry_authority_revision =
                (!entry_node_ids.is_empty()).then(|| authority.signature.clone());
            (manifest, entry_node_ids, entry_authority_revision)
        }
        None => {
            ensure_draft_board_snapshot(state, app_id, &event.board_id, &cached).await?;
            let authority =
                ensure_draft_prerun_manifest(state, app_id, &event.board_id, &cached).await?;
            let entry_node_ids = authority
                .entry_node_ids
                .iter()
                .cloned()
                .collect::<HashSet<_>>();
            let entry_authority_revision = Some(authority.signature.clone());
            let manifest =
                crate::routes::app::prerun_shared::draft_page_prerun_manifest_for_cached_board(
                    state,
                    app_id,
                    &event.board_id,
                    &cached,
                    &authority,
                    &page,
                )
                .await?;
            (manifest, entry_node_ids, entry_authority_revision)
        }
    };
    let page_execution = manifest
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
    let mut prerun = PrerunPayload::from(&*manifest);
    prerun.signature = manifest_revision.clone();
    Ok(ResolvedPageContract {
        board_etag,
        page,
        page_execution,
        manifest_revision,
        prerun,
        entry_node_ids,
        entry_authority_revision,
    })
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
    if let PageTrigger::Action {
        action_id,
        capability_jwt: Some(capability_jwt),
        ..
    } = trigger
        && action_id.starts_with(DYNAMIC_PAGE_ACTION_ID_PREFIX)
    {
        return resolve_dynamic_page_trigger(
            state,
            permission,
            app_id,
            event,
            configured_page_id,
            action_id,
            capability_jwt,
        )
        .await;
    }
    let contract = resolve_page_contract(state, app_id, event).await?;
    let manifest_revision = &contract.manifest_revision;
    let page_execution = &contract.page_execution;
    let entry_node_ids = &contract.entry_node_ids;
    let node_id = match trigger {
        PageTrigger::Action {
            action_id,
            capability_jwt,
            manifest_revision: requested_revision,
        } => {
            require_current_revision(requested_revision.as_deref(), manifest_revision)?;
            resolve_action(page_execution, action_id, capability_jwt.as_deref())?
        }
        PageTrigger::Special {
            special_event,
            manifest_revision: requested_revision,
        } => {
            require_current_revision(requested_revision.as_deref(), manifest_revision)?;
            resolve_special(page_execution, *special_event)?
        }
    };

    if !entry_node_ids.contains(&node_id) {
        return Err(ApiError::bad_request(
            "The Page action does not resolve to an executable entry",
        ));
    }

    Ok(ResolvedPageTrigger {
        board_id: event.board_id.clone(),
        board_version: event.board_version,
        board_etag: contract.board_etag,
        node_id,
        page_id: configured_page_id.to_string(),
        manifest_revision: contract.manifest_revision,
        prerun: contract.prerun,
        entry_node_ids: contract.entry_node_ids,
        entry_authority_revision: contract.entry_authority_revision,
        wasm_authority_revision: None,
    })
}

async fn resolve_dynamic_page_trigger(
    state: &AppState,
    permission: &AppPermissionResponse,
    app_id: &str,
    event: &Event,
    page_id: &str,
    action_id: &str,
    capability_jwt: &str,
) -> Result<ResolvedPageTrigger, ApiError> {
    let claims = verify_page_action_capability(capability_jwt)
        .map_err(|_| ApiError::forbidden("The Page action capability is invalid"))?;
    let caller_sub = permission.effective_user_id().map_err(|_| {
        ApiError::forbidden("Page execution requires a caller linked to a user account")
    })?;
    let binding = DynamicCapabilityBinding {
        caller_sub: &caller_sub,
        caller_technical_user: permission.technical_user_id(),
        app_id,
        event_id: &event.id,
        page_id,
        board_id: &event.board_id,
        action_id,
    };
    if !dynamic_capability_binds_request(&claims, &binding) {
        return Err(ApiError::forbidden(
            "The Page action capability is invalid for this request",
        ));
    }

    let board_etag = claims
        .target_board_etag
        .as_deref()
        .map(str::trim)
        .filter(|etag| !etag.is_empty());
    if !dynamic_capability_board_selector_matches(
        event.board_version,
        claims.target_board_version,
        board_etag,
    ) {
        return Err(ApiError::forbidden(
            "The Page action capability has an invalid board selector",
        ));
    }

    let manifest = load_exact_prerun_manifest(
        state,
        app_id,
        &event.board_id,
        claims.target_board_version,
        board_etag,
    )
    .await?;
    let mut entry_node_ids = manifest
        .entry_node_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let mut entry_authority_revision = Some(manifest.signature.clone());
    if entry_node_ids.is_empty() && claims.target_board_version.is_some() {
        // Compatibility for immutable v1/v2 prerun manifests. Those formats
        // predate compact entry authority, so derive it from the same pinned
        // board without weakening an ETag-bound Latest capability.
        let cached = state
            .master_board_shared(app_id, &event.board_id, state, claims.target_board_version)
            .await?;
        entry_node_ids = self::entry_node_ids(&cached.board);
        entry_authority_revision = None;
    }
    if !entry_node_ids.contains(&claims.target_node_id) {
        return Err(ApiError::forbidden(
            "The Page action capability target is not executable",
        ));
    }

    let mut prerun = PrerunPayload::from(manifest.as_ref());
    prerun.signature = claims.source_manifest_revision.clone();
    Ok(ResolvedPageTrigger {
        board_id: claims.target_board_id,
        board_version: claims.target_board_version,
        board_etag: claims.target_board_etag,
        node_id: claims.target_node_id,
        page_id: claims.source_page_id,
        manifest_revision: claims.source_manifest_revision,
        prerun,
        entry_node_ids,
        entry_authority_revision,
        wasm_authority_revision: claims.target_wasm_authority_revision,
    })
}

/// Resolve a static Page action against the current execution contract.
///
/// A dynamic (`da1_`) id can only be authorized by
/// `resolve_dynamic_page_trigger`, which intercepts every request carrying a
/// capability token — so on this path a dynamic id is always missing its
/// token and is refused.
fn resolve_action(
    page_execution: &PrerunPageExecution,
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
    Err(ApiError::forbidden(
        "A dynamic Page action requires its capability token",
    ))
}

/// The request-side identity a dynamic capability must bind to exactly.
struct DynamicCapabilityBinding<'a> {
    caller_sub: &'a str,
    caller_technical_user: Option<&'a str>,
    app_id: &'a str,
    event_id: &'a str,
    page_id: &'a str,
    board_id: &'a str,
    action_id: &'a str,
}

fn dynamic_capability_binds_request(
    claims: &PageActionClaims,
    binding: &DynamicCapabilityBinding<'_>,
) -> bool {
    claims.sub == binding.caller_sub
        && claims.technical_user_id.as_deref() == binding.caller_technical_user
        && claims.source_app_id == binding.app_id
        && claims.source_event_id == binding.event_id
        && claims.source_page_id == binding.page_id
        && claims.target_app_id == binding.app_id
        && claims.target_board_id == binding.board_id
        && claims.action_id == binding.action_id
        && !claims.source_manifest_revision.trim().is_empty()
        && !claims.target_node_id.trim().is_empty()
}

/// A pinned Event accepts only its exact version; an ETag-bound Latest Event
/// accepts only a versionless claim carrying an ETag. The ETag itself is then
/// bound by loading the exact prerun authority it names.
fn dynamic_capability_board_selector_matches(
    event_board_version: Option<(u32, u32, u32)>,
    claims_board_version: Option<(u32, u32, u32)>,
    claims_board_etag: Option<&str>,
) -> bool {
    match event_board_version {
        Some(version) => claims_board_version == Some(version) && claims_board_etag.is_none(),
        None => claims_board_version.is_none() && claims_board_etag.is_some(),
    }
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
    use axum::http::StatusCode;

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
            target_board_version: Some((1, 2, 3)),
            target_board_etag: None,
            target_wasm_authority_revision: Some("wasm-revision-1".into()),
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

    fn matching_dynamic_binding() -> DynamicCapabilityBinding<'static> {
        DynamicCapabilityBinding {
            caller_sub: "user-1",
            caller_technical_user: Some("technical-1"),
            app_id: "app-1",
            event_id: "event-1",
            page_id: "page-1",
            board_id: "board-1",
            action_id: "da1_action-1",
        }
    }

    fn empty_page_execution() -> PrerunPageExecution {
        let board = Board::new_detached(
            Some("board-1".into()),
            flow_like_storage::Path::from("apps").child("app-1"),
        );
        let page = flow_like::a2ui::Page::new("page-1", "Page", "/");
        PrerunPageExecution::from_page(&board, &page).unwrap()
    }

    #[test]
    fn static_path_rejects_a_dynamic_action_without_its_capability() {
        let page_execution = empty_page_execution();

        // A `da1_` id without a capability token never resolves on the static
        // path — the shortcut into `resolve_dynamic_page_trigger` only fires
        // when the request carries one.
        let refused = resolve_action(&page_execution, "da1_action-1", None).unwrap_err();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);

        // Ids outside both namespaces stay a bad request.
        let invalid = resolve_action(&page_execution, "unknown-action", None).unwrap_err();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        // Static ids reject a smuggled capability token and unknown targets.
        let static_id = format!("{PAGE_ACTION_ID_PREFIX}action-1");
        let smuggled = resolve_action(&page_execution, &static_id, Some("token")).unwrap_err();
        assert_eq!(smuggled.status(), StatusCode::BAD_REQUEST);
        let stale = resolve_action(&page_execution, &static_id, None).unwrap_err();
        assert_eq!(stale.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn dynamic_capability_accepts_an_exact_claim_binding() {
        assert!(dynamic_capability_binds_request(
            &matching_dynamic_claims(),
            &matching_dynamic_binding(),
        ));
    }

    #[test]
    fn dynamic_capability_binds_the_board_selector_to_the_event() {
        // A pinned Event accepts only its exact version, never an ETag claim.
        assert!(dynamic_capability_board_selector_matches(
            Some((1, 2, 3)),
            Some((1, 2, 3)),
            None
        ));
        assert!(!dynamic_capability_board_selector_matches(
            Some((1, 2, 3)),
            Some((1, 2, 4)),
            None
        ));
        assert!(!dynamic_capability_board_selector_matches(
            Some((1, 2, 3)),
            Some((1, 2, 3)),
            Some("etag-1")
        ));
        assert!(!dynamic_capability_board_selector_matches(
            Some((1, 2, 3)),
            None,
            Some("etag-1")
        ));

        // An ETag-bound Latest Event accepts only a versionless ETag claim;
        // the ETag value itself is then bound by loading the exact prerun
        // authority it names.
        assert!(dynamic_capability_board_selector_matches(
            None,
            None,
            Some("etag-current")
        ));
        assert!(!dynamic_capability_board_selector_matches(
            None,
            Some((1, 2, 3)),
            None
        ));
        assert!(!dynamic_capability_board_selector_matches(None, None, None));
    }

    #[test]
    fn dynamic_capability_rejects_every_mismatched_claim_binding() {
        let binding = matching_dynamic_binding();
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
            ("empty manifest revision", {
                let mut claims = matching.clone();
                claims.source_manifest_revision = "  ".into();
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
            ("action", {
                let mut claims = matching.clone();
                claims.action_id = "da1_action-2".into();
                claims
            }),
            ("empty target node", {
                let mut claims = matching;
                claims.target_node_id = " ".into();
                claims
            }),
        ];

        for (field, claims) in cases {
            assert!(
                !dynamic_capability_binds_request(&claims, &binding),
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
